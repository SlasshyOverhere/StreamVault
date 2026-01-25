// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod database;
mod config;
mod media_manager;
mod tmdb;
mod mpv_ipc;
mod gdrive;
mod transcoder;

use tauri_plugin_autostart::MacosLauncher;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use tauri::{State, Window, Manager, SystemTray, SystemTrayMenu, SystemTrayMenuItem, CustomMenuItem, SystemTrayEvent, AppHandle, WindowBuilder, WindowUrl};
use serde::Serialize;
use notify_rust::Notification;
use std::sync::mpsc;
use std::time::Duration;

// Channel for receiving OAuth codes from deep links
lazy_static::lazy_static! {
    static ref OAUTH_CODE_CHANNEL: (Mutex<mpsc::Sender<String>>, Mutex<mpsc::Receiver<String>>) = {
        let (tx, rx) = mpsc::channel();
        (Mutex::new(tx), Mutex::new(rx))
    };
}

// MPV session info
#[derive(Clone, Serialize)]
pub struct MpvSession {
    pub media_id: i64,
    pub pid: u32,
    pub title: String,
    pub start_time: i64,
}

// Application state
pub struct AppState {
    pub db: Mutex<database::Database>,
    pub config: Mutex<config::Config>,
    pub is_scanning: Arc<AtomicBool>,
    pub active_mpv_sessions: Mutex<HashMap<i64, MpvSession>>,
    pub gdrive_client: gdrive::GoogleDriveClient,
}

// API Response types
#[derive(Serialize)]
struct ApiResponse {
    message: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

// Scan event payloads
#[derive(Clone, Serialize)]
struct ScanProgressPayload {
    title: String,
    media_type: String,
}

#[derive(Clone, Serialize)]
struct ScanCompletePayload {
    movies_count: usize,
    tv_count: usize,
}

// Get library items (movies or TV shows)
#[tauri::command]
async fn get_library(
    state: State<'_, AppState>,
    media_type: String,
    search: Option<String>,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let db_type = if media_type == "tv" { "tvshow" } else { "movie" };
    db.get_library(db_type, search.as_deref())
        .map_err(|e| e.to_string())
}

// Get library filtered by cloud status
#[tauri::command]
async fn get_library_filtered(
    state: State<'_, AppState>,
    media_type: String,
    search: Option<String>,
    is_cloud: Option<bool>,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let db_type = if media_type == "tv" { "tvshow" } else { "movie" };
    db.get_library_filtered(db_type, search.as_deref(), is_cloud)
        .map_err(|e| e.to_string())
}

// Get episodes for a TV show
#[tauri::command]
async fn get_episodes(
    state: State<'_, AppState>,
    series_id: i64,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_episodes(series_id).map_err(|e| e.to_string())
}

// Get watch history
#[tauri::command]
async fn get_watch_history(
    state: State<'_, AppState>,
    limit: Option<i32>,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_watch_history(limit.unwrap_or(50))
        .map_err(|e| e.to_string())
}

// Remove a single item from watch history
#[tauri::command]
async fn remove_from_watch_history(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.remove_from_watch_history(media_id)
        .map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: "Item removed from watch history".to_string(),
    })
}

// Mark media as complete (set progress to 100%)
#[tauri::command]
async fn mark_as_complete(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    // Get the media item to find its duration
    let item = db.get_media_by_id(media_id).map_err(|e| e.to_string())?;

    let duration = item.duration_seconds.unwrap_or(3600.0); // Default 1 hour if no duration
    // Set position to duration (100% complete) - this will trigger the 95% reset in update_progress
    db.update_progress(media_id, duration, duration)
        .map_err(|e| e.to_string())?;

    Ok(ApiResponse {
        message: format!("Marked '{}' as complete", item.title),
    })
}

// Clear all watch history
#[tauri::command]
async fn clear_all_watch_history(
    state: State<'_, AppState>,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let count = db.clear_all_watch_history()
        .map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: format!("Cleared {} items from watch history", count),
    })
}

// ==================== STREAMING HISTORY COMMANDS ====================

// Save streaming progress (for Videasy player)
#[tauri::command]
async fn save_streaming_progress(
    state: State<'_, AppState>,
    tmdb_id: String,
    media_type: String,
    title: String,
    poster_path: Option<String>,
    season: Option<i32>,
    episode: Option<i32>,
    position: f64,
    duration: f64,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.save_streaming_progress(
        &tmdb_id,
        &media_type,
        &title,
        poster_path.as_deref(),
        season,
        episode,
        position,
        duration,
    ).map_err(|e| e.to_string())?;
    
    Ok(ApiResponse {
        message: "Streaming progress saved".to_string(),
    })
}

// Get streaming history
#[tauri::command]
async fn get_streaming_history(
    state: State<'_, AppState>,
    limit: Option<i32>,
) -> Result<Vec<database::StreamingHistoryItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_streaming_history(limit.unwrap_or(50))
        .map_err(|e| e.to_string())
}

// Get streaming resume info for a specific content
#[tauri::command]
async fn get_streaming_resume_info(
    state: State<'_, AppState>,
    tmdb_id: String,
    media_type: String,
    season: Option<i32>,
    episode: Option<i32>,
) -> Result<Option<database::StreamingHistoryItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_streaming_resume_info(&tmdb_id, &media_type, season, episode)
        .map_err(|e| e.to_string())
}

// Remove a single item from streaming history
#[tauri::command]
async fn remove_from_streaming_history(
    state: State<'_, AppState>,
    id: i64,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.remove_from_streaming_history(id)
        .map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: "Item removed from streaming history".to_string(),
    })
}

// Clear all streaming history
#[tauri::command]
async fn clear_all_streaming_history(
    state: State<'_, AppState>,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let count = db.clear_all_streaming_history()
        .map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: format!("Cleared {} items from streaming history", count),
    })
}

// ==================== GOOGLE DRIVE COMMANDS ====================

/// Check if user is connected to Google Drive
#[tauri::command]
async fn gdrive_is_connected(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.gdrive_client.is_authenticated())
}

/// Get Google Drive account info
#[tauri::command]
async fn gdrive_get_account_info(
    state: State<'_, AppState>,
) -> Result<gdrive::DriveAccountInfo, String> {
    state.gdrive_client.get_account_info().await
}

/// Start Google Drive OAuth flow - returns auth URL
#[tauri::command]
async fn gdrive_start_auth() -> Result<String, String> {
    let auth_url = gdrive::get_auth_url();

    // Open the URL in the default browser
    if let Err(e) = open::that(&auth_url) {
        println!("[GDRIVE] Failed to open browser: {}", e);
        // Return URL anyway so user can copy it
    }

    Ok(auth_url)
}

/// Wait for OAuth callback and complete authentication
/// The backend handles token exchange and sends tokens directly to localhost callback
#[tauri::command]
async fn gdrive_complete_auth(state: State<'_, AppState>) -> Result<gdrive::DriveAccountInfo, String> {
    println!("[GDRIVE] Waiting for OAuth callback...");

    // Wait for tokens from backend (it redirects to localhost with tokens)
    let tokens = gdrive::wait_for_oauth_callback().await?;
    println!("[GDRIVE] Received tokens from backend");

    // Store tokens
    state.gdrive_client.store_tokens(tokens)?;
    println!("[GDRIVE] Tokens stored successfully");

    // Get and return account info
    state.gdrive_client.get_account_info().await
}

/// Disconnect from Google Drive
#[tauri::command]
async fn gdrive_disconnect(state: State<'_, AppState>) -> Result<ApiResponse, String> {
    state.gdrive_client.clear_tokens()?;
    Ok(ApiResponse {
        message: "Disconnected from Google Drive".to_string(),
    })
}

/// Complete OAuth with manually entered authorization code
/// NOTE: This is deprecated. The new flow uses backend proxy which handles token exchange.
#[tauri::command]
async fn gdrive_auth_with_code(
    _state: State<'_, AppState>,
    _code: String,
) -> Result<gdrive::DriveAccountInfo, String> {
    Err("Manual code entry is no longer supported. Please use the 'Connect Google Drive' button which opens the browser for authentication.".to_string())
}

/// List folders in Google Drive
#[tauri::command]
async fn gdrive_list_folders(
    state: State<'_, AppState>,
    parent_id: Option<String>,
) -> Result<Vec<gdrive::DriveItem>, String> {
    state.gdrive_client.list_folders(parent_id.as_deref()).await
}

/// List all files in a folder
#[tauri::command]
async fn gdrive_list_files(
    state: State<'_, AppState>,
    folder_id: Option<String>,
) -> Result<gdrive::DriveListResponse, String> {
    state.gdrive_client.list_files(folder_id.as_deref(), None).await
}

/// List video files in a folder (with optional recursive scan)
#[tauri::command]
async fn gdrive_list_video_files(
    state: State<'_, AppState>,
    folder_id: String,
    recursive: bool,
) -> Result<Vec<gdrive::DriveItem>, String> {
    state.gdrive_client.list_video_files(&folder_id, recursive).await
}

/// Get streaming URL for a Google Drive file
#[tauri::command]
async fn gdrive_get_stream_url(
    state: State<'_, AppState>,
    file_id: String,
) -> Result<(String, String), String> {
    state.gdrive_client.get_stream_url(&file_id).await
}

/// Get file metadata from Google Drive
#[tauri::command]
async fn gdrive_get_file_metadata(
    state: State<'_, AppState>,
    file_id: String,
) -> Result<gdrive::DriveItem, String> {
    state.gdrive_client.get_file_metadata(&file_id).await
}

/// Cloud folder info for indexing
#[derive(serde::Deserialize)]
struct CloudFolderInfo {
    id: String,
    name: String,
    #[serde(rename = "type")]
    folder_type: String, // "movies" or "tv"
}

/// Result of cloud indexing
#[derive(serde::Serialize)]
struct CloudIndexResult {
    success: bool,
    indexed_count: usize,
    skipped_count: usize,
    movies_count: usize,
    tv_count: usize,
    message: String,
}

/// Scan a cloud folder and index its contents
/// Auto-detects movies vs TV shows based on filename patterns
#[tauri::command]
async fn gdrive_scan_folder(
    state: State<'_, AppState>,
    window: Window,
    folder_id: String,
    folder_name: String,
) -> Result<CloudIndexResult, String> {
    println!("[CLOUD] Starting scan of folder: {} (auto-detect)", folder_name);

    // Get video files from the folder
    let files = state.gdrive_client.list_video_files(&folder_id, true).await?;
    println!("[CLOUD] Found {} video files", files.len());

    // Get API key from config
    let api_key = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    // API key check is no longer needed since we have a default

    // Get image cache dir for poster downloads
    let image_cache_dir = database::get_image_cache_dir();
    std::fs::create_dir_all(&image_cache_dir).ok();

    // Clone data for the blocking task
    let folder_id_clone = folder_id.clone();

    // Get database path for creating new connection in blocking task
    let db_path = database::get_database_path();

    // Run the blocking indexing work in a separate thread
    let result = tokio::task::spawn_blocking(move || {
        use std::collections::HashMap;

        // Create a new database connection for this thread
        let db = match database::Database::new(&db_path) {
            Ok(d) => d,
            Err(e) => return Err(format!("Failed to open database: {}", e)),
        };

        let mut indexed_count = 0;
        let mut skipped_count = 0;
        let mut movies_count = 0;
        let mut tv_count = 0;

        // Cache for TV shows: title -> (db_id, tmdb_id, show_folder_id)
        let mut tv_show_cache: HashMap<String, (i64, Option<String>, String)> = HashMap::new();

        // Cache for season episodes: (tmdb_id, season) -> Vec<episode_info>
        let mut season_cache: HashMap<(String, i32), Vec<tmdb::TmdbEpisodeInfo>> = HashMap::new();

        for file in files {
            // Check if already indexed
            if db.cloud_file_exists(&file.id) {
                skipped_count += 1;
                continue;
            }

            // Parse filename to extract metadata
            let parsed = media_manager::parse_cloud_filename(&file.name);

            // Auto-detect: if we have season and episode numbers, it's a TV show
            let is_tv_show = parsed.season.is_some() && parsed.episode.is_some();

            if is_tv_show {
                // Index as TV episode
                let season = parsed.season.unwrap();
                let episode = parsed.episode.unwrap();
                let show_title = parsed.title.clone();
                let show_title_lower = show_title.to_lowercase();

                // Get the episode's actual parent folder (the TV show folder, not the tracked folder)
                let episode_parent_folder = file.parents.as_ref()
                    .and_then(|p| p.first())
                    .cloned()
                    .unwrap_or_else(|| folder_id_clone.clone());

                // Check cache first, then database, then TMDB
                let (db_show_id, tmdb_id, show_folder_id) = if let Some(cached) = tv_show_cache.get(&show_title_lower) {
                    cached.clone()
                } else {
                    // Check if show already exists in database
                    let existing = db.find_tvshow_by_title(&show_title);

                    let result = if let Ok(Some(existing_show)) = existing {
                        // Use existing show's folder or the episode's parent
                        (existing_show.id, existing_show.tmdb_id, episode_parent_folder.clone())
                    } else {
                        // Search TMDB for the show (only once per show)
                        println!("[CLOUD] Searching TMDB for show: {}", show_title);
                        let tmdb_result = tmdb::search_metadata(
                            &api_key,
                            &show_title,
                            "tv",
                            parsed.year,
                            &image_cache_dir,
                        ).ok().flatten();

                        // Create the show
                        let (title, year, overview, poster_path, tmdb_id_opt) = match &tmdb_result {
                            Some(meta) => (
                                meta.title.clone(),
                                meta.year,
                                meta.overview.clone(),
                                meta.poster_path.clone(),
                                meta.tmdb_id.clone(),
                            ),
                            None => (show_title.clone(), None, None, None, None),
                        };

                        // Use episode's parent folder as the show's folder ID (for deletion)
                        match db.insert_cloud_tvshow(
                            &title,
                            year,
                            overview.as_deref(),
                            poster_path.as_deref(),
                            &format!("gdrive:{}", episode_parent_folder),
                            &episode_parent_folder,  // Use episode's parent folder, not tracked folder
                            tmdb_id_opt.as_deref(),
                        ) {
                            Ok(show_id) => (show_id, tmdb_id_opt, episode_parent_folder.clone()),
                            Err(e) => {
                                println!("[CLOUD] Failed to insert show: {}", e);
                                continue;
                            }
                        }
                    };

                    // Cache the result
                    tv_show_cache.insert(show_title_lower.clone(), result.clone());
                    result
                };

                // Get episode metadata from cache or TMDB
                let (ep_title, ep_overview, ep_still): (Option<String>, Option<String>, Option<String>) =
                    if let Some(ref tid) = tmdb_id {
                        let cache_key = (tid.clone(), season);

                        // Check season cache
                        let episodes = if let Some(cached_episodes) = season_cache.get(&cache_key) {
                            cached_episodes.clone()
                        } else {
                            // Fetch from TMDB (only once per season)
                            println!("[CLOUD] Fetching season {} episodes for {}", season, show_title);
                            match tmdb::fetch_season_episodes(&api_key, tid, season, &show_title, &image_cache_dir) {
                                Ok(season_info) => {
                                    let eps = season_info.episodes.clone();
                                    season_cache.insert(cache_key.clone(), eps.clone());
                                    eps
                                }
                                Err(_) => {
                                    season_cache.insert(cache_key.clone(), Vec::new());
                                    Vec::new()
                                }
                            }
                        };

                        // Find our episode in the cached list
                        episodes.iter()
                            .find(|e| e.episode_number == episode)
                            .map(|e| (Some(e.name.clone()), e.overview.clone(), e.still_path.clone()))
                            .unwrap_or((None, None, None))
                    } else {
                        (None, None, None)
                    };

                // Insert episode
                if let Err(e) = db.insert_cloud_episode(
                    &show_title,
                    &file.name,
                    db_show_id,
                    season,
                    episode,
                    &file.id,
                    &show_folder_id,  // Use the show's folder ID, not tracked folder
                    ep_title.as_deref(),
                    ep_overview.as_deref(),
                    ep_still.as_deref(),
                ) {
                    println!("[CLOUD] Failed to insert episode: {}", e);
                    continue;
                }

                indexed_count += 1;
                tv_count += 1;
                println!("[CLOUD] Indexed TV: {} S{:02}E{:02}", show_title, season, episode);

            } else {
                // Index as movie
                println!("[CLOUD] Searching TMDB for movie: {}", parsed.title);
                let tmdb_result = tmdb::search_metadata(
                    &api_key,
                    &parsed.title,
                    "movie",
                    parsed.year,
                    &image_cache_dir,
                ).ok().flatten();

                let (title, year, overview, poster_path, tmdb_id) = match tmdb_result {
                    Some(meta) => (
                        meta.title,
                        meta.year,
                        meta.overview,
                        meta.poster_path,
                        meta.tmdb_id,
                    ),
                    None => (parsed.title.clone(), parsed.year, None, None, None),
                };

                // Insert into database
                if let Err(e) = db.insert_cloud_movie(
                    &title,
                    year,
                    overview.as_deref(),
                    poster_path.as_deref(),
                    &file.name,
                    &file.id,
                    &folder_id_clone,
                    tmdb_id.as_deref(),
                ) {
                    println!("[CLOUD] Failed to insert movie: {}", e);
                    continue;
                }

                indexed_count += 1;
                movies_count += 1;
                println!("[CLOUD] Indexed Movie: {}", title);
            }
        }

        Ok((indexed_count, skipped_count, movies_count, tv_count))
    }).await.map_err(|e| format!("Task failed: {}", e))??;

    let (indexed_count, skipped_count, movies_count, tv_count) = result;

    // Emit completion
    window.emit("cloud-scan-complete", serde_json::json!({
        "folder": folder_name,
        "indexed": indexed_count,
        "skipped": skipped_count,
        "movies": movies_count,
        "tv": tv_count
    })).ok();

    window.emit("library-updated", ()).ok();

    let message = format!(
        "Indexed {} items ({} movies, {} TV episodes) from '{}' ({} skipped)",
        indexed_count, movies_count, tv_count, folder_name, skipped_count
    );
    println!("[CLOUD] {}", message);

    Ok(CloudIndexResult {
        success: true,
        indexed_count,
        skipped_count,
        movies_count,
        tv_count,
        message,
    })
}

/// Delete all indexed media from a cloud folder
#[tauri::command]
async fn gdrive_delete_folder_media(
    state: State<'_, AppState>,
    window: Window,
    folder_id: String,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let deleted = db.delete_cloud_folder_media(&folder_id).map_err(|e| e.to_string())?;

    window.emit("library-updated", ()).ok();

    Ok(ApiResponse {
        message: format!("Deleted {} cloud media items", deleted),
    })
}

// ==================== CLOUD FOLDER MANAGEMENT ====================

/// Add a cloud folder to track (stored in database, auto-scanned)
#[tauri::command]
async fn add_cloud_folder(
    state: State<'_, AppState>,
    folder_id: String,
    folder_name: String,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.add_cloud_folder(&folder_id, &folder_name).map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: format!("Added cloud folder: {}", folder_name),
    })
}

/// Remove a cloud folder from tracking
#[tauri::command]
async fn remove_cloud_folder(
    state: State<'_, AppState>,
    window: Window,
    folder_id: String,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    // Delete media from this folder
    let deleted_media = db.delete_cloud_folder_media(&folder_id).map_err(|e| e.to_string())?;

    // Remove folder from tracking
    db.remove_cloud_folder(&folder_id).map_err(|e| e.to_string())?;

    window.emit("library-updated", ()).ok();

    Ok(ApiResponse {
        message: format!("Removed cloud folder and {} media items", deleted_media),
    })
}

/// Get all tracked cloud folders
#[tauri::command]
async fn get_cloud_folders(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let folders = db.get_cloud_folders().map_err(|e| e.to_string())?;

    Ok(folders.into_iter().map(|(id, name, auto_scan)| {
        serde_json::json!({
            "id": id,
            "name": name,
            "auto_scan": auto_scan
        })
    }).collect())
}

/// Scan all cloud folders for new files (incremental scan)
#[tauri::command]
async fn scan_all_cloud_folders(
    state: State<'_, AppState>,
    window: Window,
) -> Result<CloudIndexResult, String> {
    // Get all tracked folders
    let folders = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_cloud_folders().map_err(|e| e.to_string())?
    };

    if folders.is_empty() {
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: 0,
            movies_count: 0,
            tv_count: 0,
            message: "No cloud folders configured".to_string(),
        });
    }

    let mut total_indexed = 0;
    let mut total_skipped = 0;
    let mut total_movies = 0;
    let mut total_tv = 0;

    for (folder_id, folder_name, _) in folders {
        println!("[CLOUD SCAN] Scanning folder: {} ({})", folder_name, folder_id);

        // Get video files from the folder
        let files = match state.gdrive_client.list_video_files(&folder_id, true).await {
            Ok(f) => f,
            Err(e) => {
                println!("[CLOUD SCAN] Error listing files for {}: {}", folder_name, e);
                continue;
            }
        };

        // Get API key from config
        let api_key = {
            let config = state.config.lock().map_err(|e| e.to_string())?;
            tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
        };

        // API key is always available now with default

        // Get image cache dir for poster downloads
        let image_cache_dir = database::get_image_cache_dir();
        std::fs::create_dir_all(&image_cache_dir).ok();

        // Clone data for the blocking task
        let folder_id_clone = folder_id.clone();
        let db_path = database::get_database_path();

        // Run the blocking indexing work in a separate thread
        let result = tokio::task::spawn_blocking(move || {
            use std::collections::HashMap;

            let db = match database::Database::new(&db_path) {
                Ok(d) => d,
                Err(e) => return Err(format!("Failed to open database: {}", e)),
            };

            let mut indexed_count = 0;
            let mut skipped_count = 0;
            let mut movies_count = 0;
            let mut tv_count = 0;

            let mut tv_show_cache: HashMap<String, (i64, Option<String>)> = HashMap::new();
            let mut season_cache: HashMap<(String, i32), Vec<tmdb::TmdbEpisodeInfo>> = HashMap::new();

            for file in files {
                if db.cloud_file_exists(&file.id) {
                    skipped_count += 1;
                    continue;
                }

                let parsed = media_manager::parse_cloud_filename(&file.name);
                let is_tv_show = parsed.season.is_some() && parsed.episode.is_some();

                if is_tv_show {
                    let season = parsed.season.unwrap();
                    let episode = parsed.episode.unwrap();
                    let show_title = parsed.title.clone();
                    let show_title_lower = show_title.to_lowercase();

                    let (db_show_id, tmdb_id) = if let Some(cached) = tv_show_cache.get(&show_title_lower) {
                        cached.clone()
                    } else {
                        let existing = db.find_tvshow_by_title(&show_title);
                        let result = if let Ok(Some(existing_show)) = existing {
                            (existing_show.id, existing_show.tmdb_id)
                        } else {
                            let tmdb_result = tmdb::search_metadata(
                                &api_key, &show_title, "tv", parsed.year, &image_cache_dir,
                            ).ok().flatten();

                            let (title, year, overview, poster_path, tmdb_id_opt) = match &tmdb_result {
                                Some(meta) => (meta.title.clone(), meta.year, meta.overview.clone(), meta.poster_path.clone(), meta.tmdb_id.clone()),
                                None => (show_title.clone(), None, None, None, None),
                            };

                            match db.insert_cloud_tvshow(&title, year, overview.as_deref(), poster_path.as_deref(),
                                &format!("gdrive:{}", folder_id_clone), &folder_id_clone, tmdb_id_opt.as_deref()) {
                                Ok(show_id) => (show_id, tmdb_id_opt),
                                Err(_) => continue,
                            }
                        };
                        tv_show_cache.insert(show_title_lower.clone(), result.clone());
                        result
                    };

                    let (ep_title, ep_overview, ep_still): (Option<String>, Option<String>, Option<String>) =
                        if let Some(ref tid) = tmdb_id {
                            let cache_key = (tid.clone(), season);
                            let episodes = if let Some(cached_episodes) = season_cache.get(&cache_key) {
                                cached_episodes.clone()
                            } else {
                                match tmdb::fetch_season_episodes(&api_key, tid, season, &show_title, &image_cache_dir) {
                                    Ok(season_info) => {
                                        let eps = season_info.episodes.clone();
                                        season_cache.insert(cache_key.clone(), eps.clone());
                                        eps
                                    }
                                    Err(_) => {
                                        season_cache.insert(cache_key.clone(), Vec::new());
                                        Vec::new()
                                    }
                                }
                            };
                            episodes.iter()
                                .find(|e| e.episode_number == episode)
                                .map(|e| (Some(e.name.clone()), e.overview.clone(), e.still_path.clone()))
                                .unwrap_or((None, None, None))
                        } else {
                            (None, None, None)
                        };

                    if db.insert_cloud_episode(&show_title, &file.name, db_show_id, season, episode,
                        &file.id, &folder_id_clone, ep_title.as_deref(), ep_overview.as_deref(), ep_still.as_deref()).is_err() {
                        continue;
                    }

                    indexed_count += 1;
                    tv_count += 1;
                } else {
                    let tmdb_result = tmdb::search_metadata(&api_key, &parsed.title, "movie", parsed.year, &image_cache_dir).ok().flatten();

                    let (title, year, overview, poster_path, tmdb_id) = match tmdb_result {
                        Some(meta) => (meta.title, meta.year, meta.overview, meta.poster_path, meta.tmdb_id),
                        None => (parsed.title.clone(), parsed.year, None, None, None),
                    };

                    if db.insert_cloud_movie(&title, year, overview.as_deref(), poster_path.as_deref(),
                        &file.name, &file.id, &folder_id_clone, tmdb_id.as_deref()).is_err() {
                        continue;
                    }

                    indexed_count += 1;
                    movies_count += 1;
                }
            }

            Ok((indexed_count, skipped_count, movies_count, tv_count))
        }).await.map_err(|e| format!("Task failed: {}", e))??;

        let (indexed, skipped, movies, tv) = result;
        total_indexed += indexed;
        total_skipped += skipped;
        total_movies += movies;
        total_tv += tv;

        // Update last scanned timestamp
        if let Ok(db) = state.db.lock() {
            let _ = db.update_cloud_folder_scanned(&folder_id);
        }

        if indexed > 0 {
            window.emit("library-updated", ()).ok();
        }
    }

    let message = format!(
        "Cloud scan complete: {} new ({} movies, {} TV shows), {} already indexed",
        total_indexed, total_movies, total_tv, total_skipped
    );

    window.emit("cloud-scan-complete", serde_json::json!({
        "indexed": total_indexed,
        "movies": total_movies,
        "tv": total_tv,
        "skipped": total_skipped
    })).ok();

    Ok(CloudIndexResult {
        success: true,
        indexed_count: total_indexed,
        skipped_count: total_skipped,
        movies_count: total_movies,
        tv_count: total_tv,
        message,
    })
}

/// Check for new cloud files using the efficient Changes API
/// This is MUCH lighter than scanning all folders - only returns changed files
#[tauri::command]
async fn check_cloud_changes(
    state: State<'_, AppState>,
    window: Window,
) -> Result<CloudIndexResult, String> {
    let start_time = std::time::Instant::now();
    println!("[CLOUD CHANGES] ══════════════════════════════════════════");
    println!("[CLOUD CHANGES] Starting change detection poll...");

    // Check if authenticated
    if !state.gdrive_client.is_authenticated() {
        println!("[CLOUD CHANGES] Not authenticated - skipping");
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: 0,
            movies_count: 0,
            tv_count: 0,
            message: "Not connected to Google Drive".to_string(),
        });
    }

    // Get or initialize the changes token
    let current_token = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_gdrive_changes_token().map_err(|e| e.to_string())?
    };

    let page_token = match current_token {
        Some(token) => {
            println!("[CLOUD CHANGES] Using existing token: {}...", &token[..token.len().min(20)]);
            token
        },
        None => {
            // First time - get the start token
            println!("[CLOUD CHANGES] No token found - initializing changes tracking...");
            let start_token = state.gdrive_client.get_changes_start_token().await?;
            println!("[CLOUD CHANGES] Got start token: {}...", &start_token[..start_token.len().min(20)]);

            // Save it
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.set_gdrive_changes_token(&start_token).map_err(|e| e.to_string())?;
            println!("[CLOUD CHANGES] Token saved - will detect changes on next poll");

            // Return empty result - we'll catch changes on next poll
            return Ok(CloudIndexResult {
                success: true,
                indexed_count: 0,
                skipped_count: 0,
                movies_count: 0,
                tv_count: 0,
                message: "Changes tracking initialized".to_string(),
            });
        }
    };

    // Get tracked folder IDs
    let tracked_folders: std::collections::HashSet<String> = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let folders = db.get_cloud_folders().map_err(|e| e.to_string())?;
        folders.into_iter().map(|(id, _, _)| id).collect()
    };

    if tracked_folders.is_empty() {
        println!("[CLOUD CHANGES] No cloud folders configured - skipping");
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: 0,
            movies_count: 0,
            tv_count: 0,
            message: "No cloud folders configured".to_string(),
        });
    }

    println!("[CLOUD CHANGES] Tracking {} folder(s)", tracked_folders.len());

    // Get changes since last check
    let api_start = std::time::Instant::now();
    let (changed_files, new_token) = state.gdrive_client.get_video_changes(&page_token).await?;
    let api_duration = api_start.elapsed();
    println!("[CLOUD CHANGES] Changes API call took {:?}", api_duration);

    // Save the new token immediately
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.set_gdrive_changes_token(&new_token).map_err(|e| e.to_string())?;
    }

    if changed_files.is_empty() {
        let total_duration = start_time.elapsed();
        println!("[CLOUD CHANGES] No changes detected (total: {:?})", total_duration);
        println!("[CLOUD CHANGES] ══════════════════════════════════════════");
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: 0,
            movies_count: 0,
            tv_count: 0,
            message: "No new files detected".to_string(),
        });
    }

    println!("[CLOUD CHANGES] ┌─────────────────────────────────────────");
    println!("[CLOUD CHANGES] │ DETECTED {} changed video file(s)!", changed_files.len());
    for file in &changed_files {
        println!("[CLOUD CHANGES] │   • {}", file.name);
    }
    println!("[CLOUD CHANGES] └─────────────────────────────────────────");

    // Filter to only files in our tracked folders
    let files_to_index: Vec<gdrive::DriveItem> = changed_files
        .into_iter()
        .filter(|file| {
            if let Some(ref parents) = file.parents {
                let in_tracked = parents.iter().any(|p| tracked_folders.contains(p));
                if !in_tracked {
                    println!("[CLOUD CHANGES] Skipping {} (not in tracked folders)", file.name);
                }
                in_tracked
            } else {
                println!("[CLOUD CHANGES] Skipping {} (no parent folder)", file.name);
                false
            }
        })
        .collect();

    if files_to_index.is_empty() {
        let total_duration = start_time.elapsed();
        println!("[CLOUD CHANGES] No files in tracked folders (total: {:?})", total_duration);
        println!("[CLOUD CHANGES] ══════════════════════════════════════════");
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: 0,
            movies_count: 0,
            tv_count: 0,
            message: "No new files in tracked folders".to_string(),
        });
    }

    println!("[CLOUD CHANGES] {} file(s) to index in tracked folders", files_to_index.len());

    // Emit event to show indexing has started
    window.emit("cloud-indexing-started", serde_json::json!({
        "count": files_to_index.len()
    })).ok();

    let image_cache_dir = database::get_image_cache_dir();
    std::fs::create_dir_all(&image_cache_dir).ok();

    let db_path = database::get_database_path();
    let _files_count = files_to_index.len();

    println!("[CLOUD CHANGES] ┌─────────────────────────────────────────");
    println!("[CLOUD CHANGES] │ PHASE 1: Adding files immediately (no metadata)");
    println!("[CLOUD CHANGES] └─────────────────────────────────────────");
    let index_start = std::time::Instant::now();

    // PHASE 1: Add files immediately without metadata
    let phase1_result = {
        let db_path_clone = db_path.clone();
        let files_to_index_clone: Vec<_> = files_to_index.iter().map(|f| {
            (f.id.clone(), f.name.clone(), f.parents.clone())
        }).collect();

        tokio::task::spawn_blocking(move || {
            let db = match database::Database::new(&db_path_clone) {
                Ok(d) => d,
                Err(e) => return Err(format!("Failed to open database: {}", e)),
            };

            let mut indexed_items: Vec<(i64, String, String, bool, Option<i32>, Option<i32>, String)> = Vec::new(); // (id, title, file_id, is_tv, season, episode, folder_id)
            let mut skipped_count = 0;
            let mut movies_count = 0;
            let mut tv_count = 0;

            // Cache for TV show IDs to avoid creating duplicates
            let mut tv_show_cache: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

            for (file_id, file_name, parents) in files_to_index_clone {
                // Check if already indexed (by cloud_file_id OR by file_path)
                if db.cloud_file_exists(&file_id) {
                    println!("[CLOUD CHANGES]   ⊘ Skipping (already indexed by file_id): {}", file_name);
                    skipped_count += 1;
                    continue;
                }

                // Also check if file_path already exists (from previous incomplete indexing)
                if let Ok(Some(_)) = db.get_media_by_file_path(&file_name) {
                    println!("[CLOUD CHANGES]   ⊘ Skipping (file_path already exists): {}", file_name);
                    skipped_count += 1;
                    continue;
                }

                // Get the parent folder ID
                let folder_id = parents.as_ref()
                    .and_then(|p| p.first())
                    .cloned()
                    .unwrap_or_default();

                let parsed = media_manager::parse_cloud_filename(&file_name);
                let is_tv_show = parsed.season.is_some() && parsed.episode.is_some();

                if is_tv_show {
                    let season = parsed.season.unwrap();
                    let episode = parsed.episode.unwrap();
                    let show_title = parsed.title.clone();
                    let show_title_lower = show_title.to_lowercase();

                    // Get or create TV show entry (without metadata for now)
                    let db_show_id = if let Some(&cached_id) = tv_show_cache.get(&show_title_lower) {
                        println!("[CLOUD CHANGES]   Using cached show ID {} for '{}'", cached_id, show_title);
                        cached_id
                    } else {
                        // Check if show exists in DB
                        let existing = db.find_tvshow_by_title(&show_title);
                        let show_id = match existing {
                            Ok(Some(existing_show)) => {
                                println!("[CLOUD CHANGES]   Found existing show '{}' with ID {}", show_title, existing_show.id);
                                existing_show.id
                            }
                            Ok(None) => {
                                // Create TV show entry without metadata
                                // Use a unique file_path combining folder ID and show title
                                let show_path = format!("gdrive:{}:{}", folder_id, show_title.to_lowercase().replace(" ", "_"));
                                println!("[CLOUD CHANGES]   Creating new TV show '{}' with path '{}'", show_title, show_path);
                                match db.insert_cloud_tvshow(&show_title, None, None, None,
                                    &show_path, &folder_id, None) {
                                    Ok(id) => {
                                        println!("[CLOUD CHANGES]   Created TV show with ID {}", id);
                                        id
                                    }
                                    Err(e) => {
                                        println!("[CLOUD CHANGES]   ERROR creating TV show: {}", e);
                                        continue;
                                    }
                                }
                            }
                            Err(e) => {
                                println!("[CLOUD CHANGES]   ERROR finding TV show: {}", e);
                                continue;
                            }
                        };
                        tv_show_cache.insert(show_title_lower, show_id);
                        show_id
                    };

                    // Insert episode without metadata
                    match db.insert_cloud_episode(&show_title, &file_name, db_show_id, season, episode,
                        &file_id, &folder_id, None, None, None) {
                        Ok(ep_id) => {
                            let display_title = format!("{} S{:02}E{:02}", show_title, season, episode);
                            println!("[CLOUD CHANGES]   ✓ Added (no metadata): {}", display_title);
                            indexed_items.push((ep_id, show_title, file_id, true, Some(season), Some(episode), folder_id));
                            tv_count += 1;
                        }
                        Err(e) => {
                            println!("[CLOUD CHANGES]   ERROR inserting episode: {}", e);
                            continue;
                        }
                    }
                } else {
                    // Insert movie without metadata
                    match db.insert_cloud_movie(&parsed.title, parsed.year, None, None,
                        &file_name, &file_id, &folder_id, None) {
                        Ok(movie_id) => {
                            println!("[CLOUD CHANGES]   ✓ Added (no metadata): {}", parsed.title);
                            indexed_items.push((movie_id, parsed.title, file_id, false, None, None, folder_id));
                            movies_count += 1;
                        }
                        Err(_) => continue,
                    }
                }
            }

            Ok((indexed_items, skipped_count, movies_count, tv_count))
        }).await.map_err(|e| format!("Task failed: {}", e))?
    }?;

    let (indexed_items, skipped_count, movies_count, tv_count) = phase1_result;
    let indexed_count = indexed_items.len();
    let phase1_duration = index_start.elapsed();

    println!("[CLOUD CHANGES] Phase 1 took {:?} - {} file(s) added", phase1_duration, indexed_count);

    // Send notifications and emit events immediately after Phase 1
    if indexed_count > 0 {
        // Collect titles for notifications
        let titles: Vec<String> = indexed_items.iter().map(|(_, title, _, is_tv, season, episode, _)| {
            if *is_tv {
                format!("{} S{:02}E{:02}", title, season.unwrap_or(1), episode.unwrap_or(1))
            } else {
                title.clone()
            }
        }).collect();

        println!("[CLOUD CHANGES] ┌─────────────────────────────────────────");
        println!("[CLOUD CHANGES] │ ADDED TO LIBRARY:");
        for title in &titles {
            println!("[CLOUD CHANGES] │   ✓ {}", title);
        }
        println!("[CLOUD CHANGES] └─────────────────────────────────────────");

        // Emit library-updated so UI refreshes immediately
        window.emit("library-updated", ()).ok();

        // Send Windows notification for each item (simple format)
        for title in &titles {
            let mut notification = Notification::new();
            notification
                .summary("StreamVault")
                .body(&format!("{} added to your library", title))
                .appname("StreamVault")
                .timeout(notify_rust::Timeout::Milliseconds(3000));

            // Set Windows App User Model ID so notification shows from StreamVault
            #[cfg(target_os = "windows")]
            {
                notification.app_id("com.streamvault.app");
            }

            if let Err(e) = notification.show() {
                println!("[CLOUD CHANGES] Failed to send notification: {}", e);
            } else {
                println!("[CLOUD CHANGES] 🔔 Notification: {} added to your library", title);
            }
        }
    }

    // PHASE 2: Fetch metadata in background (don't block)
    if !indexed_items.is_empty() {
        let api_key = {
            let config = state.config.lock().map_err(|e| e.to_string())?;
            tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
        };

        // API key is always available now with default
        if !api_key.is_empty() {
            let db_path_bg = db_path.clone();
            let image_cache_dir_bg = image_cache_dir.clone();
            let window_bg = window.clone();
            let indexed_items_bg = indexed_items.clone();

            println!("[CLOUD CHANGES] ┌─────────────────────────────────────────");
            println!("[CLOUD CHANGES] │ PHASE 2: Fetching metadata in background...");
            println!("[CLOUD CHANGES] └─────────────────────────────────────────");

            // Spawn background task for metadata fetching
            tokio::spawn(async move {
                let metadata_start = std::time::Instant::now();

                let result = tokio::task::spawn_blocking(move || {
                    let db = match database::Database::new(&db_path_bg) {
                        Ok(d) => d,
                        Err(e) => {
                            println!("[CLOUD CHANGES BG] Failed to open database: {}", e);
                            return;
                        }
                    };

                    let mut tv_metadata_cache: std::collections::HashMap<String, Option<tmdb::TmdbMetadata>> = std::collections::HashMap::new();
                    let mut tv_show_updated: std::collections::HashSet<String> = std::collections::HashSet::new();
                    let mut season_cache: std::collections::HashMap<(String, i32), Vec<tmdb::TmdbEpisodeInfo>> = std::collections::HashMap::new();

                    for (media_id, title, _file_id, is_tv, season_opt, episode_opt, _folder_id) in indexed_items_bg {
                        if is_tv {
                            let season = season_opt.unwrap_or(1);
                            let episode = episode_opt.unwrap_or(1);
                            let title_lower = title.to_lowercase();

                            println!("[CLOUD CHANGES BG] Processing {} S{:02}E{:02}...", title, season, episode);

                            // Get or fetch TV show metadata
                            let show_meta = if let Some(cached) = tv_metadata_cache.get(&title_lower) {
                                cached.clone()
                            } else {
                                println!("[CLOUD CHANGES BG]   Searching TMDB for show '{}'...", title);
                                let meta = tmdb::search_metadata(&api_key, &title, "tv", None, &image_cache_dir_bg).ok().flatten();
                                if meta.is_some() {
                                    println!("[CLOUD CHANGES BG]   ✓ Found show metadata");
                                } else {
                                    println!("[CLOUD CHANGES BG]   ✗ Show not found on TMDB");
                                }
                                tv_metadata_cache.insert(title_lower.clone(), meta.clone());
                                meta
                            };

                            if let Some(ref meta) = show_meta {
                                // Update the parent TV show with poster (only once per show)
                                if !tv_show_updated.contains(&title_lower) {
                                    // Find the TV show by title and update it
                                    if let Ok(Some(show)) = db.find_tvshow_by_title(&title) {
                                        if db.update_metadata(show.id, meta).is_ok() {
                                            println!("[CLOUD CHANGES BG]   ✓ Updated TV show poster for '{}'", title);
                                        }
                                    }
                                    tv_show_updated.insert(title_lower.clone());
                                }

                                // Fetch episode metadata
                                if let Some(ref tmdb_id) = meta.tmdb_id {
                                    let cache_key = (tmdb_id.clone(), season);
                                    let episodes = if let Some(cached_eps) = season_cache.get(&cache_key) {
                                        cached_eps.clone()
                                    } else {
                                        println!("[CLOUD CHANGES BG]   Fetching season {} episodes from TMDB...", season);
                                        match tmdb::fetch_season_episodes(&api_key, tmdb_id, season, &title, &image_cache_dir_bg) {
                                            Ok(season_info) => {
                                                println!("[CLOUD CHANGES BG]   ✓ Got {} episodes for season {}", season_info.episodes.len(), season);
                                                let eps = season_info.episodes.clone();
                                                season_cache.insert(cache_key.clone(), eps.clone());
                                                eps
                                            }
                                            Err(e) => {
                                                println!("[CLOUD CHANGES BG]   ✗ Failed to fetch season {}: {}", season, e);
                                                season_cache.insert(cache_key.clone(), Vec::new());
                                                Vec::new()
                                            }
                                        }
                                    };

                                    // Find and update episode metadata
                                    if let Some(ep_info) = episodes.iter().find(|e| e.episode_number == episode) {
                                        if db.update_episode_metadata(
                                            media_id,
                                            Some(&ep_info.name),
                                            ep_info.overview.as_deref(),
                                            ep_info.still_path.as_deref()
                                        ).is_ok() {
                                            println!("[CLOUD CHANGES BG]   ✓ Updated episode metadata: {} S{:02}E{:02}", title, season, episode);
                                        } else {
                                            println!("[CLOUD CHANGES BG]   ✗ Failed to update episode in DB");
                                        }
                                    } else {
                                        println!("[CLOUD CHANGES BG]   ✗ Episode {} not found in TMDB season data (available: {:?})",
                                            episode,
                                            episodes.iter().map(|e| e.episode_number).collect::<Vec<_>>()
                                        );
                                    }
                                }
                            }
                        } else {
                            // Movie metadata
                            println!("[CLOUD CHANGES BG] Processing movie '{}'...", title);
                            match tmdb::search_metadata(&api_key, &title, "movie", None, &image_cache_dir_bg) {
                                Ok(Some(meta)) => {
                                    if db.update_metadata(media_id, &meta).is_ok() {
                                        println!("[CLOUD CHANGES BG]   ✓ Updated movie metadata: {}", meta.title);
                                    } else {
                                        println!("[CLOUD CHANGES BG]   ✗ Failed to update movie in DB");
                                    }
                                }
                                Ok(None) => {
                                    println!("[CLOUD CHANGES BG]   ✗ Movie not found on TMDB");
                                }
                                Err(e) => {
                                    println!("[CLOUD CHANGES BG]   ✗ TMDB search error: {}", e);
                                }
                            }
                        }
                    }
                }).await;

                let metadata_duration = metadata_start.elapsed();
                println!("[CLOUD CHANGES BG] Metadata fetch completed in {:?}", metadata_duration);

                // Emit library-updated again so UI gets the metadata
                window_bg.emit("library-updated", ()).ok();

                if let Err(e) = result {
                    println!("[CLOUD CHANGES BG] Background task error: {}", e);
                }
            });
        } else {
            println!("[CLOUD CHANGES] No TMDB API key - skipping metadata fetch");
        }
    }

    let total_duration = start_time.elapsed();
    let message = if indexed_count > 0 {
        format!("Indexed {} new files ({} movies, {} TV)", indexed_count, movies_count, tv_count)
    } else {
        "No new files to index".to_string()
    };

    println!("[CLOUD CHANGES] ══════════════════════════════════════════");
    println!("[CLOUD CHANGES] SUMMARY: {} indexed, {} skipped", indexed_count, skipped_count);
    println!("[CLOUD CHANGES] Total time: {:?}", total_duration);
    println!("[CLOUD CHANGES] ══════════════════════════════════════════");

    Ok(CloudIndexResult {
        success: true,
        indexed_count,
        skipped_count,
        movies_count,
        tv_count,
        message,
    })
}

// Clear all app data (reset to new state)
#[tauri::command]
async fn clear_all_app_data(
    state: State<'_, AppState>,
) -> Result<ApiResponse, String> {
    println!("[RESET] Starting complete app data reset...");

    // Clear database and get image cache path
    let image_cache_path = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.clear_all_data().map_err(|e| e.to_string())?
    };

    println!("[RESET] Database cleared successfully");

    // Delete image cache directory
    let cache_path = std::path::Path::new(&image_cache_path);
    if cache_path.exists() {
        match std::fs::remove_dir_all(cache_path) {
            Ok(_) => println!("[RESET] Image cache deleted successfully"),
            Err(e) => println!("[RESET] Warning: Failed to delete image cache: {}", e),
        }
        // Recreate empty image cache directory
        std::fs::create_dir_all(cache_path).ok();
    }

    println!("[RESET] App data reset complete!");

    Ok(ApiResponse {
        message: "All app data has been cleared. The app is now like new.".to_string(),
    })
}

// Response for cleanup operation
#[derive(serde::Serialize)]
struct CleanupResponse {
    success: bool,
    removed_count: usize,
    message: String,
}

// Cleanup orphaned metadata - removes entries and posters for missing files
#[tauri::command]
async fn cleanup_missing_metadata(
    state: State<'_, AppState>,
) -> Result<CleanupResponse, String> {
    println!("[CLEANUP] Starting cleanup of missing media metadata...");

    let removed_count = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let image_cache_path = database::get_image_cache_dir();
        media_manager::cleanup_orphaned_media(&db, &image_cache_path)
    };

    let message = if removed_count > 0 {
        format!("Cleaned up {} orphaned entries and their posters", removed_count)
    } else {
        "No orphaned entries found. Your library is clean!".to_string()
    };

    println!("[CLEANUP] {}", message);

    Ok(CleanupResponse {
        success: true,
        removed_count,
        message,
    })
}

// Repair broken file paths - not applicable for cloud-only mode
#[tauri::command]
async fn repair_file_paths(
    _state: State<'_, AppState>,
) -> Result<ApiResponse, String> {
    // In cloud-only mode, file paths are managed by Google Drive
    // No local file repair is needed
    Ok(ApiResponse {
        message: "Cloud media paths are managed automatically by Google Drive. No repair needed.".to_string(),
    })
}

// Response for delete operation
#[derive(serde::Serialize)]
struct DeleteResponse {
    success: bool,
    deleted_count: usize,
    failed_count: usize,
    message: String,
}

// Helper function to clean up empty parent directories after file deletion
fn cleanup_empty_parent_dirs(file_paths: &[String]) {
    use std::collections::HashSet;

    // Collect unique parent directories from deleted files
    let mut parent_dirs: HashSet<std::path::PathBuf> = HashSet::new();
    for file_path in file_paths {
        let path = std::path::Path::new(file_path);
        if let Some(parent) = path.parent() {
            parent_dirs.insert(parent.to_path_buf());
        }
    }

    // Try to remove empty directories (and their parents if also empty)
    for dir in parent_dirs {
        let mut current_dir = Some(dir);
        while let Some(dir_path) = current_dir {
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
                                    println!("[DELETE] Failed to remove directory {:?}: {}", dir_path, e);
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

// Delete media files permanently from disk (bypasses recycle bin)
// Also handles cloud files by deleting from Google Drive
#[tauri::command]
async fn delete_media_files(
    state: State<'_, AppState>,
    media_ids: Vec<i64>,
) -> Result<DeleteResponse, String> {
    if media_ids.is_empty() {
        return Err("No media IDs provided".to_string());
    }

    println!("[DELETE] Starting permanent deletion for {} items", media_ids.len());

    // Get media info including cloud details
    let media_info = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_media_delete_info(&media_ids).map_err(|e| e.to_string())?
    };

    let mut deleted_count = 0;
    let mut failed_count = 0;
    let mut deleted_file_paths: Vec<String> = Vec::new();
    let mut cloud_file_ids_to_delete: Vec<String> = Vec::new();

    // Separate cloud and local files
    for (id, file_path, is_cloud, cloud_file_id) in &media_info {
        if *is_cloud {
            // Cloud file - queue for Google Drive deletion
            if let Some(cloud_id) = cloud_file_id {
                println!("[DELETE] Queuing cloud file for deletion: {} (cloud_file_id: {})",
                    file_path.as_deref().unwrap_or("unknown"), cloud_id);
                cloud_file_ids_to_delete.push(cloud_id.clone());
            }
        } else {
            // Local file - delete from disk
            if let Some(path_str) = file_path {
                let path = std::path::Path::new(path_str);
                if path.exists() {
                    match std::fs::remove_file(path) {
                        Ok(_) => {
                            println!("[DELETE] Successfully deleted local file: {}", path_str);
                            deleted_file_paths.push(path_str.clone());
                            deleted_count += 1;
                        }
                        Err(e) => {
                            println!("[DELETE] Failed to delete {}: {}", path_str, e);
                            failed_count += 1;
                        }
                    }
                } else {
                    println!("[DELETE] Local file not found (already deleted?): {}", path_str);
                    deleted_file_paths.push(path_str.clone());
                    deleted_count += 1;
                }
            }
        }
    }

    // Delete cloud files from Google Drive
    if !cloud_file_ids_to_delete.is_empty() {
        println!("[DELETE] Deleting {} cloud files from Google Drive", cloud_file_ids_to_delete.len());
        for cloud_file_id in cloud_file_ids_to_delete {
            match state.gdrive_client.delete_file(&cloud_file_id).await {
                Ok(_) => {
                    println!("[DELETE] Successfully deleted cloud file: {}", cloud_file_id);
                    deleted_count += 1;
                }
                Err(e) => {
                    println!("[DELETE] Failed to delete cloud file {}: {}", cloud_file_id, e);
                    failed_count += 1;
                }
            }
        }
    }

    // Delete from database
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.delete_media_entries(&media_ids).map_err(|e| e.to_string())?;
    }

    // Clean up empty parent directories (only for local files)
    cleanup_empty_parent_dirs(&deleted_file_paths);

    let message = if failed_count == 0 {
        format!("Successfully deleted {} file(s)", deleted_count)
    } else {
        format!("Deleted {} file(s), {} failed", deleted_count, failed_count)
    };

    println!("[DELETE] Complete: {}", message);

    Ok(DeleteResponse {
        success: failed_count == 0,
        deleted_count,
        failed_count,
        message,
    })
}

// Episode info for delete selection modal
#[derive(serde::Serialize)]
struct EpisodeDeleteInfo {
    id: i64,
    title: String,
    season_number: Option<i32>,
    episode_number: Option<i32>,
    file_path: Option<String>,
}

// Get episodes for a TV show for delete selection
#[tauri::command]
async fn get_episodes_for_delete(
    state: State<'_, AppState>,
    series_id: i64,
) -> Result<Vec<EpisodeDeleteInfo>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let episodes = db.get_episodes(series_id).map_err(|e| e.to_string())?;
    
    let result: Vec<EpisodeDeleteInfo> = episodes.into_iter().map(|ep| {
        EpisodeDeleteInfo {
            id: ep.id,
            title: ep.title,
            season_number: ep.season_number,
            episode_number: ep.episode_number,
            file_path: ep.file_path,
        }
    }).collect();
    
    Ok(result)
}

// Delete a TV show series and optionally all its episodes
#[tauri::command]
async fn delete_series(
    state: State<'_, AppState>,
    series_id: i64,
    delete_files: bool,
) -> Result<DeleteResponse, String> {
    println!("[DELETE] Deleting series ID {} (delete_files: {})", series_id, delete_files);

    // Get series cloud info first
    let (is_cloud_series, cloud_folder_id) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_series_cloud_info(series_id).map_err(|e| e.to_string())?
    };

    println!("[DELETE] Series is_cloud: {}, cloud_folder_id: {:?}", is_cloud_series, cloud_folder_id);

    // Get all episode IDs and their cloud info
    let episode_ids: Vec<i64> = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let episodes = db.get_episodes(series_id).map_err(|e| e.to_string())?;
        episodes.into_iter().map(|ep| ep.id).collect()
    };

    let mut total_deleted = 0;
    let mut total_failed = 0;
    let mut deleted_file_paths: Vec<String> = Vec::new();

    // Delete episodes if there are any
    if !episode_ids.is_empty() {
        // Get detailed info for cloud file deletion
        let episode_info = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.get_media_delete_info(&episode_ids).map_err(|e| e.to_string())?
        };

        if delete_files {
            // Handle cloud episode files
            let mut cloud_file_ids: Vec<String> = Vec::new();
            let mut local_file_paths: Vec<String> = Vec::new();

            for (_id, file_path, is_cloud, cloud_file_id) in &episode_info {
                if *is_cloud {
                    if let Some(cloud_id) = cloud_file_id {
                        cloud_file_ids.push(cloud_id.clone());
                    }
                } else if let Some(path) = file_path {
                    local_file_paths.push(path.clone());
                }
            }

            // Delete cloud files from Google Drive
            if !cloud_file_ids.is_empty() {
                println!("[DELETE] Deleting {} cloud episode files from Google Drive", cloud_file_ids.len());
                for cloud_file_id in cloud_file_ids {
                    match state.gdrive_client.delete_file(&cloud_file_id).await {
                        Ok(_) => {
                            println!("[DELETE] Deleted cloud episode: {}", cloud_file_id);
                            total_deleted += 1;
                        }
                        Err(e) => {
                            println!("[DELETE] Failed to delete cloud episode {}: {}", cloud_file_id, e);
                            total_failed += 1;
                        }
                    }
                }
            }

            // Delete local files
            for file_path in local_file_paths {
                let path = std::path::Path::new(&file_path);
                if path.exists() {
                    match std::fs::remove_file(path) {
                        Ok(_) => {
                            println!("[DELETE] Deleted episode file: {}", file_path);
                            deleted_file_paths.push(file_path);
                            total_deleted += 1;
                        }
                        Err(e) => {
                            println!("[DELETE] Failed to delete episode {}: {}", file_path, e);
                            total_failed += 1;
                        }
                    }
                } else {
                    deleted_file_paths.push(file_path);
                    total_deleted += 1;
                }
            }
        } else {
            total_deleted = episode_ids.len();
        }

        // Delete episode entries from database
        {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.delete_media_entries(&episode_ids).map_err(|e| e.to_string())?;
        }
    }

    // Clean up empty parent directories (only for local files)
    if delete_files && !deleted_file_paths.is_empty() {
        cleanup_empty_parent_dirs(&deleted_file_paths);
    }

    // Delete the series entry itself
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.remove_media(series_id).map_err(|e| e.to_string())?;
    }

    let message = format!("Deleted series and {} episode(s)", total_deleted);
    println!("[DELETE] {}", message);

    Ok(DeleteResponse {
        success: total_failed == 0,
        deleted_count: total_deleted + 1, // +1 for the series itself
        failed_count: total_failed,
        message,
    })
}

// Remove a TV series from the database and optionally delete the cloud folder from Drive
#[tauri::command]
async fn delete_series_cloud_folder(
    state: State<'_, AppState>,
    series_id: i64,
) -> Result<ApiResponse, String> {
    // Get series info including cloud folder ID
    let (series_title, is_cloud, cloud_folder_id) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let media = db.get_media_by_id(series_id).map_err(|e| e.to_string())?;
        let (is_cloud, folder_id) = db.get_series_cloud_info(series_id).map_err(|e| e.to_string())?;
        (media.title, is_cloud, folder_id)
    };

    println!("[DELETE] Removing series '{}' (ID: {}) - is_cloud: {}, folder_id: {:?}",
        series_title, series_id, is_cloud, cloud_folder_id);

    // If it's a cloud series and has a folder ID, delete the folder from Google Drive
    if is_cloud {
        if let Some(folder_id) = cloud_folder_id {
            println!("[DELETE] Deleting cloud folder from Google Drive: {}", folder_id);
            match state.gdrive_client.delete_file(&folder_id).await {
                Ok(_) => {
                    println!("[DELETE] Successfully deleted cloud folder: {}", folder_id);
                }
                Err(e) => {
                    println!("[DELETE] Warning: Failed to delete cloud folder {}: {}", folder_id, e);
                    // Continue anyway - we still want to remove from DB
                }
            }
        }
    }

    // Delete all episodes from database first
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.remove_series_episodes(series_id).map_err(|e| e.to_string())?;
    }

    // Remove the series from database
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.remove_media(series_id).map_err(|e| e.to_string())?;
    }

    Ok(ApiResponse {
        message: format!("Series '{}' completely removed", series_title),
    })
}

// Get configuration
#[tauri::command]
async fn get_config(state: State<'_, AppState>) -> Result<config::Config, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config.clone())
}

// Save configuration
#[tauri::command]
async fn save_config(
    state: State<'_, AppState>,
    new_config: config::Config,
) -> Result<ApiResponse, String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    *config = new_config.clone();
    config::save_config(&new_config).map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: "Configuration saved.".to_string(),
    })
}

// Get scan status
#[tauri::command]
async fn get_scan_status(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.is_scanning.load(Ordering::SeqCst))
}

// Merge duplicate TV shows into single entries
#[tauri::command]
async fn merge_duplicate_shows(state: State<'_, AppState>) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let merged_count = db.merge_duplicate_tvshows().map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: format!("Merged {} duplicate TV shows", merged_count),
    })
}


// Get resume info for a media item
#[tauri::command]
async fn get_resume_info(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<database::ResumeInfo, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_resume_info(media_id).map_err(|e| e.to_string())
}

// Get media info by ID
#[tauri::command]
async fn get_media_info(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<database::MediaItem, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_media_by_id(media_id).map_err(|e| e.to_string())
}

// Get stream info for built-in player
#[derive(Serialize)]
pub struct StreamInfo {
    pub stream_url: String,
    pub file_path: String,
    pub title: String,
    pub poster: Option<String>,
    pub duration_seconds: Option<f64>,
    pub resume_position_seconds: Option<f64>,
    // Cloud streaming fields
    pub is_cloud: bool,
    pub access_token: Option<String>,
}

#[tauri::command]
async fn get_stream_info(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<StreamInfo, String> {
    let media = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_media_by_id(media_id).map_err(|e| e.to_string())?
    };

    let file_path = media.file_path.clone().unwrap_or_default();
    let is_cloud = media.is_cloud.unwrap_or(false);

    // Handle cloud media
    if is_cloud {
        if let Some(ref cloud_file_id) = media.cloud_file_id {
            // Get streaming URL and access token from Google Drive
            let (stream_url, access_token) = state.gdrive_client
                .get_stream_url(cloud_file_id)
                .await?;

            let poster = media.poster_path.as_ref().map(|p| {
                let cache_dir = database::get_image_cache_dir();
                let full_path = std::path::Path::new(&cache_dir).join(p.replace("image_cache/", ""));
                format!("asset://localhost/{}", full_path.to_string_lossy().replace("\\", "/").replace(":", ""))
            });

            return Ok(StreamInfo {
                stream_url,
                file_path,
                title: media.title,
                poster,
                duration_seconds: media.duration_seconds,
                resume_position_seconds: media.resume_position_seconds,
                is_cloud: true,
                access_token: Some(access_token),
            });
        } else {
            return Err("Cloud file ID not found".to_string());
        }
    }

    // Handle local media
    let stream_url = if !file_path.is_empty() && std::path::Path::new(&file_path).exists() {
        file_path.clone()
    } else {
        return Err("File not found".to_string());
    };

    let poster = media.poster_path.as_ref().map(|p| {
        let cache_dir = database::get_image_cache_dir();
        let full_path = std::path::Path::new(&cache_dir).join(p.replace("image_cache/", ""));
        format!("asset://localhost/{}", full_path.to_string_lossy().replace("\\", "/").replace(":", ""))
    });

    Ok(StreamInfo {
        stream_url,
        file_path,
        title: media.title,
        poster,
        duration_seconds: media.duration_seconds,
        resume_position_seconds: media.resume_position_seconds,
        is_cloud: false,
        access_token: None,
    })
}

// Update watch progress
#[tauri::command]
async fn update_progress(
    state: State<'_, AppState>,
    media_id: i64,
    current_time: f64,
    duration: f64,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_progress(media_id, current_time, duration)
        .map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: "Progress updated.".to_string(),
    })
}

// Clear progress for a media item
#[tauri::command]
async fn clear_progress(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.clear_progress(media_id).map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: "Progress cleared.".to_string(),
    })
}

// Fix match - update metadata from TMDB
#[tauri::command]
async fn fix_match(
    state: State<'_, AppState>,
    media_id: i64,
    tmdb_id: String,
    media_type: String,
) -> Result<ApiResponse, String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };

    let api_key = tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default());

    let image_cache_dir = database::get_image_cache_dir();
    let metadata = tmdb::fetch_metadata_by_id(&api_key, &tmdb_id, &media_type, &image_cache_dir)
        .map_err(|e| e.to_string())?;
    
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_metadata(media_id, &metadata).map_err(|e| e.to_string())?;
    
    Ok(ApiResponse {
        message: format!("Metadata updated for: {}", metadata.title),
    })
}

// Play media with MPV (external player) with progress tracking
#[tauri::command]
async fn play_with_mpv(
    window: Window,
    state: State<'_, AppState>,
    media_id: i64,
    resume: bool,
) -> Result<ApiResponse, String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };

    let mpv_path = config.mpv_path.as_ref()
        .ok_or_else(|| "MPV path not set".to_string())?;

    if mpv_path.is_empty() || !std::path::Path::new(mpv_path).exists() {
        return Err("MPV path not set or invalid".to_string());
    }

    let (media, resume_info) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let media = db.get_media_by_id(media_id).map_err(|e| e.to_string())?;
        let resume_info = db.get_resume_info(media_id).map_err(|e| e.to_string())?;

        // Update last_watched
        db.update_last_watched(media_id).map_err(|e| e.to_string())?;

        (media, resume_info)
    };

    let is_cloud = media.is_cloud.unwrap_or(false);
    let title = media.title.clone();
    let season_number = media.season_number;
    let episode_number = media.episode_number;
    let media_type = media.media_type.clone();

    // Get the playback URL and optional auth header
    let (playback_url, auth_header): (String, Option<String>) = if is_cloud {
        // Cloud file - get stream URL from Google Drive
        if let Some(ref cloud_file_id) = media.cloud_file_id {
            println!("[MPV] Cloud file detected, getting stream URL for file ID: {}", cloud_file_id);
            let (stream_url, access_token) = state.gdrive_client
                .get_stream_url(cloud_file_id)
                .await?;
            println!("[MPV] Got cloud stream URL, token length: {}", access_token.len());
            (stream_url, Some(format!("Authorization: Bearer {}", access_token)))
        } else {
            return Err("Cloud file ID not found".to_string());
        }
    } else {
        // Local file - verify it exists
        let file_path = media.file_path.clone().ok_or_else(|| "No file path".to_string())?;

        if !std::path::Path::new(&file_path).exists() {
            return Err(format!(
                "Video file not found: {}. The file may have been moved or deleted. Try rescanning your library.",
                file_path
            ));
        }

        (file_path, None)
    };

    // Determine start position
    let start_position = if resume && resume_info.has_progress {
        resume_info.position
    } else {
        0.0
    };

    // Launch MPV with progress tracking
    let mpv_path_clone = mpv_path.clone();
    let playback_url_clone = playback_url.clone();

    // Launch MPV with tracking (pass auth header and cache settings for cloud files)
    let cache_settings = if is_cloud && config.cloud_cache_enabled {
        config.cloud_cache_dir.as_ref().map(|dir| mpv_ipc::CloudCacheSettings {
            enabled: true,
            cache_dir: dir.clone(),
            max_size_mb: config.cloud_cache_max_mb,
        })
    } else {
        None
    };

    let pid = mpv_ipc::launch_mpv_with_tracking(
        &mpv_path_clone,
        &playback_url_clone,
        media_id,
        start_position,
        auth_header.as_deref(),
        cache_settings.as_ref(),
    )?;
    
    // Store the session
    {
        let mut sessions = state.active_mpv_sessions.lock().map_err(|e| e.to_string())?;
        sessions.insert(media_id, MpvSession {
            media_id,
            pid,
            title: title.clone(),
            start_time: chrono::Utc::now().timestamp(),
        });
    }
    
    // Spawn a background thread to monitor MPV and save progress
    let db_path = database::get_database_path();
    let window_clone = window.clone();
    
    std::thread::spawn(move || {
        println!("[MPV] Starting progress monitor for media ID: {}", media_id);
        
        if let Ok(db) = database::Database::new(&db_path) {
            let result = mpv_ipc::monitor_mpv_and_save_progress(&db, media_id, pid);
            
            // Emit event to frontend when MPV exits
            let _ = window_clone.emit("mpv-playback-ended", serde_json::json!({
                "media_id": media_id,
                "title": title,
                "season_number": season_number,
                "episode_number": episode_number,
                "media_type": media_type,
                "final_position": result.final_position,
                "final_duration": result.final_duration,
                "completed": result.completed,
            }));
            
            println!("[MPV] Playback ended for media ID: {}. Completed: {}", 
                media_id, result.completed);
        }
    });
    
    Ok(ApiResponse {
        message: format!("Playback started: {}", media.title),
    })
}

// Play media with VLC (external player)
#[tauri::command]
async fn play_with_vlc(
    state: State<'_, AppState>,
    media_id: i64,
    resume: bool,
) -> Result<ApiResponse, String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };

    let vlc_path = config.vlc_path.as_ref()
        .ok_or_else(|| "VLC path not set. Please configure it in Settings > Player.".to_string())?;

    if vlc_path.is_empty() || !std::path::Path::new(vlc_path).exists() {
        return Err("VLC path not set or invalid. Please configure it in Settings > Player.".to_string());
    }

    let (media, resume_info) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let media = db.get_media_by_id(media_id).map_err(|e| e.to_string())?;
        let resume_info = db.get_resume_info(media_id).map_err(|e| e.to_string())?;

        // Update last_watched
        db.update_last_watched(media_id).map_err(|e| e.to_string())?;

        (media, resume_info)
    };

    // Determine start position
    let start_position = if resume && resume_info.has_progress {
        resume_info.position
    } else {
        0.0
    };

    let is_cloud = media.is_cloud.unwrap_or(false);
    let title = media.title.clone();

    // Build VLC command
    let mut command = std::process::Command::new(vlc_path);

    if is_cloud {
        // VLC doesn't support authenticated Google Drive streams properly
        // The Google Drive API requires Authorization headers, which VLC can't pass
        return Err("VLC doesn't support authenticated cloud streaming. Please use MPV or the built-in player for cloud files.".to_string());
    } else {
        // Local file
        let file_path = media.file_path.clone().ok_or_else(|| "No file path".to_string())?;

        if !std::path::Path::new(&file_path).exists() {
            return Err(format!("File not found: {}", file_path));
        }

        // Add the file path
        command.arg(&file_path);

        // Add start time if resuming (as input option after the file)
        if start_position > 0.0 {
            command.arg(format!(":start-time={:.0}", start_position));
        }
    }

    // Launch VLC
    println!("[VLC] Launching with args: {:?}", command);
    command.spawn()
        .map_err(|e| format!("Failed to launch VLC: {}", e))?;

    println!("[VLC] Playback started for: {}", title);

    Ok(ApiResponse {
        message: format!("VLC playback started: {}", title),
    })
}

// Check MPV playback status (for polling from frontend if needed)
#[tauri::command]
async fn get_mpv_status(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<serde_json::Value, String> {
    // Check if there's an active session
    let session = {
        let sessions = state.active_mpv_sessions.lock().map_err(|e| e.to_string())?;
        sessions.get(&media_id).cloned()
    };
    
    match session {
        Some(session) => {
            let is_running = mpv_ipc::is_mpv_running(session.pid);
            let progress = mpv_ipc::poll_mpv_progress(media_id);
            
            // If not running, remove from active sessions
            if !is_running {
                let mut sessions = state.active_mpv_sessions.lock().map_err(|e| e.to_string())?;
                sessions.remove(&media_id);
            }
            
            Ok(serde_json::json!({
                "is_playing": is_running,
                "media_id": media_id,
                "title": session.title,
                "position": progress.as_ref().map(|p| p.position),
                "duration": progress.as_ref().map(|p| p.duration),
                "paused": progress.as_ref().map(|p| p.paused).unwrap_or(false),
            }))
        }
        None => {
            Ok(serde_json::json!({
                "is_playing": false,
                "media_id": media_id,
            }))
        }
    }
}

// Get all active MPV sessions
#[tauri::command]
async fn get_active_mpv_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<MpvSession>, String> {
    let mut sessions = state.active_mpv_sessions.lock().map_err(|e| e.to_string())?;
    
    // Filter out dead sessions
    let mut to_remove = Vec::new();
    for (media_id, session) in sessions.iter() {
        if !mpv_ipc::is_mpv_running(session.pid) {
            to_remove.push(*media_id);
        }
    }
    for id in to_remove {
        sessions.remove(&id);
    }
    
    Ok(sessions.values().cloned().collect())
}

// Get image from cache (returns the file path for asset protocol)
#[tauri::command]
async fn get_cached_image(image_name: String) -> Result<String, String> {
    let cache_dir = database::get_image_cache_dir();
    let image_path = std::path::Path::new(&cache_dir).join(&image_name);
    
    println!("[IMAGE] Looking for: {} in {}", image_name, cache_dir);
    println!("[IMAGE] Full path: {:?}", image_path);
    
    if image_path.exists() {
        let asset_url = format!("asset://localhost/{}", image_path.to_string_lossy().replace("\\", "/").replace(":", ""));
        println!("[IMAGE] Found! Asset URL: {}", asset_url);
        Ok(asset_url)
    } else {
        println!("[IMAGE] Not found: {:?}", image_path);
        Err("Image not found".to_string())
    }
}

// Get image path for Tauri's convertFileSrc (returns raw file path)
#[tauri::command]
async fn get_cached_image_path(image_name: String) -> Result<String, String> {
    let cache_dir = database::get_image_cache_dir();
    let image_path = std::path::Path::new(&cache_dir).join(&image_name);
    
    println!("[IMAGE_PATH] Looking for: {} in {}", image_name, cache_dir);
    println!("[IMAGE_PATH] Full path: {:?}", image_path);
    
    if image_path.exists() {
        let path_str = image_path.to_string_lossy().to_string();
        println!("[IMAGE_PATH] Found! Path: {}", path_str);
        Ok(path_str)
    } else {
        println!("[IMAGE_PATH] Not found: {:?}", image_path);
        Err("Image not found".to_string())
    }
}

// Read video file chunk (workaround for asset protocol issues with Windows drive letters)
#[tauri::command]
async fn read_video_chunk(
    file_path: String,
    offset: u64,
    chunk_size: u64,
) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    use std::fs::File;
    
    let mut file = File::open(&file_path)
        .map_err(|e| format!("Failed to open file: {}", e))?;
    
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Failed to seek: {}", e))?;
    
    let mut buffer = vec![0u8; chunk_size as usize];
    let bytes_read = file.read(&mut buffer)
        .map_err(|e| format!("Failed to read: {}", e))?;
    
    buffer.truncate(bytes_read);
    Ok(buffer)
}

#[tauri::command]
async fn get_video_file_size(file_path: String) -> Result<u64, String> {
    let metadata = std::fs::metadata(&file_path)
        .map_err(|e| format!("Failed to get file metadata: {}", e))?;
    Ok(metadata.len())
}

/// Check if the given credential is an access token (starts with "eyJ") or API key
fn is_access_token(credential: &str) -> bool {
    credential.starts_with("eyJ")
}

/// Build TMDB URL with proper authentication
/// - For API keys: adds ?api_key=XXX to URL
/// - For access tokens: returns URL without api_key (auth goes in header)
fn build_tmdb_api_url(path: &str, credential: &str, extra_params: &str) -> String {
    let base = "https://api.themoviedb.org/3";
    if is_access_token(credential) {
        if extra_params.is_empty() {
            format!("{}{}", base, path)
        } else {
            format!("{}{}?{}", base, path, extra_params)
        }
    } else {
        if extra_params.is_empty() {
            format!("{}{}?api_key={}", base, path, credential)
        } else {
            format!("{}{}?api_key={}&{}", base, path, credential, extra_params)
        }
    }
}

// Helper function to perform HTTP GET with retry logic and optional Bearer auth
// Configured to handle Windows connection issues (error 10054 - connection reset)
fn http_get_with_retry_auth(url: &str, credential: &str, max_retries: u32) -> Result<reqwest::blocking::Response, String> {
    let mut last_error = String::new();
    let use_bearer = is_access_token(credential);

    for attempt in 0..max_retries {
        if attempt > 0 {
            // Exponential backoff: 1000ms, 2000ms, 4000ms...
            let delay_ms = 1000 * (1 << attempt);
            std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
            println!("[HTTP] Retry attempt {} after {}ms delay", attempt + 1, delay_ms);
        }

        // Create a fresh client for each attempt to avoid stale connection issues
        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(15))
            .pool_max_idle_per_host(0)
            .tcp_keepalive(std::time::Duration::from_secs(20))
            .http1_only()
            .tcp_nodelay(true)
            .user_agent("StreamVault/1.0")
            .build() {
                Ok(c) => c,
                Err(e) => {
                    last_error = format!("Failed to build HTTP client: {}", e);
                    println!("[HTTP] Client build failed (attempt {}): {}", attempt + 1, last_error);
                    continue;
                }
            };

        let request = if use_bearer {
            client.get(url).header("Authorization", format!("Bearer {}", credential))
        } else {
            client.get(url)
        };

        match request.send() {
            Ok(response) => {
                if response.status().is_success() {
                    return Ok(response);
                } else {
                    last_error = format!("TMDB API error: {}", response.status());
                    // Don't retry on client errors (4xx)
                    if response.status().is_client_error() {
                        return Err(last_error);
                    }
                    println!("[HTTP] Server error (attempt {}): {}", attempt + 1, last_error);
                }
            }
            Err(e) => {
                last_error = format!("Network error: {}", e);
                println!("[HTTP] Request failed (attempt {}): {}", attempt + 1, last_error);
                // Continue to retry on network errors
            }
        }
    }

    Err(format!("Failed after {} retries: {}", max_retries, last_error))
}

// Helper function to perform HTTP GET with retry logic (legacy, no auth header)
// Configured to handle Windows connection issues (error 10054 - connection reset)
fn http_get_with_retry(url: &str, max_retries: u32) -> Result<reqwest::blocking::Response, String> {
    let mut last_error = String::new();
    
    for attempt in 0..max_retries {
        if attempt > 0 {
            // Exponential backoff: 1000ms, 2000ms, 4000ms...
            let delay_ms = 1000 * (1 << attempt);
            std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
            println!("[HTTP] Retry attempt {} after {}ms delay", attempt + 1, delay_ms);
        }
        
        // Create a fresh client for each attempt to avoid stale connection issues
        // This is important on Windows where error 10054 can occur with pooled connections
        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(15))
            // Disable connection pooling to avoid stale connection issues on Windows
            .pool_max_idle_per_host(0)
            // Enable TCP keepalive to detect dead connections faster
            .tcp_keepalive(std::time::Duration::from_secs(20))
            // Force HTTP/1.1 to avoid potential HTTP/2 connection issues
            .http1_only()
            // Set TCP nodelay for faster request/response
            .tcp_nodelay(true)
            // Add a user agent (some APIs block requests without one)
            .user_agent("StreamVault/1.0")
            .build() {
                Ok(c) => c,
                Err(e) => {
                    last_error = format!("Failed to build HTTP client: {}", e);
                    println!("[HTTP] Client build failed (attempt {}): {}", attempt + 1, last_error);
                    continue;
                }
            };
        
        match client.get(url).send() {
            Ok(response) => {
                if response.status().is_success() {
                    return Ok(response);
                } else {
                    last_error = format!("TMDB API error: {}", response.status());
                    // Don't retry on client errors (4xx)
                    if response.status().is_client_error() {
                        return Err(last_error);
                    }
                    println!("[HTTP] Server error (attempt {}): {}", attempt + 1, last_error);
                }
            }
            Err(e) => {
                last_error = format!("Network error: {}", e);
                println!("[HTTP] Request failed (attempt {}): {}", attempt + 1, last_error);
                // Continue to retry on network errors
            }
        }
    }
    
    Err(format!("Failed after {} retries: {}", max_retries, last_error))
}

// TMDB Search result for frontend
#[derive(serde::Serialize)]
struct TmdbSearchResultItem {
    id: i64,
    title: Option<String>,
    name: Option<String>,
    media_type: String,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    overview: Option<String>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    vote_average: Option<f64>,
}

#[derive(serde::Serialize)]
struct TmdbSearchResponse {
    results: Vec<TmdbSearchResultItem>,
    total_results: usize,
}

// TV Show details for episode selection
#[derive(serde::Serialize)]
struct TvSeasonInfo {
    season_number: i32,
    name: String,
    episode_count: i32,
    overview: Option<String>,
    poster_path: Option<String>,
    air_date: Option<String>,
}

#[derive(serde::Serialize)]
struct TvEpisodeInfo {
    episode_number: i32,
    name: String,
    overview: Option<String>,
    still_path: Option<String>,
    air_date: Option<String>,
    runtime: Option<i32>,
    vote_average: Option<f64>,
}

#[derive(serde::Serialize)]
struct TvShowDetails {
    id: i64,
    name: String,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    overview: Option<String>,
    number_of_seasons: i32,
    seasons: Vec<TvSeasonInfo>,
}

#[derive(serde::Serialize)]
struct TvSeasonDetails {
    season_number: i32,
    name: String,
    episodes: Vec<TvEpisodeInfo>,
}

// Get TV show details including seasons
#[tauri::command]
async fn get_tv_details(
    state: State<'_, AppState>,
    tv_id: i64,
) -> Result<TvShowDetails, String> {
    let credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    let url = build_tmdb_api_url(&format!("/tv/{}", tv_id), &credential, "");

    let result = tokio::task::spawn_blocking(move || -> Result<TvShowDetails, String> {
        let response = http_get_with_retry_auth(&url, &credential, 3)?;
        
        #[derive(serde::Deserialize)]
        struct RawSeason {
            season_number: i32,
            name: Option<String>,
            episode_count: i32,
            overview: Option<String>,
            poster_path: Option<String>,
            air_date: Option<String>,
        }
        
        #[derive(serde::Deserialize)]
        struct RawTvShow {
            id: i64,
            name: Option<String>,
            poster_path: Option<String>,
            backdrop_path: Option<String>,
            overview: Option<String>,
            number_of_seasons: Option<i32>,
            seasons: Option<Vec<RawSeason>>,
        }
        
        let raw: RawTvShow = response.json().map_err(|e| e.to_string())?;
        
        let seasons: Vec<TvSeasonInfo> = raw.seasons.unwrap_or_default()
            .into_iter()
            .filter(|s| s.season_number > 0) // Filter out specials (season 0)
            .map(|s| TvSeasonInfo {
                season_number: s.season_number,
                name: s.name.unwrap_or_else(|| format!("Season {}", s.season_number)),
                episode_count: s.episode_count,
                overview: s.overview,
                poster_path: s.poster_path,
                air_date: s.air_date,
            })
            .collect();
        
        Ok(TvShowDetails {
            id: raw.id,
            name: raw.name.unwrap_or_else(|| "Unknown".to_string()),
            poster_path: raw.poster_path,
            backdrop_path: raw.backdrop_path,
            overview: raw.overview,
            number_of_seasons: raw.number_of_seasons.unwrap_or(0),
            seasons,
        })
    }).await.map_err(|e| e.to_string())??;
    
    Ok(result)
}

// Get episodes for a specific season of a TV show
#[tauri::command]
async fn get_tv_season_episodes(
    state: State<'_, AppState>,
    tv_id: i64,
    season_number: i32,
) -> Result<TvSeasonDetails, String> {
    // First, try to get from local cache
    let tv_id_str = tv_id.to_string();
    let image_cache_dir = database::get_image_cache_dir();
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        if let Ok(cached_episodes) = db.get_cached_episodes_for_season(&tv_id_str, season_number) {
            if !cached_episodes.is_empty() {
                println!("[CACHE] Using cached episode data for TV {} Season {}", tv_id, season_number);
                let episodes: Vec<TvEpisodeInfo> = cached_episodes
                    .into_iter()
                    .map(|e| {
                        // Verify still_path file actually exists
                        let verified_still_path = e.still_path.and_then(|path| {
                            let full_path = std::path::Path::new(&image_cache_dir).join(&path);
                            if full_path.exists() {
                                Some(path)
                            } else {
                                None // File doesn't exist, return None
                            }
                        });

                        TvEpisodeInfo {
                            episode_number: e.episode_number,
                            name: e.episode_title.unwrap_or_else(|| format!("Episode {}", e.episode_number)),
                            overview: e.overview,
                            still_path: verified_still_path,
                            air_date: e.air_date,
                            runtime: None,
                            vote_average: None,
                        }
                    })
                    .collect();

                return Ok(TvSeasonDetails {
                    season_number,
                    name: format!("Season {}", season_number),
                    episodes,
                });
            }
        }
    }

    // Cache miss - fetch from TMDB API
    println!("[TMDB] Cache miss, fetching from API for TV {} Season {}", tv_id, season_number);

    let credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    let url = build_tmdb_api_url(&format!("/tv/{}/season/{}", tv_id, season_number), &credential, "");

    let result = tokio::task::spawn_blocking(move || -> Result<TvSeasonDetails, String> {
        let response = http_get_with_retry_auth(&url, &credential, 3)?;
        
        #[derive(serde::Deserialize)]
        struct RawEpisode {
            episode_number: i32,
            name: Option<String>,
            overview: Option<String>,
            still_path: Option<String>,
            air_date: Option<String>,
            runtime: Option<i32>,
            vote_average: Option<f64>,
        }
        
        #[derive(serde::Deserialize)]
        struct RawSeasonDetails {
            season_number: i32,
            name: Option<String>,
            episodes: Option<Vec<RawEpisode>>,
        }
        
        let raw: RawSeasonDetails = response.json().map_err(|e| e.to_string())?;
        
        let episodes: Vec<TvEpisodeInfo> = raw.episodes.unwrap_or_default()
            .into_iter()
            .map(|e| TvEpisodeInfo {
                episode_number: e.episode_number,
                name: e.name.unwrap_or_else(|| format!("Episode {}", e.episode_number)),
                overview: e.overview,
                still_path: e.still_path,
                air_date: e.air_date,
                runtime: e.runtime,
                vote_average: e.vote_average,
            })
            .collect();
        
        Ok(TvSeasonDetails {
            season_number: raw.season_number,
            name: raw.name.unwrap_or_else(|| format!("Season {}", raw.season_number)),
            episodes,
        })
    }).await.map_err(|e| e.to_string())??;

    Ok(result)
}

// Force refresh episode metadata for a TV series (re-downloads images ONLY for owned episodes)
#[tauri::command]
async fn refresh_series_metadata(
    state: State<'_, AppState>,
    tv_id: i64,
    series_title: String,
) -> Result<String, String> {
    let credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    let image_cache_dir = database::get_image_cache_dir();
    let tv_id_str = tv_id.to_string();
    let series_title_clone = series_title.clone();

    println!("[REFRESH] Starting metadata refresh for {} (TMDB ID: {})", series_title, tv_id);
    println!("[REFRESH] Image cache directory: {}", image_cache_dir);

    // Step 1: Find the series ID in our database by TMDB ID
    let (series_db_id, owned_episodes): (Option<i64>, Vec<(i64, i32, i32)>) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let series_id = db.find_series_id_by_tmdb(&tv_id_str).map_err(|e| e.to_string())?;

        if let Some(sid) = series_id {
            let episodes = db.get_owned_episodes_for_series(sid).map_err(|e| e.to_string())?;
            println!("[REFRESH] Found series DB ID: {}, owned episodes: {}", sid, episodes.len());
            (Some(sid), episodes)
        } else {
            println!("[REFRESH] Warning: Series not found in database by TMDB ID {}", tv_id);
            (None, Vec::new())
        }
    };

    if owned_episodes.is_empty() {
        return Err("No episodes found for this series in your library".to_string());
    }

    // Convert to (season, episode) tuples for the TMDB function
    let episode_list: Vec<(i32, i32)> = owned_episodes.iter()
        .map(|(_, season, episode)| (*season, *episode))
        .collect();

    println!("[REFRESH] Will only fetch metadata for {} owned episodes: {:?}",
        episode_list.len(), episode_list);

    // Clear old cached metadata for just the episodes we own
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        // Only clear metadata for this series
        if let Ok(deleted) = db.clear_cached_metadata_for_series(&tv_id_str) {
            println!("[REFRESH] Cleared {} old cached entries for series {}", deleted, tv_id);
        }
    }

    // Step 2: Fetch ONLY the episodes the user owns
    let fetched_episodes = tokio::task::spawn_blocking(move || {
        tmdb::fetch_owned_episodes_only(&credential, &tv_id_str, &series_title_clone, &image_cache_dir, &episode_list)
    }).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;

    let mut total_images = 0;

    // Step 3: Save to cached_episode_metadata table AND update the media table directly
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        for ep in &fetched_episodes {
            if ep.still_path.is_some() {
                total_images += 1;
            }

            // Save to cache table
            if let Err(e) = db.save_cached_episode_metadata(
                &tv_id.to_string(),
                ep.season_number,
                ep.episode_number,
                Some(&ep.name),
                ep.overview.as_deref(),
                ep.still_path.as_deref(),
                ep.air_date.as_deref(),
            ) {
                println!("[REFRESH] Warning: Failed to save cached metadata S{:02}E{:02}: {}",
                    ep.season_number, ep.episode_number, e);
            }

            // Also update the media table directly so episodes show the images immediately
            // Find the episode ID from our owned_episodes list
            if let Some((episode_db_id, _, _)) = owned_episodes.iter()
                .find(|(_, s, e)| *s == ep.season_number && *e == ep.episode_number)
            {
                if let Err(e) = db.update_episode_metadata(
                    *episode_db_id,
                    Some(&ep.name),
                    ep.overview.as_deref(),
                    ep.still_path.as_deref(),
                ) {
                    println!("[REFRESH] Warning: Failed to update media S{:02}E{:02}: {}",
                        ep.season_number, ep.episode_number, e);
                } else {
                    println!("[REFRESH] Updated media entry for S{:02}E{:02}",
                        ep.season_number, ep.episode_number);
                }
            }
        }
    }

    let result = format!("Refreshed {} episodes, {} images downloaded", fetched_episodes.len(), total_images);
    println!("[REFRESH] Completed: {}", result);
    Ok(result)
}

// Search TMDB for streaming - returns raw search results
#[tauri::command]
async fn search_tmdb(
    state: State<'_, AppState>,
    query: String,
) -> Result<TmdbSearchResponse, String> {
    println!("[SEARCH_TMDB] Starting search for: {}", query);

    let credential = {
        let config = state.config.lock().map_err(|e| {
            println!("[SEARCH_TMDB] Failed to lock config: {}", e);
            e.to_string()
        })?;
        let key = tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default());
        println!("[SEARCH_TMDB] Credential length: {} (is_token: {})", key.len(), is_access_token(&key));
        key
    };

    let encoded_query = percent_encoding::utf8_percent_encode(&query, percent_encoding::NON_ALPHANUMERIC).to_string();
    let url = build_tmdb_api_url("/search/multi", &credential, &format!("query={}&include_adult=false", encoded_query));

    println!("[SEARCH_TMDB] URL built, making request...");

    // Run blocking HTTP request with retry in a separate thread
    let result = tokio::task::spawn_blocking(move || -> Result<TmdbSearchResponse, String> {
        let response = http_get_with_retry_auth(&url, &credential, 3)?;
        
        #[derive(serde::Deserialize)]
        struct RawSearchResult {
            results: Vec<RawSearchItem>,
        }
        
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
        
        let raw: RawSearchResult = response.json().map_err(|e| e.to_string())?;
        
        let results: Vec<TmdbSearchResultItem> = raw.results.into_iter()
            .filter(|item| {
                let mt = item.media_type.as_deref().unwrap_or("");
                mt == "movie" || mt == "tv"
            })
            .map(|item| TmdbSearchResultItem {
                id: item.id,
                title: item.title.or(item.original_title),
                name: item.name.or(item.original_name),
                media_type: item.media_type.unwrap_or_default(),
                poster_path: item.poster_path,
                backdrop_path: item.backdrop_path,
                overview: item.overview,
                release_date: item.release_date,
                first_air_date: item.first_air_date,
                vote_average: item.vote_average,
            })
            .collect();
        
        Ok(TmdbSearchResponse {
            total_results: results.len(),
            results,
        })
    }).await.map_err(|e| e.to_string())??;
    
    Ok(result)
}

// Videasy localStorage progress format
#[derive(serde::Deserialize, serde::Serialize, Debug)]
struct VideasyProgress {
    duration: f64,
    watched: f64,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct VideasyStorageItem {
    poster: Option<String>,
    background: Option<String>,
    id: i64,
    media_type: String,
    title: String,
    progress: Option<VideasyProgress>,
}

// Open Videasy in the user's default browser
#[tauri::command]
async fn open_videasy_player(
    app_handle: tauri::AppHandle,
    _state: State<'_, AppState>,
    url: String,
    tmdb_id: String,
    media_type: String,
    title: String,
    _poster_path: Option<String>,
    season: Option<i32>,
    episode: Option<i32>,
) -> Result<ApiResponse, String> {
    println!("[VIDEASY] Opening in browser for: {} (tmdb_id: {})", title, tmdb_id);

    // Open the URL directly in the user's default browser using Tauri's shell API
    tauri::api::shell::open(&app_handle.shell_scope(), &url, None)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    let display_title = if media_type == "tv" {
        format!("{} - S{}E{}", title, season.unwrap_or(1), episode.unwrap_or(1))
    } else {
        title.clone()
    };

    Ok(ApiResponse {
        message: format!("Opening \"{}\" in browser", display_title),
    })
}

// Save progress from Videasy player (called from JavaScript)
#[tauri::command]
async fn save_videasy_progress(
    state: State<'_, AppState>,
    tmdb_id: String,
    media_type: String,
    title: String,
    poster_path: Option<String>,
    season: Option<i32>,
    episode: Option<i32>,
    position: f64,
    duration: f64,
) -> Result<ApiResponse, String> {
    println!("[VIDEASY] Saving progress: {} - {:.1}s / {:.1}s", title, position, duration);

    let db = state.db.lock().map_err(|e| e.to_string())?;

    let poster_url = poster_path.map(|p| {
        if p.starts_with("http") {
            p
        } else {
            format!("https://image.tmdb.org/t/p/w342{}", p)
        }
    });

    db.save_streaming_progress(
        &tmdb_id,
        &media_type,
        &title,
        poster_url.as_deref(),
        season,
        episode,
        position,
        duration,
    ).map_err(|e| e.to_string())?;

    Ok(ApiResponse {
        message: "Progress saved".to_string(),
    })
}

// ==================== TRANSCODING COMMANDS ====================

/// Transcode response with stream URL
#[derive(serde::Serialize)]
struct TranscodeResponse {
    session_id: u64,
    stream_url: String,
}

/// Check if a file needs transcoding for HTML5 playback
#[tauri::command]
async fn check_needs_transcode(file_path: String) -> Result<bool, String> {
    Ok(transcoder::needs_transcoding(&file_path))
}

/// Start transcoding a video file
#[tauri::command]
async fn start_transcode_stream(
    state: State<'_, AppState>,
    file_path: String,
    start_time: Option<f64>,
) -> Result<TranscodeResponse, String> {
    let ffmpeg_path = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        config.ffmpeg_path.clone()
            .ok_or_else(|| "FFmpeg path not configured. Please set it in Settings > Player.".to_string())?
    };

    if ffmpeg_path.is_empty() || !std::path::Path::new(&ffmpeg_path).exists() {
        return Err("FFmpeg path not set or invalid. Please configure it in Settings > Player.".to_string());
    }

    let (session_id, stream_url) = transcoder::start_transcode(&ffmpeg_path, &file_path, start_time)?;

    Ok(TranscodeResponse {
        session_id,
        stream_url,
    })
}

/// Stop a transcoding session
#[tauri::command]
async fn stop_transcode_stream(session_id: u64) -> Result<ApiResponse, String> {
    transcoder::stop_transcode(session_id)?;
    Ok(ApiResponse {
        message: "Transcoding stopped".to_string(),
    })
}

/// Get stream info with transcoding support
#[tauri::command]
async fn get_stream_info_with_transcode(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<StreamInfo, String> {
    let media = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_media_by_id(media_id).map_err(|e| e.to_string())?
    };

    let file_path = media.file_path.clone().unwrap_or_default();
    let is_cloud = media.is_cloud.unwrap_or(false);

    // Handle cloud media - same as get_stream_info
    if is_cloud {
        if let Some(ref cloud_file_id) = media.cloud_file_id {
            let (stream_url, access_token) = state.gdrive_client
                .get_stream_url(cloud_file_id)
                .await?;

            let poster = media.poster_path.as_ref().map(|p| {
                let cache_dir = database::get_image_cache_dir();
                let full_path = std::path::Path::new(&cache_dir).join(p.replace("image_cache/", ""));
                format!("asset://localhost/{}", full_path.to_string_lossy().replace("\\", "/").replace(":", ""))
            });

            return Ok(StreamInfo {
                stream_url,
                file_path,
                title: media.title,
                poster,
                duration_seconds: media.duration_seconds,
                resume_position_seconds: media.resume_position_seconds,
                is_cloud: true,
                access_token: Some(access_token),
            });
        } else {
            return Err("Cloud file ID not found".to_string());
        }
    }

    // Check if local file needs transcoding
    let needs_transcode = transcoder::needs_transcoding(&file_path);

    if needs_transcode {
        // Check if FFmpeg is configured
        let ffmpeg_path = {
            let config = state.config.lock().map_err(|e| e.to_string())?;
            config.ffmpeg_path.clone()
        };

        if let Some(ref path) = ffmpeg_path {
            if !path.is_empty() && std::path::Path::new(path).exists() {
                // Start transcoding
                let start_time = media.resume_position_seconds;
                let (_, stream_url) = transcoder::start_transcode(path, &file_path, start_time)?;

                let poster = media.poster_path.as_ref().map(|p| {
                    let cache_dir = database::get_image_cache_dir();
                    let full_path = std::path::Path::new(&cache_dir).join(p.replace("image_cache/", ""));
                    format!("asset://localhost/{}", full_path.to_string_lossy().replace("\\", "/").replace(":", ""))
                });

                return Ok(StreamInfo {
                    stream_url,
                    file_path,
                    title: media.title,
                    poster,
                    duration_seconds: media.duration_seconds,
                    resume_position_seconds: Some(0.0), // Already seeked in transcode
                    is_cloud: false,
                    access_token: None,
                });
            }
        }

        // FFmpeg not configured, return error with helpful message
        return Err(format!(
            "This video format requires transcoding. Please configure FFmpeg in Settings > Player, or use MPV/VLC player instead."
        ));
    }

    // No transcoding needed - return local file path
    if !file_path.is_empty() && std::path::Path::new(&file_path).exists() {
        let poster = media.poster_path.as_ref().map(|p| {
            let cache_dir = database::get_image_cache_dir();
            let full_path = std::path::Path::new(&cache_dir).join(p.replace("image_cache/", ""));
            format!("asset://localhost/{}", full_path.to_string_lossy().replace("\\", "/").replace(":", ""))
        });

        return Ok(StreamInfo {
            stream_url: file_path.clone(),
            file_path,
            title: media.title,
            poster,
            duration_seconds: media.duration_seconds,
            resume_position_seconds: media.resume_position_seconds,
            is_cloud: false,
            access_token: None,
        });
    }

    Err("File not found".to_string())
}

// ==================== CLOUD CACHE MANAGEMENT ====================

/// Cache info response
#[derive(serde::Serialize)]
struct CloudCacheInfo {
    enabled: bool,
    cache_dir: Option<String>,
    total_size_bytes: u64,
    total_size_mb: f64,
    file_count: usize,
    max_size_mb: u32,
    expiry_hours: u32,
}

/// Get cloud cache info and statistics
#[tauri::command]
async fn get_cloud_cache_info(state: State<'_, AppState>) -> Result<CloudCacheInfo, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;

    if !config.cloud_cache_enabled || config.cloud_cache_dir.is_none() {
        return Ok(CloudCacheInfo {
            enabled: false,
            cache_dir: None,
            total_size_bytes: 0,
            total_size_mb: 0.0,
            file_count: 0,
            max_size_mb: config.cloud_cache_max_mb,
            expiry_hours: config.cloud_cache_expiry_hours,
        });
    }

    let cache_dir = config.cloud_cache_dir.clone().unwrap();
    let (total_size, file_count) = calculate_cache_size(&cache_dir);

    Ok(CloudCacheInfo {
        enabled: true,
        cache_dir: Some(cache_dir),
        total_size_bytes: total_size,
        total_size_mb: total_size as f64 / (1024.0 * 1024.0),
        file_count,
        max_size_mb: config.cloud_cache_max_mb,
        expiry_hours: config.cloud_cache_expiry_hours,
    })
}

/// Calculate total size and file count of cache directory
fn calculate_cache_size(cache_dir: &str) -> (u64, usize) {
    let path = std::path::Path::new(cache_dir);
    if !path.exists() {
        return (0, 0);
    }

    let mut total_size: u64 = 0;
    let mut file_count: usize = 0;

    for entry in walkdir::WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(metadata) = entry.metadata() {
                total_size += metadata.len();
                file_count += 1;
            }
        }
    }

    (total_size, file_count)
}

/// Clean up expired cache files (older than expiry_hours)
#[tauri::command]
async fn cleanup_cloud_cache(state: State<'_, AppState>) -> Result<ApiResponse, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;

    if !config.cloud_cache_enabled || config.cloud_cache_dir.is_none() {
        return Ok(ApiResponse {
            message: "Cloud cache is not enabled".to_string(),
        });
    }

    let cache_dir = config.cloud_cache_dir.clone().unwrap();
    let expiry_hours = config.cloud_cache_expiry_hours;

    let (deleted_count, freed_bytes) = cleanup_expired_cache(&cache_dir, expiry_hours);
    let freed_mb = freed_bytes as f64 / (1024.0 * 1024.0);

    Ok(ApiResponse {
        message: format!("Cleaned up {} files, freed {:.1} MB", deleted_count, freed_mb),
    })
}

/// Clean up cache files older than expiry_hours
fn cleanup_expired_cache(cache_dir: &str, expiry_hours: u32) -> (usize, u64) {
    let path = std::path::Path::new(cache_dir);
    if !path.exists() {
        return (0, 0);
    }

    let expiry_duration = std::time::Duration::from_secs((expiry_hours as u64) * 3600);
    let now = std::time::SystemTime::now();

    let mut deleted_count = 0;
    let mut freed_bytes: u64 = 0;

    // Collect directories to potentially remove (media_X folders)
    let mut empty_dirs: Vec<std::path::PathBuf> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let entry_path = entry.path();

            if entry_path.is_dir() {
                // Check each file in the media cache subdirectory
                let mut dir_has_files = false;

                if let Ok(files) = std::fs::read_dir(&entry_path) {
                    for file in files.filter_map(|f| f.ok()) {
                        let file_path = file.path();
                        if file_path.is_file() {
                            if let Ok(metadata) = file.metadata() {
                                if let Ok(modified) = metadata.modified() {
                                    if let Ok(age) = now.duration_since(modified) {
                                        if age > expiry_duration {
                                            let size = metadata.len();
                                            if std::fs::remove_file(&file_path).is_ok() {
                                                deleted_count += 1;
                                                freed_bytes += size;
                                                println!("[CACHE] Deleted expired: {:?}", file_path);
                                            }
                                        } else {
                                            dir_has_files = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Mark empty directories for removal
                if !dir_has_files {
                    empty_dirs.push(entry_path);
                }
            }
        }
    }

    // Remove empty directories
    for dir in empty_dirs {
        if std::fs::remove_dir(&dir).is_ok() {
            println!("[CACHE] Removed empty directory: {:?}", dir);
        }
    }

    println!("[CACHE] Cleanup complete: {} files deleted, {} bytes freed", deleted_count, freed_bytes);
    (deleted_count, freed_bytes)
}

/// Clear all cloud cache
#[tauri::command]
async fn clear_cloud_cache(state: State<'_, AppState>) -> Result<ApiResponse, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;

    if config.cloud_cache_dir.is_none() {
        return Ok(ApiResponse {
            message: "No cache directory configured".to_string(),
        });
    }

    let cache_dir = config.cloud_cache_dir.clone().unwrap();
    let path = std::path::Path::new(&cache_dir);

    if !path.exists() {
        return Ok(ApiResponse {
            message: "Cache directory does not exist".to_string(),
        });
    }

    let (total_size, file_count) = calculate_cache_size(&cache_dir);

    // Remove all contents
    if let Err(e) = std::fs::remove_dir_all(path) {
        return Err(format!("Failed to clear cache: {}", e));
    }

    // Recreate empty directory
    std::fs::create_dir_all(path).ok();

    let freed_mb = total_size as f64 / (1024.0 * 1024.0);
    Ok(ApiResponse {
        message: format!("Cleared {} files, freed {:.1} MB", file_count, freed_mb),
    })
}

/// Helper function to create the main window
/// Used when showing the app from tray - creates a new window if none exists
fn create_main_window(app: &AppHandle) -> Result<tauri::Window, tauri::Error> {
    WindowBuilder::new(
        app,
        "main",
        WindowUrl::App("index.html".into())
    )
    .title("StreamVault")
    .inner_size(1200.0, 800.0)
    .resizable(true)
    .build()
}

/// Background cloud change detection polling
/// Runs independently of the window to detect new files even when minimized to tray
async fn background_cloud_poll(app_handle: AppHandle) {
    use std::time::Duration;

    // Initial delay to let app fully initialize (same as frontend)
    tokio::time::sleep(Duration::from_secs(3)).await;

    println!("[CLOUD BG] Background cloud polling started (5-second interval)");

    loop {
        // Get state from app handle
        let state: tauri::State<'_, AppState> = app_handle.state();

        // Check if authenticated
        if !state.gdrive_client.is_authenticated() {
            // Not connected - wait and retry (silent, don't spam logs)
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        // Cloud-only mode: No folder check needed - we monitor entire Drive
        println!("[CLOUD BG] Polling for changes...");

        // Perform the actual check
        match background_check_cloud_changes(&app_handle).await {
            Ok(result) => {
                if result.indexed_count > 0 {
                    println!("[CLOUD BG] ✓ Indexed {} new items ({} movies, {} TV)",
                        result.indexed_count, result.movies_count, result.tv_count);

                    // Emit event to window if it exists
                    if let Some(window) = app_handle.get_window("main") {
                        window.emit("library-updated", ()).ok();
                    }
                } else {
                    println!("[CLOUD BG] No new files detected");
                }
            }
            Err(e) => {
                println!("[CLOUD BG] Poll error: {}", e);
            }
        }

        // Wait 5 seconds before next poll (same as frontend)
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Background version of check_cloud_changes that doesn't require a Window parameter
async fn background_check_cloud_changes(app_handle: &AppHandle) -> Result<CloudIndexResult, String> {
    let state: tauri::State<'_, AppState> = app_handle.state();
    let start_time = std::time::Instant::now();

    println!("[CLOUD BG] ══════════════════════════════════════════");
    println!("[CLOUD BG] Starting change detection poll...");

    // Check if authenticated
    if !state.gdrive_client.is_authenticated() {
        println!("[CLOUD BG] Not authenticated - skipping");
        println!("[CLOUD BG] ══════════════════════════════════════════");
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: 0,
            movies_count: 0,
            tv_count: 0,
            message: "Not connected to Google Drive".to_string(),
        });
    }

    // Get or initialize the changes token
    let current_token = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_gdrive_changes_token().map_err(|e| e.to_string())?
    };

    let page_token = match current_token {
        Some(token) => {
            println!("[CLOUD BG] Using existing token: {}...", &token[..token.len().min(20)]);
            token
        },
        None => {
            // First time - get the start token
            println!("[CLOUD BG] No token found - initializing changes tracking...");
            let start_token = state.gdrive_client.get_changes_start_token().await?;
            println!("[CLOUD BG] Got start token: {}...", &start_token[..start_token.len().min(20)]);
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.set_gdrive_changes_token(&start_token).map_err(|e| e.to_string())?;
            println!("[CLOUD BG] Token saved - will detect changes on next poll");
            println!("[CLOUD BG] ══════════════════════════════════════════");
            return Ok(CloudIndexResult {
                success: true,
                indexed_count: 0,
                skipped_count: 0,
                movies_count: 0,
                tv_count: 0,
                message: "Changes tracking initialized".to_string(),
            });
        }
    };

    // Note: We no longer filter by tracked folders - index all video files in Drive
    println!("[CLOUD BG] Monitoring entire Google Drive for changes");

    // Get changes since last check
    let api_start = std::time::Instant::now();
    let (changed_files, new_token) = state.gdrive_client.get_video_changes(&page_token).await?;
    let api_duration = api_start.elapsed();
    println!("[CLOUD BG] Changes API call took {:?}", api_duration);

    // Save the new token immediately
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.set_gdrive_changes_token(&new_token).map_err(|e| e.to_string())?;
    }

    if changed_files.is_empty() {
        let total_duration = start_time.elapsed();
        println!("[CLOUD BG] No changes detected (total: {:?})", total_duration);
        println!("[CLOUD BG] ══════════════════════════════════════════");
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: 0,
            movies_count: 0,
            tv_count: 0,
            message: "No new files detected".to_string(),
        });
    }

    println!("[CLOUD BG] Detected {} changed video file(s)", changed_files.len());

    // Index all detected video files (no folder filtering)
    let files_to_index = changed_files;

    if files_to_index.is_empty() {
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: 0,
            movies_count: 0,
            tv_count: 0,
            message: "No new files detected".to_string(),
        });
    }

    // Get API key from config
    let api_key = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    let image_cache_dir = database::get_image_cache_dir();
    std::fs::create_dir_all(&image_cache_dir).ok();
    let db_path = database::get_database_path();

    // PHASE 1: Add files immediately without metadata
    let phase1_result = {
        let db_path_clone = db_path.clone();
        let files_to_index_clone: Vec<_> = files_to_index.iter().map(|f| {
            (f.id.clone(), f.name.clone(), f.parents.clone())
        }).collect();

        tokio::task::spawn_blocking(move || {
            let db = match database::Database::new(&db_path_clone) {
                Ok(d) => d,
                Err(e) => return Err(format!("Failed to open database: {}", e)),
            };

            let mut indexed_items: Vec<(i64, String, String, bool, Option<i32>, Option<i32>, String)> = Vec::new();
            let mut skipped_count = 0;
            let mut movies_count = 0;
            let mut tv_count = 0;
            let mut tv_show_cache: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

            for (file_id, file_name, parents) in files_to_index_clone {
                if db.cloud_file_exists(&file_id) {
                    skipped_count += 1;
                    continue;
                }

                if let Ok(Some(_)) = db.get_media_by_file_path(&file_name) {
                    skipped_count += 1;
                    continue;
                }

                let folder_id = parents.as_ref()
                    .and_then(|p| p.first())
                    .cloned()
                    .unwrap_or_default();

                let parsed = media_manager::parse_cloud_filename(&file_name);
                let is_tv_show = parsed.season.is_some() && parsed.episode.is_some();

                if is_tv_show {
                    let season = parsed.season.unwrap();
                    let episode = parsed.episode.unwrap();
                    let show_title = parsed.title.clone();
                    let show_title_lower = show_title.to_lowercase();

                    let db_show_id = if let Some(&cached_id) = tv_show_cache.get(&show_title_lower) {
                        cached_id
                    } else {
                        let existing = db.find_tvshow_by_title(&show_title);
                        let show_id = match existing {
                            Ok(Some(existing_show)) => existing_show.id,
                            Ok(None) => {
                                let show_path = format!("gdrive:{}:{}", folder_id, show_title.to_lowercase().replace(" ", "_"));
                                match db.insert_cloud_tvshow(&show_title, None, None, None, &show_path, &folder_id, None) {
                                    Ok(id) => id,
                                    Err(_) => continue,
                                }
                            }
                            Err(_) => continue,
                        };
                        tv_show_cache.insert(show_title_lower, show_id);
                        show_id
                    };

                    match db.insert_cloud_episode(&show_title, &file_name, db_show_id, season, episode,
                        &file_id, &folder_id, None, None, None) {
                        Ok(ep_id) => {
                            indexed_items.push((ep_id, show_title, file_id, true, Some(season), Some(episode), folder_id));
                            tv_count += 1;
                        }
                        Err(_) => continue,
                    }
                } else {
                    match db.insert_cloud_movie(&parsed.title, parsed.year, None, None,
                        &file_name, &file_id, &folder_id, None) {
                        Ok(movie_id) => {
                            indexed_items.push((movie_id, parsed.title, file_id, false, None, None, folder_id));
                            movies_count += 1;
                        }
                        Err(_) => continue,
                    }
                }
            }

            Ok((indexed_items, skipped_count, movies_count, tv_count))
        }).await.map_err(|e| format!("Task failed: {}", e))?
    }?;

    let (indexed_items, skipped_count, movies_count, tv_count) = phase1_result;
    let indexed_count = indexed_items.len();

    // Send notifications for new items
    if indexed_count > 0 {
        let titles: Vec<String> = indexed_items.iter().map(|(_, title, _, is_tv, season, episode, _)| {
            if *is_tv {
                format!("{} S{:02}E{:02}", title, season.unwrap_or(1), episode.unwrap_or(1))
            } else {
                title.clone()
            }
        }).collect();

        for title in &titles {
            let mut notification = Notification::new();
            notification
                .summary("StreamVault")
                .body(&format!("{} added to your library", title))
                .appname("StreamVault")
                .timeout(notify_rust::Timeout::Milliseconds(3000));

            // Set Windows App User Model ID so notification shows from StreamVault
            #[cfg(target_os = "windows")]
            {
                notification.app_id("com.streamvault.app");
            }

            notification.show().ok();
        }

        // Emit library-updated if window exists
        if let Some(window) = app_handle.get_window("main") {
            window.emit("library-updated", ()).ok();
        }
    }

    // PHASE 2: Fetch metadata in background (if API key configured)
    if !indexed_items.is_empty() && !api_key.is_empty() {
        let db_path_bg = db_path.clone();
        let image_cache_dir_bg = image_cache_dir.clone();
        let app_handle_clone = app_handle.clone();

        tokio::spawn(async move {
            let _ = tokio::task::spawn_blocking(move || {
                let db = match database::Database::new(&db_path_bg) {
                    Ok(d) => d,
                    Err(_) => return,
                };

                let mut tv_metadata_cache: std::collections::HashMap<String, Option<tmdb::TmdbMetadata>> = std::collections::HashMap::new();
                let mut tv_show_updated: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut season_cache: std::collections::HashMap<(String, i32), Vec<tmdb::TmdbEpisodeInfo>> = std::collections::HashMap::new();

                for (media_id, title, _file_id, is_tv, season_opt, episode_opt, _folder_id) in indexed_items {
                    if is_tv {
                        let season = season_opt.unwrap_or(1);
                        let episode = episode_opt.unwrap_or(1);
                        let title_lower = title.to_lowercase();

                        let show_meta = if let Some(cached) = tv_metadata_cache.get(&title_lower) {
                            cached.clone()
                        } else {
                            let meta = tmdb::search_metadata(&api_key, &title, "tv", None, &image_cache_dir_bg).ok().flatten();
                            tv_metadata_cache.insert(title_lower.clone(), meta.clone());
                            meta
                        };

                        if let Some(ref meta) = show_meta {
                            if !tv_show_updated.contains(&title_lower) {
                                if let Ok(Some(show)) = db.find_tvshow_by_title(&title) {
                                    db.update_metadata(show.id, meta).ok();
                                }
                                tv_show_updated.insert(title_lower.clone());
                            }

                            if let Some(ref tmdb_id) = meta.tmdb_id {
                                let cache_key = (tmdb_id.clone(), season);
                                let episodes = if let Some(cached_eps) = season_cache.get(&cache_key) {
                                    cached_eps.clone()
                                } else {
                                    match tmdb::fetch_season_episodes(&api_key, tmdb_id, season, &title, &image_cache_dir_bg) {
                                        Ok(season_info) => {
                                            let eps = season_info.episodes.clone();
                                            season_cache.insert(cache_key.clone(), eps.clone());
                                            eps
                                        }
                                        Err(_) => {
                                            season_cache.insert(cache_key.clone(), Vec::new());
                                            Vec::new()
                                        }
                                    }
                                };

                                if let Some(ep_info) = episodes.iter().find(|e| e.episode_number == episode) {
                                    db.update_episode_metadata(
                                        media_id,
                                        Some(&ep_info.name),
                                        ep_info.overview.as_deref(),
                                        ep_info.still_path.as_deref()
                                    ).ok();
                                }
                            }
                        }
                    } else {
                        if let Ok(Some(meta)) = tmdb::search_metadata(&api_key, &title, "movie", None, &image_cache_dir_bg) {
                            db.update_metadata(media_id, &meta).ok();
                        }
                    }
                }
            }).await;

            // Emit library-updated again after metadata fetch
            if let Some(window) = app_handle_clone.get_window("main") {
                window.emit("library-updated", ()).ok();
            }
        });
    }

    let total_duration = start_time.elapsed();
    println!("[CLOUD BG] Poll complete: {} indexed, {} skipped ({:?})", indexed_count, skipped_count, total_duration);

    Ok(CloudIndexResult {
        success: true,
        indexed_count,
        skipped_count,
        movies_count,
        tv_count,
        message: format!("Indexed {} new files", indexed_count),
    })
}

// ============== AUTO-UPDATE COMMANDS ==============

// GitHub PAT for accessing private releases
const GITHUB_RELEASE_TOKEN: &str = ""; // User will provide their PAT

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_notes: String,
    pub download_url: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    published_at: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: i64,
}

/// Check for updates from GitHub releases
#[tauri::command]
async fn check_for_updates() -> Result<UpdateInfo, String> {
    let current_version = env!("CARGO_PKG_VERSION");
    let repo = "SlasshyOverhere/StreamVault";

    println!("[UPDATE] Checking for updates... Current version: {}", current_version);

    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);

    let client = reqwest::Client::new();
    let mut request = client.get(&url)
        .header("User-Agent", "StreamVault-Updater")
        .header("Accept", "application/vnd.github+json");

    // Add auth header if PAT is configured
    if !GITHUB_RELEASE_TOKEN.is_empty() {
        request = request.header("Authorization", format!("Bearer {}", GITHUB_RELEASE_TOKEN));
    }

    let response = request.send().await.map_err(|e| format!("Failed to check for updates: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("GitHub API error ({}): {}", status, error_text));
    }

    let release: GitHubRelease = response.json().await.map_err(|e| format!("Failed to parse release: {}", e))?;

    // Extract version from tag (remove 'v' prefix if present)
    let latest_version = release.tag_name.trim_start_matches('v').to_string();

    // Compare versions
    let is_newer = version_compare(&latest_version, current_version);

    // Find Windows installer asset
    let download_url = release.assets.iter()
        .find(|a| a.name.ends_with(".msi") || a.name.ends_with(".exe") || a.name.ends_with("_x64-setup.exe"))
        .map(|a| a.browser_download_url.clone());

    println!("[UPDATE] Latest version: {} (newer: {})", latest_version, is_newer);

    Ok(UpdateInfo {
        available: is_newer,
        current_version: current_version.to_string(),
        latest_version,
        release_notes: release.body.unwrap_or_default(),
        download_url,
        published_at: release.published_at,
    })
}

/// Simple version comparison (assumes semver-like versions)
fn version_compare(latest: &str, current: &str) -> bool {
    let parse_version = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|s| s.parse().ok())
            .collect()
    };

    let latest_parts = parse_version(latest);
    let current_parts = parse_version(current);

    for i in 0..3 {
        let l = latest_parts.get(i).copied().unwrap_or(0);
        let c = current_parts.get(i).copied().unwrap_or(0);
        if l > c {
            return true;
        }
        if l < c {
            return false;
        }
    }
    false
}

/// Download update to temp directory
#[tauri::command]
async fn download_update(
    window: tauri::Window,
    url: String,
) -> Result<String, String> {
    use std::io::Write;

    println!("[UPDATE] Downloading update from: {}", url);

    let client = reqwest::Client::new();
    let mut request = client.get(&url);

    // Add auth header if PAT is configured
    if !GITHUB_RELEASE_TOKEN.is_empty() {
        request = request.header("Authorization", format!("Bearer {}", GITHUB_RELEASE_TOKEN));
    }

    let response = request.send().await.map_err(|e| format!("Failed to start download: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed: HTTP {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);
    let filename = url.split('/').last().unwrap_or("update.exe");
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join(filename);

    let mut file = std::fs::File::create(&file_path)
        .map_err(|e| format!("Failed to create temp file: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("Download error: {}", e))?;
        file.write_all(&chunk).map_err(|e| format!("Write error: {}", e))?;
        downloaded += chunk.len() as u64;

        // Emit progress event
        if total_size > 0 {
            let progress = (downloaded as f64 / total_size as f64) * 100.0;
            window.emit("update-download-progress", serde_json::json!({
                "downloaded": downloaded,
                "total": total_size,
                "progress": progress
            })).ok();
        }
    }

    println!("[UPDATE] Download complete: {:?}", file_path);

    Ok(file_path.to_string_lossy().to_string())
}

/// Install update and restart app
#[tauri::command]
async fn install_update(installer_path: String) -> Result<(), String> {
    use std::process::Command;

    println!("[UPDATE] Installing update from: {}", installer_path);

    // Launch the installer
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &installer_path])
            .spawn()
            .map_err(|e| format!("Failed to launch installer: {}", e))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        Command::new("open")
            .arg(&installer_path)
            .spawn()
            .map_err(|e| format!("Failed to launch installer: {}", e))?;
    }

    // Exit the app to allow installer to run
    println!("[UPDATE] Exiting app for update installation...");
    std::process::exit(0);
}

/// Get current app version
#[tauri::command]
fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn main() {
    // Load .env file from project root (for development)
    // This allows setting GDRIVE_CLIENT_ID and GDRIVE_CLIENT_SECRET
    dotenvy::dotenv().ok();

    // Prepare deep link - must be done before building the app
    // This registers the streamvault:// protocol handler
    tauri_plugin_deep_link::prepare("com.streamvault.app");

    // Initialize paths
    let db_path = database::get_database_path();
    let image_cache_dir = database::get_image_cache_dir();

    // Ensure directories exist
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::create_dir_all(&image_cache_dir).ok();

    // Initialize database
    let db = database::Database::new(&db_path)
        .expect("Failed to initialize database");

    // Load config
    let config = config::load_config().unwrap_or_default();

    // Create app state
    let state = AppState {
        db: Mutex::new(db),
        config: Mutex::new(config.clone()),
        is_scanning: Arc::new(AtomicBool::new(false)),
        active_mpv_sessions: Mutex::new(HashMap::new()),
        gdrive_client: gdrive::GoogleDriveClient::new(),
    };

    // Create system tray menu
    let show = CustomMenuItem::new("show".to_string(), "Show StreamVault");
    let quit = CustomMenuItem::new("quit".to_string(), "Quit");
    let tray_menu = SystemTrayMenu::new()
        .add_item(show)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(quit);

    let system_tray = SystemTray::new().with_menu(tray_menu);

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // Another instance tried to start - bring existing window to front
            println!("[SINGLE-INSTANCE] Another instance attempted to start, focusing existing window");
            if let Some(window) = app.get_window("main") {
                window.show().ok();
                window.unminimize().ok();
                window.set_focus().ok();
            } else {
                // Window was destroyed, create a new one
                println!("[SINGLE-INSTANCE] Creating new window...");
                match create_main_window(app) {
                    Ok(window) => {
                        window.set_focus().ok();
                        println!("[SINGLE-INSTANCE] New window created");
                    }
                    Err(e) => {
                        println!("[SINGLE-INSTANCE] Failed to create window: {}", e);
                    }
                }
            }
        }))
        .system_tray(system_tray)
        .on_system_tray_event(|app, event| {
            match event {
                SystemTrayEvent::LeftClick { .. } => {
                    // Show window on left click - create if destroyed
                    match app.get_window("main") {
                        Some(window) => {
                            window.show().ok();
                            window.set_focus().ok();
                        }
                        None => {
                            // Window was destroyed, create a new one
                            println!("[TRAY] Creating new window...");
                            match create_main_window(app) {
                                Ok(window) => {
                                    window.set_focus().ok();
                                    println!("[TRAY] New window created");
                                }
                                Err(e) => {
                                    println!("[TRAY] Failed to create window: {}", e);
                                }
                            }
                        }
                    }
                }
                SystemTrayEvent::MenuItemClick { id, .. } => {
                    match id.as_str() {
                        "show" => {
                            match app.get_window("main") {
                                Some(window) => {
                                    window.show().ok();
                                    window.set_focus().ok();
                                }
                                None => {
                                    // Window was destroyed, create a new one
                                    println!("[TRAY] Creating new window...");
                                    match create_main_window(app) {
                                        Ok(window) => {
                                            window.set_focus().ok();
                                            println!("[TRAY] New window created");
                                        }
                                        Err(e) => {
                                            println!("[TRAY] Failed to create window: {}", e);
                                        }
                                    }
                                }
                            }
                        }
                        "quit" => {
                            std::process::exit(0);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        })
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--flag1", "--flag2"])))
        .manage(state)
        .setup(|app| {
            // Register deep link handler for OAuth callback
            // The callback page redirects to: streamvault://oauth?code=XXX
            let handle = app.handle();
            tauri_plugin_deep_link::register("streamvault", move |request| {
                println!("[DEEPLINK] Received: {}", request);

                // Parse the deep link URL: streamvault://oauth?code=XXX
                if let Ok(url) = url::Url::parse(&request) {
                    // Look for the authorization code
                    if let Some(code) = url.query_pairs().find(|(k, _)| k == "code").map(|(_, v)| v.to_string()) {
                        println!("[DEEPLINK] Extracted OAuth code");

                        // Send the code through the channel
                        if let Ok(tx) = OAUTH_CODE_CHANNEL.0.lock() {
                            if let Err(e) = tx.send(code) {
                                println!("[DEEPLINK] Failed to send code: {}", e);
                            }
                        }

                        // Bring the app to front
                        if let Some(window) = handle.get_window("main") {
                            window.show().ok();
                            window.set_focus().ok();
                        }
                    }
                }
            }).ok();

            // Merge any duplicate TV shows on startup
            println!("[STARTUP] Running duplicate TV show merge...");
            let db_path = database::get_database_path();
            if let Ok(startup_db) = database::Database::new(&db_path) {
                if let Err(e) = startup_db.merge_duplicate_tvshows() {
                    println!("[STARTUP] Warning: Failed to merge duplicates: {}", e);
                }
            }

            // Clean up expired cloud cache on startup
            let config = config::load_config().unwrap_or_default();
            if config.cloud_cache_enabled {
                if let Some(ref cache_dir) = config.cloud_cache_dir {
                    println!("[STARTUP] Cleaning up expired cloud cache...");
                    let (deleted, freed) = cleanup_expired_cache(cache_dir, config.cloud_cache_expiry_hours);
                    if deleted > 0 {
                        println!("[STARTUP] Cleaned up {} expired cache files ({:.1} MB)",
                            deleted, freed as f64 / (1024.0 * 1024.0));
                    }
                }
            }

            // Start background cloud polling (runs independently of window)
            let app_handle_for_polling = app.handle();
            tauri::async_runtime::spawn(async move {
                background_cloud_poll(app_handle_for_polling).await;
            });

            Ok(())
        })
        .on_page_load(|window, payload| {
            // Inject popup blocking script into every page load
            // This runs at the webview level and can intercept iframe popups
            let url = payload.url();
            println!("[PageLoad] URL: {}", url);

            // Inject comprehensive popup blocking script
            let popup_block_script = r#"
                (function() {
                    // Block window.open
                    const originalOpen = window.open;
                    window.open = function(url, target, features) {
                        console.log('[AdBlocker] Blocked window.open:', url);
                        return null;
                    };

                    // Block popup via addEventListener
                    window.addEventListener('click', function(e) {
                        const target = e.target;
                        if (target && target.tagName === 'A') {
                            const href = target.getAttribute('href');
                            const targetAttr = target.getAttribute('target');
                            if (targetAttr === '_blank' && href && !href.includes('videasy.net')) {
                                console.log('[AdBlocker] Blocked link:', href);
                                e.preventDefault();
                                e.stopPropagation();
                            }
                        }
                    }, true);

                    // Override createElement to intercept dynamic script/iframe ads
                    const originalCreateElement = document.createElement.bind(document);
                    document.createElement = function(tagName) {
                        const element = originalCreateElement(tagName);
                        if (tagName.toLowerCase() === 'iframe') {
                            // Monitor iframe src changes
                            const originalSetAttribute = element.setAttribute.bind(element);
                            element.setAttribute = function(name, value) {
                                if (name === 'src' && value) {
                                    const blockedDomains = ['popads', 'popcash', 'propellerads', 'adsterra', 'exoclick'];
                                    if (blockedDomains.some(d => value.includes(d))) {
                                        console.log('[AdBlocker] Blocked iframe:', value);
                                        return;
                                    }
                                }
                                return originalSetAttribute(name, value);
                            };
                        }
                        return element;
                    };

                    console.log('[AdBlocker] Popup blocking injected');
                })();
            "#;

            window.eval(popup_block_script).ok();
        })
        .on_window_event(|event| {
            match event.event() {
                tauri::WindowEvent::CloseRequested { .. } => {
                    // Let the window close/destroy completely to free RAM
                    // Don't prevent close - we handle app exit separately in .run()
                    println!("[TRAY] Window closing/destroying. Backend will keep running.");
                }
                tauri::WindowEvent::Focused(focused) => {
                    if *focused {
                        // Re-inject popup blocker when window regains focus
                        let window = event.window();
                        window.eval(r#"
                            if (!window.__adBlockerActive) {
                                window.__adBlockerActive = true;
                                const origOpen = window.open;
                                window.open = function(url) {
                                    console.log('[AdBlocker] Blocked popup on focus:', url);
                                    return null;
                                };
                            }
                        "#).ok();
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_library,
            get_library_filtered,
            get_episodes,
            get_watch_history,
            remove_from_watch_history,
            clear_all_watch_history,
            mark_as_complete,
            // Streaming history commands
            save_streaming_progress,
            get_streaming_history,
            get_streaming_resume_info,
            remove_from_streaming_history,
            clear_all_streaming_history,
            // App reset command
            clear_all_app_data,
            cleanup_missing_metadata,
            repair_file_paths,
            // Other commands
            delete_media_files,
            delete_series,
            delete_series_cloud_folder,
            get_episodes_for_delete,
            get_config,
            save_config,
            get_scan_status,
            get_resume_info,
            get_media_info,
            get_stream_info,
            update_progress,
            clear_progress,
            fix_match,
            play_with_mpv,
            play_with_vlc,
            get_mpv_status,
            get_active_mpv_sessions,
            get_cached_image,
            get_cached_image_path,
            read_video_chunk,
            get_video_file_size,
            // Transcoding commands
            check_needs_transcode,
            start_transcode_stream,
            stop_transcode_stream,
            get_stream_info_with_transcode,
            search_tmdb,
            get_tv_details,
            get_tv_season_episodes,
            refresh_series_metadata,
            merge_duplicate_shows,
            // Videasy player commands
            open_videasy_player,
            save_videasy_progress,
            // Google Drive commands
            gdrive_is_connected,
            gdrive_get_account_info,
            gdrive_start_auth,
            gdrive_complete_auth,
            gdrive_auth_with_code,
            gdrive_disconnect,
            gdrive_list_folders,
            gdrive_list_files,
            gdrive_list_video_files,
            gdrive_get_stream_url,
            gdrive_get_file_metadata,
            gdrive_scan_folder,
            gdrive_delete_folder_media,
            // Cloud folder management
            add_cloud_folder,
            remove_cloud_folder,
            get_cloud_folders,
            scan_all_cloud_folders,
            check_cloud_changes,
            // Cloud cache commands
            get_cloud_cache_info,
            cleanup_cloud_cache,
            clear_cloud_cache,
            // Auto-update commands
            check_for_updates,
            download_update,
            install_update,
            get_app_version,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            // Prevent app from exiting when last window closes
            // This keeps the backend running so we can recreate the window from tray
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
                println!("[TRAY] Exit prevented. App running in background. Click tray to reopen.");
            }
        });
}