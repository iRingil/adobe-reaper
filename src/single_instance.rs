//! Ensures that only one watcher runs at a time.

use std::time::Duration;

use windows::Win32::Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::core::{PCWSTR, w};

use crate::handle::OwnedHandle;

/// Name of the system-wide mutex. `Local\` scopes it to the current user session.
const MUTEX_NAME: PCWSTR = w!("Local\\adobe-reaper-watcher");

/// Proof that this process is the only watcher. Releases the mutex when dropped.
pub struct WatcherLock(OwnedHandle);

impl WatcherLock {
    /// Tries to become the watcher, waiting up to `timeout` for the current one to exit.
    ///
    /// Returns `Ok(None)` if another watcher is still running after the timeout.
    ///
    /// # Errors
    ///
    /// Returns an error if the mutex can't be created or waited on.
    pub fn acquire(timeout: Duration) -> windows::core::Result<Option<Self>> {
        // SAFETY: plain WinAPI call; the handle is closed by `OwnedHandle`.
        let handle = OwnedHandle(unsafe { CreateMutexW(None, false, MUTEX_NAME) }?);
        let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);

        // SAFETY: `handle` is a valid mutex handle.
        match unsafe { WaitForSingleObject(handle.0, millis) } {
            // WAIT_ABANDONED: the previous watcher died without releasing the mutex
            // (e.g. was killed). The mutex is ours anyway.
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Some(Self(handle))),
            WAIT_TIMEOUT => Ok(None),
            _ => Err(windows::core::Error::from_thread()),
        }
    }
}

impl Drop for WatcherLock {
    fn drop(&mut self) {
        // SAFETY: the mutex was acquired by this thread in `acquire`.
        // After this, the `OwnedHandle` field is dropped and closes the handle.
        let _ = unsafe { ReleaseMutex(self.0.0) };
    }
}
