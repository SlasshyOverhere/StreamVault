//! Stremio meta requests: `GET /{type}/{id}/meta.json`.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::StremioAddon;

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MetaResponse {
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum MetaError {
    #[error("request failed: {0}")]
    Request(String),
}

pub fn fetch(addon: &StremioAddon, kind: &str, id: &str) -> Result<MetaResponse, MetaError> {
    let base = addon.url.trim_end_matches("/manifest.json").trim_end_matches('/');
    let url = format!("{}/{}/{}/meta.json", base, kind, id);
    let client = crate::http_client::shared_client();
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| MetaError::Request(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(MetaError::Request(format!("status {}", resp.status())));
    }
    let body: MetaResponse = resp
        .json()
        .map_err(|e| MetaError::Request(e.to_string()))?;
    Ok(body)
}
