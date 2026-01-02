// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod database;
mod config;
mod media_manager;
mod tmdb;
mod mpv_ipc;
mod watcher;

use tauri_plugin_autostart::MacosLauncher;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use tauri::{State, Window};
use serde::Serialize;

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

// Response for delete operation
#[derive(serde::Serialize)]
struct DeleteResponse {
    success: bool,
    deleted_count: usize,
    failed_count: usize,
    message: String,
}

// Delete media files permanently from disk (bypasses recycle bin)
#[tauri::command]
async fn delete_media_files(
    state: State<'_, AppState>,
    media_ids: Vec<i64>,
) -> Result<DeleteResponse, String> {
    if media_ids.is_empty() {
        return Err("No media IDs provided".to_string());
    }
    
    println!("[DELETE] Starting permanent deletion for {} items", media_ids.len());
    
    // Get file paths and delete from database
    let file_paths = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.delete_media_entries(&media_ids).map_err(|e| e.to_string())?
    };
    
    let _total_files = file_paths.len();
    let mut deleted_count = 0;
    let mut failed_count = 0;
    
    // Delete files from disk permanently
    for file_path in file_paths {
        let path = std::path::Path::new(&file_path);
        if path.exists() {
            match std::fs::remove_file(path) {
                Ok(_) => {
                    println!("[DELETE] Successfully deleted: {}", file_path);
                    deleted_count += 1;
                }
                Err(e) => {
                    println!("[DELETE] Failed to delete {}: {}", file_path, e);
                    failed_count += 1;
                }
            }
        } else {
            println!("[DELETE] File not found (already deleted?): {}", file_path);
            deleted_count += 1; // Count as success since file doesn't exist
        }
    }
    
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
    
    // Get all episode IDs first
    let episode_ids: Vec<i64> = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let episodes = db.get_episodes(series_id).map_err(|e| e.to_string())?;
        episodes.into_iter().map(|ep| ep.id).collect()
    };
    
    let mut total_deleted = 0;
    let mut total_failed = 0;
    
    // Delete episodes first if requested
    if !episode_ids.is_empty() {
        let file_paths = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.delete_media_entries(&episode_ids).map_err(|e| e.to_string())?
        };
        
        if delete_files {
            for file_path in file_paths {
                let path = std::path::Path::new(&file_path);
                if path.exists() {
                    match std::fs::remove_file(path) {
                        Ok(_) => {
                            println!("[DELETE] Deleted episode file: {}", file_path);
                            total_deleted += 1;
                        }
                        Err(e) => {
                            println!("[DELETE] Failed to delete episode {}: {}", file_path, e);
                            total_failed += 1;
                        }
                    }
                } else {
                    total_deleted += 1;
                }
            }
        } else {
            total_deleted = episode_ids.len();
        }
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

// Scan library with event emissions
#[tauri::command]
async fn scan_library(
    window: Window,
    state: State<'_, AppState>,
) -> Result<ApiResponse, String> {
    // Check if already scanning
    if state.is_scanning.load(Ordering::SeqCst) {
        return Ok(ApiResponse {
            message: "Scan already in progress.".to_string(),
        });
    }
    
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };
    
    // Mark as scanning and get a reference for the thread
    state.is_scanning.store(true, Ordering::SeqCst);
    let is_scanning = state.is_scanning.clone();
    
    // Spawn scan in background
    let db_path = database::get_database_path();
    let image_cache_dir = database::get_image_cache_dir();
    
    std::thread::spawn(move || {
        println!("[SCAN] Starting library scan...");
        
        if let Ok(db) = database::Database::new(&db_path) {
            media_manager::scan_media_folders_with_events(&db, &config, &image_cache_dir, &window);
        }
        
        // Reset scanning flag
        is_scanning.store(false, Ordering::SeqCst);
        
        // Emit scan complete event
        let _ = window.emit("scan-complete", ScanCompletePayload {
            movies_count: 0,
            tv_count: 0,
        });
        
        println!("[SCAN] Library scan complete!");
    });
    
    Ok(ApiResponse {
        message: "Scan initiated.".to_string(),
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
}

#[tauri::command]
async fn get_stream_info(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<StreamInfo, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let media = db.get_media_by_id(media_id).map_err(|e| e.to_string())?;
    
    let file_path = media.file_path.clone().unwrap_or_default();
    
    // Return the file path - frontend will use convertFileSrc to create proper asset URL
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
    
    let api_key = config.tmdb_api_key.as_ref()
        .ok_or_else(|| "TMDB API Key not set".to_string())?;
    
    if api_key.is_empty() {
        return Err("TMDB API Key not set".to_string());
    }
    
    let image_cache_dir = database::get_image_cache_dir();
    let metadata = tmdb::fetch_metadata_by_id(api_key, &tmdb_id, &media_type, &image_cache_dir)
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
    
    let (media, resume_info, file_path) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let media = db.get_media_by_id(media_id).map_err(|e| e.to_string())?;
        let resume_info = db.get_resume_info(media_id).map_err(|e| e.to_string())?;
        let file_path = media.file_path.clone().ok_or_else(|| "No file path".to_string())?;
        
        // Update last_watched
        db.update_last_watched(media_id).map_err(|e| e.to_string())?;
        
        (media, resume_info, file_path)
    };
    
    // Determine start position
    let start_position = if resume && resume_info.has_progress {
        resume_info.position
    } else {
        0.0
    };
    
    // Launch MPV with progress tracking
    let mpv_path_clone = mpv_path.clone();
    let file_path_clone = file_path.clone();
    let title = media.title.clone();
    
    // Launch MPV with tracking
    let pid = mpv_ipc::launch_mpv_with_tracking(
        &mpv_path_clone,
        &file_path_clone,
        media_id,
        start_position,
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
            .user_agent("SlasshyMediaIndexer/1.0")
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
            .user_agent("SlasshyMediaIndexer/1.0")
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
        config.tmdb_api_key.clone().unwrap_or_default()
    };

    if credential.is_empty() {
        return Err("TMDB API key/token not configured".to_string());
    }

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
        config.tmdb_api_key.clone().unwrap_or_default()
    };

    if credential.is_empty() {
        return Err("TMDB API key/token not configured".to_string());
    }

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
        config.tmdb_api_key.clone().unwrap_or_default()
    };

    if credential.is_empty() {
        return Err("TMDB API key/token not configured".to_string());
    }

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
        let key = config.tmdb_api_key.clone().unwrap_or_default();
        println!("[SEARCH_TMDB] Credential length: {} (is_token: {})", key.len(), is_access_token(&key));
        key
    };

    if credential.is_empty() {
        println!("[SEARCH_TMDB] API key/token is empty!");
        return Err("TMDB API key/token not configured".to_string());
    }

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

// Open Videasy in a webview window with progress sync
#[tauri::command]
async fn open_videasy_player(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    url: String,
    tmdb_id: String,
    media_type: String,
    title: String,
    poster_path: Option<String>,
    season: Option<i32>,
    episode: Option<i32>,
) -> Result<ApiResponse, String> {
    use tauri::Manager;

    println!("[VIDEASY] Opening player for: {} (tmdb_id: {})", title, tmdb_id);

    // Get saved progress from database
    let saved_progress = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_streaming_resume_info(&tmdb_id, &media_type, season, episode)
            .map_err(|e| e.to_string())?
    };

    // Build the storage key for Videasy localStorage
    let storage_key = if media_type == "movie" {
        format!("movie-{}", tmdb_id)
    } else {
        format!("tv-{}-{}-{}", tmdb_id, season.unwrap_or(1), episode.unwrap_or(1))
    };

    // Build the injection script for saved progress
    let inject_script = if let Some(progress) = saved_progress {
        let storage_item = VideasyStorageItem {
            poster: poster_path.clone().map(|p| format!("https://image.tmdb.org/t/p/w300{}", p)),
            background: None,
            id: tmdb_id.parse().unwrap_or(0),
            media_type: media_type.clone(),
            title: title.clone(),
            progress: Some(VideasyProgress {
                duration: progress.duration_seconds,
                watched: progress.resume_position_seconds,
            }),
        };

        let json = serde_json::to_string(&storage_item).unwrap_or_default();
        format!(
            r#"
            (function() {{
                try {{
                    console.log('[Slasshy] Injecting saved progress for {}');
                    localStorage.setItem('{}', '{}');
                    console.log('[Slasshy] Progress injected successfully');
                }} catch (e) {{
                    console.error('[Slasshy] Failed to inject progress:', e);
                }}
            }})();
            "#,
            storage_key, storage_key, json.replace('\'', "\\'").replace('\n', "\\n")
        )
    } else {
        String::new()
    };

    // Create the webview window
    let window_label = format!("videasy-{}", tmdb_id);
    let window_title = if media_type == "tv" {
        format!("{} - S{}E{}", title, season.unwrap_or(1), episode.unwrap_or(1))
    } else {
        title.clone()
    };

    // Check if window already exists
    if let Some(existing) = app_handle.get_window(&window_label) {
        existing.set_focus().map_err(|e| e.to_string())?;
        return Ok(ApiResponse {
            message: "Player window already open".to_string(),
        });
    }

    // Parse the URL
    let parsed_url: url::Url = url.parse().map_err(|e| format!("Invalid URL: {}", e))?;

    let window = tauri::WindowBuilder::new(
        &app_handle,
        &window_label,
        tauri::WindowUrl::External(parsed_url)
    )
    .title(&window_title)
    .inner_size(1280.0, 720.0)
    .min_inner_size(800.0, 450.0)
    .resizable(true)
    .fullscreen(false)
    .center()
    .build()
    .map_err(|e| e.to_string())?;

    // Comprehensive popup blocker script
    let popup_blocker = r#"
        (function() {
            'use strict';

            // Block window.open completely
            const noop = function() {
                console.log('[Slasshy] Blocked window.open');
                return null;
            };
            window.open = noop;

            // Prevent overriding window.open
            Object.defineProperty(window, 'open', {
                value: noop,
                writable: false,
                configurable: false
            });

            // Block popups from iframes
            try {
                for (let i = 0; i < window.frames.length; i++) {
                    try {
                        window.frames[i].open = noop;
                    } catch(e) {}
                }
            } catch(e) {}

            // Block target="_blank" links and ad clicks
            document.addEventListener('click', function(e) {
                let target = e.target;
                while (target && target.tagName !== 'A') {
                    target = target.parentElement;
                }
                if (target) {
                    const href = target.getAttribute('href') || '';
                    const targetAttr = target.getAttribute('target');

                    // Block external links that open in new tabs
                    if (targetAttr === '_blank' || targetAttr === '_new') {
                        if (!href.includes('videasy.net') && !href.includes('player.videasy')) {
                            console.log('[Slasshy] Blocked external link:', href);
                            e.preventDefault();
                            e.stopPropagation();
                            e.stopImmediatePropagation();
                            return false;
                        }
                    }

                    // Block common ad domains
                    const adDomains = ['popads', 'popcash', 'propeller', 'exoclick', 'juicyads',
                                       'trafficjunky', 'clickadu', 'adsterra', 'onclick', 'popunder'];
                    if (adDomains.some(d => href.includes(d))) {
                        console.log('[Slasshy] Blocked ad link:', href);
                        e.preventDefault();
                        e.stopPropagation();
                        return false;
                    }
                }
            }, true);

            // Block mousedown/mouseup popup triggers
            ['mousedown', 'mouseup', 'pointerdown', 'pointerup'].forEach(function(eventType) {
                document.addEventListener(eventType, function(e) {
                    const target = e.target;
                    if (target.tagName === 'A' || target.closest('a')) {
                        const link = target.tagName === 'A' ? target : target.closest('a');
                        const href = link.getAttribute('href') || '';
                        if (!href.includes('videasy') && (link.getAttribute('target') === '_blank')) {
                            e.stopPropagation();
                        }
                    }
                }, true);
            });

            console.log('[Slasshy] Popup blocker fully active');
        })();
    "#;

    // Clone values for the threads
    let window_for_progress = window.clone();
    let storage_key_clone = storage_key.clone();
    let tmdb_id_clone = tmdb_id.clone();
    let media_type_clone = media_type.clone();
    let title_clone = title.clone();
    let poster_path_clone = poster_path.clone();
    let db_path = database::get_database_path();
    let season_clone = season;
    let episode_clone = episode;
    let window_title_clone = window_title.clone();

    // Progress extraction and saving thread
    std::thread::spawn(move || {
        // Wait for page to load
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Inject popup blocker
        if let Err(e) = window_for_progress.eval(popup_blocker) {
            println!("[VIDEASY] Failed to inject popup blocker: {}", e);
        } else {
            println!("[VIDEASY] Popup blocker injected");
        }

        // Inject saved progress
        if !inject_script.is_empty() {
            if let Err(e) = window_for_progress.eval(&inject_script) {
                println!("[VIDEASY] Failed to inject progress: {}", e);
            } else {
                println!("[VIDEASY] Progress injected");
            }
        }

        // Track progress using window title as communication channel
        let mut last_progress_watched: f64 = 0.0;
        let mut last_progress_duration: f64 = 0.0;
        let original_title = window_title_clone;

        loop {
            std::thread::sleep(std::time::Duration::from_secs(10));

            // Inject script that puts progress in window title temporarily
            let extract_script = format!(r#"
                (function() {{
                    try {{
                        // Re-block popups
                        if (!window.__slasshyBlocked) {{
                            window.__slasshyBlocked = true;
                            window.open = function() {{ return null; }};
                        }}

                        const data = localStorage.getItem('{}');
                        if (data) {{
                            const parsed = JSON.parse(data);
                            if (parsed.progress && parsed.progress.watched > 0) {{
                                // Temporarily set title with progress data
                                const watched = parsed.progress.watched.toFixed(2);
                                const duration = parsed.progress.duration.toFixed(2);
                                document.title = 'SLASSHY_PROGRESS:' + watched + ':' + duration;

                                // Restore title after a short delay
                                setTimeout(function() {{
                                    document.title = '{}';
                                }}, 200);
                            }}
                        }}
                    }} catch (e) {{}}
                }})();
            "#, storage_key_clone, original_title.replace('\'', "\\'"));

            match window_for_progress.eval(&extract_script) {
                Ok(_) => {
                    // Wait for title to be set
                    std::thread::sleep(std::time::Duration::from_millis(100));

                    // Try to read the title
                    if let Ok(title) = window_for_progress.title() {
                        if title.starts_with("SLASSHY_PROGRESS:") {
                            let parts: Vec<&str> = title.split(':').collect();
                            if parts.len() >= 3 {
                                if let (Ok(w), Ok(d)) = (parts[1].parse::<f64>(), parts[2].parse::<f64>()) {
                                    last_progress_watched = w;
                                    last_progress_duration = d;
                                    println!("[VIDEASY] Progress extracted: {:.1}s / {:.1}s", w, d);

                                    // Save to database periodically
                                    if let Ok(db) = database::Database::new(&db_path) {
                                        let poster_url = poster_path_clone.as_ref().map(|p| {
                                            if p.starts_with("http") { p.clone() }
                                            else { format!("https://image.tmdb.org/t/p/w342{}", p) }
                                        });

                                        let _ = db.save_streaming_progress(
                                            &tmdb_id_clone,
                                            &media_type_clone,
                                            &title_clone,
                                            poster_url.as_deref(),
                                            season_clone,
                                            episode_clone,
                                            last_progress_watched,
                                            last_progress_duration,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                Err(_) => {
                    // Window is closed - save final progress
                    println!("[VIDEASY] Window closed, saving final progress...");

                    if last_progress_watched > 0.0 {
                        if let Ok(db) = database::Database::new(&db_path) {
                            let poster_url = poster_path_clone.as_ref().map(|p| {
                                if p.starts_with("http") { p.clone() }
                                else { format!("https://image.tmdb.org/t/p/w342{}", p) }
                            });

                            match db.save_streaming_progress(
                                &tmdb_id_clone,
                                &media_type_clone,
                                &title_clone,
                                poster_url.as_deref(),
                                season_clone,
                                episode_clone,
                                last_progress_watched,
                                last_progress_duration,
                            ) {
                                Ok(_) => println!("[VIDEASY] Final progress saved: {:.1}s / {:.1}s",
                                                last_progress_watched, last_progress_duration),
                                Err(e) => println!("[VIDEASY] Failed to save final progress: {}", e),
                            }
                        }
                    }
                    break;
                }
            }
        }
    });

    Ok(ApiResponse {
        message: format!("Videasy player opened for: {}", title),
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

fn main() {
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
        config: Mutex::new(config),
        is_scanning: Arc::new(AtomicBool::new(false)),
        active_mpv_sessions: Mutex::new(HashMap::new()),
    };
    
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--flag1", "--flag2"])))
        .manage(state)
        .setup(|app| {
            // Merge any duplicate TV shows on startup
            println!("[STARTUP] Running duplicate TV show merge...");
            let db_path = database::get_database_path();
            if let Ok(startup_db) = database::Database::new(&db_path) {
                if let Err(e) = startup_db.merge_duplicate_tvshows() {
                    println!("[STARTUP] Warning: Failed to merge duplicates: {}", e);
                }
            }
            
            let app_handle = app.handle();
            let mut watcher = watcher::MediaWatcher::new();
            watcher.start(app_handle);
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
            // Handle window events
            if let tauri::WindowEvent::Focused(focused) = event.event() {
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
        })
        .invoke_handler(tauri::generate_handler![
            get_library,
            get_episodes,
            get_watch_history,
            remove_from_watch_history,
            clear_all_watch_history,
            // Streaming history commands
            save_streaming_progress,
            get_streaming_history,
            get_streaming_resume_info,
            remove_from_streaming_history,
            clear_all_streaming_history,
            // App reset command
            clear_all_app_data,
            // Other commands
            delete_media_files,
            delete_series,
            get_episodes_for_delete,
            get_config,
            save_config,
            scan_library,
            get_scan_status,
            get_resume_info,
            get_media_info,
            get_stream_info,
            update_progress,
            clear_progress,
            fix_match,
            play_with_mpv,
            get_mpv_status,
            get_active_mpv_sessions,
            get_cached_image,
            get_cached_image_path,
            read_video_chunk,
            get_video_file_size,
            search_tmdb,
            get_tv_details,
            get_tv_season_episodes,
            refresh_series_metadata,
            merge_duplicate_shows,
            // Videasy player commands
            open_videasy_player,
            save_videasy_progress,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}