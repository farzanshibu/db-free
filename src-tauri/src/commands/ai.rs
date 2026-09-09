// SOT: ai-commands, ipc-ai, ipc-explain

use crate::error::AppResult;
use crate::guard;
use crate::model::{AiReply, PlanReport};
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
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExplainRequest {
    pub connection_id: String,
    pub sql: String,
}

// WHAT:  One-shot natural language to a statement.
// WHY:   Kept as its own command for callers that want a value rather than a
//        stream, but it is no longer a second assistant: it runs the same agent
//        for a single turn with tools switched off, so there is one prompt, one
//        provider path and one set of dialect rules to maintain.
#[tauri::command]
pub async fn ai_generate(state: State<'_, AppState>, req: AiGenerateRequest) -> AppResult<AiReply> {
    let settings = services::settings::get(&state)?;
    agent::ensure_configured(&settings.ai)?;
    let api_key = services::settings::ai_api_key(&state)?;

    // No chat id: this turn keeps no memory of its own.
    let chat = agent::AgentChat::default();
    let (_control, inbox, cancel) = agent::control();
    let context = build_context(&req);

    guard::session(&state, &req.connection_id, |ctx| async move {
        let turn = agent::run(
            &ctx,
            &chat,
            agent::RunOptions {
                run_id: format!("oneshot-{}", ctx.elapsed_ms()),
                chat_id: String::new(),
                settings: &settings.ai,
                api_key: api_key.as_deref(),
                autonomy: settings.ai.autonomy,
                prompt: req.prompt.clone(),
                context,
                use_tools: false,
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
    let api_key = services::settings::ai_api_key(&state)?;
    guard::session(&state, &req.connection_id, |ctx| async move {
        services::ai::explain(&ctx, &services::ai::AiRequest { settings: &settings.ai, api_key: api_key.as_deref() }, &req.sql, 500).await
    })
    .await
}
