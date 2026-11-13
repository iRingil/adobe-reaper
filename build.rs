//! Build script: embeds the application icon into the Windows executable.

fn main() {
    // Cargo reruns this script only when the icon changes (or the script itself).
    println!("cargo:rerun-if-changed=assets/icon.ico");

    // Resources exist only in Windows executables.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/icon.ico");
        resource.compile().expect("failed to embed the icon");
    }
}
