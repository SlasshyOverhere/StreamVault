# StreamVault

A modern, high-performance media library manager built with **Tauri**, **Rust**, and **React**. Automatically indexes your local video collection and Google Drive, fetches rich metadata from TMDB, and provides seamless playback through **MPV**.

![Monochrome Design](https://img.shields.io/badge/design-monochrome-black?style=flat-square)
![Tauri](https://img.shields.io/badge/Tauri-v1-blue?style=flat-square)
![React](https://img.shields.io/badge/React-18-61DAFB?style=flat-square)

## Features

### Library Management
- **Automatic Indexing** - Scans your media folders and organizes Movies & TV Shows automatically
- **Google Drive Integration** - Index your entire Google Drive with one click
- **Smart File Watcher** - Detects new files in real-time and indexes them instantly (toggleable)
- **Cloud Change Detection** - Monitors Google Drive for new content using Changes API
- **Incremental Updates** - Only indexes new content, skips already-indexed files
- **Orphan Cleanup** - Automatically removes entries for deleted files

### Metadata & Organization
- **TMDB Integration** - Fetches posters, backdrops, overviews, and ratings
- **TV Show Support** - Properly groups episodes by series and season with episode thumbnails
- **Fix Match** - Manually correct misidentified media
- **Episode Browser** - Browse seasons and episodes with full metadata

### Playback
- **MPV Integration** - Native playback of any format (MKV, MP4, AVI, HDR, etc.) without transcoding
- **Resume Playback** - Remembers your position across all media
- **Watch History** - Track what you've watched
- **Streaming Support** - Built-in Videasy player for online content

### User Experience
- **Monochrome UI** - Sleek black & white design with smooth animations
- **System Tray** - Runs in background with Windows notifications for new content
- **Onboarding** - Guided setup for first-time users
- **Context Menus** - Right-click actions for quick operations

## Tech Stack

| Layer | Technology |
|-------|------------|
| Frontend | React 18, TypeScript, TailwindCSS, Radix UI, Framer Motion |
| Backend | Rust, Tauri |
| Database | SQLite (rusqlite) |
| Playback | MPV (via IPC) |
| Metadata | TMDB API |
| Cloud | Google Drive API |

## Supported Formats

`.mkv` `.mp4` `.avi` `.mov` `.webm` `.m4v` `.wmv` `.flv` `.ts` `.m2ts`

## Prerequisites

1. **Node.js** v18+
2. **Rust** (latest stable)
3. **MPV Media Player**
   - **Windows:** Download from [mpv.io](https://mpv.io/installation/) or [SourceForge builds](https://sourceforge.net/projects/mpv-player-windows/files/) and add to system `PATH`
   - **Linux:** `sudo apt install mpv` or equivalent
   - **macOS:** `brew install mpv`

## Installation

```bash
# Clone the repository
git clone https://github.com/SlasshyOverhere/StreamVault.git
cd StreamVault

# Install dependencies
npm install

# Run in development mode
npm run tauri dev
```

## Building

```bash
# Create production build
npm run tauri build
```

Build output: `src-tauri/target/release/`

Installers will be generated in `src-tauri/target/release/bundle/`

## Configuration

### First Launch
1. Complete the onboarding wizard
2. Add your media folders (Movies, TV Shows)
3. Connect Google Drive (optional) for cloud media
4. Enter your TMDB API key (optional, for metadata)
5. Click "Scan Library" to index your content

### Settings
- **Media Folders** - Add/remove directories to scan
- **Google Drive** - Connect/disconnect cloud storage
- **TMDB API Key** - Required for poster and metadata fetching
- **Auto Indexer** - Toggle real-time file watching
- **Player Preferences** - Configure MPV behavior

### Getting a TMDB API Key
1. Create an account at [themoviedb.org](https://www.themoviedb.org/)
2. Go to Settings > API
3. Request an API key (free for personal use)
4. Copy the "API Read Access Token" into StreamVault settings

### Google Drive Setup
1. Click "Connect Google Drive" in Settings
2. Authorize StreamVault to access your Drive
3. Use "Index Drive" button in sidebar to scan your cloud media

## Project Structure

```
streamvault/
├── src/                    # React frontend
│   ├── components/         # UI components
│   ├── services/           # API & utility functions
│   └── App.tsx            # Main application
├── src-tauri/             # Rust backend
│   ├── src/
│   │   ├── main.rs        # Tauri commands & app logic
│   │   ├── database.rs    # SQLite operations
│   │   ├── media_manager.rs # Scanning & indexing
│   │   ├── watcher.rs     # File system watcher
│   │   ├── gdrive.rs      # Google Drive API client
│   │   ├── tmdb.rs        # TMDB API client
│   │   └── mpv_ipc.rs     # MPV player control
│   └── tauri.conf.json    # Tauri configuration
└── package.json
```

## How It Works

1. **Scanning** - Walks through configured media folders finding video files
2. **Cloud Sync** - Monitors Google Drive for changes using Changes API
3. **Parsing** - Extracts title, year, season/episode from filenames
4. **Metadata Fetch** - Queries TMDB for rich metadata and downloads images
5. **Database Storage** - Stores everything in local SQLite for fast access
6. **Duplicate Detection** - Uses normalized path comparison to skip already-indexed content
7. **Playback** - Launches MPV with IPC for progress tracking and resume support

## What's New in v3.0.0

- **Monochrome Design** - Complete UI overhaul with black/white/grayscale aesthetic
- **Full Drive Indexing** - Index entire Google Drive with one click
- **Simplified Cloud Settings** - Removed folder-based cloud management
- **Improved Change Detection** - Better Google Drive sync with Changes API

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

[MIT License](LICENSE)

## Acknowledgments

- [Tauri](https://tauri.app/) - Desktop app framework
- [MPV](https://mpv.io/) - Media player
- [TMDB](https://www.themoviedb.org/) - Metadata provider
- [Google Drive API](https://developers.google.com/drive) - Cloud storage
- [Radix UI](https://www.radix-ui.com/) - UI primitives
