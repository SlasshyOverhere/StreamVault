use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// IMPORTANT LIMITATION: Only one proxy can be active at a time.
/// Starting a new stream via `set_active_proxy` will stop and replace the
/// previous one. This is a known design constraint — a concurrent multi-proxy
/// design would require per-stream routing and is out of scope for now.
static ACTIVE_PROXY: Mutex<Option<RemoteStreamProxyHandle>> = Mutex::new(None);

/// Allowed Content-Type prefixes for proxied responses.
const ALLOWED_CONTENT_TYPES: &[&str] = &[
    "video/",
    "audio/",
    "application/octet-stream",
    "application/x-matroska",
    "application/vnd.rn-realmedia",
];

pub fn set_active_proxy(handle: RemoteStreamProxyHandle) {
    if let Ok(mut guard) = ACTIVE_PROXY.lock() {
        if let Some(prev) = guard.take() {
            prev.stop();
        }
        *guard = Some(handle);
    }
}

pub fn clear_active_proxy() {
    if let Ok(mut guard) = ACTIVE_PROXY.lock() {
        if let Some(prev) = guard.take() {
            prev.stop();
        }
    }
}

pub struct RemoteStreamProxyHandle {
    pub port: u16,
    pub url: String,
    stop_flag: Arc<AtomicBool>,
    /// Tracks spawned threads so they can be joined on cleanup (P2 fix).
    thread_handles: Vec<std::thread::JoinHandle<()>>,
}

impl RemoteStreamProxyHandle {
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
    pub fn localhost_url(&self) -> String {
        format!("http://127.0.0.1:{}/stream", self.port)
    }
}

impl Drop for RemoteStreamProxyHandle {
    fn drop(&mut self) {
        self.stop();
        // Join all spawned threads to prevent leaked threads on cleanup.
        for handle in self.thread_handles.drain(..) {
            let _ = handle.join();
        }
    }
}

struct ProxyState {
    file_path: std::path::PathBuf,
    final_path: std::path::PathBuf,
    total_size: AtomicU64,
    downloaded: AtomicU64,
    complete: AtomicBool,
    started: AtomicBool,
    error: RwLock<Option<String>>,
}

pub fn start_proxy(
    url: String,
    cache_dir: std::path::PathBuf,
    cache_key: String,
    title: String,
    cache_path: Option<std::path::PathBuf>,
) -> Result<RemoteStreamProxyHandle, String> {
    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|e| format!("Failed to start proxy: {}", e))?;
    let port = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr.port(),
        #[cfg(unix)]
        tiny_http::ListenAddr::Unix(_) => return Err("UNIX socket not supported".to_string()),
    };

    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("create_dir: {}", e))?;

    // Use the provided cache_path to align with main.rs, or fall back to old naming
    let (file_path, final_path) = if let Some(ref cp) = cache_path {
        let parent = cp.parent().unwrap_or(&cache_dir);
        let stem = cp.file_stem().unwrap_or_default().to_string_lossy();
        (parent.join(format!("{}.part", stem)), cp.clone())
    } else {
        let sanitized = sanitize_filename(&title);
        let file_name = format!("{}_{}", cache_key, sanitized);
        (
            cache_dir.join(format!("{}.part", file_name)),
            cache_dir.join(format!("{}.mkv", file_name)),
        )
    };

    if final_path.exists() {
        println!("[REMOTE-PROXY] Already cached: {}", final_path.display());
    }

    let state = Arc::new(ProxyState {
        file_path: file_path.clone(),
        final_path: final_path.clone(),
        total_size: AtomicU64::new(0),
        downloaded: AtomicU64::new(0),
        complete: AtomicBool::new(final_path.exists()),
        started: AtomicBool::new(false),
        error: RwLock::new(None),
    });

    let stop_flag = Arc::new(AtomicBool::new(false));

    // Download thread
    let dl_state = state.clone();
    let dl_url = url.clone();
    let dl_stop = stop_flag.clone();
    let dl_handle = std::thread::spawn(move || {
        // Polling loop with 50ms sleep to wait for the first request — not a busy-wait.
        while !dl_state.started.load(Ordering::Relaxed) && !dl_stop.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if dl_stop.load(Ordering::Relaxed) || dl_state.complete.load(Ordering::Relaxed) {
            return;
        }

        println!("[REMOTE-PROXY] Downloading: {}", dl_url);

        let client = crate::http_client::proxy_client();

        let resp = match client.get(&dl_url).send() {
            Ok(r) => r,
            Err(e) => {
                *dl_state.error.write().unwrap_or_else(|e| e.into_inner()) =
                    Some(format!("Request: {}", e));
                println!("[REMOTE-PROXY] Download request failed: {}", e);
                return;
            }
        };

        println!(
            "[REMOTE-PROXY] Upstream responded: {} {} → {}",
            resp.status().as_u16(),
            resp.status().canonical_reason().unwrap_or(""),
            resp.url()
        );

        if !resp.status().is_success() {
            *dl_state.error.write().unwrap_or_else(|e| e.into_inner()) =
                Some(format!("HTTP {}", resp.status()));
            return;
        }

        // P1 fix: Validate Content-Type of the upstream response is media.
        let upstream_ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !ALLOWED_CONTENT_TYPES
            .iter()
            .any(|allowed| upstream_ct.starts_with(allowed))
        {
            println!(
                "[REMOTE-PROXY] WARNING: unexpected Content-Type from upstream: '{}'",
                upstream_ct
            );
        }

        let total = resp.content_length().unwrap_or(0);
        dl_state.total_size.store(total, Ordering::Relaxed);

        let mut file = match std::fs::File::create(&dl_state.file_path) {
            Ok(f) => f,
            Err(e) => {
                *dl_state.error.write().unwrap_or_else(|e| e.into_inner()) =
                    Some(format!("File: {}", e));
                return;
            }
        };

        let mut body = resp;
        let mut buf = vec![0u8; 256 * 1024];
        let mut written: u64 = 0;

        loop {
            if dl_stop.load(Ordering::Relaxed) {
                return;
            }
            match body.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // P0-3 fix: surface write errors to the frontend instead of silently dropping.
                    if let Err(e) = file.write_all(&buf[..n]) {
                        *dl_state.error.write().unwrap_or_else(|e| e.into_inner()) =
                            Some(format!("Write: {}", e));
                        return;
                    }
                    written += n as u64;
                    dl_state.downloaded.store(written, Ordering::Relaxed);
                }
                // P0-3 fix: surface read errors to the frontend instead of silently returning.
                Err(e) => {
                    *dl_state.error.write().unwrap_or_else(|e| e.into_inner()) =
                        Some(format!("Read: {}", e));
                    return;
                }
            }
        }

        let _ = file.flush();
        dl_state.complete.store(true, Ordering::Relaxed);
        let _ = std::fs::rename(&dl_state.file_path, &dl_state.final_path);
        println!("[REMOTE-PROXY] Done: {} bytes", written);
    });

    // Request handler thread — one thread per request for concurrent seeking
    let req_state = state.clone();
    let req_stop = stop_flag.clone();

    let server_handle = std::thread::spawn(move || {
        println!(
            "[REMOTE-PROXY] Listening on http://127.0.0.1:{}/stream",
            port
        );
        for request in server.incoming_requests() {
            if req_stop.load(Ordering::Relaxed) {
                break;
            }
            let path = request.url().split('?').next().unwrap_or("").to_string();
            if path != "/stream" {
                let response = tiny_http::Response::from_string("Not Found")
                    .with_status_code(404)
                    .with_header(
                        tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
                    );
                let _ = request.respond(response);
                continue;
            }
            req_state.started.store(true, Ordering::Relaxed);
            let s = req_state.clone();
            // Note: per-request threads are fire-and-forget since they are short-lived
            // and blocked on I/O. Joining them would require a more complex architecture.
            std::thread::spawn(move || serve_request(request, &s));
        }
    });

    Ok(RemoteStreamProxyHandle {
        port,
        url,
        stop_flag,
        thread_handles: vec![dl_handle, server_handle],
    })
}

fn serve_request(request: tiny_http::Request, state: &Arc<ProxyState>) {
    // Wait for some data to be available (polling with 100ms sleep — not a busy-wait)
    let mut waited = 0;
    while state.downloaded.load(Ordering::Relaxed) == 0
        && !state.complete.load(Ordering::Relaxed)
        && waited < 30000
    {
        // P1 fix: use unwrap_or_else to survive poisoned locks without panicking.
        if let Some(err) = state
            .error
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let response = tiny_http::Response::from_string(format!("Error: {}", err))
                .with_status_code(502)
                .with_header(
                    tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
                );
            let _ = request.respond(response);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        waited += 100;
    }

    if state.downloaded.load(Ordering::Relaxed) == 0 {
        let response = tiny_http::Response::from_string("Timeout")
            .with_status_code(504)
            .with_header(
                tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
            );
        let _ = request.respond(response);
        return;
    }

    // Determine file to read from
    let read_path = if state.final_path.exists() {
        state.final_path.clone()
    } else {
        state.file_path.clone()
    };

    let total_size = state.total_size.load(Ordering::Relaxed);

    // Parse Range header
    let range_header = request
        .headers()
        .iter()
        .find(|h| h.field.as_str().to_ascii_lowercase() == "range")
        .map(|h| h.value.as_str().to_string());

    // ponytail: only use Range serving when we know the real total size (upstream sent valid Content-Length).
    // When total_size is implausibly small (0 or 1 byte for a video), Range would produce wrong values.
    if let Some(range_str) = range_header.filter(|_| total_size > 1024) {
        // Wait until enough data is available for the requested range (polling with 200ms sleep)
        if let Some((start, end)) = parse_range(&range_str, total_size.max(1)) {
            let needed = end + 1;
            let mut waited = 0;
            while state.downloaded.load(Ordering::Relaxed) < needed
                && !state.complete.load(Ordering::Relaxed)
                && waited < 120000
            {
                std::thread::sleep(std::time::Duration::from_millis(200));
                waited += 200;
            }

            let available = state.downloaded.load(Ordering::Relaxed);
            if available < start + 1 {
                let response = tiny_http::Response::from_string("Range Not Satisfiable")
                    .with_status_code(416)
                    .with_header(
                        tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
                    );
                let _ = request.respond(response);
                return;
            }

            let actual_end = end.min(available - 1);
            let length = actual_end - start + 1;

            // Stream the range in chunks instead of reading all into memory
            let file = std::fs::File::open(&read_path);
            let mut file = match file {
                Ok(f) => f,
                Err(e) => {
                    let response = tiny_http::Response::from_string(format!("File error: {}", e))
                        .with_status_code(500)
                        .with_header(
                            tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*")
                                .unwrap(),
                        );
                    let _ = request.respond(response);
                    return;
                }
            };

            if file.seek(SeekFrom::Start(start)).is_err() {
                let response = tiny_http::Response::from_string("Seek error")
                    .with_status_code(500)
                    .with_header(
                        tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
                    );
                let _ = request.respond(response);
                return;
            }

            let content_range = format!(
                "bytes {}-{}/{}",
                start,
                actual_end,
                total_size.max(available)
            );
            let ctype = content_type_for_path(&read_path);
            let response = tiny_http::Response::new(
                tiny_http::StatusCode::from(206),
                vec![
                    tiny_http::Header::from_bytes("Content-Type", ctype).unwrap(),
                    tiny_http::Header::from_bytes("Content-Length", length.to_string()).unwrap(),
                    tiny_http::Header::from_bytes("Content-Range", content_range).unwrap(),
                    tiny_http::Header::from_bytes("Accept-Ranges", "bytes").unwrap(),
                    tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
                ],
                file.take(length),
                Some(length as usize),
                None,
            );
            let _ = request.respond(response);
            return;
        }
    }

    // Full request — stream from file
    // ponytail: when upstream sent no Content-Length (chunked), wait for download to finish
    // so we can serve the complete file with a correct Content-Length.
    if total_size <= 1024 {
        let mut waited = 0;
        while !state.complete.load(Ordering::Relaxed) && waited < 120000 {
            if let Some(err) = state
                .error
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                let response = tiny_http::Response::from_string(format!("Error: {}", err))
                    .with_status_code(502)
                    .with_header(
                        tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
                    );
                let _ = request.respond(response);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            waited += 200;
        }
    }

    let file = match std::fs::File::open(&read_path) {
        Ok(f) => f,
        Err(e) => {
            let response = tiny_http::Response::from_string(format!("File error: {}", e))
                .with_status_code(500)
                .with_header(
                    tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
                );
            let _ = request.respond(response);
            return;
        }
    };

    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let ctype = content_type_for_path(&read_path);
    let response = tiny_http::Response::new(
        tiny_http::StatusCode::from(200),
        vec![
            tiny_http::Header::from_bytes("Content-Type", ctype).unwrap(),
            tiny_http::Header::from_bytes("Content-Length", file_size.to_string()).unwrap(),
            tiny_http::Header::from_bytes("Accept-Ranges", "bytes").unwrap(),
            tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
        ],
        file,
        Some(file_size as usize),
        None,
    );
    let _ = request.respond(response);
}

/// Determine content type from file path extension
fn content_type_for_path(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "ts" => "video/mp2t",
        "flv" => "video/x-flv",
        "wmv" => "video/x-ms-wmv",
        "mpeg" | "mpg" => "video/mpeg",
        "3gp" => "video/3gpp",
        "ogv" => "video/ogg",
        _ => "video/mp4",
    }
}

fn parse_range(range_str: &str, file_size: u64) -> Option<(u64, u64)> {
    let range_str = range_str.trim();
    if !range_str.starts_with("bytes=") {
        return None;
    }
    let range_val = &range_str[6..];
    let parts: Vec<&str> = range_val.split('-').collect();
    if parts.len() != 2 {
        return None;
    }

    if parts[0].is_empty() {
        let suffix: u64 = parts[1].parse().ok()?;
        let start = file_size.saturating_sub(suffix);
        Some((start, file_size - 1))
    } else if parts[1].is_empty() {
        let start: u64 = parts[0].parse().ok()?;
        Some((start, file_size - 1))
    } else {
        let start: u64 = parts[0].parse().ok()?;
        let end: u64 = parts[1].parse().ok()?;
        Some((start, end))
    }
}

fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .chars()
        .take(50)
        .collect()
}

pub async fn resolve_redirects(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("Client build: {}", e))?;

    let resp = client
        .head(url)
        .send()
        .await
        .map_err(|e| format!("HEAD failed: {}", e))?;

    let final_url = resp.url().to_string();
    println!("[REMOTE-PROXY] Resolved: {} -> {}", url, final_url);
    Ok(final_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    // ── parse_range ────────────────────────────────────────────────────

    #[test]
    fn parse_range_valid_start_end() {
        assert_eq!(parse_range("bytes=0-499", 1000), Some((0, 499)));
    }

    #[test]
    fn parse_range_valid_middle() {
        assert_eq!(parse_range("bytes=500-999", 1000), Some((500, 999)));
    }

    #[test]
    fn parse_range_suffix_last_500() {
        // bytes=-500 → last 500 bytes: start=500, end=999
        assert_eq!(parse_range("bytes=-500", 1000), Some((500, 999)));
    }

    #[test]
    fn parse_range_suffix_larger_than_file() {
        // bytes=-2000 on 1000-byte file → start=0, end=999
        assert_eq!(parse_range("bytes=-2000", 1000), Some((0, 999)));
    }

    #[test]
    fn parse_range_open_ended() {
        // bytes=950- → start=950, end=file_size-1
        assert_eq!(parse_range("bytes=950-", 1000), Some((950, 999)));
    }

    #[test]
    fn parse_range_exact_byte() {
        assert_eq!(parse_range("bytes=0-0", 1000), Some((0, 0)));
    }

    #[test]
    fn parse_range_missing_prefix() {
        assert_eq!(parse_range("0-499", 1000), None);
    }

    #[test]
    fn parse_range_wrong_unit() {
        assert_eq!(parse_range("bits=0-499", 1000), None);
    }

    #[test]
    fn parse_range_no_dash() {
        assert_eq!(parse_range("bytes=500", 1000), None);
    }

    #[test]
    fn parse_range_empty_string() {
        assert_eq!(parse_range("", 1000), None);
    }

    #[test]
    fn parse_range_multiple_dashes() {
        // "bytes=1-2-3" splits into 3 parts → None
        assert_eq!(parse_range("bytes=1-2-3", 1000), None);
    }

    #[test]
    fn parse_range_non_numeric() {
        assert_eq!(parse_range("bytes=abc-def", 1000), None);
    }

    #[test]
    fn parse_range_suffix_zero() {
        // bytes=-0 → suffix=0, start=1000, end=999 → saturating_sub gives 1000
        // start=1000 but file_size-1=999, so (1000, 999) — semantically empty range
        assert_eq!(parse_range("bytes=-0", 1000), Some((1000, 999)));
    }

    #[test]
    fn parse_range_whitespace_trimmed() {
        assert_eq!(parse_range("  bytes=0-499  ", 1000), Some((0, 499)));
    }

    // ── sanitize_filename ──────────────────────────────────────────────

    #[test]
    fn sanitize_filename_alphanumeric_passthrough() {
        assert_eq!(sanitize_filename("hello123"), "hello123");
    }

    #[test]
    fn sanitize_filename_preserves_dash_underscore() {
        assert_eq!(sanitize_filename("my-file_name"), "my-file_name");
    }

    #[test]
    fn sanitize_filename_replaces_special_chars() {
        assert_eq!(sanitize_filename("a b/c\\d:e"), "a_b_c_d_e");
    }

    #[test]
    fn sanitize_filename_empty() {
        assert_eq!(sanitize_filename(""), "");
    }

    #[test]
    fn sanitize_filename_all_special() {
        assert_eq!(sanitize_filename("///:::   "), "_________");
    }

    #[test]
    fn sanitize_filename_truncates_at_50() {
        let long = "a".repeat(100);
        assert_eq!(sanitize_filename(&long).len(), 50);
    }

    #[test]
    fn sanitize_filename_exactly_50() {
        let s = "a".repeat(50);
        assert_eq!(sanitize_filename(&s).len(), 50);
        assert_eq!(sanitize_filename(&s), s);
    }

    #[test]
    fn sanitize_filename_unicode_letters_kept() {
        // Rust's is_alphanumeric() includes Unicode letters, so é is kept
        assert_eq!(sanitize_filename("café"), "café");
    }

    #[test]
    fn sanitize_filename_unicode_symbols_replaced() {
        // Symbols like emojis are not alphanumeric
        assert_eq!(sanitize_filename("a★b"), "a_b");
    }

    // ── content_type_for_path ──────────────────────────────────────────

    #[test]
    fn content_type_mp4() {
        assert_eq!(content_type_for_path(Path::new("video.mp4")), "video/mp4");
    }

    #[test]
    fn content_type_mkv() {
        assert_eq!(
            content_type_for_path(Path::new("video.mkv")),
            "video/x-matroska"
        );
    }

    #[test]
    fn content_type_webm() {
        assert_eq!(content_type_for_path(Path::new("video.webm")), "video/webm");
    }

    #[test]
    fn content_type_avi() {
        assert_eq!(
            content_type_for_path(Path::new("video.avi")),
            "video/x-msvideo"
        );
    }

    #[test]
    fn content_type_mov() {
        assert_eq!(
            content_type_for_path(Path::new("video.mov")),
            "video/quicktime"
        );
    }

    #[test]
    fn content_type_ts() {
        assert_eq!(content_type_for_path(Path::new("video.ts")), "video/mp2t");
    }

    #[test]
    fn content_type_flv() {
        assert_eq!(content_type_for_path(Path::new("video.flv")), "video/x-flv");
    }

    #[test]
    fn content_type_wmv() {
        assert_eq!(
            content_type_for_path(Path::new("video.wmv")),
            "video/x-ms-wmv"
        );
    }

    #[test]
    fn content_type_mpeg() {
        assert_eq!(content_type_for_path(Path::new("video.mpeg")), "video/mpeg");
    }

    #[test]
    fn content_type_mpg() {
        assert_eq!(content_type_for_path(Path::new("video.mpg")), "video/mpeg");
    }

    #[test]
    fn content_type_3gp() {
        assert_eq!(content_type_for_path(Path::new("video.3gp")), "video/3gpp");
    }

    #[test]
    fn content_type_ogv() {
        assert_eq!(content_type_for_path(Path::new("video.ogv")), "video/ogg");
    }

    #[test]
    fn content_type_m4v() {
        assert_eq!(content_type_for_path(Path::new("video.m4v")), "video/mp4");
    }

    #[test]
    fn content_type_unknown_defaults_to_mp4() {
        assert_eq!(content_type_for_path(Path::new("file.xyz")), "video/mp4");
    }

    #[test]
    fn content_type_no_extension() {
        assert_eq!(content_type_for_path(Path::new("noext")), "video/mp4");
    }

    // ── ALLOWED_CONTENT_TYPES ──────────────────────────────────────────

    #[test]
    fn allowed_content_types_include_video() {
        assert!(ALLOWED_CONTENT_TYPES
            .iter()
            .any(|ct| ct.starts_with("video/")));
    }

    #[test]
    fn allowed_content_types_include_audio() {
        assert!(ALLOWED_CONTENT_TYPES
            .iter()
            .any(|ct| ct.starts_with("audio/")));
    }

    #[test]
    fn allowed_content_types_include_octet_stream() {
        assert!(ALLOWED_CONTENT_TYPES.contains(&"application/octet-stream"));
    }

    #[test]
    fn allowed_content_types_include_matroska() {
        assert!(ALLOWED_CONTENT_TYPES.contains(&"application/x-matroska"));
    }

    #[test]
    fn allowed_content_types_include_realmedia() {
        assert!(ALLOWED_CONTENT_TYPES.contains(&"application/vnd.rn-realmedia"));
    }

    // ── RemoteStreamProxyHandle ────────────────────────────────────────

    #[test]
    fn handle_localhost_url_format() {
        let handle = RemoteStreamProxyHandle {
            port: 12345,
            url: "https://example.com/video.mp4".to_string(),
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread_handles: vec![],
        };
        assert_eq!(handle.localhost_url(), "http://127.0.0.1:12345/stream");
    }

    #[test]
    fn handle_stop_sets_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let handle = RemoteStreamProxyHandle {
            port: 0,
            url: String::new(),
            stop_flag: flag.clone(),
            thread_handles: vec![],
        };
        assert!(!flag.load(Ordering::Relaxed));
        handle.stop();
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn handle_stop_is_idempotent() {
        let flag = Arc::new(AtomicBool::new(false));
        let handle = RemoteStreamProxyHandle {
            port: 0,
            url: String::new(),
            stop_flag: flag.clone(),
            thread_handles: vec![],
        };
        handle.stop();
        handle.stop();
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn handle_drop_sets_flag_and_joins() {
        let flag = Arc::new(AtomicBool::new(false));
        let flag2 = flag.clone();
        // Spawn a quick thread that finishes immediately
        let t = std::thread::spawn(move || {
            flag2.store(true, Ordering::Relaxed);
        });
        let handle = RemoteStreamProxyHandle {
            port: 0,
            url: String::new(),
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread_handles: vec![t],
        };
        drop(handle);
        // Thread ran to completion
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn handle_url_stored() {
        let handle = RemoteStreamProxyHandle {
            port: 8080,
            url: "https://example.com/test.mkv".to_string(),
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread_handles: vec![],
        };
        assert_eq!(handle.url, "https://example.com/test.mkv");
        assert_eq!(handle.port, 8080);
    }

    // ── set_active_proxy / clear_active_proxy ──────────────────────────

    fn make_test_handle(port: u16) -> RemoteStreamProxyHandle {
        RemoteStreamProxyHandle {
            port,
            url: format!("https://example.com/{}", port),
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread_handles: vec![],
        }
    }

    #[test]
    fn set_active_proxy_stores_handle() {
        let handle = make_test_handle(9001);
        set_active_proxy(handle);
        // Verify it was stored by checking the lock
        let guard = ACTIVE_PROXY.lock().unwrap();
        assert!(guard.is_some());
        assert_eq!(guard.as_ref().unwrap().port, 9001);
        drop(guard);
        clear_active_proxy();
    }

    #[test]
    fn set_active_proxy_replaces_previous() {
        let h1 = make_test_handle(9002);
        let flag1 = h1.stop_flag.clone();
        set_active_proxy(h1);

        let h2 = make_test_handle(9003);
        set_active_proxy(h2);

        // Previous handle's stop flag should be set (stopped on replacement)
        assert!(flag1.load(Ordering::Relaxed));

        // Current handle is the new one
        let guard = ACTIVE_PROXY.lock().unwrap();
        assert_eq!(guard.as_ref().unwrap().port, 9003);
        drop(guard);
        clear_active_proxy();
    }

    #[test]
    fn clear_active_proxy_stops_and_removes() {
        let handle = make_test_handle(9004);
        let flag = handle.stop_flag.clone();
        set_active_proxy(handle);
        assert!(!flag.load(Ordering::Relaxed));

        clear_active_proxy();

        assert!(flag.load(Ordering::Relaxed));
        let guard = ACTIVE_PROXY.lock().unwrap();
        assert!(guard.is_none());
    }

    #[test]
    fn clear_active_proxy_when_empty_is_noop() {
        // Ensure clean state
        clear_active_proxy();
        clear_active_proxy(); // double-clear should not panic
        let guard = ACTIVE_PROXY.lock().unwrap();
        assert!(guard.is_none());
    }

    // ── start_proxy ────────────────────────────────────────────────────

    /// Stop the proxy and forget the handle so Drop doesn't block on joining
    /// the server thread (which blocks on incoming_requests).
    fn stop_and_forget(h: RemoteStreamProxyHandle) {
        h.stop();
        std::mem::forget(h);
    }

    #[test]
    fn start_proxy_returns_valid_handle() {
        let tmp = tempfile::tempdir().unwrap();
        let handle = start_proxy(
            "https://example.com/video.mp4".to_string(),
            tmp.path().to_path_buf(),
            "testkey".to_string(),
            "Test Video".to_string(),
            None,
        );
        assert!(handle.is_ok());
        let h = handle.unwrap();
        assert!(h.port > 0);
        assert_eq!(h.url, "https://example.com/video.mp4");
        assert!(!h.stop_flag.load(Ordering::Relaxed));
        assert_eq!(
            h.localhost_url(),
            format!("http://127.0.0.1:{}/stream", h.port)
        );
        stop_and_forget(h);
    }

    #[test]
    fn start_proxy_with_cache_path() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("my_video.mkv");
        let handle = start_proxy(
            "https://example.com/video.mp4".to_string(),
            tmp.path().to_path_buf(),
            "testkey".to_string(),
            "Test Video".to_string(),
            Some(cache_path.clone()),
        );
        assert!(handle.is_ok());
        stop_and_forget(handle.unwrap());
    }

    #[test]
    fn start_proxy_with_none_cache_path() {
        let tmp = tempfile::tempdir().unwrap();
        let handle = start_proxy(
            "https://example.com/video.mp4".to_string(),
            tmp.path().to_path_buf(),
            "key123".to_string(),
            "My Video Title".to_string(),
            None,
        );
        assert!(handle.is_ok());
        stop_and_forget(handle.unwrap());
    }

    #[test]
    fn start_proxy_creates_cache_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        assert!(!nested.exists());
        let handle = start_proxy(
            "https://example.com/video.mp4".to_string(),
            nested.clone(),
            "key".to_string(),
            "Title".to_string(),
            None,
        );
        assert!(handle.is_ok());
        assert!(nested.exists());
        stop_and_forget(handle.unwrap());
    }

    #[test]
    fn start_proxy_binds_to_free_port() {
        let tmp = tempfile::tempdir().unwrap();
        let result = start_proxy(
            "https://example.com/video.mp4".to_string(),
            tmp.path().to_path_buf(),
            "key".to_string(),
            "Title".to_string(),
            None,
        );
        assert!(result.is_ok());
        stop_and_forget(result.unwrap());
    }

    // ── resolve_redirects ──────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_redirects_invalid_url_returns_error() {
        let result = resolve_redirects("not_a_valid_url").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("HEAD failed"));
    }

    #[tokio::test]
    async fn resolve_redirects_empty_url_returns_error() {
        let result = resolve_redirects("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn resolve_redirects_unreachable_host_returns_error() {
        let result = resolve_redirects("http://192.0.2.1:1/test").await;
        assert!(result.is_err());
    }
}
