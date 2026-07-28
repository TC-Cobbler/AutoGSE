//! Phase 7 §7.8.2's GUI glue: opens a dedicated confirm dialog on a real
//! native file drop (§7.3), instead of only filling the dashboard's App ID
//! text field as before. Runs the same background-thread resolve cascade
//! `trigger_resolve_appid` already uses (`crate::resolve_appid_detailed`),
//! just handing the richer result to this dialog instead.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::ComponentHandle;

use crate::DropConfirmDialog;

pub type DialogHolder = Rc<RefCell<Option<DropConfirmDialog>>>;

pub fn new_holder() -> DialogHolder {
    Rc::new(RefCell::new(None))
}

pub fn open_and_resolve(holder: &DialogHolder, path: PathBuf) {
    let Ok(dialog) = DropConfirmDialog::new() else { return };
    dialog.set_status_message(format!("Resolving {}...", path.display()).into());

    {
        let holder = holder.clone();
        dialog.window().on_close_requested(move || {
            holder.borrow_mut().take();
            slint::CloseRequestResponse::HideWindow
        });
    }
    {
        let weak = dialog.as_weak();
        dialog.on_cancelled(move || {
            if let Some(dialog) = weak.upgrade() {
                let _ = dialog.hide();
            }
        });
    }

    let _ = dialog.show();
    let weak = dialog.as_weak();
    *holder.borrow_mut() = Some(dialog);

    std::thread::spawn(move || {
        let result = crate::resolve_appid_detailed(&path);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(dialog) = weak.upgrade() else { return };
            match result {
                Ok(detected) => {
                    dialog.set_resolved(true);
                    dialog.set_status_message("Drop resolved — review before confirming.".into());
                    dialog.set_detected_title(detected.title.unwrap_or_else(|| "(unknown title)".to_string()).into());
                    dialog.set_detected_path(detected.tod_display.into());
                    dialog.set_detected_arch(detected.arch.into());
                    dialog.set_detected_appid(detected.app_id.to_string().into());
                }
                Err(e) => {
                    dialog.set_resolved(false);
                    dialog.set_status_message(format!("Error: {e}").into());
                }
            }
        });
    });
}
