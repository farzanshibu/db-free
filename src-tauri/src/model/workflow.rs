// SOT: workflow-graph-model, workflow-node-config, workflow-run-events, workflow-schedule-config, workflow-secret-field, workflow-rule-group

use crate::model::documents::{WorkflowBody, WorkflowStep};
use crate::model::{ColumnMeta, Value};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  Reading a workflow body always yields the graph form.
// WHY:   Older documents hold a linear `steps` list. Upgrading while reading means
//        every path that loads a document (the list, a rename, a duplicate, a
//        restored version) hands the editor and the engine a graph, with no caller
//        having to remember to convert.
impl<'de> Deserialize<'de> for WorkflowBody {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<WorkflowBody, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            #[serde(default)]
            steps: Vec<WorkflowStep>,
            #[serde(default)]
            nodes: Vec<WorkflowNode>,
            #[serde(default)]
            edges: Vec<WorkflowEdge>,
            #[serde(default)]
            schedules_active: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(WorkflowBody { steps: wire.steps, nodes: wire.nodes, edges: wire.edges, schedules_active: wire.schedules_active }.upgraded())
    }
}

impl WorkflowBody {
    // WHAT:  Forgets every sealed secret the nodes hold; answers how many there were.
    // WHY:   After the master key is lost for good they can never be opened again;
    //        clearing them lets the user type the passwords under a fresh key.
    pub fn forget_secrets(&mut self) -> usize {
        let mut forgotten = 0;
        for node in &mut self.nodes {
            let secret = match &mut node.config {
                NodeConfig::Email(email) => &mut email.password,
                NodeConfig::Slack(slack) => &mut slack.webhook,
                _ => continue,
            };
            if secret.sealed.take().is_some() {
                forgotten += 1;
            }
            secret.has_secret = false;
        }
        forgotten
    }

    // WHAT:  The graph form of this body: a legacy `steps` list becomes a chain of
    //        Query nodes left to right, then the list is emptied.
    // WHY:   The editor and the engine only understand nodes and edges. Old
    //        documents keep working, and the first save writes the graph.
    // HOW:   A step that did not stop on error becomes a node that continues on
    //        error. `allow_destructive` stays off: the old runner confirmed every
    //        destructive statement on the user's behalf (bug B4), so those nodes
    //        now ask for an explicit opt-in.
    pub fn upgraded(mut self) -> WorkflowBody {
        for edge in &mut self.edges {
            if edge.id.trim().is_empty() {
                let handle = edge.source_handle.map(|h| if h == SourceHandle::True { "true" } else { "false" });
                edge.id = format!("e-{}-{}-{}", edge.source, handle.unwrap_or("out"), edge.target);
            }
        }
        if self.steps.is_empty() || !self.nodes.is_empty() {
            self.steps.clear();
            return self;
        }
        let steps: Vec<WorkflowStep> = std::mem::take(&mut self.steps);
        let mut previous: Option<String> = None;
        for (index, step) in steps.into_iter().enumerate() {
            let id = if step.id.trim().is_empty() { format!("step-{}", index + 1) } else { step.id.clone() };
            self.nodes.push(WorkflowNode {
                id: id.clone(),
                x: 80.0 + 320.0 * index as f64,
                y: 120.0,
                label: step.name,
                continue_on_error: !step.stop_on_error,
                config: NodeConfig::Query(QueryConfig { connection_id: step.connection_id, sql: step.sql, allow_destructive: false }),
            });
            if let Some(source) = previous.replace(id.clone()) {
                self.edges.push(WorkflowEdge { id: format!("e-{source}-{id}"), source, target: id, source_handle: None });
            }
        }
        self
    }
}

// ============================================================================
// WORKFLOW GRAPH
//
// WHAT:  The body of a workflow document: nodes on a canvas joined by edges.
//        Each node carries a typed config (`NodeConfig`, tagged by `kind`).
// WHY:   Automation is a graph, not a list: a query feeds a filter that feeds an
//        email, a branch sends rows two ways, a repeat runs a chain N times.
//        Typed configs let the engine and the canvas agree on every field and
//        make an unknown node kind a deserialisation error instead of a silent no-op.
// HOW:   Every config struct is `#[serde(default)]` so a document written by an
//        older build (or by the AI) with a missing field still loads.
// WHERE: src-tauri/src/services/workflows (engine), src/features/workflows (canvas)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowNode {
    pub id: String,
    /// Canvas position. A workflow written without positions (by the AI assistant)
    /// has every node at 0, 0 and the editor lays it out on open.
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    /// Title on the canvas; blank = the kind's own name.
    #[serde(default)]
    pub label: String,
    /// Keep the run going when this node fails; it then hands on an empty result.
    #[serde(default)]
    pub continue_on_error: bool,
    pub config: NodeConfig,
}

/// Which output of the source node an edge leaves from: only a Branch has two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SourceHandle {
    True,
    False,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowEdge {
    /// Blank ids are filled in when a body is read.
    #[serde(default)]
    pub id: String,
    pub source: String,
    pub target: String,
    /// `None` = the node's only output.
    #[serde(default)]
    pub source_handle: Option<SourceHandle>,
}

// WHAT:  What a node does and how it is set up, one variant per node kind.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum NodeConfig {
    Schedule(ScheduleConfig),
    Query(QueryConfig),
    Http(HttpConfig),
    ReadFile(ReadFileConfig),
    SaveFile(SaveFileConfig),
    Insert(InsertConfig),
    Filter(FilterConfig),
    RowCountGate(GateConfig),
    Branch(BranchConfig),
    Repeat(RepeatConfig),
    Delay(DelayConfig),
    Regex(RegexConfig),
    Javascript(JavascriptConfig),
    Notification(NotificationConfig),
    Email(EmailConfig),
    Slack(SlackConfig),
    Note(NoteConfig),
}

impl NodeConfig {
    /// The `kind` tag as the wire spells it (also the label used in errors).
    pub fn kind_name(&self) -> &'static str {
        match self {
            NodeConfig::Schedule(_) => "schedule",
            NodeConfig::Query(_) => "query",
            NodeConfig::Http(_) => "http",
            NodeConfig::ReadFile(_) => "read_file",
            NodeConfig::SaveFile(_) => "save_file",
            NodeConfig::Insert(_) => "insert",
            NodeConfig::Filter(_) => "filter",
            NodeConfig::RowCountGate(_) => "row_count_gate",
            NodeConfig::Branch(_) => "branch",
            NodeConfig::Repeat(_) => "repeat",
            NodeConfig::Delay(_) => "delay",
            NodeConfig::Regex(_) => "regex",
            NodeConfig::Javascript(_) => "javascript",
            NodeConfig::Notification(_) => "notification",
            NodeConfig::Email(_) => "email",
            NodeConfig::Slack(_) => "slack",
            NodeConfig::Note(_) => "note",
        }
    }
}

// ---------------------------------------------------------------- schedule
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ScheduleMode {
    /// Every N seconds / minutes / hours / days.
    #[default]
    Interval,
    /// At a clock time, on chosen weekdays, in a chosen time zone.
    AtTime,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum IntervalUnit {
    Seconds,
    #[default]
    Minutes,
    Hours,
    Days,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ScheduleConfig {
    pub mode: ScheduleMode,
    pub interval_amount: u32,
    pub interval_unit: IntervalUnit,
    /// `HH:MM`, 24-hour, for `AtTime`.
    pub time: String,
    pub days: Vec<Weekday>,
    /// IANA name ("Europe/Paris"); blank = the machine's own zone.
    pub timezone: String,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        ScheduleConfig {
            mode: ScheduleMode::Interval,
            interval_amount: 5,
            interval_unit: IntervalUnit::Minutes,
            time: "09:00".to_string(),
            days: vec![Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri],
            timezone: String::new(),
        }
    }
}

// ---------------------------------------------------------------- data sources
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct QueryConfig {
    /// `None` = the workflow's own connection.
    pub connection_id: Option<String>,
    pub sql: String,
    /// The user's saved opt-in to run destructive statements (DROP, TRUNCATE,
    /// DELETE/UPDATE without WHERE …) in this node. Without it the guard refuses them.
    pub allow_destructive: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HttpMethod {
    #[default]
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
}

impl HttpMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Patch => "PATCH",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Head => "HEAD",
        }
    }

    /// GET and HEAD carry no body.
    pub fn has_body(self) -> bool {
        !matches!(self, HttpMethod::Get | HttpMethod::Head)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct HttpConfig {
    pub method: HttpMethod,
    pub url: String,
    /// A JSON object as text; values may use `{{templates}}`.
    pub headers: String,
    pub body: String,
    /// Dot path into the JSON response that holds the rows ("data.results").
    pub response_path: String,
    /// Lets the node call loopback, link-local and private-network addresses.
    /// Off by default, so a workflow cannot reach local services or cloud metadata.
    pub allow_private_network: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FileFormat {
    #[default]
    Csv,
    Tsv,
    Json,
    Ndjson,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ReadFileConfig {
    pub path: String,
    pub format: FileFormat,
}

// ---------------------------------------------------------------- outputs
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SaveFormat {
    #[default]
    Csv,
    Tsv,
    Json,
    Ndjson,
    Markdown,
    Sql,
}

impl SaveFormat {
    pub fn extension(self) -> &'static str {
        match self {
            SaveFormat::Csv => "csv",
            SaveFormat::Tsv => "tsv",
            SaveFormat::Json => "json",
            SaveFormat::Ndjson => "ndjson",
            SaveFormat::Markdown => "md",
            SaveFormat::Sql => "sql",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct SaveFileConfig {
    /// Absolute path; blank until the first run prompts for one.
    pub path: String,
    pub format: SaveFormat,
    /// Table named in the INSERT statements of the SQL format; blank = the file's name.
    pub table_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct InsertConfig {
    /// `None` = the workflow's own connection.
    pub connection_id: Option<String>,
    pub schema: String,
    pub table: String,
}

// ---------------------------------------------------------------- control flow
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Combinator {
    #[default]
    And,
    Or,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RuleOp {
    #[default]
    Eq,
    Neq,
    Gt,
    Lt,
    Gte,
    Lte,
    Contains,
    StartsWith,
    EndsWith,
    IsNull,
    IsNotNull,
    In,
}

impl RuleOp {
    pub fn needs_value(self) -> bool {
        !matches!(self, RuleOp::IsNull | RuleOp::IsNotNull)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Rule {
    pub field: String,
    pub op: RuleOp,
    pub value: String,
}

/// Rules joined by AND / OR. An empty group matches every row.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct RuleGroup {
    pub combinator: Combinator,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct FilterConfig {
    pub group: RuleGroup,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct BranchConfig {
    pub group: RuleGroup,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GateOp {
    #[default]
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Neq,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct GateConfig {
    pub op: GateOp,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct RepeatConfig {
    pub count: u32,
}

impl Default for RepeatConfig {
    fn default() -> Self {
        RepeatConfig { count: 3 }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DelayUnit {
    Ms,
    #[default]
    S,
    M,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct DelayConfig {
    pub amount: f64,
    pub unit: DelayUnit,
}

impl Default for DelayConfig {
    fn default() -> Self {
        DelayConfig { amount: 1.0, unit: DelayUnit::S }
    }
}

// ---------------------------------------------------------------- transformation
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RegexMode {
    /// First capture group (or the whole match) into the output field.
    #[default]
    Extract,
    /// Substitute matches in place; `$1` … refer to capture groups.
    Replace,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct RegexConfig {
    pub source_field: String,
    pub pattern: String,
    /// Any of `i` `m` `s` `g`.
    pub flags: String,
    pub mode: RegexMode,
    pub replacement: String,
    /// Blank = write back into the source field.
    pub output_field: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct JavascriptConfig {
    /// The body of a function that receives `rows`, `fields` and `ctx`.
    pub code: String,
}

// ---------------------------------------------------------------- notifications
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct NotificationConfig {
    pub title: String,
    pub body: String,
}

// WHAT:  A secret that lives inside a node's config (the SMTP password, the
//        Slack webhook URL).
// WHY:   Such a value must be stored sealed and never travel back to the UI,
//        exactly like a connection's password. The UI sees only `has_secret`.
// HOW:   UI -> core: `plain` carries what the user typed (blank keeps the stored
//        secret), `clear` drops it, `inherit_last` copies the sealed secret of the
//        most recently saved node of the same kind (so a new Email node starts
//        with the last SMTP login). Core -> disk: `sealed` is AES-GCM, base64.
//        Core -> UI: `sealed` is blanked and `plain` is never written out.
// WHERE: src-tauri/src/services/workflows/secrets.rs
#[derive(Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct NodeSecret {
    #[serde(skip_serializing_if = "sealed_stays_home")]
    #[ts(skip)]
    pub sealed: Option<String>,
    pub has_secret: bool,
    #[serde(default, skip_serializing)]
    pub plain: Option<String>,
    pub clear: bool,
    pub inherit_last: bool,
}

thread_local! {
    static PERSISTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// WHAT:  Runs `write` with sealed secrets included in the JSON it serialises.
// WHY:   A sealed secret belongs in the database and nowhere else. Outside this
//        scope `NodeSecret::sealed` is never written, so no command that returns a
//        document (list, save, rename, duplicate, a version snapshot) can carry one
//        to the UI, whatever a future caller forgets to redact.
// HOW:   A thread-local flag that only the store's document writer sets, for the
//        length of its (synchronous) serialisation.
// WHERE: src-tauri/src/store/documents.rs (`upsert_document`)
pub fn persisting<T>(write: impl FnOnce() -> T) -> T {
    let before = PERSISTING.with(|flag| flag.replace(true));
    let out = write();
    PERSISTING.with(|flag| flag.set(before));
    out
}

fn sealed_stays_home(sealed: &Option<String>) -> bool {
    sealed.is_none() || !PERSISTING.with(std::cell::Cell::get)
}

// A stray `{:?}` in a log line must not print a password.
impl std::fmt::Debug for NodeSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeSecret").field("has_secret", &self.has_secret).finish_non_exhaustive()
    }
}

/// The node kinds that hold a secret and remember their last setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NotifierKind {
    Email,
    Slack,
}

/// How the SMTP connection is secured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SmtpSecurity {
    /// Implicit TLS on port 465, otherwise STARTTLS when the server offers it.
    #[default]
    Auto,
    /// TLS from the first byte (SMTPS).
    Implicit,
    /// Upgrade with STARTTLS and refuse to continue without it.
    StartTls,
    /// Plain text: only for a local test server.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct EmailConfig {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub username: String,
    pub password: NodeSecret,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub body: String,
}

impl Default for EmailConfig {
    fn default() -> Self {
        EmailConfig {
            host: String::new(),
            port: 587,
            security: SmtpSecurity::Auto,
            username: String::new(),
            password: NodeSecret::default(),
            from: String::new(),
            to: String::new(),
            subject: String::new(),
            body: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct SlackConfig {
    /// The incoming-webhook URL: the whole URL is the secret.
    pub webhook: NodeSecret,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct NoteConfig {
    pub text: String,
}

// ============================================================================
// RUN EVENTS AND REPORTS
//
// WHAT:  What a running workflow tells the canvas, and what it leaves behind.
// WHY:   The engine runs in Rust (so a schedule can fire with the tab closed);
//        the canvas only draws what it is told, node by node, as it happens.
// HOW:   `WorkflowEvent`s are emitted on the app event `workflow:event`; the
//        command's own return value is the final `WorkflowRunReport`.
// WHERE: src-tauri/src/commands/workflows.rs (emitter), src/features/workflows (listener)
// ============================================================================

/// Where a node is in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NodeRunState {
    Running,
    Success,
    Error,
    /// Not run: every path into it was closed (a Branch side with no rows, a closed gate).
    Skipped,
    /// The user pressed Stop.
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunStatus {
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RunTrigger {
    Manual,
    Schedule,
}

/// First rows of one output of a node, for the inspector's preview grid.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodePreview {
    /// `None` = the node's only output.
    pub handle: Option<SourceHandle>,
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<Value>>,
    pub total_rows: u64,
}

/// One line the JavaScript node's `console` captured.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConsoleLine {
    /// `log`, `info`, `warn`, `error` or `debug`.
    pub level: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NodeUpdate {
    pub run_id: String,
    pub workflow_id: String,
    pub node_id: String,
    pub state: NodeRunState,
    /// Rows the node produced (or inserted).
    pub rows: Option<u64>,
    pub elapsed_ms: u64,
    pub error: Option<String>,
    /// File a Save node wrote.
    pub file_path: Option<String>,
    pub outputs: Vec<NodePreview>,
    pub logs: Vec<ConsoleLine>,
    /// Something worth saying about a successful run ("stopped at 1,000,000 rows").
    pub note: Option<String>,
    /// Which pass of a Repeat this is (1-based) and how many there are.
    pub pass: Option<u32>,
    pub passes: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum WorkflowEvent {
    #[serde(rename_all = "camelCase")]
    RunStarted { run_id: String, workflow_id: String, trigger: RunTrigger, node_ids: Vec<String> },
    Node { update: NodeUpdate },
    #[serde(rename_all = "camelCase")]
    RunFinished { run_id: String, workflow_id: String, report: crate::model::WorkflowRunReport },
}

/// A saved schedule as Settings lists it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowScheduleStatus {
    pub workflow_id: String,
    pub workflow_name: String,
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub node_id: String,
    pub mode: ScheduleMode,
    /// "Every 5 minutes", "Mon, Wed at 09:00 (Europe/Paris)".
    pub summary: String,
    pub schedules_active: bool,
    /// RFC 3339; `None` while paused or when the schedule cannot fire.
    pub next_fire_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RegexTestResult {
    pub matched: bool,
    pub output: Option<String>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reads_a_partial_document() {
        let node: WorkflowNode = serde_json::from_str(r#"{"id":"n1","x":1,"y":2,"config":{"kind":"query","sql":"select 1"}}"#).unwrap();
        assert!(!node.continue_on_error);
        let NodeConfig::Query(q) = node.config else { panic!("expected a query node") };
        assert_eq!(q.sql, "select 1");
        assert!(!q.allow_destructive, "destructive statements are opt-in");
        assert!(q.connection_id.is_none());
    }

    #[test]
    fn config_kinds_use_snake_case() {
        let node: WorkflowNode =
            serde_json::from_str(r#"{"id":"n","x":0,"y":0,"config":{"kind":"row_count_gate","op":"gte","value":5}}"#).unwrap();
        assert_eq!(node.config.kind_name(), "row_count_gate");
        assert!(serde_json::from_str::<WorkflowNode>(r#"{"id":"n","x":0,"y":0,"config":{"kind":"teleport"}}"#).is_err());
    }

    #[test]
    fn secret_never_serialises_its_plaintext() {
        let secret = NodeSecret { sealed: Some("c2VhbGVk".into()), has_secret: true, plain: Some("hunter2".into()), clear: false, inherit_last: false };
        let json = serde_json::to_string(&secret).unwrap();
        assert!(!json.contains("hunter2"), "{json}");
        assert!(!json.contains("c2VhbGVk"), "outside the store a sealed blob is never written: {json}");
        let stored = persisting(|| serde_json::to_string(&secret).unwrap());
        assert!(stored.contains("c2VhbGVk") && !stored.contains("hunter2"), "the store keeps the sealed blob: {stored}");
        let back: NodeSecret = serde_json::from_str(r#"{"plain":"typed","hasSecret":false}"#).unwrap();
        assert_eq!(back.plain.as_deref(), Some("typed"));
        assert!(back.sealed.is_none());
    }

    #[test]
    fn legacy_steps_upgrade_to_a_chain_of_query_nodes() {
        let body: WorkflowBody = serde_json::from_str(
            r#"{"steps":[
                {"id":"a","name":"first","connectionId":"c1","sql":"select 1","stopOnError":true},
                {"id":"b","name":"second","connectionId":null,"sql":"delete from t","stopOnError":false}
            ]}"#,
        )
        .unwrap();
        let body = body.upgraded();
        assert!(body.steps.is_empty());
        assert_eq!(body.nodes.len(), 2);
        assert_eq!(body.edges.len(), 1);
        assert_eq!((body.edges[0].source.as_str(), body.edges[0].target.as_str()), ("a", "b"));
        let NodeConfig::Query(second) = &body.nodes[1].config else { panic!("expected a query node") };
        assert_eq!(second.sql, "delete from t");
        assert!(!second.allow_destructive, "the old runner's blanket confirmation does not carry over");
        assert!(body.nodes[1].continue_on_error);
        assert!(!body.nodes[0].continue_on_error);
        assert_eq!(body.nodes[0].label, "first");
        let NodeConfig::Query(first) = &body.nodes[0].config else { panic!("expected a query node") };
        assert_eq!(first.connection_id.as_deref(), Some("c1"));
        // Upgrading twice changes nothing.
        let again = body.clone().upgraded();
        assert_eq!(again.nodes.len(), 2);
    }

    #[test]
    fn a_body_that_only_names_steps_is_an_empty_graph() {
        let body: WorkflowBody = serde_json::from_str(r#"{"steps":[]}"#).unwrap();
        let body = body.upgraded();
        assert!(body.nodes.is_empty() && body.edges.is_empty() && !body.schedules_active);
        let json = serde_json::to_string(&body).unwrap();
        assert_eq!(json, "{}", "an empty graph writes nothing");
    }

    #[test]
    fn a_workflow_written_without_positions_or_edge_ids_still_loads() {
        let body: WorkflowBody = serde_json::from_str(
            r#"{"nodes":[{"id":"a","config":{"kind":"query","sql":"select 1"}},{"id":"b","config":{"kind":"filter"}}],
                "edges":[{"source":"a","target":"b"}]}"#,
        )
        .unwrap();
        assert_eq!((body.nodes[0].x, body.nodes[0].y), (0.0, 0.0));
        assert_eq!(body.edges[0].id, "e-a-out-b");
    }

    #[test]
    fn edge_handle_reads_true_false_or_nothing() {
        let e: WorkflowEdge = serde_json::from_str(r#"{"id":"e","source":"a","target":"b","sourceHandle":"true"}"#).unwrap();
        assert_eq!(e.source_handle, Some(SourceHandle::True));
        let e: WorkflowEdge = serde_json::from_str(r#"{"id":"e","source":"a","target":"b"}"#).unwrap();
        assert_eq!(e.source_handle, None);
    }
}
