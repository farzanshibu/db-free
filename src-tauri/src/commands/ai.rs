// SOT: ai-commands, ipc-ai, ipc-explain, ipc-ai-settings, ipc-ai-models, ipc-ai-chats, ipc-diagram-from-schema

use crate::commands::agent::AgentChatIdRequest;
use crate::error::AppResult;
use crate::guard;
use crate::model::{
    AiChatDetail, AiChatSummary, AiKeyChange, AiModelList, AiProfile, AiReply, AiSettings, AiTestResult, AppSettings,
    Document, ModelChoice, PlanReport,
};
use crate::services;
use crate::services::agent;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiGenerateRequest {
    pub connection_id: String,
    pub prompt: String,
    #[serde(default)]
    pub current_query: Option<String>,
    #[serde(default)]
    pub current_table: Option<String>,
    #[serde(default)]
    pub error_context: Option<String>,
    #[serde(default)]
    pub conversation_history: Option<Vec<ChatMessage>>,
    /// The model to use; omitted = the default from Settings → AI.
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<ModelChoice>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExplainRequest {
    pub connection_id: String,
    pub sql: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveAiSettingsRequest {
    pub ai: AiSettings,
    /// Keys to seal (or, when empty, remove). Never echoed back.
    pub keys: Vec<AiKeyChange>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiModelsRequest {
    pub profile: AiProfile,
    /// A key typed into the form and not saved yet; wins over the stored one.
    #[serde(default)]
    #[ts(optional)]
    pub api_key: Option<String>,
    /// True for the Refresh button: ask the provider even if a list is cached.
    pub refresh: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiTestRequest {
    pub profile: AiProfile,
    #[serde(default)]
    #[ts(optional)]
    pub api_key: Option<String>,
    /// The model the user plans to use; a typo is pointed out.
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiChatsRequest {
    pub connection_id: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiChatRenameRequest {
    pub chat_id: String,
    pub title: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramFromSchemaRequest {
    pub connection_id: String,
    /// What to call the saved diagram.
    pub name: String,
    /// Tables to include (bare or schema-qualified); empty = the whole database.
    pub tables: Vec<String>,
}

// WHAT:  One-shot natural language to a statement.
// WHY:   Kept as its own command for callers that want a value rather than a
//        stream, but it is no longer a second assistant: it runs the same agent
//        for a single turn with tools switched off, so there is one prompt, one
//        provider path and one set of dialect rules to maintain. The schema
//        rides in the prompt, since there are no tools to look it up with.
#[tauri::command]
pub async fn ai_generate(state: State<'_, AppState>, req: AiGenerateRequest) -> AppResult<AiReply> {
    let settings = services::settings::get(&state)?;
    let resolved = services::ai_profiles::resolve(&state, &settings.ai, req.model.as_ref())?;
    let inner = state.inner();

    // No chat id: this turn keeps no memory of its own.
    let chat = agent::AgentChat::default();
    let (_control, inbox, cancel) = agent::control();
    let context = build_context(&req);
    let connection_id = req.connection_id.clone();

    guard::session(&state, &connection_id, |ctx| async move {
        let turn = agent::run(
            &ctx,
            &chat,
            agent::RunOptions {
                run_id: format!("oneshot-{}", ctx.elapsed_ms()),
                chat_id: String::new(),
                model: &resolved,
                autonomy: settings.ai.autonomy,
                data_access: settings.ai.data_access,
                state: inner,
                prompt: req.prompt.clone(),
                context,
                mentions: &[],
                history: Vec::new(),
                current_table: req.current_table.clone(),
                use_tools: false,
                max_steps: settings.ai.max_steps,
                resume: false,
            },
            &agent::NullSink,
            inbox,
            cancel,
        )
        .await?;
        Ok(AiReply { sql: turn.sql, text: turn.text, model: turn.model })
    })
    .await
}

/// Fold what the caller was looking at into one preamble.
fn build_context(req: &AiGenerateRequest) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(table) = req.current_table.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        parts.push(format!("Active table: {table}"));
    }
    if let Some(query) = req.current_query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        parts.push(format!("Current editor statement:\n{query}"));
    }
    if let Some(error) = req.error_context.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
        parts.push(format!("The last run failed with:\n{error}"));
    }
    if let Some(history) = req.conversation_history.as_deref() {
        for message in history.iter().rev().take(4).rev() {
            parts.push(format!("{}: {}", message.role, message.content));
        }
    }
    if parts.is_empty() { None } else { Some(parts.join("\n\n")) }
}

#[tauri::command]
pub async fn explain_query(state: State<'_, AppState>, req: ExplainRequest) -> AppResult<PlanReport> {
    let settings = services::settings::get(&state)?;
    let api_key = services::ai_profiles::default_key(&state, &settings.ai)?;
    guard::session(&state, &req.connection_id, |ctx| async move {
        services::ai::explain(&ctx, &services::ai::AiRequest { settings: &settings.ai, api_key: api_key.as_deref() }, &req.sql, 500).await
    })
    .await
}

// ---------------------------------------------------------------------------
// Settings → AI
// ---------------------------------------------------------------------------

// WHAT:  Save the assistant's settings and seal or remove keys, atomically from
//        the UI's point of view; returns the whole hydrated settings document.
// WHY:   Keys travel one way only. The general settings save never touches the
//        assistant's settings (see `services::ai_profiles::keep_stored`).
#[tauri::command]
pub async fn ai_settings_save(state: State<'_, AppState>, req: SaveAiSettingsRequest) -> AppResult<AppSettings> {
    guard::local("ai_settings_save", async { services::ai_profiles::save(&state, req.ai, &req.keys) }).await
}

/// A provider's chat models: the cache, or a fresh list when asked.
#[tauri::command]
pub async fn ai_models(state: State<'_, AppState>, req: AiModelsRequest) -> AppResult<AiModelList> {
    guard::local("ai_models", services::ai_profiles::models(&state, &req.profile, req.api_key.as_deref(), req.refresh)).await
}

/// "Test": reach the provider with this key within five seconds.
#[tauri::command]
pub async fn ai_test(state: State<'_, AppState>, req: AiTestRequest) -> AppResult<AiTestResult> {
    guard::local(
        "ai_test",
        services::ai_profiles::test(&state, &req.profile, req.api_key.as_deref(), req.model.as_deref()),
    )
    .await
}

// ---------------------------------------------------------------------------
// Saved chats
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn ai_chat_list(state: State<'_, AppState>, req: AiChatsRequest) -> AppResult<Vec<AiChatSummary>> {
    guard::local("ai_chat_list", async { services::ai_chats::list(&state, &req.connection_id) }).await
}

#[tauri::command]
pub async fn ai_chat_load(state: State<'_, AppState>, req: AgentChatIdRequest) -> AppResult<AiChatDetail> {
    guard::local("ai_chat_load", async { services::ai_chats::load(&state, &req.chat_id) }).await
}

#[tauri::command]
pub async fn ai_chat_rename(state: State<'_, AppState>, req: AiChatRenameRequest) -> AppResult<AiChatSummary> {
    guard::local("ai_chat_rename", async { services::ai_chats::rename(&state, &req.chat_id, &req.title) }).await
}

// WHAT:  Delete a saved chat and the assistant's memory of it.
#[tauri::command]
pub async fn ai_chat_delete(state: State<'_, AppState>, req: AgentChatIdRequest) -> AppResult<()> {
    guard::local("ai_chat_delete", async {
        services::ai_chats::delete(&state, &req.chat_id)?;
        state.agent_chat(&req.chat_id).await.reset().await;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Open a relationship graph in the designer
// ---------------------------------------------------------------------------

// WHAT:  Save the schema's tables, columns and foreign keys as a diagram document.
// WHY:   A relationship graph in the chat knows table names only; the designer
//        needs columns. The UI opens the document this returns.
#[tauri::command]
pub async fn ai_diagram_from_schema(state: State<'_, AppState>, req: DiagramFromSchemaRequest) -> AppResult<Document> {
    let inner = state.inner();
    let connection_id = req.connection_id.clone();
    guard::session(&state, &connection_id, |ctx| async move {
        let body = agent::documents::diagram_from_catalog(&ctx, &req.tables).await?;
        agent::documents::save_diagram(inner, &ctx, &req.name, body)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AiGenerateRequest {
        AiGenerateRequest {
            connection_id: "c".into(),
            prompt: "p".into(),
            current_query: None,
            current_table: None,
            error_context: None,
            conversation_history: None,
            model: None,
        }
    }

    #[test]
    fn nothing_to_say_is_no_context() {
        assert_eq!(build_context(&request()), None);
        let blanks = AiGenerateRequest { current_table: Some("  ".into()), current_query: Some("\n".into()), ..request() };
        assert_eq!(build_context(&blanks), None, "whitespace is not context");
    }

    #[test]
    fn what_the_caller_was_looking_at_is_folded_into_one_preamble() {
        let req = AiGenerateRequest {
            current_table: Some("orders".into()),
            current_query: Some("SELECT 1".into()),
            error_context: Some("syntax error".into()),
            conversation_history: Some(vec![
                ChatMessage { role: "user".into(), content: "a".into() },
                ChatMessage { role: "assistant".into(), content: "b".into() },
            ]),
            ..request()
        };
        let context = build_context(&req).unwrap_or_default();
        assert!(context.starts_with("Active table: orders"), "{context}");
        assert!(context.contains("Current editor statement:\nSELECT 1"));
        assert!(context.contains("The last run failed with:\nsyntax error"));
        assert!(context.ends_with("assistant: b"), "the latest exchange comes last");
    }

    #[test]
    fn only_the_last_four_messages_of_history_are_kept() {
        let history = (0..10).map(|i| ChatMessage { role: "user".into(), content: format!("m{i}") }).collect();
        let context = build_context(&AiGenerateRequest { conversation_history: Some(history), ..request() }).unwrap_or_default();
        assert!(!context.contains("m5") && context.contains("m6") && context.contains("m9"), "{context}");
    }
}
