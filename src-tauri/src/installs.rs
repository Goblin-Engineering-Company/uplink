//! WoW install discovery: auto-detect common AddOns folders across macOS /
//! Windows / Linux, plus flavor detection from the path. Manual folder-picking
//! (external drives, custom paths) is handled by the UI's dialog → add_install.

use crate::config::Install;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct DetectedInstall {
    pub path: String,
    pub label: String,
    pub flavor: String,
    pub already_added: bool,
}

/// The WoW flavor subfolders. Order = display preference.
const FLAVORS: &[(&str, &str)] = &[
    ("_retail_", "Retail"),
    ("_ptr_", "PTR"),
    ("_xptr_", "PTR"),
    ("_beta_", "Beta"),
    ("_classic_", "Classic"),
    ("_classic_era_", "Classic Era"),
    ("_classic_ptr_", "Classic PTR"),
    ("_classic_era_ptr_", "Classic Era PTR"),
];

/// Derive the DISPLAY flavor from any path containing a `_flavor_` segment.
pub fn flavor_from_path(path: &str) -> String {
    let lower = path.to_lowercase();
    for (seg, name) in FLAVORS {
        if lower.contains(seg) {
            return name.to_string();
        }
    }
    "Unknown".to_string()
}

/// The pass-through WIRE token for the ingest envelope (data contract v5, envelope
/// rule 4): the install's `_flavor_` folder segment, underscores trimmed, lowercased,
/// ≤20 chars. Unknown segments are forwarded VERBATIM (never blanked) so a future
/// `_classic_wotlk_` flows through with no code change; `""` only when there's no
/// flavor segment at all. (Distinct from `flavor_from_path`, which is display-only.)
pub fn flavor_token_from_path(path: &str) -> String {
    for comp in Path::new(path).components() {
        if let std::path::Component::Normal(os) = comp {
            let s = os.to_string_lossy();
            let b = s.as_bytes();
            if b.len() >= 3 && b[0] == b'_' && b[b.len() - 1] == b'_' {
                let inner = s.trim_matches('_').to_lowercase();
                if !inner.is_empty() {
                    return inner.chars().take(20).collect();
                }
            }
        }
    }
    String::new()
}

/// The real OS hostname (`steamdeck`, `MacBook-Pro`, …). Queried via the
/// syscall, NOT env vars — HOSTNAME/COMPUTERNAME aren't set for GUI apps on
/// macOS (nor inside the Deck flatpak), which is why the old env-only path fell
/// back to "<user>'s PC". Trims any DNS suffix; ignores an unhelpful localhost.
#[cfg(unix)]
fn os_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let s = String::from_utf8_lossy(&buf[..end]);
    let s = s.split('.').next().unwrap_or(&s).trim().to_string();
    (!s.is_empty() && !s.eq_ignore_ascii_case("localhost")).then_some(s)
}
#[cfg(not(unix))]
fn os_hostname() -> Option<String> {
    None // Windows: COMPUTERNAME env below is reliable
}

/// A friendly default machine label. Preference order (the paired *device name*
/// is preferred over this by the callers): real hostname → HOSTNAME/COMPUTERNAME
/// env → "<user>'s PC". User can always rename it.
pub fn machine_label() -> String {
    if let Some(h) = os_hostname() {
        return h;
    }
    for key in ["HOSTNAME", "COMPUTERNAME", "HOST"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.split('.').next().unwrap_or(&v).trim().to_string();
            if !v.is_empty() {
                return v;
            }
        }
    }
    if let Ok(u) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
        if !u.is_empty() {
            return format!("{}'s PC", u);
        }
    }
    "This PC".to_string()
}

/// Candidate WoW root directories (the folder that *contains* the `_flavor_`
/// dirs). We probe each flavor under each root.
fn candidate_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();

    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/Applications/World of Warcraft"));
        if let Some(h) = &home {
            roots.push(Path::new(h).join("Applications/World of Warcraft"));
        }
        // external drives / network shares: /Volumes/<name>/Applications/World of Warcraft
        if let Ok(entries) = std::fs::read_dir("/Volumes") {
            for e in entries.flatten() {
                roots.push(e.path().join("Applications/World of Warcraft"));
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        roots.push(PathBuf::from(r"C:\Program Files (x86)\World of Warcraft"));
        roots.push(PathBuf::from(r"C:\Program Files\World of Warcraft"));
        roots.push(PathBuf::from(r"C:\World of Warcraft"));
        for letter in ['D', 'E', 'F', 'G'] {
            roots.push(PathBuf::from(format!(r"{}:\World of Warcraft", letter)));
            roots.push(PathBuf::from(format!(r"{}:\Program Files (x86)\World of Warcraft", letter)));
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(h) = &home {
            // common Lutris / wine prefixes
            roots.push(Path::new(h).join("Games/world-of-warcraft/drive_c/Program Files (x86)/World of Warcraft"));
            roots.push(Path::new(h).join(".wine/drive_c/Program Files (x86)/World of Warcraft"));
            roots.push(Path::new(h).join(".var/app/net.lutris.Lutris/data/lutris/World of Warcraft"));
        }
    }

    roots
}

/// Probe EVERY flavor's `Interface/AddOns` under a single WoW `root`, returning a
/// `DetectedInstall` per flavor present (symlink-canonicalized path, `already_added`
/// flagged against `existing`). Does NOT de-dupe across calls — the caller collapses
/// duplicate canonical paths (a .dmg's `/Applications` symlink can surface the same
/// real install from two roots). This is the reusable core shared by `detect` (auto-
/// scan across `candidate_roots`) and `enumerate_picked` (a user-browsed folder).
pub fn enumerate_at(root: &Path, existing: &[Install], label: &str) -> Vec<DetectedInstall> {
    let known: Vec<String> = existing.iter().map(|i| canon_key(&i.path)).collect();
    let mut out: Vec<DetectedInstall> = Vec::new();
    for (seg, flavor) in FLAVORS {
        let addons = root.join(seg).join("Interface").join("AddOns");
        if !addons.is_dir() {
            continue;
        }
        // Resolve symlinks to the REAL path before de-duping. A Tauri install
        // .dmg mounts at /Volumes/<App> and contains an /Applications symlink,
        // so scanning /Volumes/<App>/Applications/… would otherwise surface the
        // real /Applications WoW install a SECOND time as a phantom. Canonical-
        // izing collapses it (a real external-drive install on /Volumes stays).
        let real = std::fs::canonicalize(&addons).unwrap_or_else(|_| addons.clone());
        let path = real.to_string_lossy().to_string();
        let n = norm(&path);
        out.push(DetectedInstall {
            label: label.to_string(),
            flavor: flavor.to_string(),
            already_added: known.contains(&n),
            path,
        });
    }
    out
}

/// Scan for installs. `existing` = already-configured install paths (normalized)
/// so we can flag ones the user already added. Maps `enumerate_at` over every
/// candidate root and de-dupes the combined results by canonical path.
pub fn detect(existing: &[Install], label: &str) -> Vec<DetectedInstall> {
    let mut out: Vec<DetectedInstall> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for root in candidate_roots() {
        for d in enumerate_at(&root, existing, label) {
            let n = norm(&d.path);
            if seen.contains(&n) {
                continue;
            }
            seen.push(n);
            out.push(d);
        }
    }
    out
}

/// Normalize a user-BROWSED path to a WoW *root* (the folder that holds the
/// `_flavor_` dirs). If any path component is a `_flavor_` segment, the root is its
/// parent — so a pick of `…/_retail_/Interface/AddOns`, `…/_retail_/Interface`, or
/// `…/_retail_` all resolve to the same root. Otherwise the picked path itself is
/// treated as a candidate root (it may directly contain `_flavor_` children).
fn wow_root_from_picked(picked: &str) -> PathBuf {
    let p = Path::new(picked);
    let is_flavor = |name: &str| {
        let l = name.to_lowercase();
        FLAVORS.iter().any(|(seg, _)| l == *seg)
    };
    let comps: Vec<std::path::Component> = p.components().collect();
    for (idx, c) in comps.iter().enumerate() {
        if let std::path::Component::Normal(os) = c {
            if is_flavor(&os.to_string_lossy()) {
                // Rebuild the path up to (not including) this flavor segment.
                let mut root = PathBuf::new();
                for cc in &comps[..idx] {
                    root.push(cc.as_os_str());
                }
                return root;
            }
        }
    }
    p.to_path_buf()
}

/// Enumerate every WoW flavor under a user-BROWSED path, reframing the pick as a WoW
/// *folder* (not "the AddOns folder"). Normalizes to a root (`wow_root_from_picked`)
/// then `enumerate_at`. If no flavor dirs are found under any interpretation, fall
/// back to `resolve_addons_path` and return that single entry so custom/non-standard
/// layouts (a bare AddOns folder, a symlinked path) still work.
pub fn enumerate_picked(picked: &str, existing: &[Install], label: &str) -> Vec<DetectedInstall> {
    let root = wow_root_from_picked(picked);
    let found = enumerate_at(&root, existing, label);
    if !found.is_empty() {
        return found;
    }
    // No flavor dirs — accept the resolved AddOns path as a single custom install.
    match resolve_addons_path(picked) {
        Ok(addons) => {
            let real = std::fs::canonicalize(&addons)
                .map(|r| r.to_string_lossy().to_string())
                .unwrap_or(addons);
            let known: Vec<String> = existing.iter().map(|i| canon_key(&i.path)).collect();
            let n = norm(&real);
            vec![DetectedInstall {
                label: label.to_string(),
                flavor: flavor_from_path(&real),
                already_added: known.contains(&n),
                path: real,
            }]
        }
        Err(_) => Vec::new(),
    }
}

/// Validate a manually-picked folder is (or contains) an AddOns dir; return the
/// normalized AddOns path. Accepts either the AddOns folder itself or a WoW
/// flavor folder (we append Interface/AddOns).
pub fn resolve_addons_path(picked: &str) -> Result<String, String> {
    let p = Path::new(picked);
    if p.file_name().map(|n| n.eq_ignore_ascii_case("AddOns")).unwrap_or(false) {
        return Ok(picked.to_string());
    }
    // maybe they picked Interface/ or the flavor root — try to descend
    for suffix in [
        PathBuf::from("AddOns"),
        PathBuf::from("Interface").join("AddOns"),
    ] {
        let cand = p.join(&suffix);
        if cand.is_dir() {
            return Ok(cand.to_string_lossy().to_string());
        }
    }
    // allow a not-yet-existing AddOns dir if the parent looks like Interface
    if p.file_name().map(|n| n.eq_ignore_ascii_case("Interface")).unwrap_or(false) {
        return Ok(p.join("AddOns").to_string_lossy().to_string());
    }
    // maybe they picked the WoW ROOT (holds _retail_/_classic_/… flavors) — descend.
    for (seg, _) in FLAVORS {
        let cand = p.join(seg).join("Interface").join("AddOns");
        if cand.is_dir() {
            return Ok(cand.to_string_lossy().to_string());
        }
    }
    Err("That doesn't look like a WoW folder. Pick your World of Warcraft folder, a _retail_/_classic_ folder, or its Interface/AddOns folder.".to_string())
}

fn norm(p: &str) -> String {
    p.trim_end_matches(['/', '\\']).to_lowercase()
}

/// De-dupe key for an install path: resolve symlinks to the real path, then norm.
/// Collapses the install-.dmg `/Applications` symlink onto the real install.
fn canon_key(p: &str) -> String {
    let real = std::fs::canonicalize(p)
        .map(|r| r.to_string_lossy().to_string())
        .unwrap_or_else(|_| p.to_string());
    norm(&real)
}
