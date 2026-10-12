// SOT: window-commands, multi-window, ui-zoom, native-window-theme, open-external, ui-error-log, menu-shortcuts, theme-file-commands

use crate::error::{AppError, AppResult};
use crate::guard;
use crate::model::ThemeBase;
use crate::services;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use ts_rs::TS;

/// Windows bound to one connection: `conn-<connection id>`.
const CONNECTION_PREFIX: &str = "conn-";
/// Extra windows with no connection yet: `win-1`, `win-2`…
const EXTRA_PREFIX: &str = "win-";
/// Whole-app zoom bounds (Settings, ⌘+ / ⌘-).
const MIN_ZOOM: f64 = 0.9;
const MAX_ZOOM: f64 = 1.5;
/// Longest UI error message written to the log, so a runaway string cannot flood it.
const MAX_LOG_MESSAGE: usize = 4_000;
/// Hosts `open_external` may open: this project, nothing else.
const EXTERNAL_HOSTS: &[&str] = &["github.com", "www.github.com"];

// WHAT:  The chords last sent for the native menu, so an unchanged keymap does
//        not rebuild the menu (every window sends it at startup).
// HOW:   Managed by Tauri (src-tauri/src/lib.rs). Never held across an await.
#[derive(Default)]
pub struct MenuShortcuts(Mutex<BTreeMap<String, String>>);

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenWindowRequest {
    /// Open (or focus) the window for this connection; None opens a blank extra window.
    pub connection_id: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetZoomRequest {
    pub zoom: f64,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetWindowThemeRequest {
    /// None follows the operating system.
    pub theme: Option<ThemeBase>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenExternalRequest {
    pub url: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LogUiErrorRequest {
    pub message: String,
    /// Where it happened (`window.onerror`, `unhandledrejection`, a component).
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MenuShortcutsRequest {
    /// Menu action id → chord ("Mod+Shift+N"); an empty chord clears the hint.
    pub shortcuts: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportThemeRequest {
    /// The path the OS save dialog returned.
    pub path: String,
    pub contents: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportThemeRequest {
    /// The path the OS open dialog returned.
    pub path: String,
}

// WHAT:  The window label for a connection: `conn-` + the id with every
//        character a label cannot hold replaced.
// WHY:   Labels double as capability scopes and as the window-state key; the
//        same connection must always map to the same label so its window is
//        found (and focused) instead of duplicated.
pub fn connection_label(connection_id: &str) -> String {
    let id: String = connection_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    format!("{CONNECTION_PREFIX}{id}")
}

// WHAT:  The lowest `win-N` not already taken.
// WHY:   Reusing numbers keeps the saved window geometry from growing without
//        bound: a closed `win-2` hands its state to the next `win-2`.
pub fn next_extra_label<'a>(taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: Vec<&str> = taken.into_iter().collect();
    (1_u32..)
        .map(|n| format!("{EXTRA_PREFIX}{n}"))
        .find(|candidate| !taken.contains(&candidate.as_str()))
        .unwrap_or_else(|| format!("{EXTRA_PREFIX}0"))
}

/// Keeps a zoom factor inside the supported range; NaN falls back to 100%.
pub fn clamp_zoom(zoom: f64) -> f64 {
    if zoom.is_nan() {
        1.0
    } else {
        zoom.clamp(MIN_ZOOM, MAX_ZOOM)
    }
}

// WHAT:  Opens a new window, or brings an existing one forward.
// WHY:   "New Window" (⌘⇧N) and "open connections in a new window" share one
//        entry point. A connection has exactly one window, keyed by its label.
// HOW:   The new window is built from the main window's config (overlay title
//        bar, traffic-light inset, size limits) with its own label and a
//        `?window=…&connection=…` query the UI reads to know what it is for.
// WHERE: src/stores/workspace.ts (bootstrap reads the query), tauri.conf.json
#[tauri::command]
pub async fn open_window(app: AppHandle, req: OpenWindowRequest) -> AppResult<String> {
    guard::local("open_window", async move {
        let existing: Vec<String> = app.webview_windows().keys().cloned().collect();
        let (label, query) = match req.connection_id.as_deref().map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => {
                let label = connection_label(id);
                let query = format!("window={label}&connection={}", query_escape(id));
                (label, query)
            }
            None => {
                let label = next_extra_label(existing.iter().map(String::as_str));
                let query = format!("window={label}");
                (label, query)
            }
        };
        if let Some(window) = app.get_webview_window(&label) {
            focus(&window)?;
            return Ok(label);
        }
        let mut config = app.config().app.windows.first().cloned().unwrap_or_default();
        config.label.clone_from(&label);
        config.url = WebviewUrl::App(format!("index.html?{query}").into());
        // Only the first window is restored at its saved geometry by name; extra
        // windows cascade from wherever the OS puts them.
        config.center = false;
        let window = WebviewWindowBuilder::from_config(&app, &config)
            .and_then(WebviewWindowBuilder::build)
            .map_err(|e| AppError::internal(format!("could not open a window: {e}")))?;
        focus(&window)?;
        Ok(label)
    })
    .await
}

fn focus(window: &WebviewWindow) -> AppResult<()> {
    window.show().map_err(|e| AppError::internal(format!("could not show the window: {e}")))?;
    // Both calls can fail on a window that is mid-close; the next click retries.
    let _ = window.unminimize();
    let _ = window.set_focus();
    Ok(())
}

/// Percent-encodes everything outside the unreserved set, for a query value.
fn query_escape(value: &str) -> String {
    value
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { char::from(b).to_string() } else { format!("%{b:02X}") })
        .collect()
}

// WHAT:  Closes the window that asked (⌘⇧W, the File menu).
#[tauri::command]
pub async fn close_window(window: WebviewWindow) -> AppResult<()> {
    guard::local("close_window", async move { window.close().map_err(|e| AppError::internal(format!("could not close the window: {e}"))) }).await
}

// WHAT:  Sets this window's page zoom and says what was applied.
// WHY:   Whole-app zoom (0.9–1.5) scales the entire UI; the webview does it, so
//        layout, fonts and the grid all follow with no CSS of ours.
#[tauri::command]
pub async fn set_ui_zoom(window: WebviewWindow, req: SetZoomRequest) -> AppResult<f64> {
    guard::local("set_ui_zoom", async move {
        let zoom = clamp_zoom(req.zoom);
        window.set_zoom(zoom).map_err(|e| AppError::internal(format!("could not set the zoom: {e}")))?;
        Ok(zoom)
    })
    .await
}

// WHAT:  Makes the OS window chrome (title bar, traffic lights, native menus)
//        match the app's colour scheme; None hands it back to the OS.
// WHY:   A dark app under a light title bar looks broken. "System" must pass
//        None, or the webview's own `prefers-color-scheme` would stop following
//        the OS and the theme could never switch back.
#[tauri::command]
pub async fn set_window_theme(app: AppHandle, req: SetWindowThemeRequest) -> AppResult<()> {
    guard::local("set_window_theme", async move {
        let theme = req.theme.map(|base| match base {
            ThemeBase::Light => tauri::Theme::Light,
            ThemeBase::Dark => tauri::Theme::Dark,
        });
        for window in app.webview_windows().values() {
            // A window being closed rejects this; the others should still change.
            if let Err(err) = window.set_theme(theme) {
                log::debug!("window theme not applied to {}: {err}", window.label());
            }
        }
        Ok(())
    })
    .await
}

// WHAT:  Opens an https link on this project's GitHub in the default browser.
// WHY:   The UI links to release notes and docs; the webview itself must never
//        navigate away from the app, so the OS opens it. Only https and only
//        the project host, so the UI cannot be turned into an arbitrary launcher.
#[tauri::command]
pub async fn open_external(req: OpenExternalRequest) -> AppResult<()> {
    guard::local("open_external", async move {
        let parsed = url::Url::parse(req.url.trim()).map_err(|_| AppError::invalid_input("That is not a valid link."))?;
        let host_ok = parsed.host_str().is_some_and(|host| EXTERNAL_HOSTS.contains(&host));
        if parsed.scheme() != "https" || !host_ok {
            return Err(AppError::invalid_input("Only links to this project's GitHub page can be opened."));
        }
        launch(parsed.as_str())
    })
    .await
}

// WHAT:  Reveals the log directory in the system file manager.
// WHERE: Settings → Advanced, the About dialog.
#[tauri::command]
pub async fn open_log_folder(app: AppHandle) -> AppResult<()> {
    guard::local("open_log_folder", async move {
        let dir = app.path().app_log_dir().map_err(|e| AppError::internal(format!("no log directory: {e}")))?;
        std::fs::create_dir_all(&dir).map_err(|e| AppError::internal(format!("could not create the log directory: {e}")))?;
        launch(&dir.to_string_lossy())
    })
    .await
}

/// Hands a URL or directory to the platform's opener. The target is passed as a
/// single argument, never through a shell.
fn launch(target: &str) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = Command::new("xdg-open");
    let mut child = command.arg(target).spawn().map_err(|e| AppError::internal(format!("could not open {target}: {e}")))?;
    // Reap the opener when it exits, or each opened link leaves a zombie behind.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

// WHAT:  Writes a UI-side error to the rotating log file.
// WHY:   `window.onerror` and unhandled promise rejections only reach the
//        webview console, which a user cannot send; the log file can be.
#[tauri::command]
pub async fn log_ui_error(req: LogUiErrorRequest) -> AppResult<()> {
    guard::local("log_ui_error", async move {
        let message: String = req.message.chars().take(MAX_LOG_MESSAGE).collect();
        log::error!("ui[{}]: {message}", req.source.as_deref().unwrap_or("unknown"));
        Ok(())
    })
    .await
}

// WHAT:  Writes a custom theme's JSON to the file the user picked.
// WHERE: src/features/settings/ThemesSection.tsx
#[tauri::command]
pub async fn export_theme_file(req: ExportThemeRequest) -> AppResult<String> {
    guard::local("export_theme_file", async move { services::theme_files::write(Path::new(&req.path), &req.contents) }).await
}

// WHAT:  Reads a theme file the user picked, as text for the UI to validate.
#[tauri::command]
pub async fn import_theme_file(req: ImportThemeRequest) -> AppResult<String> {
    guard::local("import_theme_file", async move { services::theme_files::read(Path::new(&req.path)) }).await
}

// WHAT:  Rebuilds the native menu with the user's current chords.
// WHY:   Menu hints must follow a rebinding in Settings. Every window sends its
//        keymap at startup, so an unchanged map is a no-op.
#[tauri::command]
pub async fn set_menu_shortcuts(app: AppHandle, state: State<'_, MenuShortcuts>, req: MenuShortcutsRequest) -> AppResult<()> {
    guard::local("set_menu_shortcuts", async move {
        {
            let mut current = state.0.lock().map_err(|_| AppError::internal("menu shortcut lock poisoned"))?;
            if *current == req.shortcuts {
                return Ok(());
            }
            current.clone_from(&req.shortcuts);
        }
        let menu = crate::menu::build(&app, &req.shortcuts).map_err(|e| AppError::internal(format!("could not build the menu: {e}")))?;
        app.set_menu(menu).map_err(|e| AppError::internal(format!("could not set the menu: {e}")))?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_labels_are_stable_and_label_safe() {
        assert_eq!(connection_label("3f2a-91bc"), "conn-3f2a-91bc");
        assert_eq!(connection_label("a b/c:d"), "conn-a_b_c_d");
        assert_eq!(connection_label("x"), connection_label("x"));
    }

    #[test]
    fn extra_labels_fill_the_lowest_gap() {
        assert_eq!(next_extra_label(["main"]), "win-1");
        assert_eq!(next_extra_label(["main", "win-1", "win-3"]), "win-2");
        assert_eq!(next_extra_label(["win-1", "win-2"]), "win-3");
    }

    #[test]
    fn zoom_is_clamped_to_the_supported_range() {
        assert!((clamp_zoom(0.5) - 0.9).abs() < f64::EPSILON);
        assert!((clamp_zoom(3.0) - 1.5).abs() < f64::EPSILON);
        assert!((clamp_zoom(1.25) - 1.25).abs() < f64::EPSILON);
        assert!((clamp_zoom(f64::NAN) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(query_escape("abc-123_x"), "abc-123_x");
        assert_eq!(query_escape("a b&c"), "a%20b%26c");
    }

    #[test]
    fn only_project_links_may_be_opened() {
        let allowed = |raw: &str| {
            url::Url::parse(raw).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some_and(|h| EXTERNAL_HOSTS.contains(&h)))
        };
        assert!(allowed("https://github.com/farzanshibu/db-free/releases"));
        assert!(!allowed("http://github.com/farzanshibu/db-free"));
        assert!(!allowed("https://evil.example/github.com"));
        assert!(!allowed("file:///etc/passwd"));
    }
}
