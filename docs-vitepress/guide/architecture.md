# Architecture

SlasshyVault is a **local-first** desktop app. All user data stays on your machine unless you explicitly choose to stream media from your Drive.

## Components

| Component | Technology | Role |
|---|---|---|
| Desktop Shell | Tauri 1.x | Native window, OS integration |
| Runtime | Rust | Media scanning, streaming, playback, DB |
| Frontend | React + TypeScript + Vite | UI |
| Database | SQLite (bundled) | Watch history, media cache |
| OAuth Server | Node.js (Express, ~120 lines) | Google OAuth code exchange |
| Relay (optional) | Cloudflare Workers | Watch Together WebSocket relay |

## Data Flow

| Action | Network | What's Sent |
|---|---|---|
| Launch app | None | — |
| Browse library | None (local SQLite) | — |
| Play local file | None | — |
| Play cloud file | `GET api.googleapis.com` | Drive file ID |
| Search metadata (with TMDB key) | `GET api.themoviedb.org` | Search query or media ID |
| Search metadata (no TMDB key, primary) | `GET v3-cinemeta.strem.io` | Search query |
| Search metadata (no TMDB key, fallback) | `GET api.imdbapi.dev` | Search query |
| Get IMDb rating | `GET api.imdbapi.dev` | IMDb ID |
| Watch Together | WebSocket → your Worker | Play/pause/seek state |
| Sign in | Redirect → OAuth backend → Google | OAuth session (5 min TTL) |

## No Telemetry

The app has zero analytics, zero crash reporting, zero tracking. There are no network requests made on launch. The full source is available for audit on [GitHub](https://github.com/SlasshyOverhere/SlasshyVault).
