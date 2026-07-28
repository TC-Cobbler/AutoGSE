use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::credentials;
use crate::error::AutoGseError;

const PREFERENCES_FILENAME: &str = "preferences.json";

/// Deliberately separate from `credentials.rs`'s DPAPI-encrypted store: this
/// is a preference, not a secret, and keeping it apart means `logout`
/// (`credentials::delete`) can never accidentally touch it — a user who
/// already chose "don't ask again" shouldn't get re-nagged just because they
/// logged out.
///
/// `Eq` is deliberately not derived (dropped once `OverlayPrefs` introduced
/// `f64` fields, which aren't `Eq`) — `PartialEq` alone is enough for the
/// equality assertions this type's tests need.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Preferences {
    #[serde(default)]
    pub anon_opt_in: bool,

    /// Saved persona defaults (roadmap Phase 6 §6.1) so a batch of games
    /// doesn't need `--account-name`/`--language` re-entered per invocation.
    /// `None` means "leave the emu's own generated default alone."
    #[serde(default)]
    pub default_account_name: Option<String>,
    #[serde(default)]
    pub default_language: Option<String>,

    /// Overlay notification tuning (roadmap Phase 6 §6.3) — a small saved
    /// profile instead of requiring hand-edits to `configs.overlay.ini`.
    /// Only applied when `--overlay` is passed; every field left `None`
    /// leaves the emu's own generated default alone.
    #[serde(default)]
    pub overlay_prefs: OverlayPrefs,

    /// Named, switchable personas (Phase 7 §7.4's "Account Profile
    /// Switcher") — distinct from `default_account_name`/`default_language`
    /// above, which remain the single implicit default applied when no CLI
    /// flag/named persona is chosen. A schema addition confirmed with the
    /// user before building, since it's new on-disk state alongside fields
    /// other code already reads.
    #[serde(default)]
    pub saved_personas: Vec<NamedPersona>,
}

/// One saved profile a user can switch an already-injected target to via
/// the GUI's config editor (Phase 7 §7.4) — `name` is the profile's own
/// label, deliberately distinct from `account_name` (the in-game Steam
/// persona name it sets).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct NamedPersona {
    pub name: String,
    #[serde(default)]
    pub account_name: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

/// Mirrors `configs.overlay.ini`'s `[overlay::appearance]` keys exactly
/// (confirmed against the real vendored file) — there is no single generic
/// "position"/"duration" key, each notification type has its own.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct OverlayPrefs {
    #[serde(default)]
    pub pos_achievement: Option<String>,
    #[serde(default)]
    pub pos_invitation: Option<String>,
    #[serde(default)]
    pub pos_chat_msg: Option<String>,
    #[serde(default)]
    pub duration_progress: Option<f64>,
    #[serde(default)]
    pub duration_achievement: Option<f64>,
    #[serde(default)]
    pub duration_invitation: Option<f64>,
    #[serde(default)]
    pub duration_chat: Option<f64>,
    #[serde(default)]
    pub notification_animation: Option<f64>,
}

/// The only values `configs.overlay.ini`'s `Pos*` keys accept (confirmed
/// against the real vendored file's own comment listing them).
pub const VALID_OVERLAY_POSITIONS: &[&str] = &["top_left", "top_center", "top_right", "bot_left", "bot_center", "bot_right"];

fn preferences_path(dir: &Path) -> PathBuf {
    dir.join(PREFERENCES_FILENAME)
}

pub fn load_in(dir: &Path) -> Result<Preferences, AutoGseError> {
    let path = preferences_path(dir);
    if !path.is_file() {
        return Ok(Preferences::default());
    }
    let bytes = std::fs::read(&path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn save_in(dir: &Path, prefs: &Preferences) -> Result<(), AutoGseError> {
    std::fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec_pretty(prefs)?;
    std::fs::write(preferences_path(dir), bytes)?;
    Ok(())
}

pub fn load() -> Result<Preferences, AutoGseError> {
    load_in(&credentials::store_dir()?)
}

pub fn set_anon_opt_in(value: bool) -> Result<(), AutoGseError> {
    let dir = credentials::store_dir()?;
    let mut prefs = load_in(&dir)?;
    prefs.anon_opt_in = value;
    save_in(&dir, &prefs)
}

/// Only overwrites the fields actually supplied — saving a new default
/// language shouldn't clear a previously saved default account name, or
/// vice versa.
pub fn set_default_persona(account_name: Option<String>, language: Option<String>) -> Result<(), AutoGseError> {
    let dir = credentials::store_dir()?;
    let mut prefs = load_in(&dir)?;
    if let Some(name) = account_name {
        prefs.default_account_name = Some(name);
    }
    if let Some(lang) = language {
        prefs.default_language = Some(lang);
    }
    save_in(&dir, &prefs)
}

/// Only overwrites fields actually supplied via `updates` — repeated
/// `configure-overlay` calls tuning one setting at a time must not clobber
/// previously saved ones.
pub fn set_overlay_prefs(updates: OverlayPrefs) -> Result<(), AutoGseError> {
    let dir = credentials::store_dir()?;
    let mut prefs = load_in(&dir)?;
    let p = &mut prefs.overlay_prefs;
    if updates.pos_achievement.is_some() {
        p.pos_achievement = updates.pos_achievement;
    }
    if updates.pos_invitation.is_some() {
        p.pos_invitation = updates.pos_invitation;
    }
    if updates.pos_chat_msg.is_some() {
        p.pos_chat_msg = updates.pos_chat_msg;
    }
    if updates.duration_progress.is_some() {
        p.duration_progress = updates.duration_progress;
    }
    if updates.duration_achievement.is_some() {
        p.duration_achievement = updates.duration_achievement;
    }
    if updates.duration_invitation.is_some() {
        p.duration_invitation = updates.duration_invitation;
    }
    if updates.duration_chat.is_some() {
        p.duration_chat = updates.duration_chat;
    }
    if updates.notification_animation.is_some() {
        p.notification_animation = updates.notification_animation;
    }
    save_in(&dir, &prefs)
}

/// Creates a new saved persona, or overwrites the existing one of the same
/// `name` — re-saving under a name already in use is an update, not a
/// duplicate entry.
pub fn save_named_persona(name: String, account_name: Option<String>, language: Option<String>) -> Result<(), AutoGseError> {
    let dir = credentials::store_dir()?;
    let mut prefs = load_in(&dir)?;
    match prefs.saved_personas.iter_mut().find(|p| p.name == name) {
        Some(existing) => {
            existing.account_name = account_name;
            existing.language = language;
        }
        None => prefs.saved_personas.push(NamedPersona { name, account_name, language }),
    }
    save_in(&dir, &prefs)
}

/// Removing a persona that isn't saved is a no-op, not an error — mirrors
/// `index::forget`'s convention for the same reason: a caller shouldn't need
/// to check existence first just to express "make sure this is gone."
pub fn delete_named_persona(name: &str) -> Result<(), AutoGseError> {
    let dir = credentials::store_dir()?;
    let mut prefs = load_in(&dir)?;
    prefs.saved_personas.retain(|p| p.name != name);
    save_in(&dir, &prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_false_when_no_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), Preferences::default());
    }

    #[test]
    fn round_trips_through_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = Preferences { anon_opt_in: true, ..Default::default() };
        save_in(dir.path(), &prefs).unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), prefs);
    }

    /// A preferences file predating a future field must still load.
    #[test]
    fn loads_preferences_missing_newer_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(preferences_path(dir.path()), "{}").unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), Preferences::default());
    }

    #[test]
    fn round_trips_default_persona_fields() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = Preferences {
            default_account_name: Some("jayeff89".to_string()),
            default_language: Some("german".to_string()),
            ..Default::default()
        };
        save_in(dir.path(), &prefs).unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), prefs);
    }

    #[test]
    fn set_default_persona_only_overwrites_supplied_fields() {
        let dir = tempfile::tempdir().unwrap();
        save_in(dir.path(), &Preferences { default_language: Some("english".to_string()), ..Default::default() }).unwrap();

        let mut prefs = load_in(dir.path()).unwrap();
        prefs.default_account_name = Some("jayeff89".to_string());
        save_in(dir.path(), &prefs).unwrap();

        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded.default_account_name.as_deref(), Some("jayeff89"));
        assert_eq!(loaded.default_language.as_deref(), Some("english"));
    }

    #[test]
    fn round_trips_overlay_prefs() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = Preferences {
            overlay_prefs: OverlayPrefs { pos_achievement: Some("top_left".to_string()), duration_achievement: Some(10.0), ..Default::default() },
            ..Default::default()
        };
        save_in(dir.path(), &prefs).unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), prefs);
    }

    #[test]
    fn set_overlay_prefs_only_overwrites_supplied_fields() {
        let dir = tempfile::tempdir().unwrap();
        save_in(
            dir.path(),
            &Preferences {
                overlay_prefs: OverlayPrefs { pos_achievement: Some("bot_right".to_string()), ..Default::default() },
                ..Default::default()
            },
        )
        .unwrap();

        let mut prefs = load_in(dir.path()).unwrap();
        prefs.overlay_prefs.duration_achievement = Some(12.0);
        save_in(dir.path(), &prefs).unwrap();

        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded.overlay_prefs.pos_achievement.as_deref(), Some("bot_right"));
        assert_eq!(loaded.overlay_prefs.duration_achievement, Some(12.0));
    }

    #[test]
    fn loads_preferences_missing_saved_personas_field() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(preferences_path(dir.path()), "{}").unwrap();
        assert_eq!(load_in(dir.path()).unwrap().saved_personas, Vec::new());
    }

    #[test]
    fn round_trips_saved_personas() {
        let dir = tempfile::tempdir().unwrap();
        let persona = NamedPersona { name: "Speedrun".to_string(), account_name: Some("speedy".to_string()), language: Some("english".to_string()) };
        let prefs = Preferences { saved_personas: vec![persona.clone()], ..Default::default() };
        save_in(dir.path(), &prefs).unwrap();
        assert_eq!(load_in(dir.path()).unwrap().saved_personas, vec![persona]);
    }

    /// Exercises the same "update by name, else push" rule
    /// `save_named_persona` applies, without needing to redirect its real
    /// `credentials::store_dir()` call — same style as this file's existing
    /// `set_default_persona_only_overwrites_supplied_fields` test.
    #[test]
    fn saved_personas_update_by_name_instead_of_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        save_in(dir.path(), &Preferences {
            saved_personas: vec![NamedPersona { name: "Speedrun".to_string(), account_name: Some("speedy".to_string()), language: None }],
            ..Default::default()
        })
        .unwrap();

        let mut prefs = load_in(dir.path()).unwrap();
        let existing = prefs.saved_personas.iter_mut().find(|p| p.name == "Speedrun").unwrap();
        existing.language = Some("german".to_string());
        save_in(dir.path(), &prefs).unwrap();

        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded.saved_personas.len(), 1);
        assert_eq!(loaded.saved_personas[0].account_name.as_deref(), Some("speedy"));
        assert_eq!(loaded.saved_personas[0].language.as_deref(), Some("german"));
    }

    #[test]
    fn saved_personas_retain_removes_only_the_matching_entry() {
        let dir = tempfile::tempdir().unwrap();
        save_in(dir.path(), &Preferences {
            saved_personas: vec![
                NamedPersona { name: "A".to_string(), account_name: None, language: None },
                NamedPersona { name: "B".to_string(), account_name: None, language: None },
            ],
            ..Default::default()
        })
        .unwrap();

        let mut prefs = load_in(dir.path()).unwrap();
        prefs.saved_personas.retain(|p| p.name != "A");
        save_in(dir.path(), &prefs).unwrap();

        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded.saved_personas.len(), 1);
        assert_eq!(loaded.saved_personas[0].name, "B");
    }
}
