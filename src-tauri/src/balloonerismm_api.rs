// Client for the public TMDB-shaped proxy at
//   https://api.balloonerismm.workers.dev
//
// Endpoints exercised (no API key required):
//   GET /movie/{ttid}                       → movie detail
//   GET /movie/{ttid}/credits               → cast + crew
//   GET /movie/{ttid}/images                → posters / backdrops / stills
//   GET /tv/{ttid}                          → TV detail (includes embedded seasons[])
//   GET /tv/{ttid}/season/{n}               → season + episodes[]
//   GET /tv/{ttid}/season/{n}/episode/{m}   → single episode
//   GET /search/multi?query={q}&page={n}    → movie+tv combined search (paginated)
//   GET /search/movie?query={q}&page={n}    → movie-only
//   GET /search/tv?query={q}&page={n}       → tv-only
//   GET /popular/all|movie|tv?page={n}      → trending suggestions
//
// Free, no key, mirrors TMDB's `/3/movie/...` shape but keyed on IMDb `tt…`
// ids (so we can use the same `tt{id}` we already have from Stremio or
// Cinemeta). Replaces `imdbapi.dev` as the no-key primary metadata source —
// the same role played by TMDB when an API key is configured.
//
// Key features we lose/gain vs imdbapi.dev:
//   • Runtime is MINUTES here (was seconds) — `TmdbMetadata.runtime_seconds`
//     multiplies by 60 at the mapper.
//   • Genres are objects `{id, name}`, not strings.
//   • ids on cast/crew are `nm…` (TMDB person ints are TMDB-style numeric ids
//     but we don't use them).
//   • Search pagination via `page_token` (base64 JSON) when `has_next_page`
//     is true; we only need the first page.
//   • Errors come back as `{"success":false,"status_code":N,"status_message":"..."}`
//     on a 4xx/5xx.

use crate::http_client;
use serde::{Deserialize, Serialize};

const BASE: &str = "https://api.balloonerismm.workers.dev";

// ── Movie detail ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieDetail {
    pub id: String,            // "tt0133093"
    pub imdb_url: Option<String>,
    pub title: Option<String>,
    pub original_title: Option<String>,
    pub original_language: Option<String>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub release_date: Option<String>, // "1999-03-31"
    pub runtime: Option<i32>,          // MINUTES
    pub vote_average: Option<f64>,
    pub vote_count: Option<i64>,
    pub popularity: Option<f64>,
    #[serde(default)]
    pub genres: Option<Vec<NamedId>>,
    #[serde(default)]
    pub spoken_languages: Option<Vec<NamedLanguage>>,
    #[serde(default)]
    pub production_countries: Option<Vec<NamedCountry>>,
    #[serde(default)]
    pub production_companies: Option<Vec<ProductionCompany>>,
}

// ── TV detail ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TvDetail {
    pub id: String, // "tt0944947"
    pub imdb_url: Option<String>,
    pub name: Option<String>,
    pub original_name: Option<String>,
    pub original_language: Option<String>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub first_air_date: Option<String>,
    pub last_air_date: Option<String>,
    pub in_production: Option<bool>,
    pub number_of_episodes: Option<i32>,
    pub number_of_seasons: Option<i32>,
    #[serde(default)]
    pub seasons: Option<Vec<TvSeasonHeader>>,
    #[serde(default)]
    pub episode_run_time: Option<Vec<i32>>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>, // "TV Series"
    pub vote_average: Option<f64>,
    pub vote_count: Option<i64>,
    pub popularity: Option<f64>,
    #[serde(default)]
    pub genres: Option<Vec<NamedId>>,
    #[serde(default)]
    pub spoken_languages: Option<Vec<NamedLanguage>>,
    #[serde(default)]
    pub production_countries: Option<Vec<NamedCountry>>,
    #[serde(default)]
    pub production_companies: Option<Vec<ProductionCompany>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TvSeasonHeader {
    pub season_number: i32,
    pub label: Option<String>,
}

// ── Season + episodes ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeasonDetail {
    pub id: Option<String>,
    pub _id: Option<String>,
    pub air_date: Option<String>,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub season_number: Option<i32>,
    pub poster_path: Option<String>,
    pub vote_average: Option<f64>,
    #[serde(default)]
    pub episodes: Option<Vec<SeasonEpisode>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeasonEpisode {
    pub id: Option<String>,
    pub air_date: Option<String>,
    pub episode_number: Option<i32>,
    pub season_number: Option<i32>,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub production_code: Option<String>,
    pub runtime: Option<i32>,
    pub still_path: Option<String>,
    pub vote_average: Option<f64>,
    pub vote_count: Option<i64>,
    #[serde(default)]
    pub crew: Option<serde_json::Value>,
    #[serde(default)]
    pub guest_stars: Option<serde_json::Value>,
}

// ── Search envelopes ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPage<T> {
    pub page: i32,
    pub page_token: Option<String>,
    pub has_next_page: Option<bool>,
    pub results: Vec<T>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieSearchResult {
    pub id: String,
    pub adult: Option<bool>,
    pub backdrop_path: Option<String>,
    #[serde(default)]
    pub genre_ids: Option<Vec<String>>,
    pub original_language: Option<String>,
    pub original_title: Option<String>,
    pub overview: Option<String>,
    pub popularity: Option<f64>,
    pub poster_path: Option<String>,
    pub release_date: Option<String>,
    pub title: Option<String>,
    pub video: Option<bool>,
    pub vote_average: Option<f64>,
    pub vote_count: Option<i64>,
    #[serde(default)]
    pub media_type: Option<String>, // set by /search/multi
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TvSearchResult {
    pub id: String,
    pub adult: Option<bool>,
    pub backdrop_path: Option<String>,
    #[serde(default)]
    pub genre_ids: Option<Vec<String>>,
    pub origin_country: Option<Vec<String>>,
    pub original_language: Option<String>,
    pub original_name: Option<String>,
    pub overview: Option<String>,
    pub popularity: Option<f64>,
    pub poster_path: Option<String>,
    pub first_air_date: Option<String>,
    pub name: Option<String>,
    pub vote_average: Option<f64>,
    pub vote_count: Option<i64>,
    #[serde(default)]
    pub media_type: Option<String>,
}

// ── Credits ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditsResponse {
    pub id: Option<String>,
    #[serde(default)]
    pub cast: Option<Vec<CastMember>>,
    #[serde(default)]
    pub crew: Option<Vec<CrewMember>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CastMember {
    pub id: Option<String>,
    pub name: Option<String>,
    pub original_name: Option<String>,
    pub profile_path: Option<String>,
    pub character: Option<String>,
    pub order: Option<i32>,
    pub credit_id: Option<String>,
    #[serde(default)]
    pub known_for_department: Option<String>,
    #[serde(default)]
    pub gender: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrewMember {
    pub id: Option<String>,
    pub name: Option<String>,
    pub original_name: Option<String>,
    pub profile_path: Option<String>,
    pub job: Option<String>,
    pub department: Option<String>,
    pub credit_id: Option<String>,
}

// ── Images ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagesResponse {
    pub id: Option<String>,
    #[serde(default)]
    pub backdrops: Option<Vec<ImageEntry>>,
    #[serde(default)]
    pub logos: Option<Vec<ImageEntry>>,
    #[serde(default)]
    pub posters: Option<Vec<ImageEntry>>,
    #[serde(default)]
    pub stills: Option<Vec<ImageEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageEntry {
    pub aspect_ratio: Option<f64>,
    pub height: Option<i32>,
    pub iso_639_1: Option<String>,
    pub file_path: Option<String>,
    pub vote_average: Option<f64>,
    pub vote_count: Option<i64>,
    pub width: Option<i32>,
}

// ── Tiny shared types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedId {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedLanguage {
    pub iso_639_1: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedCountry {
    pub iso_3166_1: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductionCompany {
    pub id: Option<String>,
    pub name: Option<String>,
    pub category: Option<String>,
    pub origin_country: Option<String>,
    pub logo_path: Option<String>,
}

// ── Helpers ────────────────────────────────────────────────────────────────

pub fn looks_like_imdb_id(id: &str) -> bool {
    crate::imdb_api::looks_like_imdb_id(id) // same rule: `tt` + digits
}

fn encode_query(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn http_get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let client = http_client::shared_client();
    let resp = client
        .get(url)
        .send()
        .map_err(|e| format!("[BALLOON] network error: {} ({})", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("[BALLOON] {} -> {}", url, status));
    }
    resp.json::<T>()
        .map_err(|e| format!("[BALLOON] failed to parse {} -> {}", url, e))
}

// ── Public API ─────────────────────────────────────────────────────────────

pub fn get_movie(imdb_id: &str) -> Result<MovieDetail, String> {
    let id = imdb_id.trim();
    if !looks_like_imdb_id(id) {
        return Err(format!("[BALLOON] not an IMDb id: {}", id));
    }
    let url = format!("{}/movie/{}", BASE, id);
    http_get_json(&url)
}

pub fn get_tv(imdb_id: &str) -> Result<TvDetail, String> {
    let id = imdb_id.trim();
    if !looks_like_imdb_id(id) {
        return Err(format!("[BALLOON] not an IMDb id: {}", id));
    }
    let url = format!("{}/tv/{}", BASE, id);
    http_get_json(&url)
}

pub fn get_season(imdb_id: &str, season_number: i32) -> Result<SeasonDetail, String> {
    let id = imdb_id.trim();
    if !looks_like_imdb_id(id) {
        return Err(format!("[BALLOON] not an IMDb id: {}", id));
    }
    if season_number < 1 {
        return Err(format!("[BALLOON] invalid season number: {}", season_number));
    }
    let url = format!("{}/tv/{}/season/{}", BASE, id, season_number);
    http_get_json(&url)
}

pub fn get_episode(
    imdb_id: &str,
    season_number: i32,
    episode_number: i32,
) -> Result<SeasonEpisode, String> {
    let id = imdb_id.trim();
    if !looks_like_imdb_id(id) {
        return Err(format!("[BALLOON] not an IMDb id: {}", id));
    }
    let url = format!(
        "{}/tv/{}/season/{}/episode/{}",
        BASE, id, season_number, episode_number
    );
    http_get_json(&url)
}

pub fn get_credits(kind: &str, imdb_id: &str) -> Result<CreditsResponse, String> {
    let id = imdb_id.trim();
    if !looks_like_imdb_id(id) {
        return Err(format!("[BALLOON] not an IMDb id: {}", id));
    }
    match kind {
        "movie" | "tv" => {}
        _ => return Err(format!("[BALLOON] invalid kind: {}", kind)),
    }
    let url = format!("{}/{}/{}/credits", BASE, kind, id);
    http_get_json(&url)
}

pub fn get_images(kind: &str, imdb_id: &str) -> Result<ImagesResponse, String> {
    let id = imdb_id.trim();
    if !looks_like_imdb_id(id) {
        return Err(format!("[BALLOON] not an IMDb id: {}", id));
    }
    match kind {
        "movie" | "tv" => {}
        _ => return Err(format!("[BALLOON] invalid kind: {}", kind)),
    }
    let url = format!("{}/{}/{}/images", BASE, kind, id);
    http_get_json(&url)
}

/// Search across movie + tv on a single call. Returns first page only.
pub fn search_multi(query: &str) -> Result<SearchPage<serde_json::Value>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(SearchPage {
            page: 1,
            page_token: None,
            has_next_page: Some(false),
            results: Vec::new(),
        });
    }
    let url = format!("{}/search/multi?query={}&page=1", BASE, encode_query(q));
    http_get_json(&url)
}

/// Movie-only search. Returns first page only.
pub fn search_movie(query: &str) -> Result<SearchPage<MovieSearchResult>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(SearchPage {
            page: 1,
            page_token: None,
            has_next_page: Some(false),
            results: Vec::new(),
        });
    }
    let url = format!("{}/search/movie?query={}&page=1", BASE, encode_query(q));
    http_get_json(&url)
}

/// TV-only search. Returns first page only.
pub fn search_tv(query: &str) -> Result<SearchPage<TvSearchResult>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(SearchPage {
            page: 1,
            page_token: None,
            has_next_page: Some(false),
            results: Vec::new(),
        });
    }
    let url = format!("{}/search/tv?query={}&page=1", BASE, encode_query(q));
    http_get_json(&url)
}

/// `kind` ∈ {"all", "movie", "tv"}.
pub fn popular(kind: &str, limit: usize) -> Result<SearchPage<serde_json::Value>, String> {
    match kind {
        "all" | "movie" | "tv" => {}
        _ => return Err(format!("[BALLOON] invalid popular kind: {}", kind)),
    }
    let url = format!("{}/popular/{}?page=1", BASE, kind);
    let page: SearchPage<serde_json::Value> = http_get_json(&url)?;
    let mut p = page;
    p.results.truncate(limit);
    Ok(p)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_query_spaces_and_specials() {
        assert_eq!(encode_query("a b c"), "a%20b%20c");
        assert_eq!(encode_query("God of War"), "God%20of%20War");
    }

    #[test]
    fn looks_like_imdb_id_matches_imdb_api() {
        assert!(looks_like_imdb_id("tt0133093"));
        assert!(!looks_like_imdb_id("nm0000206"));
        assert!(!looks_like_imdb_id(""));
        assert!(!looks_like_imdb_id("tt"));
    }

    #[test]
    fn movie_detail_serde_basic() {
        let json = r#"{
            "id": "tt0133093",
            "title": "The Matrix",
            "originalTitle": "The Matrix",
            "overview": "A hacker...",
            "releaseDate": "1999-03-31",
            "runtime": 136,
            "voteAverage": 8.7,
            "voteCount": 2260995,
            "popularity": 2260995,
            "genres": [{"id":"Action","name":"Action"},{"id":"Sci-Fi","name":"Sci-Fi"}]
        }"#;
        let m: MovieDetail = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "tt0133093");
        assert_eq!(m.title.as_deref(), Some("The Matrix"));
        assert_eq!(m.runtime, Some(136));
        assert_eq!(m.vote_average, Some(8.7));
        assert_eq!(m.genres.unwrap().len(), 2);
    }

    #[test]
    fn tv_detail_serde_basic() {
        let json = r#"{
            "id": "tt0944947",
            "name": "Game of Thrones",
            "originalName": "Game of Thrones",
            "firstAirDate": "2011-04-17",
            "lastAirDate": "2019-12-31",
            "numberOfEpisodes": 74,
            "numberOfSeasons": 8,
            "seasons": [
                {"seasonNumber": 1, "label": "1"},
                {"seasonNumber": 2, "label": "2"}
            ],
            "voteAverage": 9.2
        }"#;
        let t: TvDetail = serde_json::from_str(json).unwrap();
        assert_eq!(t.id, "tt0944947");
        assert_eq!(t.name.as_deref(), Some("Game of Thrones"));
        assert_eq!(t.first_air_date.as_deref(), Some("2011-04-17"));
        assert_eq!(t.number_of_seasons, Some(8));
        let seasons = t.seasons.unwrap();
        assert_eq!(seasons.len(), 2);
        assert_eq!(seasons[0].season_number, 1);
    }

    #[test]
    fn season_detail_serde_basic() {
        let json = r#"{
            "id": "tt0944947-1",
            "name": "Season 1",
            "seasonNumber": 1,
            "episodes": [
                {
                    "id": "tt1480055",
                    "name": "Winter Is Coming",
                    "episodeNumber": 1,
                    "seasonNumber": 1,
                    "airDate": "2011-04-17",
                    "runtime": 3720,
                    "stillPath": "https://m.media-amazon.com/images/M/x.jpg",
                    "voteAverage": 8.9
                }
            ]
        }"#;
        let s: SeasonDetail = serde_json::from_str(json).unwrap();
        let eps = s.episodes.unwrap();
        assert_eq!(eps.len(), 1);
        let e0 = &eps[0];
        assert_eq!(e0.episode_number, Some(1));
        assert_eq!(e0.runtime, Some(3720));
        assert_eq!(e0.vote_average, Some(8.9));
        assert!(e0.still_path.is_some());
    }

    #[test]
    fn search_results_envelope_deserialises() {
        let json = r#"{
            "page": 1,
            "hasNextPage": true,
            "results": [
                {"id": "tt0133093", "title": "The Matrix", "voteAverage": 8.7, "posterPath": "x.jpg"}
            ]
        }"#;
        let page: SearchPage<MovieSearchResult> = serde_json::from_str(json).unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].id, "tt0133093");
        assert_eq!(page.has_next_page, Some(true));
    }

    #[test]
    fn movie_search_empty_query_returns_empty() {
        let page = search_movie("").expect("empty query returns ok");
        assert!(page.results.is_empty());
        let page = search_tv("   ").expect("whitespace query returns ok");
        assert!(page.results.is_empty());
        let page = search_multi("").expect("empty multi returns ok");
        assert!(page.results.is_empty());
    }

    #[test]
    fn popular_validates_kind() {
        assert!(popular("all", 3).is_ok());
        assert!(popular("movie", 3).is_ok());
        assert!(popular("tv", 3).is_ok());
        assert!(popular("bogus", 3).is_err());
    }
}
