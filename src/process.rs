//! Snapshot of running processes, their parent/child relations, and termination.

use std::time::Duration;

use windows::Win32::Foundation::{ERROR_INVALID_PARAMETER, FILETIME, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

use crate::handle::OwnedHandle;

/// How long to wait for a terminated process to actually disappear.
const EXIT_WAIT: Duration = Duration::from_secs(5);

/// Result of a [`terminate`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Termination {
    /// The process was terminated by us.
    Terminated,
    /// The process had already exited (or its PID now belongs to another process).
    AlreadyGone,
}

/// Information about one running process.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: u32,
    /// Executable file name, e.g. `Photoshop.exe`.
    pub exe_name: String,
    /// Creation time as a raw `FILETIME` value; `None` if the process can't be opened.
    pub start_time: Option<u64>,
}

impl ProcessInfo {
    /// Checks whether `self` is a child of `parent`.
    ///
    /// A matching parent PID is not enough: Windows reuses PIDs, so the parent
    /// must also have started before the child. If either start time is unknown,
    /// the relation is not trusted.
    pub fn is_child_of(&self, parent: &ProcessInfo) -> bool {
        if self.parent_pid != parent.pid || self.pid == parent.pid {
            return false;
        }
        match (self.start_time, parent.start_time) {
            (Some(child), Some(parent)) => child >= parent,
            _ => false,
        }
    }
}

/// Takes a snapshot of all running processes.
///
/// # Errors
///
/// Returns an error if the snapshot can't be created.
pub fn snapshot() -> windows::core::Result<Vec<ProcessInfo>> {
    // SAFETY: plain WinAPI call; the returned handle is closed by `OwnedHandle`.
    let snap = OwnedHandle(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?);

    let mut entry = PROCESSENTRY32W {
        // The API requires the struct size to be filled in before the call.
        dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).expect("struct size fits in u32"),
        ..Default::default()
    };

    let mut processes = Vec::new();
    // SAFETY: `snap` is a valid snapshot handle and `entry` is properly initialized.
    let mut found = unsafe { Process32FirstW(snap.0, &mut entry) }.is_ok();
    while found {
        processes.push(ProcessInfo {
            pid: entry.th32ProcessID,
            parent_pid: entry.th32ParentProcessID,
            exe_name: utf16_until_nul(&entry.szExeFile),
            start_time: start_time(entry.th32ProcessID),
        });
        // SAFETY: same as above.
        found = unsafe { Process32NextW(snap.0, &mut entry) }.is_ok();
    }
    Ok(processes)
}

/// Returns all descendants of `root` (children, grandchildren, ...).
///
/// Processes for which `skip` returns `true` are left out together with their own descendants.
pub fn descendants<'a>(
    all: &'a [ProcessInfo],
    root: &'a ProcessInfo,
    skip: impl Fn(&ProcessInfo) -> bool,
) -> Vec<&'a ProcessInfo> {
    let mut result = Vec::new();
    let mut to_visit = vec![root];
    while let Some(parent) = to_visit.pop() {
        for child in all.iter().filter(|p| p.is_child_of(parent)) {
            if skip(child) {
                continue;
            }
            result.push(child);
            to_visit.push(child);
        }
    }
    result
}

/// Terminates the process and waits until it has exited.
///
/// The process is identified by PID *and* start time, so a process that reused
/// the PID is never touched.
///
/// # Errors
///
/// Returns an error if the process exists but can't be opened or terminated
/// (e.g. access denied).
pub fn terminate(process: &ProcessInfo) -> windows::core::Result<Termination> {
    let access = PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
    // SAFETY: plain WinAPI call; the returned handle is closed by `OwnedHandle`.
    let handle = match unsafe { OpenProcess(access, false, process.pid) } {
        Ok(handle) => OwnedHandle(handle),
        // There is no process with this PID anymore.
        Err(e) if e.code() == ERROR_INVALID_PARAMETER.to_hresult() => {
            return Ok(Termination::AlreadyGone);
        }
        Err(e) => return Err(e),
    };

    if start_time_of(&handle) != process.start_time || has_exited(&handle) {
        return Ok(Termination::AlreadyGone);
    }

    // SAFETY: `handle` is valid and has PROCESS_TERMINATE access.
    unsafe { TerminateProcess(handle.0, 1) }?;
    let millis = u32::try_from(EXIT_WAIT.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: `handle` is valid and has PROCESS_SYNCHRONIZE access.
    unsafe { WaitForSingleObject(handle.0, millis) };
    Ok(Termination::Terminated)
}

/// Checks whether the process behind `handle` has already exited.
fn has_exited(handle: &OwnedHandle) -> bool {
    // SAFETY: `handle` is valid; a zero timeout only checks the state.
    let state = unsafe { WaitForSingleObject(handle.0, 0) };
    state == WAIT_OBJECT_0
}

/// Reads the creation time of a process, or `None` if it can't be opened.
fn start_time(pid: u32) -> Option<u64> {
    // SAFETY: plain WinAPI call; the returned handle is closed by `OwnedHandle`.
    let handle =
        OwnedHandle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?);
    start_time_of(&handle)
}

/// Reads the creation time of the process behind `handle`.
fn start_time_of(handle: &OwnedHandle) -> Option<u64> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: `handle` is valid and all four pointers point to live locals.
    unsafe { GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;

    Some(u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime))
}

/// Converts a NUL-terminated UTF-16 buffer into a `String`.
fn utf16_until_nul(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}
