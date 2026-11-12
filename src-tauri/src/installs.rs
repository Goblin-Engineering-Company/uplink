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
    // WoW: Forever (launched 2026-11-04): its own client folder, retail API on
    // interface 16001. Without this row it was never auto-detected and showed as
    // "Unknown" (the wire token `forever` already passed through).
    ("_forever_", "Forever"),
    ("_ptr_", "PTR"),
    ("_xptr_", "PTR"),
    ("_beta_", "Beta"),
    ("_classic_", "Classic"),
    ("_classic_era_", "Classic Era"),
    ("_classic_ptr_", "Classic PTR"),
    ("_classic_era_ptr_", "Classic Era PTR"),
];

/// Display name for a wire flavor TOKEN (`flavor_token_from_path`'s output), e.g.
/// `classic_era` → "Classic Era". The token is matched EXACTLY against `FLAVORS`
/// (whose keys are the same strings with the underscores trimmed), so no token can
/// be swallowed by a shorter one that happens to be its prefix. An unrecognized
/// token — a future `_classic_wotlk_` — has no display name yet, so it renders as
/// "Unknown"; the WIRE still forwards it verbatim (see `flavor_token_from_path`).
pub fn display_for_flavor_token(token: &str) -> String {
    FLAVORS
        .iter()
        .find(|(seg, _)| seg.trim_matches('_') == token)
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Derive the DISPLAY flavor from any path containing a `_flavor_` segment.
///
/// Resolves the flavor from the `_flavor_` PATH COMPONENT (via the same scan the wire
/// token uses), never by substring. The old `lower.contains(seg)` scan mis-derived
/// three of the eight flavors because the segments are prefixes of each other and the
/// short ones are listed first: `_classic_era_` contains `_classic_` and both
/// `_classic_ptr_` / `_classic_era_ptr_` contain `_ptr_`, so the first arm won.
/// `Install.flavor` feeds `CatalogAddon::supports_flavor`, so a wrong answer here gates
/// an addon against the wrong client.
pub fn flavor_from_path(path: &str) -> String {
    display_for_flavor_token(&flavor_token_from_path(path))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every flavor directory in `FLAVORS`, each resolving to its OWN display name.
    /// Three of these (`_classic_era_`, `_classic_ptr_`, `_classic_era_ptr_`) used to
    /// resolve to "Classic" / "PTR" / "PTR" under the old substring scan (audit H2).
    #[test]
    fn display_flavor_for_every_flavor_dir() {
        let cases = [
            ("_retail_", "Retail"),
            ("_forever_", "Forever"),
            ("_ptr_", "PTR"),
            ("_xptr_", "PTR"),
            ("_beta_", "Beta"),
            ("_classic_", "Classic"),
            ("_classic_era_", "Classic Era"),
            ("_classic_ptr_", "Classic PTR"),
            ("_classic_era_ptr_", "Classic Era PTR"),
        ];
        // the table must cover the whole real flavor set, not a subset of it
        assert_eq!(cases.len(), FLAVORS.len());
        for (seg, want) in cases {
            let path = format!("/Applications/World of Warcraft/{seg}/Interface/AddOns");
            assert_eq!(flavor_from_path(&path), want, "flavor for {seg}");
        }
    }

    /// The display name and the wire token are derived from the SAME path component,
    /// so an install can never be labelled one flavor and uploaded as another.
    #[test]
    fn display_flavor_agrees_with_wire_token() {
        for (seg, _) in FLAVORS {
            let path = format!("/Volumes/Editing SSD/World of Warcraft/{seg}/Interface/AddOns");
            let token = flavor_token_from_path(&path);
            assert_eq!(token, seg.trim_matches('_'));
            assert_eq!(flavor_from_path(&path), display_for_flavor_token(&token));
        }
    }

    /// A flavor dir we don't have a display name for is still forwarded verbatim on
    /// the wire (contract rule: pass-through, never an allowlist) and shows "Unknown".
    #[test]
    fn unknown_flavor_dir_is_unknown_but_forwarded() {
        let p = "/Applications/World of Warcraft/_classic_wotlk_/Interface/AddOns";
        assert_eq!(flavor_from_path(p), "Unknown");
        assert_eq!(flavor_token_from_path(p), "classic_wotlk");
    }

    /// No `_flavor_` component at all (a custom / bare AddOns folder).
    #[test]
    fn no_flavor_segment_is_unknown() {
        assert_eq!(flavor_from_path("/Users/me/WoW/Interface/AddOns"), "Unknown");
        assert_eq!(flavor_token_from_path("/Users/me/WoW/Interface/AddOns"), "");
    }

    /// A flavor-looking string INSIDE a component name is not a flavor segment — the
    /// old substring scan would have called this install "Retail".
    #[test]
    fn flavor_substring_in_a_folder_name_does_not_match() {
        assert_eq!(
            flavor_from_path("/Users/me/wow_retail_backup/Interface/AddOns"),
            "Unknown"
        );
    }

    /// A WoW folder holding `_retail_` and `_forever_` (the user's external-drive
    /// layout) yields BOTH installs, Forever labelled as such.
    #[test]
    fn forever_is_detected_next_to_retail() {
        let root = std::env::temp_dir().join(format!("gec-uplink-forever-{}", std::process::id()));
        for seg in ["_retail_", "_forever_"] {
            std::fs::create_dir_all(root.join(seg).join("Interface/AddOns")).unwrap();
        }
        let real_root = std::fs::canonicalize(&root).unwrap();
        let found = enumerate_at(&root, &[], "Mac");
        let picked_root = found.get(1).map(|d| wow_root_from_picked(&d.path));
        let _ = std::fs::remove_dir_all(&root);
        let flavors: Vec<&str> = found.iter().map(|d| d.flavor.as_str()).collect();
        assert_eq!(flavors, vec!["Retail", "Forever"]);
        assert!(found[1].path.ends_with("_forever_/Interface/AddOns"));
        // and a pick of the Forever AddOns folder resolves to the same WoW root
        assert_eq!(picked_root, Some(real_root));
    }
}
