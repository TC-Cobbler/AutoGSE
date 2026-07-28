//! Phase 7 §7.4's business logic: viewing and editing an already-injected
//! target's `steam_settings/configs.*.ini` files, plus the DLC manager and
//! persona switcher built on top of them. Kept separate from `engine.rs`
//! (which owns the inject/revert/scan orchestration) since this is a
//! narrower, GUI-facing concern with no CLI equivalent today.

use std::path::{Path, PathBuf};

use crate::engine;
use crate::error::AutoGseError;
use crate::ini_patch::{self, IniSection};
use crate::manifest;
use crate::preferences::{self, NamedPersona};

/// The four config files a real `generate_emu_config.exe` run produces,
/// confirmed against real vendored output (roadmap Phase 6 §6.1-§6.6).
const CONFIG_FILES: &[(&str, &str)] =
    &[("User", "configs.user.ini"), ("Main", "configs.main.ini"), ("Overlay", "configs.overlay.ini"), ("App", "configs.app.ini")];

/// One `configs.*.ini` file's parsed contents, for the tabbed inspector.
pub struct ConfigFile {
    pub label: &'static str,
    pub path: PathBuf,
    pub sections: Vec<IniSection>,
}

/// Loads every config file present under `tod/steam_settings/` — missing
/// files (e.g. a `steamclient`-mode target, which never writes some of
/// these) are silently skipped rather than treated as an error, same
/// convention `engine::run_inject_single` already uses for optional files.
pub fn load_config_files(tod: &Path) -> Result<Vec<ConfigFile>, AutoGseError> {
    if manifest::load(tod)?.is_none() {
        return Err(AutoGseError::NotInjected(tod.to_path_buf()));
    }

    let settings = tod.join("steam_settings");
    let mut files = Vec::new();
    for (label, filename) in CONFIG_FILES {
        let path = settings.join(filename);
        if path.is_file() {
            files.push(ConfigFile { label, sections: ini_patch::read_all(&path)?, path });
        }
    }
    Ok(files)
}

/// One DLC entry under `configs.app.ini`'s `[app::dlcs]` section. Confirmed
/// against a real generated config (Euro Truck Simulator 2, 227300, 111
/// real DLCs): there is no per-DLC boolean at all — presence as an
/// `ID=Name` line *is* "configured as owned." `unlock_all=1` overrides every
/// entry here regardless of what's listed, so `unlocked` always reflects
/// this file's own state, independent of that global flag.
pub struct DlcEntry {
    pub app_id: String,
    pub name: String,
}

pub struct DlcState {
    pub unlock_all: bool,
    pub entries: Vec<DlcEntry>,
}

fn configs_app_ini(tod: &Path) -> PathBuf {
    tod.join("steam_settings").join("configs.app.ini")
}

pub fn load_dlc_state(tod: &Path) -> Result<DlcState, AutoGseError> {
    let sections = ini_patch::read_all(&configs_app_ini(tod))?;
    let mut unlock_all = false;
    let mut entries = Vec::new();

    if let Some(section) = sections.into_iter().find(|s| s.name == "app::dlcs") {
        for entry in section.entries {
            if entry.key == "unlock_all" {
                unlock_all = entry.value.trim() == "1";
            } else {
                entries.push(DlcEntry { app_id: entry.key, name: entry.value });
            }
        }
    }

    Ok(DlcState { unlock_all, entries })
}

pub fn set_unlock_all(tod: &Path, enabled: bool) -> Result<(), AutoGseError> {
    ini_patch::set_key(&configs_app_ini(tod), "app::dlcs", "unlock_all", if enabled { "1" } else { "0" })
}

/// Un-checking a DLC removes its line entirely (so `unlock_all=0` correctly
/// reports it as not owned); re-checking one previously known needs its
/// name handed back in, since removal doesn't keep it anywhere — the GUI
/// keeps the last-seen list client-side for exactly this round-trip.
pub fn set_dlc_unlocked(tod: &Path, app_id: &str, name: &str, unlocked: bool) -> Result<(), AutoGseError> {
    let path = configs_app_ini(tod);
    if unlocked {
        ini_patch::set_key(&path, "app::dlcs", app_id, name)
    } else {
        ini_patch::remove_key(&path, "app::dlcs", app_id)
    }
}

/// "Custom DLC ID addition" (original PRD wording): a DLC not already known
/// from the generated list — same underlying write as re-checking a known
/// one, exposed separately since the GUI's manual-entry form doesn't have a
/// pre-existing name to reuse.
pub fn add_custom_dlc(tod: &Path, app_id: &str, name: &str) -> Result<(), AutoGseError> {
    ini_patch::set_key(&configs_app_ini(tod), "app::dlcs", app_id, name)
}

/// Reads `steam_settings/supported_languages.txt` for the language dropdown
/// — empty (not an error) when the target doesn't have one, matching
/// `engine::set_language`'s own "skip validation if the file's absent" rule.
pub fn supported_languages(tod: &Path) -> Vec<String> {
    let path = tod.join("steam_settings").join("supported_languages.txt");
    std::fs::read_to_string(path)
        .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

fn configs_user_ini(tod: &Path) -> PathBuf {
    tod.join("steam_settings").join("configs.user.ini")
}

/// Directly sets the language on an already-injected target — reuses
/// `engine::set_language`'s validation so the GUI enforces the identical
/// rule the CLI's `--language` flag does, rather than a second copy of it.
pub fn set_language(tod: &Path, lang: &str) -> Result<(), AutoGseError> {
    engine::set_language(tod, &configs_user_ini(tod), lang)
}

pub fn set_account_name(tod: &Path, name: &str) -> Result<(), AutoGseError> {
    ini_patch::set_key(&configs_user_ini(tod), "user::general", "account_name", name)
}

/// Applies a saved persona directly to `tod` — the "switch" half of Phase 7
/// §7.4's Account Profile Switcher. Fields left `None` on the persona are
/// left untouched on the target, same "don't touch what wasn't supplied"
/// rule `preferences::set_default_persona` already applies.
pub fn apply_named_persona(tod: &Path, persona: &NamedPersona) -> Result<(), AutoGseError> {
    if let Some(lang) = &persona.language {
        set_language(tod, lang)?;
    }
    if let Some(name) = &persona.account_name {
        set_account_name(tod, name)?;
    }
    Ok(())
}

/// Every documented compat-flag name `engine::apply_network_compat` accepts
/// (§6.4), paired with the section its key actually lives in — the same
/// list `engine::COMPAT_FLAGS` uses, duplicated here rather than made
/// `pub(crate)` there since `engine.rs` owns the CLI-args validation path
/// and this module owns the GUI read-back path; both must agree on the
/// underlying keys, which is why the names/sections below must be kept in
/// sync with `engine.rs`'s own list, not re-derived independently.
const NETWORK_COMPAT_FLAGS: &[(&str, &str)] = &[
    ("achievements_bypass", "main::misc"),
    ("disable_steamoverlaygameid_env_var", "main::misc"),
    ("enable_steam_preowned_ids", "main::misc"),
    ("new_app_ticket", "main::general"),
];

/// Read-back model for Phase 7 §7.8.5's new "Networking" config-editor tab —
/// `engine::apply_network_compat` is write-only/CLI-invoked today, so
/// there was no existing function that reports *current* state the way
/// `load_dlc_state` already does for the DLC tab.
pub struct NetworkState {
    pub offline: bool,
    pub steam_deck: bool,
    /// Every compat flag from `NETWORK_COMPAT_FLAGS` currently set to `1`.
    pub compat_flags: Vec<String>,
}

fn configs_main_ini(tod: &Path) -> PathBuf {
    tod.join("steam_settings").join("configs.main.ini")
}

fn find_bool(sections: &[IniSection], section: &str, key: &str) -> bool {
    sections
        .iter()
        .find(|s| s.name == section)
        .and_then(|s| s.entries.iter().find(|e| e.key == key))
        .is_some_and(|e| e.value.trim() == "1")
}

pub fn load_network_state(tod: &Path) -> Result<NetworkState, AutoGseError> {
    let sections = ini_patch::read_all(&configs_main_ini(tod))?;
    let offline = find_bool(&sections, "main::connectivity", "offline");
    let steam_deck = find_bool(&sections, "main::general", "steam_deck");
    let compat_flags = NETWORK_COMPAT_FLAGS
        .iter()
        .filter(|(name, section)| find_bool(&sections, section, name))
        .map(|(name, _)| name.to_string())
        .collect();

    Ok(NetworkState { offline, steam_deck, compat_flags })
}

/// Mirrors `engine::apply_network_compat`'s `--offline` write exactly (the
/// same three `[main::connectivity]` keys), so both entry points agree on
/// what "offline" means.
pub fn set_offline(tod: &Path, enabled: bool) -> Result<(), AutoGseError> {
    let path = configs_main_ini(tod);
    let value = if enabled { "1" } else { "0" };
    ini_patch::set_key(&path, "main::connectivity", "offline", value)?;
    ini_patch::set_key(&path, "main::connectivity", "disable_networking", value)?;
    ini_patch::set_key(&path, "main::connectivity", "disable_lobby_creation", value)?;
    Ok(())
}

pub fn set_steam_deck(tod: &Path, enabled: bool) -> Result<(), AutoGseError> {
    ini_patch::set_key(&configs_main_ini(tod), "main::general", "steam_deck", if enabled { "1" } else { "0" })
}

/// Returns `Err(AutoGseError::InvalidCompatFlag)` for anything not in
/// `NETWORK_COMPAT_FLAGS` — same validation `engine::apply_network_compat`
/// applies to the CLI's `--compat-flag`, so the GUI can't silently write an
/// unrecognized key either.
pub fn set_compat_flag(tod: &Path, flag: &str, enabled: bool) -> Result<(), AutoGseError> {
    let (name, section) = NETWORK_COMPAT_FLAGS.iter().find(|(name, _)| *name == flag).ok_or_else(|| AutoGseError::InvalidCompatFlag(flag.to_string()))?;
    ini_patch::set_key(&configs_main_ini(tod), section, name, if enabled { "1" } else { "0" })
}

pub fn saved_personas() -> Result<Vec<NamedPersona>, AutoGseError> {
    Ok(preferences::load()?.saved_personas)
}

pub fn save_named_persona(name: String, account_name: Option<String>, language: Option<String>) -> Result<(), AutoGseError> {
    preferences::save_named_persona(name, account_name, language)
}

pub fn delete_named_persona(name: &str) -> Result<(), AutoGseError> {
    preferences::delete_named_persona(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_configs_main_ini(tod: &Path) -> PathBuf {
        let settings = tod.join("steam_settings");
        std::fs::create_dir_all(&settings).unwrap();
        let path = settings.join("configs.main.ini");
        std::fs::write(
            &path,
            "[main::general]\r\nnew_app_ticket=1\r\nsteam_deck=0\r\n\r\n[main::connectivity]\r\noffline=0\r\n\r\n[main::misc]\r\nachievements_bypass=0\r\n",
        )
        .unwrap();
        path
    }

    #[test]
    fn load_network_state_reads_real_shape() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_main_ini(tod.path());

        let state = load_network_state(tod.path()).unwrap();
        assert!(!state.offline);
        assert!(!state.steam_deck);
        assert_eq!(state.compat_flags, vec!["new_app_ticket".to_string()]);
    }

    #[test]
    fn set_offline_sets_all_three_connectivity_keys() {
        let tod = tempfile::tempdir().unwrap();
        let path = write_configs_main_ini(tod.path());

        set_offline(tod.path(), true).unwrap();

        let result = std::fs::read_to_string(&path).unwrap();
        assert!(result.contains("offline=1"));
        assert!(result.contains("disable_networking=1"));
        assert!(result.contains("disable_lobby_creation=1"));
        assert!(load_network_state(tod.path()).unwrap().offline);
    }

    #[test]
    fn set_steam_deck_toggles_independently() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_main_ini(tod.path());

        set_steam_deck(tod.path(), true).unwrap();

        let state = load_network_state(tod.path()).unwrap();
        assert!(state.steam_deck);
        assert!(!state.offline, "toggling steam_deck must not touch offline");
    }

    #[test]
    fn set_compat_flag_toggles_on_and_off() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_main_ini(tod.path());

        set_compat_flag(tod.path(), "achievements_bypass", true).unwrap();
        assert!(load_network_state(tod.path()).unwrap().compat_flags.contains(&"achievements_bypass".to_string()));

        set_compat_flag(tod.path(), "achievements_bypass", false).unwrap();
        assert!(!load_network_state(tod.path()).unwrap().compat_flags.contains(&"achievements_bypass".to_string()));
    }

    #[test]
    fn set_compat_flag_rejects_unknown_flag() {
        let tod = tempfile::tempdir().unwrap();
        write_configs_main_ini(tod.path());

        let result = set_compat_flag(tod.path(), "not_a_real_flag", true);
        assert!(matches!(result, Err(AutoGseError::InvalidCompatFlag(_))));
    }

    fn write_manifest(tod: &Path) {
        let manifest = manifest::GseManifest {
            version: manifest::MANIFEST_VERSION.to_string(),
            timestamp: "unix:0".to_string(),
            target_directory: tod.to_string_lossy().into_owned(),
            backed_up_files: vec![],
            app_id: Some(227300),
            arch: Some("x64".to_string()),
            app_id_source: None,
            game_title: Some("Euro Truck Simulator 2".to_string()),
            injected_files: vec![],
            mode: "regular".to_string(),
        };
        manifest::save(tod, &manifest).unwrap();
    }

    fn write_configs_app_ini(tod: &Path) -> PathBuf {
        let settings = tod.join("steam_settings");
        std::fs::create_dir_all(&settings).unwrap();
        let path = settings.join("configs.app.ini");
        std::fs::write(
            &path,
            "[app::dlcs]\r\nunlock_all=0\r\n304140=Euro Truck Simulator 2 - Brazilian Paint Jobs Pack\r\n1704460=Euro Truck Simulator 2 - Volvo Construction Equipment\r\n",
        )
        .unwrap();
        path
    }

    #[test]
    fn load_config_files_errors_when_not_injected() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_config_files(dir.path());
        assert!(matches!(result, Err(AutoGseError::NotInjected(_))));
    }

    #[test]
    fn load_config_files_skips_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path());
        write_configs_app_ini(dir.path());

        let files = load_config_files(dir.path()).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].label, "App");
    }

    #[test]
    fn load_dlc_state_parses_real_shape() {
        let dir = tempfile::tempdir().unwrap();
        write_configs_app_ini(dir.path());

        let state = load_dlc_state(dir.path()).unwrap();

        assert!(!state.unlock_all);
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.entries[0].app_id, "304140");
        assert_eq!(state.entries[0].name, "Euro Truck Simulator 2 - Brazilian Paint Jobs Pack");
    }

    #[test]
    fn set_dlc_unlocked_false_removes_the_line() {
        let dir = tempfile::tempdir().unwrap();
        write_configs_app_ini(dir.path());

        set_dlc_unlocked(dir.path(), "304140", "Euro Truck Simulator 2 - Brazilian Paint Jobs Pack", false).unwrap();

        let state = load_dlc_state(dir.path()).unwrap();
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].app_id, "1704460");
    }

    #[test]
    fn set_dlc_unlocked_true_re_adds_the_line() {
        let dir = tempfile::tempdir().unwrap();
        write_configs_app_ini(dir.path());
        set_dlc_unlocked(dir.path(), "304140", "Brazilian Paint Jobs Pack", false).unwrap();

        set_dlc_unlocked(dir.path(), "304140", "Brazilian Paint Jobs Pack", true).unwrap();

        let state = load_dlc_state(dir.path()).unwrap();
        assert!(state.entries.iter().any(|e| e.app_id == "304140" && e.name == "Brazilian Paint Jobs Pack"));
    }

    #[test]
    fn add_custom_dlc_appends_a_new_entry() {
        let dir = tempfile::tempdir().unwrap();
        write_configs_app_ini(dir.path());

        add_custom_dlc(dir.path(), "999999", "Homebrew DLC").unwrap();

        let state = load_dlc_state(dir.path()).unwrap();
        assert_eq!(state.entries.len(), 3);
        assert!(state.entries.iter().any(|e| e.app_id == "999999" && e.name == "Homebrew DLC"));
    }

    #[test]
    fn set_unlock_all_toggles_independently_of_the_entry_list() {
        let dir = tempfile::tempdir().unwrap();
        write_configs_app_ini(dir.path());

        set_unlock_all(dir.path(), true).unwrap();

        let state = load_dlc_state(dir.path()).unwrap();
        assert!(state.unlock_all);
        assert_eq!(state.entries.len(), 2, "toggling unlock_all must not touch the individual entry list");
    }

    #[test]
    fn supported_languages_is_empty_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(supported_languages(dir.path()).is_empty());
    }

    #[test]
    fn supported_languages_reads_real_shape() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("steam_settings");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(settings.join("supported_languages.txt"), "english\r\ngerman\r\nfrench\r\n").unwrap();

        assert_eq!(supported_languages(dir.path()), vec!["english", "german", "french"]);
    }

    #[test]
    fn apply_named_persona_only_writes_supplied_fields() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("steam_settings");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(settings.join("configs.user.ini"), "[user::general]\r\naccount_name=old\r\n").unwrap();

        let persona = NamedPersona { name: "Test".to_string(), account_name: Some("newname".to_string()), language: None };
        apply_named_persona(dir.path(), &persona).unwrap();

        let result = std::fs::read_to_string(settings.join("configs.user.ini")).unwrap();
        assert!(result.contains("account_name=newname"));
    }

    /// Manual QA only (real generated fixture, not run in normal `cargo
    /// test`): proves this module's parsing against real large-scale data,
    /// not just this file's own small hand-written fixtures — Euro Truck
    /// Simulator 2 (227300) has 111 real DLCs and 30 real supported
    /// languages. Regenerate the fixture first if it doesn't exist:
    /// `generate_emu_config.exe -rel_raw -clr -anon -skip_ach -skip_con
    /// -skip_inv 227300` run from
    /// `C:\Users\Johnny\AppData\Local\Temp\config_editor_live_fixture`, plus
    /// a `.gse_manifest.json` alongside it — same shape `write_manifest`
    /// above writes (app_id 227300, mode "regular", empty backed_up_files).
    /// `cargo test config_editor::tests::live_real_ets2_fixture -- --ignored`
    #[test]
    #[ignore]
    fn live_real_ets2_fixture() {
        let tod = Path::new(r"C:\Users\Johnny\AppData\Local\Temp\config_editor_live_fixture");

        let files = load_config_files(tod).expect("load real ETS2 config files");
        assert_eq!(files.len(), 4, "expected all four configs.*.ini files");
        let app_file = files.iter().find(|f| f.label == "App").expect("configs.app.ini");
        let dlcs_section = app_file.sections.iter().find(|s| s.name == "app::dlcs").expect("[app::dlcs] section");
        assert!(dlcs_section.entries.len() > 100, "expected 111ish real DLC entries, got {}", dlcs_section.entries.len());

        let dlc_state = load_dlc_state(tod).expect("load real DLC state");
        assert!(!dlc_state.unlock_all, "real ETS2 fixture ships unlock_all=0");
        assert!(dlc_state.entries.len() > 100);
        assert!(dlc_state.entries.iter().any(|e| e.app_id == "304140"));

        let languages = supported_languages(tod);
        assert!(languages.len() > 20, "expected ~30 real supported languages, got {}", languages.len());
        assert!(languages.iter().any(|l| l.eq_ignore_ascii_case("english")));

        // Exercise the actual write path against the real file too, not
        // just small synthetic fixtures elsewhere in this module.
        set_dlc_unlocked(tod, "304140", "Euro Truck Simulator 2 - Brazilian Paint Jobs Pack", false).unwrap();
        let after_removal = load_dlc_state(tod).unwrap();
        assert!(!after_removal.entries.iter().any(|e| e.app_id == "304140"));
        set_dlc_unlocked(tod, "304140", "Euro Truck Simulator 2 - Brazilian Paint Jobs Pack", true).unwrap();
        let after_readd = load_dlc_state(tod).unwrap();
        assert!(after_readd.entries.iter().any(|e| e.app_id == "304140"));
    }
}
