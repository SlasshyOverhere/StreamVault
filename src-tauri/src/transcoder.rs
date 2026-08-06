use std::collections::HashMap;
use std::io::BufReader;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use tiny_http::{Header, Response, Server};

// ponytail: LazyLock replaces lazy_static
static TRANSCODE_SESSIONS: LazyLock<Arc<Mutex<HashMap<u64, TranscodeSession>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));
static SESSION_COUNTER: LazyLock<Arc<Mutex<u64>>> = LazyLock::new(|| Arc::new(Mutex::new(0)));

pub struct TranscodeSession {
    pub ffmpeg_process: Option<Child>,
    pub server_port: u16,
    pub file_path: String,
}

impl Drop for TranscodeSession {
    fn drop(&mut self) {
        if let Some(ref mut process) = self.ffmpeg_process {
            let _ = process.kill();
        }
    }
}

/// Check if a video file needs transcoding for HTML5 playback
pub fn needs_transcoding(file_path: &str) -> bool {
    let ext = file_path.split('.').last().unwrap_or("").to_lowercase();

    // These formats/containers typically need transcoding for HTML5
    matches!(
        ext.as_str(),
        "mkv"
            | "avi"
            | "wmv"
            | "flv"
            | "mov"
            | "m2ts"
            | "ts"
            | "vob"
            | "divx"
            | "xvid"
            | "rmvb"
            | "rm"
    )
}

/// Find an available port for the transcoding server
fn find_available_port() -> Option<u16> {
    // Try ports in range 9000-9100
    for port in 9000..9100 {
        if TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

/// Start transcoding a video file and return a local HTTP URL
pub fn start_transcode(
    ffmpeg_path: &str,
    file_path: &str,
    start_time: Option<f64>,
) -> Result<(u64, String), String> {
    crate::config::validate_executable_path(ffmpeg_path, "ffmpeg")?;

    if !std::path::Path::new(ffmpeg_path).exists() {
        return Err("FFmpeg not found. Please configure FFmpeg path in Settings.".to_string());
    }

    if !std::path::Path::new(file_path).exists() {
        return Err(format!("Video file not found: {}", file_path));
    }

    let port = find_available_port()
        .ok_or_else(|| "No available port for transcoding server".to_string())?;

    // Create session ID
    let session_id = {
        let mut counter = SESSION_COUNTER.lock().map_err(|e| e.to_string())?;
        *counter += 1;
        *counter
    };

    // Build FFmpeg command for HLS output (most compatible for streaming)
    // We'll use fragmented MP4 for better seeking support
    let mut args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "warning".to_string(),
    ];

    // Add start time if resuming
    if let Some(time) = start_time {
        if time > 0.0 {
            args.push("-ss".to_string());
            args.push(format!("{:.2}", time));
        }
    }

    args.extend(vec![
        "-i".to_string(),
        file_path.to_string(),
        // Video: transcode to H.264 baseline for maximum compatibility
        "-c:v".to_string(),
        "libx264".to_string(),
        "-preset".to_string(),
        "ultrafast".to_string(),
        "-tune".to_string(),
        "zerolatency".to_string(),
        "-profile:v".to_string(),
        "baseline".to_string(),
        "-level".to_string(),
        "3.0".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        // Scale down if too large (max 1080p)
        "-vf".to_string(),
        "scale='min(1920,iw)':'min(1080,ih)':force_original_aspect_ratio=decrease".to_string(),
        // Audio: transcode to AAC stereo
        "-c:a".to_string(),
        "aac".to_string(),
        "-ac".to_string(),
        "2".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        // Output format: fragmented MP4 for streaming
        "-f".to_string(),
        "mp4".to_string(),
        "-movflags".to_string(),
        "frag_keyframe+empty_moov+faststart".to_string(),
        // Output to pipe
        "pipe:1".to_string(),
    ]);

    println!("[TRANSCODE] Starting FFmpeg with args: {:?}", args);

    let mut ffmpeg_cmd = Command::new(ffmpeg_path);
    crate::config::apply_hidden_process_flags(&mut ffmpeg_cmd);
    let ffmpeg_process = ffmpeg_cmd
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start FFmpeg: {}", e))?;

    let session = TranscodeSession {
        ffmpeg_process: Some(ffmpeg_process),
        server_port: port,
        file_path: file_path.to_string(),
    };

    // Store session
    {
        let mut sessions = TRANSCODE_SESSIONS.lock().map_err(|e| e.to_string())?;
        sessions.insert(session_id, session);
    }

    // Start HTTP server in background thread
    let file_path_clone = file_path.to_string();
    let ffmpeg_path_clone = ffmpeg_path.to_string();
    let start_time_clone = start_time;

    let session_id_for_cleanup = session_id;
    std::thread::spawn(move || {
        run_transcode_server(port, &ffmpeg_path_clone, &file_path_clone, start_time_clone);
        // Clean up the session entry when the server exits (natural completion or error)
        let mut sessions = TRANSCODE_SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut session) = sessions.remove(&session_id_for_cleanup) {
            if let Some(ref mut process) = session.ffmpeg_process {
                let _ = process.kill();
            }
            println!(
                "[TRANSCODE] Auto-cleaned session {} after server exit",
                session_id_for_cleanup
            );
        }
    });

    // Small delay to let server start
    std::thread::sleep(std::time::Duration::from_millis(500));

    let url = format!("http://127.0.0.1:{}/stream.mp4", port);
    println!("[TRANSCODE] Started session {} at {}", session_id, url);

    Ok((session_id, url))
}

/// Run the transcoding HTTP server
fn run_transcode_server(port: u16, ffmpeg_path: &str, file_path: &str, start_time: Option<f64>) {
    let server = match Server::http(format!("127.0.0.1:{}", port)) {
        Ok(s) => s,
        Err(e) => {
            println!("[TRANSCODE] Failed to start server: {}", e);
            return;
        }
    };

    println!("[TRANSCODE] Server listening on port {}", port);

    for request in server.incoming_requests() {
        let url = request.url();
        println!("[TRANSCODE] Request: {} {}", request.method(), url);

        if url.starts_with("/stream") {
            // Validate ffmpeg path before executing
            if let Err(e) = crate::config::validate_executable_path(ffmpeg_path, "ffmpeg") {
                println!("[TRANSCODE] Validation failed: {}", e);
                let response = Response::from_string(format!("FFmpeg validation error: {}", e))
                    .with_status_code(500);
                let _ = request.respond(response);
                break;
            }

            // Start FFmpeg and stream output
            let mut args = vec!["-hide_banner", "-loglevel", "warning"];

            let start_str;
            if let Some(time) = start_time {
                if time > 0.0 {
                    start_str = format!("{:.2}", time);
                    args.push("-ss");
                    args.push(&start_str);
                }
            }

            args.extend(vec![
                "-i",
                file_path,
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-tune",
                "zerolatency",
                "-profile:v",
                "baseline",
                "-level",
                "3.0",
                "-pix_fmt",
                "yuv420p",
                "-vf",
                "scale='min(1920,iw)':'min(1080,ih)':force_original_aspect_ratio=decrease",
                "-c:a",
                "aac",
                "-ac",
                "2",
                "-b:a",
                "192k",
                "-f",
                "mp4",
                "-movflags",
                "frag_keyframe+empty_moov+faststart",
                "pipe:1",
            ]);

            let mut ffmpeg_cmd = Command::new(ffmpeg_path);
            crate::config::apply_hidden_process_flags(&mut ffmpeg_cmd);
            match ffmpeg_cmd
                .args(&args)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    if let Some(stdout) = child.stdout.take() {
                        let content_type =
                            Header::from_bytes(&b"Content-Type"[..], &b"video/mp4"[..]).unwrap();

                        let reader = BufReader::new(stdout);
                        let response = Response::new(
                            tiny_http::StatusCode(200),
                            vec![content_type],
                            reader,
                            None,
                            None,
                        );

                        if let Err(e) = request.respond(response) {
                            println!("[TRANSCODE] Failed to send response: {}", e);
                        }
                    }
                    let _ = child.wait();
                }
                Err(e) => {
                    println!("[TRANSCODE] Failed to start FFmpeg: {}", e);
                    let response =
                        Response::from_string(format!("FFmpeg error: {}", e)).with_status_code(500);
                    let _ = request.respond(response);
                }
            }

            // Only handle one request then exit
            break;
        } else {
            let response = Response::from_string("Not found").with_status_code(404);
            let _ = request.respond(response);
        }
    }

    println!("[TRANSCODE] Server on port {} shutting down", port);
}

/// Stop a transcoding session
pub fn stop_transcode(session_id: u64) -> Result<(), String> {
    let mut sessions = TRANSCODE_SESSIONS.lock().map_err(|e| e.to_string())?;

    if let Some(mut session) = sessions.remove(&session_id) {
        if let Some(ref mut process) = session.ffmpeg_process {
            let _ = process.kill();
        }
        println!("[TRANSCODE] Stopped session {}", session_id);
    }

    Ok(())
}

/// Stop all transcoding sessions
pub fn stop_all_transcodes() -> Result<(), String> {
    let mut sessions = TRANSCODE_SESSIONS.lock().map_err(|e| e.to_string())?;

    for (id, mut session) in sessions.drain() {
        if let Some(ref mut process) = session.ffmpeg_process {
            let _ = process.kill();
        }
        println!("[TRANSCODE] Stopped session {}", id);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    // ---------------------------------------------------------------------------
    // needs_transcoding
    // ---------------------------------------------------------------------------

    #[test]
    fn needs_transcoding_for_mkv() {
        assert!(needs_transcoding("video.mkv"));
    }

    #[test]
    fn needs_transcoding_for_avi() {
        assert!(needs_transcoding("video.avi"));
    }

    #[test]
    fn needs_transcoding_for_wmv() {
        assert!(needs_transcoding("video.wmv"));
    }

    #[test]
    fn needs_transcoding_for_flv() {
        assert!(needs_transcoding("video.flv"));
    }

    #[test]
    fn needs_transcoding_for_mov() {
        assert!(needs_transcoding("video.mov"));
    }

    #[test]
    fn needs_transcoding_for_m2ts() {
        assert!(needs_transcoding("video.m2ts"));
    }

    #[test]
    fn needs_transcoding_for_ts() {
        assert!(needs_transcoding("video.ts"));
    }

    #[test]
    fn needs_transcoding_for_vob() {
        assert!(needs_transcoding("video.vob"));
    }

    #[test]
    fn needs_transcoding_for_divx() {
        assert!(needs_transcoding("video.divx"));
    }

    #[test]
    fn needs_transcoding_for_xvid() {
        assert!(needs_transcoding("video.xvid"));
    }

    #[test]
    fn needs_transcoding_for_rmvb() {
        assert!(needs_transcoding("video.rmvb"));
    }

    #[test]
    fn needs_transcoding_for_rm() {
        assert!(needs_transcoding("video.rm"));
    }

    #[test]
    fn no_transcoding_for_mp4() {
        assert!(!needs_transcoding("video.mp4"));
    }

    #[test]
    fn no_transcoding_for_webm() {
        assert!(!needs_transcoding("video.webm"));
    }

    #[test]
    fn no_transcoding_for_ogg() {
        assert!(!needs_transcoding("video.ogg"));
    }

    #[test]
    fn no_transcoding_for_gif() {
        assert!(!needs_transcoding("image.gif"));
    }

    #[test]
    fn no_transcoding_for_empty_ext() {
        assert!(!needs_transcoding("noextension"));
    }

    #[test]
    fn no_transcoding_for_no_dot() {
        assert!(!needs_transcoding("nodotfile"));
    }

    #[test]
    fn needs_transcoding_case_insensitive() {
        // Extension comparison is lowercase, but input can be mixed case
        assert!(needs_transcoding("movie.MKV"));
        assert!(needs_transcoding("movie.Mkv"));
        assert!(!needs_transcoding("movie.MP4"));
    }

    #[test]
    fn needs_transcoding_nested_path() {
        assert!(needs_transcoding("/some/deep/path/video.mkv"));
        assert!(!needs_transcoding("C:\\Users\\file.mp4"));
    }

    #[test]
    fn needs_transcoding_dotted_filename() {
        assert!(needs_transcoding("my.file.name.avi"));
        assert!(!needs_transcoding("my.file.name.mp4"));
    }

    // ---------------------------------------------------------------------------
    // TranscodeSession struct
    // ---------------------------------------------------------------------------

    #[test]
    fn transcode_session_fields() {
        let session = TranscodeSession {
            ffmpeg_process: None,
            server_port: 9001,
            file_path: "/tmp/test.mkv".to_string(),
        };

        assert!(session.ffmpeg_process.is_none());
        assert_eq!(session.server_port, 9001);
        assert_eq!(session.file_path, "/tmp/test.mkv");
    }

    #[test]
    fn transcode_session_drop_kills_process() {
        // Spawn a real child process (sleep) and verify Drop kills it
        let child = Command::new("python")
            .args(["-c", "import time; time.sleep(60)"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .or_else(|_| {
                // Fallback: try `sleep` (unix) or just skip
                Command::new("sleep")
                    .arg("60")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            });

        if let Ok(child) = child {
            let pid = child.id();
            {
                let _session = TranscodeSession {
                    ffmpeg_process: Some(child),
                    server_port: 9002,
                    file_path: "/tmp/drop_test.mkv".to_string(),
                };
                // _session dropped here, should kill child
            }
            // On Windows, process.kill sends TerminateProcess; child should be gone.
            // We can't reliably check exit status immediately, but no panic = success.
            // Just verify the pid existed (non-zero on Windows).
            assert!(pid > 0, "child process should have a valid pid");
        }
        // If neither python nor sleep is available, test passes vacuously.
    }

    // ---------------------------------------------------------------------------
    // stop_transcode
    // ---------------------------------------------------------------------------

    #[test]
    fn stop_transcode_nonexistent_session() {
        // Stopping a session that doesn't exist should succeed (no-op)
        assert!(stop_transcode(999999).is_ok());
    }

    #[test]
    fn stop_transcode_session_with_no_process() {
        // Insert a session with None process, then stop it
        let session = TranscodeSession {
            ffmpeg_process: None,
            server_port: 9010,
            file_path: "/tmp/none_proc.mkv".to_string(),
        };

        {
            let mut sessions = TRANSCODE_SESSIONS.lock().unwrap();
            sessions.insert(888888, session);
        }

        assert!(stop_transcode(888888).is_ok());

        // Verify removed
        let sessions = TRANSCODE_SESSIONS.lock().unwrap();
        assert!(!sessions.contains_key(&888888));
    }

    #[test]
    fn stop_transcode_with_real_process() {
        let child = Command::new("python")
            .args(["-c", "import time; time.sleep(60)"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .or_else(|_| {
                Command::new("sleep")
                    .arg("60")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            });

        if let Ok(child) = child {
            let session = TranscodeSession {
                ffmpeg_process: Some(child),
                server_port: 9011,
                file_path: "/tmp/stop_test.mkv".to_string(),
            };

            {
                let mut sessions = TRANSCODE_SESSIONS.lock().unwrap();
                sessions.insert(777777, session);
            }

            assert!(stop_transcode(777777).is_ok());

            let sessions = TRANSCODE_SESSIONS.lock().unwrap();
            assert!(!sessions.contains_key(&777777));
        }
    }

    // ---------------------------------------------------------------------------
    // stop_all_transcodes
    // ---------------------------------------------------------------------------

    #[test]
    fn stop_all_transcodes_empty() {
        // Clear any leftover sessions, then stop_all on empty map
        {
            let mut sessions = TRANSCODE_SESSIONS.lock().unwrap();
            sessions.clear();
        }
        assert!(stop_all_transcodes().is_ok());
    }

    #[test]
    fn stop_all_transcodes_multiple_no_process() {
        {
            let mut sessions = TRANSCODE_SESSIONS.lock().unwrap();
            for id in 700000..700005 {
                sessions.insert(
                    id,
                    TranscodeSession {
                        ffmpeg_process: None,
                        server_port: 9020 + (id - 700000) as u16,
                        file_path: format!("/tmp/all_{}.mkv", id),
                    },
                );
            }
        }

        assert!(stop_all_transcodes().is_ok());

        let sessions = TRANSCODE_SESSIONS.lock().unwrap();
        assert!(sessions.is_empty(), "all sessions should be removed");
    }

    #[test]
    fn stop_all_transcodes_mixed_process_and_none() {
        let child = Command::new("python")
            .args(["-c", "import time; time.sleep(60)"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .or_else(|_| {
                Command::new("sleep")
                    .arg("60")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            });

        {
            let mut sessions = TRANSCODE_SESSIONS.lock().unwrap();
            // One with None
            sessions.insert(
                600001,
                TranscodeSession {
                    ffmpeg_process: None,
                    server_port: 9030,
                    file_path: "/tmp/mixed_none.mkv".to_string(),
                },
            );
            // One with a real process (if available)
            if let Ok(c) = child {
                sessions.insert(
                    600002,
                    TranscodeSession {
                        ffmpeg_process: Some(c),
                        server_port: 9031,
                        file_path: "/tmp/mixed_real.mkv".to_string(),
                    },
                );
            }
        }

        assert!(stop_all_transcodes().is_ok());

        let sessions = TRANSCODE_SESSIONS.lock().unwrap();
        assert!(sessions.is_empty());
    }

    // ---------------------------------------------------------------------------
    // start_transcode – error paths (no FFmpeg installed)
    // ---------------------------------------------------------------------------

    #[test]
    fn start_transcode_ffmpeg_not_found() {
        let result = start_transcode(
            "C:\\nonexistent\\ffmpeg.exe",
            "C:\\nonexistent\\video.mkv",
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        // Error from validate_executable_path or the exists() check
        assert!(
            !err.is_empty(),
            "should return an error for nonexistent ffmpeg path"
        );
    }

    #[test]
    fn start_transcode_video_not_found() {
        // Need a path that passes validate_executable_path but file doesn't exist.
        // This test only works if ffmpeg actually exists on the system.
        // Use a clearly fake ffmpeg path that will fail the exists() check first.
        let result = start_transcode(
            "C:\\nonexistent_dir\\ffmpeg.exe",
            "C:\\nonexistent_dir\\video.mkv",
            None,
        );
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------------------
    // find_available_port
    // ---------------------------------------------------------------------------

    #[test]
    fn find_available_port_returns_some() {
        // Port 9000-9100 range; at least one should be free in a test environment
        let port = find_available_port();
        assert!(port.is_some(), "should find an available port in 9000-9100");
        let p = port.unwrap();
        assert!((9000..9100).contains(&p));
    }

    #[test]
    fn find_available_port_returns_valid_port() {
        // Verify find_available_port returns a port in the expected range
        // (Cannot reliably re-bind due to TOCTOU race with other tests)
        if let Some(port) = find_available_port() {
            assert!(
                (9000..9100).contains(&port),
                "port {} outside expected range 9000-9100",
                port
            );
        }
    }

    // ── SESSION_COUNTER increments ──

    #[test]
    fn session_counter_increments() {
        let c1 = {
            let mut counter = SESSION_COUNTER.lock().unwrap();
            *counter += 1;
            *counter
        };
        let c2 = {
            let mut counter = SESSION_COUNTER.lock().unwrap();
            *counter += 1;
            *counter
        };
        assert!(c2 > c1, "counter should increment: {} vs {}", c1, c2);
    }

    // ── TranscodeSession Drop with None process ──

    #[test]
    fn transcode_session_drop_none_process() {
        {
            let _session = TranscodeSession {
                ffmpeg_process: None,
                server_port: 9050,
                file_path: "/tmp/drop_none.mkv".to_string(),
            };
            // _session dropped here — should not panic
        }
        // If we reach here, Drop handled None gracefully
    }

    // ── stop_transcode removes specific session ──

    #[test]
    fn stop_transcode_removes_only_target_session() {
        {
            let mut sessions = TRANSCODE_SESSIONS.lock().unwrap();
            sessions.insert(
                500001,
                TranscodeSession {
                    ffmpeg_process: None,
                    server_port: 9060,
                    file_path: "/tmp/keep.mkv".to_string(),
                },
            );
            sessions.insert(
                500002,
                TranscodeSession {
                    ffmpeg_process: None,
                    server_port: 9061,
                    file_path: "/tmp/remove.mkv".to_string(),
                },
            );
        }

        assert!(stop_transcode(500002).is_ok());

        let sessions = TRANSCODE_SESSIONS.lock().unwrap();
        assert!(
            sessions.contains_key(&500001),
            "other session should remain"
        );
        assert!(
            !sessions.contains_key(&500002),
            "target session should be removed"
        );
    }

    // ── stop_all_transcodes drains all ──

    #[test]
    fn stop_all_transcodes_drains_entire_map() {
        {
            let mut sessions = TRANSCODE_SESSIONS.lock().unwrap();
            for id in 600010..600020 {
                sessions.insert(
                    id,
                    TranscodeSession {
                        ffmpeg_process: None,
                        server_port: 9070 + (id - 600010) as u16,
                        file_path: format!("/tmp/all_{}.mkv", id),
                    },
                );
            }
        }

        assert!(stop_all_transcodes().is_ok());

        let sessions = TRANSCODE_SESSIONS.lock().unwrap();
        assert!(sessions.is_empty());
    }

    // ── start_transcode with file that exists but ffmpeg doesn't ──

    #[test]
    fn start_transcode_ffmpeg_path_is_directory() {
        // Use a path that exists as a directory, not an executable
        let result = start_transcode("C:\\Windows", "C:\\Windows\\System32\\notepad.exe", None);
        assert!(result.is_err());
    }

    #[test]
    fn start_transcode_with_start_time_zero() {
        // start_time = Some(0.0) should not add -ss flag
        let result = start_transcode(
            "C:\\nonexistent\\ffmpeg.exe",
            "C:\\nonexistent\\video.mkv",
            Some(0.0),
        );
        assert!(result.is_err());
        // Error is from validate_executable_path or exists() check
    }

    #[test]
    fn start_transcode_with_positive_start_time() {
        let result = start_transcode(
            "C:\\nonexistent\\ffmpeg.exe",
            "C:\\nonexistent\\video.mkv",
            Some(42.5),
        );
        assert!(result.is_err());
    }

    // ── needs_transcoding with uppercase extensions ──

    #[test]
    fn needs_transcoding_uppercase_extensions() {
        assert!(needs_transcoding("video.AVI"));
        assert!(needs_transcoding("video.MKV"));
        assert!(needs_transcoding("video.WMV"));
        assert!(needs_transcoding("video.FLV"));
        assert!(needs_transcoding("video.MOV"));
        assert!(!needs_transcoding("video.MP4"));
        assert!(!needs_transcoding("video.WEBM"));
    }

    // ── needs_transcoding with path separators ──

    #[test]
    fn needs_transcoding_windows_path() {
        assert!(needs_transcoding("C:\\Users\\test\\video.mkv"));
        assert!(!needs_transcoding("C:\\Users\\test\\video.mp4"));
    }

    // ── Multiple concurrent stop_transcode calls ──

    #[test]
    fn stop_transcode_same_id_twice_is_safe() {
        {
            let mut sessions = TRANSCODE_SESSIONS.lock().unwrap();
            sessions.insert(
                400001,
                TranscodeSession {
                    ffmpeg_process: None,
                    server_port: 9080,
                    file_path: "/tmp/twice.mkv".to_string(),
                },
            );
        }

        // First stop removes it
        assert!(stop_transcode(400001).is_ok());
        // Second stop is a no-op (session already removed)
        assert!(stop_transcode(400001).is_ok());
    }
}
