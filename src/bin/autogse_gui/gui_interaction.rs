use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;

use autogse::credentials::Credentials;
use autogse::error::AutoGseError;
use autogse::interaction::Interaction;
use autogse::login_prompt::DisclosureChoice;
use autogse::steam_api::ScoredCandidate;
use slint::ComponentHandle;

use crate::{CandidateRow, DisambiguationDialog};

/// GUI implementation of `Interaction` (Phase 7 §7.0/§7.6). Today only
/// `disambiguate_app_id` backs a real feature (the dashboard's "Resolve App
/// ID" action) — every other method is unused by any flow `autogse-gui`
/// currently wires up (no inject/revert-from-GUI yet, see roadmap §7.0's
/// still-open console-bound-tools item) and returns a safe non-interactive
/// default rather than pretending to support something nothing calls yet.
///
/// **Must only be invoked from a background thread, never the Slint UI
/// thread**: `disambiguate_app_id` blocks the calling thread on a channel
/// that's only filled by a dialog callback running on the UI thread (via
/// `slint::invoke_from_event_loop`) — called from the UI thread itself, that
/// would deadlock the whole application.
pub struct GuiInteraction;

impl Interaction for GuiInteraction {
    fn disambiguate_app_id(
        &self,
        target_dir: &Path,
        candidates: &[ScoredCandidate],
    ) -> Result<(u64, Option<String>), AutoGseError> {
        let (tx, rx) = mpsc::channel::<Result<(u64, Option<String>), AutoGseError>>();

        let target_dir_display = target_dir.display().to_string();
        let candidate_rows: Vec<CandidateRow> = candidates
            .iter()
            .map(|c| CandidateRow {
                appid: c.appid.to_string().into(),
                name: c.name.clone().into(),
                score: format!("{:.0}%", c.score * 100.0).into(),
            })
            .collect();
        // Kept alongside the display rows so a selected candidate can report
        // its display name back too, matching the CLI's own
        // `(u64, Option<String>)` return shape (`appid_prompt`'s equivalent).
        let names_by_appid: HashMap<u64, String> = candidates.iter().map(|c| (c.appid, c.name.clone())).collect();

        let schedule_result = slint::invoke_from_event_loop(move || {
            let dialog = match DisambiguationDialog::new() {
                Ok(d) => d,
                Err(_) => {
                    let _ = tx.send(Err(AutoGseError::AppIdResolutionFailed(
                        "could not open the disambiguation dialog".to_string(),
                    )));
                    return;
                }
            };
            dialog.set_target_dir(target_dir_display.into());
            dialog.set_candidates(slint::ModelRc::new(slint::VecModel::from(candidate_rows)));

            // Holds the dialog's one strong handle. Callbacks below capture
            // only clones of this `Rc`, never `dialog` itself, so the
            // component doesn't end up owning a callback that owns a strong
            // reference back to itself (a cycle Slint's own docs warn never
            // gets cleaned up by refcounting alone). Whichever callback
            // fires first `take()`s it out and drops it, closing the window.
            let held: Rc<RefCell<Option<DisambiguationDialog>>> = Rc::new(RefCell::new(None));

            {
                let held = held.clone();
                let tx = tx.clone();
                let names_by_appid = names_by_appid.clone();
                dialog.on_candidate_selected(move |appid_str| {
                    let result = appid_str
                        .parse::<u64>()
                        .map(|id| (id, names_by_appid.get(&id).cloned()))
                        .map_err(|_| AutoGseError::AppIdResolutionFailed(format!("'{appid_str}' is not a valid App ID")));
                    let _ = tx.send(result);
                    if let Some(w) = held.borrow_mut().take() {
                        let _ = w.hide();
                    }
                });
            }
            {
                let held = held.clone();
                let tx = tx.clone();
                dialog.on_manual_selected(move |value| {
                    let result = value
                        .trim()
                        .parse::<u64>()
                        .map(|id| (id, None))
                        .map_err(|_| AutoGseError::AppIdResolutionFailed(format!("'{value}' is not a valid numeric Steam App ID")));
                    let _ = tx.send(result);
                    if let Some(w) = held.borrow_mut().take() {
                        let _ = w.hide();
                    }
                });
            }
            {
                let held = held.clone();
                let tx = tx.clone();
                dialog.on_cancelled(move || {
                    let _ = tx.send(Err(AutoGseError::AppIdResolutionFailed(
                        "no Steam App ID selected (dialog cancelled)".to_string(),
                    )));
                    if let Some(w) = held.borrow_mut().take() {
                        let _ = w.hide();
                    }
                });
            }

            let _ = dialog.show();
            *held.borrow_mut() = Some(dialog);
        });

        if schedule_result.is_err() {
            return Err(AutoGseError::AppIdResolutionFailed(
                "could not schedule the disambiguation dialog on the UI thread".to_string(),
            ));
        }

        // Blocks this (background) thread until one of the three dialog
        // callbacks above fires and sends a result — or the sender is
        // dropped (e.g. the window closed via the OS close button without
        // going through Cancel), which surfaces here as a `RecvError`.
        rx.recv()
            .unwrap_or_else(|_| Err(AutoGseError::AppIdResolutionFailed("disambiguation dialog closed unexpectedly".to_string())))
    }

    /// **Not yet built**: the GUI's "Resolve App ID" action (the only caller
    /// of `GuiInteraction` today) deliberately passes `interaction: None` to
    /// `discovery::resolve_target` instead of routing through this, so a
    /// non-standard DLL name just falls back to the ordinary `DllNotFound`
    /// error rather than reaching this stub. Kept honest rather than
    /// pretending to resolve something no dialog exists for yet.
    fn pick_dll(&self, _near_matches: &[PathBuf], _d_root: &Path) -> Option<PathBuf> {
        None
    }

    /// Unused by any flow wired up in `autogse-gui` yet (no login/inject
    /// UI); a real GUI-native disclosure dialog is future work.
    fn login_disclosure(&self) -> DisclosureChoice {
        DisclosureChoice::Cancelled
    }

    /// Unused by any flow wired up in `autogse-gui` yet.
    fn capture_login(&self) -> Result<Credentials, AutoGseError> {
        Err(AutoGseError::LoginFailed("interactive Steam login capture is not yet implemented in the GUI".to_string()))
    }

    /// Unused by any flow wired up in `autogse-gui` yet.
    fn confirm_save_default_persona(&self) -> bool {
        false
    }
}
