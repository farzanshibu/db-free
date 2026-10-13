// SOT: native-menu, menu-events, menu-accelerators, menu-action-ids

use crate::model::MenuActionEvent;
use std::collections::BTreeMap;
use tauri::menu::{Menu, MenuBuilder, MenuItem, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Event the UI listens on: one native menu item was chosen.
pub const MENU_EVENT: &str = "menu:action";

// WHAT:  The app's native menu bar: its items, their ids, and how a choice
//        reaches the UI.
// WHY:   macOS expects a menu bar (and drops ⌘C / ⌘V in a webview without an
//        Edit menu); Windows and Linux get the same actions in the window menu.
//        Every custom item is an *action id* that the UI already knows as a
//        keyboard action (src/lib/keymap.ts), so the menu and the shortcut run
//        the same code.
// HOW:   Choosing an item emits `menu:action` carrying the id and the label of
//        the focused window; each window ignores events that are not for it.
//        Accelerators are attached on macOS only: there the webview sees a
//        keystroke first and the menu only gets what the page left alone, so
//        nothing fires twice. On Windows and Linux an accelerator would fire
//        alongside the page's own handler, so those menus show no key hints.
//        The UI sends the user's current chords (`set_menu_shortcuts`) so a
//        rebinding in Settings reaches the menu too.
// WHERE: src-tauri/src/lib.rs (setup), src-tauri/src/commands/windows.rs
//        (set_menu_shortcuts), src/features/shell/useNativeMenu.ts (listener)
pub fn build<R: Runtime>(app: &AppHandle<R>, shortcuts: &BTreeMap<String, String>) -> tauri::Result<Menu<R>> {
    let item = |id: &str, label: &str| -> tauri::Result<MenuItem<R>> {
        let accelerator = shortcuts.get(id).and_then(|chord| to_accelerator(chord)).filter(|_| cfg!(target_os = "macos"));
        if let Some(accel) = accelerator {
            // An accelerator the platform cannot parse must not cost the item.
            if let Ok(built) = MenuItemBuilder::with_id(id, label).accelerator(accel).build(app) {
                return Ok(built);
            }
        }
        MenuItemBuilder::with_id(id, label).build(app)
    };

    let file = SubmenuBuilder::new(app, "File")
        .item(&item("new-query", "New Query")?)
        .item(&item("new-window", "New Window")?)
        .separator()
        .item(&item("close-tab", "Close Tab")?)
        .item(&item("close-window", "Close Window")?);
    #[cfg(not(target_os = "macos"))]
    let file = file.separator().item(&item("settings", "Settings")?).separator().item(&PredefinedMenuItem::quit(app, Some("Exit"))?);
    let file = file.build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .item(&PredefinedMenuItem::undo(app, None)?)
        .item(&PredefinedMenuItem::redo(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .build()?;

    let layout = SubmenuBuilder::new(app, "Layout")
        .item(&item("layout-default", "Default")?)
        .item(&item("layout-focus", "Focus")?)
        .item(&item("layout-explore", "Explore")?)
        .item(&item("layout-inspect", "Inspect")?)
        .item(&item("layout-ai", "AI")?)
        .build()?;

    let view = SubmenuBuilder::new(app, "View")
        .item(&item("palette", "Command Palette")?)
        .item(&item("toggle-sidebar", "Toggle Sidebar")?)
        .item(&item("toggle-inspector", "Toggle Inspector")?)
        .item(&item("erd", "ER Diagram")?)
        .separator()
        .item(&layout)
        .separator()
        .item(&item("zoom-in", "Zoom In")?)
        .item(&item("zoom-out", "Zoom Out")?)
        .item(&item("zoom-reset", "Actual Size")?);
    #[cfg(target_os = "macos")]
    let view = view.separator().item(&PredefinedMenuItem::fullscreen(app, None)?);
    // The web inspector is only compiled into debug builds (the `devtools` feature is off in release).
    #[cfg(debug_assertions)]
    let view = view.separator().item(&item("devtools", "Toggle Developer Tools")?);
    let view = view.build()?;

    let window = SubmenuBuilder::new(app, "Window")
        .item(&PredefinedMenuItem::minimize(app, None)?)
        .item(&PredefinedMenuItem::maximize(app, None)?)
        .separator()
        .item(&item("prev-tab", "Previous Tab")?)
        .item(&item("next-tab", "Next Tab")?)
        .build()?;

    let help = SubmenuBuilder::new(app, "Help").item(&item("whats-new", "What's New")?).item(&item("learn-more", "Learn More")?);
    #[cfg(not(target_os = "macos"))]
    let help = help.separator().item(&item("check-updates", "Check for Updates…")?).item(&item("about", "About DB Free")?);
    let help = help.build()?;

    let menu = MenuBuilder::new(app);
    #[cfg(target_os = "macos")]
    let menu = {
        let application = SubmenuBuilder::new(app, "DB Free")
            .item(&item("about", "About DB Free")?)
            .item(&item("check-updates", "Check for Updates…")?)
            .separator()
            .item(&item("settings", "Settings…")?)
            .separator()
            .item(&PredefinedMenuItem::services(app, None)?)
            .separator()
            .item(&PredefinedMenuItem::hide(app, None)?)
            .item(&PredefinedMenuItem::hide_others(app, None)?)
            .item(&PredefinedMenuItem::show_all(app, None)?)
            .separator()
            .item(&PredefinedMenuItem::quit(app, None)?)
            .build()?;
        menu.item(&application)
    };
    menu.item(&file).item(&edit).item(&view).item(&window).item(&help).build()
}

// WHAT:  Hands a chosen menu item to the window that was focused.
// WHY:   The menu is app-wide, but the action (close *this* tab, toggle *this*
//        sidebar) belongs to one window. Events are broadcast, so the target
//        label rides in the payload and every other window ignores it.
pub fn dispatch<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let focused = app.webview_windows().into_values().find(|w| w.is_focused().unwrap_or(false));
    // Developer tools belong to the webview itself; there is nothing for the UI to do.
    #[cfg(debug_assertions)]
    if id == "devtools" {
        if let Some(window) = &focused {
            if window.is_devtools_open() {
                window.close_devtools();
            } else {
                window.open_devtools();
            }
        }
        return;
    }
    let label = focused.map_or_else(|| "main".to_string(), |w| w.label().to_string());
    if let Err(err) = app.emit(MENU_EVENT, MenuActionEvent { target: label, action: id.to_string() }) {
        log::warn!("menu action {id} could not reach the UI: {err}");
    }
}

// WHAT:  A UI chord ("Mod+Shift+N") as a native accelerator ("CmdOrCtrl+Shift+N").
// WHY:   The keymap spells the platform modifier `Mod`; the menu library spells
//        it `CmdOrCtrl`. Everything else (keys, `Alt`, `Shift`, `Ctrl`) already
//        agrees. An empty chord is an unbound action: no accelerator.
pub fn to_accelerator(chord: &str) -> Option<String> {
    let parts: Vec<&str> = chord.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let (key, modifiers) = parts.split_last()?;
    let mut out: Vec<String> = Vec::with_capacity(parts.len());
    for modifier in modifiers {
        out.push(match modifier.to_ascii_lowercase().as_str() {
            "mod" => "CmdOrCtrl".to_string(),
            "ctrl" => "Ctrl".to_string(),
            "meta" | "cmd" => "Super".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            _ => return None,
        });
    }
    out.push((*key).to_string());
    Some(out.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_becomes_cmd_or_ctrl() {
        assert_eq!(to_accelerator("Mod+Shift+N").as_deref(), Some("CmdOrCtrl+Shift+N"));
        assert_eq!(to_accelerator("Mod+,").as_deref(), Some("CmdOrCtrl+,"));
        assert_eq!(to_accelerator("Mod+Shift+[").as_deref(), Some("CmdOrCtrl+Shift+["));
    }

    #[test]
    fn other_modifiers_pass_through() {
        assert_eq!(to_accelerator("Ctrl+Shift+Tab").as_deref(), Some("Ctrl+Shift+Tab"));
        assert_eq!(to_accelerator("Alt+Shift+F").as_deref(), Some("Alt+Shift+F"));
        assert_eq!(to_accelerator("Meta+K").as_deref(), Some("Super+K"));
    }

    #[test]
    fn unbound_or_malformed_chords_have_no_accelerator() {
        assert_eq!(to_accelerator(""), None);
        assert_eq!(to_accelerator("  "), None);
        assert_eq!(to_accelerator("Hyper+K"), None);
    }
}
