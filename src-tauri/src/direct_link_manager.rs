use crate::archive_manager;
use crate::media_manager;
use crate::zip_manager;
use crate::zip_parser;
use percent_encoding::percent_decode_str;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, RANGE};
use serde::{Deserialize, Serialize};

const DDL_TAIL_BYTES: u64 = 131_072;
const DDL_LOCAL_HEADER_PREFETCH_BYTES: u64 = 4096;
const DDL_MAX_CENTRAL_DIRECTORY_BYTES: u64 = 16 * 1024 * 1024;
const DDL_MAX_ZIP_ENTRIES: usize = 10_000;
const DDL_MAX_ENTRY_SIZE_BYTES: u64 = 50 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DdlSource {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub file_size: u64,
    pub archive_format: String,
    pub entry_count: usize,
    pub video_count: usize,
    pub cd_offset: u64,
    pub cd_size: u64,
    pub created_at: String,
    pub last_verified_at: String,
    pub is_expired: bool,
    /// Set for sources indexed from addon season packs. Format: "imdb_id:season:stream_name"
    pub addon_origin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DdlValidationResult {
    pub supports_range: bool,
    pub file_size: u64,
    pub filename: String,
    pub content_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DdlIndexResult {
    pub source: DdlSource,
    pub entries: Vec<DdlIndexedEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DdlIndexedEntry {
    pub entry_path: String,
    pub entry_name: String,
    pub title: String,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub compression_method: i64,
    pub local_header_offset: u64,
    pub data_start_offset: u64,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DdlRefreshResult {
    pub accepted: bool,
    pub message: String,
}

/// Validate a direct download URL: check reachability, file size, and Range support.
pub fn validate_url(url: &str) -> Result<DdlValidationResult, String> {
    let parsed = url::Url::parse(url).map_err(|_| "Invalid URL".to_string())?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err("Only HTTPS URLs are allowed for direct links".to_string());
    }
    // Check for private IP ranges (allow localhost for testing and local DDL sources)
    if let Some(host) = parsed.host_str() {
        if host.starts_with("10.")
            || host.starts_with("172.16.")
            || host.starts_with("192.168.")
            || host.starts_with("169.254.")
        {
            return Err(
                "Private/internal network URLs are not allowed for direct links".to_string(),
            );
        }
    }
    if let Some(host) = parsed.host_str() {
        let lower = host.to_lowercase();
        if lower == "::1"
            || lower.starts_with("10.")
            || lower.starts_with("172.16.")
            || lower.starts_with("192.168.")
            || lower.starts_with("169.254.")
        {
            return Err("Private/internal network URLs are not allowed".to_string());
        }
    }

    let client = crate::http_client::long_client().clone();

    let response = client
        .head(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Cannot access URL: {}", e))?;

    let file_size = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| "Cannot determine file size from URL".to_string())?;

    let accept_ranges = response
        .headers()
        .get(ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("none");
    let supports_range = !accept_ranges.eq_ignore_ascii_case("none");

    // If HEAD didn't confirm Range support, try a small Range request to verify
    let supports_range = if !supports_range {
        match client.get(url).header(RANGE, "bytes=0-0").send() {
            Ok(resp) => resp.status().as_u16() == 206,
            Err(_) => false,
        }
    } else {
        true
    };

    if !supports_range {
        return Err(
            "This URL does not support Range requests. The file host must support HTTP Range requests for streaming."
                .to_string(),
        );
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let filename = extract_filename_from_response(&response, url);

    Ok(DdlValidationResult {
        supports_range,
        file_size,
        filename,
        content_type,
    })
}

/// Index a remote archive at the given URL. Returns the DDL source and all indexed video entries.
pub fn index_archive(
    url: &str,
    validation: &DdlValidationResult,
) -> Result<DdlIndexResult, String> {
    let format = archive_manager::detect_archive_format(
        &validation.filename,
        Some(&validation.content_type),
    )
    .ok_or_else(|| "This file is not a supported archive format (ZIP, RAR, or TAR).".to_string())?;

    match format {
        archive_manager::ArchiveFormat::Zip => index_zip_archive(url, validation),
        _ => Err(format!(
            "Direct link indexing for {} archives is not yet supported. Only ZIP is currently supported.",
            format.as_str()
        )),
    }
}

/// Check if a direct link is still accessible.
pub fn check_link_health(url: &str) -> Result<bool, String> {
    let client = crate::http_client::long_client().clone();

    match client.head(url).send() {
        Ok(response) => {
            let status = response.status().as_u16();
            Ok(status < 400)
        }
        Err(_) => Ok(false),
    }
}

/// Verify that a new URL points to the same archive, then return whether it's accepted.
pub fn verify_and_refresh_link(
    source: &DdlSource,
    new_url: &str,
) -> Result<DdlRefreshResult, String> {
    let validation = validate_url(new_url)?;

    // Check file size
    if validation.file_size != source.file_size {
        return Ok(DdlRefreshResult {
            accepted: false,
            message: format!(
                "The new link points to a different file (size mismatch: expected {} bytes, got {} bytes).",
                source.file_size, validation.file_size
            ),
        });
    }

    // Verify the EOCD/CD structure matches
    let client = crate::http_client::long_client().clone();
    let tail_len = validation.file_size.min(DDL_TAIL_BYTES);
    let tail_start = validation.file_size.saturating_sub(tail_len);
    let tail = fetch_range(&client, new_url, tail_start, validation.file_size - 1)?;
    let eocd = zip_parser::find_eocd(&tail, tail_start).map_err(|e| e.to_string())?;

    if eocd.cd_offset != source.cd_offset || eocd.cd_size != source.cd_size {
        return Ok(DdlRefreshResult {
            accepted: false,
            message: "The new link points to a different archive (internal structure mismatch). Please provide a link to the same archive.".to_string(),
        });
    }

    Ok(DdlRefreshResult {
        accepted: true,
        message: "Link refreshed successfully.".to_string(),
    })
}

// ---- Internal helpers ----

fn index_zip_archive(
    url: &str,
    validation: &DdlValidationResult,
) -> Result<DdlIndexResult, String> {
    let client = crate::http_client::long_client().clone();
    let file_size = validation.file_size;

    // 1. Fetch tail to find EOCD
    let tail_len = file_size.min(DDL_TAIL_BYTES);
    let tail_start = file_size.saturating_sub(tail_len);
    let tail = fetch_range(&client, url, tail_start, file_size - 1)?;
    let eocd = zip_parser::find_eocd(&tail, tail_start).map_err(|e| e.to_string())?;

    if eocd.cd_size > DDL_MAX_CENTRAL_DIRECTORY_BYTES {
        return Err("Archive Central Directory is too large".to_string());
    }

    // 2. Fetch Central Directory
    let cd_end = eocd
        .cd_offset
        .checked_add(eocd.cd_size)
        .and_then(|v| v.checked_sub(1))
        .ok_or_else(|| "Invalid Central Directory range".to_string())?;
    let central_directory = fetch_range(&client, url, eocd.cd_offset, cd_end)?;
    let parsed_entries = zip_parser::parse_central_directory(&central_directory, eocd.cd_offset)
        .map_err(|e| e.to_string())?;

    if parsed_entries.len() > DDL_MAX_ZIP_ENTRIES {
        return Err("Archive contains too many entries".to_string());
    }

    // 3. Filter to video entries and build indexed entries
    let video_entries: Vec<_> = parsed_entries
        .iter()
        .filter(|e| !e.is_directory && zip_manager::is_video_path(&e.filename))
        .cloned()
        .collect();

    let compression_type = zip_manager::check_zip_compression_type(&video_entries);

    let mut indexed_entries = Vec::new();
    for entry in &video_entries {
        if entry.is_encrypted {
            continue;
        }
        if entry.uncompressed_size > DDL_MAX_ENTRY_SIZE_BYTES {
            continue;
        }
        if !is_supported_compression(entry.compression_method) {
            continue;
        }

        let parsed = match zip_manager::extract_episode_metadata(entry) {
            Ok(p) => p,
            Err(_) => {
                // If parsing fails, still include it with a fallback title
                let entry_name = entry
                    .filename
                    .rsplit('/')
                    .next()
                    .unwrap_or(&entry.filename)
                    .to_string();
                media_manager::ParsedMedia {
                    title: entry_name.clone(),
                    season: None,
                    episode: None,
                    year: None,
                    media_type: media_manager::MediaParseType::Movie,
                    episode_end: None,
                }
            }
        };

        let data_start_offset = fetch_data_start_offset(&client, url, entry.local_header_offset)?;
        let entry_name = entry
            .filename
            .rsplit('/')
            .next()
            .unwrap_or(&entry.filename)
            .to_string();

        indexed_entries.push(DdlIndexedEntry {
            entry_path: entry.filename.clone(),
            entry_name,
            title: parsed.title.clone(),
            season: parsed.season,
            episode: parsed.episode,
            compression_method: i64::from(entry.compression_method),
            local_header_offset: entry.local_header_offset,
            data_start_offset,
            compressed_size: entry.compressed_size,
            uncompressed_size: entry.uncompressed_size,
            crc32: format!("{:08x}", entry.crc32),
        });
    }

    if indexed_entries.is_empty() {
        return Err("No playable video files found in this archive.".to_string());
    }

    let source_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let source = DdlSource {
        id: source_id,
        url: url.to_string(),
        filename: validation.filename.clone(),
        file_size,
        archive_format: "zip".to_string(),
        entry_count: parsed_entries.len(),
        video_count: indexed_entries.len(),
        cd_offset: eocd.cd_offset,
        cd_size: eocd.cd_size,
        created_at: now.clone(),
        last_verified_at: now,
        is_expired: false,
        addon_origin: None,
    };

    println!(
        "[DDL] Indexed '{}': {} total entries, {} video entries, compression={:?}",
        validation.filename,
        parsed_entries.len(),
        indexed_entries.len(),
        compression_type
    );

    Ok(DdlIndexResult {
        source,
        entries: indexed_entries,
    })
}

fn fetch_range(client: &Client, url: &str, start: u64, end: u64) -> Result<Vec<u8>, String> {
    client
        .get(url)
        .header(RANGE, format!("bytes={}-{}", start, end))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Failed to fetch byte range {}-{}: {}", start, end, e))?
        .bytes()
        .map(|b| b.to_vec())
        .map_err(|e| format!("Failed to read response bytes: {}", e))
}

fn fetch_data_start_offset(
    client: &Client,
    url: &str,
    local_header_offset: u64,
) -> Result<u64, String> {
    let mut header_bytes = fetch_range(
        client,
        url,
        local_header_offset,
        local_header_offset + DDL_LOCAL_HEADER_PREFETCH_BYTES - 1,
    )?;

    if header_bytes.len() < 30 {
        return Err("Local file header too short".to_string());
    }

    let file_name_length = u16::from_le_bytes([header_bytes[26], header_bytes[27]]) as usize;
    let extra_length = u16::from_le_bytes([header_bytes[28], header_bytes[29]]) as usize;
    let required_len = 30 + file_name_length + extra_length;

    if header_bytes.len() < required_len {
        header_bytes = fetch_range(
            client,
            url,
            local_header_offset,
            local_header_offset + required_len as u64 - 1,
        )?;
    }

    zip_parser::parse_local_file_header(&header_bytes, local_header_offset)
        .map(|info| info.data_start_offset)
        .map_err(|e| e.to_string())
}

fn extract_filename_from_response(response: &reqwest::blocking::Response, url: &str) -> String {
    // Try Content-Disposition header first
    if let Some(disposition) = response.headers().get(CONTENT_DISPOSITION) {
        if let Ok(value) = disposition.to_str() {
            if let Some(name) = parse_content_disposition_filename(value) {
                return name;
            }
        }
    }

    // Fall back to URL path
    url.rsplit('/')
        .next()
        .and_then(|segment| {
            let name = segment.split('?').next().unwrap_or(segment);
            let decoded = percent_decode_str(name).decode_utf8_lossy();
            let decoded = decoded.trim();
            if decoded.is_empty() || decoded == "/" {
                None
            } else {
                Some(decoded.to_string())
            }
        })
        .unwrap_or_else(|| "archive.zip".to_string())
}

fn parse_content_disposition_filename(value: &str) -> Option<String> {
    // Handle: filename="some file.zip" or filename*=UTF-8''some%20file.zip
    for part in value.split(';') {
        let part = part.trim();
        if let Some(name) = part.strip_prefix("filename*=") {
            // RFC 5987 encoding
            if let Some(encoded) = name.split("''").nth(1) {
                let decoded = percent_decode_str(encoded).decode_utf8_lossy();
                let decoded = decoded.trim_matches('"').trim().to_string();
                if !decoded.is_empty() {
                    return Some(decoded);
                }
            }
        } else if let Some(name) = part.strip_prefix("filename=") {
            let name = name.trim_matches('"').trim().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

fn is_supported_compression(method: u16) -> bool {
    matches!(method, 0 | 8)
}

#[cfg(test)]
mod tests {
    use super::{
        check_link_health, index_archive, validate_url, verify_and_refresh_link, DdlSource,
    };
    use crc32fast::Hasher;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;
    use tiny_http::{Header, Method, Response, Server, StatusCode};

    struct TestServer {
        base_url: String,
        _handle: thread::JoinHandle<()>,
    }

    fn start_range_server(body: Vec<u8>, filename: &str, advertise_ranges: bool) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = Server::from_listener(listener, None).unwrap();
        let body = Arc::new(body);
        let filename = filename.to_string();

        let handle = thread::spawn(move || {
            for request in server.incoming_requests() {
                let method = request.method().clone();
                let body = Arc::clone(&body);
                let total_len = body.len() as u64;

                let content_type =
                    Header::from_bytes(&b"Content-Type"[..], &b"application/octet-stream"[..])
                        .unwrap();
                let content_disposition = Header::from_bytes(
                    &b"Content-Disposition"[..],
                    format!("attachment; filename=\"{}\"", filename),
                )
                .unwrap();

                let mut headers = vec![
                    Header::from_bytes(&b"Content-Length"[..], total_len.to_string()).unwrap(),
                    content_type,
                    content_disposition,
                ];
                if advertise_ranges {
                    headers.push(Header::from_bytes(&b"Accept-Ranges"[..], &b"bytes"[..]).unwrap());
                }

                match method {
                    Method::Head => {
                        let response = headers
                            .into_iter()
                            .fold(Response::empty(200), |response, header| {
                                response.with_header(header)
                            });
                        let _ = request.respond(response);
                    }
                    Method::Get => {
                        let range_header = request
                            .headers()
                            .iter()
                            .find(|header| header.field.equiv("Range"))
                            .map(|header| header.value.as_str().to_string());

                        if advertise_ranges {
                            if let Some(range) = range_header {
                                if let Some((start, end)) = parse_range_header(&range, total_len) {
                                    let chunk = body[start as usize..=end as usize].to_vec();
                                    let mut partial_headers = vec![
                                        Header::from_bytes(
                                            &b"Content-Length"[..],
                                            chunk.len().to_string(),
                                        )
                                        .unwrap(),
                                        Header::from_bytes(
                                            &b"Content-Type"[..],
                                            &b"application/octet-stream"[..],
                                        )
                                        .unwrap(),
                                        Header::from_bytes(
                                            &b"Content-Disposition"[..],
                                            format!("attachment; filename=\"{}\"", filename),
                                        )
                                        .unwrap(),
                                        Header::from_bytes(&b"Accept-Ranges"[..], &b"bytes"[..])
                                            .unwrap(),
                                        Header::from_bytes(
                                            &b"Content-Range"[..],
                                            format!("bytes {}-{}/{}", start, end, total_len),
                                        )
                                        .unwrap(),
                                    ];
                                    let response = partial_headers.into_iter().fold(
                                        Response::from_data(chunk)
                                            .with_status_code(StatusCode(206)),
                                        |response, header| response.with_header(header),
                                    );
                                    let _ = request.respond(response);
                                    continue;
                                }
                            }
                        }

                        let response = headers.into_iter().fold(
                            Response::from_data(body.as_ref().clone()),
                            |response, header| response.with_header(header),
                        );
                        let _ = request.respond(response);
                    }
                    _ => {
                        let _ = request.respond(Response::empty(405));
                    }
                }
            }
        });

        TestServer {
            base_url: format!("http://{}/archive.zip", addr),
            _handle: handle,
        }
    }

    fn parse_range_header(value: &str, total_len: u64) -> Option<(u64, u64)> {
        let range = value.strip_prefix("bytes=")?;
        let (start, end) = range.split_once('-')?;
        let start = start.parse::<u64>().ok()?;
        let end = if end.is_empty() {
            total_len.checked_sub(1)?
        } else {
            end.parse::<u64>().ok()?
        };
        if start > end || end >= total_len {
            return None;
        }
        Some((start, end))
    }

    fn build_test_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut file_bytes = Vec::new();
        let mut central_directory = Vec::new();

        for (path, contents) in entries {
            let local_header_offset = file_bytes.len() as u32;
            let path_bytes = path.as_bytes();

            let mut hasher = Hasher::new();
            hasher.update(contents);
            let crc32 = hasher.finalize();
            let size = contents.len() as u32;

            file_bytes.extend_from_slice(&0x04034b50u32.to_le_bytes());
            file_bytes.extend_from_slice(&20u16.to_le_bytes());
            file_bytes.extend_from_slice(&0u16.to_le_bytes());
            file_bytes.extend_from_slice(&0u16.to_le_bytes());
            file_bytes.extend_from_slice(&0u16.to_le_bytes());
            file_bytes.extend_from_slice(&0u16.to_le_bytes());
            file_bytes.extend_from_slice(&crc32.to_le_bytes());
            file_bytes.extend_from_slice(&size.to_le_bytes());
            file_bytes.extend_from_slice(&size.to_le_bytes());
            file_bytes.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
            file_bytes.extend_from_slice(&0u16.to_le_bytes());
            file_bytes.extend_from_slice(path_bytes);
            file_bytes.extend_from_slice(contents);

            central_directory.extend_from_slice(&0x02014b50u32.to_le_bytes());
            central_directory.extend_from_slice(&20u16.to_le_bytes());
            central_directory.extend_from_slice(&20u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&crc32.to_le_bytes());
            central_directory.extend_from_slice(&size.to_le_bytes());
            central_directory.extend_from_slice(&size.to_le_bytes());
            central_directory.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u16.to_le_bytes());
            central_directory.extend_from_slice(&0u32.to_le_bytes());
            central_directory.extend_from_slice(&local_header_offset.to_le_bytes());
            central_directory.extend_from_slice(path_bytes);
        }

        let cd_offset = file_bytes.len() as u32;
        let cd_size = central_directory.len() as u32;
        file_bytes.extend_from_slice(&central_directory);
        file_bytes.extend_from_slice(&0x06054b50u32.to_le_bytes());
        file_bytes.extend_from_slice(&0u16.to_le_bytes());
        file_bytes.extend_from_slice(&0u16.to_le_bytes());
        file_bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        file_bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        file_bytes.extend_from_slice(&cd_size.to_le_bytes());
        file_bytes.extend_from_slice(&cd_offset.to_le_bytes());
        file_bytes.extend_from_slice(&0u16.to_le_bytes());

        file_bytes
    }

    #[test]
    fn validate_url_extracts_filename_and_range_support() {
        let zip_bytes = build_test_zip(&[("Show.S01E01.mkv", b"episode-one")]);
        let server = start_range_server(zip_bytes, "example-season.zip", true);

        let validation = validate_url(&server.base_url).unwrap();
        assert!(validation.supports_range);
        assert_eq!(validation.filename, "example-season.zip");
        assert_eq!(validation.content_type, "application/octet-stream");
        assert!(validation.file_size > 0);
    }

    #[test]
    fn index_archive_extracts_tv_episode_metadata_from_remote_zip() {
        let zip_bytes = build_test_zip(&[
            ("If Wishes Could Kill (2026) S01E01.mkv", b"ep1"),
            ("If Wishes Could Kill (2026) S01E02.mkv", b"ep2"),
            ("If Wishes Could Kill (2026) S01E03.mkv", b"ep3"),
            ("notes/readme.txt", b"ignore me"),
        ]);
        let server = start_range_server(zip_bytes, "season-pack.zip", true);

        let validation = validate_url(&server.base_url).unwrap();
        let indexed = index_archive(&server.base_url, &validation).unwrap();

        assert_eq!(indexed.source.archive_format, "zip");
        assert_eq!(indexed.source.entry_count, 4);
        assert_eq!(indexed.source.video_count, 3);
        assert_eq!(indexed.entries.len(), 3);
        assert!(indexed
            .entries
            .iter()
            .all(|entry| entry.title == "If Wishes Could Kill"));
        assert_eq!(indexed.entries[0].season, Some(1));
        assert_eq!(indexed.entries[0].episode, Some(1));
        assert_eq!(indexed.entries[1].episode, Some(2));
        assert_eq!(indexed.entries[2].episode, Some(3));
        assert!(indexed
            .entries
            .iter()
            .all(|entry| entry.data_start_offset > entry.local_header_offset));
    }

    #[test]
    fn index_archive_rejects_urls_without_range_support() {
        let zip_bytes = build_test_zip(&[("Show.S01E01.mkv", b"episode-one")]);
        let server = start_range_server(zip_bytes, "no-range.zip", false);

        let err = validate_url(&server.base_url).unwrap_err();
        assert!(err.contains("does not support Range requests"));
    }

    #[test]
    fn verify_and_refresh_link_rejects_different_archive_size() {
        let original_zip = build_test_zip(&[("Show.S01E01.mkv", b"episode-one")]);
        let refreshed_zip = build_test_zip(&[("Show.S01E01.mkv", b"different-content")]);

        let original_server = start_range_server(original_zip.clone(), "season.zip", true);
        let refreshed_server = start_range_server(refreshed_zip, "season.zip", true);

        let source = DdlSource {
            id: "source-1".to_string(),
            url: original_server.base_url.clone(),
            filename: "season.zip".to_string(),
            file_size: original_zip.len() as u64,
            archive_format: "zip".to_string(),
            entry_count: 1,
            video_count: 1,
            cd_offset: 0,
            cd_size: 0,
            created_at: "2026-01-01 00:00:00".to_string(),
            last_verified_at: "2026-01-01 00:00:00".to_string(),
            is_expired: false,
            addon_origin: None,
        };

        let result = verify_and_refresh_link(&source, &refreshed_server.base_url).unwrap();
        assert!(!result.accepted);
        assert!(result.message.contains("different file"));
    }

    #[test]
    fn verify_and_refresh_link_accepts_same_archive_at_new_url() {
        let zip_bytes = build_test_zip(&[("Show.S01E01.mkv", b"episode-one")]);
        let server1 = start_range_server(zip_bytes.clone(), "season.zip", true);
        let server2 = start_range_server(zip_bytes, "season.zip", true);

        let validation = validate_url(&server1.base_url).unwrap();
        let indexed = index_archive(&server1.base_url, &validation).unwrap();

        let result = verify_and_refresh_link(&indexed.source, &server2.base_url).unwrap();
        assert!(result.accepted);
        assert!(result.message.contains("success"));
    }

    #[test]
    fn verify_and_refresh_link_fails_on_unreachable_url() {
        // Use a port guaranteed to have nothing listening
        let unreachable_url = "http://127.0.0.1:1/nonexistent.zip";

        let source = DdlSource {
            id: "s1".to_string(),
            url: unreachable_url.to_string(),
            filename: "season.zip".to_string(),
            file_size: 100,
            archive_format: "zip".to_string(),
            entry_count: 1,
            video_count: 1,
            cd_offset: 0,
            cd_size: 0,
            created_at: "2026-01-01 00:00:00".to_string(),
            last_verified_at: "2026-01-01 00:00:00".to_string(),
            is_expired: false,
            addon_origin: None,
        };

        let err = verify_and_refresh_link(&source, unreachable_url);
        assert!(err.is_err());
    }

    #[test]
    fn check_link_health_returns_true_for_accessible_url() {
        let zip_bytes = build_test_zip(&[("Show.S01E01.mkv", b"ep")]);
        let server = start_range_server(zip_bytes, "test.zip", true);

        let result = check_link_health(&server.base_url).unwrap();
        assert!(result);
    }

    #[test]
    fn check_link_health_returns_false_for_unreachable_url() {
        // Use a port that's almost certainly not listening
        let result = check_link_health("http://127.0.0.1:1/nonexistent.zip").unwrap();
        assert!(!result);
    }

    #[test]
    fn check_link_health_returns_true_for_4xx_status() {
        // tiny_http server with a handler that returns 404
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = Server::from_listener(listener, None).unwrap();

        let handle = thread::spawn(move || {
            for request in server.incoming_requests() {
                let _ = request.respond(Response::empty(404));
            }
        });

        let url = format!("http://{}/missing.zip", addr);
        // check_link_health returns Ok(status < 400), so 404 => false
        let result = check_link_health(&url).unwrap();
        assert!(!result);

        drop(handle);
    }

    #[test]
    fn validate_url_rejects_ftp_scheme() {
        let err = validate_url("ftp://example.com/file.zip").unwrap_err();
        assert!(err.contains("HTTPS URLs are allowed"));
    }

    #[test]
    fn validate_url_rejects_private_ip_10() {
        let err = validate_url("http://10.0.0.1/file.zip").unwrap_err();
        assert!(err.contains("Private") || err.contains("internal"));
    }

    #[test]
    fn validate_url_rejects_private_ip_192_168() {
        let err = validate_url("http://192.168.1.1/file.zip").unwrap_err();
        assert!(err.contains("Private") || err.contains("internal"));
    }

    #[test]
    fn parse_content_disposition_parses_simple_filename() {
        let result =
            super::parse_content_disposition_filename("attachment; filename=\"myfile.zip\"");
        assert_eq!(result, Some("myfile.zip".to_string()));
    }

    #[test]
    fn parse_content_disposition_parses_rfc5987_encoded() {
        let result = super::parse_content_disposition_filename(
            "attachment; filename*=UTF-8''my%20file%20%281%29.zip",
        );
        assert_eq!(result, Some("my file (1).zip".to_string()));
    }

    #[test]
    fn parse_content_disposition_returns_none_for_empty() {
        assert_eq!(super::parse_content_disposition_filename(""), None);
        assert_eq!(
            super::parse_content_disposition_filename("attachment"),
            None
        );
    }

    #[test]
    fn parse_content_disposition_takes_first_match() {
        // When filename* appears first, it wins (left-to-right scan)
        let result = super::parse_content_disposition_filename(
            "attachment; filename*=UTF-8''preferred.zip; filename=\"fallback.zip\"",
        );
        assert_eq!(result, Some("preferred.zip".to_string()));

        // When filename appears first, it wins
        let result2 = super::parse_content_disposition_filename(
            "attachment; filename=\"first.zip\"; filename*=UTF-8''second.zip",
        );
        assert_eq!(result2, Some("first.zip".to_string()));
    }

    #[test]
    fn parse_content_disposition_unquoted_filename() {
        let result = super::parse_content_disposition_filename("attachment; filename=data.zip");
        assert_eq!(result, Some("data.zip".to_string()));
    }

    #[test]
    fn is_supported_compression_accepts_stored_and_deflate() {
        assert!(super::is_supported_compression(0)); // Stored
        assert!(super::is_supported_compression(8)); // Deflate
    }

    #[test]
    fn is_supported_compression_rejects_other_methods() {
        assert!(!super::is_supported_compression(1)); // Shrunk
        assert!(!super::is_supported_compression(6));
        assert!(!super::is_supported_compression(9)); // Deflate64
        assert!(!super::is_supported_compression(12)); // BZIP2
        assert!(!super::is_supported_compression(14)); // LZMA
        assert!(!super::is_supported_compression(99));
    }

    #[test]
    fn ddl_source_serde_round_trip() {
        let source = super::DdlSource {
            id: "abc-123".to_string(),
            url: "https://example.com/file.zip".to_string(),
            filename: "file.zip".to_string(),
            file_size: 1024,
            archive_format: "zip".to_string(),
            entry_count: 5,
            video_count: 3,
            cd_offset: 500,
            cd_size: 200,
            created_at: "2026-01-01 00:00:00".to_string(),
            last_verified_at: "2026-06-01 12:00:00".to_string(),
            is_expired: false,
            addon_origin: Some("tt1234567:1:StreamName".to_string()),
        };

        let json = serde_json::to_string(&source).unwrap();
        let deserialized: super::DdlSource = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, source.id);
        assert_eq!(deserialized.url, source.url);
        assert_eq!(deserialized.filename, source.filename);
        assert_eq!(deserialized.file_size, source.file_size);
        assert_eq!(deserialized.entry_count, source.entry_count);
        assert_eq!(deserialized.video_count, source.video_count);
        assert_eq!(deserialized.cd_offset, source.cd_offset);
        assert_eq!(deserialized.cd_size, source.cd_size);
        assert_eq!(deserialized.is_expired, source.is_expired);
        assert_eq!(deserialized.addon_origin, source.addon_origin);
    }

    #[test]
    fn ddl_source_serde_with_null_addon_origin() {
        let source = super::DdlSource {
            id: "x".to_string(),
            url: "https://example.com/f.zip".to_string(),
            filename: "f.zip".to_string(),
            file_size: 100,
            archive_format: "zip".to_string(),
            entry_count: 1,
            video_count: 1,
            cd_offset: 0,
            cd_size: 0,
            created_at: "2026-01-01 00:00:00".to_string(),
            last_verified_at: "2026-01-01 00:00:00".to_string(),
            is_expired: true,
            addon_origin: None,
        };

        let json = serde_json::to_string(&source).unwrap();
        let deserialized: super::DdlSource = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.addon_origin, None);
        assert!(deserialized.is_expired);
    }

    #[test]
    fn ddl_validation_result_serde_round_trip() {
        let result = super::DdlValidationResult {
            supports_range: true,
            file_size: 999,
            filename: "archive.zip".to_string(),
            content_type: "application/zip".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: super::DdlValidationResult = serde_json::from_str(&json).unwrap();
        assert!(deserialized.supports_range);
        assert_eq!(deserialized.file_size, 999);
        assert_eq!(deserialized.filename, "archive.zip");
    }

    #[test]
    fn ddl_refresh_result_serde_round_trip() {
        let result = super::DdlRefreshResult {
            accepted: false,
            message: "size mismatch".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: super::DdlRefreshResult = serde_json::from_str(&json).unwrap();
        assert!(!deserialized.accepted);
        assert_eq!(deserialized.message, "size mismatch");
    }

    #[test]
    fn ddl_indexed_entry_serde_round_trip() {
        let entry = super::DdlIndexedEntry {
            entry_path: "Show/episode.mkv".to_string(),
            entry_name: "episode.mkv".to_string(),
            title: "Episode".to_string(),
            season: Some(1),
            episode: Some(2),
            compression_method: 8,
            local_header_offset: 100,
            data_start_offset: 150,
            compressed_size: 500,
            uncompressed_size: 1000,
            crc32: "abcdef01".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("entryPath"));
        assert!(json.contains("localHeaderOffset"));

        let deserialized: super::DdlIndexedEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.entry_path, "Show/episode.mkv");
        assert_eq!(deserialized.season, Some(1));
        assert_eq!(deserialized.episode, Some(2));
        assert_eq!(deserialized.compression_method, 8);
    }
}
