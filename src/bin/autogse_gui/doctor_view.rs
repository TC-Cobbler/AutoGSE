//! Phase 7 §7.8.6's GUI glue over `engine::collect_doctor_report` — global,
//! not per-target, same as the Overlay Settings dialog. "Live" log tail
//! (Phase 7 §7.8.6's plan) is a `slint::Timer` poll every 2s while the
//! dialog is open, not a `notify`-crate watch: simpler, and appropriate for
//! a low-frequency append-only file that's already cheap to just re-read
//! (`crate::log::tail` reads the whole capped-at-2MB file each call).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;

use autogse::engine;

use crate::{DoctorDialog, ToolCheckRow};

pub struct Session {
    #[allow(dead_code)] // kept alive purely for its Drop (closes the window + stops the timer)
    dialog: DoctorDialog,
    _poll_timer: slint::Timer,
}

pub type DialogHolder = Rc<RefCell<Option<Session>>>;

pub fn new_holder() -> DialogHolder {
    Rc::new(RefCell::new(None))
}

pub fn open(holder: &DialogHolder) -> Result<(), String> {
    let dialog = DoctorDialog::new().map_err(|e| e.to_string())?;
    refresh(&dialog);

    {
        let weak = dialog.as_weak();
        dialog.on_refresh_requested(move || {
            if let Some(dialog) = weak.upgrade() {
                refresh(&dialog);
            }
        });
    }

    let poll_timer = slint::Timer::default();
    {
        let weak = dialog.as_weak();
        poll_timer.start(slint::TimerMode::Repeated, Duration::from_secs(2), move || {
            if let Some(dialog) = weak.upgrade() {
                refresh(&dialog);
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
    *holder.borrow_mut() = Some(Session { dialog, _poll_timer: poll_timer });
    Ok(())
}

fn refresh(dialog: &DoctorDialog) {
    let report = engine::collect_doctor_report();

    dialog.set_dpapi_ok(report.dpapi_ok);
    dialog.set_dpapi_detail(report.dpapi_detail.into());
    dialog.set_known_target_summary(match report.known_target_count {
        Ok(n) => format!("{n} injected"),
        Err(e) => format!("Error: {e}"),
    }.into());
    dialog.set_version_summary(format!("v{}", env!("CARGO_PKG_VERSION")).into());

    let tool_rows: Vec<ToolCheckRow> = report
        .tool_checks
        .into_iter()
        .map(|c| ToolCheckRow { name: c.name.into(), status: (if c.ok { "Pass" } else { "Fail" }).into(), ok: c.ok })
        .collect();
    dialog.set_tool_checks(slint::ModelRc::new(slint::VecModel::from(tool_rows)));

    let log_lines: Vec<slint::SharedString> = report.log_tail.into_iter().map(Into::into).collect();
    dialog.set_log_lines(slint::ModelRc::new(slint::VecModel::from(log_lines)));
}
