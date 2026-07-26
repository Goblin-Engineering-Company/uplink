//! Device-token storage. The plaintext token exists only in the pairing response
//! and here — never in the JSON config.
//!
//! We store it in a `device-token` file in the app data dir (owner-only perms on
//! Unix), NOT the OS keychain. The keychain is nominally more secure, but on an
//! unsigned/ad-hoc-signed build macOS can't persist an "Always Allow" grant, so
//! every launch pops an approval dialog — an unusable prompt storm during
//! development. A file protected by normal filesystem permissions is the standard
//! desktop-app credential pattern (cf. ~/.aws/credentials, npm tokens) and never
//! prompts. If a code-signed release later wants the keychain, reintroduce it
//! behind this same API. An in-memory cache still fronts the file so we touch disk
//! at most once per launch.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

// Absolute path to the token file, set once at startup from the app config dir.
static TOKEN_PATH: OnceLock<PathBuf> = OnceLock::new();
// `None` = not loaded yet / no token.
static CACHE: Mutex<Option<String>> = Mutex::new(None);

/// Point token storage at `<config_dir>/device-token`. Called by BOTH entry points
/// (the GUI `setup()` and the headless harness) before any token access.
pub fn init(config_dir: &Path) {
    let _ = TOKEN_PATH.set(config_dir.join("device-token"));
}

fn token_path() -> Option<PathBuf> {
    TOKEN_PATH.get().cloned()
}

pub fn store_token(token: &str) -> Result<(), String> {
    let path = token_path().ok_or("token path not initialized")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, token).map_err(|e| format!("token write failed: {e}"))?;
    // owner read/write only — no group/other access
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    *CACHE.lock().unwrap() = Some(token.to_string()); // seed the cache
    Ok(())
}

pub fn get_token() -> Option<String> {
    let mut guard = CACHE.lock().unwrap();
    if let Some(t) = guard.as_ref() {
        return Some(t.clone());
    }
    // headless/CI escape hatch: an explicit env token wins (used by the sync-once
    // harness so it needs no on-disk pairing).
    if let Ok(t) = std::env::var("UPLINK_DEVICE_TOKEN") {
        if !t.is_empty() {
            *guard = Some(t.clone());
            return Some(t);
        }
    }
    let path = token_path()?;
    let t = std::fs::read_to_string(&path).ok()?;
    let t = t.trim().to_string();
    if t.is_empty() {
        return None;
    }
    *guard = Some(t.clone());
    Some(t)
}

pub fn has_token() -> bool {
    get_token().is_some()
}

pub fn clear_token() -> Result<(), String> {
    *CACHE.lock().unwrap() = None;
    if let Some(path) = token_path() {
        match std::fs::remove_file(&path) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("token clear failed: {e}")),
        }
    } else {
        Ok(())
    }
}
