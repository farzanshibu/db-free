// SOT: app-settings, settings-model, ai-settings, execution-mode, run-scope, key-bindings, user-snippets, theme-setting

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

// WHAT:  Every user preference, as one typed document (stored as JSON under the
//        settings key "app"). Unknown fields are ignored; missing ones default.
// WHY:   The settings page edits this struct directly; the guard and the UI read
//        the same source (execution mode decides review vs direct edits).
// WHERE: src-tauri/src/services/settings.rs, src/features/settings/SettingsPage.tsx
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExecutionMode {
    /// Edits queue in the Pending Changes panel until Commit.
    Review,
    /// Edits apply immediately.
    Direct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunScope {
    /// Run every statement in the editor when nothing is selected.
    All,
    /// Run only the statement under the cursor.
    Current,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AiProvider {
    None,
    Anthropic,
    Openai,
    Openrouter,
    Ollama,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiSettings {
    pub provider: AiProvider,
    pub model: String,
    /// Override for self-hosted / proxy endpoints (Ollama defaults to http://127.0.0.1:11434).
    pub base_url: Option<String>,
    /// Never contains the key itself; the key is sealed separately in the store.
    #[serde(default)]
    pub has_api_key: bool,
    /// How far the assistant may act on its own before asking. Defaulted so
    /// settings written before the agent existed still load.
    #[serde(default)]
    pub autonomy: crate::model::AgentAutonomy,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            provider: AiProvider::None,
            model: "claude-opus-5".to_string(),
            base_url: None,
            has_api_key: false,
            autonomy: crate::model::AgentAutonomy::AskOnWrite,
        }
    }
}

// WHAT:  One user override of a keyboard shortcut: which action, which chord.
// WHY:   Defaults live in the UI registry; only the rebinds are stored, so a new
//        default shipped later still reaches everyone who never touched it.
// HOW:   `action` is a `ShortcutAction` id and `keys` a chord like "Mod+Shift+T";
//        an empty `keys` unbinds the action. Unknown actions are ignored on read.
// WHERE: src/lib/keymap.ts (registry, `resolveKeymap`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KeyBinding {
    pub action: String,
    pub keys: String,
}

// WHAT:  One user snippet for the query editor: typing `prefix` then Tab
//        expands `body` (CodeMirror snippet syntax, `${field}` placeholders).
// WHY:   The built-in set lives in the UI; only the user's own are stored.
// HOW:   `language` is a `SnippetLanguage` id ("sql", "cypher", "mongo",
//        "redis" or "any"); a snippet for another language is not offered.
// WHERE: src/lib/snippets.ts (registry, built-ins), src/features/editor/SqlEditor.tsx
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Snippet {
    pub prefix: String,
    pub name: String,
    pub body: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AppSettings {
    /// Colour theme: "dark" (default), "light", or "system" (follows the OS).
    pub theme: String,
    pub accent: String,
    /// Font family key for the app chrome; `src/lib/fonts.ts` maps it to a stack.
    pub ui_font: String,
    /// Font family key for code, grids and values (monospace faces).
    pub editor_font: String,
    pub ui_font_size: u8,
    pub editor_font_size: u8,
    pub grid_density: String,
    /// Zebra striping in every grid.
    pub alternating_rows: bool,
    /// Reopen a table with the sort and filters it was last closed with.
    pub remember_table_state: bool,
    /// Expandable column list under each table in the sidebar.
    pub column_preview: bool,
    /// Ceiling on rows an editor query returns before the result is trimmed.
    pub max_query_rows: u32,
    pub null_display: String,
    pub show_results_pane: bool,
    pub condense_sql_when_formatting: bool,
    pub run_scope: RunScope,
    pub execution_mode: ExecutionMode,
    pub command_menu_sections: Vec<String>,
    pub inspector_tabs: Vec<String>,
    pub confirm_destructive: bool,
    pub crash_reports_opt_in: bool,
    pub ai: AiSettings,
    /// Settings → Advanced: where pg_dump, mysqldump, mongodump… live when they
    /// are not on PATH. A file, or the directory holding it. Absent = PATH.
    pub native_tool_paths: BTreeMap<crate::model::NativeTool, String>,
    /// Shortcut rebinds on top of the registry defaults (empty: every default).
    pub keybindings: Vec<KeyBinding>,
    /// The user's own editor snippets, on top of the built-in set.
    pub snippets: Vec<Snippet>,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            theme: "dark".to_string(),
            accent: "blue".to_string(),
            ui_font: "jetbrains-mono".to_string(),
            editor_font: "jetbrains-mono".to_string(),
            ui_font_size: 13,
            editor_font_size: 13,
            grid_density: "cozy".to_string(),
            alternating_rows: true,
            remember_table_state: true,
            column_preview: true,
            max_query_rows: 10_000,
            null_display: "NULL".to_string(),
            show_results_pane: true,
            condense_sql_when_formatting: false,
            run_scope: RunScope::All,
            execution_mode: ExecutionMode::Review,
            command_menu_sections: ["create", "navigation", "connections", "tables", "saved_queries", "dashboards", "workflows", "diagrams", "settings"]
                .into_iter()
                .map(String::from)
                .collect(),
            inspector_tabs: ["fields", "json", "sql"].into_iter().map(String::from).collect(),
            confirm_destructive: true,
            crash_reports_opt_in: false,
            ai: AiSettings::default(),
            native_tool_paths: BTreeMap::new(),
            keybindings: Vec::new(),
            snippets: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // WHAT:  Settings saved by an older build (no field added since) still load,
    //        with every new field at its default.
    #[test]
    fn legacy_settings_fill_new_fields_with_defaults() {
        let legacy = r#"{"accent":"green","uiFontSize":14}"#;
        let parsed: AppSettings = serde_json::from_str(legacy).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.accent, "green");
        assert_eq!(parsed.ui_font_size, 14);
        assert!(parsed.keybindings.is_empty());
        assert!(parsed.snippets.is_empty());
        assert_eq!(parsed.theme, "dark");
    }
}
