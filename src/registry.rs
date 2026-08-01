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

/// A single dynamic verb is registered under both `exefile\shell` and
/// `Directory\shell` (Phase 11 §11.1), backed by the `IExplorerCommand` COM
/// shell extension in the separate `shell-ext` crate (`autogse_shell.dll`).
/// Its label/enabled-state is computed at Explorer menu-render time from
/// `engine::classify_target` — "Inject..." on an uninjected folder,
/// "Revert..." on an injected one — replacing the two always-visible static
/// verbs this module used to install directly (see git history/older
/// comments here for that approach, kept only as a one-time migration
/// cleanup in `install_context_menu` below for anyone upgrading from a
/// pre-Phase-11 install).
///
/// `exefile`, not `exe`: Explorer resolves a file's context menu via its
/// extension's ProgID (`.exe`'s default value is the built-in `exefile`
/// ProgID, confirmed via the registry — `HKCR\exe\shell\...`, without the
/// ProgID suffix, is not a key Explorer ever reads for file context menus;
/// registering there silently does nothing). `Directory` has no such
/// indirection — it *is* the correct, directly-read key for folders.
const ROOTS: [&str; 2] = ["exefile", "Directory"];

/// Fixed CLSID for the `IExplorerCommand` COM object in `shell-ext`'s
/// `autogse_shell.dll` (`shell-ext/src/lib.rs`'s
/// `CLSID_AUTOGSE_EXPLORER_COMMAND`). The two must always agree — this is a
/// plain string here (for writing into the registry) and a `GUID` literal
/// there (for `DllGetClassObject`'s comparison); there is no single shared
/// source of truth across the two crates beyond this comment, the same way
/// a C header's GUID and a `.rgs`/`.reg` file's text form of it are just
/// kept in sync by convention in ordinary COM code.
const AUTOGSE_EXPLORER_COMMAND_CLSID: &str = "{8C9B1A2E-3F4D-4A5B-9C6D-7E8F9A0B1C2D}";

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

/// `autogse_shell.dll` always ships as a sibling of `autogse.exe` — true in
/// both a dev build (both land in the same `target/{profile}` directory,
/// since this is one Cargo workspace) and a real install (the installer's
/// `.iss` script ships them into the same directory) — so no separate
/// dev-vs-release resolver like `goldberg::tools_root()` is needed here.
fn shell_ext_dll_path() -> Result<String, AutoGseError> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| AutoGseError::Registry("could not resolve autogse.exe's own containing directory".to_string()))?;
    Ok(dir.join("autogse_shell.dll").to_string_lossy().into_owned())
}

fn install_clsid(dll_path: &str) -> Result<(), AutoGseError> {
    let key = create_key(&format!("Software\\Classes\\CLSID\\{AUTOGSE_EXPLORER_COMMAND_CLSID}\\InprocServer32"))?;
    let result = set_string_value(key, None, dll_path).and_then(|()| set_string_value(key, Some("ThreadingModel"), "Apartment"));
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

fn uninstall_clsid() -> Result<(), AutoGseError> {
    let path = format!("Software\\Classes\\CLSID\\{AUTOGSE_EXPLORER_COMMAND_CLSID}");
    let path_wide = to_wide(&path);
    unsafe {
        let status = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path_wide.as_ptr()));
        if status.0 != 0 && status.0 != 2 {
            return Err(AutoGseError::Registry(format!("RegDeleteTreeW({path}) failed with status {}", status.0)));
        }
    }
    Ok(())
}

/// `MUIVerb`/`command` aren't written at all here — unlike the old static
/// verbs, this verb's entire behavior (label, enabled state, and the actual
/// inject/revert invocation) is delegated to the COM object named by
/// `ExplorerCommandHandler`, per the documented `IExplorerCommand` shell
/// contract.
fn install_dynamic_verb(root: &str) -> Result<(), AutoGseError> {
    let key = create_key(&format!("Software\\Classes\\{root}\\shell\\AutoGSE"))?;
    let result = set_string_value(key, Some("ExplorerCommandHandler"), AUTOGSE_EXPLORER_COMMAND_CLSID);
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

fn uninstall_dynamic_verb(root: &str) -> Result<(), AutoGseError> {
    let path = format!("Software\\Classes\\{root}\\shell\\AutoGSE");
    let path_wide = to_wide(&path);
    unsafe {
        let status = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path_wide.as_ptr()));
        if status.0 != 0 && status.0 != 2 {
            return Err(AutoGseError::Registry(format!("RegDeleteTreeW({path}) failed with status {}", status.0)));
        }
    }
    Ok(())
}

/// One-time cleanup of the pre-Phase-11 static verbs, so upgrading an
/// existing install doesn't leave stale, dead `AutoGSE_Inject`/
/// `AutoGSE_Revert` entries sitting alongside the new dynamic one.
/// `RegDeleteTreeW`'s not-found case is already treated as success below, so
/// this is also safe to run unconditionally on a machine that never had the
/// old verbs installed.
fn remove_legacy_static_verbs() -> Result<(), AutoGseError> {
    for root in ROOTS {
        for verb_key in ["AutoGSE_Inject", "AutoGSE_Revert"] {
            let path = format!("Software\\Classes\\{root}\\shell\\{verb_key}");
            let path_wide = to_wide(&path);
            unsafe {
                let status = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path_wide.as_ptr()));
                if status.0 != 0 && status.0 != 2 {
                    return Err(AutoGseError::Registry(format!("RegDeleteTreeW({path}) failed with status {}", status.0)));
                }
            }
        }
    }
    Ok(())
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
    remove_legacy_static_verbs()?;

    let dll_path = shell_ext_dll_path()?;
    install_clsid(&dll_path)?;
    for root in ROOTS {
        install_dynamic_verb(root)?;
    }

    register_aumid()?;

    Ok(())
}

pub fn uninstall_context_menu() -> Result<(), AutoGseError> {
    for root in ROOTS {
        uninstall_dynamic_verb(root)?;
    }
    uninstall_clsid()?;
    // Defensive: also clean up a pre-Phase-11 install's static verbs if this
    // machine somehow still has them (e.g. an interrupted upgrade).
    remove_legacy_static_verbs()?;

    unregister_aumid()?;

    Ok(())
}
