// Prevents additional console window on WebviewWindows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod archive_manager;
mod balloonerismm_api;
mod cf_relay;
mod cinemeta_api;
mod config;
mod database;
mod direct_link_manager;
mod download_manager;
mod gdrive;
mod gdrive_stream_proxy;
mod http_client;
mod imdb_api;
mod log_buffer;
mod media_manager;
mod mpv_ipc;
mod remote_source;
mod remote_stream_proxy;
mod stremio;
mod tmdb;
mod transcoder;
mod watch_together;
mod watch_together_mpv;
mod zip_manager;
mod zip_parser;
mod zip_stream_proxy;

mod stream_cache;

use tauri::menu::{Menu, MenuBuilder};

use tauri::tray::TrayIconBuilder;
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_deep_link::DeepLinkExt;

use chrono::{
    DateTime, Datelike, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone,
    Timelike, Utc,
};
use notify_rust::Notification as SystemNotification;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow, WebviewWindowBuilder, WebviewUrl, WindowEvent};
use uuid::Uuid;

// ponytail: LazyLock replaces lazy_static dependency
static OAUTH_CODE_CHANNEL: LazyLock<(Mutex<mpsc::Sender<String>>, Mutex<mpsc::Receiver<String>>)> =
    LazyLock::new(|| {
        let (tx, rx) = mpsc::channel();
        (Mutex::new(tx), Mutex::new(rx))
    });
static RECENT_UI_NOTIFICATIONS: LazyLock<Mutex<HashMap<String, std::time::Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static RUNNING_ADDON_PROCESS: LazyLock<Mutex<Option<tokio::process::Child>>> =
    LazyLock::new(|| Mutex::new(None));
static ADDON_STDOUT_TASK: LazyLock<Mutex<Option<tokio::task::JoinHandle<()>>>> =
    LazyLock::new(|| Mutex::new(None));
static ADDON_WATCHDOG_RUNNING: LazyLock<Mutex<bool>> = LazyLock::new(|| Mutex::new(false));
static ADDON_LOG_HISTORY: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));
// Retry counter for the watchdog — resets on successful health check
static ADDON_RESTART_COUNT: LazyLock<Mutex<u32>> = LazyLock::new(|| Mutex::new(0));
// Global app handle for emitting events from background threads
static GLOBAL_APP_HANDLE: LazyLock<Mutex<Option<AppHandle>>> = LazyLock::new(|| Mutex::new(None));

// MPV session info
#[derive(Clone, Serialize)]
pub struct MpvSession {
    pub media_id: i64,
    pub pid: u32,
    pub title: String,
    pub start_time: i64,
}

pub struct ActiveMpvSession {
    pub session: MpvSession,
    pub zip_proxy: Option<zip_stream_proxy::ZipStreamProxyHandle>,
    pub gdrive_stream_proxy: Option<gdrive_stream_proxy::GdriveStreamProxyHandle>,
}

pub struct ActiveZipStream {
    pub created_at: std::time::Instant,
    pub proxy: zip_stream_proxy::ZipStreamProxyHandle,
}

// Application state
pub struct AppState {
    pub db: Mutex<database::Database>,
    pub config: Mutex<config::Config>,
    pub is_scanning: Arc<AtomicBool>,
    pub active_mpv_sessions: Mutex<HashMap<i64, ActiveMpvSession>>,
    pub active_zip_streams: Mutex<HashMap<i64, ActiveZipStream>>,
    pub download_manager: download_manager::DownloadManager,
    pub gdrive_client: gdrive::GoogleDriveClient,
    pub watch_together: Arc<tokio::sync::Mutex<watch_together::WatchTogetherManager>>,
    pub wt_controller: Arc<tokio::sync::Mutex<Option<watch_together_mpv::WatchTogetherController>>>,
    pub oauth_listener: Arc<Mutex<Option<tokio::net::TcpListener>>>,
    pub oauth_nonce: Arc<Mutex<Option<String>>>,
    pub cache_manager: stream_cache::CacheManager,
    pub last_validation_report: Mutex<Option<database::SyncValidationReport>>,
    pub gdrive_refresh_watchdog: Arc<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
}

/// RAII guard that prevents concurrent cloud folder scans.
/// Acquired via `try_acquire` (returns `None` if already scanning)
/// and automatically releases the lock when dropped.
pub struct ScanLock<'a> {
    flag: &'a AtomicBool,
}

impl<'a> ScanLock<'a> {
    pub fn try_acquire(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| ScanLock { flag })
    }
}

impl<'a> Drop for ScanLock<'a> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
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

#[derive(Debug, Serialize, Deserialize)]
struct WatchHistorySnapshot {
    version: i32,
    exported_at: String,
    events: Vec<database::WatchHistoryEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WatchlistSnapshot {
    version: i32,
    exported_at: String,
    items: Vec<database::WatchlistItem>,
}

#[derive(Debug, Serialize)]
struct WatchHistorySyncStatus {
    synced: bool,
    merged_remote_events: usize,
    uploaded_events: usize,
    skipped_reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct WatchlistSyncStatus {
    synced: bool,
    merged_remote_items: usize,
    uploaded_items: usize,
    skipped_reason: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DdlIndexProgressPayload {
    stage: String,
    message: String,
    filename: Option<String>,
    current: Option<usize>,
    total: Option<usize>,
    season: Option<i32>,
    episode: Option<i32>,
    episode_title: Option<String>,
}

fn watchlist_merge_key(item: &database::WatchlistItem) -> String {
    format!("{}::{}", item.media_type, item.tmdb_id)
}

fn merge_watchlist_items(
    local_items: Vec<database::WatchlistItem>,
    remote_items: Vec<database::WatchlistItem>,
) -> (Vec<database::WatchlistItem>, usize) {
    let mut merged: HashMap<String, database::WatchlistItem> = HashMap::new();
    let mut remote_wins = 0usize;

    for item in local_items {
        merged.insert(watchlist_merge_key(&item), item);
    }

    for remote in remote_items {
        let key = watchlist_merge_key(&remote);
        match merged.get(&key) {
            Some(local) if local.updated_at >= remote.updated_at => {}
            Some(_) => {
                remote_wins += 1;
                merged.insert(key, remote);
            }
            None => {
                remote_wins += 1;
                merged.insert(key, remote);
            }
        }
    }

    let mut items: Vec<_> = merged.into_values().collect();
    items.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.created_at.cmp(&a.created_at))
    });
    (items, remote_wins)
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

fn enrich_media_item_archive_assessment(mut media: database::MediaItem) -> database::MediaItem {
    if let Some(assessment) = archive_manager::assess_archive_playback(&media) {
        media.archive_playback_can_play = Some(assessment.can_play);
        media.archive_playback_mode = Some(assessment.mode);
        media.archive_playback_message = Some(assessment.message);
        media.archive_playback_details = Some(assessment.details);
    }

    media
}

fn enrich_media_items_archive_assessment(
    items: Vec<database::MediaItem>,
) -> Vec<database::MediaItem> {
    items
        .into_iter()
        .map(enrich_media_item_archive_assessment)
        .collect()
}

async fn sync_watch_history_to_drive(state: &AppState) -> Result<WatchHistorySyncStatus, String> {
    if !state.gdrive_client.is_authenticated() {
        return Ok(WatchHistorySyncStatus {
            synced: false,
            merged_remote_events: 0,
            uploaded_events: 0,
            skipped_reason: Some("Google Drive is not connected".to_string()),
        });
    }

    let mut merged_remote_events = 0usize;

    if let Some(remote_json) = state.gdrive_client.load_watch_history_snapshot().await? {
        let remote_snapshot: WatchHistorySnapshot = serde_json::from_str(&remote_json)
            .map_err(|e| format!("Failed to parse remote watch history snapshot: {}", e))?;
        let db = state.db.lock().map_err(|e| e.to_string())?;
        merged_remote_events = db
            .upsert_watch_history_events(&remote_snapshot.events)
            .map_err(|e| e.to_string())?;
    }

    let events = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_watch_history_events(5000)
            .map_err(|e| e.to_string())?
    };

    let snapshot = WatchHistorySnapshot {
        version: 1,
        exported_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        events: events.clone(),
    };
    let snapshot_json = serde_json::to_string(&snapshot)
        .map_err(|e| format!("Failed to serialize watch history snapshot: {}", e))?;
    state
        .gdrive_client
        .save_watch_history_snapshot(&snapshot_json)
        .await?;

    Ok(WatchHistorySyncStatus {
        synced: true,
        merged_remote_events,
        uploaded_events: events.len(),
        skipped_reason: None,
    })
}

async fn sync_watchlist_to_drive(state: &AppState) -> Result<WatchlistSyncStatus, String> {
    if !state.gdrive_client.is_authenticated() {
        return Ok(WatchlistSyncStatus {
            synced: false,
            merged_remote_items: 0,
            uploaded_items: 0,
            skipped_reason: Some("Google Drive is not connected".to_string()),
        });
    }

    let local_items = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_watchlist_items(true).map_err(|e| e.to_string())?
    };

    let remote_items =
        if let Some(remote_json) = state.gdrive_client.load_watchlist_snapshot().await? {
            let remote_snapshot: WatchlistSnapshot = serde_json::from_str(&remote_json)
                .map_err(|e| format!("Failed to parse remote watchlist snapshot: {}", e))?;
            remote_snapshot.items
        } else {
            Vec::new()
        };

    let (merged_items, merged_remote_items) = merge_watchlist_items(local_items, remote_items);

    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.replace_watchlist_items(&merged_items)
            .map_err(|e| e.to_string())?;
    }

    let snapshot = WatchlistSnapshot {
        version: 1,
        exported_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        items: merged_items.clone(),
    };
    let snapshot_json = serde_json::to_string(&snapshot)
        .map_err(|e| format!("Failed to serialize watchlist snapshot: {}", e))?;
    state
        .gdrive_client
        .save_watchlist_snapshot(&snapshot_json)
        .await?;

    Ok(WatchlistSyncStatus {
        synced: true,
        merged_remote_items,
        uploaded_items: merged_items.len(),
        skipped_reason: None,
    })
}

// Get library items (movies or TV shows)
#[tauri::command]
async fn get_library(
    state: State<'_, AppState>,
    media_type: String,
    search: Option<String>,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let db_type = if media_type == "tv" {
        "tvshow"
    } else {
        "movie"
    };
    let items = db
        .get_library(db_type, search.as_deref())
        .map_err(|e| e.to_string())?;
    Ok(enrich_media_items_archive_assessment(items))
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
    let db_type = if media_type == "tv" {
        "tvshow"
    } else {
        "movie"
    };
    let items = db
        .get_library_filtered(db_type, search.as_deref(), is_cloud)
        .map_err(|e| e.to_string())?;
    Ok(enrich_media_items_archive_assessment(items))
}

// Combined search across all libraries (local + cloud, movies + TV) in one IPC call
#[tauri::command]
async fn search_all_libraries(
    state: State<'_, AppState>,
    search: String,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    for (db_type, is_cloud) in [
        ("movie", Some(false)),
        ("tvshow", Some(false)),
        ("movie", Some(true)),
        ("tvshow", Some(true)),
    ] {
        results.extend(
            db.get_library_filtered(db_type, Some(search.as_str()), is_cloud)
                .map_err(|e| e.to_string())?,
        );
    }
    // Deduplicate by id
    results.sort_by_key(|item| item.id);
    results.dedup_by_key(|item| item.id);
    Ok(enrich_media_items_archive_assessment(results))
}

// Search library by cast member name
#[tauri::command]
async fn search_media_by_cast(
    state: State<'_, AppState>,
    cast_name: String,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let items = db
        .search_media_by_cast(&cast_name)
        .map_err(|e| e.to_string())?;
    Ok(enrich_media_items_archive_assessment(items))
}

// Get DDL library items
#[tauri::command]
async fn get_ddl_media(
    state: State<'_, AppState>,
    media_type: String,
    search: Option<String>,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let db_type = if media_type == "tv" {
        "tvshow"
    } else {
        "movie"
    };
    let items = db
        .get_ddl_media(db_type, search.as_deref())
        .map_err(|e| e.to_string())?;
    Ok(enrich_media_items_archive_assessment(items))
}

// Get episodes for a TV show
#[tauri::command]
async fn get_nickname(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_setting("nickname").map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_nickname(state: State<'_, AppState>, nickname: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.set_setting("nickname", nickname.trim())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_recently_added(
    state: State<'_, AppState>,
    limit: Option<i32>,
    is_cloud: Option<bool>,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let items = db
        .get_recently_added(limit.unwrap_or(10), is_cloud)
        .map_err(|e| e.to_string())?;
    Ok(enrich_media_items_archive_assessment(items))
}

#[tauri::command]
async fn get_episodes(
    state: State<'_, AppState>,
    series_id: i64,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let items = db.get_episodes(series_id).map_err(|e| e.to_string())?;
    Ok(enrich_media_items_archive_assessment(items))
}

// Get watch history
#[tauri::command]
async fn get_watch_history(
    state: State<'_, AppState>,
    limit: Option<i32>,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let items = db
        .get_watch_history(limit.unwrap_or(50))
        .map_err(|e| e.to_string())?;
    Ok(enrich_media_items_archive_assessment(items))
}

#[tauri::command]
async fn get_watch_history_events(
    state: State<'_, AppState>,
    limit: Option<i32>,
) -> Result<Vec<database::WatchHistoryEvent>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_watch_history_events(limit.unwrap_or(200))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_library_stats(
    state: State<'_, AppState>,
    is_cloud: Option<bool>,
) -> Result<database::LibraryStats, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_library_stats(is_cloud).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_top_space_consumers(
    state: State<'_, AppState>,
    limit: Option<i32>,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_top_space_consumers(limit.unwrap_or(10))
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

#[tauri::command]
async fn remove_watch_history_entry(
    state: State<'_, AppState>,
    event_id: String,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.remove_watch_history_event(&event_id)
        .map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: "Watch history entry removed".to_string(),
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
                                                            // Set position to duration (100% complete) - this will trigger the watched reset in update_progress
    db.update_progress(media_id, duration, duration)
        .map_err(|e| e.to_string())?;

    Ok(ApiResponse {
        message: format!("Marked '{}' as complete", item.title),
    })
}

// Clear all watch history
#[tauri::command]
async fn clear_all_watch_history(state: State<'_, AppState>) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let count = db.clear_all_watch_history().map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: format!("Cleared {} items from watch history", count),
    })
}

#[tauri::command]
async fn sync_watch_history(state: State<'_, AppState>) -> Result<WatchHistorySyncStatus, String> {
    sync_watch_history_to_drive(&state).await
}

#[tauri::command]
async fn sync_watchlist(state: State<'_, AppState>) -> Result<WatchlistSyncStatus, String> {
    sync_watchlist_to_drive(&state).await
}

// ==================== SOCIAL SYNC COMMANDS ====================

// Get aggregated watch stats for social sync
#[tauri::command]
async fn get_watch_stats(
    state: State<'_, AppState>,
) -> Result<database::WatchStatsAggregated, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_watch_stats().map_err(|e| e.to_string())
}

// Get recently completed watch activities since a timestamp
#[tauri::command]
async fn get_recent_watch_activities(
    state: State<'_, AppState>,
    since_timestamp: String,
) -> Result<Vec<database::WatchActivityItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_recent_watch_activities(&since_timestamp)
        .map_err(|e| e.to_string())
}

// Get all analytics data for the analytics dashboard
#[tauri::command]
async fn get_analytics_data(state: State<'_, AppState>) -> Result<database::AnalyticsData, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_analytics_data().map_err(|e| e.to_string())
}

// ==================== GOOGLE DRIVE COMMANDS ====================

/// Check if user is connected to Google Drive
#[tauri::command]
async fn gdrive_is_connected(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.gdrive_client.validate_tokens().await)
}

/// Get Google Drive access token for social features
#[tauri::command]
async fn gdrive_get_access_token(state: State<'_, AppState>) -> Result<String, String> {
    state.gdrive_client.get_access_token().await
}

/// Lightweight snapshot of the OAuth session for the JS auth-state hook.
/// Returns token presence plus the most-recent permanent refresh failure
/// so the UI can distinguish fully-signed-in from refresh-lost-but-library-ok.
#[tauri::command]
async fn gdrive_get_auth_status(
    state: State<'_, AppState>,
) -> Result<gdrive::GDriveAuthStatus, String> {
    Ok(state.gdrive_client.get_auth_status())
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
async fn gdrive_start_auth(state: State<'_, AppState>) -> Result<String, String> {
    // Drop any previous listener first to free the port
    *state.oauth_listener.lock().unwrap() = None;

    // Start local TCP listener BEFORE opening browser so it's ready
    // to receive the callback redirect from the backend
    let listener = gdrive::start_oauth_listener().await?;
    *state.oauth_listener.lock().unwrap() = Some(listener);

    // Generate a cryptographic nonce for CSRF/client-session binding.
    // The backend will store this alongside the OAuth state and return it
    // in the callback redirect, so we can verify the flow was initiated by us.
    let nonce = Uuid::new_v4().to_string();
    *state.oauth_nonce.lock().unwrap() = Some(nonce.clone());

    let auth_url = gdrive::get_auth_url_with_nonce(&nonce);

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
async fn gdrive_complete_auth(
    state: State<'_, AppState>,
) -> Result<gdrive::DriveAccountInfo, String> {
    // Take the listener that was started in gdrive_start_auth
    let listener = state.oauth_listener.lock().unwrap().take().ok_or_else(|| {
        "OAuth callback listener not started. Did you call gdrive_start_auth first?".to_string()
    })?;

    // Retrieve the expected nonce (set in gdrive_start_auth)
    let expected_nonce = state.oauth_nonce.lock().unwrap().take();

    // Wait for tokens from backend (it redirects to localhost with tokens)
    let tokens =
        gdrive::wait_for_oauth_callback_with_nonce(&listener, expected_nonce.as_deref()).await?;
    println!("[GDRIVE] Received tokens from backend (nonce verified)");

    // Store tokens
    state.gdrive_client.store_tokens(tokens)?;
    println!("[GDRIVE] Tokens stored successfully");

    // Get and return account info
    state.gdrive_client.get_account_info().await
}

/// Abort the GDrive refresh watchdog JoinHandle if present. Used on app
/// exit to ensure the background 25-min polling task is cancelled cleanly
/// rather than lingering until the tokio runtime itself drops.
fn abort_gdrive_refresh_watchdog(app_handle: &AppHandle) {
    if let Ok(mut h) = app_handle
        .state::<AppState>()
        .gdrive_refresh_watchdog
        .lock()
    {
        if let Some(handle) = h.take() {
            println!("[EXIT] Aborting GDrive refresh watchdog");
            handle.abort();
        }
    }
}

/// Disconnect from Google Drive
#[tauri::command]
async fn gdrive_disconnect(state: State<'_, AppState>) -> Result<ApiResponse, String> {
    state.gdrive_client.revoke_and_clear_tokens().await?;
    Ok(ApiResponse {
        message: "Disconnected from Google Drive".to_string(),
    })
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
    state
        .gdrive_client
        .list_files(folder_id.as_deref(), None)
        .await
}

/// List video files in a folder (with optional recursive scan)
#[tauri::command]
async fn gdrive_list_video_files(
    state: State<'_, AppState>,
    folder_id: String,
    recursive: bool,
) -> Result<Vec<gdrive::DriveItem>, String> {
    state
        .gdrive_client
        .list_video_files(&folder_id, recursive)
        .await
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

/// Share a Google Drive file with a user by email
#[derive(serde::Serialize)]
struct ShareResult {
    success: bool,
    message: String,
}

#[tauri::command]
async fn gdrive_share_file(
    state: State<'_, AppState>,
    file_id: String,
    email: String,
    role: Option<String>,
) -> Result<ShareResult, String> {
    let role = role.unwrap_or_else(|| "reader".to_string());
    state
        .gdrive_client
        .create_permission(&file_id, &email, &role)
        .await?;
    Ok(ShareResult {
        success: true,
        message: format!("Successfully shared with {}", email),
    })
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
    /// Human-readable reasons for skipped files (e.g., "duplicate", "unsupported format", "permission denied")
    #[serde(skip_serializing_if = "Option::is_none", default)]
    skipped_reasons: Option<Vec<String>>,
}

/// Resolve an existing TV show using normalized/fuzzy matching to avoid split series
/// when cloud filenames use slightly different punctuation or formatting.
fn find_existing_cloud_tvshow(
    db: &database::Database,
    show_title: &str,
    year: Option<i32>,
) -> Option<database::MediaItem> {
    let series_id = db
        .find_series_by_tmdb_or_title(None, show_title, year)
        .ok()
        .flatten()?;
    db.get_media_by_id(series_id).ok()
}

fn find_existing_cloud_tvshow_by_path(
    db: &database::Database,
    show_path: &str,
) -> Option<database::MediaItem> {
    db.get_media_by_file_path(show_path)
        .ok()
        .flatten()
        .filter(|item| item.media_type == "tvshow")
}

fn cloud_tvshow_path(folder_id: &str, show_title: &str) -> String {
    let slug = show_title
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_");
    format!("gdrive:{}:{}", folder_id, slug)
}

fn auto_merge_duplicate_tvshows(db: &database::Database, source: &str) -> i32 {
    match db.repair_misparented_archive_episodes() {
        Ok(count) if count > 0 => {
            println!(
                "[REPAIR] Re-parented {} archive episode(s) before merge from '{}'",
                count, source
            );
        }
        Ok(_) => {}
        Err(e) => {
            println!(
                "[REPAIR] Archive episode parent repair from '{}' failed: {}",
                source, e
            );
        }
    }

    match db.merge_duplicate_tvshows() {
        Ok(count) => {
            if count > 0 {
                println!(
                    "[MERGE] Auto-merge from '{}' consolidated {} duplicate TV show entries",
                    source, count
                );
            }
            count
        }
        Err(e) => {
            println!("[MERGE] Auto-merge from '{}' failed: {}", source, e);
            0
        }
    }
}

/// Scan a cloud folder and index its contents
/// Auto-detects movies vs TV shows based on filename patterns
#[tauri::command]
async fn gdrive_scan_folder(
    state: State<'_, AppState>,
    window: WebviewWindow,
    folder_id: String,
    folder_name: String,
) -> Result<CloudIndexResult, String> {
    // Acquire scan lock to prevent concurrent scans from racing
    let _scan_lock = ScanLock::try_acquire(&state.is_scanning).ok_or_else(|| {
        "A cloud scan is already in progress. Please wait for it to complete.".to_string()
    })?;

    println!(
        "[CLOUD] Starting scan of folder: {} (auto-detect)",
        folder_name
    );

    // Get video files from the folder
    let files = state
        .gdrive_client
        .list_video_files(&folder_id, true)
        .await?;
    println!("[CLOUD] Found {} video files", files.len());

    let failed_retry_ids: Vec<String> = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_cloud_index_failures(250)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|item| item.cloud_file_id)
            .collect()
    };

    let mut files = files;
    if !failed_retry_ids.is_empty() {
        let existing_ids: std::collections::HashSet<String> =
            files.iter().map(|item| item.id.clone()).collect();
        let mut retry_loaded = 0usize;
        for file_id in failed_retry_ids {
            if existing_ids.contains(&file_id) {
                continue;
            }
            match state.gdrive_client.get_file_metadata(&file_id).await {
                Ok(item) if gdrive::is_supported_cloud_media_item(&item) => {
                    files.push(item);
                    retry_loaded += 1;
                }
                Ok(_) => {}
                Err(error) => {
                    println!(
                        "[CLOUD] Failed to reload skipped file {} for retry: {}",
                        file_id, error
                    );
                }
            }
        }
        if retry_loaded > 0 {
            println!(
                "[CLOUD] Added {} previously skipped file(s) to this manual rescan",
                retry_loaded
            );
        }
    }

    let unsupported_archives: Vec<String> = files
        .iter()
        .filter(|file| is_unsupported_archive_drive_item(file))
        .map(|file| {
            let _ = unsupported_archive_reason(file);
            file.name.clone()
        })
        .collect();
    let files: Vec<_> = files
        .into_iter()
        .filter(|file| !is_unsupported_archive_drive_item(file))
        .collect();

    notify_unsupported_archives_window(&window, &unsupported_archives);

    // Get API key from config
    let (api_key, zip_indexing_enabled, archive_cache_config) = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        (
            tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default()),
            config.zip_indexing_enabled,
            build_zip_cache_config(&config),
        )
    };

    // API key check is no longer needed since we have a default

    // Get image cache dir for poster downloads
    let image_cache_dir = database::get_image_cache_dir();
    std::fs::create_dir_all(&image_cache_dir).ok();

    // Clone data for the blocking task
    let folder_id_clone = folder_id.clone();
    let zip_files_detected: Vec<String> = if zip_indexing_enabled {
        files
            .iter()
            .filter(|file| is_zip_drive_item(file))
            .map(|file| file.name.clone())
            .collect()
    } else {
        Vec::new()
    };
    let zip_access_token = if zip_indexing_enabled && files.iter().any(is_zip_drive_item) {
        Some(state.gdrive_client.get_access_token().await?)
    } else {
        None
    };

    if !zip_files_detected.is_empty() {
        let archive_name = zip_files_detected.first().map(|name| name.as_str());
        emit_zip_processing_event(
            &window,
            "detected",
            zip_files_detected.len(),
            archive_name,
            None,
            &format!(
                "Archive{} detected in {}. Processing episode entries...",
                if zip_files_detected.len() == 1 {
                    ""
                } else {
                    "s"
                },
                folder_name
            ),
        );
    }

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
        let mut entry_counter = 0usize;
        let mut skipped_reasons: Vec<String> = Vec::new();

        // Cache for TV shows: title -> (db_id, tmdb_id, show_folder_id)
        let mut tv_show_cache: HashMap<String, (i64, Option<String>, String)> = HashMap::new();

        // Cache for season episodes: (tmdb_id, season) -> Vec<episode_info>
        let mut season_cache: HashMap<(String, i32), Vec<tmdb::TmdbEpisodeInfo>> = HashMap::new();

        for file in files {
            // Check if already indexed
            if db.cloud_file_exists(&file.id) {
                let _ = db.clear_cloud_index_failure(&file.id);
                skipped_count += 1;
                skipped_reasons.push(format!("{} — already indexed (cloud ID exists)", file.name));
                continue;
            }

            if is_zip_drive_item(&file) {
                if !zip_indexing_enabled {
                    skipped_count += 1;
                    skipped_reasons.push(format!("{} — ZIP indexing disabled in settings", file.name));
                    continue;
                }

                let Some(access_token) = zip_access_token.as_deref() else {
                    skipped_count += 1;
                    skipped_reasons.push(format!("{} — ZIP access token unavailable", file.name));
                    continue;
                };

                match index_zip_archive_with_metadata(
                    &db,
                    access_token,
                    &file,
                    &folder_id_clone,
                    &api_key,
                    &image_cache_dir,
                    &archive_cache_config,
                    &mut tv_show_cache,
                    &mut season_cache,
                ) {
                    Ok(indexed_items) => {
                        if indexed_items.is_empty() {
                            let _ = db.upsert_cloud_index_failure(
                                &file.id,
                                &file.name,
                                "Archive was scanned but no playable TV episode entries were identified",
                            );
                            skipped_count += 1;
                            skipped_reasons.push(format!("{} — ZIP archive contained no playable TV episodes", file.name));
                            continue;
                        }

                        let _ = db.clear_cloud_index_failure(&file.id);

                        for (media_id, title, _, _, season, episode, _, _) in indexed_items {
                            indexed_count += 1;
                            tv_count += 1;
                            entry_counter += 1;
                            println!(
                                "[INDEX] #{} archive episode '{}' S{:02}E{:02} (db_id: {}, archive_file_id: {})",
                                entry_counter,
                                title,
                                season.unwrap_or(0),
                                episode.unwrap_or(0),
                                media_id,
                                file.id
                            );
                        }
                        continue;
                    }
                    Err(error) => {
                        println!("[ARCHIVE] Failed to index '{}': {}", file.name, error);
                        let _ = db.upsert_cloud_index_failure(&file.id, &file.name, &error);
                        skipped_count += 1;
                        skipped_reasons.push(format!("{} — ZIP indexing error: {}", file.name, error));
                        continue;
                    }
                }
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
                    let result = if let Some(existing_show) =
                        find_existing_cloud_tvshow(&db, &show_title, parsed.year)
                    {
                        // Use existing show's folder or the episode's parent
                        (existing_show.id, existing_show.tmdb_id, episode_parent_folder.clone())
                    } else {
                        let show_path = cloud_tvshow_path(&episode_parent_folder, &show_title);
                        if let Some(existing_show) =
                            find_existing_cloud_tvshow_by_path(&db, &show_path)
                        {
                            println!(
                                "[CLOUD] Reusing existing show by path '{}' with ID {}",
                                show_path, existing_show.id
                            );
                            (
                                existing_show.id,
                                existing_show.tmdb_id,
                                episode_parent_folder.clone(),
                            )
                        } else {
                        println!("[CLOUD] Searching metadata for show: {}", show_title);
                        let mut tmdb_result = tmdb::search_metadata_with_fallback(
                            &api_key,
                            &show_title,
                            "tv",
                            parsed.year,
                            &image_cache_dir,
                        ).ok().flatten();

                        // Log TMDB search result
                        if let Some(ref meta) = tmdb_result {
                            println!("[TMDB] Search result for \"{}\": poster_path={:?}, imdb_id={:?}", show_title, meta.poster_path, meta.imdb_id);
                        }

                        // Always prefer imdbapi.dev poster over TMDB
                        if let Some(ref meta) = tmdb_result {
                            if let Some(ref imdb_id) = meta.imdb_id {
                                println!("[IMDBAPI] Trying imdbapi.dev poster for \"{}\" (imdb_id: {})", show_title, imdb_id);
                                let imdb_url = format!("https://api.imdbapi.dev/titles/{}", imdb_id);
                                if let Ok(resp) = http_client::shared_client().get(&imdb_url).send() {
                                    if let Ok(json) = resp.json::<serde_json::Value>() {
                                        if let Some(img_url) = json.get("primaryImage").and_then(|i| i.get("url")).and_then(|u| u.as_str()) {
                                            if let Some(cached) = tmdb::cache_imdb_image(img_url, std::path::Path::new(&image_cache_dir), &tmdb::ImageType::SeriesBanner) {
                                                println!("[IMDBAPI] Using imdbapi.dev poster as primary for \"{}\"", show_title);
                                                if let Some(ref mut m) = tmdb_result {
                                                    m.poster_path = Some(cached);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Create the show
                        let (title, year, overview, cast_names, poster_path, tmdb_id_opt) = match &tmdb_result {
                            Some(meta) => (
                                meta.title.clone(),
                                meta.year,
                                meta.overview.clone(),
                                meta.cast_names.clone(),
                                meta.poster_path.clone(),
                                meta.tmdb_id.clone(),
                            ),
                            None => (show_title.clone(), None, None, None, None, None),
                        };

                        let title = media_manager::prefer_title_with_leading_article(&show_title, &title);

                        // Use episode's parent folder as the show's folder ID (for deletion)
                        match db.insert_cloud_tvshow(
                            &title,
                            year,
                            overview.as_deref(),
                            cast_names.as_deref(),
                            poster_path.as_deref(),
                            &show_path,
                            &episode_parent_folder,  // Use episode's parent folder, not tracked folder
                            tmdb_id_opt.as_deref(),
                        ) {
                            Ok(show_id) => (show_id, tmdb_id_opt, episode_parent_folder.clone()),
                            Err(e) => {
                                if let Some(existing_show) =
                                    find_existing_cloud_tvshow_by_path(&db, &show_path)
                                {
                                    println!(
                                        "[CLOUD] Insert collided for '{}'; reusing existing show ID {}",
                                        show_path, existing_show.id
                                    );
                                    (
                                        existing_show.id,
                                        existing_show.tmdb_id,
                                        episode_parent_folder.clone(),
                                    )
                                } else {
                                    println!("[CLOUD] Failed to insert show: {}", e);
                                    continue;
                                }
                            }
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
                let file_size_bytes = file.size.as_ref().and_then(|s| s.parse::<i64>().ok());
                let ep_id = match db.insert_cloud_episode(
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
                    file_size_bytes,
                ) {
                    Ok(id) => id,
                    Err(e) => {
                        println!("[CLOUD] Failed to insert episode: {}", e);
                        continue;
                    }
                };

                indexed_count += 1;
                tv_count += 1;
                entry_counter += 1;
                let _ = db.clear_cloud_index_failure(&file.id);
                println!("[CLOUD] Indexed TV: {} S{:02}E{:02}", show_title, season, episode);
                println!(
                    "[INDEX] #{} TV episode '{}' (db_id: {}, cloud_file_id: {})",
                    entry_counter,
                    file.name,
                    ep_id,
                    file.id
                );

            } else {
                println!("[CLOUD] Searching metadata for movie: {}", parsed.title);
                let mut tmdb_result = tmdb::search_metadata_with_fallback(
                    &api_key,
                    &parsed.title,
                    "movie",
                    parsed.year,
                    &image_cache_dir,
                ).ok().flatten();

                // Log TMDB search result
                if let Some(ref meta) = tmdb_result {
                    println!("[TMDB] Search result for \"{}\": poster_path={:?}, imdb_id={:?}", parsed.title, meta.poster_path, meta.imdb_id);
                }

                // Always prefer imdbapi.dev poster over TMDB
                if let Some(ref meta) = tmdb_result {
                    if let Some(ref imdb_id) = meta.imdb_id {
                        println!("[IMDBAPI] Trying imdbapi.dev poster for \"{}\" (imdb_id: {})", parsed.title, imdb_id);
                        let imdb_url = format!("https://api.imdbapi.dev/titles/{}", imdb_id);
                        if let Ok(resp) = http_client::shared_client().get(&imdb_url).send() {
                            if let Ok(json) = resp.json::<serde_json::Value>() {
                                if let Some(img_url) = json.get("primaryImage").and_then(|i| i.get("url")).and_then(|u| u.as_str()) {
                                    if let Some(cached) = tmdb::cache_imdb_image(img_url, std::path::Path::new(&image_cache_dir), &tmdb::ImageType::MovieBanner) {
                                        println!("[IMDBAPI] Using imdbapi.dev poster as primary for \"{}\"", parsed.title);
                                        if let Some(ref mut m) = tmdb_result {
                                            m.poster_path = Some(cached);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let (title, year, overview, cast_names, director, poster_path, tmdb_id, runtime_seconds) = match tmdb_result {
                    Some(meta) => (
                        meta.title,
                        meta.year,
                        meta.overview,
                        meta.cast_names,
                        meta.director,
                        meta.poster_path,
                        meta.tmdb_id,
                        meta.runtime_seconds.unwrap_or(0.0),
                    ),
                    None => (parsed.title.clone(), parsed.year, None, None, None, None, None, 0.0),
                };

                let title = media_manager::prefer_title_with_leading_article(&parsed.title, &title);

                // Insert into database
                let movie_id = match db.insert_cloud_movie(
                    &title,
                    year,
                    overview.as_deref(),
                    cast_names.as_deref(),
                    director.as_deref(),
                    poster_path.as_deref(),
                    &file.name,
                    &file.id,
                    &folder_id_clone,
                    runtime_seconds,
                    tmdb_id.as_deref(),
                ) {
                    Ok(id) => id,
                    Err(e) => {
                        println!("[CLOUD] Failed to insert movie: {}", e);
                        continue;
                    }
                };

                indexed_count += 1;
                movies_count += 1;
                entry_counter += 1;
                let _ = db.clear_cloud_index_failure(&file.id);
                println!("[CLOUD] Indexed Movie: {}", title);
                println!(
                    "[INDEX] #{} Movie '{}' (db_id: {}, cloud_file_id: {})",
                    entry_counter,
                    file.name,
                    movie_id,
                    file.id
                );
            }
        }

        Ok((indexed_count, skipped_count, movies_count, tv_count, skipped_reasons))
    }).await.map_err(|e| format!("Task failed: {}", e))??;

    let (indexed_count, skipped_count, movies_count, tv_count, mut skipped_reasons) = result;

    // Add TAR archive skip reasons
    for archive_name in &unsupported_archives {
        skipped_reasons.push(format!(
            "{} — TAR archives are not supported (requires sequential read)",
            archive_name
        ));
    }
    let skipped_count = skipped_count + unsupported_archives.len();

    if !zip_files_detected.is_empty() {
        let archive_name = zip_files_detected.first().map(|name| name.as_str());
        let zip_indexed_count = indexed_count; // items actually indexed from ZIPs
        let status_msg = if zip_indexed_count > 0 {
            format!(
                "Finished processing {} ZIP archive(s). {} episode(s) added to your library.",
                zip_files_detected.len(),
                zip_indexed_count
            )
        } else {
            format!(
                "Finished processing {} ZIP archive(s). No episodes could be indexed — check file naming or archive integrity.",
                zip_files_detected.len()
            )
        };
        emit_zip_processing_event(
            &window,
            if zip_indexed_count > 0 {
                "complete"
            } else {
                "warning"
            },
            zip_files_detected.len(),
            archive_name,
            None,
            &status_msg,
        );
    }

    if indexed_count > 0 {
        if let Ok(db) = state.db.lock() {
            let merged = auto_merge_duplicate_tvshows(&db, "gdrive_scan_folder");
            if merged > 0 {
                window.emit("library-updated", ()).ok();
            }
        }
    }

    // Emit completion
    window
        .emit(
            "cloud-scan-complete",
            serde_json::json!({
                "folder": folder_name,
                "indexed": indexed_count,
                "skipped": skipped_count,
                "movies": movies_count,
                "tv": tv_count
            }),
        )
        .ok();

    window.emit("library-updated", ()).ok();

    let message = format!(
        "Indexed {} items ({} movies, {} TV episodes) from '{}' ({} skipped)",
        indexed_count, movies_count, tv_count, folder_name, skipped_count
    );
    println!("[CLOUD] {}", message);

    let final_skipped_reasons = if skipped_reasons.is_empty() {
        None
    } else {
        Some(skipped_reasons)
    };

    Ok(CloudIndexResult {
        success: indexed_count > 0 || skipped_count == 0,
        indexed_count,
        skipped_count,
        movies_count,
        tv_count,
        message,
        skipped_reasons: final_skipped_reasons,
    })
}

/// Delete all indexed media from a cloud folder
#[tauri::command]
async fn gdrive_delete_folder_media(
    state: State<'_, AppState>,
    window: WebviewWindow,
    folder_id: String,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let deleted = db
        .delete_cloud_folder_media(&folder_id)
        .map_err(|e| e.to_string())?;

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
    db.add_cloud_folder(&folder_id, &folder_name)
        .map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: format!("Added cloud folder: {}", folder_name),
    })
}

/// Remove a cloud folder from tracking
#[tauri::command]
async fn remove_cloud_folder(
    state: State<'_, AppState>,
    window: WebviewWindow,
    folder_id: String,
) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    // Delete media from this folder
    let deleted_media = db
        .delete_cloud_folder_media(&folder_id)
        .map_err(|e| e.to_string())?;

    // Remove folder from tracking
    db.remove_cloud_folder(&folder_id)
        .map_err(|e| e.to_string())?;

    window.emit("library-updated", ()).ok();

    Ok(ApiResponse {
        message: format!("Removed cloud folder and {} media items", deleted_media),
    })
}

/// Get all tracked cloud folders
#[tauri::command]
async fn get_cloud_folders(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let folders = db.get_cloud_folders().map_err(|e| e.to_string())?;

    Ok(folders
        .into_iter()
        .map(|(id, name, auto_scan)| {
            serde_json::json!({
                "id": id,
                "name": name,
                "auto_scan": auto_scan
            })
        })
        .collect())
}

/// Scan all cloud folders for new files (incremental scan)
#[tauri::command]
async fn scan_all_cloud_folders(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<CloudIndexResult, String> {
    // Acquire scan lock to prevent concurrent scans from racing
    let _scan_lock = ScanLock::try_acquire(&state.is_scanning).ok_or_else(|| {
        "A cloud scan is already in progress. Please wait for it to complete.".to_string()
    })?;

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
            message:
                "No cloud folders configured. Use the Indexing prompt or add folders in Settings."
                    .to_string(),
            skipped_reasons: None,
        });
    }

    let mut total_indexed = 0;
    let mut total_skipped = 0;
    let mut total_movies = 0;
    let mut total_tv = 0;
    let mut all_skipped_reasons: Vec<String> = Vec::new();

    for (folder_id, folder_name, auto_scan) in folders {
        if !auto_scan {
            println!(
                "[CLOUD SCAN] Skipping folder '{}' (auto_scan disabled)",
                folder_name
            );
            continue;
        }

        println!(
            "[CLOUD SCAN] Scanning folder: {} ({})",
            folder_name, folder_id
        );

        // Get video files from the folder
        let files = match state.gdrive_client.list_video_files(&folder_id, true).await {
            Ok(f) => f,
            Err(e) => {
                println!(
                    "[CLOUD SCAN] Error listing files for {}: {}",
                    folder_name, e
                );
                continue;
            }
        };
        let unsupported_archives: Vec<String> = files
            .iter()
            .filter(|file| is_unsupported_archive_drive_item(file))
            .map(|file| file.name.clone())
            .collect();
        if !unsupported_archives.is_empty() {
            notify_unsupported_archives_window(&window, &unsupported_archives);
        }
        let files: Vec<_> = files
            .into_iter()
            .filter(|file| !is_unsupported_archive_drive_item(file))
            .collect();

        // Get API key from config
        let (api_key, zip_indexing_enabled, archive_cache_config) = {
            let config = state.config.lock().map_err(|e| e.to_string())?;
            (
                tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default()),
                config.zip_indexing_enabled,
                build_zip_cache_config(&config),
            )
        };

        // API key is always available now with default

        // Get image cache dir for poster downloads
        let image_cache_dir = database::get_image_cache_dir();
        std::fs::create_dir_all(&image_cache_dir).ok();

        // Clone data for the blocking task
        let folder_id_clone = folder_id.clone();
        let db_path = database::get_database_path();
        let zip_access_token = if zip_indexing_enabled && files.iter().any(is_zip_drive_item) {
            Some(state.gdrive_client.get_access_token().await?)
        } else {
            None
        };

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
            let mut skipped_reasons: Vec<String> = Vec::new();

            let mut tv_show_cache: HashMap<String, (i64, Option<String>, String)> = HashMap::new();
            let mut season_cache: HashMap<(String, i32), Vec<tmdb::TmdbEpisodeInfo>> =
                HashMap::new();

            for file in files {
                if db.cloud_file_exists(&file.id) {
                    skipped_count += 1;
                    skipped_reasons
                        .push(format!("{} — already indexed (cloud ID exists)", file.name));
                    continue;
                }

                if is_zip_drive_item(&file) {
                    if !zip_indexing_enabled {
                        skipped_count += 1;
                        skipped_reasons
                            .push(format!("{} — ZIP indexing disabled in settings", file.name));
                        continue;
                    }

                    let Some(access_token) = zip_access_token.as_deref() else {
                        skipped_count += 1;
                        skipped_reasons
                            .push(format!("{} — ZIP access token unavailable", file.name));
                        continue;
                    };

                    match index_zip_archive_with_metadata(
                        &db,
                        access_token,
                        &file,
                        &folder_id_clone,
                        &api_key,
                        &image_cache_dir,
                        &archive_cache_config,
                        &mut tv_show_cache,
                        &mut season_cache,
                    ) {
                        Ok(indexed_items) => {
                            if indexed_items.is_empty() {
                                skipped_count += 1;
                            } else {
                                indexed_count += indexed_items.len();
                                tv_count += indexed_items.len();
                            }
                            continue;
                        }
                        Err(error) => {
                            println!("[ZIP] Failed to index '{}': {}", file.name, error);
                            skipped_count += 1;
                            skipped_reasons
                                .push(format!("{} — ZIP indexing error: {}", file.name, error));
                            continue;
                        }
                    }
                }

                let parsed = media_manager::parse_cloud_filename(&file.name);
                let is_tv_show = parsed.season.is_some() && parsed.episode.is_some();

                if is_tv_show {
                    let season = parsed.season.unwrap();
                    let episode = parsed.episode.unwrap();
                    let show_title = parsed.title.clone();
                    let show_title_lower = show_title.to_lowercase();

                    let (db_show_id, tmdb_id, _show_folder_id) = if let Some(cached) =
                        tv_show_cache.get(&show_title_lower)
                    {
                        cached.clone()
                    } else {
                        let result = if let Some(existing_show) =
                            find_existing_cloud_tvshow(&db, &show_title, parsed.year)
                        {
                            (
                                existing_show.id,
                                existing_show.tmdb_id,
                                folder_id_clone.clone(),
                            )
                        } else {
                            let show_path = cloud_tvshow_path(&folder_id_clone, &show_title);
                            if let Some(existing_show) =
                                find_existing_cloud_tvshow_by_path(&db, &show_path)
                            {
                                (
                                    existing_show.id,
                                    existing_show.tmdb_id,
                                    folder_id_clone.clone(),
                                )
                            } else {
                                let tmdb_result = tmdb::search_metadata_with_fallback(
                                    &api_key,
                                    &show_title,
                                    "tv",
                                    parsed.year,
                                    &image_cache_dir,
                                )
                                .ok()
                                .flatten();

                                let (title, year, overview, cast_names, poster_path, tmdb_id_opt) =
                                    match &tmdb_result {
                                        Some(meta) => (
                                            meta.title.clone(),
                                            meta.year,
                                            meta.overview.clone(),
                                            meta.cast_names.clone(),
                                            meta.poster_path.clone(),
                                            meta.tmdb_id.clone(),
                                        ),
                                        None => (show_title.clone(), None, None, None, None, None),
                                    };

                                let title = media_manager::prefer_title_with_leading_article(
                                    &show_title,
                                    &title,
                                );

                                match db.insert_cloud_tvshow(
                                    &title,
                                    year,
                                    overview.as_deref(),
                                    cast_names.as_deref(),
                                    poster_path.as_deref(),
                                    &show_path,
                                    &folder_id_clone,
                                    tmdb_id_opt.as_deref(),
                                ) {
                                    Ok(show_id) => (show_id, tmdb_id_opt, folder_id_clone.clone()),
                                    Err(_) => {
                                        if let Some(existing_show) =
                                            find_existing_cloud_tvshow_by_path(&db, &show_path)
                                        {
                                            (
                                                existing_show.id,
                                                existing_show.tmdb_id,
                                                folder_id_clone.clone(),
                                            )
                                        } else {
                                            continue;
                                        }
                                    }
                                }
                            }
                        };
                        tv_show_cache.insert(show_title_lower.clone(), result.clone());
                        result
                    };

                    let (ep_title, ep_overview, ep_still): (
                        Option<String>,
                        Option<String>,
                        Option<String>,
                    ) = if let Some(ref tid) = tmdb_id {
                        let cache_key = (tid.clone(), season);
                        let episodes = if let Some(cached_episodes) = season_cache.get(&cache_key) {
                            cached_episodes.clone()
                        } else {
                            match tmdb::fetch_season_episodes(
                                &api_key,
                                tid,
                                season,
                                &show_title,
                                &image_cache_dir,
                            ) {
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
                        episodes
                            .iter()
                            .find(|e| e.episode_number == episode)
                            .map(|e| {
                                (
                                    Some(e.name.clone()),
                                    e.overview.clone(),
                                    e.still_path.clone(),
                                )
                            })
                            .unwrap_or((None, None, None))
                    } else {
                        (None, None, None)
                    };

                    let file_size_bytes = file.size.as_ref().and_then(|s| s.parse::<i64>().ok());

                    if db
                        .insert_cloud_episode(
                            &show_title,
                            &file.name,
                            db_show_id,
                            season,
                            episode,
                            &file.id,
                            &folder_id_clone,
                            ep_title.as_deref(),
                            ep_overview.as_deref(),
                            ep_still.as_deref(),
                            file_size_bytes,
                        )
                        .is_err()
                    {
                        skipped_count += 1;
                        skipped_reasons.push(format!(
                            "{} — database insert failed for episode",
                            file.name
                        ));
                        continue;
                    }

                    indexed_count += 1;
                    tv_count += 1;
                } else {
                    let tmdb_result = tmdb::search_metadata_with_fallback(
                        &api_key,
                        &parsed.title,
                        "movie",
                        parsed.year,
                        &image_cache_dir,
                    )
                    .ok()
                    .flatten();

                    let (
                        title,
                        year,
                        overview,
                        cast_names,
                        director,
                        poster_path,
                        tmdb_id,
                        runtime_seconds,
                    ) = match tmdb_result {
                        Some(meta) => (
                            meta.title,
                            meta.year,
                            meta.overview,
                            meta.cast_names,
                            meta.director,
                            meta.poster_path,
                            meta.tmdb_id,
                            meta.runtime_seconds.unwrap_or(0.0),
                        ),
                        None => (
                            parsed.title.clone(),
                            parsed.year,
                            None,
                            None,
                            None,
                            None,
                            None,
                            0.0,
                        ),
                    };

                    let title =
                        media_manager::prefer_title_with_leading_article(&parsed.title, &title);

                    if db
                        .insert_cloud_movie(
                            &title,
                            year,
                            overview.as_deref(),
                            cast_names.as_deref(),
                            director.as_deref(),
                            poster_path.as_deref(),
                            &file.name,
                            &file.id,
                            &folder_id_clone,
                            runtime_seconds,
                            tmdb_id.as_deref(),
                        )
                        .is_err()
                    {
                        skipped_count += 1;
                        skipped_reasons
                            .push(format!("{} — database insert failed for movie", file.name));
                        continue;
                    }

                    indexed_count += 1;
                    movies_count += 1;
                }
            }

            Ok((
                indexed_count,
                skipped_count,
                movies_count,
                tv_count,
                skipped_reasons,
            ))
        })
        .await
        .map_err(|e| format!("Task failed: {}", e))?;

        let result = result?;

        let (indexed, skipped, movies, tv, mut skipped_reasons) = result;

        // Add TAR archive skip reasons
        for archive_name in &unsupported_archives {
            skipped_reasons.push(format!(
                "{} — TAR archives are not supported (requires sequential read)",
                archive_name
            ));
        }
        total_skipped += skipped + unsupported_archives.len();

        total_indexed += indexed;
        total_movies += movies;
        total_tv += tv;

        all_skipped_reasons.extend(skipped_reasons);

        // Update last scanned timestamp
        if let Ok(db) = state.db.lock() {
            let _ = db.update_cloud_folder_scanned(&folder_id);
        }

        if indexed > 0 {
            window.emit("library-updated", ()).ok();
        }
    }

    if total_indexed > 0 {
        if let Ok(db) = state.db.lock() {
            let merged = auto_merge_duplicate_tvshows(&db, "scan_all_cloud_folders");
            if merged > 0 {
                window.emit("library-updated", ()).ok();
            }
        }
    }

    let message = format!(
        "Cloud scan complete: {} new ({} movies, {} TV shows), {} already indexed",
        total_indexed, total_movies, total_tv, total_skipped
    );

    window
        .emit(
            "cloud-scan-complete",
            serde_json::json!({
                "indexed": total_indexed,
                "movies": total_movies,
                "tv": total_tv,
                "skipped": total_skipped
            }),
        )
        .ok();

    let skipped_reasons = if all_skipped_reasons.is_empty() {
        None
    } else {
        Some(all_skipped_reasons)
    };

    Ok(CloudIndexResult {
        success: true,
        indexed_count: total_indexed,
        skipped_count: total_skipped,
        movies_count: total_movies,
        tv_count: total_tv,
        message,
        skipped_reasons,
    })
}

/// Check for new cloud files using the efficient Changes API
/// This is MUCH lighter than scanning all folders - only returns changed files
#[tauri::command]
async fn check_cloud_changes(
    state: State<'_, AppState>,
    window: WebviewWindow,
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
            skipped_reasons: None,
        });
    }

    // Get or initialize the changes token
    let current_token = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_gdrive_changes_token().map_err(|e| e.to_string())?
    };

    let page_token = match current_token {
        Some(token) => {
            println!(
                "[CLOUD CHANGES] Using existing token: {}...",
                &token[..token.len().min(20)]
            );
            token
        }
        None => {
            // First time - get the start token
            println!("[CLOUD CHANGES] No token found - initializing changes tracking...");
            let start_token = state.gdrive_client.get_changes_start_token().await?;
            println!(
                "[CLOUD CHANGES] Got start token: {}...",
                &start_token[..start_token.len().min(20)]
            );

            // Save it
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.set_gdrive_changes_token(&start_token)
                .map_err(|e| e.to_string())?;
            println!("[CLOUD CHANGES] Token saved - will detect changes on next poll");

            // Return empty result - we'll catch changes on next poll
            return Ok(CloudIndexResult {
                success: true,
                indexed_count: 0,
                skipped_count: 0,
                movies_count: 0,
                tv_count: 0,
                message: "Changes tracking initialized".to_string(),
                skipped_reasons: None,
            });
        }
    };

    // Get tracked folder IDs (only those with auto_scan enabled)
    let tracked_folders: std::collections::HashSet<String> = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let folders = db.get_cloud_folders().map_err(|e| e.to_string())?;
        folders
            .into_iter()
            .filter(|(_, _, auto_scan)| *auto_scan)
            .map(|(id, _, _)| id)
            .collect()
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
            skipped_reasons: None,
        });
    }

    println!(
        "[CLOUD CHANGES] Tracking {} folder(s)",
        tracked_folders.len()
    );

    // Get changes since last check
    let api_start = std::time::Instant::now();
    let (changed_files, removed_file_ids, new_token) =
        state.gdrive_client.get_video_changes(&page_token).await?;
    let api_duration = api_start.elapsed();
    println!("[CLOUD CHANGES] Changes API call took {:?}", api_duration);

    // Token is saved AFTER indexing completes (see end of function).
    // Saving before indexing would permanently lose failed files from detection.

    let mut removed_titles: Vec<String> = Vec::new();
    if !removed_file_ids.is_empty() {
        if let Ok(db) = state.db.lock() {
            for file_id in removed_file_ids {
                if let Ok(Some((_id, title, _media_type, _parent_id))) =
                    db.remove_media_by_cloud_file_id(&file_id)
                {
                    removed_titles.push(title);
                }
            }

            if !removed_titles.is_empty() {
                let _ = db.cleanup_empty_series();
            }
        }

        if !removed_titles.is_empty() {
            window.emit("library-updated", ()).ok();

            // Group TV episodes by series name to avoid spamming notifications
            let mut series_episodes: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();
            let mut non_tv_titles: Vec<String> = Vec::new();

            for title in &removed_titles {
                // Check if this is a TV episode (format: "Title SXXEXX" or "Title SXEX")
                // Examples: "Breaking Bad S01E01", "Breaking Bad S1E1", "Breaking Bad S10E15"
                let is_tv_episode = if let Some(pos) = title.find(" S") {
                    let rest = &title[pos + 2..]; // Skip " S"
                    if rest.len() >= 3 && rest.starts_with(|c: char| c.is_ascii_digit()) {
                        // Look for pattern: digit(s) + 'E' + digit(s)
                        if let Some(e_pos) = rest.find(|c: char| c.to_ascii_uppercase() == 'E') {
                            if e_pos > 0 && e_pos < rest.len() - 1 {
                                let season_part = &rest[..e_pos];
                                let episode_part = &rest[e_pos + 1..];
                                // Both season and episode should be numeric
                                season_part.chars().all(|c| c.is_ascii_digit())
                                    && episode_part.chars().all(|c| c.is_ascii_digit())
                                    && !season_part.is_empty()
                                    && !episode_part.is_empty()
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_tv_episode {
                    // Extract series name (everything before " S")
                    if let Some(pos) = title.find(" S") {
                        let series_name = &title[..pos];
                        series_episodes
                            .entry(series_name.to_string())
                            .or_insert_with(Vec::new)
                            .push(title.clone());
                        continue;
                    }
                }
                // Not a TV episode, add to regular titles
                non_tv_titles.push(title.clone());
            }

            let mut messages = Vec::new();

            for (series_name, episodes) in &series_episodes {
                let episode_count = episodes.len();
                if episode_count == 1 {
                    messages.push(format!("{} (1 episode)", series_name));
                } else {
                    messages.push(format!("{} ({} episodes)", series_name, episode_count));
                }
            }

            for title in &non_tv_titles {
                messages.push(title.clone());
            }

            if !messages.is_empty() {
                let message = if messages.len() == 1 {
                    format!("{} removed (deleted from Drive)", messages[0])
                } else {
                    format!("{} items removed (deleted from Drive)", messages.len())
                };

                dispatch_notification(&window, "SlasshyVault", &message, "info");
            }
        }
    }

    if changed_files.is_empty() {
        let total_duration = start_time.elapsed();
        println!(
            "[CLOUD CHANGES] No changes detected (total: {:?})",
            total_duration
        );
        println!("[CLOUD CHANGES] ══════════════════════════════════════════");
        // Save token even when no changes — the page token has moved forward
        if let Ok(db) = state.db.lock() {
            let _ = db.set_gdrive_changes_token(&new_token);
        }
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: 0,
            movies_count: 0,
            tv_count: 0,
            message: if removed_titles.is_empty() {
                "No new files detected".to_string()
            } else {
                format!(
                    "Removed {} item(s) deleted from Drive",
                    removed_titles.len()
                )
            },
            skipped_reasons: None,
        });
    }

    println!("[CLOUD CHANGES] ┌─────────────────────────────────────────");
    println!(
        "[CLOUD CHANGES] │ DETECTED {} changed video file(s)!",
        changed_files.len()
    );
    for file in &changed_files {
        println!("[CLOUD CHANGES] │   • {}", file.name);
    }
    println!("[CLOUD CHANGES] └─────────────────────────────────────────");

    // Build complete set of all descendant folder IDs under tracked folders
    // so we can detect files in subdirectories (not just immediate children)
    println!(
        "[CLOUD CHANGES] Building folder tree for {} tracked folder(s)...",
        tracked_folders.len()
    );
    let tree_build_start = std::time::Instant::now();
    let mut all_tracked_folder_ids: std::collections::HashSet<String> = tracked_folders.clone();
    for folder_id in &tracked_folders {
        match state.gdrive_client.list_all_folder_ids(folder_id).await {
            Ok(descendant_ids) => {
                all_tracked_folder_ids.extend(descendant_ids);
            }
            Err(e) => {
                println!("[CLOUD CHANGES] Warning: failed to list subfolders for {}: {}. Will only check direct parents.", folder_id, e);
            }
        }
    }
    let tree_build_duration = tree_build_start.elapsed();
    println!(
        "[CLOUD CHANGES] Folder tree built in {:?} ({} total folders)",
        tree_build_duration,
        all_tracked_folder_ids.len()
    );

    // Filter to only files in our tracked folders (including subfolders)
    let files_to_index: Vec<gdrive::DriveItem> = changed_files
        .into_iter()
        .filter(|file| {
            if let Some(ref parents) = file.parents {
                let in_tracked = parents.iter().any(|p| all_tracked_folder_ids.contains(p));
                if !in_tracked {
                    println!(
                        "[CLOUD CHANGES] Skipping {} (not in tracked folder tree)",
                        file.name
                    );
                }
                in_tracked
            } else {
                println!("[CLOUD CHANGES] Skipping {} (no parent folder)", file.name);
                false
            }
        })
        .collect();

    let unsupported_archives: Vec<String> = files_to_index
        .iter()
        .filter(|file| is_unsupported_archive_drive_item(file))
        .map(|file| file.name.clone())
        .collect();
    if !unsupported_archives.is_empty() {
        notify_unsupported_archives_window(&window, &unsupported_archives);
    }
    let files_to_index: Vec<gdrive::DriveItem> = files_to_index
        .into_iter()
        .filter(|file| !is_unsupported_archive_drive_item(file))
        .collect();

    if files_to_index.is_empty() {
        let total_duration = start_time.elapsed();
        println!(
            "[CLOUD CHANGES] No files in tracked folders (total: {:?})",
            total_duration
        );
        println!("[CLOUD CHANGES] ══════════════════════════════════════════");
        // Save token — changes were consumed even though they're outside tracked folders
        if let Ok(db) = state.db.lock() {
            let _ = db.set_gdrive_changes_token(&new_token);
        }
        let skipped_reasons: Option<Vec<String>> = if unsupported_archives.is_empty() {
            None
        } else {
            Some(
                unsupported_archives
                    .iter()
                    .map(|a| {
                        format!(
                            "{} — TAR archives are not supported (requires sequential read)",
                            a
                        )
                    })
                    .collect(),
            )
        };
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: unsupported_archives.len(),
            movies_count: 0,
            tv_count: 0,
            message: if unsupported_archives.is_empty() {
                "No new files in tracked folders".to_string()
            } else {
                format!(
                    "Skipped {} unsupported TAR archive(s) in tracked folders",
                    unsupported_archives.len()
                )
            },
            skipped_reasons,
        });
    }

    println!(
        "[CLOUD CHANGES] {} file(s) to index in tracked folders",
        files_to_index.len()
    );

    // Emit event to show indexing has started
    window
        .emit(
            "cloud-indexing-started",
            serde_json::json!({
                "count": files_to_index.len()
            }),
        )
        .ok();

    let image_cache_dir = database::get_image_cache_dir();
    std::fs::create_dir_all(&image_cache_dir).ok();

    let db_path = database::get_database_path();
    let _files_count = files_to_index.len();
    let (zip_indexing_enabled, archive_cache_config) = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        (config.zip_indexing_enabled, build_zip_cache_config(&config))
    };
    let zip_access_token = if zip_indexing_enabled && files_to_index.iter().any(is_zip_drive_item) {
        Some(state.gdrive_client.get_access_token().await?)
    } else {
        None
    };

    println!("[CLOUD CHANGES] ┌─────────────────────────────────────────");
    println!("[CLOUD CHANGES] │ PHASE 1: Adding files immediately (no metadata)");
    println!("[CLOUD CHANGES] └─────────────────────────────────────────");
    let index_start = std::time::Instant::now();

    // PHASE 1: Add files immediately without metadata
    let phase1_result = {
        let db_path_clone = db_path.clone();
        let files_to_index_clone: Vec<_> = files_to_index
            .iter()
            .map(|f| (f.id.clone(), f.name.clone(), f.parents.clone()))
            .collect();

        tokio::task::spawn_blocking(move || {
            let db = match database::Database::new(&db_path_clone) {
                Ok(d) => d,
                Err(e) => return Err(format!("Failed to open database: {}", e)),
            };

            let mut indexed_items: Vec<(
                i64,
                String,
                String,
                bool,
                Option<i32>,
                Option<i32>,
                String,
                Option<i32>,
            )> = Vec::new(); // (id, title, file_id, is_tv, season, episode, folder_id, year)
            let mut skipped_count = 0;
            let mut movies_count = 0;
            let mut tv_count = 0;
            let mut skipped_reasons: Vec<String> = Vec::new();

            // Cache for TV show IDs to avoid creating duplicates
            let mut tv_show_cache: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();

            for (file_id, file_name, parents) in files_to_index_clone {
                // Check if already indexed (by cloud_file_id OR by file_path)
                if db.cloud_file_exists(&file_id) {
                    let _ = db.clear_cloud_index_failure(&file_id);
                    println!(
                        "[CLOUD CHANGES]   ⊘ Skipping (already indexed by file_id): {}",
                        file_name
                    );
                    skipped_count += 1;
                    skipped_reasons.push(format!("{} — already indexed (cloud ID exists)", file_name));
                    continue;
                }

                // Get the parent folder ID
                let folder_id = parents
                    .as_ref()
                    .and_then(|p| p.first())
                    .cloned()
                    .unwrap_or_default();

                let pseudo_file = gdrive::DriveItem {
                    id: file_id.clone(),
                    name: file_name.clone(),
                    mime_type: if file_name.to_ascii_lowercase().ends_with(".zip") {
                        "application/zip".to_string()
                    } else {
                        String::new()
                    },
                    size: None,
                    modified_time: None,
                    parents: parents.clone(),
                    web_content_link: None,
                };

                if is_zip_drive_item(&pseudo_file) {
                    if !zip_indexing_enabled {
                        skipped_count += 1;
                        skipped_reasons.push(format!("{} — ZIP indexing disabled in settings", file_name));
                        continue;
                    }

                    let Some(access_token) = zip_access_token.as_deref() else {
                        skipped_count += 1;
                        skipped_reasons.push(format!("{} — ZIP access token unavailable", file_name));
                        continue;
                    };

                        match index_zip_archive_without_metadata(
                            &db,
                            access_token,
                            &pseudo_file,
                            &folder_id,
                            &archive_cache_config,
                            &mut tv_show_cache,
                        ) {
                        Ok(items) => {
                            if items.is_empty() {
                                let _ = db.upsert_cloud_index_failure(
                                    &file_id,
                                    &file_name,
                                    "Archive was scanned but no playable TV episode entries were identified",
                                );
                                skipped_count += 1;
                                skipped_reasons.push(format!("{} — ZIP archive contained no playable TV episodes", file_name));
                            } else {
                                let _ = db.clear_cloud_index_failure(&file_id);
                                tv_count += items.len();
                                indexed_items.extend(items);
                            }
                            continue;
                        }
                        Err(error) => {
                            println!("[ZIP] Failed to index '{}': {}", file_name, error);
                            let _ = db.upsert_cloud_index_failure(&file_id, &file_name, &error);
                            skipped_count += 1;
                            skipped_reasons.push(format!("{} — ZIP indexing error: {}", file_name, error));
                            continue;
                        }
                    }
                }

                let parsed = media_manager::parse_cloud_filename(&file_name);
                let is_tv_show = parsed.season.is_some() && parsed.episode.is_some();

                if is_tv_show {
                    let season = parsed.season.unwrap();
                    let episode = parsed.episode.unwrap();
                    let show_title = parsed.title.clone();
                    let show_title_lower = show_title.to_lowercase();

                    // Get or create TV show entry (without metadata for now)
                    let db_show_id = if let Some(&cached_id) = tv_show_cache.get(&show_title_lower)
                    {
                        println!(
                            "[CLOUD CHANGES]   Using cached show ID {} for '{}'",
                            cached_id, show_title
                        );
                        cached_id
                    } else {
                        let show_id = if let Some(existing_show) =
                            find_existing_cloud_tvshow(&db, &show_title, parsed.year)
                        {
                            println!(
                                "[CLOUD CHANGES]   Found existing show '{}' with ID {}",
                                show_title, existing_show.id
                            );
                            existing_show.id
                        } else {
                            // Create TV show entry without metadata
                            // Use a unique file_path combining folder ID and show title
                            let show_path = cloud_tvshow_path(&folder_id, &show_title);
                            println!(
                                "[CLOUD CHANGES]   Creating new TV show '{}' with path '{}'",
                                show_title, show_path
                            );
                            if let Some(existing_show) =
                                find_existing_cloud_tvshow_by_path(&db, &show_path)
                            {
                                println!(
                                    "[CLOUD CHANGES]   Reusing existing TV show with ID {}",
                                    existing_show.id
                                );
                                existing_show.id
                            } else {
                                match db.insert_cloud_tvshow(
                                    &show_title,
                                    None,
                                    None,
                                    None,
                                    None,
                                    &show_path,
                                    &folder_id,
                                    None,
                                ) {
                                    Ok(id) => {
                                        println!("[CLOUD CHANGES]   Created TV show with ID {}", id);
                                        id
                                    }
                                    Err(e) => {
                                        if let Some(existing_show) =
                                            find_existing_cloud_tvshow_by_path(&db, &show_path)
                                        {
                                            println!(
                                                "[CLOUD CHANGES]   Insert collided; reusing TV show with ID {}",
                                                existing_show.id
                                            );
                                            existing_show.id
                                        } else {
                                            println!("[CLOUD CHANGES]   ERROR creating TV show: {}", e);
                                            skipped_count += 1;
                                            skipped_reasons.push(format!("{} — database insert failed for TV show", file_name));
                                            continue;
                                        }
                                    }
                                }
                            }
                        };
                        tv_show_cache.insert(show_title_lower, show_id);
                        show_id
                    };

                    // Insert episode without metadata
                    match db.insert_cloud_episode(
                        &show_title,
                        &file_name,
                        db_show_id,
                        season,
                        episode,
                        &file_id,
                        &folder_id,
                        None,
                        None,
                        None,
                        None,
                    ) {
                        Ok(ep_id) => {
                            let display_title =
                                format!("{} S{:02}E{:02}", show_title, season, episode);
                            println!("[CLOUD CHANGES]   ✓ Added (no metadata): {}", display_title);
                            indexed_items.push((
                                ep_id,
                                show_title,
                                file_id,
                                true,
                                Some(season),
                                Some(episode),
                                folder_id,
                                parsed.year,
                            ));
                            tv_count += 1;
                        }
                        Err(e) => {
                            println!("[CLOUD CHANGES]   ERROR inserting episode: {}", e);
                            skipped_count += 1;
                            skipped_reasons.push(format!("{} — database insert failed for episode", file_name));
                            continue;
                        }
                    }
                } else {
                    // Insert movie without metadata
                    match db.insert_cloud_movie(
                        &parsed.title,
                        parsed.year,
                        None,
                        None,
                        None,
                        None,
                        &file_name,
                        &file_id,
                        &folder_id,
                        0.0,
                        None,
                    ) {
                        Ok(movie_id) => {
                            println!("[CLOUD CHANGES]   ✓ Added (no metadata): {}", parsed.title);
                            indexed_items.push((
                                movie_id,
                                parsed.title,
                                file_id,
                                false,
                                None,
                                None,
                                folder_id,
                                parsed.year,
                            ));
                            movies_count += 1;
                        }
                        Err(_) => {
                            skipped_count += 1;
                            skipped_reasons.push(format!("{} — database insert failed for movie", file_name));
                            continue;
                        }
                    }
                }
            }

            Ok((indexed_items, skipped_count, movies_count, tv_count, skipped_reasons))
        })
        .await
        .map_err(|e| format!("Task failed: {}", e))?
    };

    let phase1_result = phase1_result?;

    let (indexed_items, skipped_count, movies_count, tv_count, mut skipped_reasons) = phase1_result;

    // Add TAR archive skip reasons
    for archive_name in &unsupported_archives {
        skipped_reasons.push(format!(
            "{} — TAR archives are not supported (requires sequential read)",
            archive_name
        ));
    }
    let skipped_count = skipped_count + unsupported_archives.len();
    let indexed_count = indexed_items.len();
    let phase1_duration = index_start.elapsed();

    println!(
        "[CLOUD CHANGES] Phase 1 took {:?} - {} file(s) added",
        phase1_duration, indexed_count
    );

    // Send notifications and emit events immediately after Phase 1
    if indexed_count > 0 {
        // Collect titles for notifications
        let titles: Vec<String> = indexed_items
            .iter()
            .map(|(_, title, _, is_tv, season, episode, _, _)| {
                if *is_tv {
                    format!(
                        "{} S{:02}E{:02}",
                        title,
                        season.unwrap_or(1),
                        episode.unwrap_or(1)
                    )
                } else {
                    title.clone()
                }
            })
            .collect();

        println!("[CLOUD CHANGES] ┌─────────────────────────────────────────");
        println!("[CLOUD CHANGES] │ ADDED TO LIBRARY:");
        for title in &titles {
            println!("[CLOUD CHANGES] │   ✓ {}", title);
        }
        println!("[CLOUD CHANGES] └─────────────────────────────────────────");

        // Emit library-updated so UI refreshes immediately
        window.emit("library-updated", ()).ok();

        // Send WebviewWindows notification for each item (simple format)
        for title in &titles {
            dispatch_notification(
                &window,
                "SlasshyVault",
                &format!("{} added to your library", title),
                "success",
            );
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

                    for (media_id, title, _file_id, is_tv, season_opt, episode_opt, _folder_id, year) in indexed_items_bg {
                        if is_tv {
                            let season = season_opt.unwrap_or(1);
                            let episode = episode_opt.unwrap_or(1);
                            let title_lower = title.to_lowercase();

                            println!("[CLOUD CHANGES BG] Processing {} S{:02}E{:02}...", title, season, episode);

                            // Get or fetch TV show metadata
                            let show_meta = if let Some(cached) = tv_metadata_cache.get(&title_lower) {
                                cached.clone()
                            } else {
                                println!("[CLOUD CHANGES BG]   Searching metadata for show '{}'...", title);
                                let meta = tmdb::search_metadata_with_fallback(&api_key, &title, "tv", year, &image_cache_dir_bg).ok().flatten();
                                if meta.is_some() {
                                    println!("[CLOUD CHANGES BG]   ✓ Found show metadata");
                                } else {
                                    println!("[CLOUD CHANGES BG]   ✗ Show metadata not found");
                                }
                                tv_metadata_cache.insert(title_lower.clone(), meta.clone());
                                meta
                            };

                            if let Some(ref meta) = show_meta {
                                // Update the parent TV show with poster (only once per show)
                                if !tv_show_updated.contains(&title_lower) {
                                    if let Some(show) = find_existing_cloud_tvshow(&db, &title, None) {
                                        let mut show_meta_to_apply = meta.clone();
                                        show_meta_to_apply.title = media_manager::prefer_title_with_leading_article(&title, &show_meta_to_apply.title);
                                        if db.update_metadata(show.id, &show_meta_to_apply).is_ok() {
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
                            println!("[CLOUD CHANGES BG] Processing movie '{}'...", title);
                            match tmdb::search_metadata_with_fallback(&api_key, &title, "movie", year, &image_cache_dir_bg) {
                                Ok(Some(meta)) => {
                                    let mut movie_meta = meta;
                                    movie_meta.title = media_manager::prefer_title_with_leading_article(&title, &movie_meta.title);
                                    if db.update_metadata(media_id, &movie_meta).is_ok() {
                                        println!("[CLOUD CHANGES BG]   ✓ Updated movie metadata: {}", movie_meta.title);
                                    } else {
                                        println!("[CLOUD CHANGES BG]   ✗ Failed to update movie in DB");
                                    }
                                }
                                Ok(None) => {
                                    println!("[CLOUD CHANGES BG]   ✗ Movie metadata not found");
                                }
                                Err(e) => {
                                    println!("[CLOUD CHANGES BG]   ✗ TMDB search error: {}", e);
                                }
                            }
                        }
                    }
                }).await;

                let metadata_duration = metadata_start.elapsed();
                println!(
                    "[CLOUD CHANGES BG] Metadata fetch completed in {:?}",
                    metadata_duration
                );

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

    if indexed_count > 0 {
        if let Ok(db) = state.db.lock() {
            let merged = auto_merge_duplicate_tvshows(&db, "check_cloud_changes");
            if merged > 0 {
                window.emit("library-updated", ()).ok();
            }
        }
    }

    let total_duration = start_time.elapsed();
    let message = if indexed_count > 0 {
        format!(
            "Indexed {} new files ({} movies, {} TV)",
            indexed_count, movies_count, tv_count
        )
    } else {
        "No new files to index".to_string()
    };

    println!("[CLOUD CHANGES] ══════════════════════════════════════════");
    println!(
        "[CLOUD CHANGES] SUMMARY: {} indexed, {} skipped",
        indexed_count, skipped_count
    );
    println!("[CLOUD CHANGES] Total time: {:?}", total_duration);
    println!("[CLOUD CHANGES] ══════════════════════════════════════════");

    // Save token now that indexing has completed successfully.
    // WARNING: If the app crashes between the save and the return,
    // the next poll will resume from after these changes, which is correct.
    if let Ok(db) = state.db.lock() {
        let _ = db.set_gdrive_changes_token(&new_token);
    }

    let final_skipped_reasons = if skipped_reasons.is_empty() {
        None
    } else {
        Some(skipped_reasons)
    };

    Ok(CloudIndexResult {
        success: indexed_count > 0 || skipped_count == 0,
        indexed_count,
        skipped_count,
        movies_count,
        tv_count,
        message,
        skipped_reasons: final_skipped_reasons,
    })
}

// Restart the app
#[tauri::command]
fn restart_app() -> Result<(), String> {
    println!("[RESTART] Restarting app...");
    let exe_path = std::env::current_exe().map_err(|e| format!("Failed to get exe path: {}", e))?;
    std::process::Command::new(&exe_path)
        .spawn()
        .map_err(|e| format!("Failed to spawn new process: {}", e))?;
    std::process::exit(0);
}

// Clear all app data (reset to new state)
#[tauri::command]
async fn clear_all_app_data(
    state: State<'_, AppState>,
    confirmed: bool,
) -> Result<ApiResponse, String> {
    if !confirmed {
        return Err("Operation cancelled by user".to_string());
    }
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
        std::fs::create_dir_all(cache_path).ok();
    }

    // Delete zip cache directory
    let zip_cache_path = database::get_zip_cache_dir();
    let zip_path = std::path::Path::new(&zip_cache_path);
    if zip_path.exists() {
        std::fs::remove_dir_all(zip_path).ok();
        println!("[RESET] Zip cache deleted successfully");
    }

    // Delete Google Drive tokens file
    let tokens_dir =
        std::path::Path::new(&database::get_app_data_dir().to_string_lossy().to_string())
            .join("gdrive_tokens.json");
    if tokens_dir.exists() {
        std::fs::remove_file(&tokens_dir).ok();
        println!("[RESET] Google Drive tokens deleted");
    }

    // Delete config file (will be recreated as default on next launch)
    let config_path = database::get_config_path();
    let config_path = std::path::Path::new(&config_path);
    if config_path.exists() {
        std::fs::remove_file(config_path).ok();
        println!("[RESET] Config file deleted (will recreate with defaults on restart)");
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
async fn cleanup_missing_metadata(state: State<'_, AppState>) -> Result<CleanupResponse, String> {
    println!("[CLEANUP] Starting cleanup of missing media metadata...");

    let removed_count = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let image_cache_path = database::get_image_cache_dir();
        media_manager::cleanup_orphaned_media(&db, &image_cache_path)
    };

    let message = if removed_count > 0 {
        format!(
            "Cleaned up {} orphaned entries and their posters",
            removed_count
        )
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

// Response for delete operation
#[derive(serde::Serialize)]
struct DeleteResponse {
    success: bool,
    deleted_count: usize,
    failed_count: usize,
    message: String,
}

// Delete media files permanently from disk (bypasses recycle bin)
// Also handles cloud files by deleting from Google Drive
#[tauri::command]
async fn delete_media_files(
    state: State<'_, AppState>,
    window: WebviewWindow,
    media_ids: Vec<i64>,
    confirmed: bool,
) -> Result<DeleteResponse, String> {
    if !confirmed {
        return Err("Operation cancelled by user".to_string());
    }
    if media_ids.is_empty() {
        return Err("No media IDs provided".to_string());
    }

    println!(
        "[DELETE] Starting permanent deletion for {} items",
        media_ids.len()
    );

    // Get media info including cloud details
    let (media_info, parent_series_ids) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let info = db
            .get_media_delete_info(&media_ids)
            .map_err(|e| e.to_string())?;
        let parents = db
            .get_parent_series_ids(&media_ids)
            .map_err(|e| e.to_string())?;
        (info, parents)
    };

    let mut deleted_count = 0;
    let mut deleted_zip_archives = 0;
    let mut failed_count = 0;
    let mut deleted_file_paths: Vec<String> = Vec::new();
    let mut cloud_file_ids_to_delete: HashSet<String> = HashSet::new();
    let mut zip_archive_ids_to_delete: HashSet<String> = HashSet::new();
    let mut db_media_ids_to_delete: Vec<i64> = Vec::new();

    // Separate cloud and local files
    for (id, file_path, is_cloud, cloud_file_id, _cloud_folder_id, parent_zip_id, ddl_source_id) in
        media_info
    {
        if is_cloud {
            if ddl_source_id.is_some() {
                // DDL item - no cloud file to delete on Google Drive, just remove from DB
                db_media_ids_to_delete.push(id);
                deleted_count += 1;
                continue;
            }

            if let Some(zip_file_id) = parent_zip_id {
                println!(
                    "[DELETE] Queuing ZIP archive for deletion via representative item {}: {}",
                    id, zip_file_id
                );
                zip_archive_ids_to_delete.insert(zip_file_id);
                continue;
            }

            // Cloud file - queue for Google Drive deletion
            if let Some(cloud_id) = cloud_file_id {
                println!(
                    "[DELETE] Queuing cloud file for deletion: {} (cloud_file_id: {})",
                    file_path.as_deref().unwrap_or("unknown"),
                    cloud_id
                );
                cloud_file_ids_to_delete.insert(cloud_id);
                db_media_ids_to_delete.push(id);
            }
        } else {
            // Local file - delete from disk (with path canonicalization)
            if let Some(path_str) = file_path {
                let raw_path = std::path::Path::new(&path_str);
                let canonical = raw_path
                    .canonicalize()
                    .unwrap_or_else(|_| raw_path.to_path_buf());
                if canonical.exists() {
                    match std::fs::remove_file(&canonical) {
                        Ok(_) => {
                            println!("[DELETE] Successfully deleted local file: {}", path_str);
                            deleted_file_paths.push(path_str);
                            deleted_count += 1;
                        }
                        Err(e) => {
                            println!("[DELETE] Failed to delete {}: {}", path_str, e);
                            failed_count += 1;
                        }
                    }
                } else {
                    println!(
                        "[DELETE] Local file not found (already deleted?): {}",
                        path_str
                    );
                    deleted_file_paths.push(path_str);
                    deleted_count += 1;
                }
            }
            db_media_ids_to_delete.push(id);
        }
    }

    if !zip_archive_ids_to_delete.is_empty() {
        println!(
            "[DELETE] Deleting {} ZIP archive(s) from Google Drive",
            zip_archive_ids_to_delete.len()
        );
        for zip_file_id in zip_archive_ids_to_delete {
            match state.gdrive_client.delete_file(&zip_file_id).await {
                Ok(_) => {
                    let archive_summary = {
                        let db = state.db.lock().map_err(|e| e.to_string())?;
                        db.remove_media_by_cloud_file_id(&zip_file_id)
                            .map_err(|e| e.to_string())?
                    };

                    if let Some((_, title, media_type, _)) = archive_summary {
                        println!("[DELETE] Successfully deleted {}: {}", media_type, title);
                    } else {
                        println!("[DELETE] Successfully deleted ZIP archive: {}", zip_file_id);
                    }

                    deleted_count += 1;
                    deleted_zip_archives += 1;
                }
                Err(e) => {
                    println!(
                        "[DELETE] Failed to delete ZIP archive {}: {}",
                        zip_file_id, e
                    );
                    failed_count += 1;
                }
            }
        }
    }

    // Delete cloud files from Google Drive
    if !cloud_file_ids_to_delete.is_empty() {
        println!(
            "[DELETE] Deleting {} cloud files from Google Drive",
            cloud_file_ids_to_delete.len()
        );
        let delete_futures: Vec<_> = cloud_file_ids_to_delete
            .iter()
            .map(|cloud_file_id| state.gdrive_client.delete_file(cloud_file_id))
            .collect();
        let results = futures_util::future::join_all(delete_futures).await;
        for (cloud_file_id, result) in cloud_file_ids_to_delete.iter().zip(results) {
            match result {
                Ok(_) => {
                    println!(
                        "[DELETE] Successfully deleted cloud file: {}",
                        cloud_file_id
                    );
                    deleted_count += 1;
                }
                Err(e) => {
                    let is_permission_error =
                        e.contains("403") || e.contains("insufficientFilePermissions");
                    if is_permission_error {
                        println!(
                            "[DELETE] Permission denied for cloud file {} (insufficient permissions). Removing from library only.",
                            cloud_file_id
                        );
                        deleted_count += 1;
                    } else {
                        println!(
                            "[DELETE] Failed to delete cloud file {}: {}",
                            cloud_file_id, e
                        );
                        failed_count += 1;
                    }
                }
            }
        }
    }

    // Delete from database
    if !db_media_ids_to_delete.is_empty() {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.delete_media_entries(&db_media_ids_to_delete)
            .map_err(|e| e.to_string())?;
    }

    // Cleanup empty series if we deleted the last episode(s)
    let mut cleaned_empty_series = false;
    if !parent_series_ids.is_empty() {
        let series_cleanup: Vec<(i64, String, bool, Option<String>, bool)> = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let mut list = Vec::new();
            for series_id in parent_series_ids {
                let has_episodes = db
                    .series_has_episodes(series_id)
                    .map_err(|e| e.to_string())?;
                if has_episodes {
                    continue;
                }

                let media = db.get_media_by_id(series_id).map_err(|e| e.to_string())?;
                let (is_cloud, folder_id) = db
                    .get_series_cloud_info(series_id)
                    .map_err(|e| e.to_string())?;
                let is_tracked_folder = if let Some(ref folder) = folder_id {
                    db.get_cloud_folders()
                        .map(|folders| folders.iter().any(|(id, _, _)| id == folder))
                        .unwrap_or(false)
                } else {
                    false
                };

                list.push((
                    series_id,
                    media.title,
                    is_cloud,
                    folder_id,
                    is_tracked_folder,
                ));
            }
            list
        };

        for (series_id, title, is_cloud, folder_id, is_tracked_folder) in series_cleanup {
            if is_cloud {
                if let Some(folder_id) = folder_id {
                    if is_tracked_folder {
                        println!(
                            "[DELETE] SAFETY: Refusing to delete tracked root folder: {}",
                            folder_id
                        );
                    } else {
                        println!(
                            "[DELETE] Skipping cloud folder delete for empty series '{}': {}",
                            title, folder_id
                        );
                    }
                }
            }

            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.remove_media(series_id).map_err(|e| e.to_string())?;
            cleaned_empty_series = true;
        }
    }

    if cleaned_empty_series {
        window.emit("library-updated", ()).ok();
    }

    // Clean up empty parent directories (only for local files)
    media_manager::cleanup_empty_parent_dirs(&deleted_file_paths);

    let message =
        if failed_count == 0 && deleted_zip_archives > 0 && deleted_count == deleted_zip_archives {
            format!(
                "Successfully deleted {} ZIP archive(s)",
                deleted_zip_archives
            )
        } else if failed_count == 0 && deleted_zip_archives > 0 {
            format!(
                "Successfully deleted {} item(s), including {} ZIP archive(s)",
                deleted_count, deleted_zip_archives
            )
        } else if failed_count == 0 {
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

// Delete ALL media files — cloud files from Drive + all media entries from DB
// Preserves watch history, streaming history, settings, reminders, watchlist
#[tauri::command]
async fn delete_all_media_files(
    state: State<'_, AppState>,
    window: WebviewWindow,
    confirmed: bool,
) -> Result<DeleteResponse, String> {
    if !confirmed {
        return Err("Operation cancelled by user".to_string());
    }

    println!("[DELETE-ALL] Starting delete all media files");

    // Get cloud file IDs and clear DB entries
    let (cloud_file_ids, image_cache_path) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.delete_all_media_entries().map_err(|e| e.to_string())?
    };

    let total_cloud = cloud_file_ids.len();
    let mut deleted_count = 0usize;
    let mut failed_count = 0usize;

    // Delete cloud files from Google Drive
    if !cloud_file_ids.is_empty() {
        println!(
            "[DELETE-ALL] Deleting {} cloud files from Google Drive",
            total_cloud
        );
        let delete_futures: Vec<_> = cloud_file_ids
            .iter()
            .map(|cloud_file_id| state.gdrive_client.delete_file(cloud_file_id))
            .collect();
        let results = futures_util::future::join_all(delete_futures).await;
        for (cloud_file_id, result) in cloud_file_ids.iter().zip(results) {
            match result {
                Ok(_) => {
                    deleted_count += 1;
                }
                Err(e) => {
                    let is_permission_error =
                        e.contains("403") || e.contains("insufficientFilePermissions");
                    if is_permission_error {
                        println!(
                            "[DELETE-ALL] Permission denied for {} (removing from DB anyway)",
                            cloud_file_id
                        );
                        deleted_count += 1;
                    } else {
                        println!("[DELETE-ALL] Failed to delete {}: {}", cloud_file_id, e);
                        failed_count += 1;
                    }
                }
            }
        }
    }

    // Clean up image cache directory
    let cache_path = std::path::Path::new(&image_cache_path);
    if cache_path.exists() {
        match std::fs::remove_dir_all(cache_path) {
            Ok(_) => {
                let _ = std::fs::create_dir_all(cache_path);
                println!("[DELETE-ALL] Cleared image cache: {}", image_cache_path);
            }
            Err(e) => println!("[DELETE-ALL] Failed to clear image cache: {}", e),
        }
    }

    // Notify frontend
    window.emit("library-updated", ()).ok();

    let message = if failed_count == 0 {
        format!(
            "Deleted all media files. {} cloud file(s) removed from Drive.",
            deleted_count
        )
    } else {
        format!(
            "Deleted all media. {} cloud file(s) removed, {} failed.",
            deleted_count, failed_count
        )
    };

    println!("[DELETE-ALL] Complete: {}", message);

    Ok(DeleteResponse {
        success: failed_count == 0,
        deleted_count,
        failed_count,
        message,
    })
}

// Delete cloud files by Google Drive file IDs (for selective delete)
// Deletes from Drive + removes matching DB entries. Works without needing media IDs.
#[tauri::command]
async fn delete_cloud_files_by_drive_ids(
    state: State<'_, AppState>,
    window: WebviewWindow,
    drive_file_ids: Vec<String>,
    confirmed: bool,
) -> Result<DeleteResponse, String> {
    if !confirmed {
        return Err("Operation cancelled by user".to_string());
    }
    if drive_file_ids.is_empty() {
        return Err("No files selected".to_string());
    }

    println!(
        "[DELETE-SELECTIVE] Starting deletion of {} Drive files",
        drive_file_ids.len()
    );

    let mut deleted_count = 0usize;
    let mut failed_count = 0usize;

    // Delete each file from Google Drive
    let delete_futures: Vec<_> = drive_file_ids
        .iter()
        .map(|drive_file_id| state.gdrive_client.delete_file(drive_file_id))
        .collect();
    let results = futures_util::future::join_all(delete_futures).await;
    for (drive_file_id, result) in drive_file_ids.iter().zip(results) {
        match result {
            Ok(_) => {
                deleted_count += 1;
            }
            Err(e) => {
                let is_permission_error =
                    e.contains("403") || e.contains("insufficientFilePermissions");
                if is_permission_error {
                    println!(
                        "[DELETE-SELECTIVE] Permission denied for {} (removing anyway)",
                        drive_file_id
                    );
                    deleted_count += 1;
                } else {
                    println!(
                        "[DELETE-SELECTIVE] Failed to delete {}: {}",
                        drive_file_id, e
                    );
                    failed_count += 1;
                }
            }
        }
    }

    // Remove matching entries from DB (non-fatal — Drive deletion already succeeded)
    match state.db.lock() {
        Ok(db) => {
            if let Err(e) = db.delete_media_by_cloud_file_ids(&drive_file_ids) {
                println!(
                    "[DELETE-SELECTIVE] DB cleanup failed (files already deleted from Drive): {}",
                    e
                );
            }
        }
        Err(e) => {
            println!("[DELETE-SELECTIVE] Could not acquire DB lock for cleanup (will be cleaned by next scan): {}", e);
        }
    }

    // Notify frontend
    window.emit("library-updated", ()).ok();

    let message = if failed_count == 0 {
        format!(
            "Deleted {} file(s) from Google Drive and library",
            deleted_count
        )
    } else {
        format!("Deleted {} file(s), {} failed", deleted_count, failed_count)
    };

    println!("[DELETE-SELECTIVE] Complete: {}", message);

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
    episode_title: Option<String>,
    season_number: Option<i32>,
    episode_number: Option<i32>,
    file_path: Option<String>,
    parent_zip_id: Option<String>,
    delete_kind: String,
    archive_episode_count: Option<i32>,
    file_size_bytes: Option<i64>,
}

// Get episodes for a TV show for delete selection
#[tauri::command]
async fn get_episodes_for_delete(
    state: State<'_, AppState>,
    series_id: i64,
) -> Result<Vec<EpisodeDeleteInfo>, String> {
    let episodes = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_episodes(series_id).map_err(|e| e.to_string())?
    };

    let mut zip_archive_details: HashMap<String, (String, i32, Option<i64>)> = HashMap::new();
    for episode in &episodes {
        if let Some(zip_file_id) = episode.parent_zip_id.as_deref() {
            let archive_info = zip_archive_details
                .entry(zip_file_id.to_string())
                .or_insert_with(|| {
                    // Try zip_archives table first, then ddl_sources table for DDL items
                    state
                        .db
                        .lock()
                        .ok()
                        .and_then(|db| {
                            db.get_zip_archive(zip_file_id)
                                .ok()
                                .map(|a| (a.filename, 0, Some(a.file_size_bytes)))
                                .or_else(|| {
                                    db.get_ddl_source(zip_file_id)
                                        .ok()
                                        .map(|s| (s.filename, 0, None))
                                })
                        })
                        .unwrap_or_else(|| ("ZIP archive".to_string(), 0, None))
                });
            archive_info.1 += 1;
        }
    }

    let mut seen_zip_archives = HashSet::new();
    let mut result = Vec::new();

    for episode in episodes {
        if let Some(zip_file_id) = episode.parent_zip_id.clone() {
            if episode.ddl_source_id.is_some() {
                // DDL episode — show individually (not grouped), each can be deleted on its own
                result.push(EpisodeDeleteInfo {
                    id: episode.id,
                    title: episode.title,
                    episode_title: episode.episode_title,
                    season_number: episode.season_number,
                    episode_number: episode.episode_number,
                    file_path: Some("Direct-link item".to_string()),
                    parent_zip_id: Some(zip_file_id),
                    delete_kind: "ddl_source".to_string(),
                    archive_episode_count: None,
                    file_size_bytes: None,
                });
                continue;
            }

            if seen_zip_archives.insert(zip_file_id.clone()) {
                let (archive_name, archive_episode_count, file_size_bytes) = zip_archive_details
                    .get(&zip_file_id)
                    .cloned()
                    .unwrap_or_else(|| ("ZIP archive".to_string(), 1, None));
                let archive_suffix = if archive_episode_count == 1 {
                    "episode"
                } else {
                    "episodes"
                };

                result.push(EpisodeDeleteInfo {
                    id: episode.id,
                    title: archive_name,
                    episode_title: None,
                    season_number: None,
                    episode_number: None,
                    file_path: Some(format!(
                        "Deletes the ZIP archive from Google Drive and removes {} indexed {}.",
                        archive_episode_count, archive_suffix
                    )),
                    parent_zip_id: Some(zip_file_id),
                    delete_kind: "zip_archive".to_string(),
                    archive_episode_count: Some(archive_episode_count),
                    file_size_bytes,
                });
            }

            continue;
        }

        let file_path = episode.file_path.clone();
        let mut file_size_bytes = episode.file_size_bytes.or_else(|| {
            if !episode.is_cloud.unwrap_or(false) {
                // Try to get file size from file system for local files
                file_path
                    .as_ref()
                    .and_then(|path| match std::fs::metadata(path) {
                        Ok(metadata) => Some(metadata.len() as i64),
                        Err(e) => {
                            eprintln!("[DEBUG] Failed to get file size for {}: {}", path, e);
                            None
                        }
                    })
            } else {
                None
            }
        });

        if file_size_bytes.is_none() {
            if let Some(cloud_file_id) = episode.cloud_file_id.as_deref() {
                if let Ok(metadata) = state.gdrive_client.get_file_metadata(cloud_file_id).await {
                    file_size_bytes = metadata
                        .size
                        .as_deref()
                        .and_then(|value| value.parse::<i64>().ok());

                    if let Some(size) = file_size_bytes {
                        if let Ok(db) = state.db.lock() {
                            let _ = db.update_file_size(episode.id, size);
                        }
                    }
                }
            }
        }

        if file_size_bytes.is_none() {
            eprintln!("[DEBUG] No file size for episode: {} (path: {:?}, is_cloud: {:?}, cloud_file_id: {:?})", 
                episode.title, episode.file_path, episode.is_cloud, episode.cloud_file_id);
        }

        result.push(EpisodeDeleteInfo {
            id: episode.id,
            title: episode.title,
            episode_title: episode.episode_title,
            season_number: episode.season_number,
            episode_number: episode.episode_number,
            file_path,
            parent_zip_id: None,
            delete_kind: "episode".to_string(),
            archive_episode_count: None,
            file_size_bytes,
        });
    }

    Ok(result)
}

// Delete a TV show series and optionally all its episodes
#[tauri::command]
async fn delete_series(
    state: State<'_, AppState>,
    series_id: i64,
    delete_files: bool,
    confirmed: bool,
) -> Result<DeleteResponse, String> {
    if !confirmed {
        return Err("Operation cancelled by user".to_string());
    }
    println!(
        "[DELETE] Deleting series ID {} (delete_files: {})",
        series_id, delete_files
    );

    // Get series cloud info first
    let (is_cloud_series, cloud_folder_id) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_series_cloud_info(series_id)
            .map_err(|e| e.to_string())?
    };

    println!(
        "[DELETE] Series is_cloud: {}, cloud_folder_id: {:?}",
        is_cloud_series, cloud_folder_id
    );

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
            db.get_media_delete_info(&episode_ids)
                .map_err(|e| e.to_string())?
        };

        if delete_files {
            // Handle cloud episode files
            let mut cloud_file_ids: Vec<String> = Vec::new();
            let mut local_file_paths: Vec<String> = Vec::new();

            for (
                _id,
                file_path,
                is_cloud,
                cloud_file_id,
                _cloud_folder_id,
                _parent_zip_id,
                ddl_source_id,
            ) in episode_info
            {
                if is_cloud && ddl_source_id.is_none() {
                    if let Some(cloud_id) = cloud_file_id {
                        cloud_file_ids.push(cloud_id);
                    }
                } else if !is_cloud && file_path.is_some() {
                    if let Some(path) = file_path {
                        local_file_paths.push(path);
                    }
                }
            }

            // Delete cloud files from Google Drive
            if !cloud_file_ids.is_empty() {
                println!(
                    "[DELETE] Deleting {} cloud episode files from Google Drive",
                    cloud_file_ids.len()
                );
                for cloud_file_id in cloud_file_ids {
                    match state.gdrive_client.delete_file(&cloud_file_id).await {
                        Ok(_) => {
                            println!("[DELETE] Deleted cloud episode: {}", cloud_file_id);
                            total_deleted += 1;
                        }
                        Err(e) => {
                            println!(
                                "[DELETE] Failed to delete cloud episode {}: {}",
                                cloud_file_id, e
                            );
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
            db.delete_media_entries(&episode_ids)
                .map_err(|e| e.to_string())?;
        }
    }

    // Clean up empty parent directories (only for local files)
    if delete_files && !deleted_file_paths.is_empty() {
        media_manager::cleanup_empty_parent_dirs(&deleted_file_paths);
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

// Remove a TV series from the database and delete only matching cloud files
#[tauri::command]
async fn delete_series_cloud_folder(
    state: State<'_, AppState>,
    series_id: i64,
) -> Result<ApiResponse, String> {
    // Get series info including cloud folder ID
    let (series_title, is_cloud, cloud_folder_id) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let media = db.get_media_by_id(series_id).map_err(|e| e.to_string())?;
        let (is_cloud, folder_id) = db
            .get_series_cloud_info(series_id)
            .map_err(|e| e.to_string())?;
        (media.title, is_cloud, folder_id)
    };

    println!(
        "[DELETE] Removing series '{}' (ID: {}) - is_cloud: {}, folder_id: {:?}",
        series_title, series_id, is_cloud, cloud_folder_id
    );

    // Never delete parent folders from Drive; only delete matching files
    if is_cloud {
        let episode_ids: Vec<i64> = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let episodes = db.get_episodes(series_id).map_err(|e| e.to_string())?;
            episodes.into_iter().map(|ep| ep.id).collect()
        };

        if !episode_ids.is_empty() {
            let episode_info = {
                let db = state.db.lock().map_err(|e| e.to_string())?;
                db.get_media_delete_info(&episode_ids)
                    .map_err(|e| e.to_string())?
            };

            for (
                _id,
                _file_path,
                _is_cloud,
                cloud_file_id,
                _cloud_folder_id,
                _parent_zip_id,
                ddl_source_id,
            ) in episode_info
            {
                if let Some(cloud_id) = cloud_file_id {
                    if ddl_source_id.is_none() {
                        println!(
                            "[DELETE] Deleting cloud file for series '{}': {}",
                            series_title, cloud_id
                        );
                        if let Err(e) = state.gdrive_client.delete_file(&cloud_id).await {
                            println!(
                                "[DELETE] Warning: Failed to delete cloud file {}: {}",
                                cloud_id, e
                            );
                        }
                    }
                }
            }
        }
    }

    // Delete all episodes from database first
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.remove_series_episodes(series_id)
            .map_err(|e| e.to_string())?;
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

// Auto-detect MPV executable on the system
#[tauri::command]
async fn auto_detect_mpv(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let result = config::auto_detect_mpv(&mut config);
    Ok(result)
}

// Get info about bundled MPV (exists, path)
#[derive(Serialize)]
struct BundledMpvInfo {
    exists: bool,
    path: String,
}

#[tauri::command]
async fn get_bundled_mpv_info() -> BundledMpvInfo {
    BundledMpvInfo {
        exists: config::bundled_mpv_exists(),
        path: config::get_bundled_mpv_path().to_string_lossy().to_string(),
    }
}

// Download bundled MPV RAR from GitHub repo, extract it, and set the path
#[tauri::command]
async fn download_bundled_mpv(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> Result<String, String> {
    if cfg!(target_os = "linux") {
        let _ = (window, state);
        return Err(
            "Install MPV with your Linux package manager, then use Auto-detect or choose its path."
                .to_string(),
        );
    }

    use futures_util::StreamExt;
    use std::io::Write;

    let url = config::get_bundled_mpv_download_url();
    println!("[MPV-BUNDLED] Downloading MPV archive from: {}", url);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("Failed to build download client: {}", e))?;

    let response = client
        .get(&url)
        .header("User-Agent", "SlasshyVault-MPV-Updater")
        .send()
        .await
        .map_err(|e| format!("Failed to download bundled MPV: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Failed to download MPV: HTTP {}. Make sure the file exists at: {}",
            status,
            config::get_bundled_mpv_download_url()
        ));
    }

    let total_size = response.content_length().unwrap_or(0);
    println!("[MPV-BUNDLED] File size: {} bytes", total_size);

    // Remove existing bundled MPV before writing new one
    let _ = config::remove_bundled_mpv();

    // Ensure the bundled MPV directory exists
    let mpv_dir = config::get_bundled_mpv_dir();
    std::fs::create_dir_all(&mpv_dir)
        .map_err(|e| format!("Failed to create bundled MPV directory: {}", e))?;

    // Download to a temp RAR file
    let rar_path = config::get_bundled_mpv_rar_temp_path();
    let mut file = std::fs::File::create(&rar_path)
        .map_err(|e| format!("Failed to create temp RAR file: {}", e))?;

    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("Download error: {}", e))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Write error: {}", e))?;
        downloaded += chunk.len() as u64;

        if total_size > 0 {
            let progress = (downloaded as f64 / total_size as f64) * 100.0;
            window
                .emit(
                    "mpv-download-progress",
                    serde_json::json!({
                        "downloaded": downloaded,
                        "total": total_size,
                        "progress": progress
                    }),
                )
                .ok();
        }
    }

    println!("[MPV-BUNDLED] Download complete: {:?}", rar_path);
    println!("[MPV-BUNDLED] Extracting RAR archive...");

    // Extract the RAR using unrar crate
    let rar_path_str = rar_path.to_string_lossy().to_string();
    let extract_dest = mpv_dir.to_string_lossy().to_string();

    let _result =
        tokio::task::spawn_blocking(move || extract_rar_to_dir(&rar_path_str, &extract_dest))
            .await
            .map_err(|e| format!("Extraction task failed: {}", e))?
            .map_err(|e| format!("Failed to extract MPV archive: {}", e))?;

    // Clean up the RAR file after extraction
    let _ = std::fs::remove_file(&rar_path);

    println!("[MPV-BUNDLED] Extraction complete. Unblocking files...");

    // Unblock all extracted files to prevent WebviewWindows UAC/SmartScreen prompts
    #[cfg(windows)]
    {
        use std::process::Command;
        let mpv_dir_str = mpv_dir.to_string_lossy().to_string();
        let _ = Command::new("powershell")
            .env("MPV_UNBLOCK_DIR", mpv_dir_str)
            .args([
                "-Command",
                "Get-ChildItem -Recurse -LiteralPath $env:MPV_UNBLOCK_DIR | Unblock-File -ErrorAction SilentlyContinue",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    println!("[MPV-BUNDLED] Finding mpv.exe...");

    // Find the extracted mpv.exe
    let mpv_path = config::get_bundled_mpv_path();
    if !mpv_path.exists() {
        return Err(format!(
            "Extraction completed but mpv.exe was not found inside the archive."
        ));
    }

    let path_str = mpv_path.to_string_lossy().to_string();
    println!("[MPV-BUNDLED] Found MPV at: {}", path_str);

    // Save to config
    {
        let mut config = state.config.lock().map_err(|e| e.to_string())?;
        config.mpv_path = Some(path_str.clone());
        config::save_config(&config).map_err(|e| format!("Failed to save config: {}", e))?;
    }

    Ok(path_str)
}

/// Extract a RAR archive to a destination directory using the unrar crate
fn extract_rar_to_dir(rar_path: &str, dest_dir: &str) -> Result<(), String> {
    use std::path::Path;
    use unrar::Archive as RarArchive;

    let dest = Path::new(dest_dir);
    std::fs::create_dir_all(dest)
        .map_err(|e| format!("Failed to create extraction directory: {}", e))?;

    let mut archive = RarArchive::new(rar_path)
        .open_for_processing()
        .map_err(|e| format!("Failed to open RAR archive: {}", e))?;

    while let Some(header) = archive.read_header().map_err(|e| e.to_string())? {
        let entry_path = header.entry().filename.to_string_lossy().to_string();
        let sanitized = entry_path
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string();

        let output_path = dest.join(&sanitized);

        if header.entry().is_directory() {
            std::fs::create_dir_all(&output_path)
                .map_err(|e| format!("Failed to create directory '{}': {}", sanitized, e))?;
            archive = header.skip().map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!("Failed to create parent dir for '{}': {}", sanitized, e)
                })?;
            }
            archive = header.extract_to(&output_path).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
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
    _app_handle: AppHandle,
    state: State<'_, AppState>,
    new_config: config::Config,
    confirmed: bool,
) -> Result<ApiResponse, String> {
    if !confirmed {
        return Err("Operation cancelled by user".to_string());
    }
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let mut merged = new_config.clone();
    // Always preserve addon_sources from backend (Settings form doesn't manage them)
    if merged.addon_sources.is_empty() && !config.addon_sources.is_empty() {
        merged.addon_sources = config.addon_sources.clone();
    }
    // Derive addon_url from the default source — never trust the form's stale addon_url
    merged.addon_url = merged
        .addon_sources
        .iter()
        .find(|s| s.is_default && s.enabled)
        .map(|s| s.url.clone());
    *config = merged.clone();
    config::save_config(&merged).map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: "Configuration saved.".to_string(),
    })
}

// Get recent backend logs for developer console
#[tauri::command]
async fn get_recent_logs() -> Result<Vec<String>, String> {
    Ok(log_buffer::drain())
}

// Clear all backend logs
#[tauri::command]
async fn clear_logs() -> Result<(), String> {
    log_buffer::clear();
    Ok(())
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

#[tauri::command]
async fn find_duplicate_media(
    state: State<'_, AppState>,
) -> Result<Vec<database::DuplicateGroup>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.find_duplicate_media().map_err(|e| e.to_string())
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
    let item = db.get_media_by_id(media_id).map_err(|e| e.to_string())?;
    Ok(enrich_media_item_archive_assessment(item))
}

#[tauri::command]
async fn resolve_watch_history_media(
    state: State<'_, AppState>,
    event: database::WatchHistoryEvent,
) -> Result<database::MediaItem, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;

    if let Some(media_id) = event.media_id {
        if let Ok(item) = db.get_media_by_id(media_id) {
            return Ok(enrich_media_item_archive_assessment(item));
        }
    }

    if event.media_type == "tvepisode" {
        let parent = if let Some(parent_id) = event.parent_media_id {
            db.get_media_by_id(parent_id).ok()
        } else if let Some(parent_tmdb_id) = event.parent_tmdb_id.as_deref() {
            db.find_media_by_tmdb(parent_tmdb_id, "tvshow")
                .map_err(|e| e.to_string())?
        } else if let Some(parent_title) = event.parent_title.as_deref() {
            db.find_tvshow_by_title(parent_title)
                .map_err(|e| e.to_string())?
        } else {
            None
        };

        if let (Some(parent), Some(season), Some(episode)) =
            (parent, event.season_number, event.episode_number)
        {
            if let Some(item) = db
                .find_episode_by_parent_and_numbers(parent.id, season, episode)
                .map_err(|e| e.to_string())?
            {
                return Ok(enrich_media_item_archive_assessment(item));
            }
        }
    } else if let Some(tmdb_id) = event.tmdb_id.as_deref() {
        if let Some(item) = db
            .find_media_by_tmdb(tmdb_id, &event.media_type)
            .map_err(|e| e.to_string())?
        {
            return Ok(enrich_media_item_archive_assessment(item));
        }
    }

    Err("Media item could not be resolved from watch history".to_string())
}

#[tauri::command]
async fn get_archive_playback_assessment(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<Option<archive_manager::ArchivePlaybackAssessment>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let item = db.get_media_by_id(media_id).map_err(|e| e.to_string())?;
    Ok(archive_manager::assess_archive_playback(&item))
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

#[derive(Clone, Serialize)]
pub struct AudioTrackInfo {
    pub stream_index: i32,
    pub track_id: Option<i32>,
    pub language_code: Option<String>,
    pub label: String,
    pub detail: Option<String>,
    pub mpv_value: Option<String>,
}

pub type SubtitleTrackInfo = AudioTrackInfo;

#[derive(Clone, Serialize)]
struct MpvAudioTracksDetectedPayload {
    media_id: i64,
    series_id: Option<i64>,
    season_number: Option<i32>,
    tracks: Vec<AudioTrackInfo>,
}

#[derive(Clone, Serialize)]
struct MpvSubtitleTracksDetectedPayload {
    media_id: i64,
    series_id: Option<i64>,
    season_number: Option<i32>,
    tracks: Vec<SubtitleTrackInfo>,
}

#[derive(Deserialize)]
struct FfprobeStreamsOutput {
    #[serde(default)]
    streams: Vec<FfprobeAudioStream>,
}

#[derive(Deserialize)]
struct FfprobeVideoProbeOutput {
    #[serde(default)]
    streams: Vec<FfprobeVideoStream>,
    format: Option<FfprobeFormatInfo>,
}

#[derive(Deserialize)]
struct FfprobeVideoStream {
    width: Option<i32>,
    height: Option<i32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    codec_name: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeFormatInfo {
    format_name: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeAudioStream {
    index: i32,
    #[serde(default)]
    tags: HashMap<String, String>,
}

struct AudioProbeSource {
    stream_url: String,
    access_token: Option<String>,
    temp_zip_proxy: Option<zip_stream_proxy::ZipStreamProxyHandle>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaTechnicalDetails {
    width: Option<i32>,
    height: Option<i32>,
    fps: Option<f64>,
    resolution_label: Option<String>,
    container: Option<String>,
    extension: Option<String>,
    video_codec: Option<String>,
    file_size_bytes: Option<i64>,
    sample_from_episode: Option<bool>,
}

#[derive(Default)]
struct DetectedMpvTracks {
    audio_tracks: Vec<AudioTrackInfo>,
    subtitle_tracks: Vec<SubtitleTrackInfo>,
}

#[derive(Deserialize)]
struct MpvIpcMessage {
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    data: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    request_id: Option<u64>,
}

fn resolve_ffprobe_path(config: &config::Config) -> Option<String> {
    let configured = config
        .ffprobe_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if let Some(path) = configured {
        if std::path::Path::new(&path).exists()
            && config::validate_executable_path(&path, "ffprobe").is_ok()
        {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(ffmpeg_path) = config
            .ffmpeg_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let sibling = std::path::Path::new(ffmpeg_path).with_file_name("ffprobe");
            if sibling.exists()
                && config::validate_executable_path(&sibling.to_string_lossy(), "ffprobe").is_ok()
            {
                return Some(sibling.to_string_lossy().to_string());
            }
        }

        let output = std::process::Command::new("which")
            .arg("ffprobe")
            .output()
            .ok()?;
        if output.status.success() {
            if let Some(path) = String::from_utf8(output.stdout)
                .ok()
                .and_then(|paths| paths.lines().map(str::trim).find(|value| !value.is_empty()).map(str::to_string))
                .filter(|path| std::path::Path::new(path).is_file())
                .filter(|path| config::validate_executable_path(path, "ffprobe").is_ok())
            {
                return Some(path.to_string());
            }
        }

        return None;
    }

    if let Some(ffmpeg_path) = config
        .ffmpeg_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let sibling = std::path::Path::new(ffmpeg_path).with_file_name("ffprobe.exe");
        if sibling.exists()
            && config::validate_executable_path(&sibling.to_string_lossy(), "ffprobe").is_ok()
        {
            return Some(sibling.to_string_lossy().to_string());
        }
    }

    let mut where_cmd = std::process::Command::new("where");
    where_cmd.arg("ffprobe.exe");
    config::apply_hidden_process_flags(&mut where_cmd);
    if let Ok(output) = where_cmd.output() {
        if output.status.success() {
            if let Ok(paths) = String::from_utf8(output.stdout) {
                if let Some(path) = paths.lines().map(str::trim).find(|value| !value.is_empty()) {
                    if std::path::Path::new(path).exists()
                        && config::validate_executable_path(path, "ffprobe").is_ok()
                    {
                        return Some(path.to_string());
                    }
                }
            }
        }
    }

    None
}

fn infer_language_from_text(value: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let normalized = value.trim().to_lowercase();

    match normalized.as_str() {
        "en" | "eng" | "english" => Some(("en", "English", "en,eng,english")),
        "hi" | "hin" | "hindi" => Some(("hi", "Hindi", "hi,hin,hindi")),
        "ta" | "tam" | "tamil" => Some(("ta", "Tamil", "ta,tam,tamil")),
        "te" | "tel" | "telugu" => Some(("te", "Telugu", "te,tel,telugu")),
        "ml" | "mal" | "malayalam" => Some(("ml", "Malayalam", "ml,mal,malayalam")),
        "ja" | "jpn" | "japanese" => Some(("ja", "Japanese", "ja,jpn,japanese")),
        "ko" | "kor" | "korean" => Some(("ko", "Korean", "ko,kor,korean")),
        "ar" | "ara" | "arabic" => Some(("ar", "Arabic", "ar,ara,arabic")),
        "fr" | "fra" | "fre" | "french" => Some(("fr", "French", "fr,fra,french")),
        "es" | "spa" | "spanish" => Some(("es", "Spanish", "es,spa,spanish")),
        "de" | "deu" | "ger" | "german" => Some(("de", "German", "de,deu,german")),
        "it" | "ita" | "italian" => Some(("it", "Italian", "it,ita,italian")),
        "ru" | "rus" | "russian" => Some(("ru", "Russian", "ru,rus,russian")),
        "und" | "" => None,
        _ => {
            let contains = |needle: &str| normalized.contains(needle);

            if contains("english") {
                Some(("en", "English", "en,eng,english"))
            } else if contains("hindi") {
                Some(("hi", "Hindi", "hi,hin,hindi"))
            } else if contains("tamil") {
                Some(("ta", "Tamil", "ta,tam,tamil"))
            } else if contains("telugu") {
                Some(("te", "Telugu", "te,tel,telugu"))
            } else if contains("malayalam") {
                Some(("ml", "Malayalam", "ml,mal,malayalam"))
            } else if contains("japanese") {
                Some(("ja", "Japanese", "ja,jpn,japanese"))
            } else if contains("korean") {
                Some(("ko", "Korean", "ko,kor,korean"))
            } else if contains("arabic") {
                Some(("ar", "Arabic", "ar,ara,arabic"))
            } else if contains("french") {
                Some(("fr", "French", "fr,fra,french"))
            } else if contains("spanish") {
                Some(("es", "Spanish", "es,spa,spanish"))
            } else if contains("german") {
                Some(("de", "German", "de,deu,german"))
            } else if contains("italian") {
                Some(("it", "Italian", "it,ita,italian"))
            } else if contains("russian") {
                Some(("ru", "Russian", "ru,rus,russian"))
            } else {
                None
            }
        }
    }
}

fn build_audio_track_info(
    stream_index: i32,
    track_id: Option<i32>,
    language_tag: Option<String>,
    title: Option<String>,
) -> AudioTrackInfo {
    let language_tag = language_tag
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let title = title
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let inferred = language_tag
        .as_deref()
        .and_then(infer_language_from_text)
        .or_else(|| title.as_deref().and_then(infer_language_from_text));

    let (language_code, mpv_value) = if let Some((code, _name, mpv_value)) = inferred {
        (Some(code.to_string()), Some(mpv_value.to_string()))
    } else if let Some(tag) = language_tag.as_deref() {
        (Some(tag.to_lowercase()), Some(tag.to_lowercase()))
    } else {
        (None, None)
    };

    let label = match (language_tag.as_deref(), title.as_deref()) {
        (Some(language), Some(track_title))
            if !track_title
                .to_lowercase()
                .contains(&language.to_lowercase()) =>
        {
            format!("{} {}", language, track_title)
        }
        (_, Some(track_title)) => track_title.to_string(),
        (Some(language), None) => language.to_string(),
        _ => format!("Track {}", stream_index + 1),
    };

    let detail = match title {
        Some(track_title) if track_title.to_lowercase() != label.to_lowercase() => {
            Some(track_title)
        }
        _ => language_tag
            .filter(|tag| tag.to_lowercase() != label.to_lowercase())
            .filter(|tag| tag.to_lowercase() != language_code.clone().unwrap_or_default()),
    };

    AudioTrackInfo {
        stream_index,
        track_id,
        language_code,
        label,
        detail,
        mpv_value,
    }
}

fn normalize_audio_track(stream: FfprobeAudioStream) -> AudioTrackInfo {
    let language_tag = stream
        .tags
        .get("language")
        .or_else(|| stream.tags.get("LANGUAGE"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let title = stream
        .tags
        .get("title")
        .or_else(|| stream.tags.get("handler_name"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    build_audio_track_info(stream.index, None, language_tag, title)
}

fn normalize_mpv_audio_track(track: &serde_json::Value) -> Option<AudioTrackInfo> {
    if track.get("type").and_then(|value| value.as_str()) != Some("audio") {
        return None;
    }

    let stream_index = track
        .get("ff-index")
        .and_then(|value| value.as_i64())
        .or_else(|| track.get("id").and_then(|value| value.as_i64()))
        .unwrap_or(0) as i32;
    let track_id = track
        .get("id")
        .and_then(|value| value.as_i64())
        .map(|value| value as i32);
    let language_tag = track
        .get("lang")
        .or_else(|| track.get("language"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let title = track
        .get("title")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let codec = track
        .get("codec")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let selected = track
        .get("selected")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let mut normalized = build_audio_track_info(stream_index, track_id, language_tag, title);
    if let Some(id) = track_id.filter(|value| *value > 0) {
        normalized.mpv_value = Some(format!("aid:{}", id));
    }
    let mut detail_parts = Vec::new();

    if let Some(detail) = normalized.detail.take() {
        detail_parts.push(detail);
    }

    if let Some(codec_name) = codec {
        let already_listed = detail_parts
            .iter()
            .any(|part| part.eq_ignore_ascii_case(&codec_name));
        if !already_listed {
            detail_parts.push(codec_name);
        }
    }

    if selected {
        detail_parts.push("Selected".to_string());
    }

    normalized.detail = if detail_parts.is_empty() {
        None
    } else {
        Some(detail_parts.join(" • "))
    };

    Some(normalized)
}

fn normalize_mpv_subtitle_track(track: &serde_json::Value) -> Option<SubtitleTrackInfo> {
    if track.get("type").and_then(|value| value.as_str()) != Some("sub") {
        return None;
    }

    let stream_index = track
        .get("ff-index")
        .and_then(|value| value.as_i64())
        .or_else(|| track.get("id").and_then(|value| value.as_i64()))
        .unwrap_or(0) as i32;
    let track_id = track
        .get("id")
        .and_then(|value| value.as_i64())
        .map(|value| value as i32);
    let language_tag = track
        .get("lang")
        .or_else(|| track.get("language"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let title = track
        .get("title")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let codec = track
        .get("codec")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let selected = track
        .get("selected")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let default_track = track
        .get("default")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let forced = track
        .get("forced")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let mut normalized = build_audio_track_info(stream_index, track_id, language_tag, title);
    if let Some(id) = track_id.filter(|value| *value > 0) {
        normalized.mpv_value = Some(format!("sid:{}", id));
    }

    let mut detail_parts = Vec::new();

    if let Some(detail) = normalized.detail.take() {
        detail_parts.push(detail);
    }

    if let Some(codec_name) = codec {
        let already_listed = detail_parts
            .iter()
            .any(|part| part.eq_ignore_ascii_case(&codec_name));
        if !already_listed {
            detail_parts.push(codec_name);
        }
    }

    if forced {
        detail_parts.push("Forced".to_string());
    }

    if default_track {
        detail_parts.push("Default".to_string());
    }

    if selected {
        detail_parts.push("Selected".to_string());
    }

    normalized.detail = if detail_parts.is_empty() {
        None
    } else {
        Some(detail_parts.join(" • "))
    };

    Some(normalized)
}

fn parse_mpv_audio_tracks(value: &serde_json::Value) -> Vec<AudioTrackInfo> {
    let serde_json::Value::Array(items) = value else {
        return Vec::new();
    };

    let mut seen = HashSet::new();
    let mut tracks = Vec::new();

    for track in items.iter().filter_map(normalize_mpv_audio_track) {
        let key = (
            track.stream_index,
            track.label.to_lowercase(),
            track.mpv_value.clone().unwrap_or_default(),
        );
        if seen.insert(key) {
            tracks.push(track);
        }
    }

    tracks
}

fn parse_mpv_subtitle_tracks(value: &serde_json::Value) -> Vec<SubtitleTrackInfo> {
    let serde_json::Value::Array(items) = value else {
        return Vec::new();
    };

    let mut seen = HashSet::new();
    let mut tracks = Vec::new();

    for track in items.iter().filter_map(normalize_mpv_subtitle_track) {
        let key = (
            track.stream_index,
            track.label.to_lowercase(),
            track.mpv_value.clone().unwrap_or_default(),
        );
        if seen.insert(key) {
            tracks.push(track);
        }
    }

    tracks
}

fn parse_mpv_tracks(value: &serde_json::Value) -> DetectedMpvTracks {
    DetectedMpvTracks {
        audio_tracks: parse_mpv_audio_tracks(value),
        subtitle_tracks: parse_mpv_subtitle_tracks(value),
    }
}

#[cfg(windows)]
fn detect_tracks_from_running_mpv(pipe_name: &str) -> Result<DetectedMpvTracks, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    // NOTE(from_raw_handle safety): File::from_raw_handle takes ownership of the
    // WebviewWindows handle. The returned File will close the handle on drop. This function
    // must NOT be called more than once for the same pipe, because the handle would
    // already be consumed. The `break` after successful acquisition prevents retry
    // within this loop; callers are responsible for ensuring single invocation.
    let mut file = None;
    for _ in 0..100 {
        let wide_name: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                wide_name.as_ptr(),
                0x80000000 | 0x40000000,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            )
        };

        if handle != INVALID_HANDLE_VALUE {
            file = Some(unsafe { std::fs::File::from_raw_handle(handle as *mut std::ffi::c_void) });
            break;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    let mut pipe_file =
        file.ok_or_else(|| format!("Failed to connect to MPV pipe: {}", pipe_name))?;
    let read_file = pipe_file
        .try_clone()
        .map_err(|error| format!("Failed to clone MPV pipe handle: {}", error))?;
    let mut reader = BufReader::new(read_file);

    let observe_tracks = serde_json::json!({
        "command": ["observe_property", 91, "track-list"],
    });
    let read_tracks = serde_json::json!({
        "command": ["get_property", "track-list"],
        "request_id": 901,
    });

    writeln!(pipe_file, "{}", observe_tracks)
        .map_err(|error| format!("Failed to observe MPV track-list: {}", error))?;
    writeln!(pipe_file, "{}", read_tracks)
        .map_err(|error| format!("Failed to request MPV track-list: {}", error))?;

    loop {
        let mut line = String::new();
        let bytes_read = reader
            .read_line(&mut line)
            .map_err(|error| format!("Failed to read MPV IPC response: {}", error))?;

        if bytes_read == 0 {
            return Ok(DetectedMpvTracks::default());
        }

        let message = match serde_json::from_str::<MpvIpcMessage>(line.trim()) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        match message.event.as_deref() {
            Some("file-loaded") => {
                writeln!(pipe_file, "{}", read_tracks)
                    .map_err(|error| format!("Failed to refresh MPV track-list: {}", error))?;
            }
            Some("shutdown") | Some("end-file") => return Ok(DetectedMpvTracks::default()),
            _ => {}
        }

        let is_track_update = matches!(message.event.as_deref(), Some("property-change"))
            && message.name.as_deref() == Some("track-list");
        let is_track_response = message.request_id == Some(901)
            && message.error.as_deref() != Some("property unavailable");

        if is_track_update || is_track_response {
            if let Some(data) = message.data.as_ref() {
                let tracks = parse_mpv_tracks(data);
                if !tracks.audio_tracks.is_empty() || !tracks.subtitle_tracks.is_empty() {
                    return Ok(tracks);
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn detect_tracks_from_running_mpv(_pipe_name: &str) -> Result<DetectedMpvTracks, String> {
    Err("MPV IPC track detection is currently supported only on WebviewWindows".to_string())
}

/// Write auth headers to a temporary file and return its path.
/// This avoids leaking tokens in process listings (visible via `ps` on Linux).
fn temp_file_for_headers(header_content: &str) -> Result<String, String> {
    let temp_dir = std::env::temp_dir();
    let file_name = format!("ffprobe_headers_{}.txt", uuid::Uuid::new_v4());
    let file_path = temp_dir.join(file_name);
    let file_path_str = file_path.to_string_lossy().to_string();
    std::fs::write(&file_path, header_content)
        .map_err(|e| format!("Failed to write header file: {}", e))?;
    Ok(file_path_str)
}

fn probe_tracks_with_ffprobe(
    ffprobe_path: &str,
    source: &str,
    access_token: Option<&str>,
    stream_selector: &str,
    error_label: &str,
) -> Result<Vec<AudioTrackInfo>, String> {
    config::validate_executable_path(ffprobe_path, "ffprobe")?;

    let mut command = std::process::Command::new(ffprobe_path);
    config::apply_hidden_process_flags(&mut command);
    command
        .arg("-v")
        .arg("error")
        .arg("-probesize")
        .arg("1048576")
        .arg("-analyzeduration")
        .arg("1000000")
        .arg("-select_streams")
        .arg(stream_selector)
        .arg("-show_entries")
        .arg("stream=index:stream_tags=language,title,handler_name,LANGUAGE")
        .arg("-of")
        .arg("json");

    let _header_file = if let Some(token) = access_token.filter(|value| !value.trim().is_empty()) {
        let header_content = format!("Authorization: Bearer {}\r\n", token);
        let path = temp_file_for_headers(&header_content)?;
        command.arg("-headers").arg(&path);
        Some(path)
    } else {
        None
    };

    let output = command
        .arg(source)
        .output()
        .map_err(|error| format!("Failed to run ffprobe: {}", error))?;

    // Clean up header temp file
    if let Some(path) = _header_file {
        let _ = std::fs::remove_file(&path);
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "ffprobe could not read {} streams: {}",
            error_label,
            stderr.trim()
        ));
    }

    let parsed: FfprobeStreamsOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Failed to parse ffprobe output: {}", error))?;

    Ok(parsed
        .streams
        .into_iter()
        .map(normalize_audio_track)
        .collect())
}

fn probe_audio_tracks_with_ffprobe(
    ffprobe_path: &str,
    source: &str,
    access_token: Option<&str>,
) -> Result<Vec<AudioTrackInfo>, String> {
    probe_tracks_with_ffprobe(ffprobe_path, source, access_token, "a", "audio")
}

fn probe_subtitle_tracks_with_ffprobe(
    ffprobe_path: &str,
    source: &str,
    access_token: Option<&str>,
) -> Result<Vec<SubtitleTrackInfo>, String> {
    probe_tracks_with_ffprobe(ffprobe_path, source, access_token, "s", "subtitle")
}

fn parse_ffprobe_frame_rate(value: Option<&str>) -> Option<f64> {
    let raw = value?.trim();
    if raw.is_empty() || raw == "0/0" {
        return None;
    }

    if let Some((num, den)) = raw.split_once('/') {
        let numerator = num.trim().parse::<f64>().ok()?;
        let denominator = den.trim().parse::<f64>().ok()?;
        if denominator <= 0.0 {
            return None;
        }
        let fps = numerator / denominator;
        return if fps.is_finite() && fps > 0.0 {
            Some(fps)
        } else {
            None
        };
    }

    let fps = raw.parse::<f64>().ok()?;
    if fps.is_finite() && fps > 0.0 {
        Some(fps)
    } else {
        None
    }
}

fn normalize_container_name(value: Option<&str>, extension: Option<&str>) -> Option<String> {
    let raw = value
        .map(str::trim)
        .filter(|current| !current.is_empty())
        .map(str::to_string)
        .or_else(|| extension.map(str::to_string))?;

    let normalized = match raw.to_lowercase().as_str() {
        "matroska" => "MKV".to_string(),
        "mov,mp4,m4a,3gp,3g2,mj2" => "MP4".to_string(),
        "mov" => "MOV".to_string(),
        "avi" => "AVI".to_string(),
        "webm" => "WEBM".to_string(),
        "mpegts" => "TS".to_string(),
        other => other
            .split(',')
            .next()
            .map(str::trim)
            .unwrap_or(other)
            .to_uppercase(),
    };

    Some(normalized)
}

fn resolution_label_from_dimensions(width: Option<i32>, height: Option<i32>) -> Option<String> {
    let height = height?;
    let width = width.unwrap_or_default();

    let label = if width >= 3800 || height >= 2100 {
        "2160p"
    } else if width >= 2500 || height >= 1400 {
        "1440p"
    } else if width >= 1800 || height >= 1000 {
        "1080p"
    } else if width >= 1200 || height >= 700 {
        "720p"
    } else if height > 0 {
        return Some(format!("{}p", height));
    } else {
        return None;
    };

    Some(label.to_string())
}

fn probe_media_technical_details_with_ffprobe(
    ffprobe_path: &str,
    source: &str,
    access_token: Option<&str>,
    extension: Option<&str>,
    file_size_bytes: Option<i64>,
) -> Result<MediaTechnicalDetails, String> {
    config::validate_executable_path(ffprobe_path, "ffprobe")?;

    let mut command = std::process::Command::new(ffprobe_path);
    config::apply_hidden_process_flags(&mut command);
    command
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=width,height,avg_frame_rate,r_frame_rate,codec_name:format=format_name")
        .arg("-of")
        .arg("json");

    let _header_file = if let Some(token) = access_token.filter(|value| !value.trim().is_empty()) {
        let header_content = format!("Authorization: Bearer {}\r\n", token);
        let path = temp_file_for_headers(&header_content)?;
        command.arg("-headers").arg(&path);
        Some(path)
    } else {
        None
    };

    let output = command
        .arg(source)
        .output()
        .map_err(|error| format!("Failed to run ffprobe: {}", error))?;

    // Clean up header temp file
    if let Some(path) = _header_file {
        let _ = std::fs::remove_file(&path);
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "ffprobe could not read video metadata: {}",
            stderr.trim()
        ));
    }

    let parsed: FfprobeVideoProbeOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Failed to parse ffprobe video output: {}", error))?;
    let stream = parsed.streams.into_iter().next();
    let width = stream.as_ref().and_then(|current| current.width);
    let height = stream.as_ref().and_then(|current| current.height);
    let fps = parse_ffprobe_frame_rate(
        stream
            .as_ref()
            .and_then(|current| current.avg_frame_rate.as_deref())
            .or_else(|| {
                stream
                    .as_ref()
                    .and_then(|current| current.r_frame_rate.as_deref())
            }),
    );
    let video_codec = stream.and_then(|current| current.codec_name);
    let container = normalize_container_name(
        parsed
            .format
            .as_ref()
            .and_then(|current| current.format_name.as_deref()),
        extension,
    );

    Ok(MediaTechnicalDetails {
        width,
        height,
        fps,
        resolution_label: resolution_label_from_dimensions(width, height),
        container,
        extension: extension.map(|current| current.to_uppercase()),
        video_codec,
        file_size_bytes,
        sample_from_episode: None,
    })
}

async fn resolve_audio_probe_source(
    state: &AppState,
    media: &database::MediaItem,
) -> Result<AudioProbeSource, String> {
    let file_path = media.file_path.clone().unwrap_or_default();
    let is_cloud = media.is_cloud.unwrap_or(false);
    let is_zip_media = media.parent_zip_id.is_some();

    if is_cloud {
        if is_zip_media {
            match archive_manager::archive_format_for_media(media) {
                archive_manager::ArchiveFormat::Zip => {
                    match zip_manager::zip_entry_compression_method(media)
                        .map_err(|e| e.to_string())?
                    {
                        0 => {
                            let (stream_url, proxy) =
                                build_temporary_zip_stream_url(state, media).await?;
                            return Ok(AudioProbeSource {
                                stream_url,
                                access_token: None,
                                temp_zip_proxy: Some(proxy),
                            });
                        }
                        8 => {
                            let extracted_path = build_zip_extracted_path(state, media).await?;
                            return Ok(AudioProbeSource {
                                stream_url: extracted_path,
                                access_token: None,
                                temp_zip_proxy: None,
                            });
                        }
                        method => {
                            return Err(format!(
                                "ZIP entry compression method {} is not supported for audio detection",
                                method
                            ));
                        }
                    }
                }
                archive_manager::ArchiveFormat::Tar | archive_manager::ArchiveFormat::Rar => {
                    if archive_manager::build_archive_stream_info(media).is_ok() {
                        let (stream_url, proxy) =
                            build_temporary_zip_stream_url(state, media).await?;
                        return Ok(AudioProbeSource {
                            stream_url,
                            access_token: None,
                            temp_zip_proxy: Some(proxy),
                        });
                    }

                    let extracted_path = build_zip_extracted_path(state, media).await?;
                    return Ok(AudioProbeSource {
                        stream_url: extracted_path,
                        access_token: None,
                        temp_zip_proxy: None,
                    });
                }
            }
        }

        if let Some(ref cloud_file_id) = media.cloud_file_id {
            let (stream_url, access_token) =
                state.gdrive_client.get_stream_url(cloud_file_id).await?;
            return Ok(AudioProbeSource {
                stream_url,
                access_token: Some(access_token),
                temp_zip_proxy: None,
            });
        }

        return Err("Cloud file ID not found".to_string());
    }

    if file_path.is_empty() || !std::path::Path::new(&file_path).exists() {
        return Err("File not found".to_string());
    }

    Ok(AudioProbeSource {
        stream_url: file_path,
        access_token: None,
        temp_zip_proxy: None,
    })
}

fn infer_extension_for_media(media: &database::MediaItem) -> Option<String> {
    media
        .zip_entry_path
        .as_deref()
        .or(media.file_path.as_deref())
        .and_then(|path| {
            std::path::Path::new(path)
                .extension()
                .and_then(|ext| ext.to_str())
        })
        .map(|ext| ext.trim().trim_start_matches('.').to_string())
        .filter(|ext| !ext.is_empty())
}

fn infer_download_filename(media: &database::MediaItem, metadata_name: Option<&str>) -> String {
    let fallback = format!("media-{}", media.id);
    let base_name = metadata_name
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .or_else(|| {
            media
                .zip_entry_path
                .as_deref()
                .and_then(|path| std::path::Path::new(path).file_name())
                .and_then(|value| value.to_str())
                .map(|value| value.to_string())
        })
        .or_else(|| {
            media
                .file_path
                .as_deref()
                .and_then(|path| std::path::Path::new(path).file_name())
                .and_then(|value| value.to_str())
                .map(|value| value.to_string())
        })
        .unwrap_or_else(|| {
            let extension = infer_extension_for_media(media)
                .map(|value| format!(".{}", value))
                .unwrap_or_default();
            format!("{}{}", media.title, extension)
        });

    download_manager::sanitize_download_filename(&base_name, &fallback)
}

#[tauri::command]
async fn get_download_jobs(
    state: State<'_, AppState>,
) -> Result<Vec<download_manager::DownloadJobSnapshot>, String> {
    let mut jobs = state.download_manager.list_jobs();
    let db = state.db.lock().map_err(|e| e.to_string())?;
    for job in &mut jobs {
        job.source_exists = db.get_media_by_id(job.media_id).is_ok();
        job.target_exists = std::path::Path::new(&job.target_path).exists();
    }
    Ok(jobs)
}

#[tauri::command]
async fn cancel_download_job(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    job_id: String,
) -> Result<download_manager::DownloadJobSnapshot, String> {
    let snapshot = state.download_manager.cancel_job(&job_id)?;
    download_manager::emit_job_update(&app_handle, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
async fn delete_download_job(state: State<'_, AppState>, job_id: String) -> Result<(), String> {
    state.download_manager.delete_job(&job_id)
}

#[tauri::command]
async fn clear_download_history(
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    state.download_manager.clear_history();
    // Refresh all jobs for the frontend
    let jobs = state.download_manager.list_jobs();
    let _ = app_handle.emit("download-queue-cleared", jobs);
    Ok(())
}

#[tauri::command]
async fn open_download_job_target(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    job_id: String,
) -> Result<(), String> {
    let snapshot = state
        .download_manager
        .get_job(&job_id)
        .ok_or_else(|| "Download job not found".to_string())?;
    let path = std::path::PathBuf::from(&snapshot.target_path);
    let target = if path.exists() {
        path
    } else {
        path.parent()
            .map(|value| value.to_path_buf())
            .unwrap_or_else(download_manager::default_downloads_dir)
    };
    open::that_detached(&target).map_err(|error| error.to_string())?;
    let _ = app_handle.emit("download-folder-opened", snapshot);
    Ok(())
}

#[tauri::command]
async fn start_media_download(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    media_id: i64,
) -> Result<download_manager::DownloadJobSnapshot, String> {
    let media = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_media_by_id(media_id).map_err(|e| e.to_string())?
    };

    let downloads_dir = download_manager::default_downloads_dir();
    std::fs::create_dir_all(&downloads_dir).map_err(|e| e.to_string())?;

    if media.is_cloud.unwrap_or(false) {
        if media.parent_zip_id.is_some() {
            let archive_format = archive_manager::archive_format_for_media(&media);
            let method = media.zip_compression_method.unwrap_or(-1);
            let supports_direct_range = match archive_format {
                archive_manager::ArchiveFormat::Zip => method == 0,
                archive_manager::ArchiveFormat::Tar | archive_manager::ArchiveFormat::Rar => {
                    method == 0
                }
            };

            if supports_direct_range {
                let stream_info = archive_manager::build_archive_stream_info(&media)?;
                let metadata_name = media
                    .zip_entry_path
                    .as_deref()
                    .and_then(|path| std::path::Path::new(path).file_name())
                    .and_then(|value| value.to_str());
                let file_name = infer_download_filename(&media, metadata_name);
                let target_path = download_manager::unique_target_path(&downloads_dir, &file_name);
                let total_bytes = stream_info
                    .byte_end
                    .saturating_sub(stream_info.byte_start)
                    .saturating_add(1);

                return Ok(download_manager::start_parallel_download(
                    app_handle,
                    state.download_manager.clone(),
                    state.gdrive_client.clone(),
                    download_manager::ParallelDownloadRequest {
                        media_id,
                        title: media.title.clone(),
                        file_name,
                        target_path,
                        file_id: stream_info.zip_file_id,
                        range_start: stream_info.byte_start,
                        total_bytes,
                        source_kind: "archive-range".to_string(),
                        chunk_bytes: download_manager::default_parallel_chunk_bytes(),
                        concurrency: download_manager::default_parallel_concurrency(),
                    },
                ));
            }

            if matches!(archive_format, archive_manager::ArchiveFormat::Zip) && method == 8 {
                let file_name = infer_download_filename(&media, None);
                let target_path = download_manager::unique_target_path(&downloads_dir, &file_name);
                let estimated_total_bytes = media.zip_uncompressed_size.unwrap_or(0).max(0) as u64;
                let (snapshot, cancel_flag) = state.download_manager.create_job(
                    media_id,
                    media.title.clone(),
                    file_name.clone(),
                    target_path.clone(),
                    estimated_total_bytes,
                    "archive-deflate-direct".to_string(),
                );
                download_manager::emit_job_update(&app_handle, &snapshot);

                let gdrive_client = state.gdrive_client.clone();
                let manager = state.download_manager.clone();
                let media_for_extract = media.clone();
                let job_id = snapshot.id.clone();
                let title = media.title.clone();
                let app_handle_clone = app_handle.clone();

                println!(
                    "[DOWNLOAD] queued direct ZIP deflate job {} for media {} ({})",
                    job_id, media_id, title
                );

                tokio::spawn(async move {
                    if let Some(updated) = manager.update_job(&job_id, |job| {
                        job.status = "preparing".to_string();
                    }) {
                        download_manager::emit_job_update(&app_handle_clone, &updated);
                    }

                    if let Some(parent) = target_path.parent() {
                        if let Err(error) = std::fs::create_dir_all(parent) {
                            let error = format!("Failed to create download directory: {}", error);
                            println!("[DOWNLOAD] ZIP deflate job {} failed: {}", job_id, error);
                            if let Some(updated) = manager.update_job(&job_id, |job| {
                                job.status = "failed".to_string();
                                job.error = Some(error.clone());
                            }) {
                                download_manager::emit_job_update(&app_handle_clone, &updated);
                            }
                            return;
                        }
                    }

                    let access_token = match gdrive_client.get_access_token().await {
                        Ok(token) => token,
                        Err(error) => {
                            println!(
                                "[DOWNLOAD] ZIP deflate job {} failed to get token: {}",
                                job_id, error
                            );
                            if let Some(updated) = manager.update_job(&job_id, |job| {
                                job.status = "failed".to_string();
                                job.error = Some(error.clone());
                            }) {
                                download_manager::emit_job_update(&app_handle_clone, &updated);
                            }
                            return;
                        }
                    };

                    let temp_path = target_path.with_extension(format!(
                        "{}.part",
                        target_path
                            .extension()
                            .and_then(|value| value.to_str())
                            .unwrap_or("download")
                    ));
                    let manager_for_progress = manager.clone();
                    let app_for_progress = app_handle_clone.clone();
                    let job_id_for_progress = job_id.clone();
                    let progress_step = 4 * 1024 * 1024_u64;

                    println!(
                        "[DOWNLOAD] ZIP deflate job {} extracting directly to {}",
                        job_id,
                        temp_path.to_string_lossy()
                    );

                    if let Some(updated) = manager.update_job(&job_id, |job| {
                        job.status = "downloading".to_string();
                    }) {
                        download_manager::emit_job_update(&app_handle_clone, &updated);
                    }

                    let started_at = std::time::Instant::now();
                    let media_for_worker = media_for_extract.clone();
                    let temp_path_for_worker = temp_path.clone();
                    let extract_result = tokio::task::spawn_blocking(move || {
                        let mut last_emitted = 0u64;
                        zip_manager::extract_zip_entry_to_path_with_progress(
                            &access_token,
                            &media_for_worker,
                            &temp_path_for_worker,
                            |written, total| {
                                if written < total
                                    && written.saturating_sub(last_emitted) < progress_step
                                {
                                    return;
                                }
                                last_emitted = written;
                                let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
                                if let Some(updated) =
                                    manager_for_progress.update_job(&job_id_for_progress, |job| {
                                        job.downloaded_bytes = written;
                                        job.total_bytes = total;
                                        job.progress = ((written as f64 / total.max(1) as f64)
                                            * 100.0)
                                            .clamp(0.0, 100.0);
                                        job.speed_bytes_per_second = Some(written as f64 / elapsed);
                                    })
                                {
                                    download_manager::emit_job_update(&app_for_progress, &updated);
                                }
                            },
                        )
                    })
                    .await;

                    match extract_result {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            let _ = std::fs::remove_file(&temp_path);
                            let error = error.to_string();
                            println!(
                                "[DOWNLOAD] ZIP deflate job {} extraction failed: {}",
                                job_id, error
                            );
                            if let Some(updated) = manager.update_job(&job_id, |job| {
                                job.status = "failed".to_string();
                                job.error = Some(error.clone());
                            }) {
                                download_manager::emit_job_update(&app_handle_clone, &updated);
                            }
                            return;
                        }
                        Err(error) => {
                            let _ = std::fs::remove_file(&temp_path);
                            let error = error.to_string();
                            println!(
                                "[DOWNLOAD] ZIP deflate job {} join failed: {}",
                                job_id, error
                            );
                            if let Some(updated) = manager.update_job(&job_id, |job| {
                                job.status = "failed".to_string();
                                job.error = Some(error.clone());
                            }) {
                                download_manager::emit_job_update(&app_handle_clone, &updated);
                            }
                            return;
                        }
                    }

                    if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = std::fs::remove_file(&temp_path);
                        println!("[DOWNLOAD] ZIP deflate job {} cancelled", job_id);
                        if let Some(updated) = manager.update_job(&job_id, |job| {
                            job.status = "cancelled".to_string();
                        }) {
                            download_manager::emit_job_update(&app_handle_clone, &updated);
                        }
                        return;
                    }

                    if let Err(error) = std::fs::rename(&temp_path, &target_path) {
                        let _ = std::fs::remove_file(&temp_path);
                        let error = format!("Failed to finalize extracted download: {}", error);
                        println!(
                            "[DOWNLOAD] ZIP deflate job {} finalize failed: {}",
                            job_id, error
                        );
                        if let Some(updated) = manager.update_job(&job_id, |job| {
                            job.status = "failed".to_string();
                            job.error = Some(error.clone());
                        }) {
                            download_manager::emit_job_update(&app_handle_clone, &updated);
                        }
                        return;
                    }

                    if let Some(updated) = manager.update_job(&job_id, |job| {
                        job.status = "completed".to_string();
                        job.downloaded_bytes = job.total_bytes;
                        job.progress = 100.0;
                        job.target_exists = true;
                        job.speed_bytes_per_second = None;
                        job.error = None;
                    }) {
                        download_manager::emit_job_update(&app_handle_clone, &updated);
                    }
                });

                return Ok(snapshot);
            }

            let file_name = infer_download_filename(&media, None);
            let target_path = download_manager::unique_target_path(&downloads_dir, &file_name);
            let estimated_total_bytes = media.zip_uncompressed_size.unwrap_or(0).max(0) as u64;
            let (snapshot, cancel_flag) = state.download_manager.create_job(
                media_id,
                media.title.clone(),
                file_name.clone(),
                target_path.clone(),
                estimated_total_bytes,
                "archive-extract".to_string(),
            );
            download_manager::emit_job_update(&app_handle, &snapshot);

            let cache_config = {
                let config = state.config.lock().map_err(|e| e.to_string())?;
                build_zip_cache_config(&config)
            };
            let gdrive_client = state.gdrive_client.clone();
            let manager = state.download_manager.clone();
            let media_for_extract = media.clone();
            let job_id = snapshot.id.clone();
            let title = media.title.clone();
            let app_handle_clone = app_handle.clone();

            println!(
                "[DOWNLOAD] queued archive extract job {} for media {} ({})",
                job_id, media_id, title
            );

            tokio::spawn(async move {
                if let Some(updated) = manager.update_job(&job_id, |job| {
                    job.status = "preparing".to_string();
                }) {
                    download_manager::emit_job_update(&app_handle_clone, &updated);
                }

                let access_token = match gdrive_client.get_access_token().await {
                    Ok(token) => token,
                    Err(error) => {
                        println!(
                            "[DOWNLOAD] archive extract job {} failed to get token: {}",
                            job_id, error
                        );
                        if let Some(updated) = manager.update_job(&job_id, |job| {
                            job.status = "failed".to_string();
                            job.error = Some(error.clone());
                        }) {
                            download_manager::emit_job_update(&app_handle_clone, &updated);
                        }
                        return;
                    }
                };

                println!(
                    "[DOWNLOAD] extracting archive entry for job {} from media {}",
                    job_id, media_id
                );
                let extracted_path = match tokio::task::spawn_blocking(move || {
                    archive_manager::extract_archive_entry_to_cache(
                        &access_token,
                        &media_for_extract,
                        &cache_config,
                    )
                })
                .await
                {
                    Ok(Ok(path)) => path,
                    Ok(Err(error)) => {
                        println!(
                            "[DOWNLOAD] archive extract job {} extraction failed: {}",
                            job_id, error
                        );
                        if let Some(updated) = manager.update_job(&job_id, |job| {
                            job.status = "failed".to_string();
                            job.error = Some(error.clone());
                        }) {
                            download_manager::emit_job_update(&app_handle_clone, &updated);
                        }
                        return;
                    }
                    Err(error) => {
                        let error = error.to_string();
                        println!(
                            "[DOWNLOAD] archive extract job {} task join failed: {}",
                            job_id, error
                        );
                        if let Some(updated) = manager.update_job(&job_id, |job| {
                            job.status = "failed".to_string();
                            job.error = Some(error.clone());
                        }) {
                            download_manager::emit_job_update(&app_handle_clone, &updated);
                        }
                        return;
                    }
                };

                if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    println!(
                        "[DOWNLOAD] archive extract job {} was cancelled after extract",
                        job_id
                    );
                    if let Some(updated) = manager.update_job(&job_id, |job| {
                        job.status = "cancelled".to_string();
                    }) {
                        download_manager::emit_job_update(&app_handle_clone, &updated);
                    }
                    return;
                }

                let source_path = std::path::PathBuf::from(&extracted_path);
                let total_bytes = match std::fs::metadata(&source_path) {
                    Ok(metadata) => metadata.len(),
                    Err(error) => {
                        let error = error.to_string();
                        println!(
                            "[DOWNLOAD] archive extract job {} missing extracted file: {}",
                            job_id, error
                        );
                        if let Some(updated) = manager.update_job(&job_id, |job| {
                            job.status = "failed".to_string();
                            job.error = Some(error.clone());
                        }) {
                            download_manager::emit_job_update(&app_handle_clone, &updated);
                        }
                        return;
                    }
                };

                if let Some(updated) = manager.update_job(&job_id, |job| {
                    job.total_bytes = total_bytes;
                    job.error = None;
                }) {
                    download_manager::emit_job_update(&app_handle_clone, &updated);
                }

                println!(
                    "[DOWNLOAD] archive extract job {} handing off {} bytes to local copy",
                    job_id, total_bytes
                );
                download_manager::run_local_copy_job(
                    app_handle_clone,
                    manager,
                    job_id,
                    cancel_flag,
                    download_manager::LocalCopyRequest {
                        media_id,
                        title,
                        file_name,
                        target_path,
                        source_path,
                        total_bytes,
                        source_kind: "archive-extract".to_string(),
                    },
                )
                .await;
            });

            return Ok(snapshot);
        }

        let file_id = media
            .cloud_file_id
            .clone()
            .ok_or_else(|| "Cloud file ID not found".to_string())?;
        let metadata = state.gdrive_client.get_file_metadata(&file_id).await?;
        let total_bytes = metadata
            .size
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .or_else(|| {
                media
                    .file_size_bytes
                    .and_then(|value| u64::try_from(value).ok())
            })
            .ok_or_else(|| "Cloud file size not available for download".to_string())?;
        if total_bytes == 0 {
            return Err("Cloud file is empty and cannot be downloaded".to_string());
        }
        let file_name = infer_download_filename(&media, Some(&metadata.name));
        let target_path = download_manager::unique_target_path(&downloads_dir, &file_name);
        return Ok(download_manager::start_parallel_download(
            app_handle,
            state.download_manager.clone(),
            state.gdrive_client.clone(),
            download_manager::ParallelDownloadRequest {
                media_id,
                title: media.title.clone(),
                file_name,
                target_path,
                file_id,
                range_start: 0,
                total_bytes,
                source_kind: "cloud-drive".to_string(),
                chunk_bytes: download_manager::default_parallel_chunk_bytes(),
                concurrency: download_manager::default_parallel_concurrency(),
            },
        ));
    }

    let source_path = media
        .file_path
        .clone()
        .ok_or_else(|| "Local file path not found".to_string())?;
    let source_path = std::path::PathBuf::from(source_path);
    if !source_path.exists() {
        return Err("Local file does not exist".to_string());
    }
    let total_bytes = std::fs::metadata(&source_path)
        .map_err(|e| e.to_string())?
        .len();
    let file_name = infer_download_filename(&media, None);
    let target_path = download_manager::unique_target_path(&downloads_dir, &file_name);
    Ok(download_manager::start_local_copy(
        app_handle,
        state.download_manager.clone(),
        download_manager::LocalCopyRequest {
            media_id,
            title: media.title.clone(),
            file_name,
            target_path,
            source_path,
            total_bytes,
            source_kind: "local-copy".to_string(),
        },
    ))
}

#[tauri::command]
async fn get_media_technical_details(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<MediaTechnicalDetails, String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };
    let ffprobe_path = resolve_ffprobe_path(&config);

    let media = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_media_by_id(media_id).map_err(|e| e.to_string())?
    };

    let extension = infer_extension_for_media(&media);
    let file_size_bytes = media.file_size_bytes.or_else(|| {
        media.file_path.as_ref().and_then(|path| {
            if media.is_cloud.unwrap_or(false) || !std::path::Path::new(path).exists() {
                None
            } else {
                std::fs::metadata(path)
                    .ok()
                    .map(|metadata| metadata.len() as i64)
            }
        })
    });

    if let Some(ffprobe_path) = ffprobe_path {
        if let Ok(mut source) = resolve_audio_probe_source(&state, &media).await {
            let result = probe_media_technical_details_with_ffprobe(
                &ffprobe_path,
                &source.stream_url,
                source.access_token.as_deref(),
                extension.as_deref(),
                file_size_bytes,
            );

            if let Some(proxy) = source.temp_zip_proxy.take() {
                let _ = stop_zip_proxy_handle_blocking(proxy).await;
            }

            if let Ok(details) = result {
                return Ok(details);
            }
        }
    }

    Ok(MediaTechnicalDetails {
        width: None,
        height: None,
        fps: None,
        resolution_label: None,
        container: normalize_container_name(None, extension.as_deref()),
        extension: extension.map(|current| current.to_uppercase()),
        video_codec: None,
        file_size_bytes,
        sample_from_episode: None,
    })
}

#[tauri::command]
async fn get_audio_tracks(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<Vec<AudioTrackInfo>, String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };
    let ffprobe_path = resolve_ffprobe_path(&config).ok_or_else(|| {
        "FFprobe is not configured. Set FFprobe in Settings > Player to detect audio tracks."
            .to_string()
    })?;
    let media = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_media_by_id(media_id).map_err(|e| e.to_string())?
    };

    let mut source = resolve_audio_probe_source(&state, &media).await?;
    let result = probe_audio_tracks_with_ffprobe(
        &ffprobe_path,
        &source.stream_url,
        source.access_token.as_deref(),
    );

    if let Some(proxy) = source.temp_zip_proxy.take() {
        let _ = stop_zip_proxy_handle_blocking(proxy).await;
    }

    result
}

#[tauri::command]
async fn get_subtitle_tracks(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<Vec<SubtitleTrackInfo>, String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };
    let ffprobe_path = resolve_ffprobe_path(&config).ok_or_else(|| {
        "FFprobe is not configured. Set FFprobe in Settings > Player to detect subtitle tracks."
            .to_string()
    })?;
    let media = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_media_by_id(media_id).map_err(|e| e.to_string())?
    };

    let mut source = resolve_audio_probe_source(&state, &media).await?;
    let result = probe_subtitle_tracks_with_ffprobe(
        &ffprobe_path,
        &source.stream_url,
        source.access_token.as_deref(),
    );

    if let Some(proxy) = source.temp_zip_proxy.take() {
        let _ = stop_zip_proxy_handle_blocking(proxy).await;
    }

    result
}

#[tauri::command]
async fn get_stream_info(state: State<'_, AppState>, media_id: i64) -> Result<StreamInfo, String> {
    let media = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_media_by_id(media_id).map_err(|e| e.to_string())?
    };

    let file_path = media.file_path.clone().unwrap_or_default();
    let is_cloud = media.is_cloud.unwrap_or(false);
    let is_zip_media = media.parent_zip_id.is_some();

    // Handle cloud media
    if is_cloud {
        if is_zip_media {
            match archive_manager::archive_format_for_media(&media) {
                archive_manager::ArchiveFormat::Zip => {
                    match zip_manager::zip_entry_compression_method(&media)
                        .map_err(|e| e.to_string())?
                    {
                        0 => {
                            let stream_url = build_zip_stream_url(&state, &media, media_id).await?;

                            return Ok(StreamInfo {
                                stream_url,
                                file_path,
                                title: media.title,
                                poster: poster_asset_url(media.poster_path.as_ref()),
                                duration_seconds: media.duration_seconds,
                                resume_position_seconds: media.resume_position_seconds,
                                is_cloud: true,
                                access_token: None,
                            });
                        }
                        8 => {
                            let extracted_path = build_zip_extracted_path(&state, &media).await?;

                            return Ok(StreamInfo {
                                stream_url: extracted_path.clone(),
                                file_path: extracted_path,
                                title: media.title,
                                poster: poster_asset_url(media.poster_path.as_ref()),
                                duration_seconds: media.duration_seconds,
                                resume_position_seconds: media.resume_position_seconds,
                                is_cloud: false,
                                access_token: None,
                            });
                        }
                        method => {
                            return Err(format!(
                                "ZIP entry compression method {} is not supported for playback",
                                method
                            ));
                        }
                    }
                }
                archive_manager::ArchiveFormat::Tar | archive_manager::ArchiveFormat::Rar => {
                    if archive_manager::build_archive_stream_info(&media).is_ok() {
                        let stream_url = build_zip_stream_url(&state, &media, media_id).await?;
                        return Ok(StreamInfo {
                            stream_url,
                            file_path,
                            title: media.title,
                            poster: poster_asset_url(media.poster_path.as_ref()),
                            duration_seconds: media.duration_seconds,
                            resume_position_seconds: media.resume_position_seconds,
                            is_cloud: true,
                            access_token: None,
                        });
                    }

                    let extracted_path = build_zip_extracted_path(&state, &media).await?;

                    return Ok(StreamInfo {
                        stream_url: extracted_path.clone(),
                        file_path: extracted_path,
                        title: media.title,
                        poster: poster_asset_url(media.poster_path.as_ref()),
                        duration_seconds: media.duration_seconds,
                        resume_position_seconds: media.resume_position_seconds,
                        is_cloud: false,
                        access_token: None,
                    });
                }
            }
        }

        if let Some(ref cloud_file_id) = media.cloud_file_id {
            // Get streaming URL and access token from Google Drive
            let (stream_url, access_token) =
                state.gdrive_client.get_stream_url(cloud_file_id).await?;

            return Ok(StreamInfo {
                stream_url,
                file_path,
                title: media.title,
                poster: poster_asset_url(media.poster_path.as_ref()),
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

    Ok(StreamInfo {
        stream_url,
        file_path,
        title: media.title,
        poster: poster_asset_url(media.poster_path.as_ref()),
        duration_seconds: media.duration_seconds,
        resume_position_seconds: media.resume_position_seconds,
        is_cloud: false,
        access_token: None,
    })
}

#[tauri::command]
async fn zip_analyze(
    state: State<'_, AppState>,
    zip_file_id: String,
) -> Result<zip_manager::ZipAnalysisResult, String> {
    let access_token = state.gdrive_client.get_access_token().await?;
    let analysis = tokio::task::spawn_blocking(move || {
        zip_manager::analyze_zip_for_preview(&access_token, &zip_file_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(analysis)
}

#[tauri::command]
async fn zip_index_episodes(
    state: State<'_, AppState>,
    window: WebviewWindow,
    zip_file_id: String,
    folder_id: String,
) -> Result<zip_manager::ZipIndexResult, String> {
    let (api_key, zip_indexing_enabled) = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        (
            tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default()),
            config.zip_indexing_enabled,
        )
    };

    if !zip_indexing_enabled {
        return Err("ZIP indexing is disabled in Settings > Cloud Storage.".to_string());
    }

    let access_token = state.gdrive_client.get_access_token().await?;
    let drive_item = state.gdrive_client.get_file_metadata(&zip_file_id).await?;
    emit_zip_processing_event(
        &window,
        "detected",
        1,
        Some(&drive_item.name),
        None,
        &format!(
            "Archive detected: {}. Processing episode entries...",
            drive_item.name
        ),
    );
    let image_cache_dir = database::get_image_cache_dir();
    std::fs::create_dir_all(&image_cache_dir).ok();
    let db_path = database::get_database_path();
    let drive_item_for_index = drive_item.clone();
    let archive_cache_config = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        build_zip_cache_config(&config)
    };

    let indexed_items = tokio::task::spawn_blocking(move || {
        let db = database::Database::new(&db_path).map_err(|e| e.to_string())?;
        let mut tv_show_cache: HashMap<String, (i64, Option<String>, String)> = HashMap::new();
        let mut season_cache: HashMap<(String, i32), Vec<tmdb::TmdbEpisodeInfo>> = HashMap::new();

        let indexed = index_zip_archive_with_metadata(
            &db,
            &access_token,
            &drive_item_for_index,
            &folder_id,
            &api_key,
            &image_cache_dir,
            &archive_cache_config,
            &mut tv_show_cache,
            &mut season_cache,
        )?;

        let repaired = db
            .repair_misparented_archive_episodes()
            .map_err(|e| e.to_string())?;
        if repaired > 0 {
            println!(
                "[ZIP] Re-parented {} archive episode(s) after direct ZIP indexing",
                repaired
            );
        }

        Ok::<Vec<IndexedCloudItem>, String>(indexed)
    })
    .await
    .map_err(|e| e.to_string())?;

    if let Err(error) = &indexed_items {
        emit_zip_processing_event(
            &window,
            "error",
            1,
            Some(&drive_item.name),
            None,
            &format!("ZIP processing failed: {}", error),
        );
    }

    let indexed_items = indexed_items?;

    emit_zip_processing_event(
        &window,
        "complete",
        1,
        Some(&drive_item.name),
        Some(indexed_items.len()),
        &format!(
            "Finished processing {}. Indexed {} episode(s).",
            drive_item.name,
            indexed_items.len()
        ),
    );

    window.emit("library-updated", ()).ok();

    Ok(zip_manager::ZipIndexResult {
        indexed_count: indexed_items.len(),
        skipped_count: 0,
        message: format!(
            "Indexed {} episode(s) from ZIP archive",
            indexed_items.len()
        ),
    })
}

#[tauri::command]
async fn zip_get_stream_info(
    state: State<'_, AppState>,
    media_id: i64,
) -> Result<zip_manager::ZipStreamInfo, String> {
    let media = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_media_by_id(media_id).map_err(|e| e.to_string())?
    };

    zip_manager::build_zip_stream_info(&media).map_err(|e| e.to_string())
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
async fn clear_progress(state: State<'_, AppState>, media_id: i64) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.clear_progress(media_id).map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: "Progress cleared.".to_string(),
    })
}

#[tauri::command]
async fn update_episode_duration(
    state: State<'_, AppState>,
    media_id: i64,
    duration_seconds: f64,
) -> Result<ApiResponse, String> {
    if duration_seconds <= 0.0 {
        return Ok(ApiResponse {
            message: "Skipped: invalid duration.".to_string(),
        });
    }
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_duration(media_id, duration_seconds)
        .map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: "Duration updated.".to_string(),
    })
}

// Fix match - update metadata from TMDB (or OMDb hybrid)
#[tauri::command]
async fn fix_match(
    window: WebviewWindow,
    state: State<'_, AppState>,
    media_id: i64,
    tmdb_id: String,
    media_type: String,
    imdb_id: Option<String>,
) -> Result<ApiResponse, String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };

    let api_key = tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default());
    let _omdb_credential = get_omdb_credential(&config.omdb_api_key.clone().unwrap_or_default());
    let image_cache_dir = database::get_image_cache_dir();

    println!(
        "[META] fix_match called for media_id={}, tmdb_id={}, imdb_id={:?}",
        media_id, tmdb_id, imdb_id
    );

    let mut metadata = if let Some(ref imdb) = imdb_id {
        // IMDb ID provided: resolve via TMDB /find, then fetch full TMDB metadata
        let api_key_c = api_key.clone();
        let imdb_c = imdb.clone();
        let img_c = image_cache_dir.clone();

        tokio::time::timeout(
            Duration::from_secs(40),
            tokio::task::spawn_blocking(move || -> Result<tmdb::TmdbMetadata, String> {
                let (tmdb_id_resolved, resolved_type) =
                    find_tmdb_id_by_imdb_id(&api_key_c, &imdb_c)
                        .ok_or_else(|| format!("No TMDB match found for IMDb ID {}", imdb_c))?;

                let mut meta = tmdb::fetch_metadata_by_id(
                    &api_key_c,
                    &tmdb_id_resolved,
                    &resolved_type,
                    &img_c,
                )
                .map_err(|e| format!("TMDB fetch failed: {}", e))?;

                meta.imdb_id = Some(imdb_c);
                Ok(meta)
            }),
        )
        .await
        .map_err(|_| "Fix Match timed out while fetching metadata".to_string())?
        .map_err(|e| e.to_string())??
    } else {
        // Standard TMDB-only mode
        let api_key_clone = api_key.clone();
        let tmdb_id_clone = tmdb_id.clone();
        let media_type_clone = media_type.clone();
        let image_cache_dir_clone = image_cache_dir.clone();

        let mut meta = tokio::time::timeout(
            Duration::from_secs(40),
            tokio::task::spawn_blocking(move || {
                tmdb::fetch_metadata_by_id(
                    &api_key_clone,
                    &tmdb_id_clone,
                    &media_type_clone,
                    &image_cache_dir_clone,
                )
                .map_err(|e| e.to_string())
            }),
        )
        .await
        .map_err(|_| "Fix Match timed out while fetching metadata. Please try again.".to_string())?
        .map_err(|e| e.to_string())??;

        // Try to resolve imdb_id from TMDB external_ids if not already set
        if meta.imdb_id.is_none() {
            let api_key_c = api_key.clone();
            let tmdb_id_c = tmdb_id.clone();
            let media_type_c = media_type.clone();
            let ext_imdb = tokio::time::timeout(
                Duration::from_secs(10),
                tokio::task::spawn_blocking(move || -> Option<String> {
                    let ext_url = build_tmdb_api_url(
                        &format!(
                            "/{}/{}/external_ids",
                            if media_type_c == "tv" { "tv" } else { "movie" },
                            tmdb_id_c
                        ),
                        &api_key_c,
                        "",
                    );
                    let client = http_client::shared_client();
                    let use_bearer = crate::is_access_token(&api_key_c);
                    let req = if use_bearer {
                        client
                            .get(&ext_url)
                            .header("Authorization", format!("Bearer {}", api_key_c))
                    } else {
                        client.get(&ext_url)
                    };
                    let resp = req.send().ok()?;
                    if !resp.status().is_success() {
                        return None;
                    }
                    let json: serde_json::Value = resp.json().ok()?;
                    json.get("imdb_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                }),
            )
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten();
            if let Some(imdb) = ext_imdb {
                println!("[TMDB] Resolved imdb_id from external_ids: {}", imdb);
                meta.imdb_id = Some(imdb);
            }
        }

        meta
    };

    println!(
        "[TMDB] fix_match metadata: poster={:?}",
        metadata.poster_path
    );
    let updated_title = metadata.title.clone();
    let updated_tmdb_id = metadata.tmdb_id.clone();

    let parent_id = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.update_metadata(media_id, &metadata)
            .map_err(|e| e.to_string())?;
        db.get_media_by_id(media_id)
            .ok()
            .and_then(|item| item.parent_id)
    };

    // Always try imdbapi.dev for poster — prefer it over TMDB if available
    if let Some(ref imdb_id) = metadata.imdb_id {
        let image_cache_dir = database::get_image_cache_dir();
        let image_type = if media_type == "tv" {
            tmdb::ImageType::SeriesBanner
        } else {
            tmdb::ImageType::MovieBanner
        };
        let imdb_url = format!("https://api.imdbapi.dev/titles/{}", imdb_id);
        if let Ok(resp) = http_client::shared_client().get(&imdb_url).send() {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                if let Some(img_url) = json
                    .get("primaryImage")
                    .and_then(|i| i.get("url"))
                    .and_then(|u| u.as_str())
                {
                    if let Some(cached_path) = tmdb::cache_imdb_image(
                        img_url,
                        std::path::Path::new(&image_cache_dir),
                        &image_type,
                    ) {
                        println!(
                            "[IMDBAPI] fix_match poster override: Ok(\"{}\")",
                            cached_path
                        );
                        metadata.poster_path = Some(cached_path.clone());
                        if let Ok(db) = state.db.lock() {
                            let _ = db.update_poster_path(media_id, &cached_path);
                        }
                    } else {
                        println!("[IMDBAPI] fix_match poster: Err(cache failed)");
                    }
                } else {
                    println!("[IMDBAPI] fix_match poster: Err(no image in response), keeping TMDB poster");
                }
            }
        }
    }

    let payload = serde_json::json!({
        "type": "metadata-updated",
        "title": updated_title,
        "media_id": media_id,
        "parent_id": parent_id,
        "media_type": media_type,
        "tmdb_id": updated_tmdb_id,
    });
    window.emit("media-metadata-updated", payload.clone()).ok();
    window.emit("library-updated", payload).ok();

    Ok(ApiResponse {
        message: format!("Metadata updated for: {}", metadata.title),
    })
}

// Play media with MPV (external player) with progress tracking
#[cfg(windows)]
fn ensure_zip_proxy_firewall_rule() {
    use std::process::Command;

    let rule_name = "SlasshyVault ZIP Proxy";

    // Check if rule already exists (no elevation needed for show)
    let check = Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "show",
            "rule",
            &format!("name={}", rule_name),
        ])
        .output();
    if let Ok(output) = check {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("SlasshyVault ZIP Proxy") && !stdout.contains("0 rules") {
            println!("[ZIP PROXY] Firewall rule already exists");
            return;
        }
    }

    // Only prompt UAC once per process lifetime
    static ATTEMPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if ATTEMPTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => return,
    };
    let port = zip_stream_proxy::ZIP_PROXY_PORT;

    // Base64 encode the command to avoid string injection issues entirely,
    // and since the nested process runs as admin, it won't inherit process-level
    // environment variables, so we must bake the arguments safely.
    // Convert to UTF-16LE as required by PowerShell -EncodedCommand.
    let netsh_script = format!(
        "& {{ netsh advfirewall firewall delete rule name='{rule}'; netsh advfirewall firewall add rule name='{rule}' dir=in action=allow protocol=TCP localport={port} program='{exe}' remoteip=127.0.0.1,::1 enable=yes }}",
        rule = rule_name,
        port = port,
        exe = exe.replace('\'', "''"), // Escape single quotes in path
    );
    let encoded_script = base64::encode(
        netsh_script
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect::<Vec<u8>>(),
    );

    let ps = format!(
        "Start-Process -FilePath 'powershell.exe' -Verb RunAs -WebviewWindowStyle Hidden -Wait -ArgumentList @('-NoProfile','-WebviewWindowStyle','Hidden','-EncodedCommand','{}')",
        encoded_script
    );

    match Command::new("powershell")
        .args(["-Command", &ps])
        .spawn() {
        Ok(_) => println!("[ZIP PROXY] Firewall rule requested — accept the UAC prompt to allow MPV loopback connections"),
        Err(e) => println!("[ZIP PROXY] Could not launch PowerShell to set up firewall: {}", e),
    }
}

async fn start_zip_proxy_blocking(
    proxy_spec: zip_stream_proxy::ProxyStreamSpec,
) -> Result<zip_stream_proxy::ZipStreamProxyHandle, String> {
    tokio::task::spawn_blocking(move || zip_stream_proxy::start_proxy(proxy_spec))
        .await
        .map_err(|error| format!("ZIP proxy task join error: {}", error))?
}

async fn stop_zip_proxy_handle_blocking(
    mut proxy: zip_stream_proxy::ZipStreamProxyHandle,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || proxy.stop())
        .await
        .map_err(|error| format!("ZIP proxy stop join error: {}", error))
}

#[tauri::command]
async fn play_with_mpv(
    window: WebviewWindow,
    state: State<'_, AppState>,
    media_id: i64,
    resume: bool,
    audio_language: Option<String>,
    subtitle_language: Option<String>,
    subtitle_file_path: Option<String>,
    duration_seconds_override: Option<f64>,
    file_size_bytes_override: Option<i64>,
) -> Result<ApiResponse, String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };

    let mpv_path = config
        .mpv_path
        .as_ref()
        .ok_or_else(|| "MPV path not set".to_string())?;

    if mpv_path.is_empty() || !std::path::Path::new(mpv_path).exists() {
        return Err("MPV path not set or invalid".to_string());
    }

    let (media, resume_info) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let media = db.get_media_by_id(media_id).map_err(|e| e.to_string())?;
        let resume_info = db.get_resume_info(media_id).map_err(|e| e.to_string())?;

        (media, resume_info)
    };

    let is_cloud = media.is_cloud.unwrap_or(false);
    let title = media.title.clone();
    let episode_title = media.episode_title.clone();
    let season_number = media.season_number;
    let episode_number = media.episode_number;
    let media_type = media.media_type.clone();
    let series_id = if media.media_type == "tvshow" {
        Some(media.id)
    } else {
        media.parent_id
    };
    let audio_language = audio_language.and_then(|value| {
        let normalized = value.trim().to_string();
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    });
    let subtitle_language = subtitle_language.and_then(|value| {
        let normalized = value.trim().to_string();
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    });
    let subtitle_file_path = subtitle_file_path.and_then(|value| {
        let normalized = value.trim().to_string();
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    });

    // Determine start position before picking the playback source so ZIP playback can
    // choose between proxy streaming and cache-backed local playback.
    let start_position = if resume && resume_info.has_progress {
        resume_info.position
    } else {
        0.0
    };

    // Get the playback URL and optional auth header
    let is_zip_media = media.parent_zip_id.is_some();
    let zip_compression_method = if is_zip_media
        && archive_manager::archive_format_for_media(&media) == archive_manager::ArchiveFormat::Zip
    {
        Some(zip_manager::zip_entry_compression_method(&media).map_err(|e| e.to_string())?)
    } else {
        None
    };

    let is_ddl_media = media.ddl_source_id.is_some();

    let (playback_url, auth_header, zip_proxy, gdrive_proxy, playback_is_cloud): (
        String,
        Option<String>,
        Option<zip_stream_proxy::ZipStreamProxyHandle>,
        Option<gdrive_stream_proxy::GdriveStreamProxyHandle>,
        bool,
    ) = if is_ddl_media && is_zip_media {
        // Direct Download Link media — use ProxyAuth::None
        let ddl_source_id = media
            .ddl_source_id
            .as_deref()
            .or(media.parent_zip_id.as_deref())
            .ok_or_else(|| "DDL media missing source ID".to_string())?;
        let ddl_url = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            let source = db
                .get_ddl_source(ddl_source_id)
                .map_err(|e| e.to_string())?;
            if source.is_expired {
                return Err("Link expired. Please refresh the link with a new URL.".to_string());
            }
            source.url
        };

        match archive_manager::archive_format_for_media(&media) {
            archive_manager::ArchiveFormat::Zip => {
                let comp_method = zip_compression_method.unwrap_or_default();
                if comp_method == 0 {
                    // Stored ZIP — use streaming proxy with cache support
                    let stream_info = archive_manager::build_archive_stream_info(&media)?;

                    // Build cache spec (same as GDrive path)
                    let (cache_spec, local_cache_path, cache_is_complete) = {
                        let cache_config = build_zip_cache_config(&config);
                        match zip_manager::inspect_stream_cache_target(&media, &cache_config) {
                            Ok(snapshot) => {
                                let local_cache_path = choose_store_zip_local_cache_for_mpv(
                                    &media,
                                    &snapshot,
                                    start_position,
                                );
                                (
                                    Some(zip_stream_proxy::ProxyCacheSpec {
                                        cache_paths: snapshot.paths,
                                        cache_config,
                                        // Direct-link hosts are often heavily throttled per
                                        // connection. If cache priming starts immediately, it
                                        // competes with MPV's first read and slows startup.
                                        start_delay_ms: 5_000,
                                        throttle_delay_ms: 250,
                                    }),
                                    local_cache_path,
                                    snapshot.is_complete,
                                )
                            }
                            Err(_) => (None, None, false),
                        }
                    };

                    let proxy =
                        start_zip_proxy_blocking(zip_stream_proxy::build_direct_link_proxy_spec(
                            ddl_url,
                            &stream_info,
                            cache_spec,
                        ))
                        .await?;

                    if let Some(local_path) = local_cache_path {
                        if cache_is_complete {
                            println!(
                                "[DDL] Using local complete cache for MPV: '{}' -> {}",
                                media.title, local_path
                            );
                            (local_path, None, Some(proxy), None, false)
                        } else {
                            println!(
                                "[DDL] Using streaming proxy with partial cache: '{}' -> {}",
                                media.title,
                                zip_stream_proxy::localhost_stream_url(proxy.port)
                            );
                            (
                                zip_stream_proxy::localhost_stream_url(proxy.port),
                                None,
                                Some(proxy),
                                None,
                                true,
                            )
                        }
                    } else {
                        println!(
                            "[DDL] Using streaming proxy for MPV: '{}' -> {}",
                            media.title,
                            zip_stream_proxy::localhost_stream_url(proxy.port)
                        );
                        (
                            zip_stream_proxy::localhost_stream_url(proxy.port),
                            None,
                            Some(proxy),
                            None,
                            true,
                        )
                    }
                } else {
                    // Compressed ZIP — extract first, then play locally
                    let extracted_path = build_zip_extracted_path(&state, &media).await?;
                    (extracted_path, None, None, None, false)
                }
            }
            _ => {
                // Non-ZIP archives (tar/rar): use extract path
                let stream_info = archive_manager::build_archive_stream_info(&media)?;
                let proxy = start_zip_proxy_blocking(
                    zip_stream_proxy::build_direct_link_proxy_spec(ddl_url, &stream_info, None),
                )
                .await?;
                (
                    zip_stream_proxy::localhost_stream_url(proxy.port),
                    None,
                    Some(proxy),
                    None,
                    true,
                )
            }
        }
    } else if is_cloud {
        if is_zip_media {
            if should_extract_zip_for_mpv(&media, start_position)? {
                let extracted_path = build_zip_extracted_path(&state, &media).await?;
                let is_store_mkv = zip_compression_method.unwrap_or_default() == 0
                    && media
                        .zip_entry_path
                        .as_deref()
                        .or(media.file_path.as_deref())
                        .map(|path| path.to_ascii_lowercase().ends_with(".mkv"))
                        .unwrap_or(false);
                println!(
                    "[ZIP] Using {}cache for MPV: '{}' -> {}",
                    if is_store_mkv {
                        "fully prepared local "
                    } else {
                        "extracted "
                    },
                    media.title,
                    extracted_path
                );
                (extracted_path, None, None, None, false)
            } else {
                match archive_manager::archive_format_for_media(&media) {
                    archive_manager::ArchiveFormat::Zip => match zip_compression_method
                        .unwrap_or_default()
                    {
                        0 => {
                            let stream_info = archive_manager::build_archive_stream_info(&media)?;
                            let drive_url = state
                                .gdrive_client
                                .build_stream_url(&stream_info.zip_file_id);
                            let (
                                cache_spec,
                                local_cache_path,
                                cache_is_complete,
                                use_partial_cache_via_proxy,
                            ) = {
                                let cache_config = build_zip_cache_config(&config);
                                match zip_manager::inspect_stream_cache_target(
                                    &media,
                                    &cache_config,
                                ) {
                                    Ok(snapshot) => {
                                        let local_cache_path = choose_store_zip_local_cache_for_mpv(
                                            &media,
                                            &snapshot,
                                            start_position,
                                        );
                                        let use_partial_cache_via_proxy =
                                            !snapshot.is_complete && local_cache_path.is_some();
                                        (
                                            Some(zip_stream_proxy::ProxyCacheSpec {
                                                cache_paths: snapshot.paths,
                                                cache_config,
                                                start_delay_ms: 0,
                                                throttle_delay_ms: 0,
                                            }),
                                            local_cache_path,
                                            snapshot.is_complete,
                                            use_partial_cache_via_proxy,
                                        )
                                    }
                                    Err(error) => {
                                        println!(
                                            "[ZIP CACHE] Falling back to stream-only proxy for '{}': {:?}",
                                            media.title, error
                                        );
                                        (None, None, false, false)
                                    }
                                }
                            };
                            let proxy =
                                start_zip_proxy_blocking(zip_stream_proxy::build_proxy_spec(
                                    drive_url,
                                    state.gdrive_client.clone(),
                                    &stream_info,
                                    cache_spec,
                                ))
                                .await?;
                            if let Some(local_path) = local_cache_path {
                                if use_partial_cache_via_proxy {
                                    println!(
                                        "[ZIP] Using streaming proxy for MPV with local partial cache assist: '{}' (partial cache: {}) -> {}",
                                        media.title,
                                        local_path,
                                        zip_stream_proxy::localhost_stream_url(proxy.port)
                                    );
                                    (
                                        zip_stream_proxy::localhost_stream_url(proxy.port),
                                        None,
                                        Some(proxy),
                                        None,
                                        true,
                                    )
                                } else {
                                    println!(
                                        "[ZIP] Using local {}cache for MPV: '{}' -> {}",
                                        if cache_is_complete {
                                            "complete "
                                        } else {
                                            "partial "
                                        },
                                        media.title,
                                        local_path
                                    );
                                    (local_path, None, Some(proxy), None, false)
                                }
                            } else {
                                println!(
                                    "[ZIP] Using streaming proxy for MPV: '{}' -> {}",
                                    media.title,
                                    zip_stream_proxy::localhost_stream_url(proxy.port)
                                );
                                (
                                    zip_stream_proxy::localhost_stream_url(proxy.port),
                                    None,
                                    Some(proxy),
                                    None,
                                    true,
                                )
                            }
                        }
                        8 => {
                            let extracted_path = build_zip_extracted_path(&state, &media).await?;
                            (extracted_path, None, None, None, false)
                        }
                        method => {
                            return Err(format!(
                                "ZIP entry compression method {} is not supported for playback",
                                method
                            ));
                        }
                    },
                    archive_manager::ArchiveFormat::Tar | archive_manager::ArchiveFormat::Rar => {
                        let (stream_url, proxy) =
                            build_temporary_zip_stream_url(&state, &media).await?;
                        println!(
                            "[ARCHIVE] Using direct proxy passthrough for MPV: '{}' -> {}",
                            media.title, stream_url
                        );
                        (stream_url, None, Some(proxy), None, true)
                    }
                }
            }
        } else {
            // Cloud non-ZIP file: route through the GDrive stream proxy so MPV
            // never bakes a long-lived token into its --http-header-fields. The
            // proxy signs each upstream request with the live token (refreshes
            // on 401), so 2-hour movies no longer break mid-playback when the
            // initial access_token expires.
            if let Some(ref cloud_file_id) = media.cloud_file_id {
                let proxy = gdrive_stream_proxy::start_gdrive_stream_proxy(
                    state.gdrive_client.clone(),
                    cloud_file_id.clone(),
                )
                .await?;
                let proxy_url = proxy.localhost_url();
                println!(
                    "[MPV] Routing cloud playback through GDrive stream proxy at {} for file_id={}",
                    proxy_url, cloud_file_id
                );
                (proxy_url, None, None, Some(proxy), true)
            } else {
                return Err("Cloud file ID not found".to_string());
            }
        }
    } else {
        // Local file - verify it exists
        let file_path = media
            .file_path
            .clone()
            .ok_or_else(|| "No file path".to_string())?;

        if !std::path::Path::new(&file_path).exists() {
            return Err(format!(
                "Video file not found: {}. The file may have been moved or deleted. Try rescanning your library.",
                file_path
            ));
        }

        (file_path, None, None, None, false)
    };

    config::validate_executable_path(&mpv_path, "mpv")?;
    let pipe_prefix = if is_dev_runtime() {
        "slasshyvault-mpv-dev"
    } else {
        "slasshyvault-mpv"
    };
    let mpv_audio_probe_pipe = if is_zip_media {
        Some(format!(
            r"\\.\pipe\{}-{}-{}",
            pipe_prefix,
            media_id,
            chrono::Utc::now().timestamp_millis()
        ))
    } else {
        None
    };

    // Launch MPV with progress tracking
    let mpv_path_clone = mpv_path.clone();
    let playback_url_clone = playback_url.clone();

    // Launch MPV with tracking (pass auth header and cache settings for cloud files)
    let cache_settings = if playback_is_cloud && config.cloud_cache_enabled {
        config
            .cloud_cache_dir
            .as_ref()
            .map(|dir| mpv_ipc::CloudCacheSettings {
                enabled: true,
                cache_dir: dir.clone(),
                max_size_mb: config.cloud_cache_max_mb,
            })
    } else {
        None
    };

    let display_title = build_mpv_display_title(&media);

    // Resolve effective duration — prefer frontend override (TMDB data already resolved),
    // fall back to DB value, then None.
    let effective_duration = duration_seconds_override
        .filter(|&v| v > 0.0)
        .or(media.duration_seconds);

    // Resolve effective file size — prefer frontend override,
    // then for ZIP media use zip_uncompressed_size (individual entry, not archive),
    // then compressed, then raw file_size_bytes.
    let effective_file_size = if media.parent_zip_id.is_some() {
        file_size_bytes_override
            .filter(|&v| v > 0)
            .or(media.zip_uncompressed_size)
            .or(media.zip_compressed_size)
            .or(media.file_size_bytes)
    } else {
        file_size_bytes_override
            .filter(|&v| v > 0)
            .or(media.file_size_bytes)
    };

    println!(
        "[MPV] Dynamic cache inputs: file_size_bytes={:?}, zip_uncompressed={:?}, zip_compressed={:?}, override_size={:?} -> effective={:?}, duration={:?}, override_duration={:?} -> effective_duration={:?}",
        media.file_size_bytes, media.zip_uncompressed_size, media.zip_compressed_size,
        file_size_bytes_override, effective_file_size, media.duration_seconds,
        duration_seconds_override, effective_duration
    );

    let pid = match mpv_ipc::launch_mpv_with_tracking(
        &mpv_path_clone,
        &playback_url_clone,
        media_id,
        Some(&display_title),
        start_position,
        auth_header.as_deref(),
        cache_settings.as_ref(),
        audio_language.as_deref(),
        subtitle_language.as_deref(),
        subtitle_file_path.as_deref(),
        mpv_audio_probe_pipe.as_deref(),
        effective_file_size,
        effective_duration,
    ) {
        Ok(pid) => pid,
        Err(error) => {
            if let Some(proxy) = zip_proxy {
                let _ = stop_zip_proxy_handle_blocking(proxy).await;
            }
            return Err(error);
        }
    };

    // Store the session
    {
        let mut sessions = state
            .active_mpv_sessions
            .lock()
            .map_err(|e| e.to_string())?;
        sessions.insert(
            media_id,
            ActiveMpvSession {
                session: MpvSession {
                    media_id,
                    pid,
                    title: title.clone(),
                    start_time: chrono::Utc::now().timestamp(),
                },
                zip_proxy,
                gdrive_stream_proxy: gdrive_proxy,
            },
        );
    }

    // Spawn a background thread to monitor MPV and save progress
    let db_path = database::get_database_path();
    let window_clone = window.clone();
    let app_handle = window.app_handle().clone();

    let window_for_tracks = window.clone();
    if let Some(pipe_name) = mpv_audio_probe_pipe {
        let track_window = window_for_tracks.clone();
        std::thread::spawn(move || match detect_tracks_from_running_mpv(&pipe_name) {
            Ok(tracks) => {
                if !tracks.audio_tracks.is_empty() {
                    let payload = MpvAudioTracksDetectedPayload {
                        media_id,
                        series_id,
                        season_number,
                        tracks: tracks.audio_tracks,
                    };
                    let _ = track_window.emit("mpv-audio-tracks-detected", payload);
                }

                if !tracks.subtitle_tracks.is_empty() {
                    let payload = MpvSubtitleTracksDetectedPayload {
                        media_id,
                        series_id,
                        season_number,
                        tracks: tracks.subtitle_tracks,
                    };
                    let _ = track_window.emit("mpv-subtitle-tracks-detected", payload);
                }
            }
            Err(error) => {
                println!(
                    "[MPV] Track detection via playback pipe failed for media {}: {}",
                    media_id, error
                );
            }
        });
    }

    std::thread::spawn(move || {
        println!("[MPV] Starting progress monitor for media ID: {}", media_id);

        if let Ok(db) = database::Database::new(&db_path) {
            let result = mpv_ipc::monitor_mpv_and_save_progress(&db, media_id, pid);

            // Only mark as last_watched when actual playback progress was recorded
            if result.final_position.is_some()
                && result.final_duration.is_some()
                && result.final_duration.unwrap_or(0.0) > 0.0
            {
                let _ = db.update_last_watched(media_id);
            }

            // Emit event to frontend when MPV exits
            let _ = window_clone.emit(
                "mpv-playback-ended",
                serde_json::json!({
                    "media_id": media_id,
                    "title": title,
                    "episode_title": episode_title,
                    "season_number": season_number,
                    "episode_number": episode_number,
                    "media_type": media_type,
                    "final_position": result.final_position,
                    "final_duration": result.final_duration,
                    "completed": result.completed,
                }),
            );

            println!(
                "[MPV] Playback ended for media ID: {}. Completed: {}",
                media_id, result.completed
            );
        }

        {
            let state: tauri::State<'_, AppState> = app_handle.state();
            if let Ok(mut sessions) = state.active_mpv_sessions.lock() {
                if let Some(mut session) = sessions.remove(&media_id) {
                    if let Some(proxy) = session.zip_proxy.as_mut() {
                        proxy.stop();
                    }
                    if let Some(stream_proxy) = session.gdrive_stream_proxy.as_mut() {
                        stream_proxy.stop();
                    }
                }
            };
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

    let vlc_path = config
        .vlc_path
        .as_ref()
        .ok_or_else(|| "VLC path not set. Please configure it in Settings > Player.".to_string())?;

    if vlc_path.is_empty() || !std::path::Path::new(vlc_path).exists() {
        return Err(
            "VLC path not set or invalid. Please configure it in Settings > Player.".to_string(),
        );
    }

    // Security check: Validate VLC executable
    config::validate_executable_path(vlc_path, "vlc")?;

    let (media, resume_info) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let media = db.get_media_by_id(media_id).map_err(|e| e.to_string())?;
        let resume_info = db.get_resume_info(media_id).map_err(|e| e.to_string())?;

        // Update last_watched
        db.update_last_watched(media_id)
            .map_err(|e| e.to_string())?;

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
        let file_path = media
            .file_path
            .clone()
            .ok_or_else(|| "No file path".to_string())?;

        if !std::path::Path::new(&file_path).exists() {
            return Err(format!("File not found: {}", file_path));
        }

        // Add start time if resuming as a global option before the -- separator
        if start_position > 0.0 {
            command.arg(format!("--start-time={:.0}", start_position));
        }

        // Add the -- separator to prevent argument injection
        command.arg("--");

        // Add the file path
        command.arg(&file_path);
    }

    // Launch VLC
    println!("[VLC] Launching with args: {:?}", command);
    command
        .spawn()
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
        let sessions = state
            .active_mpv_sessions
            .lock()
            .map_err(|e| e.to_string())?;
        sessions
            .get(&media_id)
            .map(|session| session.session.clone())
    };

    match session {
        Some(session) => {
            let is_running = mpv_ipc::is_mpv_running(session.pid);
            let progress = mpv_ipc::poll_mpv_progress(media_id);

            // If not running, remove from active sessions
            if !is_running {
                let (zip_proxy, gdrive_proxy) = {
                    let mut sessions = state
                        .active_mpv_sessions
                        .lock()
                        .map_err(|e| e.to_string())?;
                    sessions
                        .remove(&media_id)
                        .map(|mut s| (s.zip_proxy.take(), s.gdrive_stream_proxy.take()))
                }
                .unwrap_or((None, None));
                if let Some(zip) = zip_proxy {
                    let _ = stop_zip_proxy_handle_blocking(zip).await;
                }
                if let Some(gdrv) = gdrive_proxy {
                    println!(
                        "[GDRIVE-PROXY] Stopping cloud-playback proxy for media {}",
                        media_id
                    );
                    gdrv.stop();
                    drop(gdrv);
                }
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
        None => Ok(serde_json::json!({
            "is_playing": false,
            "media_id": media_id,
        })),
    }
}

// Get all active MPV sessions
#[tauri::command]
async fn get_active_mpv_sessions(state: State<'_, AppState>) -> Result<Vec<MpvSession>, String> {
    let proxies_to_stop = {
        let mut sessions = state
            .active_mpv_sessions
            .lock()
            .map_err(|e| e.to_string())?;

        let mut to_remove = Vec::new();
        for (media_id, session) in sessions.iter() {
            if !mpv_ipc::is_mpv_running(session.session.pid) {
                to_remove.push(*media_id);
            }
        }
        let mut proxies = Vec::with_capacity(to_remove.len());
        for id in to_remove {
            if let Some(mut session) = sessions.remove(&id) {
                if let Some(proxy) = session.zip_proxy.take() {
                    proxies.push(proxy);
                }
            }
        }
        proxies
    };

    for proxy in proxies_to_stop {
        let _ = stop_zip_proxy_handle_blocking(proxy).await;
    }

    let sessions = state
        .active_mpv_sessions
        .lock()
        .map_err(|e| e.to_string())?;

    Ok(sessions
        .values()
        .map(|session| session.session.clone())
        .collect())
}

// Get image from cache (returns the file path for asset protocol)
#[tauri::command]
async fn get_cached_image(image_name: String) -> Result<String, String> {
    let cache_dir_str = database::get_image_cache_dir();
    let cache_dir = std::path::Path::new(&cache_dir_str);
    let image_path = cache_dir.join(&image_name);

    println!("[IMAGE] Looking for: {} in {}", image_name, cache_dir_str);
    println!("[IMAGE] Full path: {:?}", image_path);

    // Validate path to prevent traversal
    let canonical_cache = match cache_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => return Err(format!("Cache directory error: {}", e)),
    };

    // Canonicalize the target path - this will fail if the file doesn't exist
    // effectively checking for existence as well
    let canonical_path = match image_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            println!("[IMAGE] Not found: {:?}", image_path);
            return Err("Image not found".to_string());
        }
    };

    // Check if the canonical path starts with the canonical cache path
    if !canonical_path.starts_with(&canonical_cache) {
        println!(
            "[SECURITY] Path traversal attempt blocked: {:?}",
            canonical_path
        );
        return Err("Access denied".to_string());
    }

    let path_cow = canonical_path.to_string_lossy();
    let path_str = path_cow.as_ref();

    #[cfg(windows)]
    let path_str = if path_str.starts_with(r"\\?\") {
        &path_str[4..]
    } else {
        path_str
    };

    let asset_url = format!(
        "asset://localhost/{}",
        path_str.replace("\\", "/").replace(":", "")
    );
    println!("[IMAGE] Found! Asset URL: {}", asset_url);
    Ok(asset_url)
}

// Get image path for Tauri's convertFileSrc (returns raw file path)
#[tauri::command]
async fn get_cached_image_path(image_name: String) -> Result<String, String> {
    let cache_dir_str = database::get_image_cache_dir();
    let cache_dir = std::path::Path::new(&cache_dir_str);
    let image_path = cache_dir.join(&image_name);

    println!(
        "[IMAGE_PATH] Looking for: {} in {}",
        image_name, cache_dir_str
    );
    println!("[IMAGE_PATH] Full path: {:?}", image_path);

    // Validate path to prevent traversal
    let canonical_cache = match cache_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => return Err(format!("Cache directory error: {}", e)),
    };

    let canonical_path = match image_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            println!("[IMAGE_PATH] Not found: {:?}", image_path);
            return Err("Image not found".to_string());
        }
    };

    if !canonical_path.starts_with(&canonical_cache) {
        println!(
            "[SECURITY] Path traversal attempt blocked: {:?}",
            canonical_path
        );
        return Err("Access denied".to_string());
    }

    let path_cow = canonical_path.to_string_lossy();
    let path_str = path_cow.as_ref();

    #[cfg(windows)]
    let path_str = if path_str.starts_with(r"\\?\") {
        &path_str[4..]
    } else {
        path_str
    };

    let final_path = path_str.to_string();
    println!("[IMAGE_PATH] Found! Path: {}", final_path);
    Ok(final_path)
}

// Read video file chunk (workaround for asset protocol issues with WebviewWindows drive letters)
#[tauri::command]
async fn read_video_chunk(
    state: State<'_, AppState>,
    file_path: String,
    offset: u64,
    chunk_size: u64,
) -> Result<Vec<u8>, String> {
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    // Security check: Verify file is in library
    let is_authorized = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.media_exists(&file_path).map_err(|e| e.to_string())?
    };

    if !is_authorized {
        println!(
            "[SECURITY] Blocked access to non-library file: {}",
            file_path
        );
        return Err("Access denied: File not found in library".to_string());
    }

    let mut file = File::open(&file_path).map_err(|e| format!("Failed to open file: {}", e))?;

    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Failed to seek: {}", e))?;

    let mut buffer = vec![0u8; chunk_size as usize];
    let bytes_read = file
        .read(&mut buffer)
        .map_err(|e| format!("Failed to read: {}", e))?;

    buffer.truncate(bytes_read);
    Ok(buffer)
}

#[tauri::command]
async fn get_video_file_size(state: State<'_, AppState>, file_path: String) -> Result<u64, String> {
    // Security check: Verify file is in library
    let is_authorized = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.media_exists(&file_path).map_err(|e| e.to_string())?
    };

    if !is_authorized {
        println!(
            "[SECURITY] Blocked access to non-library file: {}",
            file_path
        );
        return Err("Access denied: File not found in library".to_string());
    }

    let metadata =
        std::fs::metadata(&file_path).map_err(|e| format!("Failed to get file metadata: {}", e))?;
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
// Configured to handle WebviewWindows connection issues (error 10054 - connection reset)
fn http_get_with_retry_auth(
    url: &str,
    credential: &str,
    max_retries: u32,
) -> Result<reqwest::blocking::Response, String> {
    let mut last_error = String::new();
    let use_bearer = is_access_token(credential);

    for attempt in 0..max_retries {
        if attempt > 0 {
            // Exponential backoff: 1000ms, 2000ms, 4000ms...
            let delay_ms = 1000 * (1 << attempt);
            std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
            println!(
                "[HTTP] Retry attempt {} after {}ms delay",
                attempt + 1,
                delay_ms
            );
        }

        // Use the shared global client to avoid creating/dropping a reqwest
        // blocking client inside spawn_blocking (which panics in reqwest 0.12).
        let client = http_client::shared_client();

        let request = if use_bearer {
            client
                .get(url)
                .header("Authorization", format!("Bearer {}", credential))
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
                    println!(
                        "[HTTP] Server error (attempt {}): {}",
                        attempt + 1,
                        last_error
                    );
                }
            }
            Err(e) => {
                last_error = format!("Network error: {}", e);
                println!(
                    "[HTTP] Request failed (attempt {}): {}",
                    attempt + 1,
                    last_error
                );
                // Continue to retry on network errors
            }
        }
    }

    Err(format!(
        "Failed after {} retries: {}",
        max_retries, last_error
    ))
}

// Helper function to perform HTTP GET with retry logic (legacy, no auth header)
// Configured to handle WebviewWindows connection issues (error 10054 - connection reset)
fn http_get_with_retry(url: &str, max_retries: u32) -> Result<reqwest::blocking::Response, String> {
    let mut last_error = String::new();

    for attempt in 0..max_retries {
        if attempt > 0 {
            // Exponential backoff: 1000ms, 2000ms, 4000ms...
            let delay_ms = 1000 * (1 << attempt);
            std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
            println!(
                "[HTTP] Retry attempt {} after {}ms delay",
                attempt + 1,
                delay_ms
            );
        }

        // Use the shared global client to avoid creating/dropping a reqwest
        // blocking client inside spawn_blocking (which panics in reqwest 0.12).
        let client = http_client::shared_client();

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
                    println!(
                        "[HTTP] Server error (attempt {}): {}",
                        attempt + 1,
                        last_error
                    );
                }
            }
            Err(e) => {
                last_error = format!("Network error: {}", e);
                println!(
                    "[HTTP] Request failed (attempt {}): {}",
                    attempt + 1,
                    last_error
                );
                // Continue to retry on network errors
            }
        }
    }

    Err(format!(
        "Failed after {} retries: {}",
        max_retries, last_error
    ))
}

// ============================================
// OMDb Helpers
// ============================================

/// Use user's OMDb API key if provided, otherwise use imdbapi.dev for ratings
fn get_omdb_credential(user_key: &str) -> String {
    user_key.trim().to_string()
}

fn build_omdb_url(credential: &str, imdb_id: &str) -> String {
    if credential.is_empty() {
        // Fallback to imdbapi.dev when no OMDb key configured
        format!("https://api.imdbapi.dev/titles/{}", imdb_id)
    } else {
        format!(
            "https://www.omdbapi.com/?i={}&apikey={}",
            imdb_id, credential
        )
    }
}

#[derive(serde::Deserialize)]
struct OmdbEpisodeRating {
    #[serde(default)]
    imdbRating: Option<String>,
    #[serde(default)]
    imdbVotes: Option<String>,
    #[serde(default)]
    Response: Option<String>,
    #[serde(default)]
    Error: Option<String>,
}

fn fetch_imdb_rating_for_id(credential: &str, imdb_id: &str) -> Option<OmdbEpisodeRating> {
    let url = build_omdb_url(credential, imdb_id);

    let client = http_client::shared_client();

    let response = client.get(&url).send().ok()?;
    if !response.status().is_success() {
        return None;
    }

    let rating: OmdbEpisodeRating = response.json().ok()?;
    if rating.Response.as_deref() == Some("False") {
        return None;
    }

    Some(rating)
}

fn find_tmdb_id_by_imdb_id(tmdb_credential: &str, imdb_id: &str) -> Option<(String, String)> {
    // Returns (tmdb_id, media_type)
    let url = build_tmdb_api_url(
        &format!("/find/{}", imdb_id),
        tmdb_credential,
        "external_source=imdb_id",
    );

    let client = http_client::shared_client();

    let use_bearer = crate::is_access_token(tmdb_credential) && !tmdb_credential.is_empty();

    let req = if use_bearer {
        client
            .get(&url)
            .header("Authorization", format!("Bearer {}", tmdb_credential))
    } else {
        client.get(&url)
    };

    #[derive(serde::Deserialize)]
    struct TmdbFindResult {
        #[serde(default)]
        movie_results: Vec<TmdbFindItem>,
        #[serde(default)]
        tv_results: Vec<TmdbFindItem>,
    }

    #[derive(serde::Deserialize)]
    struct TmdbFindItem {
        id: i64,
    }

    let response = req.send().ok()?;
    if !response.status().is_success() {
        return None;
    }

    let result: TmdbFindResult = response.json().ok()?;

    result
        .movie_results
        .first()
        .map(|r| (r.id.to_string(), "movie".to_string()))
        .or_else(|| {
            result
                .tv_results
                .first()
                .map(|r| (r.id.to_string(), "tv".to_string()))
        })
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
    imdb_id: Option<String>,
}

#[derive(serde::Serialize)]
struct TmdbSearchResponse {
    results: Vec<TmdbSearchResultItem>,
    total_results: usize,
}

#[derive(serde::Serialize)]
struct TmdbTrendingItem {
    id: i64,
    title: String,
    media_type: String,
}

#[derive(serde::Serialize)]
struct TmdbTrendingResponse {
    results: Vec<TmdbTrendingItem>,
}

#[derive(serde::Serialize)]
struct MovieDetails {
    id: i64,
    title: String,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    overview: Option<String>,
    release_date: Option<String>,
    runtime: Option<i32>,
    director: Option<String>,
    vote_average: Option<f64>,
    imdb_id: Option<String>,
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
    season_number: Option<i32>,
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
    first_air_date: Option<String>,
    status: Option<String>,
    number_of_episodes: Option<i32>,
    number_of_seasons: i32,
    seasons: Vec<TvSeasonInfo>,
    creator: Option<String>,
    last_episode_to_air: Option<TvEpisodeInfo>,
    next_episode_to_air: Option<TvEpisodeInfo>,
}

#[derive(serde::Serialize)]
struct TvSeasonDetails {
    season_number: i32,
    name: String,
    episodes: Vec<TvEpisodeInfo>,
}

#[tauri::command]
async fn get_movie_details(
    state: State<'_, AppState>,
    movie_id: i64,
    imdb_id: Option<String>,
) -> Result<MovieDetails, String> {
    let credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    if credential.trim().is_empty() {
        return tokio::task::spawn_blocking(move || -> Result<MovieDetails, String> {
            let imdb_id = imdb_id.clone().or_else(|| {
                if movie_id > 0 {
                    Some(format!("tt{:07}", movie_id))
                } else {
                    None
                }
            });
            if let Some(tt) = imdb_id.as_deref() {
                if let Ok(meta) = cinemeta_api::get_title(cinemeta_api::CinemetaKind::Movie, tt) {
                    return Ok(cinemeta_to_movie_details(&meta));
                }
                if let Ok(meta) = balloonerismm_api::get_movie(tt) {
                    return Ok(balloonerism_to_movie_details(&meta));
                }
            }
            Err("[IMDB] no api_key and no IMDb id — cannot fetch details".into())
        })
        .await
        .map_err(|e| e.to_string())?;
    }

    let url = build_tmdb_api_url(
        &format!("/movie/{}", movie_id),
        &credential,
        "append_to_response=credits",
    );

    let result = tokio::task::spawn_blocking(move || -> Result<MovieDetails, String> {
        let response = http_get_with_retry_auth(&url, &credential, 3)?;

        #[derive(serde::Deserialize)]
        struct TmdbCrewMember {
            job: String,
            name: String,
        }

        #[derive(serde::Deserialize)]
        struct TmdbCredits {
            crew: Option<Vec<TmdbCrewMember>>,
        }

        #[derive(serde::Deserialize)]
        struct RawMovieDetails {
            id: i64,
            title: Option<String>,
            original_title: Option<String>,
            poster_path: Option<String>,
            backdrop_path: Option<String>,
            overview: Option<String>,
            release_date: Option<String>,
            runtime: Option<i32>,
            vote_average: Option<f64>,
            imdb_id: Option<String>,
            credits: Option<TmdbCredits>,
        }

        let raw: RawMovieDetails = response.json().map_err(|e| e.to_string())?;
        let title = raw
            .title
            .clone()
            .or(raw.original_title.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let director = raw
            .credits
            .as_ref()
            .and_then(|c| c.crew.as_ref())
            .and_then(|crew| crew.iter().find(|m| m.job == "Director"))
            .map(|m| m.name.clone());

        Ok(MovieDetails {
            id: raw.id,
            title,
            poster_path: raw.poster_path,
            backdrop_path: raw.backdrop_path,
            overview: raw.overview,
            release_date: raw.release_date,
            runtime: raw.runtime,
            director,
            vote_average: raw.vote_average,
            imdb_id: raw.imdb_id,
        })
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(result)
}

#[tauri::command]
async fn validate_hubdrive_url(url: String) -> Result<serde_json::Value, String> {
    let result = tokio::task::spawn_blocking(move || remote_source::validate_hubdrive_url(&url))
        .await
        .map_err(|e| e.to_string())??;
    Ok(serde_json::json!({ "isValid": result.0, "title": result.1 }))
}

/// Verify a single stream URL via HEAD request (fast, no body download)
#[tauri::command]
async fn verify_stream_url(url: String) -> Result<bool, String> {
    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| e.to_string())?;
        match client.head(&url).send() {
            Ok(resp) => {
                let status = resp.status().as_u16();
                Ok((200..400).contains(&status))
            }
            Err(_) => Ok(false),
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    result
}

/// Verify multiple stream URLs with staggered delays to avoid rate limiting.
/// Returns a map of url -> isAlive.
#[tauri::command]
async fn verify_stream_urls(
    urls: Vec<String>,
) -> Result<std::collections::HashMap<String, bool>, String> {
    let mut results = std::collections::HashMap::new();
    for (i, url) in urls.iter().enumerate() {
        // Stagger requests: 150ms between each to avoid rate limiting
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        let url_clone = url.clone();
        let alive = tokio::task::spawn_blocking(move || -> Result<bool, String> {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()
                .map_err(|e| e.to_string())?;
            match client.head(&url_clone).send() {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    Ok((200..400).contains(&status))
                }
                Err(_) => Ok(false),
            }
        })
        .await
        .map_err(|e| e.to_string())??;
        results.insert(url.clone(), alive);
    }
    Ok(results)
}

fn cinemeta_to_movie_details(t: &cinemeta_api::CinemetaTitle) -> MovieDetails {
    let runtime = t
        .runtime
        .as_deref()
        .and_then(|s| s.split_whitespace().next())
        .and_then(|n| n.parse::<i32>().ok());
    let director = t.director.clone().and_then(|v| v.into_iter().next());
    MovieDetails {
        id: t.moviedb_id.unwrap_or_else(|| {
            t.id.trim_start_matches("tt")
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<i64>()
                .unwrap_or(0)
        }),
        title: t.name.clone(),
        poster_path: t.poster.clone(),
        backdrop_path: t.background.clone(),
        overview: t.description.clone(),
        release_date: t.released.clone().or_else(|| t.year.clone()),
        runtime,
        director,
        vote_average: t.imdb_rating.as_ref().and_then(|s| s.parse::<f64>().ok()),
        imdb_id: t.imdb_id.clone().or_else(|| Some(t.id.clone())),
    }
}

fn balloonerism_to_movie_details(t: &balloonerismm_api::MovieDetail) -> MovieDetails {
    let runtime = t.runtime.filter(|m| *m > 0);
    MovieDetails {
        id: t
            .id
            .trim_start_matches("tt")
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<i64>()
            .unwrap_or(0),
        title: t
            .title
            .clone()
            .or_else(|| t.original_title.clone())
            .unwrap_or_else(|| "Unknown".to_string()),
        // Poster (and backdrop) come from a separate /images call. Both are
        // already absolute Amazon CDN URLs so we use them directly.
        poster_path: None, // populated by the routes that need it via `cache_imdb_image`
        backdrop_path: None,
        overview: t.overview.clone(),
        release_date: t.release_date.clone(),
        runtime,
        director: None, // resolved by the route via /credits
        vote_average: t.vote_average,
        imdb_id: Some(t.id.clone()),
    }
}

fn cinemeta_to_tv_details(t: &cinemeta_api::CinemetaTitle) -> TvShowDetails {
    let videos = t.videos.clone().unwrap_or_default();
    let mut seasons: Vec<TvSeasonInfo> = Vec::new();
    let mut last_episode: Option<RawAirEpisodeShape> = None;
    let mut next_episode: Option<RawAirEpisodeShape> = None;
    let mut total_episodes: i32 = 0;
    for v in &videos {
        if let Some(s) = v.season {
            if s > 0 {
                total_episodes += 1;
                if last_episode.is_none()
                    || v.episode.unwrap_or(0) > last_episode.as_ref().unwrap().episode_number
                {
                    last_episode = Some(RawAirEpisodeShape {
                        season_number: v.season,
                        episode_number: v.episode.unwrap_or(v.number.unwrap_or(0)),
                        name: v.name.clone(),
                        overview: v.overview.clone().or_else(|| v.description.clone()),
                        still_path: v.thumbnail.clone(),
                        air_date: v.released.clone().or_else(|| v.first_aired.clone()),
                        runtime: None,
                        vote_average: v.rating.as_ref().and_then(|r| r.parse().ok()),
                    });
                }
            }
        }
    }
    if seasons.is_empty() {
        let mut by_season: std::collections::BTreeMap<i32, Vec<RawAirEpisodeShape>> =
            std::collections::BTreeMap::new();
        for v in &videos {
            if let Some(s) = v.season {
                let entry = by_season.entry(s).or_default();
                entry.push(RawAirEpisodeShape {
                    season_number: v.season,
                    episode_number: v.episode.unwrap_or(v.number.unwrap_or(0)),
                    name: v.name.clone(),
                    overview: v.overview.clone().or_else(|| v.description.clone()),
                    still_path: v.thumbnail.clone(),
                    air_date: v.released.clone().or_else(|| v.first_aired.clone()),
                    runtime: None,
                    vote_average: v.rating.as_ref().and_then(|r| r.parse().ok()),
                });
            }
        }
        for (s, eps) in by_season {
            let name = format!("Season {}", s);
            let ep_count = eps.len() as i32;
            seasons.push(TvSeasonInfo {
                season_number: s,
                name,
                episode_count: ep_count,
                overview: None,
                poster_path: None,
                air_date: None,
            });
        }
    } else {
        let mut by_season: std::collections::BTreeMap<i32, Vec<RawAirEpisodeShape>> =
            std::collections::BTreeMap::new();
        for v in &videos {
            if let Some(s) = v.season {
                if s == 0 {
                    continue;
                }
                by_season.entry(s).or_default().push(RawAirEpisodeShape {
                    season_number: v.season,
                    episode_number: v.episode.unwrap_or(v.number.unwrap_or(0)),
                    name: v.name.clone(),
                    overview: v.overview.clone().or_else(|| v.description.clone()),
                    still_path: v.thumbnail.clone(),
                    air_date: v.released.clone().or_else(|| v.first_aired.clone()),
                    runtime: None,
                    vote_average: v.rating.as_ref().and_then(|r| r.parse().ok()),
                });
            }
        }
        seasons.clear();
        for (s, eps) in by_season {
            let name = format!("Season {}", s);
            seasons.push(TvSeasonInfo {
                season_number: s,
                name,
                episode_count: eps.len() as i32,
                overview: None,
                poster_path: None,
                air_date: None,
            });
        }
    }
    let map_to_ep = |e: RawAirEpisodeShape| TvEpisodeInfo {
        season_number: e.season_number,
        episode_number: e.episode_number,
        name: e
            .name
            .unwrap_or_else(|| format!("Episode {}", e.episode_number)),
        overview: e.overview,
        still_path: e.still_path,
        air_date: e.air_date,
        runtime: e.runtime,
        vote_average: e.vote_average,
    };
    TvShowDetails {
        id: t.moviedb_id.unwrap_or_else(|| {
            t.id.trim_start_matches("tt")
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<i64>()
                .unwrap_or(0)
        }),
        name: t.name.clone(),
        poster_path: t.poster.clone(),
        backdrop_path: t.background.clone(),
        overview: t.description.clone(),
        first_air_date: t.released.clone().or_else(|| t.year.clone()),
        status: t.status.clone(),
        number_of_episodes: Some(total_episodes),
        number_of_seasons: seasons.len() as i32,
        seasons,
        creator: None,
        last_episode_to_air: last_episode.map(|e| map_to_ep(e.clone())),
        next_episode_to_air: next_episode.map(map_to_ep),
    }
}

#[derive(Clone)]
struct RawAirEpisodeShape {
    season_number: Option<i32>,
    episode_number: i32,
    name: Option<String>,
    overview: Option<String>,
    still_path: Option<String>,
    air_date: Option<String>,
    runtime: Option<i32>,
    vote_average: Option<f64>,
}

fn balloonerism_to_tv_details(t: &balloonerismm_api::TvDetail) -> TvShowDetails {
    let runtime = t
        .episode_run_time
        .as_ref()
        .and_then(|arr| arr.first().copied())
        .map(|m| m * 60); // store in seconds (matches MovieDetails.runtime convention used elsewhere)
    let seasons_count = t
        .number_of_seasons
        .unwrap_or_else(|| t.seasons.as_ref().map(|s| s.len() as i32).unwrap_or(0));
    TvShowDetails {
        id: t
            .id
            .trim_start_matches("tt")
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<i64>()
            .unwrap_or(0),
        name: t
            .name
            .clone()
            .or_else(|| t.original_name.clone())
            .unwrap_or_else(|| "Unknown".to_string()),
        poster_path: None,
        backdrop_path: None,
        overview: t.overview.clone(),
        first_air_date: t.first_air_date.clone(),
        status: if t.in_production.unwrap_or(false) {
            Some("Returning Series".to_string())
        } else {
            Some("Ended".to_string())
        },
        number_of_episodes: t.number_of_episodes.map(|n| n),
        number_of_seasons: seasons_count,
        seasons: t
            .seasons
            .as_ref()
            .map(|hs| {
                hs.iter()
                    .map(|h| TvSeasonInfo {
                        season_number: h.season_number,
                        name: format!("Season {}", h.season_number),
                        episode_count: 0, // populated by /season/{n} detail calls
                        overview: None,
                        poster_path: None,
                        air_date: None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        creator: None,
        last_episode_to_air: None,
        next_episode_to_air: None,
        // runtime is not part of TvShowDetails — leave as None; consumers
        // can compute from episode_run_time.
        ..default_tv_show_details_for_unused_runtime(runtime)
    }
}

fn default_tv_show_details_for_unused_runtime(_runtime: Option<i32>) -> TvShowDetails {
    // Tiny shim so the `..` struct-updater compiles when TvShowDetails fields
    // change. Returns a default-initialized value; runtime is ignored.
    TvShowDetails {
        id: 0,
        name: String::new(),
        poster_path: None,
        backdrop_path: None,
        overview: None,
        first_air_date: None,
        status: None,
        number_of_episodes: None,
        number_of_seasons: 0,
        seasons: Vec::new(),
        creator: None,
        last_episode_to_air: None,
        next_episode_to_air: None,
    }
}

#[tauri::command]
async fn resolve_imdb_id(
    state: State<'_, AppState>,
    tmdb_id: i64,
    media_type: String,
) -> Result<Option<String>, String> {
    let api_key = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        config.tmdb_api_key.clone().unwrap_or_default()
    };
    if api_key.trim().is_empty() {
        return Ok(None);
    }
    let result =
        tokio::task::spawn_blocking(move || tmdb::fetch_imdb_id(&api_key, tmdb_id, &media_type))
            .await
            .map_err(|e| e.to_string())?;
    Ok(result)
}

// Get TV show details including seasons
#[tauri::command]
async fn get_tv_details(
    state: State<'_, AppState>,
    tv_id: i64,
    imdb_id: Option<String>,
) -> Result<TvShowDetails, String> {
    let credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    if credential.trim().is_empty() {
        return tokio::task::spawn_blocking(move || -> Result<TvShowDetails, String> {
            let imdb_id = imdb_id.clone().or_else(|| {
                if tv_id > 0 {
                    Some(format!("tt{:07}", tv_id))
                } else {
                    None
                }
            });
            if let Some(tt) = imdb_id.as_deref() {
                if let Ok(meta) = cinemeta_api::get_title(cinemeta_api::CinemetaKind::Series, tt) {
                    return Ok(cinemeta_to_tv_details(&meta));
                }
                if let Ok(meta) = balloonerismm_api::get_tv(tt) {
                    return Ok(balloonerism_to_tv_details(&meta));
                }
            }
            Err("[BALLOON] no api_key and no IMDb id — cannot fetch TV details".into())
        })
        .await
        .map_err(|e| e.to_string())?;
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
        struct TmdbCreator {
            name: String,
        }

        #[derive(serde::Deserialize)]
        struct RawTvShow {
            id: i64,
            name: Option<String>,
            poster_path: Option<String>,
            backdrop_path: Option<String>,
            overview: Option<String>,
            first_air_date: Option<String>,
            status: Option<String>,
            number_of_episodes: Option<i32>,
            number_of_seasons: Option<i32>,
            seasons: Option<Vec<RawSeason>>,
            created_by: Option<Vec<TmdbCreator>>,
            last_episode_to_air: Option<RawAirEpisode>,
            next_episode_to_air: Option<RawAirEpisode>,
        }

        #[derive(serde::Deserialize)]
        struct RawAirEpisode {
            season_number: Option<i32>,
            episode_number: i32,
            name: Option<String>,
            overview: Option<String>,
            still_path: Option<String>,
            air_date: Option<String>,
            runtime: Option<i32>,
            vote_average: Option<f64>,
        }

        let raw: RawTvShow = response.json().map_err(|e| e.to_string())?;
        let title = raw.name.clone().unwrap_or_else(|| "Unknown".to_string());

        let creator = raw
            .created_by
            .as_ref()
            .and_then(|c| c.first())
            .map(|c| c.name.clone());

        let seasons: Vec<TvSeasonInfo> = raw
            .seasons
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.season_number > 0) // Filter out specials (season 0)
            .map(|s| TvSeasonInfo {
                season_number: s.season_number,
                name: s
                    .name
                    .unwrap_or_else(|| format!("Season {}", s.season_number)),
                episode_count: s.episode_count,
                overview: s.overview,
                poster_path: s.poster_path,
                air_date: s.air_date,
            })
            .collect();

        let map_air_episode = |episode: RawAirEpisode| TvEpisodeInfo {
            season_number: episode.season_number,
            episode_number: episode.episode_number,
            name: episode
                .name
                .unwrap_or_else(|| format!("Episode {}", episode.episode_number)),
            overview: episode.overview,
            still_path: episode.still_path,
            air_date: episode.air_date,
            runtime: episode.runtime,
            vote_average: episode.vote_average,
        };

        Ok(TvShowDetails {
            id: raw.id,
            name: title,
            poster_path: raw.poster_path,
            backdrop_path: raw.backdrop_path,
            overview: raw.overview,
            first_air_date: raw.first_air_date,
            status: raw.status,
            number_of_episodes: raw.number_of_episodes,
            number_of_seasons: raw.number_of_seasons.unwrap_or(0),
            seasons,
            creator,
            last_episode_to_air: raw.last_episode_to_air.map(map_air_episode),
            next_episode_to_air: raw.next_episode_to_air.map(map_air_episode),
        })
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(result)
}

// Get episodes for a specific season of a TV show
#[tauri::command]
async fn get_tv_season_episodes(
    state: State<'_, AppState>,
    tv_id: i64,
    season_number: i32,
    imdb_id: Option<String>,
) -> Result<TvSeasonDetails, String> {
    // First, try to get from local cache
    let tv_id_str = tv_id.to_string();
    let _image_cache_dir = database::get_image_cache_dir();
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        if let Ok(cached_episodes) = db.get_cached_episodes_for_season(&tv_id_str, season_number) {
            if !cached_episodes.is_empty() {
                println!(
                    "[CACHE] Using cached episode data for TV {} Season {}",
                    tv_id, season_number
                );
                let episodes: Vec<TvEpisodeInfo> = cached_episodes
                    .into_iter()
                    .map(|e| TvEpisodeInfo {
                        season_number: Some(season_number),
                        episode_number: e.episode_number,
                        name: e
                            .episode_title
                            .unwrap_or_else(|| format!("Episode {}", e.episode_number)),
                        overview: e.overview,
                        still_path: e.still_path,
                        air_date: e.air_date,
                        runtime: None,
                        vote_average: e.vote_average,
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

    println!(
        "[TV EPISODES] Cache miss, fetching season {} for TV {}",
        season_number, tv_id
    );

    let credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    if credential.trim().is_empty() {
        let resolved_imdb_id = imdb_id
            .clone()
            .or_else(|| {
                let db = state.db.lock().ok()?;
                db.find_media_by_tmdb(&tv_id.to_string(), "tvshow")
                    .ok()
                    .flatten()
                    .and_then(|m| m.imdb_id)
            })
            .or_else(|| {
                if tv_id > 0 {
                    Some(format!("tt{:07}", tv_id))
                } else {
                    None
                }
            });
        if let Some(tt) = resolved_imdb_id.as_deref() {
            if let Ok(meta) = cinemeta_api::get_title(cinemeta_api::CinemetaKind::Series, tt) {
                let mut episodes: Vec<TvEpisodeInfo> = Vec::new();
                for v in meta.videos.unwrap_or_default() {
                    if v.season == Some(season_number) {
                        episodes.push(TvEpisodeInfo {
                            season_number: Some(season_number),
                            episode_number: v.episode.unwrap_or(v.number.unwrap_or(0)),
                            name: v.name.unwrap_or_else(|| {
                                format!(
                                    "S{:02}E{:02}",
                                    season_number,
                                    v.episode.unwrap_or(v.number.unwrap_or(0))
                                )
                            }),
                            overview: v.overview.or(v.description),
                            still_path: v.thumbnail,
                            air_date: v.released.or(v.first_aired),
                            runtime: None,
                            vote_average: v.rating.and_then(|s| s.parse().ok()),
                        });
                    }
                }
                if !episodes.is_empty() {
                    return Ok(TvSeasonDetails {
                        season_number,
                        name: format!("Season {}", season_number),
                        episodes,
                    });
                }
            }
        }
        return Err(format!(
            "[IMDB] no api_key and no IMDb id for tv_id={}",
            tv_id
        ));
    }

    // Try to get show's imdb_id from database for imdbapi.dev image fallback
    let show_imdb_id_from_db: Option<String> = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.find_media_by_tmdb(&tv_id.to_string(), "tvshow")
            .ok()
            .flatten()
            .and_then(|m| m.imdb_id)
    };

    let url = build_tmdb_api_url(
        &format!("/tv/{}/season/{}", tv_id, season_number),
        &credential,
        "",
    );

    // ALL blocking HTTP in std::thread::spawn to avoid tokio runtime panic
    let (ep_tx, ep_rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let result: Result<TvSeasonDetails, String> = (|| {
            let response = http_get_with_retry_auth(&url, &credential, 3)?;

            #[derive(serde::Deserialize)]
            struct RawEpisode {
                season_number: Option<i32>,
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

            let image_cache_dir = database::get_image_cache_dir();
            let episodes: Vec<TvEpisodeInfo> = raw
                .episodes
                .unwrap_or_default()
                .into_iter()
                .map(|e| {
                    let _cached_still = if let Some(ref tmdb_path) = e.still_path {
                        if !tmdb_path.is_empty() {
                            let image_type = tmdb::ImageType::EpisodeBanner {
                                season: raw.season_number,
                                episode: e.episode_number,
                            };
                            tmdb::cache_image_organized(
                                tmdb_path,
                                &image_cache_dir,
                                "episode",
                                image_type,
                            )
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    TvEpisodeInfo {
                        season_number: e.season_number.or(Some(raw.season_number)),
                        episode_number: e.episode_number,
                        name: e
                            .name
                            .unwrap_or_else(|| format!("Episode {}", e.episode_number)),
                        overview: e.overview,
                        still_path: e.still_path,
                        air_date: e.air_date,
                        runtime: e.runtime,
                        vote_average: e.vote_average,
                    }
                })
                .collect();

            Ok(TvSeasonDetails {
                season_number: raw.season_number,
                name: raw
                    .name
                    .unwrap_or_else(|| format!("Season {}", raw.season_number)),
                episodes,
            })
        })();
        let _ = ep_tx.send(result);
    });
    let mut result = ep_rx.await.map_err(|e| e.to_string())??;

    // Cache the fetched episode data so subsequent calls don't miss
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        for ep in &result.episodes {
            let _ = db.save_cached_episode_metadata(
                &tv_id_str,
                season_number,
                ep.episode_number,
                Some(&ep.name),
                ep.overview.as_deref(),
                ep.still_path.as_deref(),
                ep.air_date.as_deref(),
                ep.vote_average,
            );
        }
    }

    // Try to get better episode images from imdbapi.dev (also in thread to avoid panic)
    if let Some(ref show_imdb_id) = show_imdb_id_from_db {
        println!(
            "[IMDBAPI] Fetching episode images for show {} season {}",
            show_imdb_id, season_number
        );
        let show_imdb_id_clone = show_imdb_id.clone();
        let (img_tx, img_rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let mut image_count = 0usize;
            let mut results: Vec<(i32, String)> = Vec::new();
            let imdb_url = format!(
                "https://api.imdbapi.dev/titles/{}/episodes?season={}",
                show_imdb_id_clone, season_number
            );
            if let Ok(resp) = http_client::shared_client().get(&imdb_url).send() {
                if let Ok(json) = resp.json::<serde_json::Value>() {
                    if let Some(episodes) = json.get("episodes").and_then(|e| e.as_array()) {
                        for ep in episodes {
                            if let (Some(ep_num), Some(img_url)) = (
                                ep.get("episodeNumber").and_then(|n| n.as_i64()),
                                ep.get("primaryImage")
                                    .and_then(|i| i.get("url"))
                                    .and_then(|u| u.as_str()),
                            ) {
                                let image_cache_dir = database::get_image_cache_dir();
                                let image_type = tmdb::ImageType::EpisodeBanner {
                                    season: season_number,
                                    episode: ep_num as i32,
                                };
                                if let Some(cached) = tmdb::cache_imdb_image(
                                    img_url,
                                    std::path::Path::new(&image_cache_dir),
                                    &image_type,
                                ) {
                                    image_count += 1;
                                    results.push((ep_num as i32, cached));
                                }
                            }
                        }
                    }
                }
            }
            println!(
                "[IMDBAPI] Got {} episode images from imdbapi.dev",
                image_count
            );
            let _ = img_tx.send(results);
        });
        // Apply cached images to result
        if let Ok(results) = img_rx.await {
            for (ep_num, path) in results {
                for info in &mut result.episodes {
                    if info.episode_number == ep_num {
                        info.still_path = Some(path.clone());
                    }
                }
            }
        }
    }

    Ok(result)
}

#[derive(serde::Serialize, Clone)]
struct ImdbEpisodeRating {
    imdb_id: String,
    imdb_rating: Option<f64>,
    imdb_votes: Option<i64>,
    still_url: Option<String>,
    title: Option<String>,
    plot: Option<String>,
}

#[tauri::command]
async fn get_episode_imdb_ratings(
    state: State<'_, AppState>,
    tv_id: i64,
    season_number: i32,
    episode_numbers: Vec<i32>,
    show_imdb_id: Option<String>,
) -> Result<std::collections::HashMap<i32, ImdbEpisodeRating>, String> {
    let omdb_credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        get_omdb_credential(&config.omdb_api_key.clone().unwrap_or_default())
    };

    let tmdb_credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    // ALL blocking HTTP in std::thread::spawn to avoid tokio runtime panic (reqwest 0.12)
    let (ratings_tx, ratings_rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut results: std::collections::HashMap<i32, ImdbEpisodeRating> =
            std::collections::HashMap::new();
        let mut missing_from_imdbapi: Vec<i32> = Vec::new();

        // Resolve show IMDb ID from TMDB if not provided
        let show_imdb_id = match show_imdb_id {
            Some(id) => Some(id),
            None => {
                let ext_url = build_tmdb_api_url(
                    &format!("/tv/{}/external_ids", tv_id),
                    &tmdb_credential,
                    "",
                );
                #[derive(serde::Deserialize)]
                struct ShowExternalIds {
                    imdb_id: Option<String>,
                }
                let client = http_client::shared_client();
                let use_bearer = crate::is_access_token(&tmdb_credential);
                let req = if use_bearer {
                    client
                        .get(&ext_url)
                        .header("Authorization", format!("Bearer {}", &tmdb_credential))
                } else {
                    client.get(&ext_url)
                };
                match req.send() {
                    Ok(resp) if resp.status().is_success() => resp
                        .json::<ShowExternalIds>()
                        .ok()
                        .and_then(|ids| ids.imdb_id)
                        .filter(|id| !id.trim().is_empty()),
                    _ => None,
                }
            }
        };

        // Step 1: Try imdbapi.dev batch fetch if we have the show's IMDb ID
        if let Some(ref show_id) = show_imdb_id {
            println!(
                "[IMDBAPI] Fetching episode ratings for show {} season {}",
                show_id, season_number
            );
            let url = format!(
                "https://api.imdbapi.dev/titles/{}/episodes?season={}",
                show_id, season_number
            );
            let client = http_client::shared_client();
            let resp = client
                .get(&url)
                .timeout(std::time::Duration::from_secs(10))
                .send();

            #[derive(serde::Deserialize)]
            struct ImdbApiEpisode {
                #[serde(default)]
                id: Option<String>,
                #[serde(default)]
                episodeNumber: Option<i32>,
                #[serde(default)]
                rating: Option<ImdbApiEpisodeRating>,
                #[serde(default)]
                title: Option<String>,
                #[serde(default)]
                plot: Option<String>,
                #[serde(default)]
                runtimeSeconds: Option<i32>,
                #[serde(default)]
                season: Option<String>,
                #[serde(default)]
                primaryImage: Option<PrimaryImage>,
            }
            #[derive(serde::Deserialize)]
            struct ImdbApiEpisodeRating {
                #[serde(default)]
                aggregateRating: Option<f64>,
                #[serde(default)]
                voteCount: Option<i64>,
            }
            #[derive(serde::Deserialize)]
            struct PrimaryImage {
                #[serde(default)]
                url: Option<String>,
                #[serde(default)]
                width: Option<i32>,
                #[serde(default)]
                height: Option<i32>,
            }

            match resp {
                Ok(r) if r.status().is_success() => {
                    #[derive(serde::Deserialize)]
                    struct EpisodesResponse {
                        #[serde(default)]
                        episodes: Vec<ImdbApiEpisode>,
                    }
                    if let Ok(data) = r.json::<EpisodesResponse>() {
                        let episode_set: std::collections::HashSet<i32> =
                            episode_numbers.iter().copied().collect();
                        for ep in data.episodes {
                            let ep_num = match ep.episodeNumber {
                                Some(n) if episode_set.contains(&n) => n,
                                _ => continue,
                            };
                            if let (Some(imdb_id), Some(rating)) = (ep.id, ep.rating) {
                                if rating.aggregateRating.is_some() {
                                    results.insert(
                                        ep_num,
                                        ImdbEpisodeRating {
                                            imdb_id,
                                            imdb_rating: rating.aggregateRating,
                                            imdb_votes: rating.voteCount,
                                            still_url: ep.primaryImage.and_then(|img| img.url),
                                            title: ep.title.clone(),
                                            plot: ep.plot.clone(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                    println!(
                        "[IMDBAPI] Got {} episode ratings from imdbapi.dev",
                        results.len()
                    );
                }
                _ => {}
            }

            // Track which episodes we still need
            for &ep_num in &episode_numbers {
                if !results.contains_key(&ep_num) {
                    missing_from_imdbapi.push(ep_num);
                }
            }
        } else {
            missing_from_imdbapi = episode_numbers.clone();
        }

        // Step 2: For missing episodes, resolve IMDb ID via TMDB and try OMDb
        for ep_num in &missing_from_imdbapi {
            let ext_url = build_tmdb_api_url(
                &format!(
                    "/tv/{}/season/{}/episode/{}/external_ids",
                    tv_id, season_number, ep_num
                ),
                &tmdb_credential,
                "",
            );

            #[derive(serde::Deserialize)]
            struct EpisodeExternalIds {
                #[serde(default)]
                imdb_id: Option<String>,
            }

            let imdb_id: Option<String> = {
                let client = http_client::shared_client();
                let use_bearer = crate::is_access_token(&tmdb_credential);
                let req = if use_bearer {
                    client
                        .get(&ext_url)
                        .header("Authorization", format!("Bearer {}", &tmdb_credential))
                } else {
                    client.get(&ext_url)
                };
                match req.send() {
                    Ok(resp) if resp.status().is_success() => resp
                        .json::<EpisodeExternalIds>()
                        .ok()
                        .and_then(|ids| ids.imdb_id)
                        .filter(|id| !id.trim().is_empty()),
                    _ => None,
                }
            };

            let Some(imdb_id) = imdb_id else { continue };

            // Try OMDb
            println!(
                "[OMDB] Fallback: fetching rating for episode {} via OMDb (imdb_id: {})",
                ep_num, imdb_id
            );
            let rating = fetch_imdb_rating_for_id(&omdb_credential, &imdb_id);
            let parsed_rating = rating
                .as_ref()
                .and_then(|r| r.imdbRating.as_deref().and_then(|v| v.parse::<f64>().ok()));
            let parsed_votes = rating.as_ref().and_then(|r| {
                r.imdbVotes
                    .as_deref()
                    .and_then(|v| v.replace(',', "").parse::<i64>().ok())
            });

            results.insert(
                *ep_num,
                ImdbEpisodeRating {
                    imdb_id,
                    imdb_rating: parsed_rating,
                    imdb_votes: parsed_votes,
                    still_url: None,
                    title: None,
                    plot: None,
                },
            );
        }

        let _ = ratings_tx.send(results);
    });
    let results = ratings_rx.await.map_err(|e| e.to_string())?;

    Ok(results)
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct ImdbAward {
    event: String,
    year: Option<i32>,
    category: String,
    is_winner: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct ImdbDetails {
    imdb_id: String,
    title: Option<String>,
    aggregate_rating: Option<f64>,
    vote_count: Option<i64>,
    metacritic_score: Option<i32>,
    metacritic_url: Option<String>,
    metacritic_review_count: Option<i32>,
    plot: Option<String>,
    genres: Option<Vec<String>>,
    directors: Option<Vec<String>>,
    writers: Option<Vec<String>>,
    stars: Option<Vec<String>>,
    runtime_seconds: Option<i32>,
    origin_countries: Option<Vec<String>>,
    interests: Option<Vec<String>>,
    start_year: Option<i32>,
    end_year: Option<i32>,
    mpaa_rating: Option<String>,
    domestic_gross: Option<String>,
    worldwide_gross: Option<String>,
    opening_weekend_gross: Option<String>,
    production_budget: Option<String>,
    total_nominations: Option<i32>,
    total_wins: Option<i32>,
    awards: Option<Vec<ImdbAward>>,
    primary_image_url: Option<String>,
}

#[tauri::command]
async fn get_imdb_details(
    state: State<'_, AppState>,
    imdb_id: Option<String>,
    tmdb_id: Option<i64>,
    media_type: Option<String>,
) -> Result<ImdbDetails, String> {
    // Extract credentials from state before spawn_blocking
    let tmdb_credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    // All blocking HTTP work must happen off the tokio async runtime thread
    let details = tokio::task::spawn_blocking(move || {
        // Resolve IMDb ID
        let resolved_imdb_id = if let Some(id) = imdb_id {
            id
        } else if let Some(tid) = tmdb_id {
            let mtype = media_type.unwrap_or_else(|| "movie".to_string());
            let ext_path = if mtype == "tv" {
                format!("/tv/{}/external_ids", tid)
            } else {
                format!("/movie/{}/external_ids", tid)
            };
            let ext_url = build_tmdb_api_url(&ext_path, &tmdb_credential, "");

            #[derive(serde::Deserialize)]
            struct ExternalIds {
                imdb_id: Option<String>,
            }

            let client = http_client::shared_client();
            let use_bearer = crate::is_access_token(&tmdb_credential);
            let req = if use_bearer {
                client
                    .get(&ext_url)
                    .header("Authorization", format!("Bearer {}", &tmdb_credential))
            } else {
                client.get(&ext_url)
            };
            let resp = req.send().map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("TMDB external_ids error: {}", resp.status()));
            }
            let ids: ExternalIds = resp.json().map_err(|e| e.to_string())?;
            let resolved = ids
                .imdb_id
                .ok_or_else(|| "No IMDb ID found for this TMDB entry".to_string())?;
            println!("[TMDB] Resolved imdb_id {} from tmdb_id {}", resolved, tid);
            resolved
        } else {
            return Err("Either imdb_id or tmdb_id must be provided".to_string());
        };

        let mut details = ImdbDetails {
            imdb_id: resolved_imdb_id.clone(),
            ..Default::default()
        };

        #[derive(serde::Deserialize, Clone)]
        struct MoneyAmount {
            amount: Option<f64>,
            currency: Option<String>,
        }

        // 1. Title details
        println!("[IMDBAPI] Fetching titles for {}", resolved_imdb_id);
        let title_url = format!("https://api.imdbapi.dev/titles/{}", resolved_imdb_id);
        let client = http_client::shared_client();
        if let Ok(resp) = client
            .get(&title_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
        {
            if resp.status().is_success() {
                #[derive(serde::Deserialize)]
                struct PrimaryImage {
                    url: Option<String>,
                    width: Option<i32>,
                    height: Option<i32>,
                }

                #[derive(serde::Deserialize)]
                struct TitleResponse {
                    primaryImage: Option<PrimaryImage>,
                    primaryTitle: Option<String>,
                    startYear: Option<i32>,
                    endYear: Option<i32>,
                    runtimeSeconds: Option<i32>,
                    genres: Option<Vec<String>>,
                    interests: Option<Vec<InterestItem>>,
                    plot: Option<String>,
                    originCountries: Option<Vec<CountryItem>>,
                    rating: Option<TitleRating>,
                    metacritic: Option<MetacriticInfo>,
                    directors: Option<Vec<PersonName>>,
                    writers: Option<Vec<PersonName>>,
                    stars: Option<Vec<PersonName>>,
                }
                #[derive(serde::Deserialize)]
                struct TitleRating {
                    aggregateRating: Option<f64>,
                    voteCount: Option<i64>,
                }
                #[derive(serde::Deserialize)]
                struct MetacriticInfo {
                    url: Option<String>,
                    score: Option<i32>,
                    reviewCount: Option<i32>,
                }
                #[derive(serde::Deserialize)]
                struct PersonName {
                    displayName: Option<String>,
                }
                #[derive(serde::Deserialize)]
                struct CountryItem {
                    name: Option<String>,
                }
                #[derive(serde::Deserialize)]
                struct InterestItem {
                    name: Option<String>,
                }

                if let Ok(title) = resp.json::<TitleResponse>() {
                    details.title = title.primaryTitle;
                    details.start_year = title.startYear;
                    details.end_year = title.endYear;
                    details.runtime_seconds = title.runtimeSeconds;
                    details.genres = title.genres;
                    details.interests = title
                        .interests
                        .map(|v| v.into_iter().filter_map(|i| i.name).collect());
                    details.plot = title.plot;
                    details.origin_countries = title
                        .originCountries
                        .map(|v| v.into_iter().filter_map(|c| c.name).collect());
                    details.directors = title
                        .directors
                        .map(|v| v.into_iter().filter_map(|p| p.displayName).collect());
                    details.writers = title
                        .writers
                        .map(|v| v.into_iter().filter_map(|p| p.displayName).collect());
                    details.stars = title
                        .stars
                        .map(|v| v.into_iter().filter_map(|p| p.displayName).collect());
                    if let Some(r) = title.rating {
                        details.aggregate_rating = r.aggregateRating;
                        details.vote_count = r.voteCount;
                    }
                    if let Some(m) = title.metacritic {
                        details.metacritic_score = m.score;
                        details.metacritic_url = m.url;
                        details.metacritic_review_count = m.reviewCount;
                    }
                    details.primary_image_url = title.primaryImage.and_then(|img| img.url);
                    println!("[IMDBAPI] Got titles data for {}", resolved_imdb_id);
                }
            }
        }

        // 2. Certificates (MPAA rating)
        println!("[IMDBAPI] Fetching certificates for {}", resolved_imdb_id);
        let certs_url = format!(
            "https://api.imdbapi.dev/titles/{}/certificates",
            resolved_imdb_id
        );
        if let Ok(resp) = client
            .get(&certs_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
        {
            if resp.status().is_success() {
                #[derive(serde::Deserialize)]
                struct CertificatesResponse {
                    certificates: Option<Vec<CertificateEntry>>,
                }
                #[derive(serde::Deserialize)]
                struct CertificateEntry {
                    country: Option<CountryCode>,
                    rating: Option<String>,
                }
                #[derive(serde::Deserialize)]
                struct CountryCode {
                    code: Option<String>,
                }

                if let Ok(data) = resp.json::<CertificatesResponse>() {
                    if let Some(cert_list) = data.certificates {
                        details.mpaa_rating = cert_list
                            .iter()
                            .find(|c| {
                                c.country.as_ref().and_then(|co| co.code.as_deref()) == Some("US")
                            })
                            .and_then(|c| c.rating.clone())
                            .or_else(|| cert_list.first().and_then(|c| c.rating.clone()));
                    }
                    println!("[IMDBAPI] Got certificates data for {}", resolved_imdb_id);
                }
            }
        }

        // 3. Box office
        println!("[IMDBAPI] Fetching boxOffice for {}", resolved_imdb_id);
        let box_url = format!(
            "https://api.imdbapi.dev/titles/{}/boxOffice",
            resolved_imdb_id
        );
        if let Ok(resp) = client
            .get(&box_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
        {
            if resp.status().is_success() {
                #[derive(serde::Deserialize)]
                struct BoxOfficeResponse {
                    domesticGross: Option<MoneyAmount>,
                    worldwideGross: Option<MoneyAmount>,
                    openingWeekendGross: Option<MoneyAmount>,
                    productionBudget: Option<MoneyAmount>,
                }
                if let Ok(data) = resp.json::<BoxOfficeResponse>() {
                    let format_money = |m: &Option<MoneyAmount>| -> Option<String> {
                        m.as_ref().and_then(|a| {
                            a.amount.map(|amt| {
                                let currency = a.currency.as_deref().unwrap_or("USD");
                                if amt >= 1_000_000_000.0 {
                                    format!("{} {:.1}B", currency, amt / 1_000_000_000.0)
                                } else if amt >= 1_000_000.0 {
                                    format!("{} {:.1}M", currency, amt / 1_000_000.0)
                                } else {
                                    format!("{} {:.0}", currency, amt)
                                }
                            })
                        })
                    };
                    details.domestic_gross = format_money(&data.domesticGross);
                    details.worldwide_gross = format_money(&data.worldwideGross);
                    details.opening_weekend_gross = format_money(&data.openingWeekendGross);
                    details.production_budget = format_money(&data.productionBudget);
                    println!("[IMDBAPI] Got boxOffice data for {}", resolved_imdb_id);
                }
            }
        }

        // 4. Awards
        println!(
            "[IMDBAPI] Fetching awardNominations for {}",
            resolved_imdb_id
        );
        let awards_url = format!(
            "https://api.imdbapi.dev/titles/{}/awardNominations",
            resolved_imdb_id
        );
        if let Ok(resp) = client
            .get(&awards_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
        {
            if resp.status().is_success() {
                #[derive(serde::Deserialize)]
                struct AwardsResponse {
                    stats: Option<AwardStats>,
                    nominations: Option<Vec<AwardNomination>>,
                }
                #[derive(serde::Deserialize)]
                struct AwardStats {
                    nominationCount: Option<i32>,
                    winCount: Option<i32>,
                }
                #[derive(serde::Deserialize)]
                struct AwardNomination {
                    event: Option<String>,
                    year: Option<i32>,
                    category: Option<String>,
                    isWinner: Option<bool>,
                }

                if let Ok(data) = resp.json::<AwardsResponse>() {
                    if let Some(stats) = data.stats {
                        details.total_nominations = stats.nominationCount;
                        details.total_wins = stats.winCount;
                    }
                    if let Some(noms) = data.nominations {
                        let award_list: Vec<ImdbAward> = noms
                            .into_iter()
                            .filter(|n| n.isWinner == Some(true))
                            .take(20)
                            .map(|n| ImdbAward {
                                event: n.event.unwrap_or_default(),
                                year: n.year,
                                category: n.category.unwrap_or_default(),
                                is_winner: n.isWinner.unwrap_or(false),
                            })
                            .collect();
                        if !award_list.is_empty() {
                            details.awards = Some(award_list);
                        }
                    }
                    println!(
                        "[IMDBAPI] Got awardNominations data for {}",
                        resolved_imdb_id
                    );
                }
            }
        }

        Ok(details)
    })
    .await
    .map_err(|e| e.to_string())?;

    details
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct SeverityBreakdown {
    severity_level: String,
    vote_count: i32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct ParentsGuideCategory {
    category: String,
    severity_breakdowns: Option<Vec<SeverityBreakdown>>,
}

#[tauri::command]
async fn get_parents_guide(imdb_id: String) -> Result<Vec<ParentsGuideCategory>, String> {
    let result = tokio::task::spawn_blocking(move || {
        let url = format!("https://api.imdbapi.dev/titles/{}/parentsGuide", imdb_id);
        println!("[IMDBAPI] Fetching parentsGuide for {}", imdb_id);
        let client = http_client::shared_client();
        let resp = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("parentsGuide API error: {}", resp.status()));
        }

        #[derive(serde::Deserialize)]
        struct ApiParentsGuideResponse {
            parentsGuide: Option<Vec<ApiParentsGuideCategory>>,
        }
        #[derive(serde::Deserialize)]
        struct ApiParentsGuideCategory {
            category: Option<String>,
            severityBreakdowns: Option<Vec<ApiSeverityBreakdown>>,
        }
        #[derive(serde::Deserialize)]
        struct ApiSeverityBreakdown {
            severityLevel: Option<String>,
            voteCount: Option<i32>,
        }

        let data: ApiParentsGuideResponse = resp.json().map_err(|e| e.to_string())?;
        let categories = data
            .parentsGuide
            .unwrap_or_default()
            .into_iter()
            .map(|cat| ParentsGuideCategory {
                category: cat.category.unwrap_or_default(),
                severity_breakdowns: cat.severityBreakdowns.map(|v| {
                    v.into_iter()
                        .map(|s| SeverityBreakdown {
                            severity_level: s.severityLevel.unwrap_or_default(),
                            vote_count: s.voteCount.unwrap_or(0),
                        })
                        .collect()
                }),
            })
            .collect();

        println!("[IMDBAPI] Got parentsGuide data for {}", imdb_id);
        Ok(categories)
    })
    .await
    .map_err(|e| e.to_string())?;

    result
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct TmdbReview {
    author: String,
    content: String,
    rating: Option<f64>,
    created_at: Option<String>,
    url: Option<String>,
}

#[tauri::command]
async fn get_tmdb_reviews(
    state: State<'_, AppState>,
    tmdb_id: i64,
    media_type: String,
) -> Result<Vec<TmdbReview>, String> {
    let credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    if credential.trim().is_empty() {
        return Ok(Vec::new());
    }

    let path = if media_type == "tv" {
        format!("/tv/{}/reviews", tmdb_id)
    } else {
        format!("/movie/{}/reviews", tmdb_id)
    };
    let url = build_tmdb_api_url(&path, &credential, "");

    let reviews = tokio::task::spawn_blocking(move || {
        let client = http_client::shared_client();
        let use_bearer = crate::is_access_token(&credential);
        let req = if use_bearer {
            client
                .get(&url)
                .header("Authorization", format!("Bearer {}", &credential))
        } else {
            client.get(&url)
        };

        #[derive(serde::Deserialize)]
        struct ReviewsResponse {
            results: Option<Vec<ReviewEntry>>,
        }
        #[derive(serde::Deserialize)]
        struct ReviewEntry {
            author: Option<String>,
            content: Option<String>,
            created_at: Option<String>,
            url: Option<String>,
            author_details: Option<AuthorDetails>,
        }
        #[derive(serde::Deserialize)]
        struct AuthorDetails {
            rating: Option<f64>,
        }

        match req.send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(data) = resp.json::<ReviewsResponse>() {
                    data.results
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|r| {
                            let content = r.content?;
                            if content.trim().is_empty() {
                                return None;
                            }
                            Some(TmdbReview {
                                author: r.author.unwrap_or_else(|| "Anonymous".to_string()),
                                content,
                                rating: r.author_details.and_then(|a| a.rating),
                                created_at: r.created_at,
                                url: r.url,
                            })
                        })
                        .take(10)
                        .collect()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(reviews)
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
    let credential_for_imdb = credential.clone();
    let credential_for_poster = credential.clone();

    let image_cache_dir = database::get_image_cache_dir();
    let image_cache_dir_for_imdb = image_cache_dir.clone();
    let image_cache_dir_for_poster = image_cache_dir.clone();
    let tv_id_str = tv_id.to_string();
    let series_title_clone = series_title.clone();

    println!(
        "[REFRESH] Starting metadata refresh for {} (TMDB ID: {})",
        series_title, tv_id
    );
    println!("[REFRESH] Image cache directory: {}", image_cache_dir);

    // Step 1: Find the series ID in our database by TMDB ID
    let (_series_db_id, owned_episodes): (Option<i64>, Vec<(i64, i32, i32)>) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let series_id = db
            .find_series_id_by_tmdb(&tv_id_str)
            .map_err(|e| e.to_string())?;

        if let Some(sid) = series_id {
            let episodes = db
                .get_owned_episodes_for_series(sid)
                .map_err(|e| e.to_string())?;
            println!(
                "[REFRESH] Found series DB ID: {}, owned episodes: {}",
                sid,
                episodes.len()
            );
            (Some(sid), episodes)
        } else {
            println!(
                "[REFRESH] Warning: Series not found in database by TMDB ID {}",
                tv_id
            );
            (None, Vec::new())
        }
    };

    if owned_episodes.is_empty() {
        return Err("No episodes found for this series in your library".to_string());
    }

    // Convert to (season, episode) tuples for the TMDB function
    let episode_list: Vec<(i32, i32)> = owned_episodes
        .iter()
        .map(|(_, season, episode)| (*season, *episode))
        .collect();

    println!(
        "[REFRESH] Will only fetch metadata for {} owned episodes: {:?}",
        episode_list.len(),
        episode_list
    );

    // Clear old cached metadata for just the episodes we own
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        // Only clear metadata for this series
        if let Ok(deleted) = db.clear_cached_metadata_for_series(&tv_id_str) {
            println!(
                "[REFRESH] Cleared {} old cached entries for series {}",
                deleted, tv_id
            );
        }
    }

    // Step 2: Fetch ONLY the episodes the user owns
    // Use std::thread::spawn to avoid tokio runtime panic (reqwest 0.12 blocking client)
    let (ep_tx, ep_rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let result = tmdb::fetch_owned_episodes_only(
            &credential,
            &tv_id_str,
            &series_title_clone,
            &image_cache_dir,
            &episode_list,
        );
        let _ = ep_tx.send(result);
    });
    let fetched_episodes = ep_rx
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

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
                ep.vote_average,
            ) {
                println!(
                    "[REFRESH] Warning: Failed to save cached metadata S{:02}E{:02}: {}",
                    ep.season_number, ep.episode_number, e
                );
            }

            // Also update the media table directly so episodes show the images immediately
            // Find the episode ID from our owned_episodes list
            if let Some((episode_db_id, _, _)) = owned_episodes
                .iter()
                .find(|(_, s, e)| *s == ep.season_number && *e == ep.episode_number)
            {
                if let Err(e) = db.update_episode_metadata(
                    *episode_db_id,
                    Some(&ep.name),
                    ep.overview.as_deref(),
                    ep.still_path.as_deref(),
                ) {
                    println!(
                        "[REFRESH] Warning: Failed to update media S{:02}E{:02}: {}",
                        ep.season_number, ep.episode_number, e
                    );
                } else {
                    println!(
                        "[REFRESH] Updated media entry for S{:02}E{:02}",
                        ep.season_number, ep.episode_number
                    );
                }
            }
        }
    }

    // Step 4: Try to get better images from imdbapi.dev
    {
        let owned_for_imdb = owned_episodes.clone();
        // Use std::thread::spawn to avoid tokio runtime panic (reqwest 0.12 blocking client)
        let (imdb_tx, imdb_rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let show_imdb_id = tmdb_imdb_id_for_show(tv_id, &credential_for_imdb);
            let Some(show_imdb_id) = show_imdb_id else {
                println!(
                    "[REFRESH] No IMDb ID found for TMDB ID {}, skipping imdbapi.dev images",
                    tv_id
                );
                let _ = imdb_tx.send(Ok(Vec::new()));
                return;
            };

            println!(
                "[IMDBAPI] Refreshing episode images for show {}",
                show_imdb_id
            );

            let mut seasons: std::collections::BTreeSet<i32> = std::collections::BTreeSet::new();
            for (_, s, _) in &owned_for_imdb {
                seasons.insert(*s);
            }

            let mut results: Vec<(i32, i32, String)> = Vec::new();

            for season_num in &seasons {
                let imdb_url = format!(
                    "https://api.imdbapi.dev/titles/{}/episodes?season={}",
                    show_imdb_id, season_num
                );
                let client = http_client::shared_client();
                let resp = match client
                    .get(&imdb_url)
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                {
                    Ok(r) if r.status().is_success() => r,
                    Ok(r) => {
                        println!(
                            "[REFRESH] imdbapi.dev returned status {} for season {}",
                            r.status(),
                            season_num
                        );
                        continue;
                    }
                    Err(e) => {
                        println!(
                            "[REFRESH] imdbapi.dev request failed for season {}: {}",
                            season_num, e
                        );
                        continue;
                    }
                };

                let json: serde_json::Value = match resp.json() {
                    Ok(j) => j,
                    Err(e) => {
                        println!("[REFRESH] Failed to parse imdbapi.dev response: {}", e);
                        continue;
                    }
                };

                let episodes = match json.get("episodes").and_then(|e| e.as_array()) {
                    Some(eps) => eps,
                    None => continue,
                };

                for ep in episodes {
                    let ep_num = match ep.get("episodeNumber").and_then(|n| n.as_i64()) {
                        Some(n) => n,
                        None => continue,
                    };
                    let img_url = match ep
                        .get("primaryImage")
                        .and_then(|i| i.get("url"))
                        .and_then(|u| u.as_str())
                    {
                        Some(url) => url,
                        None => continue,
                    };

                    let is_owned = owned_for_imdb
                        .iter()
                        .any(|(_, s, e)| *s == *season_num && *e == ep_num as i32);
                    if !is_owned {
                        continue;
                    }

                    let image_type = tmdb::ImageType::EpisodeBanner {
                        season: *season_num,
                        episode: ep_num as i32,
                    };
                    if let Some(cached_path) = tmdb::cache_imdb_image(
                        img_url,
                        std::path::Path::new(&image_cache_dir_for_imdb),
                        &image_type,
                    ) {
                        results.push((*season_num, ep_num as i32, cached_path));
                    }
                }
            }

            let _ = imdb_tx.send(Ok(results));
        });
        let imdb_results: Result<Vec<(i32, i32, String)>, String> =
            imdb_rx.await.map_err(|e| e.to_string())?;

        match imdb_results {
            Ok(results) if !results.is_empty() => {
                let db = state.db.lock().map_err(|e| e.to_string())?;
                for (season, episode, cached_path) in &results {
                    let _ = db.update_episode_still_path(tv_id, *season, *episode, cached_path);
                    println!(
                        "[REFRESH] Updated S{:02}E{:02} with imdbapi.dev image",
                        season, episode
                    );
                }
                total_images += results.len();
                println!(
                    "[IMDBAPI] Updated {} episode still paths from imdbapi.dev",
                    results.len()
                );
            }
            Ok(_) => {
                println!("[REFRESH] imdbapi.dev: no additional images found");
            }
            Err(e) => {
                println!("[REFRESH] imdbapi.dev image fetch failed: {}", e);
            }
        }
    }

    // Step 8: Refresh show-level poster from TMDB + imdbapi.dev
    // ALL blocking HTTP in std::thread::spawn to avoid tokio runtime panic
    println!("[REFRESH] Refreshing show poster...");
    let mut new_poster_path: Option<String> = None;

    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut poster: Option<String> = None;

        // Step A: TMDB poster
        let tmdb_result = tmdb::fetch_metadata_by_id(
            &credential_for_poster,
            &tv_id.to_string(),
            "tv",
            &image_cache_dir_for_poster,
        );
        if let Ok(ref tmdb_meta) = tmdb_result {
            if tmdb_meta.poster_path.is_some() {
                poster = tmdb_meta.poster_path.clone();
                println!("[REFRESH] TMDB show poster: {:?}", poster);
            }
            // Step B: imdbapi.dev poster (overrides TMDB)
            if let Some(ref show_imdb_id) = tmdb_meta.imdb_id {
                let imdb_url = format!("https://api.imdbapi.dev/titles/{}", show_imdb_id);
                if let Ok(resp) = http_client::shared_client().get(&imdb_url).send() {
                    if let Ok(json) = resp.json::<serde_json::Value>() {
                        if let Some(img_url) = json
                            .get("primaryImage")
                            .and_then(|i| i.get("url"))
                            .and_then(|u| u.as_str())
                        {
                            let img_cache = database::get_image_cache_dir();
                            if let Some(cached) = tmdb::cache_imdb_image(
                                img_url,
                                std::path::Path::new(&img_cache),
                                &tmdb::ImageType::SeriesBanner,
                            ) {
                                poster = Some(cached);
                                println!("[REFRESH] imdbapi.dev poster override: {:?}", poster);
                            }
                        }
                    }
                }
            }
        }
        let _ = tx.send(poster);
    });
    let poster_result = rx.await.map_err(|e| e.to_string());
    if let Ok(Some(poster)) = poster_result {
        new_poster_path = Some(poster);
    }

    // Update the show's poster_path in the database
    if let (Some(ref poster), Some(show_db_id)) = (&new_poster_path, _series_db_id) {
        if let Ok(db) = state.db.lock() {
            let _ = db.update_poster_path(show_db_id, poster);
            println!("[REFRESH] Updated show poster_path: {}", poster);
        }
    }

    let result = format!(
        "Refreshed {} episodes, {} images downloaded",
        fetched_episodes.len(),
        total_images
    );
    println!(
        "[REFRESH] Completed: {} (poster_updated={})",
        result,
        new_poster_path.is_some()
    );
    Ok(result)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MovieReminderInput {
    tmdb_id: String,
    media_type: String,
    title: String,
    poster_path: Option<String>,
    season_number: Option<i32>,
    episode_number: Option<i32>,
    release_date: Option<String>,
    reminder_at: String,
    source: Option<String>,
    tracking_mode: Option<String>,
    tracking_season_number: Option<i32>,
    notes: Option<String>,
    is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchlistItemInput {
    tmdb_id: String,
    media_type: String,
    title: String,
    poster_path: Option<String>,
    release_date: Option<String>,
    notes: Option<String>,
    is_active: Option<bool>,
    notification_enabled: Option<bool>,
    notification_mode: Option<String>,
    notification_interval_minutes: Option<i32>,
    notify_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TmdbReleaseSchedule {
    tmdb_id: i64,
    media_type: String,
    title: String,
    season_number: Option<i32>,
    episode_number: Option<i32>,
    release_date: Option<String>,
    suggested_reminder_at: Option<String>,
    source: String,
    precision: String,
    editable: bool,
}

#[derive(Debug, Clone)]
struct ReminderScheduleTarget {
    title: String,
    poster_path: Option<String>,
    season_number: Option<i32>,
    episode_number: Option<i32>,
    release_date: Option<String>,
    reminder_at: String,
    source: String,
    tracking_season_number: Option<i32>,
}

fn parse_reminder_datetime_to_utc(value: &str) -> Result<String, String> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.with_timezone(&Utc).to_rfc3339());
    }

    if let Ok(parsed) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        let local = Local
            .from_local_datetime(&parsed)
            .single()
            .ok_or_else(|| "Reminder time is ambiguous in the local timezone".to_string())?;
        return Ok(local.with_timezone(&Utc).to_rfc3339());
    }

    if let Ok(parsed) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        let local = Local
            .from_local_datetime(&parsed)
            .single()
            .ok_or_else(|| "Reminder time is ambiguous in the local timezone".to_string())?;
        return Ok(local.with_timezone(&Utc).to_rfc3339());
    }

    Err("Reminder time must be an ISO/RFC3339 datetime".to_string())
}

fn suggested_reminder_at_from_release_date(release_date: Option<&str>) -> Option<String> {
    let date = NaiveDate::parse_from_str(release_date?, "%Y-%m-%d").ok()?;
    let local = Local
        .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .single()?;
    Some(local.with_timezone(&Utc).to_rfc3339())
}

fn is_release_date_in_future(release_date: Option<&str>) -> bool {
    suggested_reminder_at_from_release_date(release_date)
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|target| target.with_timezone(&Utc) > Utc::now())
        .unwrap_or(false)
}

fn tvmaze_get_json<T: for<'de> Deserialize<'de>>(url: &str) -> Result<Option<T>, String> {
    let client = http_client::shared_client();

    let response = client.get(url).send().map_err(|e| e.to_string())?;
    if response.status().as_u16() == 404 {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("TVmaze returned HTTP {}", response.status()));
    }

    response.json::<T>().map(Some).map_err(|e| e.to_string())
}

fn tvmaze_lookup_show_id_by_imdb(imdb_id: &str) -> Option<i64> {
    if imdb_id.trim().is_empty() {
        return None;
    }

    #[derive(Deserialize)]
    struct TvmazeShow {
        id: i64,
    }

    let url = format!(
        "https://api.tvmaze.com/lookup/shows?imdb={}",
        imdb_id.trim()
    );
    tvmaze_get_json::<TvmazeShow>(&url)
        .ok()
        .flatten()
        .map(|show| show.id)
}

fn apply_streaming_provider_heuristics(
    provider_name: Option<&str>,
    release_date_str: Option<&str>,
) -> Option<String> {
    let provider = provider_name?.trim().to_ascii_lowercase();
    let date = NaiveDate::parse_from_str(release_date_str?, "%Y-%m-%d").ok()?;

    let tz_pt: chrono_tz::Tz = "America/Los_Angeles".parse().unwrap();
    let tz_et: chrono_tz::Tz = "America/New_York".parse().unwrap();

    if provider.contains("netflix") || provider.contains("disney") {
        let pt_midnight = date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(tz_pt)
            .single()?;
        return Some(pt_midnight.with_timezone(&Utc).to_rfc3339());
    } else if provider.contains("hulu") || provider.contains("apple") {
        let et_midnight = date
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(tz_et)
            .single()?;
        return Some(et_midnight.with_timezone(&Utc).to_rfc3339());
    } else if provider.contains("amazon") || provider.contains("prime") {
        return Some(date.and_hms_opt(0, 0, 0).unwrap().and_utc().to_rfc3339());
    } else if provider.contains("max") || provider.contains("hbo") {
        let et_9pm = date
            .and_hms_opt(21, 0, 0)
            .unwrap()
            .and_local_timezone(tz_et)
            .single()?;
        return Some(et_9pm.with_timezone(&Utc).to_rfc3339());
    } else if provider.contains("crunchyroll") {
        return Some(date.and_hms_opt(12, 0, 0).unwrap().and_utc().to_rfc3339());
    }
    None
}

fn tmdb_exact_episode_reminder_at_heuristics(
    tmdb_id: i64,
    release_date_str: Option<&str>,
    credential: &str,
) -> Option<String> {
    #[derive(Deserialize)]
    struct ProviderInfo {
        provider_name: Option<String>,
    }

    #[derive(Deserialize)]
    struct CountryProviders {
        flatrate: Option<Vec<ProviderInfo>>,
        free: Option<Vec<ProviderInfo>>,
    }

    #[derive(Deserialize)]
    struct WatchProvidersResult {
        results: std::collections::HashMap<String, CountryProviders>,
    }

    let url = build_tmdb_api_url(&format!("/tv/{}/watch/providers", tmdb_id), credential, "");
    let raw: WatchProvidersResult = http_get_with_retry_auth(&url, credential, 2)
        .ok()?
        .json()
        .ok()?;

    let mut found_provider = None;
    for (_, providers) in &raw.results {
        if let Some(flatrate) = &providers.flatrate {
            for provider in flatrate {
                if let Some(name) = &provider.provider_name {
                    let name_lower = name.to_ascii_lowercase();
                    if name_lower.contains("netflix")
                        || name_lower.contains("disney")
                        || name_lower.contains("hulu")
                        || name_lower.contains("apple")
                        || name_lower.contains("amazon")
                        || name_lower.contains("prime")
                        || name_lower.contains("max")
                        || name_lower.contains("hbo")
                        || name_lower.contains("crunchyroll")
                    {
                        found_provider = Some(name.clone());
                        break;
                    }
                }
            }
        }
        if found_provider.is_some() {
            break;
        }
    }

    apply_streaming_provider_heuristics(found_provider.as_deref(), release_date_str)
}

fn tvmaze_exact_episode_reminder_at(show_id: i64, season: i32, episode: i32) -> Option<String> {
    #[derive(Deserialize)]
    struct TvmazeCountry {
        code: Option<String>,
        timezone: Option<String>,
    }

    #[derive(Deserialize)]
    struct TvmazeChannel {
        name: Option<String>,
        country: Option<TvmazeCountry>,
    }

    #[derive(Deserialize)]
    struct TvmazeSchedule {
        days: Option<Vec<String>>,
        time: Option<String>,
    }

    #[derive(Deserialize)]
    struct TvmazeShow {
        schedule: Option<TvmazeSchedule>,
        network: Option<TvmazeChannel>,
        #[serde(rename = "webChannel")]
        web_channel: Option<TvmazeChannel>,
    }

    #[derive(Deserialize)]
    struct TvmazeEpisode {
        airdate: Option<String>,
        airtime: Option<String>,
        airstamp: Option<String>,
    }

    let show_url = format!("https://api.tvmaze.com/shows/{}", show_id);
    let show = tvmaze_get_json::<TvmazeShow>(&show_url).ok().flatten();

    let episode_url = format!(
        "https://api.tvmaze.com/shows/{}/episodebynumber?season={}&number={}",
        show_id, season, episode
    );
    let episode = tvmaze_get_json::<TvmazeEpisode>(&episode_url)
        .ok()
        .flatten()?;

    let provider_name = show.as_ref().and_then(|show| {
        show.network
            .as_ref()
            .and_then(|network| network.name.as_deref())
            .or_else(|| {
                show.web_channel
                    .as_ref()
                    .and_then(|channel| channel.name.as_deref())
            })
    });

    if let Some(heuristic_time) =
        apply_streaming_provider_heuristics(provider_name, episode.airdate.as_deref())
    {
        return Some(heuristic_time);
    }

    let parsed_airstamp = episode
        .airstamp
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok());

    if let Some(parsed_airstamp) = parsed_airstamp {
        let utc_time = parsed_airstamp.with_timezone(&Utc).time();

        let is_noon_utc = utc_time.hour() == 12 && utc_time.minute() == 0 && utc_time.second() == 0;
        let is_midnight_utc =
            utc_time.hour() == 0 && utc_time.minute() == 0 && utc_time.second() == 0;

        let timezone_name = show.as_ref().and_then(|show| {
            show.network
                .as_ref()
                .and_then(|network| network.country.as_ref())
                .and_then(|country| country.timezone.as_deref())
                .or_else(|| {
                    show.web_channel
                        .as_ref()
                        .and_then(|channel| channel.country.as_ref())
                        .and_then(|country| country.timezone.as_deref())
                })
        });

        let is_global_or_unknown = timezone_name.map_or(true, |tz| {
            tz.trim().is_empty() || tz.eq_ignore_ascii_case("global")
        });

        if !((is_noon_utc || is_midnight_utc) && is_global_or_unknown) {
            return Some(parsed_airstamp.with_timezone(&Utc).to_rfc3339());
        }
    }

    let airtime = episode
        .airtime
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            show.as_ref()
                .and_then(|show| show.schedule.as_ref())
                .and_then(|schedule| schedule.time.as_deref())
                .filter(|value| !value.trim().is_empty())
        });

    let timezone_name = show
        .as_ref()
        .and_then(|show| {
            show.network
                .as_ref()
                .and_then(|network| network.country.as_ref())
                .and_then(|country| country.timezone.as_deref())
                .or_else(|| {
                    show.web_channel
                        .as_ref()
                        .and_then(|channel| channel.country.as_ref())
                        .and_then(|country| country.timezone.as_deref())
                })
        })
        .filter(|value| !value.trim().is_empty() && !value.eq_ignore_ascii_case("global"));

    if let (Some(airdate), Some(airtime), Some(timezone_name)) =
        (episode.airdate.as_deref(), airtime, timezone_name)
    {
        if let (Ok(date), Ok(time), Ok(timezone)) = (
            NaiveDate::parse_from_str(airdate, "%Y-%m-%d"),
            NaiveTime::parse_from_str(airtime, "%H:%M"),
            timezone_name.parse::<chrono_tz::Tz>(),
        ) {
            let local_dt = date.and_time(time);
            let zoned = match timezone.from_local_datetime(&local_dt) {
                LocalResult::Single(value) => Some(value),
                LocalResult::Ambiguous(earliest, _) => Some(earliest),
                LocalResult::None => None,
            };

            if let Some(zoned) = zoned {
                return Some(zoned.with_timezone(&Utc).to_rfc3339());
            }
        }
    }

    if let Some(airdate) = episode.airdate.as_deref() {
        if let Ok(date) = NaiveDate::parse_from_str(airdate, "%Y-%m-%d") {
            if let Some(local_midnight) = Local
                .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
                .single()
            {
                return Some(local_midnight.with_timezone(&Utc).to_rfc3339());
            }
        }
    }

    None
}

fn tmdb_imdb_id_for_show(tmdb_id: i64, credential: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct RawExternalIds {
        imdb_id: Option<String>,
    }

    let url = build_tmdb_api_url(&format!("/tv/{}/external_ids", tmdb_id), credential, "");
    let raw: RawExternalIds = http_get_with_retry_auth(&url, credential, 2)
        .ok()?
        .json()
        .ok()?;
    raw.imdb_id.filter(|value| !value.trim().is_empty())
}

fn enrich_tv_schedule_with_tvmaze(
    schedule: &mut TmdbReleaseSchedule,
    credential: &str,
    tmdb_id: i64,
) {
    if schedule.media_type != "tv" {
        return;
    }

    let (Some(season), Some(episode)) = (schedule.season_number, schedule.episode_number) else {
        return;
    };

    if let Some(tmdb_time) = tmdb_exact_episode_reminder_at_heuristics(
        tmdb_id,
        schedule.release_date.as_deref(),
        credential,
    ) {
        schedule.suggested_reminder_at = Some(tmdb_time);
        schedule.source = "tmdb-heuristics".to_string();
        schedule.precision = "datetime".to_string();
        return;
    }

    let Some(imdb_id) = tmdb_imdb_id_for_show(tmdb_id, credential) else {
        return;
    };
    let Some(tvmaze_show_id) = tvmaze_lookup_show_id_by_imdb(&imdb_id) else {
        return;
    };
    let Some(reminder_at) = tvmaze_exact_episode_reminder_at(tvmaze_show_id, season, episode)
    else {
        return;
    };

    schedule.suggested_reminder_at = Some(reminder_at);
    schedule.source = "tvmaze".to_string();
    schedule.precision = "datetime".to_string();
}

fn reminder_input_to_new<'a>(
    input: &'a MovieReminderInput,
    reminder_at_utc: &'a str,
) -> database::NewMovieReminder<'a> {
    database::NewMovieReminder {
        tmdb_id: input.tmdb_id.trim(),
        media_type: input.media_type.trim(),
        title: input.title.trim(),
        poster_path: input.poster_path.as_deref(),
        season_number: input.season_number,
        episode_number: input.episode_number,
        release_date: input.release_date.as_deref(),
        reminder_at: reminder_at_utc,
        source: input.source.as_deref().unwrap_or("manual"),
        tracking_mode: input.tracking_mode.as_deref().unwrap_or("single"),
        tracking_season_number: input.tracking_season_number,
        notes: input.notes.as_deref(),
        is_active: input.is_active.unwrap_or(true),
    }
}

fn validate_reminder_input(input: &MovieReminderInput) -> Result<(), String> {
    if input.tmdb_id.trim().is_empty() {
        return Err("TMDB id is required".to_string());
    }
    if input.title.trim().is_empty() {
        return Err("Title is required".to_string());
    }
    match input.media_type.trim() {
        "movie" | "tv" => Ok(()),
        _ => Err("media_type must be movie or tv".to_string()),
    }
}

fn normalize_tracking_mode(input: &MovieReminderInput) -> Result<String, String> {
    let trimmed = input.tracking_mode.as_deref().unwrap_or("single").trim();
    match trimmed {
        "single" | "tv_season" => Ok(trimmed.to_string()),
        _ => Err("tracking_mode must be single or tv_season".to_string()),
    }
}

fn validate_tracking_mode(input: &MovieReminderInput) -> Result<(), String> {
    let tracking_mode = normalize_tracking_mode(input)?;
    if input.media_type.trim() == "movie" && tracking_mode != "single" {
        return Err("Movies only support single reminders".to_string());
    }
    Ok(())
}

fn normalize_watchlist_notification_mode(input: &WatchlistItemInput) -> Result<String, String> {
    let mode = input
        .notification_mode
        .as_deref()
        .unwrap_or("single")
        .trim()
        .to_ascii_lowercase();

    match mode.as_str() {
        "single" | "spam" => Ok(mode),
        _ => Err("notification_mode must be single or spam".to_string()),
    }
}

fn validate_watchlist_input(input: &WatchlistItemInput) -> Result<(), String> {
    if input.tmdb_id.trim().is_empty() {
        return Err("TMDB id is required".to_string());
    }
    if input.title.trim().is_empty() {
        return Err("Title is required".to_string());
    }
    match input.media_type.trim() {
        "movie" | "tv" => {}
        _ => return Err("media_type must be movie or tv".to_string()),
    }

    let notification_enabled = input.notification_enabled.unwrap_or(false);
    let mode = normalize_watchlist_notification_mode(input)?;

    if notification_enabled && input.notify_at.is_none() {
        return Err("notify_at is required when notifications are enabled".to_string());
    }

    if mode == "spam" {
        let interval = input.notification_interval_minutes.unwrap_or(0);
        if interval <= 0 {
            return Err("Spam reminders require a positive notification interval".to_string());
        }
    }

    Ok(())
}

#[tauri::command]
async fn create_movie_reminder(
    state: State<'_, AppState>,
    reminder: MovieReminderInput,
) -> Result<database::MovieReminder, String> {
    validate_reminder_input(&reminder)?;
    validate_tracking_mode(&reminder)?;
    let reminder_at_utc = parse_reminder_datetime_to_utc(&reminder.reminder_at)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.create_movie_reminder(reminder_input_to_new(&reminder, &reminder_at_utc))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_movie_reminder(
    state: State<'_, AppState>,
    id: i64,
    reminder: MovieReminderInput,
) -> Result<database::MovieReminder, String> {
    validate_reminder_input(&reminder)?;
    validate_tracking_mode(&reminder)?;
    let reminder_at_utc = parse_reminder_datetime_to_utc(&reminder.reminder_at)?;
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.update_movie_reminder(id, reminder_input_to_new(&reminder, &reminder_at_utc))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_movie_reminders(
    state: State<'_, AppState>,
    include_inactive: Option<bool>,
) -> Result<Vec<database::MovieReminder>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_movie_reminders(include_inactive.unwrap_or(false))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_movie_reminder(state: State<'_, AppState>, id: i64) -> Result<ApiResponse, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_movie_reminder(id).map_err(|e| e.to_string())?;
    Ok(ApiResponse {
        message: "Reminder deleted".to_string(),
    })
}

#[tauri::command]
async fn set_movie_reminder_active(
    state: State<'_, AppState>,
    id: i64,
    is_active: bool,
) -> Result<database::MovieReminder, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.set_movie_reminder_active(id, is_active)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_watchlist_items(
    state: State<'_, AppState>,
    include_inactive: Option<bool>,
) -> Result<Vec<database::WatchlistItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_watchlist_items(include_inactive.unwrap_or(false))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_or_update_watchlist_item(
    state: State<'_, AppState>,
    item: WatchlistItemInput,
) -> Result<database::WatchlistItem, String> {
    validate_watchlist_input(&item)?;
    let mode = normalize_watchlist_notification_mode(&item)?;
    let notify_at_utc = item
        .notify_at
        .as_deref()
        .map(parse_reminder_datetime_to_utc)
        .transpose()?;

    let created = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.create_or_update_watchlist_item(database::NewWatchlistItem {
            tmdb_id: item.tmdb_id.trim(),
            media_type: item.media_type.trim(),
            title: item.title.trim(),
            poster_path: item.poster_path.as_deref(),
            release_date: item.release_date.as_deref(),
            notes: item.notes.as_deref(),
            is_active: item.is_active.unwrap_or(true),
            notification_enabled: item.notification_enabled.unwrap_or(false),
            notification_mode: &mode,
            notification_interval_minutes: item.notification_interval_minutes,
            notify_at: notify_at_utc.as_deref(),
        })
        .map_err(|e| e.to_string())?
    };

    let _ = sync_watchlist_to_drive(&state).await;
    Ok(created)
}

#[tauri::command]
async fn update_watchlist_item(
    state: State<'_, AppState>,
    id: i64,
    item: WatchlistItemInput,
) -> Result<database::WatchlistItem, String> {
    validate_watchlist_input(&item)?;
    let mode = normalize_watchlist_notification_mode(&item)?;
    let notify_at_utc = item
        .notify_at
        .as_deref()
        .map(parse_reminder_datetime_to_utc)
        .transpose()?;

    let updated = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.update_watchlist_item(
            id,
            database::NewWatchlistItem {
                tmdb_id: item.tmdb_id.trim(),
                media_type: item.media_type.trim(),
                title: item.title.trim(),
                poster_path: item.poster_path.as_deref(),
                release_date: item.release_date.as_deref(),
                notes: item.notes.as_deref(),
                is_active: item.is_active.unwrap_or(true),
                notification_enabled: item.notification_enabled.unwrap_or(false),
                notification_mode: &mode,
                notification_interval_minutes: item.notification_interval_minutes,
                notify_at: notify_at_utc.as_deref(),
            },
        )
        .map_err(|e| e.to_string())?
    };

    let _ = sync_watchlist_to_drive(&state).await;
    Ok(updated)
}

#[tauri::command]
async fn delete_watchlist_item(state: State<'_, AppState>, id: i64) -> Result<ApiResponse, String> {
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.delete_watchlist_item(id).map_err(|e| e.to_string())?;
    }
    let _ = sync_watchlist_to_drive(&state).await;
    Ok(ApiResponse {
        message: "Watchlist item deleted".to_string(),
    })
}

#[tauri::command]
async fn get_tmdb_release_schedule(
    state: State<'_, AppState>,
    tmdb_id: i64,
    media_type: String,
    season_number: Option<i32>,
    episode_number: Option<i32>,
    imdb_id: Option<String>,
) -> Result<TmdbReleaseSchedule, String> {
    let credential = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    let media_type = media_type.trim().to_string();
    if media_type != "movie" && media_type != "tv" {
        return Err("media_type must be movie or tv".to_string());
    }

    if credential.trim().is_empty() {
        return tokio::task::spawn_blocking(move || -> Result<TmdbReleaseSchedule, String> {
            let imdb_id = imdb_id.clone().or_else(|| {
                if tmdb_id > 0 {
                    Some(format!("tt{:07}", tmdb_id))
                } else {
                    None
                }
            });
            let kind = if media_type == "tv" {
                cinemeta_api::CinemetaKind::Series
            } else {
                cinemeta_api::CinemetaKind::Movie
            };
            if let Some(tt) = imdb_id.as_deref() {
                if let Ok(meta) = cinemeta_api::get_title(kind, tt) {
                    let release_date = meta.released.clone().filter(|s| !s.trim().is_empty());
                    let suggested =
                        suggested_reminder_at_from_release_date(release_date.as_deref());
                    let title = meta.name.clone();
                    let mut schedule = TmdbReleaseSchedule {
                        tmdb_id,
                        media_type: media_type.clone(),
                        title,
                        season_number,
                        episode_number,
                        release_date,
                        suggested_reminder_at: suggested,
                        source: "imdb".to_string(),
                        precision: "date".to_string(),
                        editable: true,
                    };
                    return Ok(schedule);
                }
            }
            Err("[IMDB] no api_key and no IMDb id".into())
        })
        .await
        .map_err(|e| e.to_string())?;
    }

    tokio::task::spawn_blocking(move || -> Result<TmdbReleaseSchedule, String> {
        if media_type == "movie" {
            #[derive(Deserialize)]
            struct RawMovie {
                title: Option<String>,
                original_title: Option<String>,
                release_date: Option<String>,
            }

            let url = build_tmdb_api_url(&format!("/movie/{}", tmdb_id), &credential, "");
            let raw: RawMovie = http_get_with_retry_auth(&url, &credential, 3)?
                .json()
                .map_err(|e| e.to_string())?;
            let release_date = raw.release_date.filter(|value| !value.trim().is_empty());
            let suggested = suggested_reminder_at_from_release_date(release_date.as_deref());

            return Ok(TmdbReleaseSchedule {
                tmdb_id,
                media_type,
                title: raw
                    .title
                    .or(raw.original_title)
                    .unwrap_or_else(|| "Unknown movie".to_string()),
                season_number: None,
                episode_number: None,
                release_date,
                suggested_reminder_at: suggested,
                source: "tmdb".to_string(),
                precision: "date".to_string(),
                editable: true,
            });
        }

        #[derive(Deserialize)]
        struct RawTv {
            name: Option<String>,
            original_name: Option<String>,
            first_air_date: Option<String>,
            next_episode_to_air: Option<RawAirEpisode>,
        }

        #[derive(Clone, Deserialize)]
        struct RawAirEpisode {
            name: Option<String>,
            air_date: Option<String>,
            season_number: Option<i32>,
            episode_number: i32,
        }

        #[derive(Deserialize)]
        struct RawSeasonSchedule {
            episodes: Option<Vec<RawAirEpisode>>,
        }

        let url = build_tmdb_api_url(&format!("/tv/{}", tmdb_id), &credential, "");
        let raw: RawTv = http_get_with_retry_auth(&url, &credential, 3)?
            .json()
            .map_err(|e| e.to_string())?;
        let show_title = raw
            .name
            .or(raw.original_name)
            .unwrap_or_else(|| "Unknown show".to_string());

        let build_episode_schedule = |episode: RawAirEpisode, show_title: &str| {
            let release_date = episode
                .air_date
                .clone()
                .filter(|value| !value.trim().is_empty());
            let suggested = suggested_reminder_at_from_release_date(release_date.as_deref());
            let episode_title = episode
                .name
                .filter(|name| {
                    !name.trim().is_empty()
                        && name.to_lowercase() != format!("episode {}", episode.episode_number)
                })
                .map(|name| format!("{} - {}", show_title, name))
                .unwrap_or_else(|| {
                    format!(
                        "{} - S{:02}E{:02}",
                        show_title,
                        episode.season_number.unwrap_or_default(),
                        episode.episode_number
                    )
                });

            let mut schedule = TmdbReleaseSchedule {
                tmdb_id,
                media_type: media_type.clone(),
                title: episode_title,
                season_number: episode.season_number,
                episode_number: Some(episode.episode_number),
                release_date,
                suggested_reminder_at: suggested,
                source: "tmdb".to_string(),
                precision: "date".to_string(),
                editable: true,
            };
            enrich_tv_schedule_with_tvmaze(&mut schedule, &credential, tmdb_id);
            schedule
        };

        if let (Some(season), Some(episode)) = (season_number, episode_number) {
            if let Some(next) = &raw.next_episode_to_air {
                if next.season_number == Some(season) && next.episode_number == episode {
                    return Ok(build_episode_schedule(next.clone(), &show_title));
                }
            }

            let season_url = build_tmdb_api_url(
                &format!("/tv/{}/season/{}", tmdb_id, season),
                &credential,
                "",
            );
            let raw_season: RawSeasonSchedule =
                http_get_with_retry_auth(&season_url, &credential, 3)?
                    .json()
                    .map_err(|e| e.to_string())?;

            let mut episode_info = raw_season
                .episodes
                .unwrap_or_default()
                .into_iter()
                .find(|item| item.episode_number == episode)
                .ok_or_else(|| "Episode not found on TMDB".to_string())?;

            episode_info.season_number = Some(season);
            return Ok(build_episode_schedule(episode_info, &show_title));
        }

        if let Some(next_episode) = raw.next_episode_to_air {
            if is_release_date_in_future(next_episode.air_date.as_deref()) {
                return Ok(build_episode_schedule(next_episode, &show_title));
            }

            if let Some(season) = next_episode.season_number {
                let season_url = build_tmdb_api_url(
                    &format!("/tv/{}/season/{}", tmdb_id, season),
                    &credential,
                    "",
                );

                if let Ok(response) = http_get_with_retry_auth(&season_url, &credential, 3) {
                    if let Ok(raw_season) = response.json::<RawSeasonSchedule>() {
                        if let Some(future_episode) = raw_season
                            .episodes
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|episode| {
                                episode.episode_number > next_episode.episode_number
                                    && is_release_date_in_future(episode.air_date.as_deref())
                            })
                            .min_by_key(|episode| episode.air_date.clone())
                        {
                            return Ok(build_episode_schedule(future_episode, &show_title));
                        }
                    }
                }
            }

            return Ok(build_episode_schedule(next_episode, &show_title));
        }

        let release_date = raw.first_air_date.filter(|value| !value.trim().is_empty());
        let suggested = suggested_reminder_at_from_release_date(release_date.as_deref());

        Ok(TmdbReleaseSchedule {
            tmdb_id,
            media_type,
            title: show_title,
            season_number: None,
            episode_number: None,
            release_date,
            suggested_reminder_at: suggested,
            source: "tmdb".to_string(),
            precision: "date".to_string(),
            editable: true,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn resolve_next_tv_season_reminder_target(
    reminder: &database::MovieReminder,
    credential: &str,
    now_utc: &DateTime<Utc>,
) -> Result<Option<ReminderScheduleTarget>, String> {
    #[derive(Clone, Deserialize)]
    struct RawAirEpisode {
        name: Option<String>,
        air_date: Option<String>,
        season_number: Option<i32>,
        episode_number: i32,
    }

    #[derive(Clone, Deserialize)]
    struct RawSeason {
        season_number: i32,
        episode_count: i32,
        air_date: Option<String>,
    }

    #[derive(Deserialize)]
    struct RawTv {
        name: Option<String>,
        original_name: Option<String>,
        poster_path: Option<String>,
        first_air_date: Option<String>,
        next_episode_to_air: Option<RawAirEpisode>,
        seasons: Option<Vec<RawSeason>>,
    }

    #[derive(Clone, Deserialize)]
    struct RawSeasonEpisode {
        episode_number: i32,
        name: Option<String>,
        air_date: Option<String>,
    }

    #[derive(Deserialize)]
    struct RawSeasonDetails {
        episodes: Option<Vec<RawSeasonEpisode>>,
    }

    let tmdb_id_num = reminder
        .tmdb_id
        .trim()
        .parse::<i64>()
        .map_err(|_| "Invalid TMDB id on reminder".to_string())?;
    let show_url = build_tmdb_api_url(&format!("/tv/{}", tmdb_id_num), credential, "");
    let raw: RawTv = http_get_with_retry_auth(&show_url, credential, 3)?
        .json()
        .map_err(|e| e.to_string())?;

    let show_title = raw
        .name
        .or(raw.original_name)
        .unwrap_or_else(|| reminder.title.clone());
    let poster_path = reminder.poster_path.clone().or(raw.poster_path);
    let tvmaze_show_id = tmdb_imdb_id_for_show(tmdb_id_num, credential)
        .and_then(|imdb_id| tvmaze_lookup_show_id_by_imdb(&imdb_id));
    let tracking_season = reminder
        .tracking_season_number
        .or(reminder.season_number)
        .or_else(|| {
            raw.next_episode_to_air
                .as_ref()
                .and_then(|episode| episode.season_number)
        });

    let build_target = |episode: RawAirEpisode,
                        explicit_tracking_season: Option<i32>|
     -> Option<ReminderScheduleTarget> {
        let release_date = episode
            .air_date
            .clone()
            .filter(|value| !value.trim().is_empty());
        let suggested = suggested_reminder_at_from_release_date(release_date.as_deref())?;
        let (reminder_at, source) = if let Some(tmdb_time) =
            tmdb_exact_episode_reminder_at_heuristics(
                tmdb_id_num,
                release_date.as_deref(),
                credential,
            ) {
            (tmdb_time, "tmdb-heuristics".to_string())
        } else if let (Some(show_id), Some(season)) = (tvmaze_show_id, episode.season_number) {
            match tvmaze_exact_episode_reminder_at(show_id, season, episode.episode_number) {
                Some(exact_time) => (exact_time, "tvmaze".to_string()),
                None => (suggested, "tmdb".to_string()),
            }
        } else {
            (suggested, "tmdb".to_string())
        };
        let episode_label = episode
            .name
            .as_ref()
            .filter(|name| {
                !name.trim().is_empty()
                    && name.to_lowercase() != format!("episode {}", episode.episode_number)
            })
            .map(|name| format!("{} - {}", show_title, name))
            .unwrap_or_else(|| match episode.season_number {
                Some(season) => format!(
                    "{} - S{:02}E{:02}",
                    show_title, season, episode.episode_number
                ),
                None => format!("{} - Episode {}", show_title, episode.episode_number),
            });

        Some(ReminderScheduleTarget {
            title: episode_label,
            poster_path: poster_path.clone(),
            season_number: episode.season_number,
            episode_number: Some(episode.episode_number),
            release_date,
            reminder_at,
            source,
            tracking_season_number: explicit_tracking_season.or(episode.season_number),
        })
    };

    if tracking_season.is_none() {
        if let Some(next_episode) = raw.next_episode_to_air.clone() {
            if is_release_date_in_future(next_episode.air_date.as_deref()) {
                return Ok(build_target(
                    next_episode.clone(),
                    next_episode.season_number,
                ));
            }
        }

        if is_release_date_in_future(raw.first_air_date.as_deref()) {
            let reminder_at =
                suggested_reminder_at_from_release_date(raw.first_air_date.as_deref())
                    .ok_or_else(|| "Unable to derive premiere reminder time".to_string())?;
            return Ok(Some(ReminderScheduleTarget {
                title: show_title,
                poster_path,
                season_number: None,
                episode_number: None,
                release_date: raw.first_air_date,
                reminder_at,
                source: "tmdb".to_string(),
                tracking_season_number: None,
            }));
        }
    }

    let Some(tracking_season) = tracking_season else {
        return Ok(None);
    };

    if let Some(next_episode) = raw.next_episode_to_air.clone() {
        if next_episode.season_number == Some(tracking_season)
            && is_release_date_in_future(next_episode.air_date.as_deref())
        {
            return Ok(build_target(next_episode, Some(tracking_season)));
        }
        if let Some(next_season) = next_episode.season_number {
            if next_season > tracking_season {
                return Ok(None);
            }
        }
    }

    let season_url = build_tmdb_api_url(
        &format!("/tv/{}/season/{}", tmdb_id_num, tracking_season),
        credential,
        "",
    );
    let raw_season: RawSeasonDetails = http_get_with_retry_auth(&season_url, credential, 3)?
        .json()
        .map_err(|e| e.to_string())?;

    let next_future = raw_season
        .episodes
        .unwrap_or_default()
        .into_iter()
        .filter_map(|episode| {
            let air_date = episode.air_date.clone()?;
            let target = suggested_reminder_at_from_release_date(Some(&air_date))?;
            let parsed = DateTime::parse_from_rfc3339(&target)
                .ok()?
                .with_timezone(&Utc);
            if parsed <= *now_utc {
                return None;
            }

            Some((
                air_date.clone(),
                RawAirEpisode {
                    name: episode.name,
                    air_date: Some(air_date),
                    season_number: Some(tracking_season),
                    episode_number: episode.episode_number,
                },
            ))
        })
        .min_by_key(|(air_date, _)| air_date.clone())
        .map(|(_, episode)| episode);

    if let Some(next_episode) = next_future {
        return Ok(build_target(next_episode, Some(tracking_season)));
    }

    let has_same_season_metadata = raw
        .seasons
        .unwrap_or_default()
        .into_iter()
        .any(|season| season.season_number == tracking_season && season.episode_count > 0);

    if has_same_season_metadata {
        return Ok(None);
    }

    Ok(None)
}

// Search metadata for streaming - returns raw search results.
// Routes to TMDB if the user has set an API key, otherwise to the free
// api.imdbapi.dev service.
#[tauri::command]
async fn search_tmdb(
    state: State<'_, AppState>,
    query: String,
) -> Result<TmdbSearchResponse, String> {
    println!("[SEARCH_META] Starting search for: {}", query);

    let credential = {
        let config = state.config.lock().map_err(|e| {
            println!("[SEARCH_META] Failed to lock config: {}", e);
            e.to_string()
        })?;
        let key = tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default());
        println!(
            "[SEARCH_META] API key length: {} (is_token: {})",
            key.len(),
            is_access_token(&key)
        );
        key
    };

    let source = match tmdb::pick_source(&credential) {
        tmdb::MetadataSource::Tmdb => "TMDB (themoviedb.org)",
        tmdb::MetadataSource::Cinemeta => {
            "Cinemeta (v3-cinemeta.strem.io) → Balloonerismm (balloonerismm.workers.dev)"
        }
    };
    println!("[SEARCH_META] Source: {}", source);

    let raw_results =
        tokio::task::spawn_blocking(move || -> Result<Vec<tmdb::TmdbSearchListItem>, String> {
            tmdb::search_multi_raw_with_fallback(&credential, &query, None)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;

    let results = raw_results
        .into_iter()
        .map(|item| TmdbSearchResultItem {
            id: item.id,
            title: item.title,
            name: item.name,
            media_type: item.media_type,
            poster_path: item.poster_path,
            backdrop_path: item.backdrop_path,
            overview: item.overview,
            release_date: item.release_date,
            first_air_date: item.first_air_date,
            vote_average: item.vote_average,
            imdb_id: item.imdb_id,
        })
        .collect::<Vec<_>>();

    Ok(TmdbSearchResponse {
        total_results: results.len(),
        results,
    })
}

#[derive(serde::Serialize)]
struct HybridSearchResult {
    title: String,
    year: Option<String>,
    imdb_id: String,
    media_type: String,
    plot: Option<String>,
    poster_url: Option<String>,
    genre: Option<String>,
    director: Option<String>,
    actors: Option<String>,
    imdb_rating: Option<f64>,
    // TMDB cross-ref data
    tmdb_id: Option<i64>,
    tmdb_poster_path: Option<String>,
    tmdb_backdrop_path: Option<String>,
    tmdb_vote_average: Option<f64>,
}

#[derive(serde::Serialize)]
struct HybridSearchResponse {
    results: Vec<HybridSearchResult>,
}

#[tauri::command]
async fn search_content(
    state: State<'_, AppState>,
    query: String,
    _year: Option<i32>,
    media_type: Option<String>,
) -> Result<HybridSearchResponse, String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };

    let tmdb_credential = tmdb::get_tmdb_credential(&config.tmdb_api_key.unwrap_or_default());

    let tmdb_c = tmdb_credential.clone();
    let q = query.clone();
    let mt = media_type.clone();

    let results =
        tokio::task::spawn_blocking(move || -> Result<Vec<HybridSearchResult>, String> {
            let search_type = mt.as_deref();
            let tmdb_results = tmdb::search_multi_raw_with_fallback(&tmdb_c, &q, search_type)
                .map_err(|e| e.to_string())?;

            Ok(tmdb_results
                .into_iter()
                .filter(|item| mt.as_ref().map_or(true, |want| item.media_type == *want))
                .map(|item| HybridSearchResult {
                    title: item.title.unwrap_or_else(|| item.name.unwrap_or_default()),
                    year: item
                        .release_date
                        .as_deref()
                        .or(item.first_air_date.as_deref())
                        .and_then(|d| d.get(..4))
                        .map(|y| y.to_string()),
                    imdb_id: item.imdb_id.clone().unwrap_or_default(),
                    media_type: item.media_type,
                    plot: item.overview,
                    poster_url: item.poster_path.as_ref().map(|p| {
                        if p.starts_with("http") {
                            p.clone()
                        } else {
                            format!("https://image.tmdb.org/t/p/w500{}", p)
                        }
                    }),
                    genre: None,
                    director: None,
                    actors: None,
                    imdb_rating: item.vote_average,
                    tmdb_id: if item.imdb_id.is_some() {
                        None
                    } else {
                        Some(item.id)
                    },
                    tmdb_poster_path: item.poster_path.clone(),
                    tmdb_backdrop_path: item.backdrop_path.clone(),
                    tmdb_vote_average: item.vote_average,
                })
                .collect())
        })
        .await
        .map_err(|e| e.to_string())??;

    Ok(HybridSearchResponse { results })
}

#[tauri::command]
async fn get_tmdb_trending(state: State<'_, AppState>) -> Result<TmdbTrendingResponse, String> {
    println!("[TRENDING] Fetching trending movie and TV suggestions");

    let credential = {
        let config = state.config.lock().map_err(|e| {
            println!("[TRENDING] Failed to lock config: {}", e);
            e.to_string()
        })?;
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    if credential.trim().is_empty() {
        let raw = tokio::task::spawn_blocking(
            move || -> Result<Vec<tmdb::TmdbTrendingListItem>, String> {
                let mut out: Vec<tmdb::TmdbTrendingListItem> = Vec::new();
                for kind in [
                    cinemeta_api::CinemetaKind::Movie,
                    cinemeta_api::CinemetaKind::Series,
                ] {
                    if let Ok(items) = cinemeta_api::trending(kind, 3) {
                        for t in items {
                            out.push(tmdb::TmdbTrendingListItem {
                                id: t.moviedb_id.unwrap_or_else(|| {
                                    t.id.trim_start_matches("tt")
                                        .chars()
                                        .take_while(|c| c.is_ascii_digit())
                                        .collect::<String>()
                                        .parse::<i64>()
                                        .unwrap_or(0)
                                }),
                                title: t.name,
                                media_type: kind.as_str().to_string(),
                            });
                        }
                    }
                }
                Ok(out)
            },
        )
        .await
        .map_err(|e| e.to_string())??;
        let results = raw
            .into_iter()
            .map(|item| TmdbTrendingItem {
                id: item.id,
                title: item.title,
                media_type: item.media_type,
            })
            .collect();
        return Ok(TmdbTrendingResponse { results });
    }

    let raw_results =
        tokio::task::spawn_blocking(move || -> Result<Vec<tmdb::TmdbTrendingListItem>, String> {
            tmdb::trending_suggestions_raw(&credential, 3).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;

    let results = raw_results
        .into_iter()
        .map(|item| TmdbTrendingItem {
            id: item.id,
            title: item.title,
            media_type: item.media_type,
        })
        .collect();

    Ok(TmdbTrendingResponse { results })
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
    // Security check: Verify file is in library
    let is_authorized = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.media_exists(&file_path).map_err(|e| e.to_string())?
    };

    if !is_authorized {
        println!(
            "[SECURITY] Blocked access to non-library file for transcoding: {}",
            file_path
        );
        return Err("Access denied: File not found in library".to_string());
    }

    let ffmpeg_path = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        config.ffmpeg_path.clone().ok_or_else(|| {
            "FFmpeg path not configured. Please set it in Settings > Player.".to_string()
        })?
    };

    if ffmpeg_path.is_empty() || !std::path::Path::new(&ffmpeg_path).exists() {
        return Err(
            "FFmpeg path not set or invalid. Please configure it in Settings > Player.".to_string(),
        );
    }

    let (session_id, stream_url) =
        transcoder::start_transcode(&ffmpeg_path, &file_path, start_time)?;

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
    let is_zip_media = media.parent_zip_id.is_some();

    // Handle cloud media - same as get_stream_info
    if is_cloud {
        if is_zip_media {
            match archive_manager::archive_format_for_media(&media) {
                archive_manager::ArchiveFormat::Zip => {
                    match zip_manager::zip_entry_compression_method(&media)
                        .map_err(|e| e.to_string())?
                    {
                        0 => {
                            let stream_url = build_zip_stream_url(&state, &media, media_id).await?;

                            return Ok(StreamInfo {
                                stream_url,
                                file_path,
                                title: media.title,
                                poster: poster_asset_url(media.poster_path.as_ref()),
                                duration_seconds: media.duration_seconds,
                                resume_position_seconds: media.resume_position_seconds,
                                is_cloud: true,
                                access_token: None,
                            });
                        }
                        8 => {}
                        method => {
                            return Err(format!(
                                "ZIP entry compression method {} is not supported for playback",
                                method
                            ));
                        }
                    }
                }
                archive_manager::ArchiveFormat::Tar | archive_manager::ArchiveFormat::Rar => {
                    if archive_manager::build_archive_stream_info(&media).is_ok() {
                        let stream_url = build_zip_stream_url(&state, &media, media_id).await?;

                        return Ok(StreamInfo {
                            stream_url,
                            file_path,
                            title: media.title,
                            poster: poster_asset_url(media.poster_path.as_ref()),
                            duration_seconds: media.duration_seconds,
                            resume_position_seconds: media.resume_position_seconds,
                            is_cloud: true,
                            access_token: None,
                        });
                    }
                }
            }

            let extracted_path = build_zip_extracted_path(&state, &media).await?;
            let poster = poster_asset_url(media.poster_path.as_ref());
            let needs_transcode = transcoder::needs_transcoding(&extracted_path);

            if needs_transcode {
                let ffmpeg_path = {
                    let config = state.config.lock().map_err(|e| e.to_string())?;
                    config.ffmpeg_path.clone()
                };

                if let Some(ref path) = ffmpeg_path {
                    if !path.is_empty() && std::path::Path::new(path).exists() {
                        let start_time = media.resume_position_seconds;
                        let (_, stream_url) =
                            transcoder::start_transcode(path, &extracted_path, start_time)?;

                        return Ok(StreamInfo {
                            stream_url,
                            file_path: extracted_path,
                            title: media.title,
                            poster,
                            duration_seconds: media.duration_seconds,
                            resume_position_seconds: Some(0.0),
                            is_cloud: false,
                            access_token: None,
                        });
                    }
                }

                return Err("This archived video format requires transcoding. Please configure FFmpeg in Settings > Player, or use MPV/VLC player instead.".to_string());
            }

            return Ok(StreamInfo {
                stream_url: extracted_path.clone(),
                file_path: extracted_path,
                title: media.title,
                poster,
                duration_seconds: media.duration_seconds,
                resume_position_seconds: media.resume_position_seconds,
                is_cloud: false,
                access_token: None,
            });
        }

        if let Some(ref cloud_file_id) = media.cloud_file_id {
            let (stream_url, access_token) =
                state.gdrive_client.get_stream_url(cloud_file_id).await?;

            return Ok(StreamInfo {
                stream_url,
                file_path,
                title: media.title,
                poster: poster_asset_url(media.poster_path.as_ref()),
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

                let poster = poster_asset_url(media.poster_path.as_ref());

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
        let poster = poster_asset_url(media.poster_path.as_ref());

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

    for entry in walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
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
        message: format!(
            "Cleaned up {} files, freed {:.1} MB",
            deleted_count, freed_mb
        ),
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
                                                println!(
                                                    "[CACHE] Deleted expired: {:?}",
                                                    file_path
                                                );
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

    println!(
        "[CACHE] Cleanup complete: {} files deleted, {} bytes freed",
        deleted_count, freed_bytes
    );
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
fn create_main_window(app: &AppHandle) -> Result<WebviewWindow, tauri::Error> {
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title(runtime_window_title())
        .inner_size(1200.0, 800.0)
        .resizable(true)
        .decorations(false)
        .build()?;

    apply_window_corner_radius(&window);

    Ok(window)
}

fn restore_or_create_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        window.unminimize().ok();
        window.show().ok();
        window.set_focus().ok();
        return;
    }

    println!("[WINDOW] Main window missing, creating a new one...");
    match create_main_window(app) {
        Ok(window) => {
            window.unminimize().ok();
            window.show().ok();
            window.set_focus().ok();
            println!("[WINDOW] Main window created");
        }
        Err(e) => {
            println!("[WINDOW] Failed to create main window: {}", e);
        }
    }
}

fn is_dev_runtime() -> bool {
    cfg!(debug_assertions)
}

fn runtime_window_title() -> &'static str {
    if is_dev_runtime() {
        "SlasshyVault Dev"
    } else {
        "SlasshyVault"
    }
}

fn runtime_app_identifier() -> &'static str {
    if is_dev_runtime() {
        "com.slasshyvault.app.dev"
    } else {
        "com.slasshyvault.app"
    }
}

fn runtime_deep_link_scheme() -> &'static str {
    if is_dev_runtime() {
        "slasshyvault-dev"
    } else {
        "slasshyvault"
    }
}

#[cfg(target_os = "windows")]
fn apply_window_corner_radius(window: &WebviewWindow) {
    use windows_sys::Win32::Graphics::Dwm::DwmSetWebviewWindowAttribute;

    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: u32 = 2;

    if let Ok(hwnd) = window.hwnd() {
        let preference = DWMWCP_ROUND;
        unsafe {
            let _ = DwmSetWebviewWindowAttribute(
                hwnd.0 as _,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &preference as *const _ as *const std::ffi::c_void,
                std::mem::size_of_val(&preference) as u32,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_window_corner_radius(_window: &WebviewWindow) {}

fn send_system_notification(app_handle: &AppHandle, summary: &str, body: &str) {
    #[cfg(target_os = "windows")]
    {
        let windows_app_id = app_handle.config().identifier.clone();
        let mut notification = SystemNotification::new();
        notification
            .summary(summary)
            .body(body)
            .appname("SlasshyVault")
            .app_id(&windows_app_id)
            .timeout(notify_rust::Timeout::Milliseconds(5000));
        if let Err(err) = notification.show() {
            println!("[NOTIFY] notify-rust failed: {}", err);
        }
        return;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut notification = SystemNotification::new();
        notification
            .summary(summary)
            .body(body)
            .appname("SlasshyVault")
            .timeout(notify_rust::Timeout::Milliseconds(5000));

        if let Err(err) = notification.show() {
            println!("[NOTIFY] notify-rust failed: {}", err);
        }
    }
}

fn format_reminder_notification_body(reminder: &database::MovieReminder) -> String {
    match (
        reminder.media_type.as_str(),
        reminder.season_number,
        reminder.episode_number,
    ) {
        ("tv", Some(season), Some(episode)) => {
            format!(
                "{} S{:02}E{:02} is ready to watch.",
                reminder.title, season, episode
            )
        }
        ("tv", _, _) => format!("{} is on your watch reminder list.", reminder.title),
        _ => format!("{} is ready to watch.", reminder.title),
    }
}

fn should_continue_tv_reminder(reminder: &database::MovieReminder) -> bool {
    reminder.media_type == "tv" && reminder.tracking_mode == "tv_season"
}

fn format_watchlist_notification_body(item: &database::WatchlistItem) -> String {
    if item.notification_mode == "spam" {
        format!("{} is still waiting in your watchlist.", item.title)
    } else {
        format!("{} is on your watchlist.", item.title)
    }
}

async fn run_watchlist_scheduler(app_handle: AppHandle) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));

    loop {
        interval.tick().await;

        let notifications_enabled = {
            let state = app_handle.state::<AppState>();
            let result = match state.config.lock() {
                Ok(config) => config.notifications_enabled,
                Err(error) => {
                    println!("[WATCHLIST] Failed to lock config: {}", error);
                    false
                }
            };
            result
        };

        if !notifications_enabled {
            continue;
        }

        let now_utc = Utc::now().to_rfc3339();
        let due_items = {
            let state = app_handle.state::<AppState>();
            let result = match state.db.lock() {
                Ok(db) => db.get_due_watchlist_notifications(&now_utc),
                Err(error) => {
                    println!("[WATCHLIST] Failed to lock database: {}", error);
                    continue;
                }
            };
            result
        };

        let due_items = match due_items {
            Ok(items) => items,
            Err(error) => {
                println!("[WATCHLIST] Failed to load due notifications: {}", error);
                continue;
            }
        };

        for item in due_items {
            send_system_notification(
                &app_handle,
                "SlasshyVault watchlist",
                &format_watchlist_notification_body(&item),
            );
            let _ = app_handle.emit("watchlist-reminder-fired", item.clone());

            let state = app_handle.state::<AppState>();
            if let Ok(db) = state.db.lock() {
                let result = if item.notification_mode == "spam" {
                    let minutes = item.notification_interval_minutes.unwrap_or(30).max(1) as i64;
                    let next_notify_at =
                        (Utc::now() + chrono::Duration::minutes(minutes)).to_rfc3339();
                    db.advance_watchlist_notification(item.id, &next_notify_at, &now_utc)
                } else {
                    db.disable_watchlist_notification(item.id, &now_utc)
                };

                if let Err(error) = result {
                    println!(
                        "[WATCHLIST] Failed to advance notification {}: {}",
                        item.id, error
                    );
                } else {
                    let _ = app_handle.emit("refresh-watchlist", ());
                }
            }

            let _ = sync_watchlist_to_drive(&state).await;
        }
    }
}

async fn run_movie_reminder_scheduler(app_handle: AppHandle) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));

    loop {
        interval.tick().await;

        let notifications_enabled = {
            let state = app_handle.state::<AppState>();
            let result = match state.config.lock() {
                Ok(config) => config.notifications_enabled,
                Err(error) => {
                    println!("[REMINDERS] Failed to lock config: {}", error);
                    false
                }
            };
            result
        };

        if !notifications_enabled {
            continue;
        }

        let now_utc = Utc::now().to_rfc3339();
        let due_reminders = {
            let state = app_handle.state::<AppState>();
            let result = match state.db.lock() {
                Ok(db) => db.get_due_movie_reminders(&now_utc),
                Err(error) => {
                    println!("[REMINDERS] Failed to lock database: {}", error);
                    continue;
                }
            };
            result
        };

        let due_reminders = match due_reminders {
            Ok(items) => items,
            Err(error) => {
                println!("[REMINDERS] Failed to load due reminders: {}", error);
                continue;
            }
        };

        for reminder in due_reminders {
            let body = format_reminder_notification_body(&reminder);
            send_system_notification(&app_handle, "SlasshyVault reminder", &body);
            let _ = app_handle.emit("movie-reminder-fired", reminder.clone());

            if should_continue_tv_reminder(&reminder) {
                let credential = {
                    let state = app_handle.state::<AppState>();
                    let resolved = match state.config.lock() {
                        Ok(config) => tmdb::get_tmdb_credential(
                            &config.tmdb_api_key.clone().unwrap_or_default(),
                        ),
                        Err(error) => {
                            println!(
                                "[REMINDERS] Failed to lock config for reminder {}: {}",
                                reminder.id, error
                            );
                            String::new()
                        }
                    };
                    resolved
                };

                let now_dt = Utc::now();
                let next_target = if credential.is_empty() {
                    Err("Missing TMDB credential".to_string())
                } else {
                    tokio::task::spawn_blocking({
                        let reminder = reminder.clone();
                        let credential = credential.clone();
                        move || {
                            resolve_next_tv_season_reminder_target(&reminder, &credential, &now_dt)
                        }
                    })
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|result| result)
                };

                match next_target {
                    Ok(Some(next_target)) => {
                        let state = app_handle.state::<AppState>();
                        if let Ok(db) = state.db.lock() {
                            if let Err(error) = db.advance_movie_reminder(
                                reminder.id,
                                &next_target.title,
                                next_target.poster_path.as_deref(),
                                next_target.season_number,
                                next_target.episode_number,
                                next_target.release_date.as_deref(),
                                &next_target.reminder_at,
                                &next_target.source,
                                next_target.tracking_season_number,
                                &now_utc,
                            ) {
                                println!(
                                    "[REMINDERS] Failed to advance reminder {}: {}",
                                    reminder.id, error
                                );
                            } else {
                                let _ = app_handle.emit("refresh-reminders", ());
                            }
                        }
                        continue;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        println!(
                            "[REMINDERS] Failed to resolve next TV reminder target for {}: {}",
                            reminder.id, error
                        );
                        continue;
                    }
                }
            }

            let state = app_handle.state::<AppState>();
            if let Ok(db) = state.db.lock() {
                if let Err(error) = db.mark_movie_reminder_notified(reminder.id, &now_utc) {
                    println!(
                        "[REMINDERS] Failed to mark reminder {} notified: {}",
                        reminder.id, error
                    );
                } else {
                    let _ = app_handle.emit("refresh-reminders", ());
                }
            };
        }
    }
}

/// Format a standardized "added" notification message for a single item.
/// For TV episodes, includes the S##E## designator.
fn format_added_notification(
    title: &str,
    is_tv: bool,
    season: Option<i32>,
    episode: Option<i32>,
) -> String {
    if is_tv {
        format!(
            "{} S{:02}E{:02} added to your library",
            title,
            season.unwrap_or(1),
            episode.unwrap_or(1)
        )
    } else {
        format!("{} added to your library", title)
    }
}

/// Format a standardized "removed" notification message.
/// When count > 1, uses plural form; otherwise uses the item title directly.
#[allow(dead_code)]
fn format_removed_notification(title: &str, count: Option<usize>) -> String {
    match count {
        Some(n) if n > 1 => format!("{} - {} items removed (deleted from Drive)", title, n),
        _ => format!("{} removed (deleted from Drive)", title),
    }
}

type IndexedCloudItem = (
    i64,
    String,
    String,
    bool,
    Option<i32>,
    Option<i32>,
    String,
    Option<i32>,
);

fn poster_asset_url(poster_path: Option<&String>) -> Option<String> {
    poster_path.and_then(|path| {
        let cache_dir = database::get_image_cache_dir();
        // Prevent path traversal by extracting just the file name.
        // If no valid file name is found (e.g. path is empty or ".."), return None.
        let file_name = std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())?;

        let full_path = std::path::Path::new(&cache_dir).join(file_name);

        let path_cow = full_path.to_string_lossy();
        let path_str = path_cow.as_ref();

        #[cfg(windows)]
        let path_str = if path_str.starts_with(r"\\?\") {
            &path_str[4..]
        } else {
            path_str
        };

        Some(format!(
            "asset://localhost/{}",
            path_str.replace("\\", "/").replace(":", "")
        ))
    })
}

fn take_expired_zip_streams(state: &AppState) -> Vec<zip_stream_proxy::ZipStreamProxyHandle> {
    if let Ok(mut streams) = state.active_zip_streams.lock() {
        let stale_ids: Vec<i64> = streams
            .iter()
            .filter_map(|(media_id, stream)| {
                if stream.created_at.elapsed() > Duration::from_secs(21_600) {
                    Some(*media_id)
                } else {
                    None
                }
            })
            .collect();

        let mut proxies = Vec::with_capacity(stale_ids.len());

        for media_id in stale_ids {
            if let Some(stream) = streams.remove(&media_id) {
                proxies.push(stream.proxy);
            }
        }

        return proxies;
    }

    Vec::new()
}

fn take_zip_stream_proxy(
    state: &AppState,
    media_id: i64,
) -> Option<zip_stream_proxy::ZipStreamProxyHandle> {
    if let Ok(mut streams) = state.active_zip_streams.lock() {
        return streams.remove(&media_id).map(|stream| stream.proxy);
    }

    None
}

fn build_zip_cache_config(config: &config::Config) -> zip_manager::ZipCacheConfig {
    let cache_dir = config
        .zip_cache_dir
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(database::get_zip_cache_dir);

    zip_manager::ZipCacheConfig {
        cache_dir,
        max_size_bytes: u64::from(config.zip_cache_max_gb.max(1))
            .saturating_mul(1024 * 1024 * 1024),
        expiry_days: config.zip_cache_expiry_days.max(1),
    }
}

fn should_extract_zip_for_mpv(
    media: &database::MediaItem,
    _start_position: f64,
) -> Result<bool, String> {
    match archive_manager::archive_format_for_media(media) {
        archive_manager::ArchiveFormat::Zip => {
            match zip_manager::zip_entry_compression_method(media).map_err(|e| e.to_string())? {
                8 => Ok(true),
                0 => Ok(false),
                method => Err(format!(
                    "ZIP entry compression method {} is not supported for playback",
                    method
                )),
            }
        }
        archive_manager::ArchiveFormat::Tar | archive_manager::ArchiveFormat::Rar => {
            Ok(archive_manager::build_archive_stream_info(media).is_err())
        }
    }
}

fn choose_store_zip_local_cache_for_mpv(
    media: &database::MediaItem,
    cache_snapshot: &zip_manager::ZipCacheSnapshot,
    start_position: f64,
) -> Option<String> {
    if cache_snapshot.is_complete {
        return Some(
            cache_snapshot
                .paths
                .cache_path
                .to_string_lossy()
                .to_string(),
        );
    }

    if cache_snapshot.available_bytes == 0 {
        return None;
    }
    let is_mkv = media
        .zip_entry_path
        .as_deref()
        .or(media.file_path.as_deref())
        .map(|path| path.to_ascii_lowercase().ends_with(".mkv"))
        .unwrap_or(false);

    let duration_seconds = media.duration_seconds?;
    let total_size = media.zip_uncompressed_size? as f64;
    if duration_seconds <= 0.0 || total_size <= 0.0 {
        return None;
    }

    let bytes_per_second = total_size / duration_seconds;
    let startup_floor_bytes = if start_position > 0.0 {
        if is_mkv {
            24 * 1024 * 1024
        } else {
            18 * 1024 * 1024
        }
    } else if is_mkv {
        10 * 1024 * 1024
    } else {
        6 * 1024 * 1024
    } as f64;

    let required_window_seconds = if start_position > 0.0 {
        if is_mkv {
            8.0
        } else {
            5.0
        }
    } else if is_mkv {
        2.5
    } else {
        1.5
    };

    let required_bytes =
        ((start_position + required_window_seconds) * bytes_per_second).max(startup_floor_bytes);

    if cache_snapshot.available_bytes as f64 >= required_bytes {
        return Some(cache_snapshot.paths.temp_path.to_string_lossy().to_string());
    }

    if start_position <= 1.0 && cache_snapshot.available_bytes as f64 >= startup_floor_bytes {
        return Some(cache_snapshot.paths.temp_path.to_string_lossy().to_string());
    }

    None
}

async fn build_zip_extracted_path(
    state: &AppState,
    media: &database::MediaItem,
) -> Result<String, String> {
    let cache_config = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        build_zip_cache_config(&config)
    };

    if let Err(error) = zip_manager::cleanup_stale_zip_cache(&cache_config) {
        println!("[ZIP] Cache cleanup warning: {}", error);
    }

    let is_ddl = media.ddl_source_id.is_some();
    let access_token = if is_ddl {
        String::new()
    } else {
        state.gdrive_client.get_access_token().await?
    };
    let media = media.clone();
    let cache_config = cache_config.clone();

    tokio::task::spawn_blocking(move || {
        archive_manager::extract_archive_entry_to_cache(&access_token, &media, &cache_config)
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn build_zip_stream_url(
    state: &AppState,
    media: &database::MediaItem,
    media_id: i64,
) -> Result<String, String> {
    let cache_config = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        build_zip_cache_config(&config)
    };

    if let Err(error) = zip_manager::cleanup_stale_zip_cache(&cache_config) {
        println!("[ZIP] Cache cleanup warning: {}", error);
    }
    let mut proxies_to_stop = take_expired_zip_streams(state);
    if let Some(proxy) = take_zip_stream_proxy(state, media_id) {
        proxies_to_stop.push(proxy);
    }
    for proxy in proxies_to_stop {
        let _ = stop_zip_proxy_handle_blocking(proxy).await;
    }

    let stream_info = archive_manager::build_archive_stream_info(media)?;

    let is_ddl = media.ddl_source_id.is_some();

    let drive_url = if is_ddl {
        let ddl_source_id = media
            .ddl_source_id
            .as_deref()
            .or(media.parent_zip_id.as_deref())
            .ok_or_else(|| "DDL media missing source ID".to_string())?;
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let source = db
            .get_ddl_source(ddl_source_id)
            .map_err(|e| e.to_string())?;
        if source.is_expired {
            return Err("Link expired. Please refresh the link with a new URL.".to_string());
        }
        source.url
    } else {
        state
            .gdrive_client
            .build_stream_url(&stream_info.zip_file_id)
    };
    let proxy_cache_spec = match archive_manager::archive_format_for_media(media) {
        archive_manager::ArchiveFormat::Zip => {
            match zip_manager::zip_entry_compression_method(media) {
                Ok(0) => match zip_manager::prepare_stream_cache_target(media, &cache_config) {
                    Ok(cache_paths) => Some(zip_stream_proxy::ProxyCacheSpec {
                        cache_paths,
                        cache_config: cache_config.clone(),
                        start_delay_ms: 4_000,
                        throttle_delay_ms: 250,
                    }),
                    Err(error) => {
                        println!(
                            "[ZIP CACHE] Falling back to stream-only built-in proxy for '{}': {:?}",
                            media.title, error
                        );
                        None
                    }
                },
                _ => None,
            }
        }
        archive_manager::ArchiveFormat::Tar | archive_manager::ArchiveFormat::Rar => None,
    };
    let proxy_spec = if is_ddl {
        zip_stream_proxy::build_direct_link_proxy_spec(drive_url, &stream_info, proxy_cache_spec)
    } else {
        zip_stream_proxy::build_proxy_spec(
            drive_url,
            state.gdrive_client.clone(),
            &stream_info,
            proxy_cache_spec,
        )
    };

    let proxy = start_zip_proxy_blocking(proxy_spec).await?;
    let stream_url = zip_stream_proxy::localhost_stream_url(proxy.port);

    let mut streams = state.active_zip_streams.lock().map_err(|e| e.to_string())?;
    streams.insert(
        media_id,
        ActiveZipStream {
            created_at: std::time::Instant::now(),
            proxy,
        },
    );

    Ok(stream_url)
}

async fn build_temporary_zip_stream_url(
    state: &AppState,
    media: &database::MediaItem,
) -> Result<(String, zip_stream_proxy::ZipStreamProxyHandle), String> {
    let stream_info = archive_manager::build_archive_stream_info(media)?;
    let proxy_spec = if let Some(ddl_source_id) = media
        .ddl_source_id
        .as_deref()
        .or(media.parent_zip_id.as_deref())
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let source = db
            .get_ddl_source(ddl_source_id)
            .map_err(|e| e.to_string())?;
        if source.is_expired {
            return Err("Link expired. Please refresh the link with a new URL.".to_string());
        }

        zip_stream_proxy::build_direct_link_proxy_spec(source.url, &stream_info, None)
    } else {
        let drive_url = state
            .gdrive_client
            .build_stream_url(&stream_info.zip_file_id);
        zip_stream_proxy::build_proxy_spec(
            drive_url,
            state.gdrive_client.clone(),
            &stream_info,
            None,
        )
    };

    let proxy = start_zip_proxy_blocking(proxy_spec).await?;

    Ok((zip_stream_proxy::localhost_stream_url(proxy.port), proxy))
}

fn is_zip_drive_item(file: &gdrive::DriveItem) -> bool {
    gdrive::is_supported_archive_item(file)
}

fn is_unsupported_archive_drive_item(file: &gdrive::DriveItem) -> bool {
    gdrive::is_unsupported_archive_item(file)
}

fn unsupported_archive_reason(file: &gdrive::DriveItem) -> Option<String> {
    if !is_unsupported_archive_drive_item(file) {
        return None;
    }

    Some(
        "TAR archives are not supported because indexing them requires reading the entire archive sequentially from Google Drive, which can consume full bandwidth and cannot behave like ZIP store mode."
            .to_string(),
    )
}

fn notify_unsupported_archives_window(window: &WebviewWindow, archive_names: &[String]) {
    if archive_names.is_empty() {
        return;
    }

    let message = if archive_names.len() == 1 {
        format!(
            "{} is not supported. TAR archives require a full sequential read from Google Drive during indexing, which consumes full bandwidth and cannot work like ZIP store mode.",
            archive_names[0]
        )
    } else {
        format!(
            "{} TAR archive(s) were skipped. TAR archives require a full sequential read from Google Drive during indexing, which consumes full bandwidth and cannot work like ZIP store mode.",
            archive_names.len()
        )
    };

    dispatch_notification(window, "Unsupported TAR Archive", &message, "warning");
}

fn notify_unsupported_archives_handle(app_handle: &AppHandle, archive_names: &[String]) {
    if archive_names.is_empty() {
        return;
    }

    let message = if archive_names.len() == 1 {
        format!(
            "{} is not supported. TAR archives require a full sequential read from Google Drive during indexing, which consumes full bandwidth and cannot work like ZIP store mode.",
            archive_names[0]
        )
    } else {
        format!(
            "{} TAR archive(s) were skipped. TAR archives require a full sequential read from Google Drive during indexing, which consumes full bandwidth and cannot work like ZIP store mode.",
            archive_names.len()
        )
    };

    dispatch_notification_from_handle(app_handle, "Unsupported TAR Archive", &message, "warning");
}

fn ensure_cloud_show_with_metadata(
    db: &database::Database,
    api_key: &str,
    image_cache_dir: &str,
    tv_show_cache: &mut HashMap<String, (i64, Option<String>, String)>,
    show_title: &str,
    year: Option<i32>,
    folder_id: &str,
) -> Result<(i64, Option<String>, String), String> {
    let cache_key = show_title.to_lowercase();
    if let Some(cached) = tv_show_cache.get(&cache_key) {
        return Ok(cached.clone());
    }

    let result = if let Some(existing_show) = find_existing_cloud_tvshow(db, show_title, year) {
        (
            existing_show.id,
            existing_show.tmdb_id,
            folder_id.to_string(),
        )
    } else {
        let show_path = cloud_tvshow_path(folder_id, show_title);
        if let Some(existing_show) = find_existing_cloud_tvshow_by_path(db, &show_path) {
            (
                existing_show.id,
                existing_show.tmdb_id,
                folder_id.to_string(),
            )
        } else {
            let tmdb_result = tmdb::search_metadata_with_fallback(
                api_key,
                show_title,
                "tv",
                year,
                image_cache_dir,
            )
            .ok()
            .flatten();

            let (title, year, overview, cast_names, poster_path, tmdb_id_opt) = match &tmdb_result {
                Some(meta) => (
                    meta.title.clone(),
                    meta.year,
                    meta.overview.clone(),
                    meta.cast_names.clone(),
                    meta.poster_path.clone(),
                    meta.tmdb_id.clone(),
                ),
                None => (show_title.to_string(), None, None, None, None, None),
            };

            let preferred_title =
                media_manager::prefer_title_with_leading_article(show_title, &title);
            let show_id = db
                .insert_cloud_tvshow(
                    &preferred_title,
                    year,
                    overview.as_deref(),
                    cast_names.as_deref(),
                    poster_path.as_deref(),
                    &show_path,
                    folder_id,
                    tmdb_id_opt.as_deref(),
                )
                .or_else(|e| {
                    find_existing_cloud_tvshow_by_path(db, &show_path)
                        .map(|existing_show| existing_show.id)
                        .ok_or(e)
                })
                .map_err(|e| e.to_string())?;

            (show_id, tmdb_id_opt, folder_id.to_string())
        }
    };

    tv_show_cache.insert(cache_key, result.clone());
    Ok(result)
}

fn ensure_cloud_show_without_metadata(
    db: &database::Database,
    tv_show_cache: &mut HashMap<String, i64>,
    show_title: &str,
    year: Option<i32>,
    folder_id: &str,
) -> Result<i64, String> {
    let cache_key = show_title.to_lowercase();
    if let Some(cached) = tv_show_cache.get(&cache_key) {
        return Ok(*cached);
    }

    let show_path = cloud_tvshow_path(folder_id, show_title);
    let show_id = if let Some(existing_show) = find_existing_cloud_tvshow(db, show_title, year) {
        existing_show.id
    } else if let Some(existing_show) = find_existing_cloud_tvshow_by_path(db, &show_path) {
        existing_show.id
    } else {
        db.insert_cloud_tvshow(
            show_title, None, None, None, None, &show_path, folder_id, None,
        )
        .or_else(|e| {
            find_existing_cloud_tvshow_by_path(db, &show_path)
                .map(|existing_show| existing_show.id)
                .ok_or(e)
        })
        .map_err(|e| e.to_string())?
    };

    tv_show_cache.insert(cache_key, show_id);
    Ok(show_id)
}

fn fetch_episode_metadata(
    api_key: &str,
    image_cache_dir: &str,
    season_cache: &mut HashMap<(String, i32), Vec<tmdb::TmdbEpisodeInfo>>,
    tmdb_id: Option<&String>,
    show_title: &str,
    season: i32,
    episode: i32,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(tmdb_id) = tmdb_id else {
        return (None, None, None);
    };

    let cache_key = (tmdb_id.clone(), season);
    let episodes = if let Some(cached_episodes) = season_cache.get(&cache_key) {
        cached_episodes.clone()
    } else {
        match tmdb::fetch_season_episodes(api_key, tmdb_id, season, show_title, image_cache_dir) {
            Ok(season_info) => {
                let episodes = season_info.episodes.clone();
                season_cache.insert(cache_key.clone(), episodes.clone());
                episodes
            }
            Err(_) => {
                season_cache.insert(cache_key.clone(), Vec::new());
                Vec::new()
            }
        }
    };

    episodes
        .iter()
        .find(|item| item.episode_number == episode)
        .map(|item| {
            (
                Some(item.name.clone()),
                item.overview.clone(),
                item.still_path.clone(),
            )
        })
        .unwrap_or((None, None, None))
}

fn index_zip_archive_with_metadata(
    db: &database::Database,
    access_token: &str,
    file: &gdrive::DriveItem,
    folder_id_fallback: &str,
    api_key: &str,
    image_cache_dir: &str,
    cache_config: &zip_manager::ZipCacheConfig,
    tv_show_cache: &mut HashMap<String, (i64, Option<String>, String)>,
    season_cache: &mut HashMap<(String, i32), Vec<tmdb::TmdbEpisodeInfo>>,
) -> Result<Vec<IndexedCloudItem>, String> {
    let analyzed = archive_manager::analyze_archive_from_drive(
        access_token,
        &file.id,
        &file.name,
        Some(&file.mime_type),
        cache_config,
    )?;

    if analyzed.indexed_entries.is_empty() {
        return Ok(Vec::new());
    }

    db.insert_zip_archive(&analyzed.archive)
        .map_err(|e| e.to_string())?;
    let archive_format = analyzed.archive.archive_format.clone();

    let folder_id = file
        .parents
        .as_ref()
        .and_then(|parents| parents.first())
        .cloned()
        .unwrap_or_else(|| folder_id_fallback.to_string());

    let mut indexed_items = Vec::new();

    for entry in analyzed.indexed_entries {
        let season = entry.parsed.season.unwrap_or(0);
        let episode = entry.parsed.episode.unwrap_or(0);
        if season <= 0 || episode <= 0 {
            continue;
        }

        let show_title = entry.parsed.title.clone();
        let (show_id, tmdb_id, show_folder_id) = ensure_cloud_show_with_metadata(
            db,
            api_key,
            image_cache_dir,
            tv_show_cache,
            &show_title,
            entry.parsed.year,
            &folder_id,
        )?;

        let (episode_title, overview, still_path) = fetch_episode_metadata(
            api_key,
            image_cache_dir,
            season_cache,
            tmdb_id.as_ref(),
            &show_title,
            season,
            episode,
        );

        let media_id = db
            .insert_cloud_episode_from_zip(
                &show_title,
                show_id,
                season,
                episode,
                &show_folder_id,
                &archive_format,
                &file.id,
                &entry.entry_path,
                entry.local_header_offset as i64,
                entry.data_start_offset as i64,
                entry.compressed_size as i64,
                entry.uncompressed_size as i64,
                &entry.crc32,
                entry.compression_method as i64,
                episode_title.as_deref(),
                overview.as_deref(),
                still_path.as_deref(),
            )
            .map_err(|e| e.to_string())?;

        indexed_items.push((
            media_id,
            show_title,
            file.id.clone(),
            true,
            Some(season),
            Some(episode),
            show_folder_id,
            entry.parsed.year,
        ));
    }

    Ok(indexed_items)
}

fn index_zip_archive_without_metadata(
    db: &database::Database,
    access_token: &str,
    file: &gdrive::DriveItem,
    folder_id_fallback: &str,
    cache_config: &zip_manager::ZipCacheConfig,
    tv_show_cache: &mut HashMap<String, i64>,
) -> Result<Vec<IndexedCloudItem>, String> {
    let analyzed = archive_manager::analyze_archive_from_drive(
        access_token,
        &file.id,
        &file.name,
        Some(&file.mime_type),
        cache_config,
    )?;

    if analyzed.indexed_entries.is_empty() {
        return Ok(Vec::new());
    }

    db.insert_zip_archive(&analyzed.archive)
        .map_err(|e| e.to_string())?;
    let archive_format = analyzed.archive.archive_format.clone();

    let folder_id = file
        .parents
        .as_ref()
        .and_then(|parents| parents.first())
        .cloned()
        .unwrap_or_else(|| folder_id_fallback.to_string());

    let mut indexed_items = Vec::new();

    for entry in analyzed.indexed_entries {
        let season = entry.parsed.season.unwrap_or(0);
        let episode = entry.parsed.episode.unwrap_or(0);
        if season <= 0 || episode <= 0 {
            continue;
        }

        let show_title = entry.parsed.title.clone();
        let show_id = ensure_cloud_show_without_metadata(
            db,
            tv_show_cache,
            &show_title,
            entry.parsed.year,
            &folder_id,
        )?;

        let media_id = db
            .insert_cloud_episode_from_zip(
                &show_title,
                show_id,
                season,
                episode,
                &folder_id,
                &archive_format,
                &file.id,
                &entry.entry_path,
                entry.local_header_offset as i64,
                entry.data_start_offset as i64,
                entry.compressed_size as i64,
                entry.uncompressed_size as i64,
                &entry.crc32,
                entry.compression_method as i64,
                None,
                None,
                None,
            )
            .map_err(|e| e.to_string())?;

        indexed_items.push((
            media_id,
            show_title,
            file.id.clone(),
            true,
            Some(season),
            Some(episode),
            folder_id.clone(),
            entry.parsed.year,
        ));
    }

    Ok(indexed_items)
}

fn emit_ui_notification(window: &WebviewWindow, title: &str, message: &str, kind: &str) {
    let notification_key = format!("{}|{}|{}", kind, title, message);
    let now = std::time::Instant::now();
    let dedupe_window = Duration::from_secs(3);

    if let Ok(mut recent) = RECENT_UI_NOTIFICATIONS.lock() {
        recent.retain(|_, instant| now.duration_since(*instant) <= dedupe_window);
        if let Some(last_seen) = recent.get(&notification_key) {
            if now.duration_since(*last_seen) <= dedupe_window {
                return;
            }
        }
        recent.insert(notification_key, now);
    }

    if let Err(e) = window.emit(
        "notification",
        serde_json::json!({ "type": kind, "title": title, "message": message }),
    ) {
        eprintln!("[NOTIFY] Failed to emit notification: {}", e);
    }
}

fn should_show_in_app_notification(window: &WebviewWindow) -> bool {
    if window.is_minimized().unwrap_or(false) {
        return false;
    }

    window.is_focused().unwrap_or(false)
}

fn dispatch_notification(window: &WebviewWindow, title: &str, message: &str, kind: &str) {
    emit_ui_notification(window, title, message, kind);

    if !should_show_in_app_notification(window) {
        send_system_notification(&window.app_handle(), title, message);
    }
}

fn emit_ui_notification_from_handle(
    app_handle: &AppHandle,
    title: &str,
    message: &str,
    kind: &str,
) {
    if let Some(window) = app_handle.get_webview_window("main") {
        dispatch_notification(&window, title, message, kind);
    } else {
        // WebviewWindow not available, send system notification only
        send_system_notification(app_handle, title, message);
    }
}

fn dispatch_notification_from_handle(
    app_handle: &AppHandle,
    title: &str,
    message: &str,
    kind: &str,
) {
    emit_ui_notification_from_handle(app_handle, title, message, kind);
}

fn emit_zip_processing_event(
    window: &WebviewWindow,
    phase: &str,
    archive_count: usize,
    archive_name: Option<&str>,
    episodes_indexed: Option<usize>,
    message: &str,
) {
    let payload = ZipProcessingEventPayload {
        phase: phase.to_string(),
        archive_count,
        archive_name: archive_name.map(|value| value.to_string()),
        episodes_indexed,
        message: message.to_string(),
    };

    window.emit("zip-processing-status", payload).ok();

    // Send WebviewWindows notification for detected / complete phases
    match phase {
        "detected" => {
            let title = format!(
                "{} archive{} detected",
                archive_count,
                if archive_count == 1 { "" } else { "s" }
            );
            let name = archive_name.unwrap_or("Unknown");
            send_system_notification(
                &window.app_handle(),
                &title,
                &format!("Found in background scan: {}", name),
            );
        }
        "complete" => {
            let count = episodes_indexed.unwrap_or(0);
            let title = format!("📦 Indexing complete");
            let body = if count > 0 {
                format!(
                    "{} episode{} indexed from {} archive{}",
                    count,
                    if count == 1 { "" } else { "s" },
                    archive_count,
                    if archive_count == 1 { "" } else { "s" }
                )
            } else {
                format!(
                    "{} archive{} processed",
                    archive_count,
                    if archive_count == 1 { "" } else { "s" }
                )
            };
            send_system_notification(&window.app_handle(), &title, &body);
        }
        _ => {}
    }
}

fn build_mpv_display_title(media: &database::MediaItem) -> String {
    let inferred_episode_numbers = media
        .zip_entry_path
        .as_deref()
        .or(media.file_path.as_deref())
        .and_then(infer_episode_numbers_from_path);

    let season = media
        .season_number
        .or(inferred_episode_numbers.map(|(season, _)| season));
    let episode = media
        .episode_number
        .or(inferred_episode_numbers.map(|(_, episode)| episode));
    let is_episode = matches!(media.media_type.as_str(), "tv" | "tvepisode" | "episode")
        || season.is_some()
        || episode.is_some();

    if is_episode {
        let season = season.unwrap_or(1);
        let episode = episode.unwrap_or(1);
        let mut display_title = format!("{} S{:02}E{:02}", media.title, season, episode);

        if let Some(episode_title) = media
            .episode_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != media.title)
        {
            display_title.push_str(" - ");
            display_title.push_str(episode_title);
        }

        display_title
    } else {
        media.title.clone()
    }
}

fn infer_episode_numbers_from_path(path: &str) -> Option<(i32, i32)> {
    let pattern = regex::Regex::new(r"(?i)\bS(?P<season>\d{1,2})E(?P<episode>\d{1,3})\b").ok()?;
    let captures = pattern.captures(path)?;
    let season = captures.name("season")?.as_str().parse::<i32>().ok()?;
    let episode = captures.name("episode")?.as_str().parse::<i32>().ok()?;
    Some((season, episode))
}

fn emit_zip_processing_event_from_handle(
    app_handle: &AppHandle,
    phase: &str,
    archive_count: usize,
    archive_name: Option<&str>,
    episodes_indexed: Option<usize>,
    message: &str,
) {
    if let Some(window) = app_handle.get_webview_window("main") {
        emit_zip_processing_event(
            &window,
            phase,
            archive_count,
            archive_name,
            episodes_indexed,
            message,
        );
    }
}

/// Background cloud change detection polling
/// Runs independently of the window to detect new files even when minimized to tray
async fn background_cloud_poll(app_handle: AppHandle) {
    use std::time::Duration;

    // Initial delay to let app fully initialize (same as frontend)
    tokio::time::sleep(Duration::from_secs(3)).await;

    println!("[CLOUD BG] Background cloud polling started (5-second interval)");
    let mut last_zip_cache_cleanup = std::time::Instant::now();
    const ZIP_CACHE_CLEANUP_INTERVAL: Duration = Duration::from_secs(3600); // 1 hour

    loop {
        // Get state from app handle
        let state: tauri::State<'_, AppState> = app_handle.state();

        // Check if authenticated
        if !state.gdrive_client.is_authenticated() {
            // Not connected - wait and retry (silent, don't spam logs)
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        // Periodic zip cache cleanup (runs regardless of cloud poll results)
        if last_zip_cache_cleanup.elapsed() >= ZIP_CACHE_CLEANUP_INTERVAL {
            let zip_cache_config = {
                let config = state
                    .config
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                build_zip_cache_config(&config)
            };
            match tokio::task::spawn_blocking(move || {
                zip_manager::cleanup_stale_zip_cache(&zip_cache_config)
            })
            .await
            {
                Ok(Ok(())) => {
                    println!("[ZIP CACHE] Periodic cleanup completed");
                }
                Ok(Err(e)) => {
                    println!("[ZIP CACHE] Periodic cleanup warning: {}", e);
                }
                Err(e) => {
                    println!("[ZIP CACHE] Periodic cleanup task error: {}", e);
                }
            }
            last_zip_cache_cleanup = std::time::Instant::now();
        }

        // Cloud-only mode: No folder check needed - we monitor entire Drive
        println!("[CLOUD BG] Polling for changes...");

        // Perform the actual check
        match background_check_cloud_changes(&app_handle).await {
            Ok(result) => {
                if result.indexed_count > 0 {
                    println!(
                        "[CLOUD BG] ✓ Indexed {} new items ({} movies, {} TV)",
                        result.indexed_count, result.movies_count, result.tv_count
                    );

                    // Emit event to window if it exists
                    if let Some(window) = app_handle.get_webview_window("main") {
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

/// Background version of check_cloud_changes that doesn't require a WebviewWindow parameter
async fn background_check_cloud_changes(
    app_handle: &AppHandle,
) -> Result<CloudIndexResult, String> {
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
            skipped_reasons: None,
        });
    }

    // Get or initialize the changes token
    let current_token = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_gdrive_changes_token().map_err(|e| e.to_string())?
    };

    let page_token = match current_token {
        Some(token) => {
            println!(
                "[CLOUD BG] Using existing token: {}...",
                &token[..token.len().min(20)]
            );
            token
        }
        None => {
            // First time - get the start token
            println!("[CLOUD BG] No token found - initializing changes tracking...");
            let start_token = state.gdrive_client.get_changes_start_token().await?;
            println!(
                "[CLOUD BG] Got start token: {}...",
                &start_token[..start_token.len().min(20)]
            );
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.set_gdrive_changes_token(&start_token)
                .map_err(|e| e.to_string())?;
            println!("[CLOUD BG] Token saved - will detect changes on next poll");
            println!("[CLOUD BG] ══════════════════════════════════════════");
            return Ok(CloudIndexResult {
                success: true,
                indexed_count: 0,
                skipped_count: 0,
                movies_count: 0,
                tv_count: 0,
                message: "Changes tracking initialized".to_string(),
                skipped_reasons: None,
            });
        }
    };

    // Note: We no longer filter by tracked folders - index all video files in Drive
    println!("[CLOUD BG] Monitoring entire Google Drive for changes");

    // Get changes since last check
    let api_start = std::time::Instant::now();
    let (changed_files, removed_file_ids, new_token) =
        state.gdrive_client.get_video_changes(&page_token).await?;
    let api_duration = api_start.elapsed();
    println!("[CLOUD BG] Changes API call took {:?}", api_duration);

    // Token is saved AFTER indexing completes (see end of function).
    // Saving before indexing would permanently lose failed files from detection.

    let mut removed_titles: Vec<String> = Vec::new();
    if !removed_file_ids.is_empty() {
        if let Ok(db) = state.db.lock() {
            for file_id in removed_file_ids {
                if let Ok(Some((_id, title, _media_type, _parent_id))) =
                    db.remove_media_by_cloud_file_id(&file_id)
                {
                    removed_titles.push(title);
                }
            }

            if !removed_titles.is_empty() {
                let _ = db.cleanup_empty_series();
            }
        }

        if !removed_titles.is_empty() {
            if let Some(window) = app_handle.get_webview_window("main") {
                window.emit("library-updated", ()).ok();
            }

            // Group TV episodes by series name to avoid spamming notifications
            let mut series_episodes: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();
            let mut non_tv_titles: Vec<String> = Vec::new();

            for title in &removed_titles {
                // Check if this is a TV episode (format: "Title SXXEXX" or "Title SXEX")
                // Examples: "Breaking Bad S01E01", "Breaking Bad S1E1", "Breaking Bad S10E15"
                let is_tv_episode = if let Some(pos) = title.find(" S") {
                    let rest = &title[pos + 2..]; // Skip " S"
                    if rest.len() >= 3 && rest.starts_with(|c: char| c.is_ascii_digit()) {
                        // Look for pattern: digit(s) + 'E' + digit(s)
                        if let Some(e_pos) = rest.find(|c: char| c.to_ascii_uppercase() == 'E') {
                            if e_pos > 0 && e_pos < rest.len() - 1 {
                                let season_part = &rest[..e_pos];
                                let episode_part = &rest[e_pos + 1..];
                                // Both season and episode should be numeric
                                season_part.chars().all(|c| c.is_ascii_digit())
                                    && episode_part.chars().all(|c| c.is_ascii_digit())
                                    && !season_part.is_empty()
                                    && !episode_part.is_empty()
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if is_tv_episode {
                    // Extract series name (everything before " S")
                    if let Some(pos) = title.find(" S") {
                        let series_name = &title[..pos];
                        series_episodes
                            .entry(series_name.to_string())
                            .or_insert_with(Vec::new)
                            .push(title.clone());
                        continue;
                    }
                }
                // Not a TV episode, add to regular titles
                non_tv_titles.push(title.clone());
            }

            let mut messages = Vec::new();

            for (series_name, episodes) in &series_episodes {
                let episode_count = episodes.len();
                if episode_count == 1 {
                    messages.push(format!("{} (1 episode)", series_name));
                } else {
                    messages.push(format!("{} ({} episodes)", series_name, episode_count));
                }
            }

            for title in &non_tv_titles {
                messages.push(title.clone());
            }

            if !messages.is_empty() {
                let message = if messages.len() == 1 {
                    format!("{} removed (deleted from Drive)", messages[0])
                } else {
                    format!("{} items removed (deleted from Drive)", messages.len())
                };

                dispatch_notification_from_handle(app_handle, "SlasshyVault", &message, "info");
            }
        }
    }

    if changed_files.is_empty() {
        let total_duration = start_time.elapsed();
        println!(
            "[CLOUD BG] No changes detected (total: {:?})",
            total_duration
        );
        println!("[CLOUD BG] ══════════════════════════════════════════");
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: 0,
            movies_count: 0,
            tv_count: 0,
            message: if removed_titles.is_empty() {
                "No new files detected".to_string()
            } else {
                format!(
                    "Removed {} item(s) deleted from Drive",
                    removed_titles.len()
                )
            },
            skipped_reasons: None,
        });
    }

    println!(
        "[CLOUD BG] Detected {} changed video file(s)",
        changed_files.len()
    );

    // Index all detected video files (no folder filtering)
    let unsupported_archives: Vec<String> = changed_files
        .iter()
        .filter(|file| is_unsupported_archive_drive_item(file))
        .map(|file| file.name.clone())
        .collect();
    if !unsupported_archives.is_empty() {
        notify_unsupported_archives_handle(app_handle, &unsupported_archives);
    }
    let files_to_index: Vec<_> = changed_files
        .into_iter()
        .filter(|file| !is_unsupported_archive_drive_item(file))
        .collect();

    if files_to_index.is_empty() {
        return Ok(CloudIndexResult {
            success: true,
            indexed_count: 0,
            skipped_count: unsupported_archives.len(),
            movies_count: 0,
            tv_count: 0,
            message: if unsupported_archives.is_empty() {
                "No new files detected".to_string()
            } else {
                format!(
                    "Skipped {} unsupported TAR archive(s)",
                    unsupported_archives.len()
                )
            },
            skipped_reasons: None,
        });
    }

    // Get API key from config
    let (api_key, archive_cache_config) = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        (
            tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default()),
            build_zip_cache_config(&config),
        )
    };

    let image_cache_dir = database::get_image_cache_dir();
    std::fs::create_dir_all(&image_cache_dir).ok();
    let db_path = database::get_database_path();
    let zip_indexing_enabled = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        config.zip_indexing_enabled
    };
    let zip_files_detected: Vec<String> = if zip_indexing_enabled {
        files_to_index
            .iter()
            .filter(|file| is_zip_drive_item(file))
            .map(|file| file.name.clone())
            .collect()
    } else {
        Vec::new()
    };
    let zip_access_token = if zip_indexing_enabled && files_to_index.iter().any(is_zip_drive_item) {
        Some(state.gdrive_client.get_access_token().await?)
    } else {
        None
    };

    if !zip_files_detected.is_empty() {
        let archive_name = zip_files_detected.first().map(|name| name.as_str());
        emit_zip_processing_event_from_handle(
            app_handle,
            "detected",
            zip_files_detected.len(),
            archive_name,
            None,
            &format!(
                "Archive{} detected in Google Drive. Processing episode entries...",
                if zip_files_detected.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ),
        );
    }

    // PHASE 1: Add files immediately without metadata
    let phase1_result = {
        let db_path_clone = db_path.clone();
        let files_to_index_clone: Vec<_> = files_to_index
            .iter()
            .map(|f| (f.id.clone(), f.name.clone(), f.parents.clone()))
            .collect();

        tokio::task::spawn_blocking(move || {
            let db = match database::Database::new(&db_path_clone) {
                Ok(d) => d,
                Err(e) => return Err(format!("Failed to open database: {}", e)),
            };

            let mut indexed_items: Vec<(
                i64,
                String,
                String,
                bool,
                Option<i32>,
                Option<i32>,
                String,
                Option<i32>,
            )> = Vec::new();
            let mut skipped_count = 0;
            let mut movies_count = 0;
            let mut tv_count = 0;
            let mut skipped_reasons: Vec<String> = Vec::new();
            let mut entry_counter = 0usize;
            let mut tv_show_cache: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();

            for (file_id, file_name, parents) in files_to_index_clone {
                if db.cloud_file_exists(&file_id) {
                    let _ = db.clear_cloud_index_failure(&file_id);
                    skipped_count += 1;
                    skipped_reasons.push(format!("{} — already indexed (cloud ID exists)", file_name));
                    continue;
                }

                if let Ok(Some(_)) = db.get_media_by_file_path(&file_name) {
                    skipped_count += 1;
                    skipped_reasons.push(format!("{} — already indexed (file path exists)", file_name));
                    continue;
                }

                let folder_id = parents
                    .as_ref()
                    .and_then(|p| p.first())
                    .cloned()
                    .unwrap_or_default();

                let pseudo_file = gdrive::DriveItem {
                    id: file_id.clone(),
                    name: file_name.clone(),
                    mime_type: if file_name.to_ascii_lowercase().ends_with(".zip") {
                        "application/zip".to_string()
                    } else {
                        String::new()
                    },
                    size: None,
                    modified_time: None,
                    parents: parents.clone(),
                    web_content_link: None,
                };

                if is_zip_drive_item(&pseudo_file) {
                    if !zip_indexing_enabled {
                        skipped_count += 1;
                        skipped_reasons.push(format!("{} — ZIP indexing disabled in settings", file_name));
                        continue;
                    }

                    let Some(access_token) = zip_access_token.as_deref() else {
                        skipped_count += 1;
                        skipped_reasons.push(format!("{} — ZIP access token unavailable", file_name));
                        continue;
                    };

                    match index_zip_archive_without_metadata(
                        &db,
                        access_token,
                        &pseudo_file,
                        &folder_id,
                        &archive_cache_config,
                        &mut tv_show_cache,
                    ) {
                                Ok(items) => {
                                    if items.is_empty() {
                                        let _ = db.upsert_cloud_index_failure(
                                            &file_id,
                                            &file_name,
                                            "Archive was scanned but no playable TV episode entries were identified",
                                        );
                                        skipped_count += 1;
                                        skipped_reasons.push(format!("{} — ZIP archive contained no playable TV episodes", file_name));
                                    } else {
                                        let _ = db.clear_cloud_index_failure(&file_id);
                                        tv_count += items.len();
                                        entry_counter += items.len();
                                        indexed_items.extend(items);
                                    }
                                    continue;
                                }
                                Err(error) => {
                                    println!("[ZIP] Failed to index '{}': {}", file_name, error);
                                    let _ = db.upsert_cloud_index_failure(&file_id, &file_name, &error);
                                    skipped_count += 1;
                                    skipped_reasons.push(format!("{} — ZIP indexing error: {}", file_name, error));
                                    continue;
                                }
                    }
                }

                let parsed = media_manager::parse_cloud_filename(&file_name);
                let is_tv_show = parsed.season.is_some() && parsed.episode.is_some();

                if is_tv_show {
                    let season = parsed.season.unwrap();
                    let episode = parsed.episode.unwrap();
                    let show_title = parsed.title.clone();
                    let show_title_lower = show_title.to_lowercase();

                    let db_show_id = if let Some(&cached_id) = tv_show_cache.get(&show_title_lower)
                    {
                        cached_id
                    } else {
                        let show_id = if let Some(existing_show) =
                            find_existing_cloud_tvshow(&db, &show_title, parsed.year)
                        {
                            existing_show.id
                        } else {
                            let show_path = cloud_tvshow_path(&folder_id, &show_title);
                            if let Some(existing_show) =
                                find_existing_cloud_tvshow_by_path(&db, &show_path)
                            {
                                existing_show.id
                            } else {
                                match db.insert_cloud_tvshow(
                                    &show_title,
                                    None,
                                    None,
                                    None,
                                    None,
                                    &show_path,
                                    &folder_id,
                                    None,
                                ) {
                                    Ok(id) => id,
                                    Err(_) => {
                                        if let Some(existing_show) =
                                            find_existing_cloud_tvshow_by_path(&db, &show_path)
                                        {
                                            existing_show.id
                                        } else {
                                            continue;
                                        }
                                    }
                                }
                            }
                        };
                        tv_show_cache.insert(show_title_lower, show_id);
                        show_id
                    };

                    match db.insert_cloud_episode(
                        &show_title,
                        &file_name,
                        db_show_id,
                        season,
                        episode,
                        &file_id,
                        &folder_id,
                        None,
                        None,
                        None,
                        None,
                    ) {
                        Ok(ep_id) => {
                            indexed_items.push((
                                ep_id,
                                show_title,
                                file_id.clone(),
                                true,
                                Some(season),
                                Some(episode),
                                folder_id,
                                parsed.year,
                            ));
                            tv_count += 1;
                            entry_counter += 1;
                            println!(
                                "[INDEX] #{} TV episode '{}' (db_id: {}, cloud_file_id: {})",
                                entry_counter, file_name, ep_id, file_id
                            );
                        }
                        Err(_) => continue,
                    }
                } else {
                    match db.insert_cloud_movie(
                        &parsed.title,
                        parsed.year,
                        None,
                        None,
                        None,
                        None,
                        &file_name,
                        &file_id,
                        &folder_id,
                        0.0,
                        None,
                    ) {
                        Ok(movie_id) => {
                            indexed_items.push((
                                movie_id,
                                parsed.title,
                                file_id.clone(),
                                false,
                                None,
                                None,
                                folder_id,
                                parsed.year,
                            ));
                            movies_count += 1;
                            entry_counter += 1;
                            println!(
                                "[INDEX] #{} Movie '{}' (db_id: {}, cloud_file_id: {})",
                                entry_counter, file_name, movie_id, file_id
                            );
                        }
                        Err(_) => continue,
                    }
                }
            }

            Ok((indexed_items, skipped_count, movies_count, tv_count, skipped_reasons))
        })
        .await
        .map_err(|e| format!("Task failed: {}", e))?
    }?;

    let (indexed_items, skipped_count, movies_count, tv_count, mut skipped_reasons) = phase1_result;

    // Add TAR archive skip reasons
    for archive_name in &unsupported_archives {
        skipped_reasons.push(format!(
            "{} — TAR archives are not supported (requires sequential read)",
            archive_name
        ));
    }
    let skipped_count = skipped_count + unsupported_archives.len();
    let indexed_count = indexed_items.len();

    if !zip_files_detected.is_empty() {
        let archive_name = zip_files_detected.first().map(|name| name.as_str());
        let zip_indexed_count = indexed_count;
        let status_msg = if zip_indexed_count > 0 {
            format!(
                "Finished processing {} ZIP archive(s). {} episode(s) added to your library.",
                zip_files_detected.len(),
                zip_indexed_count
            )
        } else {
            format!(
                "Finished processing {} ZIP archive(s). No episodes could be indexed — check file naming or archive integrity.",
                zip_files_detected.len()
            )
        };
        emit_zip_processing_event_from_handle(
            app_handle,
            if zip_indexed_count > 0 {
                "complete"
            } else {
                "warning"
            },
            zip_files_detected.len(),
            archive_name,
            None,
            &status_msg,
        );
    }

    // Send a consolidated notification for new items (batched to avoid notification spam)
    if indexed_count > 0 {
        // Group additions by type: TV series (grouped) vs movies
        let mut series_episodes: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut movie_count: usize = 0;
        let mut single_movie_title: Option<String> = None;

        for (_, title, _, is_tv, season, episode, _, _) in &indexed_items {
            if *is_tv {
                // Group by series name. Use format_added_notification for single-episode case.
                *series_episodes.entry(title.clone()).or_insert(0) += 1;
                let _ = (season, episode); // consumed by format_added_notification when needed
            } else {
                movie_count += 1;
                if movie_count == 1 {
                    single_movie_title = Some(title.clone());
                }
            }
        }

        // Build consolidated notification messages
        let mut notification_parts: Vec<String> = Vec::new();

        // TV series summary
        for (series_name, ep_count) in &series_episodes {
            if *ep_count == 1 {
                notification_parts.push(format!("{} (1 episode)", series_name));
            } else {
                notification_parts.push(format!("{} ({} episodes)", series_name, ep_count));
            }
        }

        // Movies summary
        if movie_count == 1 {
            if let Some(ref title) = single_movie_title {
                notification_parts.push(format_added_notification(title, false, None, None));
            }
        } else if movie_count > 1 {
            notification_parts.push(format!("{} movies added to your library", movie_count));
        }

        // Send a single consolidated notification
        if !notification_parts.is_empty() {
            let message = if notification_parts.len() == 1 {
                // Single entry: use the specific message directly
                notification_parts[0].clone()
            } else {
                // Multiple entries: summarise
                format!("{} items added to your library", indexed_count)
            };

            dispatch_notification_from_handle(app_handle, "SlasshyVault", &message, "success");
        }

        // Emit library-updated if window exists
        if let Some(window) = app_handle.get_webview_window("main") {
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

                let mut tv_metadata_cache: std::collections::HashMap<
                    String,
                    Option<tmdb::TmdbMetadata>,
                > = std::collections::HashMap::new();
                let mut tv_show_updated: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                let mut season_cache: std::collections::HashMap<
                    (String, i32),
                    Vec<tmdb::TmdbEpisodeInfo>,
                > = std::collections::HashMap::new();

                for (media_id, title, _file_id, is_tv, season_opt, episode_opt, _folder_id, year) in
                    indexed_items
                {
                    if is_tv {
                        let season = season_opt.unwrap_or(1);
                        let episode = episode_opt.unwrap_or(1);
                        let title_lower = title.to_lowercase();

                        let show_meta = if let Some(cached) = tv_metadata_cache.get(&title_lower) {
                            cached.clone()
                        } else {
                            let meta = tmdb::search_metadata_with_fallback(
                                &api_key,
                                &title,
                                "tv",
                                year,
                                &image_cache_dir_bg,
                            )
                            .ok()
                            .flatten();
                            tv_metadata_cache.insert(title_lower.clone(), meta.clone());
                            meta
                        };

                        if let Some(ref meta) = show_meta {
                            if !tv_show_updated.contains(&title_lower) {
                                if let Some(show) = find_existing_cloud_tvshow(&db, &title, None) {
                                    let mut show_meta_to_apply = meta.clone();
                                    show_meta_to_apply.title =
                                        media_manager::prefer_title_with_leading_article(
                                            &title,
                                            &show_meta_to_apply.title,
                                        );
                                    db.update_metadata(show.id, &show_meta_to_apply).ok();
                                }
                                tv_show_updated.insert(title_lower.clone());
                            }

                            if let Some(ref tmdb_id) = meta.tmdb_id {
                                let cache_key = (tmdb_id.clone(), season);
                                let episodes =
                                    if let Some(cached_eps) = season_cache.get(&cache_key) {
                                        cached_eps.clone()
                                    } else {
                                        match tmdb::fetch_season_episodes(
                                            &api_key,
                                            tmdb_id,
                                            season,
                                            &title,
                                            &image_cache_dir_bg,
                                        ) {
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

                                if let Some(ep_info) =
                                    episodes.iter().find(|e| e.episode_number == episode)
                                {
                                    db.update_episode_metadata(
                                        media_id,
                                        Some(&ep_info.name),
                                        ep_info.overview.as_deref(),
                                        ep_info.still_path.as_deref(),
                                    )
                                    .ok();
                                }
                            }
                        }
                    } else {
                        if let Ok(Some(meta)) = tmdb::search_metadata_with_fallback(
                            &api_key,
                            &title,
                            "movie",
                            year,
                            &image_cache_dir_bg,
                        ) {
                            let mut movie_meta = meta;
                            movie_meta.title = media_manager::prefer_title_with_leading_article(
                                &title,
                                &movie_meta.title,
                            );
                            db.update_metadata(media_id, &movie_meta).ok();
                        }
                    }
                }
            })
            .await;

            // Emit library-updated again after metadata fetch
            if let Some(window) = app_handle_clone.get_webview_window("main") {
                window.emit("library-updated", ()).ok();
            }
        });
    }

    if indexed_count > 0 {
        let db_path_merge = database::get_database_path();
        if let Ok(db) = database::Database::new(&db_path_merge) {
            let merged = auto_merge_duplicate_tvshows(&db, "background_check_cloud_changes");
            if merged > 0 {
                if let Some(window) = app_handle.get_webview_window("main") {
                    window.emit("library-updated", ()).ok();
                }
            }
        }
    }

    let total_duration = start_time.elapsed();
    println!(
        "[CLOUD BG] Poll complete: {} indexed, {} skipped ({:?})",
        indexed_count, skipped_count, total_duration
    );

    let final_skipped_reasons = if skipped_reasons.is_empty() {
        None
    } else {
        Some(skipped_reasons)
    };

    // Save token AFTER indexing completes so failed files are retried on next poll
    if let Ok(db) = state.db.lock() {
        let _ = db.set_gdrive_changes_token(&new_token);
    }

    Ok(CloudIndexResult {
        success: indexed_count > 0 || skipped_count == 0,
        indexed_count,
        skipped_count,
        movies_count,
        tv_count,
        message: format!("Indexed {} new files", indexed_count),
        skipped_reasons: final_skipped_reasons,
    })
}

// ============== AUTO-UPDATE COMMANDS ==============

// GitHub PAT for accessing private releases
const GITHUB_RELEASE_TOKEN: &str = ""; // User will provide their PAT
const ALLOWED_REPO: &str = "SlasshyOverhere/SlasshyVault";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_notes: String,
    pub download_url: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ZipProcessingEventPayload {
    phase: String,
    archive_count: usize,
    archive_name: Option<String>,
    episodes_indexed: Option<usize>,
    message: String,
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

const RELEASES_PAGE_URL: &str = "https://github.com/SlasshyOverhere/SlasshyVault/releases/latest";
const RELEASES_METADATA_URL: &str =
    "https://github.com/SlasshyOverhere/SlasshyVault/releases/latest/download/latest.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TauriLatestManifest {
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    pub_date: Option<String>,
    platforms: std::collections::HashMap<String, TauriLatestPlatform>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct TauriLatestPlatform {
    url: String,
    #[allow(dead_code)]
    signature: String,
}

fn open_latest_release_page_for_manual_install(context: &str) {
    println!(
        "[UPDATE] Opening latest releases page for manual install after failure in {}: {}",
        context, RELEASES_PAGE_URL
    );
    if let Err(e) = open::that_detached(RELEASES_PAGE_URL) {
        println!(
            "[UPDATE] Failed to open releases page for manual install fallback: {}",
            e
        );
    }
}

fn manual_update_error(context: &str, details: impl std::fmt::Display) -> String {
    let details_str = details.to_string();
    // ponytail: removed sentry
    open_latest_release_page_for_manual_install(context);
    format!(
        "Auto updater issue, please install manually. Opening latest release page. {}",
        details_str
    )
}

/// Check for updates from GitHub releases
#[tauri::command]
async fn check_for_updates() -> Result<UpdateInfo, String> {
    let current_version = env!("CARGO_PKG_VERSION");

    println!(
        "[UPDATE] Checking for updates... Current version: {}",
        current_version
    );
    println!("[UPDATE] Metadata URL: {}", RELEASES_METADATA_URL);

    let parsed_url = url::Url::parse(RELEASES_METADATA_URL).map_err(|e| {
        manual_update_error(
            "check_for_updates",
            format!("Invalid release metadata URL: {}.", e),
        )
    })?;

    if !is_authorized_update_url(&parsed_url, false) {
        return Err(manual_update_error(
            "check_for_updates",
            format!(
                "Unauthorized release metadata URL. Must be from GitHub repository: {}.",
                ALLOWED_REPO
            ),
        ));
    }

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| {
            manual_update_error(
                "check_for_updates",
                format!("Failed to build update metadata client: {}.", e),
            )
        })?;

    println!("[UPDATE] Fetching release metadata...");
    let response = client
        .get(RELEASES_METADATA_URL)
        .header("User-Agent", "SlasshyVault-Updater")
        .send()
        .await
        .map_err(|e| {
            println!("[UPDATE] ERROR: Network error during update check: {}", e);
            manual_update_error(
                "check_for_updates",
                format!("Failed to check for updates: {}.", e),
            )
        })?;

    let status = response.status();
    println!("[UPDATE] Response status: {}", status);

    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();

        // Handle specific error cases
        match status.as_u16() {
            403 => {
                println!("[UPDATE] ERROR: Release metadata access denied or forbidden");
                println!("[UPDATE] Response body: {}", error_text);

                return Err(manual_update_error(
                    "check_for_updates",
                    format!(
                        "Release metadata access denied (403). Details: {}",
                        error_text
                    ),
                ));
            }
            404 => {
                println!("[UPDATE] ERROR: Release metadata not found (404)");
                println!("[UPDATE] Response body: {}", error_text);
                return Err(manual_update_error(
                    "check_for_updates",
                    format!(
                        "Latest release metadata not found at {}.",
                        RELEASES_METADATA_URL
                    ),
                ));
            }
            _ => {
                println!(
                    "[UPDATE] ERROR: Release metadata error {}: {}",
                    status, error_text
                );
                return Err(manual_update_error(
                    "check_for_updates",
                    format!("Release metadata error ({}): {}", status, error_text),
                ));
            }
        }
    }

    println!("[UPDATE] Parsing latest.json...");
    let metadata_bytes = response.bytes().await.map_err(|e| {
        println!("[UPDATE] ERROR: Failed to read latest.json body: {}", e);
        manual_update_error(
            "check_for_updates",
            format!("Failed to read release metadata: {}.", e),
        )
    })?;
    let metadata_text = String::from_utf8_lossy(&metadata_bytes);
    let metadata_text = metadata_text.trim_start_matches('\u{feff}');
    let manifest: TauriLatestManifest = serde_json::from_str(metadata_text).map_err(|e| {
        println!("[UPDATE] ERROR: Failed to parse latest.json: {}", e);
        manual_update_error(
            "check_for_updates",
            format!("Failed to parse release metadata: {}.", e),
        )
    })?;

    let latest_version = manifest.version.trim_start_matches('v').to_string();
    println!("[UPDATE] Latest version from metadata: {}", latest_version);

    let is_newer = version_compare(&latest_version, current_version);
    println!("[UPDATE] Version comparison result: newer={}", is_newer);

    // When versions match, check if this is a same-version re-release (hotfix)
    // by comparing the remote pub_date against the last installed pub_date.
    let is_newer = if !is_newer && latest_version == current_version {
        if let (Some(remote_date), Some(local_date)) =
            (&manifest.pub_date, read_installed_pub_date())
        {
            let is_rerelease = remote_date > &local_date;
            if is_rerelease {
                println!("[UPDATE] Same version but newer pub_date detected (re-release): remote={} local={}", remote_date, local_date);
            }
            is_rerelease
        } else {
            false
        }
    } else {
        is_newer
    };

    println!(
        "[UPDATE] Available platforms in metadata: {:?}",
        manifest.platforms.keys().collect::<Vec<_>>()
    );

    let download_url = manifest
        .platforms
        .get("windows-x86_64-nsis")
        .or_else(|| manifest.platforms.get("windows-x86_64"))
        .map(|platform| platform.url.clone());

    if download_url.is_none() {
        println!("[UPDATE] WARNING: No WebviewWindows updater package found in latest.json");
        for (platform, value) in &manifest.platforms {
            println!("[UPDATE]   - {} => {}", platform, value.url);
        }
    }

    println!(
        "[UPDATE] Update check complete: available={}, latest={}",
        is_newer, latest_version
    );

    Ok(UpdateInfo {
        available: is_newer,
        current_version: current_version.to_string(),
        latest_version,
        release_notes: manifest.notes,
        download_url,
        published_at: manifest.pub_date,
    })
}

/// Simple version comparison (assumes semver-like versions)
fn version_compare(latest: &str, current: &str) -> bool {
    let parse_version =
        |v: &str| -> Vec<u32> { v.split('.').filter_map(|s| s.parse().ok()).collect() };

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
    window: WebviewWindow,
    url: String,
    pub_date: Option<String>,
) -> Result<String, String> {
    use std::io::Write;

    println!("[UPDATE] Starting download...");
    println!("[UPDATE] Download URL: {}", url);

    let parsed_url = url::Url::parse(&url).map_err(|e| {
        println!("[UPDATE] ERROR: Invalid URL format: {}", e);
        manual_update_error("download_update", format!("Invalid download URL: {}.", e))
    })?;

    println!("[UPDATE] Validating URL authorization...");
    if !is_authorized_update_url(&parsed_url, false) {
        println!("[UPDATE] ERROR: URL not authorized: {}", url);
        println!("[UPDATE] Allowed repo: {}", ALLOWED_REPO);
        return Err(manual_update_error(
            "download_update",
            format!(
                "Unauthorized update URL. Must be from GitHub repository: {}.",
                ALLOWED_REPO
            ),
        ));
    }
    println!("[UPDATE] URL authorization passed");

    // Use a custom redirect policy to ensure redirects don't lead to malicious sites
    println!("[UPDATE] Setting up redirect policy...");
    let custom_policy = reqwest::redirect::Policy::custom(move |attempt| {
        if !is_authorized_update_url(attempt.url(), true) {
            println!("[UPDATE] Blocking unauthorized redirect: {}", attempt.url());
            return attempt.error("Unauthorized redirect URL");
        }
        if attempt.previous().len() > 5 {
            println!("[UPDATE] Too many redirects: {:?}", attempt.previous());
            return attempt.error("Too many redirects");
        }
        println!("[UPDATE] Following redirect to: {}", attempt.url());
        attempt.follow()
    });

    let client = reqwest::Client::builder()
        .redirect(custom_policy)
        .build()
        .map_err(|e| {
            println!("[UPDATE] ERROR: Failed to build HTTP client: {}", e);
            manual_update_error(
                "download_update",
                format!("Failed to build download client: {}.", e),
            )
        })?;

    let mut request = client.get(&url);

    // Add auth header if PAT is configured
    if !GITHUB_RELEASE_TOKEN.is_empty() {
        println!("[UPDATE] Adding authentication to download request");
        request = request.header("Authorization", format!("Bearer {}", GITHUB_RELEASE_TOKEN));
    }

    println!("[UPDATE] Sending download request...");
    let response = request.send().await.map_err(|e| {
        println!("[UPDATE] ERROR: Failed to start download: {}", e);
        manual_update_error(
            "download_update",
            format!("Failed to start download: {}.", e),
        )
    })?;

    let status = response.status();
    println!("[UPDATE] Download response status: {}", status);

    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        println!(
            "[UPDATE] ERROR: Download failed with status {}: {}",
            status, error_text
        );

        match status.as_u16() {
            403 => {
                if error_text.contains("rate limit") {
                    return Err(manual_update_error(
                        "download_update",
                        "GitHub API rate limit exceeded during download. Please configure a GitHub PAT or wait and try again.",
                    ));
                }
                return Err(manual_update_error(
                    "download_update",
                    format!(
                        "Download forbidden (403). The release may require authentication. Details: {}",
                        error_text
                    ),
                ));
            }
            404 => {
                return Err(manual_update_error(
                    "download_update",
                    "Update file not found (404). The release may have been removed or the URL is incorrect.",
                ));
            }
            _ => {
                return Err(manual_update_error(
                    "download_update",
                    format!("Download failed: HTTP {} - {}", status, error_text),
                ));
            }
        }
    }

    let total_size = response.content_length().unwrap_or(0);
    println!(
        "[UPDATE] File size: {} bytes ({:.2} MB)",
        total_size,
        total_size as f64 / 1024.0 / 1024.0
    );

    let filename = sanitize_update_filename(&parsed_url);
    println!("[UPDATE] Filename: {}", filename);

    let staging_dir = updater_staging_root().join(Uuid::new_v4().to_string());
    println!("[UPDATE] Staging directory: {:?}", staging_dir);

    std::fs::create_dir_all(&staging_dir).map_err(|e| {
        println!("[UPDATE] ERROR: Failed to create staging directory: {}", e);
        manual_update_error(
            "download_update",
            format!("Failed to create updater staging directory: {}.", e),
        )
    })?;

    let file_path = staging_dir.join(filename);

    let mut file = std::fs::File::create(&file_path).map_err(|e| {
        println!("[UPDATE] ERROR: Failed to create temp file: {}", e);
        manual_update_error(
            "download_update",
            format!("Failed to create updater file: {}.", e),
        )
    })?;

    println!("[UPDATE] Writing file to disk...");
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| {
            println!("[UPDATE] ERROR: Download chunk error: {}", e);
            manual_update_error("download_update", format!("Download error: {}.", e))
        })?;

        file.write_all(&chunk).map_err(|e| {
            println!("[UPDATE] ERROR: Write error: {}", e);
            manual_update_error("download_update", format!("Write error: {}.", e))
        })?;

        downloaded += chunk.len() as u64;

        // Emit progress event
        if total_size > 0 {
            let progress = (downloaded as f64 / total_size as f64) * 100.0;
            window
                .emit(
                    "update-download-progress",
                    serde_json::json!({
                        "downloaded": downloaded,
                        "total": total_size,
                        "progress": progress
                    }),
                )
                .ok();

            // Log progress every 10%
            if (progress as u32) % 10 == 0 && (progress as u32) > 0 && downloaded < total_size {
                println!(
                    "[UPDATE] Progress: {:.1}% ({}/{})",
                    progress, downloaded, total_size
                );
            }
        }
    }

    println!("[UPDATE] Download complete!");
    println!("[UPDATE] Saved to: {:?}", file_path);
    println!("[UPDATE] Final size: {} bytes", downloaded);

    // Persist the pub_date so we can detect same-version re-releases next check
    if let Some(pd) = pub_date {
        save_installed_pub_date(&pd);
    }

    Ok(file_path.to_string_lossy().to_string())
}

/// Save the pub_date of a successfully downloaded update so we can detect
/// same-version re-releases (hotfixes) on the next check.
fn save_installed_pub_date(pub_date: &str) {
    let path = database::get_app_data_dir().join("last_update_pub_date.txt");
    if let Err(e) = std::fs::write(&path, pub_date) {
        println!("[UPDATE] WARNING: Failed to save installed pub_date: {}", e);
    } else {
        println!("[UPDATE] Saved installed pub_date: {}", pub_date);
    }
}

/// Read the pub_date of the last installed update, if any.
fn read_installed_pub_date() -> Option<String> {
    let path = database::get_app_data_dir().join("last_update_pub_date.txt");
    match std::fs::read_to_string(&path) {
        Ok(s) => {
            let trimmed = s.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        Err(_) => None,
    }
}

fn updater_staging_root() -> std::path::PathBuf {
    let dir_name = if is_dev_runtime() {
        "slasshyvault-updater-dev"
    } else {
        "slasshyvault-updater"
    };
    std::env::temp_dir().join(dir_name)
}

fn is_authorized_update_url(url: &url::Url, is_redirect: bool) -> bool {
    if url.scheme() != "https" {
        println!("[UPDATE-SECURITY] Rejected non-HTTPS URL: {}", url);
        return false;
    }

    if is_redirect {
        println!(
            "[UPDATE-SECURITY] Allowing HTTPS redirect during update download: {}",
            url
        );
        return true;
    }

    let host_matches = matches!(url.host_str(), Some("github.com") | Some("www.github.com"));
    let path_matches = url.path().contains(ALLOWED_REPO);

    if host_matches && path_matches {
        println!(
            "[UPDATE-SECURITY] Allowing update URL from approved GitHub repo: {}",
            url
        );
        true
    } else {
        println!(
            "[UPDATE-SECURITY] Rejected update URL because it does not match github.com + {}: {}",
            ALLOWED_REPO, url
        );
        false
    }
}

fn sanitize_update_filename(parsed_url: &url::Url) -> String {
    let fallback = "slasshyvault-update.bin";
    let Some(raw_filename) = parsed_url
        .path_segments()
        .and_then(|segments| segments.last())
        .filter(|segment| !segment.is_empty())
    else {
        return fallback.to_string();
    };

    let sanitized = raw_filename
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    let trimmed = sanitized.trim_matches('.');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn get_valid_installer_path(path_str: &str) -> Result<std::path::PathBuf, String> {
    let path = std::path::Path::new(path_str);

    // ponytail: dunce removed — \\?\ prefix harmless for Path::starts_with
    let canonical_path = path
        .canonicalize()
        .map_err(|e| format!("Invalid installer path: {}", e))?;

    let staging_dir = updater_staging_root();
    let canonical_staging = staging_dir
        .canonicalize()
        .map_err(|e| format!("Failed to resolve updater staging directory: {}", e))?;

    // Ensure it's inside the app-owned updater staging directory
    if !canonical_path.starts_with(&canonical_staging) {
        return Err("Installer must be located in the updater staging directory".to_string());
    }

    // Check extensions
    let ext = canonical_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    #[cfg(target_os = "windows")]
    if ext != "exe" && ext != "msi" && ext != "nsis" && ext != "zip" {
        return Err("Only .exe, .msi, .nsis, or .zip installers are allowed".to_string());
    }

    #[cfg(target_os = "macos")]
    if ext != "dmg" && ext != "pkg" && ext != "app" {
        return Err("Only .dmg, .pkg, or .app installers are allowed".to_string());
    }

    #[cfg(target_os = "linux")]
    if ext != "deb" && ext != "rpm" && ext != "appimage" {
        return Err("Only .deb, .rpm, or .AppImage installers are allowed".to_string());
    }

    let metadata = std::fs::metadata(&canonical_path)
        .map_err(|e| format!("Failed to inspect installer path: {}", e))?;

    #[cfg(target_os = "macos")]
    if ext == "app" {
        if !metadata.is_dir() {
            return Err("A .app installer must be a directory bundle".to_string());
        }
    } else if !metadata.is_file() {
        return Err("Installer path must point to a file".to_string());
    }

    #[cfg(not(target_os = "macos"))]
    if !metadata.is_file() {
        return Err("Installer path must point to a file".to_string());
    }

    Ok(canonical_path)
}

#[cfg(target_os = "windows")]
fn resolve_windows_installer_from_package(
    package_path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let ext = package_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext == "exe" || ext == "msi" || ext == "nsis" {
        return Ok(package_path.to_path_buf());
    }

    if ext != "zip" {
        return Err(format!(
            "Unsupported WebviewWindows updater package: {}",
            package_path.display()
        ));
    }

    // Log ZIP file diagnostics before extraction
    let zip_size = std::fs::metadata(package_path)
        .map(|m| m.len())
        .unwrap_or(0);
    println!(
        "[UPDATE] Extracting updater ZIP: path={}, size={} bytes",
        package_path.display(),
        zip_size
    );

    let extract_dir = package_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("extracted");
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create extracted installer directory: {}", e))?;

    let mut extract_cmd = std::process::Command::new("powershell");
    config::apply_hidden_process_flags(&mut extract_cmd);

    // Build the zip path and dest as strings — \\?\ prefix confuses Expand-Archive
    let zip_str = package_path.to_string_lossy().replace(r"\\?\", "");
    let dest_str = extract_dir.to_string_lossy().replace(r"\\?\", "");

    let output = extract_cmd
        .env("ZIP_PATH", &zip_str)
        .env("DEST_PATH", &dest_str)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Expand-Archive -LiteralPath $env:ZIP_PATH -DestinationPath $env:DEST_PATH -Force",
        ])
        .output()
        .map_err(|e| format!("Failed to extract updater ZIP: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "No extraction error output was captured.".to_string()
        };
        return Err(format!(
            "Failed to extract updater ZIP. PowerShell exit code: {:?}. {}",
            output.status.code(),
            details
        ));
    }

    // Walk extracted directory: log every entry with size/type, and collect installer candidates
    let mut all_extracted_entries = Vec::new();
    let mut installer_candidates = Vec::new();

    for entry in walkdir::WalkDir::new(&extract_dir) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                println!("[UPDATE] WARNING: Error walking extracted directory: {}", e);
                continue;
            }
        };
        let file_size = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
        let is_file = entry.file_type().is_file();
        let is_dir = entry.file_type().is_dir();
        let path = entry.into_path();

        let type_tag = if is_dir {
            "DIR"
        } else if is_file {
            "FILE"
        } else {
            "OTHER"
        };
        println!(
            "[UPDATE] Extracted content: [{}] {} ({} bytes)",
            type_tag,
            path.display(),
            file_size
        );

        all_extracted_entries.push((path.clone(), file_size, type_tag.to_string()));

        if is_file {
            if let Some(found_ext) = path.extension().and_then(|e| e.to_str()) {
                let found_ext = found_ext.to_ascii_lowercase();
                if found_ext == "exe" || found_ext == "msi" || found_ext == "nsis" {
                    installer_candidates.push(path);
                }
            }
        }
    }

    println!(
        "[UPDATE] Extracted {} entries from updater ZIP, found {} installer candidates",
        all_extracted_entries.len(),
        installer_candidates.len()
    );

    installer_candidates.sort();
    installer_candidates.into_iter().next().ok_or_else(|| {
        let mut details =
            String::from("No .exe, .msi, or .nsis installer found inside updater ZIP. Contents:\n");
        for (path, size, type_tag) in &all_extracted_entries {
            details.push_str(&format!(
                "  [{}] {} ({} bytes)\n",
                type_tag,
                path.display(),
                size
            ));
        }
        details
    })
}

/// Install update and restart app
#[tauri::command]
async fn install_update(installer_path: String) -> Result<(), String> {
    println!("[UPDATE] Validating installer path before launch");

    let safe_path = get_valid_installer_path(&installer_path)
        .map_err(|e| manual_update_error("install_update", e))?;
    #[cfg(target_os = "windows")]
    let safe_path = resolve_windows_installer_from_package(&safe_path)
        .map_err(|e| manual_update_error("install_update", e))?;
    #[cfg(target_os = "linux")]
    if safe_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("appimage"))
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&safe_path)
            .map_err(|e| {
                manual_update_error(
                    "install_update",
                    format!("Failed to inspect AppImage: {}", e),
                )
            })?
            .permissions();
        permissions.set_mode(permissions.mode() | 0o111);
        std::fs::set_permissions(&safe_path, permissions).map_err(|e| {
            manual_update_error(
                "install_update",
                format!("Failed to make AppImage executable: {}", e),
            )
        })?;
    }

    #[cfg(target_os = "linux")]
    if safe_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "deb" | "rpm"))
    {
        return Err(manual_update_error(
            "install_update",
            format!(
                "Downloaded {} package. Install it with your system package manager, then restart SlasshyVault.",
                safe_path.extension().and_then(|extension| extension.to_str()).unwrap_or("Linux")
            ),
        ));
    }

    println!(
        "[UPDATE] Installing update from safe path: {}",
        safe_path.display()
    );

    // Launch the installer securely without tying the child process to this app.
    if let Err(e) = open::that_detached(&safe_path) {
        return Err(manual_update_error(
            "install_update",
            format!("Failed to launch installer: {}.", e),
        ));
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

#[cfg(test)]
mod install_update_tests {
    use super::{get_valid_installer_path, updater_staging_root};
    use std::fs;
    use std::path::{Path, PathBuf};
    use uuid::Uuid;

    #[cfg(target_os = "windows")]
    const TEST_INSTALLER_NAME: &str = "slasshyvault-update.exe";
    #[cfg(target_os = "macos")]
    const TEST_INSTALLER_NAME: &str = "slasshyvault-update.pkg";
    #[cfg(target_os = "linux")]
    const TEST_INSTALLER_NAME: &str = "slasshyvault-update.deb";

    fn remove_test_artifact(path: &Path) {
        if path.is_dir() {
            let _ = fs::remove_dir_all(path);
        } else {
            let _ = fs::remove_file(path);
        }

        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir(parent);
        }
    }

    fn create_temp_installer(name: &str) -> PathBuf {
        let dir =
            updater_staging_root().join(format!("slasshyvault-installer-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);

        #[cfg(target_os = "macos")]
        if name.ends_with(".app") {
            fs::create_dir_all(&path).unwrap();
            return path;
        }

        fs::write(&path, b"test-installer").unwrap();
        path
    }

    #[test]
    fn accepts_allowed_installer_in_staging_dir() {
        let installer_path = create_temp_installer(TEST_INSTALLER_NAME);
        let validated = get_valid_installer_path(installer_path.to_str().unwrap()).unwrap();
        // canonical_path has \\?\ prefix on WebviewWindows; compare against canonicalized staging root
        let canonical_staging = updater_staging_root().canonicalize().unwrap();
        assert!(validated.starts_with(&canonical_staging));
        remove_test_artifact(&installer_path);
    }

    #[test]
    fn rejects_disallowed_extension_in_temp_dir() {
        let installer_path = create_temp_installer("slasshyvault-update.txt");
        let error = get_valid_installer_path(installer_path.to_str().unwrap()).unwrap_err();
        assert!(error.contains("allowed"));
        remove_test_artifact(&installer_path);
    }

    #[test]
    fn rejects_installer_outside_staging_dir() {
        let installer_path = std::env::current_dir()
            .unwrap()
            .join(format!("slasshyvault-outside-temp-{}", TEST_INSTALLER_NAME));
        fs::write(&installer_path, b"test-installer").unwrap();

        let error = get_valid_installer_path(installer_path.to_str().unwrap()).unwrap_err();
        assert!(error.contains("updater staging directory"));
        remove_test_artifact(&installer_path);
    }
}

// ==================== ADDITIONAL TESTS FOR PURE HELPERS ====================

#[cfg(test)]
mod version_compare_tests {
    use super::version_compare;

    #[test]
    fn higher_major_is_newer() {
        assert!(version_compare("2.0.0", "1.9.9"));
    }

    #[test]
    fn higher_minor_is_newer() {
        assert!(version_compare("1.5.0", "1.4.9"));
    }

    #[test]
    fn higher_patch_is_newer() {
        assert!(version_compare("1.0.1", "1.0.0"));
    }

    #[test]
    fn equal_versions_not_newer() {
        assert!(!version_compare("1.0.0", "1.0.0"));
    }

    #[test]
    fn older_version_not_newer() {
        assert!(!version_compare("1.0.0", "2.0.0"));
    }

    #[test]
    fn handles_missing_patch() {
        assert!(version_compare("1.1", "1.0"));
    }

    #[test]
    fn handles_single_component() {
        assert!(version_compare("3", "2"));
    }

    #[test]
    fn handles_zero_version() {
        assert!(!version_compare("0.0.0", "0.0.0"));
    }

    #[test]
    fn handles_two_digit_components() {
        assert!(version_compare("1.12.3", "1.11.9"));
    }

    #[test]
    fn lower_patch_not_newer() {
        assert!(!version_compare("1.0.0", "1.0.1"));
    }
}

#[cfg(test)]
mod authorized_update_url_tests {
    use super::is_authorized_update_url;
    use super::ALLOWED_REPO;

    fn make_url(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    #[test]
    fn rejects_http_scheme() {
        let url = make_url(
            "http://github.com/SlasshyOverhere/SlasshyVault/releases/download/v1/test.exe",
        );
        assert!(!is_authorized_update_url(&url, false));
    }

    #[test]
    fn rejects_ftp_scheme() {
        let url =
            make_url("ftp://github.com/SlasshyOverhere/SlasshyVault/releases/download/v1/test.exe");
        assert!(!is_authorized_update_url(&url, false));
    }

    #[test]
    fn allows_github_https_with_correct_repo() {
        let url = make_url(&format!(
            "https://github.com/{}/releases/download/v1/test.exe",
            ALLOWED_REPO
        ));
        assert!(is_authorized_update_url(&url, false));
    }

    #[test]
    fn rejects_wrong_github_repo() {
        let url =
            make_url("https://github.com/SomeOtherOrg/SomeOtherRepo/releases/download/v1/test.exe");
        assert!(!is_authorized_update_url(&url, false));
    }

    #[test]
    fn rejects_non_github_host() {
        let url =
            make_url("https://evil.com/SlasshyOverhere/SlasshyVault/releases/download/v1/test.exe");
        assert!(!is_authorized_update_url(&url, false));
    }

    #[test]
    fn allows_any_https_redirect() {
        let url = make_url("https://objects.githubusercontent.com/some-redirect");
        assert!(is_authorized_update_url(&url, true));
    }

    #[test]
    fn rejects_http_redirect() {
        let url = make_url("http://objects.githubusercontent.com/some-redirect");
        assert!(!is_authorized_update_url(&url, true));
    }

    #[test]
    fn allows_www_github_host() {
        let url = make_url(&format!(
            "https://www.github.com/{}/releases/download/v1/test.exe",
            ALLOWED_REPO
        ));
        assert!(is_authorized_update_url(&url, false));
    }

    #[test]
    fn rejects_github_wrong_path() {
        let url = make_url("https://github.com/WrongOrg/WrongRepo/releases/download/v1/test.exe");
        assert!(!is_authorized_update_url(&url, false));
    }
}

#[cfg(test)]
mod sanitize_filename_tests {
    use super::sanitize_update_filename;

    fn make_url(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    #[test]
    fn extracts_filename_from_url() {
        let url = make_url(
            "https://github.com/org/repo/releases/download/v1/SlasshyVault-Setup-3.0.57.exe",
        );
        assert_eq!(
            sanitize_update_filename(&url),
            "SlasshyVault-Setup-3.0.57.exe"
        );
    }

    #[test]
    fn replaces_special_chars() {
        // URL percent-encodes spaces; % and @ and ! are all non-alphanumeric -> _
        let url = make_url("https://example.com/path/file%20name%40special!.exe");
        let result = sanitize_update_filename(&url);
        assert!(result.contains("special_"));
        assert!(!result.contains("@"));
        assert!(!result.contains("!"));
    }

    #[test]
    fn returns_fallback_for_empty_path() {
        let url = make_url("https://example.com/");
        assert_eq!(sanitize_update_filename(&url), "slasshyvault-update.bin");
    }

    #[test]
    fn trims_leading_trailing_dots() {
        let url = make_url("https://example.com/...hidden...");
        assert_eq!(sanitize_update_filename(&url), "hidden");
    }

    #[test]
    fn returns_fallback_for_only_dots() {
        let url = make_url("https://example.com/...");
        assert_eq!(sanitize_update_filename(&url), "slasshyvault-update.bin");
    }

    #[test]
    fn preserves_dashes_and_underscores() {
        let url = make_url("https://example.com/my_file-name.exe");
        assert_eq!(sanitize_update_filename(&url), "my_file-name.exe");
    }
}

#[cfg(test)]
mod ffprobe_parsing_tests {
    use super::parse_ffprobe_frame_rate;

    #[test]
    fn parses_fraction() {
        let fps = parse_ffprobe_frame_rate(Some("30000/1001"));
        assert!(fps.is_some());
        let fps = fps.unwrap();
        assert!((fps - 29.97).abs() < 0.01);
    }

    #[test]
    fn parses_integer_string() {
        let fps = parse_ffprobe_frame_rate(Some("25"));
        assert_eq!(fps, Some(25.0));
    }

    #[test]
    fn returns_none_for_none() {
        assert_eq!(parse_ffprobe_frame_rate(None), None);
    }

    #[test]
    fn returns_none_for_empty() {
        assert_eq!(parse_ffprobe_frame_rate(Some("")), None);
    }

    #[test]
    fn returns_none_for_zero_over_zero() {
        assert_eq!(parse_ffprobe_frame_rate(Some("0/0")), None);
    }

    #[test]
    fn returns_none_for_zero_denominator() {
        assert_eq!(parse_ffprobe_frame_rate(Some("30/0")), None);
    }

    #[test]
    fn handles_whitespace() {
        let fps = parse_ffprobe_frame_rate(Some("  24000/1001  "));
        assert!(fps.is_some());
        assert!((fps.unwrap() - 23.976).abs() < 0.01);
    }

    #[test]
    fn returns_none_for_garbage() {
        assert_eq!(parse_ffprobe_frame_rate(Some("abc")), None);
    }

    #[test]
    fn parses_60fps() {
        let fps = parse_ffprobe_frame_rate(Some("60000/1000"));
        assert_eq!(fps, Some(60.0));
    }
}

#[cfg(test)]
mod container_name_tests {
    use super::normalize_container_name;

    #[test]
    fn normalizes_matroska() {
        assert_eq!(
            normalize_container_name(Some("matroska"), None),
            Some("MKV".to_string())
        );
    }

    #[test]
    fn normalizes_mov_mp4_variant() {
        assert_eq!(
            normalize_container_name(Some("mov,mp4,m4a,3gp,3g2,mj2"), None),
            Some("MP4".to_string())
        );
    }

    #[test]
    fn normalizes_avi() {
        assert_eq!(
            normalize_container_name(Some("avi"), None),
            Some("AVI".to_string())
        );
    }

    #[test]
    fn normalizes_webm() {
        assert_eq!(
            normalize_container_name(Some("webm"), None),
            Some("WEBM".to_string())
        );
    }

    #[test]
    fn normalizes_mpegts() {
        assert_eq!(
            normalize_container_name(Some("mpegts"), None),
            Some("TS".to_string())
        );
    }

    #[test]
    fn falls_back_to_extension() {
        assert_eq!(
            normalize_container_name(None, Some("mkv")),
            Some("MKV".to_string())
        );
    }

    #[test]
    fn returns_none_for_both_none() {
        assert_eq!(normalize_container_name(None, None), None);
    }

    #[test]
    fn handles_case_insensitive() {
        assert_eq!(
            normalize_container_name(Some("MATROSKA"), None),
            Some("MKV".to_string())
        );
    }

    #[test]
    fn unknown_format_uppercased() {
        assert_eq!(
            normalize_container_name(Some("flac"), None),
            Some("FLAC".to_string())
        );
    }
}

#[cfg(test)]
mod resolution_label_tests {
    use super::resolution_label_from_dimensions;

    #[test]
    fn detects_4k() {
        assert_eq!(
            resolution_label_from_dimensions(Some(3840), Some(2160)),
            Some("2160p".to_string())
        );
    }

    #[test]
    fn detects_1440p() {
        assert_eq!(
            resolution_label_from_dimensions(Some(2560), Some(1440)),
            Some("1440p".to_string())
        );
    }

    #[test]
    fn detects_1080p() {
        assert_eq!(
            resolution_label_from_dimensions(Some(1920), Some(1080)),
            Some("1080p".to_string())
        );
    }

    #[test]
    fn detects_720p() {
        assert_eq!(
            resolution_label_from_dimensions(Some(1280), Some(720)),
            Some("720p".to_string())
        );
    }

    #[test]
    fn uses_raw_height_for_small() {
        assert_eq!(
            resolution_label_from_dimensions(Some(640), Some(360)),
            Some("360p".to_string())
        );
    }

    #[test]
    fn returns_none_for_zero_height() {
        // width must also be small so it doesn't match a resolution bucket
        assert_eq!(resolution_label_from_dimensions(Some(0), Some(0)), None);
    }

    #[test]
    fn returns_none_for_no_height() {
        assert_eq!(resolution_label_from_dimensions(Some(1920), None), None);
    }

    #[test]
    fn handles_width_only_default() {
        assert_eq!(
            resolution_label_from_dimensions(None, Some(1080)),
            Some("1080p".to_string())
        );
    }

    #[test]
    fn detects_4k_by_height_only() {
        assert_eq!(
            resolution_label_from_dimensions(None, Some(2160)),
            Some("2160p".to_string())
        );
    }
}

#[cfg(test)]
mod language_inference_tests {
    use super::infer_language_from_text;

    #[test]
    fn recognizes_en() {
        assert_eq!(
            infer_language_from_text("en"),
            Some(("en", "English", "en,eng,english"))
        );
    }

    #[test]
    fn recognizes_eng() {
        assert_eq!(
            infer_language_from_text("eng"),
            Some(("en", "English", "en,eng,english"))
        );
    }

    #[test]
    fn recognizes_english() {
        assert_eq!(
            infer_language_from_text("English"),
            Some(("en", "English", "en,eng,english"))
        );
    }

    #[test]
    fn recognizes_hindi() {
        assert_eq!(
            infer_language_from_text("hi"),
            Some(("hi", "Hindi", "hi,hin,hindi"))
        );
    }

    #[test]
    fn recognizes_japanese() {
        assert_eq!(
            infer_language_from_text("ja"),
            Some(("ja", "Japanese", "ja,jpn,japanese"))
        );
    }

    #[test]
    fn recognizes_unknown_with_english_substring() {
        assert_eq!(
            infer_language_from_text("English 5.1"),
            Some(("en", "English", "en,eng,english"))
        );
    }

    #[test]
    fn returns_none_for_empty() {
        assert_eq!(infer_language_from_text(""), None);
    }

    #[test]
    fn returns_none_for_und() {
        assert_eq!(infer_language_from_text("und"), None);
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(
            infer_language_from_text("  en  "),
            Some(("en", "English", "en,eng,english"))
        );
    }

    #[test]
    fn recognizes_german_deu() {
        assert_eq!(
            infer_language_from_text("deu"),
            Some(("de", "German", "de,deu,german"))
        );
    }

    #[test]
    fn returns_none_for_garbage() {
        assert_eq!(infer_language_from_text("xyz123"), None);
    }

    #[test]
    fn recognizes_spanish() {
        assert_eq!(
            infer_language_from_text("es"),
            Some(("es", "Spanish", "es,spa,spanish"))
        );
    }
}

#[cfg(test)]
mod validate_addon_url_tests {
    use super::validate_addon_url;

    #[test]
    fn accepts_valid_https_url() {
        assert!(validate_addon_url("https://example.com/addon").is_ok());
    }

    #[test]
    fn accepts_valid_http_url() {
        assert!(validate_addon_url("http://localhost:3255/addon").is_ok());
    }

    #[test]
    fn rejects_empty_url() {
        assert!(validate_addon_url("").is_err());
    }

    #[test]
    fn rejects_ftp_scheme() {
        assert!(validate_addon_url("ftp://example.com/addon").is_err());
    }

    #[test]
    fn rejects_file_scheme() {
        assert!(validate_addon_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn rejects_zero_ip() {
        assert!(validate_addon_url("http://0.0.0.0:8080/addon").is_err());
    }

    #[test]
    fn accepts_localhost() {
        assert!(validate_addon_url("http://localhost:8080/addon").is_ok());
    }

    #[test]
    fn rejects_localhost_localdomain() {
        assert!(validate_addon_url("http://localhost.localdomain/addon").is_err());
    }

    #[test]
    fn rejects_no_scheme() {
        assert!(validate_addon_url("example.com/addon").is_err());
    }

    #[test]
    fn trims_whitespace() {
        assert!(validate_addon_url("  https://example.com  ").is_ok());
    }

    #[test]
    fn rejects_link_local() {
        assert!(validate_addon_url("http://169.254.1.1/addon").is_err());
    }
}

#[cfg(test)]
mod notification_format_tests {
    use super::{format_added_notification, format_removed_notification};

    #[test]
    fn added_movie_format() {
        let msg = format_added_notification("Inception", false, None, None);
        assert_eq!(msg, "Inception added to your library");
    }

    #[test]
    fn added_tv_episode_format() {
        let msg = format_added_notification("Breaking Bad", true, Some(1), Some(5));
        assert_eq!(msg, "Breaking Bad S01E05 added to your library");
    }

    #[test]
    fn added_tv_defaults_to_s01e01() {
        let msg = format_added_notification("Show", true, None, None);
        assert_eq!(msg, "Show S01E01 added to your library");
    }

    #[test]
    fn removed_single_item() {
        let msg = format_removed_notification("Movie", None);
        assert_eq!(msg, "Movie removed (deleted from Drive)");
    }

    #[test]
    fn removed_single_count() {
        let msg = format_removed_notification("Movie", Some(1));
        assert_eq!(msg, "Movie removed (deleted from Drive)");
    }

    #[test]
    fn removed_multiple() {
        let msg = format_removed_notification("Show", Some(5));
        assert_eq!(msg, "Show - 5 items removed (deleted from Drive)");
    }
}

#[cfg(test)]
mod reminder_validation_tests {
    use super::{
        normalize_tracking_mode, validate_reminder_input, validate_tracking_mode,
        MovieReminderInput,
    };

    fn valid_input() -> MovieReminderInput {
        MovieReminderInput {
            tmdb_id: "12345".to_string(),
            media_type: "movie".to_string(),
            title: "Test Movie".to_string(),
            poster_path: None,
            season_number: None,
            episode_number: None,
            release_date: None,
            reminder_at: "2025-01-01T00:00:00Z".to_string(),
            source: None,
            tracking_mode: None,
            tracking_season_number: None,
            notes: None,
            is_active: None,
        }
    }

    #[test]
    fn valid_movie_passes() {
        assert!(validate_reminder_input(&valid_input()).is_ok());
    }

    #[test]
    fn empty_tmdb_id_fails() {
        let mut input = valid_input();
        input.tmdb_id = "".to_string();
        assert!(validate_reminder_input(&input).is_err());
    }

    #[test]
    fn empty_title_fails() {
        let mut input = valid_input();
        input.title = "".to_string();
        assert!(validate_reminder_input(&input).is_err());
    }

    #[test]
    fn invalid_media_type_fails() {
        let mut input = valid_input();
        input.media_type = "book".to_string();
        assert!(validate_reminder_input(&input).is_err());
    }

    #[test]
    fn tv_media_type_passes() {
        let mut input = valid_input();
        input.media_type = "tv".to_string();
        assert!(validate_reminder_input(&input).is_ok());
    }

    #[test]
    fn movie_tracking_mode_must_be_single() {
        let mut input = valid_input();
        input.media_type = "movie".to_string();
        input.tracking_mode = Some("tv_season".to_string());
        assert!(validate_tracking_mode(&input).is_err());
    }

    #[test]
    fn tv_tracking_mode_tv_season_ok() {
        let mut input = valid_input();
        input.media_type = "tv".to_string();
        input.tracking_mode = Some("tv_season".to_string());
        assert!(validate_tracking_mode(&input).is_ok());
    }

    #[test]
    fn normalize_defaults_to_single() {
        let input = valid_input();
        assert_eq!(normalize_tracking_mode(&input).unwrap(), "single");
    }

    #[test]
    fn normalize_invalid_mode_fails() {
        let mut input = valid_input();
        input.tracking_mode = Some("continuous".to_string());
        assert!(normalize_tracking_mode(&input).is_err());
    }
}

#[cfg(test)]
mod watchlist_validation_tests {
    use super::{
        normalize_watchlist_notification_mode, validate_watchlist_input, WatchlistItemInput,
    };

    fn valid_input() -> WatchlistItemInput {
        WatchlistItemInput {
            tmdb_id: "12345".to_string(),
            media_type: "movie".to_string(),
            title: "Test Movie".to_string(),
            poster_path: None,
            release_date: None,
            notes: None,
            is_active: None,
            notification_enabled: None,
            notification_mode: None,
            notification_interval_minutes: None,
            notify_at: None,
        }
    }

    #[test]
    fn valid_input_passes() {
        assert!(validate_watchlist_input(&valid_input()).is_ok());
    }

    #[test]
    fn empty_tmdb_id_fails() {
        let mut input = valid_input();
        input.tmdb_id = "".to_string();
        assert!(validate_watchlist_input(&input).is_err());
    }

    #[test]
    fn empty_title_fails() {
        let mut input = valid_input();
        input.title = "".to_string();
        assert!(validate_watchlist_input(&input).is_err());
    }

    #[test]
    fn invalid_media_type_fails() {
        let mut input = valid_input();
        input.media_type = "book".to_string();
        assert!(validate_watchlist_input(&input).is_err());
    }

    #[test]
    fn notification_enabled_without_notify_at_fails() {
        let mut input = valid_input();
        input.notification_enabled = Some(true);
        input.notify_at = None;
        assert!(validate_watchlist_input(&input).is_err());
    }

    #[test]
    fn notification_enabled_with_notify_at_passes() {
        let mut input = valid_input();
        input.notification_enabled = Some(true);
        input.notify_at = Some("2025-01-01T00:00:00Z".to_string());
        assert!(validate_watchlist_input(&input).is_ok());
    }

    #[test]
    fn spam_mode_requires_positive_interval() {
        let mut input = valid_input();
        input.notification_mode = Some("spam".to_string());
        input.notification_interval_minutes = Some(0);
        assert!(validate_watchlist_input(&input).is_err());
    }

    #[test]
    fn spam_mode_with_interval_passes() {
        let mut input = valid_input();
        input.notification_mode = Some("spam".to_string());
        input.notification_interval_minutes = Some(60);
        assert!(validate_watchlist_input(&input).is_ok());
    }

    #[test]
    fn normalize_single_mode() {
        let input = valid_input();
        assert_eq!(
            normalize_watchlist_notification_mode(&input).unwrap(),
            "single"
        );
    }

    #[test]
    fn normalize_spam_mode() {
        let mut input = valid_input();
        input.notification_mode = Some("spam".to_string());
        assert_eq!(
            normalize_watchlist_notification_mode(&input).unwrap(),
            "spam"
        );
    }

    #[test]
    fn normalize_invalid_mode_fails() {
        let mut input = valid_input();
        input.notification_mode = Some("burst".to_string());
        assert!(normalize_watchlist_notification_mode(&input).is_err());
    }
}

#[cfg(test)]
mod episode_inference_tests {
    use super::infer_episode_numbers_from_path;

    #[test]
    fn parses_standard_s01e05() {
        let result = infer_episode_numbers_from_path("Breaking.Bad.S01E05.720p.mkv");
        assert_eq!(result, Some((1, 5)));
    }

    #[test]
    fn parses_s1e1() {
        let result = infer_episode_numbers_from_path("Show.S1E1.mkv");
        assert_eq!(result, Some((1, 1)));
    }

    #[test]
    fn parses_s10e100() {
        let result = infer_episode_numbers_from_path("Show.S10E100.mkv");
        assert_eq!(result, Some((10, 100)));
    }

    #[test]
    fn case_insensitive() {
        let result = infer_episode_numbers_from_path("show.s03e12.mkv");
        assert_eq!(result, Some((3, 12)));
    }

    #[test]
    fn returns_none_for_no_match() {
        let result = infer_episode_numbers_from_path("movie.mkv");
        assert_eq!(result, None);
    }

    #[test]
    fn returns_none_for_empty() {
        let result = infer_episode_numbers_from_path("");
        assert_eq!(result, None);
    }
}

#[cfg(test)]
mod compute_hash_tests {
    use super::compute_partial_hash;
    use std::io::Write;
    use uuid::Uuid;

    fn temp_file_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("hash_test_{}", Uuid::new_v4()))
    }

    #[test]
    fn computes_hash_for_content() {
        let path = temp_file_path();
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello world test data").unwrap();
        drop(f);

        let hash = compute_partial_hash(path.to_str().unwrap());
        assert!(hash.is_some());
        assert_eq!(hash.unwrap().len(), 8);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn returns_none_for_missing_file() {
        let hash = compute_partial_hash("/nonexistent/path/file.bin");
        assert!(hash.is_none());
    }

    #[test]
    fn returns_none_for_empty_file() {
        let path = temp_file_path();
        std::fs::File::create(&path).unwrap();

        let hash = compute_partial_hash(path.to_str().unwrap());
        assert!(hash.is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn same_content_same_hash() {
        let path1 = temp_file_path();
        let path2 = temp_file_path();
        let data = b"deterministic content for hash test";
        std::fs::File::create(&path1)
            .unwrap()
            .write_all(data)
            .unwrap();
        std::fs::File::create(&path2)
            .unwrap()
            .write_all(data)
            .unwrap();

        let h1 = compute_partial_hash(path1.to_str().unwrap());
        let h2 = compute_partial_hash(path2.to_str().unwrap());
        assert_eq!(h1, h2);

        let _ = std::fs::remove_file(&path1);
        let _ = std::fs::remove_file(&path2);
    }
}

#[cfg(test)]
mod augment_match_key_tests {
    use super::augment_match_key_with_phash;
    use std::io::Write;
    use uuid::Uuid;

    #[test]
    fn returns_none_when_no_key_and_no_path() {
        assert_eq!(augment_match_key_with_phash(None, None), None);
    }

    #[test]
    fn returns_none_when_no_path() {
        // Function early-returns None via ? operator when file_path is None
        assert_eq!(
            augment_match_key_with_phash(Some("key".to_string()), None),
            None
        );
    }

    #[test]
    fn returns_key_when_path_empty() {
        assert_eq!(
            augment_match_key_with_phash(Some("key".to_string()), Some("")),
            Some("key".to_string())
        );
    }

    #[test]
    fn appends_phash_when_file_exists() {
        let path = std::env::temp_dir().join(format!("phash_test_{}", Uuid::new_v4()));
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"test data")
            .unwrap();

        let result =
            augment_match_key_with_phash(Some("key".to_string()), Some(path.to_str().unwrap()));
        assert!(result.is_some());
        let val = result.unwrap();
        assert!(val.starts_with("key|phash:"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn phash_only_when_no_existing_key() {
        let path = std::env::temp_dir().join(format!("phash_test_{}", Uuid::new_v4()));
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"test data")
            .unwrap();

        let result = augment_match_key_with_phash(None, Some(path.to_str().unwrap()));
        assert!(result.is_some());
        let val = result.unwrap();
        assert!(val.starts_with("phash:"));

        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::{
        is_dev_runtime, runtime_app_identifier, runtime_deep_link_scheme, runtime_window_title,
        updater_staging_root,
    };

    #[test]
    fn is_dev_runtime_returns_bool() {
        let _ = is_dev_runtime();
    }

    #[test]
    fn updater_staging_root_contains_expected_name() {
        let root = updater_staging_root();
        let name = root.file_name().unwrap().to_str().unwrap();
        assert!(
            name == "slasshyvault-updater" || name == "slasshyvault-updater-dev",
            "Unexpected staging root name: {}",
            name
        );
    }

    #[test]
    fn updater_staging_root_is_under_temp() {
        let root = updater_staging_root();
        let temp = std::env::temp_dir();
        assert!(root.starts_with(&temp));
    }

    #[test]
    fn runtime_window_title_not_empty() {
        assert!(!runtime_window_title().is_empty());
    }

    #[test]
    fn runtime_app_identifier_contains_slasshyvault() {
        assert!(runtime_app_identifier().contains("slasshyvault"));
    }

    #[test]
    fn runtime_deep_link_scheme_contains_slasshyvault() {
        assert!(runtime_deep_link_scheme().contains("slasshyvault"));
    }

    #[test]
    fn dev_and_release_differ() {
        let title = runtime_window_title();
        let id = runtime_app_identifier();
        let scheme = runtime_deep_link_scheme();

        if is_dev_runtime() {
            assert_eq!(title, "SlasshyVault Dev");
            assert_eq!(id, "com.slasshyvault.app.dev");
            assert_eq!(scheme, "slasshyvault-dev");
        } else {
            assert_eq!(title, "SlasshyVault");
            assert_eq!(id, "com.slasshyvault.app");
            assert_eq!(scheme, "slasshyvault");
        }
    }
}

#[cfg(test)]
mod segments_private_ipv6_tests {
    use super::segments_are_private_ipv6;

    #[test]
    fn fc00_is_private() {
        let ip: std::net::Ipv6Addr = "fc00::1".parse().unwrap();
        assert!(segments_are_private_ipv6(ip));
    }

    #[test]
    fn fe80_is_private() {
        let ip: std::net::Ipv6Addr = "fe80::1".parse().unwrap();
        assert!(segments_are_private_ipv6(ip));
    }

    #[test]
    fn global_unicast_not_private() {
        let ip: std::net::Ipv6Addr = "2001:db8::1".parse().unwrap();
        assert!(!segments_are_private_ipv6(ip));
    }

    #[test]
    fn fd00_is_private() {
        let ip: std::net::Ipv6Addr = "fd00::1".parse().unwrap();
        assert!(segments_are_private_ipv6(ip));
    }
}

// ==================== WATCH TOGETHER COMMANDS ====================

/// Compute a CRC32 hash of the first 5MB of a file for content verification.
/// Returns a hex string to append to the media match key.
fn compute_partial_hash(file_path: &str) -> Option<String> {
    use crc32fast::Hasher as Crc32Hasher;
    use std::io::Read;

    let mut file = std::fs::File::open(file_path).ok()?;
    let mut hasher = Crc32Hasher::new();
    let mut buf = [0u8; 8192];
    let max_bytes: usize = 5 * 1024 * 1024; // 5MB
    let mut total_read: usize = 0;

    loop {
        if total_read >= max_bytes {
            break;
        }
        let to_read = std::cmp::min(buf.len(), max_bytes - total_read);
        match file.read(&mut buf[..to_read]) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                total_read += n;
            }
            Err(_) => return None,
        }
    }

    if total_read == 0 {
        return None;
    }

    Some(format!("{:08x}", hasher.finalize()))
}

/// Augment a media match key with a partial file hash if a local file path is available.
fn augment_match_key_with_phash(
    media_match_key: Option<String>,
    file_path: Option<&str>,
) -> Option<String> {
    let path = file_path?;
    if path.is_empty() {
        return media_match_key;
    }
    let hash = compute_partial_hash(path)?;
    let phash_token = format!("phash:{}", hash);
    match media_match_key {
        Some(key) if !key.is_empty() => Some(format!("{}|{}", key, phash_token)),
        _ => Some(phash_token),
    }
}

/// Create a Watch Together room
#[tauri::command]
async fn wt_create_room(
    state: State<'_, AppState>,
    window: WebviewWindow,
    media_id: i64,
    title: String,
    media_match_key: Option<String>,
    nickname: String,
    file_path: Option<String>,
) -> Result<watch_together::RoomInfo, String> {
    let media_match_key = augment_match_key_with_phash(media_match_key, file_path.as_deref());
    let wt = state.watch_together.clone();
    let manager = wt.lock().await;

    // Set up event callback to emit to frontend AND apply sync corrections
    let window_clone = window.clone();
    let wt_ctrl = state.wt_controller.clone();
    manager
        .set_event_callback(move |event| {
            // If this is a state_update, apply sync to MPV controller
            if let watch_together::WatchEvent::StateUpdate {
                position, paused, ..
            } = &event
            {
                let pos = *position;
                let is_paused = *paused;
                let ctrl = wt_ctrl.clone();
                tokio::spawn(async move {
                    let ctrl_guard = ctrl.lock().await;
                    if let Some(ref controller) = *ctrl_guard {
                        if let Err(e) = controller.apply_sync(pos, is_paused).await {
                            println!("[WT] Sync apply error: {}", e);
                        }
                    }
                });
            }
            // Also forward sync_command to MPV controller
            if let watch_together::WatchEvent::SyncCommand { ref command } = event {
                let action = command.action.clone();
                let pos = command.position;
                let ctrl = wt_ctrl.clone();
                tokio::spawn(async move {
                    let ctrl_guard = ctrl.lock().await;
                    if let Some(ref controller) = *ctrl_guard {
                        let (current_pos, current_paused) =
                            controller.get_estimated_position().await;
                        match action.as_str() {
                            "play" | "resume" => {
                                if current_paused {
                                    let _ = controller.set_paused(false).await;
                                }
                                if (pos - current_pos).abs() > 0.12 {
                                    let _ = controller.seek_to(pos).await;
                                }
                            }
                            "pause" => {
                                if !current_paused {
                                    let _ = controller.set_paused(true).await;
                                }
                                if (pos - current_pos).abs() > 0.12 {
                                    let _ = controller.seek_to(pos).await;
                                }
                            }
                            "seek" => {
                                if (pos - current_pos).abs() > 0.05 {
                                    let _ = controller.seek_to(pos).await;
                                }
                            }
                            _ => {}
                        }
                    }
                });
            }
            // Show OSD messages directly inside MPV (like Syncplay)
            if let watch_together::WatchEvent::ShowOsd {
                message,
                duration_ms,
            } = &event
            {
                let msg = message.clone();
                let dur = *duration_ms;
                let ctrl = wt_ctrl.clone();
                tokio::spawn(async move {
                    let ctrl_guard = ctrl.lock().await;
                    if let Some(ref controller) = *ctrl_guard {
                        let _ = controller.show_osd(&msg, dur).await;
                    }
                });
            }
            let _ = window_clone.emit("wt-event", &event);
        })
        .await;

    manager
        .create_room(media_id, title, media_match_key, nickname)
        .await
}

/// Join a Watch Together room
#[tauri::command]
async fn wt_join_room(
    state: State<'_, AppState>,
    window: WebviewWindow,
    room_code: String,
    media_id: i64,
    media_title: Option<String>,
    media_match_key: Option<String>,
    nickname: String,
    file_path: Option<String>,
) -> Result<watch_together::RoomInfo, String> {
    let media_match_key = augment_match_key_with_phash(media_match_key, file_path.as_deref());
    let wt = state.watch_together.clone();
    let manager = wt.lock().await;

    // Set up event callback with sync corrections
    let window_clone = window.clone();
    let wt_ctrl = state.wt_controller.clone();
    manager
        .set_event_callback(move |event| {
            if let watch_together::WatchEvent::StateUpdate {
                position, paused, ..
            } = &event
            {
                let pos = *position;
                let is_paused = *paused;
                let ctrl = wt_ctrl.clone();
                tokio::spawn(async move {
                    let ctrl_guard = ctrl.lock().await;
                    if let Some(ref controller) = *ctrl_guard {
                        if let Err(e) = controller.apply_sync(pos, is_paused).await {
                            println!("[WT] Sync apply error: {}", e);
                        }
                    }
                });
            }
            if let watch_together::WatchEvent::SyncCommand { ref command } = event {
                let action = command.action.clone();
                let pos = command.position;
                let ctrl = wt_ctrl.clone();
                tokio::spawn(async move {
                    let ctrl_guard = ctrl.lock().await;
                    if let Some(ref controller) = *ctrl_guard {
                        let (current_pos, current_paused) =
                            controller.get_estimated_position().await;
                        match action.as_str() {
                            "play" | "resume" => {
                                if current_paused {
                                    let _ = controller.set_paused(false).await;
                                }
                                if (pos - current_pos).abs() > 0.12 {
                                    let _ = controller.seek_to(pos).await;
                                }
                            }
                            "pause" => {
                                if !current_paused {
                                    let _ = controller.set_paused(true).await;
                                }
                                if (pos - current_pos).abs() > 0.12 {
                                    let _ = controller.seek_to(pos).await;
                                }
                            }
                            "seek" => {
                                if (pos - current_pos).abs() > 0.05 {
                                    let _ = controller.seek_to(pos).await;
                                }
                            }
                            _ => {}
                        }
                    }
                });
            }
            // Show OSD messages directly inside MPV (like Syncplay)
            if let watch_together::WatchEvent::ShowOsd {
                message,
                duration_ms,
            } = &event
            {
                let msg = message.clone();
                let dur = *duration_ms;
                let ctrl = wt_ctrl.clone();
                tokio::spawn(async move {
                    let ctrl_guard = ctrl.lock().await;
                    if let Some(ref controller) = *ctrl_guard {
                        let _ = controller.show_osd(&msg, dur).await;
                    }
                });
            }
            let _ = window_clone.emit("wt-event", &event);
        })
        .await;

    manager
        .join_room(room_code, media_id, media_title, media_match_key, nickname)
        .await
}

/// Leave the current Watch Together room
#[tauri::command]
async fn wt_leave_room(state: State<'_, AppState>) -> Result<(), String> {
    let wt = state.watch_together.clone();
    let manager = wt.lock().await;
    manager.leave_room().await
}

/// Set ready status with video duration
#[tauri::command]
async fn wt_set_ready(state: State<'_, AppState>, duration: f64) -> Result<(), String> {
    let wt = state.watch_together.clone();
    let manager = wt.lock().await;
    manager.set_ready(duration).await
}

/// Start playback (host only)
#[tauri::command]
async fn wt_start_playback(state: State<'_, AppState>) -> Result<(), String> {
    let wt = state.watch_together.clone();
    let manager = wt.lock().await;
    manager.start_playback().await
}

/// Send a sync command
#[tauri::command]
async fn wt_send_sync(
    state: State<'_, AppState>,
    action: String,
    position: f64,
) -> Result<(), String> {
    let wt = state.watch_together.clone();
    let manager = wt.lock().await;
    manager.send_sync(&action, position).await
}

/// Get current room state
#[tauri::command]
async fn wt_get_room_state(
    state: State<'_, AppState>,
) -> Result<Option<watch_together::RoomInfo>, String> {
    let wt = state.watch_together.clone();
    let manager = wt.lock().await;
    Ok(manager.get_room_state().await)
}

/// Check if Watch Together session is active
#[tauri::command]
async fn wt_is_active(state: State<'_, AppState>) -> Result<bool, String> {
    let wt = state.watch_together.clone();
    let manager = wt.lock().await;
    Ok(manager.is_active().await)
}

/// Get local Watch Together client ID for current session
#[tauri::command]
async fn wt_get_client_id(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let wt = state.watch_together.clone();
    let manager = wt.lock().await;
    Ok(manager.get_client_id().await)
}

/// Launch MPV in Watch Together sync mode
#[tauri::command]
async fn wt_launch_mpv(
    state: State<'_, AppState>,
    window: WebviewWindow,
    media_id: i64,
    session_id: String,
    start_position: f64,
) -> Result<u32, String> {
    // Get media info and config
    let (file_path, mpv_path, is_cloud, cloud_file_id, display_title) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let media = db.get_media_by_id(media_id).map_err(|e| e.to_string())?;
        let config = state.config.lock().map_err(|e| e.to_string())?;
        let mpv = config.mpv_path.clone().unwrap_or_else(|| "mpv".to_string());
        let display_title = build_mpv_display_title(&media);
        (
            media.file_path,
            mpv,
            media.is_cloud.unwrap_or(false),
            media.cloud_file_id,
            display_title,
        )
    };

    let file_or_url = if is_cloud {
        if let Some(file_id) = cloud_file_id {
            let (url, _token) = state
                .gdrive_client
                .get_stream_url(&file_id)
                .await
                .map_err(|e| format!("Failed to get cloud streaming URL: {}", e))?;
            url
        } else {
            return Err("Cloud file has no file ID".to_string());
        }
    } else {
        file_path.ok_or("No file path for local media")?
    };

    let auth_header: Option<String> = if is_cloud {
        state
            .gdrive_client
            .get_access_token()
            .await
            .ok()
            .map(|t| format!("Authorization: Bearer {}", t))
    } else {
        None
    };

    // Determine if host
    let is_host = {
        let wt = state.watch_together.clone();
        let manager = wt.lock().await;
        manager.is_host().await
    };

    // Launch MPV with pipe-based IPC (replaces old file-based approach)
    let (pid, mut controller) = watch_together_mpv::launch_mpv_wt(
        &mpv_path,
        &file_or_url,
        media_id,
        Some(&display_title),
        &session_id,
        start_position,
        auth_header.as_deref(),
        is_host,
    )?;

    // Connect to MPV's named pipe for bidirectional IPC
    controller.connect().await?;

    // Take the event receiver before storing the controller
    let mut event_rx = controller
        .take_event_rx()
        .await
        .ok_or("Failed to get MPV event receiver")?;

    // Store the controller in AppState
    {
        let mut wt_ctrl = state.wt_controller.lock().await;
        *wt_ctrl = Some(controller);
    }

    // Store MPV PID in watch session
    {
        let wt = state.watch_together.clone();
        let manager = wt.lock().await;
        manager.set_mpv_pid(pid).await;
    }

    // Spawn the sync orchestration loop
    let wt_clone = state.watch_together.clone();
    let wt_ctrl = state.wt_controller.clone();
    let session_id_clone = session_id.clone();
    let window_clone = window.clone();

    tokio::spawn(async move {
        let mut state_report_interval =
            tokio::time::interval(std::time::Duration::from_millis(500));
        state_report_interval.tick().await;

        loop {
            tokio::select! {
                // Handle MPV events from the pipe reader
                Some(mpv_event) = event_rx.recv() => {
                    match mpv_event {
                        watch_together_mpv::MpvSyncEvent::PauseChanged { paused, position } => {
                            println!("[WT] MPV pause changed: paused={}, pos={:.2}", paused, position);
                            let action = if paused { "pause" } else { "play" };
                            let manager = wt_clone.lock().await;
                            let _ = manager.send_sync(action, position).await;
                        }
                        watch_together_mpv::MpvSyncEvent::Seeked { position } => {
                            println!("[WT] MPV user seek to {:.2}", position);
                            let manager = wt_clone.lock().await;
                            let _ = manager.send_sync("seek", position).await;
                        }
                        watch_together_mpv::MpvSyncEvent::Ended => {
                            println!("[WT] MPV process ended");
                            watch_together_mpv::cleanup_session(&session_id_clone);
                            let _ = window_clone.emit("wt-mpv-ended", ());
                            // Clear the controller
                            let mut ctrl = wt_ctrl.lock().await;
                            *ctrl = None;
                            break;
                        }
                        watch_together_mpv::MpvSyncEvent::PositionUpdate { .. } => {
                            // Handled internally by the controller's local_state
                        }
                    }
                }

                // Periodic state report to server (Syncplay-style)
                _ = state_report_interval.tick() => {
                    let ctrl = wt_ctrl.lock().await;
                    if let Some(ref controller) = *ctrl {
                        let (pos, paused) = controller.get_estimated_position().await;
                        drop(ctrl);
                        let manager = wt_clone.lock().await;
                        let session = manager.session.lock().await;
                        if let Some(ref session) = *session {
                            let _ = session.send_message(
                                watch_together::ClientMessage::StateReport {
                                    position: pos,
                                    paused,
                                }
                            ).await;
                        }
                    }
                }
            }
        }
    });

    Ok(pid)
}

/// Send a command to MPV in Watch Together mode
#[tauri::command]
async fn wt_send_mpv_command(
    state: State<'_, AppState>,
    session_id: String,
    action: String,
    position: f64,
) -> Result<(), String> {
    let ctrl = state.wt_controller.lock().await;
    if let Some(ref controller) = *ctrl {
        match action.as_str() {
            "play" | "resume" => {
                controller.set_paused(false).await?;
                if position > 0.0 {
                    controller.seek_to(position).await?;
                }
            }
            "pause" => {
                controller.set_paused(true).await?;
            }
            "seek" => {
                controller.seek_to(position).await?;
            }
            _ => {
                return Err(format!("Unknown action: {}", action));
            }
        }
        Ok(())
    } else {
        // Fallback to file-based IPC
        mpv_ipc::send_mpv_sync_command(&session_id, &action, position)
    }
}

async fn run_startup_metadata_enrichment(app_handle: AppHandle) {
    let state = app_handle.state::<AppState>();

    let credential = {
        let config = match state.config.lock() {
            Ok(c) => c,
            Err(e) => {
                println!("[STARTUP] Metadata enrichment skipped (config lock): {}", e);
                return;
            }
        };
        tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default())
    };

    let candidates = {
        let db = match state.db.lock() {
            Ok(d) => d,
            Err(e) => {
                println!("[STARTUP] Metadata enrichment skipped (db lock): {}", e);
                return;
            }
        };

        match db.get_media_needing_metadata_enrichment(5000) {
            Ok(rows) => rows,
            Err(e) => {
                println!(
                    "[STARTUP] Failed to load metadata enrichment candidates: {}",
                    e
                );
                return;
            }
        }
    };

    if candidates.is_empty() {
        println!("[STARTUP] Metadata enrichment not needed.");
        return;
    }

    let total = candidates.len();
    println!(
        "[STARTUP] Metadata enrichment queued for {} item(s) with {} parallel workers...",
        total,
        16.min(total).max(1)
    );

    let db_path = database::get_database_path();
    let image_cache_dir = database::get_image_cache_dir();

    // Parallel worker design: chunk candidates into groups, each gets its own DB connection.
    // WAL mode allows concurrent readers+writes without contention.
    let num_workers = 16usize.min(total).max(1);
    let chunk_size = (total + num_workers - 1) / num_workers;
    let updated = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let no_match = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::with_capacity(num_workers);

    for chunk in candidates.chunks(chunk_size) {
        let chunk = chunk.to_vec();
        let credential = credential.clone();
        let db_path = db_path.clone();
        let image_cache_dir = image_cache_dir.clone();
        let updated = updated.clone();
        let failed = failed.clone();
        let no_match = no_match.clone();

        handles.push(tokio::task::spawn_blocking(move || -> Result<(), String> {
            let db = database::Database::new(&db_path).map_err(|e| e.to_string())?;
            // Begin a batch transaction for this chunk — 1 commit per ~16 items
            // instead of individual commits per item
            db.begin_batch().map_err(|e| e.to_string())?;

            for item in &chunk {
                let tmdb_media_type = if item.media_type == "tvshow" {
                    "tv"
                } else {
                    "movie"
                };

                let metadata_result = if let Some(ref tid) = item.tmdb_id {
                    let cleaned = tid.trim();
                    if cleaned.is_empty() {
                        tmdb::search_metadata_with_fallback(
                            &credential,
                            &item.title,
                            tmdb_media_type,
                            item.year,
                            &image_cache_dir,
                        )
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| "No TMDB match found".to_string())
                    } else {
                        tmdb::fetch_metadata_by_id_with_fallback(
                            &credential,
                            cleaned,
                            tmdb_media_type,
                            &image_cache_dir,
                        )
                        .map_err(|e| e.to_string())
                    }
                } else {
                    tmdb::search_metadata_with_fallback(
                        &credential,
                        &item.title,
                        tmdb_media_type,
                        item.year,
                        &image_cache_dir,
                    )
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "No TMDB match found".to_string())
                };

                match metadata_result {
                    Ok(mut metadata) => {
                        metadata.title = media_manager::prefer_title_with_leading_article(
                            &item.title,
                            &metadata.title,
                        );

                        // PRIMARY poster source: balloonerismm `/images` if we have an imdb_id
                        let tmdb_poster = metadata.poster_path.clone();
                        if let Some(ref imdb_id) = metadata.imdb_id {
                            println!("[BALLOON] Enrichment primary: trying poster for \"{}\" (imdb_id: {})", item.title, imdb_id);
                            let image_type = if item.media_type == "tv" || item.media_type == "tvshow" {
                                tmdb::ImageType::SeriesBanner
                            } else {
                                tmdb::ImageType::MovieBanner
                            };
                            let kind = if item.media_type == "tv" || item.media_type == "tvshow" {
                                "tv"
                            } else {
                                "movie"
                            };
                            if let Ok(imgs) = balloonerismm_api::get_images(kind, imdb_id) {
                                if let Some(img_url) = imgs.pick_best_poster_url() {
                                    if let Some(cached_path) = tmdb::cache_imdb_image(
                                        &img_url,
                                        std::path::Path::new(&image_cache_dir),
                                        &image_type,
                                    ) {
                                        println!("[BALLOON] Enrichment poster (primary) result: Ok(\"{}\")", cached_path);
                                        metadata.poster_path = Some(cached_path);
                                    } else {
                                        println!("[BALLOON] Enrichment poster (primary) result: Err(cache failed), keeping TMDB poster");
                                    }
                                } else {
                                    println!("[BALLOON] Enrichment poster (primary) result: Err(no posters in response), keeping TMDB poster");
                                }
                            } else {
                                println!("[BALLOON] Enrichment poster (primary) result: Err(/images request failed), keeping TMDB poster");
                            }
                        }

                        if metadata.poster_path == tmdb_poster && metadata.poster_path.is_some() {
                            println!("[TMDB] Using TMDB poster for \"{}\": poster={:?}", item.title, metadata.poster_path);
                        } else if metadata.poster_path.is_some() {
                            println!("[TMDB] Enriched \"{}\": balloonerismm poster used as primary", item.title);
                        }

                        if db.update_metadata(item.id, &metadata).is_ok() {
                            updated.fetch_add(1, Ordering::Relaxed);
                        } else {
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(err) => {
                        if err.contains("No TMDB match found") {
                            no_match.fetch_add(1, Ordering::Relaxed);
                        } else {
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }

            // Single commit per chunk
            db.commit_batch().map_err(|e| e.to_string())?;
            Ok(())
        }));
    }

    // Wait for all parallel workers
    for handle in handles {
        if let Err(e) = handle.await.unwrap_or_else(|e| Err(e.to_string())) {
            println!("[STARTUP] Enrichment worker failed: {}", e);
        }
    }

    let updated = updated.load(Ordering::Relaxed);
    let failed = failed.load(Ordering::Relaxed);
    let no_match = no_match.load(Ordering::Relaxed);

    // Check remaining after parallel pass
    let remaining = match database::Database::new(&db_path) {
        Ok(db) => db
            .get_media_needing_metadata_enrichment(1)
            .map(|rows| rows.len())
            .unwrap_or(0),
        Err(_) => 0,
    };

    println!(
        "[STARTUP] Metadata enrichment done. updated={}, failed={}, unmatched={}, remaining={}",
        updated, failed, no_match, remaining
    );

    if updated > 0 {
        if let Some(window) = app_handle.get_webview_window("main") {
            let _ = window.emit(
                "library-updated",
                serde_json::json!({ "type": "metadata-enriched", "updated": updated }),
            );
        }
    }
}

// ---- Direct Download Link (DDL) commands ----

#[tauri::command]
async fn ddl_validate_url(url: String) -> Result<direct_link_manager::DdlValidationResult, String> {
    tokio::task::spawn_blocking(move || direct_link_manager::validate_url(&url))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn ddl_index_archive(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
    url: String,
    validation: direct_link_manager::DdlValidationResult,
    addon_origin: Option<String>,
) -> Result<direct_link_manager::DdlSource, String> {
    let emit_ddl_progress = |stage: &str,
                             message: String,
                             filename: Option<String>,
                             current: Option<usize>,
                             total: Option<usize>,
                             season: Option<i32>,
                             episode: Option<i32>,
                             episode_title: Option<String>| {
        let _ = window.emit(
            "ddl-index-progress",
            DdlIndexProgressPayload {
                stage: stage.to_string(),
                message,
                filename,
                current,
                total,
                season,
                episode,
                episode_title,
            },
        );
    };

    emit_ddl_progress(
        "probing-archive",
        "Reading archive structure...".to_string(),
        Some(validation.filename.clone()),
        None,
        None,
        None,
        None,
        None,
    );

    let result =
        tokio::task::spawn_blocking(move || direct_link_manager::index_archive(&url, &validation))
            .await
            .map_err(|e| e.to_string())??;

    // Get config for TMDB API key and image cache dir
    let (api_key, image_cache_dir) = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        (
            tmdb::get_tmdb_credential(&config.tmdb_api_key.clone().unwrap_or_default()),
            database::get_image_cache_dir(),
        )
    };
    std::fs::create_dir_all(&image_cache_dir).ok();

    // Save source to database
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.upsert_ddl_source(
            &result.source.id,
            &result.source.url,
            &result.source.filename,
            result.source.file_size as i64,
            &result.source.archive_format,
            result.source.entry_count as i64,
            result.source.video_count as i64,
            result.source.cd_offset as i64,
            result.source.cd_size as i64,
            addon_origin.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    }

    // Determine if this is a multi-episode (TV show) or single file (movie)
    let has_episodes = result.entries.iter().any(|e| e.season.is_some());
    let is_tvshow = has_episodes || result.entries.len() > 1;

    emit_ddl_progress(
        "archive-indexed",
        format!(
            "Found {} playable entr{}.",
            result.entries.len(),
            if result.entries.len() == 1 {
                "y"
            } else {
                "ies"
            }
        ),
        Some(result.source.filename.clone()),
        Some(0),
        Some(result.entries.len()),
        None,
        None,
        None,
    );

    let parent_id = if is_tvshow {
        // Build a few title candidates because season-pack archive names are often noisy.
        let archive_filename = &result.source.filename;
        let archive_parsed = media_manager::parse_cloud_filename(archive_filename);
        let first_entry_title = result
            .entries
            .first()
            .map(|e| e.title.trim().to_string())
            .unwrap_or_default();

        let mut title_candidates = Vec::new();
        if archive_parsed.title.len() > 2 && archive_parsed.title.to_lowercase() != "archive" {
            title_candidates.push(archive_parsed.title.clone());
        }
        if !first_entry_title.is_empty()
            && !title_candidates
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&first_entry_title))
        {
            title_candidates.push(first_entry_title.clone());
        }
        if title_candidates.is_empty() {
            title_candidates.push(result.source.filename.clone());
        }

        let series_title = title_candidates
            .first()
            .cloned()
            .unwrap_or_else(|| result.source.filename.clone());
        let series_year = archive_parsed.year;

        // Search TMDB for the show using the strongest title/year hints first.
        let tmdb_result = {
            let mut matched: Option<(String, tmdb::TmdbMetadata)> = None;
            for candidate in &title_candidates {
                emit_ddl_progress(
                    "fetching-show-metadata",
                    format!("Matching show metadata for '{}'...", candidate),
                    Some(result.source.filename.clone()),
                    Some(0),
                    Some(result.entries.len()),
                    None,
                    None,
                    None,
                );
                println!(
                    "[DDL] Searching TMDB for show: {} (year: {:?})",
                    candidate, series_year
                );
                let api_key = api_key.clone();
                let candidate_owned = candidate.clone();
                let image_cache_dir = image_cache_dir.clone();
                match tokio::task::spawn_blocking(move || {
                    tmdb::search_metadata_with_fallback(
                        &api_key,
                        &candidate_owned,
                        "tv",
                        series_year,
                        &image_cache_dir,
                    )
                })
                .await
                .map_err(|e| e.to_string())?
                {
                    Ok(Some(meta)) => {
                        matched = Some((candidate.clone(), meta));
                        break;
                    }
                    Ok(None) => {
                        println!("[DDL] No TMDB TV match for '{}'", candidate);
                    }
                    Err(err) => {
                        println!("[DDL] TMDB TV search failed for '{}': {}", candidate, err);
                    }
                }
            }
            matched
        };

        let (title, year, overview, cast_names, poster_path, tmdb_id) = match &tmdb_result {
            Some((matched_title, meta)) => (
                media_manager::prefer_title_with_leading_article(matched_title, &meta.title),
                meta.year,
                meta.overview.clone(),
                meta.cast_names.clone(),
                meta.poster_path.clone(),
                meta.tmdb_id.clone(),
            ),
            None => (series_title.clone(), None, None, None, None, None),
        };

        let parent_id = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.insert_ddl_tvshow(
                &title,
                &result.source.id,
                year,
                overview.as_deref(),
                cast_names.as_deref(),
                poster_path.as_deref(),
                tmdb_id.as_deref(),
            )
            .map_err(|e| e.to_string())?
        };

        // Pre-fetch episode metadata from TMDB
        if let Some(ref tid) = tmdb_id {
            // Get all unique seasons
            let seasons: std::collections::HashSet<i32> =
                result.entries.iter().filter_map(|e| e.season).collect();
            for season_num in seasons {
                let season_episode_total = result
                    .entries
                    .iter()
                    .filter(|entry| entry.season == Some(season_num))
                    .count();
                emit_ddl_progress(
                    "fetching-episode-metadata",
                    format!("Fetching episode metadata for Season {}...", season_num),
                    Some(result.source.filename.clone()),
                    Some(0),
                    Some(season_episode_total),
                    Some(season_num),
                    None,
                    None,
                );
                let api_key = api_key.clone();
                let tmdb_id = tid.clone();
                let title_clone = title.clone();
                let image_cache_dir = image_cache_dir.clone();
                match tokio::task::spawn_blocking(move || {
                    tmdb::fetch_season_episodes(
                        &api_key,
                        &tmdb_id,
                        season_num,
                        &title_clone,
                        &image_cache_dir,
                    )
                })
                .await
                .map_err(|e| e.to_string())?
                {
                    Ok(season_info) => {
                        // Cache all episode metadata for later lookup
                        let db = state.db.lock().map_err(|e| e.to_string())?;
                        for ep in &season_info.episodes {
                            let _ = db.save_cached_episode_metadata(
                                tid,
                                ep.season_number,
                                ep.episode_number,
                                Some(&ep.name),
                                ep.overview.as_deref(),
                                ep.still_path.as_deref(),
                                ep.air_date.as_deref(),
                                ep.vote_average,
                            );
                        }
                        emit_ddl_progress(
                            "fetching-episode-metadata",
                            format!("Cached metadata for Season {}.", season_num),
                            Some(result.source.filename.clone()),
                            Some(season_episode_total),
                            Some(season_episode_total),
                            Some(season_num),
                            None,
                            season_info.episodes.last().map(|ep| ep.name.clone()),
                        );
                        println!(
                            "[DDL] Cached {} episode stills for S{:02}",
                            season_info.episodes.len(),
                            season_num
                        );
                    }
                    Err(err) => {
                        println!(
                            "[DDL] Failed to fetch season metadata for '{}' S{:02}: {}",
                            title, season_num, err
                        );
                    }
                }
            }
        }

        Some((parent_id, tmdb_id))
    } else {
        None
    };

    // Insert each entry as a media item
    for (idx, entry) in result.entries.iter().enumerate() {
        emit_ddl_progress(
            "adding-entry",
            if let (Some(season), Some(episode)) = (entry.season, entry.episode) {
                format!("Adding S{:02}E{:02} to library...", season, episode)
            } else {
                format!("Adding '{}' to library...", entry.entry_name)
            },
            Some(result.source.filename.clone()),
            Some(idx + 1),
            Some(result.entries.len()),
            entry.season,
            entry.episode,
            Some(entry.title.clone()),
        );

        let title = if entry.title.is_empty() {
            entry.entry_name.clone()
        } else {
            entry.title.clone()
        };
        let season = entry
            .season
            .or(if parent_id.is_some() { Some(1) } else { None });
        let episode = entry.episode.or(if parent_id.is_some() {
            Some((idx + 1) as i32)
        } else {
            None
        });

        // Fetch episode-specific metadata from TMDB cache
        let (ep_title, ep_overview, ep_still) = if let Some((_, Some(ref tmdb_id))) = parent_id {
            if let (Some(s), Some(e)) = (season, episode) {
                let db = state.db.lock().map_err(|e| e.to_string())?;
                db.get_cached_episode_metadata(tmdb_id, s, e)
                    .ok()
                    .flatten()
                    .map(|cached| (cached.episode_title, cached.overview, cached.still_path))
                    .unwrap_or((None, None, None))
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        let pid = parent_id.as_ref().map(|(id, _)| *id);
        let inserted_id = {
            let db = state.db.lock().map_err(|e| e.to_string())?;
            db.insert_ddl_episode(
                &title,
                pid,
                season,
                episode,
                &result.source.id,
                &result.source.archive_format,
                &entry.entry_path,
                entry.local_header_offset as i64,
                entry.data_start_offset as i64,
                entry.compressed_size as i64,
                entry.uncompressed_size as i64,
                &entry.crc32,
                entry.compression_method,
                ep_title.as_deref(),
                ep_overview.as_deref(),
                ep_still.as_deref(),
            )
            .map_err(|e| e.to_string())?
        };

        // For single movies, also enrich with TMDB right after insert
        if !is_tvshow {
            let movie_title = if !entry.title.is_empty() && entry.title.len() > 3 {
                entry.title.clone()
            } else {
                // If entry title is weak, use archive title if it looks like a movie
                let archive_parsed = media_manager::parse_cloud_filename(&result.source.filename);
                if archive_parsed.media_type == media_manager::MediaParseType::Movie {
                    archive_parsed.title
                } else {
                    entry.title.clone()
                }
            };

            println!("[DDL] Searching TMDB for movie: {}", movie_title);
            let api_key = api_key.clone();
            let movie_title_for_search = movie_title.clone();
            let image_cache_dir = image_cache_dir.clone();
            match tokio::task::spawn_blocking(move || {
                tmdb::search_metadata_with_fallback(
                    &api_key,
                    &movie_title_for_search,
                    "movie",
                    None,
                    &image_cache_dir,
                )
            })
            .await
            .map_err(|e| e.to_string())?
            {
                Ok(Some(meta)) => {
                    let db = state.db.lock().map_err(|e| e.to_string())?;
                    let _ = db.update_metadata(inserted_id, &meta);
                    println!(
                        "[DDL] Enriched movie '{}' with TMDB metadata (Poster: {:?})",
                        movie_title, meta.poster_path
                    );
                }
                Ok(None) => {
                    println!("[DDL] No TMDB movie match for '{}'", movie_title);
                }
                Err(err) => {
                    println!(
                        "[DDL] TMDB movie search failed for '{}': {}",
                        movie_title, err
                    );
                }
            }
        }
    }

    println!(
        "[DDL] Saved source '{}' with {} media entries (TMDB enriched)",
        result.source.filename,
        result.entries.len()
    );

    emit_ddl_progress(
        "completed",
        format!(
            "Added {} entr{} successfully.",
            result.entries.len(),
            if result.entries.len() == 1 {
                "y"
            } else {
                "ies"
            }
        ),
        Some(result.source.filename.clone()),
        Some(result.entries.len()),
        Some(result.entries.len()),
        None,
        None,
        None,
    );

    let _ = window.emit(
        "library-updated",
        serde_json::json!({
            "type": "ddl-indexed",
            "title": result.source.filename,
            "source_id": result.source.id,
        }),
    );

    Ok(result.source)
}

#[tauri::command]
fn ddl_get_sources(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<direct_link_manager::DdlSource>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_ddl_sources().map_err(|e| e.to_string())
}

#[tauri::command]
async fn ddl_check_link_health(
    state: tauri::State<'_, AppState>,
    source_id: String,
) -> Result<bool, String> {
    let url = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_ddl_source_url(&source_id)
            .map_err(|e| e.to_string())?
    };

    let healthy = tokio::task::spawn_blocking(move || direct_link_manager::check_link_health(&url))
        .await
        .map_err(|e| e.to_string())??;

    if !healthy {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.mark_ddl_source_expired(&source_id)
            .map_err(|e| e.to_string())?;
    }

    Ok(healthy)
}

#[tauri::command]
async fn ddl_refresh_link(
    state: tauri::State<'_, AppState>,
    source_id: String,
    new_url: String,
) -> Result<direct_link_manager::DdlRefreshResult, String> {
    let source = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_ddl_source(&source_id).map_err(|e| e.to_string())?
    };

    let url_for_verify = new_url.clone();
    let result = tokio::task::spawn_blocking(move || {
        direct_link_manager::verify_and_refresh_link(&source, &url_for_verify)
    })
    .await
    .map_err(|e| e.to_string())??;

    if result.accepted {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.update_ddl_source_url(&source_id, &new_url)
            .map_err(|e| e.to_string())?;
        println!("[DDL] Refreshed link for source '{}'", source_id);
    }

    Ok(result)
}

#[tauri::command]
fn ddl_delete_source(state: tauri::State<'_, AppState>, source_id: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.delete_ddl_source_and_media(&source_id)
        .map_err(|e| e.to_string())?;
    println!("[DDL] Deleted source '{}'", source_id);
    Ok(())
}

#[tauri::command]
fn ddl_get_source_media(
    state: tauri::State<'_, AppState>,
    source_id: String,
) -> Result<Vec<database::MediaItem>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_media_by_ddl_source(&source_id)
        .map_err(|e| e.to_string())
}

/// Index a season pack stream URL into Direct Links, storing addon origin for auto-refresh
#[tauri::command]
async fn index_season_pack_to_ddl(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
    url: String,
    imdb_id: String,
    season_number: i32,
    stream_name: String,
) -> Result<direct_link_manager::DdlSource, String> {
    let origin = format!("{}:{}:{}", imdb_id, season_number, stream_name);

    // Validate URL — must support HTTP 206 (range requests)
    let url_clone = url.clone();
    let validation =
        tokio::task::spawn_blocking(move || direct_link_manager::validate_url(&url_clone))
            .await
            .map_err(|e| e.to_string())??;

    if !validation.supports_range {
        return Err(
            "This stream URL does not support seeking (HTTP 206). Cannot index to Direct Links."
                .to_string(),
        );
    }

    // Delegate to ddl_index_archive with addon_origin
    ddl_index_archive(window, state, url, validation, Some(origin)).await
}

/// Auto-refresh an expired DDL source by re-querying the addon server
#[tauri::command]
async fn auto_refresh_ddl_from_addon(
    state: tauri::State<'_, AppState>,
    source_id: String,
) -> Result<Option<String>, String> {
    let source = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_ddl_source(&source_id).map_err(|e| e.to_string())?
    };

    let origin = match &source.addon_origin {
        Some(o) => o.clone(),
        None => return Ok(None),
    };

    // Parse "imdb_id:season:stream_name"
    let parts: Vec<String> = origin.splitn(3, ':').map(|s| s.to_string()).collect();
    if parts.len() < 2 {
        return Ok(None);
    }
    let imdb_id = parts[0].clone();
    let season: i32 = parts[1]
        .parse()
        .map_err(|_| "Invalid season in addon_origin".to_string())?;

    // Get addon URL from config
    let addon_url = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        if let Some(src) = config
            .addon_sources
            .iter()
            .find(|s| s.is_default)
            .or(config.addon_sources.first())
        {
            src.url.clone()
        } else if let Some(ref url) = config.addon_url {
            url.clone()
        } else {
            return Ok(None);
        }
    };

    // Re-query addon for season streams
    let file_size = source.file_size;
    let source_for_verify = source.clone();
    let addon_url_clone = addon_url.clone();

    let stream_name_fallback = parts.get(2).cloned().unwrap_or_default();

    let new_url = tokio::task::spawn_blocking(move || {
        let streams =
            remote_source::fetch_season_streams(&imdb_id, season, &addon_url_clone, true)?;

        // Find matching stream by file_size
        for (_ep, ep_streams) in &streams {
            for s in ep_streams {
                if s.video_size > 0 && (s.video_size as u64) == file_size {
                    return Ok::<String, String>(s.url.clone());
                }
            }
        }

        // Fallback: match by stream name
        for (_ep, ep_streams) in &streams {
            for s in ep_streams {
                if s.name == stream_name_fallback {
                    return Ok(s.url.clone());
                }
            }
        }

        Err("No matching stream found in addon response".to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Validate the new URL matches the original archive structure
    let url_for_verify = new_url.clone();
    let refresh_result = tokio::task::spawn_blocking(move || {
        direct_link_manager::verify_and_refresh_link(&source_for_verify, &url_for_verify)
    })
    .await
    .map_err(|e| e.to_string())??;

    if refresh_result.accepted {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.update_ddl_source_url(&source_id, &new_url)
            .map_err(|e| e.to_string())?;
        println!(
            "[DDL] Auto-refreshed expired link for source '{}' from addon",
            source_id
        );
        Ok(Some(new_url))
    } else {
        println!(
            "[DDL] Auto-refresh failed for source '{}': {}",
            source_id, refresh_result.message
        );
        Ok(None)
    }
}

/// Returns the OLD app data directory (pre-rename "StreamVault") respecting dev/prod isolation.
/// Dev builds target "StreamVault-Dev", production targets "StreamVault".
fn get_old_app_data_dir() -> std::path::PathBuf {
    let dir_name = if cfg!(debug_assertions) {
        "StreamVault-Dev"
    } else {
        "StreamVault"
    };
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return std::path::PathBuf::from(appdata).join(dir_name);
        }
    }
    dirs::home_dir()
        .map(|h| h.join(format!(".{}", dir_name)))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// Returns the NEW app data directory (post-rename "SlasshyVault"), delegating to database::get_app_data_dir.
fn get_new_app_data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(database::get_app_data_dir().to_string_lossy().as_ref())
}

fn copy_dir_all(
    src: impl AsRef<std::path::Path>,
    dst: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(&src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.as_ref().join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn migrate_app_data() {
    let old_dir = get_old_app_data_dir();
    let new_dir = get_new_app_data_dir();

    // If old dir doesn't exist, nothing to migrate (fresh install or already migrated)
    if !old_dir.exists() {
        println!(
            "[MIGRATE] Old data directory {:?} does not exist, no migration needed",
            old_dir
        );
        return;
    }

    // If both old and new databases exist, migration was already completed
    let old_db = old_dir.join("media_library.db");
    let new_db = new_dir.join("media_library.db");
    if old_db.exists() && new_db.exists() {
        if let Ok(db) = database::Database::new(&new_db.to_string_lossy()) {
            if let Ok(Some(_)) = db.get_setting("migration_completed") {
                println!("[MIGRATE] Migration already completed (flag found), skipping");
                return;
            }
        }
    }

    println!(
        "[MIGRATE] Migrating app data from {:?} to {:?}",
        old_dir, new_dir
    );

    match copy_dir_all(&old_dir, &new_dir) {
        Ok(_) => {
            println!("[MIGRATE] Successfully copied data to {:?}", new_dir);

            if new_db.exists() {
                match database::Database::new(&new_db.to_string_lossy()) {
                    Ok(_db) => {
                        println!("[MIGRATE] Verified database at new location");
                        // Mark migration as completed in the new DB (old dir is preserved)
                        let _ = _db.set_setting("migration_completed", "true");
                    }
                    Err(e) => {
                        println!(
                            "[MIGRATE] Warning: Database verification failed: {}. Rolling back.",
                            e
                        );
                        let _ = std::fs::remove_dir_all(&new_dir);
                    }
                }
            } else {
                println!("[MIGRATE] Warning: Database file not found at new location");
            }
        }
        Err(e) => {
            println!("[MIGRATE] Warning: Failed to copy data: {}. Startup will continue with fresh data.", e);
            let _ = std::fs::remove_dir_all(&new_dir);
        }
    }
}

// ── Progressive Download Helpers ─────────────────────────────────────

/// Download the first `buffer_bytes` from `url` to `dest_path` using an HTTP Range request.
async fn progressive_download(
    url: &str,
    dest_path: &std::path::Path,
    buffer_bytes: u64,
) -> Result<(), String> {
    use reqwest::header;

    // Ensure parent directory exists
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create_dir: {}", e))?;
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("client build: {}", e))?;

    // Try Range request first; fall back to full download if server doesn't support it
    let range_end = buffer_bytes.saturating_sub(1);
    let range_header = format!("bytes=0-{}", range_end);

    let resp = client
        .get(url)
        .header(header::RANGE, &range_header)
        .send()
        .await
        .map_err(|e| format!("request: {}", e))?;

    let status = resp.status();
    if !status.is_success() && status.as_u16() != 206 {
        return Err(format!("HTTP {}", status));
    }

    let mut file = std::fs::File::create(dest_path).map_err(|e| format!("file create: {}", e))?;
    let mut bytes_written: u64 = 0;

    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("chunk read: {}", e))?;
        use std::io::Write;
        file.write_all(&chunk)
            .map_err(|e| format!("file write: {}", e))?;
        bytes_written += chunk.len() as u64;
        if bytes_written >= buffer_bytes {
            break;
        }
    }

    use std::io::Write as _;
    file.flush().map_err(|e| format!("file flush: {}", e))?;
    println!(
        "[PROGRESSIVE-DL] Downloaded {} bytes to {}",
        bytes_written,
        dest_path.display()
    );
    Ok(())
}

/// Continue downloading from `buffer_bytes` offset to the end of the file.
async fn download_remaining(
    url: &str,
    dest_path: &std::path::Path,
    start_offset: u64,
    _total_bytes: u64,
) -> Result<(), String> {
    use reqwest::header;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3600)) // 1 hour for large files
        .build()
        .map_err(|e| format!("client build: {}", e))?;

    let range_header = format!("bytes={}-", start_offset);

    let resp = client
        .get(url)
        .header(header::RANGE, &range_header)
        .send()
        .await
        .map_err(|e| format!("request: {}", e))?;

    let status = resp.status();
    if !status.is_success() && status.as_u16() != 206 {
        return Err(format!("HTTP {}", status));
    }

    // Open file in append mode
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(dest_path)
        .map_err(|e| format!("file open: {}", e))?;
    use std::io::Seek;
    file.seek(std::io::SeekFrom::Start(start_offset))
        .map_err(|e| format!("file seek: {}", e))?;

    let mut bytes_written: u64 = 0;
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("chunk read: {}", e))?;
        use std::io::Write;
        file.write_all(&chunk)
            .map_err(|e| format!("file write: {}", e))?;
        bytes_written += chunk.len() as u64;
    }

    use std::io::Write as _;
    file.flush().map_err(|e| format!("file flush: {}", e))?;
    println!(
        "[PROGRESSIVE-DL] Downloaded remaining {} bytes (total offset {})",
        bytes_written,
        start_offset + bytes_written
    );
    Ok(())
}

// ── Remote Source Commands ──────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteStreamResponse {
    name: String,
    description: String,
    url: String,
    video_size: i64,
    not_web_ready: bool,
    parsed_quality: String,
    parsed_source: String,
    #[serde(default)]
    recommended: bool,
    #[serde(default)]
    is_hubdrive: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupedStreamsResponse {
    quality: String,
    streams: Vec<RemoteStreamResponse>,
}

#[tauri::command]
async fn remote_get_movie_streams(
    state: State<'_, AppState>,
    imdb_id: String,
    force_refresh: Option<bool>,
) -> Result<Vec<GroupedStreamsResponse>, String> {
    let base_url = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        get_active_addon_url_from_config(&config)?
    };

    let refresh = force_refresh.unwrap_or(false);
    let streams = tokio::task::spawn_blocking(move || {
        remote_source::fetch_movie_streams(&imdb_id, &base_url, refresh)
    })
    .await
    .map_err(|e| e.to_string())??;

    let grouped = remote_source::group_streams(streams);
    let response: Vec<GroupedStreamsResponse> = grouped
        .into_iter()
        .map(|g| GroupedStreamsResponse {
            quality: g.quality,
            streams: g
                .streams
                .into_iter()
                .map(|s| RemoteStreamResponse {
                    name: s.name,
                    description: s.description,
                    url: s.url,
                    video_size: s.video_size,
                    not_web_ready: s.not_web_ready,
                    parsed_quality: s.parsed_quality,
                    parsed_source: s.parsed_source,
                    recommended: s.recommended,
                    is_hubdrive: s.is_hubdrive,
                })
                .collect(),
        })
        .collect();

    Ok(response)
}

#[tauri::command]
async fn remote_get_series_streams(
    state: State<'_, AppState>,
    imdb_id: String,
    season: i32,
    episode: i32,
    force_refresh: Option<bool>,
) -> Result<Vec<GroupedStreamsResponse>, String> {
    let base_url = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        get_active_addon_url_from_config(&config)?
    };

    let refresh = force_refresh.unwrap_or(false);
    let streams = tokio::task::spawn_blocking(move || {
        remote_source::fetch_series_streams(&imdb_id, season, episode, &base_url, refresh)
    })
    .await
    .map_err(|e| e.to_string())??;

    let grouped = remote_source::group_streams(streams);
    let response: Vec<GroupedStreamsResponse> = grouped
        .into_iter()
        .map(|g| GroupedStreamsResponse {
            quality: g.quality,
            streams: g
                .streams
                .into_iter()
                .map(|s| RemoteStreamResponse {
                    name: s.name,
                    description: s.description,
                    url: s.url,
                    video_size: s.video_size,
                    not_web_ready: s.not_web_ready,
                    parsed_quality: s.parsed_quality,
                    parsed_source: s.parsed_source,
                    recommended: s.recommended,
                    is_hubdrive: s.is_hubdrive,
                })
                .collect(),
        })
        .collect();

    Ok(response)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SeasonStreamsEpisodeResponse {
    episode: i32,
    grouped_streams: Vec<GroupedStreamsResponse>,
}

#[tauri::command]
async fn remote_get_season_streams(
    state: State<'_, AppState>,
    imdb_id: String,
    season: i32,
    force_refresh: Option<bool>,
) -> Result<Vec<SeasonStreamsEpisodeResponse>, String> {
    let base_url = {
        let config = state.config.lock().map_err(|e| e.to_string())?;
        get_active_addon_url_from_config(&config)?
    };

    let refresh = force_refresh.unwrap_or(false);
    let ep_map = tokio::task::spawn_blocking(move || {
        remote_source::fetch_season_streams(&imdb_id, season, &base_url, refresh)
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut result: Vec<SeasonStreamsEpisodeResponse> = ep_map
        .into_iter()
        .map(|(ep, streams)| {
            let grouped = remote_source::group_streams(streams);
            SeasonStreamsEpisodeResponse {
                episode: ep,
                grouped_streams: grouped
                    .into_iter()
                    .map(|g| GroupedStreamsResponse {
                        quality: g.quality,
                        streams: g
                            .streams
                            .into_iter()
                            .map(|s| RemoteStreamResponse {
                                name: s.name,
                                description: s.description,
                                url: s.url,
                                video_size: s.video_size,
                                not_web_ready: s.not_web_ready,
                                parsed_quality: s.parsed_quality,
                                parsed_source: s.parsed_source,
                                recommended: s.recommended,
                                is_hubdrive: s.is_hubdrive,
                            })
                            .collect(),
                    })
                    .collect(),
            }
        })
        .collect();

    result.sort_by(|a, b| a.episode.cmp(&b.episode));
    Ok(result)
}

#[derive(Serialize)]
struct RemotePlaybackResponse {
    media_id: i64,
    has_resume: bool,
    position: f64,
    duration: f64,
    progress_percent: f64,
}

#[tauri::command]
async fn remote_play_with_mpv(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    url: String,
    title: String,
    _video_size: i64,
    _media_identifier: String,
    quality_label: String,
    // Netflix-style metadata
    media_type: String,
    tmdb_id: i64,
    _season_number: Option<i32>,
    _episode_number: Option<i32>,
    _episode_title: Option<String>,
    _poster_path: Option<String>,
    _still_path: Option<String>,
    _overview: Option<String>,
    _year: Option<i32>,
    _start_position: f64,
) -> Result<RemotePlaybackResponse, String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };

    let mpv_path = config
        .mpv_path
        .as_ref()
        .ok_or_else(|| "MPV path not set".to_string())?;

    if mpv_path.is_empty() || !std::path::Path::new(mpv_path).exists() {
        return Err("MPV path not set or invalid".to_string());
    }

    println!(
        "[REMOTE-MPV] Playing '{}' (quality: {}, type: {}, tmdb: {})",
        title, quality_label, media_type, tmdb_id
    );

    // ponytail: external sources are HTTP 200 (no range/seek), so no resume, no tracking, no DB, no cache
    // Just fire MPV and emit ended event when it exits

    // Launch MPV with the URL directly — no Lua script, no progress tracking
    let mut cmd = std::process::Command::new(mpv_path);
    cmd.arg(format!("--force-media-title={}", title));
    cmd.arg("--cache=yes");
    cmd.arg("--demuxer-max-bytes=500MiB");
    cmd.arg("--network-timeout=30");
    cmd.arg("--save-position-on-quit=no");
    cmd.arg("--keep-open=no");
    cmd.arg("--");
    cmd.arg(&url);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to launch MPV: {}", e))?;

    println!(
        "[REMOTE-MPV] Launched MPV (PID: {}) for '{}'",
        child.id(),
        title
    );

    // Background thread: wait for MPV to exit, emit ended event
    let title_clone = title.clone();
    let app_clone = app_handle.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        println!("[REMOTE-MPV] Playback ended for '{}'", title_clone);
        let _ = app_clone.emit(
            "mpv-playback-ended",
            serde_json::json!({
                "completed": false,
                "title": title_clone,
            }),
        );
    });

    Ok(RemotePlaybackResponse {
        media_id: 0,
        has_resume: false,
        position: 0.0,
        duration: 0.0,
        progress_percent: 0.0,
    })
}

#[tauri::command]
async fn remote_clear_progress(state: State<'_, AppState>, media_id: i64) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.clear_progress(media_id).map_err(|e| e.to_string())
}

/// Check resume info for a remote media item BEFORE launching playback.
/// Looks up resume progress from the remote_playback_progress table.
#[tauri::command]
async fn remote_get_resume_info(
    state: State<'_, AppState>,
    tmdb_id: i64,
    media_type: String,
    season_number: Option<i32>,
    episode_number: Option<i32>,
) -> Result<RemotePlaybackResponse, String> {
    let tmdb_str = tmdb_id.to_string();
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let progress = db
        .get_progress_for_item(&tmdb_str, &media_type, season_number, episode_number)
        .map_err(|e| e.to_string())?;
    match progress {
        Some(p) if p.duration_seconds > 0.0 => {
            let percent = (p.resume_position_seconds / p.duration_seconds * 100.0).min(100.0);
            Ok(RemotePlaybackResponse {
                media_id: 0,
                has_resume: true,
                position: p.resume_position_seconds,
                duration: p.duration_seconds,
                progress_percent: percent,
            })
        }
        _ => Ok(RemotePlaybackResponse {
            media_id: 0,
            has_resume: false,
            position: 0.0,
            duration: 0.0,
            progress_percent: 0.0,
        }),
    }
}

#[tauri::command]
async fn remote_start_cache(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    url: String,
    cache_key: String,
    total_bytes: i64,
    title: String,
) -> Result<(), String> {
    let config = {
        let c = state.config.lock().map_err(|e| e.to_string())?;
        c.clone()
    };

    state
        .cache_manager
        .start(app_handle, config, url, cache_key, total_bytes, title)
}

#[tauri::command]
async fn remote_stop_cache(state: State<'_, AppState>, cache_key: String) -> Result<(), String> {
    state.cache_manager.stop(&cache_key)
}

#[tauri::command]
async fn remote_get_cache_status(
    state: State<'_, AppState>,
    cache_key: String,
) -> Result<Option<stream_cache::CacheStatus>, String> {
    Ok(state.cache_manager.status(&cache_key))
}

#[tauri::command]
async fn remote_get_all_cache_status(
    state: State<'_, AppState>,
) -> Result<Vec<stream_cache::CacheStatus>, String> {
    Ok(state.cache_manager.all_status())
}

#[tauri::command]
async fn remote_cleanup_cache(state: State<'_, AppState>, cache_key: String) -> Result<(), String> {
    state.cache_manager.cleanup(&cache_key)
}

#[tauri::command]
async fn remote_cleanup_all_cache(state: State<'_, AppState>) -> Result<(), String> {
    state.cache_manager.cleanup_all()
}

#[tauri::command]
async fn remote_get_library(
    state: State<'_, AppState>,
) -> Result<Vec<crate::database::RemoteBookmark>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_remote_bookmarks().map_err(|e| e.to_string())
}

#[tauri::command]
async fn remote_remove_from_library(
    state: State<'_, AppState>,
    tmdb_id: String,
    media_type: String,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.remove_remote_bookmark(&tmdb_id, &media_type)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn remote_add_to_library(
    state: State<'_, AppState>,
    tmdb_id: String,
    title: String,
    media_type: String,
    year: Option<i32>,
    poster_path: Option<String>,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.add_remote_bookmark(&tmdb_id, &media_type, &title, year, poster_path.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn remote_is_bookmarked(
    state: State<'_, AppState>,
    tmdb_id: String,
    media_type: String,
) -> Result<bool, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.is_remote_bookmarked(&tmdb_id, &media_type)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn remote_update_progress(
    state: State<'_, AppState>,
    tmdb_id: String,
    media_type: String,
    season_number: Option<i32>,
    episode_number: Option<i32>,
    resume_position_seconds: f64,
    duration_seconds: f64,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.upsert_playback_progress(
        &tmdb_id,
        &media_type,
        season_number,
        episode_number,
        resume_position_seconds,
        duration_seconds,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn remote_get_show_progress(
    state: State<'_, AppState>,
    tmdb_id: String,
) -> Result<Vec<crate::database::RemotePlaybackProgress>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_all_progress_for_show(&tmdb_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn remote_get_all_progress(
    state: State<'_, AppState>,
) -> Result<Vec<crate::database::RemotePlaybackProgress>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db.get_all_playback_progress().map_err(|e| e.to_string())
}

#[tauri::command]
async fn remote_is_cache_dir_set(state: State<'_, AppState>) -> Result<bool, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(stream_cache::CacheManager::is_cache_dir_set(&config))
}

#[tauri::command]
async fn remote_get_cache_dir(state: State<'_, AppState>) -> Result<String, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    let dir = stream_cache::CacheManager::cache_dir(&config);
    Ok(dir.to_string_lossy().to_string())
}

/// Resolve the active addon URL from addon_sources (default source first, then first enabled).
/// Falls back to legacy addon_url if no sources configured.
fn get_active_addon_url_from_config(config: &config::Config) -> Result<String, String> {
    // Try addon_sources first
    if !config.addon_sources.is_empty() {
        // Find default source
        if let Some(src) = config
            .addon_sources
            .iter()
            .find(|s| s.is_default && s.enabled)
        {
            return Ok(src.url.clone());
        }
        // Fall back to first enabled source
        if let Some(src) = config.addon_sources.iter().find(|s| s.enabled) {
            return Ok(src.url.clone());
        }
        return Err("All addon sources are disabled. Enable at least one source.".to_string());
    }
    // Legacy fallback
    config
        .addon_url
        .as_ref()
        .filter(|u| !u.is_empty())
        .cloned()
        .ok_or_else(|| {
            "No addon URL configured. Please add your addon URL in Settings > External.".to_string()
        })
}

#[tauri::command]
fn get_addon_url(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config.addon_url.clone())
}

/// Validate an addon URL against SSRF attacks.
/// - Only allows http:// and https:// schemes
/// - Blocks private/internal IP ranges (10.x, 172.16-31.x, 192.168.x, 127.x, 0.0.0.0)
/// - Blocks localhost and IPv6 loopback (::1)
fn validate_addon_url(url_str: &str) -> Result<(), String> {
    let url_str = url_str.trim();
    if url_str.is_empty() {
        return Err("Addon URL cannot be empty".to_string());
    }

    let url = url::Url::parse(url_str).map_err(|e| format!("Invalid URL: {}", e))?;

    // Only allow http and https schemes
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "Scheme '{}' is not allowed. Only http:// and https:// are permitted.",
                other
            ));
        }
    }

    // Check hostname
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no hostname".to_string())?;

    let host_lower = host.to_lowercase();

    // Allow localhost, loopback, and private IPs for local addon servers
    // Only block truly dangerous patterns
    if host_lower == "localhost.localdomain" || host_lower.ends_with(".localhost") {
        return Err("Invalid hostname.".to_string());
    }

    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        if ip.is_unspecified() {
            return Err("0.0.0.0 is not allowed.".to_string());
        }
        // Block link-local (169.254.x.x)
        if ip.is_link_local() {
            return Err("Link-local addresses are not allowed.".to_string());
        }
    }

    // Block IPv6 private/loopback
    if let Ok(ip) = host
        .trim_matches(|c| c == '[' || c == ']')
        .parse::<std::net::Ipv6Addr>()
    {
        if ip.is_loopback() {
            return Err("Loopback IPv6 address is not allowed.".to_string());
        }
        if segments_are_private_ipv6(ip) {
            return Err("Private IPv6 addresses are not allowed.".to_string());
        }
    }

    Ok(())
}

/// Check if an IPv6 address is in a private range (fc00::/7, fe80::/10)
fn segments_are_private_ipv6(ip: std::net::Ipv6Addr) -> bool {
    let segments = ip.segments();
    // fc00::/7 — unique local addresses
    if (segments[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // fe80::/10 — link-local
    if (segments[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    false
}

#[tauri::command]
fn set_addon_url(state: State<'_, AppState>, url: String) -> Result<(), String> {
    validate_addon_url(&url)?;
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    config.addon_url = Some(url.trim().to_string());
    config::save_config(&config).map_err(|e| e.to_string())?;
    Ok(())
}

/// Get all configured addon sources
#[tauri::command]
fn get_addon_sources(state: State<'_, AppState>) -> Result<Vec<config::AddonSource>, String> {
    let config = state.config.lock().map_err(|e| e.to_string())?;
    Ok(config.addon_sources.clone())
}

/// Add a new addon source
#[tauri::command]
fn add_addon_source(
    state: State<'_, AppState>,
    name: String,
    url: String,
    binary_path: Option<String>,
) -> Result<config::AddonSource, String> {
    validate_addon_url(&url)?;
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    // Dedup: prevent adding a source with the same URL
    if config.addon_sources.iter().any(|s| s.url == url.trim()) {
        return Err("A source with this URL already exists".to_string());
    }
    let source = config::AddonSource {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.trim().to_string(),
        url: url.trim().to_string(),
        enabled: true,
        is_default: config.addon_sources.is_empty(), // first source becomes default
        binary_path,
    };
    config.addon_sources.push(source.clone());
    // Also set legacy addon_url for backward compat with remote_source
    if source.is_default {
        config.addon_url = Some(source.url.clone());
    }
    config::save_config(&config).map_err(|e| e.to_string())?;

    Ok(source)
}

/// Remove an addon source by ID
#[tauri::command]
fn remove_addon_source(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let source = config.addon_sources.iter().find(|s| s.id == id);
    let has_binary = source.map(|s| s.binary_path.is_some()).unwrap_or(false);
    let was_default = source.map(|s| s.is_default).unwrap_or(false);
    config.addon_sources.retain(|s| s.id != id);
    // If this was a binary source, kill the running process and watchdog
    if has_binary {
        if let Ok(mut proc) = RUNNING_ADDON_PROCESS.lock() {
            if let Some(mut child) = proc.take() {
                let _ = child.kill();
            }
        }
        if ADDON_WATCHDOG_RUNNING
            .lock()
            .map(|mut g| {
                *g = false;
            })
            .is_err()
        {
            eprintln!("[remove_addon_source] Watchdog lock poisoned");
        }
    }
    // If we removed the default, promote the first remaining source
    if was_default {
        if let Some(first) = config.addon_sources.first_mut() {
            first.is_default = true;
            config.addon_url = Some(first.url.clone());
        } else {
            config.addon_url = None;
        }
    }
    config::save_config(&config).map_err(|e| e.to_string())?;
    Ok(())
}

/// Install a custom addon binary (e.g. Go binary). Copies the file to the app data directory
/// and creates an AddonSource with binary_path set.
#[tauri::command]
async fn install_addon_binary(
    state: State<'_, AppState>,
    file_path: String,
    name: Option<String>,
) -> Result<config::AddonSource, String> {
    let src = std::path::Path::new(&file_path);
    if !src.exists() {
        return Err("File does not exist".to_string());
    }
    // Validate size (< 50MB)
    let metadata = std::fs::metadata(src).map_err(|e| format!("Cannot read file: {}", e))?;
    if metadata.len() > 50 * 1024 * 1024 {
        return Err("File too large (max 50MB)".to_string());
    }
    // Validate extension
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    if cfg!(target_os = "windows") && ext != "exe" {
        return Err("On WebviewWindows, the binary must be an .exe file".to_string());
    }

    // Validate binary by running --version
    let mut version_cmd = tokio::process::Command::new(src);
    version_cmd.arg("--version");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        version_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = version_cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run binary: {}", e))?;
    if !output.status.success() {
        return Err("Invalid addon binary (failed --version check). Please drop a valid vault-addon .exe file.".to_string());
    }

    // Find a free port
    let port = find_free_port().await?;
    let url = format!("http://127.0.0.1:{}", port);

    let app_dir = database::get_app_data_dir();
    let bin_dir = app_dir.join("addon-binaries");
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("Cannot create bin dir: {}", e))?;

    let dest_name = if cfg!(target_os = "windows") {
        format!(
            "addon-proxy-{}.exe",
            uuid::Uuid::new_v4().to_string()[..8].to_string()
        )
    } else {
        format!(
            "addon-proxy-{}",
            uuid::Uuid::new_v4().to_string()[..8].to_string()
        )
    };
    let dest = bin_dir.join(&dest_name);
    std::fs::copy(src, &dest).map_err(|e| format!("Cannot copy binary: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&dest)
            .map_err(|e| format!("Cannot inspect copied binary: {}", e))?
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&dest, permissions)
            .map_err(|e| format!("Cannot make addon executable: {}", e))?;
    }

    let dest_path = dest.to_string_lossy().to_string();
    let source_name = name.unwrap_or_else(|| "Custom Addon Binary".to_string());

    let source = config::AddonSource {
        id: uuid::Uuid::new_v4().to_string(),
        name: source_name,
        url: url.clone(),
        enabled: true,
        is_default: false,
        binary_path: Some(dest_path.clone()),
    };

    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    if config.addon_sources.is_empty() {
        // First source becomes default
        let mut src = source.clone();
        src.is_default = true;
        config.addon_url = Some(src.url.clone());
        config.addon_sources.push(src);
    } else {
        config.addon_sources.push(source.clone());
    }
    config::save_config(&config).map_err(|e| e.to_string())?;

    // Spawn the binary immediately with --yes (auto-accept disclaimer) and dynamic port
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            match start_addon_process(
                &dest_path,
                &["--yes".to_string(), "--port".to_string(), port.to_string()],
            )
            .await
            {
                Ok(_) => println!("[addon] Binary started after install on port {}", port),
                Err(e) => eprintln!("[addon] Failed to start binary after install: {}", e),
            }
        });
    });

    Ok(source)
}

/// Remove a custom addon binary file and clear its binary_path from config.
#[tauri::command]
fn remove_addon_binary(state: State<'_, AppState>, source_id: String) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    if let Some(source) = config.addon_sources.iter().find(|s| s.id == source_id) {
        if let Some(ref bp) = source.binary_path {
            let _ = std::fs::remove_file(bp);
        }
    }
    config.addon_sources.retain(|s| s.id != source_id);
    config::save_config(&config).map_err(|e| e.to_string())?;
    Ok(())
}

/// Restart the addon binary: kill any running process, reset crash state, and re-spawn.
#[tauri::command]
async fn restart_addon(state: State<'_, AppState>) -> Result<(), String> {
    // Reset crash state
    if let Ok(mut cnt) = ADDON_RESTART_COUNT.lock() {
        *cnt = 0;
    }
    if let Ok(mut guard) = ADDON_WATCHDOG_RUNNING.lock() {
        *guard = false;
    }

    // Kill existing process
    if let Ok(mut proc) = RUNNING_ADDON_PROCESS.lock() {
        if let Some(mut child) = proc.take() {
            let _ = child.kill();
        }
    }

    // Find the default binary source
    let config = state.config.lock().map_err(|e| e.to_string())?;
    let source = config
        .addon_sources
        .iter()
        .find(|s| s.enabled && s.binary_path.is_some() && s.is_default)
        .or_else(|| {
            config
                .addon_sources
                .iter()
                .find(|s| s.enabled && s.binary_path.is_some())
        })
        .ok_or("No binary addon source found")?;
    let bp = source.binary_path.clone().unwrap();
    let port = source
        .url
        .split(':')
        .last()
        .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
        .unwrap_or(51546);
    let args: Vec<String> = vec!["--yes".to_string(), "--port".to_string(), port.to_string()];
    drop(config);

    // Spawn in background thread
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            match start_addon_process(&bp, &args).await {
                Ok(_) => println!("[addon] Restart successful"),
                Err(e) => eprintln!("[addon] Restart failed: {}", e),
            }
        });
    });

    Ok(())
}

/// Find a free TCP port on localhost
async fn find_free_port() -> Result<u16, String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Cannot bind to free port: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Cannot get port: {}", e))?
        .port();
    drop(listener);
    Ok(port)
}

/// Fetch addon version from its /version endpoint
async fn get_addon_version_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/');
    let version_url = format!("{}/version", trimmed);
    let is_loopback = trimmed.contains("127.0.0.1") || trimmed.contains("localhost");

    if is_loopback {
        // Use raw TCP for loopback to bypass reqwest loopback restrictions
        match http_client::local_http_get_raw(&version_url) {
            Ok((status, mut reader)) if (200..300).contains(&status) => {
                use std::io::Read;
                let mut body = String::new();
                let _ = reader.read_to_string(&mut body);
                serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| {
                        v.get("version")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
            }
            _ => None,
        }
    } else {
        let client = crate::http_client::shared_client();
        match client
            .get(&version_url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
        {
            Ok(resp) if resp.status().is_success() => {
                resp.json::<serde_json::Value>().ok().and_then(|v| {
                    v.get("version")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
            }
            _ => None,
        }
    }
}

/// Validate that an addon URL is reachable (server responds to /manifest.json)
/// Uses raw TCP for loopback addresses to bypass reqwest loopback restrictions.
#[tauri::command]
fn check_addon_server(url: String) -> bool {
    let trimmed = url.trim_end_matches('/').to_string();
    let is_loopback = trimmed.contains("127.0.0.1") || trimmed.contains("localhost");
    if is_loopback {
        match http_client::local_http_get_raw(&format!("{}/manifest.json", trimmed)) {
            Ok((status, _body)) => (200..300).contains(&status),
            Err(_) => false,
        }
    } else {
        let client = crate::http_client::shared_client();
        match client
            .get(format!("{}/manifest.json", trimmed))
            .timeout(std::time::Duration::from_secs(3))
            .send()
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }
}

/// Get the addon log history (last 200 lines)
#[tauri::command]
fn get_addon_logs() -> Vec<String> {
    ADDON_LOG_HISTORY
        .lock()
        .map(|h| h.clone())
        .unwrap_or_default()
}

/// Get addon version from its /version endpoint
#[tauri::command]
async fn get_addon_version(url: String) -> Option<String> {
    get_addon_version_from_url(&url).await
}

/// Set a source as the active default
#[tauri::command]
fn set_active_source(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let mut found = false;
    let mut active_url = None;
    for source in &mut config.addon_sources {
        if source.id == id {
            source.is_default = true;
            active_url = Some(source.url.clone());
            found = true;
        } else {
            source.is_default = false;
        }
    }
    if !found {
        return Err("Source not found".to_string());
    }
    if let Some(url) = active_url {
        config.addon_url = Some(url);
    }
    config::save_config(&config).map_err(|e| e.to_string())?;
    Ok(())
}

/// Enable/disable an addon source
#[tauri::command]
fn toggle_addon_source(
    state: State<'_, AppState>,
    source_id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(|e| e.to_string())?;
    let src = config
        .addon_sources
        .iter_mut()
        .find(|s| s.id == source_id)
        .ok_or("Source not found")?;
    src.enabled = enabled;
    // Keep legacy addon_url in sync with the default source
    config.addon_url = config
        .addon_sources
        .iter()
        .find(|s| s.is_default && s.enabled)
        .map(|s| s.url.clone());
    config::save_config(&config).map_err(|e| e.to_string())?;
    Ok(())
}

/// Auto-detect a running addon server on common ports via TCP connect.
#[tauri::command]
async fn auto_setup_addon() -> Result<Option<config::AddonSource>, String> {
    let ports = [51546, 3255, 8080, 12345, 4000, 5000, 7000, 9000];

    for port in &ports {
        let addr = format!("127.0.0.1:{}", port);
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return Ok(Some(config::AddonSource {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Local Addon".to_string(),
                url: format!("http://127.0.0.1:{}", port),
                enabled: true,
                is_default: true,
                binary_path: None,
            }));
        }
    }

    Ok(None)
}

/// Start an addon binary process and store it globally.
/// Spawns the binary directly with CREATE_NO_WINDOW.
async fn spawn_addon_child(binary_path: &str, args: &[String]) -> Result<(), String> {
    println!("[addon] Starting binary: {} {:?} ...", binary_path, args);
    let mut cmd = tokio::process::Command::new(binary_path);
    cmd.args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start binary '{}': {}", binary_path, e))?;

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Log stdout + emit to frontend
    if let Some(stdout) = stdout {
        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(stdout);
        let mut line = String::new();
        tokio::spawn(async move {
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let trimmed = line.trim().to_string();
                        println!("[addon] stdout: {}", trimmed);
                        // Store in log history
                        if let Ok(mut hist) = ADDON_LOG_HISTORY.lock() {
                            hist.push(format!("[stdout] {}", trimmed));
                            if hist.len() > 200 {
                                hist.remove(0);
                            }
                        }
                        // Emit to frontend
                        if let Ok(h) = GLOBAL_APP_HANDLE.lock() {
                            if let Some(ref handle) = *h {
                                let _ = handle.emit("addon-log", &trimmed);
                            }
                        }
                    }
                }
            }
        });
    }
    // Log stderr + emit to frontend
    if let Some(stderr) = stderr {
        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(stderr);
        let mut line = String::new();
        tokio::spawn(async move {
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let trimmed = line.trim().to_string();
                        println!("[addon] stderr: {}", trimmed);
                        // Store in log history
                        if let Ok(mut hist) = ADDON_LOG_HISTORY.lock() {
                            hist.push(format!("[stderr] {}", trimmed));
                            if hist.len() > 200 {
                                hist.remove(0);
                            }
                        }
                        // Emit to frontend
                        if let Ok(h) = GLOBAL_APP_HANDLE.lock() {
                            if let Some(ref handle) = *h {
                                let _ = handle.emit("addon-log", &trimmed);
                            }
                        }
                    }
                }
            }
        });
    }
    if let Ok(mut proc) = RUNNING_ADDON_PROCESS.lock() {
        *proc = Some(child);
    }
    std::mem::forget(stdin);
    println!("[addon] Binary process started");
    Ok(())
}

async fn start_addon_process(binary_path: &str, args: &[String]) -> Result<(), String> {
    spawn_addon_child(binary_path, args).await?;

    // Watchdog: restart addon if it dies (only one watchdog thread)
    let bp_owned = binary_path.to_string();
    let args_owned = args.to_vec();
    let should_spawn_watchdog = ADDON_WATCHDOG_RUNNING
        .lock()
        .map(|mut guard| {
            if *guard {
                false
            } else {
                *guard = true;
                true
            }
        })
        .unwrap_or_else(|e| {
            eprintln!("[addon] Watchdog lock poisoned: {}", e);
            true
        });
    if should_spawn_watchdog {
        // Extract port from args for health checks (args contain "--port", "NNNNN")
        let addon_port: u16 = args_owned.windows(2)
            .find(|w| w[0] == "--port")
            .and_then(|w| w[1].parse().ok())
            .unwrap_or_else(|| {
                eprintln!("[addon] WARNING: --port not in args, defaulting to 51546. Health checks may probe the wrong port.");
                51546
            });

        std::thread::spawn(move || {
            const MAX_RESTARTS: u32 = 10;
            const BACKOFF_BASE: u64 = 2; // seconds
            loop {
                std::thread::sleep(std::time::Duration::from_secs(5));
                let needs_restart = {
                    if let Ok(mut proc) = RUNNING_ADDON_PROCESS.lock() {
                        match proc.as_mut().map(|c| c.try_wait()) {
                            Some(Ok(Some(status))) => {
                                println!(
                                    "[addon] Process exited with {}, checking server...",
                                    status
                                );
                                *proc = None;
                                true
                            }
                            Some(Ok(None)) => {
                                // Healthy — reset restart counter
                                if let Ok(mut cnt) = ADDON_RESTART_COUNT.lock() {
                                    *cnt = 0;
                                }
                                false
                            }
                            Some(Err(e)) => {
                                // try_wait() error — DO NOT blindly restart.
                                // Check if the addon server is actually responding via TCP.
                                println!(
                                    "[addon] try_wait error: {}, checking if server is alive...",
                                    e
                                );
                                let addr = format!("127.0.0.1:{}", addon_port);
                                if std::net::TcpStream::connect_timeout(
                                    &addr
                                        .parse()
                                        .unwrap_or_else(|_| "127.0.0.1:0".parse().unwrap()),
                                    std::time::Duration::from_secs(2),
                                )
                                .is_ok()
                                {
                                    // Server is still responding — don't restart, just reset counter
                                    println!(
                                        "[addon] Server is alive on port {}, skipping restart",
                                        addon_port
                                    );
                                    if let Ok(mut cnt) = ADDON_RESTART_COUNT.lock() {
                                        *cnt = 0;
                                    }
                                    false
                                } else {
                                    println!("[addon] Server not responding, will restart...");
                                    true
                                }
                            }
                            None => {
                                // No process handle stored
                                let addr = format!("127.0.0.1:{}", addon_port);
                                if std::net::TcpStream::connect_timeout(
                                    &addr
                                        .parse()
                                        .unwrap_or_else(|_| "127.0.0.1:0".parse().unwrap()),
                                    std::time::Duration::from_secs(2),
                                )
                                .is_ok()
                                {
                                    println!("[addon] No handle but server alive on port {}, skipping restart", addon_port);
                                    if let Ok(mut cnt) = ADDON_RESTART_COUNT.lock() {
                                        *cnt = 0;
                                    }
                                    false
                                } else {
                                    println!("[addon] No process found and server not responding, will restart...");
                                    true
                                }
                            }
                        }
                    } else {
                        false
                    }
                };
                if needs_restart {
                    let restart_count = {
                        let mut cnt = ADDON_RESTART_COUNT
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        *cnt += 1;
                        *cnt
                    };
                    if restart_count > MAX_RESTARTS {
                        println!("[addon] Max restarts ({}) exceeded. Giving up. Restart the app to retry.", MAX_RESTARTS);
                        if let Ok(h) = GLOBAL_APP_HANDLE.lock() {
                            if let Some(ref handle) = *h {
                                let _ = handle.emit("addon-log", &"[FATAL] Addon crashed too many times. Please restart the app.");
                                let _ = handle.emit("addon-crashed", &restart_count);
                            }
                        }
                        break;
                    }
                    // Kill any zombie process that might be holding the port
                    if let Ok(mut proc) = RUNNING_ADDON_PROCESS.lock() {
                        if let Some(mut child) = proc.take() {
                            let _ = child.kill();
                        }
                    }
                    // Exponential backoff: 2s, 4s, 8s, 16s, ... capped at 60s
                    let delay = std::cmp::min(BACKOFF_BASE.pow(restart_count), 60);
                    println!(
                        "[addon] Restart attempt {}/{} in {}s...",
                        restart_count, MAX_RESTARTS, delay
                    );
                    std::thread::sleep(std::time::Duration::from_secs(delay));
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    match rt.block_on(spawn_addon_child(&bp_owned, &args_owned)) {
                        Ok(_) => println!("[addon] Restarted successfully"),
                        Err(e) => println!("[addon] Restart failed: {}", e),
                    }
                }
            }
        });
    }

    Ok(())
}

#[tauri::command]
fn remote_clear_streams_cache() -> Result<(), String> {
    // No-op: stream cache removed — addon handles caching now
    Ok(())
}

#[tauri::command]
async fn run_sync_validation(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<database::SyncValidationReport, String> {
    // Acquire scan lock — validation cannot run during a scan
    let _lock = crate::ScanLock::try_acquire(&state.is_scanning).ok_or_else(|| {
        "Cannot validate during an active scan. Wait for scan to complete.".to_string()
    })?;

    let total_steps: u8 = 5;
    let mut report = database::SyncValidationReport {
        ghost_entries: Vec::new(),
        missing_files: Vec::new(),
        failed_indexings: Vec::new(),
        orphaned_zip_entries: Vec::new(),
        stale_token: Vec::new(),
        total_issues: 0,
    };

    // --- Check 1: Ghost entries ---
    let _ = app.emit(
        "sync-validation-progress",
        serde_json::json!({
            "step": 1, "total": total_steps, "category": "ghost", "status": "checking"
        }),
    );

    let ghost_result = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db.get_all_cloud_media_ids().map_err(|e| e.to_string())?
    };

    if !ghost_result.is_empty() {
        let file_ids: Vec<String> = ghost_result.iter().map(|(fid, _, _)| fid.clone()).collect();
        match state.gdrive_client.batch_check_file_exists(&file_ids).await {
            Ok(existing) => {
                for (fid, title, _db_id) in &ghost_result {
                    if !existing.contains(fid) {
                        report.ghost_entries.push(database::SyncIssue {
                            category: "ghost".to_string(),
                            file_name: title.clone(),
                            file_id: Some(fid.clone()),
                            reason: "File was deleted or moved on Google Drive".to_string(),
                            fixable: true,
                            fix_action: "remove".to_string(),
                        });
                    }
                }
            }
            Err(_) => {
                report.ghost_entries.push(database::SyncIssue {
                    category: "ghost".to_string(),
                    file_name: "Google Drive API".to_string(),
                    file_id: None,
                    reason: "Drive API unavailable — ghost check skipped".to_string(),
                    fixable: false,
                    fix_action: "none".to_string(),
                });
            }
        }
    }

    let _ = app.emit(
        "sync-validation-result",
        serde_json::json!({
            "category": "ghost",
            "issues": report.ghost_entries,
            "count": report.ghost_entries.len()
        }),
    );

    // --- Check 2: Missing files ---
    let _ = app.emit(
        "sync-validation-progress",
        serde_json::json!({
            "step": 2, "total": total_steps, "category": "missing", "status": "checking"
        }),
    );

    let (folders, existing_ids) = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let folders = db.get_cloud_folders().map_err(|e| e.to_string())?;
        let cloud_ids = db.get_all_cloud_media_ids().map_err(|e| e.to_string())?;
        let ids: std::collections::HashSet<String> =
            cloud_ids.into_iter().map(|(fid, _, _)| fid).collect();
        (folders, ids)
    };

    for (folder_id, folder_name, _) in &folders {
        match state.gdrive_client.list_video_files(folder_id, false).await {
            Ok(files) => {
                for file in files {
                    if !existing_ids.contains(&file.id) {
                        report.missing_files.push(database::SyncIssue {
                            category: "missing".to_string(),
                            file_name: file.name.clone(),
                            file_id: Some(file.id.clone()),
                            reason: "File exists on Drive but is not in the library".to_string(),
                            fixable: true,
                            fix_action: "reindex".to_string(),
                        });
                    }
                }
            }
            Err(_) => {
                report.missing_files.push(database::SyncIssue {
                    category: "missing".to_string(),
                    file_name: folder_name.clone(),
                    file_id: None,
                    reason: format!("Could not list folder '{}' — Drive API error", folder_name),
                    fixable: false,
                    fix_action: "none".to_string(),
                });
            }
        }
    }

    let _ = app.emit(
        "sync-validation-result",
        serde_json::json!({
            "category": "missing",
            "issues": report.missing_files,
            "count": report.missing_files.len()
        }),
    );

    // --- Check 3: Failed indexings ---
    let _ = app.emit(
        "sync-validation-progress",
        serde_json::json!({
            "step": 3, "total": total_steps, "category": "failed", "status": "checking"
        }),
    );

    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let failures = db
            .get_cloud_index_failures(1000)
            .map_err(|e| e.to_string())?;
        for f in failures {
            report.failed_indexings.push(database::SyncIssue {
                category: "failed".to_string(),
                file_name: f.file_name,
                file_id: Some(f.cloud_file_id),
                reason: f.last_error,
                fixable: true,
                fix_action: "retry".to_string(),
            });
        }
    }

    let _ = app.emit(
        "sync-validation-result",
        serde_json::json!({
            "category": "failed",
            "issues": report.failed_indexings,
            "count": report.failed_indexings.len()
        }),
    );

    // --- Check 4: Orphaned ZIP entries ---
    let _ = app.emit(
        "sync-validation-progress",
        serde_json::json!({
            "step": 4, "total": total_steps, "category": "orphaned_zip", "status": "checking"
        }),
    );

    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let orphans = db.get_orphaned_zip_entries().map_err(|e| e.to_string())?;
        for (id, title, entry_path) in orphans {
            report.orphaned_zip_entries.push(database::SyncIssue {
                category: "orphaned_zip".to_string(),
                file_name: format!("{}:{}", title, entry_path),
                file_id: Some(id.to_string()),
                reason: "Parent ZIP archive was removed but child entry remains".to_string(),
                fixable: true,
                fix_action: "remove".to_string(),
            });
        }
    }

    let _ = app.emit(
        "sync-validation-result",
        serde_json::json!({
            "category": "orphaned_zip",
            "issues": report.orphaned_zip_entries,
            "count": report.orphaned_zip_entries.len()
        }),
    );

    // --- Check 5: Stale changes token ---
    let _ = app.emit(
        "sync-validation-progress",
        serde_json::json!({
            "step": 5, "total": total_steps, "category": "stale_token", "status": "checking"
        }),
    );

    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        let (has_token, _token) = db.get_changes_token_status().map_err(|e| e.to_string())?;
        if !has_token {
            report.stale_token.push(database::SyncIssue {
                category: "stale_token".to_string(),
                file_name: "gdrive_changes_token".to_string(),
                file_id: None,
                reason: "No changes token found — next scan will be a full re-scan".to_string(),
                fixable: true,
                fix_action: "refresh_token".to_string(),
            });
        }
    }

    let _ = app.emit(
        "sync-validation-result",
        serde_json::json!({
            "category": "stale_token",
            "issues": report.stale_token,
            "count": report.stale_token.len()
        }),
    );

    // --- Complete ---
    report.total_issues = report.ghost_entries.len()
        + report.missing_files.len()
        + report.failed_indexings.len()
        + report.orphaned_zip_entries.len()
        + report.stale_token.len();

    let _ = app.emit(
        "sync-validation-complete",
        serde_json::json!({
            "total_issues": report.total_issues,
            "report": report
        }),
    );

    // Store in AppState for fix_sync_issues to reference
    if let Ok(mut last_report) = state.last_validation_report.lock() {
        *last_report = Some(report.clone());
    }

    Ok(report)
}

#[tauri::command]
async fn fix_sync_issues(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    category: String,
    file_ids: Vec<String>,
) -> Result<serde_json::Value, String> {
    // Validate that we have a recent report — clone to release the lock
    let report = {
        let guard = state
            .last_validation_report
            .lock()
            .map_err(|e| e.to_string())?;
        guard
            .clone()
            .ok_or("No validation report available. Run sync validation first.")?
    };

    // Get the issues for this category to verify file_ids are valid
    let issues: Vec<database::SyncIssue> = match category.as_str() {
        "ghost" => report
            .ghost_entries
            .iter()
            .filter(|i| {
                i.file_id
                    .as_deref()
                    .map(|fid| file_ids.contains(&fid.to_string()))
                    .unwrap_or(false)
            })
            .cloned()
            .collect(),
        "missing" => report
            .missing_files
            .iter()
            .filter(|i| {
                i.file_id
                    .as_deref()
                    .map(|fid| file_ids.contains(&fid.to_string()))
                    .unwrap_or(false)
            })
            .cloned()
            .collect(),
        "failed" => report
            .failed_indexings
            .iter()
            .filter(|i| {
                i.file_id
                    .as_deref()
                    .map(|fid| file_ids.contains(&fid.to_string()))
                    .unwrap_or(false)
            })
            .cloned()
            .collect(),
        "orphaned_zip" => report
            .orphaned_zip_entries
            .iter()
            .filter(|i| {
                i.file_id
                    .as_deref()
                    .map(|fid| file_ids.contains(&fid.to_string()))
                    .unwrap_or(false)
            })
            .cloned()
            .collect(),
        "stale_token" => report.stale_token.clone(),
        _ => return Err(format!("Unknown category: {}", category)),
    };

    let total = issues.len();
    let mut fixed: usize = 0;
    let mut failed: usize = 0;

    for issue in &issues {
        let _ = app.emit(
            "sync-fix-progress",
            serde_json::json!({
                "category": category,
                "current": fixed + failed + 1,
                "total": total
            }),
        );

        match category.as_str() {
            "ghost" | "orphaned_zip" => {
                if let Some(id_str) = &issue.file_id {
                    if let Ok(id) = id_str.parse::<i64>() {
                        let db = state.db.lock().map_err(|e| e.to_string())?;
                        let _ = db.preserve_watch_progress_for_media(id);
                        match db.remove_media_by_id(id) {
                            Ok(_) => fixed += 1,
                            Err(_) => failed += 1,
                        }
                    } else {
                        // For ghosts, file_id is cloud_file_id string — preserve progress then delete
                        let db = state.db.lock().map_err(|e| e.to_string())?;
                        // Preserve watch progress for any media with this cloud_file_id
                        if let Ok(Some((media_id, _, _, _))) =
                            db.get_media_info_by_cloud_file_id(id_str)
                        {
                            let _ = db.preserve_watch_progress_for_media(media_id);
                        }
                        // Also preserve for ZIP children (parent_zip_id = cloud_file_id)
                        let _ = db.preserve_watch_progress_for_zip_children(id_str);
                        match db.remove_media_by_cloud_file_id(id_str) {
                            Ok(_) => fixed += 1,
                            Err(_) => failed += 1,
                        }
                    }
                }
            }
            "failed" => {
                if let Some(fid) = &issue.file_id {
                    let db = state.db.lock().map_err(|e| e.to_string())?;
                    let _ = db.clear_cloud_index_failure(fid);
                    drop(db);
                    fixed += 1;
                }
            }
            "missing" => {
                // Missing files are unindexed Drive files. We can't index individual files
                // without replicating the full scan logic (filename parsing, movie vs TV,
                // TMDB enrichment). Mark as acknowledged — the next background scan
                // (runs every 5s) or manual "Update Library" will index them.
                fixed += 1;
            }
            "stale_token" => {
                // ponytail: check_cloud_changes_inner does not exist yet; skip until it is added
                // When adding, call: match crate::check_cloud_changes_inner(&app, &state).await { ... }
                failed += 1;
                break; // Only one stale_token issue
            }
            _ => {}
        }
    }

    let _ = app.emit(
        "sync-fix-result",
        serde_json::json!({
            "category": category,
            "fixed": fixed,
            "failed": failed
        }),
    );

    Ok(serde_json::json!({
        "category": category,
        "fixed": fixed,
        "failed": failed,
        "total": total
    }))
}

fn build_tray_menu<R: tauri::Runtime>(app: &AppHandle<R>, state: &AppState) -> Result<Menu<R>, tauri::Error> {
    let continue_items = {
        let db_guard = state.db.lock().map_err(|e| tauri::Error::Anyhow(std::io::Error::other(e.to_string()).into()))?;
        db_guard.get_continue_watching_items(3).unwrap_or_default()
    };

    let mut menu = MenuBuilder::new(app)
        .text("show", "Show SlasshyVault")
        .separator();

    for item in continue_items {
        let label = if item.media_type == "tvepisode" {
            let season = item.season_number.unwrap_or(0);
            let episode = item.episode_number.unwrap_or(0);
            let episode_title = item.episode_title.as_deref().unwrap_or("");
            if episode_title.is_empty() {
                format!("{} S{:02}E{:02}", item.title, season, episode)
            } else {
                format!("{} S{:02}E{:02} - {}", item.title, season, episode, episode_title)
            }
        } else {
            item.title.clone()
        };
        let label = if label.len() > 45 {
            format!("{}...", &label[..42])
        } else {
            label
        };
        menu = menu.text(format!("continue_{}", item.id), format!("▶ {}", label));
    }

    menu = menu.separator().text("quit", "Quit");
    menu.build()
}

#[tauri::command]
fn refresh_tray_continue_watching(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let menu = build_tray_menu(&app, &state).map_err(|e| e.to_string())?;
    let tray = app
        .tray_by_id("main")
        .ok_or_else(|| "Tray icon not found".to_string())?;
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())
}

// ==================== SUBTITLE MANAGEMENT ====================

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct SubtitleEntry {
    id: String,
    url: String,
    lang: String,
}

#[tauri::command]
async fn fetch_subtitles(
    imdb_id: String,
    media_type: String,
) -> Result<Vec<SubtitleEntry>, String> {
    let type_str = match media_type.as_str() {
        "movie" => "movie",
        "tvshow" | "tvepisode" => "series",
        _ => return Err("Unsupported media type".into()),
    };

    let clean_id = imdb_id.trim().trim_start_matches("tt");
    let url = format!(
        "https://opensubtitles-v3.strem.io/subtitles/{}/tt{}.json",
        type_str, clean_id
    );

    let result = tokio::task::spawn_blocking(move || {
        let client = http_client::shared_client();
        let response = client
            .get(&url)
            .send()
            .map_err(|e| format!("Failed to fetch subtitles: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Subtitle API returned status {}",
                response.status()
            ));
        }

        let body: serde_json::Value = response
            .json()
            .map_err(|e| format!("Failed to parse subtitle response: {}", e))?;

        let subtitles_raw = body
            .get("subtitles")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let entries: Vec<SubtitleEntry> = subtitles_raw
            .iter()
            .filter_map(|entry| {
                let id = entry.get("id")?.as_str()?.to_string();
                let url = entry.get("url")?.as_str()?.to_string();
                let lang = entry
                    .get("lang")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                Some(SubtitleEntry { id, url, lang })
            })
            .collect();

        Ok(entries)
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?;

    result
}

#[tauri::command]
async fn download_subtitle(url: String, filename: String) -> Result<String, String> {
    let result = tokio::task::spawn_blocking(move || {
        let client = http_client::shared_client();
        let response = client
            .get(&url)
            .send()
            .map_err(|e| format!("Failed to download subtitle: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Subtitle download returned status {}",
                response.status()
            ));
        }

        let bytes = response
            .bytes()
            .map_err(|e| format!("Failed to read subtitle bytes: {}", e))?;

        let temp_dir = std::env::temp_dir().join("slasshyvault_subtitles");
        std::fs::create_dir_all(&temp_dir)
            .map_err(|e| format!("Failed to create subtitle temp dir: {}", e))?;

        // Ensure filename has a subtitle extension
        let safe_filename = if filename.ends_with(".srt")
            || filename.ends_with(".vtt")
            || filename.ends_with(".sub")
            || filename.ends_with(".ass")
        {
            filename
        } else {
            format!("{}.srt", filename)
        };

        let file_path = temp_dir.join(&safe_filename);
        std::fs::write(&file_path, &bytes)
            .map_err(|e| format!("Failed to write subtitle file: {}", e))?;

        Ok(file_path.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?;

    result
}

fn main() {
    // Set WebviewWindows AppUserModelID so the volume mixer shows "SlasshyVault" instead of "WebView2".
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        let app_id: Vec<u16> = "com.slasshyvault.app\0".encode_utf16().collect();
        unsafe {
            SetCurrentProcessExplicitAppUserModelID(app_id.as_ptr());
        }
    }

    // Initialize single-instance plugin as early as possible for production builds.
    // This catches second instances BEFORE they attempt to open the database (which would cause a crash/panic).
    // We skip this in dev mode to allow production and dev instances to run independently without conflict.
    let single_instance_plugin = if !is_dev_runtime() {
        Some(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            println!(
                "[SINGLE-INSTANCE] Another instance attempted to start, focusing existing window"
            );
            restore_or_create_main_window(app);
        }))
    } else {
        None
    };

    // Load .env file from project root (for development)
    // This allows setting VITE_SENTRY_DSN, GDRIVE_CLIENT_ID, GDRIVE_CLIENT_SECRET, etc.
    dotenvy::dotenv().ok();

    // ponytail: sentry removed

    // Migrate app data from old StreamVault directory to new SlasshyVault directory.
    // Dev builds use StreamVault-Dev → SlasshyVault-Dev (isolated from production).
    // Release builds use StreamVault → SlasshyVault (production data migration).
    migrate_app_data();

    // Deep-link handling is registered by the v2 plugin and consumed below.

    // Initialize paths
    let db_path = database::get_database_path();
    let image_cache_dir = database::get_image_cache_dir();

    // Ensure directories exist
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| println!("[INIT] Warning: Failed to create db parent dir: {}", e))
            .ok();
    }
    std::fs::create_dir_all(&image_cache_dir)
        .map_err(|e| println!("[INIT] Warning: Failed to create image cache dir: {}", e))
        .ok();

    // Initialize database with auto-healing on corruption
    let db = match database::Database::new(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "[DB] Failed to open database: {}. Attempting auto-recovery...",
                e
            );

            let backup_path = format!("{}.corrupted", db_path);

            // Try to back up and recreate
            if std::fs::rename(&db_path, &backup_path).is_ok() {
                eprintln!("[DB] Corrupted database backed up to: {}", backup_path);
                match database::Database::new(&db_path) {
                    Ok(new_db) => {
                        eprintln!("[DB] Successfully created fresh database.");
                        new_db
                    }
                    Err(e2) => {
                        eprintln!("[DB] Failed to create fresh database: {}", e2);
                        std::fs::rename(&backup_path, &db_path).ok();
                        panic!("Failed to initialize database: {}", e2);
                    }
                }
            } else {
                panic!(
                    "Failed to initialize database and could not back up corrupted file: {}",
                    e
                );
            }
        }
    };

    // Load config
    let config = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[CONFIG] Failed to load config: {}. Falling back to defaults.",
                e
            );
            config::Config::default()
        }
    };

    // Create app state
    let state = AppState {
        db: Mutex::new(db),
        config: Mutex::new(config.clone()),
        is_scanning: Arc::new(AtomicBool::new(false)),
        active_mpv_sessions: Mutex::new(HashMap::new()),
        active_zip_streams: Mutex::new(HashMap::new()),
        download_manager: download_manager::DownloadManager::default(),
        gdrive_client: gdrive::GoogleDriveClient::new(),
        watch_together: Arc::new(tokio::sync::Mutex::new(
            watch_together::WatchTogetherManager::new(),
        )),
        wt_controller: Arc::new(tokio::sync::Mutex::new(None)),
        oauth_listener: Arc::new(Mutex::new(None)),
        oauth_nonce: Arc::new(Mutex::new(None)),
        cache_manager: stream_cache::CacheManager::new(),
        last_validation_report: Mutex::new(None),
        gdrive_refresh_watchdog: Arc::new(Mutex::new(None)),
    };

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_deep_link::init());
    let builder = if let Some(plugin) = single_instance_plugin {
        builder.plugin(plugin)
    } else {
        builder
    };
    builder
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--flag1", "--flag2"]),
        ))
        .manage(state)
        .setup(move |app| {
            let state = app.state::<AppState>();
            let tray_menu = build_tray_menu(app.handle(), &state)?;
            TrayIconBuilder::with_id("main")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let id = event.id().as_ref();
                    if id == "show" {
                        restore_or_create_main_window(&app.clone());
                    } else if id == "quit" {
                        std::process::exit(0);
                    } else if let Some(media_id_str) = id.strip_prefix("continue_") {
                        if let Ok(media_id) = media_id_str.parse::<i64>() {
                            restore_or_create_main_window(&app.clone());
                            let _ = app.emit("play-continue-watching", serde_json::json!({
                                "media_id": media_id,
                            }));
                        }
                    }
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                window.set_title(runtime_window_title()).ok();
                apply_window_corner_radius(&window);
            }

            // Allow both dev and production image cache directories in the asset protocol scope
            let base_dir = if cfg!(windows) {
                std::env::var_os("APPDATA").map(PathBuf::from)
            } else {
                dirs::home_dir()
            };
            if let Some(base_dir) = base_dir {
                for dir_name in &["SlasshyVault", "SlasshyVault-Dev"] {
                    let app_dir = if cfg!(windows) {
                        base_dir.join(dir_name)
                    } else {
                        base_dir.join(format!(".{}", dir_name))
                    };
                    let cache_dir = app_dir.join("image_cache");
                    if let Err(e) = app.asset_protocol_scope().allow_directory(&cache_dir, true) {
                        println!("[ASSET-SCOPE] Warning: Failed to allow {:?}: {}", cache_dir, e);
                    } else {
                        println!("[ASSET-SCOPE] Allowed image cache: {:?}", cache_dir);
                    }
                }
            }

            // Store global app handle for background thread event emission
            if let Ok(mut h) = GLOBAL_APP_HANDLE.lock() {
                *h = Some(app.handle().clone());
            }

            // Register deep-link handler for OAuth callback.
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    let request = url.to_string();
                    println!("[DEEPLINK] Received: {}", request);
                    if let Some(code) = url.query_pairs().find(|(key, _)| key == "code").map(|(_, value)| value.to_string()) {
                        if let Ok(tx) = OAUTH_CODE_CHANNEL.0.lock() {
                            let _ = tx.send(code);
                        }
                        restore_or_create_main_window(&handle);
                    }
                }
            });

            // Merge any duplicate TV shows on startup
            println!("[STARTUP] Running duplicate TV show merge...");
            let db_path = database::get_database_path();
            if let Ok(startup_db) = database::Database::new(&db_path) {
                match startup_db.repair_misparented_archive_episodes() {
                    Ok(count) if count > 0 => {
                        println!(
                            "[STARTUP] Re-parented {} mis-linked archive episode(s)",
                            count
                        );
                    }
                    Ok(_) => {}
                    Err(e) => println!(
                        "[STARTUP] Warning: Failed to repair archive episode parents: {}",
                        e
                    ),
                }
                if let Err(e) = startup_db.merge_duplicate_tvshows() {
                    println!("[STARTUP] Warning: Failed to merge duplicates: {}", e);
                }
            }

            // Clean up expired cloud cache on startup
            let config = match config::load_config() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[CONFIG] Failed to load config: {}. Falling back to defaults.", e);
                    config::Config::default()
                }
            };
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

            // Auto-start binary addon sources on app launch
            let mut addon_started = false;
            for source in &config.addon_sources {
                if !source.enabled { continue; }
                if let Some(ref bp) = source.binary_path {
                    let bp = bp.clone();
                    let url = source.url.clone();
                    // Extract port from saved URL so the binary binds to the correct port
                    let port = url.split(':').last()
                        .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
                        .unwrap_or(51546);
                    println!("[STARTUP] Auto-starting addon from binary: {} (url: {}, port: {})", bp, url, port);
                    let args: Vec<String> = vec!["--yes".to_string(), "--port".to_string(), port.to_string()];
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async {
                            match start_addon_process(&bp, &args).await {
                                Ok(_) => println!("[STARTUP] Binary addon started successfully"),
                                Err(e) => println!("[STARTUP] Failed to start binary addon: {}", e),
                            }
                        });
                    });
                    addon_started = true;
                    break; // Only one addon process can run at a time (RUNNING_ADDON_PROCESS is single)
                }
            }
            // Fallback: if addon_url is localhost and no binary source was started,
            // probe the port. If down, log a clear message.
            if !addon_started {
                if let Some(ref url) = config.addon_url {
                    if url.contains("localhost") || url.contains("127.0.0.1") {
                        let url_clone = url.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_secs(2));
                            let host_port = url_clone.replace("http://", "").replace("https://", "").trim_end_matches('/').to_string();
                            let addr: std::net::SocketAddr = host_port.parse().unwrap_or_else(|_| {
                                // Try adding default port
                                format!("{}:80", host_port).parse().unwrap_or(([127,0,0,1], 51546).into())
                            });
                            match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(3)) {
                                Ok(_) => println!("[STARTUP] Addon server already running at {}", url_clone),
                                Err(e) => println!("[STARTUP] Addon server NOT running at {} ({}). Install via Settings > External or start manually.", url_clone, e),
                            }
                        });
                    }
                }
            }

            let zip_cache_config = build_zip_cache_config(&config);
            if let Err(error) = zip_manager::cleanup_stale_zip_cache(&zip_cache_config) {
                println!("[STARTUP] Warning: Failed to clean ZIP cache: {}", error);
            }

            // Start background cloud polling (runs independently of window)
            let app_handle_for_polling = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                background_cloud_poll(app_handle_for_polling).await;
            });

            // One-time metadata enrichment pass for existing libraries.
            let app_handle_for_metadata_enrichment = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_startup_metadata_enrichment(app_handle_for_metadata_enrichment).await;
            });

            let app_handle_for_reminders = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_movie_reminder_scheduler(app_handle_for_reminders).await;
            });

            let app_handle_for_watchlist = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_watchlist_scheduler(app_handle_for_watchlist).await;
            });

            let app_handle_for_watchlist_sync = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = app_handle_for_watchlist_sync.state::<AppState>();
                match sync_watchlist_to_drive(&state).await {
                    Ok(status) => {
                        if status.synced {
                            println!(
                                "[WATCHLIST] Startup sync complete: merged_remote_items={}, uploaded_items={}",
                                status.merged_remote_items, status.uploaded_items
                            );
                            let _ = app_handle_for_watchlist_sync.emit("refresh-watchlist", ());
                        } else if let Some(reason) = status.skipped_reason {
                            println!("[WATCHLIST] Startup sync skipped: {}", reason);
                        }
                    }
                    Err(error) => {
                        println!("[WATCHLIST] Startup sync failed: {}", error);
                    }
                }
            });

            // Background refresh watchdog for Google Drive tokens. Keeps access
            // tokens fresh while the app is idle so users are not logged out
            // after long periods and so cloud playback does not expire the
            // token mid-stream.
            let state_handle = app.state::<AppState>();
            let watchdog_handle =
                state_handle.gdrive_client.start_background_refresh_watchdog();
            if let Ok(mut guard) = state_handle.gdrive_refresh_watchdog.lock() {
                *guard = Some(watchdog_handle);
            }

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

            let _ = window.emit("inject-script", popup_block_script);
        })
        .on_window_event(|window, event| {
            match event {
                WindowEvent::CloseRequested { .. } => {
                    // Let the window close/destroy completely to free RAM
                    // Don't prevent close - we handle app exit separately in .run()
                    println!("[TRAY] WebviewWindow closing/destroying. Backend will keep running.");
                }
                WindowEvent::Focused(focused) => {
                    if *focused {
                        // Re-inject popup blocker when window regains focus
                        let _ = window.emit("inject-script", r#"
                            if (!window.__adBlockerActive) {
                                window.__adBlockerActive = true;
                                const origOpen = window.open;
                                window.open = function(url) {
                                    console.log('[AdBlocker] Blocked popup on focus:', url);
                                    return null;
                                };
                            }
                        "#);
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_nickname,
            set_nickname,
            get_recently_added,
            get_library,
            get_library_filtered,
            search_all_libraries,
            search_media_by_cast,
            get_ddl_media,
            get_library_stats,
            get_top_space_consumers,
            get_episodes,
            get_watch_history,
            get_watch_history_events,
            remove_from_watch_history,
            remove_watch_history_entry,
            clear_all_watch_history,
            sync_watch_history,
            mark_as_complete,
            // Social sync commands
            get_watch_stats,
            get_recent_watch_activities,
            get_analytics_data,
            // App reset command
            clear_all_app_data,
            restart_app,
            cleanup_missing_metadata,
            // Other commands
            delete_media_files,
            delete_all_media_files,
            delete_cloud_files_by_drive_ids,
            delete_series,
            delete_series_cloud_folder,
            get_episodes_for_delete,
            get_config,
            save_config,
            auto_detect_mpv,
            download_bundled_mpv,
            get_bundled_mpv_info,
            get_scan_status,
            get_resume_info,
            get_media_info,
            resolve_watch_history_media,
            get_archive_playback_assessment,
            get_stream_info,
            get_audio_tracks,
            get_subtitle_tracks,
            get_media_technical_details,
            zip_analyze,
            zip_index_episodes,
            zip_get_stream_info,
            update_progress,
            clear_progress,
            update_episode_duration,
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
            search_content,
            get_tmdb_trending,
            get_movie_details,
            get_tv_details,
            get_tv_season_episodes,
            get_episode_imdb_ratings,
            get_imdb_details,
            get_parents_guide,
            get_tmdb_reviews,
            refresh_series_metadata,
            get_tmdb_release_schedule,
            create_movie_reminder,
            update_movie_reminder,
            get_movie_reminders,
            delete_movie_reminder,
            set_movie_reminder_active,
            get_watchlist_items,
            create_or_update_watchlist_item,
            update_watchlist_item,
            delete_watchlist_item,
            sync_watchlist,
            merge_duplicate_shows,
            find_duplicate_media,
            // Google Drive commands
            gdrive_is_connected,
            gdrive_get_access_token,
            gdrive_get_auth_status,
            gdrive_get_account_info,
            gdrive_start_auth,
            gdrive_complete_auth,
            gdrive_disconnect,
            gdrive_list_folders,
            gdrive_list_files,
            gdrive_list_video_files,
            gdrive_get_stream_url,
            gdrive_get_file_metadata,
            gdrive_share_file,
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
            get_download_jobs,
            start_media_download,
            cancel_download_job,
            delete_download_job,
            clear_download_history,
            open_download_job_target,
            // Direct Download Link (DDL) commands
            ddl_validate_url,
            ddl_index_archive,
            ddl_get_sources,
            ddl_check_link_health,
            ddl_refresh_link,
            ddl_delete_source,
            ddl_get_source_media,
            index_season_pack_to_ddl,
            auto_refresh_ddl_from_addon,
            // Auto-update commands
            check_for_updates,
            download_update,
            install_update,
            get_app_version,
            // Developer console commands
            get_recent_logs,
            clear_logs,
            // Watch Together commands
            wt_create_room,
            wt_join_room,
            wt_leave_room,
            wt_set_ready,
            wt_start_playback,
            wt_send_sync,
            wt_get_room_state,
            wt_is_active,
            wt_get_client_id,
            wt_launch_mpv,
            wt_send_mpv_command,
            // Remote Source commands
            remote_get_movie_streams,
            remote_get_series_streams,
            remote_get_season_streams,
            resolve_imdb_id,
            validate_hubdrive_url,
            verify_stream_url,
            verify_stream_urls,
            remote_play_with_mpv,
            remote_clear_progress,
            remote_get_resume_info,
            remote_get_library,
            remote_remove_from_library,
            remote_add_to_library,
            remote_is_bookmarked,
            remote_update_progress,
            remote_get_show_progress,
            remote_get_all_progress,
            remote_start_cache,
            remote_stop_cache,
            remote_get_cache_status,
            remote_get_all_cache_status,
            remote_cleanup_cache,
            remote_cleanup_all_cache,
            remote_is_cache_dir_set,
            remote_get_cache_dir,
            get_addon_url,
            set_addon_url,
            get_addon_sources,
            add_addon_source,
            remove_addon_source,
            check_addon_server,
            get_addon_version,
            get_addon_logs,
            set_active_source,
            toggle_addon_source,
            auto_setup_addon,
            install_addon_binary,
            remove_addon_binary,
            restart_addon,
            remote_clear_streams_cache,

            run_sync_validation,
            fix_sync_issues,
            refresh_tray_continue_watching,
            fetch_subtitles,
            download_subtitle,
            // Cloudflare relay commands
            cf_relay::cf_list_accounts,
            cf_relay::cf_deploy_relay,
            cf_relay::cf_delete_relay,
            cf_relay::cf_relay_status,
            // Stremio addon commands
            stremio::stremio_add_addon,
            stremio::stremio_remove_addon,
            stremio::stremio_list_addons,
            stremio::stremio_fetch_catalog,
            stremio::stremio_fetch_meta,
            stremio::stremio_fetch_streams,
            stremio::stremio_resolve_stream,
            // Debrid service commands
            stremio::debrid_add_service,
            stremio::debrid_remove_service,
            stremio::debrid_list_services,
            stremio::debrid_set_default,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            tauri::RunEvent::ExitRequested { api, .. } => {
                api.prevent_exit();
                println!("[TRAY] Exit prevented. App running in background. Click tray to reopen.");
            }
            tauri::RunEvent::Exit => {
                if let Ok(mut proc) = RUNNING_ADDON_PROCESS.lock() {
                    if let Some(mut child) = proc.take() {
                        println!("[EXIT] Killing addon server process...");
                        let _ = child.kill();
                    }
                }
                abort_gdrive_refresh_watchdog(&app_handle);
            }
            _ => {}
        });
}
