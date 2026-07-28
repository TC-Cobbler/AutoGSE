//! Phase 7 §7.3: real OS-level file drag-and-drop. Slint 1.17.1's own
//! `DragArea`/`DropArea` elements exist, but their `DataTransfer` payload
//! type explicitly supports only plain text and images (confirmed by reading
//! `i-slint-core`'s own source — no file/path payload exists at all, and
//! Slint's own upstream tracking issue for expanding this,
//! `slint-ui/slint#1967`, is still open). So dropping a file from Windows
//! Explorer onto the window can't go through Slint's drag-and-drop API —
//! this module bypasses it entirely with a native Win32 `IDropTarget`
//! registered directly on the window's real HWND (obtained via
//! `raw-window-handle`, which Slint's `Window::window_handle()` exposes).

use std::path::PathBuf;

use windows::Win32::Foundation::{HWND, POINTL};
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, OleInitialize, OleUninitialize, ReleaseStgMedium,
    RegisterDragDrop, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::core::{Ref, Result as WinResult, implement};

#[implement(IDropTarget)]
struct FileDropTarget {
    on_drop: Box<dyn Fn(Vec<PathBuf>)>,
}

impl IDropTarget_Impl for FileDropTarget_Impl {
    fn DragEnter(
        &self,
        _pdataobj: Ref<'_, IDataObject>,
        _grfkeystate: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> WinResult<()> {
        // Every drag is accepted here; `hdrop_paths` (Drop's own extraction)
        // is what actually decides whether anything usable came through —
        // simpler than inspecting the data object twice (once here, once on
        // Drop) for a single-purpose drop target with no other consumers.
        unsafe { *pdweffect = DROPEFFECT_COPY };
        Ok(())
    }

    fn DragOver(&self, _grfkeystate: MODIFIERKEYS_FLAGS, _pt: &POINTL, pdweffect: *mut DROPEFFECT) -> WinResult<()> {
        unsafe { *pdweffect = DROPEFFECT_COPY };
        Ok(())
    }

    fn DragLeave(&self) -> WinResult<()> {
        Ok(())
    }

    fn Drop(&self, pdataobj: Ref<'_, IDataObject>, _grfkeystate: MODIFIERKEYS_FLAGS, _pt: &POINTL, pdweffect: *mut DROPEFFECT) -> WinResult<()> {
        unsafe { *pdweffect = DROPEFFECT_NONE };

        let Some(data_object) = pdataobj.as_ref() else { return Ok(()) };

        let format =
            FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };

        let paths = unsafe {
            match data_object.GetData(&format) {
                Ok(mut medium) => {
                    let paths = extract_dropped_paths(&medium);
                    ReleaseStgMedium(&mut medium);
                    paths
                }
                Err(_) => Vec::new(),
            }
        };

        eprintln!("[AutoGSE] native drop received {} path(s): {paths:?}", paths.len());
        if !paths.is_empty() {
            unsafe { *pdweffect = DROPEFFECT_COPY };
            (self.on_drop)(paths);
        }
        Ok(())
    }
}

/// `medium` must actually carry `CF_HDROP` data (checked by the caller via
/// the `FORMATETC` it requested) before this is called.
unsafe fn extract_dropped_paths(medium: &STGMEDIUM) -> Vec<PathBuf> {
    let hdrop = HDROP(unsafe { medium.u.hGlobal }.0);
    let file_count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };

    let mut paths = Vec::with_capacity(file_count as usize);
    for index in 0..file_count {
        let needed_len = unsafe { DragQueryFileW(hdrop, index, None) };
        let mut buf = vec![0u16; needed_len as usize + 1];
        let written = unsafe { DragQueryFileW(hdrop, index, Some(&mut buf)) };
        buf.truncate(written as usize);
        paths.push(PathBuf::from(String::from_utf16_lossy(&buf)));
    }
    paths
}

/// Registers a native file-drop target on `hwnd`, calling `on_drop` (on the
/// UI thread — OLE drag-and-drop callbacks arrive synchronously on whichever
/// thread registered the target, which must be the same thread pumping that
/// window's message loop) with every dropped file/folder path. Returns a
/// guard that unregisters the target and uninitializes OLE when dropped —
/// callers should hold this for the window's whole lifetime.
pub struct DropTargetGuard {
    hwnd: HWND,
}

impl DropTargetGuard {
    pub fn register(hwnd: HWND, on_drop: impl Fn(Vec<PathBuf>) + 'static) -> WinResult<Self> {
        unsafe { OleInitialize(None)? };

        // Confirmed live: winit (Slint's backend here) already registers its
        // *own* drop target on this exact HWND — for its own
        // `WindowEvent::DroppedFile` support, which Slint's own API never
        // surfaces to application code (its `DataTransfer` type carries only
        // plain text/images, see this module's top doc comment) — so it's
        // effectively dead capability from this app's point of view.
        // `RegisterDragDrop` otherwise fails outright with
        // `DRAGDROP_E_ALREADYREGISTERED` since a window can only have one
        // registered target at a time. Revoking it first is safe: nothing in
        // Slint's own code depends on that registration doing anything.
        let _ = unsafe { RevokeDragDrop(hwnd) };

        let target: IDropTarget = FileDropTarget { on_drop: Box::new(on_drop) }.into();
        if let Err(e) = unsafe { RegisterDragDrop(hwnd, &target) } {
            unsafe { OleUninitialize() };
            return Err(e);
        }

        Ok(Self { hwnd })
    }
}

impl Drop for DropTargetGuard {
    fn drop(&mut self) {
        let _ = unsafe { RevokeDragDrop(self.hwnd) };
        unsafe { OleUninitialize() };
    }
}
