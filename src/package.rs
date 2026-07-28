//! Phase 10 §10.2: portable game package export/import.
//!
//! Reuses the vendored `7za.exe` for both directions — the same tool and the
//! same `steam_misc/tools/7za/7za.exe` path beneath `tools_root()` that
//! `acw::deploy_schema`/`goldberg::generate_interfaces` already shell out to
//! for their own zip work — rather than adding a new zip crate dependency.
//! A package is a plain zip: `.gse_manifest.json` at its root plus every path
//! already listed in that manifest's `injected_files[]` (the existing
//! canonical "what AutoGSE wrote" list). Save-side achievement *unlock
//! progress* is deliberately out of scope here — that's `backup_manager`'s
//! job (Phase 8 §8.5); conflating the two would blur two already-separate
//! features.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tempfile::TempDir;

use crate::error::AutoGseError;
use crate::goldberg::{self, run_with_timeout};
use crate::manifest::{self, GseManifest};

/// `7za.exe` archiving/extraction is local, non-networked work — generous
/// headroom over what a real `steam_settings/` tree (icons, fonts, sounds,
/// achievement schema) actually takes, same reasoning as `acw.rs`'s
/// `DEPLOY_TIMEOUT`.
const PACKAGE_TIMEOUT: Duration = Duration::from_secs(60);

fn sevenzip_path() -> Result<PathBuf, AutoGseError> {
    let path = goldberg::tools_root()?.join("steam_misc").join("tools").join("7za").join("7za.exe");
    if path.is_file() {
        Ok(path)
    } else {
        Err(AutoGseError::VendoredToolsNotFound(path))
    }
}

/// `Command::current_dir` resolves a relative argument against the *new*
/// cwd, not the caller's — `export_package` sets `current_dir(tod)` for the
/// archive step, so an output path supplied relative to the caller's own cwd
/// must be made absolute first or it would land (or be looked for) in the
/// wrong place entirely.
fn absolutize(path: &Path) -> Result<PathBuf, AutoGseError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// Builds `out` as a zip containing `.gse_manifest.json` plus every path in
/// `manifest.injected_files` — run with `tod` as the working directory so
/// the archive holds the same TOD-relative paths the manifest already uses,
/// no separate path-rewriting needed on either side.
pub fn export_package(tod: &Path, manifest: &GseManifest, out: &Path) -> Result<(), AutoGseError> {
    let sevenzip = sevenzip_path()?;
    let out_abs = absolutize(out)?;

    // `7za a` appends/updates into an existing archive rather than replacing
    // it outright — a stale prior export at the same path would otherwise
    // silently retain files no longer in `injected_files`.
    if out_abs.is_file() {
        std::fs::remove_file(&out_abs)?;
    }

    let mut cmd = Command::new(&sevenzip);
    cmd.current_dir(tod);
    // `7za` infers the archive container format from `out`'s extension by
    // default — confirmed live: a `.gsepackage`-named output silently became
    // a real `7z` archive, not the zip this format is documented as being.
    // `-tzip` pins the format explicitly so it's deterministic regardless of
    // what extension a caller picks for `--out`.
    cmd.arg("a").arg("-tzip").arg(&out_abs);
    cmd.arg(manifest::MANIFEST_FILENAME);
    for rel in &manifest.injected_files {
        cmd.arg(rel);
    }
    run_with_timeout(cmd, PACKAGE_TIMEOUT, "7za.exe")
}

/// One extracted-and-parsed package, ready for `engine::run_import` to act
/// on. `extracted_dir` is kept alive (RAII) for the caller to copy files out
/// of; it's cleaned up automatically once dropped.
pub struct ExtractedPackage {
    pub manifest: GseManifest,
    pub extracted_dir: TempDir,
}

/// Extracts `package` into a fresh temp dir and parses its root
/// `.gse_manifest.json`. Fails with `PackageInvalid` (not a generic
/// extraction error) when that file is missing or unparseable — a package
/// that isn't AutoGSE's own zip shape, not a corrupted-but-recognizable one.
pub fn extract_package(package: &Path) -> Result<ExtractedPackage, AutoGseError> {
    let sevenzip = sevenzip_path()?;
    let package_abs = absolutize(package)?;
    let extracted_dir = tempfile::Builder::new().prefix("autogse_import_").tempdir()?;

    let mut cmd = Command::new(&sevenzip);
    cmd.arg("x").arg(&package_abs).arg(format!("-o{}", extracted_dir.path().display())).arg("-aoa");
    run_with_timeout(cmd, PACKAGE_TIMEOUT, "7za.exe")?;

    let manifest_path = extracted_dir.path().join(manifest::MANIFEST_FILENAME);
    if !manifest_path.is_file() {
        return Err(AutoGseError::PackageInvalid(format!(
            "{} has no {} at its root — not an AutoGSE export package",
            package.display(),
            manifest::MANIFEST_FILENAME
        )));
    }
    let bytes = std::fs::read(&manifest_path)?;
    let manifest: GseManifest = serde_json::from_slice(&bytes)
        .map_err(|e| AutoGseError::PackageInvalid(format!("corrupt {} in {}: {e}", manifest::MANIFEST_FILENAME, package.display())))?;

    Ok(ExtractedPackage { manifest, extracted_dir })
}

/// Copies every file `injected_files` lists from `extracted_dir` into `tod`,
/// preserving relative structure. Missing entries are skipped rather than
/// treated as fatal — a hand-edited or partially-rebuilt package shouldn't
/// abort an otherwise-usable import halfway through.
pub fn copy_extracted_files(extracted_dir: &Path, tod: &Path, injected_files: &[String]) -> Result<(), AutoGseError> {
    for rel in injected_files {
        let src = extracted_dir.join(rel);
        if !src.is_file() {
            continue;
        }
        let dst = tod.join(rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&src, &dst)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::MANIFEST_VERSION;

    fn sample_manifest(injected_files: Vec<String>) -> GseManifest {
        GseManifest {
            version: MANIFEST_VERSION.to_string(),
            timestamp: "unix:0".to_string(),
            target_directory: "C:\\Games\\Foo".to_string(),
            backed_up_files: vec![],
            app_id: Some(480),
            arch: Some("x64".to_string()),
            app_id_source: Some("local_manifest".to_string()),
            game_title: Some("Spacewar".to_string()),
            injected_files,
            mode: "regular".to_string(),
        }
    }

    #[test]
    fn export_then_extract_round_trips_manifest_and_files() {
        let tod = tempfile::tempdir().unwrap();
        std::fs::write(tod.path().join("steam_appid.txt"), b"480").unwrap();
        std::fs::create_dir_all(tod.path().join("steam_settings")).unwrap();
        std::fs::write(tod.path().join("steam_settings").join("configs.main.ini"), b"[main::general]\r\n").unwrap();

        let manifest = sample_manifest(vec!["steam_appid.txt".to_string(), "steam_settings/configs.main.ini".to_string()]);
        manifest::save(tod.path(), &manifest).unwrap();

        let out_dir = tempfile::tempdir().unwrap();
        let package_path = out_dir.path().join("spacewar.gsepackage");
        export_package(tod.path(), &manifest, &package_path).unwrap();
        assert!(package_path.is_file());

        let extracted = extract_package(&package_path).unwrap();
        assert_eq!(extracted.manifest.app_id, Some(480));
        assert_eq!(extracted.manifest.injected_files.len(), 2);

        let import_tod = tempfile::tempdir().unwrap();
        copy_extracted_files(extracted.extracted_dir.path(), import_tod.path(), &extracted.manifest.injected_files).unwrap();

        assert_eq!(std::fs::read(import_tod.path().join("steam_appid.txt")).unwrap(), b"480");
        assert_eq!(
            std::fs::read(import_tod.path().join("steam_settings").join("configs.main.ini")).unwrap(),
            b"[main::general]\r\n"
        );
    }

    #[test]
    fn extract_package_rejects_a_zip_with_no_manifest() {
        let sevenzip = sevenzip_path().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("not_a_manifest.txt"), b"hello").unwrap();
        let package_path = dir.path().join("bogus.zip");

        let mut cmd = Command::new(&sevenzip);
        cmd.current_dir(dir.path());
        cmd.arg("a").arg(&package_path).arg("not_a_manifest.txt");
        run_with_timeout(cmd, PACKAGE_TIMEOUT, "7za.exe").unwrap();

        let result = extract_package(&package_path);
        assert!(matches!(result, Err(AutoGseError::PackageInvalid(_))));
    }

    #[test]
    fn copy_extracted_files_skips_missing_entries_without_failing() {
        let extracted = tempfile::tempdir().unwrap();
        let tod = tempfile::tempdir().unwrap();
        let result = copy_extracted_files(extracted.path(), tod.path(), &["does_not_exist.txt".to_string()]);
        assert!(result.is_ok());
    }
}
