use std::path::PathBuf;
use std::time::Duration;

use crate::credentials;
use crate::error::AutoGseError;

/// Same reasoning as `steam_api`'s Step 4 timeout: fast enough not to stall
/// the dashboard's background scan noticeably, generous enough for a normal
/// connection to a CDN (not Steam's own, occasionally slower API).
const CDN_TIMEOUT: Duration = Duration::from_millis(3000);
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

fn build_agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(timeout))
        .timeout_global(Some(timeout))
        .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build())
        .build();
    ureq::Agent::new_with_config(config)
}

/// Phase 7 §7.2's Steam CDN header-image fetcher/cache: fetched once per
/// AppID and cached under the same `%LOCALAPPDATA%\AutoGSE\` store-directory
/// convention `credentials::store_dir()` already establishes, so the
/// dashboard doesn't re-hit the CDN on every render/rescan. Best-effort by
/// design (mirrors `acw::deploy_schema`'s convention) — a caller should treat
/// an `Err` here as "no art for this row," never a fatal dashboard error.
pub fn cached_header_path(app_id: u64) -> Result<PathBuf, AutoGseError> {
    let dir = credentials::store_dir()?.join("header_cache");
    let path = dir.join(format!("{app_id}.jpg"));
    if path.is_file() {
        return Ok(path);
    }

    let url = format!("https://cdn.akamai.steamstatic.com/steam/apps/{app_id}/header.jpg");
    let agent = build_agent(CDN_TIMEOUT);
    let mut response = agent.get(&url).call().map_err(|e| AutoGseError::HeaderFetch(format!("{url}: {e}")))?;

    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|e| AutoGseError::HeaderFetch(format!("{url}: {e}")))?;

    std::fs::create_dir_all(&dir)?;
    // Temp-sibling + rename, same atomicity convention as `backup`'s copy
    // primitive — never leaves a half-written `{app_id}.jpg` behind for a
    // concurrent reader (another dashboard refresh) to see.
    let tmp_path = dir.join(format!("{app_id}.jpg.tmp"));
    std::fs::write(&tmp_path, &bytes)?;
    std::fs::rename(&tmp_path, &path)?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manual QA only (live network call, not run in normal `cargo test`):
    /// `cargo test header_cache::tests::live_fetch_and_cache -- --ignored`
    #[test]
    #[ignore]
    fn live_fetch_and_cache() {
        // Cyberpunk 2077 (1091500) — already this codebase's go-to "known
        // real store page" example (see appid_prompt.rs's tests). Deliberately
        // *not* Spacewar (480), the usual Phase 4/6 install-smoke-test AppID:
        // confirmed live (curl) that 480 has no real store page and 404s on
        // this exact CDN path, which a real game's AppID does not.
        let path = cached_header_path(1091500).expect("live CDN fetch");
        assert!(path.is_file());
        assert!(std::fs::metadata(&path).unwrap().len() > 0);

        // Second call must hit the cache, not the network again.
        let cached_again = cached_header_path(1091500).expect("cached read");
        assert_eq!(path, cached_again);
    }
}
