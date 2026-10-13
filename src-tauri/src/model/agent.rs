// SOT: agent-model, agent-events, agent-stream, agent-tool-call, agent-artifact, agent-permission, agent-skill

use crate::model::{AiProvider, DocumentKind, QueryOutcome, StatementIntent, WidgetKind};
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
    /// Label for the approve button when "Run once" would be wrong ("Create
    /// dashboard"). None keeps the statement wording.
    #[serde(default)]
    pub action_label: Option<String>,
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
    /// A dashboard, workflow or diagram the agent saved after the user approved
    /// it. The card offers to open it.
    #[serde(rename_all = "camelCase")]
    Document {
        id: String,
        title: String,
        document_kind: DocumentKind,
        document_id: String,
        connection_id: Option<String>,
        /// True when an existing document was replaced rather than a new one made.
        updated: bool,
        /// "5 widgets", "3 steps", "4 tables · 3 relations".
        summary: String,
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

// ---------------------------------------------------------------------------
// Providers, models and data access
// ---------------------------------------------------------------------------

// WHAT:  How much of the database the assistant may see.
// WHY:   "Use AI" and "send my rows to a third party" are different decisions.
//        Schema-only lets the assistant write queries from table and column
//        names without a single value leaving the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AiDataAccess {
    /// The assistant is switched off, whatever providers are configured.
    Off,
    /// Table and column names, types and relationships. No row ever leaves.
    Schema,
    /// The schema, plus the rows its read-only queries return.
    #[default]
    Full,
}

impl AiDataAccess {
    /// Whether the tools may read rows (sample_rows, queries, renderers).
    pub fn reads_rows(self) -> bool {
        self == AiDataAccess::Full
    }
}

/// One configured provider: its kind, where it lives, and whether a key is stored.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiProfile {
    pub id: String,
    pub provider: AiProvider,
    /// What the user calls it ("Work OpenAI", "Local Ollama").
    pub label: String,
    /// Override for proxies and self-hosted servers. None = the provider's own endpoint.
    pub base_url: Option<String>,
    /// Derived on read from the sealed key store; whatever the client sends is ignored.
    #[serde(default)]
    pub has_api_key: bool,
}

// WHAT:  A key to seal (or remove) alongside an AI settings save.
// WHY:   Secrets travel in one direction only: the UI sends them here, and
//        nothing sends them back. Debug is written by hand so a logged request
//        can never print one.
#[derive(Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiKeyChange {
    pub profile_id: String,
    /// Empty removes the stored key.
    pub key: String,
}

impl std::fmt::Debug for AiKeyChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiKeyChange").field("profile_id", &self.profile_id).field("key", &"***").finish()
    }
}

/// A model picked for one message: which profile speaks it, and its id.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelChoice {
    pub profile_id: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiModel {
    pub id: String,
    pub label: String,
}

/// A provider's chat models, as fetched (or as last cached).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiModelList {
    pub profile_id: String,
    pub models: Vec<AiModel>,
    /// RFC 3339 time the list was fetched from the provider.
    pub fetched_at: Option<String>,
    /// True when this came from the cache rather than the provider.
    pub cached: bool,
}

/// What "Test" found out.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiTestResult {
    pub ok: bool,
    /// One line for the UI: "Connected — 24 models" or why it failed.
    pub message: String,
    pub model_count: u32,
    pub elapsed_ms: u64,
}

// ---------------------------------------------------------------------------
// @ mentions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MentionKind {
    Table,
    SavedQuery,
    Dashboard,
    Workflow,
    Diagram,
}

// WHAT:  One thing the user pointed at with `@` in a message.
// WHY:   The id, not the label, is what the server resolves: a table is its
//        qualified name, everything else is a record id. The label is what the
//        chat shows back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatMention {
    pub kind: MentionKind,
    pub id: String,
    pub label: String,
}

// ---------------------------------------------------------------------------
// Saved chats
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AiChatRole {
    User,
    Assistant,
}

impl AiChatRole {
    pub fn as_str(self) -> &'static str {
        match self {
            AiChatRole::User => "user",
            AiChatRole::Assistant => "assistant",
        }
    }

    pub fn parse(raw: &str) -> Option<AiChatRole> {
        match raw {
            "user" => Some(AiChatRole::User),
            "assistant" => Some(AiChatRole::Assistant),
            _ => None,
        }
    }
}

/// One row of the history popover.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiChatSummary {
    pub id: String,
    pub connection_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub message_count: u32,
}

// WHAT:  One saved message. A user message carries its mentions; an assistant
//        message carries the whole turn (tool calls, artifacts, usage) so a
//        restored chat looks exactly as it did, charts included.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiChatMessage {
    pub id: String,
    pub role: AiChatRole,
    pub text: String,
    pub created_at: String,
    #[serde(default)]
    pub mentions: Vec<ChatMention>,
    #[serde(default)]
    pub turn: Option<AgentTurn>,
    /// Set on an assistant message whose run failed.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiChatDetail {
    pub chat: AiChatSummary,
    pub messages: Vec<AiChatMessage>,
}
