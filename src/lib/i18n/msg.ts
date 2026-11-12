// Messages that ORIGINATE IN RUST.
//
// The problem this solves is not only localization. Today the core returns
// finished English prose across the IPC boundary — `SYNC_ALREADY_RUNNING = "a
// sync is already running"` in `sync.rs`, `"WoW is running (sync-while-running
// off)"`, `"Pair this device first."` in `net.rs` and `sync.rs`,
// `"That doesn't look like a WoW folder…"` in `installs.rs` — and the frontend
// renders it verbatim (`Data.tsx` prints `r.skipped` straight into the results
// table). So the product's wording lives in two languages of code, in two
// repositories' worth of style, and drifts. "Pair this device first" is
// written in three places today (twice in Rust, once as a UI tooltip), kept in
// step by hand, exactly what a code plus one dictionary entry removes.
//
// The fix, which is better design with or without translation: Rust returns a
// stable MACHINE CODE plus parameters, and the frontend owns every word.
//
//   Rust:  Msg { code: "wow_running", params: {} }
//   here:  "core.wow-running" → "WoW is running, so Uplink is waiting until you
//                               close it."
//
// The code is API: it is matched on, logged, and compared. It never changes
// because someone rewrote a sentence. The sentence changes freely because it is
// no longer API.

import type { MsgKey } from "./en";
import { translate } from "./translate";
import type { Lang, ParamValue } from "./types";

/// The wire shape. Mirrors the Rust struct this design adds:
///
/// ```rust
/// #[derive(Serialize, Clone, Debug)]
/// pub struct Msg {
///     pub code: &'static str,                       // stable, snake_case
///     #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
///     pub params: BTreeMap<String, String>,         // named, never positional
///     pub text: String,                             // English, for logs
/// }
/// ```
///
/// `text` is the English rendering Rust already produces. It is NOT for display
/// in the normal path — it is what goes in `eprintln!` and crash reports, and it
/// is the last-ditch fallback when an older UI meets a newer core that emits a
/// code the dictionary has never heard of. Shipping it costs a few bytes and
/// removes the whole class of "the new core says something the UI renders as
/// blank".
export type CoreMsg = {
  code: string;
  params?: Record<string, ParamValue>;
  text?: string;
};

/// True for a value that came across as a structured message rather than the
/// legacy bare string. Both shapes are accepted during the migration, so the
/// Rust and frontend halves can land in either order.
export function isCoreMsg(v: unknown): v is CoreMsg {
  return typeof v === "object" && v !== null && typeof (v as CoreMsg).code === "string";
}

/// Rust code → dictionary key. THE table. A code with no entry here falls back to
/// the English `text` the core sent, so a newer core never renders as nothing.
///
/// Every code below corresponds to a real string in the current Rust source; the
/// migration replaces the string with the code at that exact call site.
export const CORE_MESSAGE_KEYS: Record<string, MsgKey> = {
  // sync.rs — whole-pass and per-stream skips
  sync_already_running: "core.sync-already-running", // sync.rs SYNC_ALREADY_RUNNING
  wow_running: "core.wow-running", // sync.rs "WoW is running (sync-while-running off)"
  unchanged: "core.unchanged", // sync.rs skip
  no_exportable_data: "core.no-exportable-data", // sync.rs skip
  nothing_new: "core.nothing-new", // sync.rs skip
  unknown_stream: "core.unknown-stream", // Data.tsx "not used by the site yet"

  // pairing / auth
  not_paired: "core.not-paired", // net.rs + sync.rs
  empty_account: "core.empty-account", // commands.rs
  unexpected_pairing_status: "core.unexpected-pairing-status", // commands.rs
  token_clear_failed: "core.token-clear-failed", // auth.rs

  // installs / delivery
  unknown_install: "core.unknown-install", // commands.rs, 12 call sites
  flavor_unsupported: "core.flavor-unsupported", // commands.rs install_addon gate
  addons_dir_missing: "core.addons-dir-missing", // delivery.rs
  not_a_zip: "core.not-a-zip", // delivery.rs
  unsafe_zip_entry: "core.unsafe-zip-entry", // delivery.rs
  zip_no_addon_folder: "core.zip-no-addon-folder", // delivery.rs
  not_a_wow_folder: "core.not-a-wow-folder", // installs.rs

  // network / server
  uploads_paused: "core.uploads-paused", // net.rs 503 frozen
  dead_schema: "core.dead-schema", // net.rs 422 {error, update}
  server_error: "core.server-error", // net.rs message + status
  server_status: "core.server-status", // net.rs bare status
  bad_response: "core.bad-response", // net.rs json decode
  update_check_failed: "core.update-check-failed", // selfupdate.rs

  // SavedVariables parsing
  sv_not_a_table: "core.sv-not-a-table", // svparse.rs
  sv_too_deep: "core.sv-too-deep", // svparse.rs

  // data-quality warnings (§10.5)
  drained: "core.warn-drained", // sync.rs IngestWarning
  missing_stream: "core.warn-missing-stream", // sync.rs IngestWarning
  flavor_mismatch: "core.warn-flavor-mismatch", // sync.rs flavor_flags
};

/// Render a core message in `lang`. Accepts the legacy bare string too, so a
/// call site can be migrated before the Rust side is.
export function translateCoreMsg(lang: Lang, msg: CoreMsg | string | null | undefined): string | null {
  if (msg == null) return null;
  if (typeof msg === "string") return msg; // pre-migration: already English prose
  const key = CORE_MESSAGE_KEYS[msg.code];
  if (!key) {
    if (import.meta.env?.DEV) console.warn(`[i18n] no dictionary key for core code "${msg.code}"`);
    return msg.text ?? msg.code;
  }
  return translate(lang, key, msg.params);
}
