use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AutoGseError;

pub const MANIFEST_FILENAME: &str = ".gse_manifest.json";
pub const MANIFEST_VERSION: &str = "1.0.0";

/// PRD §5.5.2's real manifest schema.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackedUpFile {
    pub original_path: String,
    pub backup_path: String,
    pub sha256_hash: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GseManifest {
    #[serde(default = "default_version")]
    pub version: String,
    pub timestamp: String,
    pub target_directory: String,
    pub backed_up_files: Vec<BackedUpFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id_source: Option<String>,
    /// Best-effort display name; `None` when no cascade step surfaced one
    /// (e.g. a bare `--appid` override or a local `steam_appid.txt` hit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_title: Option<String>,
    /// Every file AutoGSE wrote or copied into the TOD beyond the DLL
    /// backup/swap (steam_appid.txt, the merged steam_settings/ tree,
    /// steam_interfaces.txt, ...), relative to the TOD. Populated by
    /// accumulation during inject, not a fixed schema-derived list — the
    /// exact set varies per game/tool-output shape, so revert must delete
    /// exactly what was recorded here, nothing assumed.
    #[serde(default)]
    pub injected_files: Vec<String>,
}

fn default_version() -> String {
    MANIFEST_VERSION.to_string()
}

fn manifest_path(target_dir: &Path) -> PathBuf {
    target_dir.join(MANIFEST_FILENAME)
}

pub fn exists(target_dir: &Path) -> bool {
    manifest_path(target_dir).is_file()
}

pub fn load(target_dir: &Path) -> Result<Option<GseManifest>, AutoGseError> {
    let path = manifest_path(target_dir);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)?;
    let manifest = serde_json::from_slice(&bytes)?;
    Ok(Some(manifest))
}

pub fn save(target_dir: &Path, manifest: &GseManifest) -> Result<(), AutoGseError> {
    let path = manifest_path(target_dir);
    let bytes = serde_json::to_vec_pretty(manifest)?;
    std::fs::write(&path, bytes)?;
    Ok(())
}

pub fn remove(target_dir: &Path) -> Result<(), AutoGseError> {
    let path = manifest_path(target_dir);
    if path.is_file() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest(dir: &Path) -> GseManifest {
        GseManifest {
            version: MANIFEST_VERSION.to_string(),
            timestamp: "unix:0".to_string(),
            target_directory: dir.to_string_lossy().into_owned(),
            backed_up_files: vec![BackedUpFile {
                original_path: "steam_api64.dll".to_string(),
                backup_path: "steam_api64.dll.org".to_string(),
                sha256_hash: "a".repeat(64),
            }],
            app_id: Some(1091500),
            arch: Some("x64".to_string()),
            app_id_source: Some("steam_api_fuzzy".to_string()),
            game_title: Some("Cyberpunk 2077".to_string()),
            injected_files: vec!["steam_appid.txt".to_string(), "steam_settings/configs.main.ini".to_string()],
        }
    }

    #[test]
    fn round_trips_through_save_load_remove() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!exists(dir.path()));
        assert!(load(dir.path()).unwrap().is_none());

        let manifest = sample_manifest(dir.path());
        save(dir.path(), &manifest).unwrap();

        assert!(exists(dir.path()));
        let loaded = load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.version, MANIFEST_VERSION);
        assert_eq!(loaded.backed_up_files.len(), 1);
        assert_eq!(loaded.backed_up_files[0].sha256_hash, "a".repeat(64));
        assert_eq!(loaded.app_id, Some(1091500));
        assert_eq!(loaded.arch.as_deref(), Some("x64"));
        assert_eq!(loaded.game_title.as_deref(), Some("Cyberpunk 2077"));
        assert_eq!(loaded.injected_files.len(), 2);

        remove(dir.path()).unwrap();
        assert!(!exists(dir.path()));
        assert!(load(dir.path()).unwrap().is_none());
    }

    /// An older sidecar written before `version`/`game_title`/`injected_files`
    /// existed must still load without error.
    #[test]
    fn loads_manifest_missing_newer_fields() {
        let dir = tempfile::tempdir().unwrap();
        let legacy_json = r#"{
            "timestamp": "unix:0",
            "target_directory": "C:\\Games\\Foo",
            "backed_up_files": []
        }"#;
        std::fs::write(manifest_path(dir.path()), legacy_json).unwrap();

        let loaded = load(dir.path()).unwrap().unwrap();

        assert_eq!(loaded.version, MANIFEST_VERSION);
        assert_eq!(loaded.app_id, None);
        assert_eq!(loaded.game_title, None);
        assert!(loaded.injected_files.is_empty());
    }
}
