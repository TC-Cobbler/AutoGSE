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
}
