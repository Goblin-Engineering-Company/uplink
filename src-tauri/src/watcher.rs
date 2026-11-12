//! Feedwerks §4.1 — the SavedVariables live watcher ("instant capture").
//!
//! A WoW addon's only egress is its SavedVariables file, and the client
//! flushes it ONLY on /reload or logout — so /reload IS the push trigger.
//! This watcher collapses capture latency from the ≤15-minute backstop poll
//! to a couple of seconds: it watches every install's `WTF/Account` tree,
//! debounces the flurry of writes a flush produces, then runs the EXISTING
//! sync pass. No new ingest path — it only fires the current one sooner.
//!
//! Deliberate: the triggered pass runs as `manual = true`. A flush happens
//! *because the game is running*, so the sync-while-running background gate
//! must not swallow it — a watcher fire is an intentional capture, exactly
//! like clicking Sync now. And it goes through `sync::sync_coalesced`, not
//! `sync_all` directly: a fire that lands while the backstop pass is in
//! flight must COALESCE (one follow-up pass, however many fires), never be
//! dropped by the in-flight guard — dropping it is the "instant capture that
//! silently isn't" bug.
//!
//! Failure posture (spec): if the watcher can't start (permissions, platform,
//! inotify watch limits) log and fall back to the interval backstop — capture
//! degrades to "slow", never "broken". The backstop loop in lib.rs is a
//! separate task and is untouched by anything here. Roots that FAILED to arm
//! are retried on a cadence, so a Steam Deck that hit its inotify limit at
//! launch arms itself once the limit is raised — no restart needed.

use crate::config::AppState;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;
use tauri::Manager;

/// Let launch settle (config load, pairing state) before touching disk.
const STARTUP_DELAY_SECS: u64 = 6;
/// Rescan cadence: picks up config changes (installs added, toggle flipped).
const RESCAN_SECS: u64 = 60;
/// After a root fails to arm, retry it every Nth rescan tick (≈5 min at the
/// default cadence). Bounded retry, not every tick: rebuilding drops and
/// re-adds the watches that DID arm, so a permanently unwatchable root must
/// not churn the working ones (or spam the log) once a minute forever.
const FAILED_RETRY_TICKS: u32 = 5;
/// Floor for the configured debounce — a 0 would fire mid-write.
const MIN_DEBOUNCE_MS: u64 = 100;

/// The `WTF/Account` roots for every configured install that exists on disk.
fn account_roots(state: &AppState) -> BTreeSet<PathBuf> {
    let cfg = state.config.lock().unwrap();
    cfg.installs
        .iter()
        .filter_map(|i| crate::sync::wtf_account_dir(&i.path))
        .filter(|p| p.is_dir())
        .collect()
}

/// An fs event is interesting when it touches a `.lua` file inside a
/// `SavedVariables` directory (WoW writes `<Addon>.lua` + `.lua.bak`).
fn interesting(ev: &notify::Event) -> bool {
    ev.paths.iter().any(|p| {
        let in_sv = p.components().any(|c| c.as_os_str() == "SavedVariables");
        let lua = p.extension().map(|e| e == "lua").unwrap_or(false);
        in_sv && lua
    })
}

pub fn spawn(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(STARTUP_DELAY_SECS)).await;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        // The watcher must outlive iterations; rebuilt when the root set changes
        // (install added/removed, drive mounted/unmounted, toggle flipped).
        // Held ONLY to keep the OS watches alive (drop = unwatch) — never read.
        let mut _watcher: Option<RecommendedWatcher> = None;
        // Roots that actually armed, the roots we last BUILT for, and the ones
        // that refused — kept apart so a partial failure is retried instead of
        // being recorded as "watched" and forgotten until the next restart.
        let mut armed: BTreeSet<PathBuf> = BTreeSet::new();
        let mut built_for: BTreeSet<PathBuf> = BTreeSet::new();
        let mut failed: BTreeSet<PathBuf> = BTreeSet::new();
        let mut retry_in: u32 = 0;

        loop {
            // ── (re)build the watch set ──
            {
                let state = handle.state::<AppState>();
                let enabled = state.config.lock().unwrap().watch_enabled;
                let roots = if enabled { account_roots(state.inner()) } else { BTreeSet::new() };
                // Rebuild when the wanted set moved, or when something is still
                // unarmed and its retry is due.
                let stale = roots != built_for;
                let retry_due = armed != roots && retry_in == 0;
                if stale || retry_due {
                    _watcher = None; // drop old watches first
                    let prev_armed = std::mem::take(&mut armed);
                    built_for = roots.clone();
                    let mut now_failed: BTreeSet<PathBuf> = BTreeSet::new();
                    if !roots.is_empty() {
                        let tx2 = tx.clone();
                        match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                            if let Ok(ev) = res {
                                if interesting(&ev) {
                                    let _ = tx2.send(());
                                }
                            }
                        }) {
                            Ok(mut w) => {
                                for r in &roots {
                                    match w.watch(r, RecursiveMode::Recursive) {
                                        Ok(()) => {
                                            armed.insert(r.clone());
                                        }
                                        Err(e) => {
                                            // Fail SOFT: this root just isn't instant.
                                            // Log once per new failure (a retry that
                                            // fails the same way stays quiet).
                                            if !failed.contains(r) {
                                                crate::sync::log_progress(&format!(
                                                    "watcher: cannot watch {} ({e}) — backstop poll covers it",
                                                    r.display()
                                                ));
                                            }
                                            now_failed.insert(r.clone());
                                        }
                                    }
                                }
                                if !armed.is_empty() && armed != prev_armed {
                                    crate::sync::log_progress(&format!(
                                        "watcher: instant capture armed on {} root(s)",
                                        armed.len()
                                    ));
                                }
                                _watcher = Some(w);
                            }
                            Err(e) => {
                                // Total failure: rely on the backstop; retry the build
                                // on the retry cadence.
                                if failed != roots {
                                    crate::sync::log_progress(&format!(
                                        "watcher: failed to start ({e}) — relying on the backstop poll"
                                    ));
                                }
                                now_failed = roots.clone();
                            }
                        }
                    }
                    failed = now_failed;
                    retry_in = if failed.is_empty() { 0 } else { FAILED_RETRY_TICKS };
                }
            }

            // ── wait: an fs event, or the rescan tick (picks up config changes) ──
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(RESCAN_SECS)) => {
                    retry_in = retry_in.saturating_sub(1);
                }
                got = rx.recv() => {
                    if got.is_none() { return; } // channel closed — app teardown
                    // Debounce: a flush writes several files back-to-back; keep
                    // draining until it's been quiet for the configured window.
                    let debounce = {
                        let state = handle.state::<AppState>();
                        let c = state.config.lock().unwrap();
                        c.watch_debounce_ms.max(MIN_DEBOUNCE_MS)
                    };
                    loop {
                        match tokio::time::timeout(Duration::from_millis(debounce), rx.recv()).await {
                            Ok(Some(())) => continue, // still writing — restart the quiet window
                            Ok(None) => return,
                            Err(_) => break,          // quiet — flush is done
                        }
                    }
                    let state = handle.state::<AppState>();
                    let enabled = state.config.lock().unwrap().watch_enabled;
                    if enabled && crate::auth::has_token() {
                        crate::sync::log_progress("watcher: SavedVariables changed — instant capture");
                        // Coalescing entry point: waits out an in-flight pass rather
                        // than being skipped by the guard. `None` = another fire's
                        // pass already covered this one. Each pass that actually runs
                        // records itself to the sync log and emits uplink:sync-finished
                        // from there, so a coalesced follow-up is logged too and N
                        // collapsed fires never produce N entries.
                        let _ = crate::sync::sync_coalesced(state.inner()).await;
                    }
                }
            }
        }
    });
}
