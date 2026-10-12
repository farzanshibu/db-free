// SOT: custom-theme, app-info, window-ui-events, shell-model

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

// WHAT:  Which built-in palette a custom theme starts from and which colour
//        scheme (light or dark) the native window and the editors should use.
// WHY:   A theme restates colours, not behaviour: CodeMirror's dark flag, the
//        diagram canvas and the OS window chrome still need to know whether the
//        ground is dark or light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ThemeBase {
    Light,
    Dark,
}

// WHAT:  A user-made theme: a name, its base scheme and CSS-variable overrides.
// WHY:   Stored inside `AppSettings.custom_themes`, so it travels with the rest
//        of the preferences and every window sees the same list.
// HOW:   `colors` maps a token name (`--background`) to a colour value
//        (`oklch(...)`, `#rrggbb`, `hsl(...)`, `rgb(...)`). The UI validates a
//        value before it ever reaches the document (src/lib/themeFile.ts), so
//        a theme file can never inject `url(...)` or any other CSS.
// WHERE: src/lib/themes.ts (token catalogue), src/features/settings/ThemesSection.tsx
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CustomTheme {
    /// Stable id, `custom-<random>`; the `theme` setting stores it.
    pub id: String,
    pub name: String,
    pub base: ThemeBase,
    /// Token name → colour value; tokens left out inherit from the base theme.
    pub colors: BTreeMap<String, String>,
}

// WHAT:  What the About dialog shows: the build, and where this install keeps things.
// WHY:   "Which version am I on, and where are the logs?" is the first thing a
//        bug report needs; the answer should not require a terminal.
// WHERE: src-tauri/src/commands/updates.rs (app_info), src/features/shell/AboutDialog.tsx
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub identifier: String,
    /// `macos`, `windows` or `linux`.
    pub os: String,
    pub arch: String,
    /// Directory the rotating log files live in, when the OS gave one.
    pub log_dir: Option<String>,
    /// Directory holding the app's own SQLite store.
    pub data_dir: Option<String>,
}

// WHAT:  Payload of the `app:error` event: something failed on the Rust side
//        with no request to carry the error back (a panic, a menu action).
// WHY:   The UI turns it into a toast, so a failure is never only in a log file.
// WHERE: src-tauri/src/lib.rs (panic hook), src/lib/shellEvents.ts (listener)
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppErrorEvent {
    pub message: String,
}

// WHAT:  Payload of the `menu:action` event: a native menu item was chosen.
// WHY:   The menu is app-wide, so the event is broadcast; `target` is the label
//        of the window that was focused and every other window ignores it.
// WHERE: src-tauri/src/menu.rs (emitter), src/features/shell/useNativeMenu.ts
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MenuActionEvent {
    pub target: String,
    /// A keyboard-action id (`close-tab`, `zoom-in`…) or a menu-only id (`about`, `layout-focus`).
    pub action: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    // WHAT:  Settings saved before the shell preferences existed still load, with
    //        every new field at its default.
    #[test]
    fn settings_without_the_shell_fields_load_with_defaults() {
        let parsed: crate::model::AppSettings = serde_json::from_str(r#"{"theme":"light","uiFontSize":14}"#).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.theme, "light");
        assert!(!parsed.open_connections_in_new_window);
        assert!((parsed.ui_zoom - 1.0).abs() < f64::EPSILON);
        assert!(!parsed.reuse_table_preview_tab);
        assert_eq!(parsed.grid_scrollbar_size, "standard");
        assert!(parsed.date_picker_enabled);
        assert!(parsed.command_menu_hidden_sections.is_empty());
        assert!(parsed.custom_themes.is_empty());
        assert!(parsed.check_updates_on_launch);
    }

    #[test]
    fn custom_theme_round_trips_through_json() {
        let mut colors = BTreeMap::new();
        colors.insert("--background".to_string(), "#101010".to_string());
        let theme = CustomTheme { id: "custom-1".into(), name: "Mine".into(), base: ThemeBase::Dark, colors };
        let json = serde_json::to_string(&theme).unwrap_or_default();
        assert!(json.contains("\"base\":\"dark\""));
        let back: CustomTheme = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, theme);
    }
}
