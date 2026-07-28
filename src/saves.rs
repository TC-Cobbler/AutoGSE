//! Phase 8 §8.1's shared save-path resolution — extracted from logic
//! `acw.rs::register_save_paths`/`achievements.rs::resolve_unlock_state_path`
//! each independently duplicated (the `configs.user.ini`-driven save-root
//! rule confirmed live in Phase 5/7: a non-empty `local_save_path` means
//! fully portable saves under the game's own folder; empty means Goldberg's
//! global `%APPDATA%\<saves_folder_name>` default). Both call sites now call
//! [`candidate_save_roots`] instead of maintaining their own copy.

use std::path::{Path, PathBuf};

use crate::ini_patch::{self, IniSection};
use crate::sanitize;

fn find_value(sections: &[IniSection], section: &str, key: &str) -> Option<String> {
    sections
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(section))
        .and_then(|s| s.entries.iter().find(|e| e.key.eq_ignore_ascii_case(key)))
        .map(|e| e.value.clone())
}

/// Every candidate save ROOT (not yet descended into a specific game's
/// `<AppID>` subfolder) `configs.user.ini` implies for `tod`, in priority
/// order. Returns an empty list (not an error) if `configs_user_ini` can't
/// be read — every caller already treats "can't resolve" as a soft failure,
/// not a hard one.
pub fn candidate_save_roots(tod: &Path, configs_user_ini: &Path, appdata_dir: &Path) -> Vec<PathBuf> {
    let Ok(sections) = ini_patch::read_all(configs_user_ini) else {
        return Vec::new();
    };

    let saves_folder_name = find_value(&sections, "user::saves", "saves_folder_name").unwrap_or_else(|| "GSE Saves".to_string());
    let local_save_path_raw = find_value(&sections, "user::saves", "local_save_path").unwrap_or_default();
    let local_save_path = local_save_path_raw.trim().trim_start_matches("./").trim_start_matches(".\\");

    if !local_save_path.is_empty() {
        vec![tod.join(&saves_folder_name), tod.join(local_save_path)]
    } else {
        vec![appdata_dir.join(&saves_folder_name)]
    }
}

/// The resolved Goldberg save directory for one game — `<candidate_root>\<AppID>`,
/// the first candidate that actually exists on disk. `None` if the game has
/// never been launched yet (no save folder created) or `configs_user_ini`
/// can't be resolved at all. Reads `APPDATA` from the real environment;
/// [`goldberg_save_dir_in`] takes it as a parameter for testing.
pub fn goldberg_save_dir(tod: &Path, configs_user_ini: &Path, app_id: u64) -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    goldberg_save_dir_in(tod, configs_user_ini, app_id, Path::new(&appdata))
}

pub(crate) fn goldberg_save_dir_in(tod: &Path, configs_user_ini: &Path, app_id: u64, appdata_dir: &Path) -> Option<PathBuf> {
    candidate_save_roots(tod, configs_user_ini, appdata_dir)
        .into_iter()
        .map(|root| root.join(app_id.to_string()))
        .find(|p| p.is_dir())
}

/// Same candidate-root resolution as [`goldberg_save_dir`], but returns the
/// *first* candidate regardless of whether it exists yet — for Phase 8
/// §8.2's migration destination, which doesn't exist until the migration
/// itself creates it, unlike `goldberg_save_dir`'s "must already exist"
/// contract (built for reading an already-running game's save data).
pub fn goldberg_save_target(tod: &Path, configs_user_ini: &Path, app_id: u64) -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    candidate_save_roots(tod, configs_user_ini, Path::new(&appdata)).into_iter().next().map(|root| root.join(app_id.to_string()))
}

/// Best-effort heuristic scan of the common Windows save-data locations
/// (Phase 8 §8.1 — correcting the roadmap's original "parse `generate_emu_config`
/// depot metadata" premise: confirmed against the vendored README that
/// `depots.txt` only ever carries DLC-ownership depot IDs, nothing about
/// save paths) for a subfolder matching `game_title`. Intended as a fallback
/// for games the Ludusavi manifest (§8.4) doesn't cover. Reuses
/// `sanitize::sanitize_name` so the same folder-name-vs-title fuzziness
/// this project already applies to App ID resolution (Phase 2 §5.3.3)
/// governs the match here too, rather than a second ad hoc rule.
pub fn common_save_directory_candidates(game_title: &str) -> Vec<PathBuf> {
    let needle = sanitize::sanitize_name(game_title).to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let profile = PathBuf::from(profile);
        roots.push(profile.join("Saved Games"));
        roots.push(profile.join("Documents"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(appdata));
    }
    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local_appdata));
    }

    scan_roots_for_match(&roots, &needle)
}

fn scan_roots_for_match(roots: &[PathBuf], needle: &str) -> Vec<PathBuf> {
    let mut matches = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if sanitize::sanitize_name(name).to_lowercase() == needle {
                matches.push(path);
            }
        }
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_configs_user_ini(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join("configs.user.ini");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn candidate_save_roots_global_default_when_local_save_path_empty() {
        let tod = tempfile::tempdir().unwrap();
        let ini = write_configs_user_ini(tod.path(), "[user::saves]\r\nlocal_save_path=\r\nsaves_folder_name=GSE Saves\r\n");
        let appdata = tempfile::tempdir().unwrap();

        let roots = candidate_save_roots(tod.path(), &ini, appdata.path());
        assert_eq!(roots, vec![appdata.path().join("GSE Saves")]);
    }

    #[test]
    fn candidate_save_roots_portable_when_local_save_path_set() {
        let tod = tempfile::tempdir().unwrap();
        let ini = write_configs_user_ini(tod.path(), "[user::saves]\r\nlocal_save_path=./MySave\r\nsaves_folder_name=GSE Saves\r\n");
        let appdata = tempfile::tempdir().unwrap();

        let roots = candidate_save_roots(tod.path(), &ini, appdata.path());
        assert_eq!(roots, vec![tod.path().join("GSE Saves"), tod.path().join("MySave")]);
    }

    #[test]
    fn candidate_save_roots_is_empty_when_ini_missing() {
        let tod = tempfile::tempdir().unwrap();
        let appdata = tempfile::tempdir().unwrap();
        assert!(candidate_save_roots(tod.path(), &tod.path().join("does_not_exist.ini"), appdata.path()).is_empty());
    }

    #[test]
    fn goldberg_save_dir_in_finds_existing_appid_folder() {
        let tod = tempfile::tempdir().unwrap();
        let ini = write_configs_user_ini(tod.path(), "[user::saves]\r\nlocal_save_path=\r\nsaves_folder_name=GSE Saves\r\n");
        let appdata = tempfile::tempdir().unwrap();
        let save_dir = appdata.path().join("GSE Saves").join("1332010");
        std::fs::create_dir_all(&save_dir).unwrap();

        assert_eq!(goldberg_save_dir_in(tod.path(), &ini, 1332010, appdata.path()), Some(save_dir));
    }

    #[test]
    fn goldberg_save_dir_in_is_none_when_no_candidate_exists() {
        let tod = tempfile::tempdir().unwrap();
        let ini = write_configs_user_ini(tod.path(), "[user::saves]\r\nlocal_save_path=\r\nsaves_folder_name=GSE Saves\r\n");
        let appdata = tempfile::tempdir().unwrap();

        assert_eq!(goldberg_save_dir_in(tod.path(), &ini, 1332010, appdata.path()), None);
    }

    #[test]
    fn scan_roots_for_match_finds_sanitized_name_match() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Hollow Knight")).unwrap();
        std::fs::create_dir_all(root.path().join("Some Other Game")).unwrap();

        let matches = scan_roots_for_match(&[root.path().to_path_buf()], "hollow knight");
        assert_eq!(matches, vec![root.path().join("Hollow Knight")]);
    }

    #[test]
    fn scan_roots_for_match_matches_through_noise_words() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Hollow Knight GOG")).unwrap();

        let matches = scan_roots_for_match(&[root.path().to_path_buf()], "hollow knight");
        assert_eq!(matches, vec![root.path().join("Hollow Knight GOG")]);
    }

    #[test]
    fn scan_roots_for_match_is_empty_when_root_does_not_exist() {
        let matches = scan_roots_for_match(&[PathBuf::from("Z:\\definitely\\does\\not\\exist")], "anything");
        assert!(matches.is_empty());
    }
}
