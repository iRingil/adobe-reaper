<!--suppress ALL -->
<p align="center">
    <a href="https://www.rust-lang.org">
        <img src="https://img.shields.io/badge/rust-v1.88%2B-informational" alt="rust version">
    </a>
    <a href="https://crates.io/crates/windows/0.62.2">
        <img src="https://img.shields.io/badge/windows-v0.62.2-informational" alt="windows crate version">
    </a>
    <a href="https://crates.io/crates/winresource/0.1.34">
        <img src="https://img.shields.io/badge/winresource-v0.1.34-informational" alt="winresource version">
    </a>
</p>
<p align="center">
    <a href="https://github.com/iRingil/adobe-reaper/actions/workflows/dependabot/dependabot-updates">
        <img src="https://github.com/iRingil/adobe-reaper/actions/workflows/dependabot/dependabot-updates/badge.svg" alt="Dependabot Updates">
    </a>
    <a href="https://github.com/iRingil/adobe-reaper/actions/workflows/github-code-scanning/codeql">
        <img src="https://github.com/iRingil/adobe-reaper/actions/workflows/github-code-scanning/codeql/badge.svg" alt="CodeQL">
    </a>
    <a href="https://github.com/iRingil/adobe-reaper/actions/workflows/ci.yml">
        <img src="https://github.com/iRingil/adobe-reaper/actions/workflows/ci.yml/badge.svg" alt="CI">
    </a>
    <a href="https://github.com/iRingil/adobe-reaper/actions/workflows/audit.yml">
        <img src="https://github.com/iRingil/adobe-reaper/actions/workflows/audit.yml/badge.svg" alt="Audit">
    </a>
    <a href="LICENSE.md">
        <img src="https://img.shields.io/github/license/iRingil/adobe-reaper" alt="License">
    </a>
</p>

**Adobe Reaper** is an invisible launcher for Adobe 2020 apps that finishes off the processes they leave hanging
after you close them.

Adobe Reaper is an unofficial tool and is not affiliated with or endorsed by Adobe.

## The problem

Photoshop, Premiere Pro, Illustrator, Audition and Media Encoder 2020 often don't exit when you close their window.
The main process stays in memory without a single window, together with a bunch of helpers (`dynamiclinkmanager.exe`,
`CEPHtmlEngine.exe`, `CoreSync.exe`, `CCXProcess.exe` and their `node.exe`, ...). Which app hangs varies from run to run.

This hurts in two ways:

- **Memory.** A hung Photoshop easily keeps hundreds of megabytes, and the leftovers pile up during the day.
- **The app won't start again.** Adobe apps are single-instance: a new launch is handed over to the running instance,
  so while a windowless zombie hangs around, clicking the shortcut opens nothing.

The hung processes ignore a polite close request (`taskkill` without `/F`); only killing them helps. Adobe Reaper does
that automatically, and only when the app is really gone from the screen.

## How it works

You start the apps from shortcuts that point to `adobe-reaper.exe` and pass it the app to start:

```
adobe-reaper.exe "C:\Program Files\Adobe\Adobe Photoshop 2020\Photoshop.exe"
```

1. **Zombies first.** If a windowless instance of the same app is already hanging, it is killed: otherwise it would
   swallow the launch.
2. **Launch.** The app is started; any further arguments are passed to it unchanged.
3. **One watcher.** The first Adobe Reaper becomes the watcher (a named mutex makes sure there is only one). Every
   later one just starts its app and exits: the watcher picks the new app up on its next poll, no IPC involved.
4. **Watching windows, not processes.** Every 10 seconds the watcher checks whether each Adobe app still has a
   visible window. The countdown starts only after the app's first window, so a slow splash screen is never killed.
5. **Finishing off.** When an app's windows are gone for 20 seconds (time to finish writing files), the app and its
   child processes are asked to close and, 5 seconds later, killed.
6. **Shared services last.** Creative Cloud services (`CoreSync.exe`, `CCXProcess.exe`, `CCLibrary.exe`,
   `AdobeIPCBroker.exe`, `dynamiclinkmanager.exe`) serve all apps, so they are killed only when no Adobe app is
   left, after a fresh check that no app was started meanwhile. Then the watcher exits.

Media Encoder started by Premiere Pro for an export is a separate app for the watcher: it is never killed together
with Premiere while it still has a window.

The watcher runs without a window or a console and keeps no logs, settings or files of its own. A killed process is
identified by its PID and start time, so a process that reused the PID of a dead one is never touched.

## Installation

There are no prebuilt releases: build it yourself, it takes a minute.

1. Install [Rust](https://www.rust-lang.org/tools/install) (1.88 or newer, the MSVC toolchain).
2. Build the release executable from the repository root:

   ```shell
   cargo build --release
   ```

3. Copy `target\release\adobe-reaper.exe` to a permanent place, for example `C:\Program Files\Adobe\`
   (needs administrator rights) or `%LOCALAPPDATA%\Programs\adobe-reaper\`.
4. Change the shortcuts of the Adobe apps. The Start menu ones are in
   `C:\ProgramData\Microsoft\Windows\Start Menu\Programs`; pinned taskbar icons need the same change. For each one:
    - **Target**: `"<path to>\adobe-reaper.exe" "<path to the Adobe app>.exe"`
    - **Start in**: the folder of the Adobe app, as it was
    - **Change Icon**: browse to the Adobe app's `.exe` to keep its icon

   For example, Photoshop:

   ```
   "C:\Program Files\Adobe\adobe-reaper.exe" "C:\Program Files\Adobe\Adobe Photoshop 2020\Photoshop.exe"
   ```

Apps started some other way (from a file association, by another app) are still watched while a watcher is running,
but they don't start one.

To uninstall, restore the shortcuts and delete `adobe-reaper.exe`.

## Adding more Adobe apps or processes

All the process names live in [`src/apps.rs`](src/apps.rs):

- **`MAIN_APPS`**: the main executables of the apps to watch. An app gets watched, finished off when its windows
  are gone, and killed as a zombie at launch. Add another Adobe app here (and give it a shortcut as above).
- **`SHARED_SERVICES`**: processes that serve several apps at once. They are never killed together with an app, only
  at the very end, when no app is left. Add a helper here if it outlives its parent app while other apps use it.

Ordinary child processes need no list: they are found through the parent-child tree of each app. Names are
compared without regard to case. After a change, run `cargo test` and rebuild.

The timings (poll interval, grace period, soft-close wait) are constants at the top of
[`src/watcher.rs`](src/watcher.rs).

## Privacy

Adobe Reaper doesn't use the network and writes nothing to disk: no settings, logs or caches. It only reads the list
of processes and windows of the current Windows session and closes or kills Adobe processes.

## Building from source

With Rust on Windows, from the repository root:

```shell
cargo run -- "C:\Program Files\Adobe\Adobe Photoshop 2020\Photoshop.exe"   # debug build: with a console and log output
cargo fmt                                                                   # format
cargo clippy --all-targets -- -D warnings                                   # lint
cargo test                                                                  # tests
cargo build --release                                                       # target\release\adobe-reaper.exe, no console
```

The debug build keeps a console and prints every decision of the watcher; the release build has neither.

## License

Adobe Reaper is released under the [MIT License](LICENSE.md). It is built with these third-party components:

- [windows](https://github.com/microsoft/windows-rs) (Windows API bindings): MIT or Apache-2.0
- [winresource](https://github.com/BenjaminRi/winresource) (embeds the icon, build time only): MIT

Adobe, Photoshop, Premiere Pro, Illustrator, Audition, Media Encoder and Creative Cloud are trademarks of Adobe Inc.
