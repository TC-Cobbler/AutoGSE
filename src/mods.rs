use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AutoGseError;

/// Just the fields `add_mod` itself writes — confirmed against the real
/// vendored `mods.EXAMPLE.json`, whose first (and only "preferred way")
/// entry documents exactly these four as sufficient: "primary file must
/// exist in `steam_settings/mods/<id>` ... preview file must exist in
/// `steam_settings/mods_img/<id>`". Other optional fields the example shows
/// (`steam_id_owner`, `tags`, `upvotes`, ...) are left for a user to hand-add
/// to the JSON afterward — not worth a CLI flag each for a scaffolding tool.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct ModEntry {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    primary_filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    preview_filename: Option<String>,
}

pub struct AddModRequest<'a> {
    pub id: u64,
    pub title: String,
    pub description: Option<String>,
    pub primary_file: &'a Path,
    pub preview_file: Option<&'a Path>,
}

/// Scaffolds one Steam Workshop mod entry into an already-injected target
/// (Phase 6 §6.6): copies the primary file into `steam_settings/mods/<id>/`
/// and, if supplied, the preview file into `steam_settings/mods_img/<id>/`
/// — **two separate directories**, confirmed against the real vendored
/// example (`mods.EXAMPLE/12345/` for the primary file only; preview images
/// live under a sibling `mods_img/<id>/`, not inside the same folder as
/// originally assumed) — then read-modify-writes `steam_settings/mods.json`
/// (same pattern as `acw.rs::register_save_paths`).
///
/// Returns the TOD-relative paths written, for the caller to fold into the
/// manifest's `injected_files[]`.
pub fn add_mod(tod: &Path, req: &AddModRequest) -> Result<Vec<String>, AutoGseError> {
    let settings_dir = tod.join("steam_settings");
    let mut written = Vec::new();

    let primary_name = req
        .primary_file
        .file_name()
        .ok_or_else(|| AutoGseError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "primary file has no filename")))?;
    let mod_dir = settings_dir.join("mods").join(req.id.to_string());
    std::fs::create_dir_all(&mod_dir)?;
    std::fs::copy(req.primary_file, mod_dir.join(primary_name))?;
    written.push(format!("steam_settings/mods/{}/{}", req.id, primary_name.to_string_lossy()));

    let preview_filename = if let Some(preview_file) = req.preview_file {
        let preview_name = preview_file
            .file_name()
            .ok_or_else(|| AutoGseError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "preview file has no filename")))?;
        let preview_dir = settings_dir.join("mods_img").join(req.id.to_string());
        std::fs::create_dir_all(&preview_dir)?;
        std::fs::copy(preview_file, preview_dir.join(preview_name))?;
        written.push(format!("steam_settings/mods_img/{}/{}", req.id, preview_name.to_string_lossy()));
        Some(preview_name.to_string_lossy().into_owned())
    } else {
        None
    };

    let mods_json_path = settings_dir.join("mods.json");
    let mut entries: BTreeMap<String, Value> = if mods_json_path.is_file() {
        serde_json::from_slice(&std::fs::read(&mods_json_path)?).unwrap_or_default()
    } else {
        BTreeMap::new()
    };

    let entry = ModEntry {
        title: req.title.clone(),
        description: req.description.clone(),
        primary_filename: primary_name.to_string_lossy().into_owned(),
        preview_filename,
    };
    entries.insert(req.id.to_string(), serde_json::to_value(entry)?);
    std::fs::write(&mods_json_path, serde_json::to_vec_pretty(&entries)?)?;

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file(dir: &Path, name: &str, contents: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn add_mod_copies_primary_only_and_writes_mods_json() {
        let tod = tempfile::tempdir().unwrap();
        let src_dir = tempfile::tempdir().unwrap();
        let primary = make_file(src_dir.path(), "metadata.json", b"mod data");

        let written = add_mod(
            tod.path(),
            &AddModRequest { id: 12345, title: "Some Workshop Item".to_string(), description: None, primary_file: &primary, preview_file: None },
        )
        .unwrap();

        assert_eq!(written, vec!["steam_settings/mods/12345/metadata.json".to_string()]);
        assert!(tod.path().join("steam_settings/mods/12345/metadata.json").is_file());
        assert!(!tod.path().join("steam_settings/mods_img/12345").exists());

        let mods_json = std::fs::read_to_string(tod.path().join("steam_settings/mods.json")).unwrap();
        let parsed: BTreeMap<String, Value> = serde_json::from_str(&mods_json).unwrap();
        assert_eq!(parsed["12345"]["title"], "Some Workshop Item");
        assert_eq!(parsed["12345"]["primary_filename"], "metadata.json");
        assert!(parsed["12345"].get("preview_filename").is_none());
    }

    #[test]
    fn add_mod_copies_preview_into_separate_mods_img_dir() {
        let tod = tempfile::tempdir().unwrap();
        let src_dir = tempfile::tempdir().unwrap();
        let primary = make_file(src_dir.path(), "test.sav", b"save data");
        let preview = make_file(src_dir.path(), "preview.png", b"fake png bytes");

        let written = add_mod(
            tod.path(),
            &AddModRequest {
                id: 111,
                title: "Example".to_string(),
                description: Some("desc".to_string()),
                primary_file: &primary,
                preview_file: Some(&preview),
            },
        )
        .unwrap();

        assert_eq!(
            written,
            vec!["steam_settings/mods/111/test.sav".to_string(), "steam_settings/mods_img/111/preview.png".to_string()]
        );
        assert!(tod.path().join("steam_settings/mods/111/test.sav").is_file());
        assert!(tod.path().join("steam_settings/mods_img/111/preview.png").is_file());

        let mods_json = std::fs::read_to_string(tod.path().join("steam_settings/mods.json")).unwrap();
        let parsed: BTreeMap<String, Value> = serde_json::from_str(&mods_json).unwrap();
        assert_eq!(parsed["111"]["preview_filename"], "preview.png");
        assert_eq!(parsed["111"]["description"], "desc");
    }

    #[test]
    fn add_mod_preserves_existing_entries_and_dedups_on_id_reuse() {
        let tod = tempfile::tempdir().unwrap();
        let settings_dir = tod.path().join("steam_settings");
        std::fs::create_dir_all(&settings_dir).unwrap();
        std::fs::write(settings_dir.join("mods.json"), r#"{"999": {"title": "Pre-existing", "primary_filename": "x.dat"}}"#).unwrap();

        let src_dir = tempfile::tempdir().unwrap();
        let primary = make_file(src_dir.path(), "new.dat", b"data");
        add_mod(
            tod.path(),
            &AddModRequest { id: 999, title: "Replaced".to_string(), description: None, primary_file: &primary, preview_file: None },
        )
        .unwrap();

        let mods_json = std::fs::read_to_string(settings_dir.join("mods.json")).unwrap();
        let parsed: BTreeMap<String, Value> = serde_json::from_str(&mods_json).unwrap();
        assert_eq!(parsed.len(), 1, "re-adding the same id must replace, not duplicate");
        assert_eq!(parsed["999"]["title"], "Replaced");
    }

    #[test]
    fn add_mod_preserves_unrelated_existing_mod_entries() {
        let tod = tempfile::tempdir().unwrap();
        let settings_dir = tod.path().join("steam_settings");
        std::fs::create_dir_all(&settings_dir).unwrap();
        std::fs::write(settings_dir.join("mods.json"), r#"{"1": {"title": "Other Mod", "primary_filename": "a.dat"}}"#).unwrap();

        let src_dir = tempfile::tempdir().unwrap();
        let primary = make_file(src_dir.path(), "b.dat", b"data");
        add_mod(tod.path(), &AddModRequest { id: 2, title: "New Mod".to_string(), description: None, primary_file: &primary, preview_file: None })
            .unwrap();

        let mods_json = std::fs::read_to_string(settings_dir.join("mods.json")).unwrap();
        let parsed: BTreeMap<String, Value> = serde_json::from_str(&mods_json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed["1"]["title"], "Other Mod");
        assert_eq!(parsed["2"]["title"], "New Mod");
    }
}
