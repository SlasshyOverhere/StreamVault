use rusqlite::types::FromSql;
use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::zip_manager;

const APP_NAME: &str = "SlasshyVault";
const AUTO_MARK_WATCHED_THRESHOLD_PERCENT: f64 = 93.0;
const AUTO_MARK_WATCHED_THRESHOLD_RATIO: f64 = 0.93;

/// Get the app data directory, with separate paths for dev and production builds
/// Dev builds use "SlasshyVault-Dev" to keep data isolated from production
pub fn get_app_data_dir() -> PathBuf {
    // Use a different directory name for debug/dev builds
    let dir_name = if cfg!(debug_assertions) {
        format!("{}-Dev", APP_NAME)
    } else {
        APP_NAME.to_string()
    };

    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join(&dir_name);
        }
    }

    dirs::home_dir()
        .map(|h| h.join(format!(".{}", dir_name)))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn get_database_path() -> String {
    get_app_data_dir()
        .join("media_library.db")
        .to_string_lossy()
        .to_string()
}

pub fn get_image_cache_dir() -> String {
    get_app_data_dir()
        .join("image_cache")
        .to_string_lossy()
        .to_string()
}

pub fn get_zip_cache_dir() -> String {
    get_app_data_dir()
        .join("zip_cache")
        .to_string_lossy()
        .to_string()
}

pub fn get_config_path() -> String {
    get_app_data_dir()
        .join("media_config.json")
        .to_string_lossy()
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: i64,
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub cast_names: Option<String>,
    pub director: Option<String>,
    pub poster_path: Option<String>,
    pub file_path: Option<String>,
    pub media_type: String,
    pub duration_seconds: Option<f64>,
    pub resume_position_seconds: Option<f64>,
    pub last_watched: Option<String>,
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub parent_id: Option<i64>,
    pub progress_percent: Option<f64>,
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub episode_title: Option<String>,
    pub still_path: Option<String>,
    // Cloud storage fields
    pub is_cloud: Option<bool>,
    pub cloud_file_id: Option<String>,
    pub cloud_folder_id: Option<String>,
    pub archive_format: Option<String>,
    pub parent_zip_id: Option<String>,
    pub zip_entry_path: Option<String>,
    pub zip_local_header_offset: Option<i64>,
    pub zip_data_start_offset: Option<i64>,
    pub zip_compressed_size: Option<i64>,
    pub zip_uncompressed_size: Option<i64>,
    pub zip_crc32: Option<String>,
    pub zip_compression_method: Option<i64>,
    pub file_size_bytes: Option<i64>,
    pub ddl_source_id: Option<String>,
    pub archive_playback_can_play: Option<bool>,
    pub archive_playback_mode: Option<String>,
    pub archive_playback_message: Option<String>,
    pub archive_playback_details: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteBookmark {
    pub tmdb_id: String,
    pub media_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub poster_path: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemotePlaybackProgress {
    pub tmdb_id: String,
    pub media_type: String,
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub resume_position_seconds: f64,
    pub duration_seconds: f64,
    pub last_watched: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZipArchiveRecord {
    pub zip_file_id: String,
    pub filename: String,
    pub archive_format: String,
    pub file_size_bytes: i64,
    pub compression_type: String,
    pub central_dir_offset: i64,
    pub central_dir_size: i64,
    pub total_entries: i64,
    pub video_entries: i64,
    pub last_analyzed: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MetadataEnrichmentCandidate {
    pub id: i64,
    pub title: String,
    pub year: Option<i32>,
    pub media_type: String,
    pub tmdb_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeInfo {
    pub has_progress: bool,
    pub position: f64,
    pub duration: f64,
    pub time_str: String,
    pub progress_percent: f64,
}

/// Cached episode metadata from TMDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedEpisodeMetadata {
    pub episode_title: Option<String>,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub air_date: Option<String>,
    pub vote_average: Option<f64>,
}

/// Full cached episode metadata (includes season/episode numbers)
#[derive(Debug, Clone)]
pub struct CachedEpisodeMetadataFull {
    pub episode_title: Option<String>,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub air_date: Option<String>,
    pub season_number: i32,
    pub episode_number: i32,
    pub vote_average: Option<f64>,
}

/// Streaming history item for online content (Videasy, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingHistoryItem {
    pub id: i64,
    pub tmdb_id: String,
    pub media_type: String, // "movie" or "tv"
    pub title: String,
    pub poster_path: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub resume_position_seconds: f64,
    pub duration_seconds: f64,
    pub progress_percent: f64,
    pub last_watched: String,
}

/// Aggregated watch statistics for social sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchStatsAggregated {
    pub movies_watched: i64,
    pub episodes_watched: i64,
    pub total_watch_time_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryStats {
    pub movies: i64,
    pub shows: i64,
    pub episodes: i64,
}

/// A recently completed watch activity for social sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchActivityItem {
    pub content_id: String,
    pub title: String,
    pub content_type: String,  // "movie" or "tv"
    pub activity_type: String, // "watched_movie" or "watched_episode"
    pub poster_path: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub duration_seconds: Option<f64>,
    pub last_watched: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchHistoryEvent {
    pub event_id: String,
    pub media_id: Option<i64>,
    pub parent_media_id: Option<i64>,
    pub title: String,
    pub parent_title: Option<String>,
    pub media_type: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub still_path: Option<String>,
    pub tmdb_id: Option<String>,
    pub parent_tmdb_id: Option<String>,
    pub episode_title: Option<String>,
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub is_cloud: bool,
    pub progress_percent: f64,
    pub resume_position_seconds: f64,
    pub duration_seconds: f64,
    pub completed: bool,
    pub started_at: String,
    pub ended_at: String,
    pub updated_at: String,
}

// ==================== ANALYTICS TYPES ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsOverview {
    pub total_watch_time_seconds: f64,
    pub movies_completed: i64,
    pub episodes_completed: i64,
    pub total_completion_rate: f64,
    pub current_streak_days: i32,
    pub total_events: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatmapDay {
    pub date: String,
    pub watch_seconds: f64,
    pub event_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyWatchPoint {
    pub date: String,
    pub watch_seconds: f64,
    pub movie_count: i64,
    pub episode_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentTypeBreakdown {
    pub content_type: String,
    pub count: i64,
    pub total_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceBreakdown {
    pub source: String,
    pub count: i64,
    pub total_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopWatchedItem {
    pub title: String,
    pub parent_title: Option<String>,
    pub media_type: String,
    pub watch_count: i64,
    pub total_seconds: f64,
    pub poster_path: Option<String>,
    pub tmdb_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HourDistribution {
    pub hour: i32,
    pub event_count: i64,
    pub total_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayOfWeekDistribution {
    pub day_of_week: i32,
    pub event_count: i64,
    pub total_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionFunnel {
    pub started: i64,
    pub in_progress_25: i64,
    pub mostly_done_75: i64,
    pub completed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsData {
    pub overview: AnalyticsOverview,
    pub heatmap: Vec<HeatmapDay>,
    pub daily_trend: Vec<DailyWatchPoint>,
    pub content_breakdown: Vec<ContentTypeBreakdown>,
    pub source_breakdown: Vec<SourceBreakdown>,
    pub top_watched: Vec<TopWatchedItem>,
    pub hour_distribution: Vec<HourDistribution>,
    pub day_distribution: Vec<DayOfWeekDistribution>,
    pub completion_funnel: CompletionFunnel,
    pub library_stats: LibraryStats,
    pub recent_events: Vec<WatchHistoryEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudIndexFailure {
    pub cloud_file_id: String,
    pub file_name: String,
    pub last_error: String,
    pub last_attempt: String,
}

pub struct Database {
    conn: Connection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovieReminder {
    pub id: i64,
    pub tmdb_id: String,
    pub media_type: String,
    pub title: String,
    pub poster_path: Option<String>,
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub release_date: Option<String>,
    pub reminder_at: String,
    pub source: String,
    pub tracking_mode: String,
    pub tracking_season_number: Option<i32>,
    pub notes: Option<String>,
    pub is_active: bool,
    pub notified_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchlistItem {
    pub id: i64,
    pub tmdb_id: String,
    pub media_type: String,
    pub title: String,
    pub poster_path: Option<String>,
    pub release_date: Option<String>,
    pub notes: Option<String>,
    pub is_active: bool,
    pub notification_enabled: bool,
    pub notification_mode: String,
    pub notification_interval_minutes: Option<i32>,
    pub notify_at: Option<String>,
    pub last_notified_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewMovieReminder<'a> {
    pub tmdb_id: &'a str,
    pub media_type: &'a str,
    pub title: &'a str,
    pub poster_path: Option<&'a str>,
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub release_date: Option<&'a str>,
    pub reminder_at: &'a str,
    pub source: &'a str,
    pub tracking_mode: &'a str,
    pub tracking_season_number: Option<i32>,
    pub notes: Option<&'a str>,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct NewWatchlistItem<'a> {
    pub tmdb_id: &'a str,
    pub media_type: &'a str,
    pub title: &'a str,
    pub poster_path: Option<&'a str>,
    pub release_date: Option<&'a str>,
    pub notes: Option<&'a str>,
    pub is_active: bool,
    pub notification_enabled: bool,
    pub notification_mode: &'a str,
    pub notification_interval_minutes: Option<i32>,
    pub notify_at: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncIssue {
    pub category: String,
    pub file_name: String,
    pub file_id: Option<String>,
    pub reason: String,
    pub fixable: bool,
    pub fix_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub items: Vec<MediaItem>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncValidationReport {
    pub ghost_entries: Vec<SyncIssue>,
    pub missing_files: Vec<SyncIssue>,
    pub failed_indexings: Vec<SyncIssue>,
    pub orphaned_zip_entries: Vec<SyncIssue>,
    pub stale_token: Vec<SyncIssue>,
    pub total_issues: usize,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        // WAL mode: concurrent reads + writes without blocking each other.
        // Critical for background enrichment writing while UI reads library.
        let _ = conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()));
        conn.execute("PRAGMA synchronous = NORMAL", [])?;
        // Enable foreign key enforcement so ON DELETE CASCADE works.
        conn.execute("PRAGMA foreign_keys = ON", [])?;
        let db = Database { conn };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        // Create media table if not exists
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS media (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                year INTEGER,
                overview TEXT,
                cast_names TEXT,
                director TEXT,
                poster_path TEXT,
                file_path TEXT NOT NULL UNIQUE,
                media_type TEXT NOT NULL,
                parent_id INTEGER,
                season_number INTEGER,
                episode_number INTEGER,
                duration_seconds REAL DEFAULT 0,
                resume_position_seconds REAL DEFAULT 0,
                last_watched TIMESTAMP DEFAULT NULL,
                tmdb_id TEXT DEFAULT NULL,
                FOREIGN KEY (parent_id) REFERENCES media (id) ON DELETE CASCADE
            )",
            [],
        )?;

        // Check for missing columns and add them
        let columns: Vec<String> = self
            .conn
            .prepare("PRAGMA table_info(media)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();

        if !columns.contains(&"parent_id".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN parent_id INTEGER REFERENCES media(id) ON DELETE CASCADE", [])?;
        }
        if !columns.contains(&"season_number".to_string()) {
            self.conn
                .execute("ALTER TABLE media ADD COLUMN season_number INTEGER", [])?;
        }
        if !columns.contains(&"episode_number".to_string()) {
            self.conn
                .execute("ALTER TABLE media ADD COLUMN episode_number INTEGER", [])?;
        }
        if !columns.contains(&"duration_seconds".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN duration_seconds REAL DEFAULT 0",
                [],
            )?;
        }
        if !columns.contains(&"resume_position_seconds".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN resume_position_seconds REAL DEFAULT 0",
                [],
            )?;
        }
        if !columns.contains(&"last_watched".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN last_watched TIMESTAMP DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"tmdb_id".to_string()) {
            self.conn
                .execute("ALTER TABLE media ADD COLUMN tmdb_id TEXT DEFAULT NULL", [])?;
        }
        if !columns.contains(&"imdb_id".to_string()) {
            self.conn
                .execute("ALTER TABLE media ADD COLUMN imdb_id TEXT DEFAULT NULL", [])?;
        }
        if !columns.contains(&"episode_title".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN episode_title TEXT DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"still_path".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN still_path TEXT DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"cast_names".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN cast_names TEXT DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"director".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN director TEXT DEFAULT NULL",
                [],
            )?;
        }

        // Cloud storage columns
        if !columns.contains(&"is_cloud".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN is_cloud INTEGER DEFAULT 0",
                [],
            )?;
        }
        if !columns.contains(&"cloud_file_id".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN cloud_file_id TEXT DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"archive_format".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN archive_format TEXT DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"cloud_folder_id".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN cloud_folder_id TEXT DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"parent_zip_id".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN parent_zip_id TEXT DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"zip_entry_path".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN zip_entry_path TEXT DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"zip_local_header_offset".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN zip_local_header_offset INTEGER DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"zip_data_start_offset".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN zip_data_start_offset INTEGER DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"zip_compressed_size".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN zip_compressed_size INTEGER DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"zip_uncompressed_size".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN zip_uncompressed_size INTEGER DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"zip_crc32".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN zip_crc32 TEXT DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"zip_compression_method".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN zip_compression_method INTEGER DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"file_size_bytes".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN file_size_bytes INTEGER DEFAULT NULL",
                [],
            )?;
        }
        if !columns.contains(&"is_remote_library".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN is_remote_library INTEGER DEFAULT 0",
                [],
            )?;
        }

        // Create cached_episode_metadata table for pre-fetched episode info from TMDB
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS cached_episode_metadata (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                series_tmdb_id TEXT NOT NULL,
                season_number INTEGER NOT NULL,
                episode_number INTEGER NOT NULL,
                episode_title TEXT,
                overview TEXT,
                still_path TEXT,
                air_date TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(series_tmdb_id, season_number, episode_number)
            )",
            [],
        )?;

        // Add vote_average column if missing
        let _ = self.conn.execute(
            "ALTER TABLE cached_episode_metadata ADD COLUMN vote_average REAL DEFAULT NULL",
            [],
        );

        // Create streaming history table for online content (Videasy, etc.)
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS streaming_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tmdb_id TEXT NOT NULL,
                media_type TEXT NOT NULL,
                title TEXT NOT NULL,
                poster_path TEXT,
                season INTEGER,
                episode INTEGER,
                resume_position_seconds REAL DEFAULT 0,
                duration_seconds REAL DEFAULT 0,
                last_watched TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Lightweight bookmark table for External tab "My Library"
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS remote_bookmarks (
                tmdb_id TEXT NOT NULL,
                media_type TEXT NOT NULL,
                title TEXT NOT NULL,
                year INTEGER,
                poster_path TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (tmdb_id, media_type)
            )",
            [],
        )?;

        // Playback progress for remote content (movies and individual episodes)
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS remote_playback_progress (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tmdb_id TEXT NOT NULL,
                media_type TEXT NOT NULL,
                season_number INTEGER DEFAULT -1,
                episode_number INTEGER DEFAULT -1,
                resume_position_seconds REAL DEFAULT 0,
                duration_seconds REAL DEFAULT 0,
                last_watched TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(tmdb_id, media_type, season_number, episode_number)
            )",
            [],
        )?;

        // One-time migration: copy existing remote library items to bookmarks table
        let bookmark_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM remote_bookmarks", [], |r| r.get(0))
            .unwrap_or(0);
        if bookmark_count == 0 {
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO remote_bookmarks (tmdb_id, media_type, title, year, poster_path)
                 SELECT tmdb_id,
                        CASE WHEN media_type = 'tvshow' THEN 'tv' ELSE 'movie' END,
                        title, year, poster_path
                 FROM media
                 WHERE file_path LIKE 'remote://%' AND is_remote_library = 1 AND tmdb_id IS NOT NULL",
                [],
            );
        }

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS watch_history_events (
                event_id TEXT PRIMARY KEY,
                media_id INTEGER,
                parent_media_id INTEGER,
                title TEXT NOT NULL,
                parent_title TEXT,
                media_type TEXT NOT NULL,
                year INTEGER,
                overview TEXT,
                poster_path TEXT,
                still_path TEXT,
                tmdb_id TEXT,
                parent_tmdb_id TEXT,
                episode_title TEXT,
                season_number INTEGER,
                episode_number INTEGER,
                is_cloud INTEGER NOT NULL DEFAULT 0,
                progress_percent REAL NOT NULL DEFAULT 0,
                resume_position_seconds REAL NOT NULL DEFAULT 0,
                duration_seconds REAL NOT NULL DEFAULT 0,
                completed INTEGER NOT NULL DEFAULT 0,
                started_at TIMESTAMP NOT NULL,
                ended_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL
            )",
            [],
        )?;

        // Clean up duplicate entries before creating unique index
        // Keep only the most recent entry for each unique combination
        self.conn.execute(
            "DELETE FROM streaming_history WHERE id NOT IN (
                SELECT MAX(id) FROM streaming_history
                GROUP BY tmdb_id, media_type, COALESCE(season, -1), COALESCE(episode, -1)
            )",
            [],
        )?;

        // Create unique index that handles NULL values properly using COALESCE
        // This will now succeed since duplicates are removed
        self.conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_streaming_unique
             ON streaming_history (tmdb_id, media_type, COALESCE(season, -1), COALESCE(episode, -1))",
            [],
        )?;

        // Create cloud_folders table for storing Google Drive folder configurations
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS cloud_folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                folder_id TEXT NOT NULL UNIQUE,
                folder_name TEXT NOT NULL,
                auto_scan INTEGER DEFAULT 1,
                last_scanned TIMESTAMP,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Add changes_page_token column if it doesn't exist (migration)
        self.conn
            .execute(
                "ALTER TABLE cloud_folders ADD COLUMN changes_page_token TEXT",
                [],
            )
            .ok(); // Ignore error if column already exists

        // Create app_settings table for storing global settings like the changes token
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS cloud_index_failures (
                cloud_file_id TEXT PRIMARY KEY,
                file_name TEXT NOT NULL,
                last_error TEXT NOT NULL,
                last_attempt TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS movie_reminders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tmdb_id TEXT NOT NULL,
                media_type TEXT NOT NULL,
                title TEXT NOT NULL,
                poster_path TEXT,
                season_number INTEGER,
                episode_number INTEGER,
                release_date TEXT,
                reminder_at TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'manual',
                tracking_mode TEXT NOT NULL DEFAULT 'single',
                tracking_season_number INTEGER,
                notes TEXT,
                is_active INTEGER NOT NULL DEFAULT 1,
                notified_at TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_movie_reminders_due
             ON movie_reminders(is_active, reminder_at)",
            [],
        )?;

        self.conn
            .execute(
                "ALTER TABLE movie_reminders ADD COLUMN tracking_mode TEXT NOT NULL DEFAULT 'single'",
                [],
            )
            .ok();

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS watchlist_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tmdb_id TEXT NOT NULL,
                media_type TEXT NOT NULL,
                title TEXT NOT NULL,
                poster_path TEXT,
                release_date TEXT,
                notes TEXT,
                is_active INTEGER NOT NULL DEFAULT 1,
                notification_enabled INTEGER NOT NULL DEFAULT 0,
                notification_mode TEXT NOT NULL DEFAULT 'single',
                notification_interval_minutes INTEGER,
                notify_at TEXT,
                last_notified_at TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(tmdb_id, media_type)
            )",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_watchlist_due
             ON watchlist_items(notification_enabled, notify_at)",
            [],
        )?;

        self.conn
            .execute(
                "ALTER TABLE watchlist_items ADD COLUMN notification_mode TEXT NOT NULL DEFAULT 'single'",
                [],
            )
            .ok();
        self.conn
            .execute(
                "ALTER TABLE watchlist_items ADD COLUMN notification_interval_minutes INTEGER",
                [],
            )
            .ok();
        self.conn
            .execute("ALTER TABLE watchlist_items ADD COLUMN notify_at TEXT", [])
            .ok();
        self.conn
            .execute(
                "ALTER TABLE watchlist_items ADD COLUMN last_notified_at TEXT",
                [],
            )
            .ok();
        self.conn
            .execute(
                "ALTER TABLE movie_reminders ADD COLUMN tracking_season_number INTEGER",
                [],
            )
            .ok();

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS zip_archives (
                zip_file_id TEXT PRIMARY KEY,
                filename TEXT NOT NULL,
                archive_format TEXT NOT NULL DEFAULT 'zip',
                file_size_bytes INTEGER NOT NULL,
                compression_type TEXT NOT NULL,
                central_dir_offset INTEGER,
                central_dir_size INTEGER,
                total_entries INTEGER,
                video_entries INTEGER,
                last_analyzed TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;
        self.conn
            .execute(
                "ALTER TABLE zip_archives ADD COLUMN archive_format TEXT NOT NULL DEFAULT 'zip'",
                [],
            )
            .ok();

        // Direct Download Link (DDL) sources table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS ddl_sources (
                id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                filename TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                archive_format TEXT NOT NULL DEFAULT 'zip',
                entry_count INTEGER NOT NULL DEFAULT 0,
                video_count INTEGER NOT NULL DEFAULT 0,
                cd_offset INTEGER NOT NULL DEFAULT 0,
                cd_size INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                last_verified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                is_expired INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        // Migrate ddl_sources: add addon_origin column for season pack auto-refresh
        let ddl_columns: Vec<String> = self
            .conn
            .prepare("PRAGMA table_info(ddl_sources)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        if !ddl_columns.contains(&"addon_origin".to_string()) {
            self.conn.execute(
                "ALTER TABLE ddl_sources ADD COLUMN addon_origin TEXT DEFAULT NULL",
                [],
            )?;
        }

        // Add ddl_source_id column to media table for linking to DDL sources
        if !columns.contains(&"ddl_source_id".to_string()) {
            self.conn.execute(
                "ALTER TABLE media ADD COLUMN ddl_source_id TEXT DEFAULT NULL",
                [],
            )?;
        }

        // Cover the common library list queries: filter by media_type, then order by title.
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_media_type_title ON media(media_type, title)",
            [],
        )?;

        // Episode lists always fetch by parent_id and then sort by season/episode.
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_media_parent_order ON media(parent_id, season_number, episode_number)",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_media_last_watched ON media(last_watched DESC) WHERE last_watched IS NOT NULL",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_media_cloud_folder_id ON media(cloud_folder_id) WHERE cloud_folder_id IS NOT NULL",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_media_type_tmdb_id ON media(media_type, tmdb_id) WHERE tmdb_id IS NOT NULL AND tmdb_id != ''",
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_media_cloud_file_id ON media(cloud_file_id) WHERE cloud_file_id IS NOT NULL",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_cloud_index_failures_last_attempt ON cloud_index_failures(last_attempt DESC)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_media_parent_zip_id ON media(parent_zip_id) WHERE parent_zip_id IS NOT NULL",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_zip_archives_file_id ON zip_archives(zip_file_id)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_media_ddl_source_id ON media(ddl_source_id) WHERE ddl_source_id IS NOT NULL",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_watch_history_events_ended_at ON watch_history_events(ended_at DESC)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_watch_history_events_media_recent ON watch_history_events(media_id, updated_at DESC)",
            [],
        )?;
        self.conn.execute(
            "INSERT INTO watch_history_events (
                event_id,
                media_id,
                parent_media_id,
                title,
                parent_title,
                media_type,
                year,
                overview,
                poster_path,
                still_path,
                tmdb_id,
                parent_tmdb_id,
                episode_title,
                season_number,
                episode_number,
                is_cloud,
                progress_percent,
                resume_position_seconds,
                duration_seconds,
                completed,
                started_at,
                ended_at,
                updated_at
            )
            SELECT
                lower(hex(randomblob(16))),
                m.id,
                m.parent_id,
                m.title,
                p.title,
                m.media_type,
                CASE WHEN m.media_type = 'tvepisode' THEN p.year ELSE m.year END,
                m.overview,
                CASE WHEN m.media_type = 'tvepisode' THEN p.poster_path ELSE m.poster_path END,
                m.still_path,
                m.tmdb_id,
                p.tmdb_id,
                m.episode_title,
                m.season_number,
                m.episode_number,
                COALESCE(m.is_cloud, 0),
                CASE
                    WHEN COALESCE(m.duration_seconds, 0) > 0 AND COALESCE(m.resume_position_seconds, 0) = 0 THEN 100
                    WHEN COALESCE(m.duration_seconds, 0) > 0 THEN (COALESCE(m.resume_position_seconds, 0) * 100.0) / m.duration_seconds
                    ELSE 0
                END,
                COALESCE(m.resume_position_seconds, 0),
                COALESCE(m.duration_seconds, 0),
                CASE
                    WHEN COALESCE(m.duration_seconds, 0) > 0 AND COALESCE(m.resume_position_seconds, 0) = 0 THEN 1
                    ELSE 0
                END,
                m.last_watched,
                m.last_watched,
                m.last_watched
            FROM media m
            LEFT JOIN media p ON m.parent_id = p.id
            WHERE m.last_watched IS NOT NULL
              AND m.media_type IN ('movie', 'tvepisode')
              AND NOT EXISTS (
                  SELECT 1
                  FROM watch_history_events w
                  WHERE w.media_id = m.id
                    AND w.ended_at = m.last_watched
              )",
            [],
        )?;
        // Indexes for streaming_history and remote_playback_progress
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_streaming_history_last_watched ON streaming_history(last_watched DESC)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_remote_playback_last_watched ON remote_playback_progress(last_watched DESC)",
            [],
        )?;

        // One-time reconciliation of legacy watch history events (moved from get_watch_history_events)
        if let Err(e) = self.reconcile_legacy_watch_history_events() {
            eprintln!(
                "[DB] Warning: Failed to reconcile legacy watch history events: {}",
                e
            );
        }

        Ok(())
    }

    pub fn get_library(&self, media_type: &str, search: Option<&str>) -> Result<Vec<MediaItem>> {
        let mut sql = String::from(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, imdb_id, episode_title, still_path,
                    archive_format,
                    is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, ddl_source_id
             FROM media WHERE media_type = ?",
        );

        if search.is_some() {
            sql.push_str(" AND title LIKE ?");
        }
        sql.push_str(" ORDER BY title");

        let mut stmt = self.conn.prepare(&sql)?;

        let items = if let Some(query) = search {
            stmt.query_map(
                params![media_type, format!("%{}%", query)],
                Self::map_media_item,
            )?
        } else {
            stmt.query_map(params![media_type], Self::map_media_item)?
        };

        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    /// Get library filtered by cloud status
    pub fn get_library_filtered(
        &self,
        media_type: &str,
        search: Option<&str>,
        is_cloud: Option<bool>,
    ) -> Result<Vec<MediaItem>> {
        let mut sql = String::from(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, imdb_id, episode_title, still_path,
                    archive_format,
                    is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, ddl_source_id
             FROM media WHERE media_type = ?",
        );

        // Add cloud filter if specified
        if let Some(cloud) = is_cloud {
            if cloud {
                sql.push_str(" AND is_cloud = 1");
            } else {
                sql.push_str(" AND (is_cloud = 0 OR is_cloud IS NULL)");
            }
        }

        if search.is_some() {
            sql.push_str(" AND title LIKE ?");
        }
        // Only exclude remote:// items when explicitly requesting local-only content
        if is_cloud == Some(false) {
            sql.push_str(" AND (file_path IS NULL OR file_path NOT LIKE 'remote://%')");
        }
        sql.push_str(" ORDER BY title");

        let mut stmt = self.conn.prepare(&sql)?;

        let items = if let Some(query) = search {
            stmt.query_map(
                params![media_type, format!("%{}%", query)],
                Self::map_media_item,
            )?
        } else {
            stmt.query_map(params![media_type], Self::map_media_item)?
        };

        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    /// Get DDL media items (movies and TV shows indexed from direct download links)
    pub fn get_ddl_media(&self, media_type: &str, search: Option<&str>) -> Result<Vec<MediaItem>> {
        let mut sql = String::from(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    archive_format,
                    is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, ddl_source_id
             FROM media WHERE media_type = ? AND ddl_source_id IS NOT NULL",
        );

        if let Some(_query) = search {
            sql.push_str(" AND title LIKE ?");
        }
        sql.push_str(" ORDER BY last_watched DESC, title");

        let mut stmt = self.conn.prepare(&sql)?;

        let items = if let Some(query) = search {
            stmt.query_map(
                params![media_type, format!("%{}%", query)],
                Self::map_media_item,
            )?
        } else {
            stmt.query_map(params![media_type], Self::map_media_item)?
        };

        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn get_recently_added(&self, limit: i32, is_cloud: Option<bool>) -> Result<Vec<MediaItem>> {
        let mut sql = String::from(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    archive_format,
                    is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, ddl_source_id
             FROM media WHERE media_type IN ('movie', 'tvshow')"
        );

        if let Some(cloud) = is_cloud {
            if cloud {
                sql.push_str(" AND is_cloud = 1");
            } else {
                sql.push_str(" AND (is_cloud = 0 OR is_cloud IS NULL)");
            }
        }

        // Only exclude remote:// items when explicitly requesting local-only content
        if is_cloud == Some(false) {
            sql.push_str(" AND (file_path IS NULL OR file_path NOT LIKE 'remote://%')");
        }
        sql.push_str(" ORDER BY id DESC LIMIT ?");

        let mut stmt = self.conn.prepare(&sql)?;
        let items = stmt.query_map(params![limit], Self::map_media_item)?;

        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn search_media_by_cast(&self, cast_name: &str) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, imdb_id, episode_title, still_path,
                    archive_format,
                    is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, ddl_source_id
             FROM media WHERE cast_names LIKE '%' || ? || '%' AND media_type IN ('movie', 'tvshow')
             ORDER BY last_watched DESC",
        )?;

        let items = stmt.query_map(params![cast_name], Self::map_media_item)?;

        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn get_episodes(&self, series_id: i64) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, imdb_id, episode_title, still_path,
                    archive_format,
                    is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, file_size_bytes, ddl_source_id
             FROM media WHERE parent_id = ? ORDER BY season_number, episode_number",
        )?;

        let items = stmt.query_map(params![series_id], Self::map_media_item)?;
        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn get_top_space_consumers(&self, limit: i32) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, imdb_id, episode_title, still_path,
                    archive_format,
                    is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, file_size_bytes, ddl_source_id
             FROM media WHERE file_size_bytes IS NOT NULL AND media_type IN ('movie', 'tvshow')
             ORDER BY file_size_bytes DESC LIMIT ?",
        )?;
        let items = stmt.query_map(params![limit], Self::map_media_item)?;
        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn get_continue_watching_items(&self, limit: i32) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                m.id,
                CASE WHEN m.media_type = 'tvepisode' THEN p.title ELSE m.title END as title,
                CASE WHEN m.media_type = 'tvepisode' THEN p.year ELSE m.year END as year,
                m.overview,
                CASE WHEN m.media_type = 'tvepisode' THEN p.cast_names ELSE m.cast_names END as cast_names,
                CASE WHEN m.media_type = 'tvepisode' THEN p.director ELSE m.director END as director,
                CASE WHEN m.media_type = 'tvepisode' THEN p.poster_path ELSE m.poster_path END as poster_path,
                m.file_path,
                m.media_type,
                m.duration_seconds,
                m.resume_position_seconds,
                m.last_watched,
                m.season_number,
                m.episode_number,
                m.parent_id,
                CASE WHEN m.media_type = 'tvepisode' THEN p.tmdb_id ELSE m.tmdb_id END as tmdb_id,
                m.episode_title,
                m.still_path,
                m.archive_format,
                m.is_cloud,
                m.cloud_file_id,
                m.parent_zip_id,
                m.zip_entry_path,
                m.zip_local_header_offset,
                m.zip_data_start_offset,
                m.zip_compressed_size,
                m.zip_uncompressed_size,
                m.zip_crc32,
                m.zip_compression_method,
                m.ddl_source_id
             FROM media m
             LEFT JOIN media p ON m.parent_id = p.id
             WHERE COALESCE(m.resume_position_seconds, 0) > 0
               AND m.last_watched IS NOT NULL
               AND m.media_type IN ('movie', 'tvepisode')
               AND (m.duration_seconds IS NULL
                    OR m.duration_seconds <= 0
                    OR (m.resume_position_seconds / m.duration_seconds) < 0.93)
             ORDER BY m.last_watched DESC
             LIMIT ?"
        )?;

        let items = stmt.query_map(params![limit], Self::map_media_item)?;
        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn get_watch_history(&self, limit: i32) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                m.id,
                CASE WHEN m.media_type = 'tvepisode' THEN p.title ELSE m.title END as title,
                CASE WHEN m.media_type = 'tvepisode' THEN p.year ELSE m.year END as year,
                m.overview,
                CASE WHEN m.media_type = 'tvepisode' THEN p.cast_names ELSE m.cast_names END as cast_names,
                CASE WHEN m.media_type = 'tvepisode' THEN p.director ELSE m.director END as director,
                CASE WHEN m.media_type = 'tvepisode' THEN p.poster_path ELSE m.poster_path END as poster_path,
                m.file_path,
                m.media_type,
                m.duration_seconds,
                m.resume_position_seconds,
                m.last_watched,
                m.season_number,
                m.episode_number,
                m.parent_id,
                CASE WHEN m.media_type = 'tvepisode' THEN p.tmdb_id ELSE m.tmdb_id END as tmdb_id,
                m.episode_title,
                m.still_path,
                m.archive_format,
                m.is_cloud,
                m.cloud_file_id,
                m.parent_zip_id,
                m.zip_entry_path,
                m.zip_local_header_offset,
                m.zip_data_start_offset,
                m.zip_compressed_size,
                m.zip_uncompressed_size,
                m.zip_crc32,
                m.zip_compression_method,
                m.ddl_source_id
             FROM media m
             LEFT JOIN media p ON m.parent_id = p.id
             WHERE m.last_watched IS NOT NULL
               AND m.media_type IN ('movie', 'tvepisode')
             ORDER BY m.last_watched DESC
             LIMIT ?"
        )?;

        let items = stmt.query_map(params![limit], Self::map_media_item)?;
        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn get_watch_history_events(&self, limit: i32) -> Result<Vec<WatchHistoryEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                event_id,
                media_id,
                parent_media_id,
                title,
                parent_title,
                media_type,
                year,
                overview,
                poster_path,
                still_path,
                tmdb_id,
                parent_tmdb_id,
                episode_title,
                season_number,
                episode_number,
                is_cloud,
                progress_percent,
                resume_position_seconds,
                duration_seconds,
                completed,
                started_at,
                ended_at,
                updated_at
             FROM watch_history_events
             ORDER BY ended_at DESC
             LIMIT ?",
        )?;

        let items = stmt.query_map(params![limit], Self::map_watch_history_event)?;
        items.collect()
    }

    fn reconcile_legacy_watch_history_events(&self) -> Result<usize> {
        let mut stmt = self.conn.prepare(
            "SELECT
                w.event_id,
                w.ended_at,
                COALESCE(m.duration_seconds, 0),
                COALESCE(m.resume_position_seconds, 0),
                m.last_watched
             FROM watch_history_events w
             LEFT JOIN media m ON m.id = w.media_id
             WHERE w.completed = 0
               AND COALESCE(w.progress_percent, 0) <= 0
               AND COALESCE(w.resume_position_seconds, 0) <= 0
               AND COALESCE(w.duration_seconds, 0) <= 0
               AND w.media_id IS NOT NULL",
        )?;

        let candidates = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;

        let mut repaired = 0usize;

        for candidate in candidates {
            let (event_id, ended_at, media_duration, media_resume, media_last_watched) = candidate?;

            if media_duration <= 0.0 {
                continue;
            }

            let Some(last_watched) = media_last_watched else {
                continue;
            };

            if last_watched != ended_at {
                continue;
            }

            let completed = media_resume <= 0.0;
            let progress_percent = if completed {
                100.0
            } else {
                ((media_resume / media_duration) * 100.0).clamp(0.0, 100.0)
            };
            let resume_position_seconds = if completed { 0.0 } else { media_resume };

            self.conn.execute(
                "UPDATE watch_history_events
                 SET
                    progress_percent = ?,
                    resume_position_seconds = ?,
                    duration_seconds = ?,
                    completed = ?,
                    updated_at = CASE
                        WHEN datetime(updated_at) > datetime('now') THEN updated_at
                        ELSE datetime('now')
                    END
                 WHERE event_id = ?",
                params![
                    progress_percent,
                    resume_position_seconds,
                    media_duration,
                    if completed { 1 } else { 0 },
                    event_id,
                ],
            )?;

            repaired += 1;
        }

        Ok(repaired)
    }

    pub fn get_media_by_id(&self, id: i64) -> Result<MediaItem> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    archive_format,
                    is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, file_size_bytes, ddl_source_id
             FROM media WHERE id = ?",
        )?;

        stmt.query_row(params![id], Self::map_media_item)
    }

    pub fn get_resume_info(&self, media_id: i64) -> Result<ResumeInfo> {
        let mut stmt = self
            .conn
            .prepare("SELECT resume_position_seconds, duration_seconds FROM media WHERE id = ?")?;

        let (position, duration): (f64, f64) = stmt.query_row(params![media_id], |row| {
            Ok((
                row.get::<_, Option<f64>>(0)?.unwrap_or(0.0),
                row.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
            ))
        })?;

        let progress_percent = if duration > 0.0 {
            (position / duration) * 100.0
        } else {
            0.0
        };

        // Don't return progress once it should count as watched.
        if progress_percent > AUTO_MARK_WATCHED_THRESHOLD_PERCENT {
            return Ok(ResumeInfo {
                has_progress: false,
                position: 0.0,
                duration,
                time_str: "00:00:00".to_string(),
                progress_percent: 0.0,
            });
        }

        let has_progress = position > 0.0 && duration > 0.0;

        let hours = (position / 3600.0) as i32;
        let minutes = ((position % 3600.0) / 60.0) as i32;
        let seconds = (position % 60.0) as i32;
        let time_str = format!("{:02}:{:02}:{:02}", hours, minutes, seconds);

        Ok(ResumeInfo {
            has_progress,
            position,
            duration,
            time_str,
            progress_percent,
        })
    }

    pub fn update_progress(&self, media_id: i64, current_time: f64, duration: f64) -> Result<()> {
        // Clear progress once playback is close enough to count as watched.
        let progress_percent = if duration > 0.0 {
            current_time / duration
        } else {
            0.0
        };

        if progress_percent > AUTO_MARK_WATCHED_THRESHOLD_RATIO {
            self.conn.execute(
                "UPDATE media SET resume_position_seconds = 0, duration_seconds = ?, 
                 last_watched = datetime('now') WHERE id = ?",
                params![duration, media_id],
            )?;
        } else {
            self.conn.execute(
                "UPDATE media SET resume_position_seconds = ?, 
                 duration_seconds = CASE WHEN ? > 0 THEN ? ELSE duration_seconds END,
                 last_watched = datetime('now') WHERE id = ?",
                params![current_time, duration, duration, media_id],
            )?;
        }

        // Only record history events for plays >= 10 seconds to filter out
        // accidental clicks and brief previews.
        if current_time >= 10.0 {
            self.record_watch_event(media_id, current_time, duration)?;
        }

        Ok(())
    }

    pub fn record_watch_event(
        &self,
        media_id: i64,
        current_time: f64,
        duration: f64,
    ) -> Result<()> {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let mut stmt = self.conn.prepare(
            "SELECT
                m.id,
                m.parent_id,
                m.title,
                p.title,
                m.media_type,
                CASE WHEN m.media_type = 'tvepisode' THEN p.year ELSE m.year END,
                m.overview,
                CASE WHEN m.media_type = 'tvepisode' THEN p.poster_path ELSE m.poster_path END,
                m.still_path,
                m.tmdb_id,
                p.tmdb_id,
                m.episode_title,
                m.season_number,
                m.episode_number,
                COALESCE(m.is_cloud, 0)
             FROM media m
             LEFT JOIN media p ON m.parent_id = p.id
             WHERE m.id = ?",
        )?;

        let snapshot = stmt.query_row(params![media_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<i32>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<i32>>(12)?,
                row.get::<_, Option<i32>>(13)?,
                row.get::<_, i64>(14)? != 0,
            ))
        })?;

        let completed =
            duration > 0.0 && (current_time / duration) > AUTO_MARK_WATCHED_THRESHOLD_RATIO;
        let progress_percent = if duration > 0.0 {
            if completed {
                100.0
            } else {
                (current_time / duration) * 100.0
            }
        } else {
            0.0
        };
        let resume_position_seconds = if completed {
            0.0
        } else {
            current_time.max(0.0)
        };
        let duration_seconds = duration.max(0.0);

        let existing: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT event_id, started_at
                 FROM watch_history_events
                 WHERE media_id = ?
                   AND datetime(updated_at) >= datetime('now', '-20 minutes')
                 ORDER BY updated_at DESC
                 LIMIT 1",
                params![media_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        if let Some((event_id, started_at)) = existing {
            self.conn.execute(
                "UPDATE watch_history_events
                 SET
                    parent_media_id = ?,
                    title = ?,
                    parent_title = ?,
                    media_type = ?,
                    year = ?,
                    overview = ?,
                    poster_path = ?,
                    still_path = ?,
                    tmdb_id = ?,
                    parent_tmdb_id = ?,
                    episode_title = ?,
                    season_number = ?,
                    episode_number = ?,
                    is_cloud = ?,
                    progress_percent = ?,
                    resume_position_seconds = ?,
                    duration_seconds = ?,
                    completed = ?,
                    started_at = ?,
                    ended_at = ?,
                    updated_at = ?
                 WHERE event_id = ?",
                params![
                    snapshot.1,
                    snapshot.2,
                    snapshot.3,
                    snapshot.4,
                    snapshot.5,
                    snapshot.6,
                    snapshot.7,
                    snapshot.8,
                    snapshot.9,
                    snapshot.10,
                    snapshot.11,
                    snapshot.12,
                    snapshot.13,
                    if snapshot.14 { 1 } else { 0 },
                    progress_percent,
                    resume_position_seconds,
                    duration_seconds,
                    if completed { 1 } else { 0 },
                    started_at,
                    now,
                    now,
                    event_id,
                ],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO watch_history_events (
                    event_id,
                    media_id,
                    parent_media_id,
                    title,
                    parent_title,
                    media_type,
                    year,
                    overview,
                    poster_path,
                    still_path,
                    tmdb_id,
                    parent_tmdb_id,
                    episode_title,
                    season_number,
                    episode_number,
                    is_cloud,
                    progress_percent,
                    resume_position_seconds,
                    duration_seconds,
                    completed,
                    started_at,
                    ended_at,
                    updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    snapshot.0,
                    snapshot.1,
                    snapshot.2,
                    snapshot.3,
                    snapshot.4,
                    snapshot.5,
                    snapshot.6,
                    snapshot.7,
                    snapshot.8,
                    snapshot.9,
                    snapshot.10,
                    snapshot.11,
                    snapshot.12,
                    snapshot.13,
                    if snapshot.14 { 1 } else { 0 },
                    progress_percent,
                    resume_position_seconds,
                    duration_seconds,
                    if completed { 1 } else { 0 },
                    now,
                    now,
                    now,
                ],
            )?;
        }

        Ok(())
    }

    pub fn clear_progress(&self, media_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE media SET resume_position_seconds = 0 WHERE id = ?",
            params![media_id],
        )?;
        Ok(())
    }

    pub fn update_last_watched(&self, media_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE media SET last_watched = datetime('now') WHERE id = ?",
            params![media_id],
        )?;
        Ok(())
    }

    /// Remove a single item from watch history by clearing its last_watched timestamp
    pub fn remove_from_watch_history(&self, media_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE media SET last_watched = NULL, resume_position_seconds = 0 WHERE id = ?",
            params![media_id],
        )?;
        self.conn.execute(
            "DELETE FROM watch_history_events WHERE media_id = ?",
            params![media_id],
        )?;
        Ok(())
    }

    pub fn remove_watch_history_event(&self, event_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM watch_history_events WHERE event_id = ?",
            params![event_id],
        )?;
        Ok(())
    }

    /// Clear all watch history by resetting last_watched for all items
    pub fn clear_all_watch_history(&self) -> Result<i32> {
        let count = self.conn.execute(
            "UPDATE media SET last_watched = NULL, resume_position_seconds = 0 WHERE last_watched IS NOT NULL",
            [],
        )?;
        self.conn.execute("DELETE FROM watch_history_events", [])?;
        Ok(count as i32)
    }

    pub fn upsert_watch_history_events(&self, events: &[WatchHistoryEvent]) -> Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }

        self.conn.execute("BEGIN", [])?;

        let mut merged = 0usize;
        {
            let mut stmt = self.conn.prepare(
                "INSERT INTO watch_history_events (
                    event_id, media_id, parent_media_id, title, parent_title,
                    media_type, year, overview, poster_path, still_path,
                    tmdb_id, parent_tmdb_id, episode_title, season_number, episode_number,
                    is_cloud, progress_percent, resume_position_seconds, duration_seconds,
                    completed, started_at, ended_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(event_id) DO UPDATE SET
                    media_id = excluded.media_id,
                    parent_media_id = excluded.parent_media_id,
                    title = excluded.title,
                    parent_title = excluded.parent_title,
                    media_type = excluded.media_type,
                    year = excluded.year,
                    overview = excluded.overview,
                    poster_path = excluded.poster_path,
                    still_path = excluded.still_path,
                    tmdb_id = excluded.tmdb_id,
                    parent_tmdb_id = excluded.parent_tmdb_id,
                    episode_title = excluded.episode_title,
                    season_number = excluded.season_number,
                    episode_number = excluded.episode_number,
                    is_cloud = excluded.is_cloud,
                    progress_percent = excluded.progress_percent,
                    resume_position_seconds = excluded.resume_position_seconds,
                    duration_seconds = excluded.duration_seconds,
                    completed = excluded.completed,
                    started_at = excluded.started_at,
                    ended_at = excluded.ended_at,
                    updated_at = excluded.updated_at
                WHERE excluded.updated_at > watch_history_events.updated_at",
            )?;

            for event in events {
                let rows = stmt.execute(params![
                    event.event_id,
                    event.media_id,
                    event.parent_media_id,
                    event.title,
                    event.parent_title,
                    event.media_type,
                    event.year,
                    event.overview,
                    event.poster_path,
                    event.still_path,
                    event.tmdb_id,
                    event.parent_tmdb_id,
                    event.episode_title,
                    event.season_number,
                    event.episode_number,
                    if event.is_cloud { 1 } else { 0 },
                    event.progress_percent,
                    event.resume_position_seconds,
                    event.duration_seconds,
                    if event.completed { 1 } else { 0 },
                    event.started_at,
                    event.ended_at,
                    event.updated_at,
                ])?;
                merged += rows;
            }
        }

        self.conn.execute("COMMIT", [])?;
        Ok(merged)
    }

    // ==================== STREAMING HISTORY FUNCTIONS ====================

    /// Save or update streaming history entry
    pub fn save_streaming_progress(
        &self,
        tmdb_id: &str,
        media_type: &str,
        title: &str,
        poster_path: Option<&str>,
        season: Option<i32>,
        episode: Option<i32>,
        position: f64,
        duration: f64,
    ) -> Result<()> {
        // First try to find existing entry using COALESCE for NULL-safe comparison
        let existing_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM streaming_history
             WHERE tmdb_id = ? AND media_type = ?
             AND COALESCE(season, -1) = COALESCE(?, -1)
             AND COALESCE(episode, -1) = COALESCE(?, -1)",
                params![tmdb_id, media_type, season, episode],
                |row| row.get(0),
            )
            .ok();

        if let Some(id) = existing_id {
            // Update existing entry
            self.conn.execute(
                "UPDATE streaming_history SET
                    title = ?,
                    poster_path = COALESCE(?, poster_path),
                    resume_position_seconds = ?,
                    duration_seconds = CASE WHEN ? > 0 THEN ? ELSE duration_seconds END,
                    last_watched = datetime('now')
                 WHERE id = ?",
                params![title, poster_path, position, duration, duration, id],
            )?;
        } else {
            // Insert new entry
            self.conn.execute(
                "INSERT INTO streaming_history (tmdb_id, media_type, title, poster_path, season, episode, resume_position_seconds, duration_seconds, last_watched)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))",
                params![tmdb_id, media_type, title, poster_path, season, episode, position, duration],
            )?;
        }
        Ok(())
    }

    /// Get streaming history (most recent first)
    pub fn get_streaming_history(&self, limit: i32) -> Result<Vec<StreamingHistoryItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, tmdb_id, media_type, title, poster_path, season, episode, 
                    resume_position_seconds, duration_seconds, last_watched
             FROM streaming_history 
             ORDER BY last_watched DESC 
             LIMIT ?",
        )?;

        let items = stmt.query_map(params![limit], |row| {
            let duration: f64 = row.get::<_, f64>(8).unwrap_or(0.0);
            let position: f64 = row.get::<_, f64>(7).unwrap_or(0.0);
            let progress_percent = if duration > 0.0 {
                (position / duration) * 100.0
            } else {
                0.0
            };

            Ok(StreamingHistoryItem {
                id: row.get(0)?,
                tmdb_id: row.get(1)?,
                media_type: row.get(2)?,
                title: row.get(3)?,
                poster_path: row.get(4)?,
                season: row.get(5)?,
                episode: row.get(6)?,
                resume_position_seconds: position,
                duration_seconds: duration,
                progress_percent,
                last_watched: row.get(9)?,
            })
        })?;

        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    /// Get streaming resume info for a specific content
    pub fn get_streaming_resume_info(
        &self,
        tmdb_id: &str,
        media_type: &str,
        season: Option<i32>,
        episode: Option<i32>,
    ) -> Result<Option<StreamingHistoryItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, tmdb_id, media_type, title, poster_path, season, episode, 
                    resume_position_seconds, duration_seconds, last_watched
             FROM streaming_history 
             WHERE tmdb_id = ? AND media_type = ? AND 
                   (season IS ? OR (season IS NULL AND ? IS NULL)) AND 
                   (episode IS ? OR (episode IS NULL AND ? IS NULL))",
        )?;

        match stmt.query_row(
            params![tmdb_id, media_type, season, season, episode, episode],
            |row| {
                let duration: f64 = row.get::<_, f64>(8).unwrap_or(0.0);
                let position: f64 = row.get::<_, f64>(7).unwrap_or(0.0);
                let progress_percent = if duration > 0.0 {
                    (position / duration) * 100.0
                } else {
                    0.0
                };

                Ok(StreamingHistoryItem {
                    id: row.get(0)?,
                    tmdb_id: row.get(1)?,
                    media_type: row.get(2)?,
                    title: row.get(3)?,
                    poster_path: row.get(4)?,
                    season: row.get(5)?,
                    episode: row.get(6)?,
                    resume_position_seconds: position,
                    duration_seconds: duration,
                    progress_percent,
                    last_watched: row.get(9)?,
                })
            },
        ) {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Remove a single item from streaming history
    pub fn remove_from_streaming_history(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM streaming_history WHERE id = ?", params![id])?;
        Ok(())
    }

    /// Clear all streaming history
    pub fn clear_all_streaming_history(&self) -> Result<i32> {
        let count = self.conn.execute("DELETE FROM streaming_history", [])?;
        Ok(count as i32)
    }

    pub fn update_metadata(
        &self,
        media_id: i64,
        metadata: &super::tmdb::TmdbMetadata,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE media
             SET title = ?,
                 year = ?,
                 overview = ?,
                 cast_names = ?,
                 director = ?,
                 poster_path = ?,
                 tmdb_id = ?,
                 imdb_id = ?,
                 duration_seconds = CASE
                     WHEN duration_seconds <= 0 AND ? > 0 THEN ?
                     ELSE duration_seconds
                 END
             WHERE id = ?",
            params![
                metadata.title,
                metadata.year,
                metadata.overview,
                metadata.cast_names,
                metadata.director,
                metadata.poster_path,
                metadata.tmdb_id,
                metadata.imdb_id,
                metadata.runtime_seconds.unwrap_or(0.0),
                metadata.runtime_seconds.unwrap_or(0.0),
                media_id
            ],
        )?;
        Ok(())
    }

    // Batch transaction helpers for bulk enrichment
    pub fn begin_batch(&self) -> Result<()> {
        self.conn.execute("BEGIN", [])?;
        Ok(())
    }

    pub fn commit_batch(&self) -> Result<()> {
        self.conn.execute("COMMIT", [])?;
        Ok(())
    }

    pub fn update_poster_path(&self, id: i64, poster_path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE media SET poster_path = ?1 WHERE id = ?2",
            rusqlite::params![poster_path, id],
        )?;
        Ok(())
    }

    pub fn media_exists(&self, file_path: &str) -> Result<bool> {
        let path = std::path::Path::new(file_path);
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let canonical_str = canonical_path.to_string_lossy().to_string();
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM media WHERE file_path = ?")?;
        let exists = stmt.exists(params![canonical_str])?;
        Ok(exists)
    }

    /// Get all file paths currently in the database (for folder tracker sync)
    /// Only returns actual file paths (excludes TV series parent entries which don't have real file paths)
    pub fn get_all_file_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_path FROM media
             WHERE file_path IS NOT NULL
             AND file_path != ''
             AND media_type != 'tvshow'
             AND (file_path LIKE '%.mkv'
                  OR file_path LIKE '%.mp4'
                  OR file_path LIKE '%.avi'
                  OR file_path LIKE '%.mov'
                  OR file_path LIKE '%.webm'
                  OR file_path LIKE '%.m4v'
                  OR file_path LIKE '%.wmv'
                  OR file_path LIKE '%.flv'
                  OR file_path LIKE '%.ts'
                  OR file_path LIKE '%.m2ts')",
        )?;

        let paths = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(paths)
    }

    /// Get media item by file path - used for file watcher to identify media for removal
    pub fn get_media_by_file_path(&self, file_path: &str) -> Result<Option<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    is_cloud, cloud_file_id
             FROM media WHERE file_path = ?",
        )?;

        match stmt.query_row(params![file_path], Self::map_media_item) {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Remove media by file path and return image paths for cleanup
    pub fn remove_media_by_file_path(
        &self,
        file_path: &str,
    ) -> Result<Option<(i64, String, Option<String>, Option<String>)>> {
        // First get the media info so we can return it for cleanup
        let media_info: Option<(i64, String, Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT id, title, poster_path, still_path FROM media WHERE file_path = ?",
                params![file_path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok();

        if let Some((id, _, _, _)) = &media_info {
            // Delete the entry
            self.conn
                .execute("DELETE FROM media WHERE id = ?", params![id])?;
        }

        Ok(media_info)
    }

    /// Check if a TV show series still has any episodes after removal
    /// If not, it should also be removed
    pub fn cleanup_empty_series(&self) -> Result<Vec<(i64, Option<String>)>> {
        // Find tvshows with no episodes
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.poster_path FROM media m
             WHERE m.media_type = 'tvshow'
             AND NOT EXISTS (SELECT 1 FROM media e WHERE e.parent_id = m.id)",
        )?;

        let empty_series: Vec<(i64, Option<String>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        // Delete empty series
        for (id, _) in &empty_series {
            self.conn
                .execute("DELETE FROM media WHERE id = ?", params![id])?;
        }

        Ok(empty_series)
    }

    pub fn find_series_by_folder(&self, folder_path: &str) -> Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM media WHERE file_path = ? AND media_type = 'tvshow'")?;

        match stmt.query_row(params![folder_path], |row| row.get::<_, i64>(0)) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Find a TV show series by TMDB ID first, then by normalized title as fallback.
    /// This allows consolidating episodes from different directories under the same series.
    pub fn find_series_by_tmdb_or_title(
        &self,
        tmdb_id: Option<&str>,
        title: &str,
        year: Option<i32>,
    ) -> Result<Option<i64>> {
        // First, try to find by TMDB ID if available (most reliable match)
        if let Some(tid) = tmdb_id {
            if !tid.is_empty() {
                let mut stmt = self
                    .conn
                    .prepare("SELECT id FROM media WHERE tmdb_id = ? AND media_type = 'tvshow'")?;

                if let Ok(id) = stmt.query_row(params![tid], |row| row.get::<_, i64>(0)) {
                    return Ok(Some(id));
                }
            }
        }

        // Normalize the search title for better matching
        let normalized_title = Self::normalize_title_for_db(title);

        // Fallback: try to find by title and year (case-insensitive)
        // First try with exact year match
        if let Some(y) = year {
            let mut stmt = self.conn.prepare(
                "SELECT id FROM media WHERE LOWER(title) = LOWER(?) AND year = ? AND media_type = 'tvshow'"
            )?;
            if let Ok(id) = stmt.query_row(params![title, y], |row| row.get::<_, i64>(0)) {
                return Ok(Some(id));
            }

            // Try with normalized title
            let mut stmt2 = self.conn.prepare(
                "SELECT id FROM media WHERE LOWER(title) = LOWER(?) AND year = ? AND media_type = 'tvshow'"
            )?;
            if let Ok(id) =
                stmt2.query_row(params![&normalized_title, y], |row| row.get::<_, i64>(0))
            {
                return Ok(Some(id));
            }

            // Try with year ±1 (common for releases spanning year boundary)
            let mut stmt3 = self.conn.prepare(
                "SELECT id FROM media WHERE LOWER(title) = LOWER(?) AND (year = ? OR year = ? OR year = ?) AND media_type = 'tvshow'"
            )?;
            if let Ok(id) =
                stmt3.query_row(params![title, y, y - 1, y + 1], |row| row.get::<_, i64>(0))
            {
                return Ok(Some(id));
            }
        }

        // Try matching by just title (without year) - useful when year isn't in filename
        let mut stmt4 = self.conn.prepare(
            "SELECT id FROM media WHERE LOWER(title) = LOWER(?) AND media_type = 'tvshow'",
        )?;
        if let Ok(id) = stmt4.query_row(params![title], |row| row.get::<_, i64>(0)) {
            return Ok(Some(id));
        }

        // Try with normalized title without year
        let mut stmt5 = self.conn.prepare(
            "SELECT id FROM media WHERE LOWER(title) = LOWER(?) AND media_type = 'tvshow'",
        )?;
        if let Ok(id) = stmt5.query_row(params![&normalized_title], |row| row.get::<_, i64>(0)) {
            return Ok(Some(id));
        }

        // Final attempt: fuzzy match using LIKE with the first significant word
        let first_word = normalized_title
            .split_whitespace()
            .next()
            .unwrap_or(&normalized_title);
        if first_word.len() >= 3 {
            let mut stmt6 = self.conn.prepare(
                "SELECT id, title FROM media WHERE LOWER(title) LIKE ? AND media_type = 'tvshow'",
            )?;
            let pattern = format!("{}%", first_word.to_lowercase());

            let result: Result<(i64, String), _> =
                stmt6.query_row(params![pattern], |row| Ok((row.get(0)?, row.get(1)?)));

            if let Ok((id, db_title)) = result {
                // Check if the titles are similar enough
                if Self::titles_are_similar(&normalized_title, &db_title) {
                    return Ok(Some(id));
                }
            }
        }

        Ok(None)
    }

    /// Normalize a title for database comparison
    fn normalize_title_for_db(title: &str) -> String {
        let mut normalized = title.to_lowercase();

        // Replace common variations
        normalized = normalized.replace('&', "and");
        normalized = normalized.replace("'", "");
        normalized = normalized.replace("'", "");
        normalized = normalized.replace(":", "");
        normalized = normalized.replace("-", " ");
        normalized = normalized.replace("_", " ");
        normalized = normalized.replace(".", " ");

        // Remove leading "the"
        if normalized.starts_with("the ") {
            normalized = normalized[4..].to_string();
        }

        // Collapse whitespace
        normalized.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Check if two titles are similar enough to be the same series
    fn titles_are_similar(a: &str, b: &str) -> bool {
        let norm_a = Self::normalize_title_for_db(a);
        let norm_b = Self::normalize_title_for_db(b);

        if norm_a == norm_b {
            return true;
        }

        // Check word overlap with stricter thresholds to avoid false merges
        let words_a: std::collections::HashSet<&str> = norm_a.split_whitespace().collect();
        let words_b: std::collections::HashSet<&str> = norm_b.split_whitespace().collect();

        let len_a = words_a.len();
        let len_b = words_b.len();

        // If either title is a single word, only an exact match should pass (handled above)
        if len_a <= 1 || len_b <= 1 {
            return false;
        }

        let intersection = words_a.intersection(&words_b).count();
        if intersection == 0 {
            return false;
        }

        let union = words_a.union(&words_b).count();
        let jaccard = intersection as f32 / union as f32;
        let smaller = len_a.min(len_b);

        // For short titles (2 words), require all words to match
        if smaller <= 2 {
            return intersection == smaller;
        }

        // For longer titles, require high overlap and at least 2 matching words
        intersection >= smaller.saturating_sub(1) && intersection >= 2 && jaccard >= 0.6
    }

    pub fn insert_movie(
        &self,
        title: &str,
        year: Option<i32>,
        overview: Option<&str>,
        cast_names: Option<&str>,
        director: Option<&str>,
        poster_path: Option<&str>,
        file_path: &str,
        duration: f64,
        tmdb_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO media (title, year, overview, cast_names, director, poster_path, file_path, media_type, duration_seconds, tmdb_id) 
             VALUES (?, ?, ?, ?, ?, ?, ?, 'movie', ?, ?)",
            params![title, year, overview, cast_names, director, poster_path, file_path, duration, tmdb_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_tvshow(
        &self,
        title: &str,
        year: Option<i32>,
        overview: Option<&str>,
        cast_names: Option<&str>,
        poster_path: Option<&str>,
        folder_path: &str,
        tmdb_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO media (title, year, overview, cast_names, poster_path, file_path, media_type, tmdb_id) 
             VALUES (?, ?, ?, ?, ?, ?, 'tvshow', ?)",
            params![title, year, overview, cast_names, poster_path, folder_path, tmdb_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_episode(
        &self,
        title: &str,
        file_path: &str,
        parent_id: i64,
        season: i32,
        episode: i32,
        duration: f64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO media (title, file_path, media_type, parent_id, season_number, episode_number, duration_seconds)
             VALUES (?, ?, 'tvepisode', ?, ?, ?, ?)",
            params![title, file_path, parent_id, season, episode, duration],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert episode with full metadata (title, overview, still image)
    pub fn insert_episode_with_metadata(
        &self,
        title: &str,
        file_path: &str,
        parent_id: i64,
        season: i32,
        episode: i32,
        duration: f64,
        episode_title: Option<&str>,
        overview: Option<&str>,
        still_path: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO media (title, file_path, media_type, parent_id, season_number, episode_number, duration_seconds, episode_title, overview, still_path)
             VALUES (?, ?, 'tvepisode', ?, ?, ?, ?, ?, ?, ?)",
            params![title, file_path, parent_id, season, episode, duration, episode_title, overview, still_path],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Update an existing episode with metadata
    pub fn update_episode_metadata(
        &self,
        episode_id: i64,
        episode_title: Option<&str>,
        overview: Option<&str>,
        still_path: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE media SET episode_title = ?, overview = ?, still_path = ? WHERE id = ?",
            params![episode_title, overview, still_path, episode_id],
        )?;
        Ok(())
    }

    /// Update only the still_path for an episode identified by parent + season + episode number
    pub fn update_episode_still_path(
        &self,
        parent_id: i64,
        season: i32,
        episode: i32,
        still_path: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE media SET still_path = ?1 WHERE parent_id = ?2 AND season_number = ?3 AND episode_number = ?4",
            rusqlite::params![still_path, parent_id, season, episode],
        )?;
        Ok(())
    }

    // ==================== CLOUD MEDIA METHODS ====================

    /// Insert a cloud movie
    /// Uses cloud_file_id as unique file_path to avoid collisions across folders
    pub fn insert_cloud_movie(
        &self,
        title: &str,
        year: Option<i32>,
        overview: Option<&str>,
        cast_names: Option<&str>,
        director: Option<&str>,
        poster_path: Option<&str>,
        _file_name: &str,
        cloud_file_id: &str,
        cloud_folder_id: &str,
        duration: f64,
        tmdb_id: Option<&str>,
    ) -> Result<i64> {
        // Use cloud_file_id as file_path for unique identification across folders
        let unique_path = format!("gdrive:{}", cloud_file_id);
        self.conn.execute(
            "INSERT INTO media (title, year, overview, cast_names, director, poster_path, file_path, media_type, duration_seconds, tmdb_id, is_cloud, cloud_file_id, cloud_folder_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'movie', ?, ?, 1, ?, ?)
             ON CONFLICT(file_path) DO UPDATE SET
                title = excluded.title,
                year = excluded.year,
                overview = excluded.overview,
                cast_names = excluded.cast_names,
                director = excluded.director,
                poster_path = excluded.poster_path,
                duration_seconds = excluded.duration_seconds,
                tmdb_id = excluded.tmdb_id,
                cloud_folder_id = excluded.cloud_folder_id",
            params![title, year, overview, cast_names, director, poster_path, unique_path, duration, tmdb_id, cloud_file_id, cloud_folder_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a cloud TV show
    pub fn insert_cloud_tvshow(
        &self,
        title: &str,
        year: Option<i32>,
        overview: Option<&str>,
        cast_names: Option<&str>,
        poster_path: Option<&str>,
        folder_name: &str,
        cloud_folder_id: &str,
        tmdb_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO media (title, year, overview, cast_names, poster_path, file_path, media_type, tmdb_id, is_cloud, cloud_folder_id)
             VALUES (?, ?, ?, ?, ?, ?, 'tvshow', ?, 1, ?)
             ON CONFLICT(file_path) DO UPDATE SET
                title = excluded.title,
                year = excluded.year,
                overview = excluded.overview,
                cast_names = excluded.cast_names,
                poster_path = excluded.poster_path,
                tmdb_id = excluded.tmdb_id,
                cloud_folder_id = excluded.cloud_folder_id",
            params![title, year, overview, cast_names, poster_path, folder_name, tmdb_id, cloud_folder_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a cloud episode
    /// Uses cloud_file_id as unique file_path to avoid collisions across folders
    pub fn insert_cloud_episode(
        &self,
        title: &str,
        _file_name: &str,
        parent_id: i64,
        season: i32,
        episode: i32,
        cloud_file_id: &str,
        cloud_folder_id: &str,
        episode_title: Option<&str>,
        overview: Option<&str>,
        still_path: Option<&str>,
        file_size_bytes: Option<i64>,
    ) -> Result<i64> {
        let unique_path = format!("gdrive:{}", cloud_file_id);
        self.conn.execute(
            "INSERT INTO media (title, file_path, media_type, parent_id, season_number, episode_number,
                               is_cloud, cloud_file_id, cloud_folder_id, episode_title, overview, still_path, file_size_bytes)
              VALUES (?, ?, 'tvepisode', ?, ?, ?, 1, ?, ?, ?, ?, ?, ?)
              ON CONFLICT(file_path) DO UPDATE SET
                 title = excluded.title,
                 parent_id = excluded.parent_id,
                 season_number = excluded.season_number,
                 episode_number = excluded.episode_number,
                 cloud_folder_id = excluded.cloud_folder_id,
                 episode_title = excluded.episode_title,
                 overview = excluded.overview,
                 still_path = excluded.still_path,
                 file_size_bytes = excluded.file_size_bytes",
            params![title, unique_path, parent_id, season, episode, cloud_file_id, cloud_folder_id,
                   episode_title, overview, still_path, file_size_bytes],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_zip_archive(&self, archive: &zip_manager::ZipArchiveInfo) -> Result<()> {
        self.conn.execute(
            "INSERT INTO zip_archives (
                zip_file_id, filename, archive_format, file_size_bytes, compression_type,
                central_dir_offset, central_dir_size, total_entries, video_entries,
                last_analyzed
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(zip_file_id) DO UPDATE SET
                filename = excluded.filename,
                archive_format = excluded.archive_format,
                file_size_bytes = excluded.file_size_bytes,
                compression_type = excluded.compression_type,
                central_dir_offset = excluded.central_dir_offset,
                central_dir_size = excluded.central_dir_size,
                total_entries = excluded.total_entries,
                video_entries = excluded.video_entries,
                last_analyzed = CURRENT_TIMESTAMP",
            params![
                &archive.zip_file_id,
                &archive.filename,
                &archive.archive_format,
                archive.file_size_bytes as i64,
                format!("{:?}", archive.compression_type).to_lowercase(),
                archive.central_dir_offset as i64,
                archive.central_dir_size as i64,
                archive.total_entries.try_into().unwrap_or(0),
                archive.video_entries.try_into().unwrap_or(0),
            ],
        )?;
        Ok(())
    }

    pub fn get_zip_archive(&self, zip_file_id: &str) -> Result<ZipArchiveRecord> {
        self.conn.query_row(
            "SELECT zip_file_id, filename, archive_format, file_size_bytes, compression_type,
                    central_dir_offset, central_dir_size, total_entries, video_entries, last_analyzed
             FROM zip_archives WHERE zip_file_id = ?",
            params![zip_file_id],
            |row| {
                Ok(ZipArchiveRecord {
                    zip_file_id: row.get(0)?,
                    filename: row.get(1)?,
                    archive_format: row.get(2)?,
                    file_size_bytes: row.get(3)?,
                    compression_type: row.get(4)?,
                    central_dir_offset: row.get(5)?,
                    central_dir_size: row.get(6)?,
                    total_entries: row.get(7)?,
                    video_entries: row.get(8)?,
                    last_analyzed: row.get(9).unwrap_or(None),
                })
            },
        )
    }

    pub fn insert_cloud_episode_from_zip(
        &self,
        title: &str,
        parent_id: i64,
        season: i32,
        episode: i32,
        cloud_folder_id: &str,
        archive_format: &str,
        zip_file_id: &str,
        zip_entry_path: &str,
        zip_local_header_offset: i64,
        zip_data_start_offset: i64,
        zip_compressed_size: i64,
        zip_uncompressed_size: i64,
        zip_crc32: &str,
        zip_compression_method: i64,
        episode_title: Option<&str>,
        overview: Option<&str>,
        still_path: Option<&str>,
    ) -> Result<i64> {
        let virtual_path = format!("{}://{}/{}", archive_format, zip_file_id, zip_entry_path);

        self.conn.execute(
            "INSERT INTO media (
                title, file_path, media_type, parent_id, season_number, episode_number,
                is_cloud, cloud_file_id, cloud_folder_id, episode_title, overview, still_path,
                archive_format,
                parent_zip_id, zip_entry_path, zip_local_header_offset, zip_data_start_offset,
                zip_compressed_size, zip_uncompressed_size, zip_crc32, zip_compression_method
            ) VALUES (?, ?, 'tvepisode', ?, ?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(file_path) DO UPDATE SET
                title = excluded.title,
                parent_id = excluded.parent_id,
                season_number = excluded.season_number,
                episode_number = excluded.episode_number,
                cloud_folder_id = excluded.cloud_folder_id,
                episode_title = excluded.episode_title,
                overview = excluded.overview,
                still_path = excluded.still_path,
                archive_format = excluded.archive_format,
                parent_zip_id = excluded.parent_zip_id,
                zip_entry_path = excluded.zip_entry_path,
                zip_local_header_offset = excluded.zip_local_header_offset,
                zip_data_start_offset = excluded.zip_data_start_offset,
                zip_compressed_size = excluded.zip_compressed_size,
                zip_uncompressed_size = excluded.zip_uncompressed_size,
                zip_crc32 = excluded.zip_crc32,
                zip_compression_method = excluded.zip_compression_method",
            params![
                title,
                virtual_path,
                parent_id,
                season,
                episode,
                zip_file_id,
                cloud_folder_id,
                episode_title,
                overview,
                still_path,
                archive_format,
                zip_file_id,
                zip_entry_path,
                zip_local_header_offset,
                zip_data_start_offset,
                zip_compressed_size,
                zip_uncompressed_size,
                zip_crc32,
                zip_compression_method,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_zip_archive(&self, zip_file_id: &str) -> Result<usize> {
        self.conn.execute(
            "DELETE FROM media WHERE parent_zip_id = ?",
            params![zip_file_id],
        )?;
        let deleted = self.conn.execute(
            "DELETE FROM zip_archives WHERE zip_file_id = ?",
            params![zip_file_id],
        )?;
        Ok(deleted)
    }

    /// Check if a cloud file already exists in the database
    pub fn cloud_file_exists(&self, cloud_file_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM media WHERE cloud_file_id = ? OR parent_zip_id = ?",
                params![cloud_file_id, cloud_file_id],
                |_| Ok(()),
            )
            .is_ok()
    }

    // ---- Direct Download Link (DDL) methods ----

    pub fn upsert_ddl_source(
        &self,
        id: &str,
        url: &str,
        filename: &str,
        file_size: i64,
        archive_format: &str,
        entry_count: i64,
        video_count: i64,
        cd_offset: i64,
        cd_size: i64,
        addon_origin: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ddl_sources (id, url, filename, file_size, archive_format, entry_count, video_count, cd_offset, cd_size, addon_origin, created_at, last_verified_at, is_expired)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, 0)
             ON CONFLICT(id) DO UPDATE SET
                url = excluded.url,
                filename = excluded.filename,
                file_size = excluded.file_size,
                archive_format = excluded.archive_format,
                entry_count = excluded.entry_count,
                video_count = excluded.video_count,
                cd_offset = excluded.cd_offset,
                cd_size = excluded.cd_size,
                addon_origin = excluded.addon_origin,
                last_verified_at = CURRENT_TIMESTAMP,
                is_expired = 0",
            params![id, url, filename, file_size, archive_format, entry_count, video_count, cd_offset, cd_size, addon_origin],
        )?;
        Ok(())
    }

    pub fn get_ddl_sources(&self) -> Result<Vec<crate::direct_link_manager::DdlSource>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url, filename, file_size, archive_format, entry_count, video_count,
                    cd_offset, cd_size, created_at, last_verified_at, is_expired, addon_origin
             FROM ddl_sources ORDER BY created_at DESC",
        )?;
        let sources = stmt
            .query_map([], |row| {
                Ok(crate::direct_link_manager::DdlSource {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    filename: row.get(2)?,
                    file_size: row.get::<_, i64>(3)? as u64,
                    archive_format: row.get(4)?,
                    entry_count: row.get::<_, i64>(5)? as usize,
                    video_count: row.get::<_, i64>(6)? as usize,
                    cd_offset: row.get::<_, i64>(7)? as u64,
                    cd_size: row.get::<_, i64>(8)? as u64,
                    created_at: row.get(9)?,
                    last_verified_at: row.get(10)?,
                    is_expired: row.get::<_, i64>(11)? != 0,
                    addon_origin: row.get(12)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(sources)
    }

    pub fn get_ddl_source(&self, source_id: &str) -> Result<crate::direct_link_manager::DdlSource> {
        self.conn.query_row(
            "SELECT id, url, filename, file_size, archive_format, entry_count, video_count,
                    cd_offset, cd_size, created_at, last_verified_at, is_expired, addon_origin
             FROM ddl_sources WHERE id = ?",
            params![source_id],
            |row| {
                Ok(crate::direct_link_manager::DdlSource {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    filename: row.get(2)?,
                    file_size: row.get::<_, i64>(3)? as u64,
                    archive_format: row.get(4)?,
                    entry_count: row.get::<_, i64>(5)? as usize,
                    video_count: row.get::<_, i64>(6)? as usize,
                    cd_offset: row.get::<_, i64>(7)? as u64,
                    cd_size: row.get::<_, i64>(8)? as u64,
                    created_at: row.get(9)?,
                    last_verified_at: row.get(10)?,
                    is_expired: row.get::<_, i64>(11)? != 0,
                    addon_origin: row.get(12)?,
                })
            },
        )
    }

    pub fn update_ddl_source_url(&self, source_id: &str, new_url: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE ddl_sources SET url = ?, last_verified_at = CURRENT_TIMESTAMP, is_expired = 0 WHERE id = ?",
            params![new_url, source_id],
        )?;
        Ok(())
    }

    pub fn mark_ddl_source_expired(&self, source_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE ddl_sources SET is_expired = 1 WHERE id = ?",
            params![source_id],
        )?;
        Ok(())
    }

    pub fn delete_ddl_source_and_media(&self, source_id: &str) -> Result<usize> {
        // First delete parent shows that only have DDL episodes
        self.conn.execute(
            "DELETE FROM media WHERE id IN (
                SELECT DISTINCT parent_id FROM media WHERE ddl_source_id = ? AND parent_id IS NOT NULL
            ) AND NOT EXISTS (
                SELECT 1 FROM media child WHERE child.parent_id = media.id AND child.ddl_source_id IS NULL
            )",
            params![source_id],
        )?;
        // Delete the episode/movie entries
        self.conn.execute(
            "DELETE FROM media WHERE ddl_source_id = ?",
            params![source_id],
        )?;
        let deleted = self
            .conn
            .execute("DELETE FROM ddl_sources WHERE id = ?", params![source_id])?;
        Ok(deleted)
    }

    pub fn get_media_by_ddl_source(&self, source_id: &str) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    archive_format,
                    is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, ddl_source_id
             FROM media WHERE ddl_source_id = ?
             ORDER BY season_number, episode_number, title",
        )?;
        let items = stmt
            .query_map(params![source_id], Self::map_media_item)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(items)
    }

    pub fn insert_ddl_episode(
        &self,
        title: &str,
        parent_id: Option<i64>,
        season: Option<i32>,
        episode: Option<i32>,
        ddl_source_id: &str,
        archive_format: &str,
        zip_entry_path: &str,
        zip_local_header_offset: i64,
        zip_data_start_offset: i64,
        zip_compressed_size: i64,
        zip_uncompressed_size: i64,
        zip_crc32: &str,
        zip_compression_method: i64,
        episode_title: Option<&str>,
        episode_overview: Option<&str>,
        episode_still_path: Option<&str>,
    ) -> Result<i64> {
        let virtual_path = format!(
            "ddl://{}:{}/{}",
            ddl_source_id, archive_format, zip_entry_path
        );
        let media_type = if parent_id.is_some() {
            "tvepisode"
        } else {
            "movie"
        };

        self.conn.execute(
            "INSERT INTO media (
                title, file_path, media_type, parent_id, season_number, episode_number,
                is_cloud, archive_format, ddl_source_id,
                parent_zip_id, zip_entry_path, zip_local_header_offset, zip_data_start_offset,
                zip_compressed_size, zip_uncompressed_size, zip_crc32, zip_compression_method,
                episode_title, overview, still_path
            ) VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                title,
                virtual_path,
                media_type,
                parent_id,
                season,
                episode,
                archive_format,
                ddl_source_id,
                ddl_source_id,
                zip_entry_path,
                zip_local_header_offset,
                zip_data_start_offset,
                zip_compressed_size,
                zip_uncompressed_size,
                zip_crc32,
                zip_compression_method,
                episode_title,
                episode_overview,
                episode_still_path,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_ddl_tvshow(
        &self,
        title: &str,
        ddl_source_id: &str,
        year: Option<i32>,
        overview: Option<&str>,
        cast_names: Option<&str>,
        poster_path: Option<&str>,
        tmdb_id: Option<&str>,
    ) -> Result<i64> {
        let virtual_path = format!("ddl://{}:show", ddl_source_id);
        self.conn.execute(
            "INSERT INTO media (title, file_path, media_type, is_cloud, ddl_source_id, year, overview, cast_names, poster_path, tmdb_id)
             VALUES (?, ?, 'tvshow', 1, ?, ?, ?, ?, ?, ?)",
            params![title, virtual_path, ddl_source_id, year, overview, cast_names, poster_path, tmdb_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_ddl_source_url(&self, source_id: &str) -> Result<String> {
        self.conn.query_row(
            "SELECT url FROM ddl_sources WHERE id = ?",
            params![source_id],
            |row| row.get(0),
        )
    }

    pub fn upsert_cloud_index_failure(
        &self,
        cloud_file_id: &str,
        file_name: &str,
        last_error: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO cloud_index_failures (cloud_file_id, file_name, last_error, last_attempt)
             VALUES (?, ?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(cloud_file_id) DO UPDATE SET
                file_name = excluded.file_name,
                last_error = excluded.last_error,
                last_attempt = CURRENT_TIMESTAMP",
            params![cloud_file_id, file_name, last_error],
        )?;
        Ok(())
    }

    pub fn clear_cloud_index_failure(&self, cloud_file_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM cloud_index_failures WHERE cloud_file_id = ?",
            params![cloud_file_id],
        )?;
        Ok(())
    }

    pub fn get_cloud_index_failures(&self, limit: usize) -> Result<Vec<CloudIndexFailure>> {
        let limit = (limit as i64).min(1000);
        let mut stmt = self.conn.prepare(
            "SELECT cloud_file_id, file_name, last_error, COALESCE(last_attempt, '')
             FROM cloud_index_failures
             ORDER BY last_attempt DESC
             LIMIT ?",
        )?;

        let items = stmt.query_map(params![limit], |row| {
            Ok(CloudIndexFailure {
                cloud_file_id: row.get(0)?,
                file_name: row.get(1)?,
                last_error: row.get(2)?,
                last_attempt: row.get(3)?,
            })
        })?;

        items.collect()
    }

    /// Get cloud media by folder ID
    pub fn get_cloud_media_by_folder(&self, cloud_folder_id: &str) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    archive_format, is_cloud, cloud_file_id
             FROM media WHERE cloud_folder_id = ?",
        )?;

        let items = stmt.query_map(params![cloud_folder_id], Self::map_media_item)?;
        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    /// Get all cloud media IDs for logging/indexing purposes
    pub fn get_cloud_media_index_list(
        &self,
    ) -> Result<Vec<(i64, String, Option<String>, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, file_path, cloud_file_id FROM media WHERE is_cloud = 1")?;

        let items = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;

        Ok(items.filter_map(|r| r.ok()).collect())
    }

    /// Delete all cloud media for a folder
    pub fn delete_cloud_folder_media(&self, cloud_folder_id: &str) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM media WHERE cloud_folder_id = ?",
            params![cloud_folder_id],
        )?;
        Ok(deleted)
    }

    /// Check how many media entries are tied to a cloud folder and how many belong to a series
    pub fn get_cloud_folder_usage_counts(
        &self,
        cloud_folder_id: &str,
        series_id: i64,
    ) -> Result<(i64, i64)> {
        let total: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM media WHERE cloud_folder_id = ?",
            params![cloud_folder_id],
            |row| row.get(0),
        )?;

        let series_related: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM media WHERE cloud_folder_id = ? AND (id = ? OR parent_id = ?)",
            params![cloud_folder_id, series_id, series_id],
            |row| row.get(0),
        )?;

        Ok((total, series_related))
    }

    // ==================== CLOUD FOLDER MANAGEMENT ====================

    /// Add a cloud folder to track
    pub fn add_cloud_folder(&self, folder_id: &str, folder_name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO cloud_folders (folder_id, folder_name, auto_scan)
             VALUES (?, ?, 1)
             ON CONFLICT(folder_id) DO UPDATE SET
                folder_name = excluded.folder_name,
                auto_scan = excluded.auto_scan",
            params![folder_id, folder_name],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Remove a cloud folder
    pub fn remove_cloud_folder(&self, folder_id: &str) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM cloud_folders WHERE folder_id = ?",
            params![folder_id],
        )?;
        Ok(deleted)
    }

    /// Get all cloud folders
    pub fn get_cloud_folders(&self) -> Result<Vec<(String, String, bool)>> {
        let mut stmt = self.conn.prepare(
            "SELECT folder_id, folder_name, auto_scan FROM cloud_folders ORDER BY created_at",
        )?;

        let items = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)? == 1,
            ))
        })?;

        items.collect()
    }

    /// Update last scanned timestamp for a folder
    pub fn update_cloud_folder_scanned(&self, folder_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE cloud_folders SET last_scanned = CURRENT_TIMESTAMP WHERE folder_id = ?",
            params![folder_id],
        )?;
        Ok(())
    }

    /// Get all cloud file IDs currently in the database for a folder
    pub fn get_cloud_file_ids_for_folder(&self, folder_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT cloud_file_id FROM media WHERE cloud_folder_id = ? AND cloud_file_id IS NOT NULL"
        )?;

        let items = stmt.query_map(params![folder_id], |row| row.get::<_, String>(0))?;

        items.collect()
    }

    // ==================== APP SETTINGS (for Changes Token etc.) ====================

    /// Get a setting value by key
    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let result = self.conn.query_row(
            "SELECT value FROM app_settings WHERE key = ?",
            params![key],
            |row| row.get::<_, String>(0),
        );

        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Set a setting value
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO app_settings (key, value, updated_at) VALUES (?, ?, CURRENT_TIMESTAMP)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Get the Google Drive changes page token
    pub fn get_gdrive_changes_token(&self) -> Result<Option<String>> {
        self.get_setting("gdrive_changes_token")
    }

    /// Set the Google Drive changes page token
    pub fn set_gdrive_changes_token(&self, token: &str) -> Result<()> {
        self.set_setting("gdrive_changes_token", token)
    }

    // ==================== MOVIE / TV REMINDERS ====================

    pub fn create_movie_reminder(&self, reminder: NewMovieReminder<'_>) -> Result<MovieReminder> {
        self.conn.execute(
            "INSERT INTO movie_reminders (
                tmdb_id, media_type, title, poster_path, season_number, episode_number,
                release_date, reminder_at, source, tracking_mode, tracking_season_number,
                notes, is_active, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)",
            params![
                reminder.tmdb_id,
                reminder.media_type,
                reminder.title,
                reminder.poster_path,
                reminder.season_number,
                reminder.episode_number,
                reminder.release_date,
                reminder.reminder_at,
                reminder.source,
                reminder.tracking_mode,
                reminder.tracking_season_number,
                reminder.notes,
                if reminder.is_active { 1 } else { 0 },
            ],
        )?;

        let id = self.conn.last_insert_rowid();
        self.get_movie_reminder(id)
    }

    pub fn update_movie_reminder(
        &self,
        id: i64,
        reminder: NewMovieReminder<'_>,
    ) -> Result<MovieReminder> {
        self.conn.execute(
            "UPDATE movie_reminders
             SET tmdb_id = ?, media_type = ?, title = ?, poster_path = ?,
                 season_number = ?, episode_number = ?, release_date = ?,
                 reminder_at = ?, source = ?, tracking_mode = ?, tracking_season_number = ?,
                 notes = ?, is_active = ?,
                 notified_at = NULL, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
            params![
                reminder.tmdb_id,
                reminder.media_type,
                reminder.title,
                reminder.poster_path,
                reminder.season_number,
                reminder.episode_number,
                reminder.release_date,
                reminder.reminder_at,
                reminder.source,
                reminder.tracking_mode,
                reminder.tracking_season_number,
                reminder.notes,
                if reminder.is_active { 1 } else { 0 },
                id,
            ],
        )?;

        self.get_movie_reminder(id)
    }

    pub fn get_movie_reminder(&self, id: i64) -> Result<MovieReminder> {
        self.conn.query_row(
            "SELECT id, tmdb_id, media_type, title, poster_path, season_number, episode_number,
                    release_date, reminder_at, source, tracking_mode, tracking_season_number,
                    notes, is_active, notified_at,
                    created_at, updated_at
             FROM movie_reminders WHERE id = ?",
            params![id],
            Self::map_movie_reminder,
        )
    }

    pub fn get_movie_reminders(&self, include_inactive: bool) -> Result<Vec<MovieReminder>> {
        let sql = if include_inactive {
            "SELECT id, tmdb_id, media_type, title, poster_path, season_number, episode_number,
                    release_date, reminder_at, source, tracking_mode, tracking_season_number,
                    notes, is_active, notified_at,
                    created_at, updated_at
             FROM movie_reminders
             ORDER BY reminder_at ASC"
        } else {
            "SELECT id, tmdb_id, media_type, title, poster_path, season_number, episode_number,
                    release_date, reminder_at, source, tracking_mode, tracking_season_number,
                    notes, is_active, notified_at,
                    created_at, updated_at
             FROM movie_reminders
             WHERE is_active = 1
             ORDER BY reminder_at ASC"
        };

        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], Self::map_movie_reminder)?;
        rows.collect()
    }

    pub fn delete_movie_reminder(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM movie_reminders WHERE id = ?", params![id])?;
        Ok(())
    }

    pub fn set_movie_reminder_active(&self, id: i64, is_active: bool) -> Result<MovieReminder> {
        self.conn.execute(
            "UPDATE movie_reminders
             SET is_active = ?, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
            params![if is_active { 1 } else { 0 }, id],
        )?;
        self.get_movie_reminder(id)
    }

    pub fn get_due_movie_reminders(&self, now_utc: &str) -> Result<Vec<MovieReminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, tmdb_id, media_type, title, poster_path, season_number, episode_number,
                    release_date, reminder_at, source, tracking_mode, tracking_season_number,
                    notes, is_active, notified_at,
                    created_at, updated_at
             FROM movie_reminders
             WHERE is_active = 1
               AND reminder_at <= ?
             ORDER BY reminder_at ASC",
        )?;

        let rows = stmt.query_map(params![now_utc], Self::map_movie_reminder)?;
        rows.collect()
    }

    pub fn mark_movie_reminder_notified(&self, id: i64, notified_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE movie_reminders
             SET notified_at = ?, is_active = 0, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
            params![notified_at, id],
        )?;
        Ok(())
    }

    pub fn advance_movie_reminder(
        &self,
        id: i64,
        title: &str,
        poster_path: Option<&str>,
        season_number: Option<i32>,
        episode_number: Option<i32>,
        release_date: Option<&str>,
        reminder_at: &str,
        source: &str,
        tracking_season_number: Option<i32>,
        notified_at: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE movie_reminders
             SET title = ?, poster_path = ?, season_number = ?, episode_number = ?,
                 release_date = ?, reminder_at = ?, source = ?,
                 tracking_season_number = ?, notified_at = ?, is_active = 1,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
            params![
                title,
                poster_path,
                season_number,
                episode_number,
                release_date,
                reminder_at,
                source,
                tracking_season_number,
                notified_at,
                id,
            ],
        )?;
        Ok(())
    }

    pub fn get_watchlist_item(&self, id: i64) -> Result<WatchlistItem> {
        self.conn.query_row(
            "SELECT id, tmdb_id, media_type, title, poster_path, release_date, notes,
                    is_active, notification_enabled, notification_mode,
                    notification_interval_minutes, notify_at, last_notified_at,
                    created_at, updated_at
             FROM watchlist_items
             WHERE id = ?",
            params![id],
            Self::map_watchlist_item,
        )
    }

    pub fn get_watchlist_items(&self, include_inactive: bool) -> Result<Vec<WatchlistItem>> {
        let sql = if include_inactive {
            "SELECT id, tmdb_id, media_type, title, poster_path, release_date, notes,
                    is_active, notification_enabled, notification_mode,
                    notification_interval_minutes, notify_at, last_notified_at,
                    created_at, updated_at
             FROM watchlist_items
             ORDER BY updated_at DESC, created_at DESC"
        } else {
            "SELECT id, tmdb_id, media_type, title, poster_path, release_date, notes,
                    is_active, notification_enabled, notification_mode,
                    notification_interval_minutes, notify_at, last_notified_at,
                    created_at, updated_at
             FROM watchlist_items
             WHERE is_active = 1
             ORDER BY updated_at DESC, created_at DESC"
        };

        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], Self::map_watchlist_item)?;
        rows.collect()
    }

    pub fn create_or_update_watchlist_item(
        &self,
        item: NewWatchlistItem<'_>,
    ) -> Result<WatchlistItem> {
        self.conn.execute(
            "INSERT INTO watchlist_items (
                tmdb_id, media_type, title, poster_path, release_date, notes, is_active,
                notification_enabled, notification_mode, notification_interval_minutes, notify_at,
                updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
             ON CONFLICT(tmdb_id, media_type) DO UPDATE SET
                title = excluded.title,
                poster_path = excluded.poster_path,
                release_date = excluded.release_date,
                notes = excluded.notes,
                is_active = excluded.is_active,
                notification_enabled = excluded.notification_enabled,
                notification_mode = excluded.notification_mode,
                notification_interval_minutes = excluded.notification_interval_minutes,
                notify_at = excluded.notify_at,
                updated_at = CURRENT_TIMESTAMP",
            params![
                item.tmdb_id,
                item.media_type,
                item.title,
                item.poster_path,
                item.release_date,
                item.notes,
                if item.is_active { 1 } else { 0 },
                if item.notification_enabled { 1 } else { 0 },
                item.notification_mode,
                item.notification_interval_minutes,
                item.notify_at,
            ],
        )?;

        self.conn.query_row(
            "SELECT id, tmdb_id, media_type, title, poster_path, release_date, notes,
                    is_active, notification_enabled, notification_mode,
                    notification_interval_minutes, notify_at, last_notified_at,
                    created_at, updated_at
             FROM watchlist_items
             WHERE tmdb_id = ? AND media_type = ?",
            params![item.tmdb_id, item.media_type],
            Self::map_watchlist_item,
        )
    }

    pub fn update_watchlist_item(
        &self,
        id: i64,
        item: NewWatchlistItem<'_>,
    ) -> Result<WatchlistItem> {
        self.conn.execute(
            "UPDATE watchlist_items
             SET tmdb_id = ?, media_type = ?, title = ?, poster_path = ?, release_date = ?,
                 notes = ?, is_active = ?, notification_enabled = ?, notification_mode = ?,
                 notification_interval_minutes = ?, notify_at = ?, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
            params![
                item.tmdb_id,
                item.media_type,
                item.title,
                item.poster_path,
                item.release_date,
                item.notes,
                if item.is_active { 1 } else { 0 },
                if item.notification_enabled { 1 } else { 0 },
                item.notification_mode,
                item.notification_interval_minutes,
                item.notify_at,
                id,
            ],
        )?;
        self.get_watchlist_item(id)
    }

    pub fn delete_watchlist_item(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM watchlist_items WHERE id = ?", params![id])?;
        Ok(())
    }

    pub fn get_due_watchlist_notifications(&self, now_utc: &str) -> Result<Vec<WatchlistItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, tmdb_id, media_type, title, poster_path, release_date, notes,
                    is_active, notification_enabled, notification_mode,
                    notification_interval_minutes, notify_at, last_notified_at,
                    created_at, updated_at
             FROM watchlist_items
             WHERE is_active = 1
               AND notification_enabled = 1
               AND notify_at IS NOT NULL
               AND notify_at <= ?
             ORDER BY notify_at ASC",
        )?;

        let rows = stmt.query_map(params![now_utc], Self::map_watchlist_item)?;
        rows.collect()
    }

    pub fn disable_watchlist_notification(&self, id: i64, notified_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE watchlist_items
             SET notification_enabled = 0, last_notified_at = ?, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
            params![notified_at, id],
        )?;
        Ok(())
    }

    pub fn advance_watchlist_notification(
        &self,
        id: i64,
        next_notify_at: &str,
        notified_at: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE watchlist_items
             SET notify_at = ?, last_notified_at = ?, updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
            params![next_notify_at, notified_at, id],
        )?;
        Ok(())
    }

    pub fn replace_watchlist_items(&self, items: &[WatchlistItem]) -> Result<()> {
        self.conn.execute("DELETE FROM watchlist_items", [])?;

        for item in items {
            self.conn.execute(
                "INSERT INTO watchlist_items (
                    id, tmdb_id, media_type, title, poster_path, release_date, notes, is_active,
                    notification_enabled, notification_mode, notification_interval_minutes,
                    notify_at, last_notified_at, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    item.id,
                    item.tmdb_id,
                    item.media_type,
                    item.title,
                    item.poster_path,
                    item.release_date,
                    item.notes,
                    if item.is_active { 1 } else { 0 },
                    if item.notification_enabled { 1 } else { 0 },
                    item.notification_mode,
                    item.notification_interval_minutes,
                    item.notify_at,
                    item.last_notified_at,
                    item.created_at,
                    item.updated_at,
                ],
            )?;
        }

        Ok(())
    }

    /// Get movie/TV entries that still need enriched metadata for hover cards.
    pub fn get_media_needing_metadata_enrichment(
        &self,
        limit: usize,
    ) -> Result<Vec<MetadataEnrichmentCandidate>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, media_type, tmdb_id
             FROM media
             WHERE media_type IN ('movie', 'tvshow')
               AND (
                   tmdb_id IS NULL OR tmdb_id = ''
                    OR overview IS NULL OR TRIM(overview) = ''
                    OR cast_names IS NULL OR TRIM(cast_names) = ''
                    OR director IS NULL OR TRIM(director) = ''
                    OR poster_path IS NULL OR TRIM(poster_path) = ''
                    OR (media_type = 'movie' AND (duration_seconds IS NULL OR duration_seconds <= 0))
                )
             ORDER BY id ASC
             LIMIT ?",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(MetadataEnrichmentCandidate {
                id: row.get(0)?,
                title: row.get(1)?,
                year: row.get(2)?,
                media_type: row.get(3)?,
                tmdb_id: row.get(4)?,
            })
        })?;

        rows.collect()
    }

    /// Look up a media entry by cloud_file_id. Returns (id, title, media_type, parent_id).
    pub fn get_media_info_by_cloud_file_id(
        &self,
        cloud_file_id: &str,
    ) -> Result<Option<(i64, String, String, Option<i64>)>> {
        self.conn
            .query_row(
                "SELECT id, title, media_type, parent_id FROM media WHERE cloud_file_id = ?",
                params![cloud_file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other.into()),
            })
    }

    /// Preserve watch progress for ZIP children of a given archive before they get deleted.
    pub fn preserve_watch_progress_for_zip_children(&self, cloud_file_id: &str) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT id, COALESCE(resume_position_seconds, 0), COALESCE(duration_seconds, 0)
             FROM media WHERE parent_zip_id = ? AND last_watched IS NOT NULL",
        )?;
        let children: Vec<(i64, f64, f64)> = stmt
            .query_map(params![cloud_file_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        for (child_id, resume, duration) in children {
            let _ = self.record_watch_event(child_id, resume, duration);
        }
        Ok(())
    }

    /// Remove media by cloud file ID and return basic info
    pub fn remove_media_by_cloud_file_id(
        &self,
        cloud_file_id: &str,
    ) -> Result<Option<(i64, String, String, Option<i64>)>> {
        let zip_episode_count: Option<i64> = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM media WHERE parent_zip_id = ?",
                params![cloud_file_id],
                |row| row.get(0),
            )
            .ok();

        if let Some(episode_count) = zip_episode_count.filter(|count| *count > 0) {
            let archive_name = self
                .conn
                .query_row(
                    "SELECT filename FROM zip_archives WHERE zip_file_id = ?",
                    params![cloud_file_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap_or_else(|_| "ZIP archive".to_string());
            self.conn.execute(
                "DELETE FROM media WHERE parent_zip_id = ?",
                params![cloud_file_id],
            )?;
            self.conn.execute(
                "DELETE FROM zip_archives WHERE zip_file_id = ?",
                params![cloud_file_id],
            )?;

            return Ok(Some((
                -1,
                format!("{} ({} episode(s))", archive_name, episode_count),
                "zip_archive".to_string(),
                None,
            )));
        }

        let media_info: Option<(i64, String, String, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT id, title, media_type, parent_id FROM media WHERE cloud_file_id = ?",
                params![cloud_file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok();

        if let Some((id, _, _, _)) = &media_info {
            self.conn
                .execute("DELETE FROM media WHERE id = ?", params![id])?;
        }

        Ok(media_info)
    }

    /// Get all episodes user has for a series (returns id, season_number, episode_number)
    pub fn get_owned_episodes_for_series(&self, series_id: i64) -> Result<Vec<(i64, i32, i32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, season_number, episode_number FROM media
             WHERE parent_id = ? AND media_type = 'tvepisode'
             ORDER BY season_number, episode_number",
        )?;

        let items = stmt.query_map(params![series_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i32>>(1)?.unwrap_or(1),
                row.get::<_, Option<i32>>(2)?.unwrap_or(1),
            ))
        })?;

        items.collect()
    }

    /// Find series ID by TMDB ID
    pub fn find_series_id_by_tmdb(&self, tmdb_id: &str) -> Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM media WHERE tmdb_id = ? AND media_type = 'tvshow'")?;

        match stmt.query_row(params![tmdb_id], |row| row.get::<_, i64>(0)) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Find a TV show by title (case-insensitive) - returns the MediaItem
    pub fn find_tvshow_by_title(&self, title: &str) -> Result<Option<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    archive_format, is_cloud, cloud_file_id
             FROM media WHERE LOWER(title) = LOWER(?) AND media_type = 'tvshow'",
        )?;

        match stmt.query_row(params![title], Self::map_media_item) {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn find_media_by_tmdb(&self, tmdb_id: &str, media_type: &str) -> Result<Option<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, imdb_id, episode_title, still_path,
                    archive_format, is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, file_size_bytes
             FROM media
             WHERE tmdb_id = ? AND media_type = ?
             LIMIT 1",
        )?;

        match stmt.query_row(params![tmdb_id, media_type], Self::map_media_item) {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn find_episode_by_parent_and_numbers(
        &self,
        parent_id: i64,
        season_number: i32,
        episode_number: i32,
    ) -> Result<Option<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    archive_format, is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, file_size_bytes
             FROM media
             WHERE parent_id = ?
               AND media_type = 'tvepisode'
               AND COALESCE(season_number, 0) = ?
               AND COALESCE(episode_number, 0) = ?
             LIMIT 1",
        )?;

        match stmt.query_row(
            params![parent_id, season_number, episode_number],
            Self::map_media_item,
        ) {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn cloud_tvshow_path(folder_id: &str, show_title: &str) -> String {
        let slug = show_title
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_");
        format!("gdrive:{}:{}", folder_id, slug)
    }

    fn find_cloud_tvshow_by_title_and_folder(
        &self,
        title: &str,
        folder_id: &str,
    ) -> Result<Option<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id
             FROM media
             WHERE LOWER(TRIM(title)) = LOWER(TRIM(?))
               AND media_type = 'tvshow'
               AND COALESCE(cloud_folder_id, '') = ?
             ORDER BY id
             LIMIT 1",
        )?;

        match stmt.query_row(params![title, folder_id], |row| row.get::<_, i64>(0)) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn repair_misparented_archive_episodes(&self) -> Result<usize> {
        let groups = {
            let mut stmt = self.conn.prepare(
                "SELECT e.title, COALESCE(e.cloud_folder_id, '') AS folder_id
                 FROM media e
                 LEFT JOIN media p ON e.parent_id = p.id
                 WHERE e.media_type = 'tvepisode'
                   AND e.parent_zip_id IS NOT NULL
                   AND COALESCE(e.is_cloud, 0) = 1
                   AND (
                       p.id IS NULL
                       OR p.media_type != 'tvshow'
                       OR LOWER(TRIM(COALESCE(p.title, ''))) != LOWER(TRIM(e.title))
                   )
                 GROUP BY LOWER(TRIM(e.title)), COALESCE(e.cloud_folder_id, '')",
            )?;

            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;

            rows.filter_map(|row| row.ok()).collect::<Vec<_>>()
        };

        let mut repaired = 0usize;

        for (title, folder_id) in groups {
            let show_id = if let Some(existing_id) =
                self.find_cloud_tvshow_by_title_and_folder(&title, &folder_id)?
            {
                existing_id
            } else {
                let show_path = Self::cloud_tvshow_path(&folder_id, &title);
                match self.insert_cloud_tvshow(
                    &title, None, None, None, None, &show_path, &folder_id, None,
                ) {
                    Ok(id) => id,
                    Err(_) => self
                        .find_series_by_folder(&show_path)?
                        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?,
                }
            };

            repaired += self.conn.execute(
                "UPDATE media
                 SET parent_id = ?
                 WHERE media_type = 'tvepisode'
                   AND parent_zip_id IS NOT NULL
                   AND LOWER(TRIM(title)) = LOWER(TRIM(?))
                   AND COALESCE(cloud_folder_id, '') = ?
                   AND (parent_id IS NULL OR parent_id != ?)",
                params![show_id, title, folder_id, show_id],
            )?;
        }

        Ok(repaired)
    }

    // ==================== CACHED EPISODE METADATA FUNCTIONS ====================

    /// Save cached episode metadata from TMDB (for pre-fetching)
    pub fn save_cached_episode_metadata(
        &self,
        series_tmdb_id: &str,
        season_number: i32,
        episode_number: i32,
        episode_title: Option<&str>,
        overview: Option<&str>,
        still_path: Option<&str>,
        air_date: Option<&str>,
        vote_average: Option<f64>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO cached_episode_metadata
             (series_tmdb_id, season_number, episode_number, episode_title, overview, still_path, air_date, vote_average, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))",
            params![series_tmdb_id, season_number, episode_number, episode_title, overview, still_path, air_date, vote_average],
        )?;
        Ok(())
    }

    /// Get cached episode metadata
    pub fn get_cached_episode_metadata(
        &self,
        series_tmdb_id: &str,
        season_number: i32,
        episode_number: i32,
    ) -> Result<Option<CachedEpisodeMetadata>> {
        let mut stmt = self.conn.prepare(
            "SELECT episode_title, overview, still_path, air_date, vote_average
             FROM cached_episode_metadata
             WHERE series_tmdb_id = ? AND season_number = ? AND episode_number = ?",
        )?;

        match stmt.query_row(
            params![series_tmdb_id, season_number, episode_number],
            |row| {
                Ok(CachedEpisodeMetadata {
                    episode_title: row.get(0)?,
                    overview: row.get(1)?,
                    still_path: row.get(2)?,
                    air_date: row.get(3)?,
                    vote_average: row.get(4)?,
                })
            },
        ) {
            Ok(metadata) => Ok(Some(metadata)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Check if episode metadata is cached for a series
    pub fn has_cached_metadata_for_series(&self, series_tmdb_id: &str) -> Result<bool> {
        let count: i32 = self.conn.query_row(
            "SELECT COUNT(*) FROM cached_episode_metadata WHERE series_tmdb_id = ?",
            params![series_tmdb_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Clear cached episode metadata for a series (for refresh)
    pub fn clear_cached_metadata_for_series(&self, series_tmdb_id: &str) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM cached_episode_metadata WHERE series_tmdb_id = ?",
            params![series_tmdb_id],
        )?;
        Ok(deleted)
    }

    /// Get all cached episodes for a series
    pub fn get_all_cached_episodes_for_series(
        &self,
        series_tmdb_id: &str,
    ) -> Result<Vec<CachedEpisodeMetadata>> {
        let mut stmt = self.conn.prepare(
            "SELECT episode_title, overview, still_path, air_date, season_number, episode_number, vote_average
             FROM cached_episode_metadata
             WHERE series_tmdb_id = ?
             ORDER BY season_number, episode_number",
        )?;

        let items = stmt.query_map(params![series_tmdb_id], |row| {
            Ok(CachedEpisodeMetadataFull {
                episode_title: row.get(0)?,
                overview: row.get(1)?,
                still_path: row.get(2)?,
                air_date: row.get(3)?,
                season_number: row.get(4)?,
                episode_number: row.get(5)?,
                vote_average: row.get(6)?,
            })
        })?;

        items
            .filter_map(|r| {
                r.ok().map(|f| CachedEpisodeMetadata {
                    episode_title: f.episode_title,
                    overview: f.overview,
                    still_path: f.still_path,
                    air_date: f.air_date,
                    vote_average: f.vote_average,
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    /// Get cached episodes for a specific season of a series
    pub fn get_cached_episodes_for_season(
        &self,
        series_tmdb_id: &str,
        season_number: i32,
    ) -> Result<Vec<CachedEpisodeMetadataFull>> {
        let mut stmt = self.conn.prepare(
            "SELECT episode_title, overview, still_path, air_date, season_number, episode_number, vote_average
             FROM cached_episode_metadata
             WHERE series_tmdb_id = ? AND season_number = ?
             ORDER BY episode_number",
        )?;

        let items = stmt.query_map(params![series_tmdb_id, season_number], |row| {
            Ok(CachedEpisodeMetadataFull {
                episode_title: row.get(0)?,
                overview: row.get(1)?,
                still_path: row.get(2)?,
                air_date: row.get(3)?,
                season_number: row.get(4)?,
                episode_number: row.get(5)?,
                vote_average: row.get(6)?,
            })
        })?;

        items.collect()
    }

    /// Get all media entries (for cleanup purposes)
    pub fn get_all_media(&self) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    archive_format, is_cloud, cloud_file_id
             FROM media",
        )?;

        let items = stmt.query_map([], Self::map_media_item)?;
        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    /// Get all poster paths currently in use (including still_paths and cached episode images)
    pub fn get_all_poster_paths(&self) -> Result<Vec<String>> {
        let mut all_paths = Vec::new();

        // Get poster paths from media table
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT poster_path FROM media WHERE poster_path IS NOT NULL")?;
        let paths = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for path in paths.filter_map(|r| r.ok()) {
            all_paths.push(path);
        }

        // Get still paths from media table
        let mut stmt2 = self
            .conn
            .prepare("SELECT DISTINCT still_path FROM media WHERE still_path IS NOT NULL")?;
        let still_paths = stmt2.query_map([], |row| row.get::<_, String>(0))?;
        for path in still_paths.filter_map(|r| r.ok()) {
            all_paths.push(path);
        }

        // Get still paths from cached episode metadata
        let mut stmt3 = self.conn.prepare(
            "SELECT DISTINCT still_path FROM cached_episode_metadata WHERE still_path IS NOT NULL",
        )?;
        let cached_paths = stmt3.query_map([], |row| row.get::<_, String>(0))?;
        for path in cached_paths.filter_map(|r| r.ok()) {
            all_paths.push(path);
        }

        Ok(all_paths)
    }

    /// Remove a media entry by ID
    pub fn remove_media(&self, id: i64) -> Result<Option<String>> {
        // First get the poster path so we can clean it up
        let poster_path: Option<String> = self
            .conn
            .query_row(
                "SELECT poster_path FROM media WHERE id = ?",
                params![id],
                |row| row.get(0),
            )
            .ok();

        // Delete the entry
        self.conn
            .execute("DELETE FROM media WHERE id = ?", params![id])?;

        Ok(poster_path)
    }

    /// Remove all episodes for a series
    pub fn remove_series_episodes(&self, series_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM media WHERE parent_id = ?", params![series_id])?;
        Ok(())
    }

    /// Get file paths for multiple media IDs (for deletion)
    pub fn get_media_file_paths(&self, ids: &[i64]) -> Result<Vec<(i64, Option<String>)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "SELECT id, file_path FROM media WHERE id IN ({})",
            placeholders.join(", ")
        );

        let mut stmt = self.conn.prepare(&query)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

        let results = stmt.query_map(params.as_slice(), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;

        results
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    /// Get cloud info for a series (is_cloud, cloud_folder_id)
    pub fn get_series_cloud_info(&self, series_id: i64) -> Result<(bool, Option<String>)> {
        let mut stmt = self
            .conn
            .prepare("SELECT COALESCE(is_cloud, 0), cloud_folder_id FROM media WHERE id = ?")?;

        stmt.query_row(params![series_id], |row| {
            Ok((row.get::<_, i32>(0)? == 1, row.get::<_, Option<String>>(1)?))
        })
    }

    /// Get media info for deletion (file_path, is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, ddl_source_id)
    pub fn get_media_delete_info(
        &self,
        ids: &[i64],
    ) -> Result<
        Vec<(
            i64,
            Option<String>,
            bool,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        )>,
    > {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "SELECT id, file_path, COALESCE(is_cloud, 0) as is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, ddl_source_id FROM media WHERE id IN ({})",
            placeholders.join(", ")
        );

        let mut stmt = self.conn.prepare(&query)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

        let results = stmt.query_map(params.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i32>(2)? == 1,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?;

        results
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    /// Get parent series IDs for a list of episode IDs
    pub fn get_parent_series_ids(&self, ids: &[i64]) -> Result<Vec<i64>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "SELECT DISTINCT parent_id FROM media WHERE id IN ({}) AND media_type = 'tvepisode' AND parent_id IS NOT NULL",
            placeholders.join(", ")
        );

        let mut stmt = self.conn.prepare(&query)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

        let results = stmt.query_map(params.as_slice(), |row| row.get::<_, i64>(0))?;

        Ok(results.filter_map(|r| r.ok()).collect())
    }

    /// Delete multiple media entries and return their file paths for cleanup
    pub fn delete_media_entries(&self, ids: &[i64]) -> Result<Vec<String>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        // First get the file paths
        let file_paths: Vec<String> = self
            .get_media_file_paths(ids)?
            .into_iter()
            .filter_map(|(_, path)| path)
            .collect();

        // Delete all entries
        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "DELETE FROM media WHERE id IN ({})",
            placeholders.join(", ")
        );

        let params: Vec<&dyn rusqlite::ToSql> =
            ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
        self.conn.execute(&query, params.as_slice())?;

        Ok(file_paths)
    }

    /// Check if a series has any remaining episodes
    pub fn series_has_episodes(&self, series_id: i64) -> Result<bool> {
        let count: i32 = self.conn.query_row(
            "SELECT COUNT(*) FROM media WHERE parent_id = ?",
            params![series_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Merge duplicate TV shows into a single entry.
    /// Groups by TMDB ID first, then by normalized title.
    /// Keeps the entry with the most complete metadata as the primary.
    /// Find duplicate media entries: same tmdb_id or similar normalized titles.
    /// Only checks movies and tvshows (not episodes).
    pub fn find_duplicate_media(&self) -> Result<Vec<DuplicateGroup>> {
        let select_cols = "id, title, year, overview, cast_names, director, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, imdb_id, episode_title, still_path,
                    archive_format,
                    is_cloud, cloud_file_id, cloud_folder_id, parent_zip_id, zip_entry_path, zip_local_header_offset,
                    zip_data_start_offset, zip_compressed_size, zip_uncompressed_size, zip_crc32,
                    zip_compression_method, file_size_bytes, ddl_source_id";

        let mut groups: Vec<DuplicateGroup> = Vec::new();

        // 1) Exact tmdb_id duplicates (movies + tvshows, not episodes)
        {
            let sql = format!(
                "SELECT {} FROM media WHERE media_type IN ('movie','tvshow') AND tmdb_id IS NOT NULL AND tmdb_id != '' ORDER BY tmdb_id, id",
                select_cols
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let items: Vec<MediaItem> = stmt
                .query_map([], Self::map_media_item)?
                .filter_map(|r| r.ok())
                .collect();

            // Group by tmdb_id
            let mut by_tmdb: std::collections::HashMap<String, Vec<MediaItem>> =
                std::collections::HashMap::new();
            for item in items {
                if let Some(ref tid) = item.tmdb_id {
                    by_tmdb.entry(tid.clone()).or_default().push(item);
                }
            }
            for (tid, mut dupes) in by_tmdb {
                if dupes.len() > 1 {
                    dupes.sort_by(|a, b| a.id.cmp(&b.id));
                    groups.push(DuplicateGroup {
                        items: dupes,
                        reason: format!("Same TMDB ID ({})", tid),
                    });
                }
            }
        }

        // 2) Fuzzy title matches: normalized lowercase title for same media_type
        {
            let sql = format!(
                "SELECT {} FROM media WHERE media_type IN ('movie','tvshow') ORDER BY media_type, id",
                select_cols
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let items: Vec<MediaItem> = stmt
                .query_map([], Self::map_media_item)?
                .filter_map(|r| r.ok())
                .collect();

            // Normalize: lowercase, strip non-alphanumeric
            let normalize = |s: &str| -> String {
                s.to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect()
            };

            // Group by (media_type, normalized_title)
            let mut by_title: std::collections::HashMap<(String, String), Vec<MediaItem>> =
                std::collections::HashMap::new();
            for item in items {
                let key = (item.media_type.clone(), normalize(&item.title));
                by_title.entry(key).or_default().push(item);
            }
            for ((_mt, _norm_title), dupes) in by_title {
                if dupes.len() > 1 {
                    // Avoid adding if all items already covered by tmdb_id groups
                    let all_ids: Vec<i64> = dupes.iter().map(|d| d.id).collect();
                    let already_covered = groups.iter().any(|g| {
                        let group_ids: Vec<i64> = g.items.iter().map(|i| i.id).collect();
                        all_ids.iter().all(|id| group_ids.contains(id))
                    });
                    if !already_covered {
                        let sample_title = dupes[0].title.clone();
                        let mut sorted = dupes;
                        sorted.sort_by(|a, b| a.id.cmp(&b.id));
                        groups.push(DuplicateGroup {
                            items: sorted,
                            reason: format!("Similar title (\"{}\")", sample_title),
                        });
                    }
                }
            }
        }

        // Sort groups by count descending
        groups.sort_by(|a, b| b.items.len().cmp(&a.items.len()));
        Ok(groups)
    }

    pub fn merge_duplicate_tvshows(&self) -> Result<i32> {
        println!("[MERGE] Looking for duplicate TV shows to merge...");
        let mut merged_count = 0;

        // Step 1: Find and merge duplicates with same TMDB ID
        let tmdb_duplicates: Vec<(String, Vec<i64>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT tmdb_id, GROUP_CONCAT(id) as ids, COUNT(*) as cnt 
                 FROM media 
                 WHERE media_type = 'tvshow' AND tmdb_id IS NOT NULL AND tmdb_id != ''
                 GROUP BY tmdb_id 
                 HAVING cnt > 1",
            )?;

            let results: Vec<(String, Vec<i64>)> = stmt
                .query_map([], |row| {
                    let tmdb_id: String = row.get(0)?;
                    let ids_str: String = row.get(1)?;
                    let ids: Vec<i64> = ids_str
                        .split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .collect();
                    Ok((tmdb_id, ids))
                })?
                .filter_map(|r| r.ok())
                .collect();
            results
        };

        for (tmdb_id, ids) in tmdb_duplicates {
            if ids.len() > 1 {
                println!(
                    "[MERGE] Found {} duplicates with TMDB ID: {}",
                    ids.len(),
                    tmdb_id
                );
                merged_count += self.merge_series_entries(&ids)?;
            }
        }

        // Step 2: Find and merge duplicates by normalized title.
        // This catches punctuation/spacing variants like:
        // "Monarch: Legacy of Monsters" vs "Monarch Legacy of Monsters".
        // Safety guard: never merge groups that contain multiple different TMDB IDs,
        // and avoid merging same-name-but-different-era shows with wide year gaps.
        let normalized_duplicates: Vec<(String, Vec<i64>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, title, tmdb_id, year
                 FROM media
                 WHERE media_type = 'tvshow'",
            )?;

            let rows: Vec<(i64, String, Option<String>, Option<i32>)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i32>>(3)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();

            let mut groups: std::collections::HashMap<
                String,
                Vec<(i64, Option<String>, Option<i32>)>,
            > = std::collections::HashMap::new();

            for (id, title, tmdb_id, year) in rows {
                let normalized = Self::normalize_title_for_db(&title);
                groups
                    .entry(normalized)
                    .or_default()
                    .push((id, tmdb_id, year));
            }

            let mut candidates: Vec<(String, Vec<i64>)> = Vec::new();

            for (normalized_title, entries) in groups {
                if entries.len() < 2 {
                    continue;
                }

                let mut tmdb_ids: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                let mut years: Vec<i32> = Vec::new();

                for (_, tmdb_id, year) in &entries {
                    if let Some(tid) = tmdb_id.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty())
                    {
                        tmdb_ids.insert(tid.to_string());
                    }
                    if let Some(y) = year {
                        years.push(*y);
                    }
                }

                if tmdb_ids.len() > 1 {
                    println!(
                        "[MERGE] Skipping normalized title '{}' (multiple TMDB IDs detected)",
                        normalized_title
                    );
                    continue;
                }

                if !years.is_empty() {
                    let min_year = *years.iter().min().unwrap_or(&0);
                    let max_year = *years.iter().max().unwrap_or(&0);
                    if max_year - min_year > 1 {
                        println!(
                            "[MERGE] Skipping normalized title '{}' (year spread {}-{})",
                            normalized_title, min_year, max_year
                        );
                        continue;
                    }
                }

                let ids = entries.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
                candidates.push((normalized_title, ids));
            }

            candidates
        };

        for (title, ids) in normalized_duplicates {
            if ids.len() > 1 {
                println!(
                    "[MERGE] Found {} duplicates with normalized title: {}",
                    ids.len(),
                    title
                );
                merged_count += self.merge_series_entries(&ids)?;
            }
        }

        if merged_count > 0 {
            println!("[MERGE] Merged {} duplicate TV show entries", merged_count);
        } else {
            println!("[MERGE] No duplicates found");
        }

        Ok(merged_count)
    }

    /// Merge a list of series IDs into one primary entry.
    /// Picks the best entry (has TMDB ID + poster) as primary, moves all episodes to it.
    fn merge_series_entries(&self, ids: &[i64]) -> Result<i32> {
        if ids.len() < 2 {
            return Ok(0);
        }

        // Find the best entry to keep (prefer one with TMDB ID and poster)
        let mut best_id: i64 = ids[0];
        let mut best_score = 0;

        for &id in ids {
            let score: i32 = self
                .conn
                .query_row(
                    "SELECT 
                    (CASE WHEN tmdb_id IS NOT NULL AND tmdb_id != '' THEN 10 ELSE 0 END) +
                    (CASE WHEN poster_path IS NOT NULL AND poster_path != '' THEN 5 ELSE 0 END) +
                    (CASE WHEN overview IS NOT NULL AND overview != '' THEN 2 ELSE 0 END) +
                    (CASE WHEN year IS NOT NULL THEN 1 ELSE 0 END)
                 FROM media WHERE id = ?",
                    params![id],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            if score > best_score {
                best_score = score;
                best_id = id;
            }
        }

        // Get the best entry's metadata for reference
        let best_title: String = self
            .conn
            .query_row(
                "SELECT title FROM media WHERE id = ?",
                params![best_id],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "Unknown".to_string());

        println!(
            "[MERGE] Keeping series ID {} ({}) as primary",
            best_id, best_title
        );

        let mut merged = 0;

        // Move all episodes from other entries to the best entry
        for &id in ids {
            if id != best_id {
                // Count episodes that will be moved
                let episode_count: i32 = self
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM media WHERE parent_id = ?",
                        params![id],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);

                println!(
                    "[MERGE] Moving {} episodes from series {} to {}",
                    episode_count, id, best_id
                );

                // Move episodes to primary series
                self.conn.execute(
                    "UPDATE media SET parent_id = ? WHERE parent_id = ?",
                    params![best_id, id],
                )?;

                // Delete the duplicate series entry
                self.conn
                    .execute("DELETE FROM media WHERE id = ?", params![id])?;

                merged += 1;
            }
        }

        Ok(merged)
    }

    /// Delete all media entries (local + cloud) and related data, preserving watch history and settings.
    /// Returns (cloud_file_ids, image_cache_path) for the caller to handle file cleanup.
    pub fn delete_all_media_entries(&self) -> Result<(Vec<String>, String)> {
        // Collect cloud file IDs for GDrive deletion
        let cloud_file_ids: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT cloud_file_id FROM media WHERE cloud_file_id IS NOT NULL AND cloud_file_id != ''"
            )?;
            let ids: Vec<String> = stmt
                .query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            ids
        };

        // Delete related data first (foreign keys)
        self.conn
            .execute("DELETE FROM cached_episode_metadata", [])?;
        self.conn.execute("DELETE FROM zip_archives", [])?;
        self.conn.execute("DELETE FROM ddl_sources", [])?;
        self.conn.execute("DELETE FROM cloud_folders", [])?;
        self.conn.execute("DELETE FROM cloud_index_failures", [])?;

        // Delete all media entries
        self.conn.execute("DELETE FROM media", [])?;

        Ok((cloud_file_ids, get_image_cache_dir()))
    }

    /// Delete media entries by their Google Drive file IDs
    pub fn delete_media_by_cloud_file_ids(&self, cloud_file_ids: &[String]) -> Result<usize> {
        if cloud_file_ids.is_empty() {
            return Ok(0);
        }
        let mut deleted = 0usize;
        for cloud_file_id in cloud_file_ids {
            deleted += self.conn.execute(
                "DELETE FROM media WHERE cloud_file_id = ?",
                params![cloud_file_id],
            )?;
        }
        // Also clean up orphaned cached_episode_metadata
        self.conn.execute(
            "DELETE FROM cached_episode_metadata WHERE NOT EXISTS (SELECT 1 FROM media WHERE media.id = cached_episode_metadata.media_id)",
            [],
        )?;
        Ok(deleted)
    }

    /// Clear ALL app data - deletes every table and returns paths for file cleanup
    /// Returns the image cache path for the caller to delete
    pub fn clear_all_data(&self) -> Result<String> {
        // Disable foreign key checks temporarily for clean deletion order
        self.conn.execute("PRAGMA foreign_keys = OFF", [])?;

        // Delete ALL data from ALL tables
        self.conn.execute("DELETE FROM streaming_history", [])?;
        self.conn.execute("DELETE FROM watch_history_events", [])?;
        self.conn.execute("DELETE FROM cloud_folders", [])?;
        self.conn.execute("DELETE FROM cloud_index_failures", [])?;
        self.conn.execute("DELETE FROM app_settings", [])?;
        self.conn.execute("DELETE FROM movie_reminders", [])?;
        self.conn.execute("DELETE FROM watchlist_items", [])?;
        self.conn.execute("DELETE FROM media", [])?;
        self.conn
            .execute("DELETE FROM cached_episode_metadata", [])?;
        self.conn.execute("DELETE FROM zip_archives", [])?;
        self.conn.execute("DELETE FROM ddl_sources", [])?;

        self.conn.execute("PRAGMA foreign_keys = ON", [])?;

        // Return the image cache path for the caller to delete
        Ok(get_image_cache_dir())
    }

    /// Get all media items with broken file paths (filename only, no directory)
    pub fn get_broken_file_paths(&self) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, file_path FROM media
             WHERE file_path IS NOT NULL
               AND file_path != ''
               AND file_path NOT LIKE 'tvshow://%'
               AND file_path NOT LIKE '%/%'
               AND file_path NOT LIKE '%\\%'",
        )?;

        let items = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;

        items.collect()
    }

    /// Update the file path for a media item
    pub fn update_file_path(&self, media_id: i64, new_path: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE media SET file_path = ? WHERE id = ?",
            params![new_path, media_id],
        )?;
        Ok(())
    }

    pub fn update_file_size(&self, media_id: i64, file_size_bytes: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE media SET file_size_bytes = ? WHERE id = ?",
            params![file_size_bytes, media_id],
        )?;
        Ok(())
    }

    pub fn update_duration(&self, media_id: i64, duration_seconds: f64) -> Result<()> {
        self.conn.execute(
            "UPDATE media SET duration_seconds = ? WHERE id = ? AND (duration_seconds IS NULL OR duration_seconds = 0)",
            params![duration_seconds, media_id],
        )?;
        Ok(())
    }

    // ==================== SOCIAL SYNC FUNCTIONS ====================

    /// Get aggregated watch stats from both media and streaming_history tables.
    /// "Completed" means: for media table, resume_position = 0 AND last_watched IS NOT NULL AND duration > 0
    /// (because update_progress resets position to 0 once playback passes 93%).
    /// For streaming_history, completed means duration > 0 AND progress > 93%.
    pub fn get_watch_stats(&self) -> Result<WatchStatsAggregated> {
        // Query completed items from the media table
        let (media_movies, media_episodes, media_time): (i64, i64, f64) = self.conn.query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN media_type = 'movie' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN media_type = 'tvepisode' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(duration_seconds), 0)
             FROM media
             WHERE last_watched IS NOT NULL
               AND resume_position_seconds = 0
               AND duration_seconds > 0
               AND media_type IN ('movie', 'tvepisode')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;

        // Query completed items from streaming_history table
        let (stream_movies, stream_episodes, stream_time): (i64, i64, f64) = self.conn.query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN media_type = 'movie' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN media_type = 'tv' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(duration_seconds), 0)
             FROM streaming_history
             WHERE duration_seconds > 0
               AND (resume_position_seconds = 0
                    OR (resume_position_seconds * 1.0 / duration_seconds) > 0.93)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;

        Ok(WatchStatsAggregated {
            movies_watched: media_movies + stream_movies,
            episodes_watched: media_episodes + stream_episodes,
            total_watch_time_seconds: media_time + stream_time,
        })
    }

    pub fn get_library_stats(&self, is_cloud: Option<bool>) -> Result<LibraryStats> {
        let mut sql = String::from(
            "SELECT
                COALESCE(SUM(CASE WHEN media_type = 'movie' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN media_type = 'tvshow' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN media_type = 'tvepisode' THEN 1 ELSE 0 END), 0)
             FROM media",
        );

        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<LibraryStats> {
            Ok(LibraryStats {
                movies: row.get(0)?,
                shows: row.get(1)?,
                episodes: row.get(2)?,
            })
        };

        if let Some(cloud) = is_cloud {
            sql.push_str(" WHERE COALESCE(is_cloud, 0) = ?");
            let cloud_value = if cloud { 1 } else { 0 };
            self.conn.query_row(&sql, params![cloud_value], map_row)
        } else {
            sql.push_str(" WHERE (file_path IS NULL OR file_path NOT LIKE 'remote://%')");
            self.conn.query_row(&sql, [], map_row)
        }
    }

    /// Get recently completed watch activities since a given timestamp.
    /// Returns items from both media and streaming_history tables,
    /// unified into WatchActivityItem structs ready for social API.
    pub fn get_recent_watch_activities(
        &self,
        since_timestamp: &str,
    ) -> Result<Vec<WatchActivityItem>> {
        let mut activities = Vec::new();

        // From media table: completed items since timestamp
        // For episodes, join to parent to get the show's tmdb_id and title
        let mut stmt = self.conn.prepare(
            "SELECT
                COALESCE(
                    CASE WHEN m.media_type = 'tvepisode' THEN p.tmdb_id ELSE m.tmdb_id END,
                    CAST(m.id AS TEXT)
                ) as content_id,
                CASE WHEN m.media_type = 'tvepisode' THEN COALESCE(p.title, m.title) ELSE m.title END as title,
                CASE WHEN m.media_type = 'tvepisode' THEN 'tv' ELSE 'movie' END as content_type,
                CASE WHEN m.media_type = 'tvepisode' THEN 'watched_episode' ELSE 'watched_movie' END as activity_type,
                CASE WHEN m.media_type = 'tvepisode' THEN p.poster_path ELSE m.poster_path END as poster_path,
                m.season_number,
                m.episode_number,
                m.duration_seconds,
                m.last_watched
             FROM media m
             LEFT JOIN media p ON m.parent_id = p.id
             WHERE m.last_watched IS NOT NULL
               AND m.last_watched > ?
               AND m.resume_position_seconds = 0
               AND m.duration_seconds > 0
               AND m.media_type IN ('movie', 'tvepisode')
             ORDER BY m.last_watched DESC"
        )?;

        let media_items = stmt.query_map(params![since_timestamp], |row| {
            Ok(WatchActivityItem {
                content_id: row.get(0)?,
                title: row.get(1)?,
                content_type: row.get(2)?,
                activity_type: row.get(3)?,
                poster_path: row.get(4)?,
                season: row.get(5)?,
                episode: row.get(6)?,
                duration_seconds: row.get(7)?,
                last_watched: row.get(8)?,
            })
        })?;

        for item in media_items {
            if let Ok(activity) = item {
                activities.push(activity);
            }
        }

        // From streaming_history: completed items since timestamp
        let mut stmt2 = self.conn.prepare(
            "SELECT
                tmdb_id,
                title,
                CASE WHEN media_type = 'movie' THEN 'movie' ELSE 'tv' END as content_type,
                CASE WHEN media_type = 'movie' THEN 'watched_movie' ELSE 'watched_episode' END as activity_type,
                poster_path,
                season,
                episode,
                duration_seconds,
                last_watched
             FROM streaming_history
             WHERE last_watched > ?
               AND duration_seconds > 0
               AND (resume_position_seconds = 0
                    OR (resume_position_seconds * 1.0 / duration_seconds) > 0.93)
             ORDER BY last_watched DESC"
        )?;

        let stream_items = stmt2.query_map(params![since_timestamp], |row| {
            Ok(WatchActivityItem {
                content_id: row.get(0)?,
                title: row.get(1)?,
                content_type: row.get(2)?,
                activity_type: row.get(3)?,
                poster_path: row.get(4)?,
                season: row.get(5)?,
                episode: row.get(6)?,
                duration_seconds: row.get(7)?,
                last_watched: row.get(8)?,
            })
        })?;

        for item in stream_items {
            if let Ok(activity) = item {
                activities.push(activity);
            }
        }

        // Sort all activities by last_watched descending
        activities.sort_by(|a, b| b.last_watched.cmp(&a.last_watched));

        Ok(activities)
    }

    /// Get all analytics data in a single call for the analytics dashboard.
    /// Pulls from ALL three data sources: watch_history_events, media, and streaming_history.
    pub fn get_analytics_data(&self) -> Result<AnalyticsData> {
        // === UNIFIED VIEW: combine watch_history_events + media + streaming_history ===
        // CTE creates a unified timeline of all watch activity across all sources.

        // 1. Overview totals from all sources
        let (total_events, completed_events, total_watch_time): (i64, i64, f64) = self.conn.query_row(
            "WITH all_activity AS (
                -- watch_history_events (detailed event log)
                SELECT
                    event_id as id,
                    media_type,
                    duration_seconds * (progress_percent / 100.0) as watched_sec,
                    CASE WHEN completed = 1 THEN 1 ELSE 0 END as is_completed,
                    ended_at as activity_date,
                    started_at,
                    is_cloud,
                    COALESCE(parent_title, title) as display_title,
                    parent_title,
                    poster_path,
                    COALESCE(parent_tmdb_id, tmdb_id) as tid,
                    progress_percent as progress
                FROM watch_history_events
                UNION ALL
                -- streaming_history: online streaming watches
                SELECT
                    'stream-' || id,
                    media_type,
                    CASE WHEN resume_position_seconds = 0 THEN duration_seconds ELSE resume_position_seconds END,
                    CASE WHEN resume_position_seconds = 0 AND duration_seconds > 0
                         OR (resume_position_seconds * 1.0 / duration_seconds) > 0.93 THEN 1 ELSE 0 END,
                    last_watched,
                    last_watched,
                    1,
                    title,
                    NULL,
                    poster_path,
                    tmdb_id,
                    CASE WHEN resume_position_seconds = 0 AND duration_seconds > 0 THEN 100.0
                         WHEN duration_seconds > 0 THEN (resume_position_seconds / duration_seconds) * 100.0
                         ELSE 0 END
                FROM streaming_history
                WHERE duration_seconds > 0
            )
            SELECT
                COUNT(*),
                (SELECT COUNT(DISTINCT media_id) FROM watch_history_events WHERE completed = 1),
                COALESCE(SUM(watched_sec), 0)
            FROM all_activity",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;

        // Count unique completed movies and episodes (deduplicated by media_id)
        let (movies_completed, episodes_completed): (i64, i64) = self.conn.query_row(
            "SELECT
                COALESCE(COUNT(DISTINCT CASE WHEN media_type = 'movie' AND completed = 1 THEN media_id END), 0),
                COALESCE(COUNT(DISTINCT CASE WHEN media_type IN ('tvepisode', 'tv') AND completed = 1 THEN media_id END), 0)
            FROM watch_history_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        let completion_rate = if total_events > 0 {
            (completed_events as f64 / total_events as f64) * 100.0
        } else {
            0.0
        };

        // 2. Current streak: consecutive days from all sources
        let streak_days = self.compute_watch_streak_unified()?;

        // 3. Heatmap: daily activity for last 365 days from watch_history_events + streaming_history
        let mut heatmap_stmt = self.conn.prepare(
            "WITH all_days AS (
                SELECT date(ended_at) as day, duration_seconds * (progress_percent / 100.0) as sec FROM watch_history_events WHERE ended_at >= date('now', '-365 days')
                UNION ALL
                SELECT date(last_watched), CASE WHEN resume_position_seconds = 0 THEN duration_seconds ELSE resume_position_seconds END FROM streaming_history WHERE last_watched >= date('now', '-365 days') AND duration_seconds > 0
            )
            SELECT day, COALESCE(SUM(sec), 0), COUNT(*) FROM all_days GROUP BY day ORDER BY day"
        )?;
        let heatmap: Vec<HeatmapDay> = heatmap_stmt
            .query_map([], |row| {
                Ok(HeatmapDay {
                    date: row.get(0)?,
                    watch_seconds: row.get(1)?,
                    event_count: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // 4. Daily trend: last 90 days with media_type split
        let mut trend_stmt = self.conn.prepare(
            "WITH all_trend AS (
                SELECT date(ended_at) as day, duration_seconds * (progress_percent / 100.0) as sec, media_type FROM watch_history_events WHERE ended_at >= date('now', '-90 days')
                UNION ALL
                SELECT date(last_watched), CASE WHEN resume_position_seconds = 0 THEN duration_seconds ELSE resume_position_seconds END, CASE WHEN media_type = 'tv' THEN 'tvepisode' ELSE media_type END FROM streaming_history WHERE last_watched >= date('now', '-90 days') AND duration_seconds > 0
            )
            SELECT day,
                COALESCE(SUM(sec), 0),
                COALESCE(SUM(CASE WHEN media_type = 'movie' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN media_type = 'tvepisode' THEN 1 ELSE 0 END), 0)
            FROM all_trend GROUP BY day ORDER BY day"
        )?;
        let daily_trend: Vec<DailyWatchPoint> = trend_stmt
            .query_map([], |row| {
                Ok(DailyWatchPoint {
                    date: row.get(0)?,
                    watch_seconds: row.get(1)?,
                    movie_count: row.get(2)?,
                    episode_count: row.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // 5. Content breakdown by type
        let mut type_stmt = self.conn.prepare(
            "WITH all_types AS (
                SELECT media_type, duration_seconds * (progress_percent / 100.0) as sec FROM watch_history_events
                UNION ALL
                SELECT CASE WHEN media_type = 'tv' THEN 'tvepisode' ELSE media_type END, CASE WHEN resume_position_seconds = 0 THEN duration_seconds ELSE resume_position_seconds END FROM streaming_history WHERE duration_seconds > 0
            )
            SELECT media_type, COUNT(*), COALESCE(SUM(sec), 0) FROM all_types GROUP BY media_type"
        )?;
        let content_breakdown: Vec<ContentTypeBreakdown> = type_stmt
            .query_map([], |row| {
                Ok(ContentTypeBreakdown {
                    content_type: row.get(0)?,
                    count: row.get(1)?,
                    total_seconds: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // 6. Source breakdown: cloud vs local
        let mut source_stmt = self.conn.prepare(
            "WITH all_sources AS (
                SELECT CASE WHEN is_cloud = 1 THEN 'cloud' ELSE 'local' END as src, duration_seconds * (progress_percent / 100.0) as sec FROM watch_history_events
                UNION ALL
                SELECT 'cloud', CASE WHEN resume_position_seconds = 0 THEN duration_seconds ELSE resume_position_seconds END FROM streaming_history WHERE duration_seconds > 0
            )
            SELECT src, COUNT(*), COALESCE(SUM(sec), 0) FROM all_sources GROUP BY src"
        )?;
        let source_breakdown: Vec<SourceBreakdown> = source_stmt
            .query_map([], |row| {
                Ok(SourceBreakdown {
                    source: row.get(0)?,
                    count: row.get(1)?,
                    total_seconds: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // 7. Top watched content from all sources (grouped by show/movie title)
        // Uses COUNT(DISTINCT media_id) to count unique episodes, not play sessions.
        let mut top_stmt = self.conn.prepare(
            "WITH all_titles AS (
                SELECT
                    COALESCE(parent_title, title) as display_title,
                    parent_title,
                    CASE WHEN media_type = 'tvepisode' THEN 'tvshow' ELSE media_type END as type,
                    duration_seconds * (progress_percent / 100.0) as sec,
                    poster_path,
                    COALESCE(parent_tmdb_id, tmdb_id) as tid,
                    media_id,
                    COALESCE(parent_tmdb_id, tmdb_id, '') || '-S' || COALESCE(CAST(season_number AS TEXT), '') || 'E' || COALESCE(CAST(episode_number AS TEXT), '') as ep_key
                FROM watch_history_events
                UNION ALL
                SELECT
                    s.title, NULL, 'tvshow',
                    CASE WHEN s.resume_position_seconds = 0 THEN s.duration_seconds ELSE s.resume_position_seconds END,
                    s.poster_path, s.tmdb_id, NULL,
                    COALESCE(s.tmdb_id, '') || '-S' || COALESCE(CAST(s.season AS TEXT), '') || 'E' || COALESCE(CAST(s.episode AS TEXT), '')
                FROM streaming_history s
                WHERE s.duration_seconds > 0 AND s.media_type = 'tv'
                UNION ALL
                SELECT
                    s.title, NULL, s.media_type,
                    CASE WHEN s.resume_position_seconds = 0 THEN s.duration_seconds ELSE s.resume_position_seconds END,
                    s.poster_path, s.tmdb_id, s.id, NULL
                FROM streaming_history s
                WHERE s.duration_seconds > 0 AND s.media_type = 'movie'
            )
            SELECT display_title, parent_title, type,
                COUNT(DISTINCT COALESCE(media_id, ep_key)),
                COALESCE(SUM(sec), 0), MAX(poster_path), MAX(tid)
            FROM all_titles
            GROUP BY display_title
            ORDER BY COUNT(DISTINCT COALESCE(media_id, ep_key)) DESC
            LIMIT 10"
        )?;
        let top_watched: Vec<TopWatchedItem> = top_stmt
            .query_map([], |row| {
                Ok(TopWatchedItem {
                    title: row.get(0)?,
                    parent_title: row.get(1)?,
                    media_type: row.get(2)?,
                    watch_count: row.get(3)?,
                    total_seconds: row.get(4)?,
                    poster_path: row.get(5)?,
                    tmdb_id: row.get(6)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // 8. Hour distribution (0-23)
        let mut hour_stmt = self.conn.prepare(
            "WITH all_hours AS (
                SELECT CAST(strftime('%H', started_at) AS INTEGER) as hr, duration_seconds * (progress_percent / 100.0) as sec FROM watch_history_events
                UNION ALL
                SELECT CAST(strftime('%H', last_watched) AS INTEGER), CASE WHEN resume_position_seconds = 0 THEN duration_seconds ELSE resume_position_seconds END FROM streaming_history WHERE duration_seconds > 0
            )
            SELECT hr, COUNT(*), COALESCE(SUM(sec), 0) FROM all_hours GROUP BY hr ORDER BY hr"
        )?;
        let hour_distribution: Vec<HourDistribution> = hour_stmt
            .query_map([], |row| {
                Ok(HourDistribution {
                    hour: row.get(0)?,
                    event_count: row.get(1)?,
                    total_seconds: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // 9. Day of week distribution (0=Sun, 6=Sat)
        let mut dow_stmt = self.conn.prepare(
            "WITH all_dows AS (
                SELECT CAST(strftime('%w', started_at) AS INTEGER) as dow, duration_seconds * (progress_percent / 100.0) as sec FROM watch_history_events
                UNION ALL
                SELECT CAST(strftime('%w', last_watched) AS INTEGER), CASE WHEN resume_position_seconds = 0 THEN duration_seconds ELSE resume_position_seconds END FROM streaming_history WHERE duration_seconds > 0
            )
            SELECT dow, COUNT(*), COALESCE(SUM(sec), 0) FROM all_dows GROUP BY dow ORDER BY dow"
        )?;
        let day_distribution: Vec<DayOfWeekDistribution> = dow_stmt
            .query_map([], |row| {
                Ok(DayOfWeekDistribution {
                    day_of_week: row.get(0)?,
                    event_count: row.get(1)?,
                    total_seconds: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        // 10. Completion funnel (unique content items, not play sessions)
        let (started, in_progress, mostly_done, completed): (i64, i64, i64, i64) = self.conn.query_row(
            "WITH all_progress AS (
                SELECT media_id, progress_percent as pct FROM watch_history_events
                UNION ALL
                SELECT NULL, CASE WHEN resume_position_seconds = 0 AND duration_seconds > 0 THEN 100.0
                     WHEN duration_seconds > 0 THEN (resume_position_seconds / duration_seconds) * 100.0
                     ELSE 0 END FROM streaming_history WHERE duration_seconds > 0
            )
            SELECT
                COUNT(DISTINCT media_id),
                COALESCE(SUM(CASE WHEN pct >= 25 AND pct < 75 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN pct >= 75 AND pct < 93 THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN pct >= 93 THEN 1 ELSE 0 END), 0)
            FROM all_progress
            WHERE media_id IS NOT NULL",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

        // 11. Library stats (reuse existing query)
        let library_stats = self.get_library_stats(None)?;

        // 12. Recent events (reuse existing query)
        let recent_events = self.get_watch_history_events(20)?;

        Ok(AnalyticsData {
            overview: AnalyticsOverview {
                total_watch_time_seconds: total_watch_time,
                movies_completed,
                episodes_completed,
                total_completion_rate: completion_rate,
                current_streak_days: streak_days,
                total_events,
            },
            heatmap,
            daily_trend,
            content_breakdown,
            source_breakdown,
            top_watched,
            hour_distribution,
            day_distribution,
            completion_funnel: CompletionFunnel {
                started,
                in_progress_25: in_progress,
                mostly_done_75: mostly_done,
                completed,
            },
            library_stats,
            recent_events,
        })
    }

    /// Compute consecutive days with watch activity from all sources.
    fn compute_watch_streak_unified(&self) -> Result<i32> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT day FROM (
                SELECT date(ended_at) as day FROM watch_history_events
                UNION
                SELECT date(last_watched) FROM media WHERE last_watched IS NOT NULL
                UNION
                SELECT date(last_watched) FROM streaming_history WHERE last_watched IS NOT NULL
            ) ORDER BY day DESC",
        )?;

        let days: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        if days.is_empty() {
            return Ok(0);
        }

        let mut streak: i32 = 0;
        let mut expected = chrono::Local::now().format("%Y-%m-%d").to_string();

        // If today has no events, check if yesterday does
        if days.first().map(|d| d.as_str()) != Some(expected.as_str()) {
            expected = (chrono::Local::now() - chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string();
            if days.first().map(|d| d.as_str()) != Some(expected.as_str()) {
                return Ok(0);
            }
        }

        for day in &days {
            if day.as_str() == expected.as_str() {
                streak += 1;
                expected = (chrono::NaiveDate::parse_from_str(&expected, "%Y-%m-%d")
                    .unwrap_or_default()
                    - chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string();
            } else if day.as_str() < expected.as_str() {
                break;
            }
        }

        Ok(streak)
    }

    fn map_media_item(row: &rusqlite::Row) -> rusqlite::Result<MediaItem> {
        let duration: Option<f64> = Self::get_optional_named(row, "duration_seconds");
        let resume_pos: Option<f64> = Self::get_optional_named(row, "resume_position_seconds");
        let last_watched: Option<String> = Self::get_optional_named(row, "last_watched");

        // Calculate progress_percent
        // If resume_position is 0 but last_watched is set and duration > 0,
        // the item was completed (progress was reset after reaching 95%+)
        let progress_percent = match (resume_pos, duration, &last_watched) {
            // Completed: position reset to 0 after finishing, but has watch history
            (Some(pos), Some(dur), Some(_)) if pos == 0.0 && dur > 0.0 => Some(100.0),
            // In progress: has position and duration
            (Some(pos), Some(dur), _) if dur > 0.0 => Some((pos / dur) * 100.0),
            // Default
            _ => Some(0.0),
        };

        // Get is_cloud as integer and convert to bool
        let is_cloud_int: Option<i32> = Self::get_optional_named(row, "is_cloud");
        let is_cloud = is_cloud_int.map(|v| v != 0);

        Ok(MediaItem {
            id: row.get("id")?,
            title: row.get("title")?,
            year: Self::get_optional_named(row, "year"),
            overview: Self::get_optional_named(row, "overview"),
            cast_names: Self::get_optional_named(row, "cast_names"),
            director: Self::get_optional_named(row, "director"),
            poster_path: Self::get_optional_named(row, "poster_path"),
            file_path: Self::get_optional_named(row, "file_path"),
            media_type: row.get("media_type")?,
            duration_seconds: duration,
            resume_position_seconds: resume_pos,
            last_watched,
            season_number: Self::get_optional_named(row, "season_number"),
            episode_number: Self::get_optional_named(row, "episode_number"),
            parent_id: Self::get_optional_named(row, "parent_id"),
            progress_percent,
            tmdb_id: Self::get_optional_named(row, "tmdb_id"),
            imdb_id: Self::get_optional_named(row, "imdb_id"),
            episode_title: Self::get_optional_named(row, "episode_title"),
            still_path: Self::get_optional_named(row, "still_path"),
            archive_format: Self::get_optional_named(row, "archive_format"),
            is_cloud,
            cloud_file_id: Self::get_optional_named(row, "cloud_file_id"),
            cloud_folder_id: Self::get_optional_named(row, "cloud_folder_id"),
            parent_zip_id: Self::get_optional_named(row, "parent_zip_id"),
            zip_entry_path: Self::get_optional_named(row, "zip_entry_path"),
            zip_local_header_offset: Self::get_optional_named(row, "zip_local_header_offset"),
            zip_data_start_offset: Self::get_optional_named(row, "zip_data_start_offset"),
            zip_compressed_size: Self::get_optional_named(row, "zip_compressed_size"),
            zip_uncompressed_size: Self::get_optional_named(row, "zip_uncompressed_size"),
            zip_crc32: Self::get_optional_named(row, "zip_crc32"),
            zip_compression_method: Self::get_optional_named(row, "zip_compression_method"),
            file_size_bytes: Self::get_optional_named(row, "file_size_bytes"),
            ddl_source_id: Self::get_optional_named(row, "ddl_source_id"),
            archive_playback_can_play: None,
            archive_playback_mode: None,
            archive_playback_message: None,
            archive_playback_details: None,
        })
    }

    fn map_watch_history_event(row: &rusqlite::Row) -> rusqlite::Result<WatchHistoryEvent> {
        Ok(WatchHistoryEvent {
            event_id: row.get(0)?,
            media_id: row.get(1)?,
            parent_media_id: row.get(2)?,
            title: row.get(3)?,
            parent_title: row.get(4)?,
            media_type: row.get(5)?,
            year: row.get(6)?,
            overview: row.get(7)?,
            poster_path: row.get(8)?,
            still_path: row.get(9)?,
            tmdb_id: row.get(10)?,
            parent_tmdb_id: row.get(11)?,
            episode_title: row.get(12)?,
            season_number: row.get(13)?,
            episode_number: row.get(14)?,
            is_cloud: row.get::<_, i64>(15)? != 0,
            progress_percent: row.get(16)?,
            resume_position_seconds: row.get(17)?,
            duration_seconds: row.get(18)?,
            completed: row.get::<_, i64>(19)? != 0,
            started_at: row.get(20)?,
            ended_at: row.get(21)?,
            updated_at: row.get(22)?,
        })
    }

    fn map_movie_reminder(row: &rusqlite::Row) -> rusqlite::Result<MovieReminder> {
        Ok(MovieReminder {
            id: row.get(0)?,
            tmdb_id: row.get(1)?,
            media_type: row.get(2)?,
            title: row.get(3)?,
            poster_path: row.get(4)?,
            season_number: row.get(5)?,
            episode_number: row.get(6)?,
            release_date: row.get(7)?,
            reminder_at: row.get(8)?,
            source: row.get(9)?,
            tracking_mode: row.get(10)?,
            tracking_season_number: row.get(11)?,
            notes: row.get(12)?,
            is_active: row.get::<_, i64>(13)? != 0,
            notified_at: row.get(14)?,
            created_at: row.get(15)?,
            updated_at: row.get(16)?,
        })
    }

    fn map_watchlist_item(row: &rusqlite::Row) -> rusqlite::Result<WatchlistItem> {
        Ok(WatchlistItem {
            id: row.get(0)?,
            tmdb_id: row.get(1)?,
            media_type: row.get(2)?,
            title: row.get(3)?,
            poster_path: row.get(4)?,
            release_date: row.get(5)?,
            notes: row.get(6)?,
            is_active: row.get::<_, i64>(7)? != 0,
            notification_enabled: row.get::<_, i64>(8)? != 0,
            notification_mode: row.get(9)?,
            notification_interval_minutes: row.get(10)?,
            notify_at: row.get(11)?,
            last_notified_at: row.get(12)?,
            created_at: row.get(13)?,
            updated_at: row.get(14)?,
        })
    }

    fn get_optional_named<T>(row: &rusqlite::Row, name: &str) -> Option<T>
    where
        T: FromSql,
    {
        let idx = row.as_ref().column_index(name).ok()?;
        row.get(idx).ok()
    }

    // ── Remote Source (External tab) helpers ──

    pub fn add_remote_bookmark(
        &self,
        tmdb_id: &str,
        media_type: &str,
        title: &str,
        year: Option<i32>,
        poster_path: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO remote_bookmarks (tmdb_id, media_type, title, year, poster_path)
             VALUES (?, ?, ?, ?, ?)",
            params![tmdb_id, media_type, title, year, poster_path],
        )?;
        Ok(())
    }

    pub fn remove_remote_bookmark(&self, tmdb_id: &str, media_type: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM remote_bookmarks WHERE tmdb_id = ? AND media_type = ?",
            params![tmdb_id, media_type],
        )?;
        let _ = self.conn.execute(
            "DELETE FROM remote_playback_progress WHERE tmdb_id = ? AND media_type = ?",
            params![tmdb_id, media_type],
        );
        Ok(())
    }

    pub fn get_remote_bookmarks(&self) -> Result<Vec<RemoteBookmark>> {
        let mut stmt = self.conn.prepare(
            "SELECT tmdb_id, media_type, title, year, poster_path, created_at
             FROM remote_bookmarks
             ORDER BY created_at DESC",
        )?;
        let items = stmt.query_map([], |row| {
            Ok(RemoteBookmark {
                tmdb_id: row.get(0)?,
                media_type: row.get(1)?,
                title: row.get(2)?,
                year: row.get(3)?,
                poster_path: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn is_remote_bookmarked(&self, tmdb_id: &str, media_type: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare(
            "SELECT 1 FROM remote_bookmarks WHERE tmdb_id = ? AND media_type = ? LIMIT 1",
        )?;
        let mut rows = stmt.query(params![tmdb_id, media_type])?;
        Ok(rows.next()?.is_some())
    }

    pub fn upsert_playback_progress(
        &self,
        tmdb_id: &str,
        media_type: &str,
        season_number: Option<i32>,
        episode_number: Option<i32>,
        resume_position_seconds: f64,
        duration_seconds: f64,
    ) -> Result<()> {
        let sn = season_number.unwrap_or(-1);
        let en = episode_number.unwrap_or(-1);
        self.conn.execute(
            "INSERT OR REPLACE INTO remote_playback_progress
             (tmdb_id, media_type, season_number, episode_number, resume_position_seconds, duration_seconds, last_watched)
             VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)",
            params![tmdb_id, media_type, sn, en, resume_position_seconds, duration_seconds],
        )?;
        Ok(())
    }

    pub fn get_progress_for_item(
        &self,
        tmdb_id: &str,
        media_type: &str,
        season_number: Option<i32>,
        episode_number: Option<i32>,
    ) -> Result<Option<RemotePlaybackProgress>> {
        let sn = season_number.unwrap_or(-1);
        let en = episode_number.unwrap_or(-1);
        let mut stmt = self.conn.prepare(
            "SELECT tmdb_id, media_type, season_number, episode_number, resume_position_seconds, duration_seconds, last_watched
             FROM remote_playback_progress
             WHERE tmdb_id = ? AND media_type = ?
               AND season_number = ? AND episode_number = ?
             LIMIT 1"
        )?;
        let mut rows = stmt.query(params![tmdb_id, media_type, sn, en])?;
        match rows.next()? {
            Some(row) => {
                let sn: i32 = row.get(2)?;
                let en: i32 = row.get(3)?;
                Ok(Some(RemotePlaybackProgress {
                    tmdb_id: row.get(0)?,
                    media_type: row.get(1)?,
                    season_number: if sn < 0 { None } else { Some(sn) },
                    episode_number: if en < 0 { None } else { Some(en) },
                    resume_position_seconds: row.get(4)?,
                    duration_seconds: row.get(5)?,
                    last_watched: row.get(6)?,
                }))
            }
            None => Ok(None),
        }
    }

    pub fn get_all_progress_for_show(&self, tmdb_id: &str) -> Result<Vec<RemotePlaybackProgress>> {
        let mut stmt = self.conn.prepare(
            "SELECT tmdb_id, media_type, season_number, episode_number, resume_position_seconds, duration_seconds, last_watched
             FROM remote_playback_progress
             WHERE tmdb_id = ? AND media_type = 'tv'
             ORDER BY season_number, episode_number"
        )?;
        let items = stmt.query_map(params![tmdb_id], |row| {
            let sn: i32 = row.get(2)?;
            let en: i32 = row.get(3)?;
            Ok(RemotePlaybackProgress {
                tmdb_id: row.get(0)?,
                media_type: row.get(1)?,
                season_number: if sn < 0 { None } else { Some(sn) },
                episode_number: if en < 0 { None } else { Some(en) },
                resume_position_seconds: row.get(4)?,
                duration_seconds: row.get(5)?,
                last_watched: row.get(6)?,
            })
        })?;
        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn get_all_playback_progress(&self) -> Result<Vec<RemotePlaybackProgress>> {
        let mut stmt = self.conn.prepare(
            "SELECT tmdb_id, media_type, season_number, episode_number, resume_position_seconds, duration_seconds, last_watched
             FROM remote_playback_progress
             ORDER BY last_watched DESC"
        )?;
        let items = stmt.query_map([], |row| {
            let sn: i32 = row.get(2)?;
            let en: i32 = row.get(3)?;
            Ok(RemotePlaybackProgress {
                tmdb_id: row.get(0)?,
                media_type: row.get(1)?,
                season_number: if sn < 0 { None } else { Some(sn) },
                episode_number: if en < 0 { None } else { Some(en) },
                resume_position_seconds: row.get(4)?,
                duration_seconds: row.get(5)?,
                last_watched: row.get(6)?,
            })
        })?;
        items
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>()
            .into_iter()
            .map(Ok)
            .collect()
    }

    pub fn clear_playback_progress(
        &self,
        tmdb_id: &str,
        media_type: &str,
        season_number: Option<i32>,
        episode_number: Option<i32>,
    ) -> Result<()> {
        let sn = season_number.unwrap_or(-1);
        let en = episode_number.unwrap_or(-1);
        self.conn.execute(
            "DELETE FROM remote_playback_progress
             WHERE tmdb_id = ? AND media_type = ?
               AND season_number = ? AND episode_number = ?",
            params![tmdb_id, media_type, sn, en],
        )?;
        Ok(())
    }

    /// Returns all cloud media entries with their cloud_file_id and title.
    /// Used by the ghost entry check to verify files still exist on Drive.
    pub fn get_all_cloud_media_ids(&self) -> Result<Vec<(String, String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT cloud_file_id, title, id FROM media
             WHERE is_cloud = 1 AND cloud_file_id IS NOT NULL AND cloud_file_id != ''",
        )?;
        let items = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        items.collect()
    }

    /// Returns ZIP child entries whose parent ZIP no longer exists in the media table.
    pub fn get_orphaned_zip_entries(&self) -> Result<Vec<(i64, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.title, m.zip_entry_path FROM media m
             WHERE m.parent_zip_id IS NOT NULL
             AND m.ddl_source_id IS NULL
             AND NOT EXISTS (SELECT 1 FROM media p WHERE p.id = m.parent_zip_id)",
        )?;
        let items = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        items.collect()
    }

    /// Returns (has_token, token_value) for the gdrive_changes_token setting.
    pub fn get_changes_token_status(&self) -> Result<(bool, Option<String>)> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM app_settings WHERE key = 'gdrive_changes_token'")?;
        let mut rows = stmt.query_map([], |row| Ok(row.get::<_, String>(0)?))?;
        match rows.next() {
            Some(Ok(token)) => Ok((true, Some(token))),
            _ => Ok((false, None)),
        }
    }

    /// Preserve watch progress for a media entry (and its children) before deletion.
    /// Snapshots current progress into watch_history_events so Continue Watching
    /// survives ghost/orphan cleanup. No-op if the entry has no last_watched.
    pub fn preserve_watch_progress_for_media(&self, media_id: i64) -> Result<()> {
        // Preserve for the entry itself
        let (resume, duration, last_watched): (f64, f64, Option<String>) = self
            .conn
            .query_row(
                "SELECT COALESCE(resume_position_seconds, 0), COALESCE(duration_seconds, 0), last_watched FROM media WHERE id = ?",
                params![media_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap_or((0.0, 0.0, None));

        if last_watched.is_some() && (resume > 0.0 || duration > 0.0) {
            let _ = self.record_watch_event(media_id, resume, duration);
        }

        // Preserve for child episodes (CASCADE will delete them too)
        let children: Vec<(i64, f64, f64)> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, COALESCE(resume_position_seconds, 0), COALESCE(duration_seconds, 0)
                 FROM media WHERE parent_id = ? AND last_watched IS NOT NULL",
            )?;
            let rows: Vec<(i64, f64, f64)> = stmt
                .query_map(params![media_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        for (child_id, child_resume, child_duration) in children {
            let _ = self.record_watch_event(child_id, child_resume, child_duration);
        }

        Ok(())
    }

    /// Removes a media entry by ID. CASCADE handles child episodes.
    pub fn remove_media_by_id(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM media WHERE id = ?", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::fs;
    use std::path::PathBuf;
    use uuid::Uuid;

    /// Create a temp DB, return (db, path) so caller can clean up.
    fn make_db() -> (Database, PathBuf) {
        let db_path =
            std::env::temp_dir().join(format!("slasshyvault-db-test-{}.db", Uuid::new_v4()));
        let db = Database::new(db_path.to_str().unwrap()).unwrap();
        (db, db_path)
    }

    fn cleanup(db: Database, db_path: PathBuf) {
        drop(db);
        let _ = fs::remove_file(db_path);
    }

    // ==================== INIT / MIGRATION ====================

    #[test]
    fn init_migrates_legacy_media_table_before_creating_indexes() {
        let db_path =
            std::env::temp_dir().join(format!("slasshyvault-db-test-{}.db", Uuid::new_v4()));

        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute(
                "CREATE TABLE media (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL,
                    year INTEGER,
                    overview TEXT,
                    poster_path TEXT,
                    file_path TEXT NOT NULL UNIQUE,
                    media_type TEXT NOT NULL,
                    parent_id INTEGER,
                    season_number INTEGER,
                    episode_number INTEGER,
                    duration_seconds REAL DEFAULT 0,
                    resume_position_seconds REAL DEFAULT 0,
                    last_watched TIMESTAMP DEFAULT NULL,
                    tmdb_id TEXT DEFAULT NULL,
                    FOREIGN KEY (parent_id) REFERENCES media (id) ON DELETE CASCADE
                )",
                [],
            )
            .unwrap();
        }

        let db = Database::new(db_path.to_str().unwrap()).unwrap();
        let index_names: Vec<String> = {
            let mut stmt = db.conn.prepare("PRAGMA index_list('media')").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .filter_map(|row| row.ok())
                .collect()
        };

        for expected in [
            "idx_media_type_title",
            "idx_media_parent_order",
            "idx_media_last_watched",
            "idx_media_cloud_folder_id",
            "idx_media_type_tmdb_id",
            "idx_media_cloud_file_id",
        ] {
            assert!(
                index_names.iter().any(|name| name == expected),
                "missing index: {}",
                expected
            );
        }

        drop(db);
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn init_creates_all_tables() {
        let (db, db_path) = make_db();
        let table_names: Vec<String> = {
            let mut stmt = db
                .conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert!(table_names.contains(&"media".to_string()));
        assert!(table_names.contains(&"app_settings".to_string()));
        assert!(table_names.contains(&"movie_reminders".to_string()));
        assert!(table_names.contains(&"watchlist_items".to_string()));
        assert!(table_names.contains(&"cloud_folders".to_string()));
        assert!(table_names.contains(&"ddl_sources".to_string()));
        assert!(table_names.contains(&"zip_archives".to_string()));
        assert!(table_names.contains(&"watch_history_events".to_string()));
        assert!(table_names.contains(&"streaming_history".to_string()));
        cleanup(db, db_path);
    }

    // ==================== CRUD: INSERT ====================

    #[test]
    fn insert_movie_returns_id_and_persists() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Test Movie",
                Some(2024),
                Some("An overview"),
                Some("Actor A"),
                Some("Director X"),
                Some("/poster.jpg"),
                "/movies/test.mkv",
                7200.0,
                Some("tmdb123"),
            )
            .unwrap();
        assert!(id > 0);

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.title, "Test Movie");
        assert_eq!(item.year, Some(2024));
        assert_eq!(item.overview.as_deref(), Some("An overview"));
        assert_eq!(item.cast_names.as_deref(), Some("Actor A"));
        assert_eq!(item.director.as_deref(), Some("Director X"));
        assert_eq!(item.poster_path.as_deref(), Some("/poster.jpg"));
        assert_eq!(item.file_path.as_deref(), Some("/movies/test.mkv"));
        assert_eq!(item.media_type, "movie");
        assert_eq!(item.duration_seconds, Some(7200.0));
        assert_eq!(item.tmdb_id.as_deref(), Some("tmdb123"));
        cleanup(db, db_path);
    }

    #[test]
    fn insert_tvshow_returns_id_and_persists() {
        let (db, db_path) = make_db();
        let id = db
            .insert_tvshow(
                "Test Show",
                Some(2023),
                Some("Show overview"),
                Some("Actor B"),
                Some("/show_poster.jpg"),
                "/tv/test-show",
                Some("tmdb456"),
            )
            .unwrap();
        assert!(id > 0);

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.title, "Test Show");
        assert_eq!(item.year, Some(2023));
        assert_eq!(item.media_type, "tvshow");
        assert_eq!(item.tmdb_id.as_deref(), Some("tmdb456"));
        cleanup(db, db_path);
    }

    #[test]
    fn insert_episode_returns_id_and_persists() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", Some(2024), None, None, None, "/tv/show", None)
            .unwrap();
        let ep_id = db
            .insert_episode("Pilot", "/tv/show/s01e01.mkv", show_id, 1, 1, 2400.0)
            .unwrap();
        assert!(ep_id > 0);

        let item = db.get_media_by_id(ep_id).unwrap();
        assert_eq!(item.title, "Pilot");
        assert_eq!(item.media_type, "tvepisode");
        assert_eq!(item.parent_id, Some(show_id));
        assert_eq!(item.season_number, Some(1));
        assert_eq!(item.episode_number, Some(1));
        assert_eq!(item.duration_seconds, Some(2400.0));
        cleanup(db, db_path);
    }

    #[test]
    fn insert_episode_with_metadata_persists_all_fields() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/show", None)
            .unwrap();
        let ep_id = db
            .insert_episode_with_metadata(
                "Show",
                "/tv/show/s01e02.mkv",
                show_id,
                1,
                2,
                2500.0,
                Some("The Second"),
                Some("Episode overview"),
                Some("/still.jpg"),
            )
            .unwrap();

        let item = db.get_media_by_id(ep_id).unwrap();
        assert_eq!(item.episode_title.as_deref(), Some("The Second"));
        assert_eq!(item.overview.as_deref(), Some("Episode overview"));
        assert_eq!(item.still_path.as_deref(), Some("/still.jpg"));
        cleanup(db, db_path);
    }

    // ==================== CRUD: REMOVE ====================

    #[test]
    fn remove_media_returns_poster_path_and_deletes() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "To Delete",
                None,
                None,
                None,
                None,
                Some("/poster.jpg"),
                "/del.mkv",
                0.0,
                None,
            )
            .unwrap();
        let poster = db.remove_media(id).unwrap();
        assert_eq!(poster.as_deref(), Some("/poster.jpg"));

        // Verify deleted
        assert!(db.get_media_by_id(id).is_err());
        cleanup(db, db_path);
    }

    #[test]
    fn remove_media_by_file_path_returns_info_and_deletes() {
        let (db, db_path) = make_db();
        db.insert_movie(
            "Path Movie",
            None,
            None,
            None,
            None,
            Some("/p.jpg"),
            "/path/movie.mkv",
            0.0,
            None,
        )
        .unwrap();

        let info = db.remove_media_by_file_path("/path/movie.mkv").unwrap();
        assert!(info.is_some());
        let (id, title, poster, _still) = info.unwrap();
        assert!(id > 0);
        assert_eq!(title, "Path Movie");
        assert_eq!(poster.as_deref(), Some("/p.jpg"));

        // Verify deleted
        assert!(db
            .remove_media_by_file_path("/path/movie.mkv")
            .unwrap()
            .is_none());
        cleanup(db, db_path);
    }

    #[test]
    fn remove_series_episodes_deletes_all_children() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/show", None)
            .unwrap();
        db.insert_episode("Ep1", "/tv/show/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();
        db.insert_episode("Ep2", "/tv/show/s01e02.mkv", show_id, 1, 2, 0.0)
            .unwrap();

        db.remove_series_episodes(show_id).unwrap();
        let eps = db.get_episodes(show_id).unwrap();
        assert!(eps.is_empty());
        cleanup(db, db_path);
    }

    // ==================== CRUD: GET / QUERY ====================

    #[test]
    fn get_media_by_id_not_found_returns_err() {
        let (db, db_path) = make_db();
        assert!(db.get_media_by_id(99999).is_err());
        cleanup(db, db_path);
    }

    #[test]
    fn get_library_returns_items_by_type() {
        let (db, db_path) = make_db();
        db.insert_movie("Alpha", None, None, None, None, None, "/a.mkv", 0.0, None)
            .unwrap();
        db.insert_movie("Beta", None, None, None, None, None, "/b.mkv", 0.0, None)
            .unwrap();
        db.insert_tvshow("Show", None, None, None, None, "/tv/show", None)
            .unwrap();

        let movies = db.get_library("movie", None).unwrap();
        assert_eq!(movies.len(), 2);
        let shows = db.get_library("tvshow", None).unwrap();
        assert_eq!(shows.len(), 1);
        cleanup(db, db_path);
    }

    #[test]
    fn get_library_search_filters_by_title() {
        let (db, db_path) = make_db();
        db.insert_movie("Alpha", None, None, None, None, None, "/a.mkv", 0.0, None)
            .unwrap();
        db.insert_movie("Beta", None, None, None, None, None, "/b.mkv", 0.0, None)
            .unwrap();

        let found = db.get_library("movie", Some("lph")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Alpha");
        cleanup(db, db_path);
    }

    #[test]
    fn get_library_filtered_by_cloud_status() {
        let (db, db_path) = make_db();
        db.insert_movie(
            "Local",
            None,
            None,
            None,
            None,
            None,
            "/local.mkv",
            0.0,
            None,
        )
        .unwrap();
        db.insert_cloud_movie(
            "Cloud",
            None,
            None,
            None,
            None,
            None,
            "cloud.mkv",
            "cf1",
            "folder1",
            0.0,
            None,
        )
        .unwrap();

        let local_only = db.get_library_filtered("movie", None, Some(false)).unwrap();
        assert_eq!(local_only.len(), 1);
        assert_eq!(local_only[0].title, "Local");

        let cloud_only = db.get_library_filtered("movie", None, Some(true)).unwrap();
        assert_eq!(cloud_only.len(), 1);
        assert_eq!(cloud_only[0].title, "Cloud");
        cleanup(db, db_path);
    }

    #[test]
    fn get_recently_added_returns_limited_results() {
        let (db, db_path) = make_db();
        for i in 0..5 {
            db.insert_movie(
                &format!("Movie {}", i),
                None,
                None,
                None,
                None,
                None,
                &format!("/m{}.mkv", i),
                0.0,
                None,
            )
            .unwrap();
        }

        let recent = db.get_recently_added(3, None).unwrap();
        assert_eq!(recent.len(), 3);
        // Should be ordered by id DESC
        assert_eq!(recent[0].title, "Movie 4");
        cleanup(db, db_path);
    }

    #[test]
    fn get_episodes_returns_children_ordered() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/show", None)
            .unwrap();
        db.insert_episode("Ep2", "/s01e02.mkv", show_id, 1, 2, 0.0)
            .unwrap();
        db.insert_episode("Ep1", "/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();
        db.insert_episode("Ep3", "/s02e01.mkv", show_id, 2, 1, 0.0)
            .unwrap();

        let eps = db.get_episodes(show_id).unwrap();
        assert_eq!(eps.len(), 3);
        // Ordered by season, episode
        assert_eq!(eps[0].episode_number, Some(1));
        assert_eq!(eps[1].episode_number, Some(2));
        assert_eq!(eps[2].season_number, Some(2));
        cleanup(db, db_path);
    }

    // ==================== WATCH HISTORY ====================

    #[test]
    fn update_progress_sets_resume_position() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Movie", None, None, None, None, None, "/m.mkv", 7200.0, None,
            )
            .unwrap();

        db.update_progress(id, 3600.0, 7200.0).unwrap();

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.resume_position_seconds, Some(3600.0));
        assert_eq!(item.duration_seconds, Some(7200.0));
        assert!(item.last_watched.is_some());
        cleanup(db, db_path);
    }

    #[test]
    fn update_progress_near_end_clears_position() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Movie", None, None, None, None, None, "/m.mkv", 7200.0, None,
            )
            .unwrap();

        // 95% watched -> should clear progress (threshold is 93%)
        db.update_progress(id, 6840.0, 7200.0).unwrap();

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.resume_position_seconds, Some(0.0));
        assert!(item.last_watched.is_some());
        cleanup(db, db_path);
    }

    #[test]
    fn get_resume_info_returns_correct_state() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Movie", None, None, None, None, None, "/m.mkv", 3600.0, None,
            )
            .unwrap();

        // No progress yet
        let info = db.get_resume_info(id).unwrap();
        assert!(!info.has_progress);
        assert_eq!(info.position, 0.0);

        // Set progress
        db.update_progress(id, 1800.0, 3600.0).unwrap();
        let info = db.get_resume_info(id).unwrap();
        assert!(info.has_progress);
        assert!((info.position - 1800.0).abs() < 1.0);
        assert!((info.progress_percent - 50.0).abs() < 1.0);
        assert_eq!(info.time_str, "00:30:00");
        cleanup(db, db_path);
    }

    #[test]
    fn clear_progress_resets_position() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Movie", None, None, None, None, None, "/m.mkv", 3600.0, None,
            )
            .unwrap();
        db.update_progress(id, 1800.0, 3600.0).unwrap();
        db.clear_progress(id).unwrap();

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.resume_position_seconds, Some(0.0));
        cleanup(db, db_path);
    }

    #[test]
    fn get_watch_history_returns_watched_items() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Watched", None, None, None, None, None, "/w.mkv", 3600.0, None,
            )
            .unwrap();
        db.update_progress(id, 1800.0, 3600.0).unwrap();

        let history = db.get_watch_history(10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].title, "Watched");
        cleanup(db, db_path);
    }

    #[test]
    fn get_watch_history_excludes_unwatched() {
        let (db, db_path) = make_db();
        db.insert_movie(
            "Unwatched",
            None,
            None,
            None,
            None,
            None,
            "/u.mkv",
            3600.0,
            None,
        )
        .unwrap();

        let history = db.get_watch_history(10).unwrap();
        assert!(history.is_empty());
        cleanup(db, db_path);
    }

    #[test]
    fn record_watch_event_creates_event() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Movie", None, None, None, None, None, "/m.mkv", 3600.0, None,
            )
            .unwrap();

        // update_progress calls record_watch_event internally for plays >= 10s
        db.update_progress(id, 1800.0, 3600.0).unwrap();

        let events = db.get_watch_history_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "Movie");
        assert_eq!(events[0].media_type, "movie");
        assert!(!events[0].completed);
        cleanup(db, db_path);
    }

    #[test]
    fn record_watch_event_marks_completed() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Movie", None, None, None, None, None, "/m.mkv", 3600.0, None,
            )
            .unwrap();

        // 95% -> completed
        db.update_progress(id, 3420.0, 3600.0).unwrap();

        let events = db.get_watch_history_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].completed);
        cleanup(db, db_path);
    }

    // ==================== SETTINGS ====================

    #[test]
    fn setting_roundtrip() {
        let (db, db_path) = make_db();

        assert!(db.get_setting("nonexistent").unwrap().is_none());

        db.set_setting("theme", "dark").unwrap();
        let val = db.get_setting("theme").unwrap();
        assert_eq!(val.as_deref(), Some("dark"));

        // Overwrite
        db.set_setting("theme", "light").unwrap();
        let val = db.get_setting("theme").unwrap();
        assert_eq!(val.as_deref(), Some("light"));

        cleanup(db, db_path);
    }

    // ==================== CLOUD ====================

    #[test]
    fn insert_cloud_movie_persists() {
        let (db, db_path) = make_db();
        let id = db
            .insert_cloud_movie(
                "Cloud Movie",
                Some(2024),
                Some("Cloud overview"),
                None,
                None,
                None,
                "cloud.mkv",
                "file-123",
                "folder-abc",
                5400.0,
                Some("tmdb-cloud"),
            )
            .unwrap();
        assert!(id > 0);

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.title, "Cloud Movie");
        assert_eq!(item.is_cloud, Some(true));
        assert_eq!(item.cloud_file_id.as_deref(), Some("file-123"));
        assert_eq!(item.file_path, Some("gdrive:file-123".to_string()));
        cleanup(db, db_path);
    }

    #[test]
    fn insert_cloud_tvshow_persists() {
        let (db, db_path) = make_db();
        let id = db
            .insert_cloud_tvshow(
                "Cloud Show",
                Some(2023),
                None,
                None,
                None,
                "cloud-show-folder",
                "folder-xyz",
                Some("tmdb-show"),
            )
            .unwrap();

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.media_type, "tvshow");
        assert_eq!(item.is_cloud, Some(true));
        cleanup(db, db_path);
    }

    #[test]
    fn cloud_folders_crud() {
        let (db, db_path) = make_db();

        // Empty
        let folders = db.get_cloud_folders().unwrap();
        assert!(folders.is_empty());

        // Add
        let _ = db.add_cloud_folder("f1", "My Folder").unwrap();
        let _ = db.add_cloud_folder("f2", "Other Folder").unwrap();
        let folders = db.get_cloud_folders().unwrap();
        assert_eq!(folders.len(), 2);

        // Remove
        let removed = db.remove_cloud_folder("f1").unwrap();
        assert_eq!(removed, 1);
        let folders = db.get_cloud_folders().unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].0, "f2");

        cleanup(db, db_path);
    }

    // ==================== ZIP ARCHIVES ====================

    #[test]
    fn zip_archive_roundtrip() {
        let (db, db_path) = make_db();
        let archive = zip_manager::ZipArchiveInfo {
            zip_file_id: "zip-001".to_string(),
            filename: "test.zip".to_string(),
            archive_format: "zip".to_string(),
            file_size_bytes: 1024000,
            compression_type: crate::zip_parser::ZipCompressionType::Deflate,
            central_dir_offset: 1020000,
            central_dir_size: 4000,
            total_entries: 10,
            video_entries: 3,
        };

        db.insert_zip_archive(&archive).unwrap();

        let record = db.get_zip_archive("zip-001").unwrap();
        assert_eq!(record.zip_file_id, "zip-001");
        assert_eq!(record.filename, "test.zip");
        assert_eq!(record.archive_format, "zip");
        assert_eq!(record.file_size_bytes, 1024000);
        assert_eq!(record.total_entries, 10);
        assert_eq!(record.video_entries, 3);

        // Delete
        let deleted = db.delete_zip_archive("zip-001").unwrap();
        assert_eq!(deleted, 1);

        // Verify deleted
        assert!(db.get_zip_archive("zip-001").is_err());
        cleanup(db, db_path);
    }

    #[test]
    fn insert_zip_archive_upserts_on_conflict() {
        let (db, db_path) = make_db();
        let archive = zip_manager::ZipArchiveInfo {
            zip_file_id: "zip-upsert".to_string(),
            filename: "original.zip".to_string(),
            archive_format: "zip".to_string(),
            file_size_bytes: 500,
            compression_type: crate::zip_parser::ZipCompressionType::Deflate,
            central_dir_offset: 400,
            central_dir_size: 100,
            total_entries: 5,
            video_entries: 1,
        };
        db.insert_zip_archive(&archive).unwrap();

        // Upsert with different data
        let updated = zip_manager::ZipArchiveInfo {
            zip_file_id: "zip-upsert".to_string(),
            filename: "updated.zip".to_string(),
            archive_format: "7z".to_string(),
            file_size_bytes: 800,
            compression_type: crate::zip_parser::ZipCompressionType::Store,
            central_dir_offset: 700,
            central_dir_size: 100,
            total_entries: 8,
            video_entries: 2,
        };
        db.insert_zip_archive(&updated).unwrap();

        let record = db.get_zip_archive("zip-upsert").unwrap();
        assert_eq!(record.filename, "updated.zip");
        assert_eq!(record.archive_format, "7z");
        assert_eq!(record.file_size_bytes, 800);
        assert_eq!(record.total_entries, 8);

        cleanup(db, db_path);
    }

    // ==================== METADATA ====================

    #[test]
    fn update_poster_path_changes_path() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie("M", None, None, None, None, None, "/m.mkv", 0.0, None)
            .unwrap();

        db.update_poster_path(id, "/new_poster.jpg").unwrap();

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.poster_path.as_deref(), Some("/new_poster.jpg"));
        cleanup(db, db_path);
    }

    #[test]
    fn update_metadata_changes_fields() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Old Title",
                None,
                None,
                None,
                None,
                None,
                "/m.mkv",
                0.0,
                None,
            )
            .unwrap();

        let meta = crate::tmdb::TmdbMetadata {
            title: "New Title".to_string(),
            year: Some(2025),
            overview: Some("Great movie".to_string()),
            cast_names: Some("Actor 1, Actor 2".to_string()),
            director: Some("Director Y".to_string()),
            poster_path: Some("/new_poster.jpg".to_string()),
            tmdb_id: Some("tmdb999".to_string()),
            imdb_id: Some("imdb999".to_string()),
            runtime_seconds: Some(5400.0),
            imdb_image_url: None,
        };
        db.update_metadata(id, &meta).unwrap();

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.title, "New Title");
        assert_eq!(item.year, Some(2025));
        assert_eq!(item.overview.as_deref(), Some("Great movie"));
        assert_eq!(item.cast_names.as_deref(), Some("Actor 1, Actor 2"));
        assert_eq!(item.director.as_deref(), Some("Director Y"));
        assert_eq!(item.poster_path.as_deref(), Some("/new_poster.jpg"));
        assert_eq!(item.tmdb_id.as_deref(), Some("tmdb999"));
        // Duration should be set since it was 0
        assert_eq!(item.duration_seconds, Some(5400.0));
        cleanup(db, db_path);
    }

    #[test]
    fn update_metadata_preserves_existing_duration() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie("M", None, None, None, None, None, "/m.mkv", 3600.0, None)
            .unwrap();

        let meta = crate::tmdb::TmdbMetadata {
            title: "M".to_string(),
            year: None,
            overview: None,
            cast_names: None,
            director: None,
            poster_path: None,
            tmdb_id: None,
            imdb_id: None,
            runtime_seconds: Some(5400.0),
            imdb_image_url: None,
        };
        db.update_metadata(id, &meta).unwrap();

        let item = db.get_media_by_id(id).unwrap();
        // Duration should NOT change because it was already > 0
        assert_eq!(item.duration_seconds, Some(3600.0));
        cleanup(db, db_path);
    }

    // ==================== STATS ====================

    #[test]
    fn get_library_stats_returns_counts_without_loading_rows() {
        let (db, db_path) = make_db();

        db.insert_movie(
            "Movie One",
            Some(2024),
            None,
            None,
            None,
            None,
            "movie-1.mkv",
            120.0,
            None,
        )
        .unwrap();
        db.insert_tvshow("Show One", Some(2024), None, None, None, "show-1", None)
            .unwrap();
        db.insert_episode("Episode One", "episode-1.mkv", 2, 1, 1, 42.0)
            .unwrap();
        db.insert_cloud_movie(
            "Cloud Movie",
            Some(2024),
            None,
            None,
            None,
            None,
            "cloud-movie.mkv",
            "cloud-file-1",
            "cloud-folder-1",
            90.0,
            None,
        )
        .unwrap();
        db.insert_cloud_tvshow(
            "Cloud Show",
            Some(2024),
            None,
            None,
            None,
            "cloud-show",
            "cloud-folder-2",
            None,
        )
        .unwrap();

        let all_stats = db.get_library_stats(None).unwrap();
        assert_eq!(all_stats.movies, 2);
        assert_eq!(all_stats.shows, 2);
        assert_eq!(all_stats.episodes, 1);

        let cloud_stats = db.get_library_stats(Some(true)).unwrap();
        assert_eq!(cloud_stats.movies, 1);
        assert_eq!(cloud_stats.shows, 1);
        assert_eq!(cloud_stats.episodes, 0);

        cleanup(db, db_path);
    }

    #[test]
    fn get_watch_stats_returns_zero_on_empty_db() {
        let (db, db_path) = make_db();
        let stats = db.get_watch_stats().unwrap();
        assert_eq!(stats.movies_watched, 0);
        assert_eq!(stats.episodes_watched, 0);
        assert_eq!(stats.total_watch_time_seconds, 0.0);
        cleanup(db, db_path);
    }

    #[test]
    fn get_watch_stats_counts_completed_media() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Movie", None, None, None, None, None, "/m.mkv", 7200.0, None,
            )
            .unwrap();

        // Mark as watched (progress near end clears resume position)
        db.update_progress(id, 7000.0, 7200.0).unwrap();

        let stats = db.get_watch_stats().unwrap();
        assert_eq!(stats.movies_watched, 1);
        assert!(stats.total_watch_time_seconds > 0.0);
        cleanup(db, db_path);
    }

    // ==================== ANALYTICS ====================

    #[test]
    fn get_analytics_data_returns_structured_data() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "Movie", None, None, None, None, None, "/m.mkv", 7200.0, None,
            )
            .unwrap();
        db.update_progress(id, 1800.0, 7200.0).unwrap();

        let analytics = db.get_analytics_data().unwrap();
        // Should have at least one recent event
        assert!(!analytics.recent_events.is_empty());
        // Library stats should reflect the movie
        assert_eq!(analytics.library_stats.movies, 1);
        // Overview should be populated
        assert!(analytics.overview.total_events > 0);
        cleanup(db, db_path);
    }

    // ==================== DDL SOURCES ====================

    #[test]
    fn ddl_source_roundtrip() {
        let (db, db_path) = make_db();

        db.upsert_ddl_source(
            "ddl-1",
            "https://example.com/file.zip",
            "file.zip",
            1024000,
            "zip",
            5,
            3,
            1000,
            500,
            None,
        )
        .unwrap();

        let sources = db.get_ddl_sources().unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, "ddl-1");
        assert_eq!(sources[0].url, "https://example.com/file.zip");
        assert_eq!(sources[0].filename, "file.zip");
        assert_eq!(sources[0].file_size, 1024000);
        assert!(!sources[0].is_expired);

        let source = db.get_ddl_source("ddl-1").unwrap();
        assert_eq!(source.id, "ddl-1");

        cleanup(db, db_path);
    }

    #[test]
    fn ddl_source_upsert_updates_on_conflict() {
        let (db, db_path) = make_db();

        db.upsert_ddl_source(
            "ddl-2",
            "https://old.url",
            "old.zip",
            100,
            "zip",
            1,
            1,
            0,
            0,
            None,
        )
        .unwrap();

        db.upsert_ddl_source(
            "ddl-2",
            "https://new.url",
            "new.zip",
            200,
            "7z",
            5,
            3,
            100,
            50,
            Some("addon:123"),
        )
        .unwrap();

        let sources = db.get_ddl_sources().unwrap();
        assert_eq!(sources.len(), 1); // still one record
        assert_eq!(sources[0].url, "https://new.url");
        assert_eq!(sources[0].filename, "new.zip");
        assert_eq!(sources[0].file_size, 200);
        cleanup(db, db_path);
    }

    #[test]
    fn delete_ddl_source_and_media_removes_all() {
        let (db, db_path) = make_db();

        db.upsert_ddl_source(
            "ddl-del",
            "https://example.com/file.zip",
            "file.zip",
            1024,
            "zip",
            2,
            1,
            0,
            0,
            None,
        )
        .unwrap();

        // Insert a DDL media item linked to this source
        db.insert_ddl_episode(
            "DDL Ep",
            None,
            Some(1),
            Some(1),
            "ddl-del",
            "zip",
            "ep1.mkv",
            0,
            0,
            0,
            0,
            "",
            0,
            None,
            None,
            None,
        )
        .unwrap();

        let deleted = db.delete_ddl_source_and_media("ddl-del").unwrap();
        assert_eq!(deleted, 1);

        // Source should be gone
        assert!(db.get_ddl_source("ddl-del").is_err());
        let sources = db.get_ddl_sources().unwrap();
        assert!(sources.is_empty());

        cleanup(db, db_path);
    }

    // ==================== WATCHLIST ====================

    #[test]
    fn watchlist_item_create_and_get() {
        let (db, db_path) = make_db();

        let item = db
            .create_or_update_watchlist_item(NewWatchlistItem {
                tmdb_id: "tmdb-wl-1",
                media_type: "movie",
                title: "Watchlist Movie",
                poster_path: Some("/wl.jpg"),
                release_date: Some("2025-06-01"),
                notes: Some("Looks good"),
                is_active: true,
                notification_enabled: false,
                notification_mode: "off",
                notification_interval_minutes: None,
                notify_at: None,
            })
            .unwrap();

        assert!(item.id > 0);
        assert_eq!(item.tmdb_id, "tmdb-wl-1");
        assert_eq!(item.title, "Watchlist Movie");
        assert!(item.is_active);

        // Get by ID
        let fetched = db.get_watchlist_item(item.id).unwrap();
        assert_eq!(fetched.title, "Watchlist Movie");

        // Get all
        let all = db.get_watchlist_items(false).unwrap();
        assert_eq!(all.len(), 1);

        cleanup(db, db_path);
    }

    #[test]
    fn watchlist_upsert_updates_on_conflict() {
        let (db, db_path) = make_db();

        db.create_or_update_watchlist_item(NewWatchlistItem {
            tmdb_id: "tmdb-up",
            media_type: "movie",
            title: "Old Title",
            poster_path: None,
            release_date: None,
            notes: None,
            is_active: true,
            notification_enabled: false,
            notification_mode: "off",
            notification_interval_minutes: None,
            notify_at: None,
        })
        .unwrap();

        let updated = db
            .create_or_update_watchlist_item(NewWatchlistItem {
                tmdb_id: "tmdb-up",
                media_type: "movie",
                title: "New Title",
                poster_path: Some("/new.jpg"),
                release_date: Some("2026-01-01"),
                notes: Some("Updated notes"),
                is_active: false,
                notification_enabled: true,
                notification_mode: "release",
                notification_interval_minutes: Some(60),
                notify_at: Some("2026-01-01T00:00:00Z"),
            })
            .unwrap();

        assert_eq!(updated.title, "New Title");
        assert_eq!(updated.poster_path.as_deref(), Some("/new.jpg"));
        assert!(!updated.is_active);
        assert!(updated.notification_enabled);

        // Only one record
        let all = db.get_watchlist_items(true).unwrap();
        assert_eq!(all.len(), 1);
        cleanup(db, db_path);
    }

    #[test]
    fn delete_watchlist_item_removes_record() {
        let (db, db_path) = make_db();

        let item = db
            .create_or_update_watchlist_item(NewWatchlistItem {
                tmdb_id: "tmdb-del",
                media_type: "movie",
                title: "To Delete",
                poster_path: None,
                release_date: None,
                notes: None,
                is_active: true,
                notification_enabled: false,
                notification_mode: "off",
                notification_interval_minutes: None,
                notify_at: None,
            })
            .unwrap();

        db.delete_watchlist_item(item.id).unwrap();

        let all = db.get_watchlist_items(true).unwrap();
        assert!(all.is_empty());
        cleanup(db, db_path);
    }

    // ==================== MOVIE REMINDERS ====================

    #[test]
    fn movie_reminder_create_and_get() {
        let (db, db_path) = make_db();

        let reminder = db
            .create_movie_reminder(NewMovieReminder {
                tmdb_id: "tmdb-rem-1",
                media_type: "movie",
                title: "Upcoming Movie",
                poster_path: Some("/rem.jpg"),
                season_number: None,
                episode_number: None,
                release_date: Some("2026-12-25"),
                reminder_at: "2026-12-20T00:00:00Z",
                source: "tmdb",
                tracking_mode: "single",
                tracking_season_number: None,
                notes: Some("Don't miss it"),
                is_active: true,
            })
            .unwrap();

        assert!(reminder.id > 0);
        assert_eq!(reminder.title, "Upcoming Movie");
        assert!(reminder.is_active);

        // Get by ID
        let fetched = db.get_movie_reminder(reminder.id).unwrap();
        assert_eq!(fetched.title, "Upcoming Movie");

        // Get all (active only)
        let all = db.get_movie_reminders(false).unwrap();
        assert_eq!(all.len(), 1);

        // Get all including inactive
        db.set_movie_reminder_active(reminder.id, false).unwrap();
        let all = db.get_movie_reminders(false).unwrap();
        assert!(all.is_empty());
        let all_incl = db.get_movie_reminders(true).unwrap();
        assert_eq!(all_incl.len(), 1);

        cleanup(db, db_path);
    }

    #[test]
    fn delete_movie_reminder_removes_record() {
        let (db, db_path) = make_db();

        let reminder = db
            .create_movie_reminder(NewMovieReminder {
                tmdb_id: "tmdb-del",
                media_type: "movie",
                title: "To Delete",
                poster_path: None,
                season_number: None,
                episode_number: None,
                release_date: None,
                reminder_at: "2026-01-01T00:00:00Z",
                source: "tmdb",
                tracking_mode: "single",
                tracking_season_number: None,
                notes: None,
                is_active: true,
            })
            .unwrap();

        db.delete_movie_reminder(reminder.id).unwrap();
        assert!(db.get_movie_reminder(reminder.id).is_err());
        cleanup(db, db_path);
    }

    #[test]
    fn get_due_movie_reminders_returns_overdue() {
        let (db, db_path) = make_db();

        // Past reminder
        db.create_movie_reminder(NewMovieReminder {
            tmdb_id: "tmdb-due",
            media_type: "movie",
            title: "Due",
            poster_path: None,
            season_number: None,
            episode_number: None,
            release_date: None,
            reminder_at: "2020-01-01T00:00:00Z",
            source: "tmdb",
            tracking_mode: "single",
            tracking_season_number: None,
            notes: None,
            is_active: true,
        })
        .unwrap();

        // Future reminder
        db.create_movie_reminder(NewMovieReminder {
            tmdb_id: "tmdb-future",
            media_type: "movie",
            title: "Future",
            poster_path: None,
            season_number: None,
            episode_number: None,
            release_date: None,
            reminder_at: "2099-01-01T00:00:00Z",
            source: "tmdb",
            tracking_mode: "single",
            tracking_season_number: None,
            notes: None,
            is_active: true,
        })
        .unwrap();

        let due = db.get_due_movie_reminders("2025-01-01T00:00:00Z").unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].title, "Due");

        cleanup(db, db_path);
    }

    // ==================== ADDITIONAL CRUD METHODS ====================

    #[test]
    fn get_all_file_paths_returns_non_tvshow_paths() {
        let (db, db_path) = make_db();
        db.insert_movie(
            "M",
            None,
            None,
            None,
            None,
            None,
            "/movies/m.mkv",
            0.0,
            None,
        )
        .unwrap();
        db.insert_tvshow("S", None, None, None, None, "/tv/s", None)
            .unwrap();
        db.insert_episode("E", "/tv/s/s01e01.mp4", 2, 1, 1, 0.0)
            .unwrap();

        let paths = db.get_all_file_paths().unwrap();
        assert!(paths.contains(&"/movies/m.mkv".to_string()));
        assert!(paths.contains(&"/tv/s/s01e01.mp4".to_string()));
        // tvshow folder path should be excluded
        assert!(!paths.iter().any(|p| p == "/tv/s"));
        cleanup(db, db_path);
    }

    #[test]
    fn get_media_by_file_path_finds_existing() {
        let (db, db_path) = make_db();
        db.insert_movie("M", None, None, None, None, None, "/find/me.mkv", 0.0, None)
            .unwrap();

        let found = db.get_media_by_file_path("/find/me.mkv").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "M");

        let not_found = db.get_media_by_file_path("/nope.mkv").unwrap();
        assert!(not_found.is_none());
        cleanup(db, db_path);
    }

    #[test]
    fn cleanup_empty_series_removes_childless_shows() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Empty", None, None, None, None, "/tv/empty", None)
            .unwrap();

        let cleaned = db.cleanup_empty_series().unwrap();
        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned[0].0, show_id);

        // Verify deleted
        assert!(db.get_media_by_id(show_id).is_err());
        cleanup(db, db_path);
    }

    #[test]
    fn cleanup_empty_series_keeps_shows_with_episodes() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("HasEps", None, None, None, None, "/tv/he", None)
            .unwrap();
        db.insert_episode("Ep", "/tv/he/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();

        let cleaned = db.cleanup_empty_series().unwrap();
        assert!(cleaned.is_empty());

        // Show still exists
        assert!(db.get_media_by_id(show_id).is_ok());
        cleanup(db, db_path);
    }

    #[test]
    fn find_series_by_folder_returns_show_id() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/show", None)
            .unwrap();

        let found = db.find_series_by_folder("/tv/show").unwrap();
        assert_eq!(found, Some(show_id));

        let not_found = db.find_series_by_folder("/tv/other").unwrap();
        assert!(not_found.is_none());
        cleanup(db, db_path);
    }

    #[test]
    fn get_media_file_paths_returns_paths_for_ids() {
        let (db, db_path) = make_db();
        let id1 = db
            .insert_movie("A", None, None, None, None, None, "/a.mkv", 0.0, None)
            .unwrap();
        let id2 = db
            .insert_movie("B", None, None, None, None, None, "/b.mkv", 0.0, None)
            .unwrap();

        let paths = db.get_media_file_paths(&[id1, id2]).unwrap();
        assert_eq!(paths.len(), 2);

        let empty = db.get_media_file_paths(&[]).unwrap();
        assert!(empty.is_empty());
        cleanup(db, db_path);
    }

    #[test]
    fn get_parent_series_ids_returns_unique_parents() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/s", None)
            .unwrap();
        let ep1 = db
            .insert_episode("E1", "/tv/s/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();
        let ep2 = db
            .insert_episode("E2", "/tv/s/s01e02.mkv", show_id, 1, 2, 0.0)
            .unwrap();

        let parents = db.get_parent_series_ids(&[ep1, ep2]).unwrap();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0], show_id);
        cleanup(db, db_path);
    }

    #[test]
    fn series_has_episodes_returns_correctly() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/s", None)
            .unwrap();

        assert!(!db.series_has_episodes(show_id).unwrap());

        db.insert_episode("E1", "/tv/s/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();

        assert!(db.series_has_episodes(show_id).unwrap());
        cleanup(db, db_path);
    }

    #[test]
    fn delete_media_entries_removes_multiple() {
        let (db, db_path) = make_db();
        let id1 = db
            .insert_movie(
                "A",
                None,
                None,
                None,
                None,
                Some("/poster_a.jpg"),
                "/a.mkv",
                0.0,
                None,
            )
            .unwrap();
        let id2 = db
            .insert_movie(
                "B",
                None,
                None,
                None,
                None,
                Some("/poster_b.jpg"),
                "/b.mkv",
                0.0,
                None,
            )
            .unwrap();

        let posters = db.delete_media_entries(&[id1, id2]).unwrap();
        assert_eq!(posters.len(), 2);

        assert!(db.get_media_by_id(id1).is_err());
        assert!(db.get_media_by_id(id2).is_err());
        cleanup(db, db_path);
    }

    // ==================== CLEAR ALL DATA ====================

    #[test]
    fn clear_all_data_removes_everything() {
        let (db, db_path) = make_db();
        db.insert_movie("M", None, None, None, None, None, "/m.mkv", 0.0, None)
            .unwrap();
        db.set_setting("key", "val").unwrap();

        let result = db.clear_all_data().unwrap();
        assert!(!result.is_empty());

        let stats = db.get_library_stats(None).unwrap();
        assert_eq!(stats.movies, 0);
        assert!(db.get_setting("key").unwrap().is_none());
        cleanup(db, db_path);
    }

    // ==================== CLOUD MEDIA QUERIES ====================

    #[test]
    fn cloud_file_exists_returns_correctly() {
        let (db, db_path) = make_db();
        assert!(!db.cloud_file_exists("cf-1"));

        db.insert_cloud_movie(
            "Cloud M",
            None,
            None,
            None,
            None,
            None,
            "cloud.mkv",
            "cf-1",
            "folder-1",
            0.0,
            None,
        )
        .unwrap();

        assert!(db.cloud_file_exists("cf-1"));
        assert!(!db.cloud_file_exists("cf-99"));
        cleanup(db, db_path);
    }

    #[test]
    fn get_cloud_file_ids_for_folder_returns_ids() {
        let (db, db_path) = make_db();
        db.insert_cloud_movie(
            "A", None, None, None, None, None, "a.mkv", "cf-a", "folder-1", 0.0, None,
        )
        .unwrap();
        db.insert_cloud_movie(
            "B", None, None, None, None, None, "b.mkv", "cf-b", "folder-1", 0.0, None,
        )
        .unwrap();

        let ids = db.get_cloud_file_ids_for_folder("folder-1").unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"cf-a".to_string()));
        assert!(ids.contains(&"cf-b".to_string()));
        cleanup(db, db_path);
    }

    // ==================== CACHED EPISODE METADATA ====================

    #[test]
    fn cached_episode_metadata_roundtrip() {
        let (db, db_path) = make_db();

        db.save_cached_episode_metadata(
            "tmdb-show-1",
            1,
            1,
            Some("Pilot Episode"),
            Some("The first episode"),
            Some("/still/pilot.jpg"),
            Some("2024-01-01"),
            Some(8.5),
        )
        .unwrap();

        let meta = db.get_cached_episode_metadata("tmdb-show-1", 1, 1).unwrap();
        assert!(meta.is_some());
        let m = meta.unwrap();
        assert_eq!(m.episode_title.as_deref(), Some("Pilot Episode"));
        assert_eq!(m.overview.as_deref(), Some("The first episode"));
        assert_eq!(m.still_path.as_deref(), Some("/still/pilot.jpg"));
        assert_eq!(m.vote_average, Some(8.5));

        assert!(db.has_cached_metadata_for_series("tmdb-show-1").unwrap());
        assert!(!db.has_cached_metadata_for_series("tmdb-show-99").unwrap());

        let cleared = db.clear_cached_metadata_for_series("tmdb-show-1").unwrap();
        assert_eq!(cleared, 1);
        assert!(!db.has_cached_metadata_for_series("tmdb-show-1").unwrap());
        cleanup(db, db_path);
    }

    // ==================== EPISODE METADATA UPDATE ====================

    #[test]
    fn update_episode_metadata_changes_fields() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/show", None)
            .unwrap();
        let ep_id = db
            .insert_episode("Ep", "/tv/show/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();

        db.update_episode_metadata(
            ep_id,
            Some("Ep Title"),
            Some("Overview"),
            Some("/still.jpg"),
        )
        .unwrap();

        let item = db.get_media_by_id(ep_id).unwrap();
        assert_eq!(item.episode_title.as_deref(), Some("Ep Title"));
        assert_eq!(item.overview.as_deref(), Some("Overview"));
        assert_eq!(item.still_path.as_deref(), Some("/still.jpg"));
        cleanup(db, db_path);
    }

    // ==================== GDRIVE TOKEN ====================

    #[test]
    fn gdrive_changes_token_roundtrip() {
        let (db, db_path) = make_db();

        assert!(db.get_gdrive_changes_token().unwrap().is_none());

        db.set_gdrive_changes_token("token-abc").unwrap();
        let token = db.get_gdrive_changes_token().unwrap();
        assert_eq!(token.as_deref(), Some("token-abc"));

        cleanup(db, db_path);
    }

    // ==================== DDL MEDIA ====================

    #[test]
    fn insert_ddl_episode_and_query() {
        let (db, db_path) = make_db();

        db.upsert_ddl_source(
            "ddl-test",
            "https://example.com/file.zip",
            "file.zip",
            1024,
            "zip",
            2,
            1,
            0,
            0,
            None,
        )
        .unwrap();

        let ep_id = db
            .insert_ddl_episode(
                "DDL Ep",
                None,
                Some(1),
                Some(1),
                "ddl-test",
                "zip",
                "ep1.mkv",
                0,
                0,
                0,
                0,
                "",
                0,
                None,
                None,
                None,
            )
            .unwrap();

        let ddl_media = db.get_ddl_media("movie", None).unwrap();
        assert!(!ddl_media.is_empty());

        let item = db.get_media_by_id(ep_id).unwrap();
        assert_eq!(item.ddl_source_id.as_deref(), Some("ddl-test"));
        cleanup(db, db_path);
    }

    // ==================== STREAMING HISTORY ====================

    #[test]
    fn streaming_history_save_and_get() {
        let (db, db_path) = make_db();

        db.save_streaming_progress(
            "tmdb-stream-1",
            "movie",
            "Stream Movie",
            Some("/poster.jpg"),
            None,
            None,
            1800.0,
            3600.0,
        )
        .unwrap();

        let history = db.get_streaming_history(10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].title, "Stream Movie");
        assert_eq!(history[0].tmdb_id, "tmdb-stream-1");
        cleanup(db, db_path);
    }

    #[test]
    fn clear_all_streaming_history_removes_entries() {
        let (db, db_path) = make_db();

        db.save_streaming_progress("tmdb-1", "movie", "A", None, None, None, 900.0, 1800.0)
            .unwrap();
        db.save_streaming_progress("tmdb-2", "movie", "B", None, None, None, 900.0, 1800.0)
            .unwrap();

        let cleared = db.clear_all_streaming_history().unwrap();
        assert_eq!(cleared, 2);
        assert!(db.get_streaming_history(10).unwrap().is_empty());
        cleanup(db, db_path);
    }

    #[test]
    fn remove_from_streaming_history_removes_one() {
        let (db, db_path) = make_db();

        db.save_streaming_progress("tmdb-1", "movie", "A", None, None, None, 900.0, 1800.0)
            .unwrap();
        db.save_streaming_progress("tmdb-2", "movie", "B", None, None, None, 900.0, 1800.0)
            .unwrap();

        let history = db.get_streaming_history(10).unwrap();
        assert_eq!(history.len(), 2);

        db.remove_from_streaming_history(history[0].id).unwrap();
        assert_eq!(db.get_streaming_history(10).unwrap().len(), 1);
        cleanup(db, db_path);
    }

    // ==================== UNIQUE CONSTRAINTS ====================

    #[test]
    fn insert_movie_duplicate_file_path_errors() {
        let (db, db_path) = make_db();
        db.insert_movie("A", None, None, None, None, None, "/same.mkv", 0.0, None)
            .unwrap();
        // Second insert with same file_path should fail
        assert!(db
            .insert_movie("B", None, None, None, None, None, "/same.mkv", 0.0, None)
            .is_err());
        cleanup(db, db_path);
    }

    // ==================== CASCADE DELETE ====================

    #[test]
    fn delete_parent_cascades_to_episodes() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/show", None)
            .unwrap();
        let ep_id = db
            .insert_episode("Ep", "/tv/show/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();

        // Delete parent
        db.remove_media(show_id).unwrap();

        // Episode should be gone via cascade
        assert!(db.get_media_by_id(ep_id).is_err());
        cleanup(db, db_path);
    }

    // ==================== GET ALL MEDIA ====================

    #[test]
    fn get_all_media_returns_everything() {
        let (db, db_path) = make_db();
        db.insert_movie("M", None, None, None, None, None, "/m.mkv", 0.0, None)
            .unwrap();
        db.insert_tvshow("S", None, None, None, None, "/tv/s", None)
            .unwrap();
        db.insert_episode("E", "/tv/s/s01e01.mkv", 2, 1, 1, 0.0)
            .unwrap();

        let all = db.get_all_media().unwrap();
        assert_eq!(all.len(), 3);
        cleanup(db, db_path);
    }

    // ==================== EPISODE STILL PATH UPDATE ====================

    #[test]
    fn update_episode_still_path_updates_correctly() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/show", None)
            .unwrap();
        let ep_id = db
            .insert_episode("Ep", "/tv/show/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();

        db.update_episode_still_path(show_id, 1, 1, "/new_still.jpg")
            .unwrap();

        let item = db.get_media_by_id(ep_id).unwrap();
        assert_eq!(item.still_path.as_deref(), Some("/new_still.jpg"));
        cleanup(db, db_path);
    }

    // ==================== FILE SIZE / DURATION UPDATE ====================

    #[test]
    fn update_file_size_and_duration() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie("M", None, None, None, None, None, "/m.mkv", 0.0, None)
            .unwrap();

        db.update_file_size(id, 1024000).unwrap();
        db.update_duration(id, 7200.0).unwrap();

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.file_size_bytes, Some(1024000));
        assert_eq!(item.duration_seconds, Some(7200.0));
        cleanup(db, db_path);
    }

    // ==================== FIND BY TMDB ====================

    #[test]
    fn find_media_by_tmdb_returns_correct_media() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "M",
                None,
                None,
                None,
                None,
                None,
                "/m.mkv",
                0.0,
                Some("tmdb-123"),
            )
            .unwrap();

        let found = db.find_media_by_tmdb("tmdb-123", "movie").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, id);

        let not_found = db.find_media_by_tmdb("tmdb-999", "movie").unwrap();
        assert!(not_found.is_none());
        cleanup(db, db_path);
    }

    #[test]
    fn find_series_id_by_tmdb_returns_show_id() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/s", Some("tmdb-show-1"))
            .unwrap();

        let found = db.find_series_id_by_tmdb("tmdb-show-1").unwrap();
        assert_eq!(found, Some(show_id));

        let not_found = db.find_series_id_by_tmdb("tmdb-none").unwrap();
        assert!(not_found.is_none());
        cleanup(db, db_path);
    }

    // ==================== CLOUD INDEX FAILURES ====================

    #[test]
    fn cloud_index_failure_roundtrip() {
        let (db, db_path) = make_db();

        db.upsert_cloud_index_failure("cf-1", "bad.mkv", "corrupt file")
            .unwrap();

        let failures = db.get_cloud_index_failures(10).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].cloud_file_id, "cf-1");
        assert_eq!(failures[0].last_error, "corrupt file");

        db.clear_cloud_index_failure("cf-1").unwrap();
        assert!(db.get_cloud_index_failures(10).unwrap().is_empty());
        cleanup(db, db_path);
    }

    // ==================== POSTER PATHS ====================

    #[test]
    fn get_all_poster_paths_returns_paths() {
        let (db, db_path) = make_db();
        db.insert_movie(
            "A",
            None,
            None,
            None,
            None,
            Some("/a.jpg"),
            "/a.mkv",
            0.0,
            None,
        )
        .unwrap();
        db.insert_movie("B", None, None, None, None, None, "/b.mkv", 0.0, None)
            .unwrap();

        let paths = db.get_all_poster_paths().unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], "/a.jpg");
        cleanup(db, db_path);
    }

    // ==================== BROKEN FILE PATHS ====================

    #[test]
    fn get_broken_file_paths_empty_on_valid_db() {
        let (db, db_path) = make_db();
        // Paths with directory separators are NOT considered "broken"
        db.insert_movie(
            "M",
            None,
            None,
            None,
            None,
            None,
            "/nonexistent/path.mkv",
            0.0,
            None,
        )
        .unwrap();

        let broken = db.get_broken_file_paths().unwrap();
        // get_broken_file_paths finds bare filenames (no / or \) — paths with separators are fine
        assert!(broken.is_empty());
        cleanup(db, db_path);
    }

    // ==================== REMOVE MEDIA BY CLOUD FILE ID ====================

    #[test]
    fn remove_media_by_cloud_file_id_removes_and_returns_info() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_cloud_tvshow("Show", None, None, None, None, "folder", "fid-folder", None)
            .unwrap();
        let ep_id = db
            .insert_cloud_episode(
                "Ep",
                "ep.mkv",
                show_id,
                1,
                1,
                "cf-ep-1",
                "fid-folder",
                Some("Ep Title"),
                None,
                None,
                None,
            )
            .unwrap();

        let result = db.remove_media_by_cloud_file_id("cf-ep-1").unwrap();
        assert!(result.is_some());
        // Episode should be gone
        assert!(db.get_media_by_id(ep_id).is_err());
        cleanup(db, db_path);
    }

    // ==================== SETTING ROUNDTRIP ====================

    #[test]
    fn setting_set_and_get_roundtrip() {
        let (db, db_path) = make_db();

        assert!(db.get_setting("my_key").unwrap().is_none());

        db.set_setting("my_key", "my_value").unwrap();
        assert_eq!(
            db.get_setting("my_key").unwrap().as_deref(),
            Some("my_value")
        );

        // Overwrite
        db.set_setting("my_key", "updated").unwrap();
        assert_eq!(
            db.get_setting("my_key").unwrap().as_deref(),
            Some("updated")
        );

        cleanup(db, db_path);
    }

    // ==================== MOVIE REMINDERS: NOTIFY / ADVANCE ====================

    #[test]
    fn mark_movie_reminder_notified_sets_fields() {
        let (db, db_path) = make_db();
        let rem = db
            .create_movie_reminder(NewMovieReminder {
                tmdb_id: "tmdb-n",
                media_type: "movie",
                title: "N",
                poster_path: None,
                season_number: None,
                episode_number: None,
                release_date: None,
                reminder_at: "2020-01-01T00:00:00Z",
                source: "tmdb",
                tracking_mode: "single",
                tracking_season_number: None,
                notes: None,
                is_active: true,
            })
            .unwrap();

        db.mark_movie_reminder_notified(rem.id, "2020-01-02T00:00:00Z")
            .unwrap();

        let fetched = db.get_movie_reminder(rem.id).unwrap();
        assert!(!fetched.is_active);
        assert_eq!(fetched.notified_at.as_deref(), Some("2020-01-02T00:00:00Z"));
        cleanup(db, db_path);
    }

    #[test]
    fn advance_movie_reminder_updates_fields_and_reactivates() {
        let (db, db_path) = make_db();
        let rem = db
            .create_movie_reminder(NewMovieReminder {
                tmdb_id: "tmdb-adv",
                media_type: "tv",
                title: "Old Title",
                poster_path: None,
                season_number: Some(1),
                episode_number: Some(1),
                release_date: Some("2025-01-01"),
                reminder_at: "2025-01-01T00:00:00Z",
                source: "tmdb",
                tracking_mode: "season",
                tracking_season_number: Some(1),
                notes: None,
                is_active: true,
            })
            .unwrap();

        db.mark_movie_reminder_notified(rem.id, "2025-01-02T00:00:00Z")
            .unwrap();
        assert!(!db.get_movie_reminder(rem.id).unwrap().is_active);

        db.advance_movie_reminder(
            rem.id,
            "New Ep",
            Some("/p.jpg"),
            Some(1),
            Some(2),
            Some("2025-02-01"),
            "2025-02-01T00:00:00Z",
            "tmdb",
            Some(1),
            "2025-01-15T00:00:00Z",
        )
        .unwrap();

        let p = db.get_movie_reminder(rem.id).unwrap();
        assert!(p.is_active);
        assert_eq!(p.title, "New Ep");
        assert_eq!(p.episode_number, Some(2));
        assert_eq!(p.poster_path.as_deref(), Some("/p.jpg"));
        cleanup(db, db_path);
    }

    // ==================== WATCHLIST: DUE NOTIFICATIONS / REPLACE ====================

    #[test]
    fn get_due_watchlist_notifications_returns_due_items() {
        let (db, db_path) = make_db();

        db.create_or_update_watchlist_item(NewWatchlistItem {
            tmdb_id: "tmdb-due",
            media_type: "movie",
            title: "Due",
            poster_path: None,
            release_date: None,
            notes: None,
            is_active: true,
            notification_enabled: true,
            notification_mode: "release",
            notification_interval_minutes: None,
            notify_at: Some("2020-01-01T00:00:00Z"),
        })
        .unwrap();

        db.create_or_update_watchlist_item(NewWatchlistItem {
            tmdb_id: "tmdb-future",
            media_type: "movie",
            title: "Future",
            poster_path: None,
            release_date: None,
            notes: None,
            is_active: true,
            notification_enabled: true,
            notification_mode: "release",
            notification_interval_minutes: None,
            notify_at: Some("2099-01-01T00:00:00Z"),
        })
        .unwrap();

        // Inactive notification
        db.create_or_update_watchlist_item(NewWatchlistItem {
            tmdb_id: "tmdb-off",
            media_type: "movie",
            title: "Off",
            poster_path: None,
            release_date: None,
            notes: None,
            is_active: true,
            notification_enabled: false,
            notification_mode: "off",
            notification_interval_minutes: None,
            notify_at: Some("2020-01-01T00:00:00Z"),
        })
        .unwrap();

        let due = db
            .get_due_watchlist_notifications("2025-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].tmdb_id, "tmdb-due");
        cleanup(db, db_path);
    }

    #[test]
    fn replace_watchlist_items_replaces_all() {
        let (db, db_path) = make_db();

        db.create_or_update_watchlist_item(NewWatchlistItem {
            tmdb_id: "tmdb-old",
            media_type: "movie",
            title: "Old",
            poster_path: None,
            release_date: None,
            notes: None,
            is_active: true,
            notification_enabled: false,
            notification_mode: "off",
            notification_interval_minutes: None,
            notify_at: None,
        })
        .unwrap();
        assert_eq!(db.get_watchlist_items(true).unwrap().len(), 1);

        let items = vec![WatchlistItem {
            id: 100,
            tmdb_id: "tmdb-new".into(),
            media_type: "movie".into(),
            title: "New".into(),
            poster_path: None,
            release_date: None,
            notes: None,
            is_active: true,
            notification_enabled: false,
            notification_mode: "off".into(),
            notification_interval_minutes: None,
            notify_at: None,
            last_notified_at: None,
            created_at: "2025-01-01".into(),
            updated_at: "2025-01-01".into(),
        }];
        db.replace_watchlist_items(&items).unwrap();

        let all = db.get_watchlist_items(true).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].tmdb_id, "tmdb-new");
        cleanup(db, db_path);
    }

    // ==================== DDL SOURCE: URL / EXPIRED ====================

    #[test]
    fn get_ddl_source_url_returns_url() {
        let (db, db_path) = make_db();
        db.upsert_ddl_source(
            "ddl-u",
            "https://host/file.zip",
            "file.zip",
            100,
            "zip",
            1,
            1,
            0,
            0,
            None,
        )
        .unwrap();
        assert_eq!(
            db.get_ddl_source_url("ddl-u").unwrap(),
            "https://host/file.zip"
        );
        cleanup(db, db_path);
    }

    #[test]
    fn mark_ddl_source_expired_sets_flag() {
        let (db, db_path) = make_db();
        db.upsert_ddl_source(
            "ddl-e",
            "https://host/file.zip",
            "file.zip",
            100,
            "zip",
            1,
            1,
            0,
            0,
            None,
        )
        .unwrap();
        assert!(!db.get_ddl_source("ddl-e").unwrap().is_expired);

        db.mark_ddl_source_expired("ddl-e").unwrap();
        assert!(db.get_ddl_source("ddl-e").unwrap().is_expired);
        cleanup(db, db_path);
    }

    // ==================== ZIP ARCHIVE CRUD ====================

    #[test]
    fn zip_archive_insert_get_delete() {
        let (db, db_path) = make_db();
        let archive = crate::zip_manager::ZipArchiveInfo {
            zip_file_id: "zf-1".to_string(),
            filename: "test.zip".to_string(),
            archive_format: "zip".to_string(),
            file_size_bytes: 5000,
            compression_type: crate::zip_parser::ZipCompressionType::Store,
            central_dir_offset: 4000,
            central_dir_size: 500,
            total_entries: 3,
            video_entries: 2,
        };
        db.insert_zip_archive(&archive).unwrap();

        let rec = db.get_zip_archive("zf-1").unwrap();
        assert_eq!(rec.zip_file_id, "zf-1");
        assert_eq!(rec.filename, "test.zip");
        assert_eq!(rec.file_size_bytes, 5000);
        assert_eq!(rec.total_entries, 3);
        assert_eq!(rec.video_entries, 2);

        let deleted = db.delete_zip_archive("zf-1").unwrap();
        assert_eq!(deleted, 1);
        assert!(db.get_zip_archive("zf-1").is_err());
        cleanup(db, db_path);
    }

    // ==================== CACHED EPISODE METADATA ====================

    #[test]
    fn cached_episode_metadata_roundtrip_extended() {
        let (db, db_path) = make_db();

        db.save_cached_episode_metadata(
            "tmdb-ser",
            1,
            1,
            Some("Pilot"),
            Some("First ep"),
            Some("/still1.jpg"),
            Some("2024-01-01"),
            Some(8.5),
        )
        .unwrap();
        db.save_cached_episode_metadata(
            "tmdb-ser",
            1,
            2,
            Some("Ep2"),
            Some("Second ep"),
            Some("/still2.jpg"),
            Some("2024-01-08"),
            Some(7.0),
        )
        .unwrap();
        db.save_cached_episode_metadata(
            "tmdb-ser",
            2,
            1,
            Some("S2E1"),
            Some("Season 2 opener"),
            Some("/s2e1.jpg"),
            Some("2025-01-01"),
            Some(9.0),
        )
        .unwrap();

        // Single episode
        let ep = db
            .get_cached_episode_metadata("tmdb-ser", 1, 1)
            .unwrap()
            .unwrap();
        assert_eq!(ep.episode_title.as_deref(), Some("Pilot"));
        assert_eq!(ep.vote_average, Some(8.5));

        // has_cached
        assert!(db.has_cached_metadata_for_series("tmdb-ser").unwrap());
        assert!(!db.has_cached_metadata_for_series("tmdb-none").unwrap());

        // get_all_cached_episodes_for_series
        let all = db.get_all_cached_episodes_for_series("tmdb-ser").unwrap();
        assert_eq!(all.len(), 3);

        // get_cached_episodes_for_season
        let s1 = db.get_cached_episodes_for_season("tmdb-ser", 1).unwrap();
        assert_eq!(s1.len(), 2);
        assert_eq!(s1[0].episode_number, 1);
        assert_eq!(s1[0].season_number, 1);

        // clear
        let cleared = db.clear_cached_metadata_for_series("tmdb-ser").unwrap();
        assert_eq!(cleared, 3);
        assert!(!db.has_cached_metadata_for_series("tmdb-ser").unwrap());
        cleanup(db, db_path);
    }

    // ==================== REMOVE MEDIA / SERIES EPISODES ====================

    #[test]
    fn remove_media_returns_poster_and_deletes() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "M",
                None,
                None,
                None,
                None,
                Some("/poster.jpg"),
                "/m.mkv",
                0.0,
                None,
            )
            .unwrap();

        let poster = db.remove_media(id).unwrap();
        assert_eq!(poster.as_deref(), Some("/poster.jpg"));
        assert!(db.get_media_by_id(id).is_err());
        cleanup(db, db_path);
    }

    #[test]
    fn remove_series_episodes_deletes_children() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/s", None)
            .unwrap();
        let ep1 = db
            .insert_episode("E1", "/tv/s/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();
        let ep2 = db
            .insert_episode("E2", "/tv/s/s01e02.mkv", show_id, 1, 2, 0.0)
            .unwrap();

        db.remove_series_episodes(show_id).unwrap();
        assert!(db.get_media_by_id(ep1).is_err());
        assert!(db.get_media_by_id(ep2).is_err());
        // Show itself still exists
        assert!(db.get_media_by_id(show_id).is_ok());
        cleanup(db, db_path);
    }

    // ==================== MEDIA DELETE INFO / PARENT SERIES / DELETE ENTRIES ====================

    #[test]
    fn get_media_delete_info_returns_info() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie("M", None, None, None, None, None, "/m.mkv", 0.0, None)
            .unwrap();

        let info = db.get_media_delete_info(&[id]).unwrap();
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].0, id);
        assert_eq!(info[0].1.as_deref(), Some("/m.mkv"));
        assert!(!info[0].2); // not cloud

        // Empty input
        assert!(db.get_media_delete_info(&[]).unwrap().is_empty());
        cleanup(db, db_path);
    }

    #[test]
    fn get_parent_series_ids_returns_distinct_ids() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/s", None)
            .unwrap();
        let ep1 = db
            .insert_episode("E1", "/tv/s/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();
        let ep2 = db
            .insert_episode("E2", "/tv/s/s01e02.mkv", show_id, 1, 2, 0.0)
            .unwrap();

        let parents = db.get_parent_series_ids(&[ep1, ep2]).unwrap();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0], show_id);

        assert!(db.get_parent_series_ids(&[]).unwrap().is_empty());
        cleanup(db, db_path);
    }

    #[test]
    fn delete_media_entries_removes_and_returns_paths() {
        let (db, db_path) = make_db();
        let id1 = db
            .insert_movie("A", None, None, None, None, None, "/a.mkv", 0.0, None)
            .unwrap();
        let id2 = db
            .insert_movie("B", None, None, None, None, None, "/b.mkv", 0.0, None)
            .unwrap();

        let paths = db.delete_media_entries(&[id1, id2]).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"/a.mkv".to_string()));
        assert!(paths.contains(&"/b.mkv".to_string()));
        assert!(db.get_media_by_id(id1).is_err());
        assert!(db.get_media_by_id(id2).is_err());

        assert!(db.delete_media_entries(&[]).unwrap().is_empty());
        cleanup(db, db_path);
    }

    #[test]
    fn series_has_episodes_correct() {
        let (db, db_path) = make_db();
        let show_id = db
            .insert_tvshow("Show", None, None, None, None, "/tv/s", None)
            .unwrap();
        assert!(!db.series_has_episodes(show_id).unwrap());

        db.insert_episode("E1", "/tv/s/s01e01.mkv", show_id, 1, 1, 0.0)
            .unwrap();
        assert!(db.series_has_episodes(show_id).unwrap());
        cleanup(db, db_path);
    }

    // ==================== MERGE DUPLICATE TV SHOWS ====================

    #[test]
    fn merge_duplicate_tvshows_merges_by_tmdb_id() {
        let (db, db_path) = make_db();
        let s1 = db
            .insert_tvshow(
                "Show",
                Some(2024),
                Some("Overview"),
                None,
                Some("/p.jpg"),
                "/tv/s1",
                Some("tmdb-merge"),
            )
            .unwrap();
        let s2 = db
            .insert_tvshow(
                "Show Copy",
                None,
                None,
                None,
                None,
                "/tv/s2",
                Some("tmdb-merge"),
            )
            .unwrap();
        let ep = db
            .insert_episode("E1", "/tv/s2/s01e01.mkv", s2, 1, 1, 0.0)
            .unwrap();

        let merged = db.merge_duplicate_tvshows().unwrap();
        assert!(merged >= 1);

        // Episodes moved to s1 (the one with poster+overview)
        let moved_ep = db.get_media_by_id(ep).unwrap();
        assert_eq!(moved_ep.parent_id, Some(s1));

        // s2 deleted
        assert!(db.get_media_by_id(s2).is_err());
        cleanup(db, db_path);
    }

    // ==================== CLEAR ALL DATA ====================

    #[test]
    fn clear_all_data_removes_everything_extended() {
        let (db, db_path) = make_db();
        db.insert_movie("M", None, None, None, None, None, "/m.mkv", 0.0, None)
            .unwrap();
        db.set_setting("key", "val").unwrap();
        db.add_cloud_folder("fid", "fname").unwrap();
        db.upsert_ddl_source("ddl", "url", "f.zip", 1, "zip", 1, 1, 0, 0, None)
            .unwrap();
        db.create_movie_reminder(NewMovieReminder {
            tmdb_id: "tmdb",
            media_type: "movie",
            title: "R",
            poster_path: None,
            season_number: None,
            episode_number: None,
            release_date: None,
            reminder_at: "2025-01-01",
            source: "tmdb",
            tracking_mode: "single",
            tracking_season_number: None,
            notes: None,
            is_active: true,
        })
        .unwrap();

        let cache_path = db.clear_all_data().unwrap();
        assert!(!cache_path.is_empty());

        assert!(db.get_all_media().unwrap().is_empty());
        assert!(db.get_setting("key").unwrap().is_none());
        assert!(db.get_cloud_folders().unwrap().is_empty());
        assert!(db.get_ddl_sources().unwrap().is_empty());
        assert!(db.get_movie_reminders(true).unwrap().is_empty());
        cleanup(db, db_path);
    }

    // ==================== UPDATE FILE PATH ====================

    #[test]
    fn update_file_path_changes_path() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie("M", None, None, None, None, None, "/old.mkv", 0.0, None)
            .unwrap();

        db.update_file_path(id, "/new/path.mkv").unwrap();

        let item = db.get_media_by_id(id).unwrap();
        assert_eq!(item.file_path.as_deref(), Some("/new/path.mkv"));
        cleanup(db, db_path);
    }

    // ==================== WATCH STATS ====================

    #[test]
    fn get_watch_stats_empty_db_returns_zeros() {
        let (db, db_path) = make_db();
        let stats = db.get_watch_stats().unwrap();
        assert_eq!(stats.movies_watched, 0);
        assert_eq!(stats.episodes_watched, 0);
        assert_eq!(stats.total_watch_time_seconds, 0.0);
        cleanup(db, db_path);
    }

    #[test]
    fn get_watch_stats_counts_completed_media_extended() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie("M", None, None, None, None, None, "/m.mkv", 3600.0, None)
            .unwrap();

        // Mark as watched (>93% progress clears resume_position to 0)
        db.update_progress(id, 3500.0, 3600.0).unwrap();

        let stats = db.get_watch_stats().unwrap();
        assert_eq!(stats.movies_watched, 1);
        assert!(stats.total_watch_time_seconds > 0.0);
        cleanup(db, db_path);
    }

    // ==================== RECENT WATCH ACTIVITIES ====================

    #[test]
    fn get_recent_watch_activities_returns_completed_items() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie(
                "M",
                None,
                None,
                None,
                None,
                Some("/p.jpg"),
                "/m.mkv",
                7200.0,
                Some("tmdb-act"),
            )
            .unwrap();

        // Mark as watched
        db.update_progress(id, 7000.0, 7200.0).unwrap();

        let activities = db
            .get_recent_watch_activities("2020-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].content_type, "movie");
        assert_eq!(activities[0].activity_type, "watched_movie");
        cleanup(db, db_path);
    }

    // ==================== ANALYTICS DATA ====================

    #[test]
    fn get_analytics_data_returns_struct() {
        let (db, db_path) = make_db();
        let id = db
            .insert_movie("M", None, None, None, None, None, "/m.mkv", 3600.0, None)
            .unwrap();
        db.update_progress(id, 3500.0, 3600.0).unwrap();

        let data = db.get_analytics_data().unwrap();
        // Just verify it doesn't error and returns reasonable structure
        assert!(data.overview.movies_completed >= 0);
        cleanup(db, db_path);
    }

    // ==================== REMOTE BOOKMARKS ====================

    #[test]
    fn remote_bookmark_add_get_remove() {
        let (db, db_path) = make_db();

        db.add_remote_bookmark(
            "12345",
            "movie",
            "Test Movie",
            Some(2024),
            Some("/poster.jpg"),
        )
        .unwrap();
        db.add_remote_bookmark("67890", "tv", "Test Show", Some(2023), None)
            .unwrap();

        let bookmarks = db.get_remote_bookmarks().unwrap();
        assert_eq!(bookmarks.len(), 2);

        assert!(db.is_remote_bookmarked("12345", "movie").unwrap());
        assert!(db.is_remote_bookmarked("67890", "tv").unwrap());
        assert!(!db.is_remote_bookmarked("99999", "movie").unwrap());

        db.remove_remote_bookmark("12345", "movie").unwrap();
        let bookmarks = db.get_remote_bookmarks().unwrap();
        assert_eq!(bookmarks.len(), 1);
        assert!(!db.is_remote_bookmarked("12345", "movie").unwrap());

        cleanup(db, db_path);
    }

    #[test]
    fn remote_bookmark_upsert_replaces() {
        let (db, db_path) = make_db();

        db.add_remote_bookmark("111", "movie", "Old Title", None, None)
            .unwrap();
        db.add_remote_bookmark("111", "movie", "New Title", Some(2025), Some("/new.jpg"))
            .unwrap();

        let bookmarks = db.get_remote_bookmarks().unwrap();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].title, "New Title");
        assert_eq!(bookmarks[0].year, Some(2025));

        cleanup(db, db_path);
    }

    // ==================== REMOTE PLAYBACK PROGRESS ====================

    #[test]
    fn remote_progress_upsert_and_get() {
        let (db, db_path) = make_db();

        db.upsert_playback_progress("123", "tv", Some(1), Some(5), 300.0, 2400.0)
            .unwrap();
        db.upsert_playback_progress("123", "tv", Some(1), Some(6), 0.0, 2400.0)
            .unwrap();

        let ep5 = db
            .get_progress_for_item("123", "tv", Some(1), Some(5))
            .unwrap()
            .unwrap();
        assert_eq!(ep5.resume_position_seconds, 300.0);
        assert_eq!(ep5.duration_seconds, 2400.0);

        let all = db.get_all_progress_for_show("123").unwrap();
        assert_eq!(all.len(), 2);

        let everything = db.get_all_playback_progress().unwrap();
        assert_eq!(everything.len(), 2);

        cleanup(db, db_path);
    }

    #[test]
    fn remote_progress_clear() {
        let (db, db_path) = make_db();

        db.upsert_playback_progress("999", "movie", None, None, 500.0, 7200.0)
            .unwrap();
        assert!(db
            .get_progress_for_item("999", "movie", None, None)
            .unwrap()
            .is_some());

        db.clear_playback_progress("999", "movie", None, None)
            .unwrap();
        assert!(db
            .get_progress_for_item("999", "movie", None, None)
            .unwrap()
            .is_none());

        cleanup(db, db_path);
    }

    // ==================== CLOUD FOLDERS ====================

    #[test]
    fn cloud_folder_add_get_remove() {
        let (db, db_path) = make_db();

        db.add_cloud_folder("cf-1", "My Folder").unwrap();
        db.add_cloud_folder("cf-2", "Other").unwrap();

        let folders = db.get_cloud_folders().unwrap();
        assert_eq!(folders.len(), 2);
        assert!(folders.iter().any(|(id, _, _)| id == "cf-1"));

        db.remove_cloud_folder("cf-1").unwrap();
        let folders = db.get_cloud_folders().unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].0, "cf-2");
        cleanup(db, db_path);
    }

    #[test]
    fn update_cloud_folder_scanned_sets_timestamp() {
        let (db, db_path) = make_db();
        db.add_cloud_folder("cf-s", "Folder").unwrap();
        // Should not error
        db.update_cloud_folder_scanned("cf-s").unwrap();
        cleanup(db, db_path);
    }

    // ==================== METADATA ENRICHMENT ====================

    #[test]
    fn get_media_needing_metadata_enrichment_finds_incomplete() {
        let (db, db_path) = make_db();
        // Movie with no tmdb_id
        db.insert_movie(
            "Incomplete",
            None,
            None,
            None,
            None,
            None,
            "/inc.mkv",
            0.0,
            None,
        )
        .unwrap();
        // Movie with all metadata
        db.insert_movie(
            "Complete",
            Some(2024),
            Some("Overview"),
            Some("Cast"),
            Some("Dir"),
            Some("/p.jpg"),
            "/comp.mkv",
            7200.0,
            Some("tmdb-full"),
        )
        .unwrap();

        let candidates = db.get_media_needing_metadata_enrichment(100).unwrap();
        assert!(candidates.iter().any(|c| c.title == "Incomplete"));
        // Complete movie may still show up (duration 0 or other reasons), but we assert the incomplete one is there
        cleanup(db, db_path);
    }

    // ==================== BROKEN FILE PATHS (with bare filename) ====================

    #[test]
    fn get_broken_file_paths_finds_bare_filenames() {
        let (db, db_path) = make_db();
        // Manually insert a media entry with a bare filename (no path separators)
        db.conn
            .execute(
                "INSERT INTO media (title, file_path, media_type) VALUES (?, ?, ?)",
                rusqlite::params!["Bare", "barefile.mkv", "movie"],
            )
            .unwrap();

        let broken = db.get_broken_file_paths().unwrap();
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].1, "barefile.mkv");
        cleanup(db, db_path);
    }

    // ==================== CLOUD INDEX FAILURES (additional) ====================

    #[test]
    fn cloud_index_failure_upsert_updates_existing() {
        let (db, db_path) = make_db();

        db.upsert_cloud_index_failure("cf-u", "file.mkv", "old error")
            .unwrap();
        db.upsert_cloud_index_failure("cf-u", "file.mkv", "new error")
            .unwrap();

        let failures = db.get_cloud_index_failures(10).unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].last_error, "new error");
        cleanup(db, db_path);
    }

    // ==================== ZIP ARCHIVE UPSERT ON CONFLICT ====================

    #[test]
    fn zip_archive_upsert_updates_on_conflict() {
        let (db, db_path) = make_db();
        let a1 = crate::zip_manager::ZipArchiveInfo {
            zip_file_id: "zf-up".to_string(),
            filename: "old.zip".to_string(),
            archive_format: "zip".to_string(),
            file_size_bytes: 1000,
            compression_type: crate::zip_parser::ZipCompressionType::Store,
            central_dir_offset: 0,
            central_dir_size: 0,
            total_entries: 1,
            video_entries: 1,
        };
        db.insert_zip_archive(&a1).unwrap();

        let a2 = crate::zip_manager::ZipArchiveInfo {
            zip_file_id: "zf-up".to_string(),
            filename: "new.zip".to_string(),
            archive_format: "zip".to_string(),
            file_size_bytes: 2000,
            compression_type: crate::zip_parser::ZipCompressionType::Deflate,
            central_dir_offset: 100,
            central_dir_size: 50,
            total_entries: 5,
            video_entries: 3,
        };
        db.insert_zip_archive(&a2).unwrap();

        let rec = db.get_zip_archive("zf-up").unwrap();
        assert_eq!(rec.filename, "new.zip");
        assert_eq!(rec.file_size_bytes, 2000);
        assert_eq!(rec.total_entries, 5);
        cleanup(db, db_path);
    }
}
