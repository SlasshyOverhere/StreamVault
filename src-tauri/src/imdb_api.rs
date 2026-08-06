// Lightweight IMDb-id helper. The actual no-key metadata service is now
// `balloonerismm_api` (api.balloonerismm.workers.dev); this module is kept
// around because `looks_like_imdb_id("tt1234")` is shared with
// `cinemeta_api`, `balloonerismm_api`, and `tmdb.rs`.
//
// All remote endpoints previously hosted at `https://api.imdbapi.dev/*`
// have been removed — the user reported the service is dead, and balloonerismm
// has a strictly larger dataset (posters, backdrops, credits, season/episode,
// release dates, genres, recommendations).

pub fn looks_like_imdb_id(id: &str) -> bool {
    let id = id.trim();
    id.starts_with("tt") && id.len() >= 3 && id[2..].chars().all(|c| c.is_ascii_digit())
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
}
