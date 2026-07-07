//! Stremio catalog requests: `GET /{type}/{catalogId}/catalog.json`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use super::StremioAddon;

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CatalogResponse {
    #[serde(default)]
    pub metas: Vec<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("addon does not provide the catalog resource")]
    UnsupportedResource,
    #[error("request failed: {0}")]
    Request(String),
}

/// Fetch a catalog from the addon. `extra` is a query-string map.
pub fn fetch(
    addon: &StremioAddon,
    kind: &str,
    catalog_id: &str,
    extra: &HashMap<String, String>,
) -> Result<CatalogResponse, CatalogError> {
    let base = base_url(&addon.url)?;
    let mut url = format!(
        "{}/{}/{}/catalog.json",
        base.trim_end_matches('/'),
        kind,
        catalog_id
    );
    if !extra.is_empty() {
        let mut pairs: Vec<(String, String)> = extra
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        let qs = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();
        url.push('?');
        url.push_str(&qs);
    }
    let client = crate::http_client::shared_client();
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| CatalogError::Request(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(CatalogError::Request(format!("status {}", resp.status())));
    }
    let body: CatalogResponse = resp
        .json()
        .map_err(|e| CatalogError::Request(e.to_string()))?;
    Ok(body)
}

fn base_url(manifest_url: &str) -> Result<String, CatalogError> {
    let trimmed = manifest_url
        .trim_end_matches("/manifest.json")
        .trim_end_matches('/');
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_strips_manifest() {
        assert_eq!(
            base_url("https://x.com/manifest.json").unwrap(),
            "https://x.com"
        );
        assert_eq!(
            base_url("https://x.com/sub/manifest.json").unwrap(),
            "https://x.com/sub"
        );
    }
}
