//! The Tauri command surface — everything the web UI can ask the core to do.
//! JS camelCase args map to these snake_case params automatically. Commands
//! never hold the config mutex across an `.await` (lock → read/clone → unlock →
//! network → lock → write).

use crate::auth;
use crate::catalog::{self, Catalog, PairRequest};
use crate::config::{Account, AppConfig, AppState, Install};
use crate::delivery;
use crate::installs::{self, DetectedInstall};
use crate::sync::{self, SyncResult};
use crate::version;
use serde::Serialize;
use tauri::{Emitter, State};
use uuid::Uuid;

type R<T> = Result<T, String>;

fn base_url(state: &State<AppState>) -> String {
    state.config.lock().unwrap().base_url.clone()
}

// ── config / preferences ──

#[tauri::command]
pub fn get_config(state: State<AppState>) -> AppConfig {
    state.snapshot(auth::has_token())
}

#[tauri::command]
pub fn set_theme(state: State<AppState>, theme: String) -> R<()> {
    state.config.lock().unwrap().theme = theme;
    state.persist()
}

#[tauri::command]
pub fn set_base_url(state: State<AppState>, url: String) -> R<()> {
    let url = url.trim().trim_end_matches('/').to_string();
    state.config.lock().unwrap().base_url =
        if url.is_empty() { crate::config::default_base_url() } else { url };
    state.persist()
}

#[tauri::command]
pub fn set_sync_while_running(state: State<AppState>, value: bool) -> R<()> {
    state.config.lock().unwrap().sync_while_running = value;
    state.persist()
}

#[tauri::command]
pub fn set_sync_interval(state: State<AppState>, secs: u64) -> R<()> {
    // clamp to a sane floor so a stray 0 doesn't busy-loop the backstop
    state.config.lock().unwrap().sync_interval_secs = secs.max(30);
    state.persist()
}

#[tauri::command]
pub fn set_self_update_channel(state: State<AppState>, channel: String) -> R<()> {
    let c = channel.trim();
    state.config.lock().unwrap().self_update_channel =
        if c.is_empty() { "public".to_string() } else { c.to_string() };
    state.persist()
}

#[tauri::command]
pub fn set_auto_update(state: State<AppState>, enabled: bool) -> R<()> {
    state.config.lock().unwrap().auto_update_enabled = enabled;
    state.persist()
}

#[tauri::command]
pub fn set_auto_update_time(state: State<AppState>, time: String) -> R<()> {
    // validate HH:MM; invalid input resets to the 03:00 default
    let t = if crate::schedule::parse_hhmm(&time).is_some() {
        time
    } else {
        "03:00".to_string()
    };
    state.config.lock().unwrap().auto_update_time = t;
    state.persist()
}


// ── startup: launch-at-login + start-in-tray ──

/// Enable/disable launching Uplink at login. The login-item state is owned by the
/// OS (via the autostart plugin's LaunchAgent/registry entry), NOT persisted in
/// config — query it live with `get_launch_at_login`. The item is registered with
/// a `--hidden` arg (see lib.rs) so login launches are quiet.
#[tauri::command]
pub fn set_launch_at_login(app: tauri::AppHandle, enabled: bool) -> R<()> {
    use tauri_plugin_autostart::ManagerExt;
    if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|e| e.to_string())
}

/// Live query of whether the OS login item is currently registered.
#[tauri::command]
pub fn get_launch_at_login(app: tauri::AppHandle) -> R<bool> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Persist "start in the menu bar" (launch to tray only, no main window). Read at
/// launch in lib.rs setup to decide whether to show the window.
#[tauri::command]
pub fn set_start_in_tray(state: State<AppState>, enabled: bool) -> R<()> {
    state.config.lock().unwrap().start_in_tray = enabled;
    state.persist()
}

#[tauri::command]
pub fn mark_onboarded(app: tauri::AppHandle, state: State<AppState>) -> R<()> {
    // Only the genuine first-run false→true transition auto-enables launch-at-login;
    // re-entering the wizard later (already onboarded) must NOT re-toggle it (§3.10).
    let was_onboarded = {
        let mut cfg = state.config.lock().unwrap();
        let was = cfg.onboarded;
        cfg.onboarded = true;
        was
    };
    state.persist()?;
    if !was_onboarded {
        // First-run only: quietly enable launch-at-login so Uplink is ready next boot.
        // Best-effort — ignore errors (a platform/plugin may refuse); this is the ONLY
        // place autolaunch is auto-enabled.
        use tauri_plugin_autostart::ManagerExt;
        let _ = app.autolaunch().enable();
    }
    Ok(())
}

/// Per-account DISPLAY alias (redesign §4). `account` is the real WoW account folder
/// name (the identity + sync/wire key — never changed); `alias` is the label shown in
/// the UI. Trims; an empty alias CLEARS the entry (falls back to the real name). This
/// only touches `account_aliases`; `sync_accounts` and every cursor key are untouched.
#[tauri::command]
pub fn set_account_alias(state: State<AppState>, account: String, alias: String) -> R<()> {
    let account = account.trim().to_string();
    if account.is_empty() {
        return Err("empty account".to_string());
    }
    let alias = alias.trim().to_string();
    {
        let mut cfg = state.config.lock().unwrap();
        if alias.is_empty() {
            cfg.account_aliases.remove(&account);
        } else {
            cfg.account_aliases.insert(account, alias);
        }
    }
    state.persist()
}

#[tauri::command]
pub fn dismiss_broadcast(state: State<AppState>, id: i64) -> R<()> {
    state.config.lock().unwrap().dismissed_broadcast = Some(id);
    state.persist()
}

// ── auth ──

#[tauri::command]
pub async fn pair_device(state: State<'_, AppState>, code: String) -> R<Account> {
    let base = base_url(&state);
    let req = PairRequest {
        code: code.trim().to_uppercase(),
        // The device name is set on the website when the pairing code is created;
        // Uplink never collects it. Sending empty makes the server keep that name
        // (COALESCE(NULLIF('',''), name)); it returns the name for us to display.
        name: String::new(),
        platform: std::env::consts::OS.to_string(),
        app: "gec-uplink".to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let resp: catalog::PairResponse =
        crate::net::post_json(&base, "/api/devices/pair", &req, false).await?;

    auth::store_token(&resp.token)?;
    let account = Account {
        handle: resp.account.handle,
        tier: resp.account.tier,
        role: resp.account.role,
        channels: resp.account.channels,
    };
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.device_id = Some(resp.device_id);
        // Device name comes from the server (set on the website), not the client.
        if let Some(n) = resp.name.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            cfg.device_name = Some(n.to_string());
        }
        cfg.account = Some(account.clone());
    }
    state.persist()?;
    Ok(account)
}

#[tauri::command]
pub fn unpair(state: State<AppState>) -> R<()> {
    auth::clear_token()?;
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.account = None;
        cfg.device_id = None;
    }
    state.persist()
}

// ── installs ──

#[tauri::command]
pub fn detect_installs(state: State<AppState>) -> Vec<DetectedInstall> {
    let cfg = state.config.lock().unwrap();
    // Label installs by the paired DEVICE name (unique per machine) rather than
    // the system hostname — two machines can share a hostname.
    let label = cfg
        .device_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(installs::machine_label);
    installs::detect(&cfg.installs, &label)
}

/// Enumerate every WoW flavor under a user-browsed folder (the install-discovery
/// reframe): the user points at their World of Warcraft folder and we surface every
/// Retail/Classic/PTR/… install beneath it as an addable row. Non-destructive — this
/// only reads the disk; nothing is added until the user clicks Add (`add_install`).
#[tauri::command]
pub fn enumerate_installs_at(state: State<AppState>, path: String) -> R<Vec<DetectedInstall>> {
    let cfg = state.config.lock().unwrap();
    let label = cfg
        .device_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(installs::machine_label);
    Ok(installs::enumerate_picked(path.trim(), &cfg.installs, &label))
}

#[tauri::command]
pub fn add_install(state: State<AppState>, path: String, label: String) -> R<Install> {
    let resolved = installs::resolve_addons_path(path.trim())?;
    // Store the REAL path (resolve symlinks) so a browsed path de-dupes against an
    // auto-detected one and the install-.dmg /Applications symlink can't double it.
    let addons_path = std::fs::canonicalize(&resolved)
        .map(|r| r.to_string_lossy().to_string())
        .unwrap_or(resolved);
    let norm = addons_path.trim_end_matches(['/', '\\']).to_lowercase();
    let mut cfg = state.config.lock().unwrap();
    if let Some(existing) = cfg
        .installs
        .iter()
        .find(|i| i.path.trim_end_matches(['/', '\\']).to_lowercase() == norm)
    {
        return Ok(existing.clone());
    }
    let flavor = installs::flavor_from_path(&addons_path);
    let label = if label.trim().is_empty() {
        cfg.device_name.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(installs::machine_label)
    } else {
        label.trim().to_string()
    };
    let mut inst = Install {
        id: Uuid::new_v4().to_string(),
        label,
        path: addons_path.clone(),
        flavor,
        channel: "public".to_string(),
        // New installs default to auto-updating their addons (§3.7) — the "just works"
        // posture. The user can turn it off per-install in Settings.
        auto_update: true,
        sync_accounts: Vec::new(),
        // Not yet initialized (redesign §2): the first account discovery defaults this
        // install to syncing ALL its accounts, then flips this true.
        accounts_initialized: false,
        addons: Vec::new(),
        online: std::path::Path::new(&addons_path).is_dir(),
    };
    inst.online = std::path::Path::new(&addons_path).is_dir();
    cfg.installs.push(inst.clone());
    drop(cfg);
    state.persist()?;
    Ok(inst)
}

#[tauri::command]
pub fn remove_install(state: State<AppState>, id: String) -> R<()> {
    state.config.lock().unwrap().installs.retain(|i| i.id != id);
    state.persist()
}

#[tauri::command]
pub fn update_install(
    state: State<AppState>,
    id: String,
    label: String,
    channel: String,
    auto_update: bool,
) -> R<()> {
    {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&id).ok_or("unknown install")?;
        if !label.trim().is_empty() {
            inst.label = label.trim().to_string();
        }
        inst.channel = channel;
        inst.auto_update = auto_update;
    }
    state.persist()
}

#[tauri::command]
pub fn set_addon_selected(state: State<AppState>, install_id: String, slug: String, enabled: bool) -> R<()> {
    {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&install_id).ok_or("unknown install")?;
        // New addons inherit the install's auto_update DEFAULT when first enabled
        // (spec §3b). Only fresh records adopt it; an existing addon keeps its own
        // per-addon flag (which may already differ from the default).
        let default_auto = inst.auto_update;
        let is_new = !inst.addons.iter().any(|a| a.slug == slug);
        let a = inst.addon_mut(&slug);
        a.enabled = enabled;
        if enabled && is_new {
            a.auto_update = default_auto;
        }
    }
    state.persist()
}

/// Per-addon auto-update toggle (spec §3b) — the SOURCE OF TRUTH the scheduler reads.
#[tauri::command]
pub fn set_addon_auto_update(state: State<AppState>, install_id: String, slug: String, enabled: bool) -> R<()> {
    {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&install_id).ok_or("unknown install")?;
        inst.addon_mut(&slug).auto_update = enabled;
    }
    state.persist()
}

/// Per-addon data-sync toggle (redesign §3) — one of the two sync gates (the other is
/// the account selection). When off, the sweep skips this addon's streams entirely.
#[tauri::command]
pub fn set_addon_sync(state: State<AppState>, install_id: String, slug: String, on: bool) -> R<()> {
    {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&install_id).ok_or("unknown install")?;
        inst.addon_mut(&slug).sync = on;
    }
    state.persist()
}

/// The install-row auto-update checkbox (spec §3b): it's BOTH the default for
/// newly-enabled addons AND a bulk action — set the install default and flip every
/// current addon's per-addon flag to match.
#[tauri::command]
pub fn set_install_auto_update_all(state: State<AppState>, install_id: String, enabled: bool) -> R<()> {
    {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&install_id).ok_or("unknown install")?;
        inst.auto_update = enabled;
        for a in inst.addons.iter_mut() {
            a.auto_update = enabled;
        }
    }
    state.persist()
}

/// "Reset addons" (spec §3b): return every addon in this install to the install
/// defaults — each addon's `auto_update` back to the install default and its
/// `channel_override` cleared (so it follows the install channel again).
#[tauri::command]
pub fn reset_addons(state: State<AppState>, install_id: String) -> R<()> {
    {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&install_id).ok_or("unknown install")?;
        let default_auto = inst.auto_update;
        for a in inst.addons.iter_mut() {
            a.auto_update = default_auto;
            a.channel_override = None;
        }
    }
    state.persist()
}

#[tauri::command]
pub fn set_addon_version(
    state: State<AppState>,
    install_id: String,
    slug: String,
    pinned_version: Option<String>,
    channel_override: Option<String>,
) -> R<()> {
    {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&install_id).ok_or("unknown install")?;
        let a = inst.addon_mut(&slug);
        a.pinned_version = pinned_version.filter(|s| !s.is_empty());
        a.channel_override = channel_override.filter(|s| !s.is_empty());
    }
    state.persist()
}

// ── catalog + delivery ──

#[tauri::command]
pub async fn fetch_catalog(state: State<'_, AppState>) -> R<Catalog> {
    let base = base_url(&state);
    catalog::fetch_catalog(&base).await
}

#[tauri::command]
pub async fn fetch_home(app: tauri::AppHandle, state: State<'_, AppState>) -> R<serde_json::Value> {
    let base = base_url(&state);
    let me = catalog::fetch_home(&base).await?;
    let mut changed = false;
    // Set true when stamps.catalog moved, so we emit the re-fetch event AFTER
    // unlocking (never hold the mutex across app.emit).
    let mut catalog_changed = false;
    // Refresh the paired account snapshot (role / channels / tier) from /api/me so a
    // role change or a newly-granted channel shows up WITHOUT re-pairing — the account
    // is otherwise only captured at pair time and goes stale. Only refresh when we're
    // already paired (an anonymous /api/me has no `handle`); never fabricate an account.
    if let Some(handle) = me.get("handle").and_then(|v| v.as_str()) {
        let refreshed = Account {
            handle: handle.to_string(),
            tier: me.get("tier").and_then(|v| v.as_str()).map(|s| s.to_string()),
            role: me.get("role").and_then(|v| v.as_str()).unwrap_or("user").to_string(),
            channels: me
                .get("channels")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default(),
        };
        let mut cfg = state.config.lock().unwrap();
        if cfg.account.is_some() && cfg.account.as_ref() != Some(&refreshed) {
            cfg.account = Some(refreshed);
            changed = true;
        }
    }
    // Keep the DEVICE NAME in sync the same way: the server is the authority
    // (set/renamed on the website's device page); /api/me echoes the caller's
    // own device row, so a rename propagates on the next heartbeat instead of
    // requiring a re-pair. Install labels derive from this name.
    if let Some(dev_name) = me
        .get("device")
        .and_then(|d| d.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let mut cfg = state.config.lock().unwrap();
        if cfg.device_name.as_deref() != Some(dev_name) {
            cfg.device_name = Some(dev_name.to_string());
            changed = true;
        }
    }
    // Honor the server's resync signal (contract §5b): /api/me carries a top-level
    // `resync_requested_at`. If it's newer than the last honored, zero every cursor
    // now; the background sync loop's next pass does the full resync (cursors are
    // zeroed). This is the path that catches a purge even when the user never opens
    // the Data page and there's no new addon data to POST (so no ingest ack fires).
    let stamp = me.get("resync_requested_at").and_then(|v| v.as_str()).map(|s| s.to_string());
    if state.config.lock().unwrap().honor_resync_signal(stamp.as_deref()) {
        changed = true;
    }
    // Heavy-state dirty stamp (contract §4.3): /api/me carries `stamps.catalog`, an
    // opaque 16-hex fingerprint of THIS account's entitled catalog view. Compare it
    // (never parse) against the cached value; when it moves, cache the new one and
    // signal the front-end to re-fetch the gec.addons/2 catalog. Unknown `stamps.*`
    // keys are ignored (additions are non-breaking by contract).
    if let Some(cat_stamp) = me.get("stamps").and_then(|s| s.get("catalog")).and_then(|v| v.as_str()) {
        let mut cfg = state.config.lock().unwrap();
        if cfg.catalog_stamp.as_deref() != Some(cat_stamp) {
            cfg.catalog_stamp = Some(cat_stamp.to_string());
            changed = true;
            catalog_changed = true;
        }
    }
    if changed {
        state.persist()?;
    }
    // Emit AFTER dropping every lock (the blocks above all unlock at their `}`) so
    // the event listener can't deadlock against the config mutex.
    if catalog_changed {
        let _ = app.emit("uplink:catalog-changed", ());
    }
    Ok(me)
}

/// Reconcile config against what's actually on disk: for every catalog addon,
/// read the installed version from its `.toc` in each online install's AddOns
/// folder. This makes addons installed by ANY means (CurseForge, manual, or a
/// prior Uplink) show as installed with their real version, so update checks and
/// telemetry work even when Uplink didn't do the install. Returns the catalog so
/// the caller gets both in one round-trip.
#[tauri::command]
pub async fn reconcile_installed(state: State<'_, AppState>) -> R<Catalog> {
    let base = base_url(&state);
    let cat = catalog::fetch_catalog(&base).await?;
    let slugs: Vec<String> = cat.addons.iter().map(|a| a.slug.clone()).collect();
    {
        let mut cfg = state.config.lock().unwrap();
        for inst in cfg.installs.iter_mut() {
            if !std::path::Path::new(&inst.path).is_dir() {
                continue; // offline install — leave its recorded state alone
            }
            for slug in &slugs {
                let folder = delivery::folder_guess(slug);
                match delivery::installed_version(&inst.path, &folder) {
                    // present on disk → adopt it (enable + record the real version)
                    Some(ver) => {
                        let a = inst.addon_mut(slug);
                        a.installed_version = Some(ver);
                        a.enabled = true;
                    }
                    // gone from disk → clear a stale version we recorded before
                    None => {
                        if let Some(a) = inst.addons.iter_mut().find(|a| a.slug == *slug) {
                            a.installed_version = None;
                        }
                    }
                }
            }
        }
        // Prune rename-orphan records (record-only; never touches disk — folder_guess
        // redirects an old slug onto the LIVE new folder). Computed against the full
        // config, then applied; the empty-catalog safety floor lives in compute_orphans.
        let orphans = compute_orphans(&cfg, &cat);
        if !orphans.is_empty() {
            for inst in cfg.installs.iter_mut() {
                inst.addons
                    .retain(|a| !orphans.iter().any(|(id, slug)| *id == inst.id && *slug == a.slug));
            }
        }
    }
    state.persist()?;
    Ok(cat)
}

#[tauri::command]
pub async fn install_addon(
    state: State<'_, AppState>,
    install_id: String,
    slug: String,
    version: String,
    url: String,
    channel: String,
) -> R<String> {
    let addons_dir = {
        let cfg = state.config.lock().unwrap();
        cfg.installs
            .iter()
            .find(|i| i.id == install_id)
            .map(|i| i.path.clone())
            .ok_or("unknown install")?
    };

    // Rebuild a server download URL against our base_url in case the catalog baked
    // in a wrong host (e.g. a dev localhost from a mis-pointed publish).
    let base = base_url(&state);
    let url = crate::net::resolve_download_url(&url, &base);
    let outcome = delivery::download_and_install(&url, &addons_dir, &base).await?;
    let folder = outcome
        .folders
        .first()
        .cloned()
        .unwrap_or_else(|| delivery::folder_guess(&slug));

    {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&install_id).ok_or("unknown install")?;
        let a = inst.addon_mut(&slug);
        a.enabled = true;
        a.installed_version = Some(version.clone());
        a.folder = Some(folder.clone());
        let _ = channel; // channel is captured in per-addon settings already
    }
    state.persist()?;
    Ok(format!("Installed {folder} {version} ({} files)", outcome.files))
}

#[tauri::command]
pub fn uninstall_addon(state: State<AppState>, install_id: String, slug: String) -> R<()> {
    let (addons_dir, folder) = {
        let cfg = state.config.lock().unwrap();
        let inst = cfg.installs.iter().find(|i| i.id == install_id).ok_or("unknown install")?;
        let folder = inst
            .addons
            .iter()
            .find(|a| a.slug == slug)
            .and_then(|a| a.folder.clone())
            .unwrap_or_else(|| delivery::folder_guess(&slug));
        (inst.path.clone(), folder)
    };
    delivery::uninstall(&addons_dir, &folder)?;
    {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&install_id).ok_or("unknown install")?;
        let a = inst.addon_mut(&slug);
        a.enabled = false;
        a.installed_version = None;
        a.folder = None;
    }
    state.persist()
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateItem {
    pub install_id: String,
    pub slug: String,
    pub installed: Option<String>,
    pub latest: String,
    pub url: String,
    pub channel: String,
}

/// Resolve, per install+addon, the release the addon *should* be on (pinned
/// version if set, else the channel's latest) and whether it's behind.
pub(crate) fn compute_updates(cfg: &AppConfig, cat: &Catalog) -> Vec<UpdateItem> {
    let mut out = Vec::new();
    for inst in &cfg.installs {
        if !std::path::Path::new(&inst.path).is_dir() {
            continue; // offline install — don't offer updates
        }
        for a in inst.addons.iter().filter(|a| a.enabled) {
            let Some(def) = cat.addon(&a.slug) else { continue };
            if !def.supports_flavor(&inst.flavor) { continue; } // parity with compute_reconcile: never offer an update on a flavor the addon doesn't support (e.g. a Retail-only addon on a Classic install)
            let chan = inst.channel_for(&a.slug);
            let target = match &a.pinned_version {
                Some(v) => def.release(&chan, v).map(|r| (r.version.clone(), r.url.clone())),
                None => def.latest_for(&chan).map(|l| (l.version.clone(), l.url.clone())),
            };
            let Some((tv, turl)) = target else { continue };
            let installed = a.installed_version.clone();
            let behind = match &installed {
                Some(cur) => version::is_newer(&tv, cur),
                None => false, // not installed = not an "update" (that's an install)
            };
            if behind {
                out.push(UpdateItem {
                    install_id: inst.id.clone(),
                    slug: a.slug.clone(),
                    installed,
                    latest: tv,
                    url: turl,
                    channel: chan,
                });
            }
        }
    }
    out
}

/// Config addon records the catalog no longer knows AND that aren't a standalone
/// on-disk install — leftovers from a rename (`towncryer`→`megaphone`,
/// `console`→`gec-console`). Returns `(install_id, slug)` pairs to DROP.
///
/// RECORD-ONLY by design: the caller removes the config record and NEVER touches disk.
/// `folder_guess` deliberately redirects an old slug onto the LIVE new folder
/// (`towncryer`→`Megaphone`), so uninstalling a ghost would delete the wrong addon.
///
/// A record is an orphan when its slug is absent from the catalog and it is not a
/// genuine standalone install — i.e. it has no `folder`, OR its `folder` is one a
/// STILL-CATALOGED record on the same install also claims (a rename shadow). An addon
/// that is truly installed under its own unique folder but merely absent from this
/// entitlement-scoped catalog view is PRESERVED. Safety floor: an empty catalog (a
/// failed fetch) prunes nothing, so a hiccup can never wipe every record.
pub(crate) fn compute_orphans(cfg: &AppConfig, cat: &Catalog) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if cat.addons.is_empty() {
        return out;
    }
    for inst in &cfg.installs {
        // folders owned by a record whose slug the catalog STILL recognizes
        let known_folders: std::collections::HashSet<&str> = inst
            .addons
            .iter()
            .filter(|a| cat.addon(&a.slug).is_some())
            .filter_map(|a| a.folder.as_deref())
            .collect();
        for a in &inst.addons {
            if cat.addon(&a.slug).is_some() {
                continue; // still a real addon
            }
            let standalone = match a.folder.as_deref() {
                Some(f) => !known_folders.contains(f), // owns a folder no live addon claims
                None => false,                         // never installed under its own name
            };
            if !standalone {
                out.push((inst.id.clone(), a.slug.clone()));
            }
        }
    }
    out
}

#[tauri::command]
pub async fn check_updates(state: State<'_, AppState>) -> R<Vec<UpdateItem>> {
    let base = base_url(&state);
    let cat = catalog::fetch_catalog(&base).await?;
    let cfg = state.config.lock().unwrap().clone();
    Ok(compute_updates(&cfg, &cat))
}

#[tauri::command]
pub async fn update_all(state: State<'_, AppState>, install_id: String) -> R<Vec<String>> {
    let base = base_url(&state);
    let cat = catalog::fetch_catalog(&base).await?;
    let items: Vec<UpdateItem> = {
        let cfg = state.config.lock().unwrap().clone();
        compute_updates(&cfg, &cat)
            .into_iter()
            .filter(|u| u.install_id == install_id)
            .collect()
    };
    let mut done = Vec::new();
    for u in items {
        // reuse install_addon logic inline (can't call a command from a command)
        let addons_dir = {
            let cfg = state.config.lock().unwrap();
            cfg.installs.iter().find(|i| i.id == u.install_id).map(|i| i.path.clone())
        };
        let Some(addons_dir) = addons_dir else { continue };
        let dl = crate::net::resolve_download_url(&u.url, &base);
        match delivery::download_and_install(&dl, &addons_dir, &base).await {
            Ok(outcome) => {
                let folder = outcome.folders.first().cloned().unwrap_or_else(|| delivery::folder_guess(&u.slug));
                {
                    let mut cfg = state.config.lock().unwrap();
                    if let Some(inst) = cfg.install_mut(&u.install_id) {
                        let a = inst.addon_mut(&u.slug);
                        a.installed_version = Some(u.latest.clone());
                        a.folder = Some(folder.clone());
                    }
                }
                done.push(format!("{folder} → {}", u.latest));
            }
            Err(e) => done.push(format!("{}: FAILED ({e})", u.slug)),
        }
    }
    state.persist()?;
    Ok(done)
}

/// Like `compute_updates` but DIRECTION-AGNOSTIC: any enabled+installed addon
/// whose on-disk version DIFFERS from its effective-channel target (older OR
/// newer). This is what makes a channel switch actually move the addon — e.g.
/// dev → public installs the (lower-numbered) public build over the dev one.
/// Kept separate from `compute_updates` so the SCHEDULED auto-update stays
/// upgrade-only and never surprise-downgrades.
fn compute_reconcile(cfg: &AppConfig, cat: &Catalog, install_id: &str) -> Vec<UpdateItem> {
    let mut out = Vec::new();
    let Some(inst) = cfg.installs.iter().find(|i| i.id == install_id) else { return out };
    if !std::path::Path::new(&inst.path).is_dir() {
        return out; // offline — can't reach the AddOns folder
    }
    for a in inst.addons.iter().filter(|a| a.enabled) {
        let Some(def) = cat.addon(&a.slug) else { continue };
        if !def.supports_flavor(&inst.flavor) { continue; }   // flavor gate: don't offer/install an addon on a flavor it doesn't support (e.g. SBF on Classic)
        let chan = inst.channel_for(&a.slug);
        let target = match &a.pinned_version {
            Some(v) => def.release(&chan, v).map(|r| (r.version.clone(), r.url.clone())),
            None => def.latest_for(&chan).map(|l| (l.version.clone(), l.url.clone())),
        };
        let Some((tv, turl)) = target else { continue }; // no release on that channel → leave it
        let Some(cur) = a.installed_version.clone() else { continue }; // not installed → not a switch
        if cur != tv {
            out.push(UpdateItem {
                install_id: inst.id.clone(),
                slug: a.slug.clone(),
                installed: Some(cur),
                latest: tv,
                url: turl,
                channel: chan,
            });
        }
    }
    out
}

/// Reconcile every installed addon in an install to its effective-channel target
/// (any direction). Call after a channel change (per-addon override, pin, or the
/// install default). SavedVariables live in WTF/, so a downgrade keeps user data.
#[tauri::command]
pub async fn reconcile_install(state: State<'_, AppState>, install_id: String) -> R<Vec<String>> {
    let base = base_url(&state);
    let cat = catalog::fetch_catalog(&base).await?;
    let items: Vec<UpdateItem> = {
        let cfg = state.config.lock().unwrap().clone();
        compute_reconcile(&cfg, &cat, &install_id)
    };
    let mut done = Vec::new();
    for u in items {
        let addons_dir = {
            let cfg = state.config.lock().unwrap();
            cfg.installs.iter().find(|i| i.id == u.install_id).map(|i| i.path.clone())
        };
        let Some(addons_dir) = addons_dir else { continue };
        let dl = crate::net::resolve_download_url(&u.url, &base);
        match delivery::download_and_install(&dl, &addons_dir, &base).await {
            Ok(outcome) => {
                let folder = outcome.folders.first().cloned().unwrap_or_else(|| delivery::folder_guess(&u.slug));
                {
                    let mut cfg = state.config.lock().unwrap();
                    if let Some(inst) = cfg.install_mut(&u.install_id) {
                        let a = inst.addon_mut(&u.slug);
                        a.installed_version = Some(u.latest.clone());
                        a.folder = Some(folder.clone());
                    }
                }
                done.push(format!("{folder} → {}", u.latest));
            }
            Err(e) => done.push(format!("{}: FAILED ({e})", u.slug)),
        }
    }
    state.persist()?;
    Ok(done)
}

// ── telemetry: report installs to the server ──
//
// Canonical install report: POST /api/installs, schema gec.installs/1 (Uplink
// spec §9.6). Sent after any install/update/uninstall and on launch. The server
// applies SNAPSHOT-PER-INSTALL semantics: every install we list has its server
// record REPLACED by exactly what we send (so uninstalls just vanish), and any
// install we DON'T list is left untouched. Therefore we report only installs
// that are currently ONLINE (their path exists) — an unplugged/offline install
// is deliberately omitted so its record is never wiped. Vocabulary lines up
// with the catalog (§9.2): addon = catalog slug, channel = channel slug.
//
// The `installs`/`addons_installed` riders on /api/ingest still exist for data
// sync (§9.3), but Phase 1 delivery uses this endpoint only — never ingest.

#[derive(Serialize)]
struct InstallAddon {
    name: String,
    version: String,
    channel: String,
}
#[derive(Serialize)]
struct InstallEntry {
    install: String,
    flavor: String,
    addons: Vec<InstallAddon>,
}
#[derive(Serialize)]
struct InstallsReport {
    schema: &'static str,
    installs: Vec<InstallEntry>,
}

#[tauri::command]
pub async fn report_installs(state: State<'_, AppState>) -> R<()> {
    if !auth::has_token() {
        return Ok(()); // nothing to report without a paired device
    }
    let base = base_url(&state);
    let entries: Vec<InstallEntry> = {
        let cfg = state.config.lock().unwrap();
        cfg.installs
            .iter()
            // ONLINE only: computed fresh here so the snapshot is accurate at
            // send time regardless of when config.online was last refreshed.
            // Offline installs are skipped so they aren't wiped server-side.
            .filter(|inst| std::path::Path::new(&inst.path).is_dir())
            .map(|inst| InstallEntry {
                install: format!("{} · {}", inst.label, inst.flavor),
                flavor: installs::flavor_token_from_path(&inst.path),
                // A full snapshot of THIS install's addons. May be empty — that
                // is how a removed addon clears from the server's record.
                addons: inst
                    .addons
                    .iter()
                    .filter(|a| a.installed_version.is_some())
                    .map(|a| InstallAddon {
                        name: a.slug.clone(),
                        version: a.installed_version.clone().unwrap_or_default(),
                        channel: inst.channel_for(&a.slug),
                    })
                    .collect(),
            })
            .collect()
    };
    // No online installs → nothing to snapshot. Skipping is safe: an empty list
    // would wipe nothing, and never listing offline installs preserves them.
    if entries.is_empty() {
        return Ok(());
    }
    let report = InstallsReport { schema: "gec.installs/1", installs: entries };
    // ack echoes {ok, installs, addons}; we only need it to be accepted (2xx).
    let _ack: serde_json::Value = crate::net::post_json(&base, "/api/installs", &report, true).await?;
    Ok(())
}

// ── data sync (spec §6 / §9.3) ──

/// Manual "Sync now" — always runs (ignores the while-running gate + mtime
/// short-circuit), sweeps every online install's account SVs, uploads, advances
/// cursors from the ack. Returns per-stream results for the UI to surface.
#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>) -> R<Vec<SyncResult>> {
    sync::sync_all(&state, true).await
}

/// The cached sync counters (cursor / queued / last-sync / last-accepted) per
/// install·account·addon·stream — read straight from persisted config so the Data
/// and tray views render without re-parsing a multi-MB SV.
#[tauri::command]
pub fn get_sync_status(state: State<AppState>) -> Vec<crate::config::StreamState> {
    state.config.lock().unwrap().sync.clone()
}

/// Full resync: zero EVERY local stream cursor so the next sync re-sends all data
/// from the start. The server dedupes rows on (device·account·addon·stream·i), so
/// re-sending everything is safe and idempotent — no double counting, contribution
/// XP bumps only on genuinely-new rows. Returns how many cursors were cleared.
/// (Only the cursor is zeroed; totals/last-sync are refreshed on the next pass.)
#[tauri::command]
pub fn reset_cursors(state: State<AppState>) -> R<u64> {
    let cleared = state.config.lock().unwrap().zero_all_cursors();
    state.persist()?;
    Ok(cleared)
}

/// Purge ALL local sync/queue state — forget every stream cursor + queue counter. Clears a
/// STUCK/ORPHANED queue: e.g. SBF's old `fishlog` stream (renamed to `events` in the session
/// upgrade) leaves a StreamState whose `queued` can never drain because the stream is gone
/// from the SV. Live streams re-discover + re-send on the next pass; the server dedupes on
/// (device·account·addon·stream·i), so re-sending is safe. Returns how many entries dropped.
#[tauri::command]
pub fn purge_sync_queue(state: State<AppState>) -> R<u64> {
    let dropped = state.config.lock().unwrap().purge_sync_state();
    state.persist()?;
    Ok(dropped)
}

/// One WoW account folder for an install + whether it's selected for sync.
#[derive(Debug, Clone, Serialize)]
pub struct SyncAccount {
    pub account: String,
    pub selected: bool,
}

/// Discover the WoW account folders for an install and report each with its selected
/// state (redesign §2). On a FRESH install (never initialized) with an empty
/// selection, default to syncing ALL discovered accounts and flip
/// `accounts_initialized` — the "just works" default. Thereafter the stored selection
/// is honored verbatim: an empty selection means "sync nothing" and is NOT re-defaulted
/// (the old single-account auto-reselect that fought the user's uncheck is gone). A
/// machine's account folders can belong to different people, so this is how the user
/// controls exactly whose data uploads.
#[tauri::command]
pub fn list_sync_accounts(state: State<AppState>, install_id: String) -> R<Vec<SyncAccount>> {
    let (discovered, effective) = {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&install_id).ok_or("unknown install")?;
        let discovered = sync::discover_accounts(&inst.path);
        // one-time fresh-install default: sync every account we found
        if !inst.accounts_initialized && inst.sync_accounts.is_empty() && !discovered.is_empty() {
            inst.sync_accounts = discovered.clone();
            inst.accounts_initialized = true;
        }
        let effective = sync::effective_accounts(&inst.sync_accounts, &discovered);
        (discovered, effective)
    };
    state.persist()?;
    Ok(discovered
        .into_iter()
        .map(|a| SyncAccount { selected: effective.contains(&a), account: a })
        .collect())
}

/// Persist the user's chosen sync accounts for an install (checkbox toggles). The
/// selection is stored verbatim (deduped); the sweep filters to still-present folders
/// via `effective_accounts`, so a stale name here is harmless. Also marks the install
/// `accounts_initialized` so a DELIBERATE empty selection (uncheck-all) is respected
/// and never re-defaulted to all-on (redesign §2).
#[tauri::command]
pub fn set_sync_accounts(state: State<AppState>, install_id: String, accounts: Vec<String>) -> R<()> {
    {
        let mut cfg = state.config.lock().unwrap();
        let inst = cfg.install_mut(&install_id).ok_or("unknown install")?;
        let mut seen = std::collections::HashSet::new();
        inst.sync_accounts = accounts
            .into_iter()
            .filter(|a| !a.is_empty() && seen.insert(a.clone()))
            .collect();
        inst.accounts_initialized = true;
    }
    state.persist()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cat(slugs: &[&str]) -> Catalog {
        let addons: Vec<_> = slugs.iter().map(|s| json!({ "slug": s, "name": s })).collect();
        serde_json::from_value(json!({ "addons": addons })).unwrap()
    }
    fn cfg_with(addons: serde_json::Value) -> AppConfig {
        let mut c = AppConfig::default();
        c.installs = vec![serde_json::from_value(json!({
            "id": "i1", "label": "PC", "path": "/x", "flavor": "Retail", "addons": addons
        }))
        .unwrap()];
        c
    }

    // towncryer (renamed → megaphone) lingers with folder=None; megaphone is live → prune the ghost only.
    #[test]
    fn prunes_rename_orphan_with_no_folder() {
        let cfg = cfg_with(json!([
            {"slug":"megaphone","installed_version":"2026.07.16.1","folder":"Megaphone"},
            {"slug":"towncryer","installed_version":"0.1.3","folder":null}
        ]));
        assert_eq!(
            compute_orphans(&cfg, &cat(&["megaphone", "sbf"])),
            vec![("i1".to_string(), "towncryer".to_string())]
        );
    }

    // console (renamed → gec-console) points at the SAME on-disk folder as the live gec-console → prune it.
    #[test]
    fn prunes_orphan_shadowing_a_live_folder() {
        let cfg = cfg_with(json!([
            {"slug":"gec-console","installed_version":"2026.07.18.4","folder":"GEC-Console"},
            {"slug":"console","installed_version":"2026.07.18.4","folder":"GEC-Console"}
        ]));
        assert_eq!(
            compute_orphans(&cfg, &cat(&["gec-console"])),
            vec![("i1".to_string(), "console".to_string())]
        );
    }

    // a genuinely-installed addon merely absent from THIS entitlement-scoped catalog view
    // (own unique folder, shadows nothing) must be PRESERVED — never pruned.
    #[test]
    fn keeps_installed_addon_absent_from_catalog() {
        let cfg = cfg_with(json!([{"slug":"foo","installed_version":"1.2.3","folder":"Foo"}]));
        assert!(compute_orphans(&cfg, &cat(&["sbf", "haul"])).is_empty());
    }

    // safety floor: an empty/failed catalog fetch must not wipe every record.
    #[test]
    fn empty_catalog_prunes_nothing() {
        let cfg = cfg_with(json!([{"slug":"towncryer","folder":null}]));
        assert!(compute_orphans(&cfg, &cat(&[])).is_empty());
    }

    // an in-catalog addon is never an orphan.
    #[test]
    fn in_catalog_addon_never_pruned() {
        let cfg = cfg_with(json!([{"slug":"sbf","installed_version":"2026.07.25.2","folder":"SBF"}]));
        assert!(compute_orphans(&cfg, &cat(&["sbf"])).is_empty());
    }
}
