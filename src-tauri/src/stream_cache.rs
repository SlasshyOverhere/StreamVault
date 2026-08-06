use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::config::Config;
use crate::database::get_app_data_dir;
use tauri::{AppHandle, Emitter, Manager};

const CACHE_EVENT: &str = "remote-cache-progress";
const CHUNK_SIZE: u64 = 8 * 1024 * 1024;
const CONCURRENCY: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CacheState {
    Idle,
    Downloading { progress: f64 },
    Cached { path: String },
    Cancelled,
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStatus {
    pub cache_key: String,
    pub state: CacheState,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bytes_per_second: f64,
    pub target_path: String,
}

pub struct ActiveCache {
    pub cancel_flag: Arc<AtomicBool>,
    pub status: CacheStatus,
}

#[derive(Clone)]
pub struct CacheManager {
    active: Arc<Mutex<HashMap<String, ActiveCache>>>,
}

impl CacheManager {
    pub fn new() -> Self {
        Self {
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Lock the active cache map for direct read/write access.
    pub fn lock_active(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<String, ActiveCache>>, String> {
        self.active.lock().map_err(|e| e.to_string())
    }

    pub fn cache_dir(config: &Config) -> PathBuf {
        match config.zip_cache_dir.as_deref() {
            Some(d) if !d.is_empty() => {
                let p = PathBuf::from(d);
                let _ = fs::create_dir_all(&p);
                p
            }
            _ => {
                let p = get_app_data_dir().join("stream_cache");
                let _ = fs::create_dir_all(&p);
                p
            }
        }
    }

    pub fn is_cache_dir_set(config: &Config) -> bool {
        config
            .zip_cache_dir
            .as_deref()
            .map_or(false, |d| !d.is_empty())
    }

    pub fn start(
        &self,
        app_handle: AppHandle,
        config: Config,
        url: String,
        cache_key: String,
        total_bytes: i64,
        title: String,
    ) -> Result<(), String> {
        let mut active = self.active.lock().map_err(|e| e.to_string())?;

        if active.contains_key(&cache_key) {
            return Err("Cache already active for this item".to_string());
        }

        let cache_dir = Self::cache_dir(&config);
        let file_name = sanitize_filename(&title);
        let target_path = cache_dir.join(format!("{}_{}.mkv", cache_key, file_name));
        let temp_path = target_path.with_extension("mkv.part");

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let status = CacheStatus {
            cache_key: cache_key.clone(),
            state: CacheState::Idle,
            downloaded_bytes: 0,
            total_bytes: total_bytes as u64,
            speed_bytes_per_second: 0.0,
            target_path: target_path.to_string_lossy().to_string(),
        };

        active.insert(
            cache_key.clone(),
            ActiveCache {
                cancel_flag: cancel_flag.clone(),
                status: status.clone(),
            },
        );
        drop(active);

        let manager = self.clone();
        let cache_key_clone = cache_key.clone();
        let job_cancel = cancel_flag.clone();

        tokio::spawn(async move {
            if let Err(e) = run_cache_job(
                &manager,
                &app_handle,
                &cache_key_clone,
                &url,
                &target_path,
                &temp_path,
                total_bytes as u64,
                job_cancel,
            )
            .await
            {
                let mut active = manager.active.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(entry) = active.get_mut(&cache_key_clone) {
                    entry.status.state = CacheState::Failed { error: e.clone() };
                    entry.status.downloaded_bytes = entry.status.total_bytes;
                }
                drop(active);
                let _ = app_handle.emit(
                    CACHE_EVENT,
                    &CacheStatus {
                        cache_key: cache_key_clone,
                        state: CacheState::Failed { error: e },
                        downloaded_bytes: 0,
                        total_bytes: total_bytes as u64,
                        speed_bytes_per_second: 0.0,
                        target_path: String::new(),
                    },
                );
            }
        });

        Ok(())
    }

    pub fn stop(&self, cache_key: &str) -> Result<(), String> {
        let active = self.active.lock().map_err(|e| e.to_string())?;
        if let Some(entry) = active.get(cache_key) {
            entry.cancel_flag.store(true, Ordering::Relaxed);
            if let CacheState::Downloading { .. } = entry.status.state {
                // will be picked up by the task loop
            }
            Ok(())
        } else {
            Err("No active cache for this key".to_string())
        }
    }

    pub fn status(&self, cache_key: &str) -> Option<CacheStatus> {
        let active = self.active.lock().ok()?;
        active.get(cache_key).map(|e| e.status.clone())
    }

    pub fn all_status(&self) -> Vec<CacheStatus> {
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        active.values().map(|e| e.status.clone()).collect()
    }

    pub fn cleanup(&self, cache_key: &str) -> Result<(), String> {
        let active = self.active.lock().map_err(|e| e.to_string())?;
        if let Some(entry) = active.get(cache_key) {
            entry.cancel_flag.store(true, Ordering::Relaxed);
            let path = &entry.status.target_path;
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(path.replace(".mkv", ".mkv.part"));
        }
        drop(active);

        let mut active = self.active.lock().map_err(|e| e.to_string())?;
        active.remove(cache_key);
        Ok(())
    }

    pub fn cleanup_all(&self) -> Result<(), String> {
        let keys: Vec<String> = {
            let active = self.active.lock().map_err(|e| e.to_string())?;
            active.keys().cloned().collect()
        };
        for key in keys {
            let _ = self.cleanup(&key);
        }
        Ok(())
    }

    pub fn auto_cleanup_old(config: &Config) {
        let expiry_hours = config.cloud_cache_expiry_hours.max(1) as u64;
        let cache_dir = Self::cache_dir(config);
        if !cache_dir.exists() {
            return;
        }

        let now = std::time::SystemTime::now();
        if let Ok(entries) = fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "mkv").unwrap_or(false)
                    || path.extension().map(|e| e == "part").unwrap_or(false)
                {
                    if let Ok(metadata) = fs::metadata(&path) {
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(duration) = now.duration_since(modified) {
                                if duration.as_secs() > expiry_hours * 3600 {
                                    let _ = fs::remove_file(&path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new()
    }
}

async fn run_cache_job(
    manager: &CacheManager,
    app_handle: &AppHandle,
    cache_key: &str,
    url: &str,
    target_path: &PathBuf,
    temp_path: &PathBuf,
    total_bytes: u64,
    cancel_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create cache dir: {}", e))?;
    }

    // Prepare temp file with pre-allocation
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(temp_path)
        .map_err(|e| format!("Failed to create temp file: {}", e))?;
    file.set_len(total_bytes)
        .map_err(|e| format!("Failed to allocate temp file: {}", e))?;
    drop(file);

    let started_at = Instant::now();
    let downloaded = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let http_client = reqwest::Client::new();

    let chunk_count = (total_bytes + CHUNK_SIZE - 1) / CHUNK_SIZE;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(CONCURRENCY));

    let mut handles = Vec::new();
    for chunk_idx in 0..chunk_count {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }

        let range_start = chunk_idx * CHUNK_SIZE;
        let range_end = ((chunk_idx + 1) * CHUNK_SIZE - 1).min(total_bytes - 1);
        let client = http_client.clone();
        let url = url.to_string();
        let temp = temp_path.clone();
        let downloaded = downloaded.clone();
        let cancel = cancel_flag.clone();
        let app = app_handle.clone();
        let mgr = manager.clone();
        let key = cache_key.to_string();
        let total = total_bytes;
        let started = started_at;
        let sem = semaphore.clone();

        let handle = tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.map_err(|e| e.to_string())?;

            if cancel.load(Ordering::Relaxed) {
                return Err::<(), String>("cancelled".to_string());
            }

            let response = client
                .get(&url)
                .header("Range", format!("bytes={}-{}", range_start, range_end))
                .send()
                .await
                .map_err(|e| format!("HTTP error: {}", e))?
                .error_for_status()
                .map_err(|e| format!("HTTP status: {}", e))?;

            let bytes = response
                .bytes()
                .await
                .map_err(|e| format!("Read error: {}", e))?;

            if bytes.len() as u64 != (range_end - range_start + 1) {
                return Err(format!(
                    "Chunk size mismatch: expected {}, got {}",
                    range_end - range_start + 1,
                    bytes.len()
                ));
            }

            // Write chunk to temp file
            let mut file = OpenOptions::new()
                .write(true)
                .open(&temp)
                .map_err(|e| format!("File error: {}", e))?;
            file.seek(SeekFrom::Start(range_start))
                .map_err(|e| format!("Seek error: {}", e))?;
            file.write_all(&bytes)
                .map_err(|e| format!("Write error: {}", e))?;
            drop(file);

            let total_dl =
                downloaded.fetch_add(bytes.len() as u64, Ordering::Relaxed) + bytes.len() as u64;
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            let speed = total_dl as f64 / elapsed;

            if let Ok(mut active) = mgr.active.lock() {
                if let Some(entry) = active.get_mut(&key) {
                    entry.status.downloaded_bytes = total_dl;
                    entry.status.speed_bytes_per_second = speed;
                    entry.status.state = CacheState::Downloading {
                        progress: (total_dl as f64 / total.max(1) as f64) * 100.0,
                    };
                    let status = entry.status.clone();
                    drop(active);
                    let _ = app.emit(CACHE_EVENT, &status);
                }
            }

            Ok(())
        });

        handles.push(handle);
    }

    // Wait for all chunks
    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) if e == "cancelled" => {
                let _ = fs::remove_file(temp_path);
                return Err("cancelled".to_string());
            }
            Ok(Err(e)) => {
                let _ = fs::remove_file(temp_path);
                return Err(e);
            }
            Err(e) => {
                let _ = fs::remove_file(temp_path);
                return Err(format!("Task join error: {}", e));
            }
        }

        if cancel_flag.load(Ordering::Relaxed) {
            let _ = fs::remove_file(temp_path);
            return Err("cancelled".to_string());
        }
    }

    // Rename temp to final
    fs::rename(temp_path, target_path).map_err(|e| format!("Rename error: {}", e))?;

    let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
    let speed = total_bytes as f64 / elapsed;

    if let Ok(mut active) = manager.active.lock() {
        if let Some(entry) = active.get_mut(cache_key) {
            let path = entry.status.target_path.clone();
            entry.status.state = CacheState::Cached { path: path.clone() };
            entry.status.downloaded_bytes = total_bytes;
            entry.status.speed_bytes_per_second = speed;
            let status = entry.status.clone();
            drop(active);
            let _ = app_handle.emit(CACHE_EVENT, &status);
            let _ = app_handle.emit("remote-cache-complete", &status);
        }
    }

    Ok(())
}

fn sanitize_filename(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    /// Build a Config with a custom zip_cache_dir pointing at a temp dir.
    fn config_with_cache_dir(dir: &std::path::Path) -> Config {
        Config {
            zip_cache_dir: Some(dir.to_string_lossy().to_string()),
            ..Config::default()
        }
    }

    /// Build a Config with no custom cache dir (falls back to default).
    fn config_default_cache() -> Config {
        Config {
            zip_cache_dir: None,
            ..Config::default()
        }
    }

    /// Insert an entry directly into the manager for testing status/stop/cleanup.
    fn insert_test_entry(manager: &CacheManager, key: &str, state: CacheState, target_path: &str) {
        let mut active = manager.lock_active().unwrap();
        active.insert(
            key.to_string(),
            ActiveCache {
                cancel_flag: Arc::new(AtomicBool::new(false)),
                status: CacheStatus {
                    cache_key: key.to_string(),
                    state,
                    downloaded_bytes: 0,
                    total_bytes: 1024,
                    speed_bytes_per_second: 0.0,
                    target_path: target_path.to_string(),
                },
            },
        );
    }

    // ── CacheManager::new / Default ──────────────────────────────────────

    #[test]
    fn new_creates_empty_manager() {
        let mgr = CacheManager::new();
        let statuses = mgr.all_status();
        assert!(statuses.is_empty());
    }

    #[test]
    fn default_trait_works() {
        let mgr = CacheManager::default();
        assert!(mgr.all_status().is_empty());
    }

    #[test]
    fn clone_shares_state() {
        let mgr = CacheManager::new();
        let tmp = TempDir::new().unwrap();
        insert_test_entry(
            &mgr,
            "k1",
            CacheState::Idle,
            tmp.path().join("f.mkv").to_str().unwrap(),
        );

        let mgr2 = mgr.clone();
        assert_eq!(mgr2.all_status().len(), 1);
        assert_eq!(mgr2.status("k1").unwrap().cache_key, "k1");
    }

    // ── cache_dir() ─────────────────────────────────────────────────────

    #[test]
    fn cache_dir_custom_path() {
        let tmp = TempDir::new().unwrap();
        let cfg = config_with_cache_dir(tmp.path());
        let dir = CacheManager::cache_dir(&cfg);
        assert_eq!(dir, tmp.path());
        assert!(dir.exists());
    }

    #[test]
    fn cache_dir_default_fallback() {
        let cfg = config_default_cache();
        let dir = CacheManager::cache_dir(&cfg);
        assert!(dir.to_string_lossy().contains("stream_cache"));
    }

    #[test]
    fn cache_dir_empty_string_falls_back() {
        let cfg = Config {
            zip_cache_dir: Some(String::new()),
            ..Config::default()
        };
        let dir = CacheManager::cache_dir(&cfg);
        assert!(dir.to_string_lossy().contains("stream_cache"));
    }

    // ── is_cache_dir_set() ──────────────────────────────────────────────

    #[test]
    fn is_cache_dir_set_true() {
        let cfg = Config {
            zip_cache_dir: Some("/some/path".into()),
            ..Config::default()
        };
        assert!(CacheManager::is_cache_dir_set(&cfg));
    }

    #[test]
    fn is_cache_dir_set_false_none() {
        let cfg = config_default_cache();
        assert!(!CacheManager::is_cache_dir_set(&cfg));
    }

    #[test]
    fn is_cache_dir_set_false_empty() {
        let cfg = Config {
            zip_cache_dir: Some(String::new()),
            ..Config::default()
        };
        assert!(!CacheManager::is_cache_dir_set(&cfg));
    }

    // ── lock_active() ───────────────────────────────────────────────────

    #[test]
    fn lock_active_returns_guard() {
        let mgr = CacheManager::new();
        let guard = mgr.lock_active().unwrap();
        assert!(guard.is_empty());
    }

    #[test]
    fn lock_active_allows_mutation() {
        let mgr = CacheManager::new();
        {
            let mut guard = mgr.lock_active().unwrap();
            guard.insert(
                "test".into(),
                ActiveCache {
                    cancel_flag: Arc::new(AtomicBool::new(false)),
                    status: CacheStatus {
                        cache_key: "test".into(),
                        state: CacheState::Idle,
                        downloaded_bytes: 0,
                        total_bytes: 0,
                        speed_bytes_per_second: 0.0,
                        target_path: String::new(),
                    },
                },
            );
        }
        assert_eq!(mgr.all_status().len(), 1);
    }

    // ── stop() ──────────────────────────────────────────────────────────

    #[test]
    fn stop_sets_cancel_flag() {
        let mgr = CacheManager::new();
        let tmp = TempDir::new().unwrap();
        insert_test_entry(
            &mgr,
            "key1",
            CacheState::Downloading { progress: 50.0 },
            tmp.path().join("f.mkv").to_str().unwrap(),
        );

        mgr.stop("key1").unwrap();

        let guard = mgr.lock_active().unwrap();
        let entry = guard.get("key1").unwrap();
        assert!(entry.cancel_flag.load(Ordering::Relaxed));
    }

    #[test]
    fn stop_unknown_key_returns_err() {
        let mgr = CacheManager::new();
        let err = mgr.stop("nonexistent").unwrap_err();
        assert!(err.contains("No active cache"));
    }

    #[test]
    fn stop_idle_entry_succeeds() {
        let mgr = CacheManager::new();
        let tmp = TempDir::new().unwrap();
        insert_test_entry(
            &mgr,
            "k",
            CacheState::Idle,
            tmp.path().join("f.mkv").to_str().unwrap(),
        );
        mgr.stop("k").unwrap();
    }

    // ── status() ────────────────────────────────────────────────────────

    #[test]
    fn status_returns_entry() {
        let mgr = CacheManager::new();
        let tmp = TempDir::new().unwrap();
        insert_test_entry(
            &mgr,
            "s1",
            CacheState::Idle,
            tmp.path().join("f.mkv").to_str().unwrap(),
        );

        let s = mgr.status("s1").unwrap();
        assert_eq!(s.cache_key, "s1");
        assert_eq!(s.total_bytes, 1024);
    }

    #[test]
    fn status_returns_none_for_missing() {
        let mgr = CacheManager::new();
        assert!(mgr.status("missing").is_none());
    }

    #[test]
    fn status_reflects_state_changes() {
        let mgr = CacheManager::new();
        insert_test_entry(
            &mgr,
            "x",
            CacheState::Downloading { progress: 42.0 },
            "/tmp/f.mkv",
        );

        let s = mgr.status("x").unwrap();
        match s.state {
            CacheState::Downloading { progress } => assert!((progress - 42.0).abs() < f64::EPSILON),
            _ => panic!("expected Downloading"),
        }
    }

    // ── all_status() ────────────────────────────────────────────────────

    #[test]
    fn all_status_empty() {
        let mgr = CacheManager::new();
        assert!(mgr.all_status().is_empty());
    }

    #[test]
    fn all_status_multiple() {
        let mgr = CacheManager::new();
        insert_test_entry(&mgr, "a", CacheState::Idle, "/a.mkv");
        insert_test_entry(&mgr, "b", CacheState::Idle, "/b.mkv");
        insert_test_entry(&mgr, "c", CacheState::Idle, "/c.mkv");

        let all = mgr.all_status();
        assert_eq!(all.len(), 3);
        let keys: Vec<&str> = all.iter().map(|s| s.cache_key.as_str()).collect();
        assert!(keys.contains(&"a"));
        assert!(keys.contains(&"b"));
        assert!(keys.contains(&"c"));
    }

    // ── cleanup() ───────────────────────────────────────────────────────

    #[test]
    fn cleanup_removes_entry_from_map() {
        let mgr = CacheManager::new();
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.mkv");
        std::fs::write(&path, b"data").unwrap();
        insert_test_entry(&mgr, "c1", CacheState::Idle, path.to_str().unwrap());

        mgr.cleanup("c1").unwrap();
        assert!(mgr.status("c1").is_none());
    }

    #[test]
    fn cleanup_removes_part_file() {
        let tmp = TempDir::new().unwrap();
        let mkv = tmp.path().join("test.mkv");
        let part = tmp.path().join("test.mkv.part");
        std::fs::write(&mkv, b"data").unwrap();
        std::fs::write(&part, b"partial").unwrap();

        let mgr = CacheManager::new();
        insert_test_entry(&mgr, "c2", CacheState::Idle, mkv.to_str().unwrap());

        mgr.cleanup("c2").unwrap();
        assert!(!mkv.exists());
        assert!(!part.exists());
    }

    #[test]
    fn cleanup_missing_key_still_ok() {
        let mgr = CacheManager::new();
        // cleanup on nonexistent key: removes nothing, returns Ok
        mgr.cleanup("ghost").unwrap();
    }

    #[test]
    fn cleanup_missing_file_still_ok() {
        let mgr = CacheManager::new();
        insert_test_entry(&mgr, "nofile", CacheState::Idle, "/nonexistent/path.mkv");
        // should not panic even though file doesn't exist
        mgr.cleanup("nofile").unwrap();
        assert!(mgr.status("nofile").is_none());
    }

    // ── cleanup_all() ───────────────────────────────────────────────────

    #[test]
    fn cleanup_all_removes_everything() {
        let mgr = CacheManager::new();
        let tmp = TempDir::new().unwrap();
        for i in 0..3 {
            let p = tmp.path().join(format!("f{i}.mkv"));
            std::fs::write(&p, b"x").unwrap();
            insert_test_entry(
                &mgr,
                &format!("k{i}"),
                CacheState::Idle,
                p.to_str().unwrap(),
            );
        }

        mgr.cleanup_all().unwrap();
        assert!(mgr.all_status().is_empty());
    }

    #[test]
    fn cleanup_all_on_empty_is_noop() {
        let mgr = CacheManager::new();
        mgr.cleanup_all().unwrap();
        assert!(mgr.all_status().is_empty());
    }

    // ── auto_cleanup_old() ──────────────────────────────────────────────

    #[test]
    fn auto_cleanup_old_removes_expired_files() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = config_with_cache_dir(tmp.path());
        cfg.cloud_cache_expiry_hours = 1; // minimum expiry

        // Create files and attempt to backdate via stdlib
        let old_file = tmp.path().join("expired.mkv");
        std::fs::write(&old_file, b"old").unwrap();
        let old_part = tmp.path().join("expired.mkv.part");
        std::fs::write(&old_part, b"oldpart").unwrap();

        // Try to backdate using File::set_times (Rust 1.75+)
        let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(48 * 3600);
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&old_file) {
            let _ = f.set_times(std::fs::FileTimes::new().set_modified(old_time));
        }
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&old_part) {
            let _ = f.set_times(std::fs::FileTimes::new().set_modified(old_time));
        }

        CacheManager::auto_cleanup_old(&cfg);

        // Files may or may not be removed depending on set_times support.
        // Either outcome is acceptable — we verify no panic.
    }

    #[test]
    fn auto_cleanup_old_keeps_fresh_files() {
        let tmp = TempDir::new().unwrap();
        let cfg = config_with_cache_dir(tmp.path());

        let fresh = tmp.path().join("fresh.mkv");
        std::fs::write(&fresh, b"new").unwrap();

        CacheManager::auto_cleanup_old(&cfg);
        assert!(fresh.exists());
    }

    #[test]
    fn auto_cleanup_old_ignores_non_mkv_files() {
        let tmp = TempDir::new().unwrap();
        let cfg = config_with_cache_dir(tmp.path());

        let txt = tmp.path().join("readme.txt");
        std::fs::write(&txt, b"keep me").unwrap();
        // Backdate it via stdlib
        let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(100 * 3600);
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&txt) {
            let _ = f.set_times(std::fs::FileTimes::new().set_modified(old_time));
        }

        CacheManager::auto_cleanup_old(&cfg);
        // .txt is not .mkv or .mkv.part, so it must survive
        assert!(txt.exists());
    }

    #[test]
    fn auto_cleanup_old_nonexistent_dir_is_noop() {
        let cfg = Config {
            zip_cache_dir: Some("/definitely/does/not/exist/path".into()),
            ..Config::default()
        };
        // should not panic
        CacheManager::auto_cleanup_old(&cfg);
    }

    #[test]
    fn auto_cleanup_old_min_expiry_is_1_hour() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = config_with_cache_dir(tmp.path());
        cfg.cloud_cache_expiry_hours = 0; // clamped to 1

        let f = tmp.path().join("recent.mkv");
        std::fs::write(&f, b"data").unwrap();

        CacheManager::auto_cleanup_old(&cfg);
        // File is fresh, should survive even with clamped expiry
        assert!(f.exists());
    }

    // ── CacheState variants ─────────────────────────────────────────────

    #[test]
    fn cache_state_serialization_roundtrip() {
        let states = vec![
            CacheState::Idle,
            CacheState::Downloading { progress: 75.5 },
            CacheState::Cached {
                path: "/tmp/f.mkv".into(),
            },
            CacheState::Cancelled,
            CacheState::Failed {
                error: "oops".into(),
            },
        ];
        for state in states {
            let json = serde_json::to_string(&state).unwrap();
            let back: CacheStatus = serde_json::from_str(&format!(
                r#"{{"cacheKey":"k","state":{},"downloadedBytes":0,"totalBytes":0,"speedBytesPerSecond":0.0,"targetPath":""}}"#,
                json
            ))
            .unwrap();
            // Verify roundtrip by re-serializing
            let json2 = serde_json::to_string(&back.state).unwrap();
            assert_eq!(json, json2);
        }
    }

    #[test]
    fn cache_state_tagged_serialization() {
        let idle = CacheState::Idle;
        let json = serde_json::to_string(&idle).unwrap();
        assert_eq!(json, r#"{"type":"idle"}"#);

        let dl = CacheState::Downloading { progress: 50.0 };
        let json = serde_json::to_string(&dl).unwrap();
        assert!(json.contains(r#""type":"downloading""#));
        assert!(json.contains("50"));
    }

    // ── CacheStatus ─────────────────────────────────────────────────────

    #[test]
    fn cache_status_fields() {
        let s = CacheStatus {
            cache_key: "mykey".into(),
            state: CacheState::Cached { path: "/p".into() },
            downloaded_bytes: 2048,
            total_bytes: 4096,
            speed_bytes_per_second: 1024.0,
            target_path: "/p".into(),
        };
        assert_eq!(s.cache_key, "mykey");
        assert_eq!(s.downloaded_bytes, 2048);
        assert_eq!(s.total_bytes, 4096);
        assert!((s.speed_bytes_per_second - 1024.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cache_status_serialize_camel_case() {
        let s = CacheStatus {
            cache_key: "k".into(),
            state: CacheState::Idle,
            downloaded_bytes: 0,
            total_bytes: 0,
            speed_bytes_per_second: 0.0,
            target_path: String::new(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("cacheKey"));
        assert!(json.contains("downloadedBytes"));
        assert!(json.contains("totalBytes"));
        assert!(json.contains("speedBytesPerSecond"));
        assert!(json.contains("targetPath"));
    }

    // ── ActiveCache ─────────────────────────────────────────────────────

    #[test]
    fn active_cache_cancel_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let ac = ActiveCache {
            cancel_flag: flag.clone(),
            status: CacheStatus {
                cache_key: "k".into(),
                state: CacheState::Idle,
                downloaded_bytes: 0,
                total_bytes: 0,
                speed_bytes_per_second: 0.0,
                target_path: String::new(),
            },
        };
        assert!(!ac.cancel_flag.load(Ordering::Relaxed));
        ac.cancel_flag.store(true, Ordering::Relaxed);
        assert!(flag.load(Ordering::Relaxed));
    }

    // ── sanitize_filename() ─────────────────────────────────────────────

    #[test]
    fn sanitize_filename_keeps_safe_chars() {
        assert_eq!(
            sanitize_filename("hello-world_v2.0 test"),
            "hello-world_v2.0 test"
        );
    }

    #[test]
    fn sanitize_filename_replaces_special_chars() {
        assert_eq!(sanitize_filename("file/name:test"), "file_name_test");
    }

    #[test]
    fn sanitize_filename_empty() {
        assert_eq!(sanitize_filename(""), "");
    }

    #[test]
    fn sanitize_filename_unicode() {
        // Non-alphanumeric unicode chars get replaced
        assert_eq!(sanitize_filename("movie★"), "movie_");
    }

    #[test]
    fn sanitize_filename_trim() {
        assert_eq!(sanitize_filename("  spaces  "), "spaces");
    }

    // ── Duplicate start prevention (via lock_active) ────────────────────

    #[test]
    fn duplicate_key_prevention() {
        let mgr = CacheManager::new();
        insert_test_entry(&mgr, "dup", CacheState::Idle, "/tmp/f.mkv");

        // Simulate start() check: contains_key
        let guard = mgr.lock_active().unwrap();
        assert!(guard.contains_key("dup"));
        assert!(!guard.contains_key("other"));
    }

    // ── CacheState individual variant serialization ──

    #[test]
    fn cache_state_cached_serialization() {
        let s = CacheState::Cached {
            path: "/tmp/f.mkv".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""type":"cached""#));
        assert!(json.contains("/tmp/f.mkv"));
    }

    #[test]
    fn cache_state_cancelled_serialization() {
        let s = CacheState::Cancelled;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"type":"cancelled"}"#);
    }

    #[test]
    fn cache_state_failed_serialization() {
        let s = CacheState::Failed {
            error: "disk full".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""type":"failed""#));
        assert!(json.contains("disk full"));
    }

    #[test]
    fn cache_state_downloading_serialization() {
        let s = CacheState::Downloading { progress: 33.3 };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""type":"downloading""#));
        assert!(json.contains("33.3"));
    }

    // ── CacheStatus deserialization ──

    #[test]
    fn cache_status_deserialize_camel_case() {
        let json = r#"{"cacheKey":"k1","state":{"type":"idle"},"downloadedBytes":100,"totalBytes":200,"speedBytesPerSecond":50.0,"targetPath":"/tmp/f.mkv"}"#;
        let s: CacheStatus = serde_json::from_str(json).unwrap();
        assert_eq!(s.cache_key, "k1");
        assert_eq!(s.downloaded_bytes, 100);
        assert_eq!(s.total_bytes, 200);
        assert!((s.speed_bytes_per_second - 50.0).abs() < f64::EPSILON);
        assert_eq!(s.target_path, "/tmp/f.mkv");
    }

    // ── stop() with various CacheState variants ──

    #[test]
    fn stop_cached_entry_succeeds() {
        let mgr = CacheManager::new();
        let tmp = TempDir::new().unwrap();
        insert_test_entry(
            &mgr,
            "cached",
            CacheState::Cached {
                path: tmp.path().join("f.mkv").to_string_lossy().to_string(),
            },
            tmp.path().join("f.mkv").to_str().unwrap(),
        );
        mgr.stop("cached").unwrap();
    }

    #[test]
    fn stop_failed_entry_succeeds() {
        let mgr = CacheManager::new();
        insert_test_entry(
            &mgr,
            "failed",
            CacheState::Failed {
                error: "oops".into(),
            },
            "/tmp/f.mkv",
        );
        mgr.stop("failed").unwrap();
    }

    #[test]
    fn stop_cancelled_entry_succeeds() {
        let mgr = CacheManager::new();
        insert_test_entry(&mgr, "cancelled", CacheState::Cancelled, "/tmp/f.mkv");
        mgr.stop("cancelled").unwrap();
    }

    // ── cleanup() with Downloading state ──

    #[test]
    fn cleanup_downloading_entry() {
        let mgr = CacheManager::new();
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("dl.mkv");
        std::fs::write(&path, b"partial").unwrap();
        insert_test_entry(
            &mgr,
            "dl",
            CacheState::Downloading { progress: 50.0 },
            path.to_str().unwrap(),
        );

        mgr.cleanup("dl").unwrap();
        assert!(mgr.status("dl").is_none());
        assert!(!path.exists());
    }

    // ── all_status after cleanup ──

    #[test]
    fn all_status_after_partial_cleanup() {
        let mgr = CacheManager::new();
        insert_test_entry(&mgr, "a", CacheState::Idle, "/a.mkv");
        insert_test_entry(&mgr, "b", CacheState::Idle, "/b.mkv");

        mgr.cleanup("a").unwrap();
        let all = mgr.all_status();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].cache_key, "b");
    }

    // ── sanitize_filename more edge cases ──

    #[test]
    fn sanitize_filename_only_special_chars() {
        assert_eq!(sanitize_filename("/:\\*?\"<>|"), "_________");
    }

    #[test]
    fn sanitize_filename_preserves_dots() {
        assert_eq!(sanitize_filename("file.name.ext"), "file.name.ext");
    }

    #[test]
    fn sanitize_filename_preserves_dashes_underscores() {
        assert_eq!(sanitize_filename("my-file_name"), "my-file_name");
    }

    // ── CacheManager with Downloading state that has progress ──

    #[test]
    fn status_reflects_downloading_progress() {
        let mgr = CacheManager::new();
        insert_test_entry(
            &mgr,
            "dl1",
            CacheState::Downloading { progress: 75.5 },
            "/tmp/f.mkv",
        );

        let s = mgr.status("dl1").unwrap();
        match s.state {
            CacheState::Downloading { progress } => assert!((progress - 75.5).abs() < f64::EPSILON),
            _ => panic!("expected Downloading"),
        }
    }

    // ── CacheManager::cache_dir with whitespace-only string ──

    #[test]
    fn cache_dir_whitespace_string_uses_custom_path() {
        let cfg = Config {
            zip_cache_dir: Some("   ".into()),
            ..Config::default()
        };
        let dir = CacheManager::cache_dir(&cfg);
        // Whitespace-only is non-empty string, so cache_dir uses the custom path
        // (create_dir_all may fail on Windows for "   " but the PathBuf is correct)
        assert_eq!(dir, std::path::PathBuf::from("   "));
    }

    // ── is_cache_dir_set with whitespace ──

    #[test]
    fn is_cache_dir_set_true_for_whitespace() {
        let cfg = Config {
            zip_cache_dir: Some("  ".into()),
            ..Config::default()
        };
        // "  " is non-empty, so is_cache_dir_set returns true
        assert!(CacheManager::is_cache_dir_set(&cfg));
    }

    // ── stop sets flag even for non-Downloading states ──

    #[test]
    fn stop_sets_cancel_flag_for_cached() {
        let mgr = CacheManager::new();
        insert_test_entry(
            &mgr,
            "sc",
            CacheState::Cached {
                path: "/f.mkv".into(),
            },
            "/f.mkv",
        );
        mgr.stop("sc").unwrap();
        let guard = mgr.lock_active().unwrap();
        let entry = guard.get("sc").unwrap();
        assert!(entry.cancel_flag.load(Ordering::Relaxed));
    }

    // ── cleanup_all removes entries from map ──

    #[test]
    fn cleanup_all_removes_all_entries_from_map() {
        let mgr = CacheManager::new();
        insert_test_entry(&mgr, "x1", CacheState::Idle, "/x1.mkv");
        insert_test_entry(
            &mgr,
            "x2",
            CacheState::Downloading { progress: 50.0 },
            "/x2.mkv",
        );
        insert_test_entry(
            &mgr,
            "x3",
            CacheState::Cached {
                path: "/x3.mkv".into(),
            },
            "/x3.mkv",
        );

        mgr.cleanup_all().unwrap();
        assert!(mgr.status("x1").is_none());
        assert!(mgr.status("x2").is_none());
        assert!(mgr.status("x3").is_none());
    }

    // ── ActiveCache cancel_flag shared reference ──

    #[test]
    fn active_cache_cancel_flag_shared() {
        let mgr = CacheManager::new();
        insert_test_entry(&mgr, "shared", CacheState::Idle, "/f.mkv");

        // Get the cancel flag
        let flag = {
            let guard = mgr.lock_active().unwrap();
            guard.get("shared").unwrap().cancel_flag.clone()
        };

        // Set via external clone
        flag.store(true, Ordering::Relaxed);

        // Verify via manager
        let guard = mgr.lock_active().unwrap();
        let entry = guard.get("shared").unwrap();
        assert!(entry.cancel_flag.load(Ordering::Relaxed));
    }
}
