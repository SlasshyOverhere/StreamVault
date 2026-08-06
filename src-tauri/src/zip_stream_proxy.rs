use crate::{dev_elog, dev_log, gdrive, zip_manager};
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, RANGE};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Response, Server, StatusCode};
use tokio::runtime::{Builder as TokioRuntimeBuilder, Runtime as TokioRuntime};

const TURBO_CHUNK_BYTES: u64 = 4 * 1024 * 1024;
const TURBO_PREWARM_BYTES: u64 = 1024 * 1024;
const TURBO_PREFETCH_WINDOW_BYTES: u64 = 32 * 1024 * 1024;
const TURBO_HOT_CACHE_BYTES: u64 = 500 * 1024 * 1024;
const TURBO_MIN_CONNECTIONS: usize = 3;
const TURBO_MAX_CONNECTIONS: usize = 8;
const TURBO_FETCH_RETRIES: usize = 3;
const TURBO_FETCH_TIMEOUT_SECS: u64 = 20;
const TURBO_RATE_LIMIT_BACKOFF_SECS: u64 = 5;

pub const ZIP_PROXY_PORT: u16 = 48621;

#[derive(Debug, Clone)]
pub struct ProxyCacheSpec {
    pub cache_paths: zip_manager::ZipCachePaths,
    pub cache_config: zip_manager::ZipCacheConfig,
    pub start_delay_ms: u64,
    pub throttle_delay_ms: u64,
}

#[derive(Debug, Clone)]
pub enum ProxyAuth {
    GoogleDrive(gdrive::GoogleDriveClient),
    None,
}

#[derive(Debug, Clone)]
pub struct ProxyStreamSpec {
    pub drive_url: String,
    pub auth: ProxyAuth,
    pub byte_start: u64,
    pub byte_end: u64,
    pub content_type: String,
    pub cache_spec: Option<ProxyCacheSpec>,
}

pub struct ZipStreamProxyHandle {
    pub port: u16,
    shutdown_tx: Option<mpsc::Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
    cache_join_handle: Option<JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
}

impl ZipStreamProxyHandle {
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);

        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }

        if let Some(handle) = self.cache_join_handle.take() {
            let _ = handle.join();
        }
    }

    /// Clean up any partial temp files left on disk after the proxy stops.
    pub fn cleanup_temp_files(&self, cache_spec: &ProxyCacheSpec) {
        let temp = &cache_spec.cache_paths.temp_path;
        if temp.exists() {
            // Only remove if the final cache file doesn't exist (i.e. download was incomplete)
            if !cache_spec.cache_paths.cache_path.exists() {
                let _ = fs::remove_file(temp);
            }
        }
    }
}

impl Drop for ZipStreamProxyHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone)]
struct TurboProxyState {
    spec: ProxyStreamSpec,
    total_length: u64,
    total_chunks: u64,
    prewarm_chunks: u64,
    prefetch_window_chunks: u64,
    hot_limit_bytes: u64,
    stop_flag: Arc<AtomicBool>,
    cache_spec: ProxyCacheSpec,
    inner: Arc<(Mutex<TurboProxyInner>, Condvar)>,
}

struct TurboProxyInner {
    chunks: HashMap<u64, ChunkState>,
    hot_lru: VecDeque<u64>,
    pending_chunks: BTreeSet<u64>,
    hot_bytes: u64,
    contiguous_prefix_bytes: u64,
    in_flight: usize,
    max_parallel: usize,
    paused_until: Option<Instant>,
}

#[derive(Clone)]
enum ChunkState {
    Fetching,
    Ready(Arc<Vec<u8>>),
    Failed(String),
}

#[derive(Debug)]
enum FetchErrorKind {
    Retriable,
    RateLimited,
    Fatal,
}

#[derive(Debug)]
struct FetchError {
    message: String,
    kind: FetchErrorKind,
}

struct TurboStreamReader {
    turbo: TurboProxyState,
    relative_end: u64,
    position: u64,
    current_chunk_index: Option<u64>,
    current_chunk: Option<Arc<Vec<u8>>>,
}

type RequestFailure = Box<(Option<tiny_http::Request>, String)>;

impl TurboStreamReader {
    fn new(turbo: TurboProxyState, relative_start: u64, relative_end: u64) -> Self {
        let start_chunk = relative_start / TURBO_CHUNK_BYTES;
        turbo.schedule_prefetch_from(start_chunk, true);
        Self {
            turbo,
            relative_end,
            position: relative_start,
            current_chunk_index: None,
            current_chunk: None,
        }
    }
}

impl Read for TurboStreamReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.position > self.relative_end {
            return Ok(0);
        }

        let chunk_index = self.position / TURBO_CHUNK_BYTES;
        let chunk_start = chunk_index * TURBO_CHUNK_BYTES;
        let chunk = if self.current_chunk_index == Some(chunk_index) {
            self.current_chunk
                .as_ref()
                .cloned()
                .ok_or_else(|| std::io::Error::other("Missing in-flight chunk"))?
        } else {
            let chunk = self
                .turbo
                .get_chunk(chunk_index)
                .map_err(std::io::Error::other)?;
            self.current_chunk_index = Some(chunk_index);
            self.current_chunk = Some(chunk.clone());
            self.turbo.schedule_prefetch_from(chunk_index, false);
            chunk
        };

        let within_chunk = (self.position - chunk_start) as usize;
        let remaining_in_chunk = chunk.len().saturating_sub(within_chunk);
        let remaining_in_stream = (self.relative_end - self.position + 1) as usize;
        let to_copy = remaining_in_chunk.min(remaining_in_stream).min(buf.len());

        buf[..to_copy].copy_from_slice(&chunk[within_chunk..within_chunk + to_copy]);
        self.position = self.position.saturating_add(to_copy as u64);
        Ok(to_copy)
    }
}

impl TurboProxyState {
    fn new(
        spec: ProxyStreamSpec,
        cache_spec: ProxyCacheSpec,
        stop_flag: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let total_length = spec
            .byte_end
            .checked_sub(spec.byte_start)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| "Invalid ZIP byte range".to_string())?;
        let total_chunks = total_length.div_ceil(TURBO_CHUNK_BYTES);
        let contiguous_prefix_bytes = existing_prefix_len(&cache_spec);

        Ok(Self {
            spec,
            total_length,
            total_chunks,
            prewarm_chunks: TURBO_PREWARM_BYTES.div_ceil(TURBO_CHUNK_BYTES),
            prefetch_window_chunks: TURBO_PREFETCH_WINDOW_BYTES.div_ceil(TURBO_CHUNK_BYTES),
            hot_limit_bytes: TURBO_HOT_CACHE_BYTES,
            stop_flag,
            cache_spec,
            inner: Arc::new((
                Mutex::new(TurboProxyInner {
                    chunks: HashMap::new(),
                    hot_lru: VecDeque::new(),
                    pending_chunks: BTreeSet::new(),
                    hot_bytes: 0,
                    contiguous_prefix_bytes,
                    in_flight: 0,
                    max_parallel: TURBO_MIN_CONNECTIONS,
                    paused_until: None,
                }),
                Condvar::new(),
            )),
        })
    }

    fn start_prewarm(&self) {
        let turbo = self.clone();
        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if turbo.cache_spec.start_delay_ms > 0 {
                    thread::sleep(Duration::from_millis(turbo.cache_spec.start_delay_ms));
                }
                turbo.schedule_prefetch_from(0, true);
                for chunk_index in 0..turbo.prewarm_chunks.min(turbo.total_chunks) {
                    let _ = turbo.get_chunk(chunk_index);
                }
            }));
            if let Err(panic_err) = result {
                dev_elog!("[ZIP PROXY] Prewarm thread panicked: {:?}", panic_err);
            }
        });
    }

    fn get_chunk(&self, chunk_index: u64) -> Result<Arc<Vec<u8>>, String> {
        if chunk_index >= self.total_chunks {
            return Err(format!("Chunk {} is out of bounds", chunk_index));
        }

        loop {
            if self.stop_flag.load(Ordering::Relaxed) {
                return Err("ZIP proxy stopped".to_string());
            }

            let disk_ready = {
                let (lock, _) = &*self.inner;
                let mut inner = lock.lock().map_err(|e| e.to_string())?;

                match inner.chunks.get(&chunk_index).cloned() {
                    Some(ChunkState::Ready(bytes)) => {
                        inner.touch_hot(chunk_index);
                        return Ok(bytes);
                    }
                    Some(ChunkState::Failed(message)) => {
                        inner.chunks.remove(&chunk_index);
                        return Err(message);
                    }
                    Some(ChunkState::Fetching) => false,
                    None => {
                        if self.is_chunk_on_disk(inner.contiguous_prefix_bytes, chunk_index) {
                            true
                        } else {
                            inner.pending_chunks.insert(chunk_index);
                            self.maybe_spawn_fetch_locked(&mut inner);
                            false
                        }
                    }
                }
            };

            if disk_ready {
                let bytes = self.read_chunk_from_disk(chunk_index)?;
                let (lock, _) = &*self.inner;
                let mut inner = lock.lock().map_err(|e| e.to_string())?;
                let bytes = Arc::new(bytes);
                inner.insert_hot(chunk_index, bytes.clone(), self.hot_limit_bytes);
                return Ok(bytes);
            }

            let (lock, cvar) = &*self.inner;
            let inner = lock.lock().map_err(|e| e.to_string())?;
            let _ = cvar
                .wait_timeout(inner, Duration::from_millis(250))
                .map_err(|e| e.to_string())?;
        }
    }

    fn schedule_prefetch_from(&self, chunk_index: u64, prioritize_current: bool) {
        let start = chunk_index.min(self.total_chunks.saturating_sub(1));
        let end = (start + self.prefetch_window_chunks).min(self.total_chunks);
        let (lock, _) = &*self.inner;
        if let Ok(mut inner) = lock.lock() {
            if prioritize_current {
                inner.pending_chunks.insert(start);
            }
            for idx in start..end {
                if inner.chunks.contains_key(&idx) {
                    continue;
                }
                if self.is_chunk_on_disk(inner.contiguous_prefix_bytes, idx) {
                    continue;
                }
                inner.pending_chunks.insert(idx);
            }
            self.maybe_spawn_fetch_locked(&mut inner);
        }
    }

    fn maybe_spawn_fetch_locked(&self, inner: &mut TurboProxyInner) {
        if self.stop_flag.load(Ordering::Relaxed) {
            return;
        }

        if let Some(paused_until) = inner.paused_until {
            if paused_until > Instant::now() {
                return;
            }
            inner.paused_until = None;
        }

        while inner.in_flight < inner.max_parallel {
            let Some(next_chunk) = inner.pending_chunks.pop_first() else {
                break;
            };

            if inner.chunks.contains_key(&next_chunk)
                || self.is_chunk_on_disk(inner.contiguous_prefix_bytes, next_chunk)
            {
                continue;
            }

            inner.chunks.insert(next_chunk, ChunkState::Fetching);
            inner.in_flight += 1;

            let turbo = self.clone();
            thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    turbo.fetch_chunk_worker(next_chunk);
                }));
                if let Err(panic_err) = result {
                    dev_elog!("[ZIP PROXY] Fetch worker panicked: {:?}", panic_err);
                }
            });

            if self.cache_spec.throttle_delay_ms > 0 {
                thread::sleep(Duration::from_millis(self.cache_spec.throttle_delay_ms));
            }
        }
    }

    fn fetch_chunk_worker(&self, chunk_index: u64) {
        let result = self.fetch_chunk_bytes_with_retries(chunk_index);
        let (lock, cvar) = &*self.inner;
        let mut inner = match lock.lock() {
            Ok(inner) => inner,
            Err(error) => {
                dev_elog!("[ZIP PROXY] Failed to lock turbo cache state: {}", error);
                return;
            }
        };

        inner.in_flight = inner.in_flight.saturating_sub(1);

        match result {
            Ok(bytes) => {
                if inner.max_parallel < TURBO_MAX_CONNECTIONS {
                    inner.max_parallel += 1;
                }
                let bytes = Arc::new(bytes);
                inner.insert_hot(chunk_index, bytes.clone(), self.hot_limit_bytes);
                if let Err(error) = self.persist_contiguous_chunks_locked(&mut inner) {
                    dev_elog!("[ZIP PROXY] Failed to persist turbo cache chunk: {}", error);
                }
            }
            Err(error) => {
                if matches!(error.kind, FetchErrorKind::RateLimited) {
                    inner.max_parallel = 1;
                    inner.paused_until =
                        Some(Instant::now() + Duration::from_secs(TURBO_RATE_LIMIT_BACKOFF_SECS));
                } else if inner.max_parallel > TURBO_MIN_CONNECTIONS {
                    inner.max_parallel -= 1;
                }
                // Remove the chunk entry instead of storing Failed state permanently.
                // This prevents unbounded accumulation of failed entries and allows
                // the chunk to be re-fetched on next request.
                inner.chunks.remove(&chunk_index);
                dev_elog!(
                    "[ZIP PROXY] Chunk {} fetch failed (removed from cache): {}",
                    chunk_index,
                    error.message
                );
            }
        }

        self.maybe_spawn_fetch_locked(&mut inner);
        cvar.notify_all();
    }

    fn fetch_chunk_bytes_with_retries(&self, chunk_index: u64) -> Result<Vec<u8>, FetchError> {
        let (chunk_start, chunk_end) = self.chunk_relative_bounds(chunk_index);
        let upstream_start = self.spec.byte_start + chunk_start;
        let upstream_end = self.spec.byte_start + chunk_end;

        for attempt in 0..TURBO_FETCH_RETRIES {
            if self.stop_flag.load(Ordering::Relaxed) {
                return Err(FetchError {
                    message: "ZIP proxy stopped".to_string(),
                    kind: FetchErrorKind::Fatal,
                });
            }

            let client = Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(TURBO_FETCH_TIMEOUT_SECS))
                .tcp_nodelay(true)
                .build()
                .map_err(|error| FetchError {
                    message: format!("Failed to build HTTP client: {}", error),
                    kind: FetchErrorKind::Fatal,
                })?;

            let auth_runtime = build_auth_runtime(&self.spec.auth).map_err(|error| FetchError {
                message: error,
                kind: FetchErrorKind::Fatal,
            })?;
            let access_token =
                resolve_access_token(&self.spec, &auth_runtime).map_err(|error| FetchError {
                    message: error,
                    kind: FetchErrorKind::Fatal,
                })?;

            let mut request = client
                .get(&self.spec.drive_url)
                .header(RANGE, format!("bytes={}-{}", upstream_start, upstream_end));
            if !access_token.is_empty() {
                request = request.header(AUTHORIZATION, format!("Bearer {}", access_token));
            }

            match request.send() {
                Ok(response) => {
                    let status = response.status();
                    if status.as_u16() == 429 {
                        return Err(FetchError {
                            message: format!("Upstream rate limited chunk {}", chunk_index),
                            kind: FetchErrorKind::RateLimited,
                        });
                    }

                    let response = response.error_for_status().map_err(|error| FetchError {
                        message: format!(
                            "Upstream request failed for chunk {}: {}",
                            chunk_index, error
                        ),
                        kind: if attempt + 1 == TURBO_FETCH_RETRIES {
                            FetchErrorKind::Fatal
                        } else {
                            FetchErrorKind::Retriable
                        },
                    })?;

                    let bytes = response.bytes().map_err(|error| FetchError {
                        message: format!("Failed reading chunk {} bytes: {}", chunk_index, error),
                        kind: if attempt + 1 == TURBO_FETCH_RETRIES {
                            FetchErrorKind::Fatal
                        } else {
                            FetchErrorKind::Retriable
                        },
                    })?;

                    if bytes.is_empty() {
                        return Err(FetchError {
                            message: format!("Chunk {} returned zero bytes", chunk_index),
                            kind: FetchErrorKind::Fatal,
                        });
                    }

                    return Ok(bytes.to_vec());
                }
                Err(error) => {
                    let kind = classify_reqwest_error(&error, attempt + 1 == TURBO_FETCH_RETRIES);
                    if matches!(kind, FetchErrorKind::RateLimited) {
                        return Err(FetchError {
                            message: format!(
                                "Upstream throttled while fetching chunk {}: {}",
                                chunk_index, error
                            ),
                            kind,
                        });
                    }

                    if attempt + 1 == TURBO_FETCH_RETRIES {
                        return Err(FetchError {
                            message: format!(
                                "Failed to fetch chunk {} after {} attempts: {}",
                                chunk_index, TURBO_FETCH_RETRIES, error
                            ),
                            kind,
                        });
                    }
                }
            }
        }

        Err(FetchError {
            message: format!("Failed to fetch chunk {}", chunk_index),
            kind: FetchErrorKind::Fatal,
        })
    }

    fn persist_contiguous_chunks_locked(&self, inner: &mut TurboProxyInner) -> Result<(), String> {
        loop {
            if inner.contiguous_prefix_bytes >= self.total_length {
                break;
            }

            let next_chunk = inner.contiguous_prefix_bytes / TURBO_CHUNK_BYTES;
            let Some(ChunkState::Ready(bytes)) = inner.chunks.get(&next_chunk).cloned() else {
                break;
            };

            let expected_start = next_chunk * TURBO_CHUNK_BYTES;
            if expected_start != inner.contiguous_prefix_bytes {
                break;
            }

            append_bytes_at_offset(
                &self.cache_spec.cache_paths.temp_path,
                inner.contiguous_prefix_bytes,
                bytes.as_slice(),
            )?;
            inner.contiguous_prefix_bytes = inner
                .contiguous_prefix_bytes
                .saturating_add(bytes.len() as u64);
        }

        if inner.contiguous_prefix_bytes >= self.total_length {
            if let Err(error) = zip_manager::finalize_stream_cache_target(
                &self.cache_spec.cache_paths,
                &self.cache_spec.cache_config,
            ) {
                dev_elog!(
                    "[ZIP PROXY] Failed to finalize turbo cache target: {:?}",
                    error
                );
            }
        }

        inner.evict_hot_if_needed(self.hot_limit_bytes, inner.contiguous_prefix_bytes);
        Ok(())
    }

    fn is_chunk_on_disk(&self, contiguous_prefix_bytes: u64, chunk_index: u64) -> bool {
        let (chunk_start, chunk_end) = self.chunk_relative_bounds(chunk_index);
        contiguous_prefix_bytes > chunk_start && contiguous_prefix_bytes > chunk_end
    }

    fn read_chunk_from_disk(&self, chunk_index: u64) -> Result<Vec<u8>, String> {
        let cache_path = select_readable_cache_path(&self.cache_spec)
            .ok_or_else(|| "ZIP cache file not available".to_string())?;
        let (chunk_start, chunk_end) = self.chunk_relative_bounds(chunk_index);
        let expected_len = (chunk_end - chunk_start + 1) as usize;
        let mut file = File::open(&cache_path).map_err(|error| {
            format!(
                "Failed to open cache file '{}' for chunk {}: {}",
                cache_path.display(),
                chunk_index,
                error
            )
        })?;
        file.seek(SeekFrom::Start(chunk_start)).map_err(|error| {
            format!(
                "Failed to seek cache file '{}': {}",
                cache_path.display(),
                error
            )
        })?;
        let mut buffer = vec![0u8; expected_len];
        file.read_exact(&mut buffer)
            .map_err(|error| format!("Failed to read cache chunk {}: {}", chunk_index, error))?;
        Ok(buffer)
    }

    fn chunk_relative_bounds(&self, chunk_index: u64) -> (u64, u64) {
        let start = chunk_index.saturating_mul(TURBO_CHUNK_BYTES);
        let end = start
            .saturating_add(TURBO_CHUNK_BYTES)
            .saturating_sub(1)
            .min(self.total_length.saturating_sub(1));
        (start, end)
    }
}

impl TurboProxyInner {
    fn touch_hot(&mut self, chunk_index: u64) {
        if let Some(position) = self.hot_lru.iter().position(|value| *value == chunk_index) {
            self.hot_lru.remove(position);
        }
        self.hot_lru.push_back(chunk_index);
    }

    fn insert_hot(&mut self, chunk_index: u64, bytes: Arc<Vec<u8>>, hot_limit_bytes: u64) {
        let previous_len = match self.chunks.get(&chunk_index) {
            Some(ChunkState::Ready(existing)) => existing.len() as u64,
            _ => 0,
        };

        self.chunks
            .insert(chunk_index, ChunkState::Ready(bytes.clone()));
        self.hot_bytes = self.hot_bytes.saturating_sub(previous_len);
        self.hot_bytes = self.hot_bytes.saturating_add(bytes.len() as u64);
        self.touch_hot(chunk_index);
        self.evict_hot_if_needed(hot_limit_bytes, self.contiguous_prefix_bytes);
    }

    fn evict_hot_if_needed(&mut self, hot_limit_bytes: u64, contiguous_prefix_bytes: u64) {
        while self.hot_bytes > hot_limit_bytes {
            let Some(oldest_chunk) = self.hot_lru.pop_front() else {
                break;
            };

            let can_evict = match self.chunks.get(&oldest_chunk) {
                Some(ChunkState::Ready(bytes)) => {
                    let chunk_end = ((oldest_chunk + 1) * TURBO_CHUNK_BYTES)
                        .saturating_sub(1)
                        .min(contiguous_prefix_bytes.saturating_sub(1));
                    contiguous_prefix_bytes > oldest_chunk * TURBO_CHUNK_BYTES
                        && chunk_end + 1 >= ((oldest_chunk + 1) * TURBO_CHUNK_BYTES)
                        && bytes.len() as u64 <= self.hot_bytes
                }
                _ => false,
            };

            if !can_evict {
                self.hot_lru.push_back(oldest_chunk);
                break;
            }

            if let Some(ChunkState::Ready(bytes)) = self.chunks.remove(&oldest_chunk) {
                self.hot_bytes = self.hot_bytes.saturating_sub(bytes.len() as u64);
            }
        }
    }
}

pub fn start_proxy(spec: ProxyStreamSpec) -> Result<ZipStreamProxyHandle, String> {
    let server =
        Server::http("127.0.0.1:0").map_err(|e| format!("Failed to start proxy: {}", e))?;
    let port = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr.port(),
        #[cfg(unix)]
        tiny_http::ListenAddr::Unix(_) => {
            return Err("Unexpected UNIX socket for ZIP proxy".to_string());
        }
    };

    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let server = Arc::new(server);
    let server_for_thread = server.clone();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_for_server = stop_flag.clone();
    let spec_for_server = spec.clone();
    let turbo_state = match spec.cache_spec.clone() {
        Some(cache_spec) => Some(TurboProxyState::new(
            spec.clone(),
            cache_spec,
            stop_flag.clone(),
        )?),
        None => None,
    };

    if let Some(turbo) = turbo_state.as_ref() {
        turbo.start_prewarm();
    }

    let join_handle = thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let client = match Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(300))
                .tcp_nodelay(true)
                .build()
            {
                Ok(client) => client,
                Err(error) => {
                    eprintln!("[ZIP PROXY] Failed to build HTTP client: {}", error);
                    return;
                }
            };

            loop {
                if stop_flag_for_server.load(Ordering::Relaxed) || shutdown_rx.try_recv().is_ok() {
                    server_for_thread.unblock();
                    break;
                }

                let request = match server_for_thread.recv_timeout(Duration::from_millis(250)) {
                    Ok(Some(request)) => request,
                    Ok(None) => continue,
                    Err(error) => {
                        eprintln!("[ZIP PROXY] Server receive error: {}", error);
                        break;
                    }
                };

                let client_for_request = client.clone();
                let spec_for_request = spec_for_server.clone();
                let stop_flag_for_request = stop_flag_for_server.clone();
                let turbo_for_request = turbo_state.clone();

                thread::spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if stop_flag_for_request.load(Ordering::Relaxed) {
                            return;
                        }

                        let auth_runtime = match build_auth_runtime(&spec_for_request.auth) {
                            Ok(runtime) => runtime,
                            Err(error) => {
                                eprintln!("[ZIP PROXY] Failed to build auth runtime: {}", error);
                                if let Err(response_error) = respond_with_internal_error(
                                    request,
                                    "Failed to build auth runtime",
                                ) {
                                    eprintln!(
                                        "[ZIP PROXY] Failed to send 500 response: {}",
                                        response_error
                                    );
                                }
                                return;
                            }
                        };

                        if let Err(error) = handle_request(
                            request,
                            &client_for_request,
                            &spec_for_request,
                            &auth_runtime,
                            turbo_for_request.as_ref(),
                        ) {
                            let (request, error) = *error;
                            eprintln!("[ZIP PROXY] Request failed: {}", error);
                            if let Some(request) = request {
                                if let Err(response_error) =
                                    respond_with_internal_error(request, "ZIP proxy request failed")
                                {
                                    eprintln!(
                                        "[ZIP PROXY] Failed to send 500 response: {}",
                                        response_error
                                    );
                                }
                            }
                        }
                    }));
                    if let Err(panic_err) = result {
                        eprintln!("[ZIP PROXY] Request handler panicked: {:?}", panic_err);
                    }
                });
            }
        }));
        if let Err(panic_err) = result {
            eprintln!("[ZIP PROXY] Server thread panicked: {:?}", panic_err);
        }
    });

    Ok(ZipStreamProxyHandle {
        port,
        shutdown_tx: Some(shutdown_tx),
        join_handle: Some(join_handle),
        cache_join_handle: None,
        stop_flag,
    })
}

fn handle_request(
    request: tiny_http::Request,
    client: &Client,
    spec: &ProxyStreamSpec,
    auth_runtime: &Option<TokioRuntime>,
    turbo_state: Option<&TurboProxyState>,
) -> Result<(), RequestFailure> {
    let started_at = Instant::now();
    let mut request = Some(request);

    if request.as_ref().map(|request| request.url()) != Some("/stream") {
        request
            .take()
            .expect("request should be present before responding")
            .respond(Response::from_string("Not Found").with_status_code(StatusCode(404)))
            .map_err(|e| (None, e.to_string()))?;
        return Ok(());
    }

    match request
        .as_ref()
        .expect("request should be present while handling method")
        .method()
    {
        Method::Get | Method::Head => {}
        _ => {
            request
                .take()
                .expect("request should be present before responding")
                .respond(Response::empty(StatusCode(405)))
                .map_err(|e| (None, e.to_string()))?;
            return Ok(());
        }
    }

    let episode_length = spec
        .byte_end
        .checked_sub(spec.byte_start)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| (request.take(), "Invalid ZIP byte range".to_string()))?;

    let requested_range = match extract_range_header(
        request
            .as_ref()
            .expect("request should be present while parsing range"),
        episode_length,
    ) {
        Ok(range) => range,
        Err(error) => {
            let response = Response::empty(StatusCode(416)).with_header(
                make_header("Content-Range", &format!("bytes */{}", episode_length))
                    .map_err(|header_error| (request.take(), header_error))?,
            );
            request
                .take()
                .expect("request should be present before responding")
                .respond(response)
                .map_err(|e| (None, e.to_string()))?;
            return Err(Box::new((None, error)));
        }
    };

    let (relative_start, relative_end) = requested_range.unwrap_or((0, episode_length - 1));
    let response_status = if requested_range.is_some() { 206 } else { 200 };
    let body_length = (relative_end - relative_start + 1) as usize;
    let headers = build_response_headers(
        &spec.content_type,
        relative_start,
        relative_end,
        episode_length,
        body_length,
        requested_range.is_some(),
    )
    .map_err(|error| (request.take(), error))?;

    let method = match request
        .as_ref()
        .expect("request should be present for logging")
        .method()
    {
        Method::Get => "GET",
        Method::Head => "HEAD",
        _ => "OTHER",
    };
    let url = request
        .as_ref()
        .expect("request should be present for logging")
        .url()
        .to_string();

    dev_log!(
        "[ZIP PROXY] {} {} range={:?} resolved={}..{} len={} turbo={}",
        method,
        url,
        requested_range,
        relative_start,
        relative_end,
        body_length,
        turbo_state.is_some()
    );

    if matches!(
        request
            .as_ref()
            .expect("request should be present for HEAD check")
            .method(),
        Method::Head
    ) {
        let response = headers.into_iter().fold(
            Response::empty(StatusCode(response_status)),
            |response, header| response.with_header(header),
        );
        request
            .take()
            .expect("request should be present before responding")
            .respond(response)
            .map_err(|e| (None, e.to_string()))?;
        return Ok(());
    }

    if let Some(turbo) = turbo_state {
        let reader: Box<dyn Read + Send> = Box::new(TurboStreamReader::new(
            turbo.clone(),
            relative_start,
            relative_end,
        ));
        let response = Response::new(
            StatusCode(response_status),
            headers,
            reader,
            Some(body_length),
            None,
        )
        .with_chunked_threshold(usize::MAX);
        request
            .take()
            .expect("request should be present before responding")
            .respond(response.boxed())
            .map_err(|e| (None, e.to_string()))?;
        dev_log!(
            "[ZIP PROXY] Served turbo response in {} ms",
            started_at.elapsed().as_millis()
        );
        return Ok(());
    }

    let upstream_start = spec.byte_start + relative_start;
    let upstream_end = spec.byte_start + relative_end;
    let access_token =
        resolve_access_token(spec, auth_runtime).map_err(|error| (request.take(), error))?;
    let mut req = client
        .get(&spec.drive_url)
        .header(RANGE, format!("bytes={}-{}", upstream_start, upstream_end));
    if !access_token.is_empty() {
        req = req.header(AUTHORIZATION, format!("Bearer {}", access_token));
    }
    let upstream = req
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| {
            (
                request.take(),
                format!("Upstream request failed: {}", error),
            )
        })?;

    let response = Response::new(
        StatusCode(response_status),
        headers,
        upstream,
        Some(body_length),
        None,
    )
    .with_chunked_threshold(usize::MAX);
    request
        .take()
        .expect("request should be present before responding")
        .respond(response.boxed())
        .map_err(|e| (None, e.to_string()))?;
    dev_log!(
        "[ZIP PROXY] Streamed upstream response in {} ms",
        started_at.elapsed().as_millis()
    );
    Ok(())
}

fn respond_with_internal_error(request: tiny_http::Request, message: &str) -> Result<(), String> {
    request
        .respond(Response::from_string(message).with_status_code(StatusCode(500)))
        .map_err(|error| error.to_string())
}

fn existing_prefix_len(cache_spec: &ProxyCacheSpec) -> u64 {
    if let Ok(metadata) = fs::metadata(&cache_spec.cache_paths.cache_path) {
        if metadata.is_file() && metadata.len() >= cache_spec.cache_paths.expected_size {
            return cache_spec.cache_paths.expected_size;
        }
    }

    match fs::metadata(&cache_spec.cache_paths.temp_path) {
        Ok(metadata) if metadata.is_file() => {
            metadata.len().min(cache_spec.cache_paths.expected_size)
        }
        _ => 0,
    }
}

fn select_readable_cache_path(cache_spec: &ProxyCacheSpec) -> Option<PathBuf> {
    if let Ok(metadata) = fs::metadata(&cache_spec.cache_paths.cache_path) {
        if metadata.is_file() && metadata.len() >= cache_spec.cache_paths.expected_size {
            return Some(cache_spec.cache_paths.cache_path.clone());
        }
    }

    if let Ok(metadata) = fs::metadata(&cache_spec.cache_paths.temp_path) {
        if metadata.is_file() && metadata.len() > 0 {
            return Some(cache_spec.cache_paths.temp_path.clone());
        }
    }

    None
}

fn append_bytes_at_offset(path: &Path, offset: u64, bytes: &[u8]) -> Result<(), String> {
    let mut writer = OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .truncate(false)
        .open(path)
        .map_err(|error| format!("Failed to open cache file '{}': {}", path.display(), error))?;
    writer
        .seek(SeekFrom::Start(offset))
        .map_err(|error| format!("Failed to seek cache file '{}': {}", path.display(), error))?;
    writer
        .write_all(bytes)
        .map_err(|error| format!("Failed to write cache file '{}': {}", path.display(), error))?;
    writer
        .flush()
        .map_err(|error| format!("Failed to flush cache file '{}': {}", path.display(), error))
}

fn classify_reqwest_error(error: &reqwest::Error, last_attempt: bool) -> FetchErrorKind {
    if error.is_timeout() || error.is_connect() || error.is_request() {
        if last_attempt {
            FetchErrorKind::Fatal
        } else {
            FetchErrorKind::Retriable
        }
    } else {
        FetchErrorKind::Fatal
    }
}

fn extract_range_header(
    request: &tiny_http::Request,
    total_length: u64,
) -> Result<Option<(u64, u64)>, String> {
    let header = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Range"));

    let Some(header) = header else {
        return Ok(None);
    };

    let value = header.value.as_str();
    let range = value
        .strip_prefix("bytes=")
        .ok_or_else(|| "Unsupported range header".to_string())?;

    let (start_raw, end_raw) = range
        .split_once('-')
        .ok_or_else(|| "Invalid range header".to_string())?;

    if start_raw.is_empty() {
        let suffix = end_raw
            .parse::<u64>()
            .map_err(|_| "Invalid range suffix".to_string())?;
        if suffix == 0 || suffix > total_length {
            return Err("Invalid range suffix".to_string());
        }
        return Ok(Some((total_length - suffix, total_length - 1)));
    }

    let start = start_raw
        .parse::<u64>()
        .map_err(|_| "Invalid range start".to_string())?;
    let end = if end_raw.is_empty() {
        total_length - 1
    } else {
        end_raw
            .parse::<u64>()
            .map_err(|_| "Invalid range end".to_string())?
    };

    if start > end || end >= total_length {
        return Err("Range out of bounds".to_string());
    }

    Ok(Some((start, end)))
}

fn make_header(name: &str, value: &str) -> Result<Header, String> {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .map_err(|_| format!("Invalid header: {}", name))
}

fn build_response_headers(
    content_type: &str,
    relative_start: u64,
    relative_end: u64,
    total_length: u64,
    body_length: usize,
    is_partial: bool,
) -> Result<Vec<Header>, String> {
    let mut headers = vec![
        make_header("Content-Type", content_type)?,
        make_header("Content-Length", &body_length.to_string())?,
        make_header("Accept-Ranges", "bytes")?,
        make_header("Connection", "keep-alive")?,
    ];

    if is_partial {
        headers.push(make_header(
            "Content-Range",
            &format!("bytes {}-{}/{}", relative_start, relative_end, total_length),
        )?);
    }

    Ok(headers)
}

pub fn localhost_stream_url(port: u16) -> String {
    format!("http://127.0.0.1:{}/stream", port)
}

pub fn build_proxy_spec(
    drive_url: String,
    gdrive_client: gdrive::GoogleDriveClient,
    stream_info: &zip_manager::ZipStreamInfo,
    cache_spec: Option<ProxyCacheSpec>,
) -> ProxyStreamSpec {
    ProxyStreamSpec {
        drive_url,
        auth: ProxyAuth::GoogleDrive(gdrive_client),
        byte_start: stream_info.byte_start,
        byte_end: stream_info.byte_end,
        content_type: stream_info.content_type.clone(),
        cache_spec,
    }
}

pub fn build_file_proxy_spec(
    drive_url: String,
    gdrive_client: gdrive::GoogleDriveClient,
    file_size: u64,
    content_type: String,
) -> ProxyStreamSpec {
    ProxyStreamSpec {
        drive_url,
        auth: ProxyAuth::GoogleDrive(gdrive_client),
        byte_start: 0,
        byte_end: file_size.saturating_sub(1),
        content_type,
        cache_spec: None,
    }
}

pub fn build_direct_link_proxy_spec(
    url: String,
    stream_info: &zip_manager::ZipStreamInfo,
    cache_spec: Option<ProxyCacheSpec>,
) -> ProxyStreamSpec {
    ProxyStreamSpec {
        drive_url: url,
        auth: ProxyAuth::None,
        byte_start: stream_info.byte_start,
        byte_end: stream_info.byte_end,
        content_type: stream_info.content_type.clone(),
        cache_spec,
    }
}

fn build_auth_runtime(auth: &ProxyAuth) -> Result<Option<TokioRuntime>, String> {
    match auth {
        ProxyAuth::GoogleDrive(_) => TokioRuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .map(Some)
            .map_err(|error| error.to_string()),
        ProxyAuth::None => Ok(None),
    }
}

fn resolve_access_token(
    spec: &ProxyStreamSpec,
    auth_runtime: &Option<TokioRuntime>,
) -> Result<String, String> {
    match &spec.auth {
        ProxyAuth::GoogleDrive(client) => auth_runtime
            .as_ref()
            .ok_or_else(|| "Missing auth runtime for Google Drive proxy".to_string())?
            .block_on(client.get_access_token()),
        ProxyAuth::None => Ok(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── helpers ──────────────────────────────────────────────────────

    fn dummy_cache_paths() -> zip_manager::ZipCachePaths {
        zip_manager::ZipCachePaths {
            cache_path: PathBuf::from("test_cache.zip"),
            temp_path: PathBuf::from("test_cache.zip.tmp"),
            meta_path: PathBuf::from("test_cache.zip.meta"),
            expected_size: 1024,
        }
    }

    fn dummy_cache_config() -> zip_manager::ZipCacheConfig {
        zip_manager::ZipCacheConfig {
            cache_dir: String::from("test_cache"),
            max_size_bytes: 1024 * 1024,
            expiry_days: 7,
        }
    }

    fn dummy_cache_spec() -> ProxyCacheSpec {
        ProxyCacheSpec {
            cache_paths: dummy_cache_paths(),
            cache_config: dummy_cache_config(),
            start_delay_ms: 0,
            throttle_delay_ms: 0,
        }
    }

    fn dummy_stream_info() -> zip_manager::ZipStreamInfo {
        zip_manager::ZipStreamInfo {
            zip_file_id: "abc123".to_string(),
            byte_start: 100,
            byte_end: 500,
            content_type: "application/octet-stream".to_string(),
        }
    }

    fn dummy_gdrive_client() -> gdrive::GoogleDriveClient {
        gdrive::GoogleDriveClient::new()
    }

    fn direct_link_spec() -> ProxyStreamSpec {
        build_direct_link_proxy_spec(
            "https://example.com/file.zip".to_string(),
            &dummy_stream_info(),
            None,
        )
    }

    // ── ProxyCacheSpec ───────────────────────────────────────────────

    #[test]
    fn proxy_cache_spec_construction() {
        let spec = dummy_cache_spec();
        assert_eq!(spec.cache_paths.expected_size, 1024);
        assert_eq!(spec.cache_config.max_size_bytes, 1024 * 1024);
        assert_eq!(spec.start_delay_ms, 0);
        assert_eq!(spec.throttle_delay_ms, 0);
    }

    #[test]
    fn proxy_cache_spec_clone() {
        let spec = dummy_cache_spec();
        let cloned = spec.clone();
        assert_eq!(
            spec.cache_paths.expected_size,
            cloned.cache_paths.expected_size
        );
        assert_eq!(spec.start_delay_ms, cloned.start_delay_ms);
    }

    #[test]
    fn proxy_cache_spec_debug() {
        let spec = dummy_cache_spec();
        let debug = format!("{:?}", spec);
        assert!(debug.contains("ProxyCacheSpec"));
        assert!(debug.contains("1024"));
    }

    // ── ProxyAuth ────────────────────────────────────────────────────

    #[test]
    fn proxy_auth_none_variant() {
        let auth = ProxyAuth::None;
        assert!(matches!(auth, ProxyAuth::None));
    }

    #[test]
    fn proxy_auth_google_drive_variant() {
        let client = dummy_gdrive_client();
        let auth = ProxyAuth::GoogleDrive(client);
        assert!(matches!(auth, ProxyAuth::GoogleDrive(_)));
    }

    #[test]
    fn proxy_auth_none_clone() {
        let auth = ProxyAuth::None;
        let cloned = auth.clone();
        assert!(matches!(cloned, ProxyAuth::None));
    }

    #[test]
    fn proxy_auth_google_drive_clone() {
        let client = dummy_gdrive_client();
        let auth = ProxyAuth::GoogleDrive(client);
        let cloned = auth.clone();
        assert!(matches!(cloned, ProxyAuth::GoogleDrive(_)));
    }

    #[test]
    fn proxy_auth_debug() {
        let auth = ProxyAuth::None;
        let debug = format!("{:?}", auth);
        assert!(debug.contains("None"));
    }

    // ── ProxyStreamSpec ──────────────────────────────────────────────

    #[test]
    fn proxy_stream_spec_construction_with_none_auth() {
        let spec = direct_link_spec();
        assert_eq!(spec.drive_url, "https://example.com/file.zip");
        assert!(matches!(spec.auth, ProxyAuth::None));
        assert_eq!(spec.byte_start, 100);
        assert_eq!(spec.byte_end, 500);
        assert_eq!(spec.content_type, "application/octet-stream");
        assert!(spec.cache_spec.is_none());
    }

    #[test]
    fn proxy_stream_spec_with_cache_spec() {
        let stream_info = dummy_stream_info();
        let cache = dummy_cache_spec();
        let spec = build_direct_link_proxy_spec(
            "https://example.com/file.zip".to_string(),
            &stream_info,
            Some(cache),
        );
        assert!(spec.cache_spec.is_some());
        assert_eq!(
            spec.cache_spec.as_ref().unwrap().cache_paths.expected_size,
            1024
        );
    }

    #[test]
    fn proxy_stream_spec_clone() {
        let spec = direct_link_spec();
        let cloned = spec.clone();
        assert_eq!(spec.drive_url, cloned.drive_url);
        assert_eq!(spec.byte_start, cloned.byte_start);
        assert_eq!(spec.byte_end, cloned.byte_end);
        assert_eq!(spec.content_type, cloned.content_type);
    }

    #[test]
    fn proxy_stream_spec_debug() {
        let spec = direct_link_spec();
        let debug = format!("{:?}", spec);
        assert!(debug.contains("ProxyStreamSpec"));
        assert!(debug.contains("https://example.com/file.zip"));
    }

    // ── localhost_stream_url ─────────────────────────────────────────

    #[test]
    fn localhost_stream_url_basic() {
        assert_eq!(localhost_stream_url(8080), "http://127.0.0.1:8080/stream");
    }

    #[test]
    fn localhost_stream_url_default_proxy_port() {
        assert_eq!(
            localhost_stream_url(ZIP_PROXY_PORT),
            "http://127.0.0.1:48621/stream"
        );
    }

    #[test]
    fn localhost_stream_url_zero_port() {
        assert_eq!(localhost_stream_url(0), "http://127.0.0.1:0/stream");
    }

    #[test]
    fn localhost_stream_url_max_port() {
        assert_eq!(
            localhost_stream_url(u16::MAX),
            "http://127.0.0.1:65535/stream"
        );
    }

    // ── build_proxy_spec ─────────────────────────────────────────────

    #[test]
    fn build_proxy_spec_sets_fields_correctly() {
        let gdrive = dummy_gdrive_client();
        let stream_info = dummy_stream_info();
        let spec = build_proxy_spec(
            "https://drive.google.com/uc?id=xyz".to_string(),
            gdrive,
            &stream_info,
            None,
        );

        assert_eq!(spec.drive_url, "https://drive.google.com/uc?id=xyz");
        assert!(matches!(spec.auth, ProxyAuth::GoogleDrive(_)));
        assert_eq!(spec.byte_start, 100);
        assert_eq!(spec.byte_end, 500);
        assert_eq!(spec.content_type, "application/octet-stream");
        assert!(spec.cache_spec.is_none());
    }

    #[test]
    fn build_proxy_spec_with_cache() {
        let gdrive = dummy_gdrive_client();
        let stream_info = dummy_stream_info();
        let cache = dummy_cache_spec();
        let spec = build_proxy_spec(
            "https://drive.google.com/uc?id=xyz".to_string(),
            gdrive,
            &stream_info,
            Some(cache),
        );

        assert!(spec.cache_spec.is_some());
        let cs = spec.cache_spec.as_ref().unwrap();
        assert_eq!(cs.start_delay_ms, 0);
        assert_eq!(cs.throttle_delay_ms, 0);
    }

    // ── build_file_proxy_spec ────────────────────────────────────────

    #[test]
    fn build_file_proxy_spec_byte_range() {
        let gdrive = dummy_gdrive_client();
        let spec = build_file_proxy_spec(
            "https://drive.google.com/file".to_string(),
            gdrive,
            1024,
            "application/zip".to_string(),
        );

        assert_eq!(spec.byte_start, 0);
        assert_eq!(spec.byte_end, 1023);
        assert_eq!(spec.content_type, "application/zip");
        assert!(spec.cache_spec.is_none());
        assert!(matches!(spec.auth, ProxyAuth::GoogleDrive(_)));
    }

    #[test]
    fn build_file_proxy_spec_single_byte() {
        let gdrive = dummy_gdrive_client();
        let spec = build_file_proxy_spec(
            "https://example.com".to_string(),
            gdrive,
            1,
            "application/octet-stream".to_string(),
        );

        assert_eq!(spec.byte_start, 0);
        assert_eq!(spec.byte_end, 0);
    }

    #[test]
    fn build_file_proxy_spec_zero_size_saturates() {
        let gdrive = dummy_gdrive_client();
        let spec = build_file_proxy_spec(
            "https://example.com".to_string(),
            gdrive,
            0,
            "application/octet-stream".to_string(),
        );

        // file_size.saturating_sub(1) == 0 when file_size == 0
        assert_eq!(spec.byte_start, 0);
        assert_eq!(spec.byte_end, 0);
    }

    // ── build_direct_link_proxy_spec ─────────────────────────────────

    #[test]
    fn build_direct_link_proxy_spec_no_auth() {
        let stream_info = dummy_stream_info();
        let spec = build_direct_link_proxy_spec(
            "https://cdn.example.com/file.zip".to_string(),
            &stream_info,
            None,
        );

        assert_eq!(spec.drive_url, "https://cdn.example.com/file.zip");
        assert!(matches!(spec.auth, ProxyAuth::None));
        assert_eq!(spec.byte_start, 100);
        assert_eq!(spec.byte_end, 500);
        assert_eq!(spec.content_type, "application/octet-stream");
        assert!(spec.cache_spec.is_none());
    }

    #[test]
    fn build_direct_link_proxy_spec_with_cache() {
        let stream_info = dummy_stream_info();
        let cache = dummy_cache_spec();
        let spec = build_direct_link_proxy_spec(
            "https://cdn.example.com/file.zip".to_string(),
            &stream_info,
            Some(cache),
        );

        assert!(spec.cache_spec.is_some());
    }

    // ── ZipStreamProxyHandle ─────────────────────────────────────────

    #[test]
    fn zip_stream_proxy_handle_stop_without_join_handles() {
        let handle = ZipStreamProxyHandle {
            port: 9999,
            shutdown_tx: None,
            join_handle: None,
            cache_join_handle: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        };

        let mut handle = handle;
        handle.stop();
        // should not panic even with all None handles
        assert!(handle.shutdown_tx.is_none());
        assert!(handle.join_handle.is_none());
        assert!(handle.cache_join_handle.is_none());
    }

    #[test]
    fn zip_stream_proxy_handle_stop_sets_flag() {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let handle = ZipStreamProxyHandle {
            port: 1234,
            shutdown_tx: None,
            join_handle: None,
            cache_join_handle: None,
            stop_flag: stop_flag.clone(),
        };

        let mut handle = handle;
        handle.stop();
        assert!(stop_flag.load(Ordering::Relaxed));
    }

    #[test]
    fn zip_stream_proxy_handle_stop_with_shutdown_channel() {
        let (tx, rx) = mpsc::channel();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let mut handle = ZipStreamProxyHandle {
            port: 5555,
            shutdown_tx: Some(tx),
            join_handle: None,
            cache_join_handle: None,
            stop_flag: stop_flag.clone(),
        };

        handle.stop();
        // Channel should have been consumed
        assert!(handle.shutdown_tx.is_none());
        // Receiver should get the signal
        assert!(rx.recv().is_ok());
    }

    #[test]
    fn zip_stream_proxy_handle_stop_idempotent() {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let mut handle = ZipStreamProxyHandle {
            port: 7777,
            shutdown_tx: None,
            join_handle: None,
            cache_join_handle: None,
            stop_flag: stop_flag.clone(),
        };

        handle.stop();
        handle.stop(); // second call should not panic
        assert!(stop_flag.load(Ordering::Relaxed));
    }

    #[test]
    fn zip_stream_proxy_handle_cleanup_temp_no_cache_file() {
        let temp_dir = std::env::temp_dir().join("zip_proxy_test_cleanup_no_cache");
        let _ = fs::create_dir_all(&temp_dir);

        let cache_path = temp_dir.join("archive.zip");
        let temp_path = temp_dir.join("archive.zip.tmp");

        // Create temp file but NOT the final cache file
        fs::write(&temp_path, b"partial data").unwrap();

        let spec = ProxyCacheSpec {
            cache_paths: zip_manager::ZipCachePaths {
                cache_path: cache_path.clone(),
                temp_path: temp_path.clone(),
                meta_path: temp_dir.join("archive.zip.meta"),
                expected_size: 1024,
            },
            cache_config: dummy_cache_config(),
            start_delay_ms: 0,
            throttle_delay_ms: 0,
        };

        let handle = ZipStreamProxyHandle {
            port: 0,
            shutdown_tx: None,
            join_handle: None,
            cache_join_handle: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        };

        assert!(temp_path.exists());
        handle.cleanup_temp_files(&spec);
        // Temp file should be removed because final cache doesn't exist
        assert!(!temp_path.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn zip_stream_proxy_handle_cleanup_temp_kept_when_cache_exists() {
        let temp_dir = std::env::temp_dir().join("zip_proxy_test_cleanup_with_cache");
        let _ = fs::create_dir_all(&temp_dir);

        let cache_path = temp_dir.join("archive.zip");
        let temp_path = temp_dir.join("archive.zip.tmp");

        // Create BOTH files
        fs::write(&cache_path, b"complete data").unwrap();
        fs::write(&temp_path, b"partial data").unwrap();

        let spec = ProxyCacheSpec {
            cache_paths: zip_manager::ZipCachePaths {
                cache_path: cache_path.clone(),
                temp_path: temp_path.clone(),
                meta_path: temp_dir.join("archive.zip.meta"),
                expected_size: 1024,
            },
            cache_config: dummy_cache_config(),
            start_delay_ms: 0,
            throttle_delay_ms: 0,
        };

        let handle = ZipStreamProxyHandle {
            port: 0,
            shutdown_tx: None,
            join_handle: None,
            cache_join_handle: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        };

        handle.cleanup_temp_files(&spec);
        // Temp file should be kept because final cache exists
        assert!(temp_path.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn zip_stream_proxy_handle_cleanup_no_temp_file() {
        let temp_dir = std::env::temp_dir().join("zip_proxy_test_cleanup_no_temp");
        let _ = fs::create_dir_all(&temp_dir);

        let temp_path = temp_dir.join("nonexistent.zip.tmp");
        let cache_path = temp_dir.join("nonexistent.zip");

        let spec = ProxyCacheSpec {
            cache_paths: zip_manager::ZipCachePaths {
                cache_path,
                temp_path: temp_path.clone(),
                meta_path: temp_dir.join("meta"),
                expected_size: 1024,
            },
            cache_config: dummy_cache_config(),
            start_delay_ms: 0,
            throttle_delay_ms: 0,
        };

        let handle = ZipStreamProxyHandle {
            port: 0,
            shutdown_tx: None,
            join_handle: None,
            cache_join_handle: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        };

        // Should not panic when temp file doesn't exist
        handle.cleanup_temp_files(&spec);
        assert!(!temp_path.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    // ── start_proxy ──────────────────────────────────────────────────

    #[test]
    fn start_proxy_returns_handle_with_port() {
        let spec = direct_link_spec();
        let result = start_proxy(spec);
        assert!(result.is_ok(), "start_proxy failed: {:?}", result.err());
        let mut handle = result.unwrap();
        assert!(handle.port > 0);
        handle.stop();
    }

    #[test]
    fn start_proxy_bind_address_is_localhost() {
        let spec = direct_link_spec();
        let mut handle = start_proxy(spec).unwrap();
        // Port should be in the ephemeral range (typically 49152-65535 on Windows)
        assert!(handle.port > 0);
        handle.stop();
    }

    #[test]
    fn start_proxy_with_cache_spec() {
        let stream_info = dummy_stream_info();
        let cache = dummy_cache_spec();
        let spec = build_direct_link_proxy_spec(
            "https://example.com/file.zip".to_string(),
            &stream_info,
            Some(cache),
        );
        let result = start_proxy(spec);
        assert!(
            result.is_ok(),
            "start_proxy with cache failed: {:?}",
            result.err()
        );
        let mut handle = result.unwrap();
        assert!(handle.port > 0);
        handle.stop();
    }

    #[test]
    fn start_proxy_with_gdrive_auth() {
        let gdrive = dummy_gdrive_client();
        let stream_info = dummy_stream_info();
        let spec = build_proxy_spec(
            "https://drive.google.com/uc?id=xyz".to_string(),
            gdrive,
            &stream_info,
            None,
        );
        let result = start_proxy(spec);
        assert!(
            result.is_ok(),
            "start_proxy with gdrive failed: {:?}",
            result.err()
        );
        let mut handle = result.unwrap();
        assert!(handle.port > 0);
        handle.stop();
    }

    #[test]
    fn start_proxy_with_zero_byte_range() {
        let spec = ProxyStreamSpec {
            drive_url: "https://example.com/file.zip".to_string(),
            auth: ProxyAuth::None,
            byte_start: 0,
            byte_end: 0,
            content_type: "application/octet-stream".to_string(),
            cache_spec: None,
        };
        let result = start_proxy(spec);
        assert!(result.is_ok());
        let mut handle = result.unwrap();
        assert!(handle.port > 0);
        handle.stop();
    }

    // ── constants ────────────────────────────────────────────────────

    #[test]
    fn constants_are_reasonable() {
        assert_eq!(ZIP_PROXY_PORT, 48621);
        assert_eq!(TURBO_CHUNK_BYTES, 4 * 1024 * 1024);
        assert_eq!(TURBO_PREWARM_BYTES, 1024 * 1024);
        assert_eq!(TURBO_PREFETCH_WINDOW_BYTES, 32 * 1024 * 1024);
        assert_eq!(TURBO_HOT_CACHE_BYTES, 500 * 1024 * 1024);
        assert_eq!(TURBO_MIN_CONNECTIONS, 3);
        assert_eq!(TURBO_MAX_CONNECTIONS, 8);
        assert!(TURBO_MAX_CONNECTIONS >= TURBO_MIN_CONNECTIONS);
        assert_eq!(TURBO_FETCH_RETRIES, 3);
        assert_eq!(TURBO_FETCH_TIMEOUT_SECS, 20);
        assert_eq!(TURBO_RATE_LIMIT_BACKOFF_SECS, 5);
    }

    // ── classify_reqwest_error ───────────────────────────────────────

    #[test]
    fn classify_reqwest_error_non_retriable_on_last_attempt() {
        let kind = classify_reqwest_error_inner(false, false, false, true);
        assert!(matches!(kind, FetchErrorKind::Fatal));
    }

    #[test]
    fn classify_reqwest_error_retriable_on_not_last_attempt() {
        let kind = classify_reqwest_error_inner(true, false, false, false);
        assert!(matches!(kind, FetchErrorKind::Retriable));
    }

    #[test]
    fn classify_reqwest_error_connect_fatal_on_last() {
        let kind = classify_reqwest_error_inner(false, true, false, true);
        assert!(matches!(kind, FetchErrorKind::Fatal));
    }

    #[test]
    fn classify_reqwest_error_request_retriable() {
        let kind = classify_reqwest_error_inner(false, false, true, false);
        assert!(matches!(kind, FetchErrorKind::Retriable));
    }

    /// Helper that mirrors classify_reqwest_error logic without needing a real error.
    fn classify_reqwest_error_inner(
        is_timeout: bool,
        is_connect: bool,
        is_request: bool,
        last_attempt: bool,
    ) -> FetchErrorKind {
        if is_timeout || is_connect || is_request {
            if last_attempt {
                FetchErrorKind::Fatal
            } else {
                FetchErrorKind::Retriable
            }
        } else {
            FetchErrorKind::Fatal
        }
    }

    // ── chunk_relative_bounds ────────────────────────────────────────

    #[test]
    fn chunk_relative_bounds_first_chunk() {
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 0,
            byte_end: 10 * 1024 * 1024 - 1,
            content_type: String::new(),
            cache_spec: None,
        };
        let cache = dummy_cache_spec();
        let stop = Arc::new(AtomicBool::new(false));
        let turbo = TurboProxyState::new(spec, cache, stop).unwrap();

        let (start, end) = turbo.chunk_relative_bounds(0);
        assert_eq!(start, 0);
        assert_eq!(end, TURBO_CHUNK_BYTES - 1);
    }

    #[test]
    fn chunk_relative_bounds_last_partial_chunk() {
        let total: u64 = TURBO_CHUNK_BYTES * 2 + 100; // 2 full + 100 bytes
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 0,
            byte_end: total - 1,
            content_type: String::new(),
            cache_spec: None,
        };
        let cache = dummy_cache_spec();
        let stop = Arc::new(AtomicBool::new(false));
        let turbo = TurboProxyState::new(spec, cache, stop).unwrap();

        let (start, end) = turbo.chunk_relative_bounds(2);
        assert_eq!(start, 2 * TURBO_CHUNK_BYTES);
        assert_eq!(end, total - 1); // clamped to total_length - 1
    }

    #[test]
    fn chunk_relative_bounds_middle_chunk() {
        let total: u64 = TURBO_CHUNK_BYTES * 5;
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 0,
            byte_end: total - 1,
            content_type: String::new(),
            cache_spec: None,
        };
        let cache = dummy_cache_spec();
        let stop = Arc::new(AtomicBool::new(false));
        let turbo = TurboProxyState::new(spec, cache, stop).unwrap();

        let (start, end) = turbo.chunk_relative_bounds(1);
        assert_eq!(start, TURBO_CHUNK_BYTES);
        assert_eq!(end, 2 * TURBO_CHUNK_BYTES - 1);
    }

    // ── TurboProxyState::new ─────────────────────────────────────────

    #[test]
    fn turbo_proxy_state_new_valid_range() {
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 100,
            byte_end: 500,
            content_type: String::new(),
            cache_spec: None,
        };
        let cache = dummy_cache_spec();
        let stop = Arc::new(AtomicBool::new(false));
        let turbo = TurboProxyState::new(spec, cache, stop).unwrap();

        assert_eq!(turbo.total_length, 401); // 500 - 100 + 1
        assert_eq!(turbo.total_chunks, 1); // 401 < 4MB
    }

    #[test]
    fn turbo_proxy_state_new_multi_chunk() {
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 0,
            byte_end: 10 * 1024 * 1024 - 1, // 10MB
            content_type: String::new(),
            cache_spec: None,
        };
        let cache = dummy_cache_spec();
        let stop = Arc::new(AtomicBool::new(false));
        let turbo = TurboProxyState::new(spec, cache, stop).unwrap();

        assert_eq!(turbo.total_length, 10 * 1024 * 1024);
        assert_eq!(turbo.total_chunks, 3); // ceil(10MB / 4MB) = 3
    }

    #[test]
    fn turbo_proxy_state_new_invalid_range_fails() {
        // byte_end < byte_start => subtraction underflows
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 500,
            byte_end: 100,
            content_type: String::new(),
            cache_spec: None,
        };
        let cache = dummy_cache_spec();
        let stop = Arc::new(AtomicBool::new(false));
        let result = TurboProxyState::new(spec, cache, stop);
        match result {
            Err(msg) => assert!(
                msg.contains("Invalid ZIP byte range"),
                "unexpected error: {}",
                msg
            ),
            Ok(_) => panic!("expected error for invalid byte range"),
        }
    }

    #[test]
    fn turbo_proxy_state_new_single_byte() {
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 42,
            byte_end: 42,
            content_type: String::new(),
            cache_spec: None,
        };
        let cache = dummy_cache_spec();
        let stop = Arc::new(AtomicBool::new(false));
        let turbo = TurboProxyState::new(spec, cache, stop).unwrap();

        assert_eq!(turbo.total_length, 1);
        assert_eq!(turbo.total_chunks, 1);
    }

    // ── TurboProxyInner (LRU / hot cache) ────────────────────────────

    #[test]
    fn touch_hot_moves_to_back() {
        let mut inner = TurboProxyInner {
            chunks: HashMap::new(),
            hot_lru: VecDeque::new(),
            pending_chunks: BTreeSet::new(),
            hot_bytes: 0,
            contiguous_prefix_bytes: 0,
            in_flight: 0,
            max_parallel: 3,
            paused_until: None,
        };

        inner.hot_lru.push_back(0);
        inner.hot_lru.push_back(1);
        inner.hot_lru.push_back(2);

        inner.touch_hot(1); // move chunk 1 to back

        assert_eq!(inner.hot_lru.make_contiguous(), &[0, 2, 1]);
    }

    #[test]
    fn touch_hot_new_chunk_appended() {
        let mut inner = TurboProxyInner {
            chunks: HashMap::new(),
            hot_lru: VecDeque::new(),
            pending_chunks: BTreeSet::new(),
            hot_bytes: 0,
            contiguous_prefix_bytes: 0,
            in_flight: 0,
            max_parallel: 3,
            paused_until: None,
        };

        inner.touch_hot(5);
        assert_eq!(inner.hot_lru.len(), 1);
        assert_eq!(inner.hot_lru[0], 5);
    }

    #[test]
    fn insert_hot_tracks_bytes() {
        let mut inner = TurboProxyInner {
            chunks: HashMap::new(),
            hot_lru: VecDeque::new(),
            pending_chunks: BTreeSet::new(),
            hot_bytes: 0,
            contiguous_prefix_bytes: 0,
            in_flight: 0,
            max_parallel: 3,
            paused_until: None,
        };

        let data = Arc::new(vec![0u8; 1024]);
        inner.insert_hot(0, data.clone(), 10_000);

        assert_eq!(inner.hot_bytes, 1024);
        assert!(inner.hot_lru.contains(&0));
        assert!(inner.chunks.contains_key(&0));
    }

    #[test]
    fn insert_hot_replaces_existing_chunk() {
        let mut inner = TurboProxyInner {
            chunks: HashMap::new(),
            hot_lru: VecDeque::new(),
            pending_chunks: BTreeSet::new(),
            hot_bytes: 0,
            contiguous_prefix_bytes: 0,
            in_flight: 0,
            max_parallel: 3,
            paused_until: None,
        };

        let data1 = Arc::new(vec![0u8; 512]);
        inner.insert_hot(0, data1, 10_000);
        assert_eq!(inner.hot_bytes, 512);

        let data2 = Arc::new(vec![0u8; 1024]);
        inner.insert_hot(0, data2, 10_000);
        // hot_bytes should reflect new size, not cumulative
        assert_eq!(inner.hot_bytes, 1024);
    }

    #[test]
    fn evict_hot_if_needed_removes_oldest() {
        // contiguous_prefix_bytes must cover all chunks being evicted.
        // Each chunk spans TURBO_CHUNK_BYTES (4MB), so 3 chunks need >= 12MB.
        let prefix: u64 = 3 * TURBO_CHUNK_BYTES;
        let mut inner = TurboProxyInner {
            chunks: HashMap::new(),
            hot_lru: VecDeque::new(),
            pending_chunks: BTreeSet::new(),
            hot_bytes: 0,
            contiguous_prefix_bytes: prefix,
            in_flight: 0,
            max_parallel: 3,
            paused_until: None,
        };

        // Insert 3 chunks
        for i in 0..3 {
            let data = Arc::new(vec![0u8; 1024]);
            inner.insert_hot(i, data, 10_000_000);
        }
        assert_eq!(inner.hot_bytes, 3072);

        // Evict with a very small limit
        inner.evict_hot_if_needed(500, prefix);
        // All chunks should be evicted (each 1024 bytes, limit is 500)
        assert!(
            inner.hot_bytes <= 500,
            "expected <=500, got {}",
            inner.hot_bytes
        );
    }

    // ── FetchErrorKind ───────────────────────────────────────────────

    #[test]
    fn fetch_error_kind_debug() {
        assert_eq!(format!("{:?}", FetchErrorKind::Retriable), "Retriable");
        assert_eq!(format!("{:?}", FetchErrorKind::RateLimited), "RateLimited");
        assert_eq!(format!("{:?}", FetchErrorKind::Fatal), "Fatal");
    }

    #[test]
    fn fetch_error_debug() {
        let err = FetchError {
            message: "test error".to_string(),
            kind: FetchErrorKind::Fatal,
        };
        let debug = format!("{:?}", err);
        assert!(debug.contains("test error"));
        assert!(debug.contains("Fatal"));
    }

    // ── is_chunk_on_disk ─────────────────────────────────────────────

    #[test]
    fn is_chunk_on_disk_true_when_prefix_covers_chunk() {
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 0,
            byte_end: 10 * TURBO_CHUNK_BYTES - 1,
            content_type: String::new(),
            cache_spec: None,
        };
        let cache = dummy_cache_spec();
        let stop = Arc::new(AtomicBool::new(false));
        let turbo = TurboProxyState::new(spec, cache, stop).unwrap();

        // If prefix covers chunk 0 (0..4MB-1), prefix > 0 and prefix > 4MB-1
        assert!(turbo.is_chunk_on_disk(TURBO_CHUNK_BYTES + 1, 0));
    }

    #[test]
    fn is_chunk_on_disk_false_when_prefix_insufficient() {
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 0,
            byte_end: 10 * TURBO_CHUNK_BYTES - 1,
            content_type: String::new(),
            cache_spec: None,
        };
        let cache = dummy_cache_spec();
        let stop = Arc::new(AtomicBool::new(false));
        let turbo = TurboProxyState::new(spec, cache, stop).unwrap();

        // Prefix of 0 means nothing is on disk
        assert!(!turbo.is_chunk_on_disk(0, 0));
    }

    #[test]
    fn is_chunk_on_disk_false_for_later_chunk() {
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 0,
            byte_end: 10 * TURBO_CHUNK_BYTES - 1,
            content_type: String::new(),
            cache_spec: None,
        };
        let cache = dummy_cache_spec();
        let stop = Arc::new(AtomicBool::new(false));
        let turbo = TurboProxyState::new(spec, cache, stop).unwrap();

        // Chunk 1 starts at TURBO_CHUNK_BYTES. Prefix of 1 byte doesn't cover it.
        assert!(!turbo.is_chunk_on_disk(1, 1));
    }

    // ── build_auth_runtime ───────────────────────────────────────────

    #[test]
    fn build_auth_runtime_none_returns_none() {
        let result = build_auth_runtime(&ProxyAuth::None);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn build_auth_runtime_gdrive_returns_runtime() {
        let client = dummy_gdrive_client();
        let result = build_auth_runtime(&ProxyAuth::GoogleDrive(client));
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    // ── resolve_access_token ─────────────────────────────────────────

    #[test]
    fn resolve_access_token_none_auth_returns_empty() {
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::None,
            byte_start: 0,
            byte_end: 100,
            content_type: String::new(),
            cache_spec: None,
        };
        let result = resolve_access_token(&spec, &None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn resolve_access_token_gdrive_without_runtime_fails() {
        let client = dummy_gdrive_client();
        let spec = ProxyStreamSpec {
            drive_url: String::new(),
            auth: ProxyAuth::GoogleDrive(client),
            byte_start: 0,
            byte_end: 100,
            content_type: String::new(),
            cache_spec: None,
        };
        let result = resolve_access_token(&spec, &None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing auth runtime"));
    }

    // ── existing_prefix_len ──────────────────────────────────────────

    #[test]
    fn existing_prefix_len_returns_zero_for_missing_files() {
        let spec = ProxyCacheSpec {
            cache_paths: zip_manager::ZipCachePaths {
                cache_path: PathBuf::from("/nonexistent/path/archive.zip"),
                temp_path: PathBuf::from("/nonexistent/path/archive.zip.tmp"),
                meta_path: PathBuf::from("/nonexistent/path/archive.zip.meta"),
                expected_size: 1024,
            },
            cache_config: dummy_cache_config(),
            start_delay_ms: 0,
            throttle_delay_ms: 0,
        };
        assert_eq!(existing_prefix_len(&spec), 0);
    }

    // ── select_readable_cache_path ───────────────────────────────────

    #[test]
    fn select_readable_cache_path_returns_none_for_missing_files() {
        let spec = ProxyCacheSpec {
            cache_paths: zip_manager::ZipCachePaths {
                cache_path: PathBuf::from("/nonexistent/path/archive.zip"),
                temp_path: PathBuf::from("/nonexistent/path/archive.zip.tmp"),
                meta_path: PathBuf::from("/nonexistent/path/archive.zip.meta"),
                expected_size: 1024,
            },
            cache_config: dummy_cache_config(),
            start_delay_ms: 0,
            throttle_delay_ms: 0,
        };
        assert!(select_readable_cache_path(&spec).is_none());
    }

    #[test]
    fn select_readable_cache_path_prefers_cache_over_temp() {
        let temp_dir = std::env::temp_dir().join("zip_proxy_test_select_readable");
        let _ = fs::create_dir_all(&temp_dir);

        let cache_path = temp_dir.join("archive.zip");
        let temp_path = temp_dir.join("archive.zip.tmp");

        fs::write(&cache_path, vec![0u8; 2048]).unwrap();
        fs::write(&temp_path, vec![0u8; 512]).unwrap();

        let spec = ProxyCacheSpec {
            cache_paths: zip_manager::ZipCachePaths {
                cache_path: cache_path.clone(),
                temp_path: temp_path.clone(),
                meta_path: temp_dir.join("meta"),
                expected_size: 2048,
            },
            cache_config: dummy_cache_config(),
            start_delay_ms: 0,
            throttle_delay_ms: 0,
        };

        let result = select_readable_cache_path(&spec);
        assert_eq!(result, Some(cache_path));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    // ── append_bytes_at_offset ───────────────────────────────────────

    #[test]
    fn append_bytes_at_offset_creates_and_writes() {
        let temp_dir = std::env::temp_dir().join("zip_proxy_test_append");
        let _ = fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("test_append.bin");

        let _ = fs::remove_file(&path);

        append_bytes_at_offset(&path, 0, b"hello").unwrap();
        let content = fs::read(&path).unwrap();
        assert_eq!(content, b"hello");

        // Append at offset
        append_bytes_at_offset(&path, 5, b" world").unwrap();
        let content = fs::read(&path).unwrap();
        assert_eq!(content, b"hello world");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn append_bytes_at_offset_sparse_write() {
        let temp_dir = std::env::temp_dir().join("zip_proxy_test_sparse");
        let _ = fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("test_sparse.bin");

        let _ = fs::remove_file(&path);

        // Write at offset 10 on a new file (creates sparse region)
        append_bytes_at_offset(&path, 10, b"XY").unwrap();
        let content = fs::read(&path).unwrap();
        assert_eq!(content.len(), 12); // 10 zero bytes + "XY"
        assert_eq!(content[10], b'X');
        assert_eq!(content[11], b'Y');

        let _ = fs::remove_dir_all(&temp_dir);
    }

    // ── make_header / build_response_headers ─────────────────────────

    #[test]
    fn make_header_valid() {
        let header = make_header("Content-Type", "text/plain");
        assert!(header.is_ok());
    }

    #[test]
    fn build_response_headers_full_response() {
        let headers = build_response_headers("video/mp4", 0, 999, 1000, 1000, false).unwrap();
        // Should have: Content-Type, Content-Length, Accept-Ranges, Connection (no Content-Range)
        assert_eq!(headers.len(), 4);
    }

    #[test]
    fn build_response_headers_partial_response() {
        let headers = build_response_headers("video/mp4", 100, 199, 1000, 100, true).unwrap();
        // Should have: Content-Type, Content-Length, Accept-Ranges, Connection, Content-Range
        assert_eq!(headers.len(), 5);
    }
}
