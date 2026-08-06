use crate::database;
use crate::media_manager;
use crate::zip_manager;
use crate::zip_parser;
use flate2::read::GzDecoder;
use rar_stream::{
    FileMedia as RarFileMedia, ParseOptions as RarParseOptions, RarFilesPackage, ReadInterval,
};
use reqwest::header::{AUTHORIZATION, RANGE};
use reqwest::Client as AsyncClient;
use serde::Serialize;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::OnceLock;
use tar::Archive as TarArchive;
use unrar::Archive as RarArchive;

fn get_rar_runtime() -> &'static tokio::runtime::Runtime {
    static RAR_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RAR_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create RAR runtime")
    })
}

const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
const ARCHIVE_STREAM_BUFFER_BYTES: usize = 1024 * 1024;
const RAR_HEADER_PREFETCH_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    Tar,
    Rar,
}

impl ArchiveFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::Tar => "tar",
            ArchiveFormat::Rar => "rar",
        }
    }
}

#[derive(Debug, Clone)]
pub struct IndexedArchiveEntry {
    pub entry_path: String,
    pub entry_name: String,
    pub parsed: media_manager::ParsedMedia,
    pub local_header_offset: u64,
    pub data_start_offset: u64,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: String,
    pub compression_method: i64,
}

#[derive(Debug, Clone)]
pub struct AnalyzedArchive {
    pub archive: zip_manager::ZipArchiveInfo,
    pub indexed_entries: Vec<IndexedArchiveEntry>,
}

pub fn detect_archive_format(name: &str, mime_type: Option<&str>) -> Option<ArchiveFormat> {
    let lower = name.to_ascii_lowercase();
    let mime = mime_type.unwrap_or_default().to_ascii_lowercase();

    if mime == "application/zip"
        || mime == "application/x-zip-compressed"
        || lower.ends_with(".zip")
    {
        return Some(ArchiveFormat::Zip);
    }

    if mime == "application/x-rar-compressed"
        || mime == "application/vnd.rar"
        || lower.ends_with(".rar")
    {
        return Some(ArchiveFormat::Rar);
    }

    if mime == "application/x-tar"
        || mime == "application/gzip"
        || lower.ends_with(".tar")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
    {
        return Some(ArchiveFormat::Tar);
    }

    None
}

pub fn is_supported_archive_item(name: &str, mime_type: Option<&str>) -> bool {
    detect_archive_format(name, mime_type).is_some()
}

pub fn archive_format_for_media(media: &database::MediaItem) -> ArchiveFormat {
    match media.archive_format.as_deref() {
        Some("tar") => ArchiveFormat::Tar,
        Some("rar") => ArchiveFormat::Rar,
        _ => ArchiveFormat::Zip,
    }
}

pub fn analyze_archive_from_drive(
    access_token: &str,
    file_id: &str,
    filename: &str,
    mime_type: Option<&str>,
    cache_config: &zip_manager::ZipCacheConfig,
) -> Result<AnalyzedArchive, String> {
    match detect_archive_format(filename, mime_type)
        .ok_or_else(|| format!("Unsupported archive format for '{}'", filename))?
    {
        ArchiveFormat::Zip => {
            let analyzed = zip_manager::analyze_zip_from_drive(access_token, file_id)
                .map_err(|e| e.to_string())?;
            Ok(AnalyzedArchive {
                archive: analyzed.archive,
                indexed_entries: analyzed
                    .indexed_entries
                    .into_iter()
                    .map(|entry| IndexedArchiveEntry {
                        entry_path: entry.entry_path,
                        entry_name: entry.entry_name,
                        parsed: entry.parsed,
                        local_header_offset: entry.local_header_offset,
                        data_start_offset: entry.data_start_offset,
                        compressed_size: entry.compressed_size,
                        uncompressed_size: entry.uncompressed_size,
                        crc32: entry.crc32,
                        compression_method: i64::from(entry.compression_method),
                    })
                    .collect(),
            })
        }
        ArchiveFormat::Tar => analyze_tar_from_drive(access_token, file_id, filename, mime_type),
        ArchiveFormat::Rar => analyze_rar_from_drive(access_token, file_id, filename, cache_config),
    }
}

pub fn extract_archive_entry_to_cache(
    access_token: &str,
    media: &database::MediaItem,
    cache_config: &zip_manager::ZipCacheConfig,
) -> Result<String, String> {
    match archive_format_for_media(media) {
        ArchiveFormat::Zip => {
            zip_manager::extract_zip_entry_to_cache(access_token, media, cache_config)
                .map_err(|e| e.to_string())
        }
        ArchiveFormat::Tar => extract_tar_entry_to_cache(access_token, media, cache_config),
        ArchiveFormat::Rar => extract_rar_entry_to_cache(access_token, media, cache_config),
    }
}

pub fn build_archive_stream_info(
    media: &database::MediaItem,
) -> Result<zip_manager::ZipStreamInfo, String> {
    match archive_format_for_media(media) {
        ArchiveFormat::Zip => zip_manager::build_zip_stream_info(media).map_err(|e| e.to_string()),
        ArchiveFormat::Tar | ArchiveFormat::Rar => {
            let archive_file_id = media
                .parent_zip_id
                .clone()
                .ok_or_else(|| "Archive file ID not found".to_string())?;
            let method = media.zip_compression_method.unwrap_or(-1);
            if method != 0 {
                return Err("Archive entry requires extraction before playback".to_string());
            }

            let byte_start = media
                .zip_data_start_offset
                .ok_or_else(|| "Archive entry does not support direct streaming".to_string())?
                as u64;
            let compressed_size = media
                .zip_compressed_size
                .ok_or_else(|| "Archive entry size not available".to_string())?
                as u64;
            let byte_end = byte_start
                .checked_add(compressed_size.saturating_sub(1))
                .ok_or_else(|| "Archive entry range overflow".to_string())?;

            Ok(zip_manager::ZipStreamInfo {
                zip_file_id: archive_file_id,
                byte_start,
                byte_end,
                content_type: zip_manager::content_type_for_name(
                    media
                        .zip_entry_path
                        .as_deref()
                        .or(media.file_path.as_deref())
                        .unwrap_or("video/mp4"),
                ),
            })
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivePlaybackAssessment {
    pub can_play: bool,
    pub mode: String,
    pub message: String,
    pub details: Vec<String>,
}

pub fn assess_archive_playback(media: &database::MediaItem) -> Option<ArchivePlaybackAssessment> {
    if media.parent_zip_id.is_none() {
        return None;
    }

    let assessment = match archive_format_for_media(media) {
        ArchiveFormat::Zip => assess_zip_playback(media),
        ArchiveFormat::Tar => assess_tar_playback(media),
        ArchiveFormat::Rar => assess_rar_playback(media),
    };

    Some(assessment)
}

fn assess_zip_playback(media: &database::MediaItem) -> ArchivePlaybackAssessment {
    match media.zip_compression_method.unwrap_or(0) {
        0 if build_archive_stream_info(media).is_ok() => ArchivePlaybackAssessment {
            can_play: true,
            mode: "direct".to_string(),
            message: "Playable directly from the archive.".to_string(),
            details: vec![
                "ZIP entry uses store mode, so playback can stream raw bytes directly from Drive."
                    .to_string(),
            ],
        },
        8 => ArchivePlaybackAssessment {
            can_play: true,
            mode: "extract".to_string(),
            message: "Playable, but playback requires extraction first.".to_string(),
            details: vec![
                "ZIP entry is compressed with deflate, so raw byte-range streaming is not possible."
                    .to_string(),
                "Startup may be slower because the video must be extracted into cache before playback."
                    .to_string(),
            ],
        },
        method => ArchivePlaybackAssessment {
            can_play: false,
            mode: "unsupported".to_string(),
            message: "This ZIP entry is not supported for playback.".to_string(),
            details: vec![format!(
                "ZIP entry uses unsupported compression or archive metadata (method {}).",
                method
            )],
        },
    }
}

fn assess_tar_playback(media: &database::MediaItem) -> ArchivePlaybackAssessment {
    if build_archive_stream_info(media).is_ok() {
        ArchivePlaybackAssessment {
            can_play: true,
            mode: "direct".to_string(),
            message: "Playable directly from the archive.".to_string(),
            details: vec![
                "This TAR entry has direct raw byte offsets, so playback can stream without extraction."
                    .to_string(),
            ],
        }
    } else {
        ArchivePlaybackAssessment {
            can_play: true,
            mode: "extract".to_string(),
            message: "Playable, but playback requires extraction first.".to_string(),
            details: vec![
                "Compressed TAR variants such as .tar.gz or .tgz are not directly range-streamable."
                    .to_string(),
                "Playback may start slower because the video must be extracted into cache first."
                    .to_string(),
            ],
        }
    }
}

fn assess_rar_playback(media: &database::MediaItem) -> ArchivePlaybackAssessment {
    if build_archive_stream_info(media).is_ok() {
        ArchivePlaybackAssessment {
            can_play: true,
            mode: "direct".to_string(),
            message: "Playable directly from the archive.".to_string(),
            details: vec![
                "This RAR entry is stored and directly range-streamable, so playback can start without extraction."
                    .to_string(),
            ],
        }
    } else {
        ArchivePlaybackAssessment {
            can_play: true,
            mode: "extract".to_string(),
            message: "Playable, but playback requires extraction first.".to_string(),
            details: vec![
                "Most RAR entries cannot be streamed like ZIP store mode if they are compressed, solid, split across volumes, encrypted, or otherwise not directly range-addressable."
                    .to_string(),
                "Playback may start slower because the video must be extracted into cache first."
                    .to_string(),
            ],
        }
    }
}

#[derive(Clone)]
struct DriveRarMedia {
    access_token: String,
    file_id: String,
    name: String,
    length: u64,
    client: AsyncClient,
}

impl DriveRarMedia {
    fn new(access_token: &str, file_id: &str, name: &str, length: u64) -> Result<Self, String> {
        let client = AsyncClient::builder().build().map_err(|e| e.to_string())?;
        Ok(Self {
            access_token: access_token.to_string(),
            file_id: file_id.to_string(),
            name: name.to_string(),
            length,
            client,
        })
    }
}

impl RarFileMedia for DriveRarMedia {
    fn length(&self) -> u64 {
        self.length
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn read_range(
        &self,
        interval: ReadInterval,
    ) -> Pin<Box<dyn std::future::Future<Output = rar_stream::error::Result<Vec<u8>>> + Send + '_>>
    {
        let client = self.client.clone();
        let access_token = self.access_token.clone();
        let file_id = self.file_id.clone();

        Box::pin(async move {
            if interval.start > interval.end {
                return Err(rar_stream::RarError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "invalid range interval",
                )));
            }

            let response = client
                .get(format!(
                    "{}/files/{}?alt=media&supportsAllDrives=true",
                    DRIVE_API_BASE, file_id
                ))
                .header(AUTHORIZATION, format!("Bearer {}", access_token))
                .header(RANGE, format!("bytes={}-{}", interval.start, interval.end))
                .send()
                .await
                .map_err(http_error_to_rar_io)?
                .error_for_status()
                .map_err(http_error_to_rar_io)?;

            let bytes = response.bytes().await.map_err(http_error_to_rar_io)?;
            Ok(bytes.to_vec())
        })
    }
}

fn http_error_to_rar_io(error: reqwest::Error) -> rar_stream::RarError {
    rar_stream::RarError::Io(std::io::Error::other(error.to_string()))
}

fn fetch_drive_media_response(
    access_token: &str,
    file_id: &str,
) -> Result<reqwest::blocking::Response, String> {
    let client = crate::http_client::shared_client();
    client
        .get(format!(
            "{}/files/{}?alt=media&supportsAllDrives=true",
            DRIVE_API_BASE, file_id
        ))
        .header(AUTHORIZATION, format!("Bearer {}", access_token))
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|e| e.to_string())
}

fn fetch_drive_file_size(access_token: &str, file_id: &str) -> Result<u64, String> {
    let client = crate::http_client::shared_client();
    let payload = client
        .get(format!(
            "{}/files/{}?fields=size&supportsAllDrives=true",
            DRIVE_API_BASE, file_id
        ))
        .header(AUTHORIZATION, format!("Bearer {}", access_token))
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .map_err(|e| e.to_string())?;

    payload["size"]
        .as_str()
        .ok_or_else(|| "Drive file size missing from metadata response".to_string())?
        .parse::<u64>()
        .map_err(|e| e.to_string())
}

fn analyze_tar_from_drive(
    access_token: &str,
    file_id: &str,
    filename: &str,
    mime_type: Option<&str>,
) -> Result<AnalyzedArchive, String> {
    let response = fetch_drive_media_response(access_token, file_id)?;
    let file_size_bytes = response.content_length().unwrap_or(0);
    let gzip_hint = mime_type
        .map(|value| value.eq_ignore_ascii_case("application/gzip"))
        .unwrap_or(false)
        || is_gzip_tar(filename);
    let buffered = BufReader::with_capacity(ARCHIVE_STREAM_BUFFER_BYTES, response);
    let reader: Box<dyn Read> = if gzip_hint {
        Box::new(GzDecoder::new(buffered))
    } else {
        Box::new(buffered)
    };

    analyze_tar_reader(file_id, filename, file_size_bytes, reader, !gzip_hint)
}

fn analyze_tar_reader(
    file_id: &str,
    filename: &str,
    file_size_bytes: u64,
    reader: Box<dyn Read>,
    supports_passthrough: bool,
) -> Result<AnalyzedArchive, String> {
    let mut archive = TarArchive::new(reader);
    let mut indexed_entries = Vec::new();
    let mut total_entries = 0usize;

    for entry_result in archive.entries().map_err(|e| e.to_string())? {
        let entry = entry_result.map_err(|e| e.to_string())?;
        total_entries += 1;

        if entry.header().entry_type().is_dir() {
            continue;
        }

        let raw_path = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();
        let entry_path =
            zip_parser::sanitize_zip_entry_path(&raw_path).map_err(|e| e.to_string())?;
        if !zip_manager::is_video_path(&entry_path) {
            continue;
        }

        let entry_name = entry_path
            .rsplit('/')
            .next()
            .unwrap_or(&entry_path)
            .to_string();
        let parsed = media_manager::parse_cloud_filename(&entry_name);
        if parsed.season.is_none() || parsed.episode.is_none() {
            continue;
        }

        indexed_entries.push(IndexedArchiveEntry {
            entry_path,
            entry_name,
            parsed,
            local_header_offset: if supports_passthrough {
                entry.raw_header_position()
            } else {
                0
            },
            data_start_offset: if supports_passthrough {
                entry.raw_file_position()
            } else {
                0
            },
            compressed_size: if supports_passthrough {
                entry.size()
            } else {
                0
            },
            uncompressed_size: entry.size(),
            crc32: "00000000".to_string(),
            compression_method: if supports_passthrough { 0 } else { -2 },
        });
    }

    Ok(AnalyzedArchive {
        archive: zip_manager::ZipArchiveInfo {
            zip_file_id: file_id.to_string(),
            filename: filename.to_string(),
            archive_format: ArchiveFormat::Tar.as_str().to_string(),
            file_size_bytes,
            compression_type: zip_parser::ZipCompressionType::Other,
            central_dir_offset: 0,
            central_dir_size: 0,
            total_entries,
            video_entries: indexed_entries.len(),
        },
        indexed_entries,
    })
}

fn analyze_rar_from_drive(
    access_token: &str,
    file_id: &str,
    filename: &str,
    cache_config: &zip_manager::ZipCacheConfig,
) -> Result<AnalyzedArchive, String> {
    match analyze_rar_from_drive_fast(access_token, file_id, filename) {
        Ok(analyzed) => Ok(analyzed),
        Err(error) => {
            eprintln!(
                "[ARCHIVE] Fast RAR analysis failed for '{}'; falling back to local source: {}",
                filename, error
            );

            let source_path =
                download_archive_to_temp(access_token, file_id, filename, cache_config, "analyze")?;
            let result = analyze_rar_from_path(file_id, filename, &source_path);
            let _ = fs::remove_file(source_path);
            result
        }
    }
}

fn analyze_rar_from_drive_fast(
    access_token: &str,
    file_id: &str,
    filename: &str,
) -> Result<AnalyzedArchive, String> {
    let file_size_bytes = fetch_drive_file_size(access_token, file_id)?;
    let media: Arc<dyn RarFileMedia> = Arc::new(DriveRarMedia::new(
        access_token,
        file_id,
        filename,
        file_size_bytes,
    )?);
    let package = RarFilesPackage::new(vec![media]);
    let runtime = get_rar_runtime();
    let entries = runtime
        .block_on(async {
            package
                .parse(RarParseOptions {
                    header_prefetch_size: Some(RAR_HEADER_PREFETCH_BYTES),
                    ..Default::default()
                })
                .await
        })
        .map_err(|e| e.to_string())?;

    let total_entries = entries.len();
    let mut indexed_entries = Vec::new();

    for entry in entries {
        let entry_path =
            zip_parser::sanitize_zip_entry_path(&entry.name).map_err(|e| e.to_string())?;
        if !zip_manager::is_video_path(&entry_path) {
            continue;
        }

        let entry_name = entry_path
            .rsplit('/')
            .next()
            .unwrap_or(&entry_path)
            .to_string();
        let parsed = media_manager::parse_cloud_filename(&entry_name);
        if parsed.season.is_none() || parsed.episode.is_none() {
            continue;
        }

        let supports_passthrough =
            !entry.is_compressed() && !entry.is_solid() && entry.chunk_count() == 1;
        let (data_start_offset, compressed_size, compression_method) = if supports_passthrough {
            let chunk = entry
                .get_chunk(0)
                .ok_or_else(|| "RAR entry missing raw chunk metadata".to_string())?;
            (chunk.start_offset, chunk.length(), 0)
        } else {
            (0, 0, -1)
        };

        indexed_entries.push(IndexedArchiveEntry {
            entry_path,
            entry_name,
            parsed,
            local_header_offset: 0,
            data_start_offset,
            compressed_size,
            uncompressed_size: entry.length,
            crc32: "00000000".to_string(),
            compression_method,
        });
    }

    Ok(AnalyzedArchive {
        archive: zip_manager::ZipArchiveInfo {
            zip_file_id: file_id.to_string(),
            filename: filename.to_string(),
            archive_format: ArchiveFormat::Rar.as_str().to_string(),
            file_size_bytes,
            compression_type: zip_parser::ZipCompressionType::Other,
            central_dir_offset: 0,
            central_dir_size: 0,
            total_entries,
            video_entries: indexed_entries.len(),
        },
        indexed_entries,
    })
}

fn analyze_rar_from_path(
    file_id: &str,
    filename: &str,
    source_path: &Path,
) -> Result<AnalyzedArchive, String> {
    let mut indexed_entries = Vec::new();
    let mut total_entries = 0usize;

    let listing = RarArchive::new(source_path)
        .open_for_listing()
        .map_err(|e| e.to_string())?;

    for header_result in listing {
        let header = header_result.map_err(|e| e.to_string())?;
        total_entries += 1;

        if !header.is_file() || header.is_encrypted() || header.is_split() {
            continue;
        }

        let raw_path = header.filename.to_string_lossy().to_string();
        let entry_path =
            zip_parser::sanitize_zip_entry_path(&raw_path).map_err(|e| e.to_string())?;
        if !zip_manager::is_video_path(&entry_path) {
            continue;
        }

        let entry_name = entry_path
            .rsplit('/')
            .next()
            .unwrap_or(&entry_path)
            .to_string();
        let parsed = media_manager::parse_cloud_filename(&entry_name);
        if parsed.season.is_none() || parsed.episode.is_none() {
            continue;
        }

        indexed_entries.push(IndexedArchiveEntry {
            entry_path,
            entry_name,
            parsed,
            local_header_offset: 0,
            data_start_offset: 0,
            compressed_size: 0,
            uncompressed_size: header.unpacked_size,
            crc32: format!("{:08x}", header.file_crc),
            compression_method: -1,
        });
    }

    Ok(AnalyzedArchive {
        archive: zip_manager::ZipArchiveInfo {
            zip_file_id: file_id.to_string(),
            filename: filename.to_string(),
            archive_format: ArchiveFormat::Rar.as_str().to_string(),
            file_size_bytes: fs::metadata(source_path).map_err(|e| e.to_string())?.len(),
            compression_type: zip_parser::ZipCompressionType::Other,
            central_dir_offset: 0,
            central_dir_size: 0,
            total_entries,
            video_entries: indexed_entries.len(),
        },
        indexed_entries,
    })
}

fn extract_tar_entry_to_cache(
    access_token: &str,
    media: &database::MediaItem,
    cache_config: &zip_manager::ZipCacheConfig,
) -> Result<String, String> {
    let archive_id = media
        .parent_zip_id
        .as_deref()
        .ok_or_else(|| "Archive file ID not found".to_string())?;
    let entry_path = media
        .zip_entry_path
        .as_deref()
        .or(media.file_path.as_deref())
        .ok_or_else(|| "Archive entry path not found".to_string())?;
    let output_path = archive_cache_path(media, cache_config)?;
    if output_path.exists() {
        return Ok(output_path.to_string_lossy().to_string());
    }

    let source_path = download_archive_to_temp(
        access_token,
        archive_id,
        &archive_filename_for_media(media),
        cache_config,
        "extract",
    )?;

    let file = File::open(&source_path).map_err(|e| e.to_string())?;
    let reader: Box<dyn Read> = if is_gzip_tar_path(&source_path)? {
        Box::new(GzDecoder::new(file))
    } else {
        Box::new(file)
    };

    let mut archive = TarArchive::new(reader);
    let mut found = false;
    for entry_result in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry_result.map_err(|e| e.to_string())?;
        let raw_path = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();
        let sanitized =
            zip_parser::sanitize_zip_entry_path(&raw_path).map_err(|e| e.to_string())?;
        if sanitized != entry_path {
            continue;
        }

        let mut writer = File::create(&output_path).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut writer).map_err(|e| e.to_string())?;
        found = true;
        break;
    }

    let _ = fs::remove_file(source_path);
    if !found {
        return Err(format!("Archive entry '{}' not found", entry_path));
    }

    Ok(output_path.to_string_lossy().to_string())
}

fn extract_rar_entry_to_cache(
    access_token: &str,
    media: &database::MediaItem,
    cache_config: &zip_manager::ZipCacheConfig,
) -> Result<String, String> {
    let archive_id = media
        .parent_zip_id
        .as_deref()
        .ok_or_else(|| "Archive file ID not found".to_string())?;
    let entry_path = media
        .zip_entry_path
        .as_deref()
        .or(media.file_path.as_deref())
        .ok_or_else(|| "Archive entry path not found".to_string())?;
    let output_path = archive_cache_path(media, cache_config)?;
    if output_path.exists() {
        return Ok(output_path.to_string_lossy().to_string());
    }

    let source_path = download_archive_to_temp(
        access_token,
        archive_id,
        &archive_filename_for_media(media),
        cache_config,
        "extract",
    )?;

    let mut archive = RarArchive::new(&source_path)
        .open_for_processing()
        .map_err(|e| e.to_string())?;

    let mut found = false;
    while let Some(next) = archive.read_header().map_err(|e| e.to_string())? {
        let current_path = next.entry().filename.to_string_lossy().to_string();
        let sanitized =
            zip_parser::sanitize_zip_entry_path(&current_path).map_err(|e| e.to_string())?;
        if sanitized == entry_path {
            let parent = output_path
                .parent()
                .ok_or_else(|| "Invalid archive cache path".to_string())?;
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            let _ = next.extract_to(&output_path).map_err(|e| e.to_string())?;
            found = true;
            break;
        }

        archive = next.skip().map_err(|e| e.to_string())?;
    }

    let _ = fs::remove_file(source_path);
    if !found {
        return Err(format!("Archive entry '{}' not found", entry_path));
    }

    Ok(output_path.to_string_lossy().to_string())
}

fn download_archive_to_temp(
    access_token: &str,
    file_id: &str,
    filename: &str,
    cache_config: &zip_manager::ZipCacheConfig,
    suffix: &str,
) -> Result<PathBuf, String> {
    let cache_dir = PathBuf::from(&cache_config.cache_dir).join("archive_sources");
    fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
    let temp_path = cache_dir.join(format!(
        "{}-{}-{}",
        safe_filename(file_id),
        suffix,
        safe_filename(filename)
    ));

    let mut response = fetch_drive_media_response(access_token, file_id)?;

    let mut writer = File::create(&temp_path).map_err(|e| e.to_string())?;
    std::io::copy(&mut response, &mut writer).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;
    Ok(temp_path)
}

fn archive_cache_path(
    media: &database::MediaItem,
    cache_config: &zip_manager::ZipCacheConfig,
) -> Result<PathBuf, String> {
    let archive_id = media
        .parent_zip_id
        .as_deref()
        .ok_or_else(|| "Archive file ID not found".to_string())?;
    let entry_path = media
        .zip_entry_path
        .as_deref()
        .or(media.file_path.as_deref())
        .ok_or_else(|| "Archive entry path not found".to_string())?;

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    archive_id.hash(&mut hasher);
    entry_path.hash(&mut hasher);
    let hash = hasher.finish();
    let ext = Path::new(entry_path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin");
    let label = safe_filename(&format!("{}-{}", media.title, entry_path));
    let cache_dir = PathBuf::from(&cache_config.cache_dir);
    fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
    Ok(cache_dir.join(format!("archive-{}-{:016x}.{}", label, hash, ext)))
}

fn archive_filename_for_media(media: &database::MediaItem) -> String {
    let archive_id = media.parent_zip_id.as_deref().unwrap_or("archive");
    let format = archive_format_for_media(media);
    match media.file_path.as_deref() {
        Some(path) if path.contains("://") => format!("{}.{}", archive_id, format.as_str()),
        Some(path) => path.to_string(),
        None => format!("{}.{}", archive_id, format.as_str()),
    }
}

fn is_gzip_tar(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    lower.ends_with(".tar.gz") || lower.ends_with(".tgz")
}

fn is_gzip_tar_path(path: &Path) -> Result<bool, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut magic = [0u8; 2];
    let bytes_read = file.read(&mut magic).map_err(|e| e.to_string())?;
    Ok(bytes_read == 2 && magic == [0x1f, 0x8b])
}

fn safe_filename(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_media_item() -> database::MediaItem {
        database::MediaItem {
            id: 1,
            title: "Test Title".into(),
            year: None,
            overview: None,
            cast_names: None,
            director: None,
            poster_path: None,
            file_path: None,
            media_type: "video".into(),
            duration_seconds: None,
            resume_position_seconds: None,
            last_watched: None,
            season_number: None,
            episode_number: None,
            parent_id: None,
            progress_percent: None,
            tmdb_id: None,
            imdb_id: None,
            episode_title: None,
            still_path: None,
            is_cloud: None,
            cloud_file_id: None,
            cloud_folder_id: None,
            archive_format: None,
            parent_zip_id: None,
            zip_entry_path: None,
            zip_local_header_offset: None,
            zip_data_start_offset: None,
            zip_compressed_size: None,
            zip_uncompressed_size: None,
            zip_crc32: None,
            zip_compression_method: None,
            file_size_bytes: None,
            ddl_source_id: None,
            archive_playback_can_play: None,
            archive_playback_mode: None,
            archive_playback_message: None,
            archive_playback_details: None,
        }
    }

    // ---- ArchiveFormat ----

    #[test]
    fn archive_format_as_str() {
        assert_eq!(ArchiveFormat::Zip.as_str(), "zip");
        assert_eq!(ArchiveFormat::Tar.as_str(), "tar");
        assert_eq!(ArchiveFormat::Rar.as_str(), "rar");
    }

    #[test]
    fn archive_format_debug_clone_eq() {
        let zip = ArchiveFormat::Zip;
        assert_eq!(zip, ArchiveFormat::Zip);
        assert_ne!(zip, ArchiveFormat::Rar);
        let cloned = zip;
        assert_eq!(cloned, zip);
        let _ = format!("{:?}", zip);
    }

    // ---- detect_archive_format ----

    #[test]
    fn detect_zip_by_extension() {
        assert_eq!(
            detect_archive_format("movie.zip", None),
            Some(ArchiveFormat::Zip)
        );
    }

    #[test]
    fn detect_zip_by_mime_application_zip() {
        assert_eq!(
            detect_archive_format("movie", Some("application/zip")),
            Some(ArchiveFormat::Zip)
        );
    }

    #[test]
    fn detect_zip_by_mime_x_zip_compressed() {
        assert_eq!(
            detect_archive_format("movie", Some("application/x-zip-compressed")),
            Some(ArchiveFormat::Zip)
        );
    }

    #[test]
    fn detect_zip_case_insensitive_extension() {
        assert_eq!(
            detect_archive_format("MOVIE.ZIP", None),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(
            detect_archive_format("movie.Zip", None),
            Some(ArchiveFormat::Zip)
        );
    }

    #[test]
    fn detect_zip_case_insensitive_mime() {
        assert_eq!(
            detect_archive_format("movie", Some("APPLICATION/ZIP")),
            Some(ArchiveFormat::Zip)
        );
    }

    #[test]
    fn detect_rar_by_extension() {
        assert_eq!(
            detect_archive_format("movie.rar", None),
            Some(ArchiveFormat::Rar)
        );
    }

    #[test]
    fn detect_rar_by_mime_x_rar_compressed() {
        assert_eq!(
            detect_archive_format("movie", Some("application/x-rar-compressed")),
            Some(ArchiveFormat::Rar)
        );
    }

    #[test]
    fn detect_rar_by_mime_vnd_rar() {
        assert_eq!(
            detect_archive_format("movie", Some("application/vnd.rar")),
            Some(ArchiveFormat::Rar)
        );
    }

    #[test]
    fn detect_rar_case_insensitive() {
        assert_eq!(
            detect_archive_format("MOVIE.RAR", None),
            Some(ArchiveFormat::Rar)
        );
    }

    #[test]
    fn detect_tar_by_extension() {
        assert_eq!(
            detect_archive_format("archive.tar", None),
            Some(ArchiveFormat::Tar)
        );
    }

    #[test]
    fn detect_tar_by_tar_gz_extension() {
        assert_eq!(
            detect_archive_format("archive.tar.gz", None),
            Some(ArchiveFormat::Tar)
        );
    }

    #[test]
    fn detect_tar_by_tgz_extension() {
        assert_eq!(
            detect_archive_format("archive.tgz", None),
            Some(ArchiveFormat::Tar)
        );
    }

    #[test]
    fn detect_tar_by_mime_application_tar() {
        assert_eq!(
            detect_archive_format("archive", Some("application/x-tar")),
            Some(ArchiveFormat::Tar)
        );
    }

    #[test]
    fn detect_tar_by_mime_application_gzip() {
        assert_eq!(
            detect_archive_format("archive", Some("application/gzip")),
            Some(ArchiveFormat::Tar)
        );
    }

    #[test]
    fn detect_tar_case_insensitive() {
        assert_eq!(
            detect_archive_format("ARCHIVE.TAR.GZ", None),
            Some(ArchiveFormat::Tar)
        );
        assert_eq!(
            detect_archive_format("ARCHIVE.TGZ", None),
            Some(ArchiveFormat::Tar)
        );
    }

    #[test]
    fn detect_unknown_extension() {
        assert_eq!(detect_archive_format("file.txt", None), None);
        assert_eq!(detect_archive_format("file.mp4", None), None);
        assert_eq!(detect_archive_format("file.mkv", None), None);
    }

    #[test]
    fn detect_unknown_mime() {
        assert_eq!(detect_archive_format("file", Some("text/plain")), None);
    }

    #[test]
    fn detect_empty_name_and_mime() {
        assert_eq!(detect_archive_format("", None), None);
        assert_eq!(detect_archive_format("", Some("")), None);
    }

    #[test]
    fn detect_mime_takes_precedence_no_extension() {
        assert_eq!(
            detect_archive_format("noext", Some("application/zip")),
            Some(ArchiveFormat::Zip)
        );
    }

    #[test]
    fn detect_zip_not_confused_by_nonarchive_zip_in_name() {
        // "zipping.mp4" ends with ".mp4", not ".zip"
        assert_eq!(detect_archive_format("zipping.mp4", None), None);
    }

    // ---- is_supported_archive_item ----

    #[test]
    fn is_supported_archive_item_zip() {
        assert!(is_supported_archive_item("file.zip", None));
    }

    #[test]
    fn is_supported_archive_item_rar() {
        assert!(is_supported_archive_item("file.rar", None));
    }

    #[test]
    fn is_supported_archive_item_tar() {
        assert!(is_supported_archive_item("file.tar", None));
    }

    #[test]
    fn is_supported_archive_item_tar_gz() {
        assert!(is_supported_archive_item("file.tar.gz", None));
    }

    #[test]
    fn is_supported_archive_item_tgz() {
        assert!(is_supported_archive_item("file.tgz", None));
    }

    #[test]
    fn is_supported_archive_item_with_mime() {
        assert!(is_supported_archive_item("file", Some("application/zip")));
    }

    #[test]
    fn is_supported_archive_item_unsupported() {
        assert!(!is_supported_archive_item("file.mp4", None));
        assert!(!is_supported_archive_item("file.txt", None));
        assert!(!is_supported_archive_item("", None));
    }

    // ---- archive_format_for_media ----

    #[test]
    fn archive_format_for_media_zip_default() {
        let media = default_media_item();
        // archive_format is None, defaults to Zip
        assert_eq!(archive_format_for_media(&media), ArchiveFormat::Zip);
    }

    #[test]
    fn archive_format_for_media_explicit_zip() {
        let mut media = default_media_item();
        media.archive_format = Some("zip".into());
        assert_eq!(archive_format_for_media(&media), ArchiveFormat::Zip);
    }

    #[test]
    fn archive_format_for_media_tar() {
        let mut media = default_media_item();
        media.archive_format = Some("tar".into());
        assert_eq!(archive_format_for_media(&media), ArchiveFormat::Tar);
    }

    #[test]
    fn archive_format_for_media_rar() {
        let mut media = default_media_item();
        media.archive_format = Some("rar".into());
        assert_eq!(archive_format_for_media(&media), ArchiveFormat::Rar);
    }

    #[test]
    fn archive_format_for_media_unknown_defaults_zip() {
        let mut media = default_media_item();
        media.archive_format = Some("7z".into());
        assert_eq!(archive_format_for_media(&media), ArchiveFormat::Zip);
    }

    #[test]
    fn archive_format_for_media_empty_string_defaults_zip() {
        let mut media = default_media_item();
        media.archive_format = Some("".into());
        assert_eq!(archive_format_for_media(&media), ArchiveFormat::Zip);
    }

    // ---- IndexedArchiveEntry ----

    #[test]
    fn indexed_archive_entry_clone_debug() {
        let parsed = media_manager::ParsedMedia {
            title: "Ep".into(),
            year: None,
            media_type: media_manager::MediaParseType::TvEpisode,
            season: Some(1),
            episode: Some(2),
            episode_end: None,
        };
        let entry = IndexedArchiveEntry {
            entry_path: "folder/ep.mkv".into(),
            entry_name: "ep.mkv".into(),
            parsed,
            local_header_offset: 100,
            data_start_offset: 200,
            compressed_size: 500,
            uncompressed_size: 1000,
            crc32: "deadbeef".into(),
            compression_method: 0,
        };
        let cloned = entry.clone();
        assert_eq!(cloned.entry_path, "folder/ep.mkv");
        assert_eq!(cloned.entry_name, "ep.mkv");
        assert_eq!(cloned.local_header_offset, 100);
        assert_eq!(cloned.data_start_offset, 200);
        assert_eq!(cloned.compressed_size, 500);
        assert_eq!(cloned.uncompressed_size, 1000);
        assert_eq!(cloned.crc32, "deadbeef");
        assert_eq!(cloned.compression_method, 0);
        let _ = format!("{:?}", entry);
    }

    // ---- AnalyzedArchive ----

    #[test]
    fn analyzed_archive_clone_debug() {
        let analyzed = AnalyzedArchive {
            archive: zip_manager::ZipArchiveInfo {
                zip_file_id: "abc123".into(),
                filename: "test.zip".into(),
                archive_format: "zip".into(),
                file_size_bytes: 9999,
                compression_type: zip_parser::ZipCompressionType::Other,
                central_dir_offset: 0,
                central_dir_size: 0,
                total_entries: 0,
                video_entries: 0,
            },
            indexed_entries: vec![],
        };
        let cloned = analyzed.clone();
        assert_eq!(cloned.archive.zip_file_id, "abc123");
        assert_eq!(cloned.archive.filename, "test.zip");
        assert_eq!(cloned.archive.file_size_bytes, 9999);
        assert!(cloned.indexed_entries.is_empty());
        let _ = format!("{:?}", analyzed);
    }

    // ---- ArchivePlaybackAssessment ----

    #[test]
    fn archive_playback_assessment_clone_debug() {
        let assessment = ArchivePlaybackAssessment {
            can_play: true,
            mode: "direct".into(),
            message: "OK".into(),
            details: vec!["detail".into()],
        };
        let cloned = assessment.clone();
        assert!(cloned.can_play);
        assert_eq!(cloned.mode, "direct");
        assert_eq!(cloned.message, "OK");
        assert_eq!(cloned.details.len(), 1);
        let _ = format!("{:?}", assessment);
    }

    #[test]
    fn archive_playback_assessment_serializes_camel_case() {
        let assessment = ArchivePlaybackAssessment {
            can_play: false,
            mode: "unsupported".into(),
            message: "No".into(),
            details: vec![],
        };
        let json = serde_json::to_value(&assessment).unwrap();
        assert_eq!(json["canPlay"], false);
        assert_eq!(json["mode"], "unsupported");
        assert_eq!(json["message"], "No");
        assert!(json["details"].as_array().unwrap().is_empty());
    }

    // ---- assess_archive_playback ----

    #[test]
    fn assess_archive_playback_returns_none_without_parent_zip() {
        let media = default_media_item();
        assert!(assess_archive_playback(&media).is_none());
    }

    #[test]
    fn assess_zip_playback_store_method_0_can_stream() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("file123".into());
        media.archive_format = Some("zip".into());
        media.zip_compression_method = Some(0);
        media.zip_data_start_offset = Some(0);
        media.zip_compressed_size = Some(1000);
        media.zip_entry_path = Some("video.mp4".into());

        let result = assess_archive_playback(&media).unwrap();
        assert!(result.can_play);
        assert_eq!(result.mode, "direct");
    }

    #[test]
    fn assess_zip_playback_deflate_method_8_needs_extraction() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("file123".into());
        media.archive_format = Some("zip".into());
        media.zip_compression_method = Some(8);

        let result = assess_archive_playback(&media).unwrap();
        assert!(result.can_play);
        assert_eq!(result.mode, "extract");
    }

    #[test]
    fn assess_zip_playback_unsupported_method() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("file123".into());
        media.archive_format = Some("zip".into());
        media.zip_compression_method = Some(99);

        let result = assess_archive_playback(&media).unwrap();
        assert!(!result.can_play);
        assert_eq!(result.mode, "unsupported");
    }

    #[test]
    fn assess_zip_playback_default_method_0_missing_offsets_falls_back() {
        // method defaults to 0 but no offsets -> build_archive_stream_info fails
        let mut media = default_media_item();
        media.parent_zip_id = Some("file123".into());
        media.archive_format = Some("zip".into());
        media.zip_compression_method = None; // defaults to 0
                                             // no offsets set, so build_archive_stream_info returns Err -> method 0 fallback
        let result = assess_archive_playback(&media).unwrap();
        // method is 0 but stream info fails, so falls through to "unsupported"
        assert!(!result.can_play);
        assert_eq!(result.mode, "unsupported");
    }

    // ---- assess_tar_playback ----

    #[test]
    fn assess_tar_playback_with_valid_offsets_direct() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("file123".into());
        media.archive_format = Some("tar".into());
        media.zip_compression_method = Some(0);
        media.zip_data_start_offset = Some(0);
        media.zip_compressed_size = Some(500);
        media.zip_entry_path = Some("video.mkv".into());

        let result = assess_archive_playback(&media).unwrap();
        assert!(result.can_play);
        assert_eq!(result.mode, "direct");
    }

    #[test]
    fn assess_tar_playback_without_offsets_extract() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("file123".into());
        media.archive_format = Some("tar".into());
        media.zip_compression_method = Some(-2);

        let result = assess_archive_playback(&media).unwrap();
        assert!(result.can_play);
        assert_eq!(result.mode, "extract");
    }

    // ---- assess_rar_playback ----

    #[test]
    fn assess_rar_playback_with_valid_offsets_direct() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("file123".into());
        media.archive_format = Some("rar".into());
        media.zip_compression_method = Some(0);
        media.zip_data_start_offset = Some(0);
        media.zip_compressed_size = Some(500);
        media.zip_entry_path = Some("video.mkv".into());

        let result = assess_archive_playback(&media).unwrap();
        assert!(result.can_play);
        assert_eq!(result.mode, "direct");
    }

    #[test]
    fn assess_rar_playback_without_offsets_extract() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("file123".into());
        media.archive_format = Some("rar".into());
        media.zip_compression_method = Some(-1);

        let result = assess_archive_playback(&media).unwrap();
        assert!(result.can_play);
        assert_eq!(result.mode, "extract");
    }

    // ---- archive_filename_for_media ----

    #[test]
    fn archive_filename_for_media_remote_path_uses_format_extension() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("abc".into());
        media.archive_format = Some("rar".into());
        media.file_path = Some("https://drive.google.com/file".into());
        assert_eq!(archive_filename_for_media(&media), "abc.rar");
    }

    #[test]
    fn archive_filename_for_media_local_path_returns_path() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("abc".into());
        media.file_path = Some("/local/path/archive.zip".into());
        assert_eq!(
            archive_filename_for_media(&media),
            "/local/path/archive.zip"
        );
    }

    #[test]
    fn archive_filename_for_media_no_path_defaults() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("xyz".into());
        media.archive_format = Some("tar".into());
        assert_eq!(archive_filename_for_media(&media), "xyz.tar");
    }

    #[test]
    fn archive_filename_for_media_no_zip_id_fallback() {
        let mut media = default_media_item();
        media.archive_format = Some("zip".into());
        assert_eq!(archive_filename_for_media(&media), "archive.zip");
    }

    // ---- is_gzip_tar ----

    #[test]
    fn is_gzip_tar_tar_gz() {
        assert!(is_gzip_tar("file.tar.gz"));
    }

    #[test]
    fn is_gzip_tar_tgz() {
        assert!(is_gzip_tar("file.tgz"));
    }

    #[test]
    fn is_gzip_tar_case_insensitive() {
        assert!(is_gzip_tar("FILE.TAR.GZ"));
        assert!(is_gzip_tar("FILE.TGZ"));
    }

    #[test]
    fn is_gzip_tar_plain_tar() {
        assert!(!is_gzip_tar("file.tar"));
    }

    #[test]
    fn is_gzip_tar_zip() {
        assert!(!is_gzip_tar("file.zip"));
    }

    #[test]
    fn is_gzip_tar_empty() {
        assert!(!is_gzip_tar(""));
    }

    // ---- safe_filename ----

    #[test]
    fn safe_filename_alphanumeric_passthrough() {
        assert_eq!(safe_filename("abc123"), "abc123");
    }

    #[test]
    fn safe_filename_allowed_chars() {
        assert_eq!(safe_filename("file-name_v2.ext"), "file-name_v2.ext");
    }

    #[test]
    fn safe_filename_replaces_special_chars() {
        assert_eq!(safe_filename("a b/c\\d:e"), "a_b_c_d_e");
    }

    #[test]
    fn safe_filename_empty() {
        assert_eq!(safe_filename(""), "");
    }

    #[test]
    fn safe_filename_unicode_replaced() {
        assert_eq!(safe_filename("日本語"), "___");
    }

    // ---- archive_cache_path ----

    #[test]
    fn archive_cache_path_basic() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("file1".into());
        media.zip_entry_path = Some("folder/video.mp4".into());
        media.title = "My Show".into();
        let config = zip_manager::ZipCacheConfig {
            cache_dir: std::env::temp_dir()
                .join("archive_test_cache")
                .to_string_lossy()
                .to_string(),
            max_size_bytes: 0,
            expiry_days: 30,
        };
        let result = archive_cache_path(&media, &config);
        assert!(result.is_ok());
        let path = result.unwrap();
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("archive-"));
        assert!(name.ends_with(".mp4"));
    }

    #[test]
    fn archive_cache_path_no_parent_zip_id_errors() {
        let media = default_media_item();
        let config = zip_manager::ZipCacheConfig {
            cache_dir: std::env::temp_dir()
                .join("archive_test_cache")
                .to_string_lossy()
                .to_string(),
            max_size_bytes: 0,
            expiry_days: 30,
        };
        assert!(archive_cache_path(&media, &config).is_err());
    }

    #[test]
    fn archive_cache_path_no_entry_path_errors() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("file1".into());
        let config = zip_manager::ZipCacheConfig {
            cache_dir: std::env::temp_dir()
                .join("archive_test_cache")
                .to_string_lossy()
                .to_string(),
            max_size_bytes: 0,
            expiry_days: 30,
        };
        assert!(archive_cache_path(&media, &config).is_err());
    }

    #[test]
    fn archive_cache_path_uses_file_path_as_fallback() {
        let mut media = default_media_item();
        media.parent_zip_id = Some("file1".into());
        media.zip_entry_path = None;
        media.file_path = Some("video.mkv".into());
        media.title = "Show".into();
        let config = zip_manager::ZipCacheConfig {
            cache_dir: std::env::temp_dir()
                .join("archive_test_cache")
                .to_string_lossy()
                .to_string(),
            max_size_bytes: 0,
            expiry_days: 30,
        };
        let result = archive_cache_path(&media, &config);
        assert!(result.is_ok());
        let name = result
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(name.ends_with(".mkv"));
    }

    // ---- archive_format_for_media edge cases ----

    #[test]
    fn archive_format_for_media_uppercase_tar() {
        let mut media = default_media_item();
        media.archive_format = Some("TAR".into());
        // "TAR" != "tar", so it falls to default Zip
        assert_eq!(archive_format_for_media(&media), ArchiveFormat::Zip);
    }

    #[test]
    fn archive_format_for_media_uppercase_rar() {
        let mut media = default_media_item();
        media.archive_format = Some("RAR".into());
        assert_eq!(archive_format_for_media(&media), ArchiveFormat::Zip);
    }

    // ---- build_archive_stream_info ----

    #[test]
    fn build_archive_stream_info_zip_calls_zip_manager() {
        let mut media = default_media_item();
        media.archive_format = Some("zip".into());
        // No zip_file_id set -> zip_manager::build_zip_stream_info should return Err
        let result = build_archive_stream_info(&media);
        assert!(result.is_err());
    }

    #[test]
    fn build_archive_stream_info_tar_no_parent_zip_id() {
        let mut media = default_media_item();
        media.archive_format = Some("tar".into());
        // parent_zip_id is None -> should error
        let result = build_archive_stream_info(&media);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Archive file ID not found"));
    }

    #[test]
    fn build_archive_stream_info_tar_compressed_method_nonzero() {
        let mut media = default_media_item();
        media.archive_format = Some("tar".into());
        media.parent_zip_id = Some("file1".into());
        media.zip_compression_method = Some(8);
        let result = build_archive_stream_info(&media);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires extraction"));
    }

    #[test]
    fn build_archive_stream_info_tar_store_method_success() {
        let mut media = default_media_item();
        media.archive_format = Some("tar".into());
        media.parent_zip_id = Some("file1".into());
        media.zip_compression_method = Some(0);
        media.zip_data_start_offset = Some(0);
        media.zip_compressed_size = Some(500);
        media.zip_entry_path = Some("video.mp4".into());
        let result = build_archive_stream_info(&media);
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.zip_file_id, "file1");
        assert_eq!(info.byte_start, 0);
        assert_eq!(info.byte_end, 499);
    }

    #[test]
    fn build_archive_stream_info_tar_missing_data_offset() {
        let mut media = default_media_item();
        media.archive_format = Some("tar".into());
        media.parent_zip_id = Some("file1".into());
        media.zip_compression_method = Some(0);
        // no data_start_offset
        let result = build_archive_stream_info(&media);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("does not support direct streaming"));
    }

    #[test]
    fn build_archive_stream_info_tar_missing_compressed_size() {
        let mut media = default_media_item();
        media.archive_format = Some("tar".into());
        media.parent_zip_id = Some("file1".into());
        media.zip_compression_method = Some(0);
        media.zip_data_start_offset = Some(100);
        // no compressed_size
        let result = build_archive_stream_info(&media);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("size not available"));
    }

    #[test]
    fn build_archive_stream_info_rar_store_method_success() {
        let mut media = default_media_item();
        media.archive_format = Some("rar".into());
        media.parent_zip_id = Some("file1".into());
        media.zip_compression_method = Some(0);
        media.zip_data_start_offset = Some(100);
        media.zip_compressed_size = Some(800);
        media.file_path = Some("https://example.com/file".into());
        let result = build_archive_stream_info(&media);
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.byte_start, 100);
        assert_eq!(info.byte_end, 899);
    }

    // ---- extract_archive_entry_to_cache dispatches correctly ----

    #[test]
    fn extract_archive_entry_to_cache_zip_no_parent_id_errors() {
        let mut media = default_media_item();
        media.archive_format = Some("zip".into());
        let config = zip_manager::ZipCacheConfig {
            cache_dir: "tmp".into(),
            max_size_bytes: 0,
            expiry_days: 30,
        };
        let result = extract_archive_entry_to_cache("token", &media, &config);
        assert!(result.is_err());
    }

    #[test]
    fn extract_archive_entry_to_cache_tar_no_parent_id_errors() {
        let mut media = default_media_item();
        media.archive_format = Some("tar".into());
        let config = zip_manager::ZipCacheConfig {
            cache_dir: "tmp".into(),
            max_size_bytes: 0,
            expiry_days: 30,
        };
        let result = extract_archive_entry_to_cache("token", &media, &config);
        assert!(result.is_err());
    }

    #[test]
    fn extract_archive_entry_to_cache_rar_no_parent_id_errors() {
        let mut media = default_media_item();
        media.archive_format = Some("rar".into());
        let config = zip_manager::ZipCacheConfig {
            cache_dir: "tmp".into(),
            max_size_bytes: 0,
            expiry_days: 30,
        };
        let result = extract_archive_entry_to_cache("token", &media, &config);
        assert!(result.is_err());
    }

    // ---- archive_format_for_media with all known values ----

    #[test]
    fn archive_format_for_media_zip_string() {
        let mut media = default_media_item();
        media.archive_format = Some("zip".into());
        assert_eq!(archive_format_for_media(&media), ArchiveFormat::Zip);
    }

    // ---- detect_archive_format priority: mime checked first for zip ----

    #[test]
    fn detect_mime_overrides_extension() {
        // name ends with .rar but mime says zip
        assert_eq!(
            detect_archive_format("file.rar", Some("application/zip")),
            Some(ArchiveFormat::Zip)
        );
    }

    #[test]
    fn detect_tar_mime_not_matching_gzip_extension() {
        // name is .tar.gz but mime is generic
        assert_eq!(
            detect_archive_format("file.tar.gz", Some("text/plain")),
            Some(ArchiveFormat::Tar)
        );
    }

    // ---- is_gzip_tar_path with real file ----

    #[test]
    fn is_gzip_tar_path_nonexistent_file_errors() {
        let result = is_gzip_tar_path(Path::new("/nonexistent/file.tar.gz"));
        assert!(result.is_err());
    }

    // ---- constants ----

    #[test]
    fn constants_are_sensible() {
        assert_eq!(DRIVE_API_BASE, "https://www.googleapis.com/drive/v3");
        assert_eq!(ARCHIVE_STREAM_BUFFER_BYTES, 1024 * 1024);
        assert_eq!(RAR_HEADER_PREFETCH_BYTES, 1024 * 1024);
    }
}
