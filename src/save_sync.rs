//! Phase 8 §8.2's save migration between a game's real Steam-side save
//! location and its Goldberg one. Backs up the destination first — a real
//! risk this operation introduces (migration overwrites save files),
//! unlike inject/revert, which never touch the save root at all (see this
//! module's own roadmap entry for why the originally-planned automatic
//! pre-revert/pre-inject backup hook was dropped instead of built).

use std::path::{Path, PathBuf};

use crate::backup;
use crate::error::AutoGseError;
use crate::ludusavi;
use crate::saves;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrateDirection {
    ToGoldberg,
    ToSteam,
}

pub struct MigrationReport {
    pub steam_path: PathBuf,
    pub goldberg_path: PathBuf,
    pub direction: MigrateDirection,
    pub backed_up_destination: Option<PathBuf>,
}

/// Resolves the "Steam side" save location for `app_id`: prefers a real
/// Ludusavi manifest entry (§8.4 — an already-resolved, already-existing-on-disk
/// path), falls back to the common-directory heuristic scan (§8.1) keyed
/// off `game_title`, and an explicit caller-supplied override always wins
/// outright (it's what the user told us directly, more trustworthy than
/// anything inferred).
pub fn resolve_steam_save_path(app_id: u64, game_title: &str, override_path: Option<&Path>) -> Result<PathBuf, AutoGseError> {
    if let Some(p) = override_path {
        return Ok(p.to_path_buf());
    }

    if let Ok(manifest_path) = ludusavi::fetch_and_cache_manifest(false) {
        if let Ok(matches) = ludusavi::find_save_paths_for_appid(&manifest_path, app_id) {
            for m in &matches {
                for template in &m.path_templates {
                    if let Some(resolved) = ludusavi::resolve_placeholders(template, app_id) {
                        if resolved.exists() {
                            return Ok(resolved);
                        }
                    }
                }
            }
        }
    }

    saves::common_save_directory_candidates(game_title).into_iter().next().ok_or_else(|| {
        AutoGseError::SaveSync(format!(
            "could not resolve a Steam save path for '{game_title}' (AppID {app_id}) — no Ludusavi manifest entry found on disk, \
             no common-directory match, and no --steam-path override supplied"
        ))
    })
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), AutoGseError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else if file_type.is_file() {
            backup::atomic_copy(&entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

fn copy_path(src: &Path, dst: &Path) -> Result<(), AutoGseError> {
    if src.is_dir() {
        copy_dir_recursive(src, dst)
    } else if src.is_file() {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        backup::atomic_copy(src, dst)
    } else {
        Err(AutoGseError::TargetNotFound(src.to_path_buf()))
    }
}

/// Migrates save data between a game's real Steam-side save location and
/// its Goldberg one (`<save_root>\<AppID>`), backing up whatever's
/// currently at the destination first via `backup::backup_existing_path`.
pub fn migrate(
    tod: &Path,
    app_id: u64,
    game_title: &str,
    direction: MigrateDirection,
    steam_path_override: Option<&Path>,
) -> Result<MigrationReport, AutoGseError> {
    let configs_user_ini = tod.join("steam_settings").join("configs.user.ini");
    let goldberg_path = saves::goldberg_save_dir(tod, &configs_user_ini, app_id)
        .or_else(|| saves::goldberg_save_target(tod, &configs_user_ini, app_id))
        .ok_or_else(|| AutoGseError::SaveSync("could not resolve a Goldberg save directory (missing configs.user.ini or APPDATA)".to_string()))?;
    let steam_path = resolve_steam_save_path(app_id, game_title, steam_path_override)?;

    let (source, destination) = match direction {
        MigrateDirection::ToGoldberg => (steam_path.clone(), goldberg_path.clone()),
        MigrateDirection::ToSteam => (goldberg_path.clone(), steam_path.clone()),
    };

    if !source.exists() {
        return Err(AutoGseError::TargetNotFound(source));
    }

    let backed_up_destination = backup::backup_existing_path(&destination)?;
    copy_path(&source, &destination)?;

    Ok(MigrationReport { steam_path, goldberg_path, direction, backed_up_destination })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_configs_user_ini(tod: &Path) -> PathBuf {
        let settings = tod.join("steam_settings");
        std::fs::create_dir_all(&settings).unwrap();
        let path = settings.join("configs.user.ini");
        std::fs::write(&path, "[user::saves]\r\nlocal_save_path=\r\nsaves_folder_name=GSE Saves\r\n").unwrap();
        path
    }

    #[test]
    fn resolve_steam_save_path_override_always_wins() {
        let resolved = resolve_steam_save_path(1234, "Anything", Some(Path::new("C:\\explicit\\path"))).unwrap();
        assert_eq!(resolved, PathBuf::from("C:\\explicit\\path"));
    }

    #[test]
    fn migrate_to_goldberg_backs_up_existing_destination_then_copies() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_user_ini(tod.path());
        // `saves::goldberg_save_target` resolves under the real %APPDATA%
        // since `migrate` doesn't take an injectable appdata override —
        // acceptable for this test since we only assert relative behavior
        // (backup-then-copy), not a specific absolute path. Clean up after.
        let steam_dir = tempfile::tempdir().unwrap();
        std::fs::write(steam_dir.path().join("save.dat"), b"steam save data").unwrap();

        let goldberg_target = saves::goldberg_save_target(tod.path(), &tod.path().join("steam_settings/configs.user.ini"), 999_999_101).unwrap();
        std::fs::create_dir_all(&goldberg_target).unwrap();
        std::fs::write(goldberg_target.join("old.dat"), b"pre-existing goldberg save").unwrap();

        let report = migrate(tod.path(), 999_999_101, "Test Game", MigrateDirection::ToGoldberg, Some(steam_dir.path())).unwrap();

        assert_eq!(report.direction, MigrateDirection::ToGoldberg);
        assert!(report.backed_up_destination.is_some(), "pre-existing goldberg save dir must be backed up");
        assert_eq!(std::fs::read(goldberg_target.join("save.dat")).unwrap(), b"steam save data");
        assert!(!goldberg_target.join("old.dat").exists(), "old contents must be moved into the backup, not left mixed in");

        // Clean up the real-APPDATA-rooted fixture this test created.
        std::fs::remove_dir_all(&goldberg_target).ok();
        if let Some(backup) = report.backed_up_destination {
            std::fs::remove_dir_all(backup).ok();
        }
    }

    #[test]
    fn migrate_to_steam_copies_goldberg_save_to_steam_path() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_user_ini(tod.path());
        let steam_dir = tempfile::tempdir().unwrap();

        let goldberg_target = saves::goldberg_save_target(tod.path(), &tod.path().join("steam_settings/configs.user.ini"), 999_999_102).unwrap();
        std::fs::create_dir_all(&goldberg_target).unwrap();
        std::fs::write(goldberg_target.join("save.dat"), b"goldberg save data").unwrap();
        // Steam dir starts empty (no backup expected).
        std::fs::remove_dir(steam_dir.path()).unwrap();

        let report = migrate(tod.path(), 999_999_102, "Test Game", MigrateDirection::ToSteam, Some(steam_dir.path())).unwrap();

        assert_eq!(report.direction, MigrateDirection::ToSteam);
        assert!(report.backed_up_destination.is_none(), "nothing existed at the destination yet");
        assert_eq!(std::fs::read(steam_dir.path().join("save.dat")).unwrap(), b"goldberg save data");

        std::fs::remove_dir_all(&goldberg_target).ok();
    }

    #[test]
    fn migrate_errors_when_source_does_not_exist() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_user_ini(tod.path());
        let missing_steam_dir = tempfile::tempdir().unwrap().path().join("does_not_exist");

        let result = migrate(tod.path(), 999_999_103, "Test Game", MigrateDirection::ToGoldberg, Some(&missing_steam_dir));
        assert!(matches!(result, Err(AutoGseError::TargetNotFound(_))));
    }
}
