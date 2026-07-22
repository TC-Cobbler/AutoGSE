use std::path::Path;

use crate::error::AutoGseError;
use crate::goldberg::steamclient_experimental_root;
use crate::ini_patch;

/// Every file staged into the TOD for `--mode steamclient` (Phase 6 §6.5).
/// Confirmed against the real vendored `steamclient_experimental/` tree and
/// its own README: this build "will act as a `steamclient`, allowing you to
/// retain the original `steam_api(64).dll`" — a fundamentally different
/// component swap than the regular mode's `steam_api(64).dll` replacement,
/// so `steam_api(64).dll` is deliberately never touched here.
const STAGED_FILES: &[&str] = &[
    "steamclient.dll",
    "steamclient64.dll",
    "steamclient_loader_x32.exe",
    "steamclient_loader_x64.exe",
    "GameOverlayRenderer.dll",
    "GameOverlayRenderer64.dll",
    "ColdClientLoader.ini",
];

/// Copies the loader fileset into `tod` and rewrites the copied
/// `ColdClientLoader.ini`'s `[SteamClient]` section (`Exe`, `AppId`,
/// `SteamClientDll`, `SteamClient64Dll`) to point at this specific game —
/// starting from the vendored template (via `ini_patch`, reused from Phase
/// 6 §6.1) rather than generating the file from scratch, so every other
/// documented key/comment keeps its vendored default.
///
/// `game_exe_relative` must be relative to `tod` (same folder both files
/// end up in, per the README: "copy the following files to any folder" /
/// "all emu config files should be put beside the `steamclient(64).dll`").
/// Returns the TOD-relative paths written, for the manifest's
/// `injected_files[]` — `backed_up_files[]` stays empty for this mode since
/// no DLL swap happens (see `main.rs::run_inject`).
pub fn stage(tod: &Path, game_exe_relative: &str, app_id: u64) -> Result<Vec<String>, AutoGseError> {
    let src_root = steamclient_experimental_root()?;
    let mut written = Vec::new();
    for name in STAGED_FILES {
        let src = src_root.join(name);
        if src.is_file() {
            std::fs::copy(&src, tod.join(name))?;
            written.push((*name).to_string());
        }
    }

    let ini_path = tod.join("ColdClientLoader.ini");
    ini_patch::set_key(&ini_path, "SteamClient", "Exe", game_exe_relative)?;
    ini_patch::set_key(&ini_path, "SteamClient", "AppId", &app_id.to_string())?;
    ini_patch::set_key(&ini_path, "SteamClient", "SteamClientDll", "steamclient.dll")?;
    ini_patch::set_key(&ini_path, "SteamClient", "SteamClient64Dll", "steamclient64.dll")?;

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn stage_copies_real_vendored_fileset() {
        let tod = TempDir::new().unwrap();
        let written = stage(tod.path(), "Game.exe", 480).unwrap();

        for name in STAGED_FILES {
            assert!(tod.path().join(name).is_file(), "{name} should have been staged");
            assert!(written.contains(&(*name).to_string()));
        }
    }

    #[test]
    fn stage_writes_expected_cold_client_loader_ini_keys() {
        let tod = TempDir::new().unwrap();
        stage(tod.path(), "subdir\\Game.exe", 1332010).unwrap();

        let content = std::fs::read_to_string(tod.path().join("ColdClientLoader.ini")).unwrap();
        assert!(content.contains("Exe=subdir\\Game.exe"));
        assert!(content.contains("AppId=1332010"));
        assert!(content.contains("SteamClientDll=steamclient.dll"));
        assert!(content.contains("SteamClient64Dll=steamclient64.dll"));
        // The vendored template's other documented sections/comments must
        // survive untouched — confirms this patches the real template
        // rather than generating a bespoke minimal file.
        assert!(content.contains("[Injection]"));
        assert!(content.contains("[Persistence]"));
    }
}
