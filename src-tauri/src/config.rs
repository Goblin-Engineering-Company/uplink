//! Local, persisted app configuration (installs, per-install addon selection,
//! preferences). Lives as JSON in the app data dir. The device *token* never
//! lives here — it's in the OS keychain (see auth.rs). Every field has a serde
//! default so an older config file still loads after a schema addition.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Single source of truth for the default server. Overridable by the
/// `UPLINK_BASE_URL` env var and by the Settings base-URL field — but the
/// literal default string appears here and nowhere else.
pub const DEFAULT_BASE_URL: &str = "https://goblineng.co";

pub fn default_base_url() -> String {
    std::env::var("UPLINK_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

fn default_channel() -> String {
    "public".to_string()
}
/// Shared `#[serde(default)]` provider for bool fields that default ON, so an older
/// config file missing the key still loads as `true` (matches the Default impl).
fn default_true() -> bool {
    true
}
/// Backstop sync interval (seconds). Configurable via Settings; the literal
/// default appears here and nowhere else (everything-configurable rule).
pub fn default_sync_interval() -> u64 {
    900
}
fn default_theme() -> String {
    "workshop".to_string()
}
fn default_self_update_channel() -> String {
    "public".to_string()
}
fn default_auto_update_time() -> String {
    "03:00".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub handle: String,
    pub tier: Option<String>,
    pub role: String,
    #[serde(default)]
    pub channels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledAddon {
    pub slug: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub installed_version: Option<String>,
    #[serde(default)]
    pub pinned_version: Option<String>,
    #[serde(default)]
    pub channel_override: Option<String>,
    /// Per-addon auto-update flag — the SOURCE OF TRUTH for the scheduler (spec §3b).
    /// New addons inherit the install's `auto_update` default when first enabled; the
    /// install-row checkbox bulk-sets every addon here. Defaults false for old configs.
    #[serde(default)]
    pub auto_update: bool, // serde default stays false so an OLD per-addon record isn't silently flipped; a freshly-constructed record defaults ON (see `new`, §3.8)
    /// Per-addon data-sync flag (redesign §3): when false, the sweep skips this addon's
    /// streams entirely. Defaults ON via `default_true` so an older config missing the
    /// key still syncs — the "just works, consume everything quietly" posture. This is
    /// one of the two sync gates (the other is the account selection); there is no
    /// global master switch.
    #[serde(default = "default_true")]
    pub sync: bool,
    /// The actual on-disk folder name written at install time (for uninstall).
    #[serde(default)]
    pub folder: Option<String>,
}

impl InstalledAddon {
    pub fn new(slug: &str) -> Self {
        Self {
            slug: slug.to_string(),
            enabled: false,
            installed_version: None,
            pinned_version: None,
            channel_override: None,
            auto_update: true, // §3.8: freshly-created addon records default to auto-update ON
            sync: true,        // redesign §3: freshly-created addon records sync by default
            folder: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Install {
    pub id: String,
    pub label: String,
    pub path: String,
    pub flavor: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    /// Install-level auto-update default (§3.7): the default for newly-added addons
    /// AND the bulk toggle. Defaults ON so an older config missing the key still opts
    /// in to the "just works" posture.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// WoW account folder names (`WTF/Account/<ACCOUNT>`) the user has chosen to
    /// sync for this install. A machine's account folders can belong to DIFFERENT
    /// Battle.net logins, so we NEVER sweep an account the user hasn't opted into.
    /// Empty here means "none selected" and is RESPECTED (redesign §2) — a deliberately
    /// emptied selection stays empty. A fresh install is defaulted to ALL discovered
    /// accounts once (see `accounts_initialized`), not via a runtime auto-select.
    #[serde(default)]
    pub sync_accounts: Vec<String>,
    /// Have we ever initialized `sync_accounts` for this install (redesign §2)? False
    /// on a fresh install: the first time accounts are discovered (in
    /// `list_sync_accounts`) with an empty selection, we set `sync_accounts` = all
    /// discovered and flip this true. Thereafter an empty selection is a deliberate
    /// "sync nothing", never re-defaulted. `#[serde(default)]` → false for old configs,
    /// which is correct: they get the one-time all-on default on next discovery.
    #[serde(default)]
    pub accounts_initialized: bool,
    #[serde(default)]
    pub addons: Vec<InstalledAddon>,
    /// Runtime-only: is the path currently present? Recomputed on every read;
    /// persisting a stale value is harmless.
    #[serde(default)]
    pub online: bool,
}

impl Install {
    /// Get (or lazily create) the per-install record for an addon slug.
    pub fn addon_mut(&mut self, slug: &str) -> &mut InstalledAddon {
        if !self.addons.iter().any(|a| a.slug == slug) {
            self.addons.push(InstalledAddon::new(slug));
        }
        self.addons.iter_mut().find(|a| a.slug == slug).unwrap()
    }
    /// The effective delivery channel for an addon: its override else the install's.
    pub fn channel_for(&self, slug: &str) -> String {
        self.addons
            .iter()
            .find(|a| a.slug == slug)
            .and_then(|a| a.channel_override.clone())
            .unwrap_or_else(|| self.channel.clone())
    }
}

/// One per-stream sync cursor + cached display counters, keyed per
/// install · account · addon · stream (spec §6). Persisted so cursors survive a
/// restart and the Data/tray UIs can render last-sync/queued without re-parsing a
/// multi-megabyte SV on every open.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamState {
    pub install_id: String,
    pub account: String,
    pub addon: String, // catalog slug (lowercase)
    pub stream: String,
    /// Opaque server cursor (a stringified max-`t`); advanced ONLY from the ack.
    #[serde(default)]
    pub cursor: String,
    /// Entries observed at last parse that are newer than the cursor (unsent).
    #[serde(default)]
    pub queued: u64,
    /// Total syncable entries seen at last parse (for context in the UI).
    #[serde(default)]
    pub total: u64,
    /// Unix time of the last successful upload for this stream.
    #[serde(default)]
    pub last_sync: Option<i64>,
    /// Entries the server accepted on the last upload.
    #[serde(default)]
    pub last_accepted: u64,
    /// mtime (unix secs) of the SV file the last parse consumed — lets a periodic
    /// pass skip re-parsing an unchanged multi-MB file.
    #[serde(default)]
    pub file_mtime: Option<i64>,
    /// Is this stream STUCK (redesign §6)? True when the most recent sync attempt left
    /// `queued > 0` yet did NOT advance the cursor — data that won't drain (a server-
    /// `unknown` stream, or a stream renamed/removed in the SV so its old queue can
    /// never send). The Data page surfaces "Clear queue" ONLY when something is stuck.
    /// A clean advance or a fully-drained stream sets this false. `#[serde(default)]`.
    #[serde(default)]
    pub stuck: bool,
}

impl StreamState {
    pub fn matches(&self, install_id: &str, account: &str, addon: &str, stream: &str) -> bool {
        self.install_id == install_id && self.account == account && self.addon == addon && self.stream == stream
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub device_id: Option<i64>,
    #[serde(default)]
    pub account: Option<Account>,
    #[serde(default)]
    pub installs: Vec<Install>,
    #[serde(default)]
    pub sync_while_running: bool,
    #[serde(default = "default_sync_interval")]
    pub sync_interval_secs: u64,
    /// Per install·account·addon·stream cursors + cached sync counters (§6).
    #[serde(default)]
    pub sync: Vec<StreamState>,
    #[serde(default)]
    pub dismissed_broadcast: Option<i64>,
    #[serde(default)]
    pub onboarded: bool,
    /// The last server `resync_requested_at` stamp we HONORED (contract §5b). The
    /// server stamps this (ISO-8601 UTC) when an admin purges the account or resets
    /// aggregates; when a device-authenticated response (an ingest ack or /api/me)
    /// carries a stamp strictly newer than this, we zero every cursor and full-resync,
    /// then record the stamp here so the same purge never re-triggers. None = nothing
    /// honored yet (any real stamp is then "newer" — a harmless one-time baseline).
    #[serde(default)]
    pub last_resync_honored: Option<String>,
    /// Last-seen `stamps.catalog` fingerprint from /api/me (contract §4.3). Opaque
    /// 16-hex; never parsed, only compared. A change means the caller's entitled
    /// view of the `gec.addons/2` catalog moved, so re-fetch the catalog. None =
    /// never seen a stamp yet (any real stamp is then "new").
    #[serde(default)]
    pub catalog_stamp: Option<String>,
    /// Runtime-only: does the keychain hold a device token? Filled on read.
    #[serde(default)]
    pub paired: bool,
    /// Runtime-only: is this a Flatpak build (Steam Deck)? Filled on read like
    /// `paired`. A Flatpak can't self-update (read-only sandbox), so the UI hides
    /// the self-updater and the scheduler skips the app self-update step (spec §2).
    #[serde(default)]
    pub is_flatpak: bool,
    /// The app's OWN self-update track (distinct from per-install addon
    /// channels). One binary per machine, so this is app-global. Gated: only
    /// channels the account is entitled to are selectable in the UI (spec §7).
    #[serde(default = "default_self_update_channel")]
    pub self_update_channel: String,
    /// Master switch for the daily auto-update pass (app + flagged installs). Defaults
    /// ON (§3.6) — the "just works" posture; an older config missing the key opts in.
    #[serde(default = "default_true")]
    pub auto_update_enabled: bool,
    /// Local time-of-day "HH:MM" the daily auto-update pass runs (spec §9).
    #[serde(default = "default_auto_update_time")]
    pub auto_update_time: String,
    /// DEV-ONLY override for the self-update endpoint. When set (non-empty), the
    /// self-update path pulls its Tauri manifest DIRECTLY from this URL instead of
    /// the gated server channel manifest — no channel query, no `Authorization`
    /// bearer, and no 200/204/403 status probe (a plain file server has no such
    /// semantics). Signature verification against the committed updater pubkey
    /// STILL applies, so an untrusted bundle can't be installed — it's safe.
    /// Empty/None → normal production behavior.
    #[serde(default)]
    pub dev_update_url: Option<String>,
    /// Launch straight to the menu-bar/tray with NO main window shown. Independent
    /// of launch-at-login (whose state the OS/plugin owns): a manual launch with
    /// this off opens the window normally; with it on (or a `--hidden` login
    /// launch) the window stays hidden until the tray "Open Uplink" is used.
    /// Defaults ON (§3.5): an onboarded manual launch starts quietly in the menu bar.
    /// The launch logic still ALWAYS shows the window on first run (`!onboarded`),
    /// so this default never hides onboarding (lib.rs, §3.9).
    #[serde(default = "default_true")]
    pub start_in_tray: bool,
    /// Per-account DISPLAY aliases (redesign §4). Key = the real WoW account folder name
    /// (the identity + the sync/wire key — NEVER changed). Value = a user-chosen display
    /// label shown everywhere the account appears in the UI, so the real account
    /// number/name can be hidden in screen recordings. UI-only: `sync_accounts`, the
    /// wire payload, and every cursor key keep the real name. An absent/empty entry
    /// falls back to the real name.
    #[serde(default)]
    pub account_aliases: HashMap<String, String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            theme: default_theme(),
            device_name: None,
            device_id: None,
            account: None,
            installs: Vec::new(),
            sync_while_running: false,
            sync_interval_secs: default_sync_interval(),
            sync: Vec::new(),
            dismissed_broadcast: None,
            onboarded: false,
            last_resync_honored: None,
            catalog_stamp: None,
            paired: false,
            is_flatpak: false,
            self_update_channel: default_self_update_channel(),
            auto_update_enabled: true,
            auto_update_time: default_auto_update_time(),
            dev_update_url: None,
            start_in_tray: true,
            account_aliases: HashMap::new(),
        }
    }
}

// Per-process counter for unique temp filenames in the atomic save (below).
static SAVE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        let Ok(s) = std::fs::read_to_string(path) else {
            return AppConfig::default();
        };
        match serde_json::from_str(&s) {
            Ok(cfg) => cfg,
            Err(e) => {
                // Never silently discard a damaged config — the next save() would
                // overwrite it and every install / sync cursor / device_id / pref
                // would be gone with no trace (cursor loss alone forces a full
                // re-upload of every stream). Preserve it beside the original for
                // recovery, then start fresh.
                let backup = path.with_extension("json.corrupt");
                let _ = std::fs::rename(path, &backup);
                eprintln!(
                    "config: parse error ({e}); preserved the damaged file at {} \
                     and started with defaults",
                    backup.display()
                );
                AppConfig::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let s = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        // Atomic write: serialize to a unique temp sibling, then rename over the
        // target. rename(2) is atomic within a filesystem, so a crash / full disk
        // mid-write can only leave the TEMP truncated — never the live config
        // (which load() would then reset to defaults). The temp is a sibling so it
        // stays on the destination's filesystem; the unique name avoids any clash
        // if two writers ever race.
        let seq = SAVE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("config.json");
        let tmp = path.with_file_name(format!(".{fname}.{}.{seq}.tmp", std::process::id()));
        std::fs::write(&tmp, s.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp); // don't leave the temp behind on failure
            e.to_string()
        })
    }

    pub fn install_mut(&mut self, id: &str) -> Option<&mut Install> {
        self.installs.iter_mut().find(|i| i.id == id)
    }

    /// Find (or lazily create) the sync record for a stream key.
    pub fn stream_state_mut(&mut self, install_id: &str, account: &str, addon: &str, stream: &str) -> &mut StreamState {
        if !self.sync.iter().any(|s| s.matches(install_id, account, addon, stream)) {
            self.sync.push(StreamState {
                install_id: install_id.to_string(),
                account: account.to_string(),
                addon: addon.to_string(),
                stream: stream.to_string(),
                cursor: String::new(),
                queued: 0,
                total: 0,
                last_sync: None,
                last_accepted: 0,
                file_mtime: None,
                stuck: false,
            });
        }
        self.sync.iter_mut().find(|s| s.matches(install_id, account, addon, stream)).unwrap()
    }

    /// Read-only cursor lookup (empty string when unseen).
    pub fn cursor_of(&self, install_id: &str, account: &str, addon: &str, stream: &str) -> String {
        self.sync.iter().find(|s| s.matches(install_id, account, addon, stream))
            .map(|s| s.cursor.clone()).unwrap_or_default()
    }

    /// Zero EVERY local stream cursor — the "Full resync" primitive. The next sync
    /// re-sends all data from the start; the server dedupes rows on
    /// (device·account·addon·stream·i), so this is always safe/idempotent. Also
    /// clears `file_mtime` (forces a re-parse past the mtime short-circuit) and marks
    /// everything queued again. Returns how many non-empty cursors were cleared. This
    /// is the single source of truth used by the manual "Full resync" button, the
    /// headless reset, AND the server-requested resync (contract §5b).
    pub fn zero_all_cursors(&mut self) -> u64 {
        let mut cleared = 0u64;
        for s in self.sync.iter_mut() {
            if !s.cursor.is_empty() {
                cleared += 1;
            }
            s.cursor = String::new();
            s.file_mtime = None;
            s.queued = s.total;
        }
        cleared
    }

    /// Honor a server-sent `resync_requested_at` stamp (contract §5b). If `stamp` is
    /// present, non-empty, and STRICTLY newer than `last_resync_honored`, zero every
    /// cursor (via `zero_all_cursors`), record the stamp as honored, and return true
    /// so the caller triggers a full resync + persists. A `None`/absent/empty stamp
    /// never triggers. ISO-8601 UTC stamps (`…Z`) sort lexically, so a plain string
    /// compare is correct for same-format timestamps; we also treat "never honored"
    /// (None) as older-than-anything so a first real stamp records a baseline. Purely
    /// guarded by `last_resync_honored`, so it's idempotent: the same stamp only ever
    /// triggers once, no matter how many responses carry it.
    pub fn honor_resync_signal(&mut self, stamp: Option<&str>) -> bool {
        let stamp = match stamp {
            Some(s) if !s.trim().is_empty() => s.trim(),
            _ => return false,
        };
        let is_newer = match &self.last_resync_honored {
            Some(prev) if !prev.trim().is_empty() => stamp > prev.trim(),
            _ => true, // nothing honored yet → any real stamp is "newer" (baseline)
        };
        if !is_newer {
            return false;
        }
        self.zero_all_cursors();
        self.last_resync_honored = Some(stamp.to_string());
        true
    }

    /// Purge ALL local sync/queue state — drop every `StreamState` so Uplink FORGETS its
    /// cursors and queue counters. The acute use: an ORPHANED stream (e.g. SBF's old
    /// `fishlog`, renamed to `events` in the session upgrade) leaves a `StreamState` that
    /// can never drain because the stream no longer exists in the SV — so its `queued`
    /// count is stuck forever. Purging clears it. Live streams simply re-discover on the
    /// next parse and re-send from the start; the server dedupes rows on
    /// (device·account·addon·stream·i), so re-sending is always safe/idempotent. Returns
    /// how many stream entries were dropped.
    pub fn purge_sync_state(&mut self) -> u64 {
        let n = self.sync.len() as u64;
        self.sync.clear();
        n
    }
}

/// The runtime state Tauri manages: the config plus where it lives on disk.
pub struct AppState {
    pub config: std::sync::Mutex<AppConfig>,
    pub config_path: PathBuf,
}

/// Are we running inside a Flatpak sandbox (the Steam Deck build)? True when the
/// `FLATPAK_ID` env var is set OR `/.flatpak-info` exists. The self-updater is
/// disabled in that case (read-only sandbox); addon file-installs still work with
/// `--filesystem=host` (spec §2).
pub fn is_flatpak() -> bool {
    std::env::var("FLATPAK_ID").is_ok() || Path::new("/.flatpak-info").exists()
}

impl AppState {
    /// Snapshot the config with runtime fields (online, paired, is_flatpak) freshly
    /// computed.
    pub fn snapshot(&self, paired: bool) -> AppConfig {
        let mut cfg = self.config.lock().unwrap().clone();
        cfg.paired = paired;
        cfg.is_flatpak = is_flatpak();
        for inst in cfg.installs.iter_mut() {
            inst.online = Path::new(&inst.path).is_dir();
        }
        cfg
    }

    pub fn persist(&self) -> Result<(), String> {
        let cfg = self.config.lock().unwrap();
        cfg.save(&self.config_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_cursor() -> AppConfig {
        let mut cfg = AppConfig::default();
        // seed a stream with a live cursor so we can prove zeroing happened
        let st = cfg.stream_state_mut("inst", "ACCT", "sbf", "fishlog");
        st.cursor = "42".to_string();
        st.total = 100;
        st.file_mtime = Some(1_700_000_000);
        cfg
    }

    #[test]
    fn resync_newer_stamp_triggers_and_records_honored() {
        let mut cfg = cfg_with_cursor();
        assert!(cfg.last_resync_honored.is_none());
        // first real stamp from a "never honored" baseline → triggers
        assert!(cfg.honor_resync_signal(Some("2026-07-13T10:00:00Z")));
        assert_eq!(cfg.last_resync_honored.as_deref(), Some("2026-07-13T10:00:00Z"));
        // and it zeroed the cursor (full-resync primitive)
        assert_eq!(cfg.cursor_of("inst", "ACCT", "sbf", "fishlog"), "");
        assert_eq!(cfg.sync[0].file_mtime, None);

        // a strictly-newer stamp triggers again and records the newer one
        cfg.stream_state_mut("inst", "ACCT", "sbf", "fishlog").cursor = "99".to_string();
        assert!(cfg.honor_resync_signal(Some("2026-07-13T12:30:00Z")));
        assert_eq!(cfg.last_resync_honored.as_deref(), Some("2026-07-13T12:30:00Z"));
        assert_eq!(cfg.cursor_of("inst", "ACCT", "sbf", "fishlog"), "");
    }

    #[test]
    fn resync_equal_or_older_or_null_does_not_trigger() {
        let mut cfg = cfg_with_cursor();
        cfg.last_resync_honored = Some("2026-07-13T12:00:00Z".to_string());

        // equal → no trigger, cursor untouched
        assert!(!cfg.honor_resync_signal(Some("2026-07-13T12:00:00Z")));
        assert_eq!(cfg.cursor_of("inst", "ACCT", "sbf", "fishlog"), "42");

        // older → no trigger
        assert!(!cfg.honor_resync_signal(Some("2026-07-13T09:00:00Z")));
        assert_eq!(cfg.cursor_of("inst", "ACCT", "sbf", "fishlog"), "42");

        // null / absent → no trigger
        assert!(!cfg.honor_resync_signal(None));
        // empty / whitespace → no trigger
        assert!(!cfg.honor_resync_signal(Some("")));
        assert!(!cfg.honor_resync_signal(Some("   ")));

        // honored is unchanged throughout
        assert_eq!(cfg.last_resync_honored.as_deref(), Some("2026-07-13T12:00:00Z"));
        assert_eq!(cfg.cursor_of("inst", "ACCT", "sbf", "fishlog"), "42");
    }

    #[test]
    fn self_update_defaults_apply_to_old_config() {
        // an old config file without the new keys still loads with defaults
        let cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.self_update_channel, "public");
        // "just works" posture (§3.5/§3.6): auto-update and start-in-tray default ON so
        // an older config missing the keys opts into them. (The global sync master was
        // removed in the sync-UX redesign — per-addon + per-account gates replace it.)
        assert!(cfg.auto_update_enabled);
        assert!(cfg.start_in_tray);
        assert_eq!(cfg.auto_update_time, "03:00");
        // redesign §3: a per-addon record missing the `sync` key defaults ON
        let addon: InstalledAddon = serde_json::from_str(r#"{"slug":"sbf"}"#).unwrap();
        assert!(addon.sync);
    }

    #[test]
    fn zero_all_cursors_counts_and_clears() {
        let mut cfg = cfg_with_cursor();
        assert_eq!(cfg.zero_all_cursors(), 1); // one non-empty cursor cleared
        assert_eq!(cfg.cursor_of("inst", "ACCT", "sbf", "fishlog"), "");
        assert_eq!(cfg.sync[0].queued, cfg.sync[0].total); // everything queued again
        // idempotent: nothing left to clear
        assert_eq!(cfg.zero_all_cursors(), 0);
    }

    #[test]
    fn addon_new_defaults_sync_on_and_gate_needs_enabled_and_sync() {
        // redesign §3: a freshly-created addon syncs by default
        let a = InstalledAddon::new("sbf");
        assert!(a.sync);
        assert!(!a.enabled); // but isn't "enabled" until installed/selected
        // the sweep gate is `enabled && sync` — prove the truth table the snapshot uses
        let gate = |enabled: bool, sync: bool| {
            let mut x = InstalledAddon::new("sbf");
            x.enabled = enabled;
            x.sync = sync;
            x.enabled && x.sync
        };
        assert!(gate(true, true));
        assert!(!gate(true, false)); // sync off → skipped
        assert!(!gate(false, true)); // not installed → skipped
        assert!(!gate(false, false));
    }

    #[test]
    fn purge_sync_state_drops_everything() {
        let mut cfg = cfg_with_cursor();
        assert_eq!(cfg.sync.len(), 1);
        assert_eq!(cfg.purge_sync_state(), 1); // one stream entry dropped
        assert!(cfg.sync.is_empty()); // all queue state forgotten
        // idempotent: nothing left to purge
        assert_eq!(cfg.purge_sync_state(), 0);
    }
}
