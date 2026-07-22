# API Keys

| Service | Default | Custom Key? |
|---|---|---|
| TMDB (metadata, posters, search) | Cinemeta → Balloonerismm (both free) | ✅ Optional |
| IMDb rating | OMDb (free) | ✅ Optional (OMDb key) |

TMDB is **optional**. When a key is configured in Settings → API Keys, the
desktop app talks directly to `api.themoviedb.org` for metadata, posters, and
search. Without a key, it transparently falls back to:

1. **[Cinemeta](https://v3-cinemeta.strem.io)** — primary no-key source.
   Rich cast / director / imdbRating / `moviedb_id` / episode thumbs, all keyed
   on `tt...` IMDb ids.
2. **[Balloonerismm](https://api.balloonerismm.workers.dev)** — last-resort
   fallback when Cinemeta returns nothing. Free TMDB-shape mirror keyed on
   the same IMDb ids; includes credits, posters, backdrops, season/episode
   listings, and trending.

The previously bundled "TMDP" backend on `slasshyvault.onrender.com` is no
longer used for metadata — the same host still runs the Google OAuth
code-exchange backend.

| Where to obtain | URL |
|---|---|
| TMDB API key or Access Token | [themoviedb.org/settings/api](https://www.themoviedb.org/settings/api) |
| OMDb API key | [omdbapi.com/apikey.aspx](https://www.omdbapi.com/apikey.aspx) |
</content>
</invoke>