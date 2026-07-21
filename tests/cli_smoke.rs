use assert_cmd::Command;
use predicates::prelude::*;

fn bin() -> Command {
    Command::cargo_bin("autogse").unwrap()
}

/// Minimal synthetic PE bytes with a valid x64 DOS/NT header (same shape as
/// `pe::tests::synthetic_pe`), since Phase 2's pipeline now runs bitness
/// detection during inject and would reject arbitrary placeholder bytes.
fn write_fake_dll(dir: &std::path::Path) -> std::path::PathBuf {
    let dll = dir.join("steam_api64.dll");
    let mut buf = vec![0u8; 0x86];
    buf[0..2].copy_from_slice(b"MZ");
    buf[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    buf[0x80..0x84].copy_from_slice(b"PE\0\0");
    buf[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes()); // IMAGE_FILE_MACHINE_AMD64
    std::fs::write(&dll, &buf).unwrap();
    dll
}

/// Full real pipeline: real Goldberg DLL deploy, real `generate_emu_config.exe`
/// invocation (network + external process), real manifest. Not run by
/// default (network + external tools + ~5s per inject) — see the project's
/// established convention for live tests (e.g. `steam_api::tests::live_search_store`).
/// Manual run: `cargo test --test cli_smoke inject_then_revert_round_trips -- --ignored --nocapture`
#[test]
#[ignore]
fn inject_then_revert_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let dll = write_fake_dll(dir.path());

    bin()
        .args(["inject", "--path"])
        .arg(dir.path())
        .args(["--appid", "1091500"])
        .assert()
        .success();

    assert!(dir.path().join("steam_api64.dll.org").is_file(), "backup should exist after inject");
    assert!(dir.path().join(".gse_manifest.json").is_file(), "manifest should exist after inject");
    assert!(dir.path().join("steam_appid.txt").is_file(), "steam_appid.txt should be written at TOD root");
    assert_eq!(std::fs::read_to_string(dir.path().join("steam_appid.txt")).unwrap(), "1091500");
    assert!(dir.path().join("steam_settings/configs.main.ini").is_file(), "generated config should be merged in");

    let real_dll = std::fs::read(&dll).unwrap();
    let goldberg_dll = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("alex47exe-gse_fork/gen_emu_cfg-Windows-Release/generate_emu_config/_DEFAULT/0/steam_api64.dll"),
    )
    .unwrap();
    assert_eq!(real_dll, goldberg_dll, "the real Goldberg DLL should be deployed, not a placeholder");

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(".gse_manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["app_id"], 1091500);
    assert_eq!(manifest["arch"], "x64");
    assert_eq!(manifest["app_id_source"], "override");
    assert!(manifest["injected_files"].as_array().unwrap().contains(&serde_json::json!("steam_appid.txt")));

    // Idempotent re-inject: must succeed and must not touch the existing backup.
    bin().args(["inject", "--path"]).arg(dir.path()).args(["--appid", "1091500"]).assert().success();
    assert!(dir.path().join("steam_api64.dll.org").is_file());

    bin()
        .args(["revert", "--path"])
        .arg(dir.path())
        .assert()
        .success();

    assert!(!dir.path().join("steam_api64.dll.org").exists(), "backup should be gone after revert");
    assert!(!dir.path().join(".gse_manifest.json").exists(), "manifest should be gone after revert");
    assert!(!dir.path().join("steam_settings").exists(), "steam_settings/ should be fully removed after revert");
    assert!(!dir.path().join("steam_appid.txt").exists(), "steam_appid.txt should be removed after revert");
    assert_eq!(std::fs::read(&dll).unwrap(), original_bytes(), "revert must restore the original DLL bytes exactly");

    fn original_bytes() -> Vec<u8> {
        // Matches write_fake_dll's synthetic x64 PE header exactly.
        let mut buf = vec![0u8; 0x86];
        buf[0..2].copy_from_slice(b"MZ");
        buf[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        buf[0x80..0x84].copy_from_slice(b"PE\0\0");
        buf[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        buf
    }
}

#[test]
fn revert_with_nothing_to_revert_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    write_fake_dll(dir.path());

    bin().args(["revert", "--path"]).arg(dir.path()).assert().success();
}

#[test]
fn inject_fails_when_no_dll_present() {
    let dir = tempfile::tempdir().unwrap();

    bin()
        .args(["inject", "--path"])
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("no steam_api.dll or steam_api64.dll found"));
}

#[test]
fn inject_fails_on_nonexistent_target() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does_not_exist");

    bin()
        .args(["inject", "--path"])
        .arg(&missing)
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn install_then_uninstall_menu_succeeds() {
    bin().arg("install-menu").assert().success();
    bin().arg("uninstall-menu").assert().success();
}
