//! Stremio stream requests: `GET /{type}/{id}/stream.json`.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::StremioAddon;

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StreamsResponse {
    #[serde(default)]
    pub streams: Vec<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("request failed: {0}")]
    Request(String),
}

pub fn fetch(addon: &StremioAddon, kind: &str, id: &str) -> Result<StreamsResponse, StreamError> {
    let base = addon
        .url
        .trim_end_matches("/manifest.json")
        .trim_end_matches('/');
    let url = format!("{}/{}/{}/stream.json", base, kind, id);
    let client = crate::http_client::shared_client();
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(15))
        .send()
        .map_err(|e| StreamError::Request(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(StreamError::Request(format!("status {}", resp.status())));
    }
    let body: StreamsResponse = resp
        .json()
        .map_err(|e| StreamError::Request(e.to_string()))?;
    Ok(body)
}
