//! `adobe-reaper`: an invisible wrapper that finishes off hung Adobe processes.

mod apps;
mod process;
mod window;

fn main() {
    // Temporary demo: show the state of all Adobe processes.
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

    println!("Apps:");
    for app in processes.iter().filter(|p| apps::is_main_app(&p.exe_name)) {
        let state = if with_windows.contains(&app.pid) {
            "running"
        } else {
            "ZOMBIE"
        };
        println!("{:>6}  {:<25} {state}", app.pid, app.exe_name);

        for child in process::descendants(&processes, app) {
            let note = if apps::is_shared_service(&child.exe_name) {
                "(shared, skip)"
            } else {
                ""
            };
            println!("{:>6}    - {} {note}", child.pid, child.exe_name);
        }
    }

    println!("Shared services:");
    for service in processes
        .iter()
        .filter(|p| apps::is_shared_service(&p.exe_name))
    {
        println!("{:>6}  {}", service.pid, service.exe_name);
    }
}
