//! Phase 7 §7.4's GUI glue: converts `autogse::config_editor`'s plain Rust
//! types into the Slint-generated `ConfigEditorDialog`'s model types, and
//! wires its callbacks. Every operation here is local-disk INI read/write —
//! unlike the scan/App-ID-resolution features, none of this needs the
//! background-thread bridge (Phase 7 §7.0): there's no network call and no
//! `Interaction` prompt in this flow, so doing it directly on the UI thread
//! (which is what fires these callbacks anyway) keeps this simple rather
//! than adding a thread hop that would buy nothing here.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use slint::ComponentHandle;

use autogse::config_editor;
use autogse::error::AutoGseError;
use autogse::mods;

use crate::{ConfigEditorDialog, ConfigFileRow, DlcRow, IniEntryRow, IniSectionRow, ModRow, PersonaRow};

/// Holds the dialog's one strong handle across its whole open lifetime (many
/// interactions, not just one) — opening a new target replaces it, dropping
/// (and thereby closing) whatever was open before.
pub type DialogHolder = Rc<RefCell<Option<ConfigEditorDialog>>>;

/// Opens (or replaces) the config editor for `tod`. Returns a plain error
/// message rather than `AutoGseError` — the only caller is a button handler
/// that just displays whatever comes back as the status line, and a
/// `slint::PlatformError` (dialog window creation) doesn't fit any of that
/// enum's existing variants.
pub fn open(holder: &DialogHolder, tod: &Path) -> Result<(), String> {
    let dialog = ConfigEditorDialog::new().map_err(|e| e.to_string())?;
    wire_callbacks(&dialog, tod, holder);
    refresh(&dialog, tod).map_err(|e| e.to_string())?;
    let _ = dialog.show();
    *holder.borrow_mut() = Some(dialog);
    Ok(())
}

fn wire_callbacks(dialog: &ConfigEditorDialog, tod: &Path, holder: &DialogHolder) {
    let weak = dialog.as_weak();

    {
        let tod = tod.to_path_buf();
        let weak = weak.clone();
        dialog.on_set_unlock_all(move |enabled| {
            report(&weak, &tod, config_editor::set_unlock_all(&tod, enabled));
        });
    }
    {
        let tod = tod.to_path_buf();
        let weak = weak.clone();
        dialog.on_toggle_dlc(move |app_id, name, unlocked| {
            report(&weak, &tod, config_editor::set_dlc_unlocked(&tod, app_id.as_str(), name.as_str(), unlocked));
        });
    }
    {
        let tod = tod.to_path_buf();
        let weak = weak.clone();
        dialog.on_add_custom_dlc(move |id, name| {
            if id.is_empty() || name.is_empty() {
                return;
            }
            report(&weak, &tod, config_editor::add_custom_dlc(&tod, id.as_str(), name.as_str()));
        });
    }
    {
        let tod = tod.to_path_buf();
        let weak = weak.clone();
        dialog.on_select_language(move |lang| {
            report(&weak, &tod, config_editor::set_language(&tod, lang.as_str()));
        });
    }
    {
        let tod = tod.to_path_buf();
        let weak = weak.clone();
        dialog.on_apply_persona(move |name| {
            let result = config_editor::saved_personas()
                .and_then(|personas| personas.into_iter().find(|p| p.name == name.as_str()).ok_or(AutoGseError::NotInjected(tod.clone())))
                .and_then(|persona| config_editor::apply_named_persona(&tod, &persona));
            report(&weak, &tod, result);
        });
    }
    {
        let tod = tod.to_path_buf();
        let weak = weak.clone();
        dialog.on_save_current_as_persona(move |name| {
            if name.is_empty() {
                return;
            }
            // Saves whatever is *currently on disk* for this target as a
            // reusable persona, rather than requiring the user to retype an
            // account name/language that's already sitting in
            // `configs.user.ini` — reads it back the same way `refresh`
            // populates the dialog's own language field.
            let result = (|| -> Result<(), AutoGseError> {
                let files = config_editor::load_config_files(&tod)?;
                let (account_name, language) = current_persona_fields(&files);
                config_editor::save_named_persona(name.to_string(), account_name, language)
            })();
            report(&weak, &tod, result);
        });
    }
    {
        let tod = tod.to_path_buf();
        let weak = weak.clone();
        dialog.on_delete_persona(move |name| {
            report(&weak, &tod, config_editor::delete_named_persona(name.as_str()));
        });
    }
    {
        let tod = tod.to_path_buf();
        let weak = weak.clone();
        dialog.on_set_offline(move |enabled| {
            report(&weak, &tod, config_editor::set_offline(&tod, enabled));
        });
    }
    {
        let tod = tod.to_path_buf();
        let weak = weak.clone();
        dialog.on_set_steam_deck(move |enabled| {
            report(&weak, &tod, config_editor::set_steam_deck(&tod, enabled));
        });
    }
    {
        let tod = tod.to_path_buf();
        let weak = weak.clone();
        dialog.on_toggle_compat_flag(move |flag, enabled| {
            report(&weak, &tod, config_editor::set_compat_flag(&tod, flag.as_str(), enabled));
        });
    }

    // Dropping the dialog when the window is closed must also drop it out of
    // `holder` — otherwise the `Rc` here keeps it alive (and its native
    // window around) even after the user closed it.
    {
        let holder = holder.clone();
        dialog.window().on_close_requested(move || {
            holder.borrow_mut().take();
            slint::CloseRequestResponse::HideWindow
        });
    }
}

/// Every mutating callback re-reads everything from disk and re-populates
/// the dialog rather than hand-patching just the one field that changed —
/// simpler, and cheap enough for a handful of small local INI files that
/// this doesn't need to be smarter than "reload and redisplay."
fn report(weak: &slint::Weak<ConfigEditorDialog>, tod: &Path, result: Result<(), AutoGseError>) {
    let Some(dialog) = weak.upgrade() else { return };
    match result {
        Ok(()) => {
            if let Err(e) = refresh(&dialog, tod) {
                dialog.set_status_message(format!("Error refreshing after save: {e}").into());
            }
        }
        Err(e) => dialog.set_status_message(format!("Error: {e}").into()),
    }
}

fn refresh(dialog: &ConfigEditorDialog, tod: &Path) -> Result<(), AutoGseError> {
    let files = config_editor::load_config_files(tod)?;
    let dlc_state = config_editor::load_dlc_state(tod)?;
    let languages = config_editor::supported_languages(tod);
    let personas = config_editor::saved_personas()?;
    let (_, current_language) = current_persona_fields(&files);
    let network = config_editor::load_network_state(tod)?;
    // Best-effort: `mods.json` may not exist at all (no mods added yet) —
    // `mods::load_mods` already returns an empty list rather than an error
    // for that case, so this can't itself fail the whole refresh.
    let mod_entries = mods::load_mods(tod).unwrap_or_default();

    dialog.set_target_path(tod.display().to_string().into());
    dialog.set_config_files(slint::ModelRc::new(slint::VecModel::from(to_config_file_rows(&files))));
    dialog.set_unlock_all(dlc_state.unlock_all);
    dialog.set_dlc_entries(slint::ModelRc::new(slint::VecModel::from(
        dlc_state.entries.into_iter().map(|e| DlcRow { app_id: e.app_id.into(), name: e.name.into() }).collect::<Vec<_>>(),
    )));
    let languages: Vec<slint::SharedString> = languages.into_iter().map(Into::into).collect();
    dialog.set_languages(slint::ModelRc::new(slint::VecModel::from(languages)));
    dialog.set_current_language(current_language.unwrap_or_default().into());
    dialog.set_saved_personas(slint::ModelRc::new(slint::VecModel::from(
        personas.into_iter().map(|p| PersonaRow { name: p.name.into() }).collect::<Vec<_>>(),
    )));
    dialog.set_net_offline(network.offline);
    dialog.set_net_steam_deck(network.steam_deck);
    dialog.set_net_achievements_bypass(network.compat_flags.iter().any(|f| f == "achievements_bypass"));
    dialog.set_net_disable_overlay_gameid(network.compat_flags.iter().any(|f| f == "disable_steamoverlaygameid_env_var"));
    dialog.set_net_preowned_ids(network.compat_flags.iter().any(|f| f == "enable_steam_preowned_ids"));
    dialog.set_net_new_app_ticket(network.compat_flags.iter().any(|f| f == "new_app_ticket"));
    dialog.set_mods(slint::ModelRc::new(slint::VecModel::from(
        mod_entries.into_iter().map(|m| ModRow { id: m.id.into(), title: m.title.into() }).collect::<Vec<_>>(),
    )));
    dialog.set_status_message("".into());
    Ok(())
}

/// Reads `configs.user.ini`'s current `account_name`/`language` straight out
/// of the already-loaded `ConfigFile` list — no second file read.
fn current_persona_fields(files: &[config_editor::ConfigFile]) -> (Option<String>, Option<String>) {
    let Some(user_file) = files.iter().find(|f| f.label == "User") else { return (None, None) };
    let Some(section) = user_file.sections.iter().find(|s| s.name == "user::general") else { return (None, None) };
    let account_name = section.entries.iter().find(|e| e.key == "account_name").map(|e| e.value.clone());
    let language = section.entries.iter().find(|e| e.key == "language").map(|e| e.value.clone());
    (account_name, language)
}

fn to_config_file_rows(files: &[config_editor::ConfigFile]) -> Vec<ConfigFileRow> {
    files
        .iter()
        .map(|f| ConfigFileRow {
            label: f.label.into(),
            sections: slint::ModelRc::new(slint::VecModel::from(
                f.sections
                    .iter()
                    .map(|s| IniSectionRow {
                        name: s.name.clone().into(),
                        entries: slint::ModelRc::new(slint::VecModel::from(
                            s.entries.iter().map(|e| IniEntryRow { key: e.key.clone().into(), value: e.value.clone().into() }).collect::<Vec<_>>(),
                        )),
                    })
                    .collect::<Vec<_>>(),
            )),
        })
        .collect()
}
