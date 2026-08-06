use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStream {
    pub name: String,
    pub description: String,
    pub url: String,
    #[serde(default)]
    pub video_size: i64,
    #[serde(default)]
    pub not_web_ready: bool,
    #[serde(skip)]
    pub parsed_quality: String,
    #[serde(skip)]
    pub parsed_source: String,
    #[serde(default)]
    pub recommended: bool,
    #[serde(skip)]
    pub is_hubdrive: bool,
    #[serde(skip)]
    pub episode_number: Option<i32>,
}

fn is_recommended_url(url: &str) -> bool {
    url.contains("r2.dev")
}

fn is_hubdrive_url(url: &str) -> bool {
    // Only flag actual hubdrive download pages, not hubcloud CDN streams
    // ponytail: substring match, upgrade to URL parsing if false positives persist
    let lower = url.to_lowercase();
    lower.contains("://hubdrive.") || lower.contains("://hubstream.")
}

/// Validate a hubdrive URL by checking page title.
/// Expired: title contains generic "G-Drive File Sharing" text.
/// Working: title contains actual content name.
/// Returns (is_valid, title).
pub fn validate_hubdrive_url(url: &str) -> Result<(bool, String), String> {
    let client = crate::http_client::shared_client();
    let resp = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;
    let body = resp.text().map_err(|e| e.to_string())?;
    let title = if let Some(s) = body.find("<title>") {
        let after = &body[s + 7..];
        after
            .find("</title>")
            .map(|e| after[..e].trim().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let lower = title.to_lowercase();
    let expired = lower.contains("g-drive file sharing")
        || lower.contains("g=drive")
        || lower.contains("shorten your google drive");
    Ok((!expired, title))
}

// ponytail: naive regex-free ep number extraction, works for "E01", "Episode 1", "Ep 01"
fn extract_episode_number(name: &str, description: &str) -> Option<i32> {
    let combined = format!("{} {}", name, description);
    // Try "E01" or "E1" pattern first
    for cap in combined.split(|c: char| !c.is_ascii_alphanumeric()) {
        if cap.len() >= 2 && (cap.starts_with('E') || cap.starts_with('e')) {
            if let Ok(n) = cap[1..].parse::<i32>() {
                if n > 0 && n < 1000 {
                    return Some(n);
                }
            }
        }
    }
    // Try "Episode N" or "Ep N"
    let lower = combined.to_lowercase();
    for prefix in &["episode ", "ep "] {
        if let Some(pos) = lower.find(prefix) {
            let rest = &combined[pos + prefix.len()..];
            let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = num_str.parse::<i32>() {
                if n > 0 && n < 1000 {
                    return Some(n);
                }
            }
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct RawStream {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    url: String,
    #[serde(default)]
    behavior_hints: BehaviorHints,
}

#[derive(Debug, Deserialize, Default)]
struct BehaviorHints {
    #[serde(default)]
    not_web_ready: bool,
    #[serde(default, deserialize_with = "deserialize_f64_as_i64")]
    video_size: i64,
}

fn deserialize_f64_as_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: f64 = serde::Deserialize::deserialize(deserializer)?;
    Ok(v as i64)
}

#[derive(Debug, Deserialize)]
struct StreamsResponse {
    streams: Vec<RawStream>,
    #[serde(default)]
    cache_max_age: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupedStreams {
    pub quality: String,
    pub streams: Vec<RemoteStream>,
}

pub fn parse_quality(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("4k") || lower.contains("2160p") {
        "4K".to_string()
    } else if lower.contains("1080p") {
        "1080p".to_string()
    } else if lower.contains("720p") {
        "720p".to_string()
    } else if lower.contains("480p") {
        "480p".to_string()
    } else {
        "Unknown".to_string()
    }
}

pub fn parse_source(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("4k") {
        "Premium".to_string()
    } else {
        "Standard".to_string()
    }
}

pub fn fetch_movie_streams(
    imdb_id: &str,
    base_url: &str,
    _force_refresh: bool,
) -> Result<Vec<RemoteStream>, String> {
    let url = format!(
        "{}/stream/movie/{}.json",
        base_url.trim_end_matches('/'),
        imdb_id
    );
    let streams = fetch_and_parse_streams(&url)?;
    Ok(streams)
}

pub fn fetch_series_streams(
    imdb_id: &str,
    season: i32,
    episode: i32,
    base_url: &str,
    _force_refresh: bool,
) -> Result<Vec<RemoteStream>, String> {
    let url = format!(
        "{}/stream/series/{}:{}:{}.json",
        base_url.trim_end_matches('/'),
        imdb_id,
        season,
        episode
    );
    let streams = fetch_and_parse_streams(&url)?;
    Ok(streams)
}

/// Fetch all streams for an entire season using the `tt{imdb}:{season}:full` endpoint.
/// Returns a map of episode_number -> streams.
pub fn fetch_season_streams(
    imdb_id: &str,
    season: i32,
    base_url: &str,
    _force_refresh: bool,
) -> Result<HashMap<i32, Vec<RemoteStream>>, String> {
    let url = format!(
        "{}/stream/series/{}:{}:full.json",
        base_url.trim_end_matches('/'),
        imdb_id,
        season
    );
    let streams = fetch_and_parse_streams(&url)?;

    let mut by_ep: HashMap<i32, Vec<RemoteStream>> = HashMap::new();
    for s in streams {
        let ep = s.episode_number.unwrap_or(0);
        by_ep.entry(ep).or_default().push(s);
    }
    Ok(by_ep)
}

fn is_loopback_url(url: &str) -> bool {
    url.contains("127.0.0.1") || url.contains("localhost") || url.contains("[::1]")
}

fn fetch_and_parse_streams(url: &str) -> Result<Vec<RemoteStream>, String> {
    // Use raw TCP client for localhost to bypass reqwest loopback restriction
    if is_loopback_url(url) {
        let body = crate::http_client::local_http_get(url)?;
        return parse_streams_body(&body);
    }

    let client = crate::http_client::shared_client();
    let max_retries = 3u32;
    let mut last_error = String::new();

    for attempt in 0..max_retries {
        if attempt > 0 {
            let delay_ms = 1000 * (1 << attempt);
            std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
            println!(
                "[REMOTE] Stream fetch retry attempt {} after {}ms delay",
                attempt + 1,
                delay_ms
            );
        }

        let response = match client.get(url).send() {
            Ok(r) => r,
            Err(e) => {
                last_error = format!("Failed to fetch streams: {}", e);
                println!(
                    "[REMOTE] Stream fetch network error (attempt {}): {}",
                    attempt + 1,
                    last_error
                );
                continue;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            if status.is_client_error() {
                return Err(format!("Server returned {}", status));
            }
            last_error = format!("Server returned {}", status);
            println!(
                "[REMOTE] Stream fetch server error (attempt {}): {}",
                attempt + 1,
                last_error
            );
            continue;
        }

        let body = response
            .text()
            .map_err(|e| format!("Failed to read response body: {}", e))?;

        return parse_streams_body(&body);
    }

    Err(format!(
        "Failed to fetch streams after {} retries: {}",
        max_retries, last_error
    ))
}

fn parse_streams_body(body: &str) -> Result<Vec<RemoteStream>, String> {
    let raw: StreamsResponse =
        serde_json::from_str(body).map_err(|e| format!("Failed to parse response: {}", e))?;

    let streams: Vec<RemoteStream> = raw
        .streams
        .into_iter()
        .map(|s| {
            let quality = parse_quality(&s.name);
            let source = parse_source(&s.name);
            let desc = s.description.or(s.title).unwrap_or_default();
            let ep = extract_episode_number(&s.name, &desc);
            RemoteStream {
                name: s.name.clone(),
                description: desc,
                url: s.url.clone(),
                video_size: s.behavior_hints.video_size,
                not_web_ready: s.behavior_hints.not_web_ready,
                parsed_quality: quality,
                parsed_source: source,
                recommended: is_recommended_url(&s.url),
                is_hubdrive: is_hubdrive_url(&s.url),
                episode_number: ep,
            }
        })
        .collect();

    if streams.is_empty() {
        return Err("No streams available for this content".to_string());
    }

    Ok(streams)
}

pub fn group_streams(streams: Vec<RemoteStream>) -> Vec<GroupedStreams> {
    let mut groups: std::collections::HashMap<String, Vec<RemoteStream>> =
        std::collections::HashMap::new();

    for stream in streams {
        groups
            .entry(stream.parsed_quality.clone())
            .or_default()
            .push(stream);
    }

    let quality_order = ["4K", "2160p", "1080p", "720p", "480p", "Unknown"];
    let mut result: Vec<(usize, String, Vec<RemoteStream>)> = groups
        .into_iter()
        .filter_map(|(quality, mut streams)| {
            // Recommended streams first within each group, then by video size descending
            streams.sort_by(|a, b| {
                b.recommended.cmp(&a.recommended).then_with(|| {
                    b.video_size
                        .partial_cmp(&a.video_size)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            });
            let rank = quality_order
                .iter()
                .position(|q| *q == quality)
                .unwrap_or(usize::MAX);
            Some((rank, quality, streams))
        })
        .collect();

    // Sort groups: those containing recommended streams float to the top,
    // otherwise by quality rank
    result.sort_by(|(ra, _, sa), (rb, _, sb)| {
        let a_has_rec = sa.iter().any(|s| s.recommended);
        let b_has_rec = sb.iter().any(|s| s.recommended);
        b_has_rec.cmp(&a_has_rec).then_with(|| ra.cmp(rb))
    });

    result
        .into_iter()
        .map(|(_, quality, streams)| GroupedStreams { quality, streams })
        .collect()
}

pub fn format_file_size(bytes: i64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    format!("{:.2} {}", size, UNITS[unit_idx])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamVerification {
    pub url: String,
    pub active: bool,
}

fn try_probe_range_get(client: &reqwest::blocking::Client, url: &str) -> Option<bool> {
    let resp = client
        .get(url)
        .header("Range", "bytes=0-1024")
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .ok()?;

    let status = resp.status();
    if !status.is_success() && status != 206 {
        return None;
    }

    let ct = resp.headers().get("content-type")?.to_str().ok()?;
    if ct.starts_with("video/")
        || ct.starts_with("application/octet-stream")
        || ct.starts_with("application/x-mpegURL")
        || ct.starts_with("binary/")
        || ct.contains("mp4")
        || ct.contains("matroska")
        || ct.contains("webm")
    {
        Some(true)
    } else {
        None
    }
}

fn try_probe_head(client: &reqwest::blocking::Client, url: &str) -> Option<bool> {
    let resp = client
        .head(url)
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .ok()?;

    if resp.status().is_success() || resp.status() == 206 {
        Some(true)
    } else {
        None
    }
}

fn try_probe_get_status(client: &reqwest::blocking::Client, url: &str) -> Option<bool> {
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .ok()?;

    if resp.status().is_success() || resp.status() == 206 {
        Some(true)
    } else {
        None
    }
}

fn verify_single_url(url: &str) -> bool {
    let client = crate::http_client::shared_client();

    if let Some(result) = try_probe_range_get(client, url) {
        return result;
    }

    if let Some(result) = try_probe_head(client, url) {
        return result;
    }

    if let Some(result) = try_probe_get_status(client, url) {
        return result;
    }

    false
}

pub fn verify_streams(urls: &[String]) -> Vec<StreamVerification> {
    use std::sync::{Arc, Mutex};
    use std::thread;

    let results = Arc::new(Mutex::new(Vec::with_capacity(urls.len())));
    let mut handles = Vec::with_capacity(urls.len());

    for url in urls {
        let url = url.clone();
        let results = Arc::clone(&results);

        handles.push(thread::spawn(move || {
            let active = verify_single_url(&url);
            let mut res = results.lock().expect("verify_streams lock");
            res.push(StreamVerification { url, active });
        }));
    }

    for handle in handles {
        let _ = handle.join();
    }

    Arc::try_unwrap(results)
        .expect("verify_streams arc unwrap")
        .into_inner()
        .expect("verify_streams mutex into_inner")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_quality ──

    #[test]
    fn parse_quality_4k_lowercase() {
        assert_eq!(parse_quality("Movie 4k"), "4K");
    }

    #[test]
    fn parse_quality_4k_uppercase() {
        assert_eq!(parse_quality("Movie 4K HDR"), "4K");
    }

    #[test]
    fn parse_quality_2160p() {
        assert_eq!(parse_quality("Movie.2160p.BluRay"), "4K");
    }

    #[test]
    fn parse_quality_1080p() {
        assert_eq!(parse_quality("Movie.1080p.WEB-DL"), "1080p");
    }

    #[test]
    fn parse_quality_1080p_uppercase() {
        assert_eq!(parse_quality("MOVIE 1080P RIP"), "1080p");
    }

    #[test]
    fn parse_quality_720p() {
        assert_eq!(parse_quality("Movie.720p.HDRip"), "720p");
    }

    #[test]
    fn parse_quality_480p() {
        assert_eq!(parse_quality("Movie.480p.DVDRip"), "480p");
    }

    #[test]
    fn parse_quality_unknown_empty() {
        assert_eq!(parse_quality(""), "Unknown");
    }

    #[test]
    fn parse_quality_unknown_no_resolution() {
        assert_eq!(parse_quality("Some Random Title"), "Unknown");
    }

    #[test]
    fn parse_quality_respects_priority_4k_over_1080p() {
        assert_eq!(parse_quality("Movie 4K 1080p"), "4K");
    }

    #[test]
    fn parse_quality_1080p_over_720p() {
        assert_eq!(parse_quality("Movie 1080p 720p"), "1080p");
    }

    #[test]
    fn parse_quality_2160p_over_1080p() {
        assert_eq!(parse_quality("Movie 2160p 1080p"), "4K");
    }

    // ── parse_source ──

    #[test]
    fn parse_source_4k_lowercase() {
        assert_eq!(parse_source("Movie 4k"), "Premium");
    }

    #[test]
    fn parse_source_4k_uppercase() {
        assert_eq!(parse_source("Movie 4K HDR"), "Premium");
    }

    #[test]
    fn parse_source_standard_1080p() {
        assert_eq!(parse_source("Movie 1080p BluRay"), "Standard");
    }

    #[test]
    fn parse_source_standard_720p() {
        assert_eq!(parse_source("Movie 720p WEB-DL"), "Standard");
    }

    #[test]
    fn parse_source_standard_empty() {
        assert_eq!(parse_source(""), "Standard");
    }

    #[test]
    fn parse_source_standard_bluray_no_4k() {
        assert_eq!(parse_source("Movie BluRay"), "Standard");
    }

    #[test]
    fn parse_source_4k_case_insensitive() {
        assert_eq!(parse_source("4K"), "Premium");
        assert_eq!(parse_source("4k"), "Premium");
    }

    // ── format_file_size ──

    #[test]
    fn format_file_size_zero() {
        assert_eq!(format_file_size(0), "0.00 B");
    }

    #[test]
    fn format_file_size_one_byte() {
        assert_eq!(format_file_size(1), "1.00 B");
    }

    #[test]
    fn format_file_size_exact_kb() {
        assert_eq!(format_file_size(1024), "1.00 KB");
    }

    #[test]
    fn format_file_size_exact_mb() {
        assert_eq!(format_file_size(1024 * 1024), "1.00 MB");
    }

    #[test]
    fn format_file_size_exact_gb() {
        assert_eq!(format_file_size(1024_i64 * 1024 * 1024), "1.00 GB");
    }

    #[test]
    fn format_file_size_exact_tb() {
        assert_eq!(format_file_size(1024_i64 * 1024 * 1024 * 1024), "1.00 TB");
    }

    #[test]
    fn format_file_size_fractional_mb() {
        let bytes = 1500 * 1024; // ~1.46 MB
        assert_eq!(format_file_size(bytes), "1.46 MB");
    }

    #[test]
    fn format_file_size_fractional_gb() {
        let bytes = (2.5 * 1024.0 * 1024.0 * 1024.0) as i64;
        assert_eq!(format_file_size(bytes), "2.50 GB");
    }

    #[test]
    fn format_file_size_large_tb() {
        // ~5 TB
        let bytes = 5_i64 * 1024 * 1024 * 1024 * 1024;
        assert_eq!(format_file_size(bytes), "5.00 TB");
    }

    #[test]
    fn format_file_size_clamps_at_tb() {
        // Larger than TB still shows TB
        let bytes = 1024_i64 * 1024 * 1024 * 1024 * 1024; // 1 PB in bytes
        assert!(format_file_size(bytes).ends_with("TB"));
    }

    #[test]
    fn format_file_size_small_bytes() {
        assert_eq!(format_file_size(100), "100.00 B");
        assert_eq!(format_file_size(512), "512.00 B");
    }

    // ── StreamVerification struct ──

    #[test]
    fn stream_verification_construction() {
        let sv = StreamVerification {
            url: "https://example.com/video.mp4".to_string(),
            active: true,
        };
        assert_eq!(sv.url, "https://example.com/video.mp4");
        assert!(sv.active);
    }

    #[test]
    fn stream_verification_inactive() {
        let sv = StreamVerification {
            url: "https://dead-link.example".to_string(),
            active: false,
        };
        assert!(!sv.active);
    }

    #[test]
    fn stream_verification_clone() {
        let sv = StreamVerification {
            url: "https://example.com".to_string(),
            active: true,
        };
        let sv2 = sv.clone();
        assert_eq!(sv.url, sv2.url);
        assert_eq!(sv.active, sv2.active);
    }

    #[test]
    fn stream_verification_serialize_roundtrip() {
        let sv = StreamVerification {
            url: "https://example.com/v.mp4".to_string(),
            active: true,
        };
        let json = serde_json::to_string(&sv).unwrap();
        let deserialized: StreamVerification = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.url, sv.url);
        assert_eq!(deserialized.active, sv.active);
    }

    // ── GroupedStreams struct ──

    #[test]
    fn grouped_streams_construction() {
        let gs = GroupedStreams {
            quality: "1080p".to_string(),
            streams: vec![],
        };
        assert_eq!(gs.quality, "1080p");
        assert!(gs.streams.is_empty());
    }

    #[test]
    fn grouped_streams_serialize_roundtrip() {
        let gs = GroupedStreams {
            quality: "4K".to_string(),
            streams: vec![RemoteStream {
                name: "Test".to_string(),
                description: "desc".to_string(),
                url: "https://example.com".to_string(),
                video_size: 1024,
                not_web_ready: false,
                parsed_quality: "4K".to_string(),
                parsed_source: "Premium".to_string(),
                recommended: true,
                is_hubdrive: false,
                episode_number: None,
            }],
        };
        let json = serde_json::to_string(&gs).unwrap();
        let deserialized: GroupedStreams = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.quality, "4K");
        assert_eq!(deserialized.streams.len(), 1);
    }

    // ── group_streams ──

    fn make_stream(quality: &str, video_size: i64, recommended: bool) -> RemoteStream {
        RemoteStream {
            name: format!("{} Stream", quality),
            description: String::new(),
            url: "https://example.com".to_string(),
            video_size,
            not_web_ready: false,
            parsed_quality: quality.to_string(),
            parsed_source: "Standard".to_string(),
            recommended,
            is_hubdrive: false,
            episode_number: None,
        }
    }

    #[test]
    fn group_streams_empty() {
        let result = group_streams(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn group_streams_single() {
        let streams = vec![make_stream("1080p", 1000, false)];
        let result = group_streams(streams);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].quality, "1080p");
        assert_eq!(result[0].streams.len(), 1);
    }

    #[test]
    fn group_streams_same_quality() {
        let streams = vec![
            make_stream("1080p", 100, false),
            make_stream("1080p", 200, false),
            make_stream("1080p", 300, false),
        ];
        let result = group_streams(streams);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].quality, "1080p");
        assert_eq!(result[0].streams.len(), 3);
    }

    #[test]
    fn group_streams_different_qualities() {
        let streams = vec![
            make_stream("720p", 100, false),
            make_stream("1080p", 200, false),
            make_stream("4K", 300, false),
        ];
        let result = group_streams(streams);
        assert_eq!(result.len(), 3);
        // 4K should be first (best quality)
        assert_eq!(result[0].quality, "4K");
        assert_eq!(result[1].quality, "1080p");
        assert_eq!(result[2].quality, "720p");
    }

    #[test]
    fn group_streams_recommended_floats_to_top() {
        let streams = vec![
            make_stream("720p", 100, false),
            make_stream("1080p", 200, true), // recommended
        ];
        let result = group_streams(streams);
        assert_eq!(result[0].quality, "1080p");
        assert_eq!(result[1].quality, "720p");
    }

    #[test]
    fn group_streams_recommended_sorts_before_size() {
        let streams = vec![
            make_stream("1080p", 500, false),
            make_stream("1080p", 100, true), // recommended but smaller
        ];
        let result = group_streams(streams);
        assert_eq!(result[0].streams.len(), 2);
        assert!(result[0].streams[0].recommended);
        assert!(!result[0].streams[1].recommended);
    }

    #[test]
    fn group_streams_within_group_sorted_by_size_desc() {
        let streams = vec![
            make_stream("1080p", 100, false),
            make_stream("1080p", 500, false),
            make_stream("1080p", 300, false),
        ];
        let result = group_streams(streams);
        assert_eq!(result[0].streams[0].video_size, 500);
        assert_eq!(result[0].streams[1].video_size, 300);
        assert_eq!(result[0].streams[2].video_size, 100);
    }

    #[test]
    fn group_streams_mixed_qualities_ordered_correctly() {
        let streams = vec![
            make_stream("480p", 100, false),
            make_stream("4K", 500, false),
            make_stream("Unknown", 200, false),
            make_stream("1080p", 300, false),
            make_stream("720p", 400, false),
        ];
        let result = group_streams(streams);
        let qualities: Vec<&str> = result.iter().map(|g| g.quality.as_str()).collect();
        assert_eq!(qualities, vec!["4K", "1080p", "720p", "480p", "Unknown"]);
    }

    #[test]
    fn group_streams_unknown_quality_ordered_last() {
        let streams = vec![
            make_stream("Unknown", 100, false),
            make_stream("1080p", 200, false),
        ];
        let result = group_streams(streams);
        assert_eq!(result[0].quality, "1080p");
        assert_eq!(result[1].quality, "Unknown");
    }

    // ── RemoteStream serde ──

    #[test]
    fn remote_stream_serde_defaults() {
        let json = r#"{"name":"test","description":"","url":"https://example.com"}"#;
        let s: RemoteStream = serde_json::from_str(json).unwrap();
        assert_eq!(s.video_size, 0);
        assert!(!s.not_web_ready);
        assert!(!s.recommended);
        assert!(s.episode_number.is_none());
    }

    #[test]
    fn remote_stream_serde_camel_case() {
        let json = r#"{"name":"test","description":"d","url":"u","videoSize":123,"notWebReady":true,"recommended":true}"#;
        let s: RemoteStream = serde_json::from_str(json).unwrap();
        assert_eq!(s.video_size, 123);
        assert!(s.not_web_ready);
        assert!(s.recommended);
    }

    #[test]
    fn remote_stream_skipped_fields_default() {
        let json = r#"{"name":"test","description":"d","url":"u"}"#;
        let s: RemoteStream = serde_json::from_str(json).unwrap();
        assert_eq!(s.parsed_quality, "");
        assert_eq!(s.parsed_source, "");
        assert!(!s.is_hubdrive);
    }

    // ── parse_streams_body ──

    #[test]
    fn parse_streams_body_valid_json() {
        let json = r#"{"streams":[{"name":"Movie.1080p","description":"desc","url":"https://example.com/video.mp4","behavior_hints":{"video_size":500,"not_web_ready":false}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].parsed_quality, "1080p");
        assert_eq!(streams[0].video_size, 500);
    }

    #[test]
    fn parse_streams_body_empty_streams_errors() {
        let json = r#"{"streams":[]}"#;
        let result = parse_streams_body(json);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "No streams available for this content");
    }

    #[test]
    fn parse_streams_body_invalid_json_errors() {
        let result = parse_streams_body("not json");
        assert!(result.is_err());
        assert!(result.unwrap_err().starts_with("Failed to parse response:"));
    }

    #[test]
    fn parse_streams_body_extracts_episode_number() {
        // "S01 E05" with space so E05 is a separate token
        let json = r#"{"streams":[{"name":"Show.S01 E05.1080p","description":"","url":"https://example.com/video.mp4","behavior_hints":{"video_size":0,"not_web_ready":false}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams[0].episode_number, Some(5));
    }

    #[test]
    fn parse_streams_body_extracts_episode_from_description() {
        let json = r#"{"streams":[{"name":"Show.1080p","description":"Episode 12","url":"https://example.com/video.mp4","behavior_hints":{"video_size":0,"not_web_ready":false}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams[0].episode_number, Some(12));
    }

    #[test]
    fn parse_streams_body_hubdrive_url_flag() {
        let json = r#"{"streams":[{"name":"Movie.1080p","description":"","url":"https://hubdrive.site/file/abc","behavior_hints":{"video_size":0,"not_web_ready":false}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert!(streams[0].is_hubdrive);
    }

    #[test]
    fn parse_streams_body_r2dev_recommended() {
        let json = r#"{"streams":[{"name":"Movie.1080p","description":"","url":"https://pub-xxx.r2.dev/video.mp4","behavior_hints":{"video_size":0,"not_web_ready":false}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert!(streams[0].recommended);
    }

    #[test]
    fn parse_streams_body_title_fallback_to_description() {
        let json = r#"{"streams":[{"name":"Movie.1080p","title":"My Title","url":"https://example.com/video.mp4","behavior_hints":{"video_size":0,"not_web_ready":false}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams[0].description, "My Title");
    }

    // ── extract_episode_number (via parse_streams_body) ──

    #[test]
    fn extract_episode_e_prefix() {
        let json = r#"{"streams":[{"name":"Show.E03.1080p","description":"","url":"u","behavior_hints":{}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams[0].episode_number, Some(3));
    }

    #[test]
    fn extract_episode_episode_word() {
        let json = r#"{"streams":[{"name":"Show.1080p","description":"Episode 7 of 12","url":"u","behavior_hints":{}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams[0].episode_number, Some(7));
    }

    #[test]
    fn extract_episode_none_when_absent() {
        let json = r#"{"streams":[{"name":"Movie.1080p","description":"A good movie","url":"u","behavior_hints":{}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams[0].episode_number, None);
    }

    #[test]
    fn extract_episode_from_e_pattern() {
        let json = r#"{"streams":[{"name":"Show.E15.720p","description":"","url":"u","behavior_hints":{}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams[0].episode_number, Some(15));
    }

    // ── behavior_hints defaults ──

    #[test]
    fn behavior_hints_defaults() {
        let json = r#"{"streams":[{"name":"M","description":"","url":"u"}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams[0].video_size, 0);
        assert!(!streams[0].not_web_ready);
    }

    #[test]
    fn behavior_hints_video_size_from_f64() {
        let json = r#"{"streams":[{"name":"M","description":"","url":"u","behavior_hints":{"video_size":1234567.0}}]}"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams[0].video_size, 1234567);
    }

    // ── fetch URL construction ──

    #[test]
    fn fetch_movie_streams_url_format() {
        // We can't easily test the fetch without a server, but we can verify
        // the function exists and has the right signature by calling with
        // a non-existent server (it will error on network)
        let result = fetch_movie_streams("tt1234567", "http://127.0.0.1:1", false);
        assert!(result.is_err());
    }

    #[test]
    fn fetch_series_streams_url_format() {
        let result = fetch_series_streams("tt1234567", 1, 2, "http://127.0.0.1:1", false);
        assert!(result.is_err());
    }

    #[test]
    fn fetch_season_streams_url_format() {
        let result = fetch_season_streams("tt1234567", 1, "http://127.0.0.1:1", false);
        assert!(result.is_err());
    }
}
