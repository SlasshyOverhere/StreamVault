use rusqlite::{Connection, Result, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const APP_NAME: &str = "StreamVault";

/// Get the app data directory, with separate paths for dev and production builds
/// Dev builds use "StreamVault-Dev" to keep data isolated from production
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
    get_app_data_dir().join("media_library.db").to_string_lossy().to_string()
}

pub fn get_image_cache_dir() -> String {
    get_app_data_dir().join("image_cache").to_string_lossy().to_string()
}

pub fn get_config_path() -> String {
    get_app_data_dir().join("media_config.json").to_string_lossy().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaItem {
    pub id: i64,
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
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
    pub episode_title: Option<String>,
    pub still_path: Option<String>,
    // Cloud storage fields
    pub is_cloud: Option<bool>,
    pub cloud_file_id: Option<String>,
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
}

/// Streaming history item for online content (Videasy, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingHistoryItem {
    pub id: i64,
    pub tmdb_id: String,
    pub media_type: String,  // "movie" or "tv"
    pub title: String,
    pub poster_path: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub resume_position_seconds: f64,
    pub duration_seconds: f64,
    pub progress_percent: f64,
    pub last_watched: String,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
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
        let columns: Vec<String> = self.conn
            .prepare("PRAGMA table_info(media)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        
        if !columns.contains(&"parent_id".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN parent_id INTEGER REFERENCES media(id) ON DELETE CASCADE", [])?;
        }
        if !columns.contains(&"season_number".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN season_number INTEGER", [])?;
        }
        if !columns.contains(&"episode_number".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN episode_number INTEGER", [])?;
        }
        if !columns.contains(&"duration_seconds".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN duration_seconds REAL DEFAULT 0", [])?;
        }
        if !columns.contains(&"resume_position_seconds".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN resume_position_seconds REAL DEFAULT 0", [])?;
        }
        if !columns.contains(&"last_watched".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN last_watched TIMESTAMP DEFAULT NULL", [])?;
        }
        if !columns.contains(&"tmdb_id".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN tmdb_id TEXT DEFAULT NULL", [])?;
        }
        if !columns.contains(&"episode_title".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN episode_title TEXT DEFAULT NULL", [])?;
        }
        if !columns.contains(&"still_path".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN still_path TEXT DEFAULT NULL", [])?;
        }

        // Cloud storage columns
        if !columns.contains(&"is_cloud".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN is_cloud INTEGER DEFAULT 0", [])?;
        }
        if !columns.contains(&"cloud_file_id".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN cloud_file_id TEXT DEFAULT NULL", [])?;
        }
        if !columns.contains(&"cloud_folder_id".to_string()) {
            self.conn.execute("ALTER TABLE media ADD COLUMN cloud_folder_id TEXT DEFAULT NULL", [])?;
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
        self.conn.execute(
            "ALTER TABLE cloud_folders ADD COLUMN changes_page_token TEXT",
            [],
        ).ok(); // Ignore error if column already exists

        // Create app_settings table for storing global settings like the changes token
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        Ok(())
    }

    pub fn get_library(&self, media_type: &str, search: Option<&str>) -> Result<Vec<MediaItem>> {
        let mut sql = String::from(
            "SELECT id, title, year, overview, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    is_cloud, cloud_file_id,
                    is_cloud, cloud_file_id
             FROM media WHERE media_type = ?"
        );

        if search.is_some() {
            sql.push_str(" AND title LIKE ?");
        }
        sql.push_str(" ORDER BY title");

        let mut stmt = self.conn.prepare(&sql)?;

        let items = if let Some(query) = search {
            stmt.query_map(params![media_type, format!("%{}%", query)], Self::map_media_item)?
        } else {
            stmt.query_map(params![media_type], Self::map_media_item)?
        };

        items.filter_map(|r| r.ok()).collect::<Vec<_>>().into_iter().map(Ok).collect()
    }

    /// Get library filtered by cloud status
    pub fn get_library_filtered(&self, media_type: &str, search: Option<&str>, is_cloud: Option<bool>) -> Result<Vec<MediaItem>> {
        let mut sql = String::from(
            "SELECT id, title, year, overview, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    is_cloud, cloud_file_id,
                    is_cloud, cloud_file_id
             FROM media WHERE media_type = ?"
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
        sql.push_str(" ORDER BY title");

        let mut stmt = self.conn.prepare(&sql)?;

        let items = if let Some(query) = search {
            stmt.query_map(params![media_type, format!("%{}%", query)], Self::map_media_item)?
        } else {
            stmt.query_map(params![media_type], Self::map_media_item)?
        };

        items.filter_map(|r| r.ok()).collect::<Vec<_>>().into_iter().map(Ok).collect()
    }

    pub fn get_episodes(&self, series_id: i64) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    is_cloud, cloud_file_id,
                    is_cloud, cloud_file_id
             FROM media WHERE parent_id = ? ORDER BY season_number, episode_number"
        )?;

        let items = stmt.query_map(params![series_id], Self::map_media_item)?;
        items.filter_map(|r| r.ok()).collect::<Vec<_>>().into_iter().map(Ok).collect()
    }
    
    pub fn get_watch_history(&self, limit: i32) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                m.id,
                CASE WHEN m.media_type = 'tvepisode' THEN p.title ELSE m.title END as title,
                CASE WHEN m.media_type = 'tvepisode' THEN p.year ELSE m.year END as year,
                m.overview,
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
                m.still_path
             FROM media m
             LEFT JOIN media p ON m.parent_id = p.id
             WHERE m.last_watched IS NOT NULL
               AND m.media_type IN ('movie', 'tvepisode')
             ORDER BY m.last_watched DESC
             LIMIT ?"
        )?;

        let items = stmt.query_map(params![limit], Self::map_media_item)?;
        items.filter_map(|r| r.ok()).collect::<Vec<_>>().into_iter().map(Ok).collect()
    }

    pub fn get_media_by_id(&self, id: i64) -> Result<MediaItem> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    is_cloud, cloud_file_id
             FROM media WHERE id = ?"
        )?;

        stmt.query_row(params![id], Self::map_media_item)
    }
    
    pub fn get_resume_info(&self, media_id: i64) -> Result<ResumeInfo> {
        let mut stmt = self.conn.prepare(
            "SELECT resume_position_seconds, duration_seconds FROM media WHERE id = ?"
        )?;
        
        let (position, duration): (f64, f64) = stmt.query_row(params![media_id], |row| {
            Ok((
                row.get::<_, Option<f64>>(0)?.unwrap_or(0.0),
                row.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
            ))
        })?;
        
        let progress_percent = if duration > 0.0 { (position / duration) * 100.0 } else { 0.0 };
        
        // Don't return progress if >= 95%
        if progress_percent >= 95.0 {
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
        // Clear progress if >= 95%
        let progress_percent = if duration > 0.0 { current_time / duration } else { 0.0 };
        
        if progress_percent >= 0.95 {
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
        Ok(())
    }
    
    /// Clear all watch history by resetting last_watched for all items
    pub fn clear_all_watch_history(&self) -> Result<i32> {
        let count = self.conn.execute(
            "UPDATE media SET last_watched = NULL, resume_position_seconds = 0 WHERE last_watched IS NOT NULL",
            [],
        )?;
        Ok(count as i32)
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
        let existing_id: Option<i64> = self.conn.query_row(
            "SELECT id FROM streaming_history
             WHERE tmdb_id = ? AND media_type = ?
             AND COALESCE(season, -1) = COALESCE(?, -1)
             AND COALESCE(episode, -1) = COALESCE(?, -1)",
            params![tmdb_id, media_type, season, episode],
            |row| row.get(0)
        ).ok();

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
             LIMIT ?"
        )?;
        
        let items = stmt.query_map(params![limit], |row| {
            let duration: f64 = row.get::<_, f64>(8).unwrap_or(0.0);
            let position: f64 = row.get::<_, f64>(7).unwrap_or(0.0);
            let progress_percent = if duration > 0.0 { (position / duration) * 100.0 } else { 0.0 };
            
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
        
        items.filter_map(|r| r.ok()).collect::<Vec<_>>().into_iter().map(Ok).collect()
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
                   (episode IS ? OR (episode IS NULL AND ? IS NULL))"
        )?;
        
        match stmt.query_row(params![tmdb_id, media_type, season, season, episode, episode], |row| {
            let duration: f64 = row.get::<_, f64>(8).unwrap_or(0.0);
            let position: f64 = row.get::<_, f64>(7).unwrap_or(0.0);
            let progress_percent = if duration > 0.0 { (position / duration) * 100.0 } else { 0.0 };
            
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
        }) {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
    
    /// Remove a single item from streaming history
    pub fn remove_from_streaming_history(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM streaming_history WHERE id = ?",
            params![id],
        )?;
        Ok(())
    }
    
    /// Clear all streaming history
    pub fn clear_all_streaming_history(&self) -> Result<i32> {
        let count = self.conn.execute(
            "DELETE FROM streaming_history",
            [],
        )?;
        Ok(count as i32)
    }
    
    pub fn update_metadata(&self, media_id: i64, metadata: &super::tmdb::TmdbMetadata) -> Result<()> {
        self.conn.execute(
            "UPDATE media SET title = ?, year = ?, overview = ?, poster_path = ?, tmdb_id = ? WHERE id = ?",
            params![metadata.title, metadata.year, metadata.overview, metadata.poster_path, metadata.tmdb_id, media_id],
        )?;
        Ok(())
    }
    
    pub fn media_exists(&self, file_path: &str) -> Result<bool> {
        let mut stmt = self.conn.prepare("SELECT id FROM media WHERE file_path = ?")?;
        let exists = stmt.exists(params![file_path])?;
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
                  OR file_path LIKE '%.m2ts')"
        )?;

        let paths = stmt.query_map([], |row| {
            row.get::<_, String>(0)
        })?
        .filter_map(|r| r.ok())
        .collect();

        Ok(paths)
    }

    /// Get media item by file path - used for file watcher to identify media for removal
    pub fn get_media_by_file_path(&self, file_path: &str) -> Result<Option<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    is_cloud, cloud_file_id
             FROM media WHERE file_path = ?"
        )?;

        match stmt.query_row(params![file_path], Self::map_media_item) {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Remove media by file path and return image paths for cleanup
    pub fn remove_media_by_file_path(&self, file_path: &str) -> Result<Option<(i64, String, Option<String>, Option<String>)>> {
        // First get the media info so we can return it for cleanup
        let media_info: Option<(i64, String, Option<String>, Option<String>)> = self.conn.query_row(
            "SELECT id, title, poster_path, still_path FROM media WHERE file_path = ?",
            params![file_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        ).ok();

        if let Some((id, _, _, _)) = &media_info {
            // Delete the entry
            self.conn.execute("DELETE FROM media WHERE id = ?", params![id])?;
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
             AND NOT EXISTS (SELECT 1 FROM media e WHERE e.parent_id = m.id)"
        )?;

        let empty_series: Vec<(i64, Option<String>)> = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?.filter_map(|r| r.ok()).collect();

        // Delete empty series
        for (id, _) in &empty_series {
            self.conn.execute("DELETE FROM media WHERE id = ?", params![id])?;
        }

        Ok(empty_series)
    }
    
    pub fn find_series_by_folder(&self, folder_path: &str) -> Result<Option<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM media WHERE file_path = ? AND media_type = 'tvshow'"
        )?;
        
        match stmt.query_row(params![folder_path], |row| row.get::<_, i64>(0)) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
    
    /// Find a TV show series by TMDB ID first, then by normalized title as fallback.
    /// This allows consolidating episodes from different directories under the same series.
    pub fn find_series_by_tmdb_or_title(&self, tmdb_id: Option<&str>, title: &str, year: Option<i32>) -> Result<Option<i64>> {
        // First, try to find by TMDB ID if available (most reliable match)
        if let Some(tid) = tmdb_id {
            if !tid.is_empty() {
                let mut stmt = self.conn.prepare(
                    "SELECT id FROM media WHERE tmdb_id = ? AND media_type = 'tvshow'"
                )?;

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
            if let Ok(id) = stmt2.query_row(params![&normalized_title, y], |row| row.get::<_, i64>(0)) {
                return Ok(Some(id));
            }

            // Try with year ±1 (common for releases spanning year boundary)
            let mut stmt3 = self.conn.prepare(
                "SELECT id FROM media WHERE LOWER(title) = LOWER(?) AND (year = ? OR year = ? OR year = ?) AND media_type = 'tvshow'"
            )?;
            if let Ok(id) = stmt3.query_row(params![title, y, y - 1, y + 1], |row| row.get::<_, i64>(0)) {
                return Ok(Some(id));
            }
        }

        // Try matching by just title (without year) - useful when year isn't in filename
        let mut stmt4 = self.conn.prepare(
            "SELECT id FROM media WHERE LOWER(title) = LOWER(?) AND media_type = 'tvshow'"
        )?;
        if let Ok(id) = stmt4.query_row(params![title], |row| row.get::<_, i64>(0)) {
            return Ok(Some(id));
        }

        // Try with normalized title without year
        let mut stmt5 = self.conn.prepare(
            "SELECT id FROM media WHERE LOWER(title) = LOWER(?) AND media_type = 'tvshow'"
        )?;
        if let Ok(id) = stmt5.query_row(params![&normalized_title], |row| row.get::<_, i64>(0)) {
            return Ok(Some(id));
        }

        // Final attempt: fuzzy match using LIKE with the first significant word
        let first_word = normalized_title.split_whitespace().next().unwrap_or(&normalized_title);
        if first_word.len() >= 3 {
            let mut stmt6 = self.conn.prepare(
                "SELECT id, title FROM media WHERE LOWER(title) LIKE ? AND media_type = 'tvshow'"
            )?;
            let pattern = format!("{}%", first_word.to_lowercase());

            let result: Result<(i64, String), _> = stmt6.query_row(params![pattern], |row| {
                Ok((row.get(0)?, row.get(1)?))
            });

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

        // Check if one contains the other
        if norm_a.contains(&norm_b) || norm_b.contains(&norm_a) {
            return true;
        }

        // Check word overlap
        let words_a: std::collections::HashSet<&str> = norm_a.split_whitespace().collect();
        let words_b: std::collections::HashSet<&str> = norm_b.split_whitespace().collect();

        let intersection = words_a.intersection(&words_b).count();
        let smaller = words_a.len().min(words_b.len());

        // If most words match, consider them similar
        smaller > 0 && intersection >= smaller.saturating_sub(1)
    }
    
    pub fn insert_movie(&self, title: &str, year: Option<i32>, overview: Option<&str>, 
                       poster_path: Option<&str>, file_path: &str, duration: f64, tmdb_id: Option<&str>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO media (title, year, overview, poster_path, file_path, media_type, duration_seconds, tmdb_id) 
             VALUES (?, ?, ?, ?, ?, 'movie', ?, ?)",
            params![title, year, overview, poster_path, file_path, duration, tmdb_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
    
    pub fn insert_tvshow(&self, title: &str, year: Option<i32>, overview: Option<&str>,
                        poster_path: Option<&str>, folder_path: &str, tmdb_id: Option<&str>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO media (title, year, overview, poster_path, file_path, media_type, tmdb_id) 
             VALUES (?, ?, ?, ?, ?, 'tvshow', ?)",
            params![title, year, overview, poster_path, folder_path, tmdb_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
    
    pub fn insert_episode(&self, title: &str, file_path: &str, parent_id: i64,
                         season: i32, episode: i32, duration: f64) -> Result<i64> {
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

    // ==================== CLOUD MEDIA METHODS ====================

    /// Insert a cloud movie
    pub fn insert_cloud_movie(
        &self,
        title: &str,
        year: Option<i32>,
        overview: Option<&str>,
        poster_path: Option<&str>,
        file_name: &str,
        cloud_file_id: &str,
        cloud_folder_id: &str,
        tmdb_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO media (title, year, overview, poster_path, file_path, media_type, tmdb_id, is_cloud, cloud_file_id, cloud_folder_id)
             VALUES (?, ?, ?, ?, ?, 'movie', ?, 1, ?, ?)",
            params![title, year, overview, poster_path, file_name, tmdb_id, cloud_file_id, cloud_folder_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a cloud TV show
    pub fn insert_cloud_tvshow(
        &self,
        title: &str,
        year: Option<i32>,
        overview: Option<&str>,
        poster_path: Option<&str>,
        folder_name: &str,
        cloud_folder_id: &str,
        tmdb_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO media (title, year, overview, poster_path, file_path, media_type, tmdb_id, is_cloud, cloud_folder_id)
             VALUES (?, ?, ?, ?, ?, 'tvshow', ?, 1, ?)",
            params![title, year, overview, poster_path, folder_name, tmdb_id, cloud_folder_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a cloud episode
    pub fn insert_cloud_episode(
        &self,
        title: &str,
        file_name: &str,
        parent_id: i64,
        season: i32,
        episode: i32,
        cloud_file_id: &str,
        cloud_folder_id: &str,
        episode_title: Option<&str>,
        overview: Option<&str>,
        still_path: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO media (title, file_path, media_type, parent_id, season_number, episode_number,
                               is_cloud, cloud_file_id, cloud_folder_id, episode_title, overview, still_path)
             VALUES (?, ?, 'tvepisode', ?, ?, ?, 1, ?, ?, ?, ?, ?)",
            params![title, file_name, parent_id, season, episode, cloud_file_id, cloud_folder_id,
                   episode_title, overview, still_path],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Check if a cloud file already exists in the database
    pub fn cloud_file_exists(&self, cloud_file_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM media WHERE cloud_file_id = ?",
                params![cloud_file_id],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// Get cloud media by folder ID
    pub fn get_cloud_media_by_folder(&self, cloud_folder_id: &str) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    is_cloud, cloud_file_id
             FROM media WHERE cloud_folder_id = ?"
        )?;

        let items = stmt.query_map(params![cloud_folder_id], Self::map_media_item)?;
        items.filter_map(|r| r.ok()).collect::<Vec<_>>().into_iter().map(Ok).collect()
    }

    /// Delete all cloud media for a folder
    pub fn delete_cloud_folder_media(&self, cloud_folder_id: &str) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM media WHERE cloud_folder_id = ?",
            params![cloud_folder_id],
        )?;
        Ok(deleted)
    }

    // ==================== CLOUD FOLDER MANAGEMENT ====================

    /// Add a cloud folder to track
    pub fn add_cloud_folder(&self, folder_id: &str, folder_name: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT OR REPLACE INTO cloud_folders (folder_id, folder_name, auto_scan) VALUES (?, ?, 1)",
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
            "SELECT folder_id, folder_name, auto_scan FROM cloud_folders ORDER BY created_at"
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

        let items = stmt.query_map(params![folder_id], |row| {
            row.get::<_, String>(0)
        })?;

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

    /// Get all episodes user has for a series (returns id, season_number, episode_number)
    pub fn get_owned_episodes_for_series(&self, series_id: i64) -> Result<Vec<(i64, i32, i32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, season_number, episode_number FROM media
             WHERE parent_id = ? AND media_type = 'tvepisode'
             ORDER BY season_number, episode_number"
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
        let mut stmt = self.conn.prepare(
            "SELECT id FROM media WHERE tmdb_id = ? AND media_type = 'tvshow'"
        )?;

        match stmt.query_row(params![tmdb_id], |row| row.get::<_, i64>(0)) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Find a TV show by title (case-insensitive) - returns the MediaItem
    pub fn find_tvshow_by_title(&self, title: &str) -> Result<Option<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    is_cloud, cloud_file_id
             FROM media WHERE LOWER(title) = LOWER(?) AND media_type = 'tvshow'"
        )?;

        match stmt.query_row(params![title], Self::map_media_item) {
            Ok(item) => Ok(Some(item)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
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
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO cached_episode_metadata
             (series_tmdb_id, season_number, episode_number, episode_title, overview, still_path, air_date, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'))",
            params![series_tmdb_id, season_number, episode_number, episode_title, overview, still_path, air_date],
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
            "SELECT episode_title, overview, still_path, air_date
             FROM cached_episode_metadata
             WHERE series_tmdb_id = ? AND season_number = ? AND episode_number = ?"
        )?;

        match stmt.query_row(params![series_tmdb_id, season_number, episode_number], |row| {
            Ok(CachedEpisodeMetadata {
                episode_title: row.get(0)?,
                overview: row.get(1)?,
                still_path: row.get(2)?,
                air_date: row.get(3)?,
            })
        }) {
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
            |row| row.get(0)
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
    pub fn get_all_cached_episodes_for_series(&self, series_tmdb_id: &str) -> Result<Vec<CachedEpisodeMetadata>> {
        let mut stmt = self.conn.prepare(
            "SELECT episode_title, overview, still_path, air_date, season_number, episode_number
             FROM cached_episode_metadata
             WHERE series_tmdb_id = ?
             ORDER BY season_number, episode_number"
        )?;

        let items = stmt.query_map(params![series_tmdb_id], |row| {
            Ok(CachedEpisodeMetadataFull {
                episode_title: row.get(0)?,
                overview: row.get(1)?,
                still_path: row.get(2)?,
                air_date: row.get(3)?,
                season_number: row.get(4)?,
                episode_number: row.get(5)?,
            })
        })?;

        items.filter_map(|r| r.ok().map(|f| CachedEpisodeMetadata {
            episode_title: f.episode_title,
            overview: f.overview,
            still_path: f.still_path,
            air_date: f.air_date,
        })).collect::<Vec<_>>().into_iter().map(Ok).collect()
    }

    /// Get cached episodes for a specific season of a series
    pub fn get_cached_episodes_for_season(&self, series_tmdb_id: &str, season_number: i32) -> Result<Vec<CachedEpisodeMetadataFull>> {
        let mut stmt = self.conn.prepare(
            "SELECT episode_title, overview, still_path, air_date, season_number, episode_number
             FROM cached_episode_metadata
             WHERE series_tmdb_id = ? AND season_number = ?
             ORDER BY episode_number"
        )?;

        let items = stmt.query_map(params![series_tmdb_id, season_number], |row| {
            Ok(CachedEpisodeMetadataFull {
                episode_title: row.get(0)?,
                overview: row.get(1)?,
                still_path: row.get(2)?,
                air_date: row.get(3)?,
                season_number: row.get(4)?,
                episode_number: row.get(5)?,
            })
        })?;

        items.collect()
    }

    /// Get all media entries (for cleanup purposes)
    pub fn get_all_media(&self) -> Result<Vec<MediaItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, year, overview, poster_path, file_path, media_type,
                    duration_seconds, resume_position_seconds, last_watched,
                    season_number, episode_number, parent_id, tmdb_id, episode_title, still_path,
                    is_cloud, cloud_file_id
             FROM media"
        )?;

        let items = stmt.query_map([], Self::map_media_item)?;
        items.filter_map(|r| r.ok()).collect::<Vec<_>>().into_iter().map(Ok).collect()
    }
    
    /// Get all poster paths currently in use (including still_paths and cached episode images)
    pub fn get_all_poster_paths(&self) -> Result<Vec<String>> {
        let mut all_paths = Vec::new();

        // Get poster paths from media table
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT poster_path FROM media WHERE poster_path IS NOT NULL"
        )?;
        let paths = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for path in paths.filter_map(|r| r.ok()) {
            all_paths.push(path);
        }

        // Get still paths from media table
        let mut stmt2 = self.conn.prepare(
            "SELECT DISTINCT still_path FROM media WHERE still_path IS NOT NULL"
        )?;
        let still_paths = stmt2.query_map([], |row| row.get::<_, String>(0))?;
        for path in still_paths.filter_map(|r| r.ok()) {
            all_paths.push(path);
        }

        // Get still paths from cached episode metadata
        let mut stmt3 = self.conn.prepare(
            "SELECT DISTINCT still_path FROM cached_episode_metadata WHERE still_path IS NOT NULL"
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
        let poster_path: Option<String> = self.conn.query_row(
            "SELECT poster_path FROM media WHERE id = ?",
            params![id],
            |row| row.get(0)
        ).ok();
        
        // Delete the entry
        self.conn.execute("DELETE FROM media WHERE id = ?", params![id])?;
        
        Ok(poster_path)
    }
    
    /// Remove all episodes for a series
    pub fn remove_series_episodes(&self, series_id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM media WHERE parent_id = ?", params![series_id])?;
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
        let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
        
        let results = stmt.query_map(params.as_slice(), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        
        results.filter_map(|r| r.ok()).collect::<Vec<_>>().into_iter().map(Ok).collect()
    }

    /// Get cloud info for a series (is_cloud, cloud_folder_id)
    pub fn get_series_cloud_info(&self, series_id: i64) -> Result<(bool, Option<String>)> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(is_cloud, 0), cloud_folder_id FROM media WHERE id = ?"
        )?;

        stmt.query_row(params![series_id], |row| {
            Ok((
                row.get::<_, i32>(0)? == 1,
                row.get::<_, Option<String>>(1)?,
            ))
        })
    }

    /// Get media info for deletion (file_path, is_cloud, cloud_file_id)
    pub fn get_media_delete_info(&self, ids: &[i64]) -> Result<Vec<(i64, Option<String>, bool, Option<String>)>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "SELECT id, file_path, COALESCE(is_cloud, 0) as is_cloud, cloud_file_id FROM media WHERE id IN ({})",
            placeholders.join(", ")
        );

        let mut stmt = self.conn.prepare(&query)?;
        let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

        let results = stmt.query_map(params.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i32>(2)? == 1,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;

        results.filter_map(|r| r.ok()).collect::<Vec<_>>().into_iter().map(Ok).collect()
    }

    /// Delete multiple media entries and return their file paths for cleanup
    pub fn delete_media_entries(&self, ids: &[i64]) -> Result<Vec<String>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        
        // First get the file paths
        let file_paths: Vec<String> = self.get_media_file_paths(ids)?
            .into_iter()
            .filter_map(|(_, path)| path)
            .collect();
        
        // Delete all entries
        let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "DELETE FROM media WHERE id IN ({})",
            placeholders.join(", ")
        );
        
        let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
        self.conn.execute(&query, params.as_slice())?;
        
        Ok(file_paths)
    }
    
    /// Check if a series has any remaining episodes
    pub fn series_has_episodes(&self, series_id: i64) -> Result<bool> {
        let count: i32 = self.conn.query_row(
            "SELECT COUNT(*) FROM media WHERE parent_id = ?",
            params![series_id],
            |row| row.get(0)
        )?;
        Ok(count > 0)
    }
    
    /// Merge duplicate TV shows into a single entry.
    /// Groups by TMDB ID first, then by title (case-insensitive).
    /// Keeps the entry with the most complete metadata as the primary.
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
                 HAVING cnt > 1"
            )?;
            
            let results: Vec<(String, Vec<i64>)> = stmt.query_map([], |row| {
                let tmdb_id: String = row.get(0)?;
                let ids_str: String = row.get(1)?;
                let ids: Vec<i64> = ids_str.split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                Ok((tmdb_id, ids))
            })?.filter_map(|r| r.ok()).collect();
            results
        };
        
        for (tmdb_id, ids) in tmdb_duplicates {
            if ids.len() > 1 {
                println!("[MERGE] Found {} duplicates with TMDB ID: {}", ids.len(), tmdb_id);
                merged_count += self.merge_series_entries(&ids)?;
            }
        }
        
        // Step 2: Find and merge duplicates by same title (case-insensitive) without TMDB ID
        let title_duplicates: Vec<(String, Vec<i64>)> = {
            let mut stmt = self.conn.prepare(
                "SELECT LOWER(title), GROUP_CONCAT(id) as ids, COUNT(*) as cnt 
                 FROM media 
                 WHERE media_type = 'tvshow'
                 GROUP BY LOWER(title) 
                 HAVING cnt > 1"
            )?;
            
            let results: Vec<(String, Vec<i64>)> = stmt.query_map([], |row| {
                let title: String = row.get(0)?;
                let ids_str: String = row.get(1)?;
                let ids: Vec<i64> = ids_str.split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                Ok((title, ids))
            })?.filter_map(|r| r.ok()).collect();
            results
        };
        
        for (title, ids) in title_duplicates {
            if ids.len() > 1 {
                println!("[MERGE] Found {} duplicates with title: {}", ids.len(), title);
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
            let score: i32 = self.conn.query_row(
                "SELECT 
                    (CASE WHEN tmdb_id IS NOT NULL AND tmdb_id != '' THEN 10 ELSE 0 END) +
                    (CASE WHEN poster_path IS NOT NULL AND poster_path != '' THEN 5 ELSE 0 END) +
                    (CASE WHEN overview IS NOT NULL AND overview != '' THEN 2 ELSE 0 END) +
                    (CASE WHEN year IS NOT NULL THEN 1 ELSE 0 END)
                 FROM media WHERE id = ?",
                params![id],
                |row| row.get(0)
            ).unwrap_or(0);
            
            if score > best_score {
                best_score = score;
                best_id = id;
            }
        }
        
        // Get the best entry's metadata for reference
        let best_title: String = self.conn.query_row(
            "SELECT title FROM media WHERE id = ?",
            params![best_id],
            |row| row.get(0)
        ).unwrap_or_else(|_| "Unknown".to_string());
        
        println!("[MERGE] Keeping series ID {} ({}) as primary", best_id, best_title);
        
        let mut merged = 0;
        
        // Move all episodes from other entries to the best entry
        for &id in ids {
            if id != best_id {
                // Count episodes that will be moved
                let episode_count: i32 = self.conn.query_row(
                    "SELECT COUNT(*) FROM media WHERE parent_id = ?",
                    params![id],
                    |row| row.get(0)
                ).unwrap_or(0);
                
                println!("[MERGE] Moving {} episodes from series {} to {}", episode_count, id, best_id);
                
                // Move episodes to primary series
                self.conn.execute(
                    "UPDATE media SET parent_id = ? WHERE parent_id = ?",
                    params![best_id, id]
                )?;
                
                // Delete the duplicate series entry
                self.conn.execute(
                    "DELETE FROM media WHERE id = ?",
                    params![id]
                )?;
                
                merged += 1;
            }
        }
        
        Ok(merged)
    }

    /// Clear all app data - deletes database tables and image cache
    /// Returns the path to the image cache directory for cleanup
    pub fn clear_all_data(&self) -> Result<String> {
        // Delete all data from streaming_history
        self.conn.execute("DELETE FROM streaming_history", [])?;

        // Delete all data from media table
        self.conn.execute("DELETE FROM media", [])?;

        // Delete all cached episode metadata (important - stale cache causes missing images)
        self.conn.execute("DELETE FROM cached_episode_metadata", [])?;

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
               AND file_path NOT LIKE '%\\%'"
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

    fn map_media_item(row: &rusqlite::Row) -> rusqlite::Result<MediaItem> {
        let duration: Option<f64> = row.get(7)?;
        let resume_pos: Option<f64> = row.get(8)?;
        let last_watched: Option<String> = row.get(9)?;

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
        let is_cloud_int: Option<i32> = row.get(16).ok();
        let is_cloud = is_cloud_int.map(|v| v != 0);

        Ok(MediaItem {
            id: row.get(0)?,
            title: row.get(1)?,
            year: row.get(2)?,
            overview: row.get(3)?,
            poster_path: row.get(4)?,
            file_path: row.get(5)?,
            media_type: row.get(6)?,
            duration_seconds: duration,
            resume_position_seconds: resume_pos,
            last_watched: row.get(9)?,
            season_number: row.get(10)?,
            episode_number: row.get(11)?,
            parent_id: row.get(12)?,
            progress_percent,
            tmdb_id: row.get(13)?,
            episode_title: row.get(14)?,
            still_path: row.get(15)?,
            is_cloud,
            cloud_file_id: row.get(17).ok(),
        })
    }
}
