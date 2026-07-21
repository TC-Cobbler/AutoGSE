use std::time::Duration;

use serde::Deserialize;

use crate::error::AutoGseError;

/// PRD §5.3.3: Jaro-Winkler similarity threshold for an auto-accepted match.
pub const APPID_MATCH_THRESHOLD: f64 = 0.88;

/// PRD §5.3.3 originally specified `ISteamApps/GetAppList/v2` (download the
/// full catalog, fuzzy-match locally). Valve deprecated that endpoint; its
/// replacement (`IStoreService/GetAppList/v1`) requires an API key, which
/// conflicts with this project's zero-config goal. This unauthenticated
/// store-search endpoint needs no key and does its own substring/prefix
/// narrowing server-side — we still apply local Jaro-Winkler scoring to
/// whatever it returns so the >=0.88 threshold and Step 5's ranked list
/// behave identically to the originally-planned design.
const SEARCH_URL: &str = "https://store.steampowered.com/api/storesearch/";
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub appid: u64,
    pub name: String,
    pub score: f64,
}

/// Pure, network-free fuzzy matcher: scores every candidate against `name`
/// via Jaro-Winkler and returns them sorted highest-score first.
pub fn fuzzy_match(name: &str, candidates: &[(u64, String)]) -> Vec<ScoredCandidate> {
    let mut scored: Vec<ScoredCandidate> = candidates
        .iter()
        .map(|(appid, cand_name)| ScoredCandidate {
            appid: *appid,
            name: cand_name.clone(),
            score: strsim::jaro_winkler(name, cand_name),
        })
        .collect();
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

#[derive(Deserialize)]
struct StoreSearchResponse {
    items: Vec<StoreSearchItem>,
}
#[derive(Deserialize)]
struct StoreSearchItem {
    #[serde(rename = "type")]
    kind: String,
    id: u64,
    name: String,
}

fn build_agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(timeout))
        .timeout_global(Some(timeout))
        .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build())
        .build();
    ureq::Agent::new_with_config(config)
}

fn search_store(term: &str, timeout: Duration) -> Result<Vec<(u64, String)>, AutoGseError> {
    let agent = build_agent(timeout);

    let mut response = agent
        .get(SEARCH_URL)
        .query("term", term)
        .query("l", "en")
        .query("cc", "us")
        .call()
        .map_err(|e| AutoGseError::Registry(format!("steam store search failed: {e}")))?;

    let body: StoreSearchResponse = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_json()
        .map_err(|e| AutoGseError::Registry(format!("steam store search response parse failed: {e}")))?;

    Ok(body.items.into_iter().filter(|i| i.kind == "app").map(|i| (i.id, i.name)).collect())
}

/// Step 4 (PRD §5.3.3 + §8's "no internet" edge case): query Steam's public
/// store search, then score results locally via Jaro-Winkler. A network
/// failure/timeout or a query with zero results yields an empty candidate
/// list, so the caller (Step 5) falls back to manual entry only.
pub fn resolve_via_steam_api(name: &str, timeout: Duration) -> Vec<ScoredCandidate> {
    match search_store(name, timeout) {
        Ok(candidates) if !candidates.is_empty() => fuzzy_match(name, &candidates),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_candidates() -> Vec<(u64, String)> {
        vec![
            (1091500, "Cyberpunk 2077".to_string()),
            (1245620, "ELDEN RING".to_string()),
            (367520, "Hollow Knight".to_string()),
            (12345, "Cyberpunk 2077: Phantom Liberty".to_string()),
        ]
    }

    #[test]
    fn fuzzy_match_ranks_exact_name_highest() {
        let results = fuzzy_match("Cyberpunk 2077", &sample_candidates());

        assert_eq!(results[0].appid, 1091500);
        assert!(results[0].score >= APPID_MATCH_THRESHOLD);
    }

    #[test]
    fn fuzzy_match_is_sorted_descending() {
        let results = fuzzy_match("Hollow Knight", &sample_candidates());
        for pair in results.windows(2) {
            assert!(pair[0].score >= pair[1].score);
        }
    }

    #[test]
    fn fuzzy_match_handles_empty_candidate_list() {
        assert!(fuzzy_match("Anything", &[]).is_empty());
    }

    /// Manual QA only (live network call, not run in normal `cargo test`):
    /// `cargo test steam_api::tests::live_search_store -- --ignored`
    #[test]
    #[ignore]
    fn live_search_store() {
        let candidates = search_store("Cyberpunk 2077", Duration::from_millis(3000)).expect("live steam store search");
        assert!(candidates.iter().any(|(id, _)| *id == 1091500), "expected Cyberpunk 2077 (1091500) in results: {candidates:?}");

        let results = resolve_via_steam_api("Cyberpunk 2077", Duration::from_millis(3000));
        assert!(!results.is_empty());
        assert!(results[0].score >= APPID_MATCH_THRESHOLD);
    }

    #[test]
    #[ignore]
    fn live_search_store_no_results_yields_empty() {
        let results = resolve_via_steam_api("zzzzznonexistentgamezzzzz12345", Duration::from_millis(3000));
        assert!(results.is_empty());
    }
}
