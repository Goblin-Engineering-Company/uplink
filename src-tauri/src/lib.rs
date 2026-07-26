//! GEC Uplink core. Wires plugins, the menu-bar/tray icon + dropdown panel, the
//! managed app state (config + its path), and the command handlers. Desktop
//! entry is main.rs → run().

mod auth;
mod catalog;
mod commands;
mod config;
mod delivery;
mod installs;
mod net;
mod schedule;
mod selfupdate;
mod svparse;
mod sync;
mod version;

use config::{AppConfig, AppState};
use std::sync::atomic::{AtomicI64, Ordering};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

// Epoch-ms of the last time the tray dropdown hid itself on losing focus. The
// tray-icon click that closes the panel first fires a blur (which hides it), so
// the click handler would otherwise see "not visible" and re-open it. If a blur
// happened within this window, we treat the click as a close and skip re-showing.
static LAST_TRAY_BLUR_MS: AtomicI64 = AtomicI64::new(0);
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ── managed state on the BUILDER, before setup() ── the webview's first
    // invoke("get_config") can dispatch before the setup closure runs — Windows
    // WebView2 loses this race that macOS WKWebView wins — so managing state INSIDE
    // setup yields "state not managed for get_config" on Windows only. Resolve the
    // config dir with the standalone app_config_dir() (no app handle needed; it returns
    // the same path Tauri does for this bundle id, and is already what the headless
    // commands use) and manage during build(), before the webview exists — then no
    // command can ever run without state, on any platform.
    let dir = app_config_dir();
    auth::init(&dir); // token file lives next to config.json
    let config_path = dir.join("config.json");
    let state = AppState {
        config: std::sync::Mutex::new(AppConfig::load(&config_path)),
        config_path,
    };

    tauri::Builder::default()
        .manage(state)
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // ── plugins (desktop only) ── updater = signature verification, process
            // = relaunch(), notification = OS toasts, autostart = launch-at-login.
            // NON-FATAL: a plugin that fails to init on one platform must not tank
            // startup — log and carry on (that one feature just won't work).
            #[cfg(desktop)]
            {
                if let Err(e) = app.handle().plugin(tauri_plugin_updater::Builder::new().build()) {
                    eprintln!("updater plugin init failed: {e}");
                }
                if let Err(e) = app.handle().plugin(tauri_plugin_process::init()) {
                    eprintln!("process plugin init failed: {e}");
                }
                if let Err(e) = app.handle().plugin(tauri_plugin_notification::init()) {
                    eprintln!("notification plugin init failed: {e}");
                }
                // Launch-at-login. Register WITH a `--hidden` arg so a login-item
                // launch is quiet (the window-show logic below hides the main window
                // when it sees that arg), independent of the persisted start_in_tray pref.
                if let Err(e) = app.handle().plugin(tauri_plugin_autostart::init(
                    tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                    Some(vec!["--hidden"]),
                )) {
                    eprintln!("autostart plugin init failed: {e}");
                }
            }

            // ── main-window visibility on launch ── the `main` window ships hidden
            // (tauri.conf.json `visible:false`). Show it when EITHER the user isn't
            // onboarded yet (first run must ALWAYS see the wizard — start_in_tray now
            // defaults true, so this guard is what keeps onboarding visible, §3.9) OR
            // the user hasn't opted into start-in-tray and this isn't a quiet login-item
            // launch (`--hidden`). So: a --hidden login launch stays in the tray; an
            // onboarded manual launch honors start_in_tray; a not-yet-onboarded user
            // always sees the window. The tray "Open Uplink" re-shows it regardless.
            let (onboarded, start_in_tray) = {
                let st = app.state::<AppState>();
                let c = st.config.lock().unwrap();
                (c.onboarded, c.start_in_tray)
            };
            let hidden_launch = std::env::args().any(|a| a == "--hidden");
            let show = !onboarded || (!start_in_tray && !hidden_launch);
            if show {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }

            // ── tray icon + menu (menu-bar on macOS, system tray elsewhere) ──
            let open_i = MenuItem::with_id(app, "open", "Open Uplink", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit GEC Uplink", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_i, &quit_i])?;

            TrayIconBuilder::with_id("uplink-tray")
                .icon(app.default_window_icon().cloned().expect("bundled icon"))
                .tooltip("GEC Uplink")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    // left-click toggles the compact dropdown panel
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        position,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(win) = app.get_webview_window("tray") {
                            match win.is_visible() {
                                Ok(true) => {
                                    let _ = win.hide();
                                }
                                _ => {
                                    // If a blur just hid the panel (this same click on the icon
                                    // caused it), treat the click as a close — don't reopen.
                                    if now_ms() - LAST_TRAY_BLUR_MS.load(Ordering::Relaxed) < 250 {
                                        return;
                                    }
                                    // Anchor the panel just below the menu-bar icon, centered on
                                    // the click — otherwise it opens at its last/default spot
                                    // (mid-screen). `position` is the physical cursor location.
                                    if let Ok(size) = win.outer_size() {
                                        let x = position.x - (size.width as f64) / 2.0;
                                        let y = position.y + 6.0;
                                        let _ = win.set_position(tauri::PhysicalPosition::new(
                                            x.max(0.0),
                                            y.max(0.0),
                                        ));
                                    }
                                    let _ = win.show();
                                    let _ = win.set_focus();
                                }
                            }
                        }
                    }
                })
                .build(app)?;

            // ── background data-sync (spec §6): one launch pass, then an interval
            // backstop that also re-parses on SV-file mtime change. Both defer to
            // the sync-while-running gate inside sync_all; manual Sync-now bypasses
            // it. Errors are swallowed (best-effort; the UI shows per-stream state).
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // small launch delay so pairing/first-render settle before we parse
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                loop {
                    {
                        let state = handle.state::<AppState>();
                        // The per-addon + per-account gates (redesign §3/§2, applied inside
                        // sync_all) decide what actually uploads — there is no global master
                        // switch. A token is still required (nothing to upload unpaired).
                        if auth::has_token() {
                            let _ = sync::sync_all(state.inner(), false).await;
                        }
                    }
                    let secs = {
                        let state = handle.state::<AppState>();
                        let s = state.config.lock().unwrap().sync_interval_secs;
                        s.max(30)
                    };
                    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                }
            });

            // ── auto-update scheduler (spec §9): one pass shortly after launch,
            // then a daily pass when the local clock crosses auto_update_time.
            // No WoW-running gate — relaunching Uplink doesn't touch the game.
            let up_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(8)).await;
                // Stamp last_run only if the startup pass actually did work (auto
                // was enabled at start). Otherwise leave it None so a same-day
                // toggle-on fires on the next tick instead of waiting till tomorrow.
                let enabled_at_start = {
                    let st = up_handle.state::<AppState>();
                    let c = st.config.lock().unwrap();
                    c.auto_update_enabled
                };
                selfupdate::run_auto_update_pass(&up_handle).await;
                let mut last_run: Option<chrono::NaiveDate> = None;
                if enabled_at_start {
                    last_run = Some(chrono::Local::now().date_naive());
                }
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    let (enabled, target) = {
                        let state = up_handle.state::<AppState>();
                        let cfg = state.config.lock().unwrap();
                        (cfg.auto_update_enabled, cfg.auto_update_time.clone())
                    };
                    if !enabled {
                        continue;
                    }
                    let now = chrono::Local::now();
                    if schedule::due(now.time(), now.date_naive(), &target, last_run) {
                        selfupdate::run_auto_update_pass(&up_handle).await;
                        last_run = Some(now.date_naive());
                    }
                }
            });

            Ok(())
        })
        // keep the app resident in the tray: closing the main window hides it.
        .on_window_event(|window, event| match event {
            // keep the app resident in the tray: closing a window hides it.
            WindowEvent::CloseRequested { api, .. } => {
                if window.label() == "main" || window.label() == "tray" {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
            // Click-away dismisses the tray dropdown (a transient popover). Only
            // the tray — the main window stays put when it loses focus. Stamp the
            // blur so the tray-icon click that caused it reads as a close.
            WindowEvent::Focused(false) => {
                if window.label() == "tray" {
                    LAST_TRAY_BLUR_MS.store(now_ms(), Ordering::Relaxed);
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::set_theme,
            commands::set_base_url,
            commands::set_sync_while_running,
            commands::set_sync_interval,
            commands::sync_now,
            commands::get_sync_status,
            commands::reset_cursors,
            commands::purge_sync_queue,
            commands::list_sync_accounts,
            commands::set_sync_accounts,
            commands::set_account_alias,
            commands::mark_onboarded,
            commands::dismiss_broadcast,
            commands::pair_device,
            commands::unpair,
            commands::detect_installs,
            commands::enumerate_installs_at,
            commands::add_install,
            commands::remove_install,
            commands::update_install,
            commands::set_addon_selected,
            commands::set_addon_auto_update,
            commands::set_addon_sync,
            commands::set_install_auto_update_all,
            commands::reset_addons,
            commands::set_addon_version,
            commands::fetch_catalog,
            commands::fetch_home,
            commands::reconcile_installed,
            commands::install_addon,
            commands::uninstall_addon,
            commands::check_updates,
            commands::update_all,
            commands::reconcile_install,
            commands::report_installs,
            commands::set_self_update_channel,
            commands::set_auto_update,
            commands::set_auto_update_time,
            commands::set_launch_at_login,
            commands::get_launch_at_login,
            commands::set_start_in_tray,
            selfupdate::check_self_update,
            selfupdate::install_self_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running GEC Uplink");
}

/// Headless one-shot sync used by `main.rs`'s `sync-once` verification path. Loads
/// the paired config from the standard app-config dir (no Tauri/GUI), runs a manual
/// sync against the configured (production) server, and prints the per-stream
/// results as JSON. Exit code reflects success.
pub fn headless_sync() {
    let dir = app_config_dir();
    auth::init(&dir); // same token file the GUI wrote
    let config_path = dir.join("config.json");
    let config = AppConfig::load(&config_path);
    let state = AppState {
        config: std::sync::Mutex::new(config),
        config_path,
    };
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    match rt.block_on(sync::sync_all(&state, true)) {
        Ok(results) => {
            println!("{}", serde_json::to_string_pretty(&results).unwrap_or_default());
            eprintln!("sync-once: {} stream result(s)", results.len());
        }
        Err(e) => {
            eprintln!("sync-once failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Headless `reset-cursors`: zero every local stream cursor (the same thing the
/// Data page's "Full resync" button does), so the next sync re-sends everything.
/// Used by the verification path after switching to the verbatim contract, since a
/// cursor left over from the old flattened (epoch-`t`) engine would otherwise
/// suppress the index-based slice.
pub fn headless_reset_cursors() {
    let dir = app_config_dir();
    let config_path = dir.join("config.json");
    let mut config = AppConfig::load(&config_path);
    let cleared = config.zero_all_cursors();
    match config.save(&config_path) {
        Ok(()) => println!("reset-cursors: cleared {cleared} cursor(s)"),
        Err(e) => {
            eprintln!("reset-cursors failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Verification harness: parse a real SV file + summarize what would be sent, with
/// no keychain/network. Prints registry/stream/catalog counts and the first couple
/// of VERBATIM entries so the generic Lua→JSON parse is provable against a real
/// (100k–500k-line) file.
pub fn parse_only(file: &str, global: &str) {
    let t0 = std::time::Instant::now();
    let export = match svparse::parse_export_file(std::path::Path::new(file), global) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("parse failed: {e}");
            std::process::exit(1);
        }
    };
    let elapsed = t0.elapsed();
    println!("parsed {file} [{global}] in {:.2}s", elapsed.as_secs_f64());
    println!(
        "  store: producerBuild={:?} formatVersion={:?} schemaVersion={:?} registryVersion={:?} generatedAt={:?}",
        export.producer_build, export.format_version, export.schema_version, export.registry_version, export.generated_at
    );
    let mut domains: Vec<(String, usize)> = export
        .registry
        .iter()
        .map(|(k, v)| {
            // domains are carried verbatim as `{ items: [...], _byKey: {...} }`; count `.items`
            let n = v.get("items").and_then(|it| it.as_array()).map(|a| a.len()).unwrap_or(0);
            (k.clone(), n)
        })
        .collect();
    domains.sort();
    println!("  registry domains: {domains:?}");
    println!("  learned_items (db.items): {} records", export.db_items.len());
    let mut names: Vec<&String> = export.streams.keys().collect();
    names.sort();
    for name in names {
        let ps = &export.streams[name];
        // kind histogram for the stream (verbatim `k` field)
        let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for e in &ps.entries {
            let k = e.get("k").and_then(|v| v.as_str()).unwrap_or("(none)").to_string();
            *kinds.entry(k).or_insert(0) += 1;
        }
        println!("  stream '{name}' (schema {}): {} entries, kinds={:?}", ps.schema, ps.entries.len(), kinds);
        for e in ps.entries.iter().take(2) {
            println!("    · {}", serde_json::to_string(e).unwrap_or_default());
        }
    }
}

/// The same location Tauri's `app_config_dir()` resolves to for this bundle id, so
/// the headless path reads the very config the GUI app wrote.
fn app_config_dir() -> std::path::PathBuf {
    let id = "com.goblinengineering.uplink";
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::Path::new(&home).join("Library/Application Support").join(id);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return std::path::Path::new(&appdata).join(id);
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return std::path::Path::new(&xdg).join(id);
        }
        if let Ok(home) = std::env::var("HOME") {
            return std::path::Path::new(&home).join(".config").join(id);
        }
    }
    std::path::PathBuf::from(".")
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}
