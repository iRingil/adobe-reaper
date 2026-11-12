//! `adobe-reaper`: an invisible wrapper that finishes off hung Adobe processes.

mod apps;
mod process;
mod watcher;
mod window;

fn main() {
    if let Err(e) = watcher::run() {
        eprintln!("Watcher failed: {e}");
    }
}
