use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use regex::Regex;
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::core::PCWSTR;

use crate::error::AutoGseError;

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

#[derive(Debug, Default, Clone)]
pub struct VersionStrings {
    pub file_description: Option<String>,
    pub product_name: Option<String>,
    pub comments: Option<String>,
    pub original_filename: Option<String>,
}

impl VersionStrings {
    fn all_present(&self) -> impl Iterator<Item = &str> {
        [&self.file_description, &self.product_name, &self.comments, &self.original_filename]
            .into_iter()
            .filter_map(|s| s.as_deref())
    }
}

/// Loads the raw `VS_VERSIONINFO` resource block for `path`, if present.
/// Most indie/repack builds have *no* version resource at all — that's the
/// expected common case (`GetFileVersionInfoSizeW` returning 0), mapped to
/// `Ok(None)`, not an error.
fn load_raw_block(path: &Path) -> Result<Option<Vec<u8>>, AutoGseError> {
    let wide = to_wide(&path.to_string_lossy());

    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None) };
    if size == 0 {
        return Ok(None);
    }

    let mut buffer = vec![0u8; size as usize];
    let result = unsafe {
        GetFileVersionInfoW(PCWSTR(wide.as_ptr()), None, size, buffer.as_mut_ptr() as *mut c_void)
    };
    match result {
        Ok(()) => Ok(Some(buffer)),
        Err(_) => Ok(None),
    }
}

/// Queries the `\VarFileInfo\Translation` block for the (language, codepage)
/// pair needed to build the correct `\StringFileInfo\<lang><codepage>\...`
/// sub-path. Falls back to US English/Unicode (`0409`/`04B0`) if the block
/// is missing, rather than giving up outright.
fn query_translation(block: &[u8]) -> (u16, u16) {
    const FALLBACK: (u16, u16) = (0x0409, 0x04B0);
    let subblock = to_wide("\\VarFileInfo\\Translation");
    let mut ptr: *mut c_void = std::ptr::null_mut();
    let mut len: u32 = 0;

    let ok = unsafe {
        VerQueryValueW(block.as_ptr() as *const c_void, PCWSTR(subblock.as_ptr()), &mut ptr, &mut len)
    };
    if !ok.as_bool() || ptr.is_null() || len < 4 {
        return FALLBACK;
    }

    // Translation block is an array of (u16 lang, u16 codepage) pairs; take the first.
    let pair = unsafe { std::slice::from_raw_parts(ptr as *const u16, 2) };
    (pair[0], pair[1])
}

fn query_string(block: &[u8], lang: u16, codepage: u16, field: &str) -> Option<String> {
    let subblock = to_wide(&format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\{field}"));
    let mut ptr: *mut c_void = std::ptr::null_mut();
    let mut len: u32 = 0;

    let ok = unsafe {
        VerQueryValueW(block.as_ptr() as *const c_void, PCWSTR(subblock.as_ptr()), &mut ptr, &mut len)
    };
    if !ok.as_bool() || ptr.is_null() || len == 0 {
        return None;
    }

    // `len` is documented as a character count for string sub-blocks, but is
    // sometimes off-by-one/inclusive of a null terminator depending on how
    // the resource was authored; read up to `len` u16 units and additionally
    // stop at the first embedded NUL, whichever comes first.
    let units = unsafe { std::slice::from_raw_parts(ptr as *const u16, len as usize) };
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    let s = String::from_utf16_lossy(&units[..end]);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Extracts `FileDescription`/`ProductName`/`Comments`/`OriginalFilename`
/// from `path`'s PE version resource (PRD §5.3.2). All fields are `None`
/// (not an error) when the file has no version resource at all.
pub fn extract_strings(path: &Path) -> Result<VersionStrings, AutoGseError> {
    let Some(block) = load_raw_block(path)? else {
        return Ok(VersionStrings::default());
    };

    let (lang, codepage) = query_translation(&block);

    Ok(VersionStrings {
        file_description: query_string(&block, lang, codepage, "FileDescription"),
        product_name: query_string(&block, lang, codepage, "ProductName"),
        comments: query_string(&block, lang, codepage, "Comments"),
        original_filename: query_string(&block, lang, codepage, "OriginalFilename"),
    })
}

fn appid_pattern() -> &'static Regex {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)SteamAppID\D{0,3}(\d+)").expect("valid regex"))
}

/// Scans a game exe's version-resource strings for an embedded
/// `SteamAppID: <digits>` marker (PRD §5.3.2's worked example).
pub fn find_appid_in_strings(path: &Path) -> Result<Option<u64>, AutoGseError> {
    let strings = extract_strings(path)?;
    for s in strings.all_present() {
        if let Some(caps) = appid_pattern().captures(s) {
            if let Ok(id) = caps[1].parse::<u64>() {
                return Ok(Some(id));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prefer our own compiled test binary (build.rs embeds real
    /// FileDescription/ProductName via winres) as the fixture; fall back to
    /// an always-present Windows system DLL if that resource somehow isn't
    /// linked into the test harness binary.
    fn fixture_path() -> std::path::PathBuf {
        let own_exe = std::env::current_exe().expect("current_exe");
        if load_raw_block(&own_exe).ok().flatten().is_some() {
            own_exe
        } else {
            std::path::PathBuf::from(r"C:\Windows\System32\kernel32.dll")
        }
    }

    #[test]
    fn extracts_non_empty_strings_from_a_real_pe_file() {
        let strings = extract_strings(&fixture_path()).unwrap();
        assert!(
            strings.file_description.is_some() || strings.product_name.is_some(),
            "expected at least one populated version string from {:?}",
            fixture_path()
        );
    }

    #[test]
    fn missing_version_resource_yields_none_fields_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("no_version_info.dll");
        std::fs::write(&fake, b"not a real pe file").unwrap();

        let strings = extract_strings(&fake).unwrap();

        assert!(strings.file_description.is_none());
        assert!(strings.product_name.is_none());
    }

    #[test]
    fn find_appid_in_strings_parses_embedded_marker() {
        let mut vs = VersionStrings::default();
        vs.comments = Some("Assembly Version: 1.0.0.0 | SteamAppID: 1091500".to_string());
        let hay: Vec<&str> = vs.all_present().collect();
        assert!(hay[0].contains("SteamAppID"));

        let caps = appid_pattern().captures(hay[0]).unwrap();
        assert_eq!(&caps[1], "1091500");
    }

    #[test]
    fn find_appid_in_strings_none_when_no_version_resource() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("plain.dll");
        std::fs::write(&fake, b"not a real pe file").unwrap();

        let result = find_appid_in_strings(&fake).unwrap();

        assert_eq!(result, None);
    }
}
