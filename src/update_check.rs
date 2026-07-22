use std::time::Duration;

use serde::Deserialize;

use crate::error::AutoGseError;

/// Placeholder until AutoGSE has a real public GitHub repository — this
/// project has no git remote configured yet. The mechanism itself (HTTP
/// call, version comparison, opt-in-only gating) is fully built and unit
/// tested independent of this value; only the live network call needs a
/// real `owner/repo` slug once one exists.
const GITHUB_REPO_SLUG: &str = "REPLACE_ME/autogse";
const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate,
    UpdateAvailable { latest_version: String },
}

#[derive(Deserialize)]
struct ReleaseResponse {
    tag_name: String,
}

/// Pure, network-free comparison. `latest_tag` may carry a leading `v`
/// (common GitHub release-tag convention, e.g. `v0.2.0`) which
/// `env!("CARGO_PKG_VERSION")`'s own bare format never does.
fn compare(current: &str, latest_tag: &str) -> UpdateStatus {
    let latest = latest_tag.trim_start_matches('v');
    if latest.is_empty() || latest == current {
        UpdateStatus::UpToDate
    } else {
        UpdateStatus::UpdateAvailable { latest_version: latest.to_string() }
    }
}

/// Opt-in only (Phase 6 §6.9) — never called automatically anywhere in
/// AutoGSE, and never auto-downloads anything; this only ever reports a
/// message for the user to act on manually.
pub fn check_for_update() -> Result<UpdateStatus, AutoGseError> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO_SLUG}/releases/latest");
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(TIMEOUT))
        .timeout_global(Some(TIMEOUT))
        .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build())
        .build();
    let agent = ureq::Agent::new_with_config(config);

    let mut response = agent
        .get(&url)
        .header("User-Agent", "AutoGSE")
        .call()
        .map_err(|e| AutoGseError::ExternalToolFailed { tool: "update-check".to_string(), message: e.to_string() })?;

    let body: ReleaseResponse = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_json()
        .map_err(|e| AutoGseError::ExternalToolFailed { tool: "update-check".to_string(), message: e.to_string() })?;

    Ok(compare(env!("CARGO_PKG_VERSION"), &body.tag_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_reports_up_to_date_on_exact_match() {
        assert_eq!(compare("0.2.0", "v0.2.0"), UpdateStatus::UpToDate);
    }

    #[test]
    fn compare_handles_tag_with_no_leading_v() {
        assert_eq!(compare("0.2.0", "0.2.0"), UpdateStatus::UpToDate);
    }

    #[test]
    fn compare_reports_update_available_on_mismatch() {
        assert_eq!(compare("0.1.0", "v0.2.0"), UpdateStatus::UpdateAvailable { latest_version: "0.2.0".to_string() });
    }

    #[test]
    fn compare_treats_empty_tag_as_up_to_date_not_an_error() {
        assert_eq!(compare("0.2.0", ""), UpdateStatus::UpToDate);
    }

    /// Manual QA only (live network, real GitHub API) — will 404 until
    /// `GITHUB_REPO_SLUG` points at a real repository:
    /// `cargo test update_check::tests::live_check_for_update -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_check_for_update() {
        let result = check_for_update().unwrap();
        println!("{result:?}");
    }
}
