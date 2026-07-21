//! Cross-process validation of `AutoGseLock`: spawns two real `autogse`
//! processes concurrently against the same target directory and asserts the
//! named-mutex serialization keeps the backup/state artifacts consistent
//! rather than racing each other.

use std::path::Path;
use std::process::Command;

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_autogse")
}

/// Minimal synthetic PE bytes with a valid x64 DOS/NT header, since Phase 2's
/// pipeline now runs bitness detection during inject.
fn write_fake_dll(dir: &Path) {
    let mut buf = vec![0u8; 0x86];
    buf[0..2].copy_from_slice(b"MZ");
    buf[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    buf[0x80..0x84].copy_from_slice(b"PE\0\0");
    buf[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes()); // IMAGE_FILE_MACHINE_AMD64
    std::fs::write(dir.join("steam_api64.dll"), &buf).unwrap();
}

/// Exercises the real pipeline (network + `generate_emu_config.exe`) twice
/// concurrently, so it's `#[ignore]`d like the project's other live tests.
/// Manual run: `cargo test --test concurrent_lock -- --ignored --nocapture`
#[test]
#[ignore]
fn two_concurrent_injects_do_not_corrupt_state() {
    let dir = tempfile::tempdir().unwrap();
    write_fake_dll(dir.path());

    // --appid short-circuits App ID discovery, but generate_emu_config.exe
    // still runs live for real config generation.
    let mut child_a = Command::new(bin_path())
        .args(["inject", "--path"])
        .arg(dir.path())
        .args(["--silent", "--appid", "1091500"])
        .spawn()
        .unwrap();
    let mut child_b = Command::new(bin_path())
        .args(["inject", "--path"])
        .arg(dir.path())
        .args(["--silent", "--appid", "1091500"])
        .spawn()
        .unwrap();

    let status_a = child_a.wait().unwrap();
    let status_b = child_b.wait().unwrap();

    assert!(status_a.success(), "first concurrent inject should succeed");
    assert!(status_b.success(), "second concurrent inject should succeed (no-op via self-guard once the first wins)");

    let backup = dir.path().join("steam_api64.dll.org");
    let state_path = dir.path().join(".gse_manifest.json");
    assert!(backup.is_file(), "exactly one backup should exist");
    assert!(state_path.is_file(), "state sidecar should exist");

    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    let entries = state["backed_up_files"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "state must record exactly one backed-up file, not a torn/duplicated write");
}
