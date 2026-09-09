// SOT: agent-commands, ipc-agent, agent-event-emit, agent-permission-reply

use crate::error::AppResult;
use crate::guard;
use crate::model::{AgentEvent, AgentSkill, AgentTurn, PermissionDecision};
use crate::services;
use crate::services::agent::{self, EventSink, RunOptions};
use crate::state::AppState;
use serde::Deserialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use ts_rs::TS;

// WHAT:  The IPC surface for the agent: start a turn, answer its permission
//        prompt, stop it, forget the conversation.
// WHY:   A turn runs for many seconds across several tool calls, so the reply is
//        a stream of events with one final value — not a single response. This
//        is also the only layer allowed to hold an `AppHandle`, because
//        `services/` may not import tauri.
// WHERE: src-tauri/src/services/agent/mod.rs (the loop), src/lib/ipc.ts (onAgentEvent)

/// Event the chat listens on for the whole life of a run.
const AGENT_EVENT: &str = "agent:event";

// WHAT:  Forwards loop progress to the window.
// WHY:   A dropped frame costs a repaint, never the run — same rule as the
//        updater's progress events.
struct WindowSink {
    app: AppHandle,
}

impl EventSink for WindowSink {
    fn emit(&self, event: AgentEvent) {
        let _ = self.app.emit(AGENT_EVENT, event);
    }
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentChatRequest {
    pub connection_id: String,
    /// Stable per chat surface, so a tab keeps its memory between messages.
    pub chat_id: String,
    /// Minted by the caller so it can filter events before the reply lands.
    pub run_id: String,
    pub prompt: String,
    /// What the user was looking at: the editor's statement, the open table.
    #[serde(default)]
    pub context: Option<String>,
    /// False for the editor's one-shot generate, which must not read the database.
    #[serde(default)]
    pub use_tools: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentDecisionRequest {
    pub run_id: String,
    pub decision: PermissionDecision,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentRunRequest {
    pub run_id: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentChatIdRequest {
    pub chat_id: String,
}

// WHAT:  Run one user message to completion.
// WHY:   Returns the finished turn so a caller that does not want to follow the
//        stream (the editor's generate button) still gets one value.
// HOW:   Enters the block through `guard::session`, so the connection is
//        resolved and the whole turn is bounded by the standard timeout — which
//        also bounds how long a permission prompt may sit unanswered.
#[tauri::command]
pub async fn agent_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    req: AgentChatRequest,
) -> AppResult<AgentTurn> {
    let settings = services::settings::get(&state)?;
    agent::ensure_configured(&settings.ai)?;
    let api_key = services::settings::ai_api_key(&state)?;

    let inner = state.inner();
    let chat = inner.agent_chat(&req.chat_id).await;
    let (control, inbox, cancel) = agent::control();
    inner.register_run(req.run_id.clone(), Arc::new(control)).await;

    let sink = WindowSink { app };
    let run_id = req.run_id.clone();

    let result = guard::session(&state, &req.connection_id, |ctx| async move {
        agent::run(
            &ctx,
            &chat,
            RunOptions {
                run_id: req.run_id.clone(),
                chat_id: req.chat_id.clone(),
                settings: &settings.ai,
                api_key: api_key.as_deref(),
                autonomy: settings.ai.autonomy,
                prompt: req.prompt.clone(),
                context: req.context.clone(),
                use_tools: req.use_tools,
            },
            &sink,
            inbox,
            cancel,
        )
        .await
        .inspect_err(|err| {
            // The stream is the UI's source of truth while a run is live; a
            // failure has to arrive there too, not only as a rejected promise.
            sink.emit(AgentEvent::Failed {
                run_id: req.run_id.clone(),
                message: err.message().to_string(),
            });
        })
    })
    .await;

    inner.finish_run(&run_id).await;
    result
}

// WHAT:  The user's answer to a permission prompt.
#[tauri::command]
pub async fn agent_decide(state: State<'_, AppState>, req: AgentDecisionRequest) -> AppResult<()> {
    guard::local("agent_decide", async move {
        if let Some(control) = state.run_control(&req.run_id).await {
            control.decide(req.decision);
        }
        // A run that already ended needs no answer; saying so would be noise.
        Ok(())
    })
    .await
}

/// Stop a running turn. The loop finishes the step it is on and reports `cancelled`.
#[tauri::command]
pub async fn agent_cancel(state: State<'_, AppState>, req: AgentRunRequest) -> AppResult<()> {
    guard::local("agent_cancel", async move {
        if let Some(control) = state.run_control(&req.run_id).await {
            control.cancel();
        }
        Ok(())
    })
    .await
}

/// Forget a conversation. "Clear chat" in the UI.
#[tauri::command]
pub async fn agent_reset(state: State<'_, AppState>, req: AgentChatIdRequest) -> AppResult<()> {
    guard::local("agent_reset", async move {
        state.agent_chat(&req.chat_id).await.reset().await;
        Ok(())
    })
    .await
}

/// The task guides the assistant can load, for the settings screen.
#[tauri::command]
pub async fn agent_skills() -> AppResult<Vec<AgentSkill>> {
    guard::local("agent_skills", async { Ok(services::agent::skills::catalogue()) }).await
}
