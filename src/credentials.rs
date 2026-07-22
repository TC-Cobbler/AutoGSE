use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows::core::PCWSTR;

use crate::error::AutoGseError;

const CREDENTIALS_FILENAME: &str = "credentials.dat";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// `%LOCALAPPDATA%\AutoGSE` — DPAPI itself (not this path) is what actually
/// scopes `credentials.dat` to the current Windows user, so this doesn't
/// need to be anywhere more exotic.
pub fn store_dir() -> Result<PathBuf, AutoGseError> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        AutoGseError::Credentials("LOCALAPPDATA environment variable is not set".to_string())
    })?;
    Ok(PathBuf::from(local_app_data).join("AutoGSE"))
}

fn credentials_path(dir: &Path) -> PathBuf {
    dir.join(CREDENTIALS_FILENAME)
}

/// Encrypts `plaintext` with DPAPI, scoped to the current Windows user +
/// machine (no custom entropy/key needed — that's the whole point of using
/// DPAPI over a hand-rolled AES scheme). `CRYPTPROTECT_UI_FORBIDDEN` makes a
/// failure return an error instead of silently blocking on a Windows UI
/// prompt we'd never see from a CLI.
fn dpapi_protect(plaintext: &mut [u8]) -> Result<Vec<u8>, AutoGseError> {
    let input = CRYPT_INTEGER_BLOB { cbData: plaintext.len() as u32, pbData: plaintext.as_mut_ptr() };
    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptProtectData(&input, PCWSTR::null(), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            .map_err(|e| AutoGseError::Credentials(format!("CryptProtectData failed: {e}")))?;
    }

    let encrypted = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut _)));
    }
    Ok(encrypted)
}

fn dpapi_unprotect(ciphertext: &mut [u8]) -> Result<Vec<u8>, AutoGseError> {
    let input = CRYPT_INTEGER_BLOB { cbData: ciphertext.len() as u32, pbData: ciphertext.as_mut_ptr() };
    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            .map_err(|e| AutoGseError::Credentials(format!("CryptUnprotectData failed: {e}")))?;
    }

    let decrypted = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut _)));
    }
    Ok(decrypted)
}

pub fn save_in(dir: &Path, creds: &Credentials) -> Result<(), AutoGseError> {
    std::fs::create_dir_all(dir)?;
    let mut plaintext = serde_json::to_vec(creds)?;
    let encrypted = dpapi_protect(&mut plaintext)?;
    std::fs::write(credentials_path(dir), encrypted)?;
    Ok(())
}

pub fn load_in(dir: &Path) -> Result<Option<Credentials>, AutoGseError> {
    let path = credentials_path(dir);
    if !path.is_file() {
        return Ok(None);
    }
    let mut encrypted = std::fs::read(&path)?;
    let decrypted = dpapi_unprotect(&mut encrypted)?;
    let creds = serde_json::from_slice(&decrypted)?;
    Ok(Some(creds))
}

pub fn delete_in(dir: &Path) -> Result<(), AutoGseError> {
    let path = credentials_path(dir);
    if path.is_file() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn save(creds: &Credentials) -> Result<(), AutoGseError> {
    save_in(&store_dir()?, creds)
}

pub fn load() -> Result<Option<Credentials>, AutoGseError> {
    load_in(&store_dir()?)
}

pub fn delete() -> Result<(), AutoGseError> {
    delete_in(&store_dir()?)
}

/// A pure DPAPI round-trip self-test (Phase 6 §6.9's `doctor` subcommand) —
/// touches neither the real `credentials.dat` file nor any real secret, just
/// proves this Windows user profile's DPAPI store is reachable.
pub fn self_test() -> Result<(), AutoGseError> {
    const PROBE: &[u8] = b"autogse-doctor-self-test";
    let mut plaintext = PROBE.to_vec();
    let mut encrypted = dpapi_protect(&mut plaintext)?;
    let decrypted = dpapi_unprotect(&mut encrypted)?;
    if decrypted != PROBE {
        return Err(AutoGseError::Credentials("DPAPI round-trip produced mismatched plaintext".to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Credentials {
        Credentials { username: "steam_user".to_string(), password: "hunter2".to_string() }
    }

    #[test]
    fn round_trips_through_save_load_delete() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_in(dir.path()).unwrap().is_none());

        save_in(dir.path(), &sample()).unwrap();
        let loaded = load_in(dir.path()).unwrap().unwrap();
        assert_eq!(loaded, sample());

        delete_in(dir.path()).unwrap();
        assert!(load_in(dir.path()).unwrap().is_none());
    }

    #[test]
    fn stored_file_is_not_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        save_in(dir.path(), &sample()).unwrap();

        let on_disk = std::fs::read(credentials_path(dir.path())).unwrap();
        let on_disk_str = String::from_utf8_lossy(&on_disk);
        assert!(!on_disk_str.contains("hunter2"));
        assert!(!on_disk_str.contains("steam_user"));
    }

    #[test]
    fn delete_of_nonexistent_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        delete_in(dir.path()).unwrap();
    }

    #[test]
    fn self_test_round_trips_successfully() {
        self_test().unwrap();
    }
}
