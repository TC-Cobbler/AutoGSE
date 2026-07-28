//! Phase 7 §7.5's GUI glue: converts `autogse::achievements`' plain Rust
//! types into the Slint-generated `AchievementsDialog`'s model types, and
//! keeps a live filesystem watch on the target's runtime unlock-state file
//! (see `achievements.rs`'s own doc comment for how that path is resolved
//! and why it's trustworthy) so the panel updates without polling.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use slint::ComponentHandle;

use std::time::Duration;

use autogse::achievements::{self, UnlockWatcher};
use autogse::manifest;
use autogse::retroachievements;

use crate::{AchievementRow, AchievementsDialog};

/// Holds the dialog's strong handle *and* the live watcher for its whole
/// open lifetime — opening a different target drops both (closing the old
/// dialog and stopping the old watch) before starting fresh. The watcher
/// itself is `None` when no App ID could be resolved for this target (never
/// injected, or a `steamclient`-mode target with no manifest App ID) — there
/// is nothing to watch for.
pub struct Session {
    #[allow(dead_code)] // kept alive purely for its Drop (closes the window); never read directly
    dialog: AchievementsDialog,
    _watcher: Option<UnlockWatcher>,
}

pub type DialogHolder = Rc<RefCell<Option<Session>>>;

pub fn new_holder() -> DialogHolder {
    Rc::new(RefCell::new(None))
}

/// Opens (or replaces) the achievement viewer for `tod`. Returns a plain
/// error message, same convention as `config_editor_view::open` — the only
/// caller just displays whatever comes back as a status line.
pub fn open(holder: &DialogHolder, tod: &Path) -> Result<(), String> {
    let dialog = AchievementsDialog::new().map_err(|e| e.to_string())?;
    dialog.set_target_path(tod.display().to_string().into());

    let app_id = manifest::load(tod).ok().flatten().and_then(|m| m.app_id);
    refresh(&dialog, tod, app_id).map_err(|e| e.to_string())?;

    let watcher = start_watch(&dialog, tod, app_id);

    {
        let weak = dialog.as_weak();
        dialog.on_ra_fetch_requested(move |game_id_text| {
            trigger_ra_fetch(weak.clone(), game_id_text.to_string());
        });
    }

    {
        let holder = holder.clone();
        dialog.window().on_close_requested(move || {
            holder.borrow_mut().take();
            slint::CloseRequestResponse::HideWindow
        });
    }

    let _ = dialog.show();
    *holder.borrow_mut() = Some(Session { dialog, _watcher: watcher });
    Ok(())
}

fn refresh(dialog: &AchievementsDialog, tod: &Path, app_id: Option<u64>) -> Result<(), autogse::error::AutoGseError> {
    let achievements = achievements::load_with_unlock_state(tod, app_id)?;
    let count = achievements.len();
    let unlocked = achievements.iter().filter(|a| a.unlocked).count();

    let rows: Vec<AchievementRow> = achievements
        .into_iter()
        .map(|a| AchievementRow {
            name: a.name.into(),
            display_name: a.display_name.into(),
            description: a.description.into(),
            hidden: a.hidden,
            unlocked: a.unlocked,
            icon: a
                .icon_path
                .as_deref()
                .and_then(|p| slint::Image::load_from_path(p).ok())
                .unwrap_or_default(),
        })
        .collect();
    dialog.set_achievements(slint::ModelRc::new(slint::VecModel::from(rows)));
    dialog.set_unlock_ratio(if count == 0 { 0.0 } else { unlocked as f32 / count as f32 });

    dialog.set_status_message(if count == 0 {
        "No achievement data found for this target (never injected with a Steam login, or this game has none).".to_string().into()
    } else {
        format!("{unlocked} / {count} unlocked.").into()
    });
    Ok(())
}

/// RA network timeout — same 3000ms budget `header_cache`'s CDN fetch uses
/// for a comparable "don't stall the GUI on a slow/unreachable remote
/// service" call.
const RA_TIMEOUT: Duration = Duration::from_millis(3000);

/// Phase 7 §7.7: fetches a RetroAchievements game's progress and replaces
/// the dialog's `achievements` list with it (same list Phase 7 §7.5's Steam
/// data populates — "unified viewer," not two parallel lists). Runs the
/// actual fetch (a real network call, plus a badge-image fetch per
/// achievement) off the UI thread, same `std::thread::spawn` +
/// `invoke_from_event_loop` shape as `autogse_gui.rs`'s `trigger_resolve_appid`.
fn trigger_ra_fetch(weak: slint::Weak<AchievementsDialog>, game_id_text: String) {
    let Some(dialog) = weak.upgrade() else { return };
    let Ok(game_id) = game_id_text.trim().parse::<u64>() else {
        dialog.set_ra_status_message("Enter a numeric RetroAchievements Game ID.".into());
        return;
    };
    dialog.set_ra_fetching(true);
    dialog.set_ra_status_message("Loading...".into());

    std::thread::spawn(move || {
        let result = fetch_ra_data(game_id);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(dialog) = weak.upgrade() else { return };
            match result {
                Ok((entries, status)) => {
                    // `slint::Image` isn't `Send`, so it can't cross the
                    // thread boundary above — same reasoning as
                    // `autogse_gui.rs`'s `RowData`/`scan`: the background
                    // thread only ever produces plain, `Send` badge *paths*,
                    // and the actual `Image::load_from_path` decode happens
                    // right here, back on the UI thread.
                    let rows: Vec<AchievementRow> = entries
                        .into_iter()
                        .map(|e| AchievementRow {
                            name: e.id.to_string().into(),
                            display_name: e.title.into(),
                            description: e.description.into(),
                            hidden: false,
                            unlocked: e.unlocked,
                            icon: e.badge_path.as_deref().and_then(|p| slint::Image::load_from_path(p).ok()).unwrap_or_default(),
                        })
                        .collect();
                    let count = rows.len();
                    let unlocked = rows.iter().filter(|r| r.unlocked).count();
                    dialog.set_achievements(slint::ModelRc::new(slint::VecModel::from(rows)));
                    dialog.set_unlock_ratio(if count == 0 { 0.0 } else { unlocked as f32 / count as f32 });
                    dialog.set_status_message(status.into());
                    dialog.set_ra_status_message("Loaded from RetroAchievements.org.".into());
                }
                Err(e) => dialog.set_ra_status_message(format!("Error: {e}").into()),
            }
            dialog.set_ra_fetching(false);
        });
    });
}

struct RaAchievementData {
    id: u64,
    title: String,
    description: String,
    unlocked: bool,
    badge_path: Option<std::path::PathBuf>,
}

fn fetch_ra_data(game_id: u64) -> Result<(Vec<RaAchievementData>, String), String> {
    let creds = retroachievements::load()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No RetroAchievements login configured — run `autogse ra-login` first.".to_string())?;
    let progress = retroachievements::fetch_game_progress(&creds, game_id, RA_TIMEOUT).map_err(|e| e.to_string())?;

    let unlocked = progress.achievements.iter().filter(|a| a.unlocked).count();
    let status = format!(
        "{}: {} — {unlocked} / {} unlocked (RetroAchievements).",
        progress.console_name,
        progress.title,
        progress.achievements.len()
    );

    let entries: Vec<RaAchievementData> = progress
        .achievements
        .into_iter()
        .map(|a| {
            // Best-effort: a badge that fails to fetch/cache just renders
            // with no icon, same convention as every other network-art call
            // in this codebase (`header_cache`, Phase 7 §7.2's dashboard art).
            let badge_path = retroachievements::cached_badge_path(&a.badge_name, RA_TIMEOUT).ok();
            RaAchievementData { id: a.id, title: a.title, description: a.description, unlocked: a.unlocked, badge_path }
        })
        .collect();

    Ok((entries, status))
}

/// Starts watching the resolved unlock-state file, if one could be resolved
/// at all — a target that's never been launched yet (no save folder created
/// on disk) has nothing to watch until it exists, in which case
/// `resolve_unlock_state_path` returns `None` and this returns `None` too
/// rather than erroring (the dialog still shows the definitions, just
/// without a live watch).
fn start_watch(dialog: &AchievementsDialog, tod: &Path, app_id: Option<u64>) -> Option<UnlockWatcher> {
    let app_id = app_id?;
    let configs_user_ini = tod.join("steam_settings").join("configs.user.ini");
    let unlock_path = achievements::resolve_unlock_state_path(tod, &configs_user_ini, app_id)?;

    let weak = dialog.as_weak();
    let tod = tod.to_path_buf();
    // `on_change` fires on `notify`'s own background thread (see
    // `achievements::watch_unlock_state`'s doc comment) — the actual
    // re-read is a small local-disk JSON parse, cheap enough to do right
    // there rather than hopping through a second `std::thread::spawn`;
    // `invoke_from_event_loop` is still required to touch `dialog` itself,
    // since Slint property setters aren't safe to call off the UI thread.
    achievements::watch_unlock_state(&unlock_path, move || {
        let weak = weak.clone();
        let tod = tod.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(dialog) = weak.upgrade() else { return };
            let _ = refresh(&dialog, &tod, Some(app_id));
        });
    })
    .ok()
}
