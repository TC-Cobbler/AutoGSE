use std::path::PathBuf;

use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::{InitPropVariantFromStringAsVector, PropVariantClear};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::IShellLinkW;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::{Interface, GUID, HSTRING};

use crate::error::AutoGseError;
use crate::notify::AUMID;

/// `CLSID_ShellLink` — not exported as a named constant by windows-rs (no
/// coclass wrapper is generated for `ShellLink`), so this is the well-known,
/// decades-stable Win32 GUID from `shobjidl.h`.
const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_C000_000000000046);

fn shortcut_path() -> Result<PathBuf, AutoGseError> {
    let appdata = std::env::var_os("APPDATA")
        .ok_or_else(|| AutoGseError::Registry("APPDATA environment variable not set".to_string()))?;
    Ok(PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs\\AutoGSE.lnk"))
}

fn to_err(context: &str, e: windows::core::Error) -> AutoGseError {
    AutoGseError::Registry(format!("{context}: {e}"))
}

/// Creates a Start Menu shortcut to the current exe with `System.AppUserModel.ID`
/// set to [`AUMID`]. **This is not cosmetic** — confirmed empirically (toasts
/// silently failed to display without it, even though every WinRT toast API
/// call reported success) and matches Microsoft's own documented requirement:
/// an unpackaged Win32 app cannot raise a toast notification at all without a
/// Start Menu shortcut carrying its AppUserModelID
/// (learn.microsoft.com/windows/win32/shell/enable-desktop-toast-with-appusermodelid).
/// The `HKCU\Software\Classes\AppUserModelId\<AUMID>` registry entry
/// (`registry.rs`) supplies the display name/icon shown *once this exists*;
/// it does not substitute for it.
pub fn install() -> Result<(), AutoGseError> {
    let exe = std::env::current_exe()?;
    let link_path = shortcut_path()?;
    if let Some(parent) = link_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let shell_link: IShellLinkW =
            CoCreateInstance(&CLSID_SHELL_LINK, None, CLSCTX_INPROC_SERVER).map_err(|e| to_err("CoCreateInstance(ShellLink)", e))?;

        shell_link.SetPath(&HSTRING::from(exe.as_os_str())).map_err(|e| to_err("IShellLinkW::SetPath", e))?;
        shell_link
            .SetDescription(&HSTRING::from("AutoGSE: Automated Goldberg Achievement & Emulator Integrator"))
            .map_err(|e| to_err("IShellLinkW::SetDescription", e))?;

        let props: IPropertyStore = shell_link.cast().map_err(|e| to_err("IShellLinkW as IPropertyStore", e))?;
        let mut propvar =
            InitPropVariantFromStringAsVector(&HSTRING::from(AUMID)).map_err(|e| to_err("InitPropVariantFromStringAsVector", e))?;
        props.SetValue(&PKEY_AppUserModel_ID, &propvar).map_err(|e| to_err("IPropertyStore::SetValue", e))?;
        props.Commit().map_err(|e| to_err("IPropertyStore::Commit", e))?;
        let _ = PropVariantClear(&mut propvar);

        let persist_file: IPersistFile = shell_link.cast().map_err(|e| to_err("IShellLinkW as IPersistFile", e))?;
        persist_file
            .Save(&HSTRING::from(link_path.as_os_str()), true)
            .map_err(|e| to_err("IPersistFile::Save", e))?;
    }

    Ok(())
}

pub fn uninstall() -> Result<(), AutoGseError> {
    let link_path = shortcut_path()?;
    if link_path.is_file() {
        std::fs::remove_file(&link_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_path_is_under_start_menu_programs() {
        let path = shortcut_path().unwrap();
        assert!(path.to_string_lossy().contains("Start Menu\\Programs"));
        assert_eq!(path.file_name().unwrap(), "AutoGSE.lnk");
    }

    /// Manual QA only (real COM calls, real filesystem write to the Start
    /// Menu, real registry-adjacent side effect):
    /// `cargo test shortcut::tests::live_install_creates_real_shortcut -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_install_creates_real_shortcut() {
        install().unwrap();
        assert!(shortcut_path().unwrap().is_file());
        uninstall().unwrap();
        assert!(!shortcut_path().unwrap().is_file());
    }
}
