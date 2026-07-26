// Prevents an extra console window on Windows in release; noop elsewhere.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Headless one-shot sync (verification harness): `gec-uplink sync-once` runs a
    // full sync against the paired config + production server and prints the ack,
    // without launching the tray/GUI. Same binary/signature as the app, so it uses
    // the same keychain device token.
    if std::env::args().any(|a| a == "sync-once") {
        gec_uplink_lib::headless_sync();
        return;
    }
    // `gec-uplink reset-cursors` — zero local cursors (the "Full resync" button,
    // headless) so the next sync re-sends everything.
    if std::env::args().any(|a| a == "reset-cursors") {
        gec_uplink_lib::headless_reset_cursors();
        return;
    }
    // `gec-uplink parse-only <file> <Global>` — parse an SV file + summarize the
    // fishlog mapping with NO keychain/network. A pure verification of the
    // svparse+resolver path against a real (large) SavedVariables file.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "parse-only") {
        let file = args.get(i + 1).cloned().unwrap_or_default();
        let global = args.get(i + 2).cloned().unwrap_or_else(|| "SBFData".to_string());
        gec_uplink_lib::parse_only(&file, &global);
        return;
    }
    gec_uplink_lib::run()
}
