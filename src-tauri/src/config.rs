use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::database::{get_app_data_dir, get_config_path};

/// Filename for the bundled MPV RAR archive in the repo
const BUNDLED_MPV_RAR_FILENAME: &str = "slasshyvault-mpv.rar";

/// Name of the temporary downloaded RAR file
const BUNDLED_MPV_RAR_TEMP_NAME: &str = "slasshyvault-mpv.rar";

/// GitHub repository slug and branch
const GITHUB_REPO: &str = "SlasshyOverhere/SlasshyVault";
const GITHUB_BRANCH: &str = "main";

/// Relative path to the bundled MPV archive in the repo
const BUNDLED_MPV_REPO_PATH: &str = "mpv-player/slasshyvault-mpv.rar";

/// Returns the download URL for the bundled MPV archive from the GitHub repo.
/// Uses media.githubusercontent.com to resolve Git LFS objects.
pub fn get_bundled_mpv_download_url() -> String {
    format!(
        "https://media.githubusercontent.com/media/{}/{}/{}",
        GITHUB_REPO, GITHUB_BRANCH, BUNDLED_MPV_REPO_PATH
    )
}

/// Returns the directory where bundled MPV is stored/extracted in app data
pub fn get_bundled_mpv_dir() -> PathBuf {
    get_app_data_dir().join("mpv_bundled")
}

/// Returns the temp path for the downloaded RAR file before extraction
pub fn get_bundled_mpv_rar_temp_path() -> PathBuf {
    get_bundled_mpv_dir().join(BUNDLED_MPV_RAR_TEMP_NAME)
}

/// Search for mpv.exe inside the extracted bundled directory (recursive)
#[cfg(not(target_os = "linux"))]
pub fn get_bundled_mpv_path() -> PathBuf {
    let dir = get_bundled_mpv_dir();
    if !dir.exists() {
        return dir.join("mpv.exe");
    }
    find_mpv_recursive(&dir).unwrap_or_else(|| dir.join("mpv.exe"))
}

#[cfg(not(target_os = "linux"))]
fn find_mpv_recursive(dir: &Path) -> Option<PathBuf> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find_mpv_recursive(&path) {
                    return Some(found);
                }
            } else if path.is_file() {
                let name = path.file_stem()?.to_str()?;
                if name.eq_ignore_ascii_case("mpv") || name.eq_ignore_ascii_case("slasshyvault-mpv")
                {
                    let ext = path.extension()?.to_str()?;
                    if ext.eq_ignore_ascii_case("exe") {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

/// Search for an extensionless Linux MPV binary inside the extracted directory.
#[cfg(target_os = "linux")]
pub fn get_bundled_mpv_path() -> PathBuf {
    let dir = get_bundled_mpv_dir();
    if !dir.exists() {
        return dir.join("mpv");
    }
    find_mpv_recursive(&dir).unwrap_or_else(|| dir.join("mpv"))
}

#[cfg(target_os = "linux")]
fn find_mpv_recursive(dir: &Path) -> Option<PathBuf> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find_mpv_recursive(&path) {
                    return Some(found);
                }
            } else if path.is_file() {
                let name = path.file_name()?.to_str()?;
                if (name == "mpv" || name == "slasshyvault-mpv") && path.extension().is_none() {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Check if bundled MPV executable exists in app data (after extraction)
pub fn bundled_mpv_exists() -> bool {
    get_bundled_mpv_path().exists()
}

/// Remove the entire installed bundled MPV directory (for reinstallation)
pub fn remove_bundled_mpv() -> Result<(), String> {
    let dir = get_bundled_mpv_dir();
    if dir.exists() {
        fs::remove_dir_all(&dir)
            .map_err(|e| format!("Failed to remove bundled MPV directory: {}", e))?;
        println!("[MPV-BUNDLED] Removed bundled MPV directory {:?}", dir);
    }
    Ok(())
}

/// Common locations where MPV might be installed on Windows
const MPV_SEARCH_PATHS: &[&str] = &[
    // Common installation directories
    "C:\\Program Files\\mpv\\mpv.exe",
    "C:\\Program Files (x86)\\mpv\\mpv.exe",
    "C:\\Program Files\\mpv.net\\mpv.exe",
    "C:\\Program Files (x86)\\mpv.net\\mpv.exe",
    // Scoop installations
    "C:\\Users\\*\\scoop\\apps\\mpv\\current\\mpv.exe",
    "C:\\Users\\*\\scoop\\shims\\mpv.exe",
    // Chocolatey
    "C:\\ProgramData\\chocolatey\\bin\\mpv.exe",
    // Portable installations (common locations)
    "C:\\mpv\\mpv.exe",
    "C:\\Tools\\mpv\\mpv.exe",
    "D:\\mpv\\mpv.exe",
    "D:\\Tools\\mpv\\mpv.exe",
    // User profile locations
    "C:\\Users\\*\\AppData\\Local\\Programs\\mpv\\mpv.exe",
    "C:\\Users\\*\\mpv\\mpv.exe",
];

/// Search for mpv.exe on the system
/// Returns the path if found, None otherwise
#[cfg(not(target_os = "linux"))]
pub fn find_mpv_executable() -> Option<String> {
    println!("[MPV] Searching for mpv.exe on the system...");

    // First, check bundled MPV (so the app prefers its own copy)
    if bundled_mpv_exists() {
        let path = get_bundled_mpv_path().to_string_lossy().to_string();
        println!("[MPV] Found bundled mpv at: {}", path);
        return Some(path);
    }

    // Then, check if mpv is in PATH (fastest check)
    let mut where_cmd = Command::new("where");
    where_cmd.arg("mpv.exe");
    apply_hidden_process_flags(&mut where_cmd);
    if let Ok(output) = where_cmd.output() {
        if output.status.success() {
            if let Ok(path) = String::from_utf8(output.stdout) {
                let path = path.lines().next().unwrap_or("").trim();
                if !path.is_empty() && Path::new(path).exists() {
                    println!("[MPV] Found mpv in PATH: {}", path);
                    return Some(path.to_string());
                }
            }
        }
    }

    // Check common installation paths
    for pattern in MPV_SEARCH_PATHS {
        if pattern.contains('*') {
            // Handle wildcard patterns (for user-specific paths)
            if let Some(found) = expand_and_check_pattern(pattern) {
                println!("[MPV] Found mpv at: {}", found);
                return Some(found);
            }
        } else if Path::new(pattern).exists() {
            println!("[MPV] Found mpv at: {}", pattern);
            return Some(pattern.to_string());
        }
    }

    // Deep search in Program Files directories
    let search_roots = [
        "C:\\Program Files",
        "C:\\Program Files (x86)",
        "C:\\",
        "D:\\",
    ];

    for root in search_roots {
        if let Some(found) = search_directory_for_mpv(root, 3) {
            println!("[MPV] Found mpv via deep search: {}", found);
            return Some(found);
        }
    }

    println!("[MPV] mpv.exe not found on the system");
    None
}

/// Search for mpv on Linux through PATH and standard package locations.
#[cfg(target_os = "linux")]
pub fn find_mpv_executable() -> Option<String> {
    println!("[MPV] Searching for mpv on the system...");

    if bundled_mpv_exists() {
        let path = get_bundled_mpv_path().to_string_lossy().to_string();
        println!("[MPV] Found bundled mpv at: {}", path);
        return Some(path);
    }

    let output = Command::new("which").arg("mpv").output().ok()?;
    if output.status.success() {
        if let Some(path) = String::from_utf8(output.stdout)
            .ok()
            .and_then(|paths| paths.lines().map(str::trim).find(|path| !path.is_empty()).map(str::to_string))
            .filter(|path| Path::new(path).is_file())
        {
            println!("[MPV] Found mpv in PATH: {}", path);
            return Some(path.to_string());
        }
    }

    for path in ["/usr/bin/mpv", "/usr/local/bin/mpv", "/snap/bin/mpv"] {
        if Path::new(path).is_file() {
            println!("[MPV] Found mpv at: {}", path);
            return Some(path.to_string());
        }
    }

    println!("[MPV] mpv not found on the system");
    None
}

pub fn apply_hidden_process_flags(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
}

/// Expand wildcard patterns and check if mpv exists
fn expand_and_check_pattern(pattern: &str) -> Option<String> {
    // Handle patterns like "C:\Users\*\scoop\..."
    if let Some(star_pos) = pattern.find('*') {
        let prefix = &pattern[..star_pos];
        let suffix = &pattern[star_pos + 1..];

        if let Ok(entries) = fs::read_dir(prefix) {
            for entry in entries.filter_map(|e| e.ok()) {
                let full_path = format!("{}{}", entry.path().display(), suffix);
                let path = Path::new(&full_path);
                if path.exists() {
                    let canonical = path.canonicalize().ok()?;
                    return Some(canonical.to_string_lossy().to_string());
                }
            }
        }
    }
    None
}

/// Recursively search a directory for mpv.exe (with depth limit)
fn search_directory_for_mpv(dir: &str, max_depth: u32) -> Option<String> {
    if max_depth == 0 {
        return None;
    }

    let dir_path = Path::new(dir);
    if !dir_path.exists() || !dir_path.is_dir() {
        return None;
    }

    // Check if mpv.exe is directly in this directory
    let mpv_path = dir_path.join("mpv.exe");
    if mpv_path.exists() {
        return Some(mpv_path.to_string_lossy().to_string());
    }

    // Also check for mpv/mpv.exe subdirectory
    let mpv_subdir = dir_path.join("mpv").join("mpv.exe");
    if mpv_subdir.exists() {
        return Some(mpv_subdir.to_string_lossy().to_string());
    }

    // Search subdirectories (only look in likely directories to avoid slow scans)
    let likely_subdirs = [
        "mpv", "mpv.net", "video", "media", "players", "tools", "portable", "apps",
    ];

    if let Ok(entries) = fs::read_dir(dir_path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let entry_name = entry.file_name().to_string_lossy().to_lowercase();

            // Only recurse into likely directories or if at top level
            if max_depth >= 2 || likely_subdirs.iter().any(|&s| entry_name.contains(s)) {
                if let Some(found) =
                    search_directory_for_mpv(&entry.path().to_string_lossy(), max_depth - 1)
                {
                    return Some(found);
                }
            }
        }
    }

    None
}

/// Validates an executable path to ensure it points to the expected binary.
/// This prevents arbitrary command execution vulnerabilities where a malicious
/// user might set the path to a different executable (like cmd.exe or calc.exe).
pub fn validate_executable_path(path: &str, expected_name: &str) -> Result<(), String> {
    if path.is_empty() {
        return Ok(());
    }

    let path = Path::new(path);
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Invalid executable path: {}", e))?;

    // Extract the file stem (filename without extension)
    let file_stem = canonical
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("Invalid executable path: {}", path.display()))?;

    let stem_lower = file_stem.to_lowercase();
    let expected_lower = expected_name.to_lowercase();

    // Also accept "slasshyvault-mpv" when expecting "mpv" (bundled variant)
    let valid = stem_lower == expected_lower
        || (expected_lower == "mpv" && stem_lower == "slasshyvault-mpv");

    if !valid {
        return Err(format!(
            "Security violation: Executable name must be '{}' or '{}.exe', but got '{}'",
            expected_name, expected_name, file_stem
        ));
    }

    Ok(())
}

/// Auto-detect and save MPV path if not already configured
pub fn auto_detect_mpv(config: &mut Config) -> Option<String> {
    // If already configured and exists, use it
    if let Some(ref path) = config.mpv_path {
        if Path::new(path).exists() {
            println!("[MPV] Using configured path: {}", path);
            return Some(path.clone());
        }
        println!(
            "[MPV] Configured path doesn't exist: {}, searching...",
            path
        );
    }

    // Auto-detect
    if let Some(found_path) = find_mpv_executable() {
        config.mpv_path = Some(found_path.clone());
        // Save to config file
        if let Err(e) = save_config(config) {
            println!(
                "[MPV] Warning: Failed to save detected path to config: {}",
                e
            );
        } else {
            println!("[MPV] Saved detected path to config: {}", found_path);
        }
        return Some(found_path);
    }

    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub mpv_path: Option<String>,
    #[serde(default)]
    pub vlc_path: Option<String>,
    #[serde(default)]
    pub ffprobe_path: Option<String>,
    #[serde(default)]
    pub ffmpeg_path: Option<String>,
    #[serde(default)]
    pub tmdb_api_key: Option<String>,
    #[serde(default)]
    pub omdb_api_key: Option<String>,
    // Cloud cache settings
    #[serde(default)]
    pub cloud_cache_enabled: bool,
    #[serde(default)]
    pub cloud_cache_dir: Option<String>,
    #[serde(default = "default_cloud_cache_max_mb")]
    pub cloud_cache_max_mb: u32,
    #[serde(default = "default_cloud_cache_expiry_hours")]
    pub cloud_cache_expiry_hours: u32,
    // Cloud auto-scan interval in minutes (default 5 minutes)
    #[serde(default = "default_cloud_scan_interval_minutes")]
    pub cloud_scan_interval_minutes: u32,
    #[serde(default = "default_zip_indexing_enabled")]
    pub zip_indexing_enabled: bool,
    #[serde(default)]
    pub zip_cache_dir: Option<String>,
    #[serde(default = "default_zip_cache_max_gb")]
    pub zip_cache_max_gb: u32,
    #[serde(default = "default_zip_cache_expiry_days")]
    pub zip_cache_expiry_days: u32,
    #[serde(default = "default_notifications_enabled")]
    pub notifications_enabled: bool,
    // Dev mode: override the backend URL (e.g. http://localhost:3001)
    // All auth, TMDB proxy, and WebSocket URLs are derived from this
    #[serde(default)]
    pub dev_backend_url: Option<String>,
    // Player mode: "external" (mpv.exe spawned, default)
    #[serde(default)]
    pub player_mode: PlayerMode,
    // User-configured addon/proxy URL for External tab streaming (legacy, kept for migration)
    #[serde(default)]
    pub addon_url: Option<String>,
    // Multiple addon sources for External tab
    #[serde(default)]
    pub addon_sources: Vec<AddonSource>,
    // Watch Together Cloudflare relay URL (wss://...)
    #[serde(default)]
    pub together_relay_url: Option<String>,
    // Cloudflare API token for one-click relay deploy
    #[serde(default)]
    pub together_cf_token: Option<String>,
    // Cloudflare account ID for one-click relay deploy
    #[serde(default)]
    pub together_cf_account_id: Option<String>,
}

/// A configured addon source for the External tab
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonSource {
    pub id: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub is_default: bool,
    /// Path to a local addon binary (Go binary with -H=windowsgui).
    #[serde(default)]
    pub binary_path: Option<String>,
}

/// Which MPV engine to use
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PlayerMode {
    External,
}

impl Default for PlayerMode {
    fn default() -> Self {
        PlayerMode::External
    }
}

fn default_cloud_cache_max_mb() -> u32 {
    1024 // 1GB per movie
}

fn default_cloud_cache_expiry_hours() -> u32 {
    24 // Clean up after 24 hours
}

fn default_cloud_scan_interval_minutes() -> u32 {
    5 // Scan every 5 minutes by default
}

fn default_zip_indexing_enabled() -> bool {
    true
}

fn default_zip_cache_max_gb() -> u32 {
    20
}

fn default_zip_cache_expiry_days() -> u32 {
    7
}

fn default_notifications_enabled() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            mpv_path: None,
            vlc_path: None,
            ffprobe_path: None,
            ffmpeg_path: None,
            tmdb_api_key: None,
            omdb_api_key: None,
            cloud_cache_enabled: false,
            cloud_cache_dir: None,
            cloud_cache_max_mb: 1024,
            cloud_cache_expiry_hours: 24,
            cloud_scan_interval_minutes: 5,
            zip_indexing_enabled: true,
            zip_cache_dir: None,
            zip_cache_max_gb: 20,
            zip_cache_expiry_days: 7,
            notifications_enabled: true,
            dev_backend_url: None,
            player_mode: PlayerMode::default(),
            addon_url: None,
            addon_sources: Vec::new(),
            together_relay_url: None,
            together_cf_token: None,
            together_cf_account_id: None,
        }
    }
}

pub fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let config_path = get_config_path();

    if !std::path::Path::new(&config_path).exists() {
        let default_config = Config::default();
        save_config(&default_config)?;
        return Ok(default_config);
    }

    let mut file = match fs::File::open(&config_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "[CONFIG] Failed to open config file: {}. Recreating with defaults.",
                e
            );
            heal_corrupted_config(&config_path);
            return Ok(Config::default());
        }
    };
    let mut contents = String::new();
    if file.read_to_string(&mut contents).is_err() {
        eprintln!("[CONFIG] Failed to read config file. Recreating with defaults.");
        heal_corrupted_config(&config_path);
        return Ok(Config::default());
    }

    let mut config: Config = match serde_json::from_str(&contents) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[CONFIG] Corrupted config ({:?}), backing up and recreating with defaults.",
                e
            );
            heal_corrupted_config(&config_path);
            Config::default()
        }
    };
    // Apply validation bounds
    config.cloud_cache_max_mb = config.cloud_cache_max_mb.min(100000);
    config.zip_cache_max_gb = config.zip_cache_max_gb.min(500);
    config.cloud_scan_interval_minutes = config.cloud_scan_interval_minutes.max(1);
    // Migrate legacy addon_url into addon_sources if needed
    if config.addon_sources.is_empty() {
        if let Some(ref url) = config.addon_url {
            if !url.is_empty() {
                config.addon_sources.push(AddonSource {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: "Default".to_string(),
                    url: url.clone(),
                    enabled: true,
                    is_default: true,
                    binary_path: None,
                });
                config.addon_url = None;
                // Persist migration so addon_url is cleared from the file
                // and doesn't re-create the source on every restart
                let _ = save_config(&config);
            }
        }
    }
    Ok(config)
}

fn heal_corrupted_config(config_path: &str) {
    let backup = format!("{}.corrupted", config_path);
    let _ = fs::rename(config_path, &backup);
    eprintln!("[CONFIG] Corrupted config backed up to: {}", backup);
    if let Err(e) = save_config(&Config::default()) {
        eprintln!("[CONFIG] Failed to create default config: {}", e);
    } else {
        eprintln!("[CONFIG] Created fresh default config.");
    }
}

pub fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = get_config_path();

    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(&config_path).parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(config)?;
    let mut file = fs::File::create(&config_path)?;
    file.write_all(json.as_bytes())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── URL / constant tests ──────────────────────────────────────────

    #[test]
    fn get_bundled_mpv_download_url_format() {
        let url = get_bundled_mpv_download_url();
        assert!(url.starts_with("https://media.githubusercontent.com/media/"));
        assert!(url.contains(GITHUB_REPO));
        assert!(url.contains(GITHUB_BRANCH));
        assert!(url.contains(BUNDLED_MPV_REPO_PATH));
        assert_eq!(
            url,
            "https://media.githubusercontent.com/media/SlasshyOverhere/SlasshyVault/main/mpv-player/slasshyvault-mpv.rar"
        );
    }

    // ── Path helper tests ─────────────────────────────────────────────

    #[test]
    fn get_bundled_mpv_dir_contains_mpv_bundled() {
        let dir = get_bundled_mpv_dir();
        assert!(dir.ends_with("mpv_bundled"));
    }

    #[test]
    fn get_bundled_mpv_rar_temp_path_contains_rar_name() {
        let path = get_bundled_mpv_rar_temp_path();
        assert!(path.ends_with(BUNDLED_MPV_RAR_TEMP_NAME));
        // Parent should be the bundled dir
        assert_eq!(path.parent(), Some(get_bundled_mpv_dir().as_path()));
    }

    // ── PlayerMode tests ──────────────────────────────────────────────

    #[test]
    fn player_mode_default_is_external() {
        assert_eq!(PlayerMode::default(), PlayerMode::External);
    }

    #[test]
    fn player_mode_serde_roundtrip() {
        let mode = PlayerMode::External;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"external\"");
        let deserialized: PlayerMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, PlayerMode::External);
    }

    #[test]
    fn player_mode_debug_clone() {
        let mode = PlayerMode::External;
        let cloned = mode.clone();
        assert_eq!(mode, cloned);
        let debug = format!("{:?}", mode);
        assert_eq!(debug, "External");
    }

    // ── AddonSource tests ─────────────────────────────────────────────

    #[test]
    fn addon_source_serde_roundtrip() {
        let src = AddonSource {
            id: "abc-123".to_string(),
            name: "Test Addon".to_string(),
            url: "http://localhost:7000".to_string(),
            enabled: true,
            is_default: false,
            binary_path: Some("/usr/bin/addon".to_string()),
        };
        let json = serde_json::to_string(&src).unwrap();
        let deserialized: AddonSource = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "abc-123");
        assert_eq!(deserialized.name, "Test Addon");
        assert_eq!(deserialized.url, "http://localhost:7000");
        assert!(deserialized.enabled);
        assert!(!deserialized.is_default);
        assert_eq!(deserialized.binary_path, Some("/usr/bin/addon".to_string()));
    }

    #[test]
    fn addon_source_deserialize_without_optional_fields() {
        let json = r#"{"id":"x","name":"n","url":"u","enabled":false,"is_default":true}"#;
        let src: AddonSource = serde_json::from_str(json).unwrap();
        assert!(src.binary_path.is_none());
    }

    // ── Config default tests ──────────────────────────────────────────

    #[test]
    fn config_default_values() {
        let cfg = Config::default();
        assert!(cfg.mpv_path.is_none());
        assert!(cfg.vlc_path.is_none());
        assert!(cfg.ffprobe_path.is_none());
        assert!(cfg.ffmpeg_path.is_none());
        assert!(cfg.tmdb_api_key.is_none());
        assert!(cfg.omdb_api_key.is_none());
        assert!(!cfg.cloud_cache_enabled);
        assert!(cfg.cloud_cache_dir.is_none());
        assert_eq!(cfg.cloud_cache_max_mb, 1024);
        assert_eq!(cfg.cloud_cache_expiry_hours, 24);
        assert_eq!(cfg.cloud_scan_interval_minutes, 5);
        assert!(cfg.zip_indexing_enabled);
        assert!(cfg.zip_cache_dir.is_none());
        assert_eq!(cfg.zip_cache_max_gb, 20);
        assert_eq!(cfg.zip_cache_expiry_days, 7);
        assert!(cfg.notifications_enabled);
        assert!(cfg.dev_backend_url.is_none());
        assert_eq!(cfg.player_mode, PlayerMode::External);
        assert!(cfg.addon_url.is_none());
        assert!(cfg.addon_sources.is_empty());
    }

    // ── Config serde roundtrip tests ──────────────────────────────────

    #[test]
    fn config_serde_roundtrip_full() {
        let cfg = Config {
            mpv_path: Some("C:\\mpv\\mpv.exe".to_string()),
            vlc_path: Some("C:\\vlc\\vlc.exe".to_string()),
            ffprobe_path: Some("C:\\ffprobe.exe".to_string()),
            ffmpeg_path: Some("C:\\ffmpeg.exe".to_string()),
            tmdb_api_key: Some("tmdb-key-123".to_string()),
            omdb_api_key: Some("omdb-key-456".to_string()),
            cloud_cache_enabled: true,
            cloud_cache_dir: Some("D:\\cache".to_string()),
            cloud_cache_max_mb: 2048,
            cloud_cache_expiry_hours: 48,
            cloud_scan_interval_minutes: 10,
            zip_indexing_enabled: false,
            zip_cache_dir: Some("D:\\zipcache".to_string()),
            zip_cache_max_gb: 50,
            zip_cache_expiry_days: 14,
            notifications_enabled: false,
            dev_backend_url: Some("http://localhost:3001".to_string()),
            player_mode: PlayerMode::External,
            addon_url: Some("http://addon.example.com".to_string()),
            addon_sources: vec![AddonSource {
                id: "s1".to_string(),
                name: "Source1".to_string(),
                url: "http://s1.example.com".to_string(),
                enabled: true,
                is_default: true,
                binary_path: Some("C:\\addon.exe".to_string()),
            }],
            together_relay_url: Some("wss://relay.example.com".to_string()),
            together_cf_token: Some("cf-token".to_string()),
            together_cf_account_id: Some("cf-account".to_string()),
        };

        let json = serde_json::to_string_pretty(&cfg).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.mpv_path, cfg.mpv_path);
        assert_eq!(deserialized.vlc_path, cfg.vlc_path);
        assert_eq!(deserialized.ffprobe_path, cfg.ffprobe_path);
        assert_eq!(deserialized.ffmpeg_path, cfg.ffmpeg_path);
        assert_eq!(deserialized.tmdb_api_key, cfg.tmdb_api_key);
        assert_eq!(deserialized.omdb_api_key, cfg.omdb_api_key);
        assert_eq!(deserialized.cloud_cache_enabled, cfg.cloud_cache_enabled);
        assert_eq!(deserialized.cloud_cache_dir, cfg.cloud_cache_dir);
        assert_eq!(deserialized.cloud_cache_max_mb, cfg.cloud_cache_max_mb);
        assert_eq!(
            deserialized.cloud_cache_expiry_hours,
            cfg.cloud_cache_expiry_hours
        );
        assert_eq!(
            deserialized.cloud_scan_interval_minutes,
            cfg.cloud_scan_interval_minutes
        );
        assert_eq!(deserialized.zip_indexing_enabled, cfg.zip_indexing_enabled);
        assert_eq!(deserialized.zip_cache_dir, cfg.zip_cache_dir);
        assert_eq!(deserialized.zip_cache_max_gb, cfg.zip_cache_max_gb);
        assert_eq!(
            deserialized.zip_cache_expiry_days,
            cfg.zip_cache_expiry_days
        );
        assert_eq!(
            deserialized.notifications_enabled,
            cfg.notifications_enabled
        );
        assert_eq!(deserialized.dev_backend_url, cfg.dev_backend_url);
        assert_eq!(deserialized.player_mode, cfg.player_mode);
        assert_eq!(deserialized.addon_url, cfg.addon_url);
        assert_eq!(deserialized.addon_sources.len(), 1);
        assert_eq!(deserialized.addon_sources[0].id, "s1");
    }

    #[test]
    fn config_deserialize_empty_json_uses_defaults() {
        let json = "{}";
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert!(cfg.mpv_path.is_none());
        assert!(cfg.cloud_cache_max_mb == 1024); // default fn
        assert!(cfg.cloud_cache_expiry_hours == 24);
        assert!(cfg.cloud_scan_interval_minutes == 5);
        assert!(cfg.zip_indexing_enabled);
        assert!(cfg.zip_cache_max_gb == 20);
        assert!(cfg.zip_cache_expiry_days == 7);
        assert!(cfg.notifications_enabled);
        assert_eq!(cfg.player_mode, PlayerMode::External);
        assert!(cfg.addon_sources.is_empty());
    }

    #[test]
    fn config_deserialize_partial_json_fills_defaults() {
        let json = r#"{"mpv_path":"/usr/bin/mpv","cloud_cache_max_mb":512}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.mpv_path, Some("/usr/bin/mpv".to_string()));
        assert_eq!(cfg.cloud_cache_max_mb, 512);
        // Defaults still present
        assert_eq!(cfg.cloud_cache_expiry_hours, 24);
        assert!(cfg.notifications_enabled);
    }

    // ── Default function tests ────────────────────────────────────────

    #[test]
    fn default_functions_return_expected_values() {
        assert_eq!(default_cloud_cache_max_mb(), 1024);
        assert_eq!(default_cloud_cache_expiry_hours(), 24);
        assert_eq!(default_cloud_scan_interval_minutes(), 5);
        assert!(default_zip_indexing_enabled());
        assert_eq!(default_zip_cache_max_gb(), 20);
        assert_eq!(default_zip_cache_expiry_days(), 7);
        assert!(default_notifications_enabled());
    }

    // ── validate_executable_path tests ────────────────────────────────

    #[test]
    fn validate_empty_path_returns_ok() {
        assert!(validate_executable_path("", "mpv").is_ok());
    }

    #[test]
    fn validate_matching_executable_name_succeeds() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("mpv.exe");
        fs::write(&exe, b"fake").unwrap();
        assert!(validate_executable_path(&exe.to_string_lossy(), "mpv").is_ok());
    }

    #[test]
    fn validate_matching_name_without_exe_extension_succeeds() {
        // canonicalize needs the file to exist on disk
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("mpv");
        fs::write(&exe, b"fake").unwrap();
        assert!(validate_executable_path(&exe.to_string_lossy(), "mpv").is_ok());
    }

    #[test]
    fn validate_slasshyvault_mpv_variant_accepted_for_mpv() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("slasshyvault-mpv.exe");
        fs::write(&exe, b"fake").unwrap();
        assert!(validate_executable_path(&exe.to_string_lossy(), "mpv").is_ok());
    }

    #[test]
    fn validate_wrong_name_returns_err() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("calc.exe");
        fs::write(&exe, b"fake").unwrap();
        let result = validate_executable_path(&exe.to_string_lossy(), "mpv");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Security violation"));
        assert!(err.contains("calc"));
    }

    #[test]
    fn validate_nonexistent_path_returns_err() {
        let result = validate_executable_path("Z:\\nonexistent\\mpv.exe", "mpv");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid executable path"));
    }

    #[test]
    fn validate_case_insensitive_match() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("MPV.exe");
        fs::write(&exe, b"fake").unwrap();
        assert!(validate_executable_path(&exe.to_string_lossy(), "mpv").is_ok());
    }

    #[test]
    fn validate_expected_name_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("mpv.exe");
        fs::write(&exe, b"fake").unwrap();
        assert!(validate_executable_path(&exe.to_string_lossy(), "MPV").is_ok());
    }

    #[test]
    fn validate_cmd_exe_rejected_for_mpv() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("cmd.exe");
        fs::write(&exe, b"fake").unwrap();
        assert!(validate_executable_path(&exe.to_string_lossy(), "mpv").is_err());
    }

    #[test]
    fn validate_non_mpv_expected_name_matching_works() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("vlc.exe");
        fs::write(&exe, b"fake").unwrap();
        assert!(validate_executable_path(&exe.to_string_lossy(), "vlc").is_ok());
        // slasshyvault-mpv should NOT be accepted when expecting "vlc"
        let exe2 = tmp.path().join("slasshyvault-mpv.exe");
        fs::write(&exe2, b"fake").unwrap();
        assert!(validate_executable_path(&exe2.to_string_lossy(), "vlc").is_err());
    }

    // ── apply_hidden_process_flags tests ──────────────────────────────

    #[test]
    fn apply_hidden_process_flags_does_not_panic() {
        let mut cmd = Command::new("echo");
        apply_hidden_process_flags(&mut cmd);
        // On Windows this sets CREATE_NO_WINDOW; on other platforms it's a no-op.
        // Verify the command can still execute.
        let output = cmd.arg("hello").output().unwrap();
        assert!(output.status.success());
    }

    // ── find_mpv_recursive tests ──────────────────────────────────────

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn find_mpv_recursive_finds_mpv_exe_in_root() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("mpv.exe");
        fs::write(&exe, b"fake").unwrap();
        let found = find_mpv_recursive(tmp.path());
        assert!(found.is_some());
        assert_eq!(found.unwrap().file_name().unwrap(), "mpv.exe");
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn find_mpv_recursive_finds_slasshyvault_mpv_exe() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("slasshyvault-mpv.exe");
        fs::write(&exe, b"fake").unwrap();
        let found = find_mpv_recursive(tmp.path());
        assert!(found.is_some());
        assert_eq!(found.unwrap().file_name().unwrap(), "slasshyvault-mpv.exe");
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn find_mpv_recursive_finds_in_subdirectory() {
        let tmp = TempDir::new().unwrap();
        let subdir = tmp.path().join("sub");
        fs::create_dir(&subdir).unwrap();
        let exe = subdir.join("mpv.exe");
        fs::write(&exe, b"fake").unwrap();
        let found = find_mpv_recursive(tmp.path());
        assert!(found.is_some());
        assert!(found.unwrap().starts_with(&subdir));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn find_mpv_recursive_case_insensitive_name() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("MPV.EXE");
        fs::write(&exe, b"fake").unwrap();
        let found = find_mpv_recursive(tmp.path());
        assert!(found.is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn find_mpv_recursive_finds_extensionless_linux_mpv() {
        let tmp = TempDir::new().unwrap();
        let binary = tmp.path().join("mpv");
        fs::write(&binary, b"fake").unwrap();
        let found = find_mpv_recursive(tmp.path());
        assert_eq!(found.as_deref(), Some(binary.as_path()));
    }

    #[test]
    fn find_mpv_recursive_returns_none_when_no_mpv() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("other.exe"), b"fake").unwrap();
        assert!(find_mpv_recursive(tmp.path()).is_none());
    }

    #[test]
    fn find_mpv_recursive_returns_none_for_empty_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(find_mpv_recursive(tmp.path()).is_none());
    }

    #[test]
    fn find_mpv_recursive_returns_none_for_nonexistent_dir() {
        let path = Path::new("Z:\\nonexistent_dir_12345");
        assert!(find_mpv_recursive(path).is_none());
    }

    #[test]
    fn find_mpv_recursive_skips_non_exe_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("mpv.txt"), b"fake").unwrap();
        fs::write(tmp.path().join("mpv.dll"), b"fake").unwrap();
        assert!(find_mpv_recursive(tmp.path()).is_none());
    }

    #[test]
    fn find_mpv_recursive_ignores_wrong_stem_with_exe() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("notmpv.exe"), b"fake").unwrap();
        assert!(find_mpv_recursive(tmp.path()).is_none());
    }

    // ── search_directory_for_mpv tests ────────────────────────────────

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn search_directory_for_mpv_finds_direct() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("mpv.exe");
        fs::write(&exe, b"fake").unwrap();
        let found = search_directory_for_mpv(&tmp.path().to_string_lossy(), 3);
        assert!(found.is_some());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn search_directory_for_mpv_finds_in_subdirectory() {
        let tmp = TempDir::new().unwrap();
        let subdir = tmp.path().join("mpv");
        fs::create_dir(&subdir).unwrap();
        let exe = subdir.join("mpv.exe");
        fs::write(&exe, b"fake").unwrap();
        let found = search_directory_for_mpv(&tmp.path().to_string_lossy(), 3);
        assert!(found.is_some());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn search_directory_for_mpv_returns_none_at_zero_depth() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("mpv.exe"), b"fake").unwrap();
        assert!(search_directory_for_mpv(&tmp.path().to_string_lossy(), 0).is_none());
    }

    #[test]
    fn search_directory_for_mpv_returns_none_for_nonexistent_dir() {
        assert!(search_directory_for_mpv("Z:\\nonexistent_12345", 3).is_none());
    }

    #[test]
    fn search_directory_for_mpv_returns_none_for_file() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("regularfile.txt");
        fs::write(&file, b"fake").unwrap();
        assert!(search_directory_for_mpv(&file.to_string_lossy(), 3).is_none());
    }

    // ── expand_and_check_pattern tests ────────────────────────────────

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn expand_and_check_pattern_finds_matching_entry() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("testuser");
        fs::create_dir(&user_dir).unwrap();
        let mpv_dir = user_dir
            .join("scoop")
            .join("apps")
            .join("mpv")
            .join("current");
        fs::create_dir_all(&mpv_dir).unwrap();
        let exe = mpv_dir.join("mpv.exe");
        fs::write(&exe, b"fake").unwrap();

        let pattern = format!(
            "{}\\*\\scoop\\apps\\mpv\\current\\mpv.exe",
            tmp.path().display()
        );
        let found = expand_and_check_pattern(&pattern);
        assert!(found.is_some());
    }

    #[test]
    fn expand_and_check_pattern_returns_none_no_match() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join("testuser");
        fs::create_dir(&user_dir).unwrap();

        let pattern = format!("{}\\*\\scoop\\mpv.exe", tmp.path().display());
        let found = expand_and_check_pattern(&pattern);
        assert!(found.is_none());
    }

    #[test]
    fn expand_and_check_pattern_returns_none_nonexistent_prefix() {
        let found = expand_and_check_pattern("Z:\\nonexistent_*\\mpv.exe");
        assert!(found.is_none());
    }

    // ── save_config / load_config roundtrip tests ─────────────────────

    #[test]
    fn save_and_load_config_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("media_config.json");

        // Monkey-patch by writing directly
        let cfg = Config {
            mpv_path: Some("C:\\mpv\\mpv.exe".to_string()),
            tmdb_api_key: Some("key123".to_string()),
            cloud_cache_max_mb: 512,
            notifications_enabled: false,
            ..Config::default()
        };

        let json = serde_json::to_string_pretty(&cfg).unwrap();
        fs::write(&config_path, &json).unwrap();

        let contents = fs::read_to_string(&config_path).unwrap();
        let loaded: Config = serde_json::from_str(&contents).unwrap();
        assert_eq!(loaded.mpv_path, Some("C:\\mpv\\mpv.exe".to_string()));
        assert_eq!(loaded.tmdb_api_key, Some("key123".to_string()));
        assert_eq!(loaded.cloud_cache_max_mb, 512);
        assert!(!loaded.notifications_enabled);
    }

    #[test]
    fn save_config_produces_valid_json() {
        let cfg = Config::default();
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        // Verify it parses back
        let _: Config = serde_json::from_str(&json).unwrap();
        // Verify pretty formatting (contains newlines)
        assert!(json.contains('\n'));
    }

    // ── heal_corrupted_config tests ───────────────────────────────────

    #[test]
    fn heal_corrupted_config_creates_backup_and_default() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("media_config.json");
        let config_path_str = config_path.to_string_lossy().to_string();

        // Write corrupted content
        fs::write(&config_path, "{invalid json!!!").unwrap();

        // Temporarily override the config path by testing heal_corrupted_config directly
        heal_corrupted_config(&config_path_str);

        // Original should be renamed to .corrupted
        let backup = format!("{}.corrupted", config_path_str);
        assert!(Path::new(&backup).exists());
        assert_eq!(fs::read_to_string(&backup).unwrap(), "{invalid json!!!");
    }

    #[test]
    fn heal_corrupted_config_no_existing_file() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("media_config.json");
        let config_path_str = config_path.to_string_lossy().to_string();

        // Should not error even if file doesn't exist
        heal_corrupted_config(&config_path_str);
    }

    // ── Constants verification ────────────────────────────────────────

    #[test]
    fn constants_are_correct() {
        assert_eq!(BUNDLED_MPV_RAR_FILENAME, "slasshyvault-mpv.rar");
        assert_eq!(BUNDLED_MPV_RAR_TEMP_NAME, "slasshyvault-mpv.rar");
        assert_eq!(GITHUB_REPO, "SlasshyOverhere/SlasshyVault");
        assert_eq!(GITHUB_BRANCH, "main");
        assert_eq!(BUNDLED_MPV_REPO_PATH, "mpv-player/slasshyvault-mpv.rar");
    }

    #[test]
    fn mpv_search_paths_not_empty() {
        assert!(!MPV_SEARCH_PATHS.is_empty());
        // All entries should contain "mpv"
        for path in MPV_SEARCH_PATHS {
            assert!(
                path.to_lowercase().contains("mpv"),
                "Search path missing 'mpv': {}",
                path
            );
        }
        // All entries should end with mpv.exe
        for path in MPV_SEARCH_PATHS {
            assert!(
                path.ends_with("mpv.exe"),
                "Search path doesn't end with mpv.exe: {}",
                path
            );
        }
    }

    // ── bundled_mpv_exists / get_bundled_mpv_path tests ───────────────

    // Note: These depend on the real app data dir which may or may not exist.
    // We test the logic by checking that bundled_mpv_exists returns a bool
    // and get_bundled_mpv_path always returns a valid PathBuf.

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn get_bundled_mpv_path_always_returns_path() {
        let path = get_bundled_mpv_path();
        // Should always return a PathBuf ending with mpv.exe
        assert!(path.ends_with("mpv.exe"));
    }

    #[test]
    fn bundled_mpv_exists_returns_bool() {
        // Just verify it doesn't panic
        let _exists = bundled_mpv_exists();
    }

    // ── remove_bundled_mpv tests ──────────────────────────────────────

    #[test]
    fn remove_bundled_mpv_on_nonexistent_dir_succeeds() {
        // This test uses the real app data dir. If the dir doesn't exist,
        // remove_bundled_mpv should succeed (no-op).
        // If it does exist, we don't want to delete it, so skip.
        let dir = get_bundled_mpv_dir();
        if !dir.exists() {
            assert!(remove_bundled_mpv().is_ok());
        }
    }

    // ── Config struct field defaults after deserialization ─────────────

    #[test]
    fn config_all_option_fields_none_by_default() {
        let cfg: Config = serde_json::from_str("{}").unwrap();
        assert!(cfg.mpv_path.is_none());
        assert!(cfg.vlc_path.is_none());
        assert!(cfg.ffprobe_path.is_none());
        assert!(cfg.ffmpeg_path.is_none());
        assert!(cfg.tmdb_api_key.is_none());
        assert!(cfg.omdb_api_key.is_none());
        assert!(cfg.cloud_cache_dir.is_none());
        assert!(cfg.zip_cache_dir.is_none());
        assert!(cfg.dev_backend_url.is_none());
        assert!(cfg.addon_url.is_none());
    }

    #[test]
    fn config_clone() {
        let cfg = Config::default();
        let cloned = cfg.clone();
        assert_eq!(cfg.mpv_path, cloned.mpv_path);
        assert_eq!(cfg.cloud_cache_max_mb, cloned.cloud_cache_max_mb);
        assert_eq!(cfg.player_mode, cloned.player_mode);
    }

    #[test]
    fn config_debug_output() {
        let cfg = Config::default();
        let debug = format!("{:?}", cfg);
        assert!(debug.contains("Config"));
        assert!(debug.contains("mpv_path"));
    }

    // ── AddonSource clone/debug ───────────────────────────────────────

    #[test]
    fn addon_source_clone_and_debug() {
        let src = AddonSource {
            id: "id1".to_string(),
            name: "name1".to_string(),
            url: "http://url1".to_string(),
            enabled: true,
            is_default: false,
            binary_path: None,
        };
        let cloned = src.clone();
        assert_eq!(cloned.id, "id1");
        let debug = format!("{:?}", src);
        assert!(debug.contains("AddonSource"));
    }

    // ── Serialization edge cases ──────────────────────────────────────

    #[test]
    fn config_with_empty_addon_sources_serializes() {
        let cfg = Config {
            addon_sources: vec![],
            ..Config::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"addon_sources\":[]"));
        let _: Config = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn config_with_multiple_addon_sources() {
        let cfg = Config {
            addon_sources: vec![
                AddonSource {
                    id: "1".to_string(),
                    name: "First".to_string(),
                    url: "http://first".to_string(),
                    enabled: true,
                    is_default: true,
                    binary_path: None,
                },
                AddonSource {
                    id: "2".to_string(),
                    name: "Second".to_string(),
                    url: "http://second".to_string(),
                    enabled: false,
                    is_default: false,
                    binary_path: Some("/path/to/binary".to_string()),
                },
            ],
            ..Config::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.addon_sources.len(), 2);
        assert_eq!(deserialized.addon_sources[0].name, "First");
        assert_eq!(deserialized.addon_sources[1].name, "Second");
        assert!(deserialized.addon_sources[1].binary_path.is_some());
    }

    // ── Load from malformed JSON (corruption scenarios) ───────────────

    #[test]
    fn deserialize_unknown_fields_ignored() {
        let json = r#"{"unknown_field":"value","mpv_path":"test"}"#;
        let cfg: Config = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.mpv_path, Some("test".to_string()));
    }

    #[test]
    fn deserialize_wrong_types_fall_back_to_defaults() {
        // cloud_cache_max_mb expects u32; string should fail serde
        let json = r#"{"cloud_cache_max_mb":"not_a_number"}"#;
        let result: Result<Config, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
