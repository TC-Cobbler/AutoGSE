//! Phase 7 §7.8.3's GUI glue over Steam login — plain `credentials::save`/
//! `delete` (equivalent to the CLI's `login`/`logout` subcommands), not
//! `Interaction::capture_login`/`login_disclosure` (`GuiInteraction`'s stubs
//! for those stay unused — see this dialog's own `.slint` doc comment for
//! why the disclosure-flow framing doesn't fit a standalone dashboard
//! button). Local-disk DPAPI only, no network call, so this runs directly
//! on the UI thread like the config editor and achievement viewer.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;

use autogse::credentials::{self, Credentials};

use crate::LoginDialog;

pub type DialogHolder = Rc<RefCell<Option<LoginDialog>>>;

pub fn new_holder() -> DialogHolder {
    Rc::new(RefCell::new(None))
}

pub fn open(holder: &DialogHolder) -> Result<(), String> {
    let dialog = LoginDialog::new().map_err(|e| e.to_string())?;
    refresh(&dialog);

    {
        let weak = dialog.as_weak();
        dialog.on_login_requested(move |username, password| {
            let Some(dialog) = weak.upgrade() else { return };
            let username = username.trim().to_string();
            let password = password.to_string();
            if username.is_empty() || password.is_empty() {
                dialog.set_status_message("Enter both a username and password.".into());
                return;
            }
            match credentials::save(&Credentials { username: username.clone(), password }) {
                Ok(()) => {
                    refresh(&dialog);
                    dialog.set_status_message(format!("Logged in as {username}.").into());
                }
                Err(e) => dialog.set_status_message(format!("Error: {e}").into()),
            }
        });
    }

    {
        let weak = dialog.as_weak();
        dialog.on_logout_requested(move || {
            let Some(dialog) = weak.upgrade() else { return };
            match credentials::delete() {
                Ok(()) => {
                    refresh(&dialog);
                    dialog.set_status_message("Logged out. Stored Steam credentials removed.".into());
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

fn refresh(dialog: &LoginDialog) {
    match credentials::load() {
        Ok(Some(creds)) => {
            dialog.set_logged_in(true);
            dialog.set_logged_in_username(creds.username.into());
        }
        Ok(None) => {
            dialog.set_logged_in(false);
            dialog.set_logged_in_username("".into());
        }
        Err(e) => {
            dialog.set_logged_in(false);
            dialog.set_status_message(format!("Error reading stored credentials: {e}").into());
        }
    }
}
