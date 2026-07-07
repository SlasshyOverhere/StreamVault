//! Stremio `/configure` flow support.
//!
//! The webview itself is owned by the frontend (a Tauri WebviewWindow). This
//! module is responsible for the *result*: the frontend passes back a
//! captured `config` string and we re-fetch the manifest to make sure the
//! addon is now resolvable, then store both manifest and config.

use super::manifest::{fetch_and_parse, ManifestError};
use super::StremioAddon;

#[derive(Debug, thiserror::Error)]
pub enum ConfigureError {
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
}

/// Re-fetch a manifest and produce a `StremioAddon` with the given
/// configuration string attached.
pub fn resolve(url: &str, config: Option<String>) -> Result<StremioAddon, ConfigureError> {
    let parsed = fetch_and_parse(url)?;
    let mut addon = parsed.addon;
    addon.config = config;
    Ok(addon)
}

/// Sanity helper: returns the URL the frontend should open for the
/// `/configure` webview. Most addons expose it at `<base>/configure`.
pub fn configure_url(manifest_url: &str) -> String {
    let base = manifest_url
        .trim_end_matches("/manifest.json")
        .trim_end_matches('/');
    format!("{}/configure", base)
}
