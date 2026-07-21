use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufReader, Read};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use windows::Win32::Storage::FileSystem::{
    GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_READONLY, FILE_FLAGS_AND_ATTRIBUTES,
    INVALID_FILE_ATTRIBUTES,
};
use windows::core::PCWSTR;

use crate::error::AutoGseError;
use crate::manifest::BackedUpFile;

fn to_wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Clears the READONLY attribute on `path` if set. No-op otherwise.
pub fn strip_readonly(path: &Path) -> Result<(), AutoGseError> {
    let wide = to_wide(path.as_os_str());
    unsafe {
        let attrs = GetFileAttributesW(PCWSTR(wide.as_ptr()));
        if attrs == INVALID_FILE_ATTRIBUTES {
            // Let the caller's subsequent file op surface the real error.
            return Ok(());
        }
        if attrs & FILE_ATTRIBUTE_READONLY.0 != 0 {
            let cleared = FILE_FLAGS_AND_ATTRIBUTES(attrs & !FILE_ATTRIBUTE_READONLY.0);
            SetFileAttributesW(PCWSTR(wide.as_ptr()), cleared)
                .map_err(|e| AutoGseError::Io(std::io::Error::other(e.to_string())))?;
        }
    }
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String, AutoGseError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn backup_path_for(original: &Path) -> PathBuf {
    let mut name = original.file_name().unwrap_or_default().to_os_string();
    name.push(".org");
    original.with_file_name(name)
}

/// Idempotently ensures `original` has been renamed to its `.org` backup.
/// If a backup already exists, it is left untouched (not re-created from a
/// possibly-already-swapped `original`), and its existing hash is returned.
pub fn ensure_backed_up(original: &Path) -> Result<BackedUpFile, AutoGseError> {
    let backup = backup_path_for(original);

    if !backup.is_file() {
        strip_readonly(original)?;
        std::fs::rename(original, &backup)?;
    }

    let hash = sha256_file(&backup)?;
    Ok(BackedUpFile {
        original_path: original.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        backup_path: backup.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        sha256_hash: hash,
    })
}

/// Copies `src` to `dst` via a temp-sibling-then-rename sequence so a reader
/// of `dst` never observes a partially-written file (plain `fs::copy` writes
/// in place and is not interruption-safe).
pub fn atomic_copy(src: &Path, dst: &Path) -> Result<(), AutoGseError> {
    let tmp_name = format!(
        ".{}.autogse_tmp",
        dst.file_name().unwrap_or_default().to_string_lossy()
    );
    let tmp_path = dst.with_file_name(tmp_name);

    std::fs::copy(src, &tmp_path)?;
    if dst.is_file() {
        strip_readonly(dst)?;
    }
    std::fs::rename(&tmp_path, dst)?;
    Ok(())
}

/// Verifies the backup's hash still matches what was recorded, then renames
/// it back over `original`. Aborts (without touching anything) on mismatch,
/// since that means the backup was tampered with or corrupted and blindly
/// restoring it could hand the user a broken game binary.
pub fn restore_backup(original: &Path, entry: &BackedUpFile, target_dir: &Path) -> Result<(), AutoGseError> {
    let backup = target_dir.join(&entry.backup_path);
    let actual = sha256_file(&backup)?;
    if actual != entry.sha256_hash {
        return Err(AutoGseError::HashMismatch {
            path: backup.clone(),
            expected: entry.sha256_hash.clone(),
            actual,
        });
    }

    if original.is_file() {
        strip_readonly(original)?;
        std::fs::remove_file(original)?;
    }
    std::fs::rename(&backup, original)?;
    Ok(())
}

/// Renames a pre-existing `dir` aside to `<name>.bak_<unix_ts>` (appending a
/// numeric suffix on the rare same-second collision) rather than overwriting
/// it, per PRD §8's "Existing Goldberg Config Present" edge case. Returns
/// `None` (no-op) if `dir` doesn't exist. This is a one-way safety net, not
/// auto-restored by revert — see `main.rs`'s revert flow.
pub fn backup_existing_dir(dir: &Path) -> Result<Option<PathBuf>, AutoGseError> {
    if !dir.is_dir() {
        return Ok(None);
    }

    let name = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    let mut backup_path = dir.with_file_name(format!("{name}.bak_{timestamp}"));
    let mut suffix = 1u32;
    while backup_path.exists() {
        backup_path = dir.with_file_name(format!("{name}.bak_{timestamp}_{suffix}"));
        suffix += 1;
    }

    std::fs::rename(dir, &backup_path)?;
    Ok(Some(backup_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_backed_up_renames_and_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("steam_api64.dll");
        std::fs::write(&original, b"vanilla bytes").unwrap();

        let entry = ensure_backed_up(&original).unwrap();

        assert!(!original.exists());
        assert!(dir.path().join("steam_api64.dll.org").exists());
        assert_eq!(entry.sha256_hash, sha256_file(&dir.path().join("steam_api64.dll.org")).unwrap());
    }

    #[test]
    fn ensure_backed_up_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("steam_api64.dll");
        std::fs::write(&original, b"vanilla bytes").unwrap();

        let first = ensure_backed_up(&original).unwrap();

        // Simulate a second inject attempt where "original" is now the
        // emulator payload, not the real game DLL.
        std::fs::write(&original, b"emulator payload").unwrap();
        let second = ensure_backed_up(&original).unwrap();

        assert_eq!(first.sha256_hash, second.sha256_hash, "existing .org backup must not be overwritten");
    }

    #[test]
    fn atomic_copy_produces_matching_contents() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.bin");
        let dst = dir.path().join("dst.bin");
        std::fs::write(&src, b"payload").unwrap();

        atomic_copy(&src, &dst).unwrap();

        assert_eq!(std::fs::read(&dst).unwrap(), b"payload");
        // No leftover temp sibling.
        assert!(dir.path().read_dir().unwrap().count() == 2);
    }

    #[test]
    fn restore_backup_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("steam_api64.dll");
        std::fs::write(&original, b"vanilla bytes").unwrap();
        let entry = ensure_backed_up(&original).unwrap();
        atomic_copy(&dir.path().join(&entry.backup_path), &original).unwrap();

        restore_backup(&original, &entry, dir.path()).unwrap();

        assert_eq!(std::fs::read(&original).unwrap(), b"vanilla bytes");
        assert!(!dir.path().join(&entry.backup_path).exists());
    }

    #[test]
    fn restore_backup_aborts_on_hash_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("steam_api64.dll");
        std::fs::write(&original, b"vanilla bytes").unwrap();
        let mut entry = ensure_backed_up(&original).unwrap();
        entry.sha256_hash = "0".repeat(64); // tamper

        let result = restore_backup(&original, &entry, dir.path());

        assert!(matches!(result, Err(AutoGseError::HashMismatch { .. })));
        assert!(dir.path().join(&entry.backup_path).exists(), "backup must be left untouched on mismatch");
    }

    #[test]
    fn backup_existing_dir_renames_aside() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("steam_settings");
        std::fs::create_dir(&settings).unwrap();
        std::fs::write(settings.join("marker.txt"), b"pre-existing").unwrap();

        let backup = backup_existing_dir(&settings).unwrap().unwrap();

        assert!(!settings.exists());
        assert!(backup.is_dir());
        assert!(backup.file_name().unwrap().to_string_lossy().starts_with("steam_settings.bak_"));
        assert_eq!(std::fs::read(backup.join("marker.txt")).unwrap(), b"pre-existing");
    }

    #[test]
    fn backup_existing_dir_is_noop_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("steam_settings");

        assert_eq!(backup_existing_dir(&settings).unwrap(), None);
    }

    #[test]
    fn backup_existing_dir_avoids_collision() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("steam_settings");
        std::fs::create_dir(&settings).unwrap();

        // Pre-create the exact backup name a same-second call would pick.
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let colliding = dir.path().join(format!("steam_settings.bak_{timestamp}"));
        std::fs::create_dir(&colliding).unwrap();

        let backup = backup_existing_dir(&settings).unwrap().unwrap();

        assert_ne!(backup, colliding, "must not collide with an existing .bak_ path");
        assert!(backup.file_name().unwrap().to_string_lossy().starts_with(&format!("steam_settings.bak_{timestamp}")));
    }
}
