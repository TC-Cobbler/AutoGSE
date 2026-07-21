use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::core::PCWSTR;

use crate::error::AutoGseError;
use crate::notify::AUMID;

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Both entries are registered unconditionally visible under both
/// `exefile\shell` and `Directory\shell`; there is no registry-only way to
/// hide/show a verb based on whether a sibling `.gse_manifest.json`/state
/// file exists in the clicked folder (that requires an IExplorerCommand COM
/// shell extension, deferred to a later milestone). Instead `inject`/`revert`
/// self-guard at runtime and no-op harmlessly if invoked against the wrong
/// state.
///
/// `exefile`, not `exe`: Explorer resolves a file's context menu via its
/// extension's ProgID (`.exe`'s default value is the built-in `exefile`
/// ProgID, confirmed via the registry — `HKCR\exe\shell\...`, without the
/// ProgID suffix, is not a key Explorer ever reads for file context menus;
/// registering there silently does nothing). `Directory` has no such
/// indirection — it *is* the correct, directly-read key for folders.
const ROOTS: [&str; 2] = ["exefile", "Directory"];

fn verb_command_line(exe_path: &str, verb: &str) -> String {
    format!("\"{exe_path}\" {verb} --path \"%1\"")
}

fn set_string_value(key: HKEY, name: Option<&str>, value: &str) -> Result<(), AutoGseError> {
    let name_wide = name.map(to_wide);
    let name_ptr = name_wide
        .as_ref()
        .map(|w| PCWSTR(w.as_ptr()))
        .unwrap_or(PCWSTR::null());

    let mut value_wide = to_wide(value);
    let bytes = unsafe {
        std::slice::from_raw_parts(
            value_wide.as_mut_ptr() as *const u8,
            value_wide.len() * std::mem::size_of::<u16>(),
        )
    };

    unsafe {
        RegSetValueExW(key, name_ptr, Some(0), REG_SZ, Some(bytes))
            .ok()
            .map_err(|e| AutoGseError::Registry(format!("RegSetValueExW failed: {e}")))
    }
}

fn create_key(subkey: &str) -> Result<HKEY, AutoGseError> {
    let subkey_wide = to_wide(subkey);
    let mut hkey = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_wide.as_ptr()),
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
        .ok()
        .map_err(|e| AutoGseError::Registry(format!("RegCreateKeyExW({subkey}) failed: {e}")))?;
    }
    Ok(hkey)
}

fn install_verb(root: &str, verb_key: &str, display_label: &str, command_line: &str) -> Result<(), AutoGseError> {
    let verb_path = format!("Software\\Classes\\{root}\\shell\\{verb_key}");
    let key = create_key(&verb_path)?;
    let result = set_string_value(key, Some("MUIVerb"), display_label);
    unsafe {
        let _ = RegCloseKey(key);
    }
    result?;

    let command_path = format!("{verb_path}\\command");
    let command_key = create_key(&command_path)?;
    let result = set_string_value(command_key, None, command_line);
    unsafe {
        let _ = RegCloseKey(command_key);
    }
    result
}

fn aumid_key_path() -> String {
    format!("Software\\Classes\\AppUserModelId\\{AUMID}")
}

/// Registers a display name for our AppUserModelId under
/// `HKCU\Software\Classes\AppUserModelId\<AUMID>`. Confirmed necessary (not
/// just cosmetic) empirically: an unpackaged Win32 app's
/// `ToastNotifier::Show()` call reports success even when this entry is
/// missing, but the notification platform silently drops the toast rather
/// than displaying it under a generic/fallback identity.
fn register_aumid() -> Result<(), AutoGseError> {
    let key = create_key(&aumid_key_path())?;
    let result = set_string_value(key, Some("DisplayName"), "AutoGSE");
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

fn unregister_aumid() -> Result<(), AutoGseError> {
    let path = aumid_key_path();
    let path_wide = to_wide(&path);
    unsafe {
        let status = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path_wide.as_ptr()));
        if status.0 != 0 && status.0 != 2 {
            return Err(AutoGseError::Registry(format!("RegDeleteTreeW({path}) failed with status {}", status.0)));
        }
    }
    Ok(())
}

pub fn install_context_menu() -> Result<(), AutoGseError> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy().into_owned();

    let inject_cmd = verb_command_line(&exe_str, "inject");
    let revert_cmd = verb_command_line(&exe_str, "revert");

    for root in ROOTS {
        install_verb(root, "AutoGSE_Inject", "AutoGSE: Inject Achievement Emulator", &inject_cmd)?;
        install_verb(root, "AutoGSE_Revert", "AutoGSE: Revert to Vanilla", &revert_cmd)?;
    }

    register_aumid()?;

    Ok(())
}

pub fn uninstall_context_menu() -> Result<(), AutoGseError> {
    for root in ROOTS {
        for verb_key in ["AutoGSE_Inject", "AutoGSE_Revert"] {
            let path = format!("Software\\Classes\\{root}\\shell\\{verb_key}");
            let path_wide = to_wide(&path);
            unsafe {
                // ERROR_FILE_NOT_FOUND is fine here (nothing to remove); any
                // other failure is surfaced.
                let status = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path_wide.as_ptr()));
                if status.0 != 0 && status.0 != 2 {
                    return Err(AutoGseError::Registry(format!(
                        "RegDeleteTreeW({path}) failed with status {}",
                        status.0
                    )));
                }
            }
        }
    }

    unregister_aumid()?;

    Ok(())
}
