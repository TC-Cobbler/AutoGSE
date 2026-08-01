//! Phase 13's RPCS3 (PS3 emulator) trophy parser: reads a real trophy set's
//! `TROPCONF.SFM` (XML trophy metadata) and `TROPUSR.DAT` (binary unlock
//! state), confirmed directly against RPCS3's own source
//! (`Emu/Cell/Modules/sceNpTrophy.cpp`, `Loader/TROPUSR.h`) via live fetches
//! while writing this module, not training-data memory — same "verify
//! against real docs, don't guess" discipline `retroachievements.rs` already
//! established for this project (its own `img`/`images` folder-name
//! correction in Phase 5 is exactly the class of mistake this avoids).
//!
//! **Correction to the roadmap's own wording**: it names the second file
//! `TROPTRN.DAT` — RPCS3's real file is `TROPUSR.DAT`; no evidence
//! `TROPTRN.DAT` exists anywhere in RPCS3's own code.
//!
//! **No real RPCS3 install is available in this environment to verify
//! against** — same honest "scaffold now, verify later" framing Phase 7
//! §7.7 used for RetroAchievements. Every function here is unit tested
//! against synthetic fixtures built to the confirmed real format, not
//! against a real RPCS3-generated file.
//!
//! `trophy_set_dir` (the folder containing one game's TROPCONF.SFM/
//! TROPUSR.DAT directly, e.g. `<dev_hdd0>/home/00000001/trophy/<trp_name>/`)
//! is always an explicit caller-supplied input, never auto-located — RPCS3's
//! `dev_hdd0` mount point is itself user-configurable (its own
//! `config.yml`/"Manage → Virtual File System" setting), and there's no real
//! install here to verify an auto-detection path against. Matches
//! `achievements::load_definitions(tod)`'s "caller supplies the specific
//! folder" convention.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::AutoGseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TrophyGrade {
    Bronze,
    Silver,
    Gold,
    Platinum,
    /// No `ttype`/`trophy_grade` data recognized for this trophy — not an
    /// error, just nothing to report (e.g. a `TROPUSR.DAT` entry with no
    /// matching static-data table entry).
    Unknown,
}

impl TrophyGrade {
    fn from_ttype(c: char) -> TrophyGrade {
        match c {
            'B' => TrophyGrade::Bronze,
            'S' => TrophyGrade::Silver,
            'G' => TrophyGrade::Gold,
            'P' => TrophyGrade::Platinum,
            _ => TrophyGrade::Unknown,
        }
    }

    fn from_u32(n: u32) -> TrophyGrade {
        match n {
            1 => TrophyGrade::Platinum,
            2 => TrophyGrade::Gold,
            3 => TrophyGrade::Silver,
            4 => TrophyGrade::Bronze,
            _ => TrophyGrade::Unknown,
        }
    }
}

/// One trophy's static definition, parsed from `TROPCONF.SFM`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Trophy {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub hidden: bool,
    pub grade: TrophyGrade,
    /// The platinum trophy this one contributes to, if any — `None` for
    /// `SCE_NP_TROPHY_INVALID_TROPHY_ID` (`-1`), confirmed via
    /// `sceNpTrophy.cpp`'s own `pid` attribute handling.
    pub platinum_link_id: Option<u32>,
    /// `TROP<3-digit-zero-padded-id>.PNG`, a sibling of `TROPCONF.SFM` —
    /// confirmed real filename pattern, attached only if it actually exists
    /// on disk (same best-effort convention `achievements::resolve_icon`
    /// already uses for Goldberg achievement icons).
    pub icon_path: Option<PathBuf>,
}

/// One trophy set's title/description plus every trophy it defines.
#[derive(Debug, Clone, PartialEq)]
pub struct TrophySet {
    pub title: String,
    pub description: String,
    pub trophies: Vec<Trophy>,
}

/// `SCE_NP_TROPHY_INVALID_TROPHY_ID` as it appears in `TROPCONF.SFM`'s `pid`
/// attribute (plain decimal text, `-1`) — distinct from the binary format's
/// `0xFFFFFFFF` sentinel for the same value (see `parse_tropusr`).
const INVALID_TROPHY_ID_XML: i64 = -1;

fn child_text(node: roxmltree::Node, tag: &str) -> String {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == tag)
        .and_then(|c| c.text())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Parses `TROPCONF.SFM` — confirmed real XML shape (via a live fetch of
/// `sceNpTrophy.cpp`): root `<trophyconf>`, a `<title-name>`/`<title-detail>`
/// pair for the set itself, one `<trophy id="" ttype="" pid="" hidden="">`
/// element per trophy with `<name>`/`<detail>` children for title/description.
pub fn parse_tropconf(path: &Path) -> Result<TrophySet, AutoGseError> {
    let xml = std::fs::read_to_string(path)?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| AutoGseError::Rpcs3(format!("{}: invalid TROPCONF.SFM XML: {e}", path.display())))?;
    let root = doc.root_element();
    if root.tag_name().name() != "trophyconf" {
        return Err(AutoGseError::Rpcs3(format!("{}: expected a <trophyconf> root element", path.display())));
    }

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let title = child_text(root, "title-name");
    let description = child_text(root, "title-detail");

    let mut trophies = Vec::new();
    for node in root.children().filter(|c| c.is_element() && c.tag_name().name() == "trophy") {
        let Some(id) = node.attribute("id").and_then(|v| v.parse::<u32>().ok()) else { continue };
        let grade = node.attribute("ttype").and_then(|v| v.chars().next()).map(TrophyGrade::from_ttype).unwrap_or(TrophyGrade::Unknown);
        let hidden = node.attribute("hidden") == Some("y");
        let platinum_link_id =
            node.attribute("pid").and_then(|v| v.parse::<i64>().ok()).filter(|&pid| pid != INVALID_TROPHY_ID_XML).map(|pid| pid as u32);
        let icon_path = {
            let candidate = dir.join(format!("TROP{id:03}.PNG"));
            candidate.is_file().then_some(candidate)
        };

        trophies.push(Trophy { id, name: child_text(node, "name"), description: child_text(node, "detail"), hidden, grade, platinum_link_id, icon_path });
    }

    Ok(TrophySet { title, description, trophies })
}

/// One trophy's live unlock state, merged from `TROPUSR.DAT`'s two relevant
/// tables (type 4: static grade/platinum-link data; type 6: unlock state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TrophyUnlockState {
    pub trophy_id: u32,
    pub grade: TrophyGrade,
    pub platinum_link_id: Option<u32>,
    pub unlocked: bool,
    // Both exposed, neither picked as "the" canonical unlock time: RPCS3's
    // own source comments that even its authors are unsure which of these
    // two is semantically primary (`// TODO: What timestamp does
    // sceNpTrophyGetTrophyInfo want, timestamp1 or timestamp2?`). Don't
    // guess; let the caller decide.
    pub timestamp1: Option<u64>,
    pub timestamp2: Option<u64>,
}

const TROPUSR_MAGIC: u32 = 0x818F_54AD;
const TROPUSR_HEADER_SIZE: usize = 48;
const TROPUSR_TABLE_HEADER_SIZE: usize = 32;
const TABLE_TYPE_STATIC: u32 = 4;
const TABLE_TYPE_UNLOCK: u32 = 6;
/// `TROPUSREntry4`'s `trophy_pid`'s invalid-id sentinel as it appears in the
/// binary format: `0xFFFFFFFF` (`-1` reinterpreted as an unsigned 32-bit
/// value) — distinct from `TROPCONF.SFM`'s plain-text `-1` (see
/// `INVALID_TROPHY_ID_XML`).
const INVALID_TROPHY_ID_BIN: u32 = u32::MAX;

fn read_be_u32(buf: &[u8], offset: usize) -> Option<u32> {
    buf.get(offset..offset + 4)?.try_into().ok().map(u32::from_be_bytes)
}

fn read_be_u64(buf: &[u8], offset: usize) -> Option<u64> {
    buf.get(offset..offset + 8)?.try_into().ok().map(u64::from_be_bytes)
}

/// Parses `TROPUSR.DAT` — a binary, big-endian, table-based format
/// (confirmed via two live fetches of `Loader/TROPUSR.h`): a header (magic
/// `0x818F54AD`, a table count), followed by that many table headers, each
/// naming a `type`, an `offset` (absolute byte offset into the file), an
/// `entries_count`, and — critically — the real per-entry `entries_size` to
/// use when walking that table.
///
/// **Deliberately does not hardcode a fixed per-entry struct size.** RPCS3's
/// own header comments for `TROPUSREntry4`/`TROPUSREntry6` don't
/// arithmetically match their own documented `entry_size` constants (`0x50`/
/// `0x60`) — confirmed via two independent live fetches of the identical
/// file, not a transcription slip, so it's a real ambiguity in the upstream
/// source (most likely struct padding/alignment the header's own comments
/// don't reflect). Hardcoding either number risks silently misaligning
/// every entry after the first on a real file. Instead, only the small set
/// of fields actually needed is read from the *front* of each entry, and the
/// cursor always advances by the table's own recorded `entries_size` —
/// correct regardless of which byte count is the real one.
pub fn parse_tropusr(path: &Path) -> Result<HashMap<u32, TrophyUnlockState>, AutoGseError> {
    let buf = std::fs::read(path)?;

    let magic = read_be_u32(&buf, 0).ok_or_else(|| AutoGseError::Rpcs3(format!("{}: file too short for a TROPUSR.DAT header", path.display())))?;
    if magic != TROPUSR_MAGIC {
        return Err(AutoGseError::Rpcs3(format!("{}: not a TROPUSR.DAT file (bad magic)", path.display())));
    }
    let tables_count =
        read_be_u32(&buf, 8).ok_or_else(|| AutoGseError::Rpcs3(format!("{}: truncated TROPUSR.DAT header", path.display())))? as usize;

    let mut static_data: HashMap<u32, (TrophyGrade, Option<u32>)> = HashMap::new();
    let mut unlock_data: HashMap<u32, (bool, Option<u64>, Option<u64>)> = HashMap::new();

    for i in 0..tables_count {
        let base = TROPUSR_HEADER_SIZE + i * TROPUSR_TABLE_HEADER_SIZE;
        let (Some(table_type), Some(entries_size), Some(entries_count), Some(offset)) =
            (read_be_u32(&buf, base), read_be_u32(&buf, base + 4), read_be_u32(&buf, base + 12), read_be_u64(&buf, base + 16))
        else {
            break; // truncated table-header list — stop, keep whatever was already parsed
        };
        if table_type != TABLE_TYPE_STATIC && table_type != TABLE_TYPE_UNLOCK {
            continue; // a table type this parser doesn't need (there are more than these two)
        }
        let entries_size = entries_size as usize;
        let offset = offset as usize;

        for j in 0..entries_count as usize {
            let entry_base = offset + j * entries_size;
            let Some(entry_type) = read_be_u32(&buf, entry_base) else { continue };
            let Some(trophy_id) = read_be_u32(&buf, entry_base + 16) else { continue };

            if table_type == TABLE_TYPE_STATIC && entry_type == TABLE_TYPE_STATIC {
                if let Some(grade_raw) = read_be_u32(&buf, entry_base + 20) {
                    let pid = read_be_u32(&buf, entry_base + 24).filter(|&p| p != INVALID_TROPHY_ID_BIN);
                    static_data.insert(trophy_id, (TrophyGrade::from_u32(grade_raw), pid));
                }
            } else if table_type == TABLE_TYPE_UNLOCK && entry_type == TABLE_TYPE_UNLOCK {
                if let Some(state_raw) = read_be_u32(&buf, entry_base + 20) {
                    let timestamp1 = read_be_u64(&buf, entry_base + 32);
                    let timestamp2 = read_be_u64(&buf, entry_base + 40);
                    unlock_data.insert(trophy_id, (state_raw != 0, timestamp1, timestamp2));
                }
            }
        }
    }

    let mut trophy_ids: std::collections::BTreeSet<u32> = static_data.keys().copied().collect();
    trophy_ids.extend(unlock_data.keys().copied());

    Ok(trophy_ids
        .into_iter()
        .map(|id| {
            let (grade, platinum_link_id) = static_data.get(&id).copied().unwrap_or((TrophyGrade::Unknown, None));
            let (unlocked, timestamp1, timestamp2) = unlock_data.get(&id).copied().unwrap_or((false, None, None));
            (id, TrophyUnlockState { trophy_id: id, grade, platinum_link_id, unlocked, timestamp1, timestamp2 })
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrophyWithState {
    pub trophy: Trophy,
    /// `None` when `TROPUSR.DAT` is missing entirely (trophy set never
    /// launched) or has no entry for this specific trophy yet.
    pub state: Option<TrophyUnlockState>,
}

/// Combines `parse_tropconf` + `parse_tropusr` for one trophy set directory.
/// `TROPUSR.DAT` missing entirely is not an error (every trophy just reports
/// `state: None`) — same "nothing earned yet" convention
/// `achievements::load_with_unlock_state` already uses for a Goldberg target
/// with no runtime unlock file yet.
pub fn load_trophy_set_with_state(trophy_set_dir: &Path) -> Result<Vec<TrophyWithState>, AutoGseError> {
    let set = parse_tropconf(&trophy_set_dir.join("TROPCONF.SFM"))?;
    let usr_path = trophy_set_dir.join("TROPUSR.DAT");
    let states = if usr_path.is_file() { parse_tropusr(&usr_path)? } else { HashMap::new() };

    Ok(set.trophies.into_iter().map(|trophy| TrophyWithState { state: states.get(&trophy.id).copied(), trophy }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic `TROPCONF.SFM` matching the confirmed real schema: one
    /// visible bronze trophy, one hidden trophy that links to a platinum.
    const SAMPLE_TROPCONF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<trophyconf>
    <title-name>Sample Game</title-name>
    <title-detail>A synthetic trophy set for testing.</title-detail>
    <trophy id="0" ttype="P" pid="-1" hidden="n">
        <name>Platinum Trophy</name>
        <detail>Unlock every other trophy.</detail>
    </trophy>
    <trophy id="1" ttype="B" pid="0" hidden="n">
        <name>First Steps</name>
        <detail>Complete the tutorial.</detail>
    </trophy>
    <trophy id="2" ttype="G" pid="0" hidden="y">
        <name>Secret Boss</name>
        <detail>Defeat the hidden final boss.</detail>
    </trophy>
</trophyconf>
"#;

    #[test]
    fn parse_tropconf_reads_real_shape_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TROPCONF.SFM");
        std::fs::write(&path, SAMPLE_TROPCONF).unwrap();

        let set = parse_tropconf(&path).unwrap();
        assert_eq!(set.title, "Sample Game");
        assert_eq!(set.description, "A synthetic trophy set for testing.");
        assert_eq!(set.trophies.len(), 3);

        let platinum = &set.trophies[0];
        assert_eq!(platinum.id, 0);
        assert_eq!(platinum.grade, TrophyGrade::Platinum);
        assert_eq!(platinum.platinum_link_id, None, "-1 must resolve to None, not Some(u32::MAX)");
        assert!(!platinum.hidden);

        let bronze = &set.trophies[1];
        assert_eq!(bronze.grade, TrophyGrade::Bronze);
        assert_eq!(bronze.platinum_link_id, Some(0));
        assert_eq!(bronze.name, "First Steps");
        assert_eq!(bronze.description, "Complete the tutorial.");

        let hidden = &set.trophies[2];
        assert!(hidden.hidden);
        assert_eq!(hidden.grade, TrophyGrade::Gold);
    }

    #[test]
    fn parse_tropconf_attaches_icon_path_only_when_it_exists_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TROPCONF.SFM");
        std::fs::write(&path, SAMPLE_TROPCONF).unwrap();
        std::fs::write(dir.path().join("TROP001.PNG"), b"fake png").unwrap();

        let set = parse_tropconf(&path).unwrap();
        assert_eq!(set.trophies[0].icon_path, None, "TROP000.PNG was never written");
        assert_eq!(set.trophies[1].icon_path, Some(dir.path().join("TROP001.PNG")));
    }

    #[test]
    fn parse_tropconf_rejects_a_non_trophyconf_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TROPCONF.SFM");
        std::fs::write(&path, "<not-trophyconf></not-trophyconf>").unwrap();

        assert!(matches!(parse_tropconf(&path), Err(AutoGseError::Rpcs3(_))));
    }

    #[test]
    fn parse_tropconf_rejects_invalid_xml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TROPCONF.SFM");
        std::fs::write(&path, "<trophyconf><unclosed>").unwrap();

        assert!(matches!(parse_tropconf(&path), Err(AutoGseError::Rpcs3(_))));
    }

    /// Hand-builds a synthetic `TROPUSR.DAT` using the confirmed header/
    /// table-header layout. `entry_stride` is deliberately configurable so
    /// tests can prove the parser follows each table's own recorded
    /// `entries_size` rather than assuming a fixed struct size — the real
    /// ambiguity this module's own doc comment calls out.
    fn build_tropusr(static_entries: &[(u32, u32, u32)], static_stride: usize, unlock_entries: &[(u32, u32, u64, u64)], unlock_stride: usize) -> Vec<u8> {
        let mut buf = Vec::new();

        // Header (48 bytes).
        buf.extend_from_slice(&TROPUSR_MAGIC.to_be_bytes());
        buf.extend_from_slice(&0u32.to_be_bytes()); // unk1
        buf.extend_from_slice(&2u32.to_be_bytes()); // tables_count
        buf.extend_from_slice(&0u32.to_be_bytes()); // unk2
        buf.extend_from_slice(&[0u8; 32]); // reserved

        let table_headers_end = TROPUSR_HEADER_SIZE + 2 * TROPUSR_TABLE_HEADER_SIZE;
        let static_table_offset = table_headers_end;
        let unlock_table_offset = static_table_offset + static_entries.len() * static_stride;

        // Table header 0: static (type 4).
        buf.extend_from_slice(&TABLE_TYPE_STATIC.to_be_bytes());
        buf.extend_from_slice(&(static_stride as u32).to_be_bytes());
        buf.extend_from_slice(&1u32.to_be_bytes()); // unk1
        buf.extend_from_slice(&(static_entries.len() as u32).to_be_bytes());
        buf.extend_from_slice(&(static_table_offset as u64).to_be_bytes());
        buf.extend_from_slice(&0u64.to_be_bytes()); // reserved

        // Table header 1: unlock (type 6).
        buf.extend_from_slice(&TABLE_TYPE_UNLOCK.to_be_bytes());
        buf.extend_from_slice(&(unlock_stride as u32).to_be_bytes());
        buf.extend_from_slice(&1u32.to_be_bytes());
        buf.extend_from_slice(&(unlock_entries.len() as u32).to_be_bytes());
        buf.extend_from_slice(&(unlock_table_offset as u64).to_be_bytes());
        buf.extend_from_slice(&0u64.to_be_bytes());

        for &(trophy_id, grade, pid) in static_entries {
            let mut entry = vec![0u8; static_stride];
            entry[0..4].copy_from_slice(&TABLE_TYPE_STATIC.to_be_bytes());
            entry[4..8].copy_from_slice(&(static_stride as u32).to_be_bytes());
            entry[16..20].copy_from_slice(&trophy_id.to_be_bytes());
            entry[20..24].copy_from_slice(&grade.to_be_bytes());
            entry[24..28].copy_from_slice(&pid.to_be_bytes());
            buf.extend_from_slice(&entry);
        }

        for &(trophy_id, state, ts1, ts2) in unlock_entries {
            let mut entry = vec![0u8; unlock_stride];
            entry[0..4].copy_from_slice(&TABLE_TYPE_UNLOCK.to_be_bytes());
            entry[4..8].copy_from_slice(&(unlock_stride as u32).to_be_bytes());
            entry[16..20].copy_from_slice(&trophy_id.to_be_bytes());
            entry[20..24].copy_from_slice(&state.to_be_bytes());
            entry[32..40].copy_from_slice(&ts1.to_be_bytes());
            entry[40..48].copy_from_slice(&ts2.to_be_bytes());
            buf.extend_from_slice(&entry);
        }

        buf
    }

    #[test]
    fn parse_tropusr_reads_grade_pid_and_unlock_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TROPUSR.DAT");
        // grade 4 = Bronze, pid 0; unlocked, real-shaped timestamps.
        let buf = build_tropusr(&[(1, 4, 0)], 0x50, &[(1, 1, 1784651841, 1784651841)], 0x60);
        std::fs::write(&path, buf).unwrap();

        let states = parse_tropusr(&path).unwrap();
        let entry = states.get(&1).unwrap();
        assert_eq!(entry.grade, TrophyGrade::Bronze);
        assert_eq!(entry.platinum_link_id, Some(0));
        assert!(entry.unlocked);
        assert_eq!(entry.timestamp1, Some(1784651841));
        assert_eq!(entry.timestamp2, Some(1784651841));
    }

    #[test]
    fn parse_tropusr_follows_a_nonstandard_entries_size_correctly() {
        // Proves the parser walks entries by the table header's own stride,
        // not a hardcoded struct size — two entries at a stride wider than
        // either of RPCS3's own documented (and mutually inconsistent)
        // 0x50/0x60 constants, which would misalign a hardcoded-size reader
        // onto garbage bytes for the second entry.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TROPUSR.DAT");
        let buf = build_tropusr(&[(0, 1, u32::MAX), (1, 4, u32::MAX)], 0x100, &[], 0x60);
        std::fs::write(&path, buf).unwrap();

        let states = parse_tropusr(&path).unwrap();
        assert_eq!(states.get(&0).unwrap().grade, TrophyGrade::Platinum);
        assert_eq!(states.get(&0).unwrap().platinum_link_id, None, "0xFFFFFFFF must resolve to None");
        assert_eq!(states.get(&1).unwrap().grade, TrophyGrade::Bronze);
    }

    #[test]
    fn parse_tropusr_rejects_bad_magic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TROPUSR.DAT");
        std::fs::write(&path, [0u8; 48]).unwrap();

        assert!(matches!(parse_tropusr(&path), Err(AutoGseError::Rpcs3(_))));
    }

    #[test]
    fn parse_tropusr_trophy_with_no_static_entry_defaults_to_unknown_grade() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TROPUSR.DAT");
        // Only an unlock-table entry, no matching static-table entry.
        let buf = build_tropusr(&[], 0x50, &[(5, 1, 111, 222)], 0x60);
        std::fs::write(&path, buf).unwrap();

        let states = parse_tropusr(&path).unwrap();
        let entry = states.get(&5).unwrap();
        assert_eq!(entry.grade, TrophyGrade::Unknown);
        assert_eq!(entry.platinum_link_id, None);
        assert!(entry.unlocked);
    }

    #[test]
    fn load_trophy_set_with_state_merges_definitions_and_unlock_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("TROPCONF.SFM"), SAMPLE_TROPCONF).unwrap();
        let buf = build_tropusr(&[(1, 4, 0)], 0x50, &[(1, 1, 1784651841, 1784651841)], 0x60);
        std::fs::write(dir.path().join("TROPUSR.DAT"), buf).unwrap();

        let result = load_trophy_set_with_state(dir.path()).unwrap();
        assert_eq!(result.len(), 3);

        let bronze = result.iter().find(|t| t.trophy.id == 1).unwrap();
        assert!(bronze.state.unwrap().unlocked);

        let platinum = result.iter().find(|t| t.trophy.id == 0).unwrap();
        assert!(platinum.state.is_none(), "trophy 0 has no TROPUSR.DAT entry in this fixture");
    }

    #[test]
    fn load_trophy_set_with_state_is_not_an_error_when_tropusr_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("TROPCONF.SFM"), SAMPLE_TROPCONF).unwrap();

        let result = load_trophy_set_with_state(dir.path()).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.iter().all(|t| t.state.is_none()));
    }
}
