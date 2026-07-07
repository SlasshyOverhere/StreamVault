//! On-demand health probes for installed Stremio addons.

use std::time::Duration;

use super::StremioAddonStatus;

/// Probe a single addon's manifest URL with a HEAD request.
/// A 5s timeout keeps the UI snappy.
pub fn probe(url: &str) -> StremioAddonStatus {
    let client = crate::http_client::quick_client();
    match client
        .head(url)
        .timeout(Duration::from_secs(5))
        .send()
    {
        Ok(resp) if resp.status().is_success() || resp.status().is_redirection() => {
            StremioAddonStatus::Available
        }
        Ok(_) => StremioAddonStatus::Unavailable,
        Err(_) => StremioAddonStatus::Unavailable,
    }
}
