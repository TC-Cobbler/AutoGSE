use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

use crate::error::AutoGseError;

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Quote a single argv entry for consumption by CommandLineToArgvW, which is
/// what the relaunched process' own arg parser will use.
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(['"', ' ', '\t']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// Relaunches the current executable elevated ("runas"), forwarding argv (minus
/// argv[0]), waits for it to exit, and returns its exit code. Never returns Ok
/// without the child having actually finished.
pub fn relaunch_elevated(args: &[String]) -> Result<u8, AutoGseError> {
    let exe = std::env::current_exe().map_err(AutoGseError::Io)?;
    let exe_wide = to_wide(&exe.to_string_lossy());

    let param_str = args.iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ");
    let params_wide = to_wide(&param_str);
    let verb_wide = to_wide("runas");

    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        hwnd: HWND::default(),
        lpVerb: PCWSTR(verb_wide.as_ptr()),
        lpFile: PCWSTR(exe_wide.as_ptr()),
        lpParameters: PCWSTR(params_wide.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    unsafe {
        ShellExecuteExW(&mut info)
            .map_err(|e| AutoGseError::Elevation(format!("ShellExecuteExW failed: {e}")))?;

        if info.hProcess.is_invalid() {
            // User declined the UAC prompt, or the shell didn't hand back a
            // waitable handle; either way we cannot claim success.
            return Err(AutoGseError::Elevation(
                "elevation was declined or no process handle was returned".to_string(),
            ));
        }

        WaitForSingleObject(info.hProcess, INFINITE);

        let mut exit_code: u32 = 1;
        let result = GetExitCodeProcess(info.hProcess, &mut exit_code);
        let _ = CloseHandle(info.hProcess);
        result.map_err(|e| AutoGseError::Elevation(format!("GetExitCodeProcess failed: {e}")))?;

        Ok(exit_code as u8)
    }
}

pub fn is_permission_denied(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::PermissionDenied
}
