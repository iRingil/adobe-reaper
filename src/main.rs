//! `adobe-reaper`: an invisible wrapper that finishes off hung Adobe processes.
//!
//! Usage: `adobe-reaper <path to Adobe app> [app arguments...]`

// Release builds run without a console window. Debug builds (`cargo run`) keep it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Prints a line to stderr in debug builds; release builds print nothing.
///
/// Defined before the `mod` declarations so that all modules can use it.
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!($($arg)*);
        }
    };
}

mod apps;
mod handle;
mod process;
mod single_instance;
mod watcher;
mod window;

use std::env;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::Duration;

use single_instance::WatcherLock;

/// How long a new wrapper waits for a finishing watcher to exit before giving up.
const WATCHER_HANDOVER_WAIT: Duration = Duration::from_secs(5);

fn main() -> ExitCode {
    // `args_os` instead of `args`: a Windows path is not guaranteed to be valid UTF-8.
    let mut args = env::args_os().skip(1);
    let Some(app_path) = args.next().map(PathBuf::from) else {
        debug_log!("Usage: adobe-reaper <path to Adobe app> [app arguments...]");
        return ExitCode::FAILURE;
    };

    // A zombie of the same app would swallow the launch, so it goes first.
    if let Some(exe_name) = app_path.file_name().and_then(OsStr::to_str)
        && let Err(e) = watcher::kill_zombies(exe_name)
    {
        debug_log!("Failed to look for zombies of {exe_name}: {e}");
    }

    // The remaining arguments go to the app unchanged (e.g. a file to open).
    if let Err(e) = Command::new(&app_path).args(args).spawn() {
        debug_log!("Failed to start {}: {e}", app_path.display());
        return ExitCode::FAILURE;
    }

    // Become the watcher, unless one is already running: it will pick up
    // the new app on its next poll. The lock is held until `main` returns.
    let _lock = match WatcherLock::acquire(WATCHER_HANDOVER_WAIT) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            debug_log!("Another watcher is running; it will pick up the app.");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            debug_log!("Failed to check for a running watcher: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = watcher::run() {
        debug_log!("Watcher failed: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
