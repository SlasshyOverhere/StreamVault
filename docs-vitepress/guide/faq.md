# FAQ

## Does SlasshyVault collect any data?

No. No telemetry, analytics, crash reports, or tracking. The app makes zero network requests on launch. The full source is available for audit.

## What does the OAuth backend do with my data?

It exchanges Google's one-time code for tokens. Stores a session in memory for 5 minutes max, then deletes it. No database, no logs, no persistence.

## Where is my data stored?

Watch history → local SQLite. Config → local JSON. Media → your Drive or local disk. Nothing is uploaded to any server.

## Can I use my own TMDB/OMDb keys?

Yes. Settings → API Keys. Both are optional. Without a TMDB key, metadata
lookups fall back to the free imdbapi.dev service. OMDb is similarly optional —
IMDb ratings default to imdbapi.dev unless you supply an OMDb key.

## How does Watch Together work?

A Cloudflare Worker on your own account relays play/pause/seek messages between participants. The 200-line worker source is bundled in the app.

## Is there a mobile version?

No. Desktop only (Windows). Built with Tauri.
