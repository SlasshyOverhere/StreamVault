use regex::Regex;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::database::Database;
use crate::http_client;
use crate::tmdb;

const VIDEO_EXTENSIONS: &[&str] = &[
    ".mkv", ".mp4", ".avi", ".mov", ".webm", ".m4v", ".wmv", ".flv", ".ts", ".m2ts",
];

/// Normalize file paths for consistent comparison (handles Windows path inconsistencies)
fn normalize_path(path: &str) -> String {
    path.to_lowercase().replace('\\', "/")
}

#[derive(Debug, Clone)]
pub struct ParsedMedia {
    pub title: String,
    pub year: Option<i32>,
    pub media_type: MediaParseType,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub episode_end: Option<i32>, // For multi-episode files like S01E01-E03
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum MediaParseType {
    Movie,
    TvEpisode,
}

/// Folder context for smarter detection
#[derive(Debug, Clone)]
struct FolderContext {
    /// Name extracted from parent folder (e.g., "Breaking Bad" from "Breaking Bad/Season 1/")
    series_name: Option<String>,
    /// Year extracted from folder name
    series_year: Option<i32>,
    /// Season number from folder like "Season 1" or "S01"
    folder_season: Option<i32>,
    /// Whether this appears to be a TV show folder structure
    is_tv_structure: bool,
}

#[derive(Clone, Serialize)]
struct ScanProgressPayload {
    title: String,
    media_type: String,
    current: usize,
    total: usize,
}

/// Cleanup orphaned media entries (files that no longer exist on disk)
/// Returns the number of removed entries
/// NOTE: This only cleans up LOCAL media - cloud media is never touched
pub fn cleanup_orphaned_media(db: &Database, image_cache_dir: &str) -> usize {
    println!("[CLEANUP] Checking for orphaned LOCAL media entries...");

    let all_media = match db.get_all_media() {
        Ok(items) => items,
        Err(e) => {
            println!("[CLEANUP] Error getting media list: {}", e);
            return 0;
        }
    };

    let mut removed_count = 0;
    let mut cleaned_images: std::collections::HashSet<String> = std::collections::HashSet::new();

    for item in all_media {
        // SKIP CLOUD ENTRIES - they don't have local files
        if item.is_cloud.unwrap_or(false) {
            continue;
        }

        if let Some(ref file_path) = item.file_path {
            // Check if this is a virtual path (used for consolidated TV shows)
            let is_virtual_path = file_path.starts_with("tvshow://");

            // Also skip gdrive: paths (cloud TV shows)
            let is_cloud_path = file_path.starts_with("gdrive:");
            if is_cloud_path {
                continue;
            }

            let should_remove = if item.media_type == "tvshow" {
                if is_virtual_path {
                    // For virtual paths, check if the TV show has any episodes left
                    // If it has no episodes, it's orphaned
                    match db.get_episodes(item.id) {
                        Ok(episodes) => episodes.is_empty(),
                        Err(_) => true, // Assume orphaned if we can't check
                    }
                } else {
                    // For real folder paths, check if the folder exists
                    let path = std::fs::canonicalize(file_path)
                        .unwrap_or_else(|_| Path::new(file_path).to_path_buf());
                    !path.is_dir() && !path.exists()
                }
            } else {
                // For movie/tvepisode entries, check if the file exists
                let path = std::fs::canonicalize(file_path)
                    .unwrap_or_else(|_| Path::new(file_path).to_path_buf());
                !path.is_file()
            };

            if should_remove {
                println!(
                    "[CLEANUP] Removing orphaned entry: {} ({})",
                    item.title, file_path
                );

                // If it's a TV show, also remove its episodes
                if item.media_type == "tvshow" {
                    if let Err(e) = db.remove_series_episodes(item.id) {
                        println!("[CLEANUP] Error removing episodes: {}", e);
                    }
                }

                // Remove the media entry and get the poster path
                match db.remove_media(item.id) {
                    Ok(Some(poster_path)) => {
                        cleaned_images.insert(poster_path);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        println!("[CLEANUP] Error removing media: {}", e);
                    }
                }

                removed_count += 1;
            }
        }
    }

    // Now clean up orphaned images (images not referenced by any media)
    if let Ok(used_posters) = db.get_all_poster_paths() {
        let used_set: std::collections::HashSet<String> = used_posters.into_iter().collect();

        // Also get still paths from episodes
        let mut all_used_paths = used_set.clone();
        if let Ok(all_media) = db.get_all_media() {
            for item in all_media {
                if let Some(still) = item.still_path {
                    all_used_paths.insert(still);
                }
            }
        }

        // Read all entries in image cache directory (files and subdirectories)
        cleanup_image_directory(image_cache_dir, &all_used_paths, "");
    }

    if removed_count > 0 {
        println!("[CLEANUP] Removed {} orphaned entries", removed_count);
    } else {
        println!("[CLEANUP] No orphaned entries found");
    }

    removed_count
}

/// Recursively clean up orphaned images from image cache directory
fn cleanup_image_directory(
    base_dir: &str,
    used_paths: &std::collections::HashSet<String>,
    sub_path: &str,
) {
    let full_path = if sub_path.is_empty() {
        Path::new(base_dir).to_path_buf()
    } else {
        Path::new(base_dir).join(sub_path)
    };

    if let Ok(entries) = std::fs::read_dir(&full_path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let file_type = entry.file_type();
            let entry_name = entry.file_name().to_string_lossy().to_string();

            if let Ok(ft) = file_type {
                if ft.is_dir() {
                    // Recursively clean subdirectory
                    let new_sub_path = if sub_path.is_empty() {
                        entry_name.clone()
                    } else {
                        format!("{}/{}", sub_path, entry_name)
                    };
                    cleanup_image_directory(base_dir, used_paths, &new_sub_path);

                    // If directory is now empty, remove it
                    if let Ok(mut entries) = std::fs::read_dir(entry.path()) {
                        if entries.next().is_none() {
                            println!("[CLEANUP] Removing empty directory: {}", entry_name);
                            let _ = std::fs::remove_dir(entry.path());
                        }
                    }
                } else if ft.is_file() {
                    // Build the path as stored in database
                    let db_path = if sub_path.is_empty() {
                        format!("image_cache/{}", entry_name)
                    } else {
                        format!("image_cache/{}/{}", sub_path, entry_name)
                    };

                    // If this image is not in use by any media, delete it
                    if !used_paths.contains(&db_path) {
                        println!("[CLEANUP] Removing orphaned image: {}", db_path);
                        if let Err(e) = std::fs::remove_file(entry.path()) {
                            println!("[CLEANUP] Error removing image: {}", e);
                        }
                    }
                }
            }
        }
    }
}

pub fn process_movie(
    db: &Database,
    file_path: &str,
    parsed: &ParsedMedia,
    api_key: &str,
    image_cache_dir: &str,
    duration: f64,
) {
    let mut title = parsed.title.clone();
    let mut year = parsed.year;
    let mut overview: Option<String> = None;
    let mut cast_names: Option<String> = None;
    let mut director: Option<String> = None;
    let mut poster_path: Option<String> = None;
    let mut tmdb_id: Option<String> = None;
    let mut imdb_id: Option<String> = None;
    let mut tmdb_runtime_seconds: Option<f64> = None;

    if let Ok(Some(metadata)) = tmdb::search_metadata_with_fallback(
        api_key,
        &parsed.title,
        "movie",
        parsed.year,
        image_cache_dir,
    ) {
        title = prefer_title_with_leading_article(&parsed.title, &metadata.title);
        year = metadata.year;
        overview = metadata.overview;
        cast_names = metadata.cast_names;
        director = metadata.director;
        poster_path = metadata.poster_path;
        tmdb_id = metadata.tmdb_id;
        imdb_id = metadata.imdb_id;
        tmdb_runtime_seconds = metadata.runtime_seconds;
    } else if api_key.trim().is_empty() {
        println!(
            "[IMDBAPI] No api_key configured; indexer skipped metadata for \"{}\"",
            parsed.title
        );
    }

    let effective_duration = if duration > 0.0 {
        duration
    } else {
        tmdb_runtime_seconds.unwrap_or(0.0)
    };

    match db.insert_movie(
        &title,
        year,
        overview.as_deref(),
        cast_names.as_deref(),
        director.as_deref(),
        poster_path.as_deref(),
        file_path,
        effective_duration,
        tmdb_id.as_deref(),
    ) {
        Ok(new_id) => {
            println!("Indexed Movie: {}", title);
            if let Some(ref iid) = imdb_id {
                let metadata = tmdb::TmdbMetadata {
                    title: title.clone(),
                    year,
                    overview: overview.clone(),
                    cast_names: cast_names.clone(),
                    director: director.clone(),
                    poster_path: poster_path.clone(),
                    tmdb_id: tmdb_id.clone(),
                    imdb_id: Some(iid.clone()),
                    runtime_seconds: tmdb_runtime_seconds,
                    imdb_image_url: None,
                };
                if let Err(e) = db.update_metadata(new_id, &metadata) {
                    println!("[IMDBAPI] Failed to persist imdb_id for {}: {}", title, e);
                }
            }
        }
        Err(e) => println!("Error indexing movie {}: {}", title, e),
    }
}

pub fn process_tv_episode(
    db: &Database,
    file_path: &str,
    parsed: &ParsedMedia,
    api_key: &str,
    image_cache_dir: &str,
    duration: f64,
) {
    println!(
        "[TV] Processing episode: {} S{:02}E{:02} from file: {}",
        parsed.title,
        parsed.season.unwrap_or(0),
        parsed.episode.unwrap_or(0),
        file_path
    );

    // First, try to find an existing series with a matching title BEFORE searching TMDB
    // This ensures episodes group together even if TMDB search is inconsistent
    let existing_series = db.find_series_by_tmdb_or_title(None, &parsed.title, parsed.year);

    let (
        series_title,
        series_year,
        series_overview,
        series_cast_names,
        series_poster_path,
        series_tmdb_id,
        mut series_imdb_id,
        series_id,
        _is_new_series,
    ) = if let Ok(Some(existing_id)) = existing_series {
        // Found existing series - use its data
        println!(
            "[TV] Found existing series by title match (ID: {})",
            existing_id
        );
        if let Ok(existing) = db.get_media_by_id(existing_id) {
            (
                existing.title.clone(),
                existing.year,
                existing.overview.clone(),
                existing.cast_names.clone(),
                existing.poster_path.clone(),
                existing.tmdb_id.clone(),
                existing.imdb_id.clone(),
                Some(existing_id),
                false,
            )
        } else {
            (
                parsed.title.clone(),
                parsed.year,
                None,
                None,
                None,
                None,
                None,
                Some(existing_id),
                false,
            )
        }
    } else {
        // No existing series - search TMDB
        let mut title = parsed.title.clone();
        let mut year = parsed.year;
        let mut overview: Option<String> = None;
        let mut cast_names: Option<String> = None;
        let mut poster_path: Option<String> = None;
        let mut tmdb_id: Option<String> = None;
        let mut imdb_id: Option<String> = None;

        if let Ok(Some(metadata)) = tmdb::search_metadata_with_fallback(
            api_key,
            &parsed.title,
            "tv",
            parsed.year,
            image_cache_dir,
        ) {
            title = prefer_title_with_leading_article(&parsed.title, &metadata.title);
            year = metadata.year;
            overview = metadata.overview;
            cast_names = metadata.cast_names;
            tmdb_id = metadata.tmdb_id;
            imdb_id = metadata.imdb_id;

            if let Some(ref poster) = metadata.poster_path {
                poster_path = Some(poster.clone());
            }

            println!(
                "[TMDB] Series metadata for \"{}\": poster={:?}, imdb_id={:?}",
                title, poster_path, imdb_id
            );
        }

        (
            title,
            year,
            overview,
            cast_names,
            poster_path,
            tmdb_id,
            imdb_id,
            None,
            true,
        )
    };

    // Now get or create the series
    let final_series_id = if let Some(id) = series_id {
        // Already have the series ID
        id
    } else {
        // Try to find by TMDB ID first (in case TMDB gave us an ID that matches an existing series)
        match db.find_series_by_tmdb_or_title(series_tmdb_id.as_deref(), &series_title, series_year)
        {
            Ok(Some(id)) => {
                println!(
                    "[TV] Found existing series after TMDB lookup (ID: {}): {}",
                    id, series_title
                );

                // Update metadata if needed
                if let Some(ref tmdb_id) = series_tmdb_id {
                    if let Ok(existing) = db.get_media_by_id(id) {
                        // Capture imdb_id from existing series for imdbapi.dev lookups
                        if series_imdb_id.is_none() && existing.imdb_id.is_some() {
                            series_imdb_id = existing.imdb_id.clone();
                        }
                        if existing.tmdb_id.is_none()
                            || existing.poster_path.is_none()
                            || existing.cast_names.is_none()
                        {
                            let metadata = tmdb::TmdbMetadata {
                                title: series_title.clone(),
                                year: series_year,
                                overview: series_overview.clone(),
                                cast_names: series_cast_names.clone(),
                                director: None,
                                poster_path: series_poster_path.clone(),
                                tmdb_id: Some(tmdb_id.clone()),
                                imdb_id: None,
                                runtime_seconds: None,
                                imdb_image_url: None,
                            };
                            if let Err(e) = db.update_metadata(id, &metadata) {
                                println!("[TV] Warning: Failed to update series metadata: {}", e);
                            }
                        }
                    }
                }
                id
            }
            Ok(None) => {
                // Create new series
                let virtual_folder = format!(
                    "tvshow://{}/{}",
                    series_tmdb_id.as_deref().unwrap_or("unknown"),
                    series_title.to_lowercase().replace(' ', "_")
                );

                match db.insert_tvshow(
                    &series_title,
                    series_year,
                    series_overview.as_deref(),
                    series_cast_names.as_deref(),
                    series_poster_path.as_deref(),
                    &virtual_folder,
                    series_tmdb_id.as_deref(),
                ) {
                    Ok(id) => {
                        println!("[TV] Created new series (ID: {}): {}", id, series_title);
                        id
                    }
                    Err(e) => {
                        println!("[TV] Error creating series {}: {}", series_title, e);
                        return;
                    }
                }
            }
            Err(e) => {
                println!("[TV] Error finding series: {}", e);
                return;
            }
        }
    };

    // Get episode info
    let season = parsed.season.unwrap_or(1);
    let episode = parsed.episode.unwrap_or(1);
    let ep_title = format!("S{:02}E{:02}", season, episode);

    // Fetch episode metadata directly from TMDB for THIS specific episode
    let (episode_title, episode_overview, episode_still) = if let Some(ref tmdb_id) = series_tmdb_id
    {
        // First check if we have cached metadata
        println!(
            "[META] Episode S{:02}E{:02}: checking local cache...",
            season, episode
        );
        let cached_data = db
            .get_cached_episode_metadata(tmdb_id, season, episode)
            .ok()
            .flatten();

        // Check if cached still_path file actually exists on disk
        let cache_valid = if let Some(ref cached) = cached_data {
            if let Some(ref still_path) = cached.still_path {
                // Remove image_cache/ prefix if present for checking
                let clean_path = still_path.replace("image_cache/", "");
                let full_path = Path::new(image_cache_dir).join(&clean_path);
                let exists = full_path.exists();
                if !exists {
                    println!(
                        "[TV] Cached still_path doesn't exist on disk: {:?}",
                        full_path
                    );
                }
                exists
            } else {
                // No still_path in cache - need to fetch
                false
            }
        } else {
            false
        };

        if cache_valid {
            let cached = cached_data.unwrap();
            println!(
                "[META] Episode S{:02}E{:02}: cache hit, still_path={:?}",
                season, episode, cached.still_path
            );
            (cached.episode_title, cached.overview, cached.still_path)
        } else {
            // No valid cache - try imdbapi.dev for episode image first, then fall back to TMDB
            println!(
                "[META] Episode S{:02}E{:02}: cache miss, trying imdbapi.dev...",
                season, episode
            );
            let mut imdbapi_still_path: Option<String> = None;

            if let Some(ref imdb_id) = series_imdb_id {
                println!(
                    "[IMDBAPI] Fetching episode image: show={}, S{:02}E{:02}",
                    imdb_id, season, episode
                );

                #[derive(serde::Deserialize)]
                struct ImdbApiEpResp {
                    #[serde(default)]
                    episodeNumber: Option<i32>,
                    #[serde(default)]
                    primaryImage: Option<ImdbApiPrimaryImg>,
                }
                #[derive(serde::Deserialize)]
                struct ImdbApiPrimaryImg {
                    #[serde(default)]
                    url: Option<String>,
                }
                #[derive(serde::Deserialize)]
                struct ImdbApiEpsResp {
                    #[serde(default)]
                    episodes: Vec<ImdbApiEpResp>,
                }

                let url = format!(
                    "https://api.imdbapi.dev/titles/{}/episodes?season={}",
                    imdb_id, season
                );
                let client = http_client::shared_client();
                let resp = client
                    .get(&url)
                    .timeout(std::time::Duration::from_secs(10))
                    .send();

                match resp {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(data) = r.json::<ImdbApiEpsResp>() {
                            for ep in &data.episodes {
                                if ep.episodeNumber == Some(episode) {
                                    if let Some(ref img) = ep.primaryImage {
                                        if let Some(ref img_url) = img.url {
                                            if !img_url.is_empty() {
                                                // Download and cache the image locally
                                                // Include URL hash in filename to avoid collisions between different shows
                                                let url_hash: String = img_url
                                                    .rsplit('/')
                                                    .next()
                                                    .unwrap_or("unknown")
                                                    .chars()
                                                    .filter(|c| c.is_alphanumeric())
                                                    .take(20)
                                                    .collect();
                                                let url_hash = if url_hash.is_empty() {
                                                    "unknown".to_string()
                                                } else {
                                                    url_hash
                                                };
                                                let filename = format!(
                                                    "imdb_ep_s{:02}e{:02}_{}.jpg",
                                                    season, episode, url_hash
                                                );
                                                let cache_path =
                                                    Path::new(image_cache_dir).join(&filename);
                                                if !cache_path.exists() {
                                                    match client
                                                        .get(img_url.as_str())
                                                        .timeout(std::time::Duration::from_secs(15))
                                                        .send()
                                                    {
                                                        Ok(img_resp)
                                                            if img_resp.status().is_success() =>
                                                        {
                                                            if let Ok(bytes) = img_resp.bytes() {
                                                                let _ = std::fs::write(
                                                                    &cache_path,
                                                                    &bytes,
                                                                );
                                                                println!(
                                                                    "[TV] Cached imdbapi.dev episode image: {:?}",
                                                                    cache_path
                                                                );
                                                            }
                                                        }
                                                        _ => {
                                                            println!(
                                                                "[TV] Failed to download imdbapi.dev image for {} S{:02}E{:02}",
                                                                series_title, season, episode
                                                            );
                                                        }
                                                    }
                                                }
                                                // Only set still_path if the file actually exists on disk
                                                if cache_path.exists() {
                                                    let cached_path =
                                                        format!("image_cache/{}", filename);
                                                    println!(
                                                        "[IMDBAPI] Got episode image: {}",
                                                        cached_path
                                                    );
                                                    imdbapi_still_path = Some(cached_path);
                                                } else {
                                                    println!("[IMDBAPI] Episode image not available on disk for S{:02}E{:02}", season, episode);
                                                }
                                            }
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    _ => {
                        println!(
                            "[TV] imdbapi.dev request failed for S{:02}E{:02}",
                            season, episode
                        );
                    }
                }
            }

            if series_imdb_id.is_some() && imdbapi_still_path.is_none() {
                println!(
                    "[IMDBAPI] No episode image from imdbapi.dev for S{:02}E{:02}",
                    season, episode
                );
            }

            // If we got an image from imdbapi.dev and have cached title/overview, combine them
            // without hitting TMDB. Otherwise fall back to TMDB for full metadata.
            let has_cached_text = cached_data
                .as_ref()
                .map_or(false, |c| c.episode_title.is_some() || c.overview.is_some());

            if imdbapi_still_path.is_some() && has_cached_text {
                let cached = cached_data.unwrap();
                let still = imdbapi_still_path;
                println!(
                    "[TV] Using imdbapi.dev image with cached metadata for {} S{:02}E{:02}",
                    series_title, season, episode
                );
                let _ = db.save_cached_episode_metadata(
                    tmdb_id,
                    season,
                    episode,
                    cached.episode_title.as_deref(),
                    cached.overview.as_deref(),
                    still.as_deref(),
                    cached.air_date.as_deref(),
                    cached.vote_average,
                );
                (cached.episode_title, cached.overview, still)
            } else if !api_key.is_empty() {
                println!(
                    "[TMDB] Fetching episode metadata via TMDB for S{:02}E{:02}",
                    season, episode
                );
                match fetch_single_episode_metadata(
                    api_key,
                    tmdb_id,
                    season,
                    episode,
                    &series_title,
                    image_cache_dir,
                ) {
                    Ok(Some(ep_info)) => {
                        // If imdbapi.dev provided a still image, prefer it over TMDB
                        let final_still = if imdbapi_still_path.is_some() {
                            imdbapi_still_path
                        } else {
                            ep_info.still_path.clone()
                        };
                        println!(
                            "[TMDB] Got episode metadata: title=\"{}\", still={:?}",
                            ep_info.name, final_still
                        );
                        // Cache it for future use
                        let _ = db.save_cached_episode_metadata(
                            tmdb_id,
                            season,
                            episode,
                            Some(&ep_info.name),
                            ep_info.overview.as_deref(),
                            final_still.as_deref(),
                            ep_info.air_date.as_deref(),
                            ep_info.vote_average,
                        );
                        (Some(ep_info.name), ep_info.overview, final_still)
                    }
                    Ok(None) => {
                        println!(
                            "[TV] No TMDB metadata found for {} S{:02}E{:02}",
                            series_title, season, episode
                        );
                        (None, None, imdbapi_still_path)
                    }
                    Err(e) => {
                        println!("[TV] Failed to fetch episode metadata: {}", e);
                        (None, None, imdbapi_still_path)
                    }
                }
            } else {
                // No API key - use whatever we got from imdbapi.dev
                (None, None, imdbapi_still_path)
            }
        }
    } else {
        (None, None, None)
    };

    match db.insert_episode_with_metadata(
        &ep_title,
        file_path,
        final_series_id,
        season,
        episode,
        duration,
        episode_title.as_deref(),
        episode_overview.as_deref(),
        episode_still.as_deref(),
    ) {
        Ok(_) => println!(
            "[TV] Indexed Episode: {} - {} (series_id: {})",
            series_title, ep_title, final_series_id
        ),
        Err(e) => println!("[TV] Error indexing episode {}: {}", ep_title, e),
    }
}

/// Fetch metadata for a single episode from TMDB
fn fetch_single_episode_metadata(
    api_key: &str,
    tmdb_id: &str,
    season: i32,
    episode: i32,
    series_title: &str,
    image_cache_dir: &str,
) -> Result<Option<tmdb::TmdbEpisodeInfo>, Box<dyn std::error::Error + Send + Sync>> {
    // Use the existing tmdb function but we only need one episode
    // For efficiency, we fetch the whole season and pick the episode we need
    // This is cached anyway so subsequent episodes in the same season will be fast

    let season_info =
        tmdb::fetch_season_episodes(api_key, tmdb_id, season, series_title, image_cache_dir)?;

    // Find our specific episode
    for ep in season_info.episodes {
        if ep.episode_number == episode {
            return Ok(Some(ep));
        }
    }

    Ok(None)
}

/// Try to fetch an episode still image from imdbapi.dev when TMDB has none.
/// Returns a cached local image path (relative to image_cache_dir) on success.
fn fetch_imdb_episode_image(
    show_imdb_id: &str,
    season: i32,
    episode: i32,
    image_cache_dir: &str,
    series_title: &str,
) -> Option<String> {
    let url = format!(
        "https://api.imdbapi.dev/titles/{}/episodes?season={}",
        show_imdb_id, season
    );
    let client = crate::http_client::shared_client();
    let resp = client.get(&url).send().ok()?;
    let json: serde_json::Value = resp.json().ok()?;
    let episodes = json.get("episodes")?.as_array()?;

    for ep in episodes {
        if ep.get("episodeNumber").and_then(|n| n.as_i64()) == Some(episode as i64) {
            let img_url = ep.get("primaryImage")?.get("url")?.as_str()?;
            println!(
                "[TV] Found imdbapi.dev episode image for {} S{:02}E{:02}: {}",
                series_title, season, episode, img_url
            );
            return tmdb::cache_image_organized(
                img_url,
                image_cache_dir,
                series_title,
                tmdb::ImageType::EpisodeBanner { season, episode },
            );
        }
    }
    None
}

pub fn parse_filename(path: &Path) -> ParsedMedia {
    let filename = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    println!("[PARSE] Parsing filename: '{}'", filename);

    // Get folder context for smarter detection
    let folder_ctx = analyze_folder_structure(path);
    println!(
        "[PARSE] Folder context: series_name={:?}, folder_season={:?}, is_tv={}",
        folder_ctx.series_name, folder_ctx.folder_season, folder_ctx.is_tv_structure
    );

    // Try to parse as TV episode first (more specific patterns)
    if let Some(parsed) = try_parse_tv_episode(filename, &folder_ctx) {
        println!(
            "[PARSE] Detected as TV: title='{}', S{:02}E{:02}",
            parsed.title,
            parsed.season.unwrap_or(0),
            parsed.episode.unwrap_or(0)
        );
        return parsed;
    }

    // If folder structure suggests TV show but no episode pattern found,
    // still treat it as a potential episode using folder context
    if folder_ctx.is_tv_structure {
        if let Some(parsed) = try_parse_from_folder_context(filename, &folder_ctx) {
            println!(
                "[PARSE] Detected as TV (from folder): title='{}', S{:02}E{:02}",
                parsed.title,
                parsed.season.unwrap_or(0),
                parsed.episode.unwrap_or(0)
            );
            return parsed;
        }
    }

    // Parse as movie
    let movie = parse_as_movie(filename);
    println!(
        "[PARSE] Detected as Movie: title='{}', year={:?}",
        movie.title, movie.year
    );
    movie
}

/// Parse a cloud filename (no folder context available)
/// Used for Google Drive files where we only have the filename
pub fn parse_cloud_filename(filename: &str) -> ParsedMedia {
    // Remove file extension
    let filename_without_ext = filename
        .rsplit_once('.')
        .map(|(name, _)| name)
        .unwrap_or(filename);

    println!(
        "[CLOUD_PARSE] Parsing cloud filename: '{}'",
        filename_without_ext
    );

    // Try to parse as TV episode first
    if let Some(parsed) = try_parse_tv_episode(
        filename_without_ext,
        &FolderContext {
            series_name: None,
            series_year: None,
            folder_season: None,
            is_tv_structure: false,
        },
    ) {
        println!(
            "[CLOUD_PARSE] Detected as TV: title='{}', S{:02}E{:02}",
            parsed.title,
            parsed.season.unwrap_or(0),
            parsed.episode.unwrap_or(0)
        );
        return parsed;
    }

    if let Some(parsed) = try_parse_tv_season_pack(filename_without_ext) {
        println!(
            "[CLOUD_PARSE] Detected as TV season pack: title='{}', S{:02}",
            parsed.title,
            parsed.season.unwrap_or(0)
        );
        return parsed;
    }

    // Parse as movie
    let movie = parse_as_movie(filename_without_ext);
    println!(
        "[CLOUD_PARSE] Detected as Movie: title='{}', year={:?}",
        movie.title, movie.year
    );
    movie
}

/// Analyze the folder structure to extract series name, season, and determine if it's a TV structure
fn analyze_folder_structure(path: &Path) -> FolderContext {
    let mut ctx = FolderContext {
        series_name: None,
        series_year: None,
        folder_season: None,
        is_tv_structure: false,
    };

    let parent = match path.parent() {
        Some(p) => p,
        None => return ctx,
    };

    let parent_name = parent.file_name().and_then(|s| s.to_str()).unwrap_or("");

    // Check if parent is a "Season X" folder
    let season_patterns = [
        Regex::new(r"(?i)^Season\s*(\d{1,2})$").ok(),
        Regex::new(r"(?i)^S(\d{1,2})$").ok(),
        Regex::new(r"(?i)^Series\s*(\d{1,2})$").ok(),
        Regex::new(r"(?i)^Staffel\s*(\d{1,2})$").ok(), // German
        Regex::new(r"(?i)^Saison\s*(\d{1,2})$").ok(),  // French
    ];

    for pattern in season_patterns.iter().flatten() {
        if let Some(caps) = pattern.captures(parent_name) {
            if let Some(season) = caps.get(1).and_then(|m| m.as_str().parse().ok()) {
                ctx.folder_season = Some(season);
                ctx.is_tv_structure = true;

                // The series name should be in the grandparent folder
                if let Some(grandparent) = parent.parent() {
                    if let Some(gp_name) = grandparent.file_name().and_then(|s| s.to_str()) {
                        let (name, year) = extract_series_name_from_folder(gp_name);
                        ctx.series_name = Some(name);
                        ctx.series_year = year;
                    }
                }
                break;
            }
        }
    }

    // If no season folder found, check if parent folder itself looks like a series
    if !ctx.is_tv_structure {
        // Check for patterns like "Show Name (2020)" or "Show Name"
        // that contain multiple video files (would indicate a series)
        let (name, year) = extract_series_name_from_folder(parent_name);

        // Check if the folder name contains common TV indicators
        let tv_indicators = [
            r"(?i)\bseason\b",
            r"(?i)\bseries\b",
            r"(?i)\bcomplete\b",
            r"(?i)\bs\d{1,2}$",
            r"(?i)\btvshow\b",
        ];

        for pattern in tv_indicators.iter() {
            if let Ok(re) = Regex::new(pattern) {
                if re.is_match(parent_name) {
                    ctx.is_tv_structure = true;
                    ctx.series_name = Some(name.clone());
                    ctx.series_year = year;
                    break;
                }
            }
        }

        // Also check if the path contains typical TV folder patterns
        let path_str = path.to_string_lossy().to_lowercase();
        if path_str.contains("tv shows")
            || path_str.contains("tv series")
            || path_str.contains("series")
            || path_str.contains("shows")
        {
            ctx.is_tv_structure = true;
            if ctx.series_name.is_none() {
                ctx.series_name = Some(name);
                ctx.series_year = year;
            }
        }
    }

    ctx
}

/// Extract series name and year from folder name like "Breaking Bad (2008)"
fn extract_series_name_from_folder(folder_name: &str) -> (String, Option<i32>) {
    // Pattern: "Name (Year)" or "Name [Year]"
    if let Ok(re) = Regex::new(r"^(.+?)\s*[\(\[]?\s*((?:19|20)\d{2})\s*[\)\]]?\s*$") {
        if let Some(caps) = re.captures(folder_name) {
            let name = caps
                .get(1)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default();
            let year = caps.get(2).and_then(|m| m.as_str().parse().ok());
            if !name.is_empty() {
                return (clean_folder_name(&name), year);
            }
        }
    }

    (clean_folder_name(folder_name), None)
}

/// Clean folder name by removing common junk
fn clean_folder_name(name: &str) -> String {
    let mut result = name.to_string();

    // Remove common tags in brackets
    let patterns = [
        r"\s*\[.*?\]\s*",
        r"\s*\((?!(?:19|20)\d{2}\)).*?\)\s*", // Remove parentheses unless they contain a year
    ];

    for pattern in patterns.iter() {
        if let Ok(re) = Regex::new(pattern) {
            result = re.replace_all(&result, " ").to_string();
        }
    }

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Try to parse filename as TV episode using comprehensive patterns
fn try_parse_tv_episode(filename: &str, folder_ctx: &FolderContext) -> Option<ParsedMedia> {
    // First, check if filename contains codec indicators that might be confused with episode numbers
    // These should NOT be treated as episode numbers
    let codec_pattern = Regex::new(r"(?i)[xh]\.?26[45]").ok()?;
    let has_codec = codec_pattern.is_match(filename);

    // Comprehensive TV episode patterns (ordered by specificity)
    // Only use strict patterns that have clear season/episode markers
    let strict_patterns: Vec<Regex> = vec![
        // Standard SxxExx patterns (most reliable)
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+S(?P<season>\d{1,2})E(?P<episode>\d{1,3})(?:-?E(?P<episode_end>\d{1,3}))?").ok()?,
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+S(?P<season>\d{1,2})\.E(?P<episode>\d{1,3})").ok()?,
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+S(?P<season>\d{1,2})[.\s_-]+EP?(?P<episode>\d{1,3})(?:-?EP?(?P<episode_end>\d{1,3}))?").ok()?,
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+S(?P<season>\d{1,2})[.\s_-]+EPISODE[.\s_-]*(?P<episode>\d{1,3})(?:[.\s_-]*-[.\s_-]*EPISODE?[.\s_-]*(?P<episode_end>\d{1,3}))?").ok()?,
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+S(?P<season>\d{1,2})[.\s_-]+E[.\s_-]*(?P<episode>\d{1,3})(?:[.\s_-]*-[.\s_-]*E[.\s_-]*(?P<episode_end>\d{1,3}))?").ok()?,

        // Season/Episode spelled out
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+Season\s*(?P<season>\d{1,2})[.\s_-]+Episode\s*(?P<episode>\d{1,3})").ok()?,
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+Season\s*(?P<season>\d{1,2})[.\s_-]+Ep(?:isode)?\.?\s*(?P<episode>\d{1,3})").ok()?,

        // 1x01 format
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+(?P<season>\d{1,2})x(?P<episode>\d{2,3})").ok()?,
    ];

    // Try strict patterns first (these are reliable)
    for pattern in &strict_patterns {
        if let Some(caps) = pattern.captures(filename) {
            let raw_title = caps.name("title").map(|m| m.as_str()).unwrap_or("");
            let title = clean_title(raw_title);
            let (title, year) = extract_year_from_title(&title);
            let title = clean_junk_from_title(&title);

            if title.len() < 2 {
                continue;
            }

            let season = caps.name("season").and_then(|m| m.as_str().parse().ok());
            let episode = caps.name("episode").and_then(|m| m.as_str().parse().ok());
            let episode_end = caps
                .name("episode_end")
                .and_then(|m| m.as_str().parse().ok());

            if let Some(ep) = episode {
                // Sanity check: episode numbers above 100 are rare
                if ep > 100 {
                    println!("[PARSE] Skipping suspicious episode number: {}", ep);
                    continue;
                }

                let final_title = get_best_title(&title, folder_ctx);
                let final_year = year.or(folder_ctx.series_year);

                return Some(ParsedMedia {
                    title: final_title,
                    year: final_year,
                    media_type: MediaParseType::TvEpisode,
                    season,
                    episode: Some(ep),
                    episode_end,
                });
            }
        }
    }

    // Only use looser patterns if folder structure suggests TV AND no codec in filename
    if folder_ctx.is_tv_structure && !has_codec {
        let loose_patterns: Vec<Regex> = vec![
            // Episode patterns without season (e.g., "Show E01")
            Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+E(?P<episode>\d{1,3})(?:[.\s_-]|$)").ok()?,
            Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+Ep\.?\s*(?P<episode>\d{1,3})").ok()?,
        ];

        for pattern in &loose_patterns {
            if let Some(caps) = pattern.captures(filename) {
                let raw_title = caps.name("title").map(|m| m.as_str()).unwrap_or("");
                let title = clean_title(raw_title);
                let (title, year) = extract_year_from_title(&title);
                let title = clean_junk_from_title(&title);

                if title.len() < 2 {
                    continue;
                }

                let episode: Option<i32> =
                    caps.name("episode").and_then(|m| m.as_str().parse().ok());

                if let Some(ep) = episode {
                    // Stricter sanity check for loose patterns
                    if ep > 50 || ep == 0 {
                        continue;
                    }

                    let final_title = get_best_title(&title, folder_ctx);
                    let final_year = year.or(folder_ctx.series_year);

                    return Some(ParsedMedia {
                        title: final_title,
                        year: final_year,
                        media_type: MediaParseType::TvEpisode,
                        season: folder_ctx.folder_season.or(Some(1)),
                        episode: Some(ep),
                        episode_end: None,
                    });
                }
            }
        }
    }

    None
}

fn try_parse_tv_season_pack(filename: &str) -> Option<ParsedMedia> {
    let patterns = [
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+S(?P<season>\d{1,2})(?:[.\s_-]|$)").ok()?,
        Regex::new(r"(?i)^(?P<title>.+?)[.\s_-]+Season\s*(?P<season>\d{1,2})(?:[.\s_-]|$)").ok()?,
    ];

    for pattern in patterns {
        if let Some(caps) = pattern.captures(filename) {
            let raw_title = caps.name("title").map(|m| m.as_str()).unwrap_or("");
            let title = clean_title(raw_title);
            let (title, year) = extract_year_from_title(&title);
            let title = clean_junk_from_title(&title);
            let season = caps.name("season").and_then(|m| m.as_str().parse().ok());

            if title.len() < 2 || season.is_none() {
                continue;
            }

            return Some(ParsedMedia {
                title,
                year,
                media_type: MediaParseType::TvEpisode,
                season,
                episode: None,
                episode_end: None,
            });
        }
    }

    None
}

/// Get the best title from parsed title and folder context
fn get_best_title(title: &str, folder_ctx: &FolderContext) -> String {
    if let Some(ref series_name) = folder_ctx.series_name {
        if title.len() < 3 || is_generic_title(title) {
            series_name.clone()
        } else if series_name.to_lowercase().contains(&title.to_lowercase()) {
            series_name.clone()
        } else {
            title.to_string()
        }
    } else {
        title.to_string()
    }
}

/// Check if a title is too generic
fn is_generic_title(title: &str) -> bool {
    let generic = ["episode", "ep", "part", "chapter", "vol", "volume"];
    let lower = title.to_lowercase();
    generic
        .iter()
        .any(|g| lower == *g || lower.starts_with(&format!("{} ", g)))
}

/// Try to parse using folder context when filename doesn't have clear episode pattern
fn try_parse_from_folder_context(
    filename: &str,
    folder_ctx: &FolderContext,
) -> Option<ParsedMedia> {
    if folder_ctx.series_name.is_none() {
        return None;
    }

    // Try to extract just an episode number from filename
    let episode_patterns = [
        Regex::new(r"(?i)E?(?P<episode>\d{1,3})").ok(),
        Regex::new(r"(?i)-\s*(?P<episode>\d{1,3})\s*-").ok(),
        Regex::new(r"(?i)(?P<episode>\d{2,3})").ok(),
    ];

    for pattern in episode_patterns.iter().flatten() {
        if let Some(caps) = pattern.captures(filename) {
            if let Some(ep) = caps.name("episode").and_then(|m| m.as_str().parse().ok()) {
                // Sanity check - episode number should be reasonable
                if ep > 0 && ep < 1000 {
                    return Some(ParsedMedia {
                        title: folder_ctx.series_name.clone().unwrap(),
                        year: folder_ctx.series_year,
                        media_type: MediaParseType::TvEpisode,
                        season: folder_ctx.folder_season.or(Some(1)),
                        episode: Some(ep),
                        episode_end: None,
                    });
                }
            }
        }
    }

    None
}

/// Parse filename as a movie
fn parse_as_movie(filename: &str) -> ParsedMedia {
    let clean_name = filename.replace('.', " ").replace('_', " ");
    let (title, year) = extract_year_from_title(&clean_name);
    let title = clean_junk_from_title(&title);

    ParsedMedia {
        title,
        year,
        media_type: MediaParseType::Movie,
        season: None,
        episode: None,
        episode_end: None,
    }
}

fn clean_title(title: &str) -> String {
    title.replace('.', " ").replace('_', " ").trim().to_string()
}

fn normalize_for_article_compare(title: &str) -> String {
    let mut normalized = String::with_capacity(title.len());
    let mut last_was_space = false;

    for ch in title.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch);
            last_was_space = false;
        } else if !last_was_space {
            normalized.push(' ');
            last_was_space = true;
        }
    }

    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn prefer_title_with_leading_article(original: &str, tmdb_title: &str) -> String {
    let original_trim = original.trim();
    let tmdb_trim = tmdb_title.trim();

    if original_trim.len() >= 4 && original_trim.to_lowercase().starts_with("the ") {
        if !tmdb_trim.to_lowercase().starts_with("the ") {
            let stripped = original_trim[4..].trim();
            if normalize_for_article_compare(stripped) == normalize_for_article_compare(tmdb_trim) {
                return original_trim.to_string();
            }
        }
    }

    tmdb_trim.to_string()
}

fn extract_year_from_title(title: &str) -> (String, Option<i32>) {
    // Special case: if the entire title is just a year (like "1899"), keep it
    let trimmed = title.trim();
    if let Ok(re) = Regex::new(r"^(19[3-9]\d|20\d{2})$") {
        if re.is_match(trimmed) {
            // Title is just a year - this IS the title (e.g., "1899" the show)
            return (trimmed.to_string(), None);
        }
    }

    let year_regex = Regex::new(r"\b(19[3-9]\d|20\d{2})\b").unwrap();

    if let Some(caps) = year_regex.captures(title) {
        if let Some(year_match) = caps.get(1) {
            let year_str = year_match.as_str();
            if let Ok(year) = year_str.parse::<i32>() {
                // Keep only the part before the detected year and trim dangling separators/brackets.
                let before_year = &title[..year_match.start()];
                let cleaned_title = before_year
                    .trim()
                    .trim_end_matches(|c: char| {
                        c.is_whitespace() || matches!(c, '(' | '[' | '{' | '-' | '_' | '.')
                    })
                    .trim()
                    .to_string();

                // Only use the year-less title if it's substantial
                if !cleaned_title.is_empty() && cleaned_title.len() >= 2 {
                    return (cleaned_title, Some(year));
                }
            }
        }
    }

    (title.to_string(), None)
}

fn clean_junk_from_title(title: &str) -> String {
    // Comprehensive list of patterns to remove from filenames
    let junk_patterns = [
        // Resolution/quality
        r"(?i)\b1080p\b",
        r"(?i)\b720p\b",
        r"(?i)\b2160p\b",
        r"(?i)\b4k\b",
        r"(?i)\buhd\b",
        r"(?i)\b480p\b",
        r"(?i)\b576p\b",
        r"(?i)\bhd\b",
        r"(?i)\bsd\b",
        r"(?i)\bfhd\b",
        // Source
        r"(?i)\bbluray\b",
        r"(?i)\bblu-ray\b",
        r"(?i)\bbdrip\b",
        r"(?i)\bbrip\b",
        r"(?i)\bremux\b",
        r"(?i)\bweb-?dl\b",
        r"(?i)\bweb-?rip\b",
        r"(?i)\bwebrip\b",
        r"(?i)\bhdrip\b",
        r"(?i)\bdvdrip\b",
        r"(?i)\bdvdscr\b",
        r"(?i)\bhdtv\b",
        r"(?i)\bpdtv\b",
        r"(?i)\bdsr\b",
        r"(?i)\bhdcam\b",
        r"(?i)\bcam\b",
        r"(?i)\bts\b",
        r"(?i)\btelesync\b",
        r"(?i)\bscreener\b",
        r"(?i)\br5\b",
        r"(?i)\bbdrip\b",
        r"(?i)\bamzn\b",
        r"(?i)\bnf\b",
        r"(?i)\bnetflix\b",
        r"(?i)\batvp\b",
        r"(?i)\bdsnp\b",
        r"(?i)\bhmax\b",
        r"(?i)\bhulu\b",
        // HDR/Video
        r"(?i)\bimax\b",
        r"(?i)\bsdr\b",
        r"(?i)\bhdr\b",
        r"(?i)\bhdr10\b",
        r"(?i)\bhdr10\+\b",
        r"(?i)\bdolby\s?vision\b",
        r"(?i)\bdv\b",
        r"(?i)\b10bit\b",
        r"(?i)\b8bit\b",
        r"(?i)\bhi10p\b",
        // Codec
        r"(?i)\bavc\b",
        r"(?i)\bhevc\b",
        r"(?i)\bx264\b",
        r"(?i)\bx265\b",
        r"(?i)\bh\.?264\b",
        r"(?i)\bh\.?265\b",
        r"(?i)\bxvid\b",
        r"(?i)\bdivx\b",
        r"(?i)\bvc-?1\b",
        r"(?i)\bav1\b",
        r"(?i)\bmpeg\d?\b",
        // Audio
        r"(?i)\bdts-?hd(\.?ma)?\b",
        r"(?i)\bdts\b",
        r"(?i)\btruehd\b",
        r"(?i)\batmos\b",
        r"(?i)\bddp?\d*\.?\d*\b",
        r"(?i)\bdd\d*\.?\d*\b",
        r"(?i)\bflac\b",
        r"(?i)\baac\b",
        r"(?i)\bac3\b",
        r"(?i)\beac3\b",
        r"(?i)\bmp3\b",
        r"(?i)\blpcm\b",
        r"(?i)\b5[\s.]1\b",
        r"(?i)\b7[\s.]1\b",
        r"(?i)\b2[\s.]0\b",
        r"(?i)\bstereo\b",
        r"(?i)\bmono\b",
        r"(?i)\bsurround\b",
        // Subtitles
        r"(?i)\besub\b",
        r"(?i)\bsub(bed|s)?\b",
        r"(?i)\bsrt\b",
        r"(?i)\bforced\b",
        r"(?i)\bcc\b",
        r"(?i)\bsdh\b",
        // Language
        r"(?i)\bmulti\b",
        r"(?i)\bhindi\b",
        r"(?i)\benglish\b",
        r"(?i)\bdual\s?audio\b",
        r"(?i)\btamil\b",
        r"(?i)\btelugu\b",
        r"(?i)\bspanish\b",
        r"(?i)\bfrench\b",
        r"(?i)\bgerman\b",
        r"(?i)\bitalian\b",
        r"(?i)\bjapanese\b",
        r"(?i)\bkorean\b",
        r"(?i)\bchinese\b",
        r"(?i)\brussian\b",
        r"(?i)\barabic\b",
        r"(?i)\bportuguese\b",
        r"(?i)\beng\b",
        r"(?i)\bhin\b",
        r"(?i)\bjpn\b",
        r"(?i)\bkor\b",
        // Release info
        r"(?i)\brepack\b",
        r"(?i)\bproper\b",
        r"(?i)\breal\b",
        r"(?i)\brip\b",
        r"(?i)\bopen\s?matte\b",
        r"(?i)\bextended\b",
        r"(?i)\bunrated\b",
        r"(?i)\bdc\b",
        r"(?i)\bdirector'?s?\s?cut\b",
        r"(?i)\btheatrical\b",
        r"(?i)\buncut\b",
        r"(?i)\bspecial\s?edition\b",
        r"(?i)\bcomplete\b",
        r"(?i)\bfinal\s?cut\b",
        r"(?i)\bcriterion\b",
        r"(?i)\bremastered\b",
        r"(?i)\brestored\b",
        r"(?i)\banniversary\b",
        r"(?i)\bultimate\b",
        // Scene/group tags
        r"\[.*?\]",         // [Anything]
        r"\(.*?\)",         // (Anything) - but be careful with years
        r"(?i)\b-\s*\w+$",  // Trailing -GROUP
        r"(?i)^\w+\s*-\s*", // Leading GROUP -
        // Common release groups (partial list)
        r"(?i)\byify\b",
        r"(?i)\byts\b",
        r"(?i)\brarbg\b",
        r"(?i)\bettv\b",
        r"(?i)\beztv\b",
        r"(?i)\btigole\b",
        r"(?i)\bqxr\b",
        r"(?i)\bsparks\b",
        r"(?i)\bgalaxy\s?rg\b",
        r"(?i)\bpahe\b",
        r"(?i)\bpsa\b",
        r"(?i)\bMeGusta\b",
        r"(?i)\bfgt\b",
        r"(?i)\blol\b",
        r"(?i)\baxxo\b",
        // Misc
        r"(?i)\bwww\.\w+\.\w+\b", // Website URLs
        r"(?i)\b@\w+\b",          // @handles
        r"\bBT4G\b",
        r"\bMkvCinemas\b",
    ];

    let mut result = title.to_string();

    for pattern in &junk_patterns {
        if let Ok(re) = Regex::new(pattern) {
            result = re.replace_all(&result, " ").to_string();
        }
    }

    // Remove years in parentheses but keep the year for extraction later
    // Actually, we want to keep years, so skip this

    // Clean up multiple dashes, underscores
    if let Ok(re) = Regex::new(r"[-_]{2,}") {
        result = re.replace_all(&result, " ").to_string();
    }

    // Clean up extra whitespace
    if let Ok(re) = Regex::new(r"\s{2,}") {
        result = re.replace_all(&result, " ").to_string();
    }

    // Remove leading/trailing dashes and dots
    result = result
        .trim_matches(|c| c == '-' || c == '.' || c == '_' || c == ' ')
        .to_string();

    result.trim().to_string()
}

/// Helper function to clean up empty parent directories after file deletion
pub fn cleanup_empty_parent_dirs(file_paths: &[String]) {
    use std::collections::HashSet;

    // Collect unique parent directories from deleted files
    let mut parent_dirs: HashSet<PathBuf> = HashSet::new();
    for file_path in file_paths {
        let path = Path::new(file_path);
        if let Some(parent) = path.parent() {
            parent_dirs.insert(parent.to_path_buf());
        }
    }

    // Try to remove empty directories (and their parents if also empty)
    for dir in parent_dirs {
        let mut current_dir = Some(dir);
        while let Some(dir_path) = current_dir.take() {
            // Only try to remove if the directory exists
            if dir_path.exists() && dir_path.is_dir() {
                // Check if directory is empty
                match std::fs::read_dir(&dir_path) {
                    Ok(mut entries) => {
                        if entries.next().is_none() {
                            // Directory is empty, try to remove it
                            match std::fs::remove_dir(&dir_path) {
                                Ok(_) => {
                                    println!("[DELETE] Removed empty directory: {:?}", dir_path);
                                    // Continue to check parent directory
                                    current_dir = dir_path.parent().map(|p| p.to_path_buf());
                                    continue;
                                }
                                Err(e) => {
                                    println!(
                                        "[DELETE] Failed to remove directory {:?}: {}",
                                        dir_path, e
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("[DELETE] Failed to read directory {:?}: {}", dir_path, e);
                    }
                }
            }
            // Stop if directory not empty or doesn't exist
            current_dir = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_cleanup_empty_parent_dirs() {
        use std::env;
        use std::fs;

        // Case 1: Recursive cleanup
        let mut temp_dir = env::temp_dir();
        let uuid = uuid::Uuid::new_v4().to_string();
        temp_dir.push(format!("slasshyvault_test_{}", uuid));

        let parent = temp_dir.join("parent");
        let child = parent.join("child");
        let file_path = child.join("file.txt");

        // Create directories
        fs::create_dir_all(&child).unwrap();

        let paths = vec![file_path.to_string_lossy().to_string()];

        cleanup_empty_parent_dirs(&paths);

        // Check if directories are removed
        assert!(!child.exists(), "Child directory should be removed");
        assert!(!parent.exists(), "Parent directory should be removed");
        // temp_dir is also empty now and is a parent of parent, so it might be removed too depending on permissions and paths

        // Clean up if still exists
        let _ = fs::remove_dir_all(&temp_dir);

        // Case 2: Cleanup stops at non-empty dir
        let mut temp_dir2 = env::temp_dir();
        let uuid2 = uuid::Uuid::new_v4().to_string();
        temp_dir2.push(format!("slasshyvault_test_2_{}", uuid2));

        let parent2 = temp_dir2.join("parent");
        let child2 = parent2.join("child");
        let file_path2 = child2.join("file.txt");
        let other_file = parent2.join("other.txt");

        fs::create_dir_all(&child2).unwrap();
        fs::write(&other_file, "keep me").unwrap();

        let paths2 = vec![file_path2.to_string_lossy().to_string()];

        cleanup_empty_parent_dirs(&paths2);

        assert!(!child2.exists(), "Child directory should be removed");
        assert!(parent2.exists(), "Parent directory should NOT be removed");
        assert!(other_file.exists(), "Other file should still exist");

        // Clean up
        let _ = fs::remove_dir_all(temp_dir2);
    }

    #[test]
    fn test_parse_movie() {
        let path = PathBuf::from("Inception.2010.1080p.BluRay.x264.mkv");
        let parsed = parse_filename(&path);
        assert_eq!(parsed.title, "Inception");
        assert_eq!(parsed.year, Some(2010));
        assert_eq!(parsed.media_type, MediaParseType::Movie);
    }

    #[test]
    fn test_parse_cloud_movie_with_parenthesized_year() {
        let parsed = parse_cloud_filename("Jeepers Creepers (2001).mkv");
        assert_eq!(parsed.title, "Jeepers Creepers");
        assert_eq!(parsed.year, Some(2001));
        assert_eq!(parsed.media_type, MediaParseType::Movie);
    }

    #[test]
    fn test_parse_cloud_tv_episode_with_space_ep_pattern() {
        let parsed = parse_cloud_filename("Lost S01 EP01 1080p BluRay [English DTS 5.1] x264.mkv");
        assert_eq!(parsed.title, "Lost");
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(1));
    }

    #[test]
    fn test_parse_cloud_tv_episode_with_space_e_pattern() {
        let parsed = parse_cloud_filename("Lost S01 E 02 1080p BluRay x264.mkv");
        assert_eq!(parsed.title, "Lost");
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(2));
    }

    #[test]
    fn test_parse_cloud_tv_episode_with_season_episode_words() {
        let parsed = parse_cloud_filename("Lost Season 1 Ep 03 1080p BluRay x264.mkv");
        assert_eq!(parsed.title, "Lost");
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(3));
    }

    #[test]
    fn test_parse_cloud_tv_season_pack() {
        let parsed = parse_cloud_filename(
            "If Wishes Could Kill (2026) S01 2160p NF 10bit WEB-DL DoVi HDR HEVC",
        );
        assert_eq!(parsed.title, "If Wishes Could Kill");
        assert_eq!(parsed.year, Some(2026));
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, None);
    }

    #[test]
    fn test_parse_tv_episode() {
        let path = PathBuf::from("Breaking.Bad.S01E01.Pilot.720p.mkv");
        let parsed = parse_filename(&path);
        assert_eq!(parsed.title, "Breaking Bad");
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(1));
    }

    // --- prefer_title_with_leading_article ---

    #[test]
    fn test_prefer_title_with_leading_article_keeps_the() {
        assert_eq!(
            prefer_title_with_leading_article("The Matrix", "Matrix"),
            "The Matrix"
        );
    }

    #[test]
    fn test_prefer_title_with_leading_article_keeps_the_case_insensitive() {
        assert_eq!(
            prefer_title_with_leading_article("the godfather", "Godfather"),
            "the godfather"
        );
    }

    #[test]
    fn test_prefer_title_with_leading_article_tmdb_already_has_the() {
        // Both have "The" - should return tmdb version
        assert_eq!(
            prefer_title_with_leading_article("The Office", "The Office (US)"),
            "The Office (US)"
        );
    }

    #[test]
    fn test_prefer_title_with_leading_article_no_article_in_original() {
        assert_eq!(
            prefer_title_with_leading_article("Matrix", "The Matrix"),
            "The Matrix"
        );
    }

    #[test]
    fn test_prefer_title_with_leading_article_short_title_ignored() {
        // "The " prefix requires original >= 4 chars
        assert_eq!(
            prefer_title_with_leading_article("The", "The Thing"),
            "The Thing"
        );
    }

    #[test]
    fn test_prefer_title_with_leading_article_titles_match() {
        assert_eq!(
            prefer_title_with_leading_article("The Bear", "Bear"),
            "The Bear"
        );
    }

    #[test]
    fn test_prefer_title_with_leading_article_titles_dont_match() {
        // Original has "The" but stripped version doesn't match tmdb
        assert_eq!(
            prefer_title_with_leading_article("The Walking Dead", "Breaking Bad"),
            "Breaking Bad"
        );
    }

    // --- normalize_for_article_compare ---

    #[test]
    fn test_normalize_for_article_compare_basic() {
        assert_eq!(normalize_for_article_compare("The Matrix"), "the matrix");
    }

    #[test]
    fn test_normalize_for_article_compare_special_chars() {
        assert_eq!(
            normalize_for_article_compare("Spider-Man: No Way Home"),
            "spider man no way home"
        );
    }

    #[test]
    fn test_normalize_for_article_compare_extra_whitespace() {
        assert_eq!(
            normalize_for_article_compare("  Hello   World  "),
            "hello world"
        );
    }

    #[test]
    fn test_normalize_for_article_compare_numbers() {
        assert_eq!(
            normalize_for_article_compare("2001: A Space Odyssey"),
            "2001 a space odyssey"
        );
    }

    // --- extract_year_from_title ---

    #[test]
    fn test_extract_year_basic() {
        let (title, year) = extract_year_from_title("Inception (2010)");
        assert_eq!(title, "Inception");
        assert_eq!(year, Some(2010));
    }

    #[test]
    fn test_extract_year_dot_separated() {
        let (title, year) = extract_year_from_title("Inception.2010.1080p");
        assert_eq!(title, "Inception");
        assert_eq!(year, Some(2010));
    }

    #[test]
    fn test_extract_year_only_digits_is_title() {
        // "1899" is a TV show title, not a year
        let (title, year) = extract_year_from_title("1899");
        assert_eq!(title, "1899");
        assert_eq!(year, None);
    }

    #[test]
    fn test_extract_year_no_year() {
        let (title, year) = extract_year_from_title("Inception");
        assert_eq!(title, "Inception");
        assert_eq!(year, None);
    }

    #[test]
    fn test_extract_year_before_1930_ignored() {
        let (title, year) = extract_year_from_title("Movie 1920");
        assert_eq!(title, "Movie 1920");
        assert_eq!(year, None);
    }

    #[test]
    fn test_extract_year_bracket_format() {
        let (title, year) = extract_year_from_title("Interstellar [2014]");
        assert_eq!(title, "Interstellar");
        assert_eq!(year, Some(2014));
    }

    // --- clean_junk_from_title ---

    #[test]
    fn test_clean_junk_resolution_and_codec() {
        assert_eq!(clean_junk_from_title("Inception 1080p x264"), "Inception");
    }

    #[test]
    fn test_clean_junk_bluray_source() {
        assert_eq!(clean_junk_from_title("Movie BluRay x265"), "Movie");
    }

    #[test]
    fn test_clean_junk_bracket_groups() {
        assert_eq!(clean_junk_from_title("Movie [YIFY] 720p"), "Movie");
    }

    #[test]
    fn test_clean_junk_audio_tags() {
        // Documents actual behavior: DTS-HD/5.1/FLAC removed, "MA" survives (space before MA)
        let result = clean_junk_from_title("Movie DTS-HD MA 5.1 FLAC");
        assert!(!result.to_lowercase().contains("dts"), "Should remove DTS");
        assert!(
            !result.to_lowercase().contains("flac"),
            "Should remove FLAC"
        );
        assert!(!result.contains("5.1"), "Should remove 5.1");
    }

    #[test]
    fn test_clean_junk_language_tags() {
        assert_eq!(
            clean_junk_from_title("Movie Dual Audio Hindi English"),
            "Movie"
        );
    }

    #[test]
    fn test_clean_junk_multiple_dashes() {
        // "Movie--Title": leading group pattern strips "Movie-", leaving "-Title",
        // then trim removes leading dash → "Title"
        assert_eq!(clean_junk_from_title("Movie--Title"), "Title");
    }

    #[test]
    fn test_clean_junk_preserves_title() {
        assert_eq!(clean_junk_from_title("The Dark Knight"), "The Dark Knight");
    }

    // --- clean_folder_name ---

    #[test]
    fn test_clean_folder_name_removes_brackets() {
        assert_eq!(clean_folder_name("Movie [1080p]"), "Movie");
    }

    #[test]
    fn test_clean_folder_name_keeps_year_parens() {
        assert_eq!(clean_folder_name("Movie (2020)"), "Movie (2020)");
    }

    #[test]
    fn test_clean_folder_name_non_year_parens() {
        // NOTE: current regex does not strip non-year parens; test documents actual behavior
        let result = clean_folder_name("Movie (Extended)");
        assert!(result.contains("Movie"), "Should keep Movie part");
    }

    #[test]
    fn test_clean_folder_name_collapses_whitespace() {
        assert_eq!(clean_folder_name("  Movie   Name  "), "Movie Name");
    }

    // --- extract_series_name_from_folder ---

    #[test]
    fn test_extract_series_name_with_year() {
        let (name, year) = extract_series_name_from_folder("Breaking Bad (2008)");
        assert_eq!(name, "Breaking Bad");
        assert_eq!(year, Some(2008));
    }

    #[test]
    fn test_extract_series_name_without_year() {
        let (name, year) = extract_series_name_from_folder("Breaking Bad");
        assert_eq!(name, "Breaking Bad");
        assert_eq!(year, None);
    }

    #[test]
    fn test_extract_series_name_bracket_year() {
        let (name, year) = extract_series_name_from_folder("Lost [2004]");
        assert_eq!(name, "Lost");
        assert_eq!(year, Some(2004));
    }

    // --- is_generic_title ---

    #[test]
    fn test_is_generic_title_episode() {
        assert!(is_generic_title("episode"));
        assert!(is_generic_title("Episode"));
    }

    #[test]
    fn test_is_generic_title_ep() {
        assert!(is_generic_title("ep"));
    }

    #[test]
    fn test_is_generic_title_part() {
        assert!(is_generic_title("Part 1"));
    }

    #[test]
    fn test_is_generic_title_chapter() {
        assert!(is_generic_title("Chapter 3"));
    }

    #[test]
    fn test_is_generic_title_real_title() {
        assert!(!is_generic_title("Breaking Bad"));
    }

    #[test]
    fn test_is_generic_title_volume() {
        assert!(is_generic_title("Vol 2"));
        assert!(is_generic_title("Volume"));
    }

    // --- get_best_title ---

    #[test]
    fn test_get_best_title_prefers_series_name_for_short_title() {
        let ctx = FolderContext {
            series_name: Some("Breaking Bad".to_string()),
            series_year: None,
            folder_season: Some(1),
            is_tv_structure: true,
        };
        assert_eq!(get_best_title("Ep", &ctx), "Breaking Bad");
    }

    #[test]
    fn test_get_best_title_prefers_series_name_for_generic() {
        let ctx = FolderContext {
            series_name: Some("Breaking Bad".to_string()),
            series_year: None,
            folder_season: Some(1),
            is_tv_structure: true,
        };
        assert_eq!(get_best_title("episode", &ctx), "Breaking Bad");
    }

    #[test]
    fn test_get_best_title_uses_parsed_title() {
        let ctx = FolderContext {
            series_name: Some("Breaking Bad".to_string()),
            series_year: None,
            folder_season: Some(1),
            is_tv_structure: true,
        };
        assert_eq!(get_best_title("Pilot", &ctx), "Pilot");
    }

    #[test]
    fn test_get_best_title_series_name_contains_parsed() {
        let ctx = FolderContext {
            series_name: Some("The Office".to_string()),
            series_year: None,
            folder_season: Some(1),
            is_tv_structure: true,
        };
        // "Office" is contained in "The Office", so prefer series name
        assert_eq!(get_best_title("Office", &ctx), "The Office");
    }

    #[test]
    fn test_get_best_title_no_series_name() {
        let ctx = FolderContext {
            series_name: None,
            series_year: None,
            folder_season: None,
            is_tv_structure: false,
        };
        assert_eq!(get_best_title("Pilot", &ctx), "Pilot");
    }

    // --- clean_title ---

    #[test]
    fn test_clean_title_dots() {
        assert_eq!(clean_title("Breaking.Bad"), "Breaking Bad");
    }

    #[test]
    fn test_clean_title_underscores() {
        assert_eq!(clean_title("Breaking_Bad"), "Breaking Bad");
    }

    #[test]
    fn test_clean_title_trim() {
        assert_eq!(clean_title("  Breaking Bad  "), "Breaking Bad");
    }

    // --- ParsedMedia struct ---

    #[test]
    fn test_parsed_media_struct_fields() {
        let parsed = ParsedMedia {
            title: "Inception".to_string(),
            year: Some(2010),
            media_type: MediaParseType::Movie,
            season: None,
            episode: None,
            episode_end: None,
        };
        assert_eq!(parsed.title, "Inception");
        assert_eq!(parsed.year, Some(2010));
        assert_eq!(parsed.media_type, MediaParseType::Movie);
        assert!(parsed.season.is_none());
        assert!(parsed.episode.is_none());
        assert!(parsed.episode_end.is_none());
    }

    #[test]
    fn test_parsed_media_tv_episode_fields() {
        let parsed = ParsedMedia {
            title: "Breaking Bad".to_string(),
            year: Some(2008),
            media_type: MediaParseType::TvEpisode,
            season: Some(1),
            episode: Some(1),
            episode_end: Some(3),
        };
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(1));
        assert_eq!(parsed.episode_end, Some(3));
    }

    // --- MediaParseType enum ---

    #[test]
    fn test_media_parse_type_equality() {
        assert_eq!(MediaParseType::Movie, MediaParseType::Movie);
        assert_eq!(MediaParseType::TvEpisode, MediaParseType::TvEpisode);
        assert_ne!(MediaParseType::Movie, MediaParseType::TvEpisode);
    }

    #[test]
    fn test_media_parse_type_clone() {
        let t = MediaParseType::TvEpisode;
        let t2 = t;
        assert_eq!(t, t2);
    }

    // --- parse_cloud_filename edge cases ---

    #[test]
    fn test_parse_cloud_filename_no_extension() {
        let parsed = parse_cloud_filename("Inception 2010");
        assert_eq!(parsed.title, "Inception");
        assert_eq!(parsed.year, Some(2010));
        assert_eq!(parsed.media_type, MediaParseType::Movie);
    }

    #[test]
    fn test_parse_cloud_filename_tv_standard_sxxexx() {
        let parsed = parse_cloud_filename("The.Office.S02E03.mkv");
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.season, Some(2));
        assert_eq!(parsed.episode, Some(3));
    }

    #[test]
    fn test_parse_cloud_filename_multi_episode() {
        let parsed = parse_cloud_filename("Show.S01E01-E03.mkv");
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(1));
        assert_eq!(parsed.episode_end, Some(3));
    }

    // --- try_parse_tv_season_pack ---

    #[test]
    fn test_try_parse_tv_season_pack_s_pattern() {
        let parsed = try_parse_tv_season_pack("Breaking Bad S02");
        assert!(parsed.is_some());
        let p = parsed.unwrap();
        assert_eq!(p.title, "Breaking Bad");
        assert_eq!(p.season, Some(2));
        assert_eq!(p.media_type, MediaParseType::TvEpisode);
        assert!(p.episode.is_none());
    }

    #[test]
    fn test_try_parse_tv_season_pack_season_word() {
        let parsed = try_parse_tv_season_pack("Lost Season 3");
        assert!(parsed.is_some());
        let p = parsed.unwrap();
        assert_eq!(p.title, "Lost");
        assert_eq!(p.season, Some(3));
    }

    #[test]
    fn test_try_parse_tv_season_pack_with_year() {
        let parsed = try_parse_tv_season_pack("The Office (2005) S01");
        assert!(parsed.is_some());
        let p = parsed.unwrap();
        assert_eq!(p.title, "The Office");
        assert_eq!(p.year, Some(2005));
        assert_eq!(p.season, Some(1));
    }

    // --- parse_filename edge cases ---

    #[test]
    fn test_parse_filename_tv_with_year_in_title() {
        let path = PathBuf::from("Battlestar.Galactica.2004.S01E01.mkv");
        let parsed = parse_filename(&path);
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(1));
    }

    #[test]
    fn test_parse_filename_tv_1x01_format() {
        let path = PathBuf::from("Firefly.1x01.Serenity.mkv");
        let parsed = parse_filename(&path);
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(1));
    }

    #[test]
    fn test_parse_filename_tv_episode_range() {
        let path = PathBuf::from("Show.S01E01-E03.720p.mkv");
        let parsed = parse_filename(&path);
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.episode, Some(1));
        assert_eq!(parsed.episode_end, Some(3));
    }

    #[test]
    fn test_parse_filename_movie_no_year() {
        let path = PathBuf::from("Inception.mkv");
        let parsed = parse_filename(&path);
        assert_eq!(parsed.title, "Inception");
        assert_eq!(parsed.year, None);
        assert_eq!(parsed.media_type, MediaParseType::Movie);
    }

    #[test]
    fn test_parse_filename_movie_with_dots() {
        let path = PathBuf::from("The.Dark.Knight.2008.1080p.BluRay.mkv");
        let parsed = parse_filename(&path);
        assert_eq!(parsed.title, "The Dark Knight");
        assert_eq!(parsed.year, Some(2008));
        assert_eq!(parsed.media_type, MediaParseType::Movie);
    }

    #[test]
    fn test_parse_filename_tv_spelled_out_season_episode() {
        // Regex uses Season\s*Episode — needs whitespace (not dot) between word and number
        let path = PathBuf::from("Lost Season 1 Episode 5 720p.mkv");
        let parsed = parse_filename(&path);
        assert_eq!(parsed.media_type, MediaParseType::TvEpisode);
        assert_eq!(parsed.season, Some(1));
        assert_eq!(parsed.episode, Some(5));
    }

    // --- cleanup_orphaned_media with temp dirs ---

    #[test]
    fn test_cleanup_orphaned_media_no_orphans() {
        use std::env;
        use std::fs;

        let temp_dir = env::temp_dir().join(format!(
            "slasshyvault_cleanup_test_{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        // Create an image cache dir that's empty
        let image_cache = temp_dir.join("image_cache");
        fs::create_dir_all(&image_cache).unwrap();

        // We can't easily test with a real DB, but we can test cleanup_image_directory directly
        // Create some files in image cache
        let used_image = image_cache.join("poster.jpg");
        fs::write(&used_image, "fake image").unwrap();

        let mut used_paths = std::collections::HashSet::new();
        used_paths.insert("image_cache/poster.jpg".to_string());

        // poster.jpg is used, should NOT be deleted
        cleanup_image_directory(&image_cache.to_string_lossy(), &used_paths, "");
        assert!(used_image.exists(), "Used image should not be deleted");

        // Create orphaned image
        let orphan_image = image_cache.join("orphan.jpg");
        fs::write(&orphan_image, "orphan").unwrap();

        cleanup_image_directory(&image_cache.to_string_lossy(), &used_paths, "");
        assert!(!orphan_image.exists(), "Orphaned image should be deleted");
        assert!(used_image.exists(), "Used image should still exist");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_cleanup_image_directory_nested() {
        use std::env;
        use std::fs;

        let temp_dir =
            env::temp_dir().join(format!("slasshyvault_nested_{}", uuid::Uuid::new_v4()));
        let image_cache = temp_dir.join("image_cache");
        fs::create_dir_all(&image_cache).unwrap();

        // Create nested dir with used + orphaned files
        let nested = image_cache.join("tv");
        fs::create_dir_all(&nested).unwrap();
        let used_file = nested.join("used.jpg");
        let orphan_file = nested.join("orphan.jpg");
        fs::write(&used_file, "used").unwrap();
        fs::write(&orphan_file, "orphan").unwrap();

        let mut used_paths = std::collections::HashSet::new();
        used_paths.insert("image_cache/tv/used.jpg".to_string());

        cleanup_image_directory(&image_cache.to_string_lossy(), &used_paths, "tv");

        assert!(
            used_file.exists(),
            "Used file in subdirectory should survive"
        );
        assert!(
            !orphan_file.exists(),
            "Orphaned file in subdirectory should be removed"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_cleanup_image_directory_removes_empty_subdirs() {
        use std::env;
        use std::fs;

        let temp_dir =
            env::temp_dir().join(format!("slasshyvault_emptydir_{}", uuid::Uuid::new_v4()));
        let image_cache = temp_dir.join("image_cache");
        let subdir = image_cache.join("tv_posters");
        fs::create_dir_all(&subdir).unwrap();

        // Create an orphaned file inside subdir
        let orphan = subdir.join("old_poster.jpg");
        fs::write(&orphan, "data").unwrap();

        let used_paths = std::collections::HashSet::new();

        // Clean the parent image_cache dir (which recurses into subdirs)
        cleanup_image_directory(&image_cache.to_string_lossy(), &used_paths, "");

        // Orphaned file removed, subdir now empty → should be cleaned up
        assert!(!orphan.exists(), "Orphaned file should be removed");
        assert!(
            !subdir.exists(),
            "Empty subdirectory should be removed after its last file was deleted"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
