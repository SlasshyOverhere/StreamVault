//! Stremio manifest fetch + validation.

use serde::Deserialize;
use std::time::Duration;

use super::{StremioAddon, StremioAddonStatus, StremioCatalog, StremioResource};

/// Raw manifest payload as served by the addon.
#[derive(Debug, Deserialize)]
struct RawManifest {
    id: String,
    version: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    logo: Option<String>,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    resources: Vec<StremioResource>,
    #[serde(default, rename = "idPrefixes")]
    id_prefixes: Vec<String>,
    #[serde(default)]
    catalogs: Vec<StremioCatalog>,
    #[serde(default)]
    behavior_hints: Option<BehaviorHints>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct BehaviorHints {
    #[serde(default)]
    configuration_required: bool,
    #[serde(default)]
    configuration_url: Option<String>,
}

/// Behavior hints derived from the manifest.
#[derive(Debug, Clone, Default)]
pub struct ParsedBehaviorHints {
    pub configuration_required: bool,
    pub configuration_url: Option<String>,
}

/// Parsed manifest, normalized for the rest of the module.
pub struct ParsedManifest {
    pub addon: StremioAddon,
    pub behavior_hints: ParsedBehaviorHints,
}

/// Errors that can occur while fetching/parsing a manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("unsupported scheme: {0}")]
    UnsupportedScheme(String),
    #[error("could not reach the addon's manifest: {0}")]
    Unreachable(String),
    #[error("manifest is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("manifest is missing required field: {0}")]
    MissingField(&'static str),
    #[error("manifest has no `resources` array")]
    NoResources,
}

/// Normalize a user-entered URL.
///
/// - `stremio://` is rewritten to `https://`.
/// - A bare origin is suffixed with `/manifest.json`.
/// - Non-http(s) schemes are rejected.
pub fn normalize_url(input: &str) -> Result<String, ManifestError> {
    let mut s = input.trim().to_string();
    if s.is_empty() {
        return Err(ManifestError::Unreachable("empty URL".to_string()));
    }
    if let Some(rest) = s.strip_prefix("stremio://") {
        s = format!("https://{}", rest);
    } else if s.starts_with("stremio:") {
        return Err(ManifestError::UnsupportedScheme(s));
    }
    let parsed = url::Url::parse(&s)
        .map_err(|e| ManifestError::Unreachable(format!("invalid URL: {}", e)))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(ManifestError::UnsupportedScheme(parsed.scheme().to_string()));
    }
    let mut s = parsed.to_string();
    // Trim trailing slash and ensure /manifest.json suffix.
    while s.ends_with('/') {
        s.pop();
    }
    if !s.to_lowercase().ends_with("/manifest.json") {
        s.push_str("/manifest.json");
    }
    Ok(s)
}

fn parse(raw: RawManifest) -> Result<ParsedManifest, ManifestError> {
    if raw.id.trim().is_empty() {
        return Err(ManifestError::MissingField("id"));
    }
    if raw.version.trim().is_empty() {
        return Err(ManifestError::MissingField("version"));
    }
    if raw.name.trim().is_empty() {
        return Err(ManifestError::MissingField("name"));
    }
    if raw.resources.is_empty() {
        return Err(ManifestError::NoResources);
    }
    let hints = raw.behavior_hints.unwrap_or_default();
    let now = chrono::Utc::now();
    let addon = StremioAddon {
        id: raw.id,
        url: String::new(), // set by caller
        name: raw.name,
        version: raw.version,
        description: raw.description,
        logo: raw.logo,
        types: raw.types,
        resources: raw.resources,
        id_prefixes: raw.id_prefixes,
        catalogs: raw.catalogs,
        config: None,
        status: StremioAddonStatus::Available,
        installed_at: now,
        last_checked_at: now,
    };
    Ok(ParsedManifest {
        addon,
        behavior_hints: ParsedBehaviorHints {
            configuration_required: hints.configuration_required,
            configuration_url: hints.configuration_url,
        },
    })
}

/// Fetch a manifest over HTTP and parse it.
///
/// Resolution strategy:
/// 1. Normalize the URL (ensure /manifest.json suffix) and try it.
/// 2. If that fails, try the original URL as-is (user may have pasted
///    a direct manifest URL already).
/// 3. If the original URL returns HTML, scan for manifest links
///    (stremio:// deep links, <link rel="manifest">, href containing
///    manifest.json).
pub fn fetch_and_parse(url: &str) -> Result<ParsedManifest, ManifestError> {
    let client = crate::http_client::shared_client();

    // Step 1: try normalized URL.
    if let Ok(normalized) = normalize_url(url) {
        if let Ok(addon) = try_fetch_manifest(client, &normalized) {
            return Ok(addon);
        }
    }

    // Step 2: try the original URL as-is (might already be a manifest URL).
    if let Ok(addon) = try_fetch_manifest(client, url) {
        return Ok(addon);
    }

    // Step 3: fetch the original URL as a page and scan for manifest links.
    let parsed_input = url::Url::parse(url)
        .map_err(|e| ManifestError::Unreachable(format!("invalid URL: {}", e)))?;
    let base = format!(
        "{}://{}",
        parsed_input.scheme(),
        parsed_input.host_str().unwrap_or("")
    );
    let client = crate::http_client::shared_client();
    let page_resp = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| ManifestError::Unreachable(e.to_string()))?;
    let is_page_ok = page_resp.status().is_success();
    let page_ct = page_resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let page_body = if is_page_ok {
        page_resp
            .text()
            .map_err(|e| ManifestError::Unreachable(e.to_string()))?
    } else {
        return Err(ManifestError::Unreachable(format!(
            "status {}",
            page_resp.status()
        )));
    };

    // Scan HTML for manifest links.
    if let Some(manifest_url) = find_manifest_in_html(&page_body, &base) {
        if let Ok(addon) = try_fetch_manifest(client, &manifest_url) {
            return Ok(addon);
        }
    }

    Err(ManifestError::Unreachable(
        "could not find a valid manifest.json at this URL".to_string(),
    ))
}

/// Try to fetch and parse a single URL as a manifest.
fn try_fetch_manifest(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<ParsedManifest, ManifestError> {
    let mut resp = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| ManifestError::Unreachable(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(ManifestError::Unreachable(format!(
            "status {}",
            resp.status()
        )));
    }
    // Extract content-type before consuming the response.
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp
        .text()
        .map_err(|e| ManifestError::Unreachable(e.to_string()))?;

    // If content-type is clearly HTML and body isn't JSON, fail early.
    if ct.contains("html") && !body.trim_start().starts_with('{') {
        return Err(ManifestError::Unreachable(
            "response is HTML, not JSON".to_string(),
        ));
    }

    let raw: RawManifest = serde_json::from_str(&body)
        .map_err(|e| ManifestError::InvalidJson(e.to_string()))?;
    let mut parsed = parse(raw)?;
    parsed.addon.url = url.to_string();
    Ok(parsed)
}

/// Scan an HTML page body for Stremio manifest links.
fn find_manifest_in_html(html: &str, base: &str) -> Option<String> {
    // Look for stremio:// deep links: stremio://host/manifest.json
    if let Some(pos) = html.find("stremio://") {
        let after = &html[pos..];
        let end = after
            .find(|c: char| c == '"' || c == '\'' || c == ' ' || c == '<')
            .unwrap_or(after.len());
        let link = &after[..end];
        let https = link.replacen("stremio://", "https://", 1);
        return Some(https);
    }

    // Look for href="...manifest.json" or src="...manifest.json"
    if let Some(pos) = html.find("manifest.json") {
        let before = &html[..pos];
        if let Some(quote_start) = before.rfind('"').or_else(|| before.rfind('\'')) {
            let url_start = quote_start + 1;
            let potential = &html[url_start..pos + "manifest.json".len()];
            if potential.starts_with("http") {
                return Some(potential.to_string());
            }
            if potential.starts_with('/') {
                return Some(format!("{}{}", base, potential));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_stremio_scheme() {
        let s = normalize_url("stremio://addon.example.com/manifest.json").unwrap();
        assert_eq!(s, "https://addon.example.com/manifest.json");
    }

    #[test]
    fn normalize_bare_origin_appends_manifest() {
        let s = normalize_url("https://addon.example.com").unwrap();
        assert_eq!(s, "https://addon.example.com/manifest.json");
    }

    #[test]
    fn normalize_rejects_ftp() {
        assert!(normalize_url("ftp://example.com/manifest.json").is_err());
    }

    #[test]
    fn normalize_trims_trailing_slashes() {
        let s = normalize_url("https://example.com////").unwrap();
        assert_eq!(s, "https://example.com/manifest.json");
    }

    #[test]
    fn parse_requires_resources() {
        let raw: RawManifest = serde_json::from_value(serde_json::json!({
            "id": "x", "version": "1.0.0", "name": "x"
        }))
        .unwrap();
        assert!(matches!(parse(raw), Err(ManifestError::NoResources)));
    }

    #[test]
    fn parse_requires_name() {
        let raw: RawManifest = serde_json::from_value(serde_json::json!({
            "id": "x", "version": "1.0.0", "name": "", "resources": ["stream"]
        }))
        .unwrap();
        assert!(matches!(parse(raw), Err(ManifestError::MissingField("name"))));
    }
}
