// SOT: workflow-runner, topological-execution, branch-pruning, repeat-passes, run-cancellation, streamed-node-status

use super::data::Dataset;
use super::graph::{self, Graph, Group, PassIndex};
use super::nodes::{self, NodeCtx, NodeFailure, NodeOutput};
use super::WorkflowHost;
use crate::error::{AppError, AppResult};
use crate::model::documents::{WorkflowRunReport, WorkflowStepResult};
use crate::model::workflow::{
    ConsoleLine, NodeConfig, NodePreview, NodeRunState, NodeUpdate, RunStatus, RunTrigger, SourceHandle, WorkflowEdge, WorkflowEvent,
};
use crate::model::{WorkflowBody, WorkflowNode};
use crate::services::connection;
use crate::state::AppState;
use futures::future::join_all;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Rows of a node's output sent with its status for the inspector's preview.
pub const PREVIEW_ROWS: usize = 200;

pub struct RunRequest<'a> {
    pub run_id: &'a str,
    pub workflow_id: &'a str,
    /// The workflow's own connection, for nodes that name none.
    pub workflow_connection: Option<&'a str>,
    pub body: &'a WorkflowBody,
    /// The nodes to run; each brings along everything upstream of it.
    pub targets: Vec<String>,
    pub trigger: RunTrigger,
    /// Pause between levels so a person can watch the data move; 0 for unattended runs.
    pub step_delay_ms: u64,
}

struct Shared<'a> {
    state: &'a AppState,
    host: &'a dyn WorkflowHost,
    cancel: &'a CancellationToken,
    request: &'a RunRequest<'a>,
    graph: &'a Graph<'a>,
    sequence: AtomicU64,
    results: Mutex<HashMap<String, WorkflowStepResult>>,
    errors: AtomicBool,
}

// WHAT:  Runs a workflow's targets and everything they depend on.
// WHY:   The one engine behind Run node, Run all and a schedule firing, so all
//        three behave the same.
// HOW:   The plan is checked before anything runs (unknown node, bad Repeat,
//        a cycle). Targets are grouped by the Repeat nodes upstream of them; each
//        group runs its passes in order, and groups run side by side. A pass walks
//        the graph level by level, a level's nodes in parallel. A node whose every
//        incoming path is closed (a Branch side with no rows, a closed gate) is
//        skipped. A node that fails ends its pass once its level finishes, unless
//        it is set to continue on error. Stop cancels the token every node watches.
// WHERE: src-tauri/src/services/workflows/mod.rs (`run_saved`)
pub async fn execute(state: &AppState, host: &dyn WorkflowHost, cancel: &CancellationToken, request: RunRequest<'_>) -> AppResult<WorkflowRunReport> {
    let started = Instant::now();
    let graph = Graph::new(&request.body.nodes, &request.body.edges)?;
    let groups = graph.plan(&request.targets)?;
    let mut level_sets = Vec::with_capacity(groups.len());
    for group in &groups {
        level_sets.push(graph.levels(&group.nodes)?);
    }
    let mut involved: HashSet<&str> = HashSet::new();
    for group in &groups {
        involved.extend(group.nodes.iter().map(String::as_str));
    }
    let node_ids: Vec<String> = request.body.nodes.iter().filter(|n| involved.contains(n.id.as_str())).map(|n| n.id.clone()).collect();
    if let Some(problem) = graph.wiring_problem(&involved) {
        return Err(AppError::invalid_input(problem));
    }

    host.emit(WorkflowEvent::RunStarted {
        run_id: request.run_id.to_string(),
        workflow_id: request.workflow_id.to_string(),
        trigger: request.trigger,
        node_ids,
    });
    ensure_connections(state, &request, &involved).await;

    let shared = Shared {
        state,
        host,
        cancel,
        request: &request,
        graph: &graph,
        sequence: AtomicU64::new(0),
        results: Mutex::new(HashMap::new()),
        errors: AtomicBool::new(false),
    };
    join_all(groups.iter().zip(level_sets.iter()).map(|(group, levels)| run_group(&shared, group, levels))).await;

    let status = if cancel.is_cancelled() {
        RunStatus::Stopped
    } else if shared.errors.load(Ordering::SeqCst) {
        RunStatus::Failed
    } else {
        RunStatus::Completed
    };
    let results = shared.results.lock().map(|r| r.clone()).unwrap_or_default();
    let nodes: Vec<WorkflowStepResult> = request.body.nodes.iter().filter_map(|n| results.get(&n.id).cloned()).collect();
    let report = WorkflowRunReport {
        run_id: request.run_id.to_string(),
        workflow_id: request.workflow_id.to_string(),
        status,
        elapsed_ms: millis(started.elapsed()),
        nodes,
    };
    host.emit(WorkflowEvent::RunFinished { run_id: request.run_id.to_string(), workflow_id: request.workflow_id.to_string(), report: report.clone() });
    Ok(report)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

// WHAT:  Opens the connections the run's nodes use that are not open yet.
// WHY:   A schedule fires with nobody at the keyboard; the connection it needs
//        may never have been opened this session. A failure here is not fatal:
//        the node that needs it reports the real reason.
async fn ensure_connections(state: &AppState, request: &RunRequest<'_>, involved: &HashSet<&str>) {
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    for node in &request.body.nodes {
        if !involved.contains(node.id.as_str()) {
            continue;
        }
        let chosen = match &node.config {
            NodeConfig::Query(q) => q.connection_id.as_deref(),
            NodeConfig::Insert(i) => i.connection_id.as_deref(),
            _ => continue,
        };
        if let Some(id) = chosen.map(str::trim).filter(|c| !c.is_empty()).or(request.workflow_connection) {
            wanted.insert(id.to_string());
        }
    }
    if wanted.is_empty() {
        return;
    }
    let open = state.session_ids().await;
    for id in wanted.iter().filter(|id| !open.contains(*id)) {
        if let Err(err) = connection::connect(state, id, None).await {
            log::warn!("workflow run could not open connection {id}: {err}");
        }
    }
}

async fn run_group(shared: &Shared<'_>, group: &Group, levels: &[Vec<String>]) {
    let passes = graph::passes(&group.repeats);
    let total = u32::try_from(passes.len()).unwrap_or(u32::MAX);
    for (index, pass) in passes.iter().enumerate() {
        if shared.cancel.is_cancelled() {
            break;
        }
        let label = if group.repeats.is_empty() { None } else { Some((u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1), total)) };
        if !run_pass(shared, levels, pass, label).await {
            break;
        }
    }
}

type Outputs = HashMap<(String, Option<SourceHandle>), Arc<Dataset>>;

/// Is anything travelling along this edge?
fn edge_active(shared: &Shared<'_>, edge: &WorkflowEdge, outputs: &Outputs, dead: &HashSet<String>) -> bool {
    if dead.contains(&edge.source) {
        return false;
    }
    match shared.graph.node(&edge.source).map(|n| &n.config) {
        Some(NodeConfig::Branch(_)) => outputs.get(&(edge.source.clone(), edge.source_handle)).is_some_and(|d| !d.rows.is_empty()),
        _ => true,
    }
}

// WHAT:  One walk through the graph. `false` when the pass failed or was stopped.
async fn run_pass(shared: &Shared<'_>, levels: &[Vec<String>], pass: &PassIndex, label: Option<(u32, u32)>) -> bool {
    let mut outputs: Outputs = HashMap::new();
    // Nodes that run nothing and pass nothing on: skipped, or a gate that closed.
    let mut dead: HashSet<String> = HashSet::new();
    for (index, level) in levels.iter().enumerate() {
        if shared.cancel.is_cancelled() {
            return false;
        }
        let mut ready: Vec<&String> = Vec::new();
        for id in level {
            let incoming = shared.graph.incoming(id);
            let all_closed = !incoming.is_empty() && incoming.iter().all(|e| !edge_active(shared, e, &outputs, &dead));
            if all_closed {
                dead.insert(id.clone());
                publish(shared, id, label, |u| u.state = NodeRunState::Skipped, StepKind::Skipped);
            } else {
                ready.push(id);
            }
        }
        if ready.is_empty() {
            continue;
        }
        for id in &ready {
            publish(shared, id, label, |u| u.state = NodeRunState::Running, StepKind::Running);
        }
        if index > 0 && shared.request.step_delay_ms > 0 {
            let pause = Duration::from_millis(shared.request.step_delay_ms);
            tokio::select! {
                () = shared.cancel.cancelled() => return false,
                () = tokio::time::sleep(pause) => {}
            }
        }
        let finished = join_all(ready.iter().map(|id| {
            let inputs: Vec<Arc<Dataset>> = shared
                .graph
                .incoming(id)
                .iter()
                .filter(|e| edge_active(shared, e, &outputs, &dead))
                .filter_map(|e| outputs.get(&(e.source.clone(), e.source_handle)).cloned())
                .collect();
            run_node(shared, id, inputs, pass, label)
        }))
        .await;
        let mut failed = false;
        for (id, outcome) in ready.iter().zip(finished) {
            match outcome {
                Outcome::Done(output) => {
                    if output.closed {
                        dead.insert((*id).clone());
                    }
                    for (handle, data) in output.outputs {
                        outputs.insert(((*id).clone(), handle), Arc::new(data));
                    }
                }
                Outcome::Continued => {
                    outputs.insert(((*id).clone(), None), Arc::new(Dataset::empty()));
                }
                Outcome::Failed => failed = true,
            }
        }
        if failed {
            return false;
        }
    }
    true
}

enum Outcome {
    Done(NodeOutput),
    /// It failed but is set to continue; downstream sees an empty result.
    Continued,
    Failed,
}

enum StepKind {
    Running,
    Skipped,
}

/// Emits a bare status update for a node (no output yet).
fn publish(shared: &Shared<'_>, id: &str, label: Option<(u32, u32)>, change: impl FnOnce(&mut NodeUpdate), kind: StepKind) {
    let mut update = blank_update(shared, id, label, NodeRunState::Running);
    change(&mut update);
    if matches!(kind, StepKind::Skipped) {
        record(shared, id, &update, None);
    }
    shared.host.emit(WorkflowEvent::Node { update });
}

fn blank_update(shared: &Shared<'_>, id: &str, label: Option<(u32, u32)>, state: NodeRunState) -> NodeUpdate {
    NodeUpdate {
        run_id: shared.request.run_id.to_string(),
        workflow_id: shared.request.workflow_id.to_string(),
        node_id: id.to_string(),
        state,
        rows: None,
        elapsed_ms: 0,
        error: None,
        file_path: None,
        outputs: Vec::new(),
        logs: Vec::new(),
        note: None,
        pass: label.map(|(pass, _)| pass),
        passes: label.map(|(_, total)| total),
    }
}

fn record(shared: &Shared<'_>, id: &str, update: &NodeUpdate, node: Option<&WorkflowNode>) {
    let name = node.map(display_name).or_else(|| shared.graph.node(id).map(display_name)).unwrap_or_else(|| id.to_string());
    let step = WorkflowStepResult {
        node_id: id.to_string(),
        name,
        state: update.state,
        elapsed_ms: update.elapsed_ms,
        rows: update.rows,
        error: update.error.clone(),
        file_path: update.file_path.clone(),
    };
    if let Ok(mut results) = shared.results.lock() {
        results.insert(id.to_string(), step);
    }
}

fn display_name(node: &WorkflowNode) -> String {
    if node.label.trim().is_empty() {
        node.config.kind_name().replace('_', " ")
    } else {
        node.label.clone()
    }
}

fn previews(output: &NodeOutput) -> Vec<NodePreview> {
    output
        .outputs
        .iter()
        .map(|(handle, data)| {
            let set = data.preview(PREVIEW_ROWS);
            NodePreview { handle: *handle, columns: set.columns, rows: set.rows, total_rows: data.row_count() }
        })
        .collect()
}

async fn run_node(shared: &Shared<'_>, id: &str, inputs: Vec<Arc<Dataset>>, pass: &PassIndex, label: Option<(u32, u32)>) -> Outcome {
    let Some(node) = shared.graph.node(id) else {
        return Outcome::Failed;
    };
    let started = Instant::now();
    let refs: Vec<&Dataset> = inputs.iter().map(Arc::as_ref).collect();
    let merged = Dataset::union(&refs);
    let ctx = NodeCtx {
        state: shared.state,
        host: shared.host,
        cancel: shared.cancel,
        run_id: shared.request.run_id,
        workflow_connection: shared.request.workflow_connection,
        node,
        input: &merged,
        input_count: inputs.len(),
        repeat: pass.get(id).copied(),
        sequence: &shared.sequence,
    };
    let result = nodes::run(&ctx).await;
    let mut update = blank_update(shared, id, label, NodeRunState::Success);
    update.elapsed_ms = millis(started.elapsed());
    match result {
        Ok(output) => {
            update.rows = output.rows;
            update.file_path = output.file_path.clone();
            update.outputs = previews(&output);
            update.logs = output.logs.clone();
            update.note = output.note.clone();
            record(shared, id, &update, Some(node));
            shared.host.emit(WorkflowEvent::Node { update });
            Outcome::Done(output)
        }
        Err(failure) => finish_failed(shared, node, update, failure),
    }
}

fn finish_failed(shared: &Shared<'_>, node: &WorkflowNode, mut update: NodeUpdate, failure: NodeFailure) -> Outcome {
    let logs: Vec<ConsoleLine> = failure.logs;
    update.logs = logs;
    update.error = Some(failure.message);
    if failure.stopped {
        update.state = NodeRunState::Stopped;
    } else {
        update.state = NodeRunState::Error;
        shared.errors.store(true, Ordering::SeqCst);
    }
    record(shared, &node.id, &update, Some(node));
    shared.host.emit(WorkflowEvent::Node { update });
    if node.continue_on_error && !failure.stopped {
        Outcome::Continued
    } else {
        Outcome::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::workflow::{
        BranchConfig, Combinator, DelayConfig, DelayUnit, FilterConfig, GateConfig, GateOp, JavascriptConfig, QueryConfig, RepeatConfig, Rule,
        RuleGroup, RuleOp, SaveFileConfig, SaveFormat,
    };
    use crate::model::{ConnectionInput, Engine, Environment, SshTunnel, SslMode};
    use crate::store::Store;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct Recorder {
        events: StdMutex<Vec<WorkflowEvent>>,
        notices: StdMutex<Vec<(String, String)>>,
    }

    impl WorkflowHost for Recorder {
        fn emit(&self, event: WorkflowEvent) {
            if let Ok(mut events) = self.events.lock() {
                events.push(event);
            }
        }

        fn notify(&self, title: &str, body: &str) -> AppResult<()> {
            if let Ok(mut notices) = self.notices.lock() {
                notices.push((title.to_string(), body.to_string()));
            }
            Ok(())
        }
    }

    impl Recorder {
        fn updates(&self, node: &str) -> Vec<NodeUpdate> {
            let events = self.events.lock().unwrap();
            events
                .iter()
                .filter_map(|e| match e {
                    WorkflowEvent::Node { update } if update.node_id == node => Some(update.clone()),
                    _ => None,
                })
                .collect()
        }

        fn last(&self, node: &str) -> NodeUpdate {
            self.updates(node).pop().unwrap_or_else(|| panic!("no update for {node}"))
        }
    }

    async fn sqlite_state(read_only: bool) -> (AppState, String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("db-free-wf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.db").to_string_lossy().into_owned();
        std::fs::File::create(&path).unwrap();
        let store = Store::open_in_memory().unwrap();
        let input = ConnectionInput {
            name: "wf".into(),
            engine: Engine::Sqlite,
            environment: Environment::Production,
            read_only,
            host: None,
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: Some(path),
            ssl_mode: SslMode::Disable,
            ssh: SshTunnel::default(),
            ssh_secret: None,
            folder: None,
            color: None,
            favorite: false,
            auth: Default::default(),
            auth_secrets: Default::default(),
            extra: Default::default(),
        };
        let summary = store.insert_connection(&input, None).unwrap();
        let state = AppState::new(store, Box::new(MemoryKeyProvider::default()));
        connection::connect(&state, &summary.id, None).await.unwrap();
        (state, summary.id, dir)
    }

    fn node(id: &str, config: NodeConfig) -> WorkflowNode {
        WorkflowNode { id: id.into(), x: 0.0, y: 0.0, label: String::new(), continue_on_error: false, config }
    }

    fn query(id: &str, sql: &str) -> WorkflowNode {
        node(id, NodeConfig::Query(QueryConfig { connection_id: None, sql: sql.into(), allow_destructive: false }))
    }

    fn edge(a: &str, b: &str) -> WorkflowEdge {
        WorkflowEdge { id: format!("{a}-{b}"), source: a.into(), target: b.into(), source_handle: None }
    }

    fn branch_edge(a: &str, b: &str, handle: SourceHandle) -> WorkflowEdge {
        WorkflowEdge { id: format!("{a}-{b}"), source: a.into(), target: b.into(), source_handle: Some(handle) }
    }

    async fn run_all(state: &AppState, host: &Recorder, connection: &str, nodes: Vec<WorkflowNode>, edges: Vec<WorkflowEdge>) -> AppResult<WorkflowRunReport> {
        let body = WorkflowBody { nodes, edges, ..WorkflowBody::default() };
        let graph = Graph::new(&body.nodes, &body.edges)?;
        let targets = graph.leaves();
        let request = RunRequest {
            run_id: "run-1",
            workflow_id: "wf-1",
            workflow_connection: Some(connection),
            body: &body,
            targets,
            trigger: RunTrigger::Manual,
            step_delay_ms: 0,
        };
        execute(state, host, &CancellationToken::new(), request).await
    }

    const SEED: &str = "CREATE TABLE t (id INTEGER, name TEXT); INSERT INTO t VALUES (1,'ada'),(2,'bob'),(3,'cy'); SELECT * FROM t ORDER BY id";

    #[tokio::test]
    async fn a_query_feeds_a_filter_and_a_file() {
        let (state, conn, dir) = sqlite_state(false).await;
        let out = dir.join("out.csv").to_string_lossy().into_owned();
        let filter = node(
            "f",
            NodeConfig::Filter(FilterConfig { group: RuleGroup { combinator: Combinator::And, rules: vec![Rule { field: "id".into(), op: RuleOp::Gt, value: "1".into() }] } }),
        );
        let save = node("s", NodeConfig::SaveFile(SaveFileConfig { path: out.clone(), format: SaveFormat::Csv, table_name: String::new() }));
        let host = Recorder::default();
        let report = run_all(&state, &host, &conn, vec![query("q", SEED), filter, save], vec![edge("q", "f"), edge("f", "s")]).await.unwrap();
        assert_eq!(report.status, RunStatus::Completed, "{:?}", report.nodes);
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "id,name\n2,bob\n3,cy\n");
        let q = host.last("q");
        assert_eq!(q.state, NodeRunState::Success);
        assert_eq!(q.outputs[0].total_rows, 3);
        assert_eq!(host.last("s").file_path.as_deref(), Some(out.as_str()));
        assert_eq!(report.nodes.len(), 3);
        // The query ran through the guard: it is in the history.
        let history = state.with_store(|s| s.list_history(Some(&conn), None, 10)).unwrap();
        assert!(history.iter().any(|h| h.sql.contains("CREATE TABLE t")));
    }

    #[tokio::test]
    async fn templates_fill_sql_from_the_previous_node() {
        let (state, conn, _dir) = sqlite_state(false).await;
        let host = Recorder::default();
        let seed = query("seed", SEED);
        let pick = query("pick", "SELECT 'x' AS note WHERE '{{name}}' = 'ada' AND {{id}} = 1");
        let first = node("one", NodeConfig::Filter(FilterConfig::default()));
        let report = run_all(&state, &host, &conn, vec![seed, first, pick], vec![edge("seed", "one"), edge("one", "pick")]).await.unwrap();
        assert_eq!(report.status, RunStatus::Completed, "{:?}", report.nodes);
        assert_eq!(host.last("pick").outputs[0].total_rows, 1);
    }

    #[tokio::test]
    async fn a_branch_skips_the_empty_side_and_runs_the_other() {
        let (state, conn, dir) = sqlite_state(false).await;
        let host = Recorder::default();
        let rule = Rule { field: "id".into(), op: RuleOp::Gt, value: "100".into() };
        let branch = node("b", NodeConfig::Branch(BranchConfig { group: RuleGroup { combinator: Combinator::And, rules: vec![rule] } }));
        let yes = dir.join("yes.json").to_string_lossy().into_owned();
        let no = dir.join("no.json").to_string_lossy().into_owned();
        let save = |id: &str, path: &str| node(id, NodeConfig::SaveFile(SaveFileConfig { path: path.into(), format: SaveFormat::Json, table_name: String::new() }));
        let report = run_all(
            &state,
            &host,
            &conn,
            vec![query("q", SEED), branch, save("yes", &yes), save("no", &no)],
            vec![edge("q", "b"), branch_edge("b", "yes", SourceHandle::True), branch_edge("b", "no", SourceHandle::False)],
        )
        .await
        .unwrap();
        assert_eq!(report.status, RunStatus::Completed);
        assert_eq!(host.last("yes").state, NodeRunState::Skipped);
        assert_eq!(host.last("no").state, NodeRunState::Success);
        assert!(!std::path::Path::new(&yes).exists(), "a skipped branch writes nothing");
        assert!(std::path::Path::new(&no).exists());
    }

    #[tokio::test]
    async fn a_closed_gate_stops_the_notifications_after_it() {
        let (state, conn, _dir) = sqlite_state(false).await;
        let host = Recorder::default();
        let gate = node("g", NodeConfig::RowCountGate(GateConfig { op: GateOp::Gt, value: 0.0 }));
        let notify = node("n", NodeConfig::Notification(crate::model::workflow::NotificationConfig { title: "Rows: {{rows.count}}".into(), body: String::new() }));
        let none = query("q", "SELECT 1 AS x WHERE 1 = 0");
        let report = run_all(&state, &host, &conn, vec![none, gate, notify.clone()], vec![edge("q", "g"), edge("g", "n")]).await.unwrap();
        assert_eq!(report.status, RunStatus::Completed);
        assert_eq!(host.last("g").state, NodeRunState::Success);
        assert_eq!(host.last("n").state, NodeRunState::Skipped);
        assert!(host.notices.lock().unwrap().is_empty());
        // With rows, the gate opens and the notification fires with the count.
        let some = query("q", "SELECT 1 AS x UNION ALL SELECT 2");
        let host = Recorder::default();
        run_all(&state, &host, &conn, vec![some, node("g", NodeConfig::RowCountGate(GateConfig { op: GateOp::Gte, value: 2.0 })), notify], vec![edge("q", "g"), edge("g", "n")])
            .await
            .unwrap();
        assert_eq!(host.notices.lock().unwrap().as_slice(), &[("Rows: 2".to_string(), String::new())]);
    }

    #[tokio::test]
    async fn repeat_runs_the_chain_once_per_iteration() {
        let (state, conn, dir) = sqlite_state(false).await;
        let host = Recorder::default();
        let log = dir.join("log.ndjson").to_string_lossy().into_owned();
        let report = run_all(
            &state,
            &host,
            &conn,
            vec![
                node("r", NodeConfig::Repeat(RepeatConfig { count: 3 })),
                query("q", "SELECT {{i}} AS i, {{i * 10}} AS tens"),
                node("s", NodeConfig::SaveFile(SaveFileConfig { path: log.clone(), format: SaveFormat::Ndjson, table_name: String::new() })),
            ],
            vec![edge("r", "q"), edge("q", "s")],
        )
        .await
        .unwrap();
        assert_eq!(report.status, RunStatus::Completed, "{:?}", report.nodes);
        let q_updates = host.updates("q");
        let finished: Vec<_> = q_updates.iter().filter(|u| u.state == NodeRunState::Success).collect();
        assert_eq!(finished.len(), 3);
        assert_eq!(finished[2].pass, Some(3));
        assert_eq!(finished[2].passes, Some(3));
        // The save node overwrites, so the file holds the last pass.
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "{\"i\":2,\"tens\":20}\n");
    }

    #[tokio::test]
    async fn destructive_sql_needs_the_nodes_own_opt_in() {
        let (state, conn, _dir) = sqlite_state(false).await;
        let setup = query("seed", SEED);
        let wipe = |allow: bool| node("wipe", NodeConfig::Query(QueryConfig { connection_id: None, sql: "DELETE FROM t".into(), allow_destructive: allow }));
        let host = Recorder::default();
        let report = run_all(&state, &host, &conn, vec![setup.clone(), wipe(false)], vec![edge("seed", "wipe")]).await.unwrap();
        assert_eq!(report.status, RunStatus::Failed);
        let blocked = host.last("wipe");
        assert_eq!(blocked.state, NodeRunState::Error);
        assert!(blocked.error.unwrap_or_default().contains("Allow destructive statements"));
        let rows: i64 = {
            let host = Recorder::default();
            run_all(&state, &host, &conn, vec![query("count", "SELECT count(*) AS n FROM t")], vec![]).await.unwrap();
            match &host.last("count").outputs[0].rows[0][0] {
                crate::model::Value::Int(n) => *n,
                other => panic!("unexpected {other:?}"),
            }
        };
        assert_eq!(rows, 3, "the blocked DELETE removed nothing");
        // With the node's saved opt-in it runs.
        let host = Recorder::default();
        let report = run_all(&state, &host, &conn, vec![wipe(true)], vec![]).await.unwrap();
        assert_eq!(report.status, RunStatus::Completed, "{:?}", report.nodes);
    }

    #[tokio::test]
    async fn a_read_only_connection_refuses_workflow_writes() {
        let (state, conn, _dir) = sqlite_state(true).await;
        let host = Recorder::default();
        let report = run_all(&state, &host, &conn, vec![query("w", "CREATE TABLE nope (a int)")], vec![]).await.unwrap();
        assert_eq!(report.status, RunStatus::Failed);
        assert!(host.last("w").error.unwrap_or_default().contains("read-only"));
    }

    #[tokio::test]
    async fn a_failure_stops_the_pass_unless_the_node_continues() {
        let (state, conn, _dir) = sqlite_state(false).await;
        let host = Recorder::default();
        let report = run_all(&state, &host, &conn, vec![query("bad", "SELECT * FROM missing"), query("after", "SELECT 1")], vec![edge("bad", "after")]).await.unwrap();
        assert_eq!(report.status, RunStatus::Failed);
        assert!(host.updates("after").is_empty(), "nothing downstream of a failure runs");
        let mut tolerant = query("bad", "SELECT * FROM missing");
        tolerant.continue_on_error = true;
        let host = Recorder::default();
        let report = run_all(&state, &host, &conn, vec![tolerant, query("after", "SELECT 1")], vec![edge("bad", "after")]).await.unwrap();
        assert_eq!(report.status, RunStatus::Failed, "the error still counts");
        assert_eq!(host.last("after").state, NodeRunState::Success);
    }

    #[tokio::test]
    async fn stop_ends_a_delay_and_marks_the_run_stopped() {
        let (state, conn, _dir) = sqlite_state(false).await;
        let host = Recorder::default();
        let body = WorkflowBody {
            nodes: vec![node("d", NodeConfig::Delay(DelayConfig { amount: 60.0, unit: DelayUnit::S }))],
            ..WorkflowBody::default()
        };
        let cancel = CancellationToken::new();
        let stopper = cancel.clone();
        let request = RunRequest {
            run_id: "r",
            workflow_id: "w",
            workflow_connection: Some(&conn),
            body: &body,
            targets: vec!["d".into()],
            trigger: RunTrigger::Manual,
            step_delay_ms: 0,
        };
        let started = Instant::now();
        let (report, ()) = tokio::join!(execute(&state, &host, &cancel, request), async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            stopper.cancel();
        });
        assert_eq!(report.unwrap().status, RunStatus::Stopped);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(host.last("d").state, NodeRunState::Stopped);
    }

    #[tokio::test]
    async fn javascript_transforms_rows_and_reports_its_console() {
        let (state, conn, _dir) = sqlite_state(false).await;
        let host = Recorder::default();
        let js = node("js", NodeConfig::Javascript(JavascriptConfig { code: "console.log('rows', rows.length); return rows.map(r => ({ ...r, shout: r.name.toUpperCase() }));".into() }));
        let report = run_all(&state, &host, &conn, vec![query("q", SEED), js], vec![edge("q", "js")]).await.unwrap();
        assert_eq!(report.status, RunStatus::Completed, "{:?}", report.nodes);
        let done = host.last("js");
        assert_eq!(done.logs[0].text, "rows 3");
        assert_eq!(done.outputs[0].columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["id", "name", "shout"]);
    }

    #[tokio::test]
    async fn cycles_and_bad_plans_are_rejected_before_anything_runs() {
        let (state, conn, _dir) = sqlite_state(false).await;
        let host = Recorder::default();
        let body = WorkflowBody {
            nodes: vec![query("a", "SELECT 1"), query("b", "SELECT 2"), query("c", "SELECT 3")],
            edges: vec![edge("a", "b"), edge("b", "c"), edge("c", "a")],
            ..WorkflowBody::default()
        };
        let request = |target: &str| RunRequest {
            run_id: "r",
            workflow_id: "w",
            workflow_connection: Some(&conn),
            body: &body,
            targets: vec![target.to_string()],
            trigger: RunTrigger::Manual,
            step_delay_ms: 0,
        };
        assert!(execute(&state, &host, &CancellationToken::new(), request("c")).await.is_err());
        assert!(execute(&state, &host, &CancellationToken::new(), request("nope")).await.is_err());
        assert!(host.events.lock().unwrap().is_empty(), "nothing was started");
    }

    #[tokio::test]
    async fn a_branch_connection_without_a_side_is_refused_up_front() {
        let (state, conn, _dir) = sqlite_state(false).await;
        let host = Recorder::default();
        let branch = node("b", NodeConfig::Branch(BranchConfig::default()));
        let nodes = vec![query("q", SEED), branch, query("after", "SELECT 1")];
        let err = run_all(&state, &host, &conn, nodes, vec![edge("q", "b"), edge("b", "after")]).await.err();
        assert!(matches!(&err, Some(AppError::InvalidInput { message }) if message.contains("True or the False")), "{err:?}");
        assert!(host.events.lock().unwrap().is_empty(), "nothing was started");
    }

    #[tokio::test]
    async fn insert_writes_rows_and_names_the_failing_one() {
        let (state, conn, _dir) = sqlite_state(false).await;
        let host = Recorder::default();
        let insert = |table: &str| node("ins", NodeConfig::Insert(crate::model::workflow::InsertConfig { connection_id: None, schema: String::new(), table: table.into() }));
        let make = query("mk", "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT); SELECT 1 AS id, 'ada' AS name UNION ALL SELECT 2, 'bob' UNION ALL SELECT 2, 'dup'");
        let report = run_all(&state, &host, &conn, vec![make, insert("people")], vec![edge("mk", "ins")]).await.unwrap();
        assert_eq!(report.status, RunStatus::Failed);
        let failure = host.last("ins").error.unwrap_or_default();
        assert!(failure.starts_with("Row 3:") && failure.contains("2 row(s) were inserted"), "{failure}");
    }
}
