// SOT: agent-commands, ipc-agent, agent-event-emit, agent-permission-reply

use crate::error::AppResult;
use crate::guard;
use crate::model::{AgentEvent, AgentSkill, AgentTurn, ChatMention, ModelChoice, PermissionDecision};
use crate::services;
use crate::services::agent::{self, EventSink, RunOptions};
use crate::state::AppState;
use serde::Deserialize;
use std::sync::atomic::{AtomicU8, Ordering};
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
    /// False for the editor's one-shot generate, which must not read the
    /// database and is not saved as a chat.
    #[serde(default)]
    pub use_tools: bool,
    /// Things the user pointed at with `@`; resolved into the message here.
    #[serde(default)]
    #[ts(optional)]
    pub mentions: Option<Vec<ChatMention>>,
    /// The model this message uses; omitted = the default from Settings → AI.
    #[serde(default)]
    #[ts(optional)]
    pub model: Option<ModelChoice>,
    /// The table the caller is looking at; the one-shot path leads with it.
    #[serde(default)]
    #[ts(optional)]
    pub current_table: Option<String>,
    /// Carry on after a run stopped on its step limit: the chat's own transcript
    /// continues, nothing is asked and no question is saved. `prompt` is ignored.
    #[serde(default)]
    #[ts(optional)]
    pub resume: Option<bool>,
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

/// Where a persisted turn got to, so a run the block cuts short is still recorded.
const TURN_NOT_STARTED: u8 = 0;
const TURN_BEGUN: u8 = 1;
const TURN_FINISHED: u8 = 2;

// WHAT:  Run one user message to completion.
// WHY:   Returns the finished turn so a caller that does not want to follow the
//        stream (the editor's generate button) still gets one value.
// HOW:   Enters the block through `guard::session`, so the connection is
//        resolved and the whole turn is bounded by the standard timeout — which
//        also bounds how long a permission prompt may sit unanswered. A chat
//        (tools on) is saved around the run: the question first, then the
//        answer or the failure. The editor's one-shot is not a chat and leaves
//        nothing behind.
#[tauri::command]
pub async fn agent_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    req: AgentChatRequest,
) -> AppResult<AgentTurn> {
    let settings = services::settings::get(&state)?;
    let resolved = services::ai_profiles::resolve(&state, &settings.ai, req.model.as_ref())?;

    let inner = state.inner();
    let chat = inner.agent_chat_for(&req.chat_id, &req.connection_id).await;
    let (control, inbox, cancel) = agent::control();
    inner.register_run(req.run_id.clone(), Arc::new(control)).await;

    let sink = WindowSink { app };
    let run_id = req.run_id.clone();
    let chat_id = req.chat_id.clone();
    let persist = req.use_tools;
    let resume = persist && req.resume == Some(true);
    let progress = Arc::new(AtomicU8::new(TURN_NOT_STARTED));
    let run_progress = Arc::clone(&progress);
    let mentions = req.mentions.clone().unwrap_or_default();
    // The block takes the id by reference for the whole call, so the closure
    // below must not move the request's own copy out from under it.
    let connection_id = req.connection_id.clone();

    let result = guard::session(&state, &connection_id, |ctx| async move {
        let history = if persist {
            let earlier = if resume {
                services::ai_chats::resume_turn(inner, &req.chat_id)?
            } else {
                services::ai_chats::begin_turn(inner, &req.chat_id, &req.connection_id, &req.run_id, &req.prompt, &mentions)?
            };
            run_progress.store(TURN_BEGUN, Ordering::Relaxed);
            earlier
        } else {
            Vec::new()
        };

        let outcome = agent::run(
            &ctx,
            &chat,
            RunOptions {
                run_id: req.run_id.clone(),
                chat_id: req.chat_id.clone(),
                model: &resolved,
                autonomy: settings.ai.autonomy,
                data_access: settings.ai.data_access,
                state: inner,
                prompt: req.prompt.clone(),
                context: req.context.clone(),
                mentions: &mentions,
                history,
                current_table: req.current_table.clone(),
                use_tools: req.use_tools,
                max_steps: settings.ai.max_steps,
                resume,
            },
            &sink,
            inbox,
            cancel,
        )
        .await;

        if persist {
            if let Err(err) = services::ai_chats::finish_turn(inner, &req.chat_id, &req.run_id, outcome.as_ref()) {
                log::warn!("could not save the assistant's answer: {err}");
            }
            run_progress.store(TURN_FINISHED, Ordering::Relaxed);
        }
        outcome.inspect_err(|err| {
            // The stream is the UI's source of truth while a run is live; a
            // failure has to arrive there too, not only as a rejected promise.
            sink.emit(AgentEvent::Failed {
                run_id: req.run_id.clone(),
                message: err.message().to_string(),
            });
        })
    })
    .await;

    // The block can end a run (its timeout) after the question was saved and
    // before the loop got to save an answer; the failure is recorded here.
    if persist && progress.load(Ordering::Relaxed) == TURN_BEGUN {
        if let Err(err) = &result {
            if let Err(save_err) = services::ai_chats::finish_turn(inner, &chat_id, &run_id, Err(err)) {
                log::warn!("could not save the failed turn: {save_err}");
            }
        }
    }

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

/// Forget a conversation's in-memory transcript. The saved chat is untouched.
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
