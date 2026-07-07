//! Tests the AddonStore round-trip with realistic addon records.

#[cfg(test)]
mod tests {
    use crate::stremio::store::AddonStore;
    use std::fs;

    #[test]
    fn load_generic_addon_record() {
        let json = r#"{
  "version": 0,
  "addons": [
    {
      "id": "org.example.addon",
      "url": "https://example.com/manifest.json",
      "name": "Test Addon",
      "version": "1.0.0",
      "description": "A test addon",
      "logo": "https://example.com/logo.png",
      "types": ["movie", "series"],
      "resources": ["stream"],
      "idPrefixes": ["tt"],
      "catalogs": [],
      "config": null,
      "status": "available",
      "installedAt": "2026-07-07T04:08:42.331484800Z",
      "lastCheckedAt": "2026-07-07T04:08:42.331488800Z"
    }
  ]
}"#;

        let parsed: Result<crate::stremio::store::AddonFile, _> = serde_json::from_str(json);
        if let Err(e) = &parsed {
            eprintln!("Deserialize error: {}", e);
        }
        assert!(parsed.is_ok(), "Should parse: {:?}", parsed.err());
        let parsed = parsed.unwrap();
        assert_eq!(parsed.addons.len(), 1);
        assert_eq!(parsed.addons[0].id, "org.example.addon");
    }

    #[test]
    fn load_real_user_file() {
        let path = crate::stremio::addon_file_path();
        if !path.exists() {
            eprintln!("Skipping: no addon file at {:?}", path);
            return;
        }
        let s = fs::read_to_string(&path).unwrap();
        let parsed: Result<crate::stremio::store::AddonFile, _> = serde_json::from_str(&s);
        match parsed {
            Ok(file) => {
                assert!(!file.addons.is_empty(), "addon file should have entries");
                eprintln!("Loaded {} addons", file.addons.len());
                for a in &file.addons {
                    eprintln!("  - {} ({}): {} resources", a.id, a.name, a.resources.len());
                }
            }
            Err(e) => {
                panic!("Could not parse addon file at {:?}: {}", path, e);
            }
        }
    }

    #[test]
    fn upsert_through_store_preserves() {
        let tmp = std::env::temp_dir().join(format!(
            "slasshy-stremio-round-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = AddonStore {
            file: tmp.clone(),
            inner: std::sync::Mutex::new(crate::stremio::store::AddonFile::default()),
        };
        let test_json = r#"{
  "id": "org.example.addon",
  "url": "https://example.com/manifest.json",
  "name": "Test Addon",
  "version": "1.0.0",
  "description": "A test addon",
  "logo": "https://example.com/logo.png",
  "types": ["movie", "series"],
  "resources": ["stream"],
  "idPrefixes": ["tt"],
  "catalogs": [],
  "config": null,
  "status": "available",
  "installedAt": "2026-07-07T04:08:42.331484800Z",
  "lastCheckedAt": "2026-07-07T04:08:42.331488800Z"
}"#;
        let addon: crate::stremio::StremioAddon = serde_json::from_str(test_json).unwrap();
        store.upsert(addon).unwrap();
        let listed = store.list();
        assert_eq!(listed.len(), 1, "list should have one addon");
        assert_eq!(listed[0].id, "org.example.addon");
        let _ = fs::remove_file(&tmp);
    }
}
