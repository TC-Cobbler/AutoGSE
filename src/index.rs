use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::credentials;
use crate::error::AutoGseError;
use crate::manifest;

const INDEX_FILENAME: &str = "known_targets.json";

/// Local record of every folder AutoGSE has successfully injected on this
/// machine (Phase 6 §6.8's `autogse list`), keyed on TOD path. There is no
/// other way to answer "what has AutoGSE touched?" without this — unlike
/// `scan --root`, which only sees what's under a given folder right now.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
struct Index {
    #[serde(default)]
    targets: Vec<String>,
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join(INDEX_FILENAME)
}

fn load_in(dir: &Path) -> Result<Vec<PathBuf>, AutoGseError> {
    let path = index_path(dir);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(&path)?;
    let index: Index = serde_json::from_slice(&bytes)?;
    Ok(index.targets.into_iter().map(PathBuf::from).collect())
}

fn save_in(dir: &Path, targets: &[PathBuf]) -> Result<(), AutoGseError> {
    std::fs::create_dir_all(dir)?;
    let index = Index { targets: targets.iter().map(|p| p.to_string_lossy().into_owned()).collect() };
    std::fs::write(index_path(dir), serde_json::to_vec_pretty(&index)?)?;
    Ok(())
}

/// Records `tod` as a known target, called on every successful `inject`.
/// Idempotent — re-injecting an already-recorded target doesn't duplicate it.
pub fn record(tod: &Path) -> Result<(), AutoGseError> {
    record_in(&credentials::store_dir()?, tod)
}

fn record_in(dir: &Path, tod: &Path) -> Result<(), AutoGseError> {
    let mut targets = load_in(dir)?;
    if !targets.iter().any(|t| t == tod) {
        targets.push(tod.to_path_buf());
        save_in(dir, &targets)?;
    }
    Ok(())
}

/// Prunes `tod` from the index, called on every successful `revert`.
pub fn forget(tod: &Path) -> Result<(), AutoGseError> {
    forget_in(&credentials::store_dir()?, tod)
}

fn forget_in(dir: &Path, tod: &Path) -> Result<(), AutoGseError> {
    let mut targets = load_in(dir)?;
    let before = targets.len();
    targets.retain(|t| t != tod);
    if targets.len() != before {
        save_in(dir, &targets)?;
    }
    Ok(())
}

/// The `list` subcommand's data source: self-heals against manual deletion
/// or an out-of-band revert by dropping (and persisting the drop of) any
/// entry whose path no longer exists or no longer carries a manifest,
/// rather than reporting stale entries forever.
pub fn load_existing_injected() -> Result<Vec<PathBuf>, AutoGseError> {
    load_existing_injected_in(&credentials::store_dir()?)
}

fn load_existing_injected_in(dir: &Path) -> Result<Vec<PathBuf>, AutoGseError> {
    let targets = load_in(dir)?;
    let alive: Vec<PathBuf> = targets.iter().filter(|t| manifest::exists(t)).cloned().collect();
    if alive.len() != targets.len() {
        save_in(dir, &alive)?;
    }
    Ok(alive)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let target = PathBuf::from("C:\\Games\\Foo");
        record_in(dir.path(), &target).unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), vec![target]);
    }

    #[test]
    fn record_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let target = PathBuf::from("C:\\Games\\Foo");
        record_in(dir.path(), &target).unwrap();
        record_in(dir.path(), &target).unwrap();
        assert_eq!(load_in(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn forget_removes_only_the_matching_entry() {
        let dir = tempfile::tempdir().unwrap();
        let a = PathBuf::from("C:\\Games\\A");
        let b = PathBuf::from("C:\\Games\\B");
        record_in(dir.path(), &a).unwrap();
        record_in(dir.path(), &b).unwrap();

        forget_in(dir.path(), &a).unwrap();

        assert_eq!(load_in(dir.path()).unwrap(), vec![b]);
    }

    #[test]
    fn forget_of_unknown_target_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        forget_in(dir.path(), &PathBuf::from("C:\\never\\recorded")).unwrap();
    }

    #[test]
    fn load_in_defaults_to_empty_when_no_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_in(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn load_existing_injected_drops_targets_without_a_real_manifest() {
        let store_dir = tempfile::tempdir().unwrap();
        let still_injected = tempfile::tempdir().unwrap();
        let manually_reverted = tempfile::tempdir().unwrap();

        std::fs::write(still_injected.path().join(manifest::MANIFEST_FILENAME), "{}").unwrap();
        // manually_reverted has no manifest file - simulates the user
        // deleting it outside of `autogse revert`.

        record_in(store_dir.path(), still_injected.path()).unwrap();
        record_in(store_dir.path(), manually_reverted.path()).unwrap();

        let alive = load_existing_injected_in(store_dir.path()).unwrap();

        assert_eq!(alive, vec![still_injected.path().to_path_buf()]);
        // The self-heal must have persisted, not just filtered in memory.
        assert_eq!(load_in(store_dir.path()).unwrap(), vec![still_injected.path().to_path_buf()]);
    }
}
