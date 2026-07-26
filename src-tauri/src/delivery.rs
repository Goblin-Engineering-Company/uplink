//! Addon delivery: download the release zip, verify it, and unzip it into the
//! install's AddOns folder. The zip carries the addon-named folder at its root
//! (release-public.sh guarantees this), so extraction is a straight unpack.

use crate::net;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

/// Result of an install: which top-level addon folders were written.
pub struct InstallOutcome {
    pub folders: Vec<String>,
    pub files: usize,
}

/// Download `url` and unzip into `addons_dir`. Returns the folders it wrote.
/// `base_url` gates the device-token attachment (see `net::get_bytes`): the token
/// is sent only when `url` shares its origin.
pub async fn download_and_install(url: &str, addons_dir: &str, base_url: &str) -> Result<InstallOutcome, String> {
    let dir = Path::new(addons_dir);
    if !dir.is_dir() {
        return Err(format!("AddOns folder not found (offline?): {addons_dir}"));
    }
    let bytes = net::get_bytes(url, base_url).await?;
    if bytes.len() < 4 || &bytes[0..2] != b"PK" {
        return Err("downloaded file is not a zip archive".to_string());
    }
    // unzip is CPU/IO-bound and sync — run it off the async executor.
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || unzip_into(&bytes, &dir))
        .await
        .map_err(|e| e.to_string())?
}

fn unzip_into(bytes: &[u8], dir: &Path) -> Result<InstallOutcome, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("bad zip: {e}"))?;
    let mut folders: Vec<String> = Vec::new();
    let mut files = 0usize;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // zip-slip guard: use the sanitized enclosed name and reject traversal.
        let rel: PathBuf = match entry.enclosed_name() {
            Some(p) => p,
            None => return Err(format!("unsafe zip entry: {}", entry.name())),
        };
        if rel.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir)) {
            return Err(format!("unsafe zip entry: {}", entry.name()));
        }
        // record the top-level folder (the addon name)
        if let Some(Component::Normal(top)) = rel.components().next() {
            let top = top.to_string_lossy().to_string();
            if !folders.contains(&top) {
                folders.push(top);
            }
        }
        let out = dir.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            // Pre-allocate against the zip's DECLARED size, but clamp it: the size
            // is attacker-controlled (a lying header), and an unclamped reserve can
            // abort the process on a bogus multi-GB value. read_to_end still grows
            // the buffer as needed for a legitimately larger file.
            let cap = (entry.size() as usize).min(16 * 1024 * 1024);
            let mut buf = Vec::with_capacity(cap);
            entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            std::fs::write(&out, &buf).map_err(|e| e.to_string())?;
            files += 1;
        }
    }
    if folders.is_empty() {
        return Err("zip contained no addon folder".to_string());
    }
    Ok(InstallOutcome { folders, files })
}

/// Remove an addon's folder(s) from an install. We only know the addon slug, so
/// we match the folder case-insensitively (WoW folder names vary in case).
pub fn uninstall(addons_dir: &str, folder_name: &str) -> Result<(), String> {
    let dir = Path::new(addons_dir);
    if !dir.is_dir() {
        return Err(format!("AddOns folder not found (offline?): {addons_dir}"));
    }
    let mut removed = false;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().eq_ignore_ascii_case(folder_name) && e.path().is_dir() {
                std::fs::remove_dir_all(e.path()).map_err(|e| e.to_string())?;
                removed = true;
            }
        }
    }
    if removed {
        Ok(())
    } else {
        // Not on disk — treat as already-removed rather than an error.
        Ok(())
    }
}

/// The on-disk AddOns folder name for a catalog slug. The catalog uses lowercase
/// slugs (haul, sbf) but the folder ships PascalCase; we resolve the real folder
/// from the zip on install and fall back to the slug for uninstall matching.
pub fn folder_guess(slug: &str) -> String {
    match slug.to_lowercase().as_str() {
        "sbf" => "SBF".to_string(),
        "haul" => "Haul".to_string(),
        // renamed Town Cryer → Megaphone (2026-07-16); old slugs still map to the
        // new folder so a lingering catalog/config reference resolves correctly.
        "megaphone" | "towncryer" | "town-cryer" | "cryer" => "Megaphone".to_string(),
        "recall" => "Recall".to_string(),
        "gec-console" | "console" => "GEC-Console".to_string(),
        _ => slug.to_string(),
    }
}

/// Read an addon's installed version straight from its on-disk `.toc`, so Uplink
/// recognizes addons installed by ANY means (CurseForge, manual, or us) and can
/// check them against the catalog. Matches the folder case-insensitively and
/// returns the first `## Version:` value; `None` if the folder/.toc/version is absent.
pub fn installed_version(addons_dir: &str, folder: &str) -> Option<String> {
    let dir = Path::new(addons_dir);
    // find the addon folder (case-insensitive — WoW folder casing varies)
    let folder_path = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(folder) && e.path().is_dir())?
        .path();
    let actual = folder_path.file_name()?.to_string_lossy().to_string();
    // prefer `<Folder>.toc`, else the first `.toc` in the folder
    let named = folder_path.join(format!("{actual}.toc"));
    let toc = if named.is_file() {
        named
    } else {
        std::fs::read_dir(&folder_path)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().map(|x| x.eq_ignore_ascii_case("toc")).unwrap_or(false))?
    };
    let text = std::fs::read_to_string(&toc).ok()?;
    for line in text.lines() {
        // "## Version: 2026.07.05.5" (directive is case-insensitive)
        if let Some(rest) = line.trim_start().strip_prefix("##") {
            let rest = rest.trim_start();
            if rest.len() >= 8 && rest[..8].eq_ignore_ascii_case("version:") {
                let v = rest[8..].trim().to_string();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}
