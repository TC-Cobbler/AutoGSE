//! Phase 8 §8.3's corrected scope: the vendored README documents no generic
//! "Xbox/DualSense/Switch Pro" controller template at all — confirmed via
//! `alex47exe-gse_fork/release/README.release.md`'s own `## Controller`
//! section. Real controller support is per-*game* (`generate_emu_config
//! --controller`'s automatic download, or `parse_controller_vdf` against a
//! game's own Steam-Workshop-hosted `.vdf`, both already built in Phase 6
//! §6.2). The one concrete, real gap that same README section does name:
//! copying the *real* Steam glyph images from the user's actual Steam
//! install in place of the free example glyphs `generate_emu_config`
//! already deploys unconditionally.
//!
//! **Correction, found by checking live rather than trusting the README's
//! wording**: the README says `<Steam>\tenfoot\resource\images\library\controller\api`,
//! but on this machine's real, current Steam install that folder doesn't
//! exist at all — the real glyph images live one level over, in a sibling
//! `binding_icons` folder (`...\controller\binding_icons`), confirmed by
//! directly listing the real installed directory tree. Valve has evidently
//! reorganized this since the README was written; `api` is stale.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::PCWSTR;

use crate::backup;
use crate::error::AutoGseError;

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Reads `HKEY_CURRENT_USER\Software\Valve\Steam\SteamPath` — the
/// conventional per-user registry location for a real Steam install.
/// `None` if Steam isn't installed (or the key doesn't exist) — best-effort,
/// never an error, same convention as this codebase's other
/// nothing-to-do-here resolvers (`header_cache`, `acw::deploy_schema`).
pub fn find_steam_install_dir() -> Option<PathBuf> {
    let subkey = to_wide("Software\\Valve\\Steam");
    let value_name = to_wide("SteamPath");
    let mut buf = [0u16; 512];
    let mut buf_len = (buf.len() * std::mem::size_of::<u16>()) as u32;

    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value_name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut buf_len),
        )
    };

    if result != ERROR_SUCCESS {
        return None;
    }

    // `buf_len` is a byte count including the trailing NUL on success.
    let chars = (buf_len as usize / std::mem::size_of::<u16>()).saturating_sub(1);
    let value = String::from_utf16_lossy(&buf[..chars]);
    if value.is_empty() { None } else { Some(PathBuf::from(value)) }
}

fn real_glyphs_source(steam_dir: &Path) -> PathBuf {
    // Confirmed live against a real, current Steam install (see this
    // module's own doc comment) — `binding_icons`, not the README's `api`.
    steam_dir.join("tenfoot").join("resource").join("images").join("library").join("controller").join("binding_icons")
}

/// Copies the real Steam controller glyph images into an already-injected
/// target's `steam_settings/controller/glyphs`, replacing the free example
/// glyphs `generate_emu_config` deploys unconditionally. Returns `Ok(false)`
/// (not an error) when Steam isn't installed, the real glyphs folder isn't
/// present, or `tod` isn't an injected target — best-effort, not required
/// for the game to work.
pub fn deploy_real_glyphs(tod: &Path) -> Result<bool, AutoGseError> {
    deploy_real_glyphs_from(tod, find_steam_install_dir().as_deref())
}

fn deploy_real_glyphs_from(tod: &Path, steam_dir: Option<&Path>) -> Result<bool, AutoGseError> {
    let Some(steam_dir) = steam_dir else { return Ok(false) };
    let glyphs_src = real_glyphs_source(steam_dir);
    if !glyphs_src.is_dir() {
        return Ok(false);
    }
    if !tod.join("steam_settings").is_dir() {
        return Ok(false);
    }

    let glyphs_dst = tod.join("steam_settings").join("controller").join("glyphs");
    std::fs::create_dir_all(&glyphs_dst)?;
    for entry in std::fs::read_dir(&glyphs_src)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            backup::atomic_copy(&entry.path(), &glyphs_dst.join(entry.file_name()))?;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deploy_real_glyphs_is_noop_when_steam_not_found() {
        let tod = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tod.path().join("steam_settings")).unwrap();
        assert_eq!(deploy_real_glyphs_from(tod.path(), None).unwrap(), false);
    }

    #[test]
    fn deploy_real_glyphs_is_noop_when_glyphs_source_missing() {
        let tod = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tod.path().join("steam_settings")).unwrap();
        let fake_steam = tempfile::tempdir().unwrap();
        assert_eq!(deploy_real_glyphs_from(tod.path(), Some(fake_steam.path())).unwrap(), false);
    }

    #[test]
    fn deploy_real_glyphs_is_noop_when_target_not_injected() {
        let tod = tempfile::tempdir().unwrap(); // no steam_settings/ at all
        let fake_steam = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(real_glyphs_source(fake_steam.path())).unwrap();
        assert_eq!(deploy_real_glyphs_from(tod.path(), Some(fake_steam.path())).unwrap(), false);
    }

    #[test]
    fn deploy_real_glyphs_copies_real_files() {
        let tod = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tod.path().join("steam_settings")).unwrap();
        let fake_steam = tempfile::tempdir().unwrap();
        let src = real_glyphs_source(fake_steam.path());
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("xbox_a.png"), b"fake glyph bytes").unwrap();

        let result = deploy_real_glyphs_from(tod.path(), Some(fake_steam.path())).unwrap();

        assert!(result);
        assert_eq!(std::fs::read(tod.path().join("steam_settings/controller/glyphs/xbox_a.png")).unwrap(), b"fake glyph bytes");
    }

    /// Manual QA only (reads this machine's real Steam registry key/install,
    /// not run in normal `cargo test`): `cargo test
    /// controller_glyphs::tests::live_find_real_steam_install -- --ignored`
    #[test]
    #[ignore]
    fn live_find_real_steam_install() {
        let dir = find_steam_install_dir().expect("Steam should be installed on this machine");
        assert!(dir.is_dir(), "resolved Steam install dir does not exist: {}", dir.display());
        println!("real Steam install dir: {}", dir.display());
        println!("real glyphs source: {}", real_glyphs_source(&dir).display());
    }

    /// Manual QA only: exercises the *full* real pipeline (registry lookup
    /// -> real glyph source -> real copy) against a synthetic injected
    /// target, not just the registry lookup in isolation:
    /// `cargo test controller_glyphs::tests::live_deploy_into_synthetic_target -- --ignored`
    #[test]
    #[ignore]
    fn live_deploy_into_synthetic_target() {
        let tod = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tod.path().join("steam_settings")).unwrap();

        let deployed = deploy_real_glyphs(tod.path()).expect("deploy should not error");
        assert!(deployed, "expected real Steam glyphs to be found and deployed on this machine");

        let dst = tod.path().join("steam_settings/controller/glyphs");
        let count = std::fs::read_dir(&dst).unwrap().count();
        assert!(count > 0, "expected at least one real glyph file copied");
        println!("deployed {count} real glyph file(s) into {}", dst.display());
    }
}
