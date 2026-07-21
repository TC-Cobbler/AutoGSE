use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use sha2::{Digest, Sha256};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::core::PCWSTR;

use crate::error::AutoGseError;

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn mutex_name_for(dir: &Path) -> String {
    let canon = dunce::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let normalized = canon.to_string_lossy().to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    let short: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("Local\\AutoGSE_{short}")
}

/// RAII guard around a named Win32 mutex that serializes inject/revert
/// operations against the same target directory, including across separate
/// AutoGSE process invocations. Released automatically on drop, and also by
/// the OS if the holding process crashes, so there is no stale-lock cleanup
/// to implement.
pub struct AutoGseLock {
    handle: HANDLE,
}

impl AutoGseLock {
    pub fn acquire(dir: &Path, timeout_ms: u32) -> Result<Self, AutoGseError> {
        let name = to_wide(&mutex_name_for(dir));

        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }
            .map_err(|e| AutoGseError::Registry(format!("CreateMutexW failed: {e}")))?;

        let wait = unsafe { WaitForSingleObject(handle, timeout_ms) };
        match wait {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self { handle }),
            _ => {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                Err(AutoGseError::AlreadyLocked(dir.to_path_buf()))
            }
        }
    }
}

impl Drop for AutoGseLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    /// Each thread independently opens its own handle to the same named
    /// mutex (mirroring how two separate AutoGSE process invocations would
    /// each create their own handle), so this also exercises the real
    /// cross-invocation contention path, not just in-process reentrancy.
    #[test]
    fn serializes_across_threads() {
        let dir = tempfile::tempdir().unwrap();
        let log: Arc<Mutex<Vec<(u32, &'static str)>>> = Arc::new(Mutex::new(Vec::new()));

        let handles: Vec<_> = (0..4)
            .map(|id| {
                let path = dir.path().to_path_buf();
                let log = Arc::clone(&log);
                thread::spawn(move || {
                    let _lock = AutoGseLock::acquire(&path, 5_000).unwrap();
                    log.lock().unwrap().push((id, "enter"));
                    thread::sleep(Duration::from_millis(20));
                    log.lock().unwrap().push((id, "exit"));
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let log = log.lock().unwrap();
        assert_eq!(log.len(), 8);
        for pair in log.chunks(2) {
            assert_eq!(pair[0].0, pair[1].0, "critical sections overlapped: {:?}", *log);
            assert_eq!((pair[0].1, pair[1].1), ("enter", "exit"));
        }
    }
}
