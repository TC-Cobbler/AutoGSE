//! Phase 8's GUI glue over `save_sync`/`backup_manager`. Unlike the config
//! editor/achievement viewer (pure local-disk, no thread hop needed),
//! resolving the Steam-side save path can hit the network on its first call
//! (a Ludusavi manifest fetch, §8.4) and migrate/backup/restore can move a
//! real save folder's worth of data — so every operation here runs on a
//! background thread, same `std::thread::spawn` + `invoke_from_event_loop`
//! shape as `achievements_view.rs`'s RetroAchievements fetch.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::ComponentHandle;

use autogse::backup_manager;
use autogse::error::AutoGseError;
use autogse::manifest;
use autogse::save_sync::{self, MigrateDirection};

use crate::{BackupSnapshotRow, SaveSyncDialog};

pub type DialogHolder = Rc<RefCell<Option<SaveSyncDialog>>>;

pub fn new_holder() -> DialogHolder {
    Rc::new(RefCell::new(None))
}

pub fn open(holder: &DialogHolder, tod: &Path) -> Result<(), String> {
    let dialog = SaveSyncDialog::new().map_err(|e| e.to_string())?;
    dialog.set_target_path(tod.display().to_string().into());

    {
        let weak = dialog.as_weak();
        let tod = tod.to_path_buf();
        dialog.on_migrate_to_goldberg_requested(move || run_migrate(weak.clone(), tod.clone(), MigrateDirection::ToGoldberg));
    }
    {
        let weak = dialog.as_weak();
        let tod = tod.to_path_buf();
        dialog.on_migrate_to_steam_requested(move || run_migrate(weak.clone(), tod.clone(), MigrateDirection::ToSteam));
    }
    {
        let weak = dialog.as_weak();
        let tod = tod.to_path_buf();
        dialog.on_backup_now_requested(move || run_backup(weak.clone(), tod.clone(), None));
    }
    {
        let weak = dialog.as_weak();
        let tod = tod.to_path_buf();
        dialog.on_sync_now_requested(move |cloud_folder| {
            let cloud = (!cloud_folder.trim().is_empty()).then(|| PathBuf::from(cloud_folder.as_str()));
            run_backup(weak.clone(), tod.clone(), cloud);
        });
    }
    {
        let weak = dialog.as_weak();
        let tod = tod.to_path_buf();
        dialog.on_restore_requested(move |snapshot_id| run_restore(weak.clone(), tod.clone(), snapshot_id.to_string()));
    }

    {
        let holder = holder.clone();
        dialog.window().on_close_requested(move || {
            holder.borrow_mut().take();
            slint::CloseRequestResponse::HideWindow
        });
    }

    let _ = dialog.show();
    trigger_refresh(dialog.as_weak(), tod.to_path_buf());
    *holder.borrow_mut() = Some(dialog);
    Ok(())
}

struct RefreshData {
    steam_path: String,
    goldberg_path: String,
    backups: Vec<BackupSnapshotRow>,
}

fn collect_refresh_data(tod: &Path) -> Result<RefreshData, AutoGseError> {
    let Some(m) = manifest::load(tod)? else {
        return Err(AutoGseError::NotInjected(tod.to_path_buf()));
    };
    let app_id = m.app_id.ok_or_else(|| AutoGseError::SaveSync("target's manifest has no resolved App ID recorded".to_string()))?;
    let game_title = m.game_title.clone().unwrap_or_else(|| tod.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());

    let configs_user_ini = tod.join("steam_settings").join("configs.user.ini");
    let goldberg_path = autogse::saves::goldberg_save_dir(tod, &configs_user_ini, app_id)
        .or_else(|| autogse::saves::goldberg_save_target(tod, &configs_user_ini, app_id))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(could not resolve)".to_string());

    // Best-effort: an unresolved Steam path is shown as such, not a fatal
    // error for the whole dialog — the Goldberg side and backup list are
    // still useful on their own.
    let steam_path = save_sync::resolve_steam_save_path(app_id, &game_title, None).map(|p| p.display().to_string()).unwrap_or_else(|e| format!("(unresolved: {e})"));

    let backups = backup_manager::list_backups()?
        .into_iter()
        .filter(|b| b.app_id == app_id)
        .map(|b| BackupSnapshotRow {
            id: b.id.into(),
            achievements: (if b.achievements_backed_up { "yes" } else { "no" }).into(),
            save_size: (if b.save_backed_up { "yes" } else { "no" }).into(),
        })
        .collect();

    Ok(RefreshData { steam_path, goldberg_path, backups })
}

fn trigger_refresh(weak: slint::Weak<SaveSyncDialog>, tod: PathBuf) {
    if let Some(dialog) = weak.upgrade() {
        dialog.set_busy(true);
        dialog.set_status_message("Resolving save locations...".into());
    }
    std::thread::spawn(move || {
        let result = collect_refresh_data(&tod);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(dialog) = weak.upgrade() else { return };
            match result {
                Ok(data) => {
                    dialog.set_steam_path(data.steam_path.into());
                    dialog.set_goldberg_path(data.goldberg_path.into());
                    dialog.set_backups(slint::ModelRc::new(slint::VecModel::from(data.backups)));
                    dialog.set_status_message("Ready.".into());
                }
                Err(e) => dialog.set_status_message(format!("Error: {e}").into()),
            }
            dialog.set_busy(false);
        });
    });
}

fn run_migrate(weak: slint::Weak<SaveSyncDialog>, tod: PathBuf, direction: MigrateDirection) {
    if let Some(dialog) = weak.upgrade() {
        dialog.set_busy(true);
        dialog.set_status_message("Migrating...".into());
    }
    std::thread::spawn(move || {
        let result = (|| -> Result<String, AutoGseError> {
            let Some(m) = manifest::load(&tod)? else { return Err(AutoGseError::NotInjected(tod.clone())) };
            let app_id = m.app_id.ok_or_else(|| AutoGseError::SaveSync("target's manifest has no resolved App ID recorded".to_string()))?;
            let game_title = m.game_title.clone().unwrap_or_default();
            let report = save_sync::migrate(&tod, app_id, &game_title, direction, None)?;
            Ok(match report.backed_up_destination {
                Some(b) => format!("Migrated. Previous destination contents backed up to {}.", b.display()),
                None => "Migrated.".to_string(),
            })
        })();

        let weak_for_refresh = weak.clone();
        let tod_for_refresh = tod.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(dialog) = weak.upgrade() {
                match &result {
                    Ok(msg) => dialog.set_status_message(msg.clone().into()),
                    Err(e) => dialog.set_status_message(format!("Error: {e}").into()),
                }
            }
        });
        trigger_refresh(weak_for_refresh, tod_for_refresh);
    });
}

fn run_backup(weak: slint::Weak<SaveSyncDialog>, tod: PathBuf, cloud_target: Option<PathBuf>) {
    if let Some(dialog) = weak.upgrade() {
        dialog.set_busy(true);
        dialog.set_status_message("Backing up...".into());
    }
    std::thread::spawn(move || {
        let result = (|| -> Result<String, AutoGseError> {
            let Some(m) = manifest::load(&tod)? else { return Err(AutoGseError::NotInjected(tod.clone())) };
            let app_id = m.app_id.ok_or_else(|| AutoGseError::SaveSync("target's manifest has no resolved App ID recorded".to_string()))?;
            let snapshot = backup_manager::backup(&tod, app_id, cloud_target.as_deref())?;
            Ok(format!("Backup snapshot {} created.", snapshot.manifest.id))
        })();

        let weak_for_refresh = weak.clone();
        let tod_for_refresh = tod.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(dialog) = weak.upgrade() {
                match &result {
                    Ok(msg) => dialog.set_status_message(msg.clone().into()),
                    Err(e) => dialog.set_status_message(format!("Error: {e}").into()),
                }
            }
        });
        trigger_refresh(weak_for_refresh, tod_for_refresh);
    });
}

fn run_restore(weak: slint::Weak<SaveSyncDialog>, tod: PathBuf, snapshot_id: String) {
    if let Some(dialog) = weak.upgrade() {
        dialog.set_busy(true);
        dialog.set_status_message(format!("Restoring {snapshot_id}...").into());
    }
    std::thread::spawn(move || {
        let result = backup_manager::restore(&snapshot_id, &tod).map(|()| format!("Restored snapshot {snapshot_id}."));

        let weak_for_refresh = weak.clone();
        let tod_for_refresh = tod.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(dialog) = weak.upgrade() {
                match &result {
                    Ok(msg) => dialog.set_status_message(msg.clone().into()),
                    Err(e) => dialog.set_status_message(format!("Error: {e}").into()),
                }
            }
        });
        trigger_refresh(weak_for_refresh, tod_for_refresh);
    });
}
