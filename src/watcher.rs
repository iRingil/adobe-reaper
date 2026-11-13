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
use crate::process::{self, ProcessInfo, Termination};
use crate::window;

/// How often the watcher polls processes and windows.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// How long an app may stay without windows before it is finished off
/// (it may still be writing files).
const GRACE_PERIOD: Duration = Duration::from_secs(20);

/// How long to wait after asking processes to close before terminating them.
const SOFT_CLOSE_WAIT: Duration = Duration::from_secs(5);

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
    /// Repeated on every poll while the app is still alive.
    ReapApp { tree: Vec<ProcessInfo> },
    /// No app seems to be left: shared services should be finished off and the watcher
    /// should exit. The caller must recheck this on a fresh snapshot first.
    CleanUp,
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
            }
            // An app that is still alive after being reaped is reaped again.
            if new == AppState::Reaped {
                events.push(Event::ReapApp {
                    tree: app_tree(processes, app),
                });
            }
        }

        // Forget apps that have exited.
        self.apps.retain(|key, _| alive.contains(key));

        if self.apps.values().all(|state| *state == AppState::Reaped) {
            events.push(Event::CleanUp);
        }
        events
    }
}

/// Runs the watcher until no Adobe app is left.
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
                    Some(from) => debug_log!("{pid:>6}  {exe_name}: {from} -> {to}"),
                    None => debug_log!("{pid:>6}  {exe_name}: {to}"),
                },
                Event::ReapApp { tree } => reap(&tree),
                Event::CleanUp => {
                    // The apps were probably killed a moment ago, and a new one may have
                    // been started meanwhile, so decide on a fresh snapshot.
                    let fresh = process::snapshot()?;
                    if fresh.iter().any(|p| apps::is_main_app(&p.exe_name)) {
                        debug_log!("An app is still around, shared services stay for now");
                    } else {
                        debug_log!("No apps left, finishing off shared services");
                        reap(&shared_services_tree(&fresh));
                        return Ok(());
                    }
                }
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Asks the processes to close, waits a bit, then terminates the ones still alive.
///
/// `tree` is ordered parent first, so the parent can't respawn its children.
fn reap(tree: &[ProcessInfo]) {
    let pids: HashSet<u32> = tree.iter().map(|p| p.pid).collect();
    if let Err(e) = window::post_close(&pids) {
        debug_log!("Failed to post WM_CLOSE: {e}");
    }
    thread::sleep(SOFT_CLOSE_WAIT);
    kill_tree(tree);
}

/// Kills windowless leftovers ("zombies") of the app that is about to be started.
///
/// Adobe apps are single-instance: a new launch is handed over to the running
/// instance, so with a zombie around nothing would open. A windowless instance
/// at launch time is therefore always treated as a zombie and killed right away.
/// Executables that are not known Adobe apps are left alone.
///
/// # Errors
///
/// Returns an error if processes or windows can't be enumerated.
pub fn kill_zombies(exe_name: &str) -> windows::core::Result<()> {
    if !apps::is_main_app(exe_name) {
        return Ok(());
    }
    let processes = process::snapshot()?;
    let with_windows = window::pids_with_visible_windows()?;

    let zombies = processes
        .iter()
        .filter(|p| p.exe_name.eq_ignore_ascii_case(exe_name) && !with_windows.contains(&p.pid));
    for zombie in zombies {
        debug_log!(
            "{:>6}  {}: zombie found before launch",
            zombie.pid,
            zombie.exe_name
        );
        kill_tree(&app_tree(&processes, zombie));
    }
    Ok(())
}

/// Terminates the processes in order, without asking them to close first.
fn kill_tree(tree: &[ProcessInfo]) {
    for p in tree {
        match process::terminate(p) {
            Ok(Termination::Terminated) => debug_log!("{:>6}  {}: killed", p.pid, p.exe_name),
            Ok(Termination::AlreadyGone) => debug_log!("{:>6}  {}: exited", p.pid, p.exe_name),
            Err(e) => debug_log!("{:>6}  {}: failed to kill: {e}", p.pid, p.exe_name),
        }
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
        events.iter().any(|e| matches!(e, Event::CleanUp))
    }

    #[test]
    fn app_is_reaped_after_grace_period() {
        let ps = vec![proc(100, 1, "Photoshop.exe")];
        let t0 = Instant::now();
        let mut w = Watcher::default();

        w.update(&ps, &windows(&[100]), t0);
        let gone = t0 + secs(10);
        w.update(&ps, &windows(&[]), gone); // windows disappear
        let early = w.update(&ps, &windows(&[]), gone + GRACE_PERIOD - secs(1));
        let late = w.update(&ps, &windows(&[]), gone + GRACE_PERIOD);

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
        w.update(&ps, &windows(&[100]), t0 + secs(15)); // a window is back
        let events = w.update(&ps, &windows(&[]), t0 + secs(10) + GRACE_PERIOD);

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
            proc(101, 100, "CEPHtmlEngine.exe"),
            proc(102, 100, "CoreSync.exe"),
            proc(103, 102, "node.exe"),
            proc(104, 100, "Adobe Media Encoder.exe"),
            proc(105, 100, "dynamiclinkmanager.exe"),
            proc(106, 105, "TeamProjectsLocalHub.exe"),
        ];
        let tree: Vec<u32> = app_tree(&ps, &ps[0]).iter().map(|p| p.pid).collect();
        assert_eq!(tree, vec![100, 101]);
    }

    #[test]
    fn cleanup_when_no_apps_left() {
        let ps = vec![proc(200, 1, "CCXProcess.exe"), proc(201, 200, "node.exe")];
        let mut w = Watcher::default();

        let events = w.update(&ps, &windows(&[]), Instant::now());

        assert!(has_cleanup(&events));
    }

    #[test]
    fn shared_services_tree_includes_their_children() {
        let ps = vec![
            proc(200, 1, "CCXProcess.exe"),
            proc(201, 200, "node.exe"),
            proc(210, 1, "CoreSync.exe"),
            proc(300, 1, "notepad.exe"),
        ];
        let pids: Vec<u32> = shared_services_tree(&ps).iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![200, 201, 210]);
    }

    #[test]
    fn reap_is_repeated_while_the_app_is_alive() {
        let ps = vec![proc(100, 1, "Photoshop.exe")];
        let t0 = Instant::now();
        let mut w = Watcher::default();

        w.update(&ps, &windows(&[100]), t0);
        w.update(&ps, &windows(&[]), t0 + secs(10));
        w.update(&ps, &windows(&[]), t0 + secs(10) + GRACE_PERIOD);
        let retry = w.update(&ps, &windows(&[]), t0 + secs(20) + GRACE_PERIOD);

        assert_eq!(reaped_pids(&retry), vec![100]);
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
        let events = w.update(&ps, &windows(&[110]), t0 + secs(10) + GRACE_PERIOD);

        assert_eq!(reaped_pids(&events), vec![100]);
        assert!(!has_cleanup(&events));
    }
}
