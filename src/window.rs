//! Top-level windows: detecting visible ones and asking them to close.

use std::collections::HashSet;

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};
use windows::core::BOOL;

/// Returns the PIDs of all processes that own at least one visible
/// top-level window with a non-zero size.
///
/// # Errors
///
/// Returns an error if `EnumWindows` fails.
pub fn pids_with_visible_windows() -> windows::core::Result<HashSet<u32>> {
    Ok(top_level_windows()?
        .into_iter()
        .filter(|&hwnd| is_visible_with_size(hwnd))
        .filter_map(owner_pid)
        .collect())
}

/// Asks every top-level window (visible or not) of the given processes to close.
///
/// The message is posted, not sent, so a hung app can't block the caller.
///
/// # Errors
///
/// Returns an error if `EnumWindows` fails.
pub fn post_close(pids: &HashSet<u32>) -> windows::core::Result<()> {
    for hwnd in top_level_windows()? {
        if owner_pid(hwnd).is_some_and(|pid| pids.contains(&pid)) {
            // SAFETY: posting a message has no memory-safety requirements.
            // A failure means the window is already gone, so it's ignored.
            let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
    }
    Ok(())
}

/// Returns all top-level windows.
fn top_level_windows() -> windows::core::Result<Vec<HWND>> {
    let mut handles = Vec::new();

    // Smuggle a pointer to `handles` through LPARAM so the callback can fill the list.
    let lparam = LPARAM(&raw mut handles as isize);

    // SAFETY: `handles` lives until the end of this function, and EnumWindows
    // calls the callback synchronously, before returning.
    unsafe { EnumWindows(Some(collect_window), lparam) }?;

    Ok(handles)
}

/// `EnumWindows` callback: adds every window to the list.
///
/// Always returns `true` to continue the enumeration.
///
/// # Safety
///
/// `lparam` must hold a valid, exclusive `*mut Vec<HWND>`.
unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: guaranteed by the caller (see `top_level_windows`).
    let handles = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    handles.push(hwnd);
    true.into()
}

/// Returns the PID of the process that owns the window.
fn owner_pid(hwnd: HWND) -> Option<u32> {
    let mut pid = 0;
    // SAFETY: `pid` is a valid place to write to; an invalid HWND leaves it at 0.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then_some(pid)
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
