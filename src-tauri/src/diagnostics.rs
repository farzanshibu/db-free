// SOT: log-file, log-rotation, panic-hook, app-error-event

use crate::model::AppErrorEvent;
use std::any::Any;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_log::{Builder, RotationStrategy, Target, TargetKind};

/// Event the UI turns into an error toast.
pub const ERROR_EVENT: &str = "app:error";
/// A log file is rolled over once it passes this size…
const MAX_LOG_BYTES: u128 = 2 * 1024 * 1024;
/// …and this many rolled files are kept beside the live one.
const KEPT_LOG_FILES: usize = 5;

// WHAT:  The app's logger: a rotating file in the OS log directory, plus
//        stdout for `tauri dev`.
// WHY:   `env_logger` wrote to a terminal nobody has when the app is launched
//        from the Dock. A user reporting a crash can now attach a file
//        (About → Open log folder) instead of describing it.
// HOW:   Warnings and errors from every crate, info from this one, so a
//        driver's chatter does not bury the app's own lines. Files roll at
//        2 MB and five are kept: a few MB of history, never unbounded.
// WHERE: src-tauri/src/lib.rs (plugin chain), src-tauri/src/commands/windows.rs
//        (open_log_folder, log_ui_error write into the same file)
pub fn log_plugin<R: Runtime>() -> TauriPlugin<R> {
    Builder::new()
        .level(log::LevelFilter::Warn)
        .level_for("db_free_lib", log::LevelFilter::Info)
        .targets([Target::new(TargetKind::Stdout), Target::new(TargetKind::LogDir { file_name: Some("db-free".to_string()) })])
        .max_file_size(MAX_LOG_BYTES)
        .rotation_strategy(RotationStrategy::KeepSome(KEPT_LOG_FILES))
        .build()
}

// WHAT:  Routes a panic anywhere in the process to the log file and the UI.
// WHY:   A release build aborts on panic (Cargo.toml), taking the window with
//        it and leaving nothing behind. The hook runs first: it records where
//        and why, and asks the UI to show it before the process goes.
// HOW:   The previous hook still runs afterwards, so the default stderr report
//        and any debugger integration are kept.
pub fn install_panic_hook<R: Runtime>(app: AppHandle<R>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map_or_else(|| "unknown location".to_string(), |l| format!("{}:{}", l.file(), l.line()));
        let reason = panic_message(info.payload());
        log::error!("panic at {location}: {reason}");
        // The window may already be gone; a toast nobody can see is not worth a second failure.
        let _ = app.emit(ERROR_EVENT, AppErrorEvent { message: format!("DB Free hit an internal error: {reason}") });
        previous(info);
    }));
}

/// The text of a panic payload: `panic!("…")` carries a `&str`, `panic!("{x}")` a `String`.
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_message_reads_both_payload_shapes() {
        let literal: Box<dyn Any + Send> = Box::new("plain");
        let formatted: Box<dyn Any + Send> = Box::new(String::from("formatted"));
        let other: Box<dyn Any + Send> = Box::new(42_u8);
        assert_eq!(panic_message(literal.as_ref()), "plain");
        assert_eq!(panic_message(formatted.as_ref()), "formatted");
        assert_eq!(panic_message(other.as_ref()), "unknown panic");
    }
}
