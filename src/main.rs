//! `adobe-reaper`: an invisible wrapper that finishes off hung Adobe processes.

mod process;
mod window;

use std::env;

fn main() {
    // Temporary demo: print the process tree of the given exe (default: RustRover).
    let target = env::args()
        .nth(1)
        .unwrap_or_else(|| "rustrover64.exe".to_string());

    let processes = match process::snapshot() {
        Ok(processes) => processes,
        Err(e) => {
            eprintln!("Process snapshot failed: {e}");
            return;
        }
    };
    let with_windows = match window::pids_with_visible_windows() {
        Ok(pids) => pids,
        Err(e) => {
            eprintln!("EnumWindows failed: {e}");
            return;
        }
    };

    for root in processes
        .iter()
        .filter(|p| p.exe_name.eq_ignore_ascii_case(&target))
    {
        for p in std::iter::once(root).chain(process::descendants(&processes, root)) {
            let mark = if with_windows.contains(&p.pid) {
                "[window]"
            } else {
                ""
            };
            println!("{:>6} <- {:<6} {} {mark}", p.pid, p.parent_pid, p.exe_name);
        }
    }
}
