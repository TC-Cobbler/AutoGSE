use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::appid_prompt;
use crate::error::AutoGseError;
use crate::sanitize;
use crate::steam_api::{self, ScoredCandidate};
use crate::version_info;

/// PRD §5.3.3's network timeout for the Steam Web API step.
const STEAM_API_TIMEOUT: Duration = Duration::from_millis(1500);

const EXE_DENYLIST_SUBSTRINGS: &[&str] =
    &["crashreportclient", "unrealcefsubprocess", "unitycrashhandler", "vcredist", "dxsetup", "installer"];

pub struct AppIdContext<'a> {
    pub tod: &'a Path,
    /// The originally clicked path (before discovery resolved it to a TOD) —
    /// used to find the *game's* exe for Steps 2/3, distinct from `tod`.
    pub exe_hint: &'a Path,
    pub override_appid: Option<u64>,
    pub interactive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIdSource {
    Override,
    LocalFile,
    PeVersionResource,
    SteamApiFuzzy,
    Manual,
}

impl AppIdSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppIdSource::Override => "override",
            AppIdSource::LocalFile => "local_file",
            AppIdSource::PeVersionResource => "pe_version",
            AppIdSource::SteamApiFuzzy => "steam_api_fuzzy",
            AppIdSource::Manual => "manual",
        }
    }
}

pub struct AppIdResolution {
    pub app_id: u64,
    pub source: AppIdSource,
    /// Best-effort display name; `None` when the cascade step that resolved
    /// the App ID carries no name (override, local file, or a raw manual
    /// App ID entry).
    pub game_title: Option<String>,
}

/// The 5-step cascade (PRD §5.3), wired to the real Steam Web API.
pub fn resolve_app_id(ctx: &AppIdContext) -> Result<AppIdResolution, AutoGseError> {
    resolve_app_id_with(ctx, |name| steam_api::resolve_via_steam_api(name, STEAM_API_TIMEOUT))
}

/// Cascade core with Step 4's network lookup injected, so tests can drive
/// every branch (including "no confident match") without a live HTTP call.
fn resolve_app_id_with<F>(ctx: &AppIdContext, steam_lookup: F) -> Result<AppIdResolution, AutoGseError>
where
    F: FnOnce(&str) -> Vec<ScoredCandidate>,
{
    if let Some(id) = ctx.override_appid {
        return Ok(AppIdResolution { app_id: id, source: AppIdSource::Override, game_title: None });
    }

    if let Some(id) = scan_local_files(ctx.tod) {
        return Ok(AppIdResolution { app_id: id, source: AppIdSource::LocalFile, game_title: None });
    }

    let game_exe = pick_game_exe(ctx.exe_hint);

    if let Some(exe) = &game_exe {
        if let Some(id) = version_info::find_appid_in_strings(exe)? {
            let game_title = version_info::extract_strings(exe).ok().and_then(|s| s.product_name);
            return Ok(AppIdResolution { app_id: id, source: AppIdSource::PeVersionResource, game_title });
        }
    }

    let raw_name = game_exe
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| d_root_of(ctx.exe_hint).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let sanitized = sanitize::sanitize_name(&raw_name);

    let candidates = steam_lookup(&sanitized);
    if let Some(best) = candidates.first() {
        if best.score >= steam_api::APPID_MATCH_THRESHOLD {
            return Ok(AppIdResolution {
                app_id: best.appid,
                source: AppIdSource::SteamApiFuzzy,
                game_title: Some(best.name.clone()),
            });
        }
    }

    if !ctx.interactive {
        return Err(AutoGseError::AppIdResolutionFailed(
            "no confident automatic match and interactive prompt disabled (--silent)".to_string(),
        ));
    }

    let (app_id, game_title) = appid_prompt::prompt_app_id_disambiguation_stdio(ctx.tod, &candidates)?;
    Ok(AppIdResolution { app_id, source: AppIdSource::Manual, game_title })
}

/// Step 1 (PRD §5.3.1): scans `tod` and its ancestors for `steam_appid.txt`,
/// both directly and under `steam_settings/`. `steam_interfaces.txt` is
/// deliberately not treated as an App ID source here — it carries no App ID,
/// despite being grouped alongside `steam_appid.txt` in the PRD's prose.
fn scan_local_files(tod: &Path) -> Option<u64> {
    for dir in tod.ancestors() {
        if let Some(id) = read_steam_appid_txt(&dir.join("steam_appid.txt")) {
            return Some(id);
        }
        if let Some(id) = read_steam_appid_txt(&dir.join("steam_settings").join("steam_appid.txt")) {
            return Some(id);
        }
    }
    None
}

fn read_steam_appid_txt(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn d_root_of(exe_hint: &Path) -> PathBuf {
    if exe_hint.is_file() {
        exe_hint.parent().map(Path::to_path_buf).unwrap_or_else(|| exe_hint.to_path_buf())
    } else {
        exe_hint.to_path_buf()
    }
}

/// Not PRD-specified: a documented heuristic for finding "the game's exe" to
/// run Steps 2/3 against, since the PRD only describes App ID discovery in
/// terms of a single ambiguous "the executable". If `exe_hint` was itself a
/// file, use it directly (the common case: user right-clicked the exe). If
/// it was a folder, pick the largest non-denylisted `.exe` at its root level
/// (main game binaries are reliably the largest; installers/crash-handlers
/// are small). Revisable once Phase 4's real-game test matrix surfaces cases
/// this gets wrong.
fn pick_game_exe(exe_hint: &Path) -> Option<PathBuf> {
    if exe_hint.is_file() {
        return Some(exe_hint.to_path_buf());
    }

    let d_root = d_root_of(exe_hint);
    let entries = std::fs::read_dir(&d_root).ok()?;

    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_exe = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
            if !is_exe {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().to_lowercase();
            if EXE_DENYLIST_SUBSTRINGS.iter().any(|d| name.contains(d)) {
                return None;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            Some((path, size))
        })
        .max_by_key(|(_, size)| *size)
        .map(|(path, _)| path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn unreachable_lookup(_name: &str) -> Vec<ScoredCandidate> {
        panic!("steam lookup should not be reached in this test");
    }

    #[test]
    fn override_short_circuits_everything() {
        let dir = TempDir::new().unwrap();
        let ctx = AppIdContext { tod: dir.path(), exe_hint: dir.path(), override_appid: Some(1245620), interactive: false };

        let resolution = resolve_app_id_with(&ctx, unreachable_lookup).unwrap();

        assert_eq!(resolution.app_id, 1245620);
        assert_eq!(resolution.source, AppIdSource::Override);
    }

    #[test]
    fn step1_finds_direct_steam_appid_txt() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("steam_appid.txt"), b"1091500\n");
        let ctx = AppIdContext { tod: dir.path(), exe_hint: dir.path(), override_appid: None, interactive: false };

        let resolution = resolve_app_id_with(&ctx, unreachable_lookup).unwrap();

        assert_eq!(resolution.app_id, 1091500);
        assert_eq!(resolution.source, AppIdSource::LocalFile);
    }

    #[test]
    fn step1_finds_steam_settings_subfolder_variant() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("steam_settings/steam_appid.txt"), b"367520");
        let ctx = AppIdContext { tod: dir.path(), exe_hint: dir.path(), override_appid: None, interactive: false };

        let resolution = resolve_app_id_with(&ctx, unreachable_lookup).unwrap();

        assert_eq!(resolution.app_id, 367520);
    }

    #[test]
    fn step1_searches_ancestor_directories() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("steam_appid.txt"), b"55");
        let nested_tod = dir.path().join("Engine/Binaries/Win64");
        std::fs::create_dir_all(&nested_tod).unwrap();
        let ctx = AppIdContext { tod: &nested_tod, exe_hint: dir.path(), override_appid: None, interactive: false };

        let resolution = resolve_app_id_with(&ctx, unreachable_lookup).unwrap();

        assert_eq!(resolution.app_id, 55);
    }

    #[test]
    fn step4_high_confidence_match_resolves_without_prompting() {
        let dir = TempDir::new().unwrap();
        let ctx = AppIdContext { tod: dir.path(), exe_hint: dir.path(), override_appid: None, interactive: false };

        let resolution = resolve_app_id_with(&ctx, |_name| {
            vec![ScoredCandidate { appid: 1091500, name: "Cyberpunk 2077".to_string(), score: 0.95 }]
        })
        .unwrap();

        assert_eq!(resolution.app_id, 1091500);
        assert_eq!(resolution.source, AppIdSource::SteamApiFuzzy);
    }

    #[test]
    fn low_confidence_match_and_non_interactive_is_an_error() {
        let dir = TempDir::new().unwrap();
        let ctx = AppIdContext { tod: dir.path(), exe_hint: dir.path(), override_appid: None, interactive: false };

        let result = resolve_app_id_with(&ctx, |_name| {
            vec![ScoredCandidate { appid: 999, name: "Something Unrelated".to_string(), score: 0.40 }]
        });

        assert!(matches!(result, Err(AutoGseError::AppIdResolutionFailed(_))));
    }

    #[test]
    fn no_candidates_and_non_interactive_is_an_error() {
        let dir = TempDir::new().unwrap();
        let ctx = AppIdContext { tod: dir.path(), exe_hint: dir.path(), override_appid: None, interactive: false };

        let result = resolve_app_id_with(&ctx, |_name| Vec::new());

        assert!(matches!(result, Err(AutoGseError::AppIdResolutionFailed(_))));
    }

    #[test]
    fn pick_game_exe_prefers_direct_file_hint() {
        let dir = TempDir::new().unwrap();
        let exe = dir.path().join("Game.exe");
        touch(&exe, b"x");

        assert_eq!(pick_game_exe(&exe), Some(exe));
    }

    #[test]
    fn pick_game_exe_skips_denylisted_and_picks_largest() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("CrashReportClient.exe"), b"x");
        touch(&dir.path().join("UnityCrashHandler64.exe"), b"x");
        touch(&dir.path().join("SmallTool.exe"), b"xx");
        touch(&dir.path().join("MainGame.exe"), &vec![0u8; 500]);

        let chosen = pick_game_exe(dir.path()).unwrap();

        assert_eq!(chosen.file_name().unwrap().to_string_lossy(), "MainGame.exe");
    }

    #[test]
    fn pick_game_exe_none_when_no_exe_present() {
        let dir = TempDir::new().unwrap();
        assert_eq!(pick_game_exe(dir.path()), None);
    }
}
