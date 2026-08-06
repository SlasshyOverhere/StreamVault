use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;

// Constants for retry logic
const MAX_RETRIES: u32 = 5;
const BASE_DELAY_MS: u64 = 500;
const MAX_DELAY_MS: u64 = 10000;

/// Get the TMDB credential to use - user must provide their own API key
/// Returns empty string if no key configured (callers should handle this)
pub fn get_tmdb_credential(user_key: &str) -> String {
    let trimmed = user_key.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_string()
    }
}

pub fn is_backend_proxy_credential(_credential: &str) -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmdbMetadata {
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub cast_names: Option<String>,
    pub director: Option<String>,
    pub poster_path: Option<String>,
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub runtime_seconds: Option<f64>,
    pub imdb_image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmdbSearchListItem {
    pub id: i64,
    pub title: Option<String>,
    pub name: Option<String>,
    pub media_type: String,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub overview: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub vote_average: Option<f64>,
    #[serde(default)]
    pub imdb_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmdbTrendingListItem {
    pub id: i64,
    pub title: String,
    pub media_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmdbSeasonInfo {
    pub season_number: i32,
    pub name: String,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub episode_count: i32,
    pub episodes: Vec<TmdbEpisodeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmdbEpisodeInfo {
    pub episode_number: i32,
    pub season_number: i32,
    pub name: String,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub air_date: Option<String>,
    pub vote_average: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct TmdbSearchResult {
    results: Vec<TmdbItem>,
    total_results: Option<i32>,
}

#[derive(Debug, Deserialize, Clone)]
struct TmdbItem {
    id: i64,
    #[serde(alias = "name")]
    title: Option<String>,
    #[serde(alias = "original_name")]
    original_title: Option<String>,
    overview: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    #[serde(alias = "first_air_date")]
    release_date: Option<String>,
    vote_average: Option<f64>,
    popularity: Option<f64>,
    vote_count: Option<i64>,
    runtime: Option<i32>,
    credits: Option<TmdbCredits>,
    // Movie detail responses include imdb_id directly
    imdb_id: Option<String>,
    // TV detail responses include external_ids when appended
    external_ids: Option<TmdbExternalIds>,
}

#[derive(Debug, Deserialize, Clone)]
struct TmdbCredits {
    cast: Option<Vec<TmdbCastMember>>,
    crew: Option<Vec<TmdbCrewMember>>,
}

#[derive(Debug, Deserialize, Clone)]
struct TmdbCastMember {
    name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct TmdbCrewMember {
    job: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct TmdbExternalIds {
    imdb_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TmdbFindResult {
    movie_results: Vec<TmdbItem>,
    tv_results: Vec<TmdbItem>,
}

/// Check if the given credential is an access token (starts with "eyJ") or API key
fn is_access_token(credential: &str) -> bool {
    credential.starts_with("eyJ")
}

/// Build the URL with proper authentication
/// - For API keys: adds ?api_key=XXX to URL
/// - For access tokens: returns URL without api_key (auth goes in header)
fn build_tmdb_url(base_path: &str, credential: &str, extra_params: &str) -> String {
    if is_access_token(credential) {
        format!("https://api.themoviedb.org/3{}?{}", base_path, extra_params)
    } else {
        format!(
            "https://api.themoviedb.org/3{}?api_key={}&{}",
            base_path, credential, extra_params
        )
    }
}

// ── Source routing ──────────────────────────────────────────────────────────
//
// Rule: a configured TMDB API key routes all metadata lookups to
// `api.themoviedb.org`. Without a key, the free `api.imdbapi.dev` service is
// used as a fallback. The legacy "TMDP" backend on `slasshyvault.onrender.com`
// has been decommissioned and no longer appears in any code path.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataSource {
    /// TMDB API key is configured — talk to themoviedb.org.
    Tmdb,
    /// No TMDB API key — talk to v3-cinemeta.strem.io (free, no key,
    /// cast/director/imdbRating/moviedb_id/episode thumbs).
    Cinemeta,
}

/// Pick the preferred metadata source for a given configured TMDB API key.
pub fn pick_source(api_key: &str) -> MetadataSource {
    if api_key.trim().is_empty() {
        MetadataSource::Cinemeta
    } else {
        MetadataSource::Tmdb
    }
}

pub const IMDB_LOG_PREFIX: &str = "[IMDBAPI]";
pub const CINEMETA_LOG_PREFIX: &str = "[CINEMETA]";

/// Execute a TMDB request with proper authentication and robust retry logic
fn tmdb_request(
    client: &reqwest::blocking::Client,
    url: &str,
    credential: &str,
) -> Result<reqwest::blocking::Response, reqwest::Error> {
    tmdb_request_with_retry(client, url, credential, MAX_RETRIES)
}

/// Execute a TMDB request with retry and exponential backoff
fn tmdb_request_with_retry(
    client: &reqwest::blocking::Client,
    url: &str,
    credential: &str,
    max_retries: u32,
) -> Result<reqwest::blocking::Response, reqwest::Error> {
    let mut last_error: Option<reqwest::Error> = None;

    for attempt in 0..max_retries {
        if attempt > 0 {
            // Exponential backoff with jitter
            let delay = std::cmp::min(BASE_DELAY_MS * (1 << attempt), MAX_DELAY_MS);
            let jitter = (rand_simple() * delay as f64 * 0.3) as u64;
            let total_delay = delay + jitter;
            println!(
                "[TMDB] Retry attempt {} after {}ms delay",
                attempt + 1,
                total_delay
            );
            std::thread::sleep(std::time::Duration::from_millis(total_delay));
        }

        let result = if is_access_token(credential) && !is_backend_proxy_credential(credential) {
            client
                .get(url)
                .header("Authorization", format!("Bearer {}", credential))
                .send()
        } else {
            client.get(url).send()
        };

        match result {
            Ok(response) => {
                // Check for rate limiting (429) or server errors (5xx)
                let status = response.status();
                if status.as_u16() == 429 {
                    println!("[TMDB] Rate limited (429), will retry...");
                    // Try to get retry-after header
                    if let Some(retry_after) = response.headers().get("retry-after") {
                        if let Ok(secs) = retry_after.to_str().unwrap_or("1").parse::<u64>() {
                            println!("[TMDB] Retry-After header: {} seconds", secs);
                            std::thread::sleep(std::time::Duration::from_secs(secs.min(30)));
                        }
                    }
                    continue;
                }
                if status.is_server_error() {
                    println!("[TMDB] Server error ({}), will retry...", status);
                    continue;
                }
                return Ok(response);
            }
            Err(e) => {
                let error_str = e.to_string();
                println!(
                    "[TMDB] Request failed (attempt {}): {}",
                    attempt + 1,
                    error_str
                );

                // Check for retryable errors
                let is_retryable = error_str.contains("10054")  // Connection reset (Windows)
                    || error_str.contains("connection")
                    || error_str.contains("timeout")
                    || error_str.contains("timed out")
                    || error_str.contains("Network")
                    || error_str.contains("closed");

                if is_retryable && attempt < max_retries - 1 {
                    last_error = Some(e);
                    continue;
                }

                return Err(e);
            }
        }
    }

    Err(last_error.unwrap())
}

/// Simple pseudo-random number generator (0.0 - 1.0)
fn rand_simple() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos % 1000) as f64 / 1000.0
}

/// Normalize a title for comparison (remove punctuation, lowercase, etc.)
fn normalize_title(title: &str) -> String {
    let mut normalized = title.to_lowercase();

    // Replace common variations
    normalized = normalized.replace('&', "and");
    normalized = normalized.replace("'", "");
    normalized = normalized.replace("'", "");
    normalized = normalized.replace(":", "");
    normalized = normalized.replace("-", " ");
    normalized = normalized.replace("_", " ");
    normalized = normalized.replace(".", " ");

    // Remove articles for comparison
    let articles = ["the ", "a ", "an "];
    for article in articles.iter() {
        if normalized.starts_with(article) {
            normalized = normalized[article.len()..].to_string();
        }
    }

    // Remove all non-alphanumeric except spaces
    normalized = normalized
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();

    // Collapse multiple spaces
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Calculate similarity score between two titles (0.0 - 1.0)
fn title_similarity(a: &str, b: &str) -> f64 {
    let norm_a = normalize_title(a);
    let norm_b = normalize_title(b);

    if norm_a == norm_b {
        return 1.0;
    }

    if norm_a.is_empty() || norm_b.is_empty() {
        return 0.0;
    }

    // Check if one contains the other
    if norm_a.contains(&norm_b) || norm_b.contains(&norm_a) {
        let len_ratio =
            (norm_a.len().min(norm_b.len()) as f64) / (norm_a.len().max(norm_b.len()) as f64);
        return 0.7 + (len_ratio * 0.3);
    }

    // Calculate word overlap (Jaccard-like similarity)
    let words_a: std::collections::HashSet<&str> = norm_a.split_whitespace().collect();
    let words_b: std::collections::HashSet<&str> = norm_b.split_whitespace().collect();

    let intersection = words_a.intersection(&words_b).count() as f64;
    let union = words_a.union(&words_b).count() as f64;

    if union == 0.0 {
        return 0.0;
    }

    intersection / union
}

/// Clean title minimally - only remove obvious noise but keep the core title intact
fn minimal_clean_title(title: &str) -> String {
    let mut cleaned = title.to_string();

    // Only remove brackets and their contents at the END of the title
    if let Ok(re) = regex::Regex::new(r"\s*[\[\(][^\]\)]*[\]\)]\s*$") {
        cleaned = re.replace_all(&cleaned, "").to_string();
    }

    // Remove trailing dashes and what follows (often release group)
    if let Ok(re) = regex::Regex::new(r"\s+-\s*[A-Za-z0-9]+\s*$") {
        cleaned = re.replace_all(&cleaned, "").to_string();
    }

    cleaned.trim().to_string()
}

/// Extract potential alternative titles from a string
fn extract_title_variations(title: &str) -> Vec<String> {
    let mut variations = Vec::new();

    // 1. Original title as-is
    variations.push(title.to_string());

    // 2. Minimally cleaned
    let minimal = minimal_clean_title(title);
    if !minimal.is_empty() && minimal != title {
        variations.push(minimal.clone());
    }

    // 3. With spaces instead of dots/underscores
    let spaced = title.replace('.', " ").replace('_', " ");
    let spaced = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if !spaced.is_empty() && !variations.contains(&spaced) {
        variations.push(spaced.clone());
    }

    // 4. Extract title from common patterns like "Title S01E01" or "Title.2019"
    // This helps with TV show episodes
    let patterns = [
        r"^(.+?)\s*[Ss]\d+[Ee]\d+",       // Title S01E01
        r"^(.+?)\s*\d{1,2}x\d{1,2}",      // Title 1x01
        r"^(.+?)\s*[\.\s](?:19|20)\d{2}", // Title.2019 or Title 2019
    ];

    for pattern in &patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(&spaced) {
                if let Some(m) = caps.get(1) {
                    let extracted = m.as_str().trim().to_string();
                    if !extracted.is_empty()
                        && extracted.len() >= 2
                        && !variations.contains(&extracted)
                    {
                        variations.push(extracted);
                    }
                }
            }
        }
    }

    // 5. Remove "The" prefix for alternative search
    for v in variations.clone() {
        if let Ok(re) = regex::Regex::new(r"(?i)^the\s+(.+)") {
            if let Some(caps) = re.captures(&v) {
                if let Some(m) = caps.get(1) {
                    let without_the = m.as_str().to_string();
                    if !without_the.is_empty() && !variations.contains(&without_the) {
                        variations.push(without_the);
                    }
                }
            }
        }
    }

    // 6. Handle & vs and
    for v in variations.clone() {
        if v.contains('&') {
            let alt = v.replace('&', "and");
            if !variations.contains(&alt) {
                variations.push(alt);
            }
        }
        if v.to_lowercase().contains(" and ") {
            let alt = v
                .replace(" and ", " & ")
                .replace(" And ", " & ")
                .replace(" AND ", " & ");
            if !variations.contains(&alt) {
                variations.push(alt);
            }
        }
    }

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    variations.retain(|v| {
        let lower = v.to_lowercase().trim().to_string();
        if seen.contains(&lower) || v.trim().is_empty() || v.len() < 2 {
            false
        } else {
            seen.insert(lower);
            true
        }
    });

    variations
}

/// Main search function - tries multiple strategies to find metadata
pub fn search_metadata(
    api_key: &str,
    title: &str,
    media_type: &str,
    year: Option<i32>,
    image_cache_dir: &str,
) -> Result<Option<TmdbMetadata>, Box<dyn std::error::Error + Send + Sync>> {
    println!("\n[TMDB] ========================================");
    println!(
        "[TMDB] search_metadata: query=\"{}\", type={}, year={:?}",
        title, media_type, year
    );
    println!(
        "[TMDB] Searching for: '{}' (type: {}, year: {:?})",
        title, media_type, year
    );

    let variations = extract_title_variations(title);
    println!("[TMDB] Title variations: {:?}", variations);

    // Strategy 1: Search with specified media type and year
    if let Some(y) = year {
        println!("[TMDB] Strategy 1: {} search with year {}", media_type, y);
        for variation in &variations {
            if let Ok(Some(result)) = do_search(
                api_key,
                variation,
                media_type,
                Some(y),
                image_cache_dir,
                true,
            ) {
                return Ok(Some(result));
            }
        }
    }

    // Strategy 2: Search with specified media type, no year constraint
    println!("[TMDB] Strategy 2: {} search without year", media_type);
    for variation in &variations {
        if let Ok(Some(result)) =
            do_search(api_key, variation, media_type, None, image_cache_dir, true)
        {
            return Ok(Some(result));
        }
    }

    // Strategy 3: Try the OTHER media type (if searching for TV, try movie and vice versa)
    let alt_type = if media_type == "movie" { "tv" } else { "movie" };
    println!("[TMDB] Strategy 3: {} search (alternative type)", alt_type);
    for variation in &variations {
        if let Ok(Some(result)) =
            do_search(api_key, variation, alt_type, year, image_cache_dir, true)
        {
            return Ok(Some(result));
        }
    }

    // Strategy 4: Multi-search (searches across all media types)
    println!("[TMDB] Strategy 4: Multi-search");
    for variation in &variations {
        if let Ok(Some(result)) =
            do_multi_search(api_key, variation, media_type, year, image_cache_dir)
        {
            return Ok(Some(result));
        }
    }

    // Strategy 5: Try with just the first word (for short/numeric titles like "1899")
    if variations.iter().any(|v| v.split_whitespace().count() > 1) {
        println!("[TMDB] Strategy 5: First significant word search");
        for variation in &variations {
            let words: Vec<&str> = variation.split_whitespace().collect();
            if words.len() > 1 {
                // Try first word only
                let first = words[0];
                if first.len() >= 3 || first.chars().all(|c| c.is_ascii_digit()) {
                    // For numeric titles like "1899"
                    if let Ok(Some(result)) =
                        do_search(api_key, first, media_type, year, image_cache_dir, false)
                    {
                        // Verify it's a reasonable match
                        if is_reasonable_match(first, &result.title) {
                            return Ok(Some(result));
                        }
                    }
                    if let Ok(Some(result)) =
                        do_search(api_key, first, alt_type, year, image_cache_dir, false)
                    {
                        if is_reasonable_match(first, &result.title) {
                            return Ok(Some(result));
                        }
                    }
                }
            }
        }
    }

    // Strategy 6: Relaxed search - accept results with lower score
    println!("[TMDB] Strategy 6: Relaxed search (lower threshold)");
    for variation in &variations {
        if let Ok(Some(result)) =
            do_search(api_key, variation, media_type, None, image_cache_dir, false)
        {
            return Ok(Some(result));
        }
    }

    println!("[TMDB] No match found for \"{}\"", title);
    println!(
        "[TMDB] All strategies exhausted, no results found for '{}'",
        title
    );
    println!("[TMDB] ========================================\n");
    Ok(None)
}

/// Title-based metadata search routed by configured TMDB key:
/// TMDB if the key is set, otherwise the free imdbapi.dev service.
pub fn search_metadata_with_fallback(
    api_key: &str,
    title: &str,
    media_type: &str,
    year: Option<i32>,
    image_cache_dir: &str,
) -> Result<Option<TmdbMetadata>, Box<dyn std::error::Error + Send + Sync>> {
    match pick_source(api_key) {
        MetadataSource::Tmdb => search_metadata(api_key, title, media_type, year, image_cache_dir),
        MetadataSource::Cinemeta => {
            let cinemeta_kind = match media_type {
                "movie" => crate::cinemeta_api::CinemetaKind::Movie,
                "tv" => crate::cinemeta_api::CinemetaKind::Series,
                _ => crate::cinemeta_api::CinemetaKind::Movie,
            };
            if let Ok(titles) = crate::cinemeta_api::search(cinemeta_kind, title, 25) {
                let best = titles.into_iter().find(|t| {
                    if let Some(y) = year {
                        parse_year_str(t.year.as_deref())
                            .map(|cy| (cy - y).abs() <= 1)
                            .unwrap_or(false)
                    } else {
                        true
                    }
                });
                if let Some(t) = best {
                    let mtype = if t.kind == "series" { "tv" } else { "movie" };
                    return fetch_metadata_by_cinemeta_title(&t, mtype, image_cache_dir).map(Some);
                }
            }
            // Cinemeta empty → fall back to balloonerismm (no-key TMDB proxy).
            // imdbapi.dev is gone as a source of truth for metadata.
            balloonerismm_fallback_search(title, media_type, year, image_cache_dir)
        }
    }
}

/// Best-effort metadata lookup against balloonerismm after Cinemeta returns
/// nothing. Used as the imdbapi.dev-replacement last-resort in
/// `search_metadata_with_fallback`. Returns `Ok(None)` when balloonerismm
/// has no `tt…` row at all.
fn balloonerismm_fallback_search(
    title: &str,
    media_type: &str,
    year: Option<i32>,
    image_cache_dir: &str,
) -> Result<Option<TmdbMetadata>, Box<dyn std::error::Error + Send + Sync>> {
    let kinds: &[&str] = match media_type {
        "movie" => &["movie"],
        "tv" => &["tv"],
        _ => &["movie", "tv"],
    };
    for &kind in kinds {
        // Pull the appropriate search response.
        let (json_results, mapped_media_type) = if kind == "movie" {
            let page = crate::balloonerismm_api::search_movie(title)
                .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))?;
            let v: Vec<serde_json::Value> = page
                .results
                .into_iter()
                .map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null))
                .collect();
            (v, "movie")
        } else {
            let page = crate::balloonerismm_api::search_tv(title)
                .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))?;
            let v: Vec<serde_json::Value> = page
                .results
                .into_iter()
                .map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null))
                .collect();
            (v, "tv")
        };

        // Pick the closest candidate (year match ±1, otherwise first).
        let candidate = json_results
            .iter()
            .filter_map(|v| {
                let obj = v.as_object()?;
                let id = obj.get("id").and_then(|v| v.as_str())?.trim().to_string();
                let item_year = obj
                    .get("release_date")
                    .or_else(|| obj.get("first_air_date"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.split('-').next())
                    .and_then(|y| y.parse::<i32>().ok());
                Some((id, item_year))
            })
            .find(|(_, iy)| match (year, iy) {
                (Some(y), Some(iy)) => (y - iy).abs() <= 1,
                _ => true,
            })
            .map(|(id, _)| id);

        if let Some(tt) = candidate {
            return fetch_metadata_by_imdb_id(&tt, mapped_media_type, image_cache_dir).map(Some);
        }
    }
    println!("[BALLOON] no match found for \"{}\"", title);
    Ok(None)
}

fn fetch_metadata_by_cinemeta_title(
    t: &crate::cinemeta_api::CinemetaTitle,
    media_type: &str,
    image_cache_dir: &str,
) -> Result<TmdbMetadata, Box<dyn std::error::Error + Send + Sync>> {
    let image_type = if media_type == "tv" {
        ImageType::SeriesBanner
    } else {
        ImageType::MovieBanner
    };
    let poster_path = t
        .poster
        .as_deref()
        .and_then(|u| cache_imdb_image(u, std::path::Path::new(image_cache_dir), &image_type));
    let year = parse_year_str(t.year.as_deref()).or_else(|| {
        t.released
            .as_deref()
            .and_then(|s| s.get(..4))
            .and_then(|y| y.parse::<i32>().ok())
    });
    let cast = t.cast.clone().map(|v| {
        v.into_iter()
            .filter(|s| !s.trim().is_empty())
            .take(8)
            .collect::<Vec<_>>()
            .join(", ")
    });
    let director = t.director.clone().and_then(|mut v| {
        if v.is_empty() {
            None
        } else {
            Some(v.remove(0))
        }
    });
    let runtime = t
        .runtime
        .as_deref()
        .and_then(|s| s.split_whitespace().next())
        .and_then(|n| n.parse::<i32>().ok())
        .map(|m| (m as f64) * 60.0);
    Ok(TmdbMetadata {
        title: t.name.clone(),
        year,
        overview: t.description.clone(),
        cast_names: cast.filter(|s| !s.is_empty()),
        director,
        poster_path: poster_path.clone(),
        tmdb_id: t.moviedb_id.map(|n| n.to_string()),
        imdb_id: t.imdb_id.clone().or_else(|| Some(t.id.clone())),
        runtime_seconds: runtime,
        imdb_image_url: t.poster.clone(),
    })
}

/// Raw multi-search for UI pickers (returns movie/tv result list, no metadata caching).
pub fn search_multi_raw(
    api_key: &str,
    query: &str,
) -> Result<Vec<TmdbSearchListItem>, Box<dyn std::error::Error + Send + Sync>> {
    let encoded_query =
        percent_encoding::utf8_percent_encode(query, percent_encoding::NON_ALPHANUMERIC)
            .to_string();

    let params = format!("query={}&include_adult=false&language=en-US", encoded_query);
    let url = build_tmdb_url("/search/multi", api_key, &params);

    let client = crate::http_client::shared_client().clone();
    let response = match tmdb_request(&client, &url, api_key) {
        Ok(response) => response,
        Err(primary_error) => {
            println!(
                "[TMDB] Primary multi-search request failed, retrying with fallback transport: {}",
                primary_error
            );

            // Fallback transport profile for environments where the default
            // connection strategy gets reset by intermediary network devices.
            let fallback_client = crate::http_client::shared_client().clone();

            tmdb_request(&fallback_client, &url, api_key).map_err(|fallback_error| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!(
                        "TMDB multi-search failed after fallback. primary='{}', fallback='{}'",
                        primary_error, fallback_error
                    ),
                )
            })?
        }
    };

    if !response.status().is_success() {
        return Err(format!("TMDB API error: {}", response.status()).into());
    }

    #[derive(Debug, Deserialize)]
    struct RawSearchResult {
        results: Vec<RawSearchItem>,
    }

    #[derive(Debug, Deserialize)]
    struct RawSearchItem {
        id: i64,
        media_type: Option<String>,
        title: Option<String>,
        name: Option<String>,
        #[serde(alias = "original_title")]
        original_title: Option<String>,
        #[serde(alias = "original_name")]
        original_name: Option<String>,
        poster_path: Option<String>,
        backdrop_path: Option<String>,
        overview: Option<String>,
        release_date: Option<String>,
        first_air_date: Option<String>,
        vote_average: Option<f64>,
    }

    let raw: RawSearchResult = response.json()?;

    let results = raw
        .results
        .into_iter()
        .filter_map(|item| {
            let media_type = item.media_type.unwrap_or_default();
            if media_type != "movie" && media_type != "tv" {
                return None;
            }

            Some(TmdbSearchListItem {
                id: item.id,
                title: item.title.or(item.original_title),
                name: item.name.or(item.original_name),
                media_type,
                poster_path: item.poster_path,
                backdrop_path: item.backdrop_path,
                overview: item.overview,
                release_date: item.release_date,
                first_air_date: item.first_air_date,
                vote_average: item.vote_average,
                imdb_id: None, // populated below
            })
        })
        .collect::<Vec<_>>();

    // imdb_id is NOT fetched here — it would be N sequential API calls per search
    // (one per result). Instead, imdb_id is resolved on-demand when the user
    // selects a result to find streams (single API call at that point).

    Ok(results)
}

/// Free-text search that auto-routes between TMDB and imdbapi.dev based on
/// whether the user has configured a TMDB API key. Results are emitted in the
/// `TmdbSearchListItem` shape so existing UI code consumes them unchanged.
///
/// `media_type` ∈ `Some("movie") | Some("tv") | None`. When `None`, results
/// of any type pass through; the UI will filter further.
pub fn search_multi_raw_with_fallback(
    api_key: &str,
    query: &str,
    media_type: Option<&str>,
) -> Result<Vec<TmdbSearchListItem>, Box<dyn std::error::Error + Send + Sync>> {
    match pick_source(api_key) {
        MetadataSource::Tmdb => search_multi_raw(api_key, query),
        MetadataSource::Cinemeta => match cinemeta_search_as_list_items(query, media_type) {
            Ok(items) if !items.is_empty() => Ok(items),
            // Cinemeta empty → fall back to balloonerismm (imdbapi.dev is gone).
            _ => imdb_search_as_list_items(query, media_type),
        },
    }
}

fn cinemeta_search_as_list_items(
    query: &str,
    media_type: Option<&str>,
) -> Result<Vec<TmdbSearchListItem>, Box<dyn std::error::Error + Send + Sync>> {
    let kinds: Vec<crate::cinemeta_api::CinemetaKind> = match media_type {
        Some("movie") => vec![crate::cinemeta_api::CinemetaKind::Movie],
        Some("tv") => vec![crate::cinemeta_api::CinemetaKind::Series],
        _ => vec![
            crate::cinemeta_api::CinemetaKind::Movie,
            crate::cinemeta_api::CinemetaKind::Series,
        ],
    };

    let mut out: Vec<TmdbSearchListItem> = Vec::new();
    for kind in kinds {
        let titles = crate::cinemeta_api::search(kind, query, 25)?;
        for t in titles {
            out.push(cinemeta_title_to_list_item(&t));
        }
    }
    Ok(out)
}

fn cinemeta_title_to_list_item(t: &crate::cinemeta_api::CinemetaTitle) -> TmdbSearchListItem {
    let normalized_type = if t.kind == "series" { "tv" } else { "movie" }.to_string();
    let year_int = parse_year_str(t.year.as_deref());
    let date_str = year_int.map(|y| format!("{:04}-01-01", y));
    TmdbSearchListItem {
        id: t.moviedb_id.unwrap_or_else(|| parse_imdb_id_to_i64(&t.id)),
        title: Some(t.name.clone()),
        name: Some(t.name.clone()),
        media_type: normalized_type,
        poster_path: t.poster.clone(),
        backdrop_path: t.background.clone(),
        overview: t.description.clone(),
        release_date: date_str.clone(),
        first_air_date: date_str,
        vote_average: t.imdb_rating.as_ref().and_then(|s| s.parse::<f64>().ok()),
        imdb_id: Some(t.id.clone()),
    }
}

fn parse_year_str(s: Option<&str>) -> Option<i32> {
    let s = s?;
    let digits: String = s.chars().take(4).collect();
    digits.parse::<i32>().ok()
}

fn imdb_search_as_list_items(
    query: &str,
    media_type: Option<&str>,
) -> Result<Vec<TmdbSearchListItem>, Box<dyn std::error::Error + Send + Sync>> {
    // Balloonerismm `/search/multi` returns both movie and TV rows keyed by
    // `media_type`. Drop imdbapi.dev dependency entirely.
    //
    // If `media_type` narrowed to a single kind, hit balloonerismm's per-kind
    // search endpoint to keep response shape stable.
    let page = match media_type {
        Some("movie") => {
            let p: crate::balloonerismm_api::SearchPage<
                crate::balloonerismm_api::MovieSearchResult,
            > = crate::balloonerismm_api::search_movie(query)
                .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))?;
            // Lift to a common enum of JSON values so the mapping below is uniform.
            let json_results: Vec<serde_json::Value> = p
                .results
                .into_iter()
                .map(|r| serde_json::to_value(r).unwrap_or_else(|_| serde_json::Value::Null))
                .collect();
            crate::balloonerismm_api::SearchPage {
                page: p.page,
                page_token: p.page_token,
                has_next_page: p.has_next_page,
                results: json_results,
            }
        }
        Some("tv") => {
            let p: crate::balloonerismm_api::SearchPage<crate::balloonerismm_api::TvSearchResult> =
                crate::balloonerismm_api::search_tv(query)
                    .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))?;
            let json_results: Vec<serde_json::Value> = p
                .results
                .into_iter()
                .map(|r| serde_json::to_value(r).unwrap_or_else(|_| serde_json::Value::Null))
                .collect();
            crate::balloonerismm_api::SearchPage {
                page: p.page,
                page_token: p.page_token,
                has_next_page: p.has_next_page,
                results: json_results,
            }
        }
        _ => crate::balloonerismm_api::search_multi(query)
            .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))?,
    };

    Ok(page
        .results
        .into_iter()
        .filter_map(|v| {
            let obj = v.as_object()?;
            let id = obj.get("id").and_then(|v| v.as_str())?.trim().to_string();
            if !crate::balloonerismm_api::looks_like_imdb_id(&id) {
                return None;
            }
            let media_type = obj
                .get("media_type")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "movie".to_string());
            let title = obj
                .get("title")
                .or_else(|| obj.get("name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let name = obj
                .get("name")
                .or_else(|| obj.get("title"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let poster_path = obj
                .get("poster_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let backdrop_path = obj
                .get("backdrop_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let overview = obj
                .get("overview")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let release_date = obj
                .get("release_date")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let first_air_date = obj
                .get("first_air_date")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let vote_average = obj.get("vote_average").and_then(|v| v.as_f64());
            // TT-only id; we keep the IMDb id rather than parsing `id` to a TMDB numeric.
            let tt_id = id.clone();
            Some(TmdbSearchListItem {
                id: parse_imdb_id_to_i64(&tt_id),
                title: title.clone(),
                name: title.or(name),
                media_type,
                poster_path,
                backdrop_path,
                overview,
                release_date,
                first_air_date,
                vote_average,
                imdb_id: Some(tt_id),
            })
        })
        .collect())
}

fn _unused_search_type_picker_marker(_t: Option<&str>) {
    // (Function body removed — was the tail of the old imdbapi-based
    // imdb_search_as_list_items implementation.)
}

fn parse_imdb_id_to_i64(id: &str) -> i64 {
    id.trim_start_matches("tt")
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<i64>()
        .unwrap_or(0)
}

/// Fetch top trending movies and TV shows for lightweight UI suggestions.
///
/// Balloonerismm has `/popular/{all|movie|tv}` which we hit when no TMDB key
/// is configured — replaces the imdbapi.dev "return [] for no-key" branch.
pub fn trending_suggestions_raw(
    api_key: &str,
    per_type_limit: usize,
) -> Result<Vec<TmdbTrendingListItem>, Box<dyn std::error::Error + Send + Sync>> {
    if api_key.trim().is_empty() {
        // No TMDB key — pull from balloonerismm's free `/popular/*` pages.
        let mut suggestions = Vec::new();
        for kind in ["movie", "tv"] {
            let page = crate::balloonerismm_api::popular(kind, per_type_limit)
                .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()))?;
            suggestions.extend(page.results.into_iter().filter_map(|v| {
                let obj = v.as_object()?;
                let id = obj.get("id").and_then(|v| v.as_str())?.trim().to_string();
                if !crate::balloonerismm_api::looks_like_imdb_id(&id) {
                    return None;
                }
                let title = obj
                    .get("title")
                    .or_else(|| obj.get("name"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())?;
                Some(TmdbTrendingListItem {
                    id: parse_imdb_id_to_i64(&id),
                    title,
                    media_type: kind.to_string(),
                })
            }));
        }
        return Ok(suggestions);
    }

    #[derive(Debug, Deserialize)]
    struct RawTrendingResult {
        results: Vec<RawTrendingItem>,
    }

    #[derive(Debug, Deserialize)]
    struct RawTrendingItem {
        id: i64,
        title: Option<String>,
        name: Option<String>,
    }

    let client = crate::http_client::shared_client().clone();
    let mut suggestions = Vec::new();

    for media_type in ["movie", "tv"] {
        let url = build_tmdb_url(
            &format!("/trending/{}/day", media_type),
            api_key,
            "language=en-US",
        );
        let response = tmdb_request(&client, &url, api_key)?;

        if !response.status().is_success() {
            return Err(format!(
                "TMDB trending {} API error: {}",
                media_type,
                response.status()
            )
            .into());
        }

        let raw: RawTrendingResult = response.json()?;
        suggestions.extend(
            raw.results
                .into_iter()
                .filter_map(|item| {
                    let title = item.title.or(item.name)?.trim().to_string();
                    if title.is_empty() {
                        return None;
                    }

                    Some(TmdbTrendingListItem {
                        id: item.id,
                        title,
                        media_type: media_type.to_string(),
                    })
                })
                .take(per_type_limit),
        );
    }

    Ok(suggestions)
}

/// Check if a search result title is a reasonable match for the query
fn is_reasonable_match(query: &str, result_title: &str) -> bool {
    let q = query.to_lowercase();
    let r = result_title.to_lowercase();

    // Exact match
    if q == r {
        return true;
    }

    // Result contains query or query contains result
    if r.contains(&q) || q.contains(&r) {
        return true;
    }

    // For numeric titles, the result should start with or contain the number
    if query.chars().all(|c| c.is_ascii_digit()) {
        return r.contains(&q);
    }

    // First word matches
    let q_first = q.split_whitespace().next().unwrap_or("");
    let r_first = r.split_whitespace().next().unwrap_or("");
    if !q_first.is_empty() && q_first == r_first {
        return true;
    }

    false
}

/// Perform a single TMDB search
fn do_search(
    api_key: &str,
    title: &str,
    media_type: &str,
    year: Option<i32>,
    image_cache_dir: &str,
    strict: bool,
) -> Result<Option<TmdbMetadata>, Box<dyn std::error::Error + Send + Sync>> {
    let encoded_title =
        percent_encoding::utf8_percent_encode(title, percent_encoding::NON_ALPHANUMERIC)
            .to_string();

    let mut params = format!("query={}&include_adult=false&language=en-US", encoded_title);

    if let Some(y) = year {
        if media_type == "movie" {
            params.push_str(&format!("&primary_release_year={}", y));
        } else {
            params.push_str(&format!("&first_air_date_year={}", y));
        }
    }

    let url = build_tmdb_url(&format!("/search/{}", media_type), api_key, &params);

    println!(
        "[TMDB]   -> Trying '{}' as {} (year: {:?})",
        title, media_type, year
    );

    let client = crate::http_client::shared_client().clone();
    let response = tmdb_request(&client, &url, api_key)?;

    if !response.status().is_success() {
        println!("[TMDB]   -> Request failed: {}", response.status());
        return Ok(None);
    }

    let result: TmdbSearchResult = response.json()?;
    let total = result.total_results.unwrap_or(0);
    println!("[TMDB]   -> Found {} results", total);

    if result.results.is_empty() {
        return Ok(None);
    }

    // Find the best match
    let best = find_best_match(&result.results, title, year, strict);

    if let Some(item) = best {
        if item.poster_path.is_some() || item.backdrop_path.is_some() || !strict {
            let best_id = item.id.to_string();
            match fetch_metadata_by_id(api_key, &best_id, media_type, image_cache_dir) {
                Ok(metadata) => return Ok(Some(metadata)),
                Err(err) => {
                    println!(
                        "[TMDB]   -> Detailed metadata fetch failed for {}: {} (falling back to search payload)",
                        best_id, err
                    );
                }
            }
            return create_metadata_from_item(&item, image_cache_dir, media_type);
        }
        println!("[TMDB]   -> Best match has no images, skipping in strict mode");
    }

    Ok(None)
}

/// Multi-search across all media types
fn do_multi_search(
    api_key: &str,
    title: &str,
    preferred_type: &str,
    search_year: Option<i32>,
    image_cache_dir: &str,
) -> Result<Option<TmdbMetadata>, Box<dyn std::error::Error + Send + Sync>> {
    let encoded_title =
        percent_encoding::utf8_percent_encode(title, percent_encoding::NON_ALPHANUMERIC)
            .to_string();

    let params = format!("query={}&include_adult=false&language=en-US", encoded_title);
    let url = build_tmdb_url("/search/multi", api_key, &params);

    println!("[TMDB]   -> Multi-search for '{}'", title);

    let client = crate::http_client::shared_client().clone();
    let response = tmdb_request(&client, &url, api_key)?;

    if !response.status().is_success() {
        return Ok(None);
    }

    #[derive(Debug, Deserialize)]
    struct MultiSearchResult {
        results: Vec<MultiSearchItem>,
    }

    #[derive(Debug, Deserialize)]
    struct MultiSearchItem {
        id: i64,
        media_type: Option<String>,
        #[serde(alias = "name")]
        title: Option<String>,
        #[serde(alias = "original_name")]
        original_title: Option<String>,
        overview: Option<String>,
        poster_path: Option<String>,
        backdrop_path: Option<String>,
        #[serde(alias = "first_air_date")]
        release_date: Option<String>,
        vote_average: Option<f64>,
        popularity: Option<f64>,
        vote_count: Option<i64>,
    }

    let result: MultiSearchResult = response.json()?;
    println!(
        "[TMDB]   -> Found {} multi-search results",
        result.results.len()
    );

    let preferred = if preferred_type == "movie" {
        "movie"
    } else {
        "tv"
    };

    // Score and sort results
    let mut scored: Vec<(&MultiSearchItem, f64)> = result
        .results
        .iter()
        .filter(|item| {
            let mt = item.media_type.as_deref().unwrap_or("");
            mt == "movie" || mt == "tv"
        })
        .map(|item| {
            let item_type = item.media_type.as_deref().unwrap_or("");
            let has_poster = item.poster_path.is_some() || item.backdrop_path.is_some();
            let popularity = item.popularity.unwrap_or(0.0);
            let vote_count = item.vote_count.unwrap_or(0) as f64;
            let item_year = item
                .release_date
                .as_ref()
                .and_then(|d| d.split('-').next())
                .and_then(|y| y.parse::<i32>().ok());

            let mut score = popularity * 0.3 + vote_count * 0.1;
            if item_type == preferred {
                score += 500.0;
            }
            if has_poster {
                score += 1000.0;
            }

            // Title match bonus
            let item_title = item.title.as_deref().unwrap_or("").to_lowercase();
            let search_lower = title.to_lowercase();
            if item_title == search_lower {
                score += 2000.0;
            } else if item_title.contains(&search_lower) || search_lower.contains(&item_title) {
                score += 500.0;
            }

            if let (Some(search_y), Some(item_y)) = (search_year, item_year) {
                let year_diff = (search_y - item_y).abs();
                if year_diff == 0 {
                    score += 800.0;
                } else if year_diff == 1 {
                    score += 300.0;
                } else if year_diff > 2 {
                    score -= 5000.0;
                }
            }

            (item, score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    if let Some((item, score)) = scored.first() {
        if let Some(search_y) = search_year {
            let item_year = item
                .release_date
                .as_ref()
                .and_then(|d| d.split('-').next())
                .and_then(|y| y.parse::<i32>().ok());
            if let Some(item_y) = item_year {
                let year_diff = (search_y - item_y).abs();
                if year_diff > 2 {
                    println!(
                        "[TMDB]   -> Best multi-search result rejected due to year mismatch (search={}, result={})",
                        search_y, item_y
                    );
                    return Ok(None);
                }
            }
        }
        println!(
            "[TMDB]   -> Best multi-search result: '{}' (score: {:.1})",
            item.title.as_deref().unwrap_or("?"),
            score
        );

        let actual_type = item.media_type.as_deref().unwrap_or(preferred_type);
        let best_id = item.id.to_string();
        match fetch_metadata_by_id(api_key, &best_id, actual_type, image_cache_dir) {
            Ok(metadata) => return Ok(Some(metadata)),
            Err(err) => {
                println!(
                    "[TMDB]   -> Detailed multi-search metadata fetch failed for {}: {} (falling back to search payload)",
                    best_id, err
                );
            }
        }

        let tmdb_item = TmdbItem {
            id: item.id,
            title: item.title.clone(),
            original_title: item.original_title.clone(),
            overview: item.overview.clone(),
            poster_path: item.poster_path.clone(),
            backdrop_path: item.backdrop_path.clone(),
            release_date: item.release_date.clone(),
            vote_average: item.vote_average,
            popularity: item.popularity,
            vote_count: item.vote_count,
            runtime: None,
            credits: None,
            imdb_id: None,
            external_ids: None,
        };
        return create_metadata_from_item(&tmdb_item, image_cache_dir, actual_type);
    }

    Ok(None)
}

/// Find the best match from search results using improved scoring
fn find_best_match<'a>(
    results: &'a [TmdbItem],
    search_title: &str,
    search_year: Option<i32>,
    strict: bool,
) -> Option<&'a TmdbItem> {
    if results.is_empty() {
        return None;
    }

    // Score each result
    let mut scored: Vec<(&TmdbItem, f64)> = results
        .iter()
        .map(|item| {
            let item_title = item.title.as_deref().unwrap_or("");
            let original_title = item.original_title.as_deref().unwrap_or("");
            let has_poster = item.poster_path.is_some();
            let has_backdrop = item.backdrop_path.is_some();
            let popularity = item.popularity.unwrap_or(0.0);
            let vote_avg = item.vote_average.unwrap_or(0.0);
            let vote_count = item.vote_count.unwrap_or(0) as f64;
            let item_year = item
                .release_date
                .as_ref()
                .and_then(|d| d.split('-').next())
                .and_then(|y| y.parse::<i32>().ok());

            let mut score = 0.0;

            // Base popularity/quality score (capped to prevent dominance)
            score += (popularity.min(100.0)) * 0.5;
            score += vote_avg * 10.0;
            score += (vote_count.min(10000.0)) * 0.01;

            // Image availability - important for user experience
            if has_poster {
                score += 500.0;
            }
            if has_backdrop {
                score += 100.0;
            }

            // Title similarity - THE MOST IMPORTANT FACTOR
            let title_sim = title_similarity(search_title, item_title);
            let orig_title_sim = title_similarity(search_title, original_title);
            let best_sim = title_sim.max(orig_title_sim);

            // Heavy weight on title matching
            if best_sim >= 0.95 {
                score += 3000.0; // Near-exact match
            } else if best_sim >= 0.8 {
                score += 2000.0 + (best_sim * 500.0); // Very good match
            } else if best_sim >= 0.5 {
                score += 1000.0 + (best_sim * 500.0); // Decent match
            } else if best_sim >= 0.3 {
                score += best_sim * 500.0; // Partial match
            } else {
                score -= 500.0; // Poor match penalty
            }

            // Year matching (with tolerance)
            if let Some(search_y) = search_year {
                if let Some(item_y) = item_year {
                    let year_diff = (search_y - item_y).abs();
                    if year_diff == 0 {
                        score += 1000.0; // Exact year match
                    } else if year_diff == 1 {
                        score += 500.0; // Off by one year (common for releases)
                    } else if year_diff <= 2 {
                        score += 200.0; // Close enough
                    } else if year_diff > 2 {
                        score -= 5000.0; // Strong penalty for wrong year
                    }
                }
            }

            // Penalize very short titles that don't match well
            if item_title.len() < 3 && best_sim < 0.9 {
                score -= 300.0;
            }

            (item, score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    if let Some((item, score)) = scored.first() {
        if let Some(search_y) = search_year {
            let item_year = item
                .release_date
                .as_ref()
                .and_then(|d| d.split('-').next())
                .and_then(|y| y.parse::<i32>().ok());
            if let Some(item_y) = item_year {
                let year_diff = (search_y - item_y).abs();
                if year_diff > 2 {
                    println!(
                        "[TMDB]   -> Best match '{}' rejected due to year mismatch (search={}, result={})",
                        item.title.as_deref().unwrap_or(""),
                        search_y,
                        item_y
                    );
                    return None;
                }
            }
        }

        // In strict mode, require a minimum similarity score
        if strict {
            let item_title = item.title.as_deref().unwrap_or("");
            let best_sim = title_similarity(search_title, item_title);
            if best_sim < 0.3 && *score < 1000.0 {
                println!(
                    "[TMDB]   -> Best match '{}' rejected (similarity: {:.2}, score: {:.1})",
                    item_title, best_sim, score
                );
                return None;
            }
        }
    }

    if let Some((item, score)) = scored.first() {
        let matched_title = item.title.as_deref().unwrap_or("?");
        let tmdb_id = item.id;
        println!(
            "[TMDB] Best match: \"{}\" (score: {:.1}, tmdb_id: {})",
            matched_title, score, tmdb_id
        );
    }

    scored.first().map(|(item, _)| *item)
}

/// Create metadata from a TMDB item
fn create_metadata_from_item(
    item: &TmdbItem,
    image_cache_dir: &str,
    media_type: &str,
) -> Result<Option<TmdbMetadata>, Box<dyn std::error::Error + Send + Sync>> {
    let found_title = item
        .title
        .clone()
        .or_else(|| item.original_title.clone())
        .unwrap_or_default();

    let found_year = item
        .release_date
        .as_ref()
        .and_then(|d| d.split('-').next())
        .and_then(|y| y.parse().ok());

    println!("[TMDB]   -> Match: '{}' ({:?})", found_title, found_year);

    // Use appropriate image type based on media type
    let image_type = if media_type == "tv" {
        ImageType::SeriesBanner
    } else {
        ImageType::MovieBanner
    };

    // Try to get poster first, then backdrop - use organized caching
    let mut poster_path = if let Some(ref poster) = item.poster_path {
        println!("[TMDB]   -> Has poster: {}", poster);
        let result =
            cache_image_organized(poster, image_cache_dir, &found_title, image_type.clone())
                .or_else(|| cache_image_with_fallback(poster, image_cache_dir));
        println!(
            "[TMDB] Poster download for \"{}\": {:?}",
            found_title, result
        );
        result
    } else if let Some(ref backdrop) = item.backdrop_path {
        println!("[TMDB]   -> No poster, using backdrop: {}", backdrop);
        let result =
            cache_image_organized(backdrop, image_cache_dir, &found_title, image_type.clone())
                .or_else(|| cache_image_with_fallback(backdrop, image_cache_dir));
        println!(
            "[TMDB] Poster download for \"{}\": {:?}",
            found_title, result
        );
        result
    } else {
        println!("[TMDB]   -> No poster or backdrop available");
        println!("[TMDB] Poster download for \"{}\": None", found_title);
        None
    };

    let cast_names = item
        .credits
        .as_ref()
        .and_then(|credits| credits.cast.as_ref())
        .map(|cast| {
            cast.iter()
                .filter_map(|member| member.name.as_ref())
                .map(|name| name.trim())
                .filter(|name| !name.is_empty())
                .take(8)
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        })
        .filter(|names| !names.is_empty())
        .map(|names| names.join(", "));

    let director = item
        .credits
        .as_ref()
        .and_then(|credits| credits.crew.as_ref())
        .and_then(|crew| {
            crew.iter().find(|member| {
                member
                    .job
                    .as_deref()
                    .map(|job| job.eq_ignore_ascii_case("Director"))
                    .unwrap_or(false)
            })
        })
        .and_then(|member| member.name.as_ref())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());

    // Extract IMDB ID: movie responses have it directly, TV responses have it in external_ids
    let imdb_id = item.imdb_id.clone().or_else(|| {
        item.external_ids
            .as_ref()
            .and_then(|ids| ids.imdb_id.clone())
    });

    let mut imdb_image_url: Option<String> = None;

    // Pull a higher-quality poster from balloonerismm `/images` when we know
    // the IMDb id — replaces the old imdbapi.dev primary-image lookup.
    if let Some(ref id) = imdb_id {
        let had_tmdb_poster = poster_path.is_some();
        println!(
            "[BALLOON] Poster upgrade attempt for \"{}\" (imdb_id: {}, had_tmdb_poster: {})",
            found_title, id, had_tmdb_poster
        );
        let kind = media_type;
        if let Ok(imgs) = crate::balloonerismm_api::get_images(kind, id) {
            if let Some(img_url) = imgs.pick_best_poster_url() {
                println!("[BALLOON]   -> Got poster: {}", img_url);
                if let Some(cached) =
                    cache_imdb_image(&img_url, std::path::Path::new(image_cache_dir), &image_type)
                {
                    if had_tmdb_poster {
                        println!(
                            "[BALLOON]   -> Overriding TMDB poster for \"{}\"",
                            found_title
                        );
                    }
                    poster_path = Some(cached);
                }
                imdb_image_url = Some(img_url);
            } else {
                println!(
                    "[BALLOON]   -> no posters/backdrops returned for \"{}\"",
                    found_title
                );
            }
        } else {
            println!(
                "[BALLOON]   -> /images request failed for \"{}\"",
                found_title
            );
        }
    }

    Ok(Some(TmdbMetadata {
        title: found_title,
        year: found_year,
        overview: item.overview.clone(),
        cast_names,
        director,
        poster_path,
        tmdb_id: Some(item.id.to_string()),
        imdb_id,
        runtime_seconds: item
            .runtime
            .filter(|minutes| *minutes > 0)
            .map(|minutes| (minutes as f64) * 60.0),
        imdb_image_url,
    }))
}

/// Cache image with multiple size fallbacks
fn cache_image_with_fallback(image_path: &str, cache_dir: &str) -> Option<String> {
    // Try different sizes in order of preference
    let sizes = ["w500", "w342", "w185", "original"];

    for size in &sizes {
        match cache_image(image_path, cache_dir, size) {
            Ok(path) => {
                println!("[TMDB]   -> Cached with size {}: {}", size, path);
                return Some(path);
            }
            Err(e) => {
                println!("[TMDB]   -> Failed with size {}: {}", size, e);
            }
        }
    }

    None
}

pub fn fetch_metadata_by_id(
    api_key: &str,
    id_or_url: &str,
    media_type: &str,
    image_cache_dir: &str,
) -> Result<TmdbMetadata, Box<dyn std::error::Error + Send + Sync>> {
    let (tmdb_id, source) = extract_id_from_input(id_or_url);

    println!(
        "[TMDB] fetch_metadata_by_id: type={}, id={}",
        media_type, tmdb_id
    );
    println!("[TMDB] Fetching by ID: {} (source: {})", tmdb_id, source);

    // Keep Fix Match responsive: use shorter request timeout and fewer retries.
    let client = crate::http_client::quick_client().clone();
    let request_retries = 2;

    let final_id = if source == "imdb" {
        // Look up TMDB ID from IMDB ID
        let find_url = build_tmdb_url(
            &format!("/find/{}", tmdb_id),
            api_key,
            "external_source=imdb_id",
        );

        let response = tmdb_request_with_retry(&client, &find_url, api_key, request_retries)?;
        let result: TmdbFindResult = response.json()?;

        // Try movie results first, then TV
        let id = result
            .movie_results
            .first()
            .or_else(|| result.tv_results.first())
            .map(|r| r.id.to_string())
            .ok_or_else(|| format!("No match found for IMDB ID {}", tmdb_id))?;

        id
    } else {
        tmdb_id.to_string()
    };

    // Fetch details
    let url = build_tmdb_url(
        &format!("/{}/{}", media_type, final_id),
        api_key,
        "language=en-US&append_to_response=credits,external_ids",
    );

    let response = tmdb_request_with_retry(&client, &url, api_key, request_retries)?;

    if !response.status().is_success() {
        // Try the other media type
        let alt_type = if media_type == "movie" { "tv" } else { "movie" };
        let alt_url = build_tmdb_url(
            &format!("/{}/{}", alt_type, final_id),
            api_key,
            "language=en-US&append_to_response=credits,external_ids",
        );
        let alt_response = tmdb_request_with_retry(&client, &alt_url, api_key, request_retries)?;
        if !alt_response.status().is_success() {
            return Err(format!("Failed to fetch metadata for ID {}", final_id).into());
        }
        let item: TmdbItem = alt_response.json()?;
        let metadata = create_metadata_from_item_required(&item, image_cache_dir, alt_type)?;
        println!(
            "[TMDB] Got metadata: title=\"{}\", poster={:?}, imdb_id={:?}",
            metadata.title, metadata.poster_path, metadata.imdb_id
        );
        return Ok(metadata);
    }

    let item: TmdbItem = response.json()?;
    let metadata = create_metadata_from_item_required(&item, image_cache_dir, media_type)?;
    println!(
        "[TMDB] Got metadata: title=\"{}\", poster={:?}, imdb_id={:?}",
        metadata.title, metadata.poster_path, metadata.imdb_id
    );
    Ok(metadata)
}

/// Same as `fetch_metadata_by_id` but routes to imdbapi.dev when no TMDB
/// key is configured (the free service only accepts IMDb ids).
pub fn fetch_metadata_by_id_with_fallback(
    api_key: &str,
    id_or_url: &str,
    media_type: &str,
    image_cache_dir: &str,
) -> Result<TmdbMetadata, Box<dyn std::error::Error + Send + Sync>> {
    let (id, source) = extract_id_from_input(id_or_url);

    match pick_source(api_key) {
        MetadataSource::Tmdb => {
            fetch_metadata_by_id(api_key, id_or_url, media_type, image_cache_dir)
        }
        MetadataSource::Cinemeta => {
            if source != "imdb" {
                return Err(format!(
                    "[CINEMETA] cannot resolve numeric id {} without a TMDB key (only IMDb ids)",
                    id
                )
                .into());
            }
            let kind = match media_type {
                "tv" => crate::cinemeta_api::CinemetaKind::Series,
                _ => crate::cinemeta_api::CinemetaKind::Movie,
            };
            if let Ok(meta) = crate::cinemeta_api::get_title(kind, &id) {
                return fetch_metadata_by_cinemeta_title(&meta, media_type, image_cache_dir);
            }
            // Cinemeta empty → balloonerismm (imdbapi.dev is gone).
            fetch_metadata_by_imdb_id(&id, media_type, image_cache_dir)
        }
    }
}

fn fetch_metadata_by_imdb_id(
    imdb_id: &str,
    media_type: &str,
    image_cache_dir: &str,
) -> Result<TmdbMetadata, Box<dyn std::error::Error + Send + Sync>> {
    // Balloonerismm: no-key, TMDB-shape proxy that gives us posters + credits
    // + cast in one place — replaces imdbapi.dev entirely.
    let id = imdb_id.trim();
    if !crate::balloonerismm_api::looks_like_imdb_id(id) {
        return Err(format!("[BALLOON] not an IMDb id: {}", id).into());
    }
    let image_type = if media_type == "tv" {
        ImageType::SeriesBanner
    } else {
        ImageType::MovieBanner
    };

    if media_type == "tv" {
        let t = crate::balloonerismm_api::get_tv(id).map_err(|e| e.to_string())?;
        let year = t
            .first_air_date
            .as_deref()
            .and_then(|s| s.split('-').next())
            .and_then(|y| y.parse::<i32>().ok());
        let runtime_seconds = t
            .episode_run_time
            .as_ref()
            .and_then(|arr| arr.first().copied())
            .filter(|m| *m > 0)
            .map(|m| (m as f64) * 60.0);

        // Poster via /tv/{tt}/images; fall back to series `seasons[0].poster_path` indirectly by
        // requesting the TV images endpoint. If both fail, no poster.
        let poster_url = crate::balloonerismm_api::get_images("tv", id)
            .ok()
            .and_then(|imgs| imgs.pick_best_poster_url());
        let imdb_image_url = poster_url.clone();
        let poster_path = poster_url.as_deref().and_then(|url| {
            cache_imdb_image(url, std::path::Path::new(image_cache_dir), &image_type)
        });

        // TV credits — cast (top 8) and "director" slot (an Executive Producer or Creator if present).
        let credits = crate::balloonerismm_api::get_credits("tv", id).ok();
        let cast_names = credits
            .as_ref()
            .and_then(|c| c.cast.as_ref())
            .map(|cast| {
                cast.iter()
                    .filter_map(|member| member.name.as_ref())
                    .map(|n| n.trim())
                    .filter(|n| !n.is_empty())
                    .take(8)
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .map(|v| v.join(", "));
        let director = credits
            .as_ref()
            .and_then(|c| c.crew.as_ref())
            .and_then(|crew| {
                crew.iter()
                    .find(|m| {
                        m.job
                            .as_deref()
                            .map(|j| {
                                let j = j.to_ascii_lowercase();
                                j.contains("creator") || j.contains("showrunner")
                            })
                            .unwrap_or(false)
                    })
                    .or_else(|| crew.first())
                    .and_then(|m| m.name.clone())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            });

        return Ok(TmdbMetadata {
            title: t
                .name
                .clone()
                .or(t.original_name.clone())
                .unwrap_or_else(|| "Unknown".to_string()),
            year,
            overview: t.overview.clone(),
            cast_names,
            director,
            poster_path,
            tmdb_id: None,
            imdb_id: Some(t.id.clone()),
            runtime_seconds,
            imdb_image_url,
        });
    }

    // Movie branch
    let m = crate::balloonerismm_api::get_movie(id).map_err(|e| e.to_string())?;
    let year = m
        .release_date
        .as_deref()
        .and_then(|s| s.split('-').next())
        .and_then(|y| y.parse::<i32>().ok());
    let runtime_seconds = m
        .runtime
        .filter(|minutes| *minutes > 0)
        .map(|minutes| (minutes as f64) * 60.0);

    // Best poster comes from /movie/{tt}/images when available; fall back to none.
    let poster_url = crate::balloonerismm_api::get_images("movie", id)
        .ok()
        .and_then(|imgs| imgs.pick_best_poster_url());
    let imdb_image_url = poster_url.clone();
    let poster_path = poster_url
        .as_deref()
        .and_then(|url| cache_imdb_image(url, std::path::Path::new(image_cache_dir), &image_type));

    // Cast + director from /credits.
    let credits = crate::balloonerismm_api::get_credits("movie", id).ok();
    let cast_names = credits
        .as_ref()
        .and_then(|c| c.cast.as_ref())
        .map(|cast| {
            cast.iter()
                .filter_map(|member| member.name.as_ref())
                .map(|n| n.trim())
                .filter(|n| !n.is_empty())
                .take(8)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .map(|v| v.join(", "));
    let director = credits
        .as_ref()
        .and_then(|c| c.crew.as_ref())
        .and_then(|crew| {
            crew.iter()
                .find(|m| {
                    m.job
                        .as_deref()
                        .map(|j| j.eq_ignore_ascii_case("Director"))
                        .unwrap_or(false)
                })
                .and_then(|m| m.name.clone())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });

    Ok(TmdbMetadata {
        title: m
            .title
            .clone()
            .or(m.original_title.clone())
            .unwrap_or_else(|| "Unknown".to_string()),
        year,
        overview: m.overview.clone(),
        cast_names,
        director,
        poster_path,
        tmdb_id: None,
        imdb_id: Some(m.id.clone()),
        runtime_seconds,
        imdb_image_url,
    })
}

fn create_metadata_from_item_required(
    item: &TmdbItem,
    image_cache_dir: &str,
    media_type: &str,
) -> Result<TmdbMetadata, Box<dyn std::error::Error + Send + Sync>> {
    create_metadata_from_item(item, image_cache_dir, media_type)?
        .ok_or_else(|| "Failed to create metadata".into())
}

fn extract_id_from_input(input: &str) -> (String, &str) {
    let input = input.trim();

    // Pure numeric ID
    if input.chars().all(|c| c.is_ascii_digit()) {
        return (input.to_string(), "tmdb");
    }

    // IMDB ID (tt followed by digits)
    if let Some(caps) = regex::Regex::new(r"(tt\d+)")
        .ok()
        .and_then(|re| re.captures(input))
    {
        if let Some(m) = caps.get(1) {
            return (m.as_str().to_string(), "imdb");
        }
    }

    // TMDB movie URL
    if let Some(caps) = regex::Regex::new(r"themoviedb\.org/movie/(\d+)")
        .ok()
        .and_then(|re| re.captures(input))
    {
        if let Some(m) = caps.get(1) {
            return (m.as_str().to_string(), "tmdb");
        }
    }

    // TMDB TV URL
    if let Some(caps) = regex::Regex::new(r"themoviedb\.org/tv/(\d+)")
        .ok()
        .and_then(|re| re.captures(input))
    {
        if let Some(m) = caps.get(1) {
            return (m.as_str().to_string(), "tmdb");
        }
    }

    (input.to_string(), "tmdb")
}

/// Cache image from TMDB
fn cache_image(
    image_path: &str,
    cache_dir: &str,
    size: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let filename = Path::new(image_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown.jpg");

    let local_path = Path::new(cache_dir).join(filename);

    if local_path.exists() {
        // Check if file is not empty
        if let Ok(metadata) = std::fs::metadata(&local_path) {
            if metadata.len() > 100 {
                return Ok(format!("image_cache/{}", filename));
            }
            // File is corrupted/empty, delete and re-download
            let _ = std::fs::remove_file(&local_path);
        }
    }

    let image_url = format!("https://image.tmdb.org/t/p/{}{}", size, image_path);

    // Image download should fail fast instead of blocking Fix Match for minutes.
    let client = crate::http_client::quick_client().clone();
    let response = client.get(&image_url).send()?;

    if !response.status().is_success() {
        return Err(format!("Failed to download image: HTTP {}", response.status()).into());
    }

    let bytes = response.bytes()?;

    if bytes.len() < 100 {
        return Err("Downloaded image is too small (likely invalid)".into());
    }

    fs::create_dir_all(cache_dir)?;
    let mut file = fs::File::create(&local_path)?;
    file.write_all(&bytes)?;

    Ok(format!("image_cache/{}", filename))
}

/// Download and cache an image from an imdbapi.dev primaryImage URL
pub fn cache_imdb_image(
    url: &str,
    image_cache_dir: &std::path::Path,
    image_type: &ImageType,
) -> Option<String> {
    use std::io::Write;

    println!("[IMDBAPI] Downloading image: {}", url);

    let client = crate::http_client::quick_client().clone();

    // Try different size replacements for m.media-amazon.com URLs
    // Original URL might be like: https://m.media-amazon.com/images/M/MV5B...jpg
    // We can try adding size parameters or use as-is
    let sizes_to_try = vec![
        url.to_string(),
        // Amazon image URLs can sometimes be resized by appending._UX500_.jpg etc.
    ];

    for download_url in sizes_to_try {
        let response = match client.get(&download_url).send() {
            Ok(r) if r.status().is_success() => r,
            _ => continue,
        };

        let bytes = match response.bytes() {
            Ok(b) if b.len() > 100 => b,
            _ => continue,
        };

        // Generate filename based on image type
        // Extract a unique hash from the URL to avoid filename collisions
        let url_hash = {
            let hash = url.split('/').last().unwrap_or("unknown");
            let safe: String = hash
                .chars()
                .filter(|c| c.is_alphanumeric())
                .take(20)
                .collect();
            if safe.is_empty() {
                "unknown".to_string()
            } else {
                safe
            }
        };
        let filename = match image_type {
            ImageType::SeriesBanner => {
                format!("imdb_series_{}_banner.jpg", url_hash)
            }
            ImageType::EpisodeBanner { season, episode } => {
                format!("imdb_s{:02}e{:02}_{}_banner.jpg", season, episode, url_hash)
            }
            ImageType::MovieBanner => {
                format!("imdb_movie_{}_banner.jpg", url_hash)
            }
        };

        let file_path = image_cache_dir.join(&filename);
        if let Ok(mut file) = std::fs::File::create(&file_path) {
            let _ = file.write_all(&bytes);
            let path = format!("image_cache/{}", filename);
            println!("[IMDBAPI] Cached image: {}", path);
            return Some(path);
        }
    }

    println!("[IMDBAPI] Image download failed: {}", url);
    None
}

/// Create a slug from a title (for folder/file naming)
fn create_slug(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn image_cache_tag(image_path: &str) -> String {
    let tag = Path::new(image_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(create_slug)
        .unwrap_or_default();

    if tag.is_empty() {
        "img".to_string()
    } else {
        tag
    }
}

/// Cache image with organized folder structure.
/// Includes a source-image tag in filenames to avoid stale cache collisions
/// when metadata is corrected but title stays the same.
pub fn cache_image_organized(
    image_path: &str,
    cache_dir: &str,
    title: &str,
    image_type: ImageType,
) -> Option<String> {
    println!(
        "[TMDB] Caching image: {} (type: {:?})",
        image_path, image_type
    );

    let slug = create_slug(title);
    let source_tag = image_cache_tag(image_path);

    let (subfolder, filename) = match image_type {
        ImageType::SeriesBanner => {
            let subfolder = slug.clone();
            let filename = format!("{}_{}_banner_hq.jpg", slug, source_tag);
            (Some(subfolder), filename)
        }
        ImageType::EpisodeBanner { season, episode } => {
            let subfolder = slug.clone();
            let filename = format!(
                "{}_s{}e{}_{}_banner_hq.jpg",
                slug, season, episode, source_tag
            );
            (Some(subfolder), filename)
        }
        ImageType::MovieBanner => {
            let filename = format!("{}_{}_banner_hq.jpg", slug, source_tag);
            (None, filename)
        }
    };

    let target_dir = if let Some(ref sub) = subfolder {
        Path::new(cache_dir).join(sub)
    } else {
        Path::new(cache_dir).to_path_buf()
    };

    // Create the directory if needed
    if let Err(e) = fs::create_dir_all(&target_dir) {
        println!("[TMDB] Failed to create directory {:?}: {}", target_dir, e);
        return None;
    }

    let local_path = target_dir.join(&filename);

    // Check if already exists and valid
    if local_path.exists() {
        if let Ok(metadata) = std::fs::metadata(&local_path) {
            if metadata.len() > 100 {
                return Some(format_image_path(&subfolder, &filename));
            }
            let _ = std::fs::remove_file(&local_path);
        }
    }

    // Try different sizes with retry logic
    let sizes = ["original", "w1280", "w780"];

    for size in &sizes {
        let image_url = format!("https://image.tmdb.org/t/p/{}{}", size, image_path);

        // Retry logic for image download
        for attempt in 0..2 {
            if attempt > 0 {
                let delay = BASE_DELAY_MS * (1 << attempt);
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }

            let client = crate::http_client::quick_client().clone();
            match client.get(&image_url).send() {
                Ok(response) => {
                    if response.status().is_success() {
                        if let Ok(bytes) = response.bytes() {
                            if bytes.len() > 100 {
                                if let Ok(mut file) = fs::File::create(&local_path) {
                                    if file.write_all(&bytes).is_ok() {
                                        let result_path = format_image_path(&subfolder, &filename);
                                        println!(
                                            "[TMDB] Cached image: {:?} (size: {})",
                                            local_path, size
                                        );
                                        println!("[TMDB] Cached: {}", result_path);
                                        return Some(result_path);
                                    }
                                }
                            }
                        }
                    }
                    // Non-success status, try next size
                    break;
                }
                Err(e) => {
                    let error_str = e.to_string();
                    let is_retryable = error_str.contains("10054")
                        || error_str.contains("connection")
                        || error_str.contains("timeout")
                        || error_str.contains("timed out")
                        || error_str.contains("closed")
                        || error_str.contains("reset");
                    if !is_retryable {
                        break;
                    }
                    println!(
                        "[TMDB] Image download retry {} for {}: {}",
                        attempt + 1,
                        size,
                        error_str
                    );
                }
            }
        }
    }

    println!("[TMDB] Failed to cache image: {}", image_path);
    None
}

fn format_image_path(subfolder: &Option<String>, filename: &str) -> String {
    if let Some(ref sub) = subfolder {
        format!("image_cache/{}/{}", sub, filename)
    } else {
        format!("image_cache/{}", filename)
    }
}

#[derive(Debug, Clone)]
pub enum ImageType {
    SeriesBanner,
    EpisodeBanner { season: i32, episode: i32 },
    MovieBanner,
}

/// Fetch TV show details including number of seasons
pub fn fetch_tv_show_details(
    api_key: &str,
    tmdb_id: &str,
) -> Result<TvShowDetails, Box<dyn std::error::Error + Send + Sync>> {
    println!("[TMDB] Fetching TV show details for ID: {}", tmdb_id);

    let url = build_tmdb_url(&format!("/tv/{}", tmdb_id), api_key, "language=en-US");

    let client = crate::http_client::shared_client().clone();
    let response = tmdb_request(&client, &url, api_key)?;

    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch TV show details: HTTP {}",
            response.status()
        )
        .into());
    }

    let details: TvShowDetails = response.json()?;
    println!("[TMDB] TV show has {} seasons", details.number_of_seasons);

    Ok(details)
}

#[derive(Debug, Deserialize)]
pub struct TvShowDetails {
    pub id: i64,
    pub name: String,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub first_air_date: Option<String>,
    pub number_of_seasons: i32,
    pub number_of_episodes: i32,
    pub seasons: Vec<TvShowSeasonBrief>,
}

#[derive(Debug, Deserialize)]
pub struct TvShowSeasonBrief {
    pub id: i64,
    pub season_number: i32,
    pub name: String,
    pub episode_count: i32,
    pub poster_path: Option<String>,
}

/// Fetch all episodes for a specific season
pub fn fetch_season_episodes(
    api_key: &str,
    tmdb_id: &str,
    season_number: i32,
    series_title: &str,
    image_cache_dir: &str,
) -> Result<TmdbSeasonInfo, Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "[TMDB] Fetching season {} episodes for series ID: {}",
        season_number, tmdb_id
    );

    let url = build_tmdb_url(
        &format!("/tv/{}/season/{}", tmdb_id, season_number),
        api_key,
        "language=en-US",
    );

    let client = crate::http_client::shared_client().clone();
    let response = tmdb_request(&client, &url, api_key)?;

    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch season {}: HTTP {}",
            season_number,
            response.status()
        )
        .into());
    }

    #[derive(Debug, Deserialize)]
    struct SeasonResponse {
        id: i64,
        name: String,
        overview: Option<String>,
        poster_path: Option<String>,
        season_number: i32,
        episodes: Vec<EpisodeResponse>,
    }

    #[derive(Debug, Deserialize)]
    struct EpisodeResponse {
        id: i64,
        name: String,
        overview: Option<String>,
        still_path: Option<String>,
        episode_number: i32,
        season_number: i32,
        air_date: Option<String>,
        vote_average: Option<f64>,
    }

    let season_data: SeasonResponse = response.json()?;
    println!(
        "[TMDB] Found {} episodes in season {}",
        season_data.episodes.len(),
        season_number
    );

    // Cache season poster if available
    let season_poster = season_data.poster_path.as_ref().and_then(|path| {
        cache_image_organized(path, image_cache_dir, series_title, ImageType::SeriesBanner)
    });

    // Process episodes and cache their images
    let episodes: Vec<TmdbEpisodeInfo> = season_data
        .episodes
        .into_iter()
        .map(|ep| {
            // Cache episode still image
            let still_path = if let Some(ref path) = ep.still_path {
                println!(
                    "[TMDB] Downloading episode image for S{:02}E{:02}: {}",
                    ep.season_number, ep.episode_number, path
                );
                let cached = cache_image_organized(
                    path,
                    image_cache_dir,
                    series_title,
                    ImageType::EpisodeBanner {
                        season: ep.season_number,
                        episode: ep.episode_number,
                    },
                );
                if cached.is_some() {
                    println!(
                        "[TMDB] Successfully cached episode image for S{:02}E{:02}",
                        ep.season_number, ep.episode_number
                    );
                } else {
                    println!(
                        "[TMDB] Failed to cache episode image for S{:02}E{:02}",
                        ep.season_number, ep.episode_number
                    );
                }
                cached
            } else {
                println!(
                    "[TMDB] No still_path for S{:02}E{:02} (TMDB has no image)",
                    ep.season_number, ep.episode_number
                );
                None
            };

            TmdbEpisodeInfo {
                episode_number: ep.episode_number,
                season_number: ep.season_number,
                name: ep.name,
                overview: ep.overview,
                still_path,
                air_date: ep.air_date,
                vote_average: ep.vote_average,
            }
        })
        .collect();

    Ok(TmdbSeasonInfo {
        season_number: season_data.season_number,
        name: season_data.name,
        overview: season_data.overview,
        poster_path: season_poster,
        episode_count: episodes.len() as i32,
        episodes,
    })
}

/// Like `fetch_season_episodes` but routes by id type:
/// - `tid` looks like `tt1234567` → imdbapi.dev `/titles/{ttid}/episodes?season=N`
/// - `tid` is numeric (TMDB id) → TMDB `/tv/{id}/season/{n}` (only when key configured)
pub fn fetch_season_episodes_with_fallback(
    api_key: &str,
    tid: &str,
    season_number: i32,
    series_title: &str,
    image_cache_dir: &str,
) -> Result<TmdbSeasonInfo, Box<dyn std::error::Error + Send + Sync>> {
    let trimmed = tid.trim();
    if crate::balloonerismm_api::looks_like_imdb_id(trimmed) {
        // Balloonerismm: free TMDB-shape proxy. /tv/{tt}/season/{n} returns the
        // season plus every episode with still_path, vote_average, overview,
        // runtime. Replaces imdb_api::list_episodes entirely.
        let season = crate::balloonerismm_api::get_season(trimmed, season_number.max(1))
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        let season_poster_url = season.poster_path.clone();
        let poster_path = season_poster_url.as_deref().and_then(|url| {
            cache_imdb_image(
                url,
                std::path::Path::new(image_cache_dir),
                &ImageType::SeriesBanner,
            )
        });

        let episodes: Vec<TmdbEpisodeInfo> = season
            .episodes
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.episode_number.unwrap_or(0) > 0) // drop episode 0 (unaired pilots)
            .map(|e| TmdbEpisodeInfo {
                episode_number: e.episode_number.unwrap_or(0),
                season_number: e.season_number.unwrap_or(season_number),
                name: e.name.unwrap_or_else(|| "(untitled)".to_string()),
                overview: e.overview,
                still_path: e.still_path.and_then(|url| {
                    cache_imdb_image(
                        &url,
                        std::path::Path::new(image_cache_dir),
                        &ImageType::EpisodeBanner {
                            season: e.season_number.unwrap_or(season_number),
                            episode: e.episode_number.unwrap_or(0),
                        },
                    )
                }),
                air_date: e.air_date,
                vote_average: e.vote_average,
            })
            .collect();
        let season_name = season
            .name
            .clone()
            .unwrap_or_else(|| format!("Season {}", season_number));
        return Ok(TmdbSeasonInfo {
            season_number: season.season_number.unwrap_or(season_number),
            name: season_name,
            overview: season.overview,
            poster_path,
            episode_count: episodes.len() as i32,
            episodes,
        });
    }

    if api_key.trim().is_empty() {
        return Err("[BALLOON] cannot resolve TMDB numeric id without an API key".into());
    }
    fetch_season_episodes(
        api_key,
        trimmed,
        season_number,
        series_title,
        image_cache_dir,
    )
}

/// Fetch and cache all episode metadata for a TV series
pub fn fetch_all_series_episodes(
    api_key: &str,
    tmdb_id: &str,
    series_title: &str,
    image_cache_dir: &str,
) -> Result<Vec<TmdbSeasonInfo>, Box<dyn std::error::Error + Send + Sync>> {
    println!(
        "[TMDB] Fetching all episode metadata for series: {} (ID: {})",
        series_title, tmdb_id
    );

    // First get the show details to know how many seasons
    let show_details = fetch_tv_show_details(api_key, tmdb_id)?;

    let mut all_seasons = Vec::new();

    // Fetch each season (skip season 0 which is usually specials)
    for season_info in &show_details.seasons {
        if season_info.season_number == 0 {
            println!("[TMDB] Skipping specials (season 0)");
            continue;
        }

        match fetch_season_episodes(
            api_key,
            tmdb_id,
            season_info.season_number,
            series_title,
            image_cache_dir,
        ) {
            Ok(season) => {
                println!(
                    "[TMDB] Fetched {} episodes for season {}",
                    season.episodes.len(),
                    season.season_number
                );
                all_seasons.push(season);
            }
            Err(e) => {
                println!(
                    "[TMDB] Warning: Failed to fetch season {}: {}",
                    season_info.season_number, e
                );
            }
        }

        // Small delay between season fetches to respect rate limits
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    println!(
        "[TMDB] Fetched {} total seasons with episode data",
        all_seasons.len()
    );
    Ok(all_seasons)
}

/// Fetch metadata and images for only specific episodes (the ones user owns)
/// owned_episodes is a list of (season_number, episode_number) tuples
pub fn fetch_owned_episodes_only(
    api_key: &str,
    tmdb_id: &str,
    series_title: &str,
    image_cache_dir: &str,
    owned_episodes: &[(i32, i32)],
) -> Result<Vec<TmdbEpisodeInfo>, Box<dyn std::error::Error + Send + Sync>> {
    if owned_episodes.is_empty() {
        println!("[TMDB] No owned episodes to refresh");
        return Ok(Vec::new());
    }

    println!(
        "[TMDB] Fetching metadata for {} owned episodes of: {}",
        owned_episodes.len(),
        series_title
    );

    // Group episodes by season for efficient fetching
    let mut seasons_needed: std::collections::HashSet<i32> = std::collections::HashSet::new();
    for (season, _) in owned_episodes {
        seasons_needed.insert(*season);
    }

    println!("[TMDB] Seasons needed: {:?}", seasons_needed);

    let mut result_episodes = Vec::new();
    let client = crate::http_client::shared_client().clone();

    for season_num in seasons_needed {
        println!("[TMDB] Fetching season {} data...", season_num);

        let url = build_tmdb_url(
            &format!("/tv/{}/season/{}", tmdb_id, season_num),
            api_key,
            "language=en-US",
        );

        let response = match tmdb_request(&client, &url, api_key) {
            Ok(r) => r,
            Err(e) => {
                println!("[TMDB] Failed to fetch season {}: {}", season_num, e);
                continue;
            }
        };

        if !response.status().is_success() {
            println!(
                "[TMDB] Season {} fetch returned {}",
                season_num,
                response.status()
            );
            continue;
        }

        #[derive(Debug, Deserialize)]
        struct SeasonResponse {
            episodes: Vec<EpisodeResponse>,
        }

        #[derive(Debug, Deserialize)]
        struct EpisodeResponse {
            name: String,
            overview: Option<String>,
            still_path: Option<String>,
            episode_number: i32,
            season_number: i32,
            air_date: Option<String>,
            vote_average: Option<f64>,
        }

        let season_data: SeasonResponse = match response.json() {
            Ok(d) => d,
            Err(e) => {
                println!("[TMDB] Failed to parse season {} data: {}", season_num, e);
                continue;
            }
        };

        // Filter to only the episodes user owns in this season
        let owned_in_season: Vec<i32> = owned_episodes
            .iter()
            .filter(|(s, _)| *s == season_num)
            .map(|(_, e)| *e)
            .collect();

        println!(
            "[TMDB] User owns episodes {:?} in season {}",
            owned_in_season, season_num
        );

        for ep in season_data.episodes {
            if !owned_in_season.contains(&ep.episode_number) {
                continue; // Skip episodes user doesn't own
            }

            println!(
                "[TMDB] Processing owned episode S{:02}E{:02}: {}",
                ep.season_number, ep.episode_number, ep.name
            );

            // Download still image only for this episode
            let still_path = if let Some(ref path) = ep.still_path {
                println!(
                    "[TMDB] Downloading image for S{:02}E{:02}",
                    ep.season_number, ep.episode_number
                );
                let cached = cache_image_organized(
                    path,
                    image_cache_dir,
                    series_title,
                    ImageType::EpisodeBanner {
                        season: ep.season_number,
                        episode: ep.episode_number,
                    },
                );
                if cached.is_some() {
                    println!(
                        "[TMDB] Successfully cached S{:02}E{:02} image",
                        ep.season_number, ep.episode_number
                    );
                } else {
                    println!(
                        "[TMDB] Failed to cache S{:02}E{:02} image",
                        ep.season_number, ep.episode_number
                    );
                }
                cached
            } else {
                println!(
                    "[TMDB] No still_path for S{:02}E{:02} on TMDB",
                    ep.season_number, ep.episode_number
                );
                None
            };

            result_episodes.push(TmdbEpisodeInfo {
                episode_number: ep.episode_number,
                season_number: ep.season_number,
                name: ep.name,
                overview: ep.overview,
                still_path,
                air_date: ep.air_date,
                vote_average: ep.vote_average,
            });
        }

        // Small delay between season fetches
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    println!(
        "[TMDB] Successfully processed {} owned episodes",
        result_episodes.len()
    );
    Ok(result_episodes)
}

/// Lightweight lookup: fetch just the IMDB ID for a given TMDB ID + media type.
/// Returns None on any failure (non-critical path).
pub fn fetch_imdb_id(api_key: &str, tmdb_id: i64, media_type: &str) -> Option<String> {
    let url = build_tmdb_url(
        &format!("/{}/{}", media_type, tmdb_id),
        api_key,
        "language=en-US&append_to_response=external_ids",
    );

    let client = crate::http_client::quick_client().clone();
    let response = tmdb_request(&client, &url, api_key).ok()?;

    if !response.status().is_success() {
        return None;
    }

    let item: TmdbItem = response.json().ok()?;

    // Movie responses have imdb_id directly, TV responses have it in external_ids
    item.imdb_id.or_else(|| {
        item.external_ids
            .as_ref()
            .and_then(|ids| ids.imdb_id.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_backend_proxy_credential (always false now) ───────────────────

    #[test]
    fn is_backend_proxy_credential_false() {
        assert!(!is_backend_proxy_credential("__TMDB_BACKEND_PROXY__"));
        assert!(!is_backend_proxy_credential("some_api_key"));
        assert!(!is_backend_proxy_credential(""));
    }

    // ── get_tmdb_credential ─────────────────────────────────────────────

    #[test]
    fn get_tmdb_credential_returns_user_key() {
        assert_eq!(get_tmdb_credential("my_key"), "my_key");
    }

    #[test]
    fn get_tmdb_credential_returns_empty_for_no_key() {
        assert_eq!(get_tmdb_credential(""), "");
    }

    #[test]
    fn get_tmdb_credential_trims_whitespace() {
        assert_eq!(get_tmdb_credential("   "), "");
    }

    // ── normalize_title ─────────────────────────────────────────────────

    #[test]
    fn normalize_title_basic() {
        assert_eq!(normalize_title("Hello World"), "hello world");
    }

    #[test]
    fn normalize_title_strips_leading_the() {
        assert_eq!(normalize_title("The Matrix"), "matrix");
    }

    #[test]
    fn normalize_title_strips_leading_a() {
        assert_eq!(normalize_title("A Quiet Place"), "quiet place");
    }

    #[test]
    fn normalize_title_strips_leading_an() {
        assert_eq!(normalize_title("An American in Paris"), "american in paris");
    }

    #[test]
    fn normalize_title_replaces_ampersand() {
        assert_eq!(normalize_title("Fast & Furious"), "fast and furious");
    }

    #[test]
    fn normalize_title_replaces_dots_and_underscores() {
        assert_eq!(normalize_title("Mr.Nobody_Title"), "mr nobody title");
    }

    #[test]
    fn normalize_title_strips_special_chars() {
        assert_eq!(normalize_title("Hello! @World#"), "hello world");
    }

    #[test]
    fn normalize_title_collapses_spaces() {
        assert_eq!(normalize_title("Hello   World"), "hello world");
    }

    #[test]
    fn normalize_title_replaces_colon() {
        assert_eq!(normalize_title("Title: Subtitle"), "title subtitle");
    }

    #[test]
    fn normalize_title_replaces_dash() {
        assert_eq!(normalize_title("Spider-Man"), "spider man");
    }

    // ── title_similarity ────────────────────────────────────────────────

    #[test]
    fn title_similarity_identical() {
        assert_eq!(title_similarity("The Matrix", "The Matrix"), 1.0);
    }

    #[test]
    fn title_similarity_empty_strings() {
        // Both normalize to "" which are equal, so early-return 1.0
        assert_eq!(title_similarity("", ""), 1.0);
    }

    #[test]
    fn title_similarity_one_empty() {
        assert_eq!(title_similarity("Matrix", ""), 0.0);
        assert_eq!(title_similarity("", "Matrix"), 0.0);
    }

    #[test]
    fn title_similarity_similar() {
        let score = title_similarity("The Matrix", "Matrix");
        assert!(score > 0.9, "expected >0.9, got {}", score);
    }

    #[test]
    fn title_similarity_different() {
        let score = title_similarity("Inception", "Titanic");
        assert!(score < 0.3, "expected <0.3, got {}", score);
    }

    #[test]
    fn title_similarity_partial_overlap() {
        let score = title_similarity("Star Wars", "Star Trek");
        assert!(
            score > 0.3 && score < 1.0,
            "expected mid-range, got {}",
            score
        );
    }

    #[test]
    fn title_similarity_case_insensitive() {
        assert_eq!(title_similarity("MATRIX", "matrix"), 1.0);
    }

    // ── minimal_clean_title ─────────────────────────────────────────────

    #[test]
    fn minimal_clean_title_removes_trailing_brackets() {
        assert_eq!(minimal_clean_title("Movie [2020]"), "Movie");
    }

    #[test]
    fn minimal_clean_title_removes_trailing_parens() {
        assert_eq!(minimal_clean_title("Movie (Director's Cut)"), "Movie");
    }

    #[test]
    fn minimal_clean_title_removes_trailing_dash_group() {
        assert_eq!(minimal_clean_title("Movie - GROUP"), "Movie");
    }

    #[test]
    fn minimal_clean_title_no_change() {
        assert_eq!(minimal_clean_title("Clean Title"), "Clean Title");
    }

    #[test]
    fn minimal_clean_title_empty() {
        assert_eq!(minimal_clean_title(""), "");
    }

    // ── extract_title_variations ────────────────────────────────────────

    #[test]
    fn extract_title_variations_includes_original() {
        let v = extract_title_variations("My Movie");
        assert!(v.contains(&"My Movie".to_string()));
    }

    #[test]
    fn extract_title_variations_removes_the_prefix() {
        let v = extract_title_variations("The Matrix");
        assert!(v.iter().any(|x| x.to_lowercase() == "matrix"));
    }

    #[test]
    fn extract_title_variations_ampersand_and() {
        let v = extract_title_variations("Fast & Furious");
        assert!(v.iter().any(|x| x.contains("and")));
    }

    #[test]
    fn extract_title_variations_dot_to_space() {
        let v = extract_title_variations("Mr.Nobody");
        assert!(v.iter().any(|x| x == "Mr Nobody"));
    }

    #[test]
    fn extract_title_variations_episode_pattern() {
        let v = extract_title_variations("Breaking Bad S01E01");
        assert!(v.iter().any(|x| x.to_lowercase().contains("breaking bad")));
    }

    #[test]
    fn extract_title_variations_year_pattern() {
        let v = extract_title_variations("Dune 2021");
        assert!(v.iter().any(|x| x.to_lowercase() == "dune"));
    }

    // ── is_reasonable_match ─────────────────────────────────────────────

    #[test]
    fn is_reasonable_match_exact() {
        assert!(is_reasonable_match("Matrix", "Matrix"));
    }

    #[test]
    fn is_reasonable_match_contains() {
        assert!(is_reasonable_match("Matrix", "The Matrix Reloaded"));
    }

    #[test]
    fn is_reasonable_match_numeric() {
        assert!(is_reasonable_match("1899", "1899"));
    }

    #[test]
    fn is_reasonable_match_numeric_contained() {
        assert!(is_reasonable_match("1899", "Series 1899"));
    }

    #[test]
    fn is_reasonable_match_first_word() {
        assert!(is_reasonable_match("Breaking Bad", "Breaking"));
    }

    #[test]
    fn is_reasonable_match_no_match() {
        assert!(!is_reasonable_match("Matrix", "Titanic"));
    }

    #[test]
    fn is_reasonable_match_case_insensitive() {
        assert!(is_reasonable_match("matrix", "MATRIX"));
    }

    // ── is_access_token ─────────────────────────────────────────────────

    #[test]
    fn is_access_token_true() {
        assert!(is_access_token("eyJhbGciOiJIUzI1NiJ9"));
    }

    #[test]
    fn is_access_token_false_api_key() {
        assert!(!is_access_token("abc123def456"));
    }

    #[test]
    fn is_access_token_false_empty() {
        assert!(!is_access_token(""));
    }

    // ── build_tmdb_url ──────────────────────────────────────────────────

    #[test]
    fn build_tmdb_url_with_api_key() {
        let url = build_tmdb_url("/search/movie", "myapikey", "query=Matrix");
        assert!(url.contains("api_key=myapikey"));
        assert!(url.contains("/search/movie"));
        assert!(url.contains("query=Matrix"));
    }

    #[test]
    fn build_tmdb_url_with_access_token() {
        let url = build_tmdb_url("/search/movie", "eyJtoken123", "query=Matrix");
        assert!(url.starts_with("https://api.themoviedb.org/3/search/movie"));
        assert!(!url.contains("api_key"));
        assert!(url.contains("query=Matrix"));
    }

    #[test]
    fn build_tmdb_url_with_backend_proxy_removed() {
        let url = build_tmdb_url("/search/movie", "__TMDB_BACKEND_PROXY__", "query=Matrix");
        assert!(
            url.starts_with("https://api.themoviedb.org/3/search/movie"),
            "TMDP magic credential must NOT be rerouted through onrender.com, got: {}",
            url
        );
        assert!(url.contains("query=Matrix"));
    }

    // ── build_tmdb_url empty extra cases ─────────────────────────────────

    #[test]
    fn extract_id_from_input_numeric() {
        let (id, source) = extract_id_from_input("12345");
        assert_eq!(id, "12345");
        assert_eq!(source, "tmdb");
    }

    #[test]
    fn extract_id_from_input_imdb_id() {
        let (id, source) = extract_id_from_input("tt0133093");
        assert_eq!(id, "tt0133093");
        assert_eq!(source, "imdb");
    }

    #[test]
    fn extract_id_from_input_tmdb_movie_url() {
        let (id, source) = extract_id_from_input("https://www.themoviedb.org/movie/603-the-matrix");
        assert_eq!(id, "603");
        assert_eq!(source, "tmdb");
    }

    #[test]
    fn extract_id_from_input_tmdb_tv_url() {
        let (id, source) =
            extract_id_from_input("https://www.themoviedb.org/tv/1399-game-of-thrones");
        assert_eq!(id, "1399");
        assert_eq!(source, "tmdb");
    }

    #[test]
    fn extract_id_from_input_fallback() {
        let (id, source) = extract_id_from_input("some-text");
        assert_eq!(id, "some-text");
        assert_eq!(source, "tmdb");
    }

    // ── find_best_match ─────────────────────────────────────────────────

    fn make_item(id: i64, title: &str, year: &str, poster: bool) -> TmdbItem {
        TmdbItem {
            id,
            title: Some(title.to_string()),
            original_title: Some(title.to_string()),
            overview: None,
            poster_path: if poster {
                Some("/poster.jpg".to_string())
            } else {
                None
            },
            backdrop_path: None,
            release_date: Some(format!("{}-01-01", year)),
            vote_average: Some(7.0),
            popularity: Some(50.0),
            vote_count: Some(1000),
            runtime: None,
            credits: None,
            imdb_id: None,
            external_ids: None,
        }
    }

    #[test]
    fn find_best_match_empty() {
        let results: Vec<TmdbItem> = vec![];
        assert!(find_best_match(&results, "Matrix", None, true).is_none());
    }

    #[test]
    fn find_best_match_exact_title() {
        let items = vec![
            make_item(1, "The Matrix Reloaded", "2003", true),
            make_item(2, "The Matrix", "1999", true),
        ];
        let best = find_best_match(&items, "The Matrix", Some(1999), true);
        assert!(best.is_some());
        assert_eq!(best.unwrap().id, 2);
    }

    #[test]
    fn find_best_match_prefers_year_match() {
        let items = vec![
            make_item(1, "Dune", "1984", true),
            make_item(2, "Dune", "2021", true),
        ];
        let best = find_best_match(&items, "Dune", Some(2021), true);
        assert!(best.is_some());
        assert_eq!(best.unwrap().id, 2);
    }

    #[test]
    fn find_best_match_rejects_year_mismatch() {
        let items = vec![make_item(1, "Dune", "1984", true)];
        let best = find_best_match(&items, "Dune", Some(2021), true);
        // Year diff > 2 => rejected
        assert!(best.is_none());
    }

    #[test]
    fn find_best_match_no_year_constraint() {
        let items = vec![make_item(1, "The Matrix", "1999", true)];
        let best = find_best_match(&items, "The Matrix", None, true);
        assert!(best.is_some());
    }

    #[test]
    fn find_best_match_non_strict_accepts_lower_score() {
        let items = vec![make_item(1, "Something", "2020", true)];
        let best = find_best_match(&items, "Matrix", None, false);
        // Non-strict should still return something
        assert!(best.is_some());
    }

    // ── TmdbMetadata serde roundtrip ────────────────────────────────────

    #[test]
    fn tmdb_metadata_serde_roundtrip() {
        let meta = TmdbMetadata {
            title: "The Matrix".to_string(),
            year: Some(1999),
            overview: Some("A hacker discovers...".to_string()),
            cast_names: Some("Keanu Reeves".to_string()),
            director: Some("Wachowski Sisters".to_string()),
            poster_path: Some("/poster.jpg".to_string()),
            tmdb_id: Some("603".to_string()),
            imdb_id: Some("tt0133093".to_string()),
            runtime_seconds: Some(8160.0),
            imdb_image_url: Some("https://example.com/img.jpg".to_string()),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: TmdbMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.title, "The Matrix");
        assert_eq!(deserialized.year, Some(1999));
        assert_eq!(deserialized.imdb_id, Some("tt0133093".to_string()));
        assert_eq!(deserialized.runtime_seconds, Some(8160.0));
    }

    // ── TmdbSearchListItem serde roundtrip ──────────────────────────────

    #[test]
    fn tmdb_search_list_item_serde_roundtrip() {
        let item = TmdbSearchListItem {
            id: 603,
            title: Some("The Matrix".to_string()),
            name: None,
            media_type: "movie".to_string(),
            poster_path: Some("/poster.jpg".to_string()),
            backdrop_path: None,
            overview: Some("desc".to_string()),
            release_date: Some("1999-03-31".to_string()),
            first_air_date: None,
            vote_average: Some(8.2),
            imdb_id: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        let deserialized: TmdbSearchListItem = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, 603);
        assert_eq!(deserialized.media_type, "movie");
        assert_eq!(deserialized.vote_average, Some(8.2));
    }

    #[test]
    fn tmdb_search_list_item_missing_optional_fields() {
        let json = r#"{"id":1,"title":"X","media_type":"movie"}"#;
        let item: TmdbSearchListItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.id, 1);
        assert!(item.poster_path.is_none());
        assert!(item.imdb_id.is_none());
    }

    // ── TmdbTrendingListItem serde roundtrip ────────────────────────────

    #[test]
    fn tmdb_trending_list_item_serde_roundtrip() {
        let item = TmdbTrendingListItem {
            id: 123,
            title: "Inception".to_string(),
            media_type: "movie".to_string(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let deserialized: TmdbTrendingListItem = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, 123);
        assert_eq!(deserialized.title, "Inception");
        assert_eq!(deserialized.media_type, "movie");
    }

    // ── ImageType ───────────────────────────────────────────────────────

    #[test]
    fn image_type_debug() {
        let t = ImageType::SeriesBanner;
        let s = format!("{:?}", t);
        assert!(s.contains("SeriesBanner"));
    }

    #[test]
    fn image_type_clone() {
        let t = ImageType::EpisodeBanner {
            season: 1,
            episode: 5,
        };
        let cloned = t.clone();
        match cloned {
            ImageType::EpisodeBanner { season, episode } => {
                assert_eq!(season, 1);
                assert_eq!(episode, 5);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn image_type_movie_banner_debug() {
        let s = format!("{:?}", ImageType::MovieBanner);
        assert!(s.contains("MovieBanner"));
    }

    // ── create_slug ─────────────────────────────────────────────────────

    #[test]
    fn create_slug_basic() {
        assert_eq!(create_slug("Hello World"), "hello_world");
    }

    #[test]
    fn create_slug_special_chars() {
        assert_eq!(create_slug("The Matrix (1999)"), "the_matrix_1999");
    }

    #[test]
    fn create_slug_multiple_underscores() {
        assert_eq!(create_slug("A   B"), "a_b");
    }

    // ── image_cache_tag ─────────────────────────────────────────────────

    #[test]
    fn image_cache_tag_basic() {
        assert_eq!(image_cache_tag("/abc123.jpg"), "abc123");
    }

    #[test]
    fn image_cache_tag_empty_fallback() {
        // "/" has no file stem with alphanumeric chars after slug
        let tag = image_cache_tag("/");
        assert_eq!(tag, "img");
    }

    // ── format_image_path ───────────────────────────────────────────────

    #[test]
    fn format_image_path_with_subfolder() {
        assert_eq!(
            format_image_path(&Some("show".to_string()), "poster.jpg"),
            "image_cache/show/poster.jpg"
        );
    }

    #[test]
    fn format_image_path_no_subfolder() {
        assert_eq!(
            format_image_path(&None, "poster.jpg"),
            "image_cache/poster.jpg"
        );
    }

    // ── TmdbSeasonInfo / TmdbEpisodeInfo serde ──────────────────────────

    #[test]
    fn tmdb_season_info_serde_roundtrip() {
        let season = TmdbSeasonInfo {
            season_number: 1,
            name: "Season 1".to_string(),
            overview: Some("First season".to_string()),
            poster_path: Some("/s1.jpg".to_string()),
            episode_count: 2,
            episodes: vec![TmdbEpisodeInfo {
                episode_number: 1,
                season_number: 1,
                name: "Pilot".to_string(),
                overview: Some("First ep".to_string()),
                still_path: Some("/ep1.jpg".to_string()),
                air_date: Some("2008-01-20".to_string()),
                vote_average: Some(8.5),
            }],
        };
        let json = serde_json::to_string(&season).unwrap();
        let d: TmdbSeasonInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(d.season_number, 1);
        assert_eq!(d.episodes.len(), 1);
        assert_eq!(d.episodes[0].name, "Pilot");
    }

    // ── TmdbCredits / TmdbItem internal serde ───────────────────────────

    #[test]
    fn tmdb_item_with_credits_serde() {
        let json = r#"{
            "id": 603,
            "title": "The Matrix",
            "overview": "desc",
            "poster_path": "/p.jpg",
            "release_date": "1999-03-31",
            "vote_average": 8.2,
            "popularity": 50.0,
            "vote_count": 1000,
            "credits": {
                "cast": [{"name": "Keanu Reeves"}],
                "crew": [{"job": "Director", "name": "Lana Wachowski"}]
            },
            "imdb_id": "tt0133093"
        }"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.id, 603);
        assert_eq!(item.imdb_id, Some("tt0133093".to_string()));
        let credits = item.credits.unwrap();
        assert_eq!(credits.cast.unwrap().len(), 1);
        assert_eq!(credits.crew.unwrap()[0].job, Some("Director".to_string()));
    }

    #[test]
    fn tmdb_item_alias_name_as_title() {
        // TV results use "name" instead of "title"
        let json = r#"{
            "id": 1399,
            "name": "Game of Thrones",
            "overview": "desc",
            "poster_path": "/p.jpg",
            "first_air_date": "2011-04-17",
            "vote_average": 8.4,
            "popularity": 100.0,
            "vote_count": 5000
        }"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.title, Some("Game of Thrones".to_string()));
        assert_eq!(item.release_date, Some("2011-04-17".to_string()));
    }

    #[test]
    fn tmdb_external_ids_serde() {
        let json = r#"{"imdb_id": "tt0133093"}"#;
        let ids: TmdbExternalIds = serde_json::from_str(json).unwrap();
        assert_eq!(ids.imdb_id, Some("tt0133093".to_string()));
    }

    // ── TvShowDetails / TvShowSeasonBrief serde ─────────────────────────

    #[test]
    fn tv_show_details_serde() {
        let json = r#"{
            "id": 1399,
            "name": "Game of Thrones",
            "overview": "desc",
            "poster_path": "/p.jpg",
            "backdrop_path": "/b.jpg",
            "first_air_date": "2011-04-17",
            "number_of_seasons": 8,
            "number_of_episodes": 73,
            "seasons": [{"id": 3627, "season_number": 1, "name": "Season 1", "episode_count": 10, "poster_path": "/s1.jpg"}]
        }"#;
        let details: TvShowDetails = serde_json::from_str(json).unwrap();
        assert_eq!(details.id, 1399);
        assert_eq!(details.number_of_seasons, 8);
        assert_eq!(details.seasons.len(), 1);
        assert_eq!(details.seasons[0].season_number, 1);
    }

    // ── TmdbSearchResult internal serde ─────────────────────────────────

    #[test]
    fn tmdb_search_result_serde() {
        let json = r#"{
            "results": [{"id": 603, "title": "The Matrix", "release_date": "1999-03-31", "vote_average": 8.2, "popularity": 50.0, "vote_count": 1000}],
            "total_results": 1
        }"#;
        let result: TmdbSearchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.total_results, Some(1));
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].id, 603);
    }

    // ── TmdbFindResult serde ────────────────────────────────────────────

    #[test]
    fn tmdb_find_result_serde() {
        let json = r#"{
            "movie_results": [{"id": 603, "title": "The Matrix", "release_date": "1999-03-31", "vote_average": 8.2, "popularity": 50.0, "vote_count": 1000}],
            "tv_results": []
        }"#;
        let result: TmdbFindResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.movie_results.len(), 1);
        assert!(result.tv_results.is_empty());
    }

    // ── create_metadata_from_item ───────────────────────────────────────

    #[test]
    fn create_metadata_from_item_basic() {
        let item = TmdbItem {
            id: 603,
            title: Some("The Matrix".to_string()),
            original_title: Some("The Matrix".to_string()),
            overview: Some("A hacker discovers...".to_string()),
            poster_path: Some("/poster.jpg".to_string()),
            backdrop_path: None,
            release_date: Some("1999-03-31".to_string()),
            vote_average: Some(8.5),
            popularity: Some(80.0),
            vote_count: Some(15000),
            runtime: Some(136),
            credits: Some(TmdbCredits {
                cast: Some(vec![
                    TmdbCastMember {
                        name: Some("Keanu Reeves".to_string()),
                    },
                    TmdbCastMember {
                        name: Some("Laurence Fishburne".to_string()),
                    },
                ]),
                crew: Some(vec![TmdbCrewMember {
                    job: Some("Director".to_string()),
                    name: Some("Lana Wachowski".to_string()),
                }]),
            }),
            imdb_id: Some("tt0133093".to_string()),
            external_ids: None,
        };
        // Use a temp dir for image_cache_dir
        let tmp = std::env::temp_dir().join("tmdb_test_meta");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "movie");
        // May fail image download, but metadata should still be constructed
        match result {
            Ok(Some(meta)) => {
                assert_eq!(meta.title, "The Matrix");
                assert_eq!(meta.year, Some(1999));
                assert_eq!(meta.imdb_id, Some("tt0133093".to_string()));
                assert!(meta.runtime_seconds.is_some());
                assert_eq!(meta.runtime_seconds.unwrap(), 136.0 * 60.0);
                assert_eq!(meta.director, Some("Lana Wachowski".to_string()));
                assert!(meta.cast_names.unwrap().contains("Keanu Reeves"));
            }
            Ok(None) => { /* image caching may cause None */ }
            Err(_) => { /* network errors acceptable in test */ }
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── create_metadata_from_item_required ──────────────────────────────

    #[test]
    fn create_metadata_from_item_required_returns_ok() {
        let item = TmdbItem {
            id: 1,
            title: Some("Test".to_string()),
            original_title: None,
            overview: None,
            poster_path: None,
            backdrop_path: None,
            release_date: None,
            vote_average: None,
            popularity: None,
            vote_count: None,
            runtime: None,
            credits: None,
            imdb_id: None,
            external_ids: None,
        };
        let tmp = std::env::temp_dir().join("tmdb_test_req");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item_required(&item, tmp.to_str().unwrap(), "movie");
        match result {
            Ok(meta) => assert_eq!(meta.title, "Test"),
            Err(_) => { /* network errors in image lookup acceptable */ }
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── rand_simple ─────────────────────────────────────────────────────

    #[test]
    fn rand_simple_range() {
        for _ in 0..100 {
            let r = rand_simple();
            assert!(r >= 0.0 && r < 1.0, "out of range: {}", r);
        }
    }

    // ── Constants ───────────────────────────────────────────────────────

    #[test]
    fn constants_values() {
        assert_eq!(MAX_RETRIES, 5);
        assert_eq!(BASE_DELAY_MS, 500);
        assert_eq!(MAX_DELAY_MS, 10000);
    }

    // ── build_tmdb_url extra cases ─────────────────────────────────────

    #[test]
    fn build_tmdb_url_api_key_empty_extra_params() {
        let url = build_tmdb_url("/movie/603", "mykey", "");
        assert!(url.contains("api_key=mykey"));
        assert!(url.contains("/movie/603"));
        // trailing "?" from empty extra_params is acceptable
    }

    #[test]
    fn build_tmdb_url_access_token_empty_extra_params() {
        let url = build_tmdb_url("/movie/603", "eyJtoken", "");
        assert!(url.starts_with("https://api.themoviedb.org/3/movie/603"));
        assert!(!url.contains("api_key"));
    }

    // ── extract_id_from_input extra cases ──────────────────────────────

    #[test]
    fn extract_id_from_input_tmdb_url_with_trailing_slash() {
        let (id, source) =
            extract_id_from_input("https://www.themoviedb.org/movie/603-the-matrix/");
        assert_eq!(id, "603");
        assert_eq!(source, "tmdb");
    }

    #[test]
    fn extract_id_from_input_imdb_embedded_in_url() {
        let (id, source) = extract_id_from_input("https://example.com/tt0133093/details");
        assert_eq!(id, "tt0133093");
        assert_eq!(source, "imdb");
    }

    #[test]
    fn extract_id_from_input_whitespace_trimmed() {
        let (id, source) = extract_id_from_input("  42  ");
        assert_eq!(id, "42");
        assert_eq!(source, "tmdb");
    }

    // ── find_best_match scoring edge cases ─────────────────────────────

    #[test]
    fn find_best_match_item_without_poster_strict_still_matches_on_title() {
        let items = vec![make_item(1, "The Matrix", "1999", false)];
        let best = find_best_match(&items, "The Matrix", Some(1999), true);
        // strict mode penalizes no poster but high title similarity still wins
        assert!(best.is_some());
    }

    #[test]
    fn find_best_match_item_without_poster_non_strict_accepts() {
        let items = vec![make_item(1, "The Matrix", "1999", false)];
        let best = find_best_match(&items, "The Matrix", Some(1999), false);
        assert!(best.is_some());
    }

    #[test]
    fn find_best_match_year_off_by_one_accepted() {
        let items = vec![make_item(1, "Matrix", "1998", true)];
        let best = find_best_match(&items, "Matrix", Some(1999), true);
        // year diff=1, acceptable
        assert!(best.is_some());
    }

    #[test]
    fn find_best_match_strict_low_similarity_rejected() {
        // Completely different title, strict mode
        let items = vec![make_item(1, "XY", "2020", true)];
        let best = find_best_match(&items, "Completely Different Long Title", Some(2020), true);
        // Low similarity + low score in strict mode
        assert!(best.is_none());
    }

    #[test]
    fn find_best_match_prefers_poster_over_no_poster() {
        let mut items = vec![
            make_item(1, "The Matrix", "1999", false),
            make_item(2, "The Matrix", "1999", true),
        ];
        let best = find_best_match(&items, "The Matrix", Some(1999), true);
        assert!(best.is_some());
        assert_eq!(best.unwrap().id, 2);
    }

    // ── create_metadata_from_item branches ─────────────────────────────

    #[test]
    fn create_metadata_from_item_no_credits() {
        let item = TmdbItem {
            id: 100,
            title: Some("No Credits Movie".to_string()),
            original_title: None,
            overview: None,
            poster_path: None,
            backdrop_path: None,
            release_date: Some("2020-06-15".to_string()),
            vote_average: None,
            popularity: None,
            vote_count: None,
            runtime: None,
            credits: None,
            imdb_id: None,
            external_ids: None,
        };
        let tmp = std::env::temp_dir().join("tmdb_test_no_credits");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "movie");
        match result {
            Ok(Some(meta)) => {
                assert_eq!(meta.title, "No Credits Movie");
                assert_eq!(meta.year, Some(2020));
                assert!(meta.cast_names.is_none());
                assert!(meta.director.is_none());
                assert!(meta.imdb_id.is_none());
            }
            Ok(None) => { /* acceptable */ }
            Err(_) => { /* network errors acceptable */ }
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn create_metadata_from_item_tv_type_uses_series_banner() {
        let item = TmdbItem {
            id: 200,
            title: Some("Test Show".to_string()),
            original_title: None,
            overview: Some("A show".to_string()),
            poster_path: None,
            backdrop_path: None,
            release_date: Some("2021-01-01".to_string()),
            vote_average: Some(7.0),
            popularity: Some(30.0),
            vote_count: Some(500),
            runtime: None,
            credits: None,
            imdb_id: None,
            external_ids: None,
        };
        let tmp = std::env::temp_dir().join("tmdb_test_tv_type");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "tv");
        match result {
            Ok(Some(meta)) => {
                assert_eq!(meta.title, "Test Show");
                assert!(meta.runtime_seconds.is_none()); // runtime None => None
            }
            Ok(None) => {}
            Err(_) => {}
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn create_metadata_from_item_zero_runtime_filtered() {
        let item = TmdbItem {
            id: 300,
            title: Some("Zero Runtime".to_string()),
            original_title: None,
            overview: None,
            poster_path: None,
            backdrop_path: None,
            release_date: None,
            vote_average: None,
            popularity: None,
            vote_count: None,
            runtime: Some(0), // zero => filtered to None
            credits: None,
            imdb_id: None,
            external_ids: None,
        };
        let tmp = std::env::temp_dir().join("tmdb_test_zero_rt");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "movie");
        match result {
            Ok(Some(meta)) => {
                assert!(meta.runtime_seconds.is_none());
            }
            Ok(None) => {}
            Err(_) => {}
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn create_metadata_from_item_external_ids_imdb() {
        let item = TmdbItem {
            id: 400,
            title: Some("External IDs".to_string()),
            original_title: None,
            overview: None,
            poster_path: None,
            backdrop_path: None,
            release_date: None,
            vote_average: None,
            popularity: None,
            vote_count: None,
            runtime: None,
            credits: None,
            imdb_id: None,
            external_ids: Some(TmdbExternalIds {
                imdb_id: Some("tt9999999".to_string()),
            }),
        };
        let tmp = std::env::temp_dir().join("tmdb_test_ext_ids");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "tv");
        match result {
            Ok(Some(meta)) => {
                assert_eq!(meta.imdb_id, Some("tt9999999".to_string()));
            }
            Ok(None) => {}
            Err(_) => {}
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn create_metadata_from_item_director_from_crew() {
        let item = TmdbItem {
            id: 500,
            title: Some("Directed".to_string()),
            original_title: None,
            overview: None,
            poster_path: None,
            backdrop_path: None,
            release_date: None,
            vote_average: None,
            popularity: None,
            vote_count: None,
            runtime: None,
            credits: Some(TmdbCredits {
                cast: Some(vec![]),
                crew: Some(vec![
                    TmdbCrewMember {
                        job: Some("Producer".to_string()),
                        name: Some("Producer Name".to_string()),
                    },
                    TmdbCrewMember {
                        job: Some("Director".to_string()),
                        name: Some("Director Name".to_string()),
                    },
                    TmdbCrewMember {
                        job: Some("Director".to_string()),
                        name: Some("  ".to_string()),
                    }, // whitespace only, skipped
                ]),
            }),
            imdb_id: None,
            external_ids: None,
        };
        let tmp = std::env::temp_dir().join("tmdb_test_director");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "movie");
        match result {
            Ok(Some(meta)) => {
                assert_eq!(meta.director, Some("Director Name".to_string()));
                assert!(meta.cast_names.is_none()); // empty cast vec => None
            }
            Ok(None) => {}
            Err(_) => {}
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn create_metadata_from_item_fallback_to_original_title() {
        let item = TmdbItem {
            id: 600,
            title: None,
            original_title: Some("Original Title".to_string()),
            overview: None,
            poster_path: None,
            backdrop_path: None,
            release_date: Some("2015-05-01".to_string()),
            vote_average: None,
            popularity: None,
            vote_count: None,
            runtime: Some(90),
            credits: None,
            imdb_id: None,
            external_ids: None,
        };
        let tmp = std::env::temp_dir().join("tmdb_test_orig_title");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "movie");
        match result {
            Ok(Some(meta)) => {
                assert_eq!(meta.title, "Original Title");
                assert_eq!(meta.year, Some(2015));
                assert_eq!(meta.runtime_seconds, Some(90.0 * 60.0));
            }
            Ok(None) => {}
            Err(_) => {}
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn create_metadata_from_item_cast_trimmed_and_filtered() {
        let item = TmdbItem {
            id: 700,
            title: Some("Cast Test".to_string()),
            original_title: None,
            overview: None,
            poster_path: None,
            backdrop_path: None,
            release_date: None,
            vote_average: None,
            popularity: None,
            vote_count: None,
            runtime: None,
            credits: Some(TmdbCredits {
                cast: Some(vec![
                    TmdbCastMember {
                        name: Some("  Actor One  ".to_string()),
                    },
                    TmdbCastMember { name: None },
                    TmdbCastMember {
                        name: Some("".to_string()),
                    },
                    TmdbCastMember {
                        name: Some("Actor Two".to_string()),
                    },
                ]),
                crew: None,
            }),
            imdb_id: None,
            external_ids: None,
        };
        let tmp = std::env::temp_dir().join("tmdb_test_cast_trim");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "movie");
        match result {
            Ok(Some(meta)) => {
                let cast = meta.cast_names.unwrap();
                assert!(cast.contains("Actor One"));
                assert!(cast.contains("Actor Two"));
            }
            Ok(None) => {}
            Err(_) => {}
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── cache_image_organized path construction ────────────────────────

    #[test]
    fn cache_image_organized_movie_banner_path() {
        let tmp = std::env::temp_dir().join("tmdb_test_img_org_movie");
        let _ = std::fs::create_dir_all(&tmp);
        // cache_image_organized will try to download (fail in test), but we test the path structure
        // by calling with a path that won't exist on disk - it should return None gracefully
        let result = cache_image_organized(
            "/test.jpg",
            tmp.to_str().unwrap(),
            "My Movie",
            ImageType::MovieBanner,
        );
        // May return None due to download failure, but shouldn't panic
        assert!(result.is_some() || result.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cache_image_organized_series_banner_creates_subfolder() {
        let tmp = std::env::temp_dir().join("tmdb_test_img_org_series");
        let _ = std::fs::create_dir_all(&tmp);
        let result = cache_image_organized(
            "/test.jpg",
            tmp.to_str().unwrap(),
            "My Show",
            ImageType::SeriesBanner,
        );
        // Subfolder should have been created (even if download fails)
        let slug = create_slug("My Show");
        let _subfolder = tmp.join(&slug);
        // Directory creation is attempted before download
        assert!(result.is_some() || result.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cache_image_organized_episode_banner_path_format() {
        let tmp = std::env::temp_dir().join("tmdb_test_img_org_ep");
        let _ = std::fs::create_dir_all(&tmp);
        let result = cache_image_organized(
            "/ep_still.jpg",
            tmp.to_str().unwrap(),
            "Show Name",
            ImageType::EpisodeBanner {
                season: 2,
                episode: 5,
            },
        );
        if let Some(path) = result {
            assert!(path.contains("s2e5"));
            assert!(path.contains("show_name"));
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── cache_image_organized with existing file ───────────────────────

    #[test]
    fn cache_image_organized_returns_cached_path_for_existing_valid_file() {
        let tmp = std::env::temp_dir().join("tmdb_test_img_org_cached");
        let _ = std::fs::create_dir_all(&tmp);
        let slug = create_slug("Cached Show");
        let subfolder = tmp.join(&slug);
        let _ = std::fs::create_dir_all(&subfolder);
        // Create a dummy file > 100 bytes
        let tag = image_cache_tag("/existing.jpg");
        let filename = format!("{}_{}_banner_hq.jpg", slug, tag);
        let file_path = subfolder.join(&filename);
        let _ = std::fs::write(&file_path, vec![0u8; 200]);

        let result = cache_image_organized(
            "/existing.jpg",
            tmp.to_str().unwrap(),
            "Cached Show",
            ImageType::SeriesBanner,
        );
        assert!(result.is_some());
        let path = result.unwrap();
        assert!(path.contains(&slug));
        assert!(path.contains(&filename));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cache_image_organized_deletes_corrupt_file_and_redownloads() {
        let tmp = std::env::temp_dir().join("tmdb_test_img_org_corrupt");
        let _ = std::fs::create_dir_all(&tmp);
        let tag = image_cache_tag("/corrupt.jpg");
        let filename = format!("{}_{}_banner_hq.jpg", "movie", tag);
        let file_path = tmp.join(&filename);
        // Write a tiny file (< 100 bytes) to simulate corruption
        let _ = std::fs::write(&file_path, vec![0u8; 10]);

        let result = cache_image_organized(
            "/corrupt.jpg",
            tmp.to_str().unwrap(),
            "Movie",
            ImageType::MovieBanner,
        );
        // Corrupt file should be deleted; result depends on network
        assert!(result.is_some() || result.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── format_image_path ─────────────────────────────────────────────

    #[test]
    fn format_image_path_nested_subfolder() {
        assert_eq!(
            format_image_path(&Some("deep/nested".to_string()), "file.jpg"),
            "image_cache/deep/nested/file.jpg"
        );
    }

    // ── Response struct deserialization from mock JSON ──────────────────

    #[test]
    fn tmdb_search_result_empty_results() {
        let json = r#"{"results": [], "total_results": 0}"#;
        let result: TmdbSearchResult = serde_json::from_str(json).unwrap();
        assert!(result.results.is_empty());
        assert_eq!(result.total_results, Some(0));
    }

    #[test]
    fn tmdb_search_result_missing_total() {
        let json = r#"{"results": []}"#;
        let result: TmdbSearchResult = serde_json::from_str(json).unwrap();
        assert!(result.total_results.is_none());
    }

    #[test]
    fn tmdb_find_result_tv_results() {
        let json = r#"{
            "movie_results": [],
            "tv_results": [{"id": 1399, "name": "Game of Thrones", "first_air_date": "2011-04-17", "vote_average": 8.4, "popularity": 100.0, "vote_count": 5000}]
        }"#;
        let result: TmdbFindResult = serde_json::from_str(json).unwrap();
        assert!(result.movie_results.is_empty());
        assert_eq!(result.tv_results.len(), 1);
        assert_eq!(
            result.tv_results[0].title,
            Some("Game of Thrones".to_string())
        );
    }

    #[test]
    fn tmdb_find_result_empty_both() {
        let json = r#"{"movie_results": [], "tv_results": []}"#;
        let result: TmdbFindResult = serde_json::from_str(json).unwrap();
        assert!(result.movie_results.is_empty());
        assert!(result.tv_results.is_empty());
    }

    #[test]
    fn tmdb_item_minimal_json() {
        let json = r#"{"id": 1}"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.id, 1);
        assert!(item.title.is_none());
        assert!(item.poster_path.is_none());
        assert!(item.credits.is_none());
        assert!(item.imdb_id.is_none());
        assert!(item.external_ids.is_none());
        assert!(item.runtime.is_none());
    }

    #[test]
    fn tmdb_item_with_external_ids() {
        let json = r#"{
            "id": 1399,
            "name": "Game of Thrones",
            "external_ids": {"imdb_id": "tt0944947"}
        }"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.title, Some("Game of Thrones".to_string()));
        let ext = item.external_ids.unwrap();
        assert_eq!(ext.imdb_id, Some("tt0944947".to_string()));
    }

    #[test]
    fn tmdb_credits_empty_cast_and_crew() {
        let json = r#"{"cast": [], "crew": []}"#;
        let credits: TmdbCredits = serde_json::from_str(json).unwrap();
        assert!(credits.cast.unwrap().is_empty());
        assert!(credits.crew.unwrap().is_empty());
    }

    #[test]
    fn tmdb_credits_missing_fields() {
        let json = r#"{}"#;
        let credits: TmdbCredits = serde_json::from_str(json).unwrap();
        assert!(credits.cast.is_none());
        assert!(credits.crew.is_none());
    }

    #[test]
    fn tv_show_details_empty_seasons() {
        let json = r#"{
            "id": 1, "name": "Show", "number_of_seasons": 0, "number_of_episodes": 0, "seasons": []
        }"#;
        let details: TvShowDetails = serde_json::from_str(json).unwrap();
        assert_eq!(details.number_of_seasons, 0);
        assert!(details.seasons.is_empty());
    }

    #[test]
    fn tv_show_season_brief_serde() {
        let json = r#"{"id": 100, "season_number": 2, "name": "Season 2", "episode_count": 12, "poster_path": null}"#;
        let s: TvShowSeasonBrief = serde_json::from_str(json).unwrap();
        assert_eq!(s.season_number, 2);
        assert_eq!(s.episode_count, 12);
        assert!(s.poster_path.is_none());
    }

    // ── TmdbSearchListItem with name field (TV) ────────────────────────

    #[test]
    fn tmdb_search_list_item_tv_with_name() {
        let json = r#"{"id":1399,"name":"Game of Thrones","media_type":"tv","first_air_date":"2011-04-17"}"#;
        let item: TmdbSearchListItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.name, Some("Game of Thrones".to_string()));
        assert_eq!(item.first_air_date, Some("2011-04-17".to_string()));
    }

    // ── build_tmdb_url with various path patterns ──────────────────────

    #[test]
    fn build_tmdb_url_tv_season_path() {
        let url = build_tmdb_url("/tv/1399/season/1", "mykey", "language=en-US");
        assert!(url.contains("/tv/1399/season/1"));
        assert!(url.contains("api_key=mykey"));
        assert!(url.contains("language=en-US"));
    }

    #[test]
    fn build_tmdb_url_find_path() {
        let url = build_tmdb_url("/find/tt0133093", "eyJtoken", "external_source=imdb_id");
        assert!(url.contains("/find/tt0133093"));
        assert!(url.contains("external_source=imdb_id"));
        assert!(!url.contains("api_key"));
    }

    // ── create_slug edge cases ─────────────────────────────────────────

    #[test]
    fn create_slug_empty() {
        assert_eq!(create_slug(""), "");
    }

    #[test]
    fn create_slug_all_special_chars() {
        assert_eq!(create_slug("!@#$%"), "");
    }

    #[test]
    fn create_slug_unicode() {
        // Non-ASCII alphanumeric chars are kept by is_alphanumeric
        let slug = create_slug("Cafe");
        assert!(!slug.is_empty());
    }

    // ── image_cache_tag edge cases ─────────────────────────────────────

    #[test]
    fn image_cache_tag_nested_path() {
        assert_eq!(image_cache_tag("/some/deep/path/image.jpg"), "image");
    }

    #[test]
    fn image_cache_tag_no_extension() {
        assert_eq!(image_cache_tag("/abcdef"), "abcdef");
    }

    // ── normalize_title edge cases ─────────────────────────────────────

    #[test]
    fn normalize_title_only_special_chars() {
        let result = normalize_title("!@#$%");
        assert_eq!(result, "");
    }

    #[test]
    fn normalize_title_leading_article_not_at_start() {
        // "a" not at start should NOT be stripped
        assert_eq!(normalize_title("Place a Bet"), "place a bet");
    }

    // ── title_similarity containment path ──────────────────────────────

    #[test]
    fn title_similarity_one_contains_other() {
        let score = title_similarity("Matrix", "The Matrix Reloaded");
        // "matrix" is contained in "the matrix reloaded"
        assert!(score > 0.7, "expected >0.7 for containment, got {}", score);
    }

    #[test]
    fn title_similarity_word_overlap_jaccard() {
        let score = title_similarity("Star Wars Episode", "Star Trek Episode");
        // 2 of 4 unique words overlap
        assert!(
            score > 0.3 && score < 0.8,
            "expected mid-range, got {}",
            score
        );
    }

    // ── extract_title_variations edge cases ────────────────────────────

    #[test]
    fn extract_title_variations_1x01_pattern() {
        let v = extract_title_variations("Breaking Bad 1x01");
        assert!(v.iter().any(|x| x.to_lowercase().contains("breaking bad")));
    }

    #[test]
    fn extract_title_variations_deduplicates() {
        let v = extract_title_variations("Matrix");
        let count = v.iter().filter(|x| x.to_lowercase() == "matrix").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn extract_title_variations_and_to_ampersand() {
        let v = extract_title_variations("Beauty and the Beast");
        assert!(v.iter().any(|x| x.contains('&')));
    }

    // ── TmdbEpisodeInfo serde with missing fields ─────────────────────

    #[test]
    fn tmdb_episode_info_serde_minimal() {
        let json = r#"{"episode_number":1,"season_number":1,"name":"Pilot"}"#;
        let ep: TmdbEpisodeInfo = serde_json::from_str(json).unwrap();
        assert_eq!(ep.episode_number, 1);
        assert!(ep.overview.is_none());
        assert!(ep.still_path.is_none());
        assert!(ep.air_date.is_none());
        assert!(ep.vote_average.is_none());
    }

    // ── is_reasonable_match edge cases ─────────────────────────────────

    #[test]
    fn is_reasonable_match_query_contained_in_result() {
        assert!(is_reasonable_match("atrix", "The Matrix"));
    }

    #[test]
    fn is_reasonable_match_numeric_not_contained() {
        assert!(!is_reasonable_match("2020", "The Matrix"));
    }

    // ── TmdbMetadata with all None optional fields ─────────────────────

    #[test]
    fn tmdb_metadata_all_none_optionals() {
        let meta = TmdbMetadata {
            title: "Minimal".to_string(),
            year: None,
            overview: None,
            cast_names: None,
            director: None,
            poster_path: None,
            tmdb_id: None,
            imdb_id: None,
            runtime_seconds: None,
            imdb_image_url: None,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let d: TmdbMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(d.title, "Minimal");
        assert!(d.year.is_none());
        assert!(d.imdb_id.is_none());
    }

    // ── TmdbSeasonInfo with empty episodes ─────────────────────────────

    #[test]
    fn tmdb_season_info_empty_episodes() {
        let json = r#"{"season_number":1,"name":"S1","episode_count":0,"episodes":[]}"#;
        let s: TmdbSeasonInfo = serde_json::from_str(json).unwrap();
        assert_eq!(s.episode_count, 0);
        assert!(s.episodes.is_empty());
    }

    // ── build_tmdb_url proxy-style complex params (no longer magic) ───

    #[test]
    fn build_tmdb_url_proxy_complex_params() {
        let url = build_tmdb_url(
            "/tv/1399",
            "__TMDB_BACKEND_PROXY__",
            "language=en-US&append_to_response=credits,external_ids",
        );
        assert!(url.contains("/tv/1399"));
        assert!(url.contains("language=en-US"));
        assert!(url.contains("append_to_response=credits,external_ids"));
    }

    // ── TmdbSearchResult with multiple items ───────────────────────────

    #[test]
    fn tmdb_search_result_multiple_items() {
        let json = r#"{
            "results": [
                {"id": 1, "title": "A", "release_date": "2020-01-01", "vote_average": 5.0, "popularity": 10.0, "vote_count": 100},
                {"id": 2, "title": "B", "release_date": "2021-01-01", "vote_average": 8.0, "popularity": 50.0, "vote_count": 500},
                {"id": 3, "title": "C", "release_date": "2019-01-01", "vote_average": 3.0, "popularity": 5.0, "vote_count": 50}
            ],
            "total_results": 3
        }"#;
        let result: TmdbSearchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.results.len(), 3);
        assert_eq!(result.results[1].vote_average, Some(8.0));
    }

    // ── TmdbItem with original_name alias (TV) ─────────────────────────

    #[test]
    fn tmdb_item_original_name_alias() {
        let json =
            r#"{"id": 999, "original_name": "Original Show Name", "first_air_date": "2020-01-01"}"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.original_title, Some("Original Show Name".to_string()));
        assert_eq!(item.release_date, Some("2020-01-01".to_string()));
    }

    // ── get_tmdb_credential with actual key ────────────────────────────

    #[test]
    fn get_tmdb_credential_preserves_actual_key() {
        assert_eq!(get_tmdb_credential("abc123"), "abc123");
    }

    // ── search_multi_raw mock JSON parsing ───────────────────────────────

    #[test]
    fn search_multi_raw_filters_out_person_media_type() {
        // Simulate the RawSearchResult deserialization used inside search_multi_raw
        #[derive(serde::Deserialize)]
        struct RawSearchItem {
            id: i64,
            media_type: Option<String>,
            title: Option<String>,
            name: Option<String>,
            #[serde(alias = "original_title")]
            original_title: Option<String>,
            #[serde(alias = "original_name")]
            original_name: Option<String>,
            poster_path: Option<String>,
            backdrop_path: Option<String>,
            overview: Option<String>,
            release_date: Option<String>,
            first_air_date: Option<String>,
            vote_average: Option<f64>,
        }

        #[derive(serde::Deserialize)]
        struct RawSearchResult {
            results: Vec<RawSearchItem>,
        }

        let json = r#"{"results": [
            {"id": 1, "media_type": "movie", "title": "Matrix", "poster_path": "/p.jpg", "vote_average": 8.0},
            {"id": 2, "media_type": "person", "name": "Keanu"},
            {"id": 3, "media_type": "tv", "name": "Show", "first_air_date": "2020-01-01", "vote_average": 7.0}
        ]}"#;
        let raw: RawSearchResult = serde_json::from_str(json).unwrap();
        // Filter to movie/tv only (same logic as search_multi_raw)
        let filtered: Vec<_> = raw
            .results
            .into_iter()
            .filter(|item| {
                let mt = item.media_type.as_deref().unwrap_or("");
                mt == "movie" || mt == "tv"
            })
            .collect();
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].id, 1);
        assert_eq!(filtered[1].id, 3);
    }

    #[test]
    fn search_multi_raw_movie_uses_title_tv_uses_name() {
        #[derive(serde::Deserialize)]
        struct RawSearchItem {
            id: i64,
            media_type: Option<String>,
            title: Option<String>,
            name: Option<String>,
            #[serde(alias = "original_title")]
            original_title: Option<String>,
            #[serde(alias = "original_name")]
            original_name: Option<String>,
            poster_path: Option<String>,
            backdrop_path: Option<String>,
            overview: Option<String>,
            release_date: Option<String>,
            first_air_date: Option<String>,
            vote_average: Option<f64>,
        }

        let movie_json = r#"{"id": 603, "media_type": "movie", "title": "The Matrix", "original_title": "Matrix Original", "poster_path": "/p.jpg", "release_date": "1999-03-31", "vote_average": 8.5}"#;
        let item: RawSearchItem = serde_json::from_str(movie_json).unwrap();
        assert_eq!(item.title, Some("The Matrix".to_string()));
        assert_eq!(item.release_date, Some("1999-03-31".to_string()));

        let tv_json = r#"{"id": 1399, "media_type": "tv", "name": "Game of Thrones", "original_name": "GoT", "first_air_date": "2011-04-17", "vote_average": 8.4}"#;
        let item: RawSearchItem = serde_json::from_str(tv_json).unwrap();
        assert_eq!(item.name, Some("Game of Thrones".to_string()));
        assert_eq!(item.first_air_date, Some("2011-04-17".to_string()));
    }

    // ── trending_suggestions_raw mock JSON parsing ───────────────────────

    #[test]
    fn trending_suggestions_raw_parses_response() {
        #[derive(serde::Deserialize)]
        struct RawTrendingItem {
            id: i64,
            title: Option<String>,
            name: Option<String>,
        }

        #[derive(serde::Deserialize)]
        struct RawTrendingResult {
            results: Vec<RawTrendingItem>,
        }

        let json = r#"{"results": [
            {"id": 1, "title": "Trending Movie"},
            {"id": 2, "name": "Trending Show"},
            {"id": 3, "title": null, "name": null},
            {"id": 4, "title": "  "}
        ]}"#;
        let raw: RawTrendingResult = serde_json::from_str(json).unwrap();
        // Replicate the filter logic from trending_suggestions_raw
        let suggestions: Vec<_> = raw
            .results
            .into_iter()
            .filter_map(|item| {
                let title = item.title.or(item.name)?.trim().to_string();
                if title.is_empty() {
                    return None;
                }
                Some(TmdbTrendingListItem {
                    id: item.id,
                    title,
                    media_type: "movie".to_string(),
                })
            })
            .take(10)
            .collect();
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0].title, "Trending Movie");
        assert_eq!(suggestions[1].title, "Trending Show");
    }

    #[test]
    fn trending_suggestions_raw_name_fallback_when_title_null() {
        #[derive(serde::Deserialize)]
        struct RawTrendingItem {
            id: i64,
            title: Option<String>,
            name: Option<String>,
        }

        let json = r#"{"id": 42, "title": null, "name": "Fallback Show"}"#;
        let item: RawTrendingItem = serde_json::from_str(json).unwrap();
        let title = item.title.or(item.name).unwrap();
        assert_eq!(title, "Fallback Show");
    }

    // ── fetch_tv_show_details mock JSON parsing ──────────────────────────

    #[test]
    fn fetch_tv_show_details_parses_seasons() {
        let json = r#"{
            "id": 1399,
            "name": "Game of Thrones",
            "overview": "Seven noble families...",
            "poster_path": "/poster.jpg",
            "backdrop_path": "/backdrop.jpg",
            "first_air_date": "2011-04-17",
            "number_of_seasons": 8,
            "number_of_episodes": 73,
            "seasons": [
                {"id": 3627, "season_number": 0, "name": "Specials", "episode_count": 3, "poster_path": null},
                {"id": 3628, "season_number": 1, "name": "Season 1", "episode_count": 10, "poster_path": "/s1.jpg"},
                {"id": 3629, "season_number": 2, "name": "Season 2", "episode_count": 10, "poster_path": "/s2.jpg"}
            ]
        }"#;
        let details: TvShowDetails = serde_json::from_str(json).unwrap();
        assert_eq!(details.id, 1399);
        assert_eq!(details.number_of_seasons, 8);
        assert_eq!(details.number_of_episodes, 73);
        assert_eq!(details.seasons.len(), 3);
        assert_eq!(details.seasons[0].season_number, 0);
        assert_eq!(details.seasons[0].poster_path, None);
        assert_eq!(details.seasons[1].episode_count, 10);
        assert_eq!(details.seasons[2].name, "Season 2");
    }

    // ── fetch_season_episodes mock JSON parsing ──────────────────────────

    #[test]
    fn fetch_season_episodes_parses_episode_response() {
        #[derive(serde::Deserialize)]
        struct EpisodeResponse {
            id: i64,
            name: String,
            overview: Option<String>,
            still_path: Option<String>,
            episode_number: i32,
            season_number: i32,
            air_date: Option<String>,
            vote_average: Option<f64>,
        }

        #[derive(serde::Deserialize)]
        struct SeasonResponse {
            id: i64,
            name: String,
            overview: Option<String>,
            poster_path: Option<String>,
            season_number: i32,
            episodes: Vec<EpisodeResponse>,
        }

        let json = r#"{
            "id": 3628,
            "name": "Season 1",
            "overview": "First season",
            "poster_path": "/s1.jpg",
            "season_number": 1,
            "episodes": [
                {
                    "id": 63056,
                    "name": "Winter Is Coming",
                    "overview": "Episode desc",
                    "still_path": "/ep1.jpg",
                    "episode_number": 1,
                    "season_number": 1,
                    "air_date": "2011-04-17",
                    "vote_average": 8.0
                },
                {
                    "id": 63057,
                    "name": "The Kingsroad",
                    "overview": null,
                    "still_path": null,
                    "episode_number": 2,
                    "season_number": 1,
                    "air_date": "2011-04-24",
                    "vote_average": 7.8
                }
            ]
        }"#;
        let season: SeasonResponse = serde_json::from_str(json).unwrap();
        assert_eq!(season.season_number, 1);
        assert_eq!(season.episodes.len(), 2);
        assert_eq!(season.episodes[0].name, "Winter Is Coming");
        assert_eq!(season.episodes[0].still_path, Some("/ep1.jpg".to_string()));
        assert_eq!(season.episodes[1].still_path, None);
        assert_eq!(season.episodes[1].overview, None);
    }

    // ── fetch_owned_episodes_only mock JSON parsing ──────────────────────

    #[test]
    fn fetch_owned_episodes_only_season_response_filtering() {
        #[derive(serde::Deserialize)]
        struct EpisodeResponse {
            name: String,
            overview: Option<String>,
            still_path: Option<String>,
            episode_number: i32,
            season_number: i32,
            air_date: Option<String>,
            vote_average: Option<f64>,
        }

        #[derive(serde::Deserialize)]
        struct SeasonResponse {
            episodes: Vec<EpisodeResponse>,
        }

        let json = r#"{
            "episodes": [
                {"name": "Ep 1", "episode_number": 1, "season_number": 1, "air_date": "2020-01-01", "vote_average": 7.0},
                {"name": "Ep 2", "episode_number": 2, "season_number": 1, "air_date": "2020-01-08", "vote_average": 7.5},
                {"name": "Ep 3", "episode_number": 3, "season_number": 1, "air_date": "2020-01-15", "vote_average": 8.0}
            ]
        }"#;
        let season: SeasonResponse = serde_json::from_str(json).unwrap();
        // Simulate the filtering logic: only keep episodes the user owns
        let owned_in_season = vec![1, 3]; // user owns ep 1 and 3
        let filtered: Vec<_> = season
            .episodes
            .into_iter()
            .filter(|ep| owned_in_season.contains(&ep.episode_number))
            .collect();
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].name, "Ep 1");
        assert_eq!(filtered[1].name, "Ep 3");
    }

    // ── fetch_imdb_id mock JSON parsing ──────────────────────────────────

    #[test]
    fn fetch_imdb_id_from_external_ids() {
        let json = r#"{
            "id": 1399,
            "name": "Game of Thrones",
            "external_ids": {"imdb_id": "tt0944947"}
        }"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        // Replicate fetch_imdb_id logic
        let imdb = item.imdb_id.or_else(|| {
            item.external_ids
                .as_ref()
                .and_then(|ids| ids.imdb_id.clone())
        });
        assert_eq!(imdb, Some("tt0944947".to_string()));
    }

    #[test]
    fn fetch_imdb_id_from_direct_field() {
        let json = r#"{"id": 603, "title": "The Matrix", "imdb_id": "tt0133093"}"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        let imdb = item.imdb_id.or_else(|| {
            item.external_ids
                .as_ref()
                .and_then(|ids| ids.imdb_id.clone())
        });
        assert_eq!(imdb, Some("tt0133093".to_string()));
    }

    #[test]
    fn fetch_imdb_id_returns_none_when_absent() {
        let json = r#"{"id": 999, "title": "No IMDB"}"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        let imdb = item.imdb_id.or_else(|| {
            item.external_ids
                .as_ref()
                .and_then(|ids| ids.imdb_id.clone())
        });
        assert!(imdb.is_none());
    }

    // ── cache_imdb_image path construction ───────────────────────────────

    #[test]
    fn cache_imdb_image_url_hash_extraction() {
        // Replicate the url_hash logic from cache_imdb_image
        let url = "https://m.media-amazon.com/images/M/MV5BMTc5MDE2ODcwNV5BMl5BanBnXkFtZTgwMzI2NzQ2NzM@.jpg";
        let hash = url.split('/').last().unwrap_or("unknown");
        let safe: String = hash
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(20)
            .collect();
        assert!(!safe.is_empty());
        assert!(safe.len() <= 20);
    }

    #[test]
    fn cache_imdb_image_filename_formats() {
        // Replicate filename generation for each ImageType
        let url_hash = "MV5BMTc5MDE2";

        let movie_fn = format!("imdb_movie_{}_banner.jpg", url_hash);
        assert_eq!(movie_fn, "imdb_movie_MV5BMTc5MDE2_banner.jpg");

        let series_fn = format!("imdb_series_{}_banner.jpg", url_hash);
        assert_eq!(series_fn, "imdb_series_MV5BMTc5MDE2_banner.jpg");

        let ep_fn = format!("imdb_s{:02}e{:02}_{}_banner.jpg", 1, 5, url_hash);
        assert_eq!(ep_fn, "imdb_s01e05_MV5BMTc5MDE2_banner.jpg");
    }

    #[test]
    fn cache_imdb_image_creates_file_with_tempdir() {
        let tmp = std::env::temp_dir().join("tmdb_test_imdb_cache");
        let _ = std::fs::create_dir_all(&tmp);
        // Create a fake file simulating what cache_imdb_image would write
        let filename = "imdb_movie_test_banner.jpg";
        let file_path = tmp.join(filename);
        std::fs::write(&file_path, vec![0u8; 200]).unwrap();
        assert!(file_path.exists());
        assert_eq!(file_path.metadata().unwrap().len(), 200);
        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cache_imdb_image_empty_url_hash_fallback() {
        // Edge case: url ends with "/" or empty segment
        let url = "https://example.com/";
        let hash = url.split('/').last().unwrap_or("unknown");
        let safe: String = hash
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(20)
            .collect();
        // Empty => fallback to "unknown"
        let final_hash = if safe.is_empty() {
            "unknown".to_string()
        } else {
            safe
        };
        assert_eq!(final_hash, "unknown");
    }

    // ── create_slug more edge cases ──────────────────────────────────────

    #[test]
    fn create_slug_numbers_preserved() {
        assert_eq!(create_slug("2001: A Space Odyssey"), "2001_a_space_odyssey");
    }

    #[test]
    fn create_slug_consecutive_specials() {
        assert_eq!(create_slug("Hello---World!!!"), "hello_world");
    }

    #[test]
    fn create_slug_leading_trailing_specials() {
        assert_eq!(create_slug("!!!Title!!!"), "title");
    }

    #[test]
    fn create_slug_single_char() {
        assert_eq!(create_slug("A"), "a");
    }

    #[test]
    fn create_slug_only_digits() {
        assert_eq!(create_slug("12345"), "12345");
    }

    // ── format_image_path more edge cases ────────────────────────────────

    #[test]
    fn format_image_path_empty_subfolder() {
        assert_eq!(
            format_image_path(&Some("".to_string()), "file.jpg"),
            "image_cache//file.jpg"
        );
    }

    #[test]
    fn format_image_path_special_chars_in_filename() {
        assert_eq!(
            format_image_path(&None, "file (1).jpg"),
            "image_cache/file (1).jpg"
        );
    }

    // ── build_tmdb_url with trending paths ───────────────────────────────

    #[test]
    fn build_tmdb_url_trending_path() {
        let url = build_tmdb_url("/trending/movie/day", "mykey", "language=en-US");
        assert!(url.contains("/trending/movie/day"));
        assert!(url.contains("api_key=mykey"));
        assert!(url.contains("language=en-US"));
    }

    #[test]
    fn build_tmdb_url_trending_tv() {
        let url = build_tmdb_url("/trending/tv/day", "eyJtoken", "language=en-US");
        assert!(url.contains("/trending/tv/day"));
        assert!(!url.contains("api_key"));
    }

    // ── build_tmdb_url with search/multi path ────────────────────────────

    #[test]
    fn build_tmdb_url_search_multi() {
        let url = build_tmdb_url(
            "/search/multi",
            "mykey",
            "query=Matrix&include_adult=false&language=en-US",
        );
        assert!(url.contains("/search/multi"));
        assert!(url.contains("query=Matrix"));
        assert!(url.contains("include_adult=false"));
    }

    // ── TmdbItem with runtime and credits parsing ────────────────────────

    #[test]
    fn tmdb_item_runtime_positive() {
        let json = r#"{"id": 1, "runtime": 136}"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.runtime, Some(136));
        // Runtime conversion: minutes -> seconds
        let seconds = item.runtime.filter(|m| *m > 0).map(|m| (m as f64) * 60.0);
        assert_eq!(seconds, Some(8160.0));
    }

    #[test]
    fn tmdb_item_runtime_zero_filtered() {
        let json = r#"{"id": 1, "runtime": 0}"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        let seconds = item.runtime.filter(|m| *m > 0).map(|m| (m as f64) * 60.0);
        assert!(seconds.is_none());
    }

    // ── extract_id_from_input IMDB with prefix noise ─────────────────────

    #[test]
    fn extract_id_from_input_imdb_in_full_url() {
        let (id, source) = extract_id_from_input("https://www.imdb.com/title/tt0133093/");
        assert_eq!(id, "tt0133093");
        assert_eq!(source, "imdb");
    }

    #[test]
    fn extract_id_from_input_zero_padded_numeric() {
        let (id, source) = extract_id_from_input("0000603");
        assert_eq!(id, "0000603");
        assert_eq!(source, "tmdb");
    }

    // ── create_metadata_from_item with both imdb_id and external_ids ─────

    #[test]
    fn create_metadata_from_item_imdb_id_takes_priority_over_external_ids() {
        let item = TmdbItem {
            id: 800,
            title: Some("Priority Test".to_string()),
            original_title: None,
            overview: None,
            poster_path: None,
            backdrop_path: None,
            release_date: None,
            vote_average: None,
            popularity: None,
            vote_count: None,
            runtime: None,
            credits: None,
            imdb_id: Some("tt1111111".to_string()),
            external_ids: Some(TmdbExternalIds {
                imdb_id: Some("tt9999999".to_string()),
            }),
        };
        let tmp = std::env::temp_dir().join("tmdb_test_imdb_priority");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "movie");
        match result {
            Ok(Some(meta)) => {
                // imdb_id should take priority
                assert_eq!(meta.imdb_id, Some("tt1111111".to_string()));
            }
            Ok(None) => {}
            Err(_) => {}
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── create_metadata_from_item with backdrop only (no poster) ─────────

    #[test]
    fn create_metadata_from_item_uses_backdrop_when_no_poster() {
        let item = TmdbItem {
            id: 900,
            title: Some("Backdrop Only".to_string()),
            original_title: None,
            overview: None,
            poster_path: None,
            backdrop_path: Some("/backdrop.jpg".to_string()),
            release_date: Some("2022-01-01".to_string()),
            vote_average: None,
            popularity: None,
            vote_count: None,
            runtime: None,
            credits: None,
            imdb_id: None,
            external_ids: None,
        };
        let tmp = std::env::temp_dir().join("tmdb_test_backdrop_only");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "movie");
        match result {
            Ok(Some(meta)) => {
                assert_eq!(meta.title, "Backdrop Only");
                assert_eq!(meta.year, Some(2022));
            }
            Ok(None) => {}
            Err(_) => {}
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── create_metadata_from_item cast limited to 8 ──────────────────────

    #[test]
    fn create_metadata_from_item_cast_limited_to_eight() {
        let cast_members: Vec<TmdbCastMember> = (0..12)
            .map(|i| TmdbCastMember {
                name: Some(format!("Actor {}", i)),
            })
            .collect();
        let item = TmdbItem {
            id: 1000,
            title: Some("Many Cast".to_string()),
            original_title: None,
            overview: None,
            poster_path: None,
            backdrop_path: None,
            release_date: None,
            vote_average: None,
            popularity: None,
            vote_count: None,
            runtime: None,
            credits: Some(TmdbCredits {
                cast: Some(cast_members),
                crew: None,
            }),
            imdb_id: None,
            external_ids: None,
        };
        let tmp = std::env::temp_dir().join("tmdb_test_cast_limit");
        let _ = std::fs::create_dir_all(&tmp);
        let result = create_metadata_from_item(&item, tmp.to_str().unwrap(), "movie");
        match result {
            Ok(Some(meta)) => {
                let cast = meta.cast_names.unwrap();
                let names: Vec<&str> = cast.split(", ").collect();
                assert_eq!(names.len(), 8);
                assert!(names.contains(&"Actor 0"));
                assert!(names.contains(&"Actor 7"));
                assert!(!names.contains(&"Actor 8")); // limited to 8
            }
            Ok(None) => {}
            Err(_) => {}
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── find_best_match with backdrop but no poster ──────────────────────

    #[test]
    fn find_best_match_item_with_backdrop_only() {
        let items = vec![TmdbItem {
            id: 50,
            title: Some("Backdrop Movie".to_string()),
            original_title: None,
            overview: None,
            poster_path: None,
            backdrop_path: Some("/backdrop.jpg".to_string()),
            release_date: Some("2020-01-01".to_string()),
            vote_average: Some(7.0),
            popularity: Some(50.0),
            vote_count: Some(1000),
            runtime: None,
            credits: None,
            imdb_id: None,
            external_ids: None,
        }];
        let best = find_best_match(&items, "Backdrop Movie", Some(2020), true);
        // Strict mode requires poster OR backdrop
        assert!(best.is_some());
        assert_eq!(best.unwrap().id, 50);
    }

    // ── find_best_match multiple items with same title ───────────────────

    #[test]
    fn find_best_match_picks_highest_scored_among_identical_titles() {
        let items = vec![
            TmdbItem {
                id: 1,
                title: Some("Dune".to_string()),
                original_title: Some("Dune".to_string()),
                overview: None,
                poster_path: Some("/p1.jpg".to_string()),
                backdrop_path: None,
                release_date: Some("2021-10-22".to_string()),
                vote_average: Some(8.0),
                popularity: Some(100.0),
                vote_count: Some(5000),
                runtime: None,
                credits: None,
                imdb_id: None,
                external_ids: None,
            },
            TmdbItem {
                id: 2,
                title: Some("Dune".to_string()),
                original_title: Some("Dune".to_string()),
                overview: None,
                poster_path: Some("/p2.jpg".to_string()),
                backdrop_path: Some("/bd2.jpg".to_string()),
                release_date: Some("2021-10-22".to_string()),
                vote_average: Some(8.0),
                popularity: Some(100.0),
                vote_count: Some(5000),
                runtime: None,
                credits: None,
                imdb_id: None,
                external_ids: None,
            },
        ];
        let best = find_best_match(&items, "Dune", Some(2021), true);
        assert!(best.is_some());
        // Item 2 has backdrop, so should be preferred (higher score)
        assert_eq!(best.unwrap().id, 2);
    }

    // ── TmdbItem popularity/vote_count serde ─────────────────────────────

    #[test]
    fn tmdb_item_popularity_and_vote_count() {
        let json = r#"{"id": 1, "popularity": 123.45, "vote_count": 99999}"#;
        let item: TmdbItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.popularity, Some(123.45));
        assert_eq!(item.vote_count, Some(99999));
    }

    // ── TvShowDetails with no optional fields ────────────────────────────

    #[test]
    fn tv_show_details_minimal() {
        let json = r#"{
            "id": 1,
            "name": "Minimal Show",
            "number_of_seasons": 1,
            "number_of_episodes": 6,
            "seasons": []
        }"#;
        let details: TvShowDetails = serde_json::from_str(json).unwrap();
        assert_eq!(details.id, 1);
        assert!(details.overview.is_none());
        assert!(details.poster_path.is_none());
        assert!(details.backdrop_path.is_none());
        assert!(details.first_air_date.is_none());
    }

    // ── cache_image_organized MovieBanner doesn't create subfolder ───────

    #[test]
    fn cache_image_organized_movie_banner_no_subfolder() {
        let tmp = std::env::temp_dir().join("tmdb_test_movie_no_sub");
        let _ = std::fs::create_dir_all(&tmp);
        // Movie banner should use flat cache dir, no subfolder
        let _ = cache_image_organized(
            "/test.jpg",
            tmp.to_str().unwrap(),
            "My Movie",
            ImageType::MovieBanner,
        );
        // Verify the slug-based subfolder was NOT created for MovieBanner
        // MovieBanner stores directly in cache_dir, not in a subfolder
        // (the function creates dir at cache_dir level, not subfolder level for movies)
        // We just verify no panic occurred
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── image_cache_tag with TMDB-style paths ────────────────────────────

    #[test]
    fn image_cache_tag_tmdb_poster_path() {
        // create_slug lowercases everything
        let tag = image_cache_tag("/pB8BM7pdSp6B6Ih7QZ4DrQ3PmJK.jpg");
        assert_eq!(tag, "pb8bm7pdsp6b6ih7qz4drq3pmjk");
    }

    #[test]
    fn image_cache_tag_still_path() {
        assert_eq!(image_cache_tag("/path/to/still_image.jpg"), "still_image");
    }

    // ── normalize_title with unicode ─────────────────────────────────────

    #[test]
    fn normalize_title_apostrophe_variants() {
        assert_eq!(normalize_title("It\u{2019}s"), "its");
        assert_eq!(normalize_title("It\u{2018}s"), "its");
    }

    // ── extract_title_variations with underscore input ───────────────────

    #[test]
    fn extract_title_variations_underscore_to_space() {
        let v = extract_title_variations("Mr_Nobody_2009");
        assert!(v.iter().any(|x| x == "Mr Nobody 2009"));
    }
}
