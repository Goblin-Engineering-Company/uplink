// Mirrors the serde structs in src-tauri/src/config.rs and the server payloads
// (gec.addons/2, GET /api/me). Keep in sync with the Rust side.

export type Account = {
  handle: string;
  tier: string | null;
  role: string;
  channels: string[];
};

export type InstalledAddon = {
  slug: string;
  enabled: boolean;              // selected for install on this WoW install
  installed_version: string | null;
  pinned_version: string | null;    // pin a specific version (rollback/hold)
  channel_override: string | null;  // jump this addon off the install channel
  auto_update: boolean;             // per-addon auto-update (scheduler source of truth)
  sync: boolean;                    // per-addon data-sync gate (redesign §3)
};

export type Install = {
  id: string;
  label: string;
  path: string;
  flavor: string;                // Retail | Classic | PTR | Beta | Unknown
  channel: string;               // delivery channel slug, default "public"
  auto_update: boolean;
  sync_accounts: string[];       // WoW account folders opted into for sync
  accounts_initialized: boolean; // has the all-on default been seeded once? (redesign §2)
  addons: InstalledAddon[];
  online: boolean;               // runtime: path currently present?
};

// one WoW account folder for an install + whether it's selected for sync
export type SyncAccount = { account: string; selected: boolean };

export type AppConfig = {
  base_url: string;
  theme: string;
  device_name: string | null;
  device_id: number | null;
  account: Account | null;
  installs: Install[];
  sync_while_running: boolean;
  sync_interval_secs: number;
  sync: StreamState[];
  account_aliases: Record<string, string>;  // real account name → display alias (redesign §4)
  dismissed_broadcast: number | null;
  onboarded: boolean;
  paired: boolean;               // runtime: keychain has a device token?
  is_flatpak: boolean;           // runtime: Flatpak (Steam Deck) build — no self-update
  self_update_channel: string;
  auto_update_enabled: boolean;
  auto_update_time: string;
  dev_update_url: string | null;   // dev-only self-update endpoint override
  start_in_tray: boolean;          // launch to the menu bar only, no main window
};

export type SelfUpdateStatus =
  | { kind: "up_to_date" }
  | { kind: "available"; version: string; notes: string | null }
  | { kind: "channel_revoked"; was: string };

// ── data sync (spec §6) — mirrors config.rs::StreamState / sync.rs::SyncResult ──
export type StreamState = {
  install_id: string;
  account: string;
  addon: string;        // catalog slug
  stream: string;
  cursor: string;
  queued: number;       // new entries beyond the cursor at last parse
  total: number;        // syncable entries seen at last parse
  last_sync: number | null;   // unix secs of last successful upload
  last_accepted: number;
  file_mtime: number | null;
  stuck: boolean;             // queue won't drain (redesign §6) — surfaces "Clear queue"
};

export type SyncResult = {
  install_id: string;
  install_label: string;
  account: string;
  addon: string;
  stream: string;
  found: number;
  accepted: number;
  inserted: number;   // NEW rows the server stored (duplicates excluded)
  cursor: string;
  unknown: boolean;
  skipped: string | null;
  error: string | null;
  // Non-fatal data-quality note (wire §10.5 server warning, or a local flavor-mismatch
  // flag §7d.6): the data WAS accepted and the cursor advanced — this just says why
  // something is flagged. Absent on a clean row.
  warning?: string | null;
};

export type DetectedInstall = {
  path: string;
  label: string;
  flavor: string;
  already_added: boolean;
};

// ── gec.addons/2 catalog ──
export type ChannelDef = { slug: string; name: string; badge: string; sort: number };
export type ChannelLatest = { version: string; url: string };
export type ReleaseRow = { channel: string; version: string; url: string; published_at: string };
export type CatalogAddon = {
  slug: string;
  name: string;
  short: string;
  status: string;
  blurb: string;
  github_repo: string | null;
  channels: Record<string, ChannelLatest>;
  releases: ReleaseRow[];
};
export type Catalog = {
  schema: string;
  channel_defs: ChannelDef[];
  addons: CatalogAddon[];
};

// ── GET /api/me ──
// Per-addon gameplay summary cards (profile-panel design 2026-07-25). A card
// key is ABSENT when the user has no data for that addon — don't render it.
// Money in copper; times epoch seconds / durations in seconds; catch_rate 0..1.
export type SbfCard = {
  fish: number; casts: number; catches: number;
  catch_rate: number | null; fish_per_hour: number | null; time_fished_sec: number;
  top_zone: { zone: string; fish: number } | null;
  top_fish: { label: string; n: number; quality: number } | null;
  week: { fish: number; casts: number };
  last_sync: string | null;
};
export type HaulCard = {
  counted: number; gross: number; coin: number; mail: number; income: number;
  sessions: number; active_sec: number; gold_per_hour: number | null;
  best_session: { counted: number; at: number } | null;
  xp_total: number; rep_total: number; currency_kinds: number;
  week: { counted: number };
  last_sync: string | null;
};

export type Me = {
  // the caller's own device row — Rust writes device.name back into config
  // each heartbeat (website rename propagates without re-pairing)
  device?: { id: number; name: string };
  handle: string;
  tier: string | null;
  role: string;
  channels: string[];
  rank: { name: string; xp: number };
  discoveries_week: number;
  broadcast: { id: number; title: string; body: string; published_at: string } | null;
  // full list, vote-popularity order; my_votes = this user's weight on the row
  // (free vote included), my_boosts = boost weight only
  votables: Array<{ number: number; title: string; url: string; votes: number; my_votes?: number; my_boosts?: number }>;
  // spend-encouragement state: the one free vote per cycle + boost credits
  vote_state?: { free_vote_available: boolean; boost_credits: number };
  whats_new: string | null;
  // Upload-freeze / service-denial state (spec §7b). Present + `uploads:"frozen"`
  // while the server is refusing ingest globally; `message` explains WHY.
  service?: { uploads: string; message: string } | null;
  // Persistent data-quality warnings mirrored from ingest (§10.5): known-bad build /
  // schema-ahead nags, so we can surface them outside a sync pass.
  data_warnings?: Array<{ code: string; addon?: string; message: string; count?: number }>;
  // Per-addon gameplay cards — absent key = no data = no card.
  addons?: { sbf?: SbfCard; haul?: HaulCard };
};

// what an install needs updated (computed in Rust)
export type UpdateItem = {
  install_id: string;
  slug: string;
  installed: string | null;
  latest: string;
  url: string;
  channel: string;
};
