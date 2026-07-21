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
    fn rejects_unknown_machine_type() {
        assert_eq!(parse_bitness(&synthetic_pe(0x01c4)), None);
    }

    #[test]
    fn read_bitness_errors_on_missing_file() {
        let result = read_bitness(Path::new("Z:\\does\\not\\exist.dll"));
        assert!(result.is_err());
    }
}
