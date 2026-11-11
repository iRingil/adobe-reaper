//! Detection of processes that own visible windows.

use std::collections::HashSet;

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::BOOL;

/// Returns the PIDs of all processes that own at least one visible
/// top-level window with a non-zero size.
///
/// # Errors
///
/// Returns an error if `EnumWindows` fails.
pub fn pids_with_visible_windows() -> windows::core::Result<HashSet<u32>> {
    let mut pids = HashSet::new();

    // Smuggle a pointer to `pids` through LPARAM so the callback can fill the set.
    let lparam = LPARAM(&raw mut pids as isize);

    // SAFETY: `pids` lives until the end of this function, and EnumWindows
    // calls the callback synchronously, before returning.
    unsafe { EnumWindows(Some(collect_pid), lparam) }?;

    Ok(pids)
}

/// `EnumWindows` callback: adds the owner PID of every visible window to the set.
///
/// Always returns `true` to continue the enumeration.
///
/// # Safety
///
/// `lparam` must hold a valid, exclusive `*mut HashSet<u32>`.
unsafe extern "system" fn collect_pid(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if is_visible_with_size(hwnd) {
        let mut pid = 0;
        // SAFETY: `pid` is a valid place to write to.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };

        if pid != 0 {
            // SAFETY: guaranteed by the caller (see `pids_with_visible_windows`).
            let pids = unsafe { &mut *(lparam.0 as *mut HashSet<u32>) };
            pids.insert(pid);
        }
    }
    true.into()
}

/// Checks that the window is visible and has a non-zero size.
fn is_visible_with_size(hwnd: HWND) -> bool {
    // SAFETY: these functions only read window state; an invalid HWND
    // makes them report failure instead of causing undefined behavior.
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    if !visible {
        return false;
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return false;
    }
    rect.right > rect.left && rect.bottom > rect.top
}
