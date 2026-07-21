use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, ERROR_SHARING_VIOLATION};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_SHARE_MODE, OPEN_EXISTING,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::core::PCWSTR;

fn to_wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Authoritative check: attempts an exclusive (no-sharing) open of `path`.
/// Returns true if the file is currently locked by another process (i.e. a
/// concurrent writer/reader with denied sharing holds it open), which is the
/// actual condition that would break an in-place DLL swap.
pub fn is_file_locked(path: &Path) -> bool {
    let wide = to_wide(path.as_os_str());
    unsafe {
        let result = CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        );

        match result {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                false
            }
            Err(e) => e.code() == ERROR_SHARING_VIOLATION.to_hresult(),
        }
    }
}

/// Best-effort diagnostic only: scans running process names so an error
/// message can name a likely culprit. Not authoritative — a process can hold
/// a file open under a name that doesn't obviously match the game folder.
pub fn find_running_process_hint(folder_name_hint: &str) -> Option<String> {
    let needle = folder_name_hint.to_lowercase();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        let mut found = None;
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name_end = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);
                if name.to_lowercase().contains(&needle) {
                    found = Some(name);
                    break;
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
        found
    }
}
