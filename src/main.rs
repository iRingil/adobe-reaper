//! `adobe-reaper`: an invisible wrapper that finishes off hung Adobe processes.

mod window;

fn main() {
    match window::pids_with_visible_windows() {
        Ok(pids) => println!("{} processes with visible windows: {pids:?}", pids.len()),
        Err(e) => eprintln!("EnumWindows failed: {e}"),
    }
}
