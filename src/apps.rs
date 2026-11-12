//! Known Adobe executables.

/// Main executables of the Adobe applications the watcher looks after.
pub const MAIN_APPS: &[&str] = &[
    "Photoshop.exe",
    "Adobe Premiere Pro.exe",
    "Illustrator.exe",
    "Adobe Audition.exe",
    "Adobe Media Encoder.exe",
];

/// Shared Creative Cloud services.
///
/// They may be started as children of whichever app was launched first,
/// but they serve all apps, so they are killed only when no app has windows left.
pub const SHARED_SERVICES: &[&str] = &[
    "CoreSync.exe",
    "CCXProcess.exe",
    "CCLibrary.exe",
    "AdobeIPCBroker.exe",
];

/// Checks whether `exe_name` is the main executable of a watched Adobe app.
pub fn is_main_app(exe_name: &str) -> bool {
    contains_ignore_case(MAIN_APPS, exe_name)
}

/// Checks whether `exe_name` is a shared Creative Cloud service.
pub fn is_shared_service(exe_name: &str) -> bool {
    contains_ignore_case(SHARED_SERVICES, exe_name)
}

/// Windows file names are case-insensitive, so the comparison is too.
fn contains_ignore_case(list: &[&str], name: &str) -> bool {
    list.iter().any(|item| item.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_app_matches_ignoring_case() {
        assert!(is_main_app("Photoshop.exe"));
        assert!(is_main_app("PHOTOSHOP.EXE"));
        assert!(is_main_app("adobe premiere pro.exe"));
    }

    #[test]
    fn shared_service_is_not_main_app() {
        assert!(is_shared_service("CoreSync.exe"));
        assert!(!is_main_app("CoreSync.exe"));
    }

    #[test]
    fn unrelated_exe_is_ignored() {
        assert!(!is_main_app("notepad.exe"));
        assert!(!is_shared_service("notepad.exe"));
    }
}
