//! Phase 7 §7.8.6's GUI glue: a global (not per-target) preferences dialog
//! over `preferences::OverlayPrefs`. Reuses `engine::run_configure_overlay`
//! directly (rather than calling `preferences::set_overlay_prefs` itself)
//! so the GUI enforces the exact same `VALID_OVERLAY_POSITIONS` validation
//! the CLI's `configure-overlay` subcommand already does, from one place.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;

use autogse::cli::ConfigureOverlayArgs;
use autogse::engine;
use autogse::preferences;

use crate::OverlaySettingsDialog;

pub type DialogHolder = Rc<RefCell<Option<OverlaySettingsDialog>>>;

pub fn new_holder() -> DialogHolder {
    Rc::new(RefCell::new(None))
}

fn opt_string(s: &str) -> Option<String> {
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn opt_f64(s: &str) -> Result<Option<f64>, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed.parse::<f64>().map(Some).map_err(|_| format!("'{trimmed}' is not a number"))
}

pub fn open(holder: &DialogHolder) -> Result<(), String> {
    let dialog = OverlaySettingsDialog::new().map_err(|e| e.to_string())?;
    refresh(&dialog).map_err(|e| e.to_string())?;

    {
        let weak = dialog.as_weak();
        dialog.on_save_requested(move |pos_a, pos_i, pos_c, dur_p, dur_a, dur_i, dur_c, anim| {
            let Some(dialog) = weak.upgrade() else { return };
            let result = (|| -> Result<(), String> {
                let args = ConfigureOverlayArgs {
                    pos_achievement: opt_string(pos_a.as_str()),
                    pos_invitation: opt_string(pos_i.as_str()),
                    pos_chat_msg: opt_string(pos_c.as_str()),
                    duration_progress: opt_f64(dur_p.as_str())?,
                    duration_achievement: opt_f64(dur_a.as_str())?,
                    duration_invitation: opt_f64(dur_i.as_str())?,
                    duration_chat: opt_f64(dur_c.as_str())?,
                    notification_animation: opt_f64(anim.as_str())?,
                };
                engine::run_configure_overlay(&args).map_err(|e| e.to_string())
            })();
            match result {
                Ok(()) => {
                    dialog.set_status_message("Saved.".into());
                }
                Err(e) => dialog.set_status_message(format!("Error: {e}").into()),
            }
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
    *holder.borrow_mut() = Some(dialog);
    Ok(())
}

fn fmt_f64(v: Option<f64>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

fn refresh(dialog: &OverlaySettingsDialog) -> Result<(), autogse::error::AutoGseError> {
    let prefs = preferences::load()?.overlay_prefs;
    dialog.set_pos_achievement(prefs.pos_achievement.unwrap_or_default().into());
    dialog.set_pos_invitation(prefs.pos_invitation.unwrap_or_default().into());
    dialog.set_pos_chat_msg(prefs.pos_chat_msg.unwrap_or_default().into());
    dialog.set_duration_progress(fmt_f64(prefs.duration_progress).into());
    dialog.set_duration_achievement(fmt_f64(prefs.duration_achievement).into());
    dialog.set_duration_invitation(fmt_f64(prefs.duration_invitation).into());
    dialog.set_duration_chat(fmt_f64(prefs.duration_chat).into());
    dialog.set_notification_animation(fmt_f64(prefs.notification_animation).into());
    dialog.set_status_message("".into());
    Ok(())
}
