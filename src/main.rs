//! `adobe-reaper`: an invisible wrapper that finishes off hung Adobe processes.
//!
//! Usage: `adobe-reaper <path to Adobe app> [app arguments...]`

mod apps;
mod process;
mod watcher;
mod window;

use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

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

    if let Err(e) = watcher::run() {
        eprintln!("Watcher failed: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
