//! Phase 8 §8.1/§8.4's save-path database — consumes the real,
//! community-maintained Ludusavi manifest (`manifest.yaml`) directly rather
//! than depending on the published `ludusavi` crate: confirmed live that
//! even as a library it pulls in tokio/reqwest/rusqlite, a real mismatch
//! with this project's synchronous, `ureq`-only style used everywhere else
//! (no async runtime anywhere in this codebase). Same "reimplement the
//! vendor's internals natively rather than drag in their whole wrapper"
//! precedent as `goldberg::generate_interfaces`/`retroachievements.rs`.
//!
//! `serde_yaml` is a maintained-but-frozen crate (its own docs mark it
//! deprecated in favor of newer alternatives) — chosen anyway since parsing
//! here is a simple one-shot deserialize with no exotic YAML features
//! needed, and it remains the most widely-used, stable option for that.
//!
//! Manifest schema (confirmed against `ludusavi-manifest`'s own README, not
//! guessed): a game entry has `files` (map of path template -> backup
//! rules), `installDir`, `registry`, and `steam.id` (the numeric Steam
//! AppID). Only `steam.id` and `files`' keys are modeled here — everything
//! else is irrelevant to save-path resolution, so it's left as opaque
//! `serde_yaml::Value` rather than guessed at field-by-field.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::credentials;
use crate::error::AutoGseError;

const MANIFEST_URL: &str = "https://raw.githubusercontent.com/mtkennerly/ludusavi-manifest/master/data/manifest.yaml";
const MANIFEST_FILENAME: &str = "ludusavi_manifest.yaml";
/// The file is confirmed >10MB (a direct fetch attempt hit a 10MB response
/// ceiling while researching this) — too large to bundle at compile time
/// (would blow past the CLI's own <15MB footprint target), so this is
/// fetched once and cached, not vendored.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;
/// A community save-path database doesn't need to be fresher than this —
/// re-fetching every run would be wasteful for a file this large.
const STALENESS_WINDOW: Duration = Duration::from_secs(30 * 24 * 60 * 60);

fn manifest_cache_path() -> Result<PathBuf, AutoGseError> {
    Ok(credentials::store_dir()?.join(MANIFEST_FILENAME))
}

fn build_agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_connect(Some(timeout))
        .timeout_global(Some(timeout))
        .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build())
        .build();
    ureq::Agent::new_with_config(config)
}

fn is_stale(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else { return true };
    let Ok(modified) = metadata.modified() else { return true };
    modified.elapsed().map(|age| age > STALENESS_WINDOW).unwrap_or(true)
}

/// Fetches and caches the manifest if the cached copy is missing, stale
/// (older than 30 days), or `force_refresh` is set; otherwise returns the
/// existing cache path unchanged. Same temp-sibling + rename atomicity
/// convention as `backup`/`header_cache`.
pub fn fetch_and_cache_manifest(force_refresh: bool) -> Result<PathBuf, AutoGseError> {
    let path = manifest_cache_path()?;
    if path.is_file() && !force_refresh && !is_stale(&path) {
        return Ok(path);
    }

    let agent = build_agent(Duration::from_secs(30));
    let mut response = agent.get(MANIFEST_URL).call().map_err(|e| AutoGseError::Ludusavi(format!("fetch failed: {e}")))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|e| AutoGseError::Ludusavi(format!("read failed: {e}")))?;

    let dir = path.parent().expect("manifest_cache_path always has a parent");
    std::fs::create_dir_all(dir)?;
    let tmp_path = dir.join(format!("{MANIFEST_FILENAME}.tmp"));
    std::fs::write(&tmp_path, &bytes)?;
    std::fs::rename(&tmp_path, &path)?;

    Ok(path)
}

#[derive(Debug, Deserialize)]
struct SteamInfo {
    id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GameEntry {
    #[serde(default)]
    files: HashMap<String, serde_yaml::Value>,
    steam: Option<SteamInfo>,
}

/// One game's raw (placeholder-unresolved) save-path templates found for a
/// given AppID.
#[derive(Debug, Clone, PartialEq)]
pub struct SavePathMatch {
    pub game_name: String,
    pub path_templates: Vec<String>,
}

/// Parses `manifest_path` and returns every game entry whose `steam.id`
/// matches `app_id` — a direct index lookup on the AppID AutoGSE has
/// already resolved (Phase 2's cascade), not fuzzy title matching against
/// 10,000+ names.
pub fn find_save_paths_for_appid(manifest_path: &Path, app_id: u64) -> Result<Vec<SavePathMatch>, AutoGseError> {
    let content = std::fs::read(manifest_path)?;
    let manifest: HashMap<String, GameEntry> = serde_yaml::from_slice(&content).map_err(|e| AutoGseError::Ludusavi(format!("parse failed: {e}")))?;

    Ok(manifest
        .into_iter()
        .filter(|(_, entry)| entry.steam.as_ref().and_then(|s| s.id) == Some(app_id))
        .map(|(game_name, entry)| SavePathMatch { game_name, path_templates: entry.files.into_keys().collect() })
        .collect())
}

/// Resolves the Windows-relevant Ludusavi placeholders against real
/// environment values. `<storeGameId>`/`<storeUserId>` are substituted from
/// `app_id`/omitted respectively (no authenticated Steam session exists to
/// supply a real Steam user ID) — a template containing `<storeUserId>` and
/// no other unresolvable placeholder still resolves everything else and
/// leaves that one token in place rather than guessing a value, so the
/// caller can decide whether the partial result is still useful.
pub fn resolve_placeholders(template: &str, app_id: u64) -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let local_appdata = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let username = std::env::var("USERNAME").ok();

    let mut resolved = template.to_string();
    let mut substitute = |token: &str, value: Option<&Path>| {
        if let Some(value) = value {
            resolved = resolved.replace(token, &value.to_string_lossy());
        }
    };

    substitute("<winDocuments>", Some(&home.join("Documents")));
    substitute("<winAppData>", appdata.as_deref());
    substitute("<winLocalAppData>", local_appdata.as_deref());
    substitute("<winLocalAppDataLow>", Some(&home.join("AppData").join("LocalLow")));
    substitute("<winPublic>", std::env::var_os("PUBLIC").map(PathBuf::from).as_deref());
    substitute("<winProgramData>", std::env::var_os("PROGRAMDATA").map(PathBuf::from).as_deref());
    substitute("<winDir>", std::env::var_os("WINDIR").map(PathBuf::from).as_deref());
    substitute("<home>", Some(&home));
    resolved = resolved.replace("<storeGameId>", &app_id.to_string());
    if let Some(username) = &username {
        resolved = resolved.replace("<osUserName>", username);
    }

    // `<root>`/`<game>`/`<base>` depend on where the user's game library
    // lives, which this function has no way to know — and `<storeUserId>`
    // needs a real authenticated Steam session this project doesn't have.
    // Leaving them unresolved (rather than guessing) means a template that
    // still contains one isn't silently treated as a real, usable path.
    if resolved.contains('<') {
        return None;
    }

    Some(PathBuf::from(resolved))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MANIFEST: &str = r#"
Stray:
  files:
    <winDocuments>/My Games/Stray/Saved/SaveGames:
      tags:
        - save
  steam:
    id: 1332010
Some Other Game:
  files:
    <winAppData>/SomeOtherGame/save.dat: {}
  steam:
    id: 999999
No Steam Info Game:
  files:
    <winAppData>/NoSteam/save.dat: {}
"#;

    #[test]
    fn find_save_paths_for_appid_matches_real_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.yaml");
        std::fs::write(&path, SAMPLE_MANIFEST).unwrap();

        let matches = find_save_paths_for_appid(&path, 1332010).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].game_name, "Stray");
        assert_eq!(matches[0].path_templates, vec!["<winDocuments>/My Games/Stray/Saved/SaveGames".to_string()]);
    }

    #[test]
    fn find_save_paths_for_appid_is_empty_when_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.yaml");
        std::fs::write(&path, SAMPLE_MANIFEST).unwrap();

        assert!(find_save_paths_for_appid(&path, 123).unwrap().is_empty());
    }

    #[test]
    fn find_save_paths_for_appid_tolerates_entries_with_no_steam_info() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.yaml");
        std::fs::write(&path, SAMPLE_MANIFEST).unwrap();

        // Must not error out just because "No Steam Info Game" has no `steam` key.
        let matches = find_save_paths_for_appid(&path, 999999).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].game_name, "Some Other Game");
    }

    #[test]
    fn resolve_placeholders_substitutes_known_tokens() {
        let resolved = resolve_placeholders("<winDocuments>/My Games/Stray/Saved", 1332010).unwrap();
        let home = std::path::PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
        assert_eq!(resolved, home.join("Documents").join("My Games").join("Stray").join("Saved"));
    }

    #[test]
    fn resolve_placeholders_returns_none_when_an_unresolvable_token_remains() {
        assert_eq!(resolve_placeholders("<root>/<game>/saves", 1332010), None);
    }

    #[test]
    fn resolve_placeholders_substitutes_store_game_id() {
        let resolved = resolve_placeholders("<winAppData>/Game/<storeGameId>/save.dat", 1332010).unwrap();
        assert!(resolved.to_string_lossy().contains("1332010"));
    }

    /// Manual QA only (real ~10MB+ network fetch, not run in normal `cargo
    /// test`): `cargo test ludusavi::tests::live_fetch_and_find_stray -- --ignored`
    #[test]
    #[ignore]
    fn live_fetch_and_find_stray() {
        let path = fetch_and_cache_manifest(false).expect("live manifest fetch");
        assert!(path.is_file());

        // Stray (1332010) — already this codebase's go-to real, known game
        // (Phase 5/7's own live achievement tests use the same AppID).
        let matches = find_save_paths_for_appid(&path, 1332010).expect("parse real manifest");
        assert!(!matches.is_empty(), "expected at least one real Ludusavi entry for Stray (1332010)");

        // Cached copy must be reused, not re-fetched, on a second call.
        let cached_again = fetch_and_cache_manifest(false).expect("cached read");
        assert_eq!(path, cached_again);
    }
}
