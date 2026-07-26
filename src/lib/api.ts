// Thin typed wrappers over the Rust Tauri commands (src-tauri/src/commands.rs).
// The UI only ever talks to the core through these — never fetch() directly, so
// the device token (keychain) and base URL stay in Rust.
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import type {
  AppConfig, Account, DetectedInstall, Install, Catalog, Me, UpdateItem,
  StreamState, SyncResult, SyncAccount, SelfUpdateStatus,
} from "./types";

export const api = {
  getConfig: () => invoke<AppConfig>("get_config"),
  setTheme: (theme: string) => invoke<void>("set_theme", { theme }),
  setBaseUrl: (url: string) => invoke<void>("set_base_url", { url }),
  setSyncWhileRunning: (v: boolean) => invoke<void>("set_sync_while_running", { value: v }),
  setSyncInterval: (secs: number) => invoke<void>("set_sync_interval", { secs }),
  markOnboarded: () => invoke<void>("mark_onboarded"),

  // startup: launch-at-login (OS-owned, query live) + start-in-tray (config)
  setLaunchAtLogin: (enabled: boolean) => invoke<void>("set_launch_at_login", { enabled }),
  getLaunchAtLogin: () => invoke<boolean>("get_launch_at_login"),
  setStartInTray: (enabled: boolean) => invoke<void>("set_start_in_tray", { enabled }),

  // data sync (spec §6)
  syncNow: () => invoke<SyncResult[]>("sync_now"),
  getSyncStatus: () => invoke<StreamState[]>("get_sync_status"),
  resetCursors: () => invoke<number>("reset_cursors"),
  purgeSyncQueue: () => invoke<number>("purge_sync_queue"),
  listSyncAccounts: (installId: string) => invoke<SyncAccount[]>("list_sync_accounts", { installId }),
  setSyncAccounts: (installId: string, accounts: string[]) =>
    invoke<void>("set_sync_accounts", { installId, accounts }),
  // per-account display alias (real name stays the wire/sync key) — redesign §4
  setAccountAlias: (account: string, alias: string) =>
    invoke<void>("set_account_alias", { account, alias }),

  // auth
  pairDevice: (code: string) =>
    invoke<Account>("pair_device", { code }),
  unpair: () => invoke<void>("unpair"),
  // reconcile config against on-disk addons (installed by any method) + return catalog
  reconcileInstalled: () => invoke<Catalog>("reconcile_installed"),

  // installs
  detectInstalls: () => invoke<DetectedInstall[]>("detect_installs"),
  // enumerate every WoW flavor under a user-browsed folder (install-discovery reframe)
  enumerateInstallsAt: (path: string) => invoke<DetectedInstall[]>("enumerate_installs_at", { path }),
  addInstall: (path: string, label: string) =>
    invoke<Install>("add_install", { path, label }),
  removeInstall: (id: string) => invoke<void>("remove_install", { id }),
  updateInstall: (id: string, label: string, channel: string, autoUpdate: boolean) =>
    invoke<void>("update_install", { id, label, channel, autoUpdate }),
  setAddonSelected: (installId: string, slug: string, enabled: boolean) =>
    invoke<void>("set_addon_selected", { installId, slug, enabled }),
  setAddonAutoUpdate: (installId: string, slug: string, enabled: boolean) =>
    invoke<void>("set_addon_auto_update", { installId, slug, enabled }),
  // per-addon data-sync toggle (redesign §3)
  setAddonSync: (installId: string, slug: string, on: boolean) =>
    invoke<void>("set_addon_sync", { installId, slug, on }),
  setInstallAutoUpdateAll: (installId: string, enabled: boolean) =>
    invoke<void>("set_install_auto_update_all", { installId, enabled }),
  resetAddons: (installId: string) => invoke<void>("reset_addons", { installId }),
  setAddonVersion: (installId: string, slug: string, pinnedVersion: string | null, channelOverride: string | null) =>
    invoke<void>("set_addon_version", { installId, slug, pinnedVersion, channelOverride }),

  // catalog + delivery
  fetchCatalog: () => invoke<Catalog>("fetch_catalog"),
  installAddon: (installId: string, slug: string, version: string, url: string, channel: string) =>
    invoke<string>("install_addon", { installId, slug, version, url, channel }),
  uninstallAddon: (installId: string, slug: string) =>
    invoke<void>("uninstall_addon", { installId, slug }),
  checkUpdates: () => invoke<UpdateItem[]>("check_updates"),
  updateAll: (installId: string) => invoke<string[]>("update_all", { installId }),
  // reconcile every installed addon to its effective-channel target (any
  // direction) — used after a channel switch so e.g. dev→public downgrades.
  reconcileInstall: (installId: string) => invoke<string[]>("reconcile_install", { installId }),
  reportInstalls: () => invoke<void>("report_installs"),

  // self-update
  setSelfUpdateChannel: (channel: string) => invoke<void>("set_self_update_channel", { channel }),
  setAutoUpdate: (enabled: boolean) => invoke<void>("set_auto_update", { enabled }),
  setAutoUpdateTime: (time: string) => invoke<void>("set_auto_update_time", { time }),
  checkSelfUpdate: () => invoke<SelfUpdateStatus>("check_self_update"),
  installSelfUpdate: () => invoke<void>("install_self_update"),

  // home
  fetchHome: () => invoke<Me>("fetch_home"),
  dismissBroadcast: (id: number) => invoke<void>("dismiss_broadcast", { id }),

  // helpers that run in the webview
  browseFolder: () => openDialog({ directory: true, multiple: false, title: "Select your World of Warcraft folder" }),
  confirm: (message: string, title: string) => confirmDialog(message, { title, kind: "warning" }),
  // Open a web link in the OS browser. Guard the scheme: only http(s). Broadcast
  // and votable URLs come from the server, so a compromised one could be file://
  // or a custom scheme that the OS opener would hand to a local handler — block
  // those (no-op + warn) rather than open them.
  openExternal: (url: string): Promise<void> => {
    let ok = false;
    try { const p = new URL(url).protocol; ok = p === "https:" || p === "http:"; } catch { /* invalid URL */ }
    if (!ok) { console.warn("openExternal blocked non-web URL:", url); return Promise.resolve(); }
    return openUrl(url);
  },
};
