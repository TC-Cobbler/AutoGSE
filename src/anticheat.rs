//! Phase 10 §10.1: advisory, pre-injection anti-cheat/anti-tamper scanner.
//!
//! Scoped to what's actually detectable with a stable, publicly documented
//! signature — Easy Anti-Cheat and BattlEye install as a well-known,
//! consistent set of extra files (checked here via `scan_directory`);
//! VMProtect renames its sections to `.vmp0`/`.vmp1`/`.vmp2` (checked via
//! `scan_binary`, built on `pe::read_section_names`). Denuvo and Arxan are
//! deliberately **not** covered: both are SDK-integrated into the game's own
//! compiled code at build time with no stable, publicly documented
//! file/section/import signature, so a hardcoded pattern for either would be
//! one build/version away from silently missing the next game — worse than
//! admitting the gap.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::pe;

/// Mirrors `discovery.rs`'s own BFS depth budget beneath a target's D_root —
/// not shared code (that walk is private and DLL-match-specific), just the
/// same bound applied to a differently-shaped walk.
const SCAN_MAX_DEPTH: usize = 6;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntiCheatSystem {
    EasyAntiCheat,
    BattlEye,
    VmProtect,
}

impl std::fmt::Display for AntiCheatSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AntiCheatSystem::EasyAntiCheat => "Easy Anti-Cheat",
            AntiCheatSystem::BattlEye => "BattlEye",
            AntiCheatSystem::VmProtect => "VMProtect",
        })
    }
}

#[derive(Debug, Clone)]
pub struct AntiCheatFinding {
    pub system: AntiCheatSystem,
    pub detail: String,
}

const EAC_MARKER_DIRS: &[&str] = &["EasyAntiCheat"];
const EAC_MARKER_FILES: &[&str] = &["EasyAntiCheat_x64.dll", "EasyAntiCheat_x86.dll", "EasyAntiCheatDrvResources.dll"];
const BATTLEYE_MARKER_DIRS: &[&str] = &["BattlEye"];
const BATTLEYE_MARKER_FILES: &[&str] = &["BEClient_x64.dll", "BEClient.dll", "BEService.exe", "BEDaisy.sys"];

/// BFS beneath `tod` (same bounded walk shape as `discovery.rs::scan`,
/// independently implemented since that one is private and DLL-match-typed)
/// looking for EAC/BattlEye's own well-documented marker directories/files.
/// Stops recording further hits for a system once one is found — this is a
/// yes/no advisory check, not an inventory.
pub fn scan_directory(tod: &Path) -> Vec<AntiCheatFinding> {
    let mut findings = Vec::new();
    let mut found_eac = false;
    let mut found_battleye = false;

    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((tod.to_path_buf(), 0));

    while let Some((dir, depth)) = queue.pop_front() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let name = entry.file_name().to_string_lossy().into_owned();

            if metadata.is_dir() {
                if !found_eac && EAC_MARKER_DIRS.iter().any(|m| m.eq_ignore_ascii_case(&name)) {
                    found_eac = true;
                    findings.push(AntiCheatFinding {
                        system: AntiCheatSystem::EasyAntiCheat,
                        detail: format!("{} directory present", entry.path().display()),
                    });
                }
                if !found_battleye && BATTLEYE_MARKER_DIRS.iter().any(|m| m.eq_ignore_ascii_case(&name)) {
                    found_battleye = true;
                    findings.push(AntiCheatFinding {
                        system: AntiCheatSystem::BattlEye,
                        detail: format!("{} directory present", entry.path().display()),
                    });
                }

                let is_reparse_point = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
                };
                if depth < SCAN_MAX_DEPTH && !is_reparse_point {
                    queue.push_back((entry.path(), depth + 1));
                }
            } else if metadata.is_file() {
                if !found_eac && EAC_MARKER_FILES.iter().any(|m| m.eq_ignore_ascii_case(&name)) {
                    found_eac = true;
                    findings.push(AntiCheatFinding {
                        system: AntiCheatSystem::EasyAntiCheat,
                        detail: format!("{} present", entry.path().display()),
                    });
                }
                if !found_battleye && BATTLEYE_MARKER_FILES.iter().any(|m| m.eq_ignore_ascii_case(&name)) {
                    found_battleye = true;
                    findings.push(AntiCheatFinding {
                        system: AntiCheatSystem::BattlEye,
                        detail: format!("{} present", entry.path().display()),
                    });
                }
            }

            if found_eac && found_battleye {
                return findings;
            }
        }
    }

    findings
}

/// Checks `exe`'s section table for VMProtect's `.vmp*` naming signature.
/// Best-effort: a PE parse failure (not a valid PE, or a section table this
/// parser couldn't fully walk) is silently treated as "no finding" rather
/// than surfaced as an error — this is an advisory heuristic layered on top
/// of an inject flow that already validated `exe` is a real PE elsewhere.
pub fn scan_binary(exe: &Path) -> Vec<AntiCheatFinding> {
    let Ok(sections) = pe::read_section_names(exe) else {
        return Vec::new();
    };
    if sections.iter().any(|s| s.to_lowercase().starts_with(".vmp")) {
        vec![AntiCheatFinding {
            system: AntiCheatSystem::VmProtect,
            detail: format!("VMProtect section naming (.vmp*) found in {}", exe.display()),
        }]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_directory_is_empty_on_a_vanilla_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("game.exe"), b"fake").unwrap();
        assert!(scan_directory(dir.path()).is_empty());
    }

    #[test]
    fn scan_directory_detects_eac_marker_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("EasyAntiCheat")).unwrap();
        let findings = scan_directory(dir.path());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].system, AntiCheatSystem::EasyAntiCheat);
    }

    #[test]
    fn scan_directory_detects_battleye_marker_file_nested() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("Binaries").join("Win64");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("BEClient_x64.dll"), b"fake").unwrap();

        let findings = scan_directory(dir.path());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].system, AntiCheatSystem::BattlEye);
    }

    #[test]
    fn scan_directory_detects_both_systems_independently() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("EasyAntiCheat")).unwrap();
        std::fs::write(dir.path().join("BEService.exe"), b"fake").unwrap();

        let findings = scan_directory(dir.path());
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().any(|f| f.system == AntiCheatSystem::EasyAntiCheat));
        assert!(findings.iter().any(|f| f.system == AntiCheatSystem::BattlEye));
    }

    #[test]
    fn scan_binary_returns_empty_for_non_pe_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_a_pe.exe");
        std::fs::write(&path, b"not a real pe").unwrap();
        assert!(scan_binary(&path).is_empty());
    }
}
