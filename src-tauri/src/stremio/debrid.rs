//! Debrid service registry + magnet/infoHash resolution.
//!
//! Supported services (v1): Real-Debrid, AllDebrid, Premiumize.
//!
//! Each service has a fixed HTTP API base and a way to validate a credential
//! and unrestrict a magnet. The flow for a stream with only `infoHash` is:
//!
//! 1. Build `magnet:?xt=urn:btih:<infoHash>&dn=<title>`.
//! 2. Call the service-specific "unrestrict magnet" endpoint.
//! 3. Return the resulting direct URL (the player uses it as `stream.url`).

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn sample() -> DebridService {
        DebridService {
            kind: DebridKind::RealDebrid,
            username: "u".to_string(),
            api_key: "k".to_string(),
            is_default: true,
        }
    }

    #[test]
    fn debrid_hosts_match_real_debrid() {
        let mut set = HashSet::new();
        for h in DebridKind::RealDebrid.cdn_hosts() {
            set.insert(h.to_string());
        }
        assert!(set.contains("real-debrid.com"));
    }

    #[test]
    fn is_debrid_host_recognizes_known() {
        assert!(DebridKind::RealDebrid.is_debrid_host("https://cdn.real-debrid.com/x"));
        assert!(DebridKind::AllDebrid.is_debrid_host("https://uptobox.com/abc"));
        assert!(!DebridKind::RealDebrid.is_debrid_host("https://example.com/abc"));
    }

    #[test]
    fn magnet_format_is_valid() {
        let m = DebridKind::RealDebrid.build_magnet("ABCDEF1234", Some("Movie 2024"));
        assert!(m.starts_with("magnet:?xt=urn:btih:ABCDEF1234"));
        assert!(m.contains("dn=Movie%202024"));
    }

    #[test]
    fn debrid_service_serde_preserves_kind() {
        let s = sample();
        let json = serde_json::to_string(&s).unwrap();
        let back: DebridService = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, DebridKind::RealDebrid);
        assert_eq!(back.api_key, "k");
        assert!(back.is_default);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebridKind {
    RealDebrid,
    AllDebrid,
    Premiumize,
    TorBox,
    Offcloud,
    EasyDebrid,
    LinkSnappy,
    MegaDebrid,
}

impl DebridKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DebridKind::RealDebrid => "real_debrid",
            DebridKind::AllDebrid => "all_debrid",
            DebridKind::Premiumize => "premiumize",
            DebridKind::TorBox => "torbox",
            DebridKind::Offcloud => "offcloud",
            DebridKind::EasyDebrid => "easydebrid",
            DebridKind::LinkSnappy => "linksnappy",
            DebridKind::MegaDebrid => "mega_debrid",
        }
    }

    pub fn api_base(&self) -> &'static str {
        match self {
            DebridKind::RealDebrid => "https://api.real-debrid.com/rest/1.0",
            DebridKind::AllDebrid => "https://api.alldebrid.com/v4",
            DebridKind::Premiumize => "https://www.premiumize.me/api",
            DebridKind::TorBox => "https://api.torbox.app",
            DebridKind::Offcloud => "https://offcloud.com/api",
            DebridKind::EasyDebrid => "https://easydebrid.ch/api/v1",
            DebridKind::LinkSnappy => "https://api.linksnappy.com/api",
            DebridKind::MegaDebrid => "https://api.mega-debrid.eu",
        }
    }

    /// Hostnames that indicate a stream URL is already a debrid CDN link.
    pub fn cdn_hosts(&self) -> &'static [&'static str] {
        match self {
            DebridKind::RealDebrid => &["real-debrid.com"],
            DebridKind::AllDebrid => &["alldebrid.com", "uptobox.com", "4kvm.com"],
            DebridKind::Premiumize => &["premiumize.me"],
            DebridKind::TorBox => &["torbox.app"],
            DebridKind::Offcloud => &["offcloud.com"],
            DebridKind::EasyDebrid => &["easy-debrid.com", "easydebrid.ch"],
            DebridKind::LinkSnappy => &["linksnappy.com"],
            DebridKind::MegaDebrid => &["mega-debrid.eu"],
        }
    }

    pub fn is_debrid_host(&self, url: &str) -> bool {
        let lower = url.to_lowercase();
        self.cdn_hosts().iter().any(|h| lower.contains(h))
    }

    /// Build a magnet URI from an infoHash + optional display name.
    pub fn build_magnet(&self, info_hash: &str, name: Option<&str>) -> String {
        let mut m = format!("magnet:?xt=urn:btih:{}", info_hash);
        if let Some(n) = name {
            // Minimal URL-encoding for the dn= parameter.
            let encoded = url_encode(n);
            m.push_str("&dn=");
            m.push_str(&encoded);
        }
        m
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DebridService {
    pub kind: DebridKind,
    /// Display name returned by the service's `user` endpoint.
    pub username: String,
    pub api_key: String,
    pub is_default: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DebridServiceFile {
    #[serde(default = "default_version")]
    pub version: u32,
    pub services: Vec<DebridService>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, thiserror::Error)]
pub enum DebridError {
    #[error("no default debrid service configured")]
    NoDefault,
    #[error("could not validate API key: {0}")]
    InvalidKey(String),
    #[error("could not unrestrict stream: {0}")]
    Unrestrict(String),
}

/// Validate a credential by hitting the service's "user info" endpoint.
pub fn validate_key(kind: DebridKind, api_key: &str) -> Result<String, DebridError> {
    let client = crate::http_client::shared_client();
    let (url, use_bearer) = match kind {
        DebridKind::RealDebrid => (format!("{}/user", kind.api_base()), true),
        DebridKind::AllDebrid => (
            format!(
                "{}/user?agent=SlasshyVault&apikey={}",
                kind.api_base(),
                api_key
            ),
            false,
        ),
        DebridKind::Premiumize => (
            format!("{}/account/info?apikey={}", kind.api_base(), api_key),
            false,
        ),
        DebridKind::TorBox => (
            format!("{}/v1/api/user/me?api_key={}", kind.api_base(), api_key),
            false,
        ),
        DebridKind::Offcloud => (
            format!("{}/account/info?apiKey={}", kind.api_base(), api_key),
            false,
        ),
        DebridKind::EasyDebrid => (format!("{}/user?key={}", kind.api_base(), api_key), false),
        DebridKind::LinkSnappy => (
            format!("{}/account/info?apikey={}", kind.api_base(), api_key),
            false,
        ),
        DebridKind::MegaDebrid => (
            format!("{}/api/credentials?token={}", kind.api_base(), api_key),
            false,
        ),
    };
    let mut req = client.get(&url).timeout(Duration::from_secs(10));
    if use_bearer {
        req = req.bearer_auth(api_key);
    }
    let resp = req
        .send()
        .map_err(|e| DebridError::InvalidKey(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(DebridError::InvalidKey(format!("status {}", resp.status())));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| DebridError::InvalidKey(e.to_string()))?;

    let username = match kind {
        DebridKind::RealDebrid => body
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("Real-Debrid user")
            .to_string(),
        DebridKind::AllDebrid => body
            .pointer("/data/user/username")
            .and_then(|v| v.as_str())
            .unwrap_or("AllDebrid user")
            .to_string(),
        DebridKind::Premiumize => body
            .pointer("/customer_id")
            .and_then(|v| v.as_str())
            .unwrap_or("Premiumize user")
            .to_string(),
        DebridKind::TorBox => body
            .pointer("/data/email")
            .and_then(|v| v.as_str())
            .unwrap_or("TorBox user")
            .to_string(),
        DebridKind::Offcloud => body
            .pointer("/email")
            .and_then(|v| v.as_str())
            .unwrap_or("Offcloud user")
            .to_string(),
        DebridKind::EasyDebrid => body
            .pointer("/username")
            .and_then(|v| v.as_str())
            .unwrap_or("EasyDebrid user")
            .to_string(),
        DebridKind::LinkSnappy => body
            .pointer("/username")
            .and_then(|v| v.as_str())
            .unwrap_or("LinkSnappy user")
            .to_string(),
        DebridKind::MegaDebrid => body
            .pointer("/username")
            .and_then(|v| v.as_str())
            .unwrap_or("Mega-Debrid user")
            .to_string(),
    };
    Ok(username)
}

/// Unrestrict a magnet link through the service.
pub fn unrestrict_magnet(
    service: &DebridService,
    info_hash: &str,
    name: Option<&str>,
) -> Result<String, DebridError> {
    let magnet = service.kind.build_magnet(info_hash, name);
    match service.kind {
        DebridKind::RealDebrid => unrestrict_real_debrid(service, &magnet),
        DebridKind::AllDebrid => unrestrict_all_debrid(service, &magnet),
        DebridKind::Premiumize => unrestrict_premiumize(service, info_hash, name),
        DebridKind::TorBox => unrestrict_torbox(service, info_hash, name),
        DebridKind::Offcloud => unrestrict_offcloud(service, &magnet),
        DebridKind::EasyDebrid => unrestrict_easydebrid(service, &magnet),
        DebridKind::LinkSnappy => unrestrict_linksnappy(service, &magnet),
        DebridKind::MegaDebrid => unrestrict_megadebrid(service, &magnet),
    }
}

fn unrestrict_real_debrid(service: &DebridService, magnet: &str) -> Result<String, DebridError> {
    let client = crate::http_client::shared_client();
    let resp = client
        .post(format!("{}/unrestrict/link", service.kind.api_base()))
        .bearer_auth(&service.api_key)
        .timeout(Duration::from_secs(30))
        .form(&[("link", magnet)])
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(DebridError::Unrestrict(format!(
            "Real-Debrid status {}",
            resp.status()
        )));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    body.get("download")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| DebridError::Unrestrict("no download URL in response".to_string()))
}

fn unrestrict_all_debrid(service: &DebridService, magnet: &str) -> Result<String, DebridError> {
    let client = crate::http_client::shared_client();
    // Step 1: upload the magnet.
    let upload_resp = client
        .get(format!(
            "{}/magnet/upload?agent=SlasshyVault&apikey={}",
            service.kind.api_base(),
            service.api_key
        ))
        .query(&[("magnets[]", magnet)])
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    if !upload_resp.status().is_success() {
        return Err(DebridError::Unrestrict(format!(
            "AllDebrid upload status {}",
            upload_resp.status()
        )));
    }
    let upload: serde_json::Value = upload_resp
        .json()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    let magnet_id = upload
        .pointer("/data/magnets/0/id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| DebridError::Unrestrict("no magnet id".to_string()))?;

    // Step 2: poll for status until "Ready" (up to ~30s).
    for _ in 0..10 {
        std::thread::sleep(Duration::from_secs(3));
        let status_resp = client
            .get(format!(
                "{}/magnet/status?agent=SlasshyVault&apikey={}&id={}",
                service.kind.api_base(),
                service.api_key,
                magnet_id
            ))
            .timeout(Duration::from_secs(10))
            .send()
            .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
        if !status_resp.status().is_success() {
            continue;
        }
        let status: serde_json::Value = status_resp
            .json()
            .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
        let ready = status
            .pointer("/data/magnets/status")
            .and_then(|v| v.as_str())
            .map(|s| s == "Ready")
            .unwrap_or(false);
        if !ready {
            continue;
        }
        if let Some(link) = status
            .pointer("/data/magnets/links/0/link")
            .and_then(|v| v.as_str())
        {
            // Step 3: unrestrict that link.
            let unlock = client
                .get(format!(
                    "{}/link/unlock?agent=SlasshyVault&apikey={}",
                    service.kind.api_base(),
                    service.api_key
                ))
                .query(&[("link", link)])
                .timeout(Duration::from_secs(15))
                .send()
                .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
            if unlock.status().is_success() {
                let unlock_json: serde_json::Value = unlock
                    .json()
                    .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
                if let Some(u) = unlock_json.pointer("/data/link").and_then(|v| v.as_str()) {
                    return Ok(u.to_string());
                }
            }
        }
    }
    Err(DebridError::Unrestrict(
        "AllDebrid magnet did not become ready in time".to_string(),
    ))
}

fn unrestrict_premiumize(
    service: &DebridService,
    info_hash: &str,
    name: Option<&str>,
) -> Result<String, DebridError> {
    let client = crate::http_client::shared_client();
    let magnet = service.kind.build_magnet(info_hash, name);
    let create = client
        .post(format!("{}/transfer/create", service.kind.api_base()))
        .query(&[("apikey", &service.api_key), ("src", &magnet)])
        .timeout(Duration::from_secs(15))
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    if !create.status().is_success() {
        return Err(DebridError::Unrestrict(format!(
            "Premiumize status {}",
            create.status()
        )));
    }
    let id = create
        .headers()
        .get("x-transfer-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| DebridError::Unrestrict("no transfer id".to_string()))?;
    let info = client
        .get(format!("{}/transfer/info", service.kind.api_base()))
        .query(&[("apikey", &service.api_key), ("id", &id)])
        .timeout(Duration::from_secs(15))
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    if !info.status().is_success() {
        return Err(DebridError::Unrestrict(
            "Premiumize info failed".to_string(),
        ));
    }
    let body: serde_json::Value = info
        .json()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    body.pointer("/content/0/link")
        .or_else(|| body.pointer("/content/link"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| DebridError::Unrestrict("no direct link".to_string()))
}

// Helper extension removed in favor of direct api_base() calls.

/// Resolve a Stremio stream to a direct URL.
///
/// - If `stream.url` is set and is a debrid CDN URL, follow redirects via
///   `reqwest` and return the final URL.
/// - If `stream.url` is set and is not a debrid CDN URL, return it as-is.
/// - If `stream.info_hash` is set, unrestrict via the user's default debrid.
/// - If the stream is a magnet (Stremio rarely delivers these but it's
///   possible), extract the `xt` infoHash and unrestrict.
pub fn resolve_stream(
    default_service: Option<&DebridService>,
    stream: &StremioStreamLike,
) -> Result<String, DebridError> {
    if let Some(url) = stream.url.as_deref() {
        if !url.is_empty() {
            if let Some(svc) = default_service {
                if svc.kind.is_debrid_host(url) {
                    return follow_redirects(url);
                }
            }
            return Ok(url.to_string());
        }
    }
    if let Some(hash) = stream.info_hash.as_deref() {
        let svc = default_service.ok_or(DebridError::NoDefault)?;
        return unrestrict_magnet(svc, hash, stream.title.as_deref());
    }
    Err(DebridError::Unrestrict(
        "stream has neither url nor infoHash".to_string(),
    ))
}

fn follow_redirects(url: &str) -> Result<String, DebridError> {
    let client = crate::http_client::shared_client();
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(15))
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    Ok(resp.url().to_string())
}

fn unrestrict_torbox(
    service: &DebridService,
    info_hash: &str,
    name: Option<&str>,
) -> Result<String, DebridError> {
    let client = crate::http_client::shared_client();
    let api = service.api_key.clone();
    // Step 1: create a torrent from the magnet.
    let create = client
        .post(format!(
            "{}/v1/api/torrents/createtorrent",
            service.kind.api_base()
        ))
        .query(&[("api_key", &api)])
        .form(&[
            (
                "magnet",
                service.kind.build_magnet(info_hash, name).as_str(),
            ),
            ("name", name.unwrap_or("")),
        ])
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    if !create.status().is_success() {
        return Err(DebridError::Unrestrict(format!(
            "TorBox create status {}",
            create.status()
        )));
    }
    let body: serde_json::Value = create
        .json()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    let torrent_id = body
        .pointer("/data/torrent_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| DebridError::Unrestrict("no TorBox torrent id".to_string()))?;
    // Step 2: request a download link.
    let req_link = client
        .get(format!(
            "{}/v1/api/torrents/requestdl",
            service.kind.api_base()
        ))
        .query(&[
            ("api_key", &api),
            ("torrent_id", &torrent_id.to_string()),
            ("file_id", &"0".to_string()),
        ])
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    if !req_link.status().is_success() {
        return Err(DebridError::Unrestrict(format!(
            "TorBox requestdl status {}",
            req_link.status()
        )));
    }
    let link_body: serde_json::Value = req_link
        .json()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    link_body
        .pointer("/data")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| DebridError::Unrestrict("no TorBox download link".to_string()))
}

fn unrestrict_offcloud(service: &DebridService, magnet: &str) -> Result<String, DebridError> {
    let client = crate::http_client::shared_client();
    let resp = client
        .post(format!("{}/cache", service.kind.api_base()))
        .query(&[("apiKey", &service.api_key)])
        .form(&[("url", magnet)])
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(DebridError::Unrestrict(format!(
            "Offcloud status {}",
            resp.status()
        )));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    body.pointer("/requestLink")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| DebridError::Unrestrict("no Offcloud link".to_string()))
}

fn unrestrict_easydebrid(service: &DebridService, magnet: &str) -> Result<String, DebridError> {
    let client = crate::http_client::shared_client();
    let resp = client
        .post(format!("{}/link/any", service.kind.api_base()))
        .query(&[("key", &service.api_key)])
        .form(&[("link", magnet)])
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(DebridError::Unrestrict(format!(
            "EasyDebrid status {}",
            resp.status()
        )));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    body.pointer("/url")
        .or_else(|| body.pointer("/data/url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| DebridError::Unrestrict("no EasyDebrid link".to_string()))
}

fn unrestrict_linksnappy(service: &DebridService, magnet: &str) -> Result<String, DebridError> {
    let client = crate::http_client::shared_client();
    let resp = client
        .post(format!("{}/upload/any", service.kind.api_base()))
        .query(&[("apikey", &service.api_key)])
        .form(&[("link", magnet)])
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(DebridError::Unrestrict(format!(
            "LinkSnappy status {}",
            resp.status()
        )));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    body.pointer("/data/files/0/link")
        .or_else(|| body.pointer("/files/0/link"))
        .or_else(|| body.pointer("/link"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| DebridError::Unrestrict("no LinkSnappy link".to_string()))
}

fn unrestrict_megadebrid(service: &DebridService, magnet: &str) -> Result<String, DebridError> {
    let client = crate::http_client::shared_client();
    let resp = client
        .post(format!("{}/api/generate", service.kind.api_base()))
        .query(&[("token", &service.api_key)])
        .form(&[("link", magnet)])
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(DebridError::Unrestrict(format!(
            "Mega-Debrid status {}",
            resp.status()
        )));
    }
    let body: serde_json::Value = resp
        .json()
        .map_err(|e| DebridError::Unrestrict(e.to_string()))?;
    body.pointer("/link")
        .or_else(|| body.pointer("/data/link"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| DebridError::Unrestrict("no Mega-Debrid link".to_string()))
}

/// Minimal Stremio stream shape consumed by the resolver. Decouples the
/// resolver from the full `Stream` schema.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StremioStreamLike {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub info_hash: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub yt_id: Option<String>,
}
