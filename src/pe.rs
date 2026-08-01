use std::fmt;
use std::path::Path;

use crate::error::AutoGseError;

const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86,
    X64,
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Arch::X86 => "x86",
            Arch::X64 => "x64",
        })
    }
}

impl Arch {
    /// Inverse of `Display`, for reading `GseManifest.arch` back out (Phase
    /// 11 §11.3's `reinject` needs to re-resolve the vendored DLL source for
    /// an already-injected target without re-running PE bitness detection).
    pub fn parse(s: &str) -> Option<Arch> {
        match s {
            "x86" => Some(Arch::X86),
            "x64" => Some(Arch::X64),
            _ => None,
        }
    }
}

/// Reads `IMAGE_DOS_HEADER.e_lfanew` -> `IMAGE_NT_HEADERS.FileHeader.Machine`
/// to determine whether `path` is a 32-bit or 64-bit PE image (PRD §5.2.3).
pub fn read_bitness(path: &Path) -> Result<Arch, AutoGseError> {
    let bytes = std::fs::read(path)?;
    parse_bitness(&bytes).ok_or_else(|| AutoGseError::InvalidPeHeader(path.to_path_buf()))
}

fn parse_bitness(bytes: &[u8]) -> Option<Arch> {
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return None;
    }

    let e_lfanew = u32::from_le_bytes(bytes[0x3C..0x40].try_into().ok()?) as usize;

    let sig_end = e_lfanew.checked_add(4)?;
    let machine_end = sig_end.checked_add(2)?;
    if bytes.len() < machine_end {
        return None;
    }
    if &bytes[e_lfanew..sig_end] != b"PE\0\0" {
        return None;
    }

    let machine = u16::from_le_bytes(bytes[sig_end..machine_end].try_into().ok()?);
    match machine {
        IMAGE_FILE_MACHINE_I386 => Some(Arch::X86),
        IMAGE_FILE_MACHINE_AMD64 => Some(Arch::X64),
        _ => None,
    }
}

/// Reads every section name from a PE's section table (Phase 10 §10.1's
/// VMProtect check: renamed sections `.vmp0`/`.vmp1`/`.vmp2` are that
/// packer's one well-documented static signature). Purely additive alongside
/// `read_bitness`/`parse_bitness` above — same DOS/NT header offsets, walked
/// one step further into `IMAGE_FILE_HEADER`'s `NumberOfSections`/
/// `SizeOfOptionalHeader` fields to locate the section table
/// (`IMAGE_FILE_HEADER` is a fixed 20 bytes; each `IMAGE_SECTION_HEADER` is a
/// fixed 40 bytes with an 8-byte, NUL-padded `Name` as its first field).
pub fn read_section_names(path: &Path) -> Result<Vec<String>, AutoGseError> {
    let bytes = std::fs::read(path)?;
    parse_section_names(&bytes).ok_or_else(|| AutoGseError::InvalidPeHeader(path.to_path_buf()))
}

fn parse_section_names(bytes: &[u8]) -> Option<Vec<String>> {
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return None;
    }

    let e_lfanew = u32::from_le_bytes(bytes[0x3C..0x40].try_into().ok()?) as usize;
    let sig_end = e_lfanew.checked_add(4)?;
    let file_header_end = sig_end.checked_add(20)?;
    if bytes.len() < file_header_end {
        return None;
    }
    if &bytes[e_lfanew..sig_end] != b"PE\0\0" {
        return None;
    }

    let num_sections = u16::from_le_bytes(bytes[sig_end + 2..sig_end + 4].try_into().ok()?) as usize;
    let size_of_optional_header = u16::from_le_bytes(bytes[sig_end + 16..sig_end + 18].try_into().ok()?) as usize;

    let section_table_start = file_header_end.checked_add(size_of_optional_header)?;
    let mut names = Vec::with_capacity(num_sections);
    for i in 0..num_sections {
        let header_start = section_table_start.checked_add(i.checked_mul(40)?)?;
        let name_end = header_start.checked_add(8)?;
        if bytes.len() < name_end {
            // Truncated/malformed section table — return what was
            // successfully read rather than failing the whole scan on one
            // bad entry (this is an advisory heuristic, not a hard parser).
            break;
        }
        let raw = &bytes[header_start..name_end];
        let nul_pos = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        names.push(String::from_utf8_lossy(&raw[..nul_pos]).into_owned());
    }
    Some(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_pe(machine: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 0x86];
        buf[0..2].copy_from_slice(b"MZ");
        buf[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        buf[0x80..0x84].copy_from_slice(b"PE\0\0");
        buf[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        buf
    }

    /// Builds a synthetic PE with a real (zeroed) `IMAGE_OPTIONAL_HEADER` of
    /// `optional_header_size` bytes followed by `section_names.len()` real
    /// `IMAGE_SECTION_HEADER` entries, for `read_section_names` tests.
    fn synthetic_pe_with_sections(optional_header_size: u16, section_names: &[&str]) -> Vec<u8> {
        let e_lfanew: usize = 0x80;
        let sig_end = e_lfanew + 4;
        let file_header_end = sig_end + 20;
        let section_table_start = file_header_end + optional_header_size as usize;
        let total_len = section_table_start + section_names.len() * 40;

        let mut buf = vec![0u8; total_len];
        buf[0..2].copy_from_slice(b"MZ");
        buf[0x3C..0x40].copy_from_slice(&(e_lfanew as u32).to_le_bytes());
        buf[e_lfanew..sig_end].copy_from_slice(b"PE\0\0");
        buf[sig_end..sig_end + 2].copy_from_slice(&IMAGE_FILE_MACHINE_AMD64.to_le_bytes());
        buf[sig_end + 2..sig_end + 4].copy_from_slice(&(section_names.len() as u16).to_le_bytes());
        buf[sig_end + 16..sig_end + 18].copy_from_slice(&optional_header_size.to_le_bytes());

        for (i, name) in section_names.iter().enumerate() {
            let header_start = section_table_start + i * 40;
            let name_bytes = name.as_bytes();
            buf[header_start..header_start + name_bytes.len()].copy_from_slice(name_bytes);
        }
        buf
    }

    #[test]
    fn read_section_names_reads_real_section_table() {
        let buf = synthetic_pe_with_sections(224, &[".text", ".rdata", ".vmp0"]);
        let names = parse_section_names(&buf).unwrap();
        assert_eq!(names, vec![".text".to_string(), ".rdata".to_string(), ".vmp0".to_string()]);
    }

    #[test]
    fn read_section_names_handles_zero_sections() {
        let buf = synthetic_pe_with_sections(224, &[]);
        assert_eq!(parse_section_names(&buf), Some(vec![]));
    }

    #[test]
    fn read_section_names_none_on_bad_dos_magic() {
        let mut buf = synthetic_pe_with_sections(224, &[".text"]);
        buf[0..2].copy_from_slice(b"XX");
        assert_eq!(parse_section_names(&buf), None);
    }

    #[test]
    fn read_section_names_stops_gracefully_on_truncated_table() {
        let mut buf = synthetic_pe_with_sections(224, &[".text", ".rdata"]);
        // Each section header is 40 bytes with only the first 8 holding the
        // name; cut well into that 8-byte field of the 2nd header, not just
        // its trailing zero-padding, so this really exercises the
        // bounds-check path rather than accidentally leaving the name intact.
        buf.truncate(buf.len() - 36);
        let names = parse_section_names(&buf).unwrap();
        assert_eq!(names, vec![".text".to_string()]);
    }

    #[test]
    fn detects_x64() {
        assert_eq!(parse_bitness(&synthetic_pe(IMAGE_FILE_MACHINE_AMD64)), Some(Arch::X64));
    }

    #[test]
    fn detects_x86() {
        assert_eq!(parse_bitness(&synthetic_pe(IMAGE_FILE_MACHINE_I386)), Some(Arch::X86));
    }

    #[test]
    fn rejects_truncated_buffer() {
        let mut buf = synthetic_pe(IMAGE_FILE_MACHINE_AMD64);
        buf.truncate(0x50);
        assert_eq!(parse_bitness(&buf), None);
    }

    #[test]
    fn rejects_bad_dos_magic() {
        let mut buf = synthetic_pe(IMAGE_FILE_MACHINE_AMD64);
        buf[0..2].copy_from_slice(b"XX");
        assert_eq!(parse_bitness(&buf), None);
    }

    #[test]
    fn rejects_bad_nt_signature() {
        let mut buf = synthetic_pe(IMAGE_FILE_MACHINE_AMD64);
        buf[0x80..0x84].copy_from_slice(b"XXXX");
        assert_eq!(parse_bitness(&buf), None);
    }

    #[test]
    fn arch_parse_round_trips_through_display() {
        assert_eq!(Arch::parse(&Arch::X86.to_string()), Some(Arch::X86));
        assert_eq!(Arch::parse(&Arch::X64.to_string()), Some(Arch::X64));
        assert_eq!(Arch::parse("arm64"), None);
    }

    #[test]
    fn rejects_unknown_machine_type() {
        assert_eq!(parse_bitness(&synthetic_pe(0x01c4)), None);
    }

    #[test]
    fn read_bitness_errors_on_missing_file() {
        let result = read_bitness(Path::new("Z:\\does\\not\\exist.dll"));
        assert!(result.is_err());
    }
}
