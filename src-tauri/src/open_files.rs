// SOT: os-file-open, run-event-opened, single-instance-forwarding, transient-sweep

use crate::model::ConnectionOpenedEvent;
use crate::services::connection_files;
use crate::state::AppState;
use std::ffi::OsString;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, RunEvent};

// ============================================================================
// WHAT:  The glue between the operating system and "open this database file":
//        macOS `RunEvent::Opened`, the arguments of a first launch (Windows /
//        Linux file association) and the arguments a second launch forwards to
//        the running instance (single-instance plugin).
// WHY:   Double-clicking `shop.sqlite` should open it, in the app that is
//        already running if there is one, without a trip through the New
//        Connection form.
// HOW:   Each database file becomes a transient connection (see
//        services::connection_files) and the UI is told with a
//        `connection:opened` event. A launch that races the webview (a cold
//        start from a double-click) loses the event, so the UI also opens any
//        transient connection it finds when it loads its list. Transient
//        connections are swept at startup and when the app exits.
// WHERE: src-tauri/src/lib.rs (wiring), src/features/connections/ConnectionEvents.tsx (listener)
// ============================================================================

pub const OPENED_EVENT: &str = "connection:opened";

// WHAT:  Opens every database file among `args`, ignoring flags and other paths.
pub fn open_from_args(app: &AppHandle, args: impl IntoIterator<Item = OsString>) {
    let paths: Vec<PathBuf> =
        args.into_iter().map(PathBuf::from).filter(|p| connection_files::is_database_file(p) && p.is_file()).collect();
    open_paths(app, &paths);
}

fn open_paths(app: &AppHandle, paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    for path in paths {
        match connection_files::open_database_file(&state, path) {
            Ok(summary) => {
                let event = ConnectionOpenedEvent { connection_id: summary.id, name: summary.name };
                if let Err(err) = app.emit(OPENED_EVENT, event) {
                    log::warn!("could not tell the window about {}: {err}", path.display());
                }
            }
            Err(err) => log::warn!("could not open {}: {err}", path.display()),
        }
    }
}

// WHAT:  A second instance was launched: its arguments are ours to open, its window is not wanted.
pub fn on_second_launch(app: &AppHandle, argv: &[String]) {
    open_from_args(app, argv.iter().skip(1).map(OsString::from));
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

// WHAT:  Removes the transient connections an earlier run left behind (a crash skips the exit sweep).
pub fn sweep_stale_transients(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        match connection_files::sweep_transient(&state) {
            Ok(0) => {}
            Ok(removed) => log::info!("removed {removed} transient connection(s) from the last run"),
            Err(err) => log::warn!("could not clear transient connections: {err}"),
        }
    }
}

pub fn on_run_event(app: &AppHandle, event: &RunEvent) {
    match event {
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            let paths: Vec<PathBuf> = urls.iter().filter_map(|url| url.to_file_path().ok()).filter(|p| connection_files::is_database_file(p)).collect();
            open_paths(app, &paths);
        }
        RunEvent::Exit => sweep_stale_transients(app),
        _ => {}
    }
}
