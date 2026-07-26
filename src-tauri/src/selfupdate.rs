//! App self-update (Uplink updating its own binary) + the daily auto-update
//! pass. The updater is built at RUNTIME so the channel (a setting) and the
//! keychain device token ride each request (spec §8). A 403 is detected via a
//! raw status probe first, because the Tauri updater collapses 403 into
//! ReleaseNotFound.

use crate::auth;
use crate::config::AppState;
use crate::net;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::UpdaterExt;

type R<T> = Result<T, String>;

/// Result of a self-update check, tagged for the JS side.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SelfUpdateStatus {
    UpToDate,
    Available { version: String, notes: Option<String> },
    /// The selected channel returned 403 (grant revoked / not entitled). We have
    /// already reset `self_update_channel` to "public"; `was` is the old slug.
    ChannelRevoked { was: String },
}

fn manifest_url(base: &str, channel: &str) -> String {
    format!(
        "{}/api/uplink/latest.json?channel={}",
        base.trim_end_matches('/'),
        channel
    )
}

/// The Tauri-updater endpoint: the channel manifest plus the `{{target}}` template
/// Tauri substitutes with the running platform (darwin-aarch64, windows-x86_64, …).
/// The server serves the newest build FOR that platform (per-target serving);
/// without it the manifest is one global-newest row, which a mac-only sidecar push
/// makes stale for other platforms (handoff v16 §3). The plain status probe must
/// NOT carry this — it isn't templated, so the server would see a literal
/// `{{target}}`.
fn updater_endpoint(base: &str, channel: &str) -> String {
    format!("{}{}", manifest_url(base, channel), "&target={{target}}")
}

/// Build a runtime updater pointed at `url`. When `attach_bearer` is true the
/// device token rides as `Authorization: Bearer …` (the gated server channel
/// path); the dev-override path passes false so a plain file server sees no
/// bearer. Signature verification against the committed pubkey applies either way.
fn build_updater(
    app: &AppHandle,
    url: &str,
    attach_bearer: bool,
) -> R<tauri_plugin_updater::Updater> {
    let ep: url::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    let mut b = app
        .updater_builder()
        .endpoints(vec![ep])
        .map_err(|e| e.to_string())?;
    if attach_bearer {
        if let Some(tok) = auth::get_token() {
            b = b
                .header("Authorization", format!("Bearer {tok}"))
                .map_err(|e| e.to_string())?;
        }
    }
    b.build().map_err(|e| e.to_string())
}

/// The dev-override URL if it's set and non-empty (trimmed). `Some` means the
/// self-update path bypasses the gated server channel entirely.
fn dev_override_url(state: &State<'_, AppState>) -> Option<String> {
    let cfg = state.config.lock().unwrap();
    cfg.dev_update_url
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[tauri::command]
pub async fn check_self_update(app: AppHandle, state: State<'_, AppState>) -> R<SelfUpdateStatus> {
    // DEV override: pull directly from the given URL — no channel query, no bearer,
    // no status probe (a plain file server has no 204/403 semantics). Signature
    // verification against the committed pubkey still guards the install.
    if let Some(dev_url) = dev_override_url(&state) {
        let updater = build_updater(&app, &dev_url, false)?;
        return match updater.check().await.map_err(|e| e.to_string())? {
            Some(update) => Ok(SelfUpdateStatus::Available {
                version: update.version.clone(),
                notes: update.body.clone(),
            }),
            None => Ok(SelfUpdateStatus::UpToDate),
        };
    }

    let (base, channel) = {
        let cfg = state.config.lock().unwrap();
        (cfg.base_url.clone(), cfg.self_update_channel.clone())
    };
    let url = manifest_url(&base, &channel);

    // Status probe first — read the real 200/204/403 (the updater hides 403).
    match net::get_status(&url).await? {
        403 => {
            if channel != "public" {
                {
                    let mut cfg = state.config.lock().unwrap();
                    cfg.self_update_channel = "public".to_string();
                }
                state.persist()?;
            }
            Ok(SelfUpdateStatus::ChannelRevoked { was: channel })
        }
        204 => Ok(SelfUpdateStatus::UpToDate),
        200 => {
            // Probe passed with channel-only `url`; the real fetch uses the
            // `{{target}}` endpoint for per-platform serving.
            let updater = build_updater(&app, &updater_endpoint(&base, &channel), true)?;
            match updater.check().await {
                Ok(Some(update)) => Ok(SelfUpdateStatus::Available {
                    version: update.version.clone(),
                    notes: update.body.clone(),
                }),
                Ok(None) => Ok(SelfUpdateStatus::UpToDate),
                // A manifest that omits THIS platform (e.g. a partial mirror that
                // registered only some OSes) makes the updater report the target
                // as missing. There's simply no build for this OS yet — that's not
                // a user-facing failure, so degrade to up-to-date instead of the
                // scary "couldn't check for updates" error.
                Err(e) => {
                    let m = e.to_string().to_lowercase();
                    if m.contains("target") || m.contains("platform") || m.contains("not found") {
                        Ok(SelfUpdateStatus::UpToDate)
                    } else {
                        Err(e.to_string())
                    }
                }
            }
        }
        other => Err(format!("update check failed (HTTP {other})")),
    }
}

#[tauri::command]
pub async fn install_self_update(app: AppHandle, state: State<'_, AppState>) -> R<()> {
    // DEV override: same direct endpoint as the check — no channel/bearer/probe.
    let (url, attach_bearer) = match dev_override_url(&state) {
        Some(dev_url) => (dev_url, false),
        None => {
            let (base, channel) = {
                let cfg = state.config.lock().unwrap();
                (cfg.base_url.clone(), cfg.self_update_channel.clone())
            };
            // `{{target}}` endpoint so the install fetches THIS platform's build.
            (updater_endpoint(&base, &channel), true)
        }
    };
    let updater = build_updater(&app, &url, attach_bearer)?;
    if let Some(update) = updater.check().await.map_err(|e| e.to_string())? {
        let app2 = app.clone();
        let mut got: usize = 0;
        update
            .download_and_install(
                move |chunk, total| {
                    got += chunk;
                    let pct = total.map(|t| ((got as f64 / t as f64) * 100.0).min(100.0) as u32);
                    let _ = app2.emit("uplink:update-progress", pct);
                },
                || {},
            )
            .await
            .map_err(|e| e.to_string())?;
        app.restart(); // diverges (!); never returns
    }
    Ok(())
}

/// Fire a best-effort native OS notification (never raises).
fn notify(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification().builder().title(title).body(body).show();
}

/// The daily auto-update pass (spec §9): update every addon flagged
/// `auto_update` (per-addon, spec §3b), then self-update the app (which relaunches
/// on success) — UNLESS this is a Flatpak build, whose read-only sandbox can't
/// self-update (spec §2; addon file-installs still run). Best-effort throughout.
pub async fn run_auto_update_pass(app: &AppHandle) {
    let enabled = {
        let state = app.state::<AppState>();
        let cfg = state.config.lock().unwrap();
        cfg.auto_update_enabled
    };
    if !enabled {
        return;
    }

    // 1) Addons first — the app self-update may restart the process. Per-addon:
    // compute the behind-updates, keep only those whose ADDON has auto_update set.
    let base = {
        let state = app.state::<AppState>();
        let cfg = state.config.lock().unwrap();
        cfg.base_url.clone()
    };
    if let Ok(cat) = crate::catalog::fetch_catalog(&base).await {
        let items: Vec<crate::commands::UpdateItem> = {
            let state = app.state::<AppState>();
            let cfg = state.config.lock().unwrap();
            crate::commands::compute_updates(&cfg, &cat)
                .into_iter()
                // keep only updates whose per-addon auto_update flag is on
                .filter(|u| {
                    cfg.installs
                        .iter()
                        .find(|i| i.id == u.install_id)
                        .and_then(|i| i.addons.iter().find(|a| a.slug == u.slug))
                        .map(|a| a.auto_update)
                        .unwrap_or(false)
                })
                .collect()
        };
        for u in items {
            let addons_dir = {
                let state = app.state::<AppState>();
                let cfg = state.config.lock().unwrap();
                cfg.installs.iter().find(|i| i.id == u.install_id).map(|i| i.path.clone())
            };
            let Some(addons_dir) = addons_dir else { continue };
            let dl = crate::net::resolve_download_url(&u.url, &base);
            match crate::delivery::download_and_install(&dl, &addons_dir, &base).await {
                Ok(outcome) => {
                    let folder = outcome
                        .folders
                        .first()
                        .cloned()
                        .unwrap_or_else(|| crate::delivery::folder_guess(&u.slug));
                    {
                        let state = app.state::<AppState>();
                        let mut cfg = state.config.lock().unwrap();
                        if let Some(inst) = cfg.install_mut(&u.install_id) {
                            let a = inst.addon_mut(&u.slug);
                            a.installed_version = Some(u.latest.clone());
                            a.folder = Some(folder.clone());
                        }
                    }
                    let _ = app.state::<AppState>().persist();
                    notify(app, "Addon updated", &format!("{folder} → {}", u.latest));
                }
                Err(e) => notify(app, "Addon update failed", &format!("{}: {e}", u.slug)),
            }
        }
    }

    // 2) App self-update — installs + relaunches on success. SKIP on Flatpak: a
    // read-only sandbox can't replace its own binary (spec §2); updates come via
    // `flatpak update`, not Tauri.
    if crate::config::is_flatpak() {
        return;
    }
    let state = app.state::<AppState>();
    if let Ok(SelfUpdateStatus::Available { version, .. }) =
        check_self_update(app.clone(), state).await
    {
        notify(app, "Uplink update", &format!("Installing {version}…"));
        let state = app.state::<AppState>();
        let _ = install_self_update(app.clone(), state).await; // relaunches
    }
}
