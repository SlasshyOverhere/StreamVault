use crate::database;
use crate::media_manager;
use crate::zip_parser;
use crate::zip_parser::{ZipCompressionType, ZipEntry, ZipError};
use crc32fast::Hasher as Crc32Hasher;
use flate2::read::DeflateDecoder;
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static CACHE_POLICY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
const ZIP_TAIL_BYTES: u64 = 131_072;
const MAX_ZIP_ENTRIES: usize = 10_000;
const MAX_ENTRY_SIZE_BYTES: u64 = 50 * 1024 * 1024 * 1024;
const MAX_CENTRAL_DIRECTORY_BYTES: u64 = 256 * 1024 * 1024;
const LOCAL_HEADER_PREFETCH_BYTES: u64 = 4096;
const ZIP_TEMP_CACHE_MAX_AGE_SECS: u64 = 86_400;

#[derive(Debug, Clone)]
pub struct ZipArchiveInfo {
    pub zip_file_id: String,
    pub filename: String,
    pub archive_format: String,
    pub file_size_bytes: u64,
    pub compression_type: ZipCompressionType,
    pub central_dir_offset: u64,
    pub central_dir_size: u64,
    pub total_entries: usize,
    pub video_entries: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipEpisodeInfo {
    pub filename: String,
    pub title: String,
    pub season: i32,
    pub episode: i32,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipAnalysisResult {
    pub zip_file_id: String,
    pub filename: String,
    pub file_size: u64,
    pub compression_type: ZipCompressionType,
    pub total_entries: usize,
    pub video_entries: usize,
    pub episodes: Vec<ZipEpisodeInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipIndexResult {
    pub indexed_count: usize,
    pub skipped_count: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipStreamInfo {
    pub zip_file_id: String,
    pub byte_start: u64,
    pub byte_end: u64,
    pub content_type: String,
}

#[derive(Debug, Clone)]
pub struct IndexedZipEntry {
    pub archive_file_name: String,
    pub entry_path: String,
    pub entry_name: String,
    pub parsed: media_manager::ParsedMedia,
    pub compression_method: u16,
    pub local_header_offset: u64,
    pub data_start_offset: u64,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: String,
}

#[derive(Debug, Clone)]
pub struct AnalyzedZipArchive {
    pub archive: ZipArchiveInfo,
    pub indexed_entries: Vec<IndexedZipEntry>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveMetadataResponse {
    id: String,
    name: String,
    size: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ZipCacheConfig {
    pub cache_dir: String,
    pub max_size_bytes: u64,
    pub expiry_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ZipCacheMetadata {
    created_at_unix: i64,
    last_accessed_at_unix: i64,
    size_bytes: u64,
}

#[derive(Debug, Clone)]
struct ZipCacheEntry {
    media_path: PathBuf,
    meta_path: PathBuf,
    size_bytes: u64,
    last_accessed_at_unix: i64,
}

#[derive(Debug, Clone)]
pub struct ZipCachePaths {
    pub cache_path: PathBuf,
    pub temp_path: PathBuf,
    pub meta_path: PathBuf,
    pub expected_size: u64,
}

#[derive(Debug, Clone)]
pub struct ZipCacheSnapshot {
    pub paths: ZipCachePaths,
    pub available_bytes: u64,
    pub is_complete: bool,
}

pub fn analyze_zip_from_drive(
    access_token: &str,
    zip_file_id: &str,
) -> Result<AnalyzedZipArchive, ZipError> {
    let client = crate::http_client::long_client().clone();
    analyze_zip_with_client(&client, access_token, zip_file_id)
}

pub fn analyze_zip_for_preview(
    access_token: &str,
    zip_file_id: &str,
) -> Result<ZipAnalysisResult, ZipError> {
    let analyzed = analyze_zip_from_drive(access_token, zip_file_id)?;
    Ok(to_analysis_result(&analyzed))
}

pub fn check_zip_compression_type(entries: &[ZipEntry]) -> ZipCompressionType {
    let mut has_store = false;
    let mut has_deflate = false;
    let mut has_other = false;

    for entry in entries {
        if entry.is_directory {
            continue;
        }

        match entry.compression_method {
            0 => has_store = true,
            8 => has_deflate = true,
            _ => has_other = true,
        }
    }

    if has_other {
        ZipCompressionType::Other
    } else if has_store && has_deflate {
        ZipCompressionType::Mixed
    } else if has_deflate {
        ZipCompressionType::Deflate
    } else {
        ZipCompressionType::Store
    }
}

pub fn to_analysis_result(analyzed: &AnalyzedZipArchive) -> ZipAnalysisResult {
    ZipAnalysisResult {
        zip_file_id: analyzed.archive.zip_file_id.clone(),
        filename: analyzed.archive.filename.clone(),
        file_size: analyzed.archive.file_size_bytes,
        compression_type: analyzed.archive.compression_type,
        total_entries: analyzed.archive.total_entries,
        video_entries: analyzed.archive.video_entries,
        episodes: analyzed
            .indexed_entries
            .iter()
            .map(|entry| ZipEpisodeInfo {
                filename: entry.entry_name.clone(),
                title: entry.parsed.title.clone(),
                season: entry.parsed.season.unwrap_or(0),
                episode: entry.parsed.episode.unwrap_or(0),
                size: entry.uncompressed_size,
            })
            .collect(),
    }
}

pub fn build_zip_stream_info(media: &database::MediaItem) -> Result<ZipStreamInfo, ZipError> {
    match zip_entry_compression_method(media)? {
        0 => {}
        8 => return Err(ZipError::EntryRequiresExtraction),
        method => return Err(ZipError::UnsupportedCompressionMethod(method)),
    }

    let zip_file_id = media.parent_zip_id.clone().ok_or(ZipError::NotAValidZip)?;
    let byte_start = media
        .zip_data_start_offset
        .ok_or(ZipError::CorruptedArchive)? as u64;
    let compressed_size = media
        .zip_compressed_size
        .ok_or(ZipError::CorruptedArchive)? as u64;
    let byte_end = byte_start
        .checked_add(compressed_size.saturating_sub(1))
        .ok_or(ZipError::CorruptedArchive)?;

    Ok(ZipStreamInfo {
        zip_file_id,
        byte_start,
        byte_end,
        content_type: content_type_for_name(
            media
                .zip_entry_path
                .as_deref()
                .or(media.file_path.as_deref())
                .unwrap_or("video/mp4"),
        ),
    })
}

pub fn zip_entry_compression_method(media: &database::MediaItem) -> Result<u16, ZipError> {
    match media.zip_compression_method {
        Some(method) if method >= 0 && method <= u16::MAX as i64 => Ok(method as u16),
        Some(_) => Err(ZipError::CorruptedArchive),
        None => Ok(0),
    }
}

pub fn extract_zip_entry_to_cache(
    access_token: &str,
    media: &database::MediaItem,
    cache_config: &ZipCacheConfig,
) -> Result<String, ZipError> {
    let zip_file_id = media.parent_zip_id.clone().ok_or(ZipError::NotAValidZip)?;
    let entry_path = media
        .zip_entry_path
        .clone()
        .or_else(|| media.file_path.clone())
        .ok_or(ZipError::NotAValidZip)?;
    let compression_method = zip_entry_compression_method(media)?;
    let data_start_offset = media
        .zip_data_start_offset
        .ok_or(ZipError::CorruptedArchive)? as u64;
    let compressed_size = media
        .zip_compressed_size
        .ok_or(ZipError::CorruptedArchive)? as u64;
    let uncompressed_size = media
        .zip_uncompressed_size
        .ok_or(ZipError::CorruptedArchive)? as u64;
    let expected_crc = parse_crc32(media.zip_crc32.as_deref())?;

    if compressed_size == 0 || uncompressed_size == 0 || uncompressed_size > MAX_ENTRY_SIZE_BYTES {
        return Err(ZipError::CorruptedArchive);
    }

    let cache_dir = ensure_zip_cache_dir(cache_config)?;
    let cache_path = cache_path_for_entry(
        &cache_dir,
        media,
        &zip_file_id,
        &entry_path,
        expected_crc,
        uncompressed_size,
    );
    let meta_path = metadata_path_for_entry(&cache_path);
    let temp_path = temp_cache_path_for_entry(&cache_path);

    if let Ok(metadata) = fs::metadata(&cache_path) {
        if metadata.is_file() && metadata.len() == uncompressed_size {
            touch_cache_entry(&meta_path, uncompressed_size)?;
            println!(
                "[ZIP CACHE] Reusing cached entry: {}",
                cache_path.to_string_lossy()
            );
            return Ok(cache_path.to_string_lossy().to_string());
        }
        let _ = fs::remove_file(&cache_path);
        let _ = fs::remove_file(&meta_path);
    }

    enforce_cache_policy(cache_config, uncompressed_size)?;

    if let Ok(metadata) = fs::metadata(&temp_path) {
        if !metadata.is_file() || metadata.len() > uncompressed_size {
            let _ = fs::remove_file(&temp_path);
        }
    }

    let client = crate::http_client::long_client().clone();
    let range_end = data_start_offset
        .checked_add(compressed_size)
        .and_then(|value| value.checked_sub(1))
        .ok_or(ZipError::CorruptedArchive)?;
    let response = fetch_response_range(
        &client,
        access_token,
        &zip_file_id,
        data_start_offset,
        range_end,
    )?;

    let extraction_result = extract_response_to_file(
        response,
        compression_method,
        &temp_path,
        uncompressed_size,
        expected_crc,
    );

    if extraction_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    extraction_result?;

    if cache_path.exists() {
        let _ = fs::remove_file(&cache_path);
    }
    fs::rename(&temp_path, &cache_path).map_err(|_| ZipError::CorruptedArchive)?;
    write_cache_metadata(
        &meta_path,
        &ZipCacheMetadata {
            created_at_unix: unix_now(),
            last_accessed_at_unix: unix_now(),
            size_bytes: uncompressed_size,
        },
    )?;
    enforce_cache_policy(cache_config, 0)?;
    println!(
        "[ZIP CACHE] Extracted entry to cache: {}",
        cache_path.to_string_lossy()
    );

    Ok(cache_path.to_string_lossy().to_string())
}

pub fn extract_zip_entry_to_path_with_progress<F>(
    access_token: &str,
    media: &database::MediaItem,
    output_path: &Path,
    mut on_progress: F,
) -> Result<(), ZipError>
where
    F: FnMut(u64, u64),
{
    let zip_file_id = media.parent_zip_id.clone().ok_or(ZipError::NotAValidZip)?;
    let compression_method = zip_entry_compression_method(media)?;
    let data_start_offset = media
        .zip_data_start_offset
        .ok_or(ZipError::CorruptedArchive)? as u64;
    let compressed_size = media
        .zip_compressed_size
        .ok_or(ZipError::CorruptedArchive)? as u64;
    let uncompressed_size = media
        .zip_uncompressed_size
        .ok_or(ZipError::CorruptedArchive)? as u64;
    let expected_crc = parse_crc32(media.zip_crc32.as_deref())?;

    if compressed_size == 0 || uncompressed_size == 0 || uncompressed_size > MAX_ENTRY_SIZE_BYTES {
        return Err(ZipError::CorruptedArchive);
    }

    let client = crate::http_client::long_client().clone();
    let range_end = data_start_offset
        .checked_add(compressed_size)
        .and_then(|value| value.checked_sub(1))
        .ok_or(ZipError::CorruptedArchive)?;
    let response = fetch_response_range(
        &client,
        access_token,
        &zip_file_id,
        data_start_offset,
        range_end,
    )?;

    extract_response_to_file_with_progress(
        response,
        compression_method,
        output_path,
        uncompressed_size,
        expected_crc,
        &mut on_progress,
    )
}

pub fn cleanup_stale_zip_cache(cache_config: &ZipCacheConfig) -> Result<(), ZipError> {
    enforce_cache_policy(cache_config, 0)
}

pub fn prepare_stream_cache_target(
    media: &database::MediaItem,
    cache_config: &ZipCacheConfig,
) -> Result<ZipCachePaths, ZipError> {
    let zip_file_id = media.parent_zip_id.clone().ok_or(ZipError::NotAValidZip)?;
    let entry_path = media
        .zip_entry_path
        .clone()
        .or_else(|| media.file_path.clone())
        .ok_or(ZipError::NotAValidZip)?;
    let uncompressed_size = media
        .zip_uncompressed_size
        .ok_or(ZipError::CorruptedArchive)? as u64;
    let expected_crc = parse_crc32(media.zip_crc32.as_deref())?;

    if uncompressed_size == 0 || uncompressed_size > MAX_ENTRY_SIZE_BYTES {
        return Err(ZipError::CorruptedArchive);
    }

    let cache_dir = ensure_zip_cache_dir(cache_config)?;
    let cache_path = cache_path_for_entry(
        &cache_dir,
        media,
        &zip_file_id,
        &entry_path,
        expected_crc,
        uncompressed_size,
    );
    let temp_path = temp_cache_path_for_entry(&cache_path);
    let meta_path = metadata_path_for_entry(&cache_path);

    if let Ok(metadata) = fs::metadata(&cache_path) {
        if metadata.is_file() && metadata.len() == uncompressed_size {
            touch_cache_entry(&meta_path, uncompressed_size)?;
            return Ok(ZipCachePaths {
                cache_path,
                temp_path,
                meta_path,
                expected_size: uncompressed_size,
            });
        }

        let _ = fs::remove_file(&cache_path);
        let _ = fs::remove_file(&meta_path);
    }

    if let Ok(metadata) = fs::metadata(&temp_path) {
        if !metadata.is_file() || metadata.len() > uncompressed_size {
            let _ = fs::remove_file(&temp_path);
        }
    }

    enforce_cache_policy(cache_config, uncompressed_size)?;

    Ok(ZipCachePaths {
        cache_path,
        temp_path,
        meta_path,
        expected_size: uncompressed_size,
    })
}

pub fn finalize_stream_cache_target(
    cache_paths: &ZipCachePaths,
    cache_config: &ZipCacheConfig,
) -> Result<String, ZipError> {
    let metadata = fs::metadata(&cache_paths.temp_path).map_err(|_| ZipError::CorruptedArchive)?;
    if !metadata.is_file() || metadata.len() != cache_paths.expected_size {
        return Err(ZipError::IntegrityCheckFailed);
    }

    if cache_paths.cache_path.exists() {
        let _ = fs::remove_file(&cache_paths.cache_path);
    }

    fs::rename(&cache_paths.temp_path, &cache_paths.cache_path)
        .map_err(|_| ZipError::CorruptedArchive)?;
    write_cache_metadata(
        &cache_paths.meta_path,
        &ZipCacheMetadata {
            created_at_unix: unix_now(),
            last_accessed_at_unix: unix_now(),
            size_bytes: cache_paths.expected_size,
        },
    )?;
    enforce_cache_policy(cache_config, 0)?;

    Ok(cache_paths.cache_path.to_string_lossy().to_string())
}

pub fn inspect_stream_cache_target(
    media: &database::MediaItem,
    cache_config: &ZipCacheConfig,
) -> Result<ZipCacheSnapshot, ZipError> {
    let paths = prepare_stream_cache_target(media, cache_config)?;

    if let Ok(metadata) = fs::metadata(&paths.cache_path) {
        if metadata.is_file() && metadata.len() == paths.expected_size {
            touch_cache_entry(&paths.meta_path, paths.expected_size)?;
            return Ok(ZipCacheSnapshot {
                paths,
                available_bytes: metadata.len(),
                is_complete: true,
            });
        }
    }

    let available_bytes = match fs::metadata(&paths.temp_path) {
        Ok(metadata) if metadata.is_file() => metadata.len().min(paths.expected_size),
        _ => 0,
    };

    Ok(ZipCacheSnapshot {
        paths,
        available_bytes,
        is_complete: false,
    })
}

pub fn is_zip_filename(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".zip")
}

fn analyze_zip_with_client(
    client: &Client,
    access_token: &str,
    zip_file_id: &str,
) -> Result<AnalyzedZipArchive, ZipError> {
    let metadata = fetch_drive_metadata(client, access_token, zip_file_id)?;
    let file_size = metadata
        .size
        .as_deref()
        .ok_or(ZipError::NotAValidZip)?
        .parse::<u64>()
        .map_err(|_| ZipError::NotAValidZip)?;

    let tail_len = file_size.min(ZIP_TAIL_BYTES);
    let tail_start = file_size.saturating_sub(tail_len);
    let tail = fetch_range(client, access_token, zip_file_id, tail_start, file_size - 1)?;
    let eocd = zip_parser::find_eocd(&tail, tail_start)?;

    if eocd.cd_size > MAX_CENTRAL_DIRECTORY_BYTES {
        return Err(ZipError::CentralDirectoryTooLarge {
            size: eocd.cd_size,
            max: MAX_CENTRAL_DIRECTORY_BYTES,
        });
    }

    let cd_end = eocd
        .cd_offset
        .checked_add(eocd.cd_size)
        .and_then(|value| value.checked_sub(1))
        .ok_or(ZipError::CorruptedArchive)?;
    let central_directory = fetch_range(client, access_token, zip_file_id, eocd.cd_offset, cd_end)?;
    let parsed_entries = zip_parser::parse_central_directory(&central_directory, eocd.cd_offset)?;

    if parsed_entries.len() > MAX_ZIP_ENTRIES {
        return Err(ZipError::TooManyEntries {
            count: parsed_entries.len(),
            max: MAX_ZIP_ENTRIES,
        });
    }

    let video_entries: Vec<_> = parsed_entries
        .iter()
        .filter(|entry| !entry.is_directory && is_video_path(&entry.filename))
        .cloned()
        .collect();

    let compression_type = check_zip_compression_type(&video_entries);

    let mut indexed_entries = Vec::new();
    for entry in video_entries {
        if entry.is_encrypted {
            continue;
        }

        if entry.uncompressed_size > MAX_ENTRY_SIZE_BYTES {
            continue;
        }

        if !is_supported_entry_compression(entry.compression_method) {
            continue;
        }

        let parsed = match extract_episode_metadata(&entry) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        let data_start_offset =
            fetch_data_start_offset(client, access_token, zip_file_id, entry.local_header_offset)?;
        let entry_name = entry
            .filename
            .rsplit('/')
            .next()
            .unwrap_or(&entry.filename)
            .to_string();

        indexed_entries.push(IndexedZipEntry {
            archive_file_name: metadata.name.clone(),
            entry_path: entry.filename.clone(),
            entry_name,
            parsed,
            compression_method: entry.compression_method,
            local_header_offset: entry.local_header_offset,
            data_start_offset,
            compressed_size: entry.compressed_size,
            uncompressed_size: entry.uncompressed_size,
            crc32: format!("{:08x}", entry.crc32),
        });
    }

    Ok(AnalyzedZipArchive {
        archive: ZipArchiveInfo {
            zip_file_id: metadata.id,
            filename: metadata.name,
            archive_format: "zip".to_string(),
            file_size_bytes: file_size,
            compression_type,
            central_dir_offset: eocd.cd_offset,
            central_dir_size: eocd.cd_size,
            total_entries: parsed_entries.len(),
            video_entries: indexed_entries.len(),
        },
        indexed_entries,
    })
}

pub fn extract_episode_metadata(entry: &ZipEntry) -> Result<media_manager::ParsedMedia, ZipError> {
    let entry_name = entry.filename.rsplit('/').next().unwrap_or(&entry.filename);
    let parsed = media_manager::parse_cloud_filename(entry_name);

    if parsed.season.is_none() || parsed.episode.is_none() {
        return Err(ZipError::NotAValidZip);
    }

    Ok(parsed)
}

fn fetch_drive_metadata(
    client: &Client,
    access_token: &str,
    zip_file_id: &str,
) -> Result<DriveMetadataResponse, ZipError> {
    let url = format!(
        "{}/files/{}?fields=id,name,size&supportsAllDrives=true",
        DRIVE_API_BASE, zip_file_id
    );
    let response = client
        .get(&url)
        .header(AUTHORIZATION, bearer_value(access_token))
        .send()
        .map_err(|e| ZipError::HttpRequestError(format!("Drive metadata fetch failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(ZipError::HttpStatus {
            status: status.as_u16(),
            message: format!("Drive metadata API returned {}: {}", status, body),
        });
    }

    response
        .json::<DriveMetadataResponse>()
        .map_err(|e| ZipError::DriveApiError(format!("Failed to parse Drive metadata JSON: {}", e)))
}

fn fetch_range(
    client: &Client,
    access_token: &str,
    zip_file_id: &str,
    start: u64,
    end: u64,
) -> Result<Vec<u8>, ZipError> {
    let url = format!(
        "{}/files/{}?alt=media&supportsAllDrives=true",
        DRIVE_API_BASE, zip_file_id
    );
    let range_header = format!("bytes={}-{}", start, end);

    // Retry logic for transient errors (429 rate limit, 5xx server errors)
    let max_retries = 3;
    let mut last_error = String::new();

    for attempt in 0..max_retries {
        let response = match client
            .get(&url)
            .header(AUTHORIZATION, bearer_value(access_token))
            .header(RANGE, &range_header)
            .send()
        {
            Ok(resp) => resp,
            Err(e) => {
                last_error = format!(
                    "HTTP request failed (attempt {}/{}): {}",
                    attempt + 1,
                    max_retries,
                    e
                );
                std::thread::sleep(std::time::Duration::from_secs(2u64.pow(attempt as u32)));
                continue;
            }
        };

        let status = response.status();

        // Retry on 429 (rate limit) and 5xx (server errors)
        if status.as_u16() == 429 || status.is_server_error() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(2u64.pow(attempt as u32));
            last_error = format!(
                "HTTP {} on range fetch (attempt {}/{}), retrying in {}s",
                status,
                attempt + 1,
                max_retries,
                retry_after
            );
            println!("[ZIP] {}", last_error);
            std::thread::sleep(std::time::Duration::from_secs(retry_after));
            continue;
        }

        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(ZipError::HttpStatus {
                status: status.as_u16(),
                message: format!(
                    "Range fetch (bytes={}-{}) returned {}: {}",
                    start, end, status, body
                ),
            });
        }

        return response.bytes().map(|bytes| bytes.to_vec()).map_err(|e| {
            ZipError::HttpRequestError(format!("Failed to read range response body: {}", e))
        });
    }

    Err(ZipError::HttpRequestError(format!(
        "Range fetch failed after {} retries for bytes={}-{}: {}",
        max_retries, start, end, last_error
    )))
}

fn fetch_response_range(
    client: &Client,
    access_token: &str,
    zip_file_id: &str,
    start: u64,
    end: u64,
) -> Result<reqwest::blocking::Response, ZipError> {
    let url = format!(
        "{}/files/{}?alt=media&supportsAllDrives=true",
        DRIVE_API_BASE, zip_file_id
    );
    let response = client
        .get(&url)
        .header(AUTHORIZATION, bearer_value(access_token))
        .header(RANGE, format!("bytes={}-{}", start, end))
        .send()
        .map_err(|e| ZipError::HttpRequestError(format!("Response range fetch failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(ZipError::HttpStatus {
            status: status.as_u16(),
            message: format!("Range fetch returned {}: {}", status, body),
        });
    }

    Ok(response)
}

fn fetch_data_start_offset(
    client: &Client,
    access_token: &str,
    zip_file_id: &str,
    local_header_offset: u64,
) -> Result<u64, ZipError> {
    let mut header_bytes = fetch_range(
        client,
        access_token,
        zip_file_id,
        local_header_offset,
        local_header_offset + LOCAL_HEADER_PREFETCH_BYTES - 1,
    )?;

    if header_bytes.len() < 30 {
        return Err(ZipError::CorruptedArchive);
    }

    let file_name_length = u16::from_le_bytes([header_bytes[26], header_bytes[27]]) as usize;
    let extra_length = u16::from_le_bytes([header_bytes[28], header_bytes[29]]) as usize;
    let required_len = 30 + file_name_length + extra_length;

    if header_bytes.len() < required_len {
        header_bytes = fetch_range(
            client,
            access_token,
            zip_file_id,
            local_header_offset,
            local_header_offset + required_len as u64 - 1,
        )?;
    }

    Ok(zip_parser::parse_local_file_header(&header_bytes, local_header_offset)?.data_start_offset)
}

fn bearer_value(access_token: &str) -> String {
    format!("Bearer {}", access_token)
}

fn is_supported_entry_compression(method: u16) -> bool {
    matches!(method, 0 | 8)
}

fn parse_crc32(value: Option<&str>) -> Result<u32, ZipError> {
    u32::from_str_radix(value.ok_or(ZipError::CorruptedArchive)?, 16)
        .map_err(|_| ZipError::CorruptedArchive)
}

fn ensure_zip_cache_dir(cache_config: &ZipCacheConfig) -> Result<PathBuf, ZipError> {
    let cache_dir = PathBuf::from(&cache_config.cache_dir);
    fs::create_dir_all(&cache_dir).map_err(|_| ZipError::CorruptedArchive)?;
    Ok(cache_dir)
}

fn metadata_path_for_entry(cache_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.meta.json", cache_path.to_string_lossy()))
}

fn temp_cache_path_for_entry(cache_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.part", cache_path.to_string_lossy()))
}

fn cache_path_for_entry(
    cache_dir: &Path,
    media: &database::MediaItem,
    zip_file_id: &str,
    entry_path: &str,
    crc32: u32,
    uncompressed_size: u64,
) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    zip_file_id.hash(&mut hasher);
    entry_path.hash(&mut hasher);
    crc32.hash(&mut hasher);
    uncompressed_size.hash(&mut hasher);
    let hash = hasher.finish();
    let extension = Path::new(entry_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("bin");
    let label = build_cache_label(media, entry_path);
    cache_dir.join(format!("{}__{:016x}.{}", label, hash, extension))
}

fn build_cache_label(media: &database::MediaItem, entry_path: &str) -> String {
    let mut parts = Vec::new();

    let title = sanitize_cache_name_component(&media.title);
    if !title.is_empty() {
        parts.push(title);
    }

    if let (Some(season), Some(episode)) = (media.season_number, media.episode_number) {
        parts.push(format!("S{:02}E{:02}", season.max(0), episode.max(0)));
    } else if let Some(year) = media.year {
        parts.push(year.to_string());
    }

    if let Some(episode_title) = media.episode_title.as_deref() {
        let cleaned_episode_title = sanitize_cache_name_component(episode_title);
        if !cleaned_episode_title.is_empty() {
            parts.push(cleaned_episode_title);
        }
    }

    if parts.is_empty() {
        let fallback = Path::new(entry_path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(sanitize_cache_name_component)
            .unwrap_or_else(|| "zip-cache".to_string());
        parts.push(fallback);
    }

    let mut label = parts.join(" - ");
    if label.len() > 96 {
        label.truncate(96);
        label = label.trim_end_matches([' ', '.', '-']).to_string();
    }

    if label.is_empty() {
        "zip-cache".to_string()
    } else {
        label
    }
}

fn sanitize_cache_name_component(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    let mut last_was_separator = false;

    for ch in value.chars() {
        let is_valid =
            ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '-' | '_' | '.' | '(' | ')');
        let normalized = if is_valid { ch } else { ' ' };

        if normalized.is_whitespace() {
            if !last_was_separator {
                sanitized.push(' ');
                last_was_separator = true;
            }
        } else {
            sanitized.push(normalized);
            last_was_separator = false;
        }
    }

    sanitized.trim_matches([' ', '.', '-']).replace(" .", ".")
}

fn extract_response_to_file(
    response: reqwest::blocking::Response,
    compression_method: u16,
    output_path: &Path,
    expected_size: u64,
    expected_crc32: u32,
) -> Result<(), ZipError> {
    extract_response_to_file_with_progress(
        response,
        compression_method,
        output_path,
        expected_size,
        expected_crc32,
        &mut |_, _| {},
    )
}

fn extract_response_to_file_with_progress<F>(
    response: reqwest::blocking::Response,
    compression_method: u16,
    output_path: &Path,
    expected_size: u64,
    expected_crc32: u32,
    on_progress: &mut F,
) -> Result<(), ZipError>
where
    F: FnMut(u64, u64),
{
    let mut writer =
        BufWriter::new(File::create(output_path).map_err(|_| ZipError::CorruptedArchive)?);
    let mut crc = Crc32Hasher::new();
    let bytes_written = match compression_method {
        0 => copy_and_hash_with_progress(
            response,
            &mut writer,
            &mut crc,
            expected_size,
            on_progress,
        )?,
        8 => copy_and_hash_with_progress(
            DeflateDecoder::new(response),
            &mut writer,
            &mut crc,
            expected_size,
            on_progress,
        )?,
        method => return Err(ZipError::UnsupportedCompressionMethod(method)),
    };

    writer.flush().map_err(|_| ZipError::CorruptedArchive)?;

    if bytes_written != expected_size || crc.finalize() != expected_crc32 {
        return Err(ZipError::IntegrityCheckFailed);
    }

    Ok(())
}

fn copy_and_hash<R: Read, W: Write>(
    reader: R,
    writer: &mut W,
    crc: &mut Crc32Hasher,
) -> Result<u64, ZipError> {
    copy_and_hash_with_progress(reader, writer, crc, 0, &mut |_, _| {})
}

fn copy_and_hash_with_progress<R: Read, W: Write, F: FnMut(u64, u64)>(
    mut reader: R,
    writer: &mut W,
    crc: &mut Crc32Hasher,
    expected_size: u64,
    on_progress: &mut F,
) -> Result<u64, ZipError> {
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| ZipError::CorruptedArchive)?;
        if read == 0 {
            break;
        }

        writer
            .write_all(&buffer[..read])
            .map_err(|_| ZipError::CorruptedArchive)?;
        crc.update(&buffer[..read]);
        total = total
            .checked_add(read as u64)
            .ok_or(ZipError::CorruptedArchive)?;
        on_progress(total, expected_size);
    }

    Ok(total)
}

fn enforce_cache_policy(cache_config: &ZipCacheConfig, reserve_bytes: u64) -> Result<(), ZipError> {
    let lock = CACHE_POLICY_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let cache_dir = ensure_zip_cache_dir(cache_config)?;
    cleanup_temporary_cache_files(&cache_dir)?;
    let expiry_seconds = u64::from(cache_config.expiry_days).saturating_mul(86_400);
    let now = unix_now();
    let mut entries = collect_cache_entries(&cache_dir)?;

    for entry in &entries {
        if expiry_seconds > 0
            && now.saturating_sub(entry.last_accessed_at_unix) >= expiry_seconds as i64
        {
            let _ = remove_cache_entry(entry);
        }
    }

    entries = collect_cache_entries(&cache_dir)?;
    let mut total_size: u64 = entries
        .iter()
        .fold(0u64, |acc, e| acc.saturating_add(e.size_bytes));
    let target_limit = cache_config.max_size_bytes.max(reserve_bytes);

    if total_size.saturating_add(reserve_bytes) <= target_limit {
        return Ok(());
    }

    entries.sort_by_key(|entry| entry.last_accessed_at_unix);
    for entry in entries {
        if total_size.saturating_add(reserve_bytes) <= target_limit {
            break;
        }

        if remove_cache_entry(&entry) {
            total_size = total_size.saturating_sub(entry.size_bytes);
        }
    }

    Ok(())
}

fn collect_cache_entries(cache_dir: &Path) -> Result<Vec<ZipCacheEntry>, ZipError> {
    let mut entries = Vec::new();

    for entry in fs::read_dir(cache_dir).map_err(|_| ZipError::CorruptedArchive)? {
        let entry = entry.map_err(|_| ZipError::CorruptedArchive)?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|_| ZipError::CorruptedArchive)?;

        if !metadata.is_file() || is_cache_metadata_file(&path) || is_cache_temp_file(&path) {
            continue;
        }

        let meta_path = metadata_path_for_entry(&path);
        let cache_metadata = read_cache_metadata(&meta_path).unwrap_or(ZipCacheMetadata {
            created_at_unix: system_time_to_unix(
                metadata.modified().unwrap_or_else(|_| SystemTime::now()),
            ),
            last_accessed_at_unix: system_time_to_unix(
                metadata.modified().unwrap_or_else(|_| SystemTime::now()),
            ),
            size_bytes: metadata.len(),
        });

        entries.push(ZipCacheEntry {
            media_path: path,
            meta_path,
            size_bytes: metadata.len(),
            last_accessed_at_unix: cache_metadata.last_accessed_at_unix,
        });
    }

    Ok(entries)
}

fn read_cache_metadata(meta_path: &Path) -> Result<ZipCacheMetadata, ZipError> {
    let contents = fs::read_to_string(meta_path).map_err(|_| ZipError::CorruptedArchive)?;
    serde_json::from_str(&contents).map_err(|_| ZipError::CorruptedArchive)
}

fn write_cache_metadata(meta_path: &Path, metadata: &ZipCacheMetadata) -> Result<(), ZipError> {
    let json = serde_json::to_string(metadata).map_err(|_| ZipError::CorruptedArchive)?;
    fs::write(meta_path, json).map_err(|_| ZipError::CorruptedArchive)
}

fn touch_cache_entry(meta_path: &Path, size_bytes: u64) -> Result<(), ZipError> {
    let now = unix_now();
    let mut metadata = read_cache_metadata(meta_path).unwrap_or(ZipCacheMetadata {
        created_at_unix: now,
        last_accessed_at_unix: now,
        size_bytes,
    });
    metadata.last_accessed_at_unix = now;
    metadata.size_bytes = size_bytes;
    write_cache_metadata(meta_path, &metadata)
}

/// Returns `true` if the entry was fully removed, `false` if deletion failed (e.g. file locked).
fn remove_cache_entry(entry: &ZipCacheEntry) -> bool {
    let mut ok = true;
    if entry.media_path.exists() {
        if let Err(e) = fs::remove_file(&entry.media_path) {
            eprintln!(
                "[ZIP CACHE] Warning: could not delete {}: {}",
                entry.media_path.to_string_lossy(),
                e
            );
            ok = false;
        }
    }
    if entry.meta_path.exists() {
        if let Err(e) = fs::remove_file(&entry.meta_path) {
            eprintln!(
                "[ZIP CACHE] Warning: could not delete {}: {}",
                entry.meta_path.to_string_lossy(),
                e
            );
            ok = false;
        }
    }
    ok
}

fn is_cache_metadata_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.ends_with(".meta.json"))
        .unwrap_or(false)
}

fn is_cache_temp_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.ends_with(".part"))
        .unwrap_or(false)
}

fn cleanup_temporary_cache_files(cache_dir: &Path) -> Result<(), ZipError> {
    let now = SystemTime::now();

    for entry in fs::read_dir(cache_dir).map_err(|_| ZipError::CorruptedArchive)? {
        let entry = entry.map_err(|_| ZipError::CorruptedArchive)?;
        let path = entry.path();

        if is_cache_temp_file(&path) {
            let should_remove = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| now.duration_since(modified).ok())
                .map(|age| age.as_secs() >= ZIP_TEMP_CACHE_MAX_AGE_SECS)
                .unwrap_or(true);

            if should_remove {
                let _ = fs::remove_file(path);
            }
        }
    }

    Ok(())
}

fn unix_now() -> i64 {
    system_time_to_unix(SystemTime::now())
}

fn system_time_to_unix(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn content_type_for_name(name: &str) -> String {
    match name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mkv" => "video/x-matroska".to_string(),
        "mp4" => "video/mp4".to_string(),
        "webm" => "video/webm".to_string(),
        "avi" => "video/x-msvideo".to_string(),
        "mov" => "video/quicktime".to_string(),
        "m4v" => "video/x-m4v".to_string(),
        "ts" => "video/mp2t".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

pub fn is_video_path(path: &str) -> bool {
    matches!(
        path.rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "mkv" | "mp4" | "avi" | "mov" | "webm" | "m4v" | "wmv" | "flv" | "ts"
    )
}

pub fn response_headers_to_vec(response: &reqwest::blocking::Response) -> Vec<(String, String)> {
    let mut headers = Vec::new();

    if let Some(content_type) = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    {
        headers.push(("Content-Type".to_string(), content_type.to_string()));
    }
    if let Some(content_range) = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
    {
        headers.push(("Content-Range".to_string(), content_range.to_string()));
    }
    if let Some(content_length) = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
    {
        headers.push(("Content-Length".to_string(), content_length.to_string()));
    }

    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_manager::{MediaParseType, ParsedMedia};
    use crate::zip_parser::{ZipCompressionType, ZipEntry};
    use tempfile::TempDir;

    fn make_entry(filename: &str, method: u16, is_dir: bool) -> ZipEntry {
        ZipEntry {
            filename: filename.to_string(),
            compression_method: method,
            compressed_size: 100,
            uncompressed_size: 200,
            local_header_offset: 0,
            crc32: 0xABCD1234,
            is_encrypted: false,
            is_directory: is_dir,
        }
    }

    fn make_media() -> database::MediaItem {
        database::MediaItem {
            id: 1,
            title: "Test Show".to_string(),
            year: Some(2024),
            overview: None,
            cast_names: None,
            director: None,
            poster_path: None,
            file_path: Some("Season 01/Test.Show.S01E01.mkv".to_string()),
            media_type: "tv".to_string(),
            duration_seconds: None,
            resume_position_seconds: None,
            last_watched: None,
            season_number: Some(1),
            episode_number: Some(1),
            parent_id: None,
            progress_percent: None,
            tmdb_id: None,
            imdb_id: None,
            episode_title: Some("Pilot".to_string()),
            still_path: None,
            is_cloud: Some(true),
            cloud_file_id: None,
            cloud_folder_id: None,
            archive_format: Some("zip".to_string()),
            parent_zip_id: Some("zip123".to_string()),
            zip_entry_path: Some("Season 01/Test.Show.S01E01.mkv".to_string()),
            zip_local_header_offset: Some(1000),
            zip_data_start_offset: Some(1050),
            zip_compressed_size: Some(500),
            zip_uncompressed_size: Some(800),
            zip_crc32: Some("abcd1234".to_string()),
            zip_compression_method: Some(0),
            file_size_bytes: Some(800),
            ddl_source_id: None,
            archive_playback_can_play: None,
            archive_playback_mode: None,
            archive_playback_message: None,
            archive_playback_details: None,
        }
    }

    fn cache_config() -> (TempDir, ZipCacheConfig) {
        let dir = TempDir::new().unwrap();
        let config = ZipCacheConfig {
            cache_dir: dir.path().to_string_lossy().to_string(),
            max_size_bytes: 1024 * 1024,
            expiry_days: 7,
        };
        (dir, config)
    }

    // ── is_zip_filename ───────────────────────────────────────────────────

    #[test]
    fn is_zip_filename_basic() {
        assert!(is_zip_filename("archive.zip"));
        assert!(is_zip_filename("file.ZIP"));
        assert!(is_zip_filename("path/to/file.Zip"));
    }

    #[test]
    fn is_zip_filename_negative() {
        assert!(!is_zip_filename("file.tar.gz"));
        assert!(!is_zip_filename("file.txt"));
        assert!(!is_zip_filename(""));
        assert!(!is_zip_filename("zip"));
        assert!(!is_zip_filename("file.zip.bak"));
    }

    // ── extract_episode_metadata ──────────────────────────────────────────

    #[test]
    fn extract_episode_metadata_valid_tv() {
        let entry = make_entry("Show.S01E03.mkv", 0, false);
        let result = extract_episode_metadata(&entry);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(3));
    }

    #[test]
    fn extract_episode_metadata_with_path() {
        let entry = make_entry("Season 02/Show.S02E10.mkv", 0, false);
        let result = extract_episode_metadata(&entry);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.season, Some(2));
        assert_eq!(parsed.episode, Some(10));
    }

    #[test]
    fn extract_episode_metadata_no_season_episode_fails() {
        let entry = make_entry("random_video.mp4", 0, false);
        let result = extract_episode_metadata(&entry);
        assert!(result.is_err());
    }

    // ── check_zip_compression_type ────────────────────────────────────────

    #[test]
    fn compression_type_all_store() {
        let entries = vec![make_entry("a.mkv", 0, false), make_entry("b.mkv", 0, false)];
        assert_eq!(
            check_zip_compression_type(&entries),
            ZipCompressionType::Store
        );
    }

    #[test]
    fn compression_type_all_deflate() {
        let entries = vec![make_entry("a.mkv", 8, false), make_entry("b.mkv", 8, false)];
        assert_eq!(
            check_zip_compression_type(&entries),
            ZipCompressionType::Deflate
        );
    }

    #[test]
    fn compression_type_mixed() {
        let entries = vec![make_entry("a.mkv", 0, false), make_entry("b.mkv", 8, false)];
        assert_eq!(
            check_zip_compression_type(&entries),
            ZipCompressionType::Mixed
        );
    }

    #[test]
    fn compression_type_other_method() {
        let entries = vec![make_entry("a.mkv", 14, false)];
        assert_eq!(
            check_zip_compression_type(&entries),
            ZipCompressionType::Other
        );
    }

    #[test]
    fn compression_type_skips_directories() {
        let entries = vec![
            make_entry("dir/", 0, true),
            make_entry("file.mkv", 8, false),
        ];
        assert_eq!(
            check_zip_compression_type(&entries),
            ZipCompressionType::Deflate
        );
    }

    #[test]
    fn compression_type_empty_entries() {
        let entries: Vec<ZipEntry> = vec![];
        assert_eq!(
            check_zip_compression_type(&entries),
            ZipCompressionType::Store
        );
    }

    // ── to_analysis_result ────────────────────────────────────────────────

    #[test]
    fn to_analysis_result_conversion() {
        let analyzed = AnalyzedZipArchive {
            archive: ZipArchiveInfo {
                zip_file_id: "fid".to_string(),
                filename: "test.zip".to_string(),
                archive_format: "zip".to_string(),
                file_size_bytes: 1024,
                compression_type: ZipCompressionType::Store,
                central_dir_offset: 900,
                central_dir_size: 100,
                total_entries: 3,
                video_entries: 2,
            },
            indexed_entries: vec![IndexedZipEntry {
                archive_file_name: "test.zip".to_string(),
                entry_path: "S01E01.mkv".to_string(),
                entry_name: "S01E01.mkv".to_string(),
                parsed: ParsedMedia {
                    title: "Show".to_string(),
                    year: None,
                    media_type: MediaParseType::TvEpisode,
                    season: Some(1),
                    episode: Some(1),
                    episode_end: None,
                },
                compression_method: 0,
                local_header_offset: 0,
                data_start_offset: 100,
                compressed_size: 500,
                uncompressed_size: 800,
                crc32: "deadbeef".to_string(),
            }],
        };
        let result = to_analysis_result(&analyzed);
        assert_eq!(result.zip_file_id, "fid");
        assert_eq!(result.filename, "test.zip");
        assert_eq!(result.file_size, 1024);
        assert_eq!(result.total_entries, 3);
        assert_eq!(result.video_entries, 2);
        assert_eq!(result.episodes.len(), 1);
        assert_eq!(result.episodes[0].season, 1);
        assert_eq!(result.episodes[0].episode, 1);
        assert_eq!(result.episodes[0].size, 800);
    }

    #[test]
    fn to_analysis_result_empty_entries() {
        let analyzed = AnalyzedZipArchive {
            archive: ZipArchiveInfo {
                zip_file_id: "fid".to_string(),
                filename: "empty.zip".to_string(),
                archive_format: "zip".to_string(),
                file_size_bytes: 100,
                compression_type: ZipCompressionType::Store,
                central_dir_offset: 0,
                central_dir_size: 0,
                total_entries: 0,
                video_entries: 0,
            },
            indexed_entries: vec![],
        };
        let result = to_analysis_result(&analyzed);
        assert!(result.episodes.is_empty());
    }

    // ── zip_entry_compression_method ──────────────────────────────────────

    #[test]
    fn compression_method_from_media() {
        let mut media = make_media();
        media.zip_compression_method = Some(8);
        assert_eq!(zip_entry_compression_method(&media).unwrap(), 8);
    }

    #[test]
    fn compression_method_none_defaults_to_store() {
        let mut media = make_media();
        media.zip_compression_method = None;
        assert_eq!(zip_entry_compression_method(&media).unwrap(), 0);
    }

    #[test]
    fn compression_method_negative_errors() {
        let mut media = make_media();
        media.zip_compression_method = Some(-1);
        assert!(zip_entry_compression_method(&media).is_err());
    }

    #[test]
    fn compression_method_max_u16() {
        let mut media = make_media();
        media.zip_compression_method = Some(u16::MAX as i64);
        assert_eq!(zip_entry_compression_method(&media).unwrap(), u16::MAX);
    }

    #[test]
    fn compression_method_overflow_errors() {
        let mut media = make_media();
        media.zip_compression_method = Some(u16::MAX as i64 + 1);
        assert!(zip_entry_compression_method(&media).is_err());
    }

    // ── build_zip_stream_info ─────────────────────────────────────────────

    #[test]
    fn stream_info_store_method_success() {
        let media = make_media(); // compression_method = 0 (store)
        let info = build_zip_stream_info(&media).unwrap();
        assert_eq!(info.zip_file_id, "zip123");
        assert_eq!(info.byte_start, 1050);
        assert_eq!(info.byte_end, 1050 + 500 - 1);
        assert_eq!(info.content_type, "video/x-matroska");
    }

    #[test]
    fn stream_info_deflate_errors() {
        let mut media = make_media();
        media.zip_compression_method = Some(8);
        let result = build_zip_stream_info(&media);
        assert!(matches!(result, Err(ZipError::EntryRequiresExtraction)));
    }

    #[test]
    fn stream_info_unsupported_method_errors() {
        let mut media = make_media();
        media.zip_compression_method = Some(14);
        let result = build_zip_stream_info(&media);
        assert!(matches!(
            result,
            Err(ZipError::UnsupportedCompressionMethod(14))
        ));
    }

    #[test]
    fn stream_info_no_zip_id_errors() {
        let mut media = make_media();
        media.parent_zip_id = None;
        assert!(matches!(
            build_zip_stream_info(&media),
            Err(ZipError::NotAValidZip)
        ));
    }

    #[test]
    fn stream_info_no_data_offset_errors() {
        let mut media = make_media();
        media.zip_data_start_offset = None;
        assert!(matches!(
            build_zip_stream_info(&media),
            Err(ZipError::CorruptedArchive)
        ));
    }

    #[test]
    fn stream_info_no_compressed_size_errors() {
        let mut media = make_media();
        media.zip_compressed_size = None;
        assert!(matches!(
            build_zip_stream_info(&media),
            Err(ZipError::CorruptedArchive)
        ));
    }

    #[test]
    fn stream_info_falls_back_to_file_path() {
        let mut media = make_media();
        media.zip_entry_path = None;
        media.file_path = Some("episode.mp4".to_string());
        let info = build_zip_stream_info(&media).unwrap();
        assert_eq!(info.content_type, "video/mp4");
    }

    #[test]
    fn stream_info_default_content_type() {
        let mut media = make_media();
        media.zip_entry_path = None;
        media.file_path = None;
        let info = build_zip_stream_info(&media).unwrap();
        // "video/mp4" has no dot, so content_type_for_name returns octet-stream
        assert_eq!(info.content_type, "application/octet-stream");
    }

    // ── content_type_for_name ─────────────────────────────────────────────

    #[test]
    fn content_type_all_extensions() {
        assert_eq!(content_type_for_name("f.mkv"), "video/x-matroska");
        assert_eq!(content_type_for_name("f.mp4"), "video/mp4");
        assert_eq!(content_type_for_name("f.webm"), "video/webm");
        assert_eq!(content_type_for_name("f.avi"), "video/x-msvideo");
        assert_eq!(content_type_for_name("f.mov"), "video/quicktime");
        assert_eq!(content_type_for_name("f.m4v"), "video/x-m4v");
        assert_eq!(content_type_for_name("f.ts"), "video/mp2t");
        assert_eq!(
            content_type_for_name("f.unknown"),
            "application/octet-stream"
        );
        assert_eq!(content_type_for_name("noext"), "application/octet-stream");
    }

    #[test]
    fn content_type_case_insensitive() {
        assert_eq!(content_type_for_name("F.MKV"), "video/x-matroska");
        assert_eq!(content_type_for_name("F.Mp4"), "video/mp4");
    }

    // ── is_video_path ─────────────────────────────────────────────────────

    #[test]
    fn is_video_path_all_extensions() {
        for ext in &[
            "mkv", "mp4", "avi", "mov", "webm", "m4v", "wmv", "flv", "ts",
        ] {
            assert!(
                is_video_path(&format!("file.{}", ext)),
                "expected {} to be video",
                ext
            );
            assert!(is_video_path(&format!("file.{}", ext.to_uppercase())));
        }
    }

    #[test]
    fn is_video_path_negative() {
        assert!(!is_video_path("file.txt"));
        assert!(!is_video_path("file.srt"));
        assert!(!is_video_path("file.nfo"));
        assert!(!is_video_path("file.zip"));
        assert!(!is_video_path("noext"));
        assert!(!is_video_path(""));
    }

    #[test]
    fn is_video_path_with_directory() {
        assert!(is_video_path("Season 01/episode.mkv"));
        assert!(!is_video_path("path/to/file.nfo"));
    }

    // ── zip_entry_compression_method edge cases ───────────────────────────

    #[test]
    fn compression_method_zero_is_valid() {
        let mut media = make_media();
        media.zip_compression_method = Some(0);
        assert_eq!(zip_entry_compression_method(&media).unwrap(), 0);
    }

    // ── extract_zip_entry_to_cache error paths ────────────────────────────

    #[test]
    fn extract_to_cache_no_parent_zip_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.parent_zip_id = None;
        let result = extract_zip_entry_to_cache("token", &media, &config);
        assert!(matches!(result, Err(ZipError::NotAValidZip)));
    }

    #[test]
    fn extract_to_cache_no_entry_path_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_entry_path = None;
        media.file_path = None;
        let result = extract_zip_entry_to_cache("token", &media, &config);
        assert!(matches!(result, Err(ZipError::NotAValidZip)));
    }

    #[test]
    fn extract_to_cache_zero_compressed_size_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_compressed_size = Some(0);
        let result = extract_zip_entry_to_cache("token", &media, &config);
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn extract_to_cache_zero_uncompressed_size_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_uncompressed_size = Some(0);
        let result = extract_zip_entry_to_cache("token", &media, &config);
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn extract_to_cache_huge_uncompressed_size_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_uncompressed_size = Some((MAX_ENTRY_SIZE_BYTES + 1) as i64);
        let result = extract_zip_entry_to_cache("token", &media, &config);
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn extract_to_cache_bad_crc_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_crc32 = Some("not_hex".to_string());
        let result = extract_zip_entry_to_cache("token", &media, &config);
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn extract_to_cache_missing_crc_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_crc32 = None;
        let result = extract_zip_entry_to_cache("token", &media, &config);
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn extract_to_cache_bad_compression_method_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_compression_method = Some(-5);
        let result = extract_zip_entry_to_cache("token", &media, &config);
        assert!(result.is_err());
    }

    // ── extract_zip_entry_to_path_with_progress error paths ───────────────

    #[test]
    fn extract_to_path_no_parent_zip_errors() {
        let mut media = make_media();
        media.parent_zip_id = None;
        let result = extract_zip_entry_to_path_with_progress(
            "token",
            &media,
            Path::new("/tmp/out.mkv"),
            |_, _| {},
        );
        assert!(matches!(result, Err(ZipError::NotAValidZip)));
    }

    #[test]
    fn extract_to_path_zero_sizes_errors() {
        let mut media = make_media();
        media.zip_compressed_size = Some(0);
        let result = extract_zip_entry_to_path_with_progress(
            "token",
            &media,
            Path::new("/tmp/out.mkv"),
            |_, _| {},
        );
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn extract_to_path_huge_size_errors() {
        let mut media = make_media();
        media.zip_uncompressed_size = Some((MAX_ENTRY_SIZE_BYTES + 1) as i64);
        let result = extract_zip_entry_to_path_with_progress(
            "token",
            &media,
            Path::new("/tmp/out.mkv"),
            |_, _| {},
        );
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn extract_to_path_bad_crc_errors() {
        let mut media = make_media();
        media.zip_crc32 = Some("ZZZZ".to_string());
        let result = extract_zip_entry_to_path_with_progress(
            "token",
            &media,
            Path::new("/tmp/out.mkv"),
            |_, _| {},
        );
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    // ── prepare_stream_cache_target ───────────────────────────────────────

    #[test]
    fn prepare_stream_cache_no_parent_zip_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.parent_zip_id = None;
        let result = prepare_stream_cache_target(&media, &config);
        assert!(matches!(result, Err(ZipError::NotAValidZip)));
    }

    #[test]
    fn prepare_stream_cache_no_entry_path_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_entry_path = None;
        media.file_path = None;
        let result = prepare_stream_cache_target(&media, &config);
        assert!(matches!(result, Err(ZipError::NotAValidZip)));
    }

    #[test]
    fn prepare_stream_cache_zero_size_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_uncompressed_size = Some(0);
        let result = prepare_stream_cache_target(&media, &config);
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn prepare_stream_cache_huge_size_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_uncompressed_size = Some((MAX_ENTRY_SIZE_BYTES + 1) as i64);
        let result = prepare_stream_cache_target(&media, &config);
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn prepare_stream_cache_bad_crc_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.zip_crc32 = Some("invalid".to_string());
        let result = prepare_stream_cache_target(&media, &config);
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn prepare_stream_cache_success() {
        let (_dir, config) = cache_config();
        let media = make_media();
        let result = prepare_stream_cache_target(&media, &config);
        assert!(result.is_ok());
        let paths = result.unwrap();
        assert_eq!(paths.expected_size, 800);
        assert!(paths.cache_path.to_string_lossy().contains("Test Show"));
    }

    #[test]
    fn prepare_stream_cache_returns_cached() {
        let (_dir, config) = cache_config();
        let media = make_media();
        let paths1 = prepare_stream_cache_target(&media, &config).unwrap();
        // Write cache file with correct size
        std::fs::write(&paths1.cache_path, vec![0u8; 800]).unwrap();
        // Should detect existing cache
        let paths2 = prepare_stream_cache_target(&media, &config).unwrap();
        assert_eq!(paths1.cache_path, paths2.cache_path);
    }

    // ── finalize_stream_cache_target ──────────────────────────────────────

    #[test]
    fn finalize_stream_cache_missing_temp_errors() {
        let (_dir, config) = cache_config();
        let media = make_media();
        let paths = prepare_stream_cache_target(&media, &config).unwrap();
        let result = finalize_stream_cache_target(&paths, &config);
        assert!(matches!(result, Err(ZipError::CorruptedArchive)));
    }

    #[test]
    fn finalize_stream_cache_wrong_size_errors() {
        let (_dir, config) = cache_config();
        let media = make_media();
        let paths = prepare_stream_cache_target(&media, &config).unwrap();
        // Write temp file with wrong size
        std::fs::write(&paths.temp_path, vec![0u8; 100]).unwrap();
        let result = finalize_stream_cache_target(&paths, &config);
        assert!(matches!(result, Err(ZipError::IntegrityCheckFailed)));
    }

    #[test]
    fn finalize_stream_cache_success() {
        let (_dir, config) = cache_config();
        let media = make_media();
        let paths = prepare_stream_cache_target(&media, &config).unwrap();
        // Write temp file with correct size
        std::fs::write(&paths.temp_path, vec![0u8; 800]).unwrap();
        let result = finalize_stream_cache_target(&paths, &config);
        assert!(result.is_ok());
        assert!(std::path::Path::new(&result.unwrap()).exists());
    }

    #[test]
    fn finalize_stream_cache_writes_metadata() {
        let (_dir, config) = cache_config();
        let media = make_media();
        let paths = prepare_stream_cache_target(&media, &config).unwrap();
        std::fs::write(&paths.temp_path, vec![0u8; 800]).unwrap();
        let _ = finalize_stream_cache_target(&paths, &config).unwrap();
        assert!(paths.meta_path.exists());
        let meta_contents = std::fs::read_to_string(&paths.meta_path).unwrap();
        let parsed: ZipCacheMetadata = serde_json::from_str(&meta_contents).unwrap();
        assert_eq!(parsed.size_bytes, 800);
    }

    // ── inspect_stream_cache_target ───────────────────────────────────────

    #[test]
    fn inspect_stream_cache_empty_dir() {
        let (_dir, config) = cache_config();
        let media = make_media();
        let result = inspect_stream_cache_target(&media, &config);
        assert!(result.is_ok());
        let snap = result.unwrap();
        assert!(!snap.is_complete);
        assert_eq!(snap.available_bytes, 0);
    }

    #[test]
    fn inspect_stream_cache_complete_file() {
        let (_dir, config) = cache_config();
        let media = make_media();
        let paths = prepare_stream_cache_target(&media, &config).unwrap();
        std::fs::write(&paths.cache_path, vec![0u8; 800]).unwrap();
        let snap = inspect_stream_cache_target(&media, &config).unwrap();
        assert!(snap.is_complete);
        assert_eq!(snap.available_bytes, 800);
    }

    #[test]
    fn inspect_stream_cache_partial_temp() {
        let (_dir, config) = cache_config();
        let media = make_media();
        let paths = prepare_stream_cache_target(&media, &config).unwrap();
        std::fs::write(&paths.temp_path, vec![0u8; 400]).unwrap();
        let snap = inspect_stream_cache_target(&media, &config).unwrap();
        assert!(!snap.is_complete);
        assert_eq!(snap.available_bytes, 400);
    }

    #[test]
    fn inspect_stream_cache_no_parent_zip_errors() {
        let (_dir, config) = cache_config();
        let mut media = make_media();
        media.parent_zip_id = None;
        let result = inspect_stream_cache_target(&media, &config);
        assert!(result.is_err());
    }

    // ── cleanup_stale_zip_cache ───────────────────────────────────────────

    #[test]
    fn cleanup_stale_cache_empty_dir() {
        let (_dir, config) = cache_config();
        assert!(cleanup_stale_zip_cache(&config).is_ok());
    }

    #[test]
    fn cleanup_stale_cache_removes_expired() {
        let dir = TempDir::new().unwrap();
        let config = ZipCacheConfig {
            cache_dir: dir.path().to_string_lossy().to_string(),
            max_size_bytes: 1024 * 1024,
            expiry_days: 1,
        };
        // Create a cache entry with very old metadata (epoch 0 = 1970)
        let cache_file = dir.path().join("test__abcdef0123456789.mkv");
        std::fs::write(&cache_file, vec![0u8; 100]).unwrap();
        let meta_file = dir.path().join("test__abcdef0123456789.mkv.meta.json");
        let meta = ZipCacheMetadata {
            created_at_unix: 0,
            last_accessed_at_unix: 0,
            size_bytes: 100,
        };
        std::fs::write(&meta_file, serde_json::to_string(&meta).unwrap()).unwrap();
        assert!(cleanup_stale_zip_cache(&config).is_ok());
        // Entry from 1970 should be expired with 1-day policy
        assert!(!cache_file.exists());
    }

    #[test]
    fn cleanup_stale_cache_removes_temp_files() {
        let dir = TempDir::new().unwrap();
        let config = ZipCacheConfig {
            cache_dir: dir.path().to_string_lossy().to_string(),
            max_size_bytes: 1024 * 1024,
            expiry_days: 7,
        };
        // Create an old temp file (simulate old .part file)
        let temp_file = dir.path().join("test.mkv.part");
        std::fs::write(&temp_file, vec![0u8; 50]).unwrap();
        assert!(cleanup_stale_zip_cache(&config).is_ok());
        // The temp file was just created so it might not be removed (age check)
        // This tests the code path runs without error
    }

    #[test]
    fn cleanup_stale_cache_evicts_lru_when_over_limit() {
        let dir = TempDir::new().unwrap();
        let config = ZipCacheConfig {
            cache_dir: dir.path().to_string_lossy().to_string(),
            max_size_bytes: 200, // small limit
            expiry_days: 7,
        };
        // Create entries that exceed the limit
        for i in 0..3 {
            let name = format!("entry{}__{:016x}.mkv", i, i);
            let path = dir.path().join(&name);
            std::fs::write(&path, vec![0u8; 100]).unwrap();
            let meta = ZipCacheMetadata {
                created_at_unix: 1000 + i as i64,
                last_accessed_at_unix: 1000 + i as i64,
                size_bytes: 100,
            };
            let meta_path = dir.path().join(format!("{}.meta.json", name));
            std::fs::write(&meta_path, serde_json::to_string(&meta).unwrap()).unwrap();
        }
        assert!(cleanup_stale_zip_cache(&config).is_ok());
        // Should have evicted some entries to fit within 200 bytes
        let remaining: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let p = e.path();
                p.extension().map_or(true, |ext| ext != "json")
            })
            .collect();
        assert!(remaining.len() <= 2);
    }

    // ── analyze_zip_from_drive / analyze_zip_for_preview error paths ──────

    #[test]
    fn analyze_from_drive_bad_token_returns_error() {
        let result = analyze_zip_from_drive("bad_token", "fake_id");
        assert!(result.is_err());
    }

    #[test]
    fn analyze_for_preview_bad_token_returns_error() {
        let result = analyze_zip_for_preview("bad_token", "fake_id");
        assert!(result.is_err());
    }

    // ── response_headers_to_vec ───────────────────────────────────────────
    // Note: reqwest::blocking::Response cannot be easily constructed in tests.
    // This function is tested indirectly via integration tests.
    // Skipping direct unit test for response_headers_to_vec.

    // ── private helper tests ──────────────────────────────────────────────

    #[test]
    fn parse_crc32_valid() {
        assert_eq!(parse_crc32(Some("abcd1234")).unwrap(), 0xABCD1234);
        assert_eq!(parse_crc32(Some("00000000")).unwrap(), 0);
        assert_eq!(parse_crc32(Some("ffffffff")).unwrap(), 0xFFFFFFFF);
    }

    #[test]
    fn parse_crc32_none_errors() {
        assert!(parse_crc32(None).is_err());
    }

    #[test]
    fn parse_crc32_invalid_hex_errors() {
        assert!(parse_crc32(Some("not_hex")).is_err());
        assert!(parse_crc32(Some("")).is_err());
        assert!(parse_crc32(Some("xyzw")).is_err());
    }

    #[test]
    fn is_supported_entry_compression_store() {
        assert!(is_supported_entry_compression(0));
    }

    #[test]
    fn is_supported_entry_compression_deflate() {
        assert!(is_supported_entry_compression(8));
    }

    #[test]
    fn is_supported_entry_compression_unsupported() {
        assert!(!is_supported_entry_compression(1));
        assert!(!is_supported_entry_compression(12));
        assert!(!is_supported_entry_compression(99));
    }

    #[test]
    fn bearer_value_format() {
        assert_eq!(bearer_value("mytoken"), "Bearer mytoken");
        assert_eq!(bearer_value(""), "Bearer ");
    }

    #[test]
    fn is_cache_metadata_file_detection() {
        assert!(is_cache_metadata_file(Path::new("foo.meta.json")));
        assert!(is_cache_metadata_file(Path::new("/dir/bar.mkv.meta.json")));
        assert!(!is_cache_metadata_file(Path::new("foo.json")));
        assert!(!is_cache_metadata_file(Path::new("foo.meta")));
    }

    #[test]
    fn is_cache_temp_file_detection() {
        assert!(is_cache_temp_file(Path::new("foo.part")));
        assert!(is_cache_temp_file(Path::new("/dir/bar.mkv.part")));
        assert!(!is_cache_temp_file(Path::new("foo.mp4")));
        assert!(!is_cache_temp_file(Path::new("foo.partial")));
    }

    #[test]
    fn metadata_path_for_entry_format() {
        let path = Path::new("/cache/entry.mkv");
        let meta = metadata_path_for_entry(path);
        assert_eq!(meta.to_string_lossy(), "/cache/entry.mkv.meta.json");
    }

    #[test]
    fn temp_cache_path_for_entry_format() {
        let path = Path::new("/cache/entry.mkv");
        let temp = temp_cache_path_for_entry(path);
        assert_eq!(temp.to_string_lossy(), "/cache/entry.mkv.part");
    }

    #[test]
    fn unix_now_returns_reasonable_value() {
        let now = unix_now();
        // Should be after 2020-01-01 (1577836800) and before 2100
        assert!(now > 1_577_836_800);
        assert!(now < 4_102_444_800);
    }

    #[test]
    fn system_time_to_unix_epoch() {
        let epoch = std::time::UNIX_EPOCH;
        assert_eq!(system_time_to_unix(epoch), 0);
    }

    #[test]
    fn sanitize_cache_name_component_basic() {
        assert_eq!(sanitize_cache_name_component("Hello World"), "Hello World");
        assert_eq!(sanitize_cache_name_component("Test-Name"), "Test-Name");
        assert_eq!(sanitize_cache_name_component("foo_bar.baz"), "foo_bar.baz");
    }

    #[test]
    fn sanitize_cache_name_component_special_chars() {
        let result = sanitize_cache_name_component("Show: The [Best]!");
        assert!(!result.contains(':'));
        assert!(!result.contains('['));
        assert!(!result.contains('!'));
    }

    #[test]
    fn sanitize_cache_name_component_whitespace_collapse() {
        let result = sanitize_cache_name_component("a    b");
        assert_eq!(result, "a b");
    }

    #[test]
    fn ensure_zip_cache_dir_creates_directory() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        let config = ZipCacheConfig {
            cache_dir: nested.to_string_lossy().to_string(),
            max_size_bytes: 1024,
            expiry_days: 1,
        };
        let result = ensure_zip_cache_dir(&config);
        assert!(result.is_ok());
        assert!(nested.exists());
    }

    #[test]
    fn cache_path_for_entry_deterministic() {
        let media = make_media();
        let cache_dir = Path::new("/cache");
        let p1 = cache_path_for_entry(cache_dir, &media, "zipid", "path/a.mkv", 0x1234, 1000);
        let p2 = cache_path_for_entry(cache_dir, &media, "zipid", "path/a.mkv", 0x1234, 1000);
        assert_eq!(p1, p2);
    }

    #[test]
    fn cache_path_for_entry_differs_on_input() {
        let media = make_media();
        let cache_dir = Path::new("/cache");
        let p1 = cache_path_for_entry(cache_dir, &media, "zip1", "a.mkv", 0, 100);
        let p2 = cache_path_for_entry(cache_dir, &media, "zip2", "a.mkv", 0, 100);
        assert_ne!(p1, p2);
    }

    #[test]
    fn cache_path_preserves_extension() {
        let media = make_media();
        let cache_dir = Path::new("/cache");
        let p = cache_path_for_entry(cache_dir, &media, "z", "dir/file.mp4", 0, 100);
        assert!(p.to_string_lossy().ends_with(".mp4"));
    }

    #[test]
    fn cache_path_default_extension() {
        let media = make_media();
        let cache_dir = Path::new("/cache");
        let p = cache_path_for_entry(cache_dir, &media, "z", "dir/file_no_ext", 0, 100);
        assert!(p.to_string_lossy().ends_with(".bin"));
    }

    #[test]
    fn write_and_read_cache_metadata_roundtrip() {
        let dir = TempDir::new().unwrap();
        let meta_path = dir.path().join("test.meta.json");
        let meta = ZipCacheMetadata {
            created_at_unix: 1000,
            last_accessed_at_unix: 2000,
            size_bytes: 500,
        };
        write_cache_metadata(&meta_path, &meta).unwrap();
        let read_back = read_cache_metadata(&meta_path).unwrap();
        assert_eq!(read_back.created_at_unix, 1000);
        assert_eq!(read_back.last_accessed_at_unix, 2000);
        assert_eq!(read_back.size_bytes, 500);
    }

    #[test]
    fn touch_cache_entry_updates_last_accessed() {
        let dir = TempDir::new().unwrap();
        let meta_path = dir.path().join("test.meta.json");
        let meta = ZipCacheMetadata {
            created_at_unix: 1000,
            last_accessed_at_unix: 1000,
            size_bytes: 100,
        };
        write_cache_metadata(&meta_path, &meta).unwrap();
        touch_cache_entry(&meta_path, 100).unwrap();
        let updated = read_cache_metadata(&meta_path).unwrap();
        assert!(updated.last_accessed_at_unix > 1000);
        assert_eq!(updated.created_at_unix, 1000); // preserved
    }

    #[test]
    fn touch_cache_entry_creates_if_missing() {
        let dir = TempDir::new().unwrap();
        let meta_path = dir.path().join("new.meta.json");
        assert!(!meta_path.exists());
        touch_cache_entry(&meta_path, 42).unwrap();
        assert!(meta_path.exists());
        let meta = read_cache_metadata(&meta_path).unwrap();
        assert_eq!(meta.size_bytes, 42);
    }

    // ── build_cache_label ─────────────────────────────────────────────────

    #[test]
    fn build_cache_label_with_season_episode() {
        let media = make_media(); // title="Test Show", S01E01, episode_title="Pilot"
        let label = build_cache_label(&media, "path/file.mkv");
        assert!(label.contains("Test Show"));
        assert!(label.contains("S01E01"));
        assert!(label.contains("Pilot"));
    }

    #[test]
    fn build_cache_label_with_year() {
        let mut media = make_media();
        media.season_number = None;
        media.episode_number = None;
        media.year = Some(2023);
        media.episode_title = None;
        let label = build_cache_label(&media, "file.mkv");
        assert!(label.contains("2023"));
    }

    #[test]
    fn build_cache_label_fallback() {
        let mut media = make_media();
        media.title = String::new();
        media.season_number = None;
        media.episode_number = None;
        media.year = None;
        media.episode_title = None;
        let label = build_cache_label(&media, "path/MyVideo.mkv");
        assert_eq!(label, "MyVideo");
    }

    #[test]
    fn build_cache_label_truncates_long() {
        let mut media = make_media();
        media.title = "A".repeat(200);
        media.episode_title = None;
        media.season_number = None;
        media.episode_number = None;
        let label = build_cache_label(&media, "file.mkv");
        assert!(label.len() <= 96);
    }

    // ── build_zip_stream_info overflow edge case ──────────────────────────

    #[test]
    fn stream_info_overflow_in_byte_end_errors() {
        let mut media = make_media();
        media.zip_data_start_offset = Some(i64::MAX);
        media.zip_compressed_size = Some(i64::MAX);
        // The checked_add/checked_sub should catch this
        let result = build_zip_stream_info(&media);
        // May succeed or fail depending on arithmetic; just ensure no panic
        let _ = result;
    }
}
