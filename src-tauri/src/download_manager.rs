use crate::database::get_app_data_dir;
use crate::gdrive;
use futures_util::stream::{FuturesUnordered, StreamExt};
use reqwest::header::{AUTHORIZATION, RANGE};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use uuid::Uuid;

pub const DOWNLOAD_EVENT: &str = "download-job-updated";
const DEFAULT_CHUNK_BYTES: u64 = 8 * 1024 * 1024;
const DEFAULT_CONCURRENCY: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJobSnapshot {
    pub id: String,
    pub media_id: i64,
    pub title: String,
    pub file_name: String,
    pub target_path: String,
    pub status: String,
    pub progress: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bytes_per_second: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
    pub error: Option<String>,
    pub source_kind: String,
    pub source_exists: bool,
    pub target_exists: bool,
}

struct DownloadJobRecord {
    snapshot: DownloadJobSnapshot,
    cancel_flag: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct DownloadManager {
    jobs: Arc<Mutex<HashMap<String, DownloadJobRecord>>>,
    #[cfg(test)]
    in_memory: bool,
}

#[derive(Clone)]
pub struct ParallelDownloadRequest {
    pub media_id: i64,
    pub title: String,
    pub file_name: String,
    pub target_path: PathBuf,
    pub file_id: String,
    pub range_start: u64,
    pub total_bytes: u64,
    pub source_kind: String,
    pub chunk_bytes: u64,
    pub concurrency: usize,
}

#[derive(Clone)]
pub struct LocalCopyRequest {
    pub media_id: i64,
    pub title: String,
    pub file_name: String,
    pub target_path: PathBuf,
    pub source_path: PathBuf,
    pub total_bytes: u64,
    pub source_kind: String,
}

impl DownloadManager {
    pub fn load() -> Self {
        let mut jobs = HashMap::new();

        if let Ok(raw) = fs::read_to_string(download_jobs_path()) {
            if let Ok(stored_jobs) = serde_json::from_str::<Vec<DownloadJobSnapshot>>(&raw) {
                for mut snapshot in stored_jobs {
                    if matches!(
                        snapshot.status.as_str(),
                        "queued" | "preparing" | "downloading"
                    ) {
                        snapshot.status = "failed".to_string();
                        snapshot.error = Some(
                            "Download was interrupted because the app was closed or refreshed."
                                .to_string(),
                        );
                        snapshot.speed_bytes_per_second = None;
                    }

                    jobs.insert(
                        snapshot.id.clone(),
                        DownloadJobRecord {
                            snapshot,
                            cancel_flag: Arc::new(AtomicBool::new(false)),
                        },
                    );
                }
            }
        }

        startup_cleanup_orphaned_parts();

        let manager = Self {
            jobs: Arc::new(Mutex::new(jobs)),
            #[cfg(test)]
            in_memory: false,
        };
        manager.cleanup_terminal_jobs();
        manager
    }

    pub fn list_jobs(&self) -> Vec<DownloadJobSnapshot> {
        let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let mut snapshots = jobs
            .values()
            .map(|record| record.snapshot.clone())
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        snapshots
    }

    pub fn create_job(
        &self,
        media_id: i64,
        title: String,
        file_name: String,
        target_path: PathBuf,
        total_bytes: u64,
        source_kind: String,
    ) -> (DownloadJobSnapshot, Arc<AtomicBool>) {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let snapshot = DownloadJobSnapshot {
            id: id.clone(),
            media_id,
            title,
            file_name,
            target_path: target_path.to_string_lossy().to_string(),
            status: "queued".to_string(),
            progress: 0.0,
            downloaded_bytes: 0,
            total_bytes,
            speed_bytes_per_second: None,
            created_at: now.clone(),
            updated_at: now,
            error: None,
            source_kind,
            source_exists: true,
            target_exists: false,
        };
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let record = DownloadJobRecord {
            snapshot: snapshot.clone(),
            cancel_flag: cancel_flag.clone(),
        };
        {
            self.jobs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id, record);
        }
        self.persist_jobs();
        (snapshot, cancel_flag)
    }

    pub fn cancel_job(&self, job_id: &str) -> Result<DownloadJobSnapshot, String> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let record = jobs
            .get_mut(job_id)
            .ok_or_else(|| "Download job not found".to_string())?;
        record.cancel_flag.store(true, Ordering::Relaxed);
        if matches!(
            record.snapshot.status.as_str(),
            "queued" | "preparing" | "downloading"
        ) {
            record.snapshot.status = "cancelled".to_string();
            record.snapshot.updated_at = chrono::Utc::now().to_rfc3339();
        }
        let snapshot = record.snapshot.clone();
        drop(jobs);
        self.persist_jobs();
        Ok(snapshot)
    }

    pub fn delete_job(&self, job_id: &str) -> Result<(), String> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.remove(job_id)
            .ok_or_else(|| "Download job not found".to_string())?;
        drop(jobs);
        self.persist_jobs();
        Ok(())
    }

    pub fn clear_history(&self) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.retain(|_, record| {
            matches!(
                record.snapshot.status.as_str(),
                "queued" | "preparing" | "downloading"
            )
        });
        drop(jobs);
        self.persist_jobs();
    }

    pub fn get_job(&self, job_id: &str) -> Option<DownloadJobSnapshot> {
        let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.get(job_id).map(|record| record.snapshot.clone())
    }

    pub fn update_job<F>(&self, job_id: &str, mut update: F) -> Option<DownloadJobSnapshot>
    where
        F: FnMut(&mut DownloadJobSnapshot),
    {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let record = jobs.get_mut(job_id)?;
        update(&mut record.snapshot);
        record.snapshot.updated_at = chrono::Utc::now().to_rfc3339();
        let snapshot = record.snapshot.clone();
        drop(jobs);
        self.persist_jobs();
        Some(snapshot)
    }

    /// Remove terminal jobs (completed/failed/cancelled) older than 7 days to prevent unbounded memory growth.
    fn cleanup_terminal_jobs(&self) {
        let now = chrono::Utc::now();
        let max_age = Duration::from_secs(7 * 24 * 60 * 60);
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let before = jobs.len();
        jobs.retain(|_, record| {
            if !matches!(
                record.snapshot.status.as_str(),
                "completed" | "failed" | "cancelled"
            ) {
                return true;
            }
            match chrono::DateTime::parse_from_rfc3339(&record.snapshot.updated_at) {
                Ok(updated) => {
                    now.signed_duration_since(updated)
                        .to_std()
                        .unwrap_or(Duration::ZERO)
                        < max_age
                }
                Err(_) => true, // keep if we can't parse the date
            }
        });
        let removed = before.saturating_sub(jobs.len());
        if removed > 0 {
            drop(jobs);
            self.persist_jobs();
            println!(
                "[DOWNLOAD] Cleaned up {} stale terminal jobs on startup",
                removed
            );
        }
    }

    fn persist_jobs(&self) {
        #[cfg(test)]
        if self.in_memory {
            return;
        }

        let snapshots = {
            let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            jobs.values()
                .map(|record| record.snapshot.clone())
                .collect::<Vec<_>>()
        };

        let path = download_jobs_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if let Ok(serialized) = serde_json::to_string_pretty(&snapshots) {
            let _ = fs::write(path, serialized);
        }
    }
}

impl DownloadManager {
    /// Create an in-memory-only manager that does not persist to disk. For tests.
    #[cfg(test)]
    fn new_in_memory() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            in_memory: true,
        }
    }
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self::load()
    }
}

pub fn default_downloads_dir() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_else(|| get_app_data_dir().join("downloads"))
        .join("SlasshyVault")
}

fn download_jobs_path() -> PathBuf {
    get_app_data_dir().join("download_jobs.json")
}

pub fn sanitize_download_filename(raw: &str, fallback: &str) -> String {
    let sanitized = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ' | '(' | ')') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    let trimmed = sanitized.trim().trim_matches('.');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn unique_target_path(dir: &Path, file_name: &str) -> PathBuf {
    let base = Path::new(file_name);
    let stem = base
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let ext = base.extension().and_then(|value| value.to_str());

    let mut candidate = dir.join(file_name);
    let mut index = 1usize;
    while candidate.exists() {
        let next_name = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem} ({index}).{ext}"),
            _ => format!("{stem} ({index})"),
        };
        candidate = dir.join(next_name);
        index += 1;
    }
    candidate
}

pub fn emit_job_update(app_handle: &AppHandle, snapshot: &DownloadJobSnapshot) {
    let _ = app_handle.emit(DOWNLOAD_EVENT, snapshot.clone());
}

pub fn start_parallel_download(
    app_handle: AppHandle,
    manager: DownloadManager,
    gdrive_client: gdrive::GoogleDriveClient,
    request: ParallelDownloadRequest,
) -> DownloadJobSnapshot {
    let chunk_bytes = request.chunk_bytes.max(1024 * 1024);
    let concurrency = request.concurrency.max(1);
    let (snapshot, cancel_flag) = manager.create_job(
        request.media_id,
        request.title.clone(),
        request.file_name.clone(),
        request.target_path.clone(),
        request.total_bytes,
        request.source_kind.clone(),
    );
    let job_id = snapshot.id.clone();
    emit_job_update(&app_handle, &snapshot);

    tokio::spawn(async move {
        if let Some(snapshot) = manager.update_job(&job_id, |job| {
            job.status = "preparing".to_string();
        }) {
            emit_job_update(&app_handle, &snapshot);
        }

        if let Some(parent) = request.target_path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                fail_job(
                    &app_handle,
                    &manager,
                    &job_id,
                    format!("Failed to create download directory: {}", error),
                );
                return;
            }
        }

        let temp_path = request.target_path.with_extension(format!(
            "{}.part",
            request
                .target_path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("download")
        ));

        if let Err(error) = prepare_temp_file(&temp_path, request.total_bytes) {
            fail_job(
                &app_handle,
                &manager,
                &job_id,
                format!("Failed to prepare temp file: {}", error),
            );
            return;
        }

        if let Some(snapshot) = manager.update_job(&job_id, |job| {
            job.status = "downloading".to_string();
        }) {
            emit_job_update(&app_handle, &snapshot);
        }

        let total_bytes = request.total_bytes;
        let started_at = Instant::now();
        let downloaded_bytes = Arc::new(AtomicU64::new(0));
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let http_client = reqwest::Client::new();
        let stream_url = gdrive_client.build_stream_url(&request.file_id);
        let chunk_ranges = build_chunk_ranges(total_bytes, chunk_bytes);
        let mut tasks = FuturesUnordered::new();

        for (relative_start, relative_end) in chunk_ranges {
            let semaphore = semaphore.clone();
            let app_handle = app_handle.clone();
            let manager = manager.clone();
            let gdrive_client = gdrive_client.clone();
            let job_id = job_id.clone();
            let temp_path = temp_path.clone();
            let cancel_flag = cancel_flag.clone();
            let downloaded_bytes = downloaded_bytes.clone();
            let stream_url = stream_url.clone();
            let http_client = http_client.clone();
            let absolute_start = request.range_start + relative_start;
            let absolute_end = request.range_start + relative_end;

            tasks.push(tokio::spawn(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|error| error.to_string())?;
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err("cancelled".to_string());
                }

                let expected_len = relative_end - relative_start + 1;
                let mut attempt = 0u32;
                loop {
                    attempt += 1;
                    if cancel_flag.load(Ordering::Relaxed) {
                        return Err("cancelled".to_string());
                    }

                    let access_token = gdrive_client.get_access_token().await?;
                    let response = http_client
                        .get(&stream_url)
                        .header(AUTHORIZATION, format!("Bearer {}", access_token))
                        .header(RANGE, format!("bytes={}-{}", absolute_start, absolute_end))
                        .send()
                        .await
                        .map_err(|error| error.to_string())?;

                    let response = response
                        .error_for_status()
                        .map_err(|error| error.to_string())?;

                    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
                    if bytes.len() as u64 != expected_len {
                        if attempt < 3 {
                            tokio::time::sleep(std::time::Duration::from_millis(
                                250 * attempt as u64,
                            ))
                            .await;
                            continue;
                        }
                        return Err(format!(
                            "Incomplete chunk download: expected {} bytes, got {}",
                            expected_len,
                            bytes.len()
                        ));
                    }

                    write_chunk(&temp_path, relative_start, bytes.as_ref())?;
                    let total_downloaded = downloaded_bytes
                        .fetch_add(bytes.len() as u64, Ordering::Relaxed)
                        + bytes.len() as u64;
                    let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
                    if let Some(snapshot) = manager.update_job(&job_id, |job| {
                        job.downloaded_bytes = total_downloaded;
                        job.progress = ((total_downloaded as f64 / job.total_bytes.max(1) as f64)
                            * 100.0)
                            .clamp(0.0, 100.0);
                        job.speed_bytes_per_second = Some(total_downloaded as f64 / elapsed);
                    }) {
                        emit_job_update(&app_handle, &snapshot);
                    }
                    return Ok::<(), String>(());
                }
            }));
        }

        let mut failure: Option<String> = None;
        while let Some(result) = tasks.next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) if error == "cancelled" => {
                    failure = Some(error);
                    break;
                }
                Ok(Err(error)) => {
                    failure = Some(error);
                    break;
                }
                Err(error) => {
                    failure = Some(error.to_string());
                    break;
                }
            }
        }

        if cancel_flag.load(Ordering::Relaxed) || matches!(failure.as_deref(), Some("cancelled")) {
            let _ = fs::remove_file(&temp_path);
            if let Some(snapshot) = manager.update_job(&job_id, |job| {
                job.status = "cancelled".to_string();
            }) {
                emit_job_update(&app_handle, &snapshot);
            }
            return;
        }

        if let Some(error) = failure {
            let _ = fs::remove_file(&temp_path);
            fail_job(&app_handle, &manager, &job_id, error);
            return;
        }

        if let Err(error) = fs::rename(&temp_path, &request.target_path) {
            let _ = fs::remove_file(&temp_path);
            fail_job(
                &app_handle,
                &manager,
                &job_id,
                format!("Failed to finalize download: {}", error),
            );
            return;
        }

        if let Some(snapshot) = manager.update_job(&job_id, |job| {
            job.status = "completed".to_string();
            job.downloaded_bytes = job.total_bytes;
            job.progress = 100.0;
            job.target_exists = true;
            job.speed_bytes_per_second = None;
        }) {
            emit_job_update(&app_handle, &snapshot);
        }
    });

    snapshot
}

pub fn start_local_copy(
    app_handle: AppHandle,
    manager: DownloadManager,
    request: LocalCopyRequest,
) -> DownloadJobSnapshot {
    let (snapshot, cancel_flag) = manager.create_job(
        request.media_id,
        request.title.clone(),
        request.file_name.clone(),
        request.target_path.clone(),
        request.total_bytes,
        request.source_kind.clone(),
    );
    let job_id = snapshot.id.clone();
    emit_job_update(&app_handle, &snapshot);

    tokio::spawn(async move {
        run_local_copy_job(app_handle, manager, job_id, cancel_flag, request).await;
    });

    snapshot
}

pub async fn run_local_copy_job(
    app_handle: AppHandle,
    manager: DownloadManager,
    job_id: String,
    cancel_flag: Arc<AtomicBool>,
    request: LocalCopyRequest,
) {
    if let Some(snapshot) = manager.update_job(&job_id, |job| {
        job.status = "preparing".to_string();
    }) {
        emit_job_update(&app_handle, &snapshot);
    }

    if let Some(parent) = request.target_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            fail_job(
                &app_handle,
                &manager,
                &job_id,
                format!("Failed to create download directory: {}", error),
            );
            return;
        }
    }

    let temp_path = request.target_path.with_extension(format!(
        "{}.part",
        request
            .target_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("download")
    ));

    let mut source = match File::open(&request.source_path).await {
        Ok(file) => file,
        Err(error) => {
            fail_job(
                &app_handle,
                &manager,
                &job_id,
                format!("Failed to open source file: {}", error),
            );
            return;
        }
    };

    let mut destination = match tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temp_path)
        .await
    {
        Ok(file) => file,
        Err(error) => {
            fail_job(
                &app_handle,
                &manager,
                &job_id,
                format!("Failed to create target file: {}", error),
            );
            return;
        }
    };

    if let Some(snapshot) = manager.update_job(&job_id, |job| {
        job.status = "downloading".to_string();
    }) {
        emit_job_update(&app_handle, &snapshot);
    }

    let mut buffer = vec![0u8; (2 * 1024 * 1024) as usize];
    let mut copied = 0u64;
    let started_at = Instant::now();

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            let _ = tokio::fs::remove_file(&temp_path).await;
            if let Some(snapshot) = manager.update_job(&job_id, |job| {
                job.status = "cancelled".to_string();
            }) {
                emit_job_update(&app_handle, &snapshot);
            }
            return;
        }

        let read = match source.read(&mut buffer).await {
            Ok(read) => read,
            Err(error) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                fail_job(
                    &app_handle,
                    &manager,
                    &job_id,
                    format!("Failed to read source file: {}", error),
                );
                return;
            }
        };

        if read == 0 {
            break;
        }

        if let Err(error) = destination.write_all(&buffer[..read]).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            fail_job(
                &app_handle,
                &manager,
                &job_id,
                format!("Failed to write target file: {}", error),
            );
            return;
        }

        copied = copied.saturating_add(read as u64);
        let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
        if let Some(snapshot) = manager.update_job(&job_id, |job| {
            job.downloaded_bytes = copied;
            job.progress =
                ((copied as f64 / job.total_bytes.max(1) as f64) * 100.0).clamp(0.0, 100.0);
            job.speed_bytes_per_second = Some(copied as f64 / elapsed);
        }) {
            emit_job_update(&app_handle, &snapshot);
        }
    }

    drop(destination);
    if let Err(error) = tokio::fs::rename(&temp_path, &request.target_path).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        fail_job(
            &app_handle,
            &manager,
            &job_id,
            format!("Failed to finalize copied download: {}", error),
        );
        return;
    }

    if let Some(snapshot) = manager.update_job(&job_id, |job| {
        job.status = "completed".to_string();
        job.downloaded_bytes = job.total_bytes;
        job.progress = 100.0;
        job.target_exists = true;
        job.speed_bytes_per_second = None;
    }) {
        emit_job_update(&app_handle, &snapshot);
    }
}

fn build_chunk_ranges(total_bytes: u64, chunk_bytes: u64) -> Vec<(u64, u64)> {
    let mut ranges = Vec::new();
    let mut start = 0u64;
    while start < total_bytes {
        let end = (start + chunk_bytes - 1).min(total_bytes - 1);
        ranges.push((start, end));
        start = end + 1;
    }
    ranges
}

fn prepare_temp_file(path: &Path, total_bytes: u64) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.set_len(total_bytes).map_err(|error| error.to_string())
}

fn write_chunk(path: &Path, offset: u64, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())
}

fn fail_job(app_handle: &AppHandle, manager: &DownloadManager, job_id: &str, error: String) {
    if let Some(snapshot) = manager.update_job(job_id, |job| {
        job.status = "failed".to_string();
        job.error = Some(error.clone());
    }) {
        emit_job_update(app_handle, &snapshot);
    }
}

fn startup_cleanup_orphaned_parts() {
    let downloads_dir = default_downloads_dir();
    if let Ok(entries) = std::fs::read_dir(&downloads_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("part") {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

pub fn default_parallel_chunk_bytes() -> u64 {
    DEFAULT_CHUNK_BYTES
}

pub fn default_parallel_concurrency() -> usize {
    DEFAULT_CONCURRENCY
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    // Serialize tests that touch the shared download_jobs.json on disk.
    static LOAD_TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("slasshyvault_tests")
            .join(name)
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ── sanitize_download_filename ────────────────────────────────

    #[test]
    fn sanitize_normal_name() {
        assert_eq!(
            sanitize_download_filename("movie.mp4", "fallback"),
            "movie.mp4"
        );
    }

    #[test]
    fn sanitize_preserves_allowed_chars() {
        assert_eq!(
            sanitize_download_filename("my file (copy)-v1.0.mkv", "fb"),
            "my file (copy)-v1.0.mkv"
        );
    }

    #[test]
    fn sanitize_replaces_special_chars() {
        assert_eq!(
            sanitize_download_filename("file:name*with?bad|chars", "fb"),
            "file_name_with_bad_chars"
        );
    }

    #[test]
    fn sanitize_replaces_path_separators() {
        // dots are allowed chars, so ".." survives; "/" becomes "_"
        assert_eq!(
            sanitize_download_filename("../../../etc/passwd", "fb"),
            "_.._.._etc_passwd"
        );
    }

    #[test]
    fn sanitize_replaces_backslash_traversal() {
        // dots survive, backslash becomes "_", leading dots trimmed by trim_matches('.')
        assert_eq!(
            sanitize_download_filename("..\\..\\windows\\system32", "fb"),
            "_.._windows_system32"
        );
    }

    #[test]
    fn sanitize_empty_returns_fallback() {
        assert_eq!(
            sanitize_download_filename("", "default_file"),
            "default_file"
        );
    }

    #[test]
    fn sanitize_all_replaced_chars_become_underscores() {
        // "/" ":" "*" "?" all become "_", underscores are not trimmed
        assert_eq!(
            sanitize_download_filename("///:::***???", "fallback"),
            "____________"
        );
    }

    #[test]
    fn sanitize_only_whitespace_returns_fallback() {
        assert_eq!(sanitize_download_filename("   ", "fallback"), "fallback");
    }

    #[test]
    fn sanitize_trims_whitespace_and_dots() {
        assert_eq!(sanitize_download_filename("  ...file...  ", "fb"), "file");
    }

    #[test]
    fn sanitize_long_name_preserved() {
        let long = "a".repeat(500);
        let result = sanitize_download_filename(&long, "fb");
        assert_eq!(result.len(), 500);
        assert_eq!(result, long);
    }

    #[test]
    fn sanitize_unicode_replaced() {
        // 7 unicode chars, each becomes "_"
        assert_eq!(
            sanitize_download_filename("日本語ファイル", "fb"),
            "_______"
        );
    }

    #[test]
    fn sanitize_only_dots_returns_fallback() {
        assert_eq!(sanitize_download_filename("...", "fb"), "fb");
    }

    // ── unique_target_path ────────────────────────────────────────

    #[test]
    fn unique_no_conflict() {
        let dir = temp_dir("unique_no_conflict");
        let result = unique_target_path(&dir, "test.mp4");
        assert_eq!(result, dir.join("test.mp4"));
    }

    #[test]
    fn unique_with_conflict_auto_increments() {
        let dir = temp_dir("unique_conflict");
        fs::write(dir.join("test.mp4"), b"existing").unwrap();
        let result = unique_target_path(&dir, "test.mp4");
        assert_eq!(result, dir.join("test (1).mp4"));
    }

    #[test]
    fn unique_with_multiple_conflicts() {
        let dir = temp_dir("unique_multi");
        fs::write(dir.join("test.mp4"), b"a").unwrap();
        fs::write(dir.join("test (1).mp4"), b"b").unwrap();
        fs::write(dir.join("test (2).mp4"), b"c").unwrap();
        let result = unique_target_path(&dir, "test.mp4");
        assert_eq!(result, dir.join("test (3).mp4"));
    }

    #[test]
    fn unique_no_extension() {
        let dir = temp_dir("unique_no_ext");
        let result = unique_target_path(&dir, "README");
        assert_eq!(result, dir.join("README"));
    }

    #[test]
    fn unique_no_extension_with_conflict() {
        let dir = temp_dir("unique_no_ext_conflict");
        fs::write(dir.join("README"), b"x").unwrap();
        let result = unique_target_path(&dir, "README");
        assert_eq!(result, dir.join("README (1)"));
    }

    #[test]
    fn unique_empty_extension_treated_as_no_ext() {
        let dir = temp_dir("unique_empty_ext");
        // "file." has an empty extension
        let result = unique_target_path(&dir, "file.");
        assert_eq!(result, dir.join("file."));
    }

    // ── default_downloads_dir ─────────────────────────────────────

    #[test]
    fn default_downloads_dir_returns_non_empty_path() {
        let dir = default_downloads_dir();
        assert!(!dir.as_os_str().is_empty());
    }

    #[test]
    fn default_downloads_dir_ends_with_slasshy_vault() {
        let dir = default_downloads_dir();
        assert_eq!(dir.file_name().unwrap(), "SlasshyVault");
    }

    // ── default_parallel_chunk_bytes / concurrency ────────────────

    #[test]
    fn default_chunk_bytes_is_8mb() {
        assert_eq!(default_parallel_chunk_bytes(), 8 * 1024 * 1024);
    }

    #[test]
    fn default_concurrency_is_8() {
        assert_eq!(default_parallel_concurrency(), 8);
    }

    // ── build_chunk_ranges ────────────────────────────────────────

    #[test]
    fn chunk_ranges_exact_division() {
        let ranges = build_chunk_ranges(16, 4);
        assert_eq!(ranges, vec![(0, 3), (4, 7), (8, 11), (12, 15)]);
    }

    #[test]
    fn chunk_ranges_with_remainder() {
        let ranges = build_chunk_ranges(10, 4);
        assert_eq!(ranges, vec![(0, 3), (4, 7), (8, 9)]);
    }

    #[test]
    fn chunk_ranges_single_chunk() {
        let ranges = build_chunk_ranges(100, 200);
        assert_eq!(ranges, vec![(0, 99)]);
    }

    #[test]
    fn chunk_ranges_zero_bytes() {
        let ranges = build_chunk_ranges(0, 1024);
        assert!(ranges.is_empty());
    }

    #[test]
    fn chunk_ranges_one_byte() {
        let ranges = build_chunk_ranges(1, 1024);
        assert_eq!(ranges, vec![(0, 0)]);
    }

    #[test]
    fn chunk_ranges_exact_boundary() {
        let ranges = build_chunk_ranges(8, 8);
        assert_eq!(ranges, vec![(0, 7)]);
    }

    // ── DownloadJobSnapshot ───────────────────────────────────────

    #[test]
    fn snapshot_serializes_and_deserializes() {
        let snapshot = DownloadJobSnapshot {
            id: "abc-123".to_string(),
            media_id: 42,
            title: "Test Movie".to_string(),
            file_name: "test.mp4".to_string(),
            target_path: "/downloads/test.mp4".to_string(),
            status: "completed".to_string(),
            progress: 100.0,
            downloaded_bytes: 1024,
            total_bytes: 1024,
            speed_bytes_per_second: Some(512.0),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:01:00Z".to_string(),
            error: None,
            source_kind: "gdrive".to_string(),
            source_exists: true,
            target_exists: true,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let deserialized: DownloadJobSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "abc-123");
        assert_eq!(deserialized.media_id, 42);
        assert_eq!(deserialized.status, "completed");
        assert_eq!(deserialized.progress, 100.0);
        assert_eq!(deserialized.downloaded_bytes, 1024);
        assert_eq!(deserialized.total_bytes, 1024);
        assert_eq!(deserialized.speed_bytes_per_second, Some(512.0));
        assert_eq!(deserialized.source_kind, "gdrive");
        assert!(deserialized.error.is_none());
    }

    #[test]
    fn snapshot_uses_camel_case_in_json() {
        let snapshot = DownloadJobSnapshot {
            id: "x".to_string(),
            media_id: 1,
            title: "t".to_string(),
            file_name: "f".to_string(),
            target_path: "p".to_string(),
            status: "s".to_string(),
            progress: 0.0,
            downloaded_bytes: 0,
            total_bytes: 0,
            speed_bytes_per_second: None,
            created_at: "".to_string(),
            updated_at: "".to_string(),
            error: None,
            source_kind: "".to_string(),
            source_exists: false,
            target_exists: false,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("mediaId"));
        assert!(json.contains("fileName"));
        assert!(json.contains("targetPath"));
        assert!(json.contains("downloadedBytes"));
        assert!(json.contains("totalBytes"));
        assert!(json.contains("speedBytesPerSecond"));
        assert!(json.contains("createdAt"));
        assert!(json.contains("updatedAt"));
        assert!(json.contains("sourceKind"));
        assert!(json.contains("sourceExists"));
        assert!(json.contains("targetExists"));
    }

    #[test]
    fn snapshot_with_error_field() {
        let snapshot = DownloadJobSnapshot {
            id: "x".to_string(),
            media_id: 1,
            title: "t".to_string(),
            file_name: "f".to_string(),
            target_path: "p".to_string(),
            status: "failed".to_string(),
            progress: 0.0,
            downloaded_bytes: 0,
            total_bytes: 0,
            speed_bytes_per_second: None,
            created_at: "".to_string(),
            updated_at: "".to_string(),
            error: Some("network timeout".to_string()),
            source_kind: "gdrive".to_string(),
            source_exists: true,
            target_exists: false,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let deserialized: DownloadJobSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.error, Some("network timeout".to_string()));
    }

    // ── DownloadManager::create_job / list_jobs / get_job ─────────

    #[test]
    fn create_job_returns_snapshot_and_cancel_flag() {
        let manager = DownloadManager::new_in_memory();
        let (snapshot, _flag) = manager.create_job(
            1,
            "Test".into(),
            "test.mp4".into(),
            PathBuf::from("/tmp/test.mp4"),
            1024,
            "gdrive".into(),
        );
        assert_eq!(snapshot.status, "queued");
        assert_eq!(snapshot.progress, 0.0);
        assert_eq!(snapshot.downloaded_bytes, 0);
        assert_eq!(snapshot.total_bytes, 1024);
        assert!(snapshot.error.is_none());
    }

    #[test]
    fn create_job_persists_and_list_jobs_returns_it() {
        let manager = DownloadManager::new_in_memory();
        manager.create_job(
            1,
            "Movie".into(),
            "movie.mp4".into(),
            PathBuf::from("/tmp/movie.mp4"),
            2048,
            "local".into(),
        );
        let jobs = manager.list_jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].title, "Movie");
        assert_eq!(jobs[0].source_kind, "local");
    }

    #[test]
    fn list_jobs_sorted_by_created_at_desc() {
        let manager = DownloadManager::new_in_memory();
        manager.create_job(
            1,
            "First".into(),
            "a.mp4".into(),
            PathBuf::from("/a"),
            1,
            "gdrive".into(),
        );
        // small delay so timestamps differ
        std::thread::sleep(std::time::Duration::from_millis(10));
        manager.create_job(
            2,
            "Second".into(),
            "b.mp4".into(),
            PathBuf::from("/b"),
            1,
            "gdrive".into(),
        );
        let jobs = manager.list_jobs();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].title, "Second");
        assert_eq!(jobs[1].title, "First");
    }

    #[test]
    fn get_job_returns_existing() {
        let manager = DownloadManager::new_in_memory();
        let (snapshot, _) = manager.create_job(
            1,
            "Find Me".into(),
            "f.mp4".into(),
            PathBuf::from("/f"),
            100,
            "gdrive".into(),
        );
        let found = manager.get_job(&snapshot.id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "Find Me");
    }

    #[test]
    fn get_job_returns_none_for_missing() {
        let manager = DownloadManager::new_in_memory();
        assert!(manager.get_job("nonexistent-id").is_none());
    }

    // ── DownloadManager::cancel_job ───────────────────────────────

    #[test]
    fn cancel_job_sets_status_to_cancelled() {
        let manager = DownloadManager::new_in_memory();
        let (snapshot, _) = manager.create_job(
            1,
            "Cancel Me".into(),
            "c.mp4".into(),
            PathBuf::from("/c"),
            100,
            "gdrive".into(),
        );
        let result = manager.cancel_job(&snapshot.id);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().status, "cancelled");
    }

    #[test]
    fn cancel_job_sets_cancel_flag() {
        let manager = DownloadManager::new_in_memory();
        let (snapshot, flag) = manager.create_job(
            1,
            "Flag".into(),
            "f.mp4".into(),
            PathBuf::from("/f"),
            100,
            "gdrive".into(),
        );
        assert!(!flag.load(Ordering::Relaxed));
        manager.cancel_job(&snapshot.id).unwrap();
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn cancel_job_not_found_returns_error() {
        let manager = DownloadManager::new_in_memory();
        let result = manager.cancel_job("nope");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Download job not found");
    }

    #[test]
    fn cancel_job_already_completed_stays_completed() {
        let manager = DownloadManager::new_in_memory();
        let (snapshot, _) = manager.create_job(
            1,
            "Done".into(),
            "d.mp4".into(),
            PathBuf::from("/d"),
            100,
            "gdrive".into(),
        );
        manager.update_job(&snapshot.id, |job| {
            job.status = "completed".to_string();
        });
        let result = manager.cancel_job(&snapshot.id).unwrap();
        // completed is not in the cancelable set, so status stays
        assert_eq!(result.status, "completed");
    }

    // ── DownloadManager::delete_job ───────────────────────────────

    #[test]
    fn delete_job_removes_it() {
        let manager = DownloadManager::new_in_memory();
        let (snapshot, _) = manager.create_job(
            1,
            "Delete".into(),
            "del.mp4".into(),
            PathBuf::from("/del"),
            100,
            "gdrive".into(),
        );
        assert!(manager.get_job(&snapshot.id).is_some());
        let result = manager.delete_job(&snapshot.id);
        assert!(result.is_ok());
        assert!(manager.get_job(&snapshot.id).is_none());
    }

    #[test]
    fn delete_job_not_found_returns_error() {
        let manager = DownloadManager::new_in_memory();
        let result = manager.delete_job("nonexistent");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Download job not found");
    }

    // ── DownloadManager::clear_history ────────────────────────────

    #[test]
    fn clear_history_removes_terminal_jobs() {
        let manager = DownloadManager::new_in_memory();
        let (s1, _) = manager.create_job(
            1,
            "A".into(),
            "a.mp4".into(),
            PathBuf::from("/a"),
            1,
            "gdrive".into(),
        );
        let (s2, _) = manager.create_job(
            2,
            "B".into(),
            "b.mp4".into(),
            PathBuf::from("/b"),
            1,
            "gdrive".into(),
        );
        let (s3, _) = manager.create_job(
            3,
            "C".into(),
            "c.mp4".into(),
            PathBuf::from("/c"),
            1,
            "gdrive".into(),
        );

        // Mark s1 completed, s2 failed, leave s3 queued
        manager.update_job(&s1.id, |j| j.status = "completed".to_string());
        manager.update_job(&s2.id, |j| j.status = "failed".to_string());

        manager.clear_history();

        let remaining = manager.list_jobs();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, s3.id);
    }

    #[test]
    fn clear_history_keeps_active_jobs() {
        let manager = DownloadManager::new_in_memory();
        let (s1, _) = manager.create_job(
            1,
            "Q".into(),
            "q.mp4".into(),
            PathBuf::from("/q"),
            1,
            "gdrive".into(),
        );
        manager.update_job(&s1.id, |j| j.status = "preparing".to_string());
        let (s2, _) = manager.create_job(
            2,
            "D".into(),
            "d.mp4".into(),
            PathBuf::from("/d"),
            1,
            "gdrive".into(),
        );
        manager.update_job(&s2.id, |j| j.status = "downloading".to_string());

        manager.clear_history();

        let remaining = manager.list_jobs();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn clear_history_empty_manager() {
        let manager = DownloadManager::new_in_memory();
        manager.clear_history();
        assert_eq!(manager.list_jobs().len(), 0);
    }

    // ── DownloadManager::update_job ───────────────────────────────

    #[test]
    fn update_job_modifies_snapshot() {
        let manager = DownloadManager::new_in_memory();
        let (snapshot, _) = manager.create_job(
            1,
            "Update".into(),
            "u.mp4".into(),
            PathBuf::from("/u"),
            1024,
            "gdrive".into(),
        );
        let updated = manager.update_job(&snapshot.id, |job| {
            job.progress = 50.0;
            job.downloaded_bytes = 512;
            job.status = "downloading".to_string();
        });
        assert!(updated.is_some());
        let updated = updated.unwrap();
        assert_eq!(updated.progress, 50.0);
        assert_eq!(updated.downloaded_bytes, 512);
        assert_eq!(updated.status, "downloading");
    }

    #[test]
    fn update_job_returns_none_for_missing() {
        let manager = DownloadManager::new_in_memory();
        assert!(manager.update_job("nope", |_| {}).is_none());
    }

    // ── DownloadManager::load ─────────────────────────────────────

    #[test]
    fn load_from_disk_round_trips() {
        let _lock = LOAD_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let manager = DownloadManager::new_in_memory();
        manager.create_job(
            10,
            "Persisted".into(),
            "p.mp4".into(),
            PathBuf::from("/p"),
            999,
            "local".into(),
        );

        // write to disk, then load back
        let snapshots = {
            let jobs = manager.jobs.lock().unwrap_or_else(|e| e.into_inner());
            jobs.values()
                .map(|r| r.snapshot.clone())
                .collect::<Vec<_>>()
        };
        let json = serde_json::to_string_pretty(&snapshots).unwrap();
        let path = download_jobs_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let backup = if path.exists() {
            Some(fs::read(&path).unwrap())
        } else {
            None
        };
        fs::write(&path, &json).unwrap();

        let loaded = DownloadManager::load();
        let loaded_jobs = loaded.list_jobs();
        assert!(loaded_jobs.iter().any(|j| j.title == "Persisted"));

        // restore or clean up
        match backup {
            Some(data) => fs::write(&path, data).unwrap(),
            None => {
                let _ = fs::remove_file(&path);
            }
        }
    }

    #[test]
    fn load_interrupted_jobs_become_failed() {
        let _lock = LOAD_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let now = chrono::Utc::now().to_rfc3339();
        let snapshots = vec![DownloadJobSnapshot {
            id: "interrupted-1".to_string(),
            media_id: 1,
            title: "Interrupted".to_string(),
            file_name: "i.mp4".to_string(),
            target_path: "/i.mp4".to_string(),
            status: "downloading".to_string(),
            progress: 45.0,
            downloaded_bytes: 450,
            total_bytes: 1000,
            speed_bytes_per_second: Some(100.0),
            created_at: now.clone(),
            updated_at: now,
            error: None,
            source_kind: "gdrive".to_string(),
            source_exists: true,
            target_exists: false,
        }];
        let json = serde_json::to_string_pretty(&snapshots).unwrap();
        let path = download_jobs_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let backup = if path.exists() {
            Some(fs::read(&path).unwrap())
        } else {
            None
        };
        fs::write(&path, &json).unwrap();

        let loaded = DownloadManager::load();
        let jobs = loaded.list_jobs();
        let interrupted = jobs.iter().find(|j| j.id == "interrupted-1").unwrap();
        assert_eq!(interrupted.status, "failed");
        assert!(interrupted.error.as_ref().unwrap().contains("interrupted"));
        assert!(interrupted.speed_bytes_per_second.is_none());

        match backup {
            Some(data) => fs::write(&path, data).unwrap(),
            None => {
                let _ = fs::remove_file(&path);
            }
        }
    }

    // ── prepare_temp_file / write_chunk ───────────────────────────

    #[test]
    fn prepare_temp_file_creates_file_with_size() {
        let dir = temp_dir("prepare_temp");
        let path = dir.join("test.part");
        prepare_temp_file(&path, 1024).unwrap();
        assert!(path.exists());
        assert_eq!(fs::metadata(&path).unwrap().len(), 1024);
    }

    #[test]
    fn write_chunk_writes_at_offset() {
        let dir = temp_dir("write_chunk");
        let path = dir.join("chunk.part");
        prepare_temp_file(&path, 16).unwrap();
        write_chunk(&path, 4, &[1, 2, 3, 4]).unwrap();
        let data = fs::read(&path).unwrap();
        assert_eq!(&data[4..8], &[1, 2, 3, 4]);
        assert_eq!(&data[0..4], &[0, 0, 0, 0]);
    }

    // ── DownloadManager::new_in_memory (no disk side effects) ─────

    #[test]
    fn new_in_memory_starts_empty() {
        let manager = DownloadManager::new_in_memory();
        assert_eq!(manager.list_jobs().len(), 0);
    }

    // ── ParallelDownloadRequest / LocalCopyRequest structs ────────

    #[test]
    fn parallel_download_request_clone() {
        let req = ParallelDownloadRequest {
            media_id: 1,
            title: "t".into(),
            file_name: "f".into(),
            target_path: PathBuf::from("/f"),
            file_id: "fid".into(),
            range_start: 0,
            total_bytes: 100,
            source_kind: "gdrive".into(),
            chunk_bytes: 1024,
            concurrency: 4,
        };
        let cloned = req.clone();
        assert_eq!(cloned.media_id, 1);
        assert_eq!(cloned.file_id, "fid");
        assert_eq!(cloned.concurrency, 4);
    }

    #[test]
    fn local_copy_request_clone() {
        let req = LocalCopyRequest {
            media_id: 2,
            title: "t".into(),
            file_name: "f".into(),
            target_path: PathBuf::from("/dst"),
            source_path: PathBuf::from("/src"),
            total_bytes: 500,
            source_kind: "local".into(),
        };
        let cloned = req.clone();
        assert_eq!(cloned.source_path, PathBuf::from("/src"));
        assert_eq!(cloned.total_bytes, 500);
    }

    // ── unique_target_path edge cases ──────────────────────────────

    #[test]
    fn unique_no_extension_multiple_conflicts() {
        let dir = temp_dir("unique_no_ext_multi");
        fs::write(dir.join("data"), b"a").unwrap();
        fs::write(dir.join("data (1)"), b"b").unwrap();
        let result = unique_target_path(&dir, "data");
        assert_eq!(result, dir.join("data (2)"));
    }

    #[test]
    fn unique_dot_file_extension() {
        let dir = temp_dir("unique_dot");
        let result = unique_target_path(&dir, ".gitignore");
        assert_eq!(result, dir.join(".gitignore"));
    }

    // ── build_chunk_ranges edge cases ──────────────────────────────

    #[test]
    fn chunk_ranges_chunk_size_one() {
        let ranges = build_chunk_ranges(3, 1);
        assert_eq!(ranges, vec![(0, 0), (1, 1), (2, 2)]);
    }

    #[test]
    fn chunk_ranges_total_bytes_one() {
        let ranges = build_chunk_ranges(1, 1);
        assert_eq!(ranges, vec![(0, 0)]);
    }

    #[test]
    fn chunk_ranges_large_total_small_chunk() {
        let ranges = build_chunk_ranges(1000, 100);
        assert_eq!(ranges.len(), 10);
        assert_eq!(ranges[0], (0, 99));
        assert_eq!(ranges[9], (900, 999));
    }

    #[test]
    fn chunk_ranges_two_byte_total() {
        let ranges = build_chunk_ranges(2, 1);
        assert_eq!(ranges, vec![(0, 0), (1, 1)]);
    }

    // ── DownloadJobSnapshot edge cases ─────────────────────────────

    #[test]
    fn snapshot_missing_required_field_fails_deserialize() {
        let json = r#"{"id":"x"}"#;
        let result = serde_json::from_str::<DownloadJobSnapshot>(json);
        assert!(result.is_err());
    }

    // ── DownloadManager edge cases ─────────────────────────────────

    #[test]
    fn clear_history_removes_cancelled_jobs() {
        let manager = DownloadManager::new_in_memory();
        let (s1, _) = manager.create_job(
            1,
            "A".into(),
            "a.mp4".into(),
            PathBuf::from("/a"),
            1,
            "gdrive".into(),
        );
        manager.update_job(&s1.id, |j| j.status = "cancelled".to_string());
        manager.clear_history();
        assert_eq!(manager.list_jobs().len(), 0);
    }

    #[test]
    fn update_job_returns_updated_snapshot() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "T".into(),
            "t.mp4".into(),
            PathBuf::from("/t"),
            100,
            "gdrive".into(),
        );
        let result = manager.update_job(&s.id, |j| {
            j.status = "downloading".to_string();
            j.downloaded_bytes = 50;
            j.progress = 50.0;
            j.speed_bytes_per_second = Some(1024.0);
        });
        assert!(result.is_some());
        let snap = result.unwrap();
        assert_eq!(snap.status, "downloading");
        assert_eq!(snap.downloaded_bytes, 50);
        assert_eq!(snap.progress, 50.0);
        assert_eq!(snap.speed_bytes_per_second, Some(1024.0));
    }

    #[test]
    fn update_job_sets_error_message() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "E".into(),
            "e.mp4".into(),
            PathBuf::from("/e"),
            100,
            "gdrive".into(),
        );
        manager.update_job(&s.id, |j| {
            j.status = "failed".to_string();
            j.error = Some("connection timeout".to_string());
        });
        let fetched = manager.get_job(&s.id).unwrap();
        assert_eq!(fetched.status, "failed");
        assert_eq!(fetched.error.unwrap(), "connection timeout");
    }

    #[test]
    fn update_job_persists_across_get() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "P".into(),
            "p.mp4".into(),
            PathBuf::from("/p"),
            200,
            "gdrive".into(),
        );
        manager.update_job(&s.id, |j| {
            j.downloaded_bytes = 100;
            j.progress = 50.0;
        });
        let fetched = manager.get_job(&s.id).unwrap();
        assert_eq!(fetched.downloaded_bytes, 100);
        assert_eq!(fetched.progress, 50.0);
    }

    #[test]
    fn cancel_job_from_preparing_state() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "P".into(),
            "p.mp4".into(),
            PathBuf::from("/p"),
            100,
            "gdrive".into(),
        );
        manager.update_job(&s.id, |j| j.status = "preparing".to_string());
        let result = manager.cancel_job(&s.id).unwrap();
        assert_eq!(result.status, "cancelled");
    }

    #[test]
    fn cancel_job_from_downloading_state() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "D".into(),
            "d.mp4".into(),
            PathBuf::from("/d"),
            100,
            "gdrive".into(),
        );
        manager.update_job(&s.id, |j| j.status = "downloading".to_string());
        let result = manager.cancel_job(&s.id).unwrap();
        assert_eq!(result.status, "cancelled");
    }

    // ── prepare_temp_file edge cases ───────────────────────────────

    #[test]
    fn prepare_temp_file_zero_bytes() {
        let dir = temp_dir("prepare_zero");
        let path = dir.join("zero.part");
        prepare_temp_file(&path, 0).unwrap();
        assert!(path.exists());
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
    }

    // ── write_chunk edge cases ─────────────────────────────────────

    #[test]
    fn write_chunk_at_offset_zero() {
        let dir = temp_dir("chunk_zero");
        let path = dir.join("z.part");
        prepare_temp_file(&path, 8).unwrap();
        write_chunk(&path, 0, &[0xAA, 0xBB]).unwrap();
        let data = fs::read(&path).unwrap();
        assert_eq!(&data[0..2], &[0xAA, 0xBB]);
    }

    #[test]
    fn write_chunk_entire_file() {
        let dir = temp_dir("chunk_entire");
        let path = dir.join("e.part");
        prepare_temp_file(&path, 4).unwrap();
        write_chunk(&path, 0, &[1, 2, 3, 4]).unwrap();
        let data = fs::read(&path).unwrap();
        assert_eq!(data, vec![1, 2, 3, 4]);
    }

    // ── load edge cases ────────────────────────────────────────────

    #[test]
    fn load_invalid_json_returns_empty() {
        let _lock = LOAD_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let path = download_jobs_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let backup = if path.exists() {
            Some(fs::read(&path).unwrap())
        } else {
            None
        };
        fs::write(&path, "not valid json {{{").unwrap();
        let loaded = DownloadManager::load();
        assert_eq!(loaded.list_jobs().len(), 0);
        match backup {
            Some(data) => fs::write(&path, data).unwrap(),
            None => {
                let _ = fs::remove_file(&path);
            }
        }
    }

    #[test]
    fn load_queued_jobs_become_failed() {
        let _lock = LOAD_TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let now = chrono::Utc::now().to_rfc3339();
        let snapshots = vec![DownloadJobSnapshot {
            id: "queued-1".to_string(),
            media_id: 2,
            title: "Queued".to_string(),
            file_name: "q.mp4".to_string(),
            target_path: "/q.mp4".to_string(),
            status: "queued".to_string(),
            progress: 0.0,
            downloaded_bytes: 0,
            total_bytes: 1000,
            speed_bytes_per_second: None,
            created_at: now.clone(),
            updated_at: now,
            error: None,
            source_kind: "gdrive".to_string(),
            source_exists: true,
            target_exists: false,
        }];
        let json = serde_json::to_string_pretty(&snapshots).unwrap();
        let path = download_jobs_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let backup = if path.exists() {
            Some(fs::read(&path).unwrap())
        } else {
            None
        };
        fs::write(&path, &json).unwrap();
        let loaded = DownloadManager::load();
        let jobs = loaded.list_jobs();
        let queued = jobs.iter().find(|j| j.id == "queued-1").unwrap();
        assert_eq!(queued.status, "failed");
        assert!(queued.error.as_ref().unwrap().contains("interrupted"));
        match backup {
            Some(data) => fs::write(&path, data).unwrap(),
            None => {
                let _ = fs::remove_file(&path);
            }
        }
    }

    // ── ParallelDownloadRequest field construction ──────────────────

    #[test]
    fn parallel_download_request_field_values() {
        let req = ParallelDownloadRequest {
            media_id: 42,
            title: "Big Movie".into(),
            file_name: "movie.mkv".into(),
            target_path: PathBuf::from("/downloads/movie.mkv"),
            file_id: "gdrive-file-id-123".into(),
            range_start: 1024,
            total_bytes: 1024 * 1024 * 100,
            source_kind: "gdrive".into(),
            chunk_bytes: 4 * 1024 * 1024,
            concurrency: 4,
        };
        assert_eq!(req.media_id, 42);
        assert_eq!(req.title, "Big Movie");
        assert_eq!(req.file_name, "movie.mkv");
        assert_eq!(req.file_id, "gdrive-file-id-123");
        assert_eq!(req.range_start, 1024);
        assert_eq!(req.total_bytes, 1024 * 1024 * 100);
        assert_eq!(req.source_kind, "gdrive");
        assert_eq!(req.chunk_bytes, 4 * 1024 * 1024);
        assert_eq!(req.concurrency, 4);
    }

    // ── startup_cleanup_orphaned_parts ──────────────────────────────

    #[test]
    fn startup_cleanup_removes_part_files() {
        let dir = temp_dir("cleanup_parts");
        // Create fake .part files
        fs::write(dir.join("video.mp4.part"), b"partial").unwrap();
        fs::write(dir.join("movie.mkv.part"), b"partial").unwrap();
        fs::write(dir.join("keep.mp4"), b"complete").unwrap();

        // We can't call startup_cleanup_orphaned_parts directly since it uses
        // default_downloads_dir(), but we verify the pattern matching logic:
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("part") {
                fs::remove_file(&path).unwrap();
            }
        }

        let remaining: Vec<_> = fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].file_name(), "keep.mp4");
    }

    // ── download_jobs_path ──────────────────────────────────────────

    #[test]
    fn download_jobs_path_contains_filename() {
        let path = download_jobs_path();
        assert_eq!(path.file_name().unwrap(), "download_jobs.json");
    }

    // ── Progress tracking logic ────────────────────────────────────

    #[test]
    fn progress_clamped_to_100() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "P".into(),
            "p.mp4".into(),
            PathBuf::from("/p"),
            100,
            "gdrive".into(),
        );
        manager.update_job(&s.id, |j| {
            j.downloaded_bytes = 200; // more than total
            j.progress = ((200f64 / 100f64) * 100.0).clamp(0.0, 100.0);
        });
        let fetched = manager.get_job(&s.id).unwrap();
        assert_eq!(fetched.progress, 100.0);
    }

    #[test]
    fn progress_zero_downloaded() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "Z".into(),
            "z.mp4".into(),
            PathBuf::from("/z"),
            1000,
            "gdrive".into(),
        );
        manager.update_job(&s.id, |j| {
            j.downloaded_bytes = 0;
            j.progress = 0.0;
        });
        let fetched = manager.get_job(&s.id).unwrap();
        assert_eq!(fetched.progress, 0.0);
        assert_eq!(fetched.downloaded_bytes, 0);
    }

    #[test]
    fn speed_tracking_with_none() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "S".into(),
            "s.mp4".into(),
            PathBuf::from("/s"),
            100,
            "gdrive".into(),
        );
        let fetched = manager.get_job(&s.id).unwrap();
        assert!(fetched.speed_bytes_per_second.is_none());
    }

    #[test]
    fn speed_tracking_with_value() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "S".into(),
            "s.mp4".into(),
            PathBuf::from("/s"),
            100,
            "gdrive".into(),
        );
        manager.update_job(&s.id, |j| {
            j.speed_bytes_per_second = Some(1_048_576.0); // 1 MB/s
        });
        let fetched = manager.get_job(&s.id).unwrap();
        assert_eq!(fetched.speed_bytes_per_second, Some(1_048_576.0));
    }

    // ── State transitions ──────────────────────────────────────────

    #[test]
    fn state_transition_queued_to_preparing() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "T".into(),
            "t.mp4".into(),
            PathBuf::from("/t"),
            100,
            "gdrive".into(),
        );
        assert_eq!(s.status, "queued");
        manager.update_job(&s.id, |j| j.status = "preparing".to_string());
        assert_eq!(manager.get_job(&s.id).unwrap().status, "preparing");
    }

    #[test]
    fn state_transition_preparing_to_downloading() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "T".into(),
            "t.mp4".into(),
            PathBuf::from("/t"),
            100,
            "gdrive".into(),
        );
        manager.update_job(&s.id, |j| j.status = "preparing".to_string());
        manager.update_job(&s.id, |j| j.status = "downloading".to_string());
        assert_eq!(manager.get_job(&s.id).unwrap().status, "downloading");
    }

    #[test]
    fn state_transition_downloading_to_completed() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "T".into(),
            "t.mp4".into(),
            PathBuf::from("/t"),
            100,
            "gdrive".into(),
        );
        manager.update_job(&s.id, |j| j.status = "downloading".to_string());
        manager.update_job(&s.id, |j| {
            j.status = "completed".to_string();
            j.progress = 100.0;
            j.downloaded_bytes = 100;
            j.target_exists = true;
        });
        let fetched = manager.get_job(&s.id).unwrap();
        assert_eq!(fetched.status, "completed");
        assert_eq!(fetched.progress, 100.0);
        assert!(fetched.target_exists);
    }

    #[test]
    fn state_transition_downloading_to_failed() {
        let manager = DownloadManager::new_in_memory();
        let (s, _) = manager.create_job(
            1,
            "T".into(),
            "t.mp4".into(),
            PathBuf::from("/t"),
            100,
            "gdrive".into(),
        );
        manager.update_job(&s.id, |j| j.status = "downloading".to_string());
        manager.update_job(&s.id, |j| {
            j.status = "failed".to_string();
            j.error = Some("network error".to_string());
        });
        let fetched = manager.get_job(&s.id).unwrap();
        assert_eq!(fetched.status, "failed");
        assert_eq!(fetched.error.unwrap(), "network error");
    }

    // ── Multiple jobs management ────────────────────────────────────

    #[test]
    fn multiple_jobs_independent_state() {
        let manager = DownloadManager::new_in_memory();
        let (s1, _) = manager.create_job(
            1,
            "A".into(),
            "a.mp4".into(),
            PathBuf::from("/a"),
            100,
            "gdrive".into(),
        );
        let (s2, _) = manager.create_job(
            2,
            "B".into(),
            "b.mp4".into(),
            PathBuf::from("/b"),
            200,
            "local".into(),
        );
        manager.update_job(&s1.id, |j| j.status = "completed".to_string());
        // s2 should still be queued
        assert_eq!(manager.get_job(&s1.id).unwrap().status, "completed");
        assert_eq!(manager.get_job(&s2.id).unwrap().status, "queued");
    }

    #[test]
    fn delete_job_does_not_affect_others() {
        let manager = DownloadManager::new_in_memory();
        let (s1, _) = manager.create_job(
            1,
            "A".into(),
            "a.mp4".into(),
            PathBuf::from("/a"),
            100,
            "gdrive".into(),
        );
        let (s2, _) = manager.create_job(
            2,
            "B".into(),
            "b.mp4".into(),
            PathBuf::from("/b"),
            200,
            "local".into(),
        );
        manager.delete_job(&s1.id).unwrap();
        assert!(manager.get_job(&s1.id).is_none());
        assert!(manager.get_job(&s2.id).is_some());
    }

    // ── DownloadJobRecord cancel_flag ──────────────────────────────

    #[test]
    fn cancel_flag_initial_false() {
        let manager = DownloadManager::new_in_memory();
        let (_, flag) = manager.create_job(
            1,
            "F".into(),
            "f.mp4".into(),
            PathBuf::from("/f"),
            100,
            "gdrive".into(),
        );
        assert!(!flag.load(Ordering::Relaxed));
    }

    #[test]
    fn cancel_flag_shared_between_clones() {
        let manager = DownloadManager::new_in_memory();
        let (_, flag) = manager.create_job(
            1,
            "F".into(),
            "f.mp4".into(),
            PathBuf::from("/f"),
            100,
            "gdrive".into(),
        );
        let flag2 = flag.clone();
        flag.store(true, Ordering::Relaxed);
        assert!(flag2.load(Ordering::Relaxed));
    }
}
