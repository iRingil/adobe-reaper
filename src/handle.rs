//! Owned Windows handles.

use windows::Win32::Foundation::{CloseHandle, HANDLE};

/// A Windows handle that is closed automatically when dropped.
pub struct OwnedHandle(pub HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this struct and closed exactly once.
        // A failure to close can't be handled meaningfully here, so it's ignored.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
