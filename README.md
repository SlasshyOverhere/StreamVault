# SlasshyVault

Cloud-first desktop media library. Indexes your Google Drive, enriches with TMDB metadata, plays through MPV.

![Tauri](https://img.shields.io/badge/Tauri-v1-blue?style=flat-square)
![React](https://img.shields.io/badge/React-18-61DAFB?style=flat-square)
![Version](https://img.shields.io/badge/version-3.0.60-black?style=flat-square)

## Features

- Google Drive library indexing with background change detection
- TMDB metadata, posters, and episode grouping
- MPV playback with resume and watch history
- External streaming via addon (direct URL or Go binary)
- Stremio addon compatibility for connecting to self-hosted media catalogs
- Archive support (`.zip`, `.rar`)
- System tray, Windows notifications, toast alerts

## Tech Stack

| Layer | Technology |
|---|---|
| Frontend | React 18, TypeScript, Tailwind CSS |
| Backend | Rust, Tauri |
| Database | SQLite |
| Playback | MPV |
| Metadata | TMDB |
| Cloud | Google Drive API |

## Quick Start

```bash
# Prerequisites: Node.js 18+, Rust stable, MPV in PATH
git clone https://github.com/SlasshyOverhere/SlasshyVault.git
cd SlasshyVault
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

Installers output to `src-tauri/target/release/bundle/`.

## Project Structure

```text
├── src/                 React frontend
├── src-tauri/           Rust + Tauri backend
└── package.json
```

## Contributing

1. Fork, branch, change, PR.

## Disclaimer

SlasshyVault does not host, store, or distribute any media content. SlasshyVault is not affiliated with or endorsed by Stremio. [Stremio](https://www.stremio.com/) is an open-source media center whose addon standard is implemented here as a user convenience — SlasshyVault simply lets users connect their own addons. Users bring their own addons and self-hosted servers. SlasshyVault does not provide, endorse, or control any addon or its content. Users are solely responsible for ensuring their use complies with applicable copyright laws in their jurisdiction. The developers assume no liability for misuse.

## License

[MIT](LICENSE)
