// Client for the public `v3-cinemeta.strem.io` add-on used by Stremio.
// Endpoints (Stremio add-on convention):
//   GET https://v3-cinemeta.strem.io/meta/{type}/{id}.json
//   GET https://v3-cinemeta.strem.io/catalog/{type}/{catalog_id}.json?skip=N&genre=X&search=Y
//
// `type` ∈ {"movie", "series"}. Returns a `{meta: {...}}` envelope for meta
// queries and `{metas: [...]}` for catalog queries. Image URLs are full
// HTTPS-hosted (no separate CDN call required).
//
// Used as the primary no-TMDB-key fallback: richer data shape than imdbapi.dev
// (cast, director, imdbRating, year, runtime, releaseInfo, popularity,
// moviedb_id for TMDB cross-reference, episode listings).

use crate::http_client;
use serde::{Deserialize, Serialize};

const CINEMETA_BASE: &str = "https://v3-cinemeta.strem.io";
const CINEMETA_CATALOG_HOST: &str = "https://cinemeta-catalogs.strem.io";
const CINEMETA_IMAGES: &str = "https://images.metahub.space";
const CINEMETA_EPISODES: &str = "https://episodes.metahub.space";

// ── Public types ────────────────────────────────────────────────────────────

/// Title-shaped result returned by both meta and catalog endpoints.
///
/// Cinemeta's wire format mixes snake_case and camelCase inconsistently, so
/// every field opts into a per-field rename. `rename_all` is intentionally
/// omitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CinemetaTitle {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub released: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(rename = "imdbRating", default)]
    pub imdb_rating: Option<String>,
    #[serde(default)]
    pub imdb_id: Option<String>,
    #[serde(default)]
    pub moviedb_id: Option<i64>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub popularity: Option<f64>,
    #[serde(default)]
    pub cast: Option<Vec<String>>,
    #[serde(default)]
    pub director: Option<Vec<String>>,
    #[serde(default)]
    pub genre: Option<Vec<String>>,
    #[serde(default)]
    pub genres: Option<Vec<String>>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub awards: Option<String>,
    #[serde(rename = "releaseInfo", default)]
    pub release_info: Option<String>,
    #[serde(default)]
    pub trailers: Option<Vec<CinemetaTrailer>>,
    #[serde(default)]
    pub videos: Option<Vec<CinemetaVideo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CinemetaTrailer {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CinemetaVideo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub season: Option<i32>,
    #[serde(default)]
    pub number: Option<i32>,
    #[serde(default)]
    pub episode: Option<i32>,
    #[serde(rename = "firstAired", default)]
    pub first_aired: Option<String>,
    #[serde(default)]
    pub released: Option<String>,
    #[serde(default)]
    pub overview: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub rating: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MetaResponse {
    meta: CinemetaTitle,
}

#[derive(Debug, Deserialize)]
struct CatalogResponse {
    #[serde(default)]
    metas: Vec<CinemetaTitle>,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn encode_query(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn http_get<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let client = http_client::shared_client();
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("[CINEMETA] network error: {} ({})", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("[CINEMETA] {} -> {}", url, status));
    }
    resp.json::<T>()
        .map_err(|e| format!("[CINEMETA] failed to parse {} -> {}", url, e))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CinemetaKind {
    Movie,
    Series,
}

impl CinemetaKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CinemetaKind::Movie => "movie",
            CinemetaKind::Series => "series",
        }
    }
}

pub fn looks_like_imdb_id(id: &str) -> bool {
    crate::imdb_api::looks_like_imdb_id(id)
}

pub fn image_base() -> &'static str {
    CINEMETA_IMAGES
}

pub fn episodes_base() -> &'static str {
    CINEMETA_EPISODES
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Fetch a single title by IMDb id (e.g. `tt0133093`). Returns the parsed
/// `CinemetaTitle` inside the `{meta: ...}` envelope.
pub fn get_title(kind: CinemetaKind, imdb_id: &str) -> Result<CinemetaTitle, String> {
    let id = imdb_id.trim();
    if !looks_like_imdb_id(id) {
        return Err(format!("[CINEMETA] not an IMDb id: {}", id));
    }
    let url = format!("{}/meta/{}/{}.json", CINEMETA_BASE, kind.as_str(), id);
    let resp: MetaResponse = http_get(&url)?;
    Ok(resp.meta)
}

/// Search the catalog. Cinemeta's catalog extras include `search` so we use
/// `genre` as a fallback query string. The catalog id `top` works for both
/// movie and series.
pub fn search_catalog(
    kind: CinemetaKind,
    query: &str,
    limit: usize,
) -> Result<Vec<CinemetaTitle>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!(
        "{}/{}/{}.json?search={}&limit={}",
        CINEMETA_BASE,
        kind.as_str(),
        "top",
        encode_query(q),
        limit
    );
    let resp: CatalogResponse = http_get(&url)?;
    Ok(resp.metas.into_iter().take(limit).collect())
}

/// Probe Cinemeta first for a richer search. The basic manifest exposes
/// the `top` catalog with a `search` extra, but not a free-text endpoint.
pub fn search(
    kind: CinemetaKind,
    query: &str,
    limit: usize,
) -> Result<Vec<CinemetaTitle>, String> {
    search_catalog(kind, query, limit)
}

/// Returns the Cinemeta "Top" catalog sorted by `popularity`, useful for
/// trending suggestions when no TMDB key is set.
pub fn trending(kind: CinemetaKind, per_type_limit: usize) -> Result<Vec<CinemetaTitle>, String> {
    let url = format!(
        "{}/{}/{}.json?limit={}",
        CINEMETA_BASE,
        kind.as_str(),
        "top",
        per_type_limit
    );
    let resp: CatalogResponse = http_get(&url)?;
    Ok(resp.metas)
}

/// Try the catalog host directly (without the v3 redirect).
pub fn trending_direct(
    kind: CinemetaKind,
    per_type_limit: usize,
) -> Result<Vec<CinemetaTitle>, String> {
    let url = format!(
        "{}/{}/{}.json?limit={}",
        CINEMETA_CATALOG_HOST,
        kind.as_str(),
        "top",
        per_type_limit
    );
    let resp: CatalogResponse = http_get(&url)?;
    Ok(resp.metas)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_serde_movie() {
        let json = r#"{
            "id": "tt0133093",
            "type": "movie",
            "name": "The Matrix",
            "description": "When a beautiful stranger...",
            "year": "1999",
            "runtime": "136 min",
            "imdbRating": "8.7",
            "imdb_id": "tt0133093",
            "moviedb_id": 603,
            "poster": "https://images.metahub.space/poster/small/tt0133093/img",
            "background": "https://images.metahub.space/background/medium/tt0133093/img",
            "cast": ["Keanu Reeves", "Laurence Fishburne"],
            "director": ["Lana Wachowski", "Lilly Wachowski"],
            "genre": ["Action", "Sci-Fi"],
            "releaseInfo": "1999"
        }"#;
        let t: CinemetaTitle = serde_json::from_str(json).unwrap();
        assert_eq!(t.id, "tt0133093");
        assert_eq!(t.kind, "movie");
        assert_eq!(t.name, "The Matrix");
        assert_eq!(t.moviedb_id, Some(603));
        assert_eq!(t.imdb_rating.as_deref(), Some("8.7"));
        assert_eq!(t.cast.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn title_serde_series_with_videos() {
        let json = r#"{
            "id": "tt0944947",
            "type": "series",
            "name": "Game of Thrones",
            "year": "2011–2019",
            "imdbRating": "9.2",
            "imdb_id": "tt0944947",
            "moviedb_id": 1399,
            "videos": [
                {
                    "id": "tt0944947:1:1",
                    "name": "Winter Is Coming",
                    "season": 1,
                    "number": 1,
                    "episode": 1,
                    "released": "2011-04-17T00:00:00.000Z",
                    "thumbnail": "https://episodes.metahub.space/tt0944947/1/1/w780.jpg",
                    "overview": "Episode description"
                }
            ]
        }"#;
        let t: CinemetaTitle = serde_json::from_str(json).unwrap();
        assert_eq!(t.kind, "series");
        assert_eq!(t.moviedb_id, Some(1399));
        let v = t.videos.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].season, Some(1));
        assert_eq!(v[0].episode, Some(1));
        assert!(v[0].thumbnail.is_some());
    }

    #[test]
    fn meta_envelope_shape() {
        let json = r#"{"meta": {"id": "tt0133093", "type": "movie", "name": "The Matrix"}}"#;
        let env: MetaResponse = serde_json::from_str(json).unwrap();
        assert_eq!(env.meta.id, "tt0133093");
    }

    #[test]
    fn catalog_envelope_shape() {
        let json = r#"{"metas": [
            {"id": "tt0133093", "type": "movie", "name": "The Matrix"},
            {"id": "tt0111161", "type": "movie", "name": "The Shawshank Redemption"}
        ]}"#;
        let env: CatalogResponse = serde_json::from_str(json).unwrap();
        assert_eq!(env.metas.len(), 2);
    }

    #[test]
    fn kind_serializes_correctly() {
        assert_eq!(CinemetaKind::Movie.as_str(), "movie");
        assert_eq!(CinemetaKind::Series.as_str(), "series");
    }

    #[test]
    fn empty_search_returns_empty() {
        assert!(search(CinemetaKind::Movie, "", 10).unwrap().is_empty());
        assert!(search(CinemetaKind::Movie, "   ", 10).unwrap().is_empty());
    }
}
