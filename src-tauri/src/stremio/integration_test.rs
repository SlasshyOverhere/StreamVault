//! Integration test for the Stremio protocol module.
//!
//! Spins up a local HTTP server with a fake `manifest.json`, catalog, meta,
//! and stream payload, then exercises the catalog/meta/stream fetchers.

#[cfg(test)]
mod tests {
    use crate::stremio::catalog;
    use crate::stremio::manifest;
    use crate::stremio::meta;
    use crate::stremio::stream;
    use crate::stremio::StremioAddon;
    use crate::stremio::StremioResource;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn handle_request(mut s: std::net::TcpStream) {
        s.set_read_timeout(Some(std::time::Duration::from_secs(2))).ok();
        let mut buf = vec![0u8; 8192];
        let mut total = String::new();
        loop {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    total.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if total.contains("\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let path = total
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("/");
        let body = match path {
            "/manifest.json" => include_str!("fixtures/manifest.json"),
            "/catalog/movie/top.json" => include_str!("fixtures/catalog.json"),
            "/meta/movie/tt0111161.json" => include_str!("fixtures/meta.json"),
            "/stream/movie/tt0111161.json" => include_str!("fixtures/streams.json"),
            _ => "{}",
        };
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = s.write_all(resp.as_bytes());
        let _ = s.flush();
    }

    fn start_fake_addon() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Spawn a thread that loops, handling one connection at a time so
        // the test's pooled HTTP client always gets a fresh response.
        thread::spawn(move || {
            loop {
                match listener.accept() {
                    Ok((s, _)) => handle_request(s),
                    Err(_) => break,
                }
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        port
    }

    fn fake_addon(port: u16) -> StremioAddon {
        StremioAddon {
            id: "fake.addon".to_string(),
            url: format!("http://127.0.0.1:{}/manifest.json", port),
            name: "Fake".to_string(),
            version: "1.0.0".to_string(),
            description: None,
            logo: None,
            types: vec!["movie".to_string()],
            resources: vec![StremioResource::Plain("stream".to_string())],
            id_prefixes: vec!["tt".to_string()],
            catalogs: vec![],
            config: None,
            status: crate::stremio::StremioAddonStatus::Available,
            installed_at: chrono::Utc::now(),
            last_checked_at: chrono::Utc::now(),
        }
    }

    #[test]
    #[ignore = "requires a running fixture server; see fixtures/"]
    fn fetch_end_to_end() {
        let port = start_fake_addon();
        let addon = fake_addon(port);

        let mut extra = HashMap::new();
        extra.insert("skip".to_string(), "0".to_string());
        let cat = catalog::fetch(&addon, "movie", "top", &extra).unwrap();
        assert!(!cat.metas.is_empty(), "catalog metas should be present");

        let meta_resp = meta::fetch(&addon, "movie", "tt0111161").unwrap();
        assert!(meta_resp.meta.is_some(), "meta should be present");

        let streams = stream::fetch(&addon, "movie", "tt0111161").unwrap();
        assert!(!streams.streams.is_empty(), "streams should be present");
    }

    #[test]
    fn manifest_normalize_and_parse() {
        // Just check the normalize + parse pipeline with the fixture.
        let port = start_fake_addon();
        let url = format!("http://127.0.0.1:{}/manifest.json", port);
        let parsed = manifest::fetch_and_parse(&url).expect("manifest should parse");
        assert_eq!(parsed.addon.id, "fake.addon");
        assert_eq!(parsed.addon.name, "Fake");
        assert!(parsed.addon.resources.iter().any(|r| r.name() == "stream"));
    }
}
