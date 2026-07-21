use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::AutoGseError;
use crate::goldberg::run_with_timeout;

const DEPLOY_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct UserDirEntry {
    path: String,
    notify: bool,
}

/// Achievement Watcher's own data root — a real, external, already-installed
/// application's directory (confirmed present on this machine), not
/// AutoGSE's. Everything this module writes goes here for real; there is no
/// revert/rollback for it, unlike AutoGSE's own manifest-tracked changes.
fn achievement_watcher_dir() -> Result<PathBuf, AutoGseError> {
    let appdata = std::env::var_os("APPDATA")
        .ok_or_else(|| AutoGseError::AchievementWatcher("APPDATA environment variable is not set".to_string()))?;
    Ok(PathBuf::from(appdata).join("Achievement Watcher"))
}

/// Extracts `<out_dir>/steam_misc/extra_acw/extra_acw.zip` (only produced by
/// an authenticated `-acw` `generate_emu_config.exe` run, see `goldberg.rs`)
/// directly into Achievement Watcher's own folder via the vendored `7za.exe`
/// — mirrors `acw_helper.au3`'s one external-tool call, bypassing the rest
/// of that script (confirmed, like `generate_interfaces.au3`, to run via the
/// AutoIt layer but produce no observable effect).
///
/// Returns `Ok(false)` (not an error) if there's nothing to deploy: no
/// `-acw` data (anonymous run), Achievement Watcher isn't installed, or the
/// vendored `7za.exe` is missing. This is a best-effort enhancement, not a
/// requirement for the game to work.
pub fn deploy_schema(out_dir: &Path) -> Result<bool, AutoGseError> {
    deploy_schema_into(out_dir, &achievement_watcher_dir()?)
}

fn deploy_schema_into(out_dir: &Path, aw_dir: &Path) -> Result<bool, AutoGseError> {
    let archive = out_dir.join("steam_misc").join("extra_acw").join("extra_acw.zip");
    if !archive.is_file() || !aw_dir.is_dir() {
        return Ok(false);
    }
    let sevenzip = out_dir.join("steam_misc").join("tools").join("7za").join("7za.exe");
    if !sevenzip.is_file() {
        return Ok(false);
    }

    let mut cmd = Command::new(&sevenzip);
    cmd.arg("x").arg(&archive).arg(format!("-o{}", aw_dir.display())).arg("-aoa");
    run_with_timeout(cmd, DEPLOY_TIMEOUT, "7za.exe")?;
    Ok(true)
}

/// Minimal INI reader: pulls one `[section]`'s `key=value` out of
/// `configs.user.ini`. A real INI parser is overkill for the two keys this
/// module needs.
fn read_ini_value(content: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed[1..trimmed.len() - 1].eq_ignore_ascii_case(section);
            continue;
        }
        if !in_section || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            if k.trim().eq_ignore_ascii_case(key) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Registers the game's save-data path(s) in Achievement Watcher's
/// `userdir.db` — confirmed by direct inspection of a real installed copy
/// to be a plain JSON array of `{path, notify}` objects, so this is a
/// straightforward read-modify-write rather than the fragile line-based
/// text surgery `acw_helper.au3` did.
///
/// Mirrors that script's path-resolution rule from `configs.user.ini`'s
/// `[user::saves]` section: a non-empty `local_save_path` means the game
/// uses fully portable saves, so *both* `<tod>/<saves_folder_name>` and
/// `<tod>/<local_save_path>` get registered (matching the original script's
/// own redundancy); an empty one means Goldberg's global default applies,
/// so only `%APPDATA%/<saves_folder_name>` is registered. Idempotent — an
/// exact path already present is left untouched.
///
/// Returns `Ok(false)` if Achievement Watcher isn't installed or
/// `configs_user_ini` can't be read (best-effort, not fatal).
pub fn register_save_paths(tod: &Path, configs_user_ini: &Path) -> Result<bool, AutoGseError> {
    let appdata = std::env::var_os("APPDATA")
        .ok_or_else(|| AutoGseError::AchievementWatcher("APPDATA environment variable is not set".to_string()))?;
    register_save_paths_into(tod, configs_user_ini, &achievement_watcher_dir()?, Path::new(&appdata))
}

fn register_save_paths_into(tod: &Path, configs_user_ini: &Path, aw_dir: &Path, appdata_dir: &Path) -> Result<bool, AutoGseError> {
    if !aw_dir.is_dir() {
        return Ok(false);
    }
    let Ok(content) = std::fs::read_to_string(configs_user_ini) else {
        return Ok(false);
    };

    let saves_folder_name = read_ini_value(&content, "user::saves", "saves_folder_name").unwrap_or_else(|| "GSE Saves".to_string());
    let local_save_path_raw = read_ini_value(&content, "user::saves", "local_save_path").unwrap_or_default();
    let local_save_path = local_save_path_raw.trim().trim_start_matches("./").trim_start_matches(".\\");

    let candidate_paths: Vec<PathBuf> = if !local_save_path.is_empty() {
        vec![tod.join(&saves_folder_name), tod.join(local_save_path)]
    } else {
        vec![appdata_dir.join(&saves_folder_name)]
    };

    let cfg_dir = aw_dir.join("cfg");
    std::fs::create_dir_all(&cfg_dir)?;
    let userdir_path = cfg_dir.join("userdir.db");

    let mut entries: Vec<UserDirEntry> = if userdir_path.is_file() {
        serde_json::from_slice(&std::fs::read(&userdir_path)?).unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut changed = false;
    for candidate in candidate_paths {
        let path_str = candidate.to_string_lossy().into_owned();
        if !entries.iter().any(|e| e.path == path_str) {
            entries.push(UserDirEntry { path: path_str, notify: true });
            changed = true;
        }
    }

    if changed {
        std::fs::write(&userdir_path, serde_json::to_vec_pretty(&entries)?)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_ini_value_finds_key_in_section() {
        let content = "[user::saves]\nlocal_save_path=\nsaves_folder_name=GSE Saves\n";
        assert_eq!(read_ini_value(content, "user::saves", "saves_folder_name"), Some("GSE Saves".to_string()));
        assert_eq!(read_ini_value(content, "user::saves", "local_save_path"), Some("".to_string()));
    }

    #[test]
    fn read_ini_value_ignores_other_sections_and_comments() {
        let content = "[user::general]\nsaves_folder_name=WRONG\n[user::saves]\n# comment\nsaves_folder_name=Right Value\n";
        assert_eq!(read_ini_value(content, "user::saves", "saves_folder_name"), Some("Right Value".to_string()));
    }

    #[test]
    fn read_ini_value_missing_key_is_none() {
        let content = "[user::saves]\nsaves_folder_name=GSE Saves\n";
        assert_eq!(read_ini_value(content, "user::saves", "nonexistent_key"), None);
    }

    #[test]
    fn deploy_schema_is_noop_when_archive_missing() {
        let out_dir = tempfile::tempdir().unwrap();
        let aw_dir = tempfile::tempdir().unwrap();
        assert_eq!(deploy_schema_into(out_dir.path(), aw_dir.path()).unwrap(), false);
    }

    #[test]
    fn deploy_schema_is_noop_when_aw_not_installed() {
        let out_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(out_dir.path().join("steam_misc/extra_acw")).unwrap();
        std::fs::write(out_dir.path().join("steam_misc/extra_acw/extra_acw.zip"), b"fake").unwrap();
        let aw_dir = tempfile::tempdir().unwrap().path().join("does_not_exist");
        assert_eq!(deploy_schema_into(out_dir.path(), &aw_dir).unwrap(), false);
    }

    #[test]
    fn register_save_paths_is_noop_when_aw_not_installed() {
        let tod = tempfile::tempdir().unwrap();
        let ini = tod.path().join("configs.user.ini");
        std::fs::write(&ini, "[user::saves]\nsaves_folder_name=GSE Saves\n").unwrap();
        let aw_dir = tempfile::tempdir().unwrap().path().join("does_not_exist");
        let appdata = tempfile::tempdir().unwrap();
        assert_eq!(register_save_paths_into(tod.path(), &ini, &aw_dir, appdata.path()).unwrap(), false);
    }

    #[test]
    fn register_save_paths_registers_default_global_path_when_local_save_path_empty() {
        let tod = tempfile::tempdir().unwrap();
        let ini = tod.path().join("configs.user.ini");
        std::fs::write(&ini, "[user::saves]\nlocal_save_path=\nsaves_folder_name=GSE Saves\n").unwrap();
        let aw_dir = tempfile::tempdir().unwrap();
        let appdata = tempfile::tempdir().unwrap();

        let result = register_save_paths_into(tod.path(), &ini, aw_dir.path(), appdata.path()).unwrap();
        assert!(result);

        let userdir = std::fs::read_to_string(aw_dir.path().join("cfg/userdir.db")).unwrap();
        let entries: Vec<UserDirEntry> = serde_json::from_str(&userdir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, appdata.path().join("GSE Saves").to_string_lossy());
        assert!(entries[0].notify);
    }

    #[test]
    fn register_save_paths_registers_both_paths_when_local_save_path_set() {
        let tod = tempfile::tempdir().unwrap();
        let ini = tod.path().join("configs.user.ini");
        std::fs::write(&ini, "[user::saves]\nlocal_save_path=./MySave\nsaves_folder_name=GSE Saves\n").unwrap();
        let aw_dir = tempfile::tempdir().unwrap();
        let appdata = tempfile::tempdir().unwrap();

        let result = register_save_paths_into(tod.path(), &ini, aw_dir.path(), appdata.path()).unwrap();
        assert!(result);

        let userdir = std::fs::read_to_string(aw_dir.path().join("cfg/userdir.db")).unwrap();
        let entries: Vec<UserDirEntry> = serde_json::from_str(&userdir).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, tod.path().join("GSE Saves").to_string_lossy());
        assert_eq!(entries[1].path, tod.path().join("MySave").to_string_lossy());
    }

    #[test]
    fn register_save_paths_is_idempotent() {
        let tod = tempfile::tempdir().unwrap();
        let ini = tod.path().join("configs.user.ini");
        std::fs::write(&ini, "[user::saves]\nlocal_save_path=./MySave\nsaves_folder_name=GSE Saves\n").unwrap();
        let aw_dir = tempfile::tempdir().unwrap();
        let appdata = tempfile::tempdir().unwrap();

        register_save_paths_into(tod.path(), &ini, aw_dir.path(), appdata.path()).unwrap();
        register_save_paths_into(tod.path(), &ini, aw_dir.path(), appdata.path()).unwrap();

        let userdir = std::fs::read_to_string(aw_dir.path().join("cfg/userdir.db")).unwrap();
        let entries: Vec<UserDirEntry> = serde_json::from_str(&userdir).unwrap();
        assert_eq!(entries.len(), 2, "re-running must not duplicate entries");
    }

    #[test]
    fn register_save_paths_preserves_existing_unrelated_entries() {
        let tod = tempfile::tempdir().unwrap();
        let ini = tod.path().join("configs.user.ini");
        std::fs::write(&ini, "[user::saves]\nlocal_save_path=./MySave\nsaves_folder_name=GSE Saves\n").unwrap();
        let aw_dir = tempfile::tempdir().unwrap();
        let appdata = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(aw_dir.path().join("cfg")).unwrap();
        std::fs::write(
            aw_dir.path().join("cfg/userdir.db"),
            r#"[{"path": "E:/some/other/game", "notify": false}]"#,
        )
        .unwrap();

        register_save_paths_into(tod.path(), &ini, aw_dir.path(), appdata.path()).unwrap();

        let userdir = std::fs::read_to_string(aw_dir.path().join("cfg/userdir.db")).unwrap();
        let entries: Vec<UserDirEntry> = serde_json::from_str(&userdir).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, "E:/some/other/game");
        assert!(!entries[0].notify, "pre-existing entry must be untouched");
    }
}
