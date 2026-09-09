// SOT: agent-model, agent-events, agent-stream, agent-tool-call, agent-artifact, agent-permission, agent-skill

use crate::model::{QueryOutcome, StatementIntent, WidgetKind};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  Every shape the agent loop pushes across the IPC boundary.
// WHY:   The loop runs for many seconds over several tool calls, so the UI is fed
//        a stream of events rather than one reply. One tagged union keeps the
//        client's switch exhaustive: a new event variant breaks `tsc` until the
//        chat handles it.
// HOW:   Conversation state stays in the Rust session (src-tauri/src/state.rs), so
//        these types carry only what the UI renders — never provider wire blocks.
// WHERE: src-tauri/src/services/agent/ (producer), src/lib/ipc.ts (onAgentEvent)

/// How much the agent may do without asking. Stored per app, not per run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentAutonomy {
    /// Reads only. Any write or destructive statement is refused outright.
    ReadOnly,
    /// Reads run unattended; writes and destructive statements need a decision.
    /// The default: the agent is not the user, so a write waits for one.
    #[default]
    AskOnWrite,
    /// Writes run unattended; destructive statements still need a decision.
    Full,
}

/// What the user decided about one pending tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PermissionDecision {
    /// Run this call once.
    Allow,
    /// Run this call and stop asking for the same tool for the rest of the run.
    AllowForRun,
    /// Refuse; the model is told and continues without the result.
    Deny,
}

/// A tool call waiting on the user before it may run.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PermissionRequest {
    pub call_id: String,
    pub tool: String,
    /// One-line description of what the call would do.
    pub title: String,
    /// The statement itself when the tool runs one, so the user reads it before deciding.
    pub statement: Option<String>,
    /// Per-statement previews from the classifier, already annotated with why they are destructive.
    pub statements: Vec<String>,
    pub intent: StatementIntent,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ToolStatus {
    Running,
    Ok,
    Error,
    Denied,
}

/// One tool invocation, as the timeline renders it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ToolCallRecord {
    pub id: String,
    pub tool: String,
    /// Human summary of the call ("List tables in public").
    pub title: String,
    /// Pretty-printed arguments, shown when the row is expanded.
    pub input: String,
    pub status: ToolStatus,
    /// Result headline ("24 tables", "3 rows in 12 ms"). None while running.
    pub summary: Option<String>,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UiTone {
    Info,
    Success,
    Warning,
    Danger,
}

/// One figure in a KPI row.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UiStat {
    pub label: String,
    pub value: String,
    /// Small print under the figure — the denominator, the period, the caveat.
    pub hint: Option<String>,
    /// Percentage change, when the agent computed one. Drives the up/down tint.
    pub trend: Option<f64>,
}

/// A node in a rendered graph.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UiGraphNode {
    pub id: String,
    /// The type name, which decides the colour: a table, a label, a class.
    pub label: String,
    /// What is written on the node.
    pub caption: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UiGraphEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: String,
}

// WHAT:  One piece of a view the agent composed.
// WHY:   The agent needs to be able to build an answer that is a *layout* — a
//        row of figures over a chart over a table — not just one chart. This is
//        a closed vocabulary of blocks rather than markup or code, so the model
//        can only assemble components the app already ships, and nothing it
//        emits can execute in the webview.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "block", rename_all = "snake_case")]
#[ts(export)]
pub enum UiBlock {
    #[serde(rename_all = "camelCase")]
    Heading { text: String },
    #[serde(rename_all = "camelCase")]
    Text { markdown: String },
    /// A row of headline figures.
    #[serde(rename_all = "camelCase")]
    Stats { stats: Vec<UiStat> },
    #[serde(rename_all = "camelCase")]
    Chart {
        title: String,
        chart: WidgetKind,
        x_label: Option<String>,
        y_label: Option<String>,
        sql: String,
        outcome: QueryOutcome,
    },
    #[serde(rename_all = "camelCase")]
    Table { title: String, sql: String, outcome: QueryOutcome },
    #[serde(rename_all = "camelCase")]
    Graph {
        title: String,
        nodes: Vec<UiGraphNode>,
        edges: Vec<UiGraphEdge>,
    },
    /// Label/value pairs: a settings summary, a row inspected in detail.
    #[serde(rename_all = "camelCase")]
    Facts { title: String, rows: Vec<UiStat> },
    /// A highlighted note — the caveat, the warning, the thing that matters.
    #[serde(rename_all = "camelCase")]
    Callout { tone: UiTone, title: String, body: String },
    Divider,
}

/// A view the agent asked the UI to draw. Every artifact is a query plus a way to
/// render it, so the client can re-run it and switch the rendering on its own.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum AgentArtifact {
    #[serde(rename_all = "camelCase")]
    Chart {
        id: String,
        title: String,
        chart: WidgetKind,
        x_label: Option<String>,
        y_label: Option<String>,
        sql: String,
        outcome: QueryOutcome,
    },
    #[serde(rename_all = "camelCase")]
    Table {
        id: String,
        title: String,
        sql: String,
        outcome: QueryOutcome,
    },
    #[serde(rename_all = "camelCase")]
    Graph {
        id: String,
        title: String,
        sql: String,
        /// Built server-side (foreign keys, for engines with no graph query
        /// language). Empty means: derive them from `outcome` on the client,
        /// which already knows how to read a graph engine's result shape.
        nodes: Vec<UiGraphNode>,
        edges: Vec<UiGraphEdge>,
        outcome: Option<QueryOutcome>,
    },
    /// A layout the agent composed from the block vocabulary above.
    #[serde(rename_all = "camelCase")]
    Ui {
        id: String,
        title: String,
        blocks: Vec<UiBlock>,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Model round trips this turn took, including tool results.
    pub steps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AgentStop {
    EndTurn,
    /// Hit the step ceiling; the transcript is complete but the task may not be.
    MaxSteps,
    Cancelled,
    /// The provider's safety classifier declined.
    Refusal,
}

/// The finished turn. Also what `ai_generate` returns after collapsing to one step.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentTurn {
    pub run_id: String,
    pub model: String,
    /// The assistant's prose, as markdown.
    pub text: String,
    /// Last runnable statement the turn produced, for "open in editor".
    pub sql: Option<String>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub artifacts: Vec<AgentArtifact>,
    pub usage: AgentUsage,
    pub stop: AgentStop,
}

/// A prompt pack the model loads by name instead of carrying in every request.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    /// The one line that sits in context so the model knows when to load the rest.
    pub description: String,
}

// WHAT:  Everything the run emits, in order.
// WHY:   `Text` arrives token by token so the markdown grows live; the tool and
//        permission variants are what make the run legible while it works.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum AgentEvent {
    #[serde(rename_all = "camelCase")]
    Started {
        run_id: String,
        chat_id: String,
        model: String,
    },
    /// Summarized reasoning, when the provider returns it.
    #[serde(rename_all = "camelCase")]
    Thinking {
        run_id: String,
        delta: String,
    },
    #[serde(rename_all = "camelCase")]
    Text {
        run_id: String,
        delta: String,
    },
    #[serde(rename_all = "camelCase")]
    Step {
        run_id: String,
        step: u32,
    },
    #[serde(rename_all = "camelCase")]
    ToolStarted {
        run_id: String,
        call: ToolCallRecord,
    },
    #[serde(rename_all = "camelCase")]
    ToolFinished {
        run_id: String,
        call: ToolCallRecord,
    },
    #[serde(rename_all = "camelCase")]
    Artifact {
        run_id: String,
        artifact: AgentArtifact,
    },
    #[serde(rename_all = "camelCase")]
    Permission {
        run_id: String,
        request: PermissionRequest,
    },
    #[serde(rename_all = "camelCase")]
    Usage {
        run_id: String,
        usage: AgentUsage,
    },
    #[serde(rename_all = "camelCase")]
    Finished {
        run_id: String,
        turn: AgentTurn,
    },
    #[serde(rename_all = "camelCase")]
    Failed {
        run_id: String,
        message: String,
    },
}

impl AgentEvent {
    /// The run every event belongs to, so a stale listener can drop late frames.
    pub fn run_id(&self) -> &str {
        match self {
            AgentEvent::Started { run_id, .. }
            | AgentEvent::Thinking { run_id, .. }
            | AgentEvent::Text { run_id, .. }
            | AgentEvent::Step { run_id, .. }
            | AgentEvent::ToolStarted { run_id, .. }
            | AgentEvent::ToolFinished { run_id, .. }
            | AgentEvent::Artifact { run_id, .. }
            | AgentEvent::Permission { run_id, .. }
            | AgentEvent::Usage { run_id, .. }
            | AgentEvent::Finished { run_id, .. }
            | AgentEvent::Failed { run_id, .. } => run_id,
        }
    }
}
