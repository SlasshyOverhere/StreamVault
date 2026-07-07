//! Live network test for real-world Stremio addon manifest parsing.
//! Run with: cargo test live_manifest -- --ignored

#[cfg(test)]
mod tests {
    use crate::stremio::manifest;

    #[test]
    #[ignore = "live network test; run with `cargo test live_manifest -- --ignored`"]
    fn fetch_live_manifest() {
        let url = std::env::var("STREMIO_TEST_MANIFEST_URL")
            .unwrap_or_else(|_| "https://example.com/manifest.json".to_string());
        let parsed = manifest::fetch_and_parse(&url)
            .expect("manifest should parse");
        assert!(!parsed.addon.id.is_empty());
        assert!(!parsed.addon.resources.is_empty());
    }
}
