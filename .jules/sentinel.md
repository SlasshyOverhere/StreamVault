## 2024-05-24 - [Fix Command Injection in PowerShell Expand-Archive]
**Vulnerability:** Command injection when dynamically constructing a PowerShell script containing arbitrary file paths to execute `Expand-Archive`.
**Learning:** `std::process::Command` in Rust passes arguments directly, but when passing a single formatted string to `powershell -Command`, PowerShell evaluates the entire string, allowing execution of embedded subexpressions (e.g. `$(...)`). Even if wrapped in quotes, it can be bypassed or misinterpreted depending on spaces and PowerShell parsing rules.
**Prevention:** Never pass untrusted strings into `-Command`. Pass them securely as environment variables using `.env("VAR", val)` and reference them securely in the script via `$env:VAR`.

## 2024-05-24 — Fix Command Injection in Remote MPV Playback
Vulnerability: Argument injection in `play_media_remote` when passing `url` to MPV without ensuring it comes after the `--` separator.
Learning: Even if `--` is added, it only protects arguments that come *after* it. If a user-supplied URL (which could be controlled via deep links or remote sources) is placed before the `--` separator, and it starts with a hyphen (e.g. `--script=malicious.lua`), MPV will interpret it as an option, leading to arbitrary code execution.
Prevention: Always ensure user-supplied input (files, URLs) is placed strictly *after* the `--` separator when invoking external CLI tools like MPV or VLC.
## 2025-02-27 — Argument Injection in FFmpeg/FFprobe via Local File Paths
Vulnerability: Launching `ffmpeg` or `ffprobe` via `std::process::Command` by passing an untrusted/local file path directly allows an attacker to inject arguments if the path begins with a hyphen (e.g., `-foo.mp4`). This occurs because these tools do not support the `--` argument separator to reliably stop option parsing.
Learning: Even though `std::process::Command` passes arguments natively (preventing shell injection), CLI tools that parse their own positional arguments as options if they start with `-` are still vulnerable to argument injection.
Prevention: When passing local file paths to `ffmpeg` or `ffprobe`, prefix the file path with the `file:` scheme (if it's not already a valid protocol like `http://` or `https://`). This forces the tool to treat the argument as a URI/path rather than a flag.
