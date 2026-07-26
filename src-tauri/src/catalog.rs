//! The delivery catalog (GET /api/addons, schema gec.addons/2) and pairing
//! response types, plus the fetch functions. Shapes mirror
//! website/lib/uplink.ts::catalogFor and app/api/devices/pair/route.ts exactly.

use crate::net;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelDef {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub sort: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelLatest {
    pub version: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseRow {
    pub channel: String,
    pub version: String,
    pub url: String,
    #[serde(default)]
    pub published_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogAddon {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub short: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub blurb: String,
    #[serde(default)]
    pub github_repo: Option<String>,
    #[serde(default)]
    pub channels: HashMap<String, ChannelLatest>,
    #[serde(default)]
    pub releases: Vec<ReleaseRow>,
    /// WoW flavors this addon supports (e.g. ["Retail","Classic"]), matched case-insensitively against an
    /// install's flavor. EMPTY / absent = ALL flavors (backward-compatible: an older catalog imposes no
    /// restriction). The server declares this so a Retail-only addon (e.g. SBF today) is never offered or
    /// installed on a Classic install.
    #[serde(default)]
    pub flavors: Vec<String>,
}

impl CatalogAddon {
    /// Does this addon support the given install flavor? True when unrestricted (empty list) or a
    /// case-insensitive match is present. Drives the update/install flavor gate.
    pub fn supports_flavor(&self, flavor: &str) -> bool {
        self.flavors.is_empty() || self.flavors.iter().any(|f| f.eq_ignore_ascii_case(flavor))
    }

    /// Resolve the download for a channel, falling back to public.
    pub fn latest_for<'a>(&'a self, channel: &str) -> Option<&'a ChannelLatest> {
        self.channels.get(channel).or_else(|| self.channels.get("public"))
    }
    /// A specific pinned release on a channel (for rollback / hold).
    pub fn release<'a>(&'a self, channel: &str, version: &str) -> Option<&'a ReleaseRow> {
        self.releases
            .iter()
            .find(|r| r.channel == channel && r.version == version)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub channel_defs: Vec<ChannelDef>,
    #[serde(default)]
    pub addons: Vec<CatalogAddon>,
}

impl Catalog {
    pub fn addon<'a>(&'a self, slug: &str) -> Option<&'a CatalogAddon> {
        self.addons.iter().find(|a| a.slug == slug)
    }
}

// ── pairing (POST /api/devices/pair) ──
#[derive(Debug, Serialize)]
pub struct PairRequest {
    pub code: String,
    pub name: String,
    pub platform: String,
    pub app: String,
    pub app_version: String,
}

#[derive(Debug, Deserialize)]
pub struct PairAccount {
    pub handle: String,
    #[serde(default)]
    pub tier: Option<String>,
    pub role: String,
    #[serde(default)]
    pub channels: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PairResponse {
    pub token: String,
    pub device_id: i64,
    /// The device name as set on the website (present once the server supports it).
    #[serde(default)]
    pub name: Option<String>,
    pub account: PairAccount,
}

pub async fn fetch_catalog(base: &str) -> Result<Catalog, String> {
    net::get_json::<Catalog>(base, "/api/addons").await
}

/// The Home feed is returned verbatim (serde_json::Value) so the server's shape
/// can evolve without recompiling the client; the UI has the typed view.
pub async fn fetch_home(base: &str) -> Result<serde_json::Value, String> {
    net::get_json::<serde_json::Value>(base, "/api/me").await
}
