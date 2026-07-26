//! SavedVariables parsing via an embedded, sandboxed Lua (spec §2). WoW SV files
//! are full Lua — a table of assignments like `SBFData = { ... }` — so we evaluate
//! them in a `mlua` state with NO standard library (no `io`/`os`/`require`) and
//! walk the export global into a compact Rust view.
//!
//! **Verbatim, not flattened.** Per the Uplink data contract
//! (`docs/…/2026-07-13-uplink-data-contract.md`) Uplink is a *courier*: every field
//! of every stream entry, every registry item, and the learned-item catalog is
//! carried through EXACTLY as the addon wrote it. So the heart of this module is a
//! generic `lua_to_json` conversion (any Lua value → `serde_json::Value`) — a table
//! whose keys are exactly the consecutive integers `1..n` becomes a JSON array,
//! every other table becomes a JSON object (integer keys stringified), and scalars
//! map naturally. Nothing is renamed, dropped, or interpreted here.
//!
//! A file caught mid-write fails to `eval` and returns an `Err`; the caller skips
//! it and retries next pass — this never panics.

use mlua::{Lua, LuaOptions, StdLib, Table, Value};
use serde_json::{Map, Number, Value as Json};
use std::collections::HashMap;
use std::path::Path;

/// A nesting-depth backstop so a pathological (or malicious) deeply-nested table
/// can't blow the Rust stack during conversion. Real SV data nests <20 deep; 256
/// is comfortable headroom while still bounding recursion.
const MAX_DEPTH: usize = 256;

/// One stream's parsed view: its `schema` version (separated out of the Lua table)
/// plus every entry converted verbatim, oldest-first (ascending integer index).
#[derive(Debug, Clone)]
pub struct ParsedStream {
    pub schema: i64,
    pub entries: Vec<Json>,
}

/// One stream's durable counter, from the top-level `_streamMeta` SV table
/// (seq-identity design 2026-07-24). `seq` is the high-water mark (highest `seq`
/// ever assigned — never reset by a purge/wipe/reload); `base` is the deletion
/// watermark (everything with `seq < base` was intentionally deleted). Uplink slices
/// `seq > cursor` and lifts its cursor to `max(cursor, base − 1)` so a purge that
/// outran sync can never leave a stream permanently stuck.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct StreamMeta {
    pub seq: i64,
    pub base: i64,
}

/// The parsed, self-contained view of one addon export global (e.g. `SBFData`).
/// Registry domains and stream entries are kept VERBATIM as JSON values.
#[derive(Debug, Clone, Default)]
pub struct SvExport {
    // ── snapshot metadata, from `_format` (written at logout) ──
    pub producer_build: Option<String>,
    pub format_version: Option<i64>,
    pub schema_version: Option<i64>,
    pub registry_version: Option<i64>,
    pub generated_at: Option<i64>,
    /// The WHOLE `_format` export envelope, verbatim (formatVersion, producer,
    /// producerBuild, registryVersion, schemaVersion, per-stream `{schema}`,
    /// generatedAt). Rides the ingest envelope's `format` pass-through (wire §10.2).
    /// Empty when the file has no `_format`.
    pub format: Map<String, Json>,
    /// The whole `_registry`, carried VERBATIM: each domain name
    /// (`char`/`place`/`item`/`spell`/…future) → its full `{ items = […], _byKey = {…} }`
    /// wrapper, unchanged. The server resolves `ch`/`p` as `registry.<domain>.items[idx-1]`
    /// (contract §2 rule 2; the committed `testdata/sessions/*/envelope.json` fixtures are
    /// the byte target). Do NOT unwrap `items` — a bare array reads as `.items ==
    /// undefined` at ingest and silently empties every domain. Empty when there's no
    /// registry.
    pub registry: Map<String, Json>,
    /// `db.items` — the learned-item catalog (contract §5a), each record with its
    /// itemID inlined as `id`. Snapshot, not a stream.
    pub db_items: Vec<Json>,
    /// Top-level `sessions` MAP (session-model wire §10.3), each frozen record with
    /// its `sid` (the map key) inlined. Record-upsert dataset — no per-entry `i`; the
    /// completeness gate (ship only `closedAt`-bearing records) is applied by the
    /// uploader, not here. Empty when the file carries no `sessions`.
    pub sessions: Vec<Json>,
    /// `streams.<name>` → its parsed entries + schema.
    pub streams: HashMap<String, ParsedStream>,
    /// Top-level `_streamMeta.<name>` → `{seq, base}` (seq-identity design). A sibling
    /// of `streams`, NOT inside `_format`. Empty when the file predates the migration
    /// (records then carry no `seq`, and the seq-slice sends nothing until the addon
    /// ships MINOR 25 — the coordinated rollout, by design).
    pub stream_meta: HashMap<String, StreamMeta>,
}

/// Read + parse an SV file, returning the named export global's view.
/// `global` is e.g. "SBFData" / "HaulData". Errors (unreadable, bad Lua, missing
/// global) are returned as `Err` for the caller to skip-and-retry.
pub fn parse_export_file(path: &Path, global: &str) -> Result<SvExport, String> {
    let src = std::fs::read(path).map_err(|e| format!("read {path:?}: {e}"))?;
    parse_export_str(&src, global)
}

/// The core (bytes → export). Split out so tests can feed literals without a file.
pub fn parse_export_str(src: &[u8], global: &str) -> Result<SvExport, String> {
    // No standard library at all: the file is data, and this closes off io/os/
    // require/load entirely. A 1 GiB memory cap backstops a pathological file (a
    // ~9 MB SV inflates to a few tens of MB of Lua tables; 1 GiB is comfortable
    // headroom while still bounding a malicious blow-up).
    let lua = Lua::new_with(StdLib::NONE, LuaOptions::default())
        .map_err(|e| format!("lua init: {e}"))?;
    lua.set_memory_limit(1024 * 1024 * 1024).ok();

    lua.load(src)
        .set_name(global)
        .exec()
        .map_err(|e| format!("eval {global}: {e}"))?;

    let g: Value = lua.globals().get(global).map_err(|e| e.to_string())?;
    let root = match g {
        Value::Table(t) => t,
        _ => return Err(format!("global {global} is not a table (empty SV?)")),
    };

    let mut out = SvExport::default();

    // ── snapshot metadata (`_format`) ──
    if let Ok(fmt) = root.get::<Table>("_format") {
        out.producer_build = fmt.get::<String>("producerBuild").ok().filter(|s| !s.is_empty());
        out.format_version = fmt.get::<i64>("formatVersion").ok();
        out.schema_version = fmt.get::<i64>("schemaVersion").ok();
        out.registry_version = fmt.get::<i64>("registryVersion").ok();
        out.generated_at = fmt.get::<i64>("generatedAt").ok();
        // …and the whole envelope verbatim for the wire `format` pass-through (§10.2).
        if let Json::Object(m) = lua_to_json(Value::Table(fmt), 0)? {
            out.format = m;
        }
    }

    // ── frozen per-session records (`sessions` MAP, sid-keyed) ──
    // Top-level (NOT under `streams`): a sid-keyed record-upsert, so we inline the
    // map key as `sid` (the server keys on it, §10.3) and carry the record verbatim.
    if let Ok(sessions) = root.get::<Table>("sessions") {
        for pair in sessions.pairs::<Value, Value>() {
            let Ok((k, v)) = pair else { continue };
            let sid = key_to_string(&k);
            let mut rec = match lua_to_json(v, 0)? {
                Json::Object(m) => m,
                other => {
                    let mut m = Map::new();
                    m.insert("value".into(), other);
                    m
                }
            };
            rec.entry("sid".to_string()).or_insert(Json::String(sid));
            out.sessions.push(Json::Object(rec));
        }
    }

    // ── registry: every domain carried VERBATIM (the `{ items = {…}, _byKey = {…} }`
    // wrapper kept intact) ── the server resolves `ch`/`p` as `registry.<domain>.items[idx-1]`
    // (contract §2 rule 2; committed `envelope.json` fixtures are the byte target). Do NOT
    // unwrap `items`: a bare array reads as `.items == undefined` at ingest and silently
    // empties every domain — the empty-registry bug (handoff 2026-07-18).
    if let Ok(reg) = root.get::<Table>("_registry") {
        for pair in reg.pairs::<Value, Value>() {
            let Ok((k, v)) = pair else { continue };
            out.registry.insert(key_to_string(&k), lua_to_json(v, 0)?);
        }
    }

    // ── learned-item catalog (`db.items`), itemID inlined as `id` ──
    if let Ok(db) = root.get::<Table>("db") {
        if let Ok(items) = db.get::<Table>("items") {
            for pair in items.pairs::<Value, Value>() {
                let Ok((k, v)) = pair else { continue };
                let mut rec = match lua_to_json(v, 0)? {
                    Json::Object(m) => m,
                    other => {
                        // non-object record (shouldn't happen) — wrap so id can ride
                        let mut m = Map::new();
                        m.insert("value".into(), other);
                        m
                    }
                };
                // inline the itemID key (don't clobber a real `id` field if present)
                if let Some(id) = key_as_int(&k) {
                    rec.entry("id".to_string()).or_insert(Json::Number(id.into()));
                }
                out.db_items.push(Json::Object(rec));
            }
        }
    }

    // ── streams: entries verbatim, `schema` separated ──
    if let Ok(streams) = root.get::<Table>("streams") {
        // schema defaults live in `_format.streams.<name>.schema`
        let schema_defaults: HashMap<String, i64> = root
            .get::<Table>("_format")
            .and_then(|f| f.get::<Table>("streams"))
            .map(|s| {
                let mut m = HashMap::new();
                for pair in s.pairs::<String, Table>().flatten() {
                    if let Ok(sc) = pair.1.get::<i64>("schema") {
                        m.insert(pair.0, sc);
                    }
                }
                m
            })
            .unwrap_or_default();

        for pair in streams.pairs::<Value, Value>() {
            let Ok((k, v)) = pair else { continue };
            let name = key_to_string(&k);
            let Value::Table(t) = v else { continue };
            let default = schema_defaults.get(&name).copied().unwrap_or(1);
            let parsed = parse_stream(&t, default)?;
            out.streams.insert(name, parsed);
        }
    }

    // ── per-stream durable counter (`_streamMeta.<name> = {seq, base}`) ──
    // A top-level table, sibling of `streams` (seq-identity design 2026-07-24). We read
    // `seq` (high-water) and `base` (deletion watermark) for each named stream. `base`
    // defaults to 1 (nothing deleted) when the key is absent.
    if let Ok(meta) = root.get::<Table>("_streamMeta") {
        for pair in meta.pairs::<Value, Table>() {
            let Ok((k, m)) = pair else { continue };
            let name = key_to_string(&k);
            let seq = value_as_int(&m.get::<Value>("seq").unwrap_or(Value::Nil)).unwrap_or(0);
            let base = value_as_int(&m.get::<Value>("base").unwrap_or(Value::Nil)).unwrap_or(1);
            out.stream_meta.insert(name, StreamMeta { seq, base });
        }
    }

    Ok(out)
}

/// Parse one `streams.<name>` table: the `schema` field is a string key pulled
/// aside (else the `_format` default); the entries are the integer sequence `1..n`,
/// converted verbatim in order. We iterate the sequence and convert each entry
/// immediately — never holding many live Lua references at once (a Vec of thousands
/// of `Value` handles exhausts mlua's auxiliary reference stack).
fn parse_stream(t: &Table, schema_default: i64) -> Result<ParsedStream, String> {
    let schema = value_as_int(&t.get::<Value>("schema").unwrap_or(Value::Nil)).unwrap_or(schema_default);
    let mut entries = Vec::new();
    for v in t.sequence_values::<Value>() {
        let v = v.map_err(|e| format!("stream entry: {e}"))?;
        entries.push(lua_to_json(v, 0)?);
    }
    Ok(ParsedStream { schema, entries })
}

/// Generic Lua value → JSON. Tables whose keys are exactly `1..n` become arrays;
/// all other tables become objects (integer keys stringified). Scalars map
/// naturally; non-finite floats and function/userdata values (never present in SV
/// data) degrade to `null`.
fn lua_to_json(v: Value, depth: usize) -> Result<Json, String> {
    if depth > MAX_DEPTH {
        return Err(format!("table nested deeper than {MAX_DEPTH} — refusing"));
    }
    Ok(match v {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(b),
        Value::Integer(i) => Json::Number((i as i64).into()),
        Value::Number(n) => Number::from_f64(n).map(Json::Number).unwrap_or(Json::Null),
        Value::String(s) => Json::String(s.to_string_lossy().to_string()),
        Value::Table(t) => table_to_json(&t, depth)?,
        // functions / userdata / threads don't occur in a data-only SV file
        _ => Json::Null,
    })
}

/// Decide array-vs-object for a table and convert its contents.
///
/// CRITICAL: we never collect the child `Value`s into a `Vec` (thousands of live
/// mlua reference handles overflow Lua's auxiliary stack — ~8000 slots). Instead:
/// pass 1 classifies KEYS only, letting each value drop immediately; pass 2 fetches
/// and converts children one at a time (`sequence_values` for arrays, `pairs` for
/// objects), so at most one child ref (plus the recursion path) is ever alive.
fn table_to_json(t: &Table, depth: usize) -> Result<Json, String> {
    // ── pass 1: classify keys (values dropped as we go) ──
    let mut count = 0usize;
    let mut max_int = 0i64;
    let mut has_non_seq_key = false;
    for pair in t.pairs::<Value, Value>() {
        let (k, _v) = pair.map_err(|e| format!("table key: {e}"))?; // _v dropped here
        count += 1;
        match key_as_int(&k) {
            Some(i) if i >= 1 => {
                if i > max_int {
                    max_int = i;
                }
            }
            _ => has_non_seq_key = true,
        }
    }

    // Array iff every key is a positive integer and they're exactly 1..count (table
    // keys are unique, so no gap + max == count ⇒ the run 1..count).
    let is_array = !has_non_seq_key && (count == 0 || max_int as usize == count);

    if is_array {
        // ── pass 2 (array): the sequence 1..n, one value alive at a time ──
        let mut arr = Vec::with_capacity(count);
        for v in t.sequence_values::<Value>() {
            arr.push(lua_to_json(v.map_err(|e| format!("seq value: {e}"))?, depth + 1)?);
        }
        Ok(Json::Array(arr))
    } else {
        // ── pass 2 (object): convert each value immediately, keep only JSON ──
        let mut map = Map::new();
        for pair in t.pairs::<Value, Value>() {
            let (k, v) = pair.map_err(|e| format!("table pair: {e}"))?;
            map.insert(key_to_string(&k), lua_to_json(v, depth + 1)?);
        }
        Ok(Json::Object(map))
    }
}

/// An integer key value, if this Lua value is an integer (or an integral float).
fn key_as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(i) => Some(*i as i64),
        Value::Number(n) if n.is_finite() && n.fract() == 0.0 => Some(*n as i64),
        _ => None,
    }
}

/// A scalar as `i64` (for `schema` fields written as either int or float).
fn value_as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Integer(i) => Some(*i as i64),
        Value::Number(n) if n.is_finite() => Some(*n as i64),
        _ => None,
    }
}

/// A table key rendered as a JSON object key (JSON keys are always strings).
fn key_to_string(v: &Value) -> String {
    match v {
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => {
            if n.is_finite() && n.fract() == 0.0 {
                (*n as i64).to_string()
            } else {
                n.to_string()
            }
        }
        Value::String(s) => s.to_string_lossy().to_string(),
        Value::Boolean(b) => b.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SAMPLE: &[u8] = br#"
SBFData = {
  ["version"] = 1,
  ["_format"] = {
    ["producerBuild"] = "2026.07.11.4",
    ["formatVersion"] = 1, ["schemaVersion"] = 1, ["registryVersion"] = 2,
    ["generatedAt"] = 1783860305,
    ["streams"] = { ["fishlog"] = { ["schema"] = 1 } },
  },
  ["_registry"] = {
    ["char"] = { ["items"] = {
      { ["guid"]="Player-1", ["name"]="Zephire", ["class"]="DRUID",
        ["state"] = { ["gold"]=0, ["professions"] = { ["roster"] = {
          [356] = { ["name"]="Fishing", ["rank"]=300 },
          [197] = { ["name"]="Tailoring", ["rank"]=82 },
        } } } },
    } },
    ["place"] = { ["items"] = {
      { ["key"]="12>88", ["cascade"] = {
          { ["name"]="Kalimdor", ["kind"]="continent", ["mapID"]=12 },
          { ["name"]="Thunder Bluff", ["kind"]="zone", ["mapID"]=88 } } },
    } },
    ["item"] = { ["items"] = {
      { ["id"]=238366, ["name"]="Lynxfish" },
    } },
  },
  ["db"] = { ["items"] = {
    [133702] = { ["name"]="Aromatic Murloc Slime", ["slots"] = { ["chum_perception"]=true },
                 ["maps"] = { ["630"]="Azsuna" } },
  } },
  ["streams"] = {
    ["fishlog"] = {
      { ["k"]="expired", ["p"]=1, ["t"]=1782973264, ["gen"]=3, ["dur"]=22, ["x"]=45.8, ["src"]="SBF" },
      { ["k"]="caught", ["id"]=238366, ["name"]="Lynxfish", ["count"]=1, ["p"]=1, ["t"]=1782973301,
        ["q"]=1, ["ch"]=1, ["gen"]=3, ["link"]="|Hitem:238366|h[Lynxfish]|h" },
      { ["k"]="action", ["spell"]=1706, ["slotKey"]="pole", ["t"]=1782973301, ["gen"]=3 },
    },
  },
}
"#;

    #[test]
    fn snapshot_metadata_parsed() {
        let x = parse_export_str(SAMPLE, "SBFData").unwrap();
        assert_eq!(x.producer_build.as_deref(), Some("2026.07.11.4"));
        assert_eq!(x.format_version, Some(1));
        assert_eq!(x.schema_version, Some(1));
        assert_eq!(x.registry_version, Some(2));
        assert_eq!(x.generated_at, Some(1783860305));
    }

    #[test]
    fn registry_domains_keep_items_wrapper_verbatim() {
        let x = parse_export_str(SAMPLE, "SBFData").unwrap();
        // each domain is carried VERBATIM as `{ items = [...], _byKey = {...} }` — the
        // server resolves ch/p as registry.<domain>.items[idx-1], NEVER a bare array
        // (unwrapping strips `.items` and the server reads every domain as empty).
        let char_dom = x.registry.get("char").unwrap();
        assert!(char_dom.is_object(), "domain keeps its wrapper object, not a bare array");
        let chars = char_dom["items"].as_array().unwrap();
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0]["name"], json!("Zephire"));
        assert_eq!(chars[0]["state"]["gold"], json!(0));
        // non-consecutive integer keys (profession line ids) → OBJECT, not array
        let roster = &chars[0]["state"]["professions"]["roster"];
        assert!(roster.is_object());
        assert_eq!(roster["356"]["name"], json!("Fishing"));
        assert_eq!(roster["197"]["rank"], json!(82));
        // place cascade is a real array (1..n) preserved in order, under `.items`
        let places = x.registry.get("place").unwrap()["items"].as_array().unwrap();
        let cascade = places[0]["cascade"].as_array().unwrap();
        assert_eq!(cascade.len(), 2);
        assert_eq!(cascade[0]["kind"], json!("continent"));
        assert_eq!(cascade[1]["name"], json!("Thunder Bluff"));
        // item registry rides along too (spec §2.2)
        assert!(x.registry.get("item").is_some());
    }

    #[test]
    fn stream_entries_verbatim_all_fields() {
        let x = parse_export_str(SAMPLE, "SBFData").unwrap();
        let fl = x.streams.get("fishlog").unwrap();
        assert_eq!(fl.schema, 1);
        assert_eq!(fl.entries.len(), 3);
        // oldest-first order preserved
        assert_eq!(fl.entries[0]["k"], json!("expired"));
        let caught = &fl.entries[1];
        assert_eq!(caught["k"], json!("caught"));
        assert_eq!(caught["id"], json!(238366));
        assert_eq!(caught["name"], json!("Lynxfish"));
        assert_eq!(caught["q"], json!(1));
        assert_eq!(caught["count"], json!(1));
        assert_eq!(caught["ch"], json!(1));
        assert_eq!(caught["link"], json!("|Hitem:238366|h[Lynxfish]|h"));
        // float field survives
        assert_eq!(fl.entries[0]["x"], json!(45.8));
        // an `action` entry keeps its extra fields (spell/slotKey) — not dropped
        let action = &fl.entries[2];
        assert_eq!(action["k"], json!("action"));
        assert_eq!(action["spell"], json!(1706));
        assert_eq!(action["slotKey"], json!("pole"));
    }

    #[test]
    fn learned_items_inline_id_and_nested_maps() {
        let x = parse_export_str(SAMPLE, "SBFData").unwrap();
        assert_eq!(x.db_items.len(), 1);
        let it = &x.db_items[0];
        assert_eq!(it["id"], json!(133702));
        assert_eq!(it["name"], json!("Aromatic Murloc Slime"));
        // string-keyed maps stay objects; bool-valued slot flags survive
        assert_eq!(it["slots"]["chum_perception"], json!(true));
        assert_eq!(it["maps"]["630"], json!("Azsuna"));
    }

    #[test]
    fn empty_and_object_tables() {
        // consecutive 1..n → array; a hole or a string key → object
        let src = br#"X = { arr = {10,20,30}, holey = {[1]=1,[3]=3}, mixed = {[1]="a", k="b"}, empty = {} }"#;
        let x = parse_export_str(src, "X").unwrap();
        // there are no streams/registry; probe via a tiny direct conversion instead:
        // reparse through registry by wrapping — simpler: assert it didn't error.
        assert!(x.streams.is_empty());
    }

    #[test]
    fn array_vs_object_detection() {
        // Directly exercise table_to_json via a stream (entries are arrays of objects).
        let src = br#"
SBFData = { ["streams"] = { ["s"] = {
  { ["seq"] = {10,20,30}, ["holey"] = { [1]=1, [3]=3 }, ["mixed"] = { [1]="a", ["k"]="b" } },
} } } "#;
        let x = parse_export_str(src, "SBFData").unwrap();
        let e = &x.streams.get("s").unwrap().entries[0];
        assert!(e["seq"].is_array());
        assert_eq!(e["seq"], json!([10, 20, 30]));
        // gap (1,3) → object with stringified keys
        assert!(e["holey"].is_object());
        assert_eq!(e["holey"]["1"], json!(1));
        assert_eq!(e["holey"]["3"], json!(3));
        // int + string keys mixed → object
        assert!(e["mixed"].is_object());
        assert_eq!(e["mixed"]["1"], json!("a"));
        assert_eq!(e["mixed"]["k"], json!("b"));
    }

    // ── session-model streams (wire §10): sessions map + markers stream + format ──

    const SESSIONS_SAMPLE: &[u8] = br#"
HaulData = {
  version = 6,
  sessions = {
    ["closed-1"] = {
      builds = { "2026.07.16.6" },
      gameEnv = { clientBuild = "11.0.5", interface = 110005, flavor = 1 },
      schemaVersion = 6, startedAt = 100, closedAt = 700,
      pauses = {}, character = 1, prices = {}, exclusions = {},
    },
    ["open-2"] = {   -- live session: NO closedAt, uploader must withhold it
      builds = { "2026.07.16.6" },
      gameEnv = { clientBuild = "11.0.5", interface = 110005, flavor = 1 },
      schemaVersion = 6, startedAt = 800,
      pauses = {}, character = 1, prices = {}, exclusions = {},
    },
  },
  streams = {
    events  = { { t = 110, k = "coin", sid = "closed-1", amount = 50, gen = 1 } },
    markers = {
      { t = 100, k = "start", sid = "closed-1", who = "Gonefishin", gen = 1 },
      { t = 700, k = "stop",  sid = "closed-1", gen = 1 },
    },
  },
  _format = {
    formatVersion = 1, producer = "Haul", producerBuild = "2026.07.16.6",
    registryVersion = 2, schemaVersion = 6,
    streams = { events = { schema = 6 }, markers = { schema = 6 } },
    generatedAt = 999,
  },
}
"#;

    #[test]
    fn sessions_map_parsed_with_sid_inlined_and_close_gate_visible() {
        let x = parse_export_str(SESSIONS_SAMPLE, "HaulData").unwrap();
        assert_eq!(x.sessions.len(), 2);
        // each record carries its map key inlined as `sid`
        let by_sid = |sid: &str| x.sessions.iter().find(|s| s["sid"] == json!(sid)).unwrap();
        let closed = by_sid("closed-1");
        assert_eq!(closed["closedAt"], json!(700));
        assert_eq!(closed["gameEnv"]["flavor"], json!(1));
        // the open/live record parses too but has NO closedAt (uploader gate drops it)
        let open = by_sid("open-2");
        assert!(open.get("closedAt").is_none());
    }

    #[test]
    fn markers_are_their_own_stream_split_from_events() {
        let x = parse_export_str(SESSIONS_SAMPLE, "HaulData").unwrap();
        let ev = x.streams.get("events").unwrap();
        let mk = x.streams.get("markers").unwrap();
        assert_eq!(ev.entries.len(), 1);
        assert_eq!(ev.entries[0]["k"], json!("coin"));
        // markers is a SEPARATE stream (start/stop), never mixed into events
        assert_eq!(mk.entries.len(), 2);
        assert_eq!(mk.entries[0]["k"], json!("start"));
        assert_eq!(mk.entries[1]["k"], json!("stop"));
    }

    #[test]
    fn format_envelope_passed_through_verbatim() {
        let x = parse_export_str(SESSIONS_SAMPLE, "HaulData").unwrap();
        // the whole _format rides for the envelope `format` pass-through (§10.2)
        assert_eq!(x.format["formatVersion"], json!(1));
        assert_eq!(x.format["producer"], json!("Haul"));
        assert_eq!(x.format["streams"]["markers"]["schema"], json!(6));
        // scalars still extracted for internal use
        assert_eq!(x.schema_version, Some(6));
        assert_eq!(x.producer_build.as_deref(), Some("2026.07.16.6"));
    }

    // Parity against the COMMITTED dalaran-start fixture: the parser must reproduce
    // envelope.json's stream names, session record, and format (modulo the transport
    // `i`/`since` annotations, which are the uploader's layer, not the parser's).
    #[test]
    fn parses_committed_dalaran_fixture() {
        // Fixture COPIED (not moved) into the crate: the monorepo's
        // testdata/sessions/ tree is SHARED (website/tests/fixtures.test.ts consumes
        // it too), so the source of truth stays at the repo root; this in-crate copy
        // exists only so `../GEC/uplink` is self-contained and `cargo test` compiles
        // in the public mirror. Keep the two in sync if the fixture changes.
        let sv = include_bytes!("../testdata/sessions/dalaran-start/sv.lua");
        let x = parse_export_str(sv, "HaulData").unwrap();
        // streams: events + markers, oldest-first, verbatim fields
        let ev = x.streams.get("events").unwrap();
        assert_eq!(ev.entries.len(), 3);
        assert_eq!(ev.entries[0]["id"], json!(124124));
        assert_eq!(ev.entries[0]["count"], json!(3));
        assert_eq!(ev.entries[2]["k"], json!("coin"));
        assert_eq!(ev.entries[2]["amount"], json!(1850));
        let mk = x.streams.get("markers").unwrap();
        assert_eq!(mk.entries.len(), 2);
        assert_eq!(mk.entries[0]["k"], json!("start"));
        assert_eq!(mk.entries[1]["k"], json!("stop"));
        // one closed session, sid inlined, prices frozen on the record
        assert_eq!(x.sessions.len(), 1);
        let s = &x.sessions[0];
        assert_eq!(s["sid"], json!("66961b80-1a2b"));
        assert_eq!(s["closedAt"], json!(1721160600));
        assert_eq!(s["prices"]["152509"]["unit"], json!(45000));
        // registry rides along; format passes through
        assert!(x.registry.get("char").is_some());
        assert_eq!(x.format["producer"], json!("Haul"));
    }

    // ── per-stream `_streamMeta{seq,base}` (seq-identity design 2026-07-24) ──
    // Top-level SV table, sibling of `streams`. Additive: records already carry
    // their `seq` verbatim through `lua_to_json`; this table is the durable counter
    // (seq high-water) + deletion watermark (base) the slice/cursor now key on.
    #[test]
    fn stream_meta_parsed_seq_and_base() {
        let src = br#"
SBFData = {
  ["_streamMeta"] = {
    ["events"]  = { ["seq"] = 1043, ["base"] = 452 },
    ["markers"] = { ["seq"] = 20,   ["base"] = 1 },
  },
  ["streams"] = { ["events"] = {
    { ["seq"] = 1042, ["k"] = "caught" },
    { ["seq"] = 1043, ["k"] = "caught" },
  } },
}
"#;
        let x = parse_export_str(src, "SBFData").unwrap();
        let ev = x.stream_meta.get("events").unwrap();
        assert_eq!(ev.seq, 1043);
        assert_eq!(ev.base, 452);
        let mk = x.stream_meta.get("markers").unwrap();
        assert_eq!(mk.seq, 20);
        assert_eq!(mk.base, 1);
        // each record still carries its own `seq` verbatim (server dedup keys on it)
        assert_eq!(x.streams.get("events").unwrap().entries[1]["seq"], json!(1043));
    }

    // Absent `_streamMeta` (pre-migration file) parses to an empty map, never an error.
    #[test]
    fn missing_stream_meta_is_empty_not_error() {
        let x = parse_export_str(SAMPLE, "SBFData").unwrap();
        assert!(x.stream_meta.is_empty());
    }

    #[test]
    fn missing_global_errs_not_panics() {
        assert!(parse_export_str(b"OtherThing = {}", "SBFData").is_err());
    }

    #[test]
    fn broken_lua_errs() {
        assert!(parse_export_str(b"SBFData = { broken", "SBFData").is_err());
    }

    #[test]
    fn no_stdlib_available() {
        assert!(parse_export_str(b"SBFData = os.time()", "SBFData").is_err());
    }
}
