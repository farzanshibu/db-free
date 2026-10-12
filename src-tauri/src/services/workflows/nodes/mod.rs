// SOT: workflow-node-dispatch, node-context, node-output, node-failure

use super::data::Dataset;
use super::WorkflowHost;
use crate::error::AppError;
use crate::model::workflow::{ConsoleLine, NodeConfig, SourceHandle, WorkflowNode};
use crate::state::AppState;
use std::sync::atomic::AtomicU64;
use tokio_util::sync::CancellationToken;

mod io;
mod notify;
mod sql;
mod transform;

// WHAT:  Everything a node needs to run once.
// WHY:   One value instead of a dozen parameters, and one place that says what a
//        node may touch: the app state (connections, secrets), the host (desktop
//        notifications), its Stop token and its merged input.
// WHERE: src-tauri/src/services/workflows/runner.rs (builds it)
pub struct NodeCtx<'a> {
    pub state: &'a AppState,
    pub host: &'a dyn WorkflowHost,
    pub cancel: &'a CancellationToken,
    pub run_id: &'a str,
    /// The workflow's own connection, for nodes that name none.
    pub workflow_connection: Option<&'a str>,
    pub node: &'a WorkflowNode,
    /// Every active incoming edge's rows, merged (columns unioned, rows concatenated).
    pub input: &'a Dataset,
    /// How many datasets arrived (0 for a source node).
    pub input_count: usize,
    /// For a Repeat node: the current iteration (from 0) and the total.
    pub repeat: Option<(u32, u32)>,
    /// Makes the names under which statements register for Stop unique.
    pub sequence: &'a AtomicU64,
}

// WHAT:  What a node produced.
#[derive(Default)]
pub struct NodeOutput {
    /// One entry per output handle (a Branch has two, every other node at most one).
    pub outputs: Vec<(Option<SourceHandle>, Dataset)>,
    /// Rows to report when the node has no output (rows inserted, rows written).
    pub rows: Option<u64>,
    pub file_path: Option<String>,
    pub logs: Vec<ConsoleLine>,
    pub note: Option<String>,
    /// A gate that refused to let rows through: everything downstream is skipped.
    pub closed: bool,
}

impl NodeOutput {
    pub fn data(data: Dataset) -> NodeOutput {
        NodeOutput { rows: Some(data.row_count()), outputs: vec![(None, data)], ..NodeOutput::default() }
    }

    pub fn sink(rows: u64) -> NodeOutput {
        NodeOutput { rows: Some(rows), ..NodeOutput::default() }
    }
}

// WHAT:  Why a node did not finish.
#[derive(Debug, Default)]
pub struct NodeFailure {
    pub message: String,
    /// The user pressed Stop; not an error to report.
    pub stopped: bool,
    pub logs: Vec<ConsoleLine>,
}

impl NodeFailure {
    pub fn new(message: impl Into<String>) -> NodeFailure {
        NodeFailure { message: message.into(), ..NodeFailure::default() }
    }

    pub fn stopped() -> NodeFailure {
        NodeFailure { message: "Stopped".to_string(), stopped: true, ..NodeFailure::default() }
    }
}

impl From<AppError> for NodeFailure {
    fn from(err: AppError) -> NodeFailure {
        match err {
            AppError::Cancelled { .. } => NodeFailure::stopped(),
            other => NodeFailure::new(other.message().to_string()),
        }
    }
}

pub type NodeResult = Result<NodeOutput, NodeFailure>;

// WHAT:  Runs the node's own logic.
pub async fn run(ctx: &NodeCtx<'_>) -> NodeResult {
    match &ctx.node.config {
        NodeConfig::Schedule(_) => transform::schedule(),
        NodeConfig::Query(config) => sql::query(ctx, config).await,
        NodeConfig::Http(config) => io::http(ctx, config).await,
        NodeConfig::ReadFile(config) => io::read_file(config).await,
        NodeConfig::SaveFile(config) => io::save_file(ctx, config).await,
        NodeConfig::Insert(config) => sql::insert(ctx, config).await,
        NodeConfig::Filter(config) => transform::filter(ctx, config),
        NodeConfig::RowCountGate(config) => transform::gate(ctx, config),
        NodeConfig::Branch(config) => transform::branch(ctx, config),
        NodeConfig::Repeat(_) => transform::repeat(ctx),
        NodeConfig::Delay(config) => transform::delay(ctx, config).await,
        NodeConfig::Regex(config) => transform::regex(ctx, config),
        NodeConfig::Javascript(config) => transform::javascript(ctx, config).await,
        NodeConfig::Notification(config) => notify::notification(ctx, config),
        NodeConfig::Email(config) => notify::email(ctx, config).await,
        NodeConfig::Slack(config) => notify::slack(ctx, config).await,
        NodeConfig::Note(_) => Ok(NodeOutput::default()),
    }
}

// WHAT:  Races a future against the run's Stop token.
pub async fn or_stopped<T>(cancel: &CancellationToken, work: impl std::future::Future<Output = T>) -> Result<T, NodeFailure> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(NodeFailure::stopped()),
        value = work => Ok(value),
    }
}
