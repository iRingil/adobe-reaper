//! The watcher: tracks Adobe apps and decides when to finish them off.
//!
//! The decision logic ([`Watcher::update`]) is pure: it receives a snapshot of
//! processes and windows and returns events. All WinAPI calls live in [`run`].
//! This keeps the logic testable without real Adobe processes.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

use crate::apps;
use crate::process::{self, ProcessInfo};
use crate::window;

/// How often the watcher polls processes and windows.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// How long an app may stay without windows before it is finished off
/// (it may still be writing files).
const GRACE_PERIOD: Duration = Duration::from_secs(30);

/// Lifecycle of a watched app process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    /// Seen without windows so far: probably still loading (splash screen).
    /// The countdown starts only after the first window, so this state never expires.
    Starting,
    /// Has at least one visible window.
    Running,
    /// Had windows, but they disappeared.
    WindowsGone { since: Instant },
    /// Decided to finish it off.
    Reaped,
}

impl AppState {
    /// Computes the next state from the current one.
    fn next(self, has_window: bool, now: Instant) -> Self {
        match (self, has_window) {
            (Self::Reaped, _) => Self::Reaped,
            (_, true) => Self::Running,
            (Self::Running, false) => Self::WindowsGone { since: now },
            (Self::WindowsGone { since }, false) if now.duration_since(since) >= GRACE_PERIOD => {
                Self::Reaped
            }
            (state, false) => state,
        }
    }
}

impl fmt::Display for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::WindowsGone { .. } => "windows gone",
            Self::Reaped => "reaped",
        };
        f.write_str(label)
    }
}

/// Something the watcher decided during one poll.
#[derive(Debug, Clone)]
pub enum Event {
    /// An app changed its state; `from` is `None` for a newly seen app.
    StateChanged {
        pid: u32,
        exe_name: String,
        from: Option<AppState>,
        to: AppState,
    },
    /// An app must be finished off: the app itself first, then its descendants.
    ReapApp { tree: Vec<ProcessInfo> },
    /// No app is left: shared services must be finished off, then the watcher exits.
    CleanUp { services: Vec<ProcessInfo> },
}

/// Identifies a process across polls. PIDs are reused, so the start time is part of the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ProcessKey {
    pid: u32,
    start_time: Option<u64>,
}

impl ProcessKey {
    fn of(process: &ProcessInfo) -> Self {
        Self {
            pid: process.pid,
            start_time: process.start_time,
        }
    }
}

/// State of all watched apps between polls.
#[derive(Debug, Default)]
pub struct Watcher {
    apps: HashMap<ProcessKey, AppState>,
}

impl Watcher {
    /// Processes one poll and returns what has to be done.
    pub fn update(
        &mut self,
        processes: &[ProcessInfo],
        with_windows: &HashSet<u32>,
        now: Instant,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        let mut alive = HashSet::new();

        for app in processes.iter().filter(|p| apps::is_main_app(&p.exe_name)) {
            let key = ProcessKey::of(app);
            alive.insert(key);
            let has_window = with_windows.contains(&app.pid);

            let old = self.apps.get(&key).copied();
            let new = match old {
                Some(state) => state.next(has_window, now),
                None if has_window => AppState::Running,
                None => AppState::Starting,
            };
            self.apps.insert(key, new);

            if old != Some(new) {
                events.push(Event::StateChanged {
                    pid: app.pid,
                    exe_name: app.exe_name.clone(),
                    from: old,
                    to: new,
                });
                if new == AppState::Reaped {
                    events.push(Event::ReapApp {
                        tree: app_tree(processes, app),
                    });
                }
            }
        }

        // Forget apps that have exited.
        self.apps.retain(|key, _| alive.contains(key));

        if self.apps.values().all(|state| *state == AppState::Reaped) {
            events.push(Event::CleanUp {
                services: shared_services_tree(processes),
            });
        }
        events
    }
}

/// Runs the watcher until no Adobe app is left.
///
/// Dry-run mode: decisions are only printed, nothing is killed.
///
/// # Errors
///
/// Returns an error if processes or windows can't be enumerated.
pub fn run() -> windows::core::Result<()> {
    let mut watcher = Watcher::default();
    loop {
        let processes = process::snapshot()?;
        let with_windows = window::pids_with_visible_windows()?;

        for event in watcher.update(&processes, &with_windows, Instant::now()) {
            match event {
                Event::StateChanged {
                    pid,
                    exe_name,
                    from,
                    to,
                } => match from {
                    Some(from) => println!("{pid:>6}  {exe_name}: {from} -> {to}"),
                    None => println!("{pid:>6}  {exe_name}: {to}"),
                },
                Event::ReapApp { tree } => {
                    println!("[dry run] would reap:");
                    print_processes(&tree);
                }
                Event::CleanUp { services } => {
                    println!("[dry run] no apps left, would kill shared services:");
                    print_processes(&services);
                    return Ok(());
                }
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Processes that are never killed together with another process's tree:
/// other apps (e.g. Media Encoder started by Premiere) and shared services.
fn is_managed_separately(process: &ProcessInfo) -> bool {
    apps::is_main_app(&process.exe_name) || apps::is_shared_service(&process.exe_name)
}

/// The app and its descendants, except those managed separately.
fn app_tree(processes: &[ProcessInfo], app: &ProcessInfo) -> Vec<ProcessInfo> {
    std::iter::once(app)
        .chain(process::descendants(processes, app, is_managed_separately))
        .cloned()
        .collect()
}

/// All shared services and their descendants (e.g. `node.exe` under `CCXProcess.exe`).
fn shared_services_tree(processes: &[ProcessInfo]) -> Vec<ProcessInfo> {
    processes
        .iter()
        .filter(|p| apps::is_shared_service(&p.exe_name))
        .flat_map(|service| app_tree(processes, service))
        .collect()
}

fn print_processes(processes: &[ProcessInfo]) {
    for p in processes {
        println!("{:>6}    - {}", p.pid, p.exe_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// A fake process; the start time grows with the PID, so children need bigger PIDs.
    fn proc(pid: u32, parent_pid: u32, exe_name: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid,
            exe_name: exe_name.to_string(),
            start_time: Some(u64::from(pid)),
        }
    }

    fn windows(pids: &[u32]) -> HashSet<u32> {
        pids.iter().copied().collect()
    }

    fn reaped_pids(events: &[Event]) -> Vec<u32> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::ReapApp { tree } => Some(tree[0].pid),
                _ => None,
            })
            .collect()
    }

    fn has_cleanup(events: &[Event]) -> bool {
        events.iter().any(|e| matches!(e, Event::CleanUp { .. }))
    }

    #[test]
    fn app_is_reaped_after_grace_period() {
        let ps = vec![proc(100, 1, "Photoshop.exe")];
        let t0 = Instant::now();
        let mut w = Watcher::default();

        w.update(&ps, &windows(&[100]), t0);
        w.update(&ps, &windows(&[]), t0 + secs(10)); // windows disappear
        let early = w.update(&ps, &windows(&[]), t0 + secs(30));
        let late = w.update(&ps, &windows(&[]), t0 + secs(40));

        assert!(reaped_pids(&early).is_empty());
        assert_eq!(reaped_pids(&late), vec![100]);
    }

    #[test]
    fn returning_window_cancels_countdown() {
        let ps = vec![proc(100, 1, "Photoshop.exe")];
        let t0 = Instant::now();
        let mut w = Watcher::default();

        w.update(&ps, &windows(&[100]), t0);
        w.update(&ps, &windows(&[]), t0 + secs(10));
        w.update(&ps, &windows(&[100]), t0 + secs(20)); // a window is back
        let events = w.update(&ps, &windows(&[]), t0 + secs(45));

        assert!(reaped_pids(&events).is_empty());
    }

    #[test]
    fn app_that_never_had_windows_is_not_reaped() {
        let ps = vec![proc(100, 1, "Photoshop.exe")];
        let t0 = Instant::now();
        let mut w = Watcher::default();

        w.update(&ps, &windows(&[]), t0);
        let events = w.update(&ps, &windows(&[]), t0 + secs(3600));

        assert!(reaped_pids(&events).is_empty());
        assert!(!has_cleanup(&events)); // a loading app still needs the shared services
    }

    #[test]
    fn app_tree_skips_shared_services_and_other_apps() {
        let ps = vec![
            proc(100, 1, "Adobe Premiere Pro.exe"),
            proc(101, 100, "dynamiclinkmanager.exe"),
            proc(102, 100, "CoreSync.exe"),
            proc(103, 102, "node.exe"),
            proc(104, 100, "Adobe Media Encoder.exe"),
        ];
        let tree: Vec<u32> = app_tree(&ps, &ps[0]).iter().map(|p| p.pid).collect();
        assert_eq!(tree, vec![100, 101]);
    }

    #[test]
    fn cleanup_when_no_apps_left() {
        let ps = vec![proc(200, 1, "CCXProcess.exe"), proc(201, 200, "node.exe")];
        let mut w = Watcher::default();

        let events = w.update(&ps, &windows(&[]), Instant::now());

        let Some(Event::CleanUp { services }) = events.last() else {
            panic!("expected CleanUp, got {events:?}");
        };
        let pids: Vec<u32> = services.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![200, 201]);
    }

    #[test]
    fn no_cleanup_while_an_app_is_running() {
        let ps = vec![
            proc(100, 1, "Photoshop.exe"),
            proc(110, 1, "Illustrator.exe"),
        ];
        let t0 = Instant::now();
        let mut w = Watcher::default();

        w.update(&ps, &windows(&[100, 110]), t0);
        w.update(&ps, &windows(&[110]), t0 + secs(10));
        let events = w.update(&ps, &windows(&[110]), t0 + secs(40));

        assert_eq!(reaped_pids(&events), vec![100]);
        assert!(!has_cleanup(&events));
    }
}
