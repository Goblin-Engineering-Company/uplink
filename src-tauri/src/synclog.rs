//! The rolling sync log: what every sync pass did, and what triggered it.
//!
//! WHY ITS OWN FILE. `AppState::persist()` rewrites the WHOLE config, so a
//! per-sync append into `config.json` would put write pressure back on the file
//! the in-flight guard (audit C2) was just added to protect. The log therefore
//! lives beside the config as `sync-log.json`, with its own atomic write and its
//! own lock. It is also strictly disposable: a missing or damaged file degrades
//! to an empty log, and a write failure is logged and swallowed. Nothing here may
//! ever fail a sync pass.
//!
//! WHAT IT IS FOR. Before this, the only "log" the UI had was the return value of
//! the manual `sync_now` command, so a pass fired by the SavedVariables watcher or
//! by the backstop timer left no trace: data appeared on the site with the app
//! showing nothing. Every pass now records one entry carrying its trigger, so the
//! user can tell an upload they asked for from one that happened on its own.

use crate::config::AppState;
use crate::sync::SyncResult;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// How many passes the log keeps. Oldest entries fall off the end.
pub const SYNC_LOG_CAP: usize = 50;

/// The event the frontend listens on. Carries the entry that was just recorded,
/// so a view can render the pass without re-reading the file.
pub const SYNC_FINISHED_EVENT: &str = "uplink:sync-finished";

/// What caused a pass to run. The user explicitly wants to tell these apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The user pressed "Sync now" (main window or tray panel).
    Manual,
    /// The SavedVariables watcher saw the game flush its data.
    Instant,
    /// The interval backstop came round.
    Timed,
}

impl Trigger {
    /// The stable wire token. The UI maps it to display copy.
    pub fn as_str(self) -> &'static str {
        match self {
            Trigger::Manual => "manual",
            Trigger::Instant => "instant",
            Trigger::Timed => "timed",
        }
    }

    /// Does this trigger run the pass as `manual` (ignoring the while-running gate
    /// and the mtime short-circuit)? Instant capture does, by design: a flush is
    /// exactly the moment there IS something new to read.
    pub fn is_forced(self) -> bool {
        !matches!(self, Trigger::Timed)
    }
}

/// One recorded pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncLogEntry {
    /// Unix seconds when the pass finished.
    pub at: i64,
    /// `Trigger::as_str`.
    pub trigger: String,
    /// Entries sliced and sent across every stream in the pass.
    pub sent: u64,
    /// New rows the server stored (duplicates excluded).
    pub inserted: u64,
    /// How many streams reported an error.
    pub errors: u64,
    /// Set when the WHOLE pass was skipped (another sync held the lane, or the
    /// while-running gate stopped it). A skip is not a failure and must not read
    /// as one.
    pub skipped: Option<String>,
    /// Set when the pass itself failed before it could sweep anything.
    pub error: Option<String>,
    /// The per-stream detail, exactly as the manual command used to return it.
    pub results: Vec<SyncResult>,
}

/// The on-disk shape. An object rather than a bare array so a future field has
/// somewhere to go without invalidating every existing log.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct SyncLogFile {
    /// Newest first.
    entries: Vec<SyncLogEntry>,
}

/// Serializes the read/modify/write. Passes are already serialized by the sync
/// in-flight guard, but the log must not depend on that to stay intact.
fn write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// The Tauri handle used to emit `SYNC_FINISHED_EVENT`. Set once at setup; absent
/// in the headless `sync-once` path and in tests, where recording still writes the
/// file and simply emits nothing.
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Called once from `run()`'s setup so background passes can reach the UI.
pub fn set_app_handle(handle: tauri::AppHandle) {
    let _ = APP.set(handle);
}

/// Where the log lives: beside `config.json`, never inside it.
pub fn path_for(config_path: &Path) -> PathBuf {
    config_path.with_file_name("sync-log.json")
}

/// Read the log, newest first. A missing file is an empty log; a damaged one is a
/// warning and an empty log. Reading the log can never be an error the caller has
/// to handle, because there is nothing a caller could usefully do about it.
pub fn read(path: &Path) -> Vec<SyncLogEntry> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<SyncLogFile>(&raw) {
        Ok(f) => f.entries,
        Err(e) => {
            eprintln!("sync log: {} could not be parsed ({e}); starting a fresh log", path.display());
            Vec::new()
        }
    }
}

/// Prepend `entry` and cap the log. Pure, so the capping rule is testable without
/// touching a disk.
fn with_entry(mut entries: Vec<SyncLogEntry>, entry: SyncLogEntry) -> Vec<SyncLogEntry> {
    entries.insert(0, entry);
    entries.truncate(SYNC_LOG_CAP);
    entries
}

/// Atomic write, same shape as the config's: unique temp sibling, then rename over
/// the target, so a crash mid-write can only ever truncate the temp.
fn write(path: &Path, entries: &[SyncLogEntry]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let body = SyncLogFile { entries: entries.to_vec() };
    let s = serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?;
    let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("sync-log.json");
    let tmp = path.with_file_name(format!(".{fname}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, s.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Turn a finished pass into an entry. A whole-pass skip is the synthetic single
/// result `sync::skip_result` produces (no install, no addon) — recorded as
/// `skipped`, never as an error and never as a successful upload.
pub fn entry_for(trigger: Trigger, outcome: &Result<Vec<SyncResult>, String>) -> SyncLogEntry {
    let mut e = SyncLogEntry { at: now_secs(), trigger: trigger.as_str().to_string(), ..Default::default() };
    match outcome {
        Err(err) => e.error = Some(err.clone()),
        Ok(results) => {
            let whole_pass_skip = results.len() == 1
                && results[0].addon.is_empty()
                && results[0].install_id.is_empty()
                && results[0].skipped.is_some();
            if whole_pass_skip {
                e.skipped = results[0].skipped.clone();
            } else {
                for r in results {
                    e.sent += r.found;
                    e.inserted += r.inserted;
                    if r.error.is_some() {
                        e.errors += 1;
                    }
                }
                e.results = results.clone();
            }
        }
    }
    e
}

/// Record a finished pass and tell the UI. Best-effort throughout: a log that
/// cannot be written is a warning on stderr, never a failed sync.
pub fn record(state: &AppState, trigger: Trigger, outcome: &Result<Vec<SyncResult>, String>) {
    let entry = entry_for(trigger, outcome);
    let path = path_for(&state.config_path);
    {
        let _lock = write_lock().lock().unwrap_or_else(|p| p.into_inner());
        let entries = with_entry(read(&path), entry.clone());
        if let Err(e) = write(&path, &entries) {
            eprintln!("sync log: could not write {} ({e}); the pass itself was fine", path.display());
        }
    }
    if let Some(app) = APP.get() {
        use tauri::Emitter;
        let _ = app.emit(SYNC_FINISHED_EVENT, &entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at: i64) -> SyncLogEntry {
        SyncLogEntry { at, trigger: "manual".into(), ..Default::default() }
    }

    // The ring caps at SYNC_LOG_CAP and drops the OLDEST, keeping newest first.
    #[test]
    fn log_caps_and_evicts_the_oldest() {
        let mut entries: Vec<SyncLogEntry> = Vec::new();
        for at in 0..(SYNC_LOG_CAP as i64 + 10) {
            entries = with_entry(entries, entry(at));
        }
        assert_eq!(entries.len(), SYNC_LOG_CAP);
        // newest first
        assert_eq!(entries[0].at, SYNC_LOG_CAP as i64 + 9);
        // the oldest survivor is exactly CAP back from the newest; 0..9 are gone
        assert_eq!(entries[SYNC_LOG_CAP - 1].at, 10);
        assert!(!entries.iter().any(|e| e.at < 10));
    }

    // A missing file is an empty log, not an error.
    #[test]
    fn missing_log_file_reads_empty() {
        let p = std::env::temp_dir().join("uplink-synclog-missing-xyz.json");
        let _ = std::fs::remove_file(&p);
        assert!(read(&p).is_empty());
    }

    // A damaged file degrades to empty rather than propagating a parse error, and
    // the next write repairs it.
    #[test]
    fn corrupt_log_file_reads_empty_and_rewrites() {
        let p = std::env::temp_dir().join(format!("uplink-synclog-corrupt-{}.json", std::process::id()));
        std::fs::write(&p, b"{ this is not json").unwrap();
        assert!(read(&p).is_empty());
        write(&p, &[entry(7)]).unwrap();
        let back = read(&p);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].at, 7);
        let _ = std::fs::remove_file(&p);
    }

    // A whole-pass skip is recorded as a skip: no upload counted, no error raised.
    #[test]
    fn whole_pass_skip_is_not_an_error() {
        let skip = SyncResult {
            skipped: Some(crate::sync::SYNC_ALREADY_RUNNING.to_string()),
            ..Default::default()
        };
        let e = entry_for(Trigger::Manual, &Ok(vec![skip]));
        assert_eq!(e.skipped.as_deref(), Some(crate::sync::SYNC_ALREADY_RUNNING));
        assert_eq!(e.errors, 0);
        assert_eq!(e.sent, 0);
        assert!(e.error.is_none());
        assert!(e.results.is_empty());
    }

    // A real pass keeps its per-stream detail and rolls up sent/new/errors.
    #[test]
    fn real_pass_rolls_up_counts() {
        let ok = SyncResult { addon: "sbf".into(), install_id: "i".into(), found: 5, inserted: 4, ..Default::default() };
        let bad = SyncResult { addon: "haul".into(), install_id: "i".into(), found: 2, error: Some("boom".into()), ..Default::default() };
        let e = entry_for(Trigger::Timed, &Ok(vec![ok, bad]));
        assert_eq!(e.trigger, "timed");
        assert_eq!(e.sent, 7);
        assert_eq!(e.inserted, 4);
        assert_eq!(e.errors, 1);
        assert_eq!(e.results.len(), 2);
        assert!(e.skipped.is_none());
    }
}
