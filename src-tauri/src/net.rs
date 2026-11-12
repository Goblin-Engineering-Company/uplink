//! Thin HTTP helpers against the GEC server. The device token (from the
//! keychain) is attached as `Authorization: Bearer <token>` on every call. Base
//! URL is resolved from config (single source: config::DEFAULT_BASE_URL).

use crate::auth;
use flate2::write::GzEncoder;
use flate2::Compression;
use reqwest::Client;
use serde::de::DeserializeOwned;
use std::io::Write;

/// The ONE process-wide `reqwest::Client` (audit H1). A `Client` *is* the connection
/// pool + TLS config + root-cert store, so building one per request means zero
/// keep-alive reuse and a full TLS handshake every call (7 per chunked sync). Built
/// lazily on first use, then handed out as a cheap `Arc` clone.
static CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();

/// The shared HTTP client. Same signature/behaviour as before — the timeouts, the
/// UA and the error string are unchanged; only the construction is now once-per-process.
/// (`build()` can only fail on TLS-backend init, which is deterministic, so a failure
/// is re-derived rather than cached.)
pub fn client() -> Result<Client, String> {
    if let Some(c) = CLIENT.get() {
        return Ok(c.clone());
    }
    let built = Client::builder()
        .user_agent(concat!("gec-uplink/", env!("CARGO_PKG_VERSION")))
        // A stalled connection must fail, not hang forever: an unbounded request
        // would freeze the sync loop (and, headless, the whole pass). 120s covers a
        // large first-batch upload with comfortable headroom.
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(CLIENT.get_or_init(|| built).clone())
}

fn join(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'))
}

/// GET <base><path> as JSON, with the device token if we have one.
pub async fn get_json<T: DeserializeOwned>(base: &str, path: &str) -> Result<T, String> {
    let mut req = client()?.get(join(base, path));
    if let Some(tok) = auth::get_token() {
        req = req.bearer_auth(tok);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(server_error(resp).await);
    }
    resp.json::<T>().await.map_err(|e| format!("bad response: {e}"))
}

/// POST JSON body to <base><path>, requiring the device token.
pub async fn post_json<B: serde::Serialize, T: DeserializeOwned>(
    base: &str,
    path: &str,
    body: &B,
    require_auth: bool,
) -> Result<T, String> {
    let mut req = client()?.post(join(base, path)).json(body);
    match auth::get_token() {
        Some(tok) => req = req.bearer_auth(tok),
        None if require_auth => return Err("Pair this device first.".to_string()),
        None => {}
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(server_error(resp).await);
    }
    resp.json::<T>().await.map_err(|e| format!("bad response: {e}"))
}

/// POST a JSON body **gzip-compressed** (`Content-Encoding: gzip`), requiring the
/// device token. Used for the ingest path (spec §9.3) where batches can be large
/// and highly repetitive — the route `gunzipSync`s the body. Returns the parsed
/// ack. Serializing + compressing is CPU work, but batches are chunk-bounded by
/// the caller, so it stays off the critical path.
/// How many times a transient upload failure is retried before the pass gives up and
/// leaves the remainder queued. Small on purpose: the timed check is the real safety
/// net, this only smooths over a blip mid-upload.
const UPLOAD_RETRIES: u32 = 3;
/// First backoff step; doubles per attempt (400ms, 800ms, 1600ms).
const UPLOAD_RETRY_BASE_MS: u64 = 400;

pub async fn post_gzip_json<B: serde::Serialize, T: DeserializeOwned>(
    base: &str,
    path: &str,
    body: &B,
) -> Result<T, String> {
    let tok = auth::get_token().ok_or("Pair this device first.")?;
    let raw = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&raw).map_err(|e| e.to_string())?;
    let gz = enc.finish().map_err(|e| e.to_string())?;

    // Transient-failure retry, INSIDE the upload path where the status code is still
    // known. A 502/504 from the gateway (or a dropped connection) used to abort the
    // whole pass: cursors held, data stayed queued, and the remainder waited for the
    // next timed check up to `sync_interval_secs` away. Nothing was lost, but on a
    // multi-chunk upload one blip stalled everything behind it for a quarter of an hour.
    //
    // Deliberately NOT retried: any 4xx (our fault, so retrying just repeats it) and
    // 503, which the upload freeze (spec 7b) uses to say uploads are paused on purpose.
    // Retrying a freeze would hammer a server that just asked us to stop.
    let mut attempt = 0u32;
    loop {
        let sent = client()?
            .post(join(base, path))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::CONTENT_ENCODING, "gzip")
            .bearer_auth(tok.clone())
            .body(gz.clone())
            .send()
            .await;

        let retryable = match &sent {
            // transport-level: connection reset, timeout, TLS hiccup
            Err(_) => true,
            Ok(r) => {
                // 503 is deliberately NOT retried: the upload freeze (spec 7b) uses it,
                // and there is no field in the current contract that separates a freeze
                // from a transient 503 without consuming the body the error path needs.
                // Retrying a freeze would hammer a server that just asked us to stop, so
                // 503 keeps its existing behaviour of surfacing immediately.
                matches!(r.status().as_u16(), 502 | 504)
            }
        };

        if retryable && attempt < UPLOAD_RETRIES {
            let wait = UPLOAD_RETRY_BASE_MS * 2u64.pow(attempt);
            attempt += 1;
            crate::sync::log_progress(&format!(
                "upload attempt {attempt} failed (retryable), retrying in {wait}ms"
            ));
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            continue;
        }

        let resp = sent.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(server_error(resp).await);
        }
        return resp.json::<T>().await.map_err(|e| format!("bad response: {e}"));
    }
}


/// GET <url> and return ONLY the HTTP status code, attaching the device token
/// if present. Used by the self-update status probe (200/204/403) because the
/// Tauri updater collapses a 403 into ReleaseNotFound — we need the real code.
pub async fn get_status(url: &str) -> Result<u16, String> {
    let client = client()?;
    let mut req = client.get(url);
    if let Some(tok) = auth::get_token() {
        req = req.bearer_auth(tok);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16())
}

/// Download raw bytes (an addon release zip). The device token is attached ONLY
/// when the target shares `base_url`'s origin — i.e. our own gated
/// `/api/download/...` endpoint (gated URLs are resolved onto `base_url` first by
/// `resolve_download_url`). Public release zips live on GitHub (a different
/// origin) and need no auth; refusing to send the bearer anywhere else means a
/// catalog row pointing at a foreign host can't exfiltrate the token.
pub async fn get_bytes(url: &str, base_url: &str) -> Result<Vec<u8>, String> {
    let mut req = client()?.get(url);
    if same_origin(url, base_url) {
        if let Some(tok) = auth::get_token() {
            req = req.bearer_auth(tok);
        }
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(server_error(resp).await);
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

/// True when `a` and `b` share scheme + host + port (an "origin"). Used to confine
/// the device token to our own server and never leak it to a catalog-supplied host.
fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(x), Ok(y)) => {
            x.scheme() == y.scheme()
                && x.host_str() == y.host_str()
                && x.port_or_known_default() == y.port_or_known_default()
        }
        _ => false,
    }
}

async fn server_error(resp: reqwest::Response) -> String {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        // Upload freeze (spec §7b): 503 `{frozen:true, message}` — surface the human
        // message VERBATIM, not a bare "503". No cursor advances (the caller treats
        // this as a post error and holds the cursor); clients retry later by design.
        if v.get("frozen").and_then(|f| f.as_bool()) == Some(true) {
            let msg = v.get("message").and_then(|m| m.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("uploads are temporarily paused — try again later");
            return format!("Uploads paused: {msg}");
        }
        // Dead-schema rejection (§10.1): 422 `{error, update:"uplink"}` — tell the
        // user WHAT to update, not just the code.
        if let Some(msg) = v.get("error").and_then(|e| e.as_str()) {
            if let Some(upd) = v.get("update").and_then(|u| u.as_str()) {
                return format!("{msg} — please update {upd} ({})", status.as_u16());
            }
            return format!("{} ({})", msg, status.as_u16());
        }
    }
    format!("server returned {}", status)
}

/// Resolve an addon download URL against `base_url`. A gated download is a server
/// endpoint (path under `/api/`); if the catalog baked in the wrong host (e.g. a
/// wrong host from a mis-pointed publish), rebuild the URL against our
/// real `base_url` so the download still works. External/GitHub URLs (public
/// releases) have no `/api/` path and pass through unchanged.
pub fn resolve_download_url(url: &str, base_url: &str) -> String {
    if let Ok(u) = url::Url::parse(url) {
        if u.path().starts_with("/api/") {
            let mut out = format!("{}{}", base_url.trim_end_matches('/'), u.path());
            if let Some(q) = u.query() {
                out.push('?');
                out.push_str(q);
            }
            return out;
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::{client, resolve_download_url, same_origin};

    // H1: every call must hand back the SAME pooled client, not a fresh TLS stack.
    // The clients are Arc handles onto the one value parked in `CLIENT`; proving the
    // static is populated and stable proves no per-request `Client::builder()` runs.
    #[test]
    fn client_is_built_once_and_reused() {
        let _a = client().expect("client builds");
        let first = super::CLIENT.get().expect("first call populates the shared client")
            as *const reqwest::Client;
        let _b = client().expect("client builds");
        let second = super::CLIENT.get().expect("still populated") as *const reqwest::Client;
        assert!(std::ptr::eq(first, second), "client() must reuse the shared client, not rebuild it");
    }

    #[test]
    fn same_origin_confines_the_token() {
        let base = "https://goblineng.co";
        // our own gated endpoint → same origin → token attaches
        assert!(same_origin("https://goblineng.co/api/download/sbf/public/1", base));
        // a foreign host a bad catalog row could inject → NOT same origin
        assert!(!same_origin("http://attacker.example/x.zip", base));
        // public GitHub release → different origin → no token (as intended)
        assert!(!same_origin("https://github.com/org/SBF/releases/download/1/SBF.zip", base));
        // scheme mismatch is not the same origin
        assert!(!same_origin("http://goblineng.co/api/download/x", base));
        // a dev tunnel base authorizes its own downloads
        assert!(same_origin("https://t.example/api/download/x", "https://t.example"));
    }

    #[test]
    fn rewrites_server_api_host_but_passes_github_through() {
        let base = "https://goblineng.co";
        // mis-recorded host on a gated download → rebuilt against base
        assert_eq!(
            resolve_download_url("https://wrong-host.example/api/download/sbf/dev/2026.07.15.2", base),
            "https://goblineng.co/api/download/sbf/dev/2026.07.15.2"
        );
        // query preserved
        assert_eq!(
            resolve_download_url("http://wrong-host.example/api/download/x?platform=mac", base),
            "https://goblineng.co/api/download/x?platform=mac"
        );
        // public GitHub URL passes through untouched
        let gh = "https://github.com/Goblin-Engineering-Company/SBF/releases/download/2026.07.12.1/SBF-2026.07.12.1.zip";
        assert_eq!(resolve_download_url(gh, base), gh);
        // already-correct server URL is idempotent
        assert_eq!(
            resolve_download_url("https://goblineng.co/api/download/sbf/dev/1", base),
            "https://goblineng.co/api/download/sbf/dev/1"
        );
    }
}
