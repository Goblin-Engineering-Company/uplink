//! The data-sync engine (spec §6, phase 2) — reworked to the Uplink data contract
//! (`docs/…/2026-07-13-uplink-data-contract.md`). For every ONLINE install it
//! sweeps every WoW account folder's `SavedVariables/<Global>.lua`, parses it in
//! the sandboxed Lua (svparse) preserving EVERY field verbatim, and uploads to
//! `/api/ingest` (gzip). Nothing is flattened: entries ride through with all their
//! fields, the whole `_registry` rides along, and the learned-item catalog goes as
//! its own dataset.
//!
//! Envelope shape (`gec.ingest/2`, session-model wire §10): ONE POST per account
//! folder (an envelope carries one registry, so accounts never mix), carrying every
//! append-only stream in the file (`events`/`markers`/`overrides`) PLUS the upsert
//! snapshots (`sessions`, `learned_items`). Each append-only stream is sliced against
//! its own per install·account·addon·stream cursor and the cursor advances ONLY from
//! the server's per-stream ack; upsert snapshots ride once and carry no cursor.
//!
//! Identity / cursor (§10.2/§10.3 + seq-identity design 2026-07-24): every APPEND-ONLY
//! record carries its own per-stream `seq` — the addon's durable, monotonic order +
//! identity + dedup key, stamped at `Append` and stored in `_streamMeta` (a high-water
//! mark that survives compaction). Uplink slices `seq > cursor` (records ride verbatim;
//! no transport `i` is added — the server dedups on `record.seq`) and the server echoes
//! `cursor = max seq` accepted. Deletion propagates as a single per-stream watermark
//! `_streamMeta.base`: everything with `seq < base` was intentionally deleted; Uplink
//! floors its cursor at `base − 1` so a purge that outran sync can't stall the stream.
//! A hole (lost seq) or a record missing `seq` is skipped, never a stall (quarantine).
//! Upsert streams (`sessions`, `learned_*`, `places`) carry no `seq`/`base` — they're
//! keyed by their own identity (sid / item id / cascade). (`gen` can't serve as
//! identity: it's a per-login session counter shared by every entry that session.)
//!
//! Completeness (§4): a `sessions` record ships iff it has `closedAt`; the single
//! open/live session is skipped until it closes. Flavor (§7d.6): the envelope's
//! directory-derived `flavor` is VALIDATION ONLY — the record's `gameEnv.flavor` is
//! truth; on mismatch we FLAG (warn), never override. Freeze (§7b): a 503
//! `{frozen,message}` surfaces the message verbatim and advances no cursor.

use crate::config::AppState;
use crate::svparse::{self, StreamMeta, SvExport};
use crate::{delivery, net};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};
use std::path::{Path, PathBuf};

/// An addon whose SavedVariables export we know how to sweep. `addon` is the
/// envelope/telemetry name + on-disk folder root; `global` is the Lua global the
/// SV file assigns. Add a row to support a new addon — no other code changes; every
/// `streams.<name>` the file carries is swept generically (the server audits any it
/// doesn't yet model and echoes `unknown`).
pub struct AddonExport {
    pub slug: &'static str,
    pub addon: &'static str,
    pub global: &'static str,
}

pub const EXPORTS: &[AddonExport] = &[
    AddonExport { slug: "sbf", addon: "SBF", global: "SBFData" },
    // Haul stores nothing in a GECStore export yet — its file parses to no streams,
    // so it uploads nothing until it adopts the standard store. No addon-specific
    // code is needed when it does (contract §6).
    AddonExport { slug: "haul", addon: "Haul", global: "HaulData" },
];

/// The learned-item catalog rides as this dataset name (contract §5a).
const LEARNED_ITEMS: &str = "learned_items";
/// The frozen per-session records ride as this dataset name (wire §10.3).
const SESSIONS: &str = "sessions";

/// Upsert streams are keyed by their OWN identity (sid / item·spell id / cascade key),
/// so they carry NO transport `i` and drive no cursor (§10.2/§10.3). Everything else
/// found under `streams.<name>` (`events`/`markers`/`overrides`) is append-only. This
/// guards the generic sweep in case a future upsert stream is ever emitted under
/// `streams.` rather than as its own top-level table.
const UPSERT_STREAMS: &[&str] = &[SESSIONS, LEARNED_ITEMS, "learned_spells", "places"];

fn is_append_only(stream: &str) -> bool {
    !UPSERT_STREAMS.contains(&stream)
}

/// One stream's outcome from a sync pass, surfaced to the UI/logs.
#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    pub install_id: String,
    pub install_label: String,
    pub account: String,
    pub addon: String, // slug
    pub stream: String,
    pub found: u64,    // entries sliced (sent) this pass
    pub accepted: u64, // entries the server accepted (seen; may be dupes)
    pub inserted: u64, // NEW rows the server actually stored (dupes excluded)
    pub cursor: String,
    pub unknown: bool,           // server didn't recognize the stream
    pub skipped: Option<String>, // why we didn't parse/upload (e.g. "unchanged")
    pub error: Option<String>,
    /// A non-fatal data-quality note (wire §10.5 server warning, or a local
    /// flavor-mismatch flag §7d.6). Distinct from `error`: the data WAS accepted and
    /// the cursor advanced — this just tells the user why something is flagged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

// ── ingest envelope (session-model wire §10.2, gec.ingest/2) ──
#[derive(Serialize, Clone)]
struct AddonRef {
    name: String,
    version: String,
}
#[derive(Serialize)]
struct Dataset {
    /// Cursor the slice was taken after — `max seq` acked so far (append-only streams
    /// only); omitted for the snapshot upserts (`sessions`/`learned_items`).
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<String>,
    /// The stream's durable counter (`_streamMeta.<name> = {seq, base}`, seq-identity
    /// design), mirrored verbatim. `base` is the deletion watermark — the server deletes
    /// `seq < base` idempotently and lifts its cursor to `max(cursor, base − 1)`; `seq`
    /// is the high-water mark, letting the server run the seq-based gap/tail-loss check
    /// (highest present record < `seq` ⇒ tail loss). Append-only streams only; omitted
    /// for upsert snapshots.
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_meta: Option<StreamMeta>,
    entries: Vec<Json>,
}
#[derive(Serialize)]
struct Envelope {
    schema: &'static str,
    /// Producing addon (`Haul`·`SBF`·…) — envelope + telemetry name (§10.2).
    addon: String,
    account: String,
    /// Uplink's DIRECTORY-derived install token (`retail`·`classic`·…) — VALIDATION
    /// ONLY (§7d.6). `gameEnv.flavor` on each session record is the recorded truth;
    /// the server cross-checks this against it and FLAGS a mismatch, never reassigns.
    flavor: String,
    /// Pass-through of the SV `_format` export envelope verbatim (formatVersion,
    /// producer, producerBuild, registryVersion, schemaVersion, per-stream `{schema}`,
    /// generatedAt) — replaces the old hand-rolled `store` metadata (§10.2).
    #[serde(skip_serializing_if = "Map::is_empty")]
    format: Map<String, Json>,
    /// The ENTIRE `_registry` verbatim (char/place/item/spell/…), each domain as
    /// its `items` array — resolves `ch`/`p`/item indexes (§10.2).
    registry: Map<String, Json>,
    datasets: std::collections::HashMap<String, Dataset>,
    /// The installed-addon manifest (telemetry rider, §10.2).
    addons_installed: Vec<AddonRef>,
}
#[derive(Deserialize, Default)]
struct StreamAck {
    #[serde(default)]
    accepted: u64,
    #[serde(default)]
    inserted: u64,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    unknown: bool,
    // Present on an idempotent retry (server already applied this batch). We handle
    // it implicitly — accepted=0 + the stored cursor comes back, and cursor_ge keeps
    // us from regressing — so no branch reads it, but we parse it for completeness.
    #[serde(default)]
    #[allow(dead_code)]
    duplicate: bool,
}
/// A server-side data-quality warning (wire §10.5): non-fatal — the cursor still
/// advances; the server stored the data verbatim but flagged it (known-bad build,
/// flavor mismatch, incomplete/mutated session, schema-ahead, unknown stream). We
/// surface these to the user so they know WHY something is excluded from aggregates.
#[derive(Deserialize, Default, Clone)]
struct IngestWarning {
    #[serde(default)]
    code: String,
    #[serde(default)]
    addon: Option<String>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    #[allow(dead_code)]
    sids: Option<Vec<String>>,
    #[serde(default)]
    count: Option<u64>,
}
#[derive(Deserialize)]
struct IngestAck {
    #[serde(default)]
    streams: std::collections::HashMap<String, StreamAck>,
    /// Legacy-drain quarantine ack (spec v19 §10.1): while the drain is ON, an old
    /// `gec.ingest/1` batch is acked-and-archived — `200 {drained:true}` — but NEVER
    /// ingested, so the ack carries an EMPTY `streams` map. We advance the cursor (don't
    /// re-send quarantined data) yet surface it as a warning: it is NOT a normal sync.
    /// Without this field a drained body would look like an ack that simply omitted
    /// every stream — indistinguishable from a genuinely missing stream (data loss).
    #[serde(default)]
    drained: bool,
    /// The server-requested resync signal (contract §5b): an ISO-8601 UTC stamp (or
    /// null) the server sets when an admin purges the account or resets aggregates.
    /// Rides on EVERY device-authenticated response (this ack + /api/me). When it's
    /// newer than the last honored, we zero cursors + full-resync (see `sync_all`).
    #[serde(default)]
    resync_requested_at: Option<String>,
    /// Data-quality warnings for THIS submission (§10.5). New with `gec.ingest/2`.
    #[serde(default)]
    warnings: Vec<IngestWarning>,
}

/// A non-fatal note the ack-apply decision wants surfaced (routed through the same
/// `server_warnings` path as §10.5 warnings). Distinct from an `error`: the pass
/// succeeded; this just tells the user WHY a stream was quarantined or held.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum AckWarn {
    /// The whole batch was drained (legacy-drain quarantine): acked but NOT ingested.
    Drained,
    /// A well-formed `/2` ack omitted a stream it should have echoed — hold, don't advance.
    MissingStream,
}

/// The cursor outcome for ONE append-only stream from an ingest ack.
#[derive(Debug, PartialEq, Eq)]
struct AckDecision {
    /// `Some(c)` ⇒ advance the working cursor to `c`; `None` ⇒ HOLD (leave unchanged).
    cursor: Option<i64>,
    /// Server flagged the stream `unknown` → stop feeding it (also holds the cursor).
    unknown: bool,
    /// A note to surface, or `None` for a clean advance.
    warn: Option<AckWarn>,
}

/// Decide ONE stream's cursor move from an ingest ack — PURE, so the real apply loop
/// and the unit tests share exactly one decision path. The cases (spec §10.1/§10.2):
///
/// * `drained` — the batch was acked-and-quarantined (`200 {drained:true}`): ADVANCE
///   (don't re-send quarantined legacy data) but WARN, because it was never ingested.
///   Takes precedence over the stream map. (Server contract: a drained ack ECHOES every
///   posted key with per-stream `drained:true`; the top-level boolean is the stable signal
///   to key on — never infer quarantine from response shape. And per Q2 a `gec.ingest/2`
///   POST can never be drained, so this branch is purely defensive.)
/// * present + `unknown` — the server can't model the stream: HOLD, mark unknown.
/// * present — ADVANCE from the ack cursor, never regressing; fall back to the indices
///   we know we sent (`computed_cursor`) so the loop always makes progress.
/// * MISSING (and not drained) — a well-formed `/2` ack MUST echo every stream it
///   accepted, so a gap is unexpected: HOLD and WARN. Silent advance-on-missing may
///   happen ONLY under `drained` — never here (that would be real data loss).
fn decide_ack(
    drained: bool,
    ack: Option<&StreamAck>,
    computed_cursor: i64,
    current_cursor: i64,
) -> AckDecision {
    if drained {
        return AckDecision {
            cursor: Some(computed_cursor.max(current_cursor)),
            unknown: false,
            warn: Some(AckWarn::Drained),
        };
    }
    match ack {
        Some(a) if a.unknown => AckDecision { cursor: None, unknown: true, warn: None },
        Some(a) => {
            // advance from the ack cursor (never regress); fall back to the indices we
            // know we sent so the loop always makes progress
            let acked = a.cursor.as_ref().and_then(|s| s.parse::<i64>().ok());
            let next = match acked {
                Some(c) if c >= current_cursor => c,
                _ => computed_cursor.max(current_cursor),
            };
            AckDecision { cursor: Some(next), unknown: false, warn: None }
        }
        None => AckDecision { cursor: None, unknown: false, warn: Some(AckWarn::MissingStream) },
    }
}

/// A single install to sweep, snapshotted out of config so we never hold the lock
/// across a parse/await.
#[derive(Clone)]
struct InstallSnap {
    id: String,
    label: String,
    addons_path: String,
    sync_accounts: Vec<String>,
    /// Catalog slugs whose data may be swept for this install (redesign §3): an addon
    /// that is `enabled` AND has per-addon `sync` on. The sweep skips any EXPORTS row
    /// not in this set, so a sync-off (or not-installed) addon uploads nothing.
    sync_slugs: Vec<String>,
}

fn log_progress(msg: &str) {
    if std::env::var("UPLINK_SYNC_VERBOSE").is_ok() {
        eprintln!("[sync] {msg}");
    }
}

/// Derive `…/WTF/Account` from an install's `…/Interface/AddOns` path.
fn wtf_account_dir(addons_path: &str) -> Option<PathBuf> {
    let p = Path::new(addons_path);
    let flavor_root = p.parent()?.parent()?; // AddOns/.. = Interface/.. = _flavor_
    Some(flavor_root.join("WTF").join("Account"))
}

/// Account folder names under `WTF/Account` that hold a `SavedVariables` dir.
fn account_folders(wtf_account: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(wtf_account) {
        for e in entries.flatten() {
            if e.path().is_dir() && e.path().join("SavedVariables").is_dir() {
                out.push(e.file_name().to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    out
}

/// Discover the WoW account folder names for an install's AddOns path. Public so the
/// account-selection command reuses the exact discovery the sweep uses.
pub fn discover_accounts(addons_path: &str) -> Vec<String> {
    match wtf_account_dir(addons_path) {
        Some(wtf) => account_folders(&wtf),
        None => Vec::new(),
    }
}

/// The accounts an install will actually sync: exactly the `discovered` folders that
/// are in the user's `selected` set (redesign §2). Never syncs an account the user
/// didn't opt into, and an EXPLICIT empty selection means "none" and is respected —
/// there is no single-account auto-reselect (that fought the user's uncheck). The
/// fresh-install "all accounts on" default is seeded ONCE into `sync_accounts` at
/// discovery time (see `list_sync_accounts` + `Install.accounts_initialized`), not
/// re-derived here on every sweep.
pub fn effective_accounts(selected: &[String], discovered: &[String]) -> Vec<String> {
    discovered
        .iter()
        .filter(|d| selected.iter().any(|s| s == *d))
        .cloned()
        .collect()
}

fn mtime_secs(path: &Path) -> Option<i64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Best-effort "is a WoW client running?" for the sync-while-running gate.
pub fn wow_running() -> bool {
    #[cfg(target_os = "windows")]
    let (cmd, args, needles): (&str, &[&str], &[&str]) =
        ("tasklist", &[], &["wow.exe", "wowclassic.exe"]);
    #[cfg(not(target_os = "windows"))]
    let (cmd, args, needles): (&str, &[&str], &[&str]) =
        ("ps", &["-A", "-o", "comm="], &["world of warcraft", "wow.exe"]);

    match std::process::Command::new(cmd).args(args).output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
            needles.iter().any(|n| text.contains(n))
        }
        Err(_) => false,
    }
}

/// Run a full sync pass. `manual` = user pressed Sync-now (always runs, ignores the
/// while-running gate and the mtime short-circuit). One result per stream swept.
/// Never panics: a file that won't parse becomes a per-stream error and the pass
/// continues.
pub async fn sync_all(state: &AppState, manual: bool) -> Result<Vec<SyncResult>, String> {
    log_progress("checking device token…");
    if !crate::auth::has_token() {
        return Err("not paired — pair this device first".to_string());
    }
    log_progress("token OK; snapshotting installs…");

    let (base, sync_while_running, installs): (String, bool, Vec<InstallSnap>) = {
        let cfg = state.config.lock().unwrap();
        let installs = cfg
            .installs
            .iter()
            .filter(|i| Path::new(&i.path).is_dir())
            .map(|i| InstallSnap {
                id: i.id.clone(),
                label: format!("{} · {}", i.label, i.flavor),
                addons_path: i.path.clone(),
                sync_accounts: i.sync_accounts.clone(),
                // sync gate (redesign §3): only addons that are enabled AND have per-addon
                // sync on may upload. Snapshot the slug set so the sweep never holds the lock.
                sync_slugs: i
                    .addons
                    .iter()
                    .filter(|a| a.enabled && a.sync)
                    .map(|a| a.slug.clone())
                    .collect(),
            })
            .collect();
        (cfg.base_url.clone(), cfg.sync_while_running, installs)
    };

    if !manual && !sync_while_running && wow_running() {
        return Ok(vec![SyncResult {
            install_id: String::new(),
            install_label: String::new(),
            account: String::new(),
            addon: String::new(),
            stream: String::new(),
            found: 0,
            accepted: 0,
            inserted: 0,
            cursor: String::new(),
            unknown: false,
            skipped: Some("WoW is running (sync-while-running off)".to_string()),
            error: None,
            warning: None,
        }]);
    }

    log_progress(&format!("{} online install(s)", installs.len()));

    // ── resync signal (contract §5b), pre-check via /api/me ──
    // Poll the server's `resync_requested_at` BEFORE sweeping. This catches an admin
    // purge even when there's nothing new to POST (an idle install that would never
    // fire an ingest ack) — honoring here zeroes the cursors so the sweep below then
    // re-sends everything. Best-effort: a network error just means we rely on the
    // ack-based honor (below) or the next pass. Idempotent — guarded by the honored
    // stamp, so it never double-triggers.
    match poll_resync_signal(state, &base).await {
        Ok(true) => log_progress("resync signal honored via /api/me — cursors zeroed"),
        Ok(false) => {}
        Err(e) => log_progress(&format!("/api/me resync check skipped: {e}")),
    }

    // ── first full sweep ── collect the max resync stamp seen across all acks.
    let mut resync_stamp: Option<String> = None;
    let mut results = sweep_installs(state, &base, &installs, manual, &mut resync_stamp).await;

    // ── honor the ack-borne stamp (contract §5b) ──
    // If any ack in this pass carried a stamp newer than the last honored, zero the
    // cursors and run EXACTLY ONE more full sweep (the resync). Because honoring sets
    // `last_resync_honored = stamp`, that extra sweep's acks carry the same stamp and
    // won't re-trigger — bounded to one extra sweep per new stamp, no loop.
    let triggered = {
        let mut cfg = state.config.lock().unwrap();
        cfg.honor_resync_signal(resync_stamp.as_deref())
    };
    if triggered {
        state.persist()?;
        log_progress("resync signal honored via ingest ack — running one full resync sweep");
        // The extra sweep's stamp is discarded: honored already equals it, so it can't
        // trigger a third sweep even if we looked.
        let mut ignore_stamp: Option<String> = None;
        let mut more = sweep_installs(state, &base, &installs, manual, &mut ignore_stamp).await;
        results.append(&mut more);
    }

    state.persist()?;
    Ok(results)
}

/// Fetch `/api/me` and honor its `resync_requested_at` (contract §5b). Returns
/// `Ok(true)` when a resync was triggered (cursors zeroed + persisted), `Ok(false)`
/// when the stamp is absent/older/equal, `Err` on a network/parse failure. Reuses
/// `AppConfig::honor_resync_signal`, so it's idempotent with every other honor path.
async fn poll_resync_signal(state: &AppState, base: &str) -> Result<bool, String> {
    let me = crate::catalog::fetch_home(base).await?;
    let stamp = me.get("resync_requested_at").and_then(|v| v.as_str()).map(|s| s.to_string());
    let triggered = {
        let mut cfg = state.config.lock().unwrap();
        cfg.honor_resync_signal(stamp.as_deref())
    };
    if triggered {
        state.persist()?;
    }
    Ok(triggered)
}

/// Sweep every online install's opted-in accounts, POSTing each addon's streams +
/// learned_items. Accumulates the max `resync_requested_at` seen across all acks into
/// `resync_stamp` (contract §5b). Factored out of `sync_all` so the resync path can
/// re-run the exact same sweep after zeroing cursors.
async fn sweep_installs(
    state: &AppState,
    base: &str,
    installs: &[InstallSnap],
    manual: bool,
    resync_stamp: &mut Option<String>,
) -> Vec<SyncResult> {
    let mut results: Vec<SyncResult> = Vec::new();
    for inst in installs {
        let Some(wtf) = wtf_account_dir(&inst.addons_path) else {
            continue;
        };
        let discovered = account_folders(&wtf);
        let accounts = effective_accounts(&inst.sync_accounts, &discovered);
        log_progress(&format!("{}: discovered={:?} syncing={:?}", inst.label, discovered, accounts));
        if accounts.is_empty() {
            continue;
        }
        let manifest = build_manifest(&inst.addons_path);

        for account in &accounts {
            let sv_dir = wtf.join(account).join("SavedVariables");
            for exp in EXPORTS {
                // per-addon sync gate (redesign §3): skip an addon the user isn't syncing
                // (sync off) or that isn't installed/enabled on this install.
                if !inst.sync_slugs.iter().any(|s| s == exp.slug) {
                    continue;
                }
                let file = sv_dir.join(format!("{}.lua", exp.addon));
                if !file.is_file() {
                    continue;
                }
                let mut r = sync_one_account_addon(
                    state, base, inst, account, exp, &file, &manifest, manual, resync_stamp,
                )
                .await;
                results.append(&mut r);
            }
        }
    }
    results
}

/// Keep the lexically-greatest ISO-8601 stamp in `acc` (contract §5b). Same-format
/// UTC (`…Z`) stamps sort lexically, so a plain `>` is the newest-wins compare;
/// None/empty incoming is ignored.
fn merge_stamp(acc: &mut Option<String>, incoming: Option<&str>) {
    let Some(s) = incoming else { return };
    let s = s.trim();
    if s.is_empty() {
        return;
    }
    let replace = match acc {
        Some(cur) => s > cur.as_str(),
        None => true,
    };
    if replace {
        *acc = Some(s.to_string());
    }
}

/// Sweep + upload ONE addon file for one account: parse it, slice every stream by
/// its own cursor, and POST a SINGLE envelope (all streams + learned_items + the
/// whole registry). Returns one SyncResult per stream (plus learned_items). Isolated
/// so a failure is scoped to this file.
async fn sync_one_account_addon(
    state: &AppState,
    base: &str,
    inst: &InstallSnap,
    account: &str,
    exp: &AddonExport,
    file: &Path,
    manifest: &[AddonRef],
    manual: bool,
    resync_stamp: &mut Option<String>,
) -> Vec<SyncResult> {
    let mk = |stream: &str, found, accepted, inserted, cursor: String, unknown, skipped, error| SyncResult {
        install_id: inst.id.clone(),
        install_label: inst.label.clone(),
        account: account.to_string(),
        addon: exp.slug.to_string(),
        stream: stream.to_string(),
        found,
        accepted,
        inserted,
        cursor,
        unknown,
        skipped,
        error,
        warning: None,
    };

    let file_mtime = mtime_secs(file);

    // mtime short-circuit: an auto pass skips re-parsing an unchanged multi-MB file.
    // Manual always re-parses. (Any stream state recorded for this file carries the
    // last-seen mtime; they're all written together, so checking one suffices.)
    if !manual {
        let prev_mtime = {
            let cfg = state.config.lock().unwrap();
            cfg.sync
                .iter()
                .find(|s| s.install_id == inst.id && s.account == account && s.addon == exp.slug)
                .and_then(|s| s.file_mtime)
        };
        if prev_mtime.is_some() && prev_mtime == file_mtime {
            return vec![mk("", 0, 0, 0, String::new(), false, Some("unchanged".into()), None)];
        }
    }

    // Parse off the async executor (CPU/IO bound, sync Lua).
    log_progress(&format!("{} · {} · {}: parsing {:?}…", inst.label, account, exp.addon, file));
    let path = file.to_path_buf();
    let global = exp.global.to_string();
    let parsed = tokio::task::spawn_blocking(move || svparse::parse_export_file(&path, &global)).await;
    let export: SvExport = match parsed {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => return vec![mk("", 0, 0, 0, String::new(), false, None, Some(e))], // mid-write — retry next pass
        Err(e) => return vec![mk("", 0, 0, 0, String::new(), false, None, Some(format!("parse task: {e}")))],
    };

    // ── completeness gate (§4): ship only CLOSED session records ──
    // A record is complete iff it has `closedAt`; the single open/live session ships
    // only after it closes. `sessions` is a top-level record-upsert — no cursor, no
    // `i` (§10.3). We also count how many open records we withheld (for the log).
    let (closed_sessions, open_skipped): (Vec<Json>, u64) = {
        let mut closed = Vec::new();
        let mut skipped = 0u64;
        for s in &export.sessions {
            let has_close = s.get("closedAt").map(|v| !v.is_null()).unwrap_or(false);
            if has_close {
                closed.push(s.clone());
            } else {
                skipped += 1;
            }
        }
        (closed, skipped)
    };
    if open_skipped > 0 {
        log_progress(&format!(
            "{} · {} · {}: withholding {} open (unclosed) session record(s) — completeness gate §4",
            inst.label, account, exp.addon, open_skipped
        ));
    }

    // Nothing to send at all (no streams, no catalog, no closed sessions) → just
    // record mtime so the next auto pass short-circuits. Benign per-file "unchanged".
    if export.streams.is_empty() && export.db_items.is_empty() && closed_sessions.is_empty() {
        let mut cfg = state.config.lock().unwrap();
        let st = cfg.stream_state_mut(&inst.id, account, exp.slug, "fishlog");
        st.file_mtime = file_mtime;
        return vec![mk("", 0, 0, 0, String::new(), false, Some("no exportable data".into()), None)];
    }

    // ── flavor cross-check (§7d.6): the envelope's DIRECTORY-derived flavor is
    // validation-only; each record's `gameEnv.flavor` is truth. A mismatch almost
    // certainly means the file was attributed to the wrong install — FLAG it (warn),
    // never override. Unknown tokens pass unchecked (never an allowlist). ──
    let flavor = crate::installs::flavor_token_from_path(&inst.addons_path);
    let mut flavor_flags: Vec<String> = Vec::new();
    for s in &closed_sessions {
        let rec_flavor = s.get("gameEnv").and_then(|g| g.get("flavor")).and_then(|f| f.as_i64());
        if let (Some(rf), Some(matches)) = (rec_flavor, flavor_token_matches(&flavor, rec_flavor)) {
            if !matches {
                let sid = s.get("sid").and_then(|v| v.as_str()).unwrap_or("?");
                let msg = format!(
                    "session {sid}: directory flavor '{flavor}' ≠ recorded gameEnv.flavor {rf} — file may be attributed to the wrong install"
                );
                eprintln!("[sync][flavor-mismatch] {} · {} · {}: {}", inst.label, account, exp.addon, msg);
                flavor_flags.push(msg);
            }
        }
    }

    // ── per-stream state: id, entries, cursor, running totals ──
    // Only APPEND-ONLY streams (`events`/`markers`/`overrides`) become cursored plans;
    // upsert streams are handled as snapshots below (§10.2/§10.3).
    struct StreamPlan<'a> {
        name: String,
        entries: &'a [Json],
        meta: StreamMeta, // `_streamMeta{seq,base}` forwarded to the server verbatim
        cursor: i64,      // working seq cursor = max seq acked (advances per chunk)
        total: u64,      // total records present in the stream
        found: u64,      // records sent this pass (across chunks)
        accepted: u64,   // ack accepted (across chunks)
        inserted: u64,   // ack inserted (across chunks)
        unknown: bool,   // server doesn't model this stream
    }

    let mut plans: Vec<StreamPlan> = {
        let cfg = state.config.lock().unwrap();
        let mut names: Vec<&String> = export.streams.keys().filter(|n| is_append_only(n)).collect();
        names.sort();
        names
            .into_iter()
            .map(|name| {
                let ps = &export.streams[name];
                let stored = cfg.cursor_of(&inst.id, account, exp.slug, name).parse::<i64>().unwrap_or(0);
                // deletion watermark (seq-identity design): floor the cursor at `base − 1`
                // so a purge that outran sync can't leave this stream permanently stuck.
                // Default `{seq:0, base:1}` when a pre-migration file has no `_streamMeta`.
                let meta = export.stream_meta.get(name).copied().unwrap_or(StreamMeta { seq: 0, base: 1 });
                StreamPlan {
                    name: name.clone(),
                    entries: &ps.entries,
                    meta,
                    cursor: effective_cursor(stored, meta.base),
                    total: ps.entries.len() as u64,
                    found: 0,
                    accepted: 0,
                    inserted: 0,
                    unknown: false,
                }
            })
            .collect()
    };

    let learned_total = export.db_items.len() as u64;
    let mut learned_accepted = 0u64;
    let mut learned_inserted = 0u64;
    let mut learned_unknown = false;

    let sessions_total = closed_sessions.len() as u64;
    let mut sessions_accepted = 0u64;
    let mut sessions_inserted = 0u64;
    let mut sessions_unknown = false;

    let addons_installed: Vec<AddonRef> = manifest
        .iter()
        .map(|m| AddonRef { name: m.name.clone(), version: m.version.clone() })
        .collect();

    // accumulated server data-quality warnings across all chunks (§10.5)
    let mut server_warnings: Vec<IngestWarning> = Vec::new();

    // ── chunked upload (§10.1): each POST carries the FULL registry + `format` +
    // the next ascending slice (≤ CHUNK per append-only stream), using the previous
    // chunk's acked cursor as `since`. Never mixes account folders. Idempotent per
    // chunk. The upsert snapshots (`sessions`, `learned_items`) are sent ONCE, on the
    // first chunk, with no `since` and no per-entry `i` (§10.2/§10.3).
    let mut post_error: Option<String> = None;
    let mut chunk_no = 0u32;
    let mut learned_sent = false;
    let mut sessions_sent = false;
    loop {
        // build this chunk's datasets from each unexhausted append-only stream
        let mut datasets: std::collections::HashMap<String, Dataset> = std::collections::HashMap::new();
        let mut slices: Vec<(usize, i64, u64)> = Vec::new(); // (plan idx, new_cursor, sent)
        for (idx, p) in plans.iter().enumerate() {
            if p.unknown {
                continue; // stop feeding a stream the server can't apply
            }
            let (entries, new_cursor) = slice_chunk(p.entries, p.cursor, CHUNK);
            let sent = entries.len() as u64;
            if sent == 0 {
                continue;
            }
            datasets.insert(
                p.name.clone(),
                Dataset { since: Some(p.cursor.to_string()), stream_meta: Some(p.meta), entries },
            );
            slices.push((idx, new_cursor, sent));
        }
        // sessions snapshot (closed records only) rides the first chunk only — upsert,
        // no `since`, no `i` (keyed by `sid`, §10.3)
        let include_sessions = !sessions_sent && sessions_total > 0 && !sessions_unknown;
        if include_sessions {
            datasets.insert(
                SESSIONS.to_string(),
                Dataset { since: None, stream_meta: None, entries: closed_sessions.clone() },
            );
        }
        // learned_items snapshot rides the first chunk only
        let include_learned = !learned_sent && learned_total > 0 && !learned_unknown;
        if include_learned {
            datasets.insert(
                LEARNED_ITEMS.to_string(),
                Dataset { since: None, stream_meta: None, entries: export.db_items.clone() },
            );
        }

        if datasets.is_empty() {
            break; // everything sent
        }

        chunk_no += 1;
        let env = Envelope {
            schema: "gec.ingest/2",
            addon: exp.addon.to_string(),
            account: account.to_string(),
            flavor: flavor.clone(),
            format: export.format.clone(),
            registry: export.registry.clone(),
            datasets,
            addons_installed: addons_installed.clone(),
        };
        log_progress(&format!(
            "{} · {} · {}: POST chunk {} — {} stream slice(s){}{}",
            inst.label, account, exp.addon, chunk_no, slices.len(),
            if include_sessions { format!(" + {sessions_total} session record(s)") } else { String::new() },
            if include_learned { format!(" + {learned_total} learned items") } else { String::new() }
        ));

        let ack = match net::post_gzip_json::<_, IngestAck>(base, "/api/ingest", &env).await {
            Ok(a) => a,
            Err(e) => {
                post_error = Some(e); // stop; advance no cursor for the unsent remainder
                break;
            }
        };

        // record the server's resync signal (contract §5b) — newest wins across chunks
        merge_stamp(resync_stamp, ack.resync_requested_at.as_deref());
        // accumulate data-quality warnings (§10.5) across chunks
        server_warnings.extend(ack.warnings.iter().cloned());

        // apply the ack: advance each fed stream's working cursor + tallies via the
        // pure `decide_ack` decision (shared with the unit tests). Under a drained ack
        // every fed stream advances-with-warning; a missing stream in a well-formed ack
        // HOLDS + warns (never a silent advance). The drained note is per-batch, so we
        // surface it once per chunk rather than once per stream.
        let mut drained_warned = false;
        for (idx, computed_cursor, sent) in slices {
            let p = &mut plans[idx];
            p.found += sent;
            let a = ack.streams.get(&p.name);
            if let Some(sa) = a {
                p.accepted += sa.accepted;
                p.inserted += sa.inserted;
            }
            let decision = decide_ack(ack.drained, a, computed_cursor, p.cursor);
            if decision.unknown {
                p.unknown = true; // stop feeding a stream the server can't apply
            }
            if let Some(c) = decision.cursor {
                p.cursor = c;
            }
            match decision.warn {
                Some(AckWarn::Drained) => {
                    if !drained_warned {
                        drained_warned = true;
                        server_warnings.push(IngestWarning {
                            code: "drained".into(),
                            addon: Some(exp.slug.to_string()),
                            message: "batch quarantined by the legacy drain — acked but NOT ingested".into(),
                            sids: None,
                            count: None,
                        });
                    }
                }
                Some(AckWarn::MissingStream) => {
                    server_warnings.push(IngestWarning {
                        code: "missing_stream".into(),
                        addon: Some(exp.slug.to_string()),
                        message: format!(
                            "stream '{}' missing from a well-formed ack — cursor held (not advanced)",
                            p.name
                        ),
                        sids: None,
                        count: None,
                    });
                }
                None => {}
            }
        }
        if include_sessions {
            sessions_sent = true;
            if let Some(a) = ack.streams.get(SESSIONS) {
                sessions_accepted += a.accepted;
                sessions_inserted += a.inserted;
                sessions_unknown = a.unknown;
            }
        }
        if include_learned {
            learned_sent = true;
            if let Some(a) = ack.streams.get(LEARNED_ITEMS) {
                learned_accepted += a.accepted;
                learned_inserted += a.inserted;
                learned_unknown = a.unknown;
            }
        }
    }

    // ── persist per-stream cursors + counters, build results ──
    let mut out = Vec::new();
    {
        let mut cfg = state.config.lock().unwrap();
        for p in &plans {
            let st = cfg.stream_state_mut(&inst.id, account, exp.slug, &p.name);
            // capture the cursor BEFORE overwriting so we can tell if this pass advanced it
            let prev_cursor = st.cursor.parse::<i64>().unwrap_or(0);
            st.total = p.total;
            st.cursor = p.cursor.to_string();
            st.last_accepted = p.accepted;
            // queued = records still ahead of the persisted cursor by seq (unsent
            // remainder). Seq-based, not `total − cursor`: after a wipe/purge the cursor
            // is a high seq value while few records remain, so positional arithmetic
            // would underflow to a bogus queue.
            st.queued = count_after(p.entries, p.cursor);
            st.file_mtime = file_mtime;
            if post_error.is_none() && !p.unknown {
                st.last_sync = Some(now_secs());
            }
            // STUCK (redesign §6): we fed this stream but its cursor didn't move and data
            // still remains (server `unknown`, a hold, etc.) — a queue that won't drain.
            // A clean advance, or fully draining to queued==0, clears it. A network error
            // is a transient failure, not a stuck stream, so don't flag it then.
            let advanced = p.cursor > prev_cursor;
            st.stuck = post_error.is_none() && !advanced && st.queued > 0;
            out.push(mk(
                &p.name, p.found, p.accepted, p.inserted, p.cursor.to_string(),
                p.unknown, None, post_error.clone(),
            ));
        }
        // Orphaned streams (redesign §6): a StreamState for this addon·account whose
        // append-only stream is GONE from the SV (e.g. SBF's old `fishlog`, renamed to
        // `events`) never gets a plan, so its queue can never drain. If it still shows
        // queued > 0, mark it stuck so the Data page can offer "Clear queue".
        let present: std::collections::HashSet<&str> = plans.iter().map(|p| p.name.as_str()).collect();
        for st in cfg.sync.iter_mut() {
            if st.install_id == inst.id
                && st.account == account
                && st.addon == exp.slug
                && is_append_only(&st.stream)
                && !present.contains(st.stream.as_str())
                && st.queued > 0
            {
                st.stuck = true;
            }
        }
    }

    // sessions result (informational; upsert snapshot — no cursor). `found` = the
    // closed records we shipped; open records were withheld by the completeness gate.
    if sessions_total > 0 {
        let skipped = (open_skipped > 0).then(|| format!("withheld {open_skipped} open session(s)"));
        out.push(mk(
            SESSIONS, sessions_total, sessions_accepted, sessions_inserted,
            String::new(), sessions_unknown, skipped,
            if sessions_sent { None } else { post_error.clone() },
        ));
    } else if open_skipped > 0 {
        // only open records present → nothing shipped, but tell the user why
        out.push(mk(
            SESSIONS, 0, 0, 0, String::new(), false,
            Some(format!("withheld {open_skipped} open session(s) — waiting on close")), None,
        ));
    }

    // learned_items result (informational; no cursor)
    if learned_total > 0 {
        out.push(mk(
            LEARNED_ITEMS, learned_total, learned_accepted, learned_inserted,
            String::new(), learned_unknown, None,
            if learned_sent { None } else { post_error.clone() },
        ));
    }

    // ── surface data-quality notes as dedicated rows (§10.5 + local flavor flags) ──
    // These are NON-fatal: the data was accepted and cursors advanced; the row just
    // tells the user why something is flagged. TODO(frontend): a dedicated warnings
    // banner would read better than reusing the results table.
    for w in &server_warnings {
        let text = if let Some(n) = w.count {
            format!("{}: {} ({n})", w.code, w.message)
        } else {
            format!("{}: {}", w.code, w.message)
        };
        let mut row = mk("⚠ server", 0, 0, 0, String::new(), false, None, None);
        row.addon = w.addon.clone().unwrap_or_else(|| exp.slug.to_string());
        row.warning = Some(text);
        out.push(row);
    }
    for f in &flavor_flags {
        let mut row = mk("⚠ flavor", 0, 0, 0, String::new(), false, None, None);
        row.warning = Some(f.clone());
        out.push(row);
    }

    if out.is_empty() {
        out.push(mk("", 0, 0, 0, String::new(), false, Some("nothing new".into()), None));
    }
    out
}

/// Cross-check the envelope's DIRECTORY-derived flavor token against a session
/// record's `gameEnv.flavor` (`WOW_PROJECT_ID`), §7d.6. Returns `Some(true)` on a
/// consistent pair, `Some(false)` on a mismatch (→ flag), and `None` when the token
/// is UNKNOWN (pass unchecked — never an allowlist) or the record has no flavor int.
///
/// Directory tokens come from the `_flavor_` folder segment (`retail`, `classic`,
/// `classic_era`, `ptr`, …). `WOW_PROJECT_MAINLINE` = 1 is retail; every other
/// project id is a Classic line (era 2, TBC 5, Wrath 11, Cata 14, MoP 19, …). We only
/// need the retail-vs-classic partition to catch a cross-install misattribution; finer
/// Classic-line granularity is realm-level and invisible to the directory (§7d.6).
fn flavor_token_matches(token: &str, rec_flavor: Option<i64>) -> Option<bool> {
    let f = rec_flavor?;
    let is_retail_dir = token == "retail" || token == "mainline";
    let is_classic_dir = token.starts_with("classic") || token == "era" || token == "tbc" || token == "wrath";
    if is_retail_dir {
        Some(f == 1)
    } else if is_classic_dir {
        Some(f != 1)
    } else {
        None // ptr/beta/unknown → skip (validation is best-effort, never an allowlist)
    }
}

/// The per-chunk record budget for a big backlog (contract §2 rule 4: chunk the
/// ~25k first sync into ~2–5k-record POSTs, each carrying the full registry). Also
/// bounds any single body's size.
const CHUNK: usize = 4000;

/// A record's per-stream `seq` (seq-identity design): the durable, monotonic order +
/// identity + dedup key the addon stamps at `Append`. `None` for a record that carries
/// no `seq` — a malformed/legacy entry the slice quarantines (skips) rather than ships.
fn entry_seq(e: &Json) -> Option<i64> {
    e.get("seq").and_then(|v| v.as_i64())
}

/// The cursor a stream's slice starts from: the persisted cursor, floored at the
/// deletion watermark `base − 1` (seq-identity §"Deletion by watermark"). Lifting to
/// `base − 1` (NOT `base`) skips records a purge deleted before Uplink uploaded them —
/// accepted loss (spec §"Purge may outrun sync") — so a purge can never leave the
/// stream permanently stuck, while the lowest SURVIVING record (`seq == base`) still
/// reads as unsynced and uploads.
fn effective_cursor(stored: i64, base: i64) -> i64 {
    stored.max(base - 1)
}

/// Count of records still ahead of `cursor` by seq (the unsent remainder = "queued").
/// Records with no `seq` are quarantined, so they don't count toward the queue.
fn count_after(entries: &[Json], cursor: i64) -> u64 {
    entries.iter().filter(|e| matches!(entry_seq(e), Some(s) if s > cursor)).count() as u64
}

/// Slice ONE ascending chunk of a stream: up to `limit` records whose own `seq` is
/// `> after` (seq-identity design — `seq` replaces the old positional `i`). Records
/// ride VERBATIM (their `seq` already inside), so the server dedups on `record.seq`;
/// no transport annotation is added. Returns (chunk, highest-seq-in-chunk); the second
/// value == `after` when the chunk is empty (stream exhausted past the cursor).
///
/// Robust to compaction and loss: a purged front (low seqs absent) and an interior
/// HOLE (a lost seq) are simply not present, so the seq-cursor advances past them
/// without stalling. A record missing `seq` is quarantined (skipped). Relies on the
/// Append invariant that array order == ascending seq, so breaking at `limit` never
/// skips a lower-seq record.
fn slice_chunk(entries: &[Json], after: i64, limit: usize) -> (Vec<Json>, i64) {
    let mut out = Vec::new();
    let mut max_seq = after;
    for e in entries {
        let Some(seq) = entry_seq(e) else { continue }; // quarantine: no seq → set aside
        if seq <= after {
            continue;
        }
        out.push(e.clone());
        if seq > max_seq {
            max_seq = seq;
        }
        if out.len() >= limit {
            break;
        }
    }
    (out, max_seq)
}

/// The installed-addon manifest for one install.
fn build_manifest(addons_path: &str) -> Vec<AddonRef> {
    let mut out = Vec::new();
    for exp in EXPORTS {
        let folder = delivery::folder_guess(exp.slug);
        if let Some(v) = delivery::installed_version(addons_path, &folder) {
            out.push(AddonRef { name: exp.addon.to_string(), version: v });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wtf_dir_derives_from_addons_path() {
        let p = "/wow/_retail_/Interface/AddOns";
        assert_eq!(wtf_account_dir(p).unwrap(), PathBuf::from("/wow/_retail_/WTF/Account"));
    }

    // ── slice: `seq > cursor`, records carry their own `seq` (no `i` annotation) ──
    #[test]
    fn slice_sends_seq_after_cursor_no_i_annotation() {
        let entries = vec![
            json!({ "seq": 1, "k": "caught", "t": 100, "name": "A" }),
            json!({ "seq": 2, "k": "action", "t": 100, "spell": 1706 }), // same t, distinct seq
            json!({ "seq": 3, "k": "caught", "t": 110, "name": "B" }),
        ];
        // cursor 0 → send all; each record carries its own seq; NO positional `i` added
        let (all, max) = slice_chunk(&entries, 0, 100);
        assert_eq!(max, 3);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0]["seq"], json!(1));
        assert_eq!(all[2]["seq"], json!(3));
        assert!(all[0].get("i").is_none(), "server dedups on in-record seq — no `i`");
        // verbatim fields survive
        assert_eq!(all[0]["name"], json!("A"));
        assert_eq!(all[1]["spell"], json!(1706));
        // same-second records are DISTINCT (distinct seq)
        assert_eq!(all[0]["t"], all[1]["t"]);

        // cursor 1 → strictly-after keeps seq 2,3
        let (after, max) = slice_chunk(&entries, 1, 100);
        assert_eq!(after.len(), 2);
        assert_eq!(after[0]["seq"], json!(2));
        assert_eq!(after[1]["seq"], json!(3));
        assert_eq!(max, 3);

        // cursor at the head → nothing, cursor unchanged
        let (none, max) = slice_chunk(&entries, 3, 100);
        assert!(none.is_empty());
        assert_eq!(max, 3);
    }

    // ── a hole (lost seq) and a front-purge (missing low seqs) never stall the slice ──
    #[test]
    fn slice_skips_hole_and_purged_front_without_stalling() {
        // front seq 1,2 were purged (gone from the array); seq 5 is a HOLE (loss).
        let entries = vec![
            json!({ "seq": 3, "k": "a" }),
            json!({ "seq": 4, "k": "b" }),
            json!({ "seq": 6, "k": "c" }), // 5 missing
            json!({ "seq": 7, "k": "d" }),
        ];
        // cursor 0 → everything present ships; cursor lands on the highest present seq
        let (all, max) = slice_chunk(&entries, 0, 100);
        assert_eq!(all.len(), 4);
        assert_eq!(max, 7);
        // resume at 4 → 6,7 (the missing 5 is skipped, not a stall)
        let (rest, max) = slice_chunk(&entries, 4, 100);
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0]["seq"], json!(6));
        assert_eq!(max, 7);
    }

    // ── quarantine: a record with no `seq` is set aside; the batch still proceeds ──
    #[test]
    fn slice_quarantines_record_missing_seq_without_stalling() {
        let entries = vec![
            json!({ "seq": 1, "k": "a" }),
            json!({ "k": "bad-no-seq" }), // malformed — must not stall the stream
            json!({ "seq": 2, "k": "b" }),
        ];
        let (out, max) = slice_chunk(&entries, 0, 100);
        assert_eq!(out.len(), 2, "seq'd records still ship, bad one set aside");
        assert_eq!(out[0]["seq"], json!(1));
        assert_eq!(out[1]["seq"], json!(2));
        assert_eq!(max, 2);
    }

    // ── chunk boundaries by seq: ascending, contiguous, non-overlapping ──
    #[test]
    fn slice_chunk_paginates_by_seq() {
        let entries: Vec<Json> = (1..=10).map(|n| json!({ "seq": n, "k": "x" })).collect();
        let (c1, m1) = slice_chunk(&entries, 0, 4);
        assert_eq!(c1.len(), 4);
        assert_eq!(c1[0]["seq"], json!(1));
        assert_eq!(c1[3]["seq"], json!(4));
        assert_eq!(m1, 4);
        let (c2, m2) = slice_chunk(&entries, m1, 4);
        assert_eq!(c2[0]["seq"], json!(5));
        assert_eq!(c2[3]["seq"], json!(8));
        assert_eq!(m2, 8);
        let (c3, m3) = slice_chunk(&entries, m2, 4);
        assert_eq!(c3.len(), 2);
        assert_eq!(c3[0]["seq"], json!(9));
        assert_eq!(c3[1]["seq"], json!(10));
        assert_eq!(m3, 10);
        let (c4, m4) = slice_chunk(&entries, m3, 4);
        assert!(c4.is_empty());
        assert_eq!(m4, 10);
    }

    // ── wire: an append-only Dataset carries `stream_meta{seq,base}` verbatim; an
    //    upsert snapshot omits it (no seq/base identity) ──
    #[test]
    fn dataset_serializes_stream_meta_for_append_only_only() {
        let append = Dataset {
            since: Some("451".into()),
            stream_meta: Some(StreamMeta { seq: 1043, base: 452 }),
            entries: vec![json!({ "seq": 452, "k": "caught" })],
        };
        let v = serde_json::to_value(&append).unwrap();
        assert_eq!(v["since"], json!("451"));
        assert_eq!(v["stream_meta"]["seq"], json!(1043));
        assert_eq!(v["stream_meta"]["base"], json!(452));
        // the record still carries its own seq (server dedup key)
        assert_eq!(v["entries"][0]["seq"], json!(452));

        // an upsert snapshot (sessions/learned_items) omits since AND stream_meta
        let upsert = Dataset { since: None, stream_meta: None, entries: vec![json!({ "sid": "x" })] };
        let u = serde_json::to_value(&upsert).unwrap();
        assert!(u.get("since").is_none());
        assert!(u.get("stream_meta").is_none());
    }

    // ── base watermark → effective cursor floor (max(stored, base − 1)) ──
    #[test]
    fn effective_cursor_floors_at_base_minus_one() {
        // greenfield: base 1 → floor 0, stored wins
        assert_eq!(effective_cursor(0, 1), 0);
        assert_eq!(effective_cursor(42, 1), 42);
        // a purge advanced base past a lagging cursor → lift to base − 1 (skip the
        // purged-but-never-synced records so the stream can't stay stuck)
        assert_eq!(effective_cursor(100, 452), 451);
        // cursor already ahead of the watermark → keep it (never regress)
        assert_eq!(effective_cursor(500, 452), 500);
    }

    // ── queued = count of records still ahead of the cursor by seq ──
    #[test]
    fn count_after_counts_records_with_higher_seq() {
        let entries = vec![
            json!({ "seq": 3, "k": "a" }),
            json!({ "seq": 4, "k": "b" }),
            json!({ "seq": 6, "k": "c" }),
            json!({ "k": "bad" }), // no seq → not counted (quarantined)
        ];
        assert_eq!(count_after(&entries, 0), 3);
        assert_eq!(count_after(&entries, 4), 1);
        assert_eq!(count_after(&entries, 6), 0);
    }

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    // redesign §2: NO single-account auto-reselect. An empty selection is respected as
    // "sync nothing" even when exactly one account exists — so unchecking it sticks.
    // (Fresh installs get all-on once via list_sync_accounts seeding sync_accounts, not
    // via this function.)
    #[test]
    fn empty_selection_is_respected_no_single_account_reselect() {
        assert!(effective_accounts(&[], &v(&["ALICE"])).is_empty());
        assert!(effective_accounts(&[], &v(&["ALICE", "BOB"])).is_empty());
    }

    #[test]
    fn selection_is_the_intersection_with_discovered() {
        // an explicit selection is honored, order follows `discovered`
        assert_eq!(effective_accounts(&v(&["ALICE"]), &v(&["ALICE", "BOB"])), v(&["ALICE"]));
        assert_eq!(
            effective_accounts(&v(&["ALICE", "BOB"]), &v(&["ALICE", "BOB"])),
            v(&["ALICE", "BOB"])
        );
        assert_eq!(
            effective_accounts(&v(&["BOB", "ALICE"]), &v(&["ALICE", "BOB"])),
            v(&["ALICE", "BOB"])
        );
    }

    #[test]
    fn stale_selections_are_dropped() {
        // a selected name no longer on disk is filtered out — even down to zero, with no
        // single-account fallback (that was the old reselect bug).
        assert!(effective_accounts(&v(&["GONE"]), &v(&["ALICE", "BOB"])).is_empty());
        assert!(effective_accounts(&v(&["GONE"]), &v(&["ALICE"])).is_empty());
    }

    #[test]
    fn merge_stamp_keeps_newest_ignores_none_and_empty() {
        let mut acc: Option<String> = None;
        merge_stamp(&mut acc, None);
        assert_eq!(acc, None);
        merge_stamp(&mut acc, Some("  "));
        assert_eq!(acc, None);
        // first real stamp is taken
        merge_stamp(&mut acc, Some("2026-07-13T10:00:00Z"));
        assert_eq!(acc.as_deref(), Some("2026-07-13T10:00:00Z"));
        // older is ignored
        merge_stamp(&mut acc, Some("2026-07-13T09:00:00Z"));
        assert_eq!(acc.as_deref(), Some("2026-07-13T10:00:00Z"));
        // newer wins
        merge_stamp(&mut acc, Some("2026-07-13T11:30:00Z"));
        assert_eq!(acc.as_deref(), Some("2026-07-13T11:30:00Z"));
        // None never clears a captured value
        merge_stamp(&mut acc, None);
        assert_eq!(acc.as_deref(), Some("2026-07-13T11:30:00Z"));
    }

    #[test]
    fn no_accounts_discovered_is_empty() {
        assert!(effective_accounts(&[], &[]).is_empty());
        assert!(effective_accounts(&v(&["ALICE"]), &[]).is_empty());
    }

    // ── ack decision (Gap 1): drained quarantine, missing-stream hold, advance ──

    fn sa(cursor: Option<&str>, unknown: bool) -> StreamAck {
        StreamAck { cursor: cursor.map(|s| s.to_string()), unknown, ..Default::default() }
    }

    // (a) drained=true → ADVANCE to computed_cursor (never regressing) + Drained warning
    #[test]
    fn decide_ack_drained_advances_and_warns() {
        // no stream echoed (drained acks carry an empty map) — still advances + warns
        assert_eq!(
            decide_ack(true, None, 5, 2),
            AckDecision { cursor: Some(5), unknown: false, warn: Some(AckWarn::Drained) }
        );
        // never regress below the current cursor even under drain
        assert_eq!(
            decide_ack(true, None, 3, 7),
            AckDecision { cursor: Some(7), unknown: false, warn: Some(AckWarn::Drained) }
        );
        // drained takes precedence even if a stream somehow rode along
        assert_eq!(
            decide_ack(true, Some(&sa(Some("9"), false)), 5, 2),
            AckDecision { cursor: Some(5), unknown: false, warn: Some(AckWarn::Drained) }
        );
    }

    // (b) drained=false, stream present, unknown=false → advance from the ack cursor,
    //     never regressing; missing/unparseable ack cursor falls back to what we sent
    #[test]
    fn decide_ack_present_advances_from_cursor_never_regressing() {
        // ack cursor ahead of current → take it
        assert_eq!(
            decide_ack(false, Some(&sa(Some("8"), false)), 5, 2),
            AckDecision { cursor: Some(8), unknown: false, warn: None }
        );
        // ack cursor BEHIND current → don't regress; fall back to computed.max(current)
        assert_eq!(
            decide_ack(false, Some(&sa(Some("1"), false)), 5, 4),
            AckDecision { cursor: Some(5), unknown: false, warn: None }
        );
        // no ack cursor at all → fall back to what we sent
        assert_eq!(
            decide_ack(false, Some(&sa(None, false)), 5, 2),
            AckDecision { cursor: Some(5), unknown: false, warn: None }
        );
    }

    // (c) drained=false, stream present, unknown=true → HOLD (do not advance), no warn
    #[test]
    fn decide_ack_unknown_stream_does_not_advance() {
        assert_eq!(
            decide_ack(false, Some(&sa(Some("9"), true)), 5, 2),
            AckDecision { cursor: None, unknown: true, warn: None }
        );
    }

    // (d) drained=false, stream MISSING → HOLD (cursor unchanged) + MissingStream warning
    #[test]
    fn decide_ack_missing_stream_holds_and_warns() {
        assert_eq!(
            decide_ack(false, None, 5, 2),
            AckDecision { cursor: None, unknown: false, warn: Some(AckWarn::MissingStream) }
        );
    }

    // serde: `{"drained":true}` with no `streams` → drained flag set, empty stream map
    #[test]
    fn drained_body_deserializes_with_empty_streams() {
        let ack: IngestAck = serde_json::from_str(r#"{"drained":true}"#).unwrap();
        assert!(ack.drained);
        assert!(ack.streams.is_empty());
        assert!(ack.warnings.is_empty());
        // and the ordinary body defaults drained=false
        let ok: IngestAck = serde_json::from_str(r#"{"streams":{}}"#).unwrap();
        assert!(!ok.drained);
    }

    #[test]
    fn flavor_token_cross_check() {
        // retail dir + mainline record (1) → consistent
        assert_eq!(flavor_token_matches("retail", Some(1)), Some(true));
        // retail dir + classic record → mismatch (flag)
        assert_eq!(flavor_token_matches("retail", Some(2)), Some(false));
        // classic dir + a classic line (era/wrath/…) → consistent
        assert_eq!(flavor_token_matches("classic", Some(2)), Some(true));
        assert_eq!(flavor_token_matches("classic_era", Some(11)), Some(true));
        // classic dir + mainline record → mismatch
        assert_eq!(flavor_token_matches("classic", Some(1)), Some(false));
        // unknown token → skip (never an allowlist)
        assert_eq!(flavor_token_matches("ptr", Some(1)), None);
        assert_eq!(flavor_token_matches("weird_future", Some(2)), None);
        // no recorded flavor → nothing to check
        assert_eq!(flavor_token_matches("retail", None), None);
    }
}
