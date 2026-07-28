//! Phase 10 §10.3: wraps the vendored `Steamless.CLI.exe`
//! (`Steamless/Steamless.CLI.exe`) to strip Valve's SteamStub DRM wrapper
//! from a game's main executable before injecting Goldberg — some
//! SteamStub-wrapped binaries won't run correctly (or at all) once
//! `steam_api(64).dll` is swapped underneath them.
//!
//! Every real behavior documented below was confirmed live against the
//! actual vendored binary (`Steamless.CLI.exe` with no args, then against
//! real copies of a real SteamStub-protected game executable and a real
//! non-protected one) — not guessed from the tool's GitHub README, which
//! describes a different argument order (`[options] [file]`) and a
//! `-verbose` flag that doesn't actually exist.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::error::AutoGseError;
use crate::goldberg::{self, run_with_timeout};

/// A local, non-networked binary-rewrite tool — generous headroom over what
/// unpacking a real game exe (tens of MB) actually takes, same reasoning as
/// this codebase's other local-tool timeouts (e.g. `acw.rs`'s `7za.exe`
/// deploy).
const UNPACK_TIMEOUT: Duration = Duration::from_secs(30);

/// The exact stdout line Steamless prints when a binary isn't SteamStub-
/// protected — confirmed live against a real non-protected exe. Exit code
/// alone can't distinguish this benign case from a real invocation error:
/// both are exit code `1`.
const NOT_PACKED_MARKER: &str = "All unpackers failed to unpack file.";

fn cli_exe_path() -> Result<PathBuf, AutoGseError> {
    let path = goldberg::steamless_root()?.join("Steamless.CLI.exe");
    if path.is_file() {
        Ok(path)
    } else {
        Err(AutoGseError::VendoredToolsNotFound(path))
    }
}

/// The path Steamless writes its unpacked result to — confirmed live to be
/// the original file's full name (including its own `.exe` extension) with
/// `.unpacked.exe` appended, e.g. `METAL GEAR SOLID.exe.unpacked.exe`, not an
/// extension swap like `METAL GEAR SOLID.unpacked.exe`.
fn unpacked_path_for(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".unpacked.exe");
    exe.with_file_name(name)
}

/// Runs Steamless against `exe` (left untouched on disk either way — the
/// caller decides whether/how to swap the result in). `Ok(Some(path))` is a
/// real successful unpack (confirmed live: exit 0, `Successfully unpacked
/// file!`); `Ok(None)` means `exe` isn't SteamStub-protected — a normal,
/// expected outcome for most games, not a failure; `Err` covers everything
/// else (bad invocation, a real unpacker failure, missing vendored binary).
pub fn unpack(exe: &Path) -> Result<Option<PathBuf>, AutoGseError> {
    let cli = cli_exe_path()?;

    let mut cmd = Command::new(&cli);
    cmd.arg(exe);
    match run_with_timeout(cmd, UNPACK_TIMEOUT, "Steamless.CLI.exe") {
        Ok(()) => {
            let unpacked = unpacked_path_for(exe);
            if unpacked.is_file() {
                Ok(Some(unpacked))
            } else {
                // Confirmed-live success shape didn't actually produce the
                // file it should have — a real anomaly, not the documented
                // "not packed" outcome, so this is a hard error, not `None`.
                Err(AutoGseError::ExternalToolFailed {
                    tool: "Steamless.CLI.exe".to_string(),
                    message: format!("reported success but {} was not created", unpacked.display()),
                })
            }
        }
        Err(AutoGseError::ExternalToolFailed { message, .. }) if message.contains(NOT_PACKED_MARKER) => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacked_path_for_appends_after_the_original_extension() {
        let exe = Path::new("C:\\Games\\Foo\\METAL GEAR SOLID.exe");
        assert_eq!(unpacked_path_for(exe), Path::new("C:\\Games\\Foo\\METAL GEAR SOLID.exe.unpacked.exe"));
    }

    /// Real, live-confirmed integration test (Phase 10 §10.3's planning
    /// pass): runs the actual vendored `Steamless.CLI.exe` against a real
    /// copy of this machine's installed Metal Gear Solid (a genuine
    /// SteamStub Variant 3.1 (x64)-wrapped binary). `#[ignore]`d since it
    /// depends on a real Steam library existing on the machine running the
    /// test, same convention as this codebase's other `live_*` tests.
    #[test]
    #[ignore = "live: requires a real SteamStub-protected exe on this machine"]
    fn live_unpack_real_mgs1_executable() {
        let real_exe = Path::new("D:\\SteamLibrary\\steamapps\\common\\MGS1\\METAL GEAR SOLID.exe");
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("mgs1_test.exe");
        std::fs::copy(real_exe, &copy).unwrap();

        let result = unpack(&copy).unwrap();
        let unpacked = result.expect("a real SteamStub-wrapped exe must unpack, not report None");
        assert!(unpacked.is_file());
        assert!(std::fs::metadata(&unpacked).unwrap().len() > 0);
    }

    #[test]
    #[ignore = "live: requires the vendored Steamless.CLI.exe and a real non-SteamStub exe"]
    fn live_unpack_non_protected_exe_returns_none() {
        let real_exe = Path::new("D:\\SteamLibrary\\steamapps\\common\\MGS2\\launcher.exe");
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("notpacked_test.exe");
        std::fs::copy(real_exe, &copy).unwrap();

        assert_eq!(unpack(&copy).unwrap(), None);
    }
}
