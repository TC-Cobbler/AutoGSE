//! Phase 8 §8.5's local achievement/save backup & restore — corrected
//! scope, not built as originally worded in two ways:
//!
//! - The roadmap's original file-location guesses (`stats/achievements-unlock.json`,
//!   `user_stats.ini`) are the exact ones Phase 7 §7.5 already refuted with
//!   live evidence. This module reuses `achievements::resolve_unlock_state_path`/
//!   `unlock_state_target_path`/`load_unlock_state` — it doesn't reinvent
//!   file discovery.
//! - "Cloud" is scoped to any local folder path — including an
//!   already-installed OneDrive/Google Drive/Dropbox sync folder, which is
//!   just an ordinary folder on disk once that client is running. No cloud
//!   API/OAuth integration exists anywhere in this project.
//!
//! There is deliberately no automatic pre-revert/pre-inject backup hook:
//! `engine::run_revert_single` only ever deletes `injected_files` and
//! `steam_settings/` inside the TOD — it never touches the save root
//! (`GSE Saves\<AppID>\`), which lives entirely outside the TOD. Inject/
//! revert cycles don't actually endanger achievement progress at all, so
//! there's no real risk there to guard against. `save_sync::migrate`'s own
//! backup-before-copy (a real risk — migration overwrites save files) is
//! the one automatic safety net this phase adds.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::achievements;
use crate::credentials;
use crate::error::AutoGseError;
use crate::saves;

const MANIFEST_FILENAME: &str = "backup_manifest.json";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct BackupManifest {
    pub id: String,
    pub tod: String,
    pub app_id: u64,
    pub achievements_backed_up: bool,
    pub save_backed_up: bool,
}

fn backups_root() -> Result<PathBuf, AutoGseError> {
    Ok(credentials::store_dir()?.join("backups"))
}

/// A millisecond timestamp alone isn't collision-safe — two backups (of the
/// same or different games) started within the same millisecond would
/// otherwise land in the same snapshot folder and corrupt each other.
/// Confirmed the hard way: this project's own parallel test runner hit
/// exactly this collision between two unrelated tests before `app_id` was
/// folded into the id. Real usage backing up the same game twice in rapid
/// succession is exactly as exposed, so this isn't just a test-only fix.
fn snapshot_id(app_id: u64) -> String {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{millis}_{app_id}")
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
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

pub struct BackupSnapshot {
    pub path: PathBuf,
    pub manifest: BackupManifest,
}

/// Snapshots the Goldberg unlock-state file and the resolved Goldberg save
/// directory into a new timestamped folder under
/// `%LOCALAPPDATA%\AutoGSE\backups\<id>\`. Neither piece being present is
/// an error — a target with no achievement data or no save created yet
/// still gets a (partial, honestly recorded) snapshot rather than failing
/// outright. If `cloud_target` is given, the same snapshot is additionally
/// recursively copied there too.
pub fn backup(tod: &Path, app_id: u64, cloud_target: Option<&Path>) -> Result<BackupSnapshot, AutoGseError> {
    let id = snapshot_id(app_id);
    let snapshot_dir = backups_root()?.join(&id);
    std::fs::create_dir_all(&snapshot_dir)?;

    let configs_user_ini = tod.join("steam_settings").join("configs.user.ini");

    let achievements_backed_up = match achievements::resolve_unlock_state_path(tod, &configs_user_ini, app_id) {
        Some(unlock_path) if unlock_path.is_file() => {
            std::fs::copy(&unlock_path, snapshot_dir.join("achievements.json"))?;
            true
        }
        _ => false,
    };

    let save_backed_up = match saves::goldberg_save_dir(tod, &configs_user_ini, app_id) {
        Some(save_dir) => {
            copy_dir_recursive(&save_dir, &snapshot_dir.join("save"))?;
            true
        }
        None => false,
    };

    let manifest = BackupManifest { id: id.clone(), tod: tod.display().to_string(), app_id, achievements_backed_up, save_backed_up };
    std::fs::write(snapshot_dir.join(MANIFEST_FILENAME), serde_json::to_vec_pretty(&manifest)?)?;

    if let Some(cloud_target) = cloud_target {
        copy_dir_recursive(&snapshot_dir, &cloud_target.join(&id))?;
    }

    Ok(BackupSnapshot { path: snapshot_dir, manifest })
}

/// Every local snapshot on record, newest first. Skips (doesn't error on)
/// any folder under the backups root that isn't a valid snapshot — e.g. a
/// stray file a user dropped there.
pub fn list_backups() -> Result<Vec<BackupManifest>, AutoGseError> {
    let root = backups_root()?;
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut manifests = Vec::new();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Ok(bytes) = std::fs::read(entry.path().join(MANIFEST_FILENAME)) {
            if let Ok(manifest) = serde_json::from_slice::<BackupManifest>(&bytes) {
                manifests.push(manifest);
            }
        }
    }
    manifests.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(manifests)
}

/// Restores one snapshot's achievement/save data back onto `tod`. Uses
/// `unlock_state_target_path`/`goldberg_save_target` (not the "must already
/// exist" resolvers) since the destination may not exist at restore time —
/// a fresh reinstall, or exactly the data loss being restored from.
pub fn restore(snapshot_id: &str, tod: &Path) -> Result<(), AutoGseError> {
    let snapshot_dir = backups_root()?.join(snapshot_id);
    let manifest: BackupManifest = serde_json::from_slice(&std::fs::read(snapshot_dir.join(MANIFEST_FILENAME))?)?;
    let configs_user_ini = tod.join("steam_settings").join("configs.user.ini");

    if manifest.achievements_backed_up {
        if let Some(unlock_path) = achievements::unlock_state_target_path(tod, &configs_user_ini, manifest.app_id) {
            if let Some(parent) = unlock_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(snapshot_dir.join("achievements.json"), &unlock_path)?;
        }
    }

    if manifest.save_backed_up {
        if let Some(save_dir) = saves::goldberg_save_target(tod, &configs_user_ini, manifest.app_id) {
            copy_dir_recursive(&snapshot_dir.join("save"), &save_dir)?;
        }
    }

    Ok(())
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

    /// Every test here uses a throwaway high AppID against the *real*
    /// `%APPDATA%`/backups root (same convention `achievements.rs`'s
    /// `seen_unlocks_round_trips_through_save_load` already established) —
    /// unlikely to collide with anything real, cleaned up after itself.
    fn cleanup_appdata_save(app_id: u64) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let _ = std::fs::remove_dir_all(PathBuf::from(appdata).join("GSE Saves").join(app_id.to_string()));
        }
    }

    fn cleanup_backup(id: &str) {
        if let Ok(root) = backups_root() {
            let _ = std::fs::remove_dir_all(root.join(id));
        }
    }

    #[test]
    fn backup_snapshots_both_achievements_and_save_when_both_exist() {
        let app_id = 999_999_201u64;
        let tod = tempfile::tempdir().unwrap();
        write_configs_user_ini(tod.path());

        let unlock_path = achievements::unlock_state_target_path(tod.path(), &tod.path().join("steam_settings/configs.user.ini"), app_id).unwrap();
        std::fs::create_dir_all(unlock_path.parent().unwrap()).unwrap();
        std::fs::write(&unlock_path, r#"{"ACH_001":{"earned":true,"earned_time":1}}"#).unwrap();
        std::fs::write(unlock_path.parent().unwrap().join("save.dat"), b"save data").unwrap();

        let snapshot = backup(tod.path(), app_id, None).unwrap();

        assert!(snapshot.manifest.achievements_backed_up);
        assert!(snapshot.manifest.save_backed_up);
        assert_eq!(std::fs::read(snapshot.path.join("achievements.json")).unwrap(), std::fs::read(&unlock_path).unwrap());
        assert_eq!(std::fs::read(snapshot.path.join("save").join("save.dat")).unwrap(), b"save data");

        cleanup_backup(&snapshot.manifest.id);
        cleanup_appdata_save(app_id);
    }

    #[test]
    fn backup_records_partial_snapshot_when_nothing_exists_yet() {
        let app_id = 999_999_202u64;
        let tod = tempfile::tempdir().unwrap();
        write_configs_user_ini(tod.path());

        let snapshot = backup(tod.path(), app_id, None).unwrap();

        assert!(!snapshot.manifest.achievements_backed_up);
        assert!(!snapshot.manifest.save_backed_up);

        cleanup_backup(&snapshot.manifest.id);
    }

    #[test]
    fn backup_also_copies_to_cloud_target_when_given() {
        let app_id = 999_999_203u64;
        let tod = tempfile::tempdir().unwrap();
        write_configs_user_ini(tod.path());
        let cloud_dir = tempfile::tempdir().unwrap();

        let unlock_path = achievements::unlock_state_target_path(tod.path(), &tod.path().join("steam_settings/configs.user.ini"), app_id).unwrap();
        std::fs::create_dir_all(unlock_path.parent().unwrap()).unwrap();
        std::fs::write(&unlock_path, r#"{"ACH_001":{"earned":true,"earned_time":1}}"#).unwrap();

        let snapshot = backup(tod.path(), app_id, Some(cloud_dir.path())).unwrap();

        assert!(cloud_dir.path().join(&snapshot.manifest.id).join("achievements.json").is_file());

        cleanup_backup(&snapshot.manifest.id);
        cleanup_appdata_save(app_id);
    }

    #[test]
    fn restore_round_trips_achievements_and_save() {
        let app_id = 999_999_204u64;
        let tod = tempfile::tempdir().unwrap();
        write_configs_user_ini(tod.path());

        let unlock_path = achievements::unlock_state_target_path(tod.path(), &tod.path().join("steam_settings/configs.user.ini"), app_id).unwrap();
        std::fs::create_dir_all(unlock_path.parent().unwrap()).unwrap();
        std::fs::write(&unlock_path, r#"{"ACH_001":{"earned":true,"earned_time":1}}"#).unwrap();
        std::fs::write(unlock_path.parent().unwrap().join("save.dat"), b"original save").unwrap();

        let snapshot = backup(tod.path(), app_id, None).unwrap();

        // Simulate progress loss: unlock state resets, save is wiped.
        std::fs::write(&unlock_path, r#"{"ACH_001":{"earned":false,"earned_time":0}}"#).unwrap();
        std::fs::remove_file(unlock_path.parent().unwrap().join("save.dat")).unwrap();

        restore(&snapshot.manifest.id, tod.path()).unwrap();

        assert_eq!(std::fs::read_to_string(&unlock_path).unwrap(), r#"{"ACH_001":{"earned":true,"earned_time":1}}"#);
        assert_eq!(std::fs::read(unlock_path.parent().unwrap().join("save.dat")).unwrap(), b"original save");

        cleanup_backup(&snapshot.manifest.id);
        cleanup_appdata_save(app_id);
    }

    #[test]
    fn list_backups_finds_recorded_snapshot_and_skips_stray_entries() {
        let app_id = 999_999_205u64;
        let tod = tempfile::tempdir().unwrap();
        write_configs_user_ini(tod.path());

        let snapshot = backup(tod.path(), app_id, None).unwrap();

        // A stray, non-snapshot folder under the backups root must not
        // cause `list_backups` to error out.
        let root = backups_root().unwrap();
        std::fs::create_dir_all(root.join("not_a_real_snapshot")).unwrap();

        let backups = list_backups().unwrap();
        assert!(backups.iter().any(|b| b.id == snapshot.manifest.id));

        cleanup_backup(&snapshot.manifest.id);
        let _ = std::fs::remove_dir_all(root.join("not_a_real_snapshot"));
        cleanup_appdata_save(app_id);
    }
}
