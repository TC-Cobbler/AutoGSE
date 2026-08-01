//! Phase 11 §11.1: a real Win32 `IExplorerCommand` COM shell extension,
//! replacing the two always-visible static HKCU verbs `registry.rs` used to
//! install (see that module's doc comments for the pre-Phase-11 approach)
//! with one dynamic verb whose label and enabled state are computed at
//! Explorer menu-render time from `autogse::engine::classify_target` —
//! "Inject..." on an uninjected folder, "Revert..." on an injected one,
//! matching the roadmap's exact wording.
//!
//! This is a separate crate (not folded into the main `autogse` lib) because
//! it must build as a `cdylib` — a real in-process COM server DLL loaded
//! directly into Explorer's own process — while the main crate stays an
//! `rlib` behind two ordinary `.exe`s. It depends on the `autogse` lib as a
//! path dependency purely to reuse `engine::classify_target`, not to
//! duplicate that logic here.
//!
//! Registration (writing the CLSID's `InprocServer32` and each verb's
//! `ExplorerCommandHandler`) lives in `registry.rs`'s `install_context_menu`,
//! not here — this crate only implements the COM object itself and the
//! classic `DllGetClassObject`/`DllCanUnloadNow` C ABI exports Explorer's COM
//! activation machinery calls to obtain one.

use std::ffi::c_void;
use std::path::PathBuf;
use std::process::Command;

use autogse::engine::{self, ScanStatus};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOTIMPL, HMODULE};
use windows::Win32::System::Com::{CoTaskMemAlloc, IBindCtx, IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleFileNameW, GetModuleHandleExW};
use windows::Win32::UI::Shell::{ECS_ENABLED, ECS_HIDDEN, IEnumExplorerCommand, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray, SIGDN_FILESYSPATH};
use windows::core::{BOOL, GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result as WinResult, implement};

/// Fixed CLSID for this COM object — see `registry.rs`'s
/// `AUTOGSE_EXPLORER_COMMAND_CLSID` doc comment for why this value is
/// duplicated (as a string there, a `GUID` literal here) rather than shared
/// from one source.
pub const CLSID_AUTOGSE_EXPLORER_COMMAND: GUID = GUID::from_u128(0x8c9b1a2e_3f4d_4a5b_9c6d_7e8f9a0b1c2d);

/// Takes `Option<&IShellItemArray>` (not `Ref<'_, IShellItemArray>`
/// directly) because `Ref` is not `Copy`/`Clone` — callers below need to
/// inspect the same selection more than once per COM method call (e.g.
/// `Invoke` resolves the path *and* re-classifies it), and `Ref::as_ref(&self)`
/// only ever borrows, so calling it twice on the same owned `Ref` parameter
/// at each call site is what actually allows that reuse.
fn resolve_path_from_array(items: Option<&IShellItemArray>) -> Option<PathBuf> {
    let item = unsafe { items?.GetItemAt(0) }.ok()?;
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.ok()?;
    let path = unsafe { name.to_string() }.ok();
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(name.0 as *const c_void)) };
    path.map(PathBuf::from)
}

/// `None` when the selected item isn't a state this verb applies to (not
/// found, I/O error, or genuinely not an AutoGSE-relevant target at all) —
/// every caller here treats that as "hide the verb", never as an error to
/// surface, since Explorer has no channel for this handler to report one.
fn classify(items: Option<&IShellItemArray>) -> Option<ScanStatus> {
    engine::classify_target(&resolve_path_from_array(items)?).ok()
}

fn is_revert_state(status: &ScanStatus) -> bool {
    matches!(status, ScanStatus::Injected | ScanStatus::UpdateReverted)
}

fn to_com_string(s: &str) -> PWSTR {
    // COM ownership convention for an `[out]` string parameter: the callee
    // allocates via `CoTaskMemAlloc`, the caller (Explorer) frees it via
    // `CoTaskMemFree`. A null return on allocation failure is the documented
    // way to signal that to a caller expecting a `PWSTR`.
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let buf = CoTaskMemAlloc(wide.len() * std::mem::size_of::<u16>()) as *mut u16;
        if !buf.is_null() {
            std::ptr::copy_nonoverlapping(wide.as_ptr(), buf, wide.len());
        }
        PWSTR(buf)
    }
}

/// Resolves `autogse.exe`'s path as a sibling of this DLL's own on-disk
/// location — always correct in both a dev build (both land in the same
/// `target/{profile}` directory, one Cargo workspace) and a real install
/// (the installer ships both into the same directory), so no separate
/// dev-vs-release resolver is needed, unlike `goldberg::tools_root()`.
fn autogse_exe_path() -> Option<PathBuf> {
    // Any address inside this DLL's own mapped image works here — this
    // function itself is as good as any. This is the standard Win32 idiom
    // for "get my own module's HMODULE from inside a DLL with no DllMain
    // hook to stash it from," per `GetModuleHandleExW`'s own documented
    // `GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS` behavior.
    let mut hmodule = HMODULE(std::ptr::null_mut());
    unsafe {
        let addr = autogse_exe_path as *const () as *const u16;
        GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, PCWSTR(addr), &mut hmodule).ok()?;
        let mut buf = [0u16; 1024];
        let len = GetModuleFileNameW(Some(hmodule), &mut buf);
        if len == 0 {
            return None;
        }
        PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])).parent().map(|p| p.join("autogse.exe"))
    }
}

#[implement(IExplorerCommand)]
struct AutoGseExplorerCommand;

impl IExplorerCommand_Impl for AutoGseExplorerCommand_Impl {
    fn GetTitle(&self, psiitemarray: Ref<'_, IShellItemArray>) -> WinResult<PWSTR> {
        let label = match classify(psiitemarray.as_ref()) {
            Some(status) if is_revert_state(&status) => "AutoGSE: Revert to Vanilla",
            _ => "AutoGSE: Inject Achievement Emulator",
        };
        Ok(to_com_string(label))
    }

    fn GetIcon(&self, _psiitemarray: Ref<'_, IShellItemArray>) -> WinResult<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetToolTip(&self, _psiitemarray: Ref<'_, IShellItemArray>) -> WinResult<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> WinResult<GUID> {
        Ok(CLSID_AUTOGSE_EXPLORER_COMMAND)
    }

    fn GetState(&self, psiitemarray: Ref<'_, IShellItemArray>, foktobeslow: BOOL) -> WinResult<u32> {
        if !foktobeslow.as_bool() {
            // No disk I/O on the fast pass, to avoid stalling Explorer's
            // menu-render thread — Explorer always re-queries with
            // fOkToBeSlow=TRUE before actually showing the menu, so this is
            // just an optimistic placeholder, never the final answer.
            return Ok(ECS_ENABLED.0 as u32);
        }
        match classify(psiitemarray.as_ref()) {
            Some(_) => Ok(ECS_ENABLED.0 as u32),
            None => Ok(ECS_HIDDEN.0 as u32),
        }
    }

    fn Invoke(&self, psiitemarray: Ref<'_, IShellItemArray>, _pbc: Ref<'_, IBindCtx>) -> WinResult<()> {
        let Some(path) = resolve_path_from_array(psiitemarray.as_ref()) else { return Ok(()) };
        let verb = match classify(psiitemarray.as_ref()) {
            Some(status) if is_revert_state(&status) => "revert",
            _ => "inject",
        };
        // Fire-and-forget, exactly like the old static verb's own command
        // line did — Explorer launches and owns `autogse.exe` from here,
        // this DLL doesn't wait on or supervise it.
        if let Some(exe) = autogse_exe_path() {
            let _ = Command::new(exe).arg(verb).arg("--path").arg(&path).spawn();
        }
        Ok(())
    }

    fn GetFlags(&self) -> WinResult<u32> {
        Ok(0)
    }

    fn EnumSubCommands(&self) -> WinResult<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}

#[implement(IClassFactory)]
struct AutoGseClassFactory;

impl IClassFactory_Impl for AutoGseClassFactory_Impl {
    fn CreateInstance(&self, punkouter: Ref<'_, IUnknown>, riid: *const GUID, ppvobject: *mut *mut c_void) -> WinResult<()> {
        if punkouter.as_ref().is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let unknown: IUnknown = AutoGseExplorerCommand.into();
        unsafe { unknown.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, _flock: BOOL) -> WinResult<()> {
        Ok(())
    }
}

/// Standard classic-COM in-process server export. Explorer's COM activation
/// calls this after `CoCreateInstance`-style resolution finds this DLL via
/// the CLSID's `InprocServer32` registry value `registry.rs` writes.
#[unsafe(no_mangle)]
unsafe extern "system" fn DllGetClassObject(rclsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    if unsafe { *rclsid } != CLSID_AUTOGSE_EXPLORER_COMMAND {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = AutoGseClassFactory.into();
    unsafe { factory.query(riid, ppv) }
}

/// Deliberately always reports "still in use" (`S_FALSE`) rather than doing
/// real reference-count bookkeeping across every live `AutoGseExplorerCommand`/
/// `AutoGseClassFactory` instance — this DLL is only ever asked to do brief,
/// synchronous, one-off work per Explorer menu render or click, so paying for
/// correct unload tracking buys nothing here; Explorer simply keeps holding
/// (and eventually evicts) the DLL from its own shared-DLL cache regardless.
#[unsafe(no_mangle)]
unsafe extern "system" fn DllCanUnloadNow() -> HRESULT {
    windows::Win32::Foundation::S_FALSE
}
