//! `adobe-reaper`: an invisible wrapper that finishes off hung Adobe processes.
//!
//! Usage: `adobe-reaper <path to Adobe app> [app arguments...]`

mod apps;
mod handle;
mod process;
mod single_instance;
mod watcher;
mod window;

use std::env;
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
        eprintln!("Usage: adobe-reaper <path to Adobe app> [app arguments...]");
        return ExitCode::FAILURE;
    };

    // The remaining arguments go to the app unchanged (e.g. a file to open).
    if let Err(e) = Command::new(&app_path).args(args).spawn() {
        eprintln!("Failed to start {}: {e}", app_path.display());
        return ExitCode::FAILURE;
    }

    // Become the watcher, unless one is already running: it will pick up
    // the new app on its next poll. The lock is held until `main` returns.
    let _lock = match WatcherLock::acquire(WATCHER_HANDOVER_WAIT) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            println!("Another watcher is running; it will pick up the app.");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("Failed to check for a running watcher: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = watcher::run() {
        eprintln!("Watcher failed: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
