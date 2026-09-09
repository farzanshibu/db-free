// SOT: agent-loop, agent-runtime, tool-dispatch, agent-hooks, agent-session, agent-cancel

pub mod provider;
pub mod skills;
pub mod tools;

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::model::{
    AgentArtifact, AgentAutonomy, AgentEvent, AgentStop, AgentTurn, AgentUsage, AiSettings,
    PermissionDecision, PermissionRequest, ToolCallRecord, ToolStatus,
};
use provider::{Chat, ModelTurn, StreamDelta, ToolOutput, Transcript};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, mpsc};
use tools::{PermissionGate, ToolCtx};

// WHAT:  The agent run loop: ask the model, run what it asks for, feed the
//        results back, repeat until it stops asking.
// WHY:   rig gives us one provider and one message shape, but the loop has to be
//        ours — it is where a write suspends to wait for a human, where every
//        statement goes through the guard, and where each step is streamed to
//        the UI as it happens.
// HOW:   Hooks are traits, not callbacks buried in the loop: `EventSink` carries
//        progress out, `PermissionGate` carries a decision in. Both are trivial
//        to stub, so the loop is testable without a window or a provider.
// WHERE: src-tauri/src/commands/agent.rs (owns the AppHandle and emits),
//        src-tauri/src/services/agent/tools.rs (what the model may call)

/// Model round trips one user message may take. A wrong-headed run stops here
/// rather than looping on a failing tool until the token budget is gone.
pub const MAX_STEPS: u32 = 12;
/// Messages kept in a chat before the oldest exchanges are dropped.
const MAX_TRANSCRIPT: usize = 60;

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

// WHAT:  Where a run's progress goes.
// WHY:   `services/` may not import tauri (the guardrail enforces it), so the
//        loop cannot emit a window event itself. It pushes into this instead and
//        the command layer forwards.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}

/// Drops everything. Used by the one-shot path and by tests.
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: AgentEvent) {}
}

// ---------------------------------------------------------------------------
// Live chats and running turns
// ---------------------------------------------------------------------------

// WHAT:  One conversation's memory, held for as long as the tab is open.
// WHY:   Keeping the transcript here rather than shipping it from the UI on
//        every message means the stable prefix — system prompt, tool list,
//        earlier turns — is byte-identical between requests, which is the
//        precondition for provider-side prompt caching to hit at all.
pub struct AgentChat {
    transcript: Mutex<Transcript>,
}

impl Default for AgentChat {
    fn default() -> Self {
        AgentChat { transcript: Mutex::new(Transcript::new()) }
    }
}

impl AgentChat {
    pub async fn reset(&self) {
        self.transcript.lock().await.reset();
    }

    pub async fn is_empty(&self) -> bool {
        self.transcript.lock().await.is_empty()
    }
}

/// The handle the UI reaches a *running* turn through, to answer a permission
/// prompt or to stop it.
pub struct RunControl {
    cancel: Arc<AtomicBool>,
    decisions: mpsc::UnboundedSender<PermissionDecision>,
}

impl RunControl {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        // Unblock a gate that is waiting, so the run winds up instead of hanging.
        let _ = self.decisions.send(PermissionDecision::Deny);
    }

    pub fn decide(&self, decision: PermissionDecision) {
        let _ = self.decisions.send(decision);
    }
}

// ---------------------------------------------------------------------------
// The permission gate
// ---------------------------------------------------------------------------

// WHAT:  Suspends the run, shows the user what is about to happen, waits.
// WHY:   The agent is not the user. A statement that changes data needs a human
//        who has read it. "Allow for this run" exists because approving thirty
//        identical inserts one at a time is how people learn to click Allow
//        without reading.
struct ChannelGate<'a> {
    run_id: String,
    sink: &'a dyn EventSink,
    inbox: mpsc::UnboundedReceiver<PermissionDecision>,
    cancel: Arc<AtomicBool>,
    /// Tools the user waved through for the rest of this run.
    blanket: HashSet<String>,
    call_id: String,
}

impl PermissionGate for ChannelGate<'_> {
    fn ask(
        &mut self,
        request: PermissionRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = PermissionDecision> + Send + '_>> {
        Box::pin(async move {
            if self.cancel.load(Ordering::Relaxed) {
                return PermissionDecision::Deny;
            }
            if self.blanket.contains(&request.tool) {
                return PermissionDecision::Allow;
            }
            let tool = request.tool.clone();
            let request = PermissionRequest { call_id: self.call_id.clone(), ..request };
            self.sink.emit(AgentEvent::Permission { run_id: self.run_id.clone(), request });

            // A closed channel means the window went away: refuse rather than wait.
            let decision = self.inbox.recv().await.unwrap_or(PermissionDecision::Deny);
            if decision == PermissionDecision::AllowForRun {
                self.blanket.insert(tool);
            }
            decision
        })
    }
}

// ---------------------------------------------------------------------------
// Running a turn
// ---------------------------------------------------------------------------

pub struct RunOptions<'a> {
    pub run_id: String,
    pub chat_id: String,
    pub settings: &'a AiSettings,
    pub api_key: Option<&'a str>,
    pub autonomy: AgentAutonomy,
    /// What the user typed.
    pub prompt: String,
    /// What they were looking at: the editor's query, the open table.
    pub context: Option<String>,
    /// False for the one-shot NL→SQL path, which must not touch the database.
    pub use_tools: bool,
}

/// Build the control handle for a run, and the receiver the gate waits on.
pub fn control() -> (RunControl, mpsc::UnboundedReceiver<PermissionDecision>, Arc<AtomicBool>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let cancel = Arc::new(AtomicBool::new(false));
    (RunControl { cancel: Arc::clone(&cancel), decisions: tx }, rx, cancel)
}

// WHAT:  One user message, start to finish.
// WHY:   Everything the UI shows about a run — streaming prose, the tool
//        timeline, permission prompts, charts — is produced here in order.
pub async fn run(
    ctx: &SessionCtx,
    chat: &AgentChat,
    options: RunOptions<'_>,
    sink: &dyn EventSink,
    inbox: mpsc::UnboundedReceiver<PermissionDecision>,
    cancel: Arc<AtomicBool>,
) -> AppResult<AgentTurn> {
    let model = Chat::build(options.settings, options.api_key)?;
    let run_id = options.run_id.clone();

    sink.emit(AgentEvent::Started {
        run_id: run_id.clone(),
        chat_id: options.chat_id.clone(),
        model: options.settings.model.clone(),
    });

    let specs = if options.use_tools { tools::definitions(ctx) } else { Vec::new() };
    let system = system_prompt(ctx, options.use_tools);

    let mut transcript = chat.transcript.lock().await;
    transcript.trim_to(MAX_TRANSCRIPT);
    transcript.push_user(&user_message(&options));

    let mut gate = ChannelGate {
        run_id: run_id.clone(),
        sink,
        inbox,
        cancel: Arc::clone(&cancel),
        blanket: HashSet::new(),
        call_id: String::new(),
    };

    let mut prose = String::new();
    let mut records: Vec<ToolCallRecord> = Vec::new();
    let mut artifacts: Vec<AgentArtifact> = Vec::new();
    let mut last_sql: Option<String> = None;
    let mut usage = AgentUsage::default();
    let mut stop = AgentStop::MaxSteps;

    for step in 1..=MAX_STEPS {
        if cancel.load(Ordering::Relaxed) {
            stop = AgentStop::Cancelled;
            break;
        }
        usage.steps = step;
        sink.emit(AgentEvent::Step { run_id: run_id.clone(), step });

        let turn: ModelTurn = {
            let mut on_delta = |delta: StreamDelta<'_>| match delta {
                StreamDelta::Text(text) => {
                    sink.emit(AgentEvent::Text { run_id: run_id.clone(), delta: text.to_string() });
                }
                StreamDelta::Reasoning(text) => {
                    sink.emit(AgentEvent::Thinking { run_id: run_id.clone(), delta: text.to_string() });
                }
            };
            transcript.run_turn(&model, &system, &specs, &mut on_delta).await?
        };

        prose.push_str(&turn.text);
        usage.input_tokens += turn.usage.input_tokens;
        usage.output_tokens += turn.usage.output_tokens;
        sink.emit(AgentEvent::Usage { run_id: run_id.clone(), usage });

        if !turn.wants_tools() {
            stop = AgentStop::EndTurn;
            break;
        }

        let mut outputs: Vec<ToolOutput> = Vec::new();
        for call in turn.calls {
            if cancel.load(Ordering::Relaxed) {
                stop = AgentStop::Cancelled;
                break;
            }
            let started = std::time::Instant::now();
            let title = tools::title_for(&call.name, &call.arguments);
            let mut record = ToolCallRecord {
                id: call.ui_id.clone(),
                tool: call.name.clone(),
                title,
                input: serde_json::to_string_pretty(&call.arguments).unwrap_or_default(),
                status: ToolStatus::Running,
                summary: None,
                error: None,
                elapsed_ms: 0,
            };
            sink.emit(AgentEvent::ToolStarted { run_id: run_id.clone(), call: record.clone() });

            gate.call_id = call.ui_id.clone();
            let tool_ctx = ToolCtx {
                session: ctx,
                autonomy: options.autonomy,
                call_id: call.ui_id.clone(),
            };
            let result = tools::dispatch(&tool_ctx, &call.name, &call.arguments, &mut gate).await;

            record.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            if result.is_error {
                // A denial is the user's answer, not a malfunction; it reads
                // differently in the timeline.
                record.status = if result.content.contains("declined") {
                    ToolStatus::Denied
                } else {
                    ToolStatus::Error
                };
                record.error = Some(result.content.clone());
            } else {
                record.status = ToolStatus::Ok;
                record.summary = Some(result.summary.clone());
            }
            sink.emit(AgentEvent::ToolFinished { run_id: run_id.clone(), call: record.clone() });
            records.push(record);

            if let Some(artifact) = result.artifact.clone() {
                sink.emit(AgentEvent::Artifact { run_id: run_id.clone(), artifact: artifact.clone() });
                artifacts.push(artifact);
            }
            if let Some(sql) = &result.sql {
                last_sql = Some(sql.clone());
            }

            outputs.push(ToolOutput {
                call,
                content: result.content,
                is_error: result.is_error,
            });
        }

        if matches!(stop, AgentStop::Cancelled) {
            break;
        }
        transcript.push_tool_outputs(outputs);
    }

    // Prefer a statement the model actually ran over one it merely wrote about.
    let sql = last_sql.or_else(|| crate::services::ai::extract_fence(&prose));

    let turn = AgentTurn {
        run_id: run_id.clone(),
        model: options.settings.model.clone(),
        text: prose,
        sql,
        tool_calls: records,
        artifacts,
        usage,
        stop,
    };
    sink.emit(AgentEvent::Finished { run_id, turn: turn.clone() });
    Ok(turn)
}

// ---------------------------------------------------------------------------
// Prompting
// ---------------------------------------------------------------------------

// WHAT:  The system prompt. Deliberately small and, crucially, constant for a
//        given connection.
// WHY:   It used to carry the whole schema, which made it enormous, different on
//        every message, and impossible to cache. Everything variable now arrives
//        through tools instead, so this text is a stable cacheable prefix.
fn system_prompt(ctx: &SessionCtx, use_tools: bool) -> String {
    let engine = ctx.connection.engine;
    let mut out = format!(
        "You are the database assistant built into DB Free, a native database workbench. You are \
         working against a live {} connection called \"{}\".\n\n",
        engine.label(),
        ctx.connection.name
    );

    if let Some(database) = ctx
        .integration
        .current_database()
        .or_else(|| ctx.connection.database.clone())
    {
        out.push_str(&format!("Database in use: {database}\n"));
    }
    if ctx.connection.read_only {
        out.push_str(
            "This connection is READ-ONLY. Statements that change data will be refused, so do not \
             offer to run them — describe them instead.\n",
        );
    }
    out.push('\n');
    out.push_str(crate::services::ai::engine_guidelines(engine));
    out.push_str("\n\n");

    if use_tools {
        out.push_str(
            "How to work:\n\
             - You have tools that read this database. Use them. Never invent a table or column \
               name, and never answer a question about the data from memory.\n\
             - Work narrow to wide: search_schema or list_tables to find what exists, then \
               describe_table on the few tables you actually need. Do not describe every table.\n\
             - Prefer running a query over describing one — the user can see the result.\n\
             - When the answer is a trend, a comparison or a breakdown, call render_chart rather \
               than pasting a column of numbers. When the user wants the rows themselves, call \
               render_table.\n\
             - When the question is about how things connect — which tables relate, what joins to \
               what — call render_graph. A picture of the relationships beats a list of foreign \
               keys every time.\n\
             - When the answer deserves a small report rather than a paragraph — a summary of a \
               table, a health check, a comparison, an audit — call render_ui and compose it: a \
               row of headline figures, then a chart or a table, then a callout for the thing that \
               matters. Do not then repeat every number back in prose; say what it shows.\n\
             - Reads run immediately. Anything that writes or deletes is shown to the user for \
               approval first, so say what you intend before you call it, and never batch an \
               unrelated write into a read.\n\
             - If a tool fails, read the error and try a different approach; do not repeat the \
               same call.\n\n\
             Answering:\n\
             - Be brief. The user is looking at a database, not reading an essay. Lead with the \
               answer, then the evidence.\n\
             - Use markdown: fenced code blocks for statements, tables for small result sets, \
               bold for the figure that matters.\n\
             - Put any statement the user might want to keep in its own fenced code block.\n\
             - Say plainly when something is an estimate, an inference from column names, or a \
               guess you could not verify.\n\n",
        );
        out.push_str(&skills::index_text());
    } else {
        out.push_str(
            "Write a single statement that answers the request, in one fenced code block, with a \
             short explanation beneath it. You have no tools and cannot inspect the database, so \
             work only from what the user gives you and say so if it is not enough.\n",
        );
    }
    out
}

fn user_message(options: &RunOptions<'_>) -> String {
    match &options.context {
        Some(context) if !context.trim().is_empty() => {
            format!("{}\n\n{}", context.trim(), options.prompt.trim())
        }
        _ => options.prompt.trim().to_string(),
    }
}

/// Guard against a run that asks for a provider that was never configured.
pub fn ensure_configured(settings: &AiSettings) -> AppResult<()> {
    if settings.provider == crate::model::AiProvider::None {
        return Err(AppError::invalid_input(
            "Choose an AI provider in Settings → AI first.".to_string(),
        ));
    }
    Ok(())
}
