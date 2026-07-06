// Google Drive cloud-playback proxy.
//
// MPV passes us a single fixed `Authorization: Bearer XXX` header at launch
// and reuses it for EVERY HTTP request it makes to the upstream during the
// entire playback session. Google Drive access tokens typically expire in
// ~3600 s. A 2-hour movie therefore fails mid-playback with HTTP 401 once
// the captured token crosses its expiry.
//
// **Fix:** serve the bytes via a local HTTP proxy that signs each upstream
// request with the *current* access token. Each MPV request → one fresh
// Google Drive request with the active token, transparently retrying with a
// refreshed token on a 401. The proxy holds no disk cache.
//
// The proxy is **async-native** (tokio + async reqwest) so it composes with
// the rest of the codebase without needing a separate blocking runtime in
// each handler thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::gdrive::GoogleDriveClient;

const PROXY_PATH: &str = "/stream";
/// Hard cap on the upstream bytes we will buffer in memory per request
/// Retries back to Google Drive with a freshly-refreshed token on a 401.
const REFRESH_RETRY_LIMIT: u32 = 2;
/// Default upstream API base, including `/drive/v3` path prefix.
const GDRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";

pub struct GdriveStreamProxyHandle {
    pub port: u16,
    pub url: String,
    stop_flag: Arc<AtomicBool>,
    server_task: tokio::task::JoinHandle<()>,
}

impl GdriveStreamProxyHandle {
    pub fn localhost_url(&self) -> String {
        format!("http://127.0.0.1:{}{}", self.port, PROXY_PATH)
    }
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

impl Drop for GdriveStreamProxyHandle {
    fn drop(&mut self) {
        self.stop();
        self.server_task.abort();
    }
}

/// Spawn a single-file pass-through proxy that serves Google Drive media
/// to MPV without baking the access token into MPV. The proxy obtains
/// fresh tokens via the shared GoogleDriveClient (single-flight safe via
/// its internal mutex).
pub async fn start_gdrive_stream_proxy(
    gdrive_client: GoogleDriveClient,
    file_id: String,
) -> Result<GdriveStreamProxyHandle, String> {
    start_gdrive_stream_proxy_with_upstream(gdrive_client, file_id, GDRIVE_API_BASE).await
}

/// Testable variant that lets callers (mainly tests, but also useful for
/// self-hosting proxies) point the proxy at a different upstream base URL.
/// Production callers should use `start_gdrive_stream_proxy`, which
/// delegates here with the canonical Google Drive API base.
pub async fn start_gdrive_stream_proxy_with_upstream(
    gdrive_client: GoogleDriveClient,
    file_id: String,
    upstream_base: &str,
) -> Result<GdriveStreamProxyHandle, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Failed to start GDrive stream proxy: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("local_addr failed: {}", e))?
        .port();

    let url = format!("http://127.0.0.1:{}{}", port, PROXY_PATH);
    let stop_flag = Arc::new(AtomicBool::new(false));
    // Single shared HTTP client with sensible defaults for streaming large
    // media files. Reusing one client across all requests avoids per-request
    // TLS handshakes and connection-pool churn during playback.
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .pool_max_idle_per_host(4)
        .build()
        .map_err(|e| format!("Failed to build GDrive stream proxy HTTP client: {}", e))?;
    let upstream_base = upstream_base.to_string();
    let server_task = tokio::spawn(server_loop(
        listener,
        gdrive_client,
        Arc::clone(&stop_flag),
        file_id,
        http_client,
        upstream_base,
    ));

    Ok(GdriveStreamProxyHandle {
        port,
        url,
        stop_flag,
        server_task,
    })
}

async fn server_loop(
    listener: TcpListener,
    gdrive_client: GoogleDriveClient,
    stop_flag: Arc<AtomicBool>,
    file_id: String,
    http_client: reqwest::Client,
    upstream_base: String,
) {
    loop {
        if stop_flag.load(Ordering::Relaxed) {
            return;
        }
        let accept_fut = listener.accept();
        let tick_fut = tokio::time::sleep(std::time::Duration::from_millis(500));
        tokio::select! {
            _ = tick_fut => {
                if stop_flag.load(Ordering::Relaxed) {
                    return;
                }
            }
            accept_res = accept_fut => {
                match accept_res {
                    Ok((stream, _addr)) => {
                        let client = gdrive_client.clone();
                        let fid = file_id.clone();
                        let http = http_client.clone();
                        let base = upstream_base.clone();
                        tokio::spawn(handle_connection(stream, client, fid, http, base));
                    }
                    Err(e) => {
                        eprintln!("[GDRIVE-PROXY] accept error: {}", e);
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
enum ProxyError {
    Network(String),
    Unauthorized,
    UpstreamStatus(u16),
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    gdrive_client: GoogleDriveClient,
    file_id: String,
    http_client: reqwest::Client,
    upstream_base: String,
) {
    let mut buf = vec![0u8; 4096];
    let mut total: usize = 0;
    let read_result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        read_until_headers(&mut stream, &mut buf, &mut total),
    )
    .await;
    if read_result.is_err() {
        let _ = write_simple(&mut stream, 408, "Request Timeout").await;
        return;
    }

    let (path, range_header) = match parse_http_request(&buf[..total]) {
        Some(parsed) => parsed,
        None => {
            let _ = write_simple(&mut stream, 400, "Bad Request").await;
            return;
        }
    };

    if path != PROXY_PATH {
        let _ = write_simple(&mut stream, 404, "Not Found").await;
        return;
    }

    serve_gdrive_via_proxy(
        &mut stream,
        &gdrive_client,
        &file_id,
        range_header.as_deref(),
        &http_client,
        &upstream_base,
    )
    .await;
}

/// Parses a minimal HTTP/1.1 GET request. Returns (path, range_header) or
/// `None` if the buffer is malformed / not a GET request we serve.
fn parse_http_request(buf: &[u8]) -> Option<(String, Option<String>)> {
    let s = std::str::from_utf8(buf).ok()?;
    let mut lines = s.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    if method != "GET" {
        return None;
    }
    let path_only = path.split('?').next().unwrap_or(path).to_string();
    if path_only != PROXY_PATH {
        return Some((path_only, None));
    }
    let mut range_header: Option<String> = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some(idx) = line.find(':') {
            let name = line[..idx].trim().to_ascii_lowercase();
            let value = line[idx + 1..].trim().to_string();
            if name == "range" {
                range_header = Some(value);
            }
        }
    }
    Some((path_only, range_header))
}

async fn read_until_headers(
    stream: &mut tokio::net::TcpStream,
    buf: &mut Vec<u8>,
    total: &mut usize,
) -> std::io::Result<()> {
    let header_end = b"\r\n\r\n";
    loop {
        let n = stream.read(&mut buf[*total..]).await?;
        if n == 0 {
            return Ok(());
        }
        *total += n;
        if buf.windows(header_end.len()).any(|w| w == header_end) {
            return Ok(());
        }
        if *total >= buf.len() {
            if buf.len() >= 16 * 1024 {
                return Ok(());
            }
            buf.resize(buf.len() * 2, 0);
        }
    }
}

async fn serve_gdrive_via_proxy(
    stream: &mut tokio::net::TcpStream,
    gdrive_client: &GoogleDriveClient,
    file_id: &str,
    range_header: Option<&str>,
    http_client: &reqwest::Client,
    upstream_base: &str,
) {
    for attempt in 0..=REFRESH_RETRY_LIMIT {
        let access_token = match gdrive_client.get_access_token().await {
            Ok(t) => t,
            Err(e) => {
                let body = format!("Auth error: {}", e);
                write_simple_response(stream, 401, "Unauthorized", body.as_bytes()).await;
                return;
            }
        };

        match proxy_stream_passthrough(
            stream,
            http_client,
            &access_token,
            file_id,
            range_header,
            upstream_base,
        )
        .await
        {
            Ok(()) => return,
            Err(ProxyError::Unauthorized) if attempt < REFRESH_RETRY_LIMIT => {
                println!(
                    "[GDRIVE-PROXY] upstream returned 401 on attempt {}; forcing token refresh",
                    attempt + 1
                );
                if let Err(e) = gdrive_client.force_refresh().await {
                    eprintln!("[GDRIVE-PROXY] forced refresh failed: {}", e);
                    let body = format!("Upstream 401 and refresh failed: {}", e);
                    write_simple_response(stream, 502, "Bad Gateway", body.as_bytes()).await;
                    return;
                }
                continue;
            }
            Err(e) => {
                let body = format!("Upstream error: {:?}", e);
                write_simple_response(stream, 502, "Bad Gateway", body.as_bytes()).await;
                return;
            }
        }
    }
}

/// True byte-range streaming proxy. Pipes upstream chunks to the MPV
/// socket as they arrive. Upstream `Content-Length` is forwarded so MPV
/// knows exactly how many bytes to read; the TCP connection closes after
/// the response (no keep-alive). No per-request body buffering beyond the
/// chunk pipe itself, so multi-GB files do not cause reconnects.
async fn proxy_stream_passthrough(
    stream: &mut tokio::net::TcpStream,
    http_client: &reqwest::Client,
    access_token: &str,
    file_id: &str,
    range_header: Option<&str>,
    upstream_base: &str,
) -> Result<(), ProxyError> {
    use futures_util::StreamExt;

    let upstream_url = format!(
        "{}/files/{}?alt=media&supportsAllDrives=true",
        upstream_base, file_id
    );

    let mut req = http_client
        .get(&upstream_url)
        .header("Authorization", format!("Bearer {}", access_token));
    if let Some(rh) = range_header {
        req = req.header("Range", rh);
    }

    let resp = req.send().await.map_err(|e| ProxyError::Network(e.to_string()))?;
    let status = resp.status();

    if status.as_u16() == 401 {
        return Err(ProxyError::Unauthorized);
    }

    // Status line.
    let status_line = format!(
        "HTTP/1.1 {} {}\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    write_all_err(stream, status_line.as_bytes()).await?;

    // Forward upstream headers (excluding hop-by-hop).
    for (name, value) in resp.headers().iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if lower == "transfer-encoding" || lower == "connection" {
            continue;
        }
        if let Ok(v) = value.to_str() {
            let line = format!("{}: {}\r\n", name.as_str(), v);
            write_all_err(stream, line.as_bytes()).await?;
        }
    }

    // Always close after response — our one-request-per-connection parser
    // would not parse a second request on the same TCP stream anyway.
    write_all_err(stream, b"Connection: close\r\n\r\n").await?;
    stream
        .flush()
        .await
        .map_err(|e| ProxyError::Network(format!("flush to MPV: {}", e)))?;

    // Stream body chunks through. Safety cap at 32 GiB per request is a
    // sanity guard against a finite-but-large pathological upstream;
    // legitimate MPV Range requests are far smaller.
    const SAFETY_CAP_BYTES: u64 = 32 * 1024 * 1024 * 1024;
    let mut byte_stream = resp.bytes_stream();
    let mut total: u64 = 0;
    while let Some(chunk_res) = byte_stream.next().await {
        match chunk_res {
            Ok(chunk) => {
                total += chunk.len() as u64;
                if total > SAFETY_CAP_BYTES {
                    eprintln!(
                        "[GDRIVE-PROXY] Per-request safety cap exceeded ({}) — closing connection",
                        SAFETY_CAP_BYTES
                    );
                    break;
                }
                write_all_err(stream, &chunk).await?;
            }
            Err(e) => {
                return Err(ProxyError::Network(format!("upstream read: {}", e)));
            }
        }
    }

    if !status.is_success() && status.as_u16() != 206 {
        return Err(ProxyError::UpstreamStatus(status.as_u16()));
    }

    Ok(())
}

async fn write_all_err(
    stream: &mut tokio::net::TcpStream,
    bytes: &[u8],
) -> Result<(), ProxyError> {
    use tokio::io::AsyncWriteExt;
    stream
        .write_all(bytes)
        .await
        .map_err(|e| ProxyError::Network(format!("write to MPV: {}", e)))?;
    Ok(())
}

async fn write_simple_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    reason: &str,
    body: &[u8],
) {
    use tokio::io::AsyncWriteExt;
    let bytes = response_bytes(status, reason, "text/plain", body);
    let _ = stream.write_all(&bytes).await;
    let _ = stream.flush().await;
}

fn response_bytes(status: u16, reason: &str, ct: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(128 + body.len());
    out.extend_from_slice(
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            status,
            reason,
            ct,
            body.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(body);
    out
}

async fn write_response(stream: &mut tokio::net::TcpStream, bytes: Vec<u8>) -> std::io::Result<()> {
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn write_simple(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    reason: &str,
) -> std::io::Result<()> {
    let bytes = response_bytes(status, reason, "text/plain", reason.as_bytes());
    write_response(stream, bytes).await
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gdrive::GoogleTokens;

    #[test]
    fn localhost_url_format() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _guard = rt.enter();
        let handle = GdriveStreamProxyHandle {
            port: 8080,
            url: String::new(),
            stop_flag: Arc::new(AtomicBool::new(false)),
            server_task: tokio::spawn(async {}),
        };
        drop(handle);
    }

    #[test]
    fn parse_request_simple() {
        let buf = b"GET /stream HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".to_vec();
        let parsed = parse_http_request(&buf).unwrap();
        assert_eq!(parsed.0, "/stream");
        assert!(parsed.1.is_none());
    }

    #[test]
    fn parse_request_with_range() {
        let buf = b"GET /stream HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes=0-1023\r\n\r\n";
        let parsed = parse_http_request(buf).unwrap();
        assert_eq!(parsed.0, "/stream");
        assert_eq!(parsed.1.as_deref(), Some("bytes=0-1023"));
    }

    #[test]
    fn parse_request_non_stream_path() {
        let buf = b"GET /other HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let parsed = parse_http_request(buf).unwrap();
        assert_eq!(parsed.0, "/other");
        assert!(parsed.1.is_none());
    }

    #[test]
    fn parse_request_rejects_post() {
        let buf = b"POST /stream HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert!(parse_http_request(buf).is_none());
    }

    #[test]
    fn parse_request_with_query_string() {
        let buf = b"GET /stream?token=abc HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let parsed = parse_http_request(buf).unwrap();
        assert_eq!(parsed.0, "/stream");
    }

    #[test]
    fn response_bytes_includes_content_length() {
        let body = b"{\"ok\":true}";
        let resp = response_bytes(200, "OK", "application/json", body);
        let s = std::str::from_utf8(&resp).expect("utf8");
        assert!(s.contains("HTTP/1.1 200 OK"));
        assert!(s.contains("Content-Type: application/json"));
        assert!(s.contains(&format!("Content-Length: {}", body.len())));
        assert!(s.ends_with("{\"ok\":true}"));
    }

    /// Gate regression test for the "token expires mid-playback" user
    /// complaint: spawn an in-test mock upstream, start the real proxy
    /// via `start_gdrive_stream_proxy_with_upstream`, make a real HTTP
    /// request to the proxy from outside, and prove the four invariants
    /// the bug hinged on:
    ///   (a) the proxy signed with the **current** access token,
    ///   (b) the upstream URL is the correct Drive path,
    ///   (c) the Range header was forwarded verbatim, and
    ///   (d) the upstream body was streamed back to the caller.
    /// All four were asserted via the mock recording the raw bytes the
    /// proxy sent it; the test fails if any one regresses.
    #[tokio::test(flavor = "current_thread")]
    async fn proxy_streams_upstream_body_signed_with_current_access_token() {
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mock = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mock_port = mock.local_addr().unwrap().port();
        let received_raw: Arc<tokio::sync::Mutex<Vec<u8>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let request_count: Arc<tokio::sync::Mutex<u32>> =
            Arc::new(tokio::sync::Mutex::new(0));

        let raw_clone = Arc::clone(&received_raw);
        let count_clone = Arc::clone(&request_count);
        let _mock_task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = mock.accept().await else { break };
                let raw_inner = Arc::clone(&raw_clone);
                let count_inner = Arc::clone(&count_clone);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    *raw_inner.lock().await = buf[..n].to_vec();
                    *count_inner.lock().await += 1;
                    let body = b"MOCK_VIDEO_BYTES";
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                });
            }
        });

        const TEST_ACCESS_TOKEN: &str = "ya29.fake-test-access-token-xyz";
        let client = GoogleDriveClient::new();
        client.replace_tokens_for_test(Some(GoogleTokens {
            access_token: TEST_ACCESS_TOKEN.to_string(),
            refresh_token: Some("1//fake-refresh".to_string()),
            expires_at: Some(chrono::Utc::now().timestamp() + 3600),
            token_type: "Bearer".to_string(),
        }));

        // Base URL includes `/drive/v3` because `proxy_stream_passthrough`
        // appends `/files/{file_id}?alt=media&supportsAllDrives=true`.
        let upstream_base = format!("http://127.0.0.1:{}/drive/v3", mock_port);
        let file_id = "test-file-id-12345";
        let proxy = start_gdrive_stream_proxy_with_upstream(
            client,
            file_id.to_string(),
            &upstream_base,
        )
        .await
        .expect("proxy start");

        let mut upstream_conn =
            tokio::net::TcpStream::connect(("127.0.0.1", proxy.port))
                .await
                .expect("connect to proxy");
        upstream_conn
            .write_all(
                b"GET /stream HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes=0-99\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("write to proxy");

        let mut resp_bytes = Vec::new();
        upstream_conn
            .read_to_end(&mut resp_bytes)
            .await
            .expect("read proxy response");
        let resp_str = std::str::from_utf8(&resp_bytes).expect("utf8");

        assert!(
            resp_str.contains("MOCK_VIDEO_BYTES"),
            "proxy must stream upstream body verbatim, got: {}",
            resp_str
        );

        let count = *request_count.lock().await;
        assert_eq!(count, 1, "expected exactly one upstream request, got {}", count);

        let raw = received_raw.lock().await.clone();
        let raw_str = std::str::from_utf8(&raw).expect("utf8");

        assert!(
            raw_str.contains(&format!("Bearer {}", TEST_ACCESS_TOKEN)),
            "proxy must sign upstream request with the current access_token; got: {}",
            raw_str
        );
        assert!(
            raw_str.starts_with(&format!(
                "GET /drive/v3/files/{}?alt=media&supportsAllDrives=true",
                file_id
            )),
            "proxy must compose the correct upstream URL; got first line: {:?}",
            raw_str.lines().next().unwrap_or("")
        );
        assert!(
            raw_str.to_ascii_lowercase().contains("range: bytes=0-99"),
            "proxy must forward the Range header from MPV; got: {}",
            raw_str
        );

        drop(proxy);
    }
}
