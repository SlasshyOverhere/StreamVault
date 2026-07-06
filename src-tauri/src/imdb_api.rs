// Client for the free, no-key `api.imdbapi.dev` service. Endpoints exercised:
//   GET /search/titles?query={q}&limit={n}        → search titles
//   GET /titles/{ttid}                            → title detail
//   GET /titles/{ttid}/seasons                    → list seasons
//   GET /titles/{ttid}/episodes?season={n}        → list episodes
//
// Used only when the user has not configured a TMDB API key. With a TMDB
// key, `tmdb.rs` keeps making direct calls to `api.themoviedb.org`.

use crate::http_client;
use serde::{Deserialize, Serialize};

const IMDB_API_BASE: &str = "https://api.imdbapi.dev";

// All public structs use snake_case Rust field names plus a
// `rename_all = "camelCase"` serde directive, so the imdbapi.dev wire format
// deserializes directly without intermediate wrappers. `type` → `type_`
// because `type` is a Rust keyword.

/// Title detail (imdbapi.dev → `imdbapiTitle`), trimmed to fields we use.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImdbTitle {
    pub id: String,
    #[serde(rename = "type", default)]
    pub type_: Option<String>,
    #[serde(default)]
    pub primary_title: Option<String>,
    #[serde(default)]
    pub original_title: Option<String>,
    #[serde(default)]
    pub primary_image: Option<ImdbImage>,
    #[serde(default)]
    pub start_year: Option<i32>,
    #[serde(default)]
    pub end_year: Option<i32>,
    #[serde(default)]
    pub runtime_seconds: Option<i32>,
    #[serde(default)]
    pub plot: Option<String>,
    #[serde(default)]
    pub rating: Option<ImdbRating>,
    #[serde(default)]
    pub genres: Option<Vec<String>>,
    #[serde(default)]
    pub interests: Option<Vec<ImdbInterest>>,
    #[serde(default)]
    pub release_date: Option<ImdbPrecisionDate>,
    #[serde(default)]
    pub spoken_languages: Option<Vec<ImdbLanguage>>,
    #[serde(default)]
    pub countries_of_origin: Option<Vec<ImdbCountry>>,
    #[serde(default)]
    pub directors: Option<Vec<ImdbNameRef>>,
    #[serde(default)]
    pub writers: Option<Vec<ImdbNameRef>>,
    #[serde(default)]
    pub cast: Option<Vec<ImdbNameRef>>,
}

/// Single search-result row (subset of `imdbapiTitle`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImdbSearchTitle {
    pub id: String,
    #[serde(rename = "type", default)]
    pub type_: Option<String>,
    #[serde(default)]
    pub primary_title: Option<String>,
    #[serde(default)]
    pub original_title: Option<String>,
    #[serde(default)]
    pub primary_image: Option<ImdbImage>,
    #[serde(default)]
    pub start_year: Option<i32>,
    #[serde(default)]
    pub end_year: Option<i32>,
    #[serde(default)]
    pub rating: Option<ImdbRating>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImdbImage {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub width: Option<i32>,
    #[serde(default)]
    pub height: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImdbRating {
    #[serde(default)]
    pub aggregate_rating: Option<f64>,
    #[serde(default)]
    pub vote_count: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImdbPrecisionDate {
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub month: Option<i32>,
    #[serde(default)]
    pub day: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImdbLanguage {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImdbCountry {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImdbNameRef {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImdbInterest {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub is_subgenre: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImdbSeason {
    #[serde(default)]
    pub season: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub image: Option<ImdbImage>,
    #[serde(default)]
    pub episode_count: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImdbEpisode {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub primary_image: Option<ImdbImage>,
    #[serde(default)]
    pub season: Option<String>,
    #[serde(default)]
    pub episode_number: Option<i32>,
    #[serde(default)]
    pub runtime_seconds: Option<i32>,
    #[serde(default)]
    pub plot: Option<String>,
    #[serde(default)]
    pub rating: Option<ImdbRating>,
    #[serde(default)]
    pub release_date: Option<ImdbPrecisionDate>,
}

// ── Response envelopes ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SearchTitlesResponse {
    titles: Vec<ImdbSearchTitle>,
}

#[derive(Debug, Deserialize)]
struct ListSeasonsResponse {
    #[serde(default)]
    seasons: Vec<ImdbSeason>,
}

#[derive(Debug, Deserialize)]
struct ListEpisodesResponse {
    #[serde(default)]
    episodes: Vec<ImdbEpisode>,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

pub fn looks_like_imdb_id(id: &str) -> bool {
    let id = id.trim();
    id.starts_with("tt") && id.len() >= 3 && id[2..].chars().all(|c| c.is_ascii_digit())
}

fn encode_query(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn http_get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let client = http_client::shared_client();
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("[IMDB] network error: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("[IMDB] {} -> {}", url, status));
    }
    resp.json::<T>()
        .map_err(|e| format!("[IMDB] failed to parse {} -> {}", url, e))
}

// ── Public API ──────────────────────────────────────────────────────────────

pub fn search_titles(
    query: &str,
    media_type_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<ImdbSearchTitle>, String> {
    let cleaned = query.trim();
    if cleaned.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!(
        "{}/search/titles?query={}&limit={}",
        IMDB_API_BASE,
        encode_query(cleaned),
        limit.min(50)
    );

    let raw: SearchTitlesResponse = http_get_json(&url)?;
    let mut out: Vec<ImdbSearchTitle> = raw
        .titles
        .into_iter()
        .filter_map(|t| {
            if let Some(want) = media_type_filter {
                let t_typed = t.type_.as_deref().unwrap_or("");
                let pass = match want {
                    "movie" => matches!(
                        t_typed,
                        "MOVIE" | "TV_MOVIE" | "SHORT" | "VIDEO"
                    ),
                    "tv" => matches!(
                        t_typed,
                        "TV_SERIES" | "TV_MINI_SERIES" | "TV_SPECIAL"
                    ),
                    _ => true,
                };
                if !pass {
                    return None;
                }
            }
            Some(t)
        })
        .collect();
    out.sort_by(|a, b| {
        let ar = a.rating.as_ref().and_then(|r| r.aggregate_rating).unwrap_or(0.0);
        let br = b.rating.as_ref().and_then(|r| r.aggregate_rating).unwrap_or(0.0);
        br.partial_cmp(&ar).unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

pub fn get_title(title_id: &str) -> Result<ImdbTitle, String> {
    if !looks_like_imdb_id(title_id) {
        return Err(format!("[IMDB] not an IMDb id: {}", title_id));
    }
    let url = format!("{}/titles/{}", IMDB_API_BASE, title_id.trim());
    http_get_json(&url)
}

pub fn list_seasons(title_id: &str) -> Result<Vec<ImdbSeason>, String> {
    if !looks_like_imdb_id(title_id) {
        return Ok(Vec::new());
    }
    let url = format!("{}/{}/seasons", IMDB_API_BASE, title_id.trim());
    let raw: ListSeasonsResponse = http_get_json(&url)?;
    Ok(raw.seasons.into_iter().map(|s| s).collect())
}

pub fn list_episodes(title_id: &str, season: Option<u32>) -> Result<Vec<ImdbEpisode>, String> {
    if !looks_like_imdb_id(title_id) {
        return Ok(Vec::new());
    }
    let mut url = format!("{}/{}/episodes", IMDB_API_BASE, title_id.trim());
    if let Some(s) = season {
        url.push_str(&format!("?season={}", s));
    }
    let raw: ListEpisodesResponse = http_get_json(&url)?;
    Ok(raw.episodes.into_iter().map(|e| e).collect())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_imdb_id_accepts_valid() {
        assert!(looks_like_imdb_id("tt0133093"));
        assert!(looks_like_imdb_id("tt0944947"));
    }

    #[test]
    fn looks_like_imdb_id_rejects_invalid() {
        assert!(!looks_like_imdb_id("nm0000206"));
        assert!(!looks_like_imdb_id("603"));
        assert!(!looks_like_imdb_id("tt"));
        assert!(!looks_like_imdb_id(""));
        assert!(!looks_like_imdb_id("ttXYZ"));
    }

    #[test]
    fn search_response_movie() {
        let r = r#"{"titles": [{"id": "tt1", "type": "MOVIE", "primaryTitle": "A"}]}"#;
        let raw: SearchTitlesResponse = serde_json::from_str(r).unwrap();
        assert_eq!(raw.titles.len(), 1);
        assert_eq!(raw.titles[0].type_.as_deref(), Some("MOVIE"));
        assert_eq!(raw.titles[0].primary_title.as_deref(), Some("A"));
    }

    #[test]
    fn search_response_null_type() {
        let r = r#"{"titles": [{"id": "tt2", "type": null, "primaryTitle": "B"}]}"#;
        let raw: SearchTitlesResponse = serde_json::from_str(r).unwrap();
        assert_eq!(raw.titles.len(), 1);
        assert!(raw.titles[0].type_.is_none());
    }

    #[test]
    fn title_serde_basic() {
        let json = r#"{
            "id": "tt0133093",
            "type": "MOVIE",
            "primaryTitle": "The Matrix",
            "originalTitle": "The Matrix",
            "primaryImage": {"url": "https://x.example/p.jpg"},
            "startYear": 1999,
            "runtimeSeconds": 8160,
            "plot": "A hacker discovers...",
            "rating": {"aggregateRating": 8.7, "voteCount": 1700000}
        }"#;
        let title: ImdbTitle = serde_json::from_str(json).unwrap();
        assert_eq!(title.id, "tt0133093");
        assert_eq!(title.type_.as_deref(), Some("MOVIE"));
        assert_eq!(title.primary_title.as_deref(), Some("The Matrix"));
        assert_eq!(title.start_year, Some(1999));
        assert_eq!(title.runtime_seconds, Some(8160));
        assert_eq!(title.rating.unwrap().aggregate_rating, Some(8.7));
    }

    #[test]
    fn title_serde_tv_with_seasons() {
        let json = r#"{
            "id": "tt0944947",
            "type": "TV_SERIES",
            "primaryTitle": "Game of Thrones",
            "startYear": 2011,
            "endYear": 2019
        }"#;
        let t: ImdbTitle = serde_json::from_str(json).unwrap();
        assert_eq!(t.id, "tt0944947");
        assert_eq!(t.start_year, Some(2011));
        assert_eq!(t.end_year, Some(2019));
    }

    #[test]
    fn seasons_serde_basic() {
        let json = r#"{"seasons": [
            {"season": "1", "title": "Season 1", "episodeCount": 10},
            {"season": "2", "title": "Season 2", "episodeCount": 10}
        ]}"#;
        let raw: ListSeasonsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(raw.seasons.len(), 2);
        assert_eq!(raw.seasons[0].episode_count, Some(10));
    }

    #[test]
    fn episodes_serde_basic() {
        let json = r#"{"episodes": [
            {
                "id": "tt0944947-ep1",
                "title": "Winter Is Coming",
                "season": "1",
                "episodeNumber": 1,
                "runtimeSeconds": 3600,
                "releaseDate": {"year": 2011, "month": 4, "day": 17},
                "primaryImage": {"url": "https://x.example/still.jpg"}
            }
        ]}"#;
        let raw: ListEpisodesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(raw.episodes.len(), 1);
        assert_eq!(raw.episodes[0].episode_number, Some(1));
        assert_eq!(raw.episodes[0].runtime_seconds, Some(3600));
        let release = raw.episodes[0].release_date.as_ref().unwrap();
        assert_eq!(release.year, Some(2011));
        assert!(raw.episodes[0].primary_image.as_ref().unwrap().url.is_some());
    }

    #[test]
    fn search_titles_empty_query_returns_empty() {
        assert!(search_titles("", None, 10).unwrap().is_empty());
        assert!(search_titles("   ", None, 10).unwrap().is_empty());
    }
}
