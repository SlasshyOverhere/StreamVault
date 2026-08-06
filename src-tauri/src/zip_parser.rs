use serde::Serialize;
use std::path::Component;
use std::path::Path;
use thiserror::Error;

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_EOCD_SIGNATURE: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
const CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0201_4b50;
const LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const ZIP64_EXTRA_FIELD_ID: u16 = 0x0001;
const EOCD_MIN_SIZE: usize = 22;
const CENTRAL_DIRECTORY_FIXED_SIZE: usize = 46;
const LOCAL_FILE_HEADER_FIXED_SIZE: usize = 30;

#[derive(Debug, Clone, Serialize)]
pub struct EocdRecord {
    pub offset: u64,
    pub cd_offset: u64,
    pub cd_size: u64,
    pub cd_entries_total: u64,
    pub is_zip64: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ZipEntry {
    pub filename: String,
    pub compression_method: u16,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub local_header_offset: u64,
    pub crc32: u32,
    pub is_encrypted: bool,
    pub is_directory: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ZipCompressionType {
    Store,
    Deflate,
    Mixed,
    Other,
}

#[derive(Debug, Clone)]
pub struct LocalFileHeaderInfo {
    pub data_start_offset: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Zip64Extra {
    pub uncompressed_size: Option<u64>,
    pub compressed_size: Option<u64>,
    pub local_header_offset: Option<u64>,
}

#[derive(Debug, Error)]
pub enum ZipError {
    #[error("This file is not a valid ZIP archive")]
    NotAValidZip,
    #[error("Could not find the ZIP directory structure")]
    EocdNotFound,
    #[error("ZIP archive is truncated or corrupted")]
    CorruptedArchive,
    #[error("ZIP64 metadata is missing or incomplete")]
    Zip64ParsingError,
    #[error("ZIP contains an invalid path")]
    InvalidPath,
    #[error("ZIP contains a path traversal attempt")]
    PathTraversalAttempt,
    #[error("ZIP contains an invalid UTF-8 filename")]
    InvalidUtf8Filename,
    #[error("ZIP entry uses unsupported encryption")]
    EncryptedEntry,
    #[error("ZIP entry uses unsupported compression method {0}")]
    UnsupportedCompressionMethod(u16),
    #[error("Compressed ZIP entry must be extracted before playback")]
    EntryRequiresExtraction,
    #[error("Extracted ZIP entry failed integrity validation")]
    IntegrityCheckFailed,
    #[error("Drive API error: {0}")]
    DriveApiError(String),
    #[error("HTTP request failed: {0}")]
    HttpRequestError(String),
    #[error("Central directory too large: {size} bytes exceeds limit of {max} bytes")]
    CentralDirectoryTooLarge { size: u64, max: u64 },
    #[error("Too many ZIP entries: {count} exceeds limit of {max}")]
    TooManyEntries { count: usize, max: usize },
    #[error("HTTP {status}: {message}")]
    HttpStatus { status: u16, message: String },
}

pub fn find_eocd(data: &[u8], buffer_base_offset: u64) -> Result<EocdRecord, ZipError> {
    if data.len() < EOCD_MIN_SIZE {
        return Err(ZipError::EocdNotFound);
    }

    let mut idx = data.len() - EOCD_MIN_SIZE;
    loop {
        if read_u32(data, idx)? == EOCD_SIGNATURE {
            let entries_total = read_u16(data, idx + 10)? as u64;
            let cd_size_32 = read_u32(data, idx + 12)? as u64;
            let cd_offset_32 = read_u32(data, idx + 16)? as u64;
            let comment_length = read_u16(data, idx + 20)? as usize;

            if idx + EOCD_MIN_SIZE + comment_length > data.len() {
                return Err(ZipError::CorruptedArchive);
            }

            let eocd_offset = buffer_base_offset + idx as u64;
            let needs_zip64 = entries_total == u16::MAX as u64
                || cd_size_32 == u32::MAX as u64
                || cd_offset_32 == u32::MAX as u64;

            if !needs_zip64 {
                return Ok(EocdRecord {
                    offset: eocd_offset,
                    cd_offset: cd_offset_32,
                    cd_size: cd_size_32,
                    cd_entries_total: entries_total,
                    is_zip64: false,
                });
            }

            if idx < 20 || read_u32(data, idx - 20)? != ZIP64_LOCATOR_SIGNATURE {
                return Err(ZipError::Zip64ParsingError);
            }

            let zip64_eocd_offset = read_u64(data, idx - 12)?;
            if zip64_eocd_offset < buffer_base_offset {
                return Err(ZipError::Zip64ParsingError);
            }

            let relative_offset = (zip64_eocd_offset - buffer_base_offset) as usize;
            if read_u32(data, relative_offset)? != ZIP64_EOCD_SIGNATURE {
                return Err(ZipError::Zip64ParsingError);
            }

            let record_size = read_u64(data, relative_offset + 4)? as usize;
            let required = relative_offset + 12 + record_size;
            if required > data.len() {
                return Err(ZipError::Zip64ParsingError);
            }

            return Ok(EocdRecord {
                offset: eocd_offset,
                cd_offset: read_u64(data, relative_offset + 48)?,
                cd_size: read_u64(data, relative_offset + 40)?,
                cd_entries_total: read_u64(data, relative_offset + 32)?,
                is_zip64: true,
            });
        }

        if idx == 0 {
            break;
        }
        idx -= 1;
    }

    Err(ZipError::EocdNotFound)
}

pub fn parse_central_directory(
    cd_data: &[u8],
    _base_offset: u64,
) -> Result<Vec<ZipEntry>, ZipError> {
    let mut entries = Vec::new();
    let mut cursor = 0usize;

    while cursor < cd_data.len() {
        if cursor + CENTRAL_DIRECTORY_FIXED_SIZE > cd_data.len() {
            return Err(ZipError::CorruptedArchive);
        }

        if read_u32(cd_data, cursor)? != CENTRAL_DIRECTORY_SIGNATURE {
            return Err(ZipError::CorruptedArchive);
        }

        let flags = read_u16(cd_data, cursor + 8)?;
        let compression_method = read_u16(cd_data, cursor + 10)?;
        let crc32 = read_u32(cd_data, cursor + 16)?;
        let compressed_size_32 = read_u32(cd_data, cursor + 20)? as u64;
        let uncompressed_size_32 = read_u32(cd_data, cursor + 24)? as u64;
        let file_name_length = read_u16(cd_data, cursor + 28)? as usize;
        let extra_field_length = read_u16(cd_data, cursor + 30)? as usize;
        let file_comment_length = read_u16(cd_data, cursor + 32)? as usize;
        let local_header_offset_32 = read_u32(cd_data, cursor + 42)? as u64;

        let total_length = CENTRAL_DIRECTORY_FIXED_SIZE
            + file_name_length
            + extra_field_length
            + file_comment_length;
        if cursor + total_length > cd_data.len() {
            return Err(ZipError::CorruptedArchive);
        }

        let filename_start = cursor + CENTRAL_DIRECTORY_FIXED_SIZE;
        let filename_end = filename_start + file_name_length;
        let filename_bytes = &cd_data[filename_start..filename_end];
        let raw_filename =
            std::str::from_utf8(filename_bytes).map_err(|_| ZipError::InvalidUtf8Filename)?;
        let filename = sanitize_zip_entry_path(raw_filename)?;

        let extra_start = filename_end;
        let extra_end = extra_start + extra_field_length;
        let extra = &cd_data[extra_start..extra_end];

        let needs_uncompressed = uncompressed_size_32 == u32::MAX as u64;
        let needs_compressed = compressed_size_32 == u32::MAX as u64;
        let needs_offset = local_header_offset_32 == u32::MAX as u64;
        let zip64 = if needs_uncompressed || needs_compressed || needs_offset {
            parse_zip64_extra(extra, needs_uncompressed, needs_compressed, needs_offset)?
        } else {
            Zip64Extra::default()
        };

        let compressed_size = zip64.compressed_size.unwrap_or(compressed_size_32);
        let uncompressed_size = zip64.uncompressed_size.unwrap_or(uncompressed_size_32);
        let local_header_offset = zip64.local_header_offset.unwrap_or(local_header_offset_32);
        let is_directory = filename.ends_with('/');

        entries.push(ZipEntry {
            filename,
            compression_method,
            compressed_size,
            uncompressed_size,
            local_header_offset,
            crc32,
            is_encrypted: (flags & 0x0001) != 0,
            is_directory,
        });

        cursor += total_length;
    }

    Ok(entries)
}

pub fn parse_zip64_extra(
    extra_data: &[u8],
    need_uncompressed: bool,
    need_compressed: bool,
    need_offset: bool,
) -> Result<Zip64Extra, ZipError> {
    let mut cursor = 0usize;

    while cursor + 4 <= extra_data.len() {
        let header_id = read_u16(extra_data, cursor)?;
        let data_size = read_u16(extra_data, cursor + 2)? as usize;
        cursor += 4;

        if cursor + data_size > extra_data.len() {
            return Err(ZipError::Zip64ParsingError);
        }

        if header_id == ZIP64_EXTRA_FIELD_ID {
            let mut field_cursor = cursor;
            let mut zip64 = Zip64Extra::default();

            if need_uncompressed {
                zip64.uncompressed_size = Some(read_u64(extra_data, field_cursor)?);
                field_cursor += 8;
            }
            if need_compressed {
                zip64.compressed_size = Some(read_u64(extra_data, field_cursor)?);
                field_cursor += 8;
            }
            if need_offset {
                zip64.local_header_offset = Some(read_u64(extra_data, field_cursor)?);
            }

            if need_uncompressed && zip64.uncompressed_size.is_none() {
                return Err(ZipError::Zip64ParsingError);
            }
            if need_compressed && zip64.compressed_size.is_none() {
                return Err(ZipError::Zip64ParsingError);
            }
            if need_offset && zip64.local_header_offset.is_none() {
                return Err(ZipError::Zip64ParsingError);
            }

            return Ok(zip64);
        }

        cursor += data_size;
    }

    Err(ZipError::Zip64ParsingError)
}

/// Sanitize a ZIP entry path to prevent path traversal attacks.
/// Rejects absolute paths, Windows drive letters, and '..' traversal components.
pub fn sanitize_zip_entry_path(filename: &str) -> Result<String, ZipError> {
    if filename.is_empty() {
        return Err(ZipError::InvalidPath);
    }

    let normalized = filename.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(ZipError::InvalidPath);
    }

    if normalized.len() > 1 && normalized.as_bytes()[1] == b':' {
        return Err(ZipError::InvalidPath);
    }

    let mut clean_parts = Vec::new();
    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.is_empty() {
                    continue;
                }
                clean_parts.push(part.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir => return Err(ZipError::PathTraversalAttempt),
            Component::RootDir | Component::Prefix(_) => return Err(ZipError::InvalidPath),
        }
    }

    if clean_parts.is_empty() {
        return Err(ZipError::InvalidPath);
    }

    let trailing_slash = normalized.ends_with('/');
    let mut sanitized = clean_parts.join("/");
    if trailing_slash {
        sanitized.push('/');
    }
    Ok(sanitized)
}

pub fn parse_local_file_header(
    header_data: &[u8],
    local_header_offset: u64,
) -> Result<LocalFileHeaderInfo, ZipError> {
    if header_data.len() < LOCAL_FILE_HEADER_FIXED_SIZE {
        return Err(ZipError::CorruptedArchive);
    }

    if read_u32(header_data, 0)? != LOCAL_FILE_HEADER_SIGNATURE {
        return Err(ZipError::NotAValidZip);
    }

    let file_name_length = read_u16(header_data, 26)? as u64;
    let extra_field_length = read_u16(header_data, 28)? as u64;

    Ok(LocalFileHeaderInfo {
        data_start_offset: local_header_offset
            + LOCAL_FILE_HEADER_FIXED_SIZE as u64
            + file_name_length
            + extra_field_length,
    })
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, ZipError> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or(ZipError::CorruptedArchive)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, ZipError> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or(ZipError::CorruptedArchive)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, ZipError> {
    let bytes = data
        .get(offset..offset + 8)
        .ok_or(ZipError::CorruptedArchive)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

#[cfg(test)]
mod tests {
    use super::{
        find_eocd, parse_central_directory, parse_local_file_header, parse_zip64_extra,
        sanitize_zip_entry_path, Zip64Extra, ZipCompressionType, ZipError,
    };

    #[test]
    fn sanitize_rejects_path_traversal() {
        assert!(sanitize_zip_entry_path("../episode.mkv").is_err());
        assert!(sanitize_zip_entry_path("C:/episode.mkv").is_err());
        // Note: "...." is NOT path traversal — four dots is a valid filename component
        assert!(sanitize_zip_entry_path("foo/../../episode.mkv").is_err());
        assert!(sanitize_zip_entry_path("foo/..\\bar/episode.mkv").is_err());
        assert!(sanitize_zip_entry_path("/absolute/path.mkv").is_err());
        assert!(sanitize_zip_entry_path("//unc/path.mkv").is_err());
        assert!(sanitize_zip_entry_path("").is_err());
        // Note: "..." is NOT path traversal — it's a valid filename component
        assert!(sanitize_zip_entry_path("foo/./../bar.mkv").is_err());
        assert!(sanitize_zip_entry_path("foo/..").is_err());
        assert!(sanitize_zip_entry_path("..").is_err());
    }

    #[test]
    fn sanitize_accepts_valid_paths() {
        assert!(sanitize_zip_entry_path("episode.mkv").is_ok());
        assert!(sanitize_zip_entry_path("Season 01/episode.mkv").is_ok());
        assert!(sanitize_zip_entry_path("foo/bar/episode.mkv").is_ok());
        assert!(sanitize_zip_entry_path("foo/./bar.mkv").is_ok());
        assert!(sanitize_zip_entry_path("normal_path.mkv").is_ok());
    }

    #[test]
    fn sanitize_normalizes_slashes() {
        assert_eq!(
            sanitize_zip_entry_path("Season 01\\Show.S01E01.mkv").unwrap(),
            "Season 01/Show.S01E01.mkv"
        );
    }

    #[test]
    fn parse_local_header_calculates_data_offset() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&20u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.extend_from_slice(&10u32.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(b"file.mkv");
        bytes.extend_from_slice(&[1, 2, 3, 4]);

        let header = parse_local_file_header(&bytes, 200).unwrap();
        assert_eq!(header.data_start_offset, 200 + 30 + 8 + 4);
    }

    #[test]
    fn finds_simple_eocd() {
        let mut bytes = vec![0u8; 30];
        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&123u32.to_le_bytes());
        bytes.extend_from_slice(&456u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let eocd = find_eocd(&bytes, 0).unwrap();
        assert_eq!(eocd.cd_offset, 456);
        assert_eq!(eocd.cd_size, 123);
        assert_eq!(eocd.cd_entries_total, 1);
    }

    #[test]
    fn parses_central_directory_entry() {
        let mut entry = Vec::new();
        entry.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        entry.extend_from_slice(&20u16.to_le_bytes());
        entry.extend_from_slice(&20u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        entry.extend_from_slice(&100u32.to_le_bytes());
        entry.extend_from_slice(&100u32.to_le_bytes());
        entry.extend_from_slice(&15u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&2048u32.to_le_bytes());
        entry.extend_from_slice(b"Show.S01E01.mkv");

        let entries = parse_central_directory(&entry, 0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename, "Show.S01E01.mkv");
        assert_eq!(entries[0].compressed_size, 100);
        assert_eq!(ZipCompressionType::Store, ZipCompressionType::Store);
    }

    // --- find_eocd edge cases ---

    #[test]
    fn find_eocd_rejects_too_small() {
        assert!(find_eocd(&[0u8; 21], 0).is_err());
        assert!(find_eocd(&[], 0).is_err());
    }

    #[test]
    fn find_eocd_rejects_no_signature() {
        let data = vec![0xFFu8; 22];
        assert!(find_eocd(&data, 0).is_err());
    }

    #[test]
    fn find_eocd_rejects_truncated_comment() {
        // EOCD at offset 0, comment_length=10 but only 22 bytes total
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes()); // signature
        bytes.extend_from_slice(&0u16.to_le_bytes()); // disk
        bytes.extend_from_slice(&0u16.to_le_bytes()); // cd_disk
        bytes.extend_from_slice(&0u16.to_le_bytes()); // entries_this_disk
        bytes.extend_from_slice(&0u16.to_le_bytes()); // entries_total
        bytes.extend_from_slice(&0u32.to_le_bytes()); // cd_size
        bytes.extend_from_slice(&0u32.to_le_bytes()); // cd_offset
        bytes.extend_from_slice(&10u16.to_le_bytes()); // comment_length=10 but no comment bytes
        assert!(find_eocd(&bytes, 0).is_err());
    }

    #[test]
    fn find_eocd_with_comment() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes()); // entries_this_disk
        bytes.extend_from_slice(&2u16.to_le_bytes()); // entries_total
        bytes.extend_from_slice(&50u32.to_le_bytes()); // cd_size
        bytes.extend_from_slice(&100u32.to_le_bytes()); // cd_offset
        bytes.extend_from_slice(&5u16.to_le_bytes()); // comment_length=5
        bytes.extend_from_slice(b"hello"); // comment bytes

        let eocd = find_eocd(&bytes, 0).unwrap();
        assert_eq!(eocd.cd_entries_total, 2);
        assert_eq!(eocd.cd_size, 50);
        assert_eq!(eocd.cd_offset, 100);
        assert!(!eocd.is_zip64);
    }

    #[test]
    fn find_eocd_with_buffer_base_offset() {
        let mut bytes = vec![0u8; 10]; // padding before EOCD
        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&200u32.to_le_bytes());
        bytes.extend_from_slice(&300u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let eocd = find_eocd(&bytes, 500).unwrap();
        assert_eq!(eocd.offset, 500 + 10); // buffer_base + index of signature
        assert_eq!(eocd.cd_offset, 300);
        assert_eq!(eocd.cd_size, 200);
    }

    #[test]
    fn find_eocd_rejects_zip64_without_locator() {
        // u16::MAX entries triggers zip64 path, but no locator present
        let mut bytes = vec![0u8; 22];
        bytes.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&u16::MAX.to_le_bytes()); // entries_total = u16::MAX -> needs zip64
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());

        let result = find_eocd(&bytes, 0);
        assert!(result.is_err());
    }

    // --- parse_local_file_header edge cases ---

    #[test]
    fn parse_local_header_rejects_too_small() {
        assert!(parse_local_file_header(&[0u8; 29], 0).is_err());
        assert!(parse_local_file_header(&[], 0).is_err());
    }

    #[test]
    fn parse_local_header_rejects_bad_signature() {
        let mut bytes = vec![0u8; 30];
        bytes[0..4].copy_from_slice(&0x9999_9999u32.to_le_bytes());
        assert!(matches!(
            parse_local_file_header(&bytes, 0),
            Err(ZipError::NotAValidZip)
        ));
    }

    // --- parse_zip64_extra tests ---

    #[test]
    fn parse_zip64_extra_finds_uncompressed() {
        // header_id=0x0001, data_size=8, value=0x1234567890ABCDEF
        let mut extra = Vec::new();
        extra.extend_from_slice(&0x0001u16.to_le_bytes()); // header_id
        extra.extend_from_slice(&8u16.to_le_bytes()); // data_size
        extra.extend_from_slice(&0x1234_5678_90AB_CDEFu64.to_le_bytes());

        let result = parse_zip64_extra(&extra, true, false, false).unwrap();
        assert_eq!(result.uncompressed_size, Some(0x1234_5678_90AB_CDEF));
        assert!(result.compressed_size.is_none());
        assert!(result.local_header_offset.is_none());
    }

    #[test]
    fn parse_zip64_extra_finds_compressed() {
        let mut extra = Vec::new();
        extra.extend_from_slice(&0x0001u16.to_le_bytes());
        extra.extend_from_slice(&8u16.to_le_bytes());
        extra.extend_from_slice(&500u64.to_le_bytes());

        let result = parse_zip64_extra(&extra, false, true, false).unwrap();
        assert!(result.uncompressed_size.is_none());
        assert_eq!(result.compressed_size, Some(500));
        assert!(result.local_header_offset.is_none());
    }

    #[test]
    fn parse_zip64_extra_finds_offset() {
        let mut extra = Vec::new();
        extra.extend_from_slice(&0x0001u16.to_le_bytes());
        extra.extend_from_slice(&8u16.to_le_bytes());
        extra.extend_from_slice(&999u64.to_le_bytes());

        let result = parse_zip64_extra(&extra, false, false, true).unwrap();
        assert!(result.uncompressed_size.is_none());
        assert!(result.compressed_size.is_none());
        assert_eq!(result.local_header_offset, Some(999));
    }

    #[test]
    fn parse_zip64_extra_finds_all_three() {
        let mut extra = Vec::new();
        extra.extend_from_slice(&0x0001u16.to_le_bytes());
        extra.extend_from_slice(&24u16.to_le_bytes()); // 3 * 8
        extra.extend_from_slice(&100u64.to_le_bytes()); // uncompressed
        extra.extend_from_slice(&200u64.to_le_bytes()); // compressed
        extra.extend_from_slice(&300u64.to_le_bytes()); // offset

        let result = parse_zip64_extra(&extra, true, true, true).unwrap();
        assert_eq!(result.uncompressed_size, Some(100));
        assert_eq!(result.compressed_size, Some(200));
        assert_eq!(result.local_header_offset, Some(300));
    }

    #[test]
    fn parse_zip64_extra_skips_non_zip64_fields() {
        let mut extra = Vec::new();
        // non-zip64 field
        extra.extend_from_slice(&0x0002u16.to_le_bytes()); // different header_id
        extra.extend_from_slice(&4u16.to_le_bytes());
        extra.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        // actual zip64 field
        extra.extend_from_slice(&0x0001u16.to_le_bytes());
        extra.extend_from_slice(&8u16.to_le_bytes());
        extra.extend_from_slice(&42u64.to_le_bytes());

        let result = parse_zip64_extra(&extra, true, false, false).unwrap();
        assert_eq!(result.uncompressed_size, Some(42));
    }

    #[test]
    fn parse_zip64_extra_rejects_empty() {
        assert!(parse_zip64_extra(&[], true, false, false).is_err());
    }

    #[test]
    fn parse_zip64_extra_rejects_truncated_header() {
        // Only 2 bytes, need 4 for header
        assert!(parse_zip64_extra(&[0x01, 0x00], true, false, false).is_err());
    }

    #[test]
    fn parse_zip64_extra_rejects_truncated_data() {
        // header says data_size=8 but only 4 bytes follow
        let mut extra = Vec::new();
        extra.extend_from_slice(&0x0001u16.to_le_bytes());
        extra.extend_from_slice(&8u16.to_le_bytes());
        extra.extend_from_slice(&[0u8; 4]); // only 4 of 8 bytes

        assert!(parse_zip64_extra(&extra, true, false, false).is_err());
    }

    #[test]
    fn parse_zip64_extra_rejects_when_need_uncompressed_but_absent() {
        // zip64 field present but data_size=0 (no fields)
        let mut extra = Vec::new();
        extra.extend_from_slice(&0x0001u16.to_le_bytes());
        extra.extend_from_slice(&0u16.to_le_bytes()); // data_size=0

        assert!(parse_zip64_extra(&extra, true, false, false).is_err());
    }

    #[test]
    fn parse_zip64_extra_none_needed_returns_default() {
        let mut extra = Vec::new();
        extra.extend_from_slice(&0x0001u16.to_le_bytes());
        extra.extend_from_slice(&8u16.to_le_bytes());
        extra.extend_from_slice(&42u64.to_le_bytes());

        let result = parse_zip64_extra(&extra, false, false, false).unwrap();
        assert!(result.uncompressed_size.is_none());
        assert!(result.compressed_size.is_none());
        assert!(result.local_header_offset.is_none());
    }

    // --- parse_central_directory edge cases ---

    #[test]
    fn parse_cd_rejects_bad_signature() {
        let mut entry = vec![0u8; 46];
        entry[0..4].copy_from_slice(&0x9999_9999u32.to_le_bytes());
        assert!(parse_central_directory(&entry, 0).is_err());
    }

    #[test]
    fn parse_cd_rejects_truncated_entry() {
        // 30 bytes < CENTRAL_DIRECTORY_FIXED_SIZE (46)
        assert!(parse_central_directory(&vec![0u8; 30], 0).is_err());
    }

    #[test]
    fn parse_cd_rejects_truncated_extra_field() {
        // Valid header but extra_field_length extends past end
        let mut entry = Vec::new();
        entry.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // signature
        entry.extend_from_slice(&20u16.to_le_bytes()); // version
        entry.extend_from_slice(&20u16.to_le_bytes()); // version_needed
        entry.extend_from_slice(&0u16.to_le_bytes()); // flags
        entry.extend_from_slice(&0u16.to_le_bytes()); // compression
        entry.extend_from_slice(&0u16.to_le_bytes()); // mod_time
        entry.extend_from_slice(&0u16.to_le_bytes()); // mod_date
        entry.extend_from_slice(&0u32.to_le_bytes()); // crc32
        entry.extend_from_slice(&0u32.to_le_bytes()); // compressed_size
        entry.extend_from_slice(&0u32.to_le_bytes()); // uncompressed_size
        entry.extend_from_slice(&3u16.to_le_bytes()); // file_name_length=3
        entry.extend_from_slice(&100u16.to_le_bytes()); // extra_field_length=100 (too big)
        entry.extend_from_slice(&0u16.to_le_bytes()); // comment_length
        entry.extend_from_slice(&0u16.to_le_bytes()); // disk_start
        entry.extend_from_slice(&0u16.to_le_bytes()); // internal_attr
        entry.extend_from_slice(&0u32.to_le_bytes()); // external_attr
        entry.extend_from_slice(&0u32.to_le_bytes()); // local_header_offset
        entry.extend_from_slice(b"abc"); // filename (only 3 bytes, extra needs 100 more but not there)

        assert!(parse_central_directory(&entry, 0).is_err());
    }

    #[test]
    fn parse_cd_rejects_invalid_utf8_filename() {
        let mut entry = Vec::new();
        entry.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        entry.extend_from_slice(&20u16.to_le_bytes());
        entry.extend_from_slice(&20u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&3u16.to_le_bytes()); // filename length=3
        entry.extend_from_slice(&0u16.to_le_bytes()); // extra
        entry.extend_from_slice(&0u16.to_le_bytes()); // comment
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&[0xFF, 0xFE, 0xFD]); // invalid UTF-8

        let result = parse_central_directory(&entry, 0);
        assert!(result.is_err());
    }

    #[test]
    fn parse_cd_sets_directory_flag() {
        let mut entry = Vec::new();
        entry.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        entry.extend_from_slice(&20u16.to_le_bytes());
        entry.extend_from_slice(&20u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&11u16.to_le_bytes()); // filename length
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(b"mydir/file/");

        let entries = parse_central_directory(&entry, 0).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_directory);
        assert_eq!(entries[0].filename, "mydir/file/");
    }

    #[test]
    fn parse_cd_sets_encrypted_flag() {
        let mut entry = Vec::new();
        entry.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        entry.extend_from_slice(&20u16.to_le_bytes());
        entry.extend_from_slice(&20u16.to_le_bytes());
        entry.extend_from_slice(&1u16.to_le_bytes()); // flags bit 0 = encrypted
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&6u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(b"a.b c.");

        let entries = parse_central_directory(&entry, 0).unwrap();
        assert!(entries[0].is_encrypted);
    }

    #[test]
    fn parse_cd_multiple_entries() {
        let mut data = Vec::new();
        for i in 0..3u8 {
            let name = format!("file{i}.txt");
            data.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            data.extend_from_slice(&20u16.to_le_bytes());
            data.extend_from_slice(&20u16.to_le_bytes());
            data.extend_from_slice(&0u16.to_le_bytes()); // flags
            data.extend_from_slice(&8u16.to_le_bytes()); // deflate
            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(&(i as u32 * 100).to_le_bytes()); // crc32
            data.extend_from_slice(&(i as u32 * 50 + 10).to_le_bytes()); // compressed
            data.extend_from_slice(&(i as u32 * 100 + 20).to_le_bytes()); // uncompressed
            data.extend_from_slice(&(name.len() as u16).to_le_bytes()); // filename length
            data.extend_from_slice(&0u16.to_le_bytes()); // extra
            data.extend_from_slice(&0u16.to_le_bytes()); // comment
            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(&0u16.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&(i as u32 * 1000).to_le_bytes()); // local_header_offset
            data.extend_from_slice(name.as_bytes());
        }

        let entries = parse_central_directory(&data, 0).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].filename, "file0.txt");
        assert_eq!(entries[1].filename, "file1.txt");
        assert_eq!(entries[2].filename, "file2.txt");
        assert_eq!(entries[0].compression_method, 8);
        assert_eq!(entries[1].compressed_size, 60);
        assert_eq!(entries[2].uncompressed_size, 220);
        assert_eq!(entries[2].crc32, 200);
        assert_eq!(entries[0].local_header_offset, 0);
        assert_eq!(entries[1].local_header_offset, 1000);
        assert_eq!(entries[2].local_header_offset, 2000);
    }

    #[test]
    fn parse_cd_empty_data() {
        let entries = parse_central_directory(&[], 0).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_cd_with_zip64_extra() {
        let mut entry = Vec::new();
        entry.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        entry.extend_from_slice(&45u16.to_le_bytes()); // version (zip64)
        entry.extend_from_slice(&45u16.to_le_bytes()); // version_needed
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&u32::MAX.to_le_bytes()); // compressed_size -> zip64
        entry.extend_from_slice(&u32::MAX.to_le_bytes()); // uncompressed_size -> zip64
        entry.extend_from_slice(&4u16.to_le_bytes()); // filename_length
        entry.extend_from_slice(&28u16.to_le_bytes()); // extra_field_length (4 header + 24 data)
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u32.to_le_bytes());
        entry.extend_from_slice(&u32::MAX.to_le_bytes()); // local_header_offset -> zip64
        entry.extend_from_slice(b"a.mk");
        // zip64 extra field
        entry.extend_from_slice(&0x0001u16.to_le_bytes()); // header_id
        entry.extend_from_slice(&24u16.to_le_bytes()); // data_size (3 * 8)
        entry.extend_from_slice(&9999u64.to_le_bytes()); // uncompressed
        entry.extend_from_slice(&8888u64.to_le_bytes()); // compressed
        entry.extend_from_slice(&7777u64.to_le_bytes()); // offset

        let entries = parse_central_directory(&entry, 0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].uncompressed_size, 9999);
        assert_eq!(entries[0].compressed_size, 8888);
        assert_eq!(entries[0].local_header_offset, 7777);
    }

    // --- ZipCompressionType tests ---

    #[test]
    fn zip_compression_type_variants() {
        assert_eq!(ZipCompressionType::Store, ZipCompressionType::Store);
        assert_eq!(ZipCompressionType::Deflate, ZipCompressionType::Deflate);
        assert_eq!(ZipCompressionType::Mixed, ZipCompressionType::Mixed);
        assert_eq!(ZipCompressionType::Other, ZipCompressionType::Other);
        assert_ne!(ZipCompressionType::Store, ZipCompressionType::Deflate);
        assert_ne!(ZipCompressionType::Mixed, ZipCompressionType::Other);
    }

    // --- ZipError display tests ---

    #[test]
    fn zip_error_display_messages() {
        assert_eq!(
            format!("{}", ZipError::NotAValidZip),
            "This file is not a valid ZIP archive"
        );
        assert_eq!(
            format!("{}", ZipError::EocdNotFound),
            "Could not find the ZIP directory structure"
        );
        assert_eq!(
            format!("{}", ZipError::CorruptedArchive),
            "ZIP archive is truncated or corrupted"
        );
        assert_eq!(
            format!("{}", ZipError::Zip64ParsingError),
            "ZIP64 metadata is missing or incomplete"
        );
        assert_eq!(
            format!("{}", ZipError::InvalidPath),
            "ZIP contains an invalid path"
        );
        assert_eq!(
            format!("{}", ZipError::PathTraversalAttempt),
            "ZIP contains a path traversal attempt"
        );
        assert_eq!(
            format!("{}", ZipError::InvalidUtf8Filename),
            "ZIP contains an invalid UTF-8 filename"
        );
        assert_eq!(
            format!("{}", ZipError::EncryptedEntry),
            "ZIP entry uses unsupported encryption"
        );
        assert_eq!(
            format!("{}", ZipError::UnsupportedCompressionMethod(99)),
            "ZIP entry uses unsupported compression method 99"
        );
        assert_eq!(
            format!("{}", ZipError::EntryRequiresExtraction),
            "Compressed ZIP entry must be extracted before playback"
        );
        assert_eq!(
            format!("{}", ZipError::IntegrityCheckFailed),
            "Extracted ZIP entry failed integrity validation"
        );
        assert_eq!(
            format!("{}", ZipError::DriveApiError("timeout".into())),
            "Drive API error: timeout"
        );
        assert_eq!(
            format!("{}", ZipError::HttpRequestError("conn refused".into())),
            "HTTP request failed: conn refused"
        );
        assert_eq!(
            format!(
                "{}",
                ZipError::CentralDirectoryTooLarge { size: 100, max: 50 }
            ),
            "Central directory too large: 100 bytes exceeds limit of 50 bytes"
        );
        assert_eq!(
            format!(
                "{}",
                ZipError::TooManyEntries {
                    count: 200,
                    max: 100
                }
            ),
            "Too many ZIP entries: 200 exceeds limit of 100"
        );
        assert_eq!(
            format!(
                "{}",
                ZipError::HttpStatus {
                    status: 404,
                    message: "not found".into()
                }
            ),
            "HTTP 404: not found"
        );
    }

    // --- sanitize_zip_entry_path extra edge cases ---

    #[test]
    fn sanitize_rejects_root_only() {
        assert!(sanitize_zip_entry_path("/").is_err());
    }

    #[test]
    fn sanitize_rejects_dot_only() {
        // "." resolves to no components after cleaning
        assert!(sanitize_zip_entry_path(".").is_err());
    }

    #[test]
    fn sanitize_preserves_trailing_slash() {
        assert_eq!(
            sanitize_zip_entry_path("dir/subdir/").unwrap(),
            "dir/subdir/"
        );
    }

    #[test]
    fn sanitize_normalizes_mixed_slashes() {
        assert_eq!(sanitize_zip_entry_path("a\\b/c\\d").unwrap(), "a/b/c/d");
    }

    #[test]
    fn sanitize_rejects_windows_drive_variations() {
        assert!(sanitize_zip_entry_path("d:/file.txt").is_err());
        assert!(sanitize_zip_entry_path("Z:/file.txt").is_err());
        assert!(sanitize_zip_entry_path("C:\\file.txt").is_err());
    }

    #[test]
    fn sanitize_rejects_double_dot_at_end() {
        assert!(sanitize_zip_entry_path("dir/..").is_err());
    }

    #[test]
    fn sanitize_single_dot_components_ok() {
        // "a/./b" should normalize to "a/b"
        assert_eq!(sanitize_zip_entry_path("a/./b").unwrap(), "a/b");
    }

    // --- Zip64Extra default test ---

    #[test]
    fn zip64_extra_default_is_none() {
        let z = Zip64Extra::default();
        assert!(z.uncompressed_size.is_none());
        assert!(z.compressed_size.is_none());
        assert!(z.local_header_offset.is_none());
    }
}
