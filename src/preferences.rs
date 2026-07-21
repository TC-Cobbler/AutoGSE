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
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct Preferences {
    #[serde(default)]
    pub anon_opt_in: bool,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_false_when_no_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), Preferences { anon_opt_in: false });
    }

    #[test]
    fn round_trips_through_save_load() {
        let dir = tempfile::tempdir().unwrap();
        save_in(dir.path(), &Preferences { anon_opt_in: true }).unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), Preferences { anon_opt_in: true });
    }

    /// A preferences file predating a future field must still load.
    #[test]
    fn loads_preferences_missing_newer_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(preferences_path(dir.path()), "{}").unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), Preferences { anon_opt_in: false });
    }
}
