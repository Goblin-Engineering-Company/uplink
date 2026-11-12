//! Packed sessions (GECStore MINOR 30): reading `<SV>.packed.<stream>` chunks and
//! checking them against the plain stream. The contract is
//! `docs/superpowers/specs/2026-11-11-gecstore-packed-sessions-contract.md`; the
//! section numbers below (§3.3, §5, …) point into it.
//!
//! A closed session's stream entries can be stored as ONE JSON string per session
//! ("chunk") instead of a Lua table per entry. Two modes, named by
//! `_format.packed.mode`:
//!
//! - **shadow**: the stream is complete and the chunks are a copy. We upload from the
//!   stream exactly as before and only VERIFY that every packed entry equals its
//!   stream twin (§5). Nothing here changes what gets uploaded in shadow mode.
//! - **primary**: packed sessions are removed from the stream. We MERGE: stream
//!   entries + every decoded chunk entry, sorted ascending by `seq`, which is exactly
//!   what the stream held before packing. Cursors, chunking, dedupe and acks all key
//!   on `seq`, so the uploader needs no change. Any problem at all in primary mode
//!   (a seq in both places, a chunk we can't read, totals that don't add up) refuses
//!   the whole file: uploading around a hole would move the cursor past entries the
//!   server never got.
//!
//! **Numbers (§3.3).** mlua in Lua 5.1 mode hands a Lua number back as
//! `Value::Integer` whenever it's integral and fits an i64 (and, quirk included, when
//! it's within `f64::EPSILON` of one, so `1e-17` comes back as `0`). Decoded chunk
//! numbers go through that SAME rule (`lua_number_json`), so a packed entry is the
//! identical `serde_json::Value` the unpacked path builds, and the upload is
//! byte-identical either way. The §5 comparison is still numeric, as a backstop.

use crate::svparse::{lua_to_json, ParsedStream};
use mlua::{Table, Value};
use serde::Serialize;
use serde_json::{Map, Number, Value as Json};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The pack version this reader understands (`lib.PACK_VERSION` in GECStore).
pub const PACK_VERSION: i64 = 1;

/// How many items of each list `PackReport::lines` prints before summarizing.
const PRINT_CAP: usize = 20;

/// Per-file result of reading the packed copy. A dev-readable report (the
/// `verify-packed` command prints it) and the gate for primary mode.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PackReport {
    /// `_format.packed.mode` ("shadow" | "primary"), as written.
    pub mode: Option<String>,
    /// `_format.packed.version` / `encoding` / `error`, as written.
    pub version: Option<i64>,
    pub encoding: Option<String>,
    pub error: Option<String>,
    /// `_format.packed.skipped`: sid → reason. Informational: a skipped session simply
    /// stays in the stream (§3.5).
    pub skipped: BTreeMap<String, String>,
    /// File-level problems (missing / unknown envelope fields, unknown mode, …).
    pub problems: Vec<String>,
    /// One report per packed stream (v1 packs only `events`).
    pub streams: BTreeMap<String, PackStreamReport>,
}

/// The checks for one packed stream.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PackStreamReport {
    /// `_format.packed.streams.<name>.{sessions,entries}`, as written.
    pub envelope_sessions: Option<i64>,
    pub envelope_entries: Option<i64>,
    /// Chunks that passed every header check, and the entries they held.
    pub chunks_decoded: u64,
    pub entries_decoded: u64,
    /// Entries in the plain stream table, and (primary) the merged total uploaded.
    pub stream_entries: u64,
    pub merged_entries: u64,
    /// `(sid key, why)`: value not a string, or not parseable JSON / not an object.
    pub undecodable: Vec<(String, String)>,
    /// `(sid key, v as written)`: a pack version we don't know. Chunk skipped, never guessed.
    pub unknown_version: Vec<(String, String)>,
    /// `(map key, chunk.sid)` when they differ.
    pub sid_key_mismatch: Vec<(String, String)>,
    /// `(sid, detail)`: `n != len(e)`.
    pub n_mismatch: Vec<(String, String)>,
    /// `(sid, detail)`: `seq != [e[0].seq, e[last].seq]`.
    pub seq_mismatch: Vec<(String, String)>,
    /// Per-entry problems: no integer `seq`, not ascending, `sid` ≠ the chunk's sid.
    pub entry_problems: Vec<String>,
    /// Seqs that appear twice: in two chunks, or (primary) in a chunk AND the stream.
    pub duplicate_seqs: Vec<i64>,
    /// Shadow: packed seqs with no stream entry.
    pub missing_in_stream: Vec<i64>,
    /// Shadow: `(seq, first differing path + detail)`.
    pub mismatched: Vec<(i64, String)>,
    /// `(sid, count)`. Shadow: stream entries of a closed, non-skipped session that
    /// are in no chunk. Primary: entries of a packed session still left in the stream.
    pub stream_not_packed: Vec<(String, u64)>,
}

impl PackStreamReport {
    fn totals_match(&self) -> bool {
        self.envelope_sessions == Some(self.chunks_decoded as i64)
            && self.envelope_entries == Some(self.entries_decoded as i64)
    }

    pub fn is_clean(&self) -> bool {
        self.totals_match()
            && self.undecodable.is_empty()
            && self.unknown_version.is_empty()
            && self.sid_key_mismatch.is_empty()
            && self.n_mismatch.is_empty()
            && self.seq_mismatch.is_empty()
            && self.entry_problems.is_empty()
            && self.duplicate_seqs.is_empty()
            && self.missing_in_stream.is_empty()
            && self.mismatched.is_empty()
            && self.stream_not_packed.is_empty()
    }
}

impl PackReport {
    /// Zero of everything (contract §5). Skipped sessions don't count against it.
    pub fn is_clean(&self) -> bool {
        self.error.is_none() && self.problems.is_empty() && self.streams.values().all(|s| s.is_clean())
    }

    pub fn is_primary(&self) -> bool {
        self.mode.as_deref() == Some("primary")
    }

    /// One-line verdict for logs and errors.
    pub fn summary(&self) -> String {
        let mode = self.mode.as_deref().unwrap_or("?");
        let mut parts = Vec::new();
        for (name, s) in &self.streams {
            parts.push(format!(
                "{name}: {} chunks / {} entries (envelope {}/{})",
                s.chunks_decoded,
                s.entries_decoded,
                opt(s.envelope_sessions),
                opt(s.envelope_entries)
            ));
        }
        format!(
            "packed {mode}: {} [{}]",
            if self.is_clean() { "CLEAN" } else { "PROBLEMS" },
            parts.join("; ")
        )
    }

    /// The full human-readable report, one line per fact.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![self.summary()];
        out.push(format!(
            "  version={} encoding={} skipped={}",
            opt(self.version),
            self.encoding.as_deref().unwrap_or("?"),
            self.skipped.len()
        ));
        for (sid, why) in &self.skipped {
            out.push(format!("    skipped {sid}: {why}"));
        }
        if let Some(e) = &self.error {
            out.push(format!("  ERROR (envelope): {e}"));
        }
        for p in &self.problems {
            out.push(format!("  PROBLEM: {p}"));
        }
        for (name, s) in &self.streams {
            out.push(format!(
                "  [{name}] stream table {} entries, merged {}; totals {}",
                s.stream_entries,
                s.merged_entries,
                if s.totals_match() { "match" } else { "DO NOT MATCH the envelope" }
            ));
            list(&mut out, "undecodable", &s.undecodable, |(a, b)| format!("{a}: {b}"));
            list(&mut out, "unknown version", &s.unknown_version, |(a, b)| format!("{a}: v={b}"));
            list(&mut out, "sid key mismatch", &s.sid_key_mismatch, |(a, b)| format!("key {a} vs chunk sid {b}"));
            list(&mut out, "n mismatch", &s.n_mismatch, |(a, b)| format!("{a}: {b}"));
            list(&mut out, "seq mismatch", &s.seq_mismatch, |(a, b)| format!("{a}: {b}"));
            list(&mut out, "entry problems", &s.entry_problems, |x| x.clone());
            list(&mut out, "duplicate seqs", &s.duplicate_seqs, |x| x.to_string());
            list(&mut out, "missing in stream", &s.missing_in_stream, |x| x.to_string());
            list(&mut out, "mismatched", &s.mismatched, |(q, p)| format!("seq {q}: {p}"));
            list(&mut out, "stream entries not packed", &s.stream_not_packed, |(a, n)| format!("{a}: {n}"));
        }
        out
    }
}

fn opt(v: Option<i64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "?".into())
}

fn list<T>(out: &mut Vec<String>, label: &str, items: &[T], f: impl Fn(&T) -> String) {
    if items.is_empty() {
        return;
    }
    out.push(format!("    {label}: {}", items.len()));
    for x in items.iter().take(PRINT_CAP) {
        out.push(format!("      {}", f(x)));
    }
    if items.len() > PRINT_CAP {
        out.push(format!("      … {} more", items.len() - PRINT_CAP));
    }
}

/// Read `root.packed` against the already-parsed streams. `format` is the verbatim
/// `_format` envelope; `open_sid` is `root._open.sid`. Returns `None` when the file
/// has neither `packed` nor `_format.packed` (an older file: read exactly as today).
///
/// Shadow mode leaves `streams` untouched. Primary mode replaces each packed stream's
/// entries with the merged list, or returns `Err` if anything is wrong (§4.2).
pub fn apply(
    root: &Table,
    format: &Map<String, Json>,
    open_sid: Option<&str>,
    schema_defaults: &HashMap<String, i64>,
    streams: &mut HashMap<String, ParsedStream>,
) -> Result<Option<PackReport>, String> {
    let env = format.get("packed");
    let packed: Option<Table> = match root.get::<Value>("packed") {
        Ok(Value::Table(t)) => Some(t),
        _ => None,
    };
    if env.is_none() && packed.is_none() {
        return Ok(None);
    }

    let mut rep = PackReport::default();
    let env_obj = env.and_then(|v| v.as_object());
    match env_obj {
        None => rep.problems.push("`packed` present but `_format.packed` is missing or not a table".into()),
        Some(e) => {
            rep.mode = e.get("mode").and_then(|v| v.as_str()).map(String::from);
            rep.version = e.get("version").and_then(|v| v.as_i64());
            rep.encoding = e.get("encoding").and_then(|v| v.as_str()).map(String::from);
            rep.error = e.get("error").and_then(|v| v.as_str()).map(String::from);
            if let Some(sk) = e.get("skipped").and_then(|v| v.as_object()) {
                for (sid, why) in sk {
                    rep.skipped.insert(sid.clone(), why.as_str().map(String::from).unwrap_or_else(|| why.to_string()));
                }
            }
            match rep.mode.as_deref() {
                Some("shadow") | Some("primary") => {}
                other => rep.problems.push(format!("unknown mode {other:?} (expected \"shadow\" or \"primary\")")),
            }
            // version/encoding describe the chunks; a failed pack (`error`, no `packed`)
            // stamps neither, and the error already makes the report unclean
            if packed.is_some() && rep.version != Some(PACK_VERSION) {
                rep.problems.push(format!("envelope version {} (this reader knows {PACK_VERSION})", opt(rep.version)));
            }
            if packed.is_some() && rep.encoding.as_deref() != Some("json") {
                rep.problems.push(format!("envelope encoding {:?} (expected \"json\")", rep.encoding));
            }
            // every stream the envelope totals, even one with no chunks under `packed`
            if let Some(st) = e.get("streams").and_then(|v| v.as_object()) {
                for (name, tot) in st {
                    let s = rep.streams.entry(name.clone()).or_default();
                    s.envelope_sessions = tot.get("sessions").and_then(|v| v.as_i64());
                    s.envelope_entries = tot.get("entries").and_then(|v| v.as_i64());
                }
            }
        }
    }
    // A mode we don't know means we can't tell whether the stream is complete, so we
    // can't tell what to upload. Don't guess (§2.3 / §7): refuse the file.
    if !matches!(rep.mode.as_deref(), Some("shadow") | Some("primary")) {
        return Err(format!("refusing to upload: {}\n  {}", rep.summary(), rep.problems.join("\n  ")));
    }
    let primary = rep.is_primary();

    // Per packed stream: what the chunks covered, plus (primary only) their entries
    // waiting to be merged. In shadow each chunk is checked as it's decoded and then
    // dropped, so a big file never holds a second JSON copy of the whole stream.
    #[derive(Default)]
    struct Acc {
        sids: HashSet<String>,
        seqs: HashSet<i64>,
        pending: Vec<Json>,
    }
    let mut accs: BTreeMap<String, Acc> = BTreeMap::new();

    // ── decode + check every chunk of every packed stream ──
    if let Some(p) = &packed {
        for pair in p.pairs::<Value, Value>() {
            let (k, v) = pair.map_err(|e| format!("packed key: {e}"))?;
            let name = lua_str(&k);
            let Value::Table(chunks) = v else {
                rep.problems.push(format!("packed.{name} is not a table"));
                continue;
            };
            let srep = rep.streams.entry(name.clone()).or_default();
            let acc = accs.entry(name.clone()).or_default();
            let stream_entries: &[Json] = streams.get(&name).map(|s| s.entries.as_slice()).unwrap_or(&[]);
            // seq → index into the stream table
            let by_seq: HashMap<i64, usize> =
                stream_entries.iter().enumerate().filter_map(|(i, e)| entry_seq(e).map(|q| (q, i))).collect();

            for pair in chunks.pairs::<Value, Value>() {
                let (k, v) = pair.map_err(|e| format!("packed chunk: {e}"))?;
                let key = lua_str(&k);
                let Some(entries) = decode_chunk(&key, &v, srep) else { continue };
                srep.chunks_decoded += 1;
                srep.entries_decoded += entries.len() as u64;
                for e in &entries {
                    let Some(q) = entry_seq(e) else { continue }; // already an entry problem
                    if !acc.seqs.insert(q) {
                        srep.duplicate_seqs.push(q); // in two chunks
                        continue;
                    }
                    match by_seq.get(&q) {
                        Some(_) if primary => srep.duplicate_seqs.push(q), // packed AND in the stream: §4.2 violation
                        Some(&i) => {
                            if let Some(path) = first_diff(&stream_entries[i], e, "$") {
                                srep.mismatched.push((q, path));
                            }
                        }
                        None if !primary => srep.missing_in_stream.push(q),
                        None => {}
                    }
                }
                acc.sids.insert(key);
                if primary {
                    acc.pending.extend(entries);
                }
            }
        }
    }

    // ── per stream: coverage, then merge (primary) ──
    let names: Vec<String> = rep.streams.keys().cloned().collect();
    for name in names {
        let acc = accs.remove(&name).unwrap_or_default();
        let srep = rep.streams.get_mut(&name).expect("stream report exists");
        let stream_entries: &[Json] = streams.get(&name).map(|s| s.entries.as_slice()).unwrap_or(&[]);
        srep.stream_entries = stream_entries.len() as u64;

        // stream entries that should have been in a chunk
        let mut not_packed: BTreeMap<String, u64> = BTreeMap::new();
        for e in stream_entries {
            let Some(sid) = e.get("sid").and_then(|v| v.as_str()) else { continue }; // never packed
            let flag = if primary {
                acc.sids.contains(sid) // a packed session's entries live ONLY in its chunk
            } else if acc.sids.contains(sid) {
                !entry_seq(e).map(|q| acc.seqs.contains(&q)).unwrap_or(false)
            } else {
                Some(sid) != open_sid && !rep.skipped.contains_key(sid)
            };
            if flag {
                *not_packed.entry(sid.to_string()).or_insert(0) += 1;
            }
        }
        srep.stream_not_packed = not_packed.into_iter().collect();
        srep.duplicate_seqs.sort_unstable();
        srep.missing_in_stream.sort_unstable();
        srep.mismatched.sort_by_key(|(q, _)| *q);

        if primary {
            let schema_default = schema_defaults.get(&name).copied().unwrap_or(1);
            let ps = streams.entry(name.clone()).or_insert(ParsedStream { schema: schema_default, entries: Vec::new() });
            ps.entries.extend(acc.pending);
            // stable: an entry with no seq (quarantined by the uploader anyway) sorts first
            ps.entries.sort_by_key(entry_seq);
            srep.merged_entries = ps.entries.len() as u64;
        } else {
            srep.merged_entries = srep.stream_entries;
        }
    }

    if primary && !rep.is_clean() {
        let mut msg = format!("refusing to upload: {}", rep.summary());
        for l in rep.lines().iter().skip(1).take(12) {
            msg.push('\n');
            msg.push_str(l);
        }
        return Err(msg);
    }
    Ok(Some(rep))
}

/// Decode + validate one chunk (§2.2). `None` (with the reason recorded) when it
/// can't be used. Header mismatches (`n`, `seq`, sid key) are recorded but the
/// entries are still returned, so the shadow comparison still runs over them; a
/// primary-mode file with any of those is refused as a whole anyway.
fn decode_chunk(key: &str, v: &Value, srep: &mut PackStreamReport) -> Option<Vec<Json>> {
    let Value::String(s) = v else {
        srep.undecodable.push((key.to_string(), format!("value is a {}, not a string", v.type_name())));
        return None;
    };
    // §3.4: lossy UTF-8 over the WHOLE chunk, the same U+FFFD the unpacked path gets
    let text = s.to_string_lossy().to_string();
    let parsed: Json = match serde_json::from_str(&text) {
        Ok(j) => j,
        Err(e) => {
            srep.undecodable.push((key.to_string(), format!("json: {e}")));
            return None;
        }
    };
    let Json::Object(mut chunk) = lua_numbers(parsed) else {
        srep.undecodable.push((key.to_string(), "chunk is not a JSON object".into()));
        return None;
    };

    match chunk.get("v").and_then(|v| v.as_i64()) {
        Some(PACK_VERSION) => {}
        _ => {
            let shown = chunk.get("v").map(|v| v.to_string()).unwrap_or_else(|| "(none)".into());
            srep.unknown_version.push((key.to_string(), shown));
            return None;
        }
    }
    let entries = match chunk.remove("e") {
        Some(Json::Array(a)) => a,
        _ => {
            srep.undecodable.push((key.to_string(), "`e` is missing or not an array".into()));
            return None;
        }
    };
    let sid = chunk.get("sid").and_then(|v| v.as_str()).unwrap_or("");
    if sid != key {
        srep.sid_key_mismatch.push((key.to_string(), sid.to_string()));
    }
    match chunk.get("n").and_then(|v| v.as_u64()) {
        Some(n) if n as usize == entries.len() => {}
        other => srep.n_mismatch.push((key.to_string(), format!("n={other:?}, e has {}", entries.len()))),
    }
    let first = entries.first().and_then(entry_seq);
    let last = entries.last().and_then(entry_seq);
    let declared: Option<(i64, i64)> = chunk.get("seq").and_then(|v| v.as_array()).and_then(|a| match a.as_slice() {
        [x, y] => Some((x.as_i64()?, y.as_i64()?)),
        _ => None,
    });
    if declared.is_none() || declared != first.zip(last) {
        srep.seq_mismatch.push((
            key.to_string(),
            format!("declared {:?}, entries run {:?}..{:?}", chunk.get("seq"), first, last),
        ));
    }

    let mut prev: Option<i64> = None;
    for (i, e) in entries.iter().enumerate() {
        match entry_seq(e) {
            None => srep.entry_problems.push(format!("{key} e[{i}]: no integer seq")),
            Some(q) => {
                if let Some(p) = prev {
                    if q <= p {
                        srep.entry_problems.push(format!("{key} e[{i}]: seq {q} not after {p}"));
                    }
                }
                prev = Some(q);
            }
        }
        if e.get("sid").and_then(|v| v.as_str()) != Some(key) {
            srep.entry_problems.push(format!("{key} e[{i}]: sid {:?} is not the chunk's", e.get("sid")));
        }
    }
    Some(entries)
}

/// A Lua number as the unpacked path sees it: mlua's exact lua51 rule (see
/// `mlua::state::raw::stack_value`), so `Integer` iff `num_traits::cast` to i64
/// succeeds and lands within `f64::EPSILON`; else a float; non-finite → `null`
/// (`lua_to_json`).
pub fn lua_number_json(n: f64) -> Json {
    match num_traits::cast::<f64, i64>(n) {
        Some(i) if (n - (i as f64)).abs() < f64::EPSILON => Json::Number(i.into()),
        _ => Number::from_f64(n).map(Json::Number).unwrap_or(Json::Null),
    }
}

/// Re-express every number in a decoded JSON value through `lua_number_json`.
/// Every number in a chunk is the text of a Lua double (§3.3), so `as_f64` is exact
/// for anything the encoder writes.
fn lua_numbers(v: Json) -> Json {
    match v {
        Json::Number(n) => n.as_f64().map(lua_number_json).unwrap_or(Json::Number(n)),
        Json::Array(a) => Json::Array(a.into_iter().map(lua_numbers).collect()),
        Json::Object(m) => Json::Object(m.into_iter().map(|(k, v)| (k, lua_numbers(v))).collect()),
        other => other,
    }
}

fn entry_seq(e: &Json) -> Option<i64> {
    e.get("seq").and_then(|v| v.as_i64())
}

fn lua_str(v: &Value) -> String {
    match lua_to_json(v.clone(), 0) {
        Ok(Json::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

/// The first place two entries differ under §5 (objects unordered, numbers by
/// value, strings byte-for-byte), as `path: detail`; `None` when equal. `a` is the
/// stream entry, `b` the packed one.
pub fn first_diff(a: &Json, b: &Json, path: &str) -> Option<String> {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => {
            if x.as_f64() == y.as_f64() {
                None
            } else {
                Some(format!("{path}: stream {x} vs packed {y}"))
            }
        }
        (Json::Object(x), Json::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let p = format!("{path}.{k}");
                match (x.get(k), y.get(k)) {
                    (Some(_), None) => return Some(format!("{p}: missing in packed")),
                    (None, Some(_)) => return Some(format!("{p}: missing in stream")),
                    (Some(xv), Some(yv)) => {
                        if let Some(d) = first_diff(xv, yv, &p) {
                            return Some(d);
                        }
                    }
                    (None, None) => {}
                }
            }
            None
        }
        (Json::Array(x), Json::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: length {} in stream vs {} packed", x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (xv, yv))| first_diff(xv, yv, &format!("{path}[{i}]")))
        }
        _ if a == b => None,
        _ => Some(format!("{path}: stream {} vs packed {}", short(a), short(b))),
    }
}

fn short(v: &Json) -> String {
    let s = v.to_string();
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(80).collect::<String>())
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svparse::{parse_export_str, SvExport};
    use mlua::{Lua, LuaOptions, StdLib};
    use serde_json::json;

    // In-crate copies of the shared `testdata/packed/` fixtures (repo root), so the
    // public `../GEC/uplink` mirror still compiles + tests. `fixtures_match_repo_root`
    // catches drift inside the monorepo.
    const UNPACKED: &[u8] = include_bytes!("../testdata/packed/unpacked.lua");
    const SHADOW: &[u8] = include_bytes!("../testdata/packed/shadow.lua");
    const PRIMARY: &[u8] = include_bytes!("../testdata/packed/primary.lua");
    const EXPECTED: &str = include_str!("../testdata/packed/expected-events.jsonl");

    fn expected() -> Vec<Json> {
        EXPECTED.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn events(x: &SvExport) -> &[Json] {
        &x.streams.get("events").unwrap().entries
    }

    /// Replace exactly one occurrence of `from` in a fixture.
    fn mutate(src: &[u8], from: &str, to: &str) -> Vec<u8> {
        let s = std::str::from_utf8(src).unwrap();
        assert_eq!(s.matches(from).count(), 1, "mutation anchor {from:?} must be unique");
        s.replacen(from, to, 1).into_bytes()
    }

    // ── task 1: pin the number representation the unpacked path produces ──
    // mlua (lua51) returns an integral Lua number as Value::Integer, so `seq`, `t` and
    // `amount` arrive as serde_json integers, never floats. An integral float literal
    // (`3.0`) is indistinguishable in Lua 5.1 and arrives as an integer too.
    #[test]
    fn unpacked_numbers_pin_integer_representation() {
        let src = br#"HaulData = { streams = { events = {
            { seq = 5, t = 1794267000, amount = -135, x = 75.8, whole = 3.0 },
        } } }"#;
        let x = parse_export_str(src, "HaulData").unwrap();
        let e = &events(&x)[0];
        let num = |k: &str| match &e[k] {
            Json::Number(n) => n.clone(),
            other => panic!("{k} is {other:?}"),
        };
        assert!(num("seq").is_u64() && !num("seq").is_f64());
        assert_eq!(num("seq").as_u64(), Some(5));
        assert!(num("t").is_u64() && !num("t").is_f64());
        assert!(num("amount").is_i64() && !num("amount").is_u64() && !num("amount").is_f64());
        assert_eq!(num("amount").as_i64(), Some(-135));
        assert!(num("x").is_f64());
        assert!(!num("whole").is_f64(), "3.0 is integral → Integer under lua51");
        // byte-level: what the upload serializes
        assert_eq!(e.to_string(), r#"{"amount":-135,"seq":5,"t":1794267000,"whole":3,"x":75.8}"#);
    }

    // `lua_number_json` must give exactly what mlua's own conversion gives, edge
    // cases included (tiny floats collapse to 0, ±2^63, ≥2^53, non-finite).
    #[test]
    fn lua_number_json_matches_mlua_for_edge_values() {
        let lua = Lua::new_with(StdLib::NONE, LuaOptions::default()).unwrap();
        let t: Table = lua
            .load(
                "return { 0, -0.0, 1e-17, -1e-17, 2.2e-16, 0.5, -0.5, 0.1, 1, -1, 1794267000, -135, \
                 477.6665954589844, 1e15 + 0.5, 9007199254740992, 9007199254740994, 1e17, 1e20, \
                 9223372036854775807, -9223372036854775808, 1e300, -1e300, 5e-324, \
                 1/0, -1/0, 0/0 }",
            )
            .eval()
            .unwrap();
        let n = t.raw_len();
        assert_eq!(n, 26);
        for i in 1..=n {
            let v: Value = t.get(i).unwrap();
            let f: f64 = t.get(i).unwrap();
            let want = lua_to_json(v, 0).unwrap();
            assert_eq!(lua_number_json(f), want, "index {i} ({f:e})");
            // and the representation, not just the value
            assert_eq!(lua_number_json(f).to_string(), want.to_string(), "index {i}");
        }
    }

    // float_roundtrip: the shortest-round-trip text the encoder writes parses back to
    // the identical double.
    #[test]
    fn floats_parse_exactly() {
        for f in [477.6665954589844_f64, 0.1, -0.5, 75.8, 2.0f64.sqrt(), 1.0 / 3.0, 5e-324, 1.7976931348623157e308] {
            let j: Json = serde_json::from_str(&format!("{f:?}")).unwrap();
            assert_eq!(j.as_f64().unwrap().to_bits(), f.to_bits(), "{f:?}");
        }
    }

    // ── the three fixtures yield exactly expected-events.jsonl, clean report ──
    #[test]
    fn all_three_fixtures_yield_the_expected_13_entries() {
        let want = expected();
        assert_eq!(want.len(), 13);
        for (name, src) in [("unpacked", UNPACKED), ("shadow", SHADOW), ("primary", PRIMARY)] {
            let x = parse_export_str(src, "HaulData").unwrap();
            let got = events(&x);
            assert_eq!(got.len(), 13, "{name}");
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                assert_eq!(first_diff(w, g, "$"), None, "{name} #{i} (§5)");
                assert_eq!(g, w, "{name} #{i}: identical representation, not just §5-equal");
                assert_eq!(g.to_string(), w.to_string(), "{name} #{i}: byte-identical upload");
            }
            match name {
                "unpacked" => assert!(x.pack.is_none()),
                _ => {
                    let r = x.pack.as_ref().unwrap();
                    assert!(r.is_clean(), "{name}: {:#?}", r.lines());
                    assert_eq!(r.mode.as_deref(), Some(name));
                    let ev = &r.streams["events"];
                    assert_eq!((ev.chunks_decoded, ev.entries_decoded), (3, 10));
                    assert_eq!(r.skipped.keys().collect::<Vec<_>>(), vec!["6b000003-dddd"]);
                }
            }
        }
    }

    // In shadow mode the upload set is the stream, untouched: same as reading the
    // file with the packed copy stripped.
    #[test]
    fn shadow_upload_set_is_the_stream() {
        let x = parse_export_str(SHADOW, "HaulData").unwrap();
        let r = &x.pack.as_ref().unwrap().streams["events"];
        assert_eq!(r.stream_entries, 13);
        assert_eq!(r.merged_entries, 13);
        // the packed copy decodes to the same values as the stream's twins
        assert_eq!(events(&x), expected().as_slice());
    }

    // ── mutation: one changed value in a shadow chunk is caught, with its path ──
    #[test]
    fn shadow_mismatch_is_reported_with_path() {
        let src = mutate(SHADOW, r#"\"amount\":-135"#, r#"\"amount\":-136"#);
        let x = parse_export_str(&src, "HaulData").unwrap();
        let r = x.pack.as_ref().unwrap();
        assert!(!r.is_clean());
        let ev = &r.streams["events"];
        assert_eq!(ev.mismatched.len(), 1, "{:?}", ev.mismatched);
        assert_eq!(ev.mismatched[0].0, 3);
        assert!(ev.mismatched[0].1.starts_with("$.amount:"), "{}", ev.mismatched[0].1);
        // the upload still comes from the stream, unchanged
        assert_eq!(events(&x)[2]["amount"], json!(-135));
    }

    #[test]
    fn shadow_nested_and_string_mismatches_are_caught() {
        let src = mutate(SHADOW, r#"\"npcID\":4024"#, r#"\"npcID\":4025"#);
        let r = parse_export_str(&src, "HaulData").unwrap().pack.unwrap();
        assert_eq!(r.streams["events"].mismatched[0].0, 2);
        assert!(r.streams["events"].mismatched[0].1.starts_with("$.src.npcID:"));

        let src = mutate(SHADOW, r#"\"Bloodfury Slayer\""#, r#"\"Bloodfury Slayers\""#);
        let r = parse_export_str(&src, "HaulData").unwrap().pack.unwrap();
        assert!(r.streams["events"].mismatched[0].1.starts_with("$.src.name:"));
    }

    #[test]
    fn header_problems_are_reported() {
        // n wrong
        let src = mutate(SHADOW, r#"\"n\":4,\"seq\":[1,4]"#, r#"\"n\":5,\"seq\":[1,4]"#);
        let r = parse_export_str(&src, "HaulData").unwrap().pack.unwrap();
        assert_eq!(r.streams["events"].n_mismatch.len(), 1);
        assert!(!r.is_clean());
        // seq range wrong
        let src = mutate(SHADOW, r#"\"seq\":[1,4]"#, r#"\"seq\":[1,5]"#);
        let r = parse_export_str(&src, "HaulData").unwrap().pack.unwrap();
        assert_eq!(r.streams["events"].seq_mismatch.len(), 1);
        // sid field ≠ map key
        let src = mutate(SHADOW, r#"{\"v\":1,\"sid\":\"6af25781-ea47\""#, r#"{\"v\":1,\"sid\":\"6af25781-XXXX\""#);
        let r = parse_export_str(&src, "HaulData").unwrap().pack.unwrap();
        assert_eq!(r.streams["events"].sid_key_mismatch, vec![("6af25781-ea47".into(), "6af25781-XXXX".into())]);
        // envelope totals off
        let src = mutate(SHADOW, r#"["entries"] = 10,"#, r#"["entries"] = 11,"#);
        let r = parse_export_str(&src, "HaulData").unwrap().pack.unwrap();
        assert!(!r.is_clean());
    }

    #[test]
    fn unknown_chunk_version_is_skipped_not_guessed() {
        let src = mutate(SHADOW, r#"{\"v\":1,\"sid\":\"6b000002-bbbb\""#, r#"{\"v\":2,\"sid\":\"6b000002-bbbb\""#);
        let x = parse_export_str(&src, "HaulData").unwrap(); // shadow: report, still upload the stream
        let r = x.pack.as_ref().unwrap();
        let ev = &r.streams["events"];
        assert_eq!(ev.unknown_version, vec![("6b000002-bbbb".into(), "2".into())]);
        assert_eq!(ev.chunks_decoded, 2);
        // its stream entries are now "not in any chunk", and totals don't match
        assert_eq!(ev.stream_not_packed, vec![("6b000002-bbbb".into(), 2)]);
        assert!(!r.is_clean());
        assert_eq!(events(&x).len(), 13);

        // primary: the same chunk would leave a hole → the file is refused
        let src = mutate(PRIMARY, r#"{\"v\":1,\"sid\":\"6b000002-bbbb\""#, r#"{\"v\":2,\"sid\":\"6b000002-bbbb\""#);
        let err = parse_export_str(&src, "HaulData").unwrap_err();
        assert!(err.contains("refusing to upload"), "{err}");
    }

    #[test]
    fn primary_seq_in_both_places_refuses_the_file() {
        // a full stream labelled primary: every packed seq is ALSO in the stream
        let src = mutate(SHADOW, r#"["mode"] = "shadow","#, r#"["mode"] = "primary","#);
        let err = parse_export_str(&src, "HaulData").unwrap_err();
        assert!(err.contains("refusing to upload") && err.contains("duplicate seqs"), "{err}");
    }

    #[test]
    fn unknown_or_missing_mode_refuses_the_file() {
        let src = mutate(SHADOW, r#"["mode"] = "shadow","#, r#"["mode"] = "sideways","#);
        let err = parse_export_str(&src, "HaulData").unwrap_err();
        assert!(err.contains("unknown mode"), "{err}");
        // an envelope with no mode at all: same, never guessed
        let src = mutate(SHADOW, r#"["mode"] = "shadow","#, "");
        let err = parse_export_str(&src, "HaulData").unwrap_err();
        assert!(err.contains("unknown mode"), "{err}");
    }

    // envelope `error` (packing failed, `packed` absent) in shadow: the stream still
    // uploads; the report is just not clean.
    #[test]
    fn envelope_error_is_reported_not_fatal_in_shadow() {
        let src = mutate(SHADOW, r#"["mode"] = "shadow","#, r#"["mode"] = "shadow", ["error"] = "boom","#);
        let x = parse_export_str(&src, "HaulData").unwrap();
        let r = x.pack.as_ref().unwrap();
        assert_eq!(r.error.as_deref(), Some("boom"));
        assert!(!r.is_clean());
        assert_eq!(events(&x).len(), 13);
    }

    // the exact shape GECStore writes when packing throws: no `packed`, no encoding
    #[test]
    fn failed_pack_envelope_shape() {
        let src = br#"HaulData = { _format = { packed = { version = 1, mode = "shadow", error = "x" } },
            streams = { events = { { seq = 1, sid = "a" } } } }"#;
        let x = parse_export_str(src, "HaulData").unwrap();
        let r = x.pack.unwrap();
        assert_eq!(r.error.as_deref(), Some("x"));
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(x.streams["events"].entries.len(), 1);
    }

    // Lossy UTF-8 (§3.4): a bad byte inside a packed string and the same bad byte in
    // the stream both become one U+FFFD, and compare equal.
    #[test]
    fn invalid_utf8_is_lossy_on_both_paths() {
        let src: &[u8] = b"HaulData = {
  _open = { sid = \"open\" },
  _format = { packed = { version = 1, encoding = \"json\", mode = \"shadow\",
              streams = { events = { sessions = 1, entries = 1 } } } },
  streams = { events = { { seq = 1, sid = \"s1\", name = \"bad\xE2\x80 byte\" } } },
  packed = { events = { s1 = \"{\\\"v\\\":1,\\\"sid\\\":\\\"s1\\\",\\\"n\\\":1,\\\"seq\\\":[1,1],\\\"e\\\":[{\\\"seq\\\":1,\\\"sid\\\":\\\"s1\\\",\\\"name\\\":\\\"bad\xE2\x80 byte\\\"}]}\" } },
}";
        let x = parse_export_str(src, "HaulData").unwrap();
        let r = x.pack.as_ref().unwrap();
        assert!(r.is_clean(), "{:#?}", r.lines());
        assert_eq!(events(&x)[0]["name"], json!("bad\u{FFFD} byte"));
    }

    // Dev, on real data: a primary-mode file must merge back to EXACTLY the stream of
    // its shadow twin (same entries, same order, byte-identical JSON). Point it at a
    // pair made from one SV file:
    //   PACKED_SHADOW=…/x-shadow.lua PACKED_PRIMARY=…/x-primary.lua \
    //     cargo test --release real_primary_merge_equals_shadow_stream -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_primary_merge_equals_shadow_stream() {
        let read = |k: &str| std::fs::read(std::env::var(k).unwrap_or_else(|_| panic!("set {k}"))).unwrap();
        let sh = parse_export_str(&read("PACKED_SHADOW"), "HaulData").unwrap();
        let pr = parse_export_str(&read("PACKED_PRIMARY"), "HaulData").unwrap();
        assert!(sh.pack.as_ref().unwrap().is_clean() && pr.pack.as_ref().unwrap().is_clean());
        let (a, b) = (events(&sh), events(&pr));
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            let (xs, ys) = (x.to_string(), y.to_string());
            assert!(x == y && xs == ys, "#{i}: {:?}", first_diff(x, y, "$"));
        }
        println!("{} entries identical", a.len());
    }

    #[test]
    fn fixtures_match_repo_root() {
        // only meaningful inside the monorepo; the public mirror has no ../../testdata
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/packed");
        if !root.exists() {
            return;
        }
        for (name, bytes) in [
            ("unpacked.lua", UNPACKED),
            ("shadow.lua", SHADOW),
            ("primary.lua", PRIMARY),
            ("expected-events.jsonl", EXPECTED.as_bytes()),
        ] {
            let disk = std::fs::read(root.join(name)).unwrap();
            assert!(disk == bytes, "uplink/src-tauri/testdata/packed/{name} drifted from testdata/packed/{name}: re-copy it");
        }
    }
}
