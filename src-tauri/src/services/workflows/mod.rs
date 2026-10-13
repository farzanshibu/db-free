// SOT: workflow-service, workflow-engine-index, workflow-host, workflow-run-registry, run-saved-workflow, run-scheduled-fire

use crate::error::{AppError, AppResult};
use crate::model::documents::WorkflowRunReport;
use crate::model::workflow::{NotifierKind, RegexMode, RegexTestResult, RunTrigger, WorkflowEvent};
use crate::model::{Document, DocumentBody, NodeConfig, WorkflowScheduleStatus};
use crate::state::AppState;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

pub mod data;
pub mod files;
pub mod graph;
#[cfg(test)]
mod net_tests;
mod nodes;
pub mod rules;
mod runner;
pub mod policy;
pub mod schedule;
pub mod scheduler;
pub mod secrets;
pub mod template;
pub mod text;

pub use scheduler::{documents_changed, scheduler, Fire};

// WHAT:  What the engine needs from the outside world.
// WHY:   Services never touch the windowing layer. The command layer implements
//        this once, turning events into app events and `notify` into a native
//        notification; tests implement it with a recorder.
pub trait WorkflowHost: Send + Sync {
    /// Reports a run or node status change to whoever is watching.
    fn emit(&self, event: WorkflowEvent);
    /// Shows a desktop notification.
    fn notify(&self, title: &str, body: &str) -> AppResult<()>;
}

// WHAT:  The runs in flight, each with the token its Stop button trips.
// WHY:   `cancel_workflow` reaches a run by id; a schedule asks whether its
//        workflow is already running before it starts another.
// HOW:   Managed by Tauri (src-tauri/src/lib.rs). The lock is never held across an await.
#[derive(Default)]
pub struct WorkflowRuns {
    runs: Mutex<HashMap<String, (String, CancellationToken)>>,
}

impl WorkflowRuns {
    pub fn begin(&self, run_id: &str, workflow_id: &str) -> AppResult<CancellationToken> {
        let mut runs = self.runs.lock().map_err(|_| AppError::internal("workflow run lock poisoned"))?;
        if runs.contains_key(run_id) {
            return Err(AppError::invalid_input("That run is already in progress."));
        }
        let token = CancellationToken::new();
        runs.insert(run_id.to_string(), (workflow_id.to_string(), token.clone()));
        Ok(token)
    }

    /// True when a live run was told to stop; a run that already ended is not an error.
    pub fn cancel(&self, run_id: &str) -> bool {
        let token = self.runs.lock().ok().and_then(|runs| runs.get(run_id).map(|(_, token)| token.clone()));
        match token {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    pub fn end(&self, run_id: &str) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.remove(run_id);
        }
    }

    pub fn is_busy(&self, workflow_id: &str) -> bool {
        self.runs.lock().map(|runs| runs.values().any(|(id, _)| id == workflow_id)).unwrap_or(false)
    }
}

pub struct RunOptions {
    pub run_id: String,
    /// Run just this node and what it needs; `None` runs the whole workflow.
    pub target: Option<String>,
    /// A schedule firing: run only what is downstream of this trigger.
    pub trigger_node: Option<String>,
    pub trigger: RunTrigger,
    pub step_delay_ms: u64,
}

// WHAT:  Seals the secrets of a workflow document before it is stored.
pub fn seal_document(state: &AppState, doc: &Document) -> AppResult<Document> {
    let mut sealed = doc.clone();
    if let DocumentBody::Workflow(body) = &doc.body {
        sealed.body = DocumentBody::Workflow(secrets::prepare_save(state, &doc.id, body.clone())?);
    }
    Ok(sealed)
}

// WHAT:  Runs a saved workflow. The graph, the destructive opt-ins and the sealed
//        secrets are read from the store, never taken from the caller.
// WHY:   What runs is exactly what was saved. A node can run destructive SQL only if
//        its saved "Allow destructive statements" is on, and a schedule runs the
//        same saved document with the same rules.
pub async fn run_saved(state: &AppState, host: &dyn WorkflowHost, runs: &WorkflowRuns, workflow_id: &str, options: RunOptions) -> AppResult<WorkflowRunReport> {
    let doc = state.with_store(|store| store.get_document(workflow_id))?;
    let DocumentBody::Workflow(body) = &doc.body else {
        return Err(AppError::invalid_input("Document is not a workflow."));
    };
    let graph = graph::Graph::new(&body.nodes, &body.edges)?;
    let targets = match (&options.target, &options.trigger_node) {
        (Some(node), _) => vec![node.clone()],
        (None, Some(trigger)) => graph.leaves_downstream_of(trigger),
        (None, None) => graph.leaves(),
    };
    if targets.is_empty() {
        // A graph that is all cycle has no end to run; say so rather than "nothing".
        let everything = body.nodes.iter().map(|n| n.id.clone()).collect();
        graph.levels(&everything)?;
        return Err(AppError::invalid_input("This workflow has nothing to run. Add a node to the canvas."));
    }
    let token = runs.begin(&options.run_id, workflow_id)?;
    let request = runner::RunRequest {
        run_id: &options.run_id,
        workflow_id,
        workflow_connection: doc.connection_id.as_deref(),
        body,
        targets,
        trigger: options.trigger,
        step_delay_ms: options.step_delay_ms,
    };
    let result = runner::execute(state, host, &token, request).await;
    runs.end(&options.run_id);
    result
}

// WHAT:  Runs what a Schedule node triggers, unless that workflow is paused,
//        gone, or still busy with its previous run.
// WHY:   An interval shorter than a run must not pile runs up on each other.
pub async fn run_fire(state: &AppState, host: &dyn WorkflowHost, runs: &WorkflowRuns, fire: &Fire) -> AppResult<Option<WorkflowRunReport>> {
    let Ok(doc) = state.with_store(|store| store.get_document(&fire.workflow_id)) else {
        return Ok(None);
    };
    let active = matches!(&doc.body, DocumentBody::Workflow(body) if body.schedules_active && body.nodes.iter().any(|n| n.id == fire.node_id && matches!(n.config, NodeConfig::Schedule(_))));
    if !active {
        return Ok(None);
    }
    if runs.is_busy(&fire.workflow_id) {
        log::info!("workflow {} is still running; skipping this scheduled fire", fire.workflow_id);
        return Ok(None);
    }
    let options = RunOptions {
        run_id: uuid::Uuid::new_v4().to_string(),
        target: None,
        trigger_node: Some(fire.node_id.clone()),
        trigger: RunTrigger::Schedule,
        step_delay_ms: 0,
    };
    run_saved(state, host, runs, &fire.workflow_id, options).await.map(Some)
}

/// The Schedule nodes that are due right now (the command layer's clock calls this each second).
pub fn due_fires(state: &AppState) -> Vec<Fire> {
    scheduler().poll(state, chrono::Utc::now())
}

pub fn list_schedules(state: &AppState) -> AppResult<Vec<WorkflowScheduleStatus>> {
    scheduler::statuses(state, chrono::Utc::now())
}

pub fn test_regex(pattern: &str, flags: &str, mode: RegexMode, replacement: &str, sample: &str) -> RegexTestResult {
    text::test(pattern, flags, mode, replacement, sample)
}

pub fn last_node_config(state: &AppState, kind: NotifierKind) -> AppResult<Option<NodeConfig>> {
    secrets::last_node_config(state, kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::DocumentKind;
    use crate::store::Store;

    fn email_workflow(password: &str) -> Document {
        serde_json::from_value(serde_json::json!({
            "id": "", "kind": "workflow", "connectionId": null, "name": "Report", "tags": [], "createdAt": "", "updatedAt": "",
            "body": { "kind": "workflow", "data": {
                "nodes": [{ "id": "mail", "x": 10, "y": 20, "config": {
                    "kind": "email", "host": "smtp.example.com", "port": 587, "from": "a@x.io", "to": "b@x.io",
                    "password": { "plain": password, "hasSecret": false }
                } }],
                "edges": []
            } }
        }))
        .unwrap()
    }

    #[test]
    fn a_saved_secret_never_reaches_a_document_the_ui_receives() {
        let state = AppState::new(Store::open_in_memory().unwrap(), Box::new(MemoryKeyProvider::default()));
        let saved = crate::services::documents::save(&state, &email_workflow("hunter2")).unwrap();
        // What save, list and get hand back (and so what a command serialises to the UI):
        let listed = crate::services::documents::list(&state, DocumentKind::Workflow).unwrap();
        let fetched = crate::services::documents::get(&state, &saved.id).unwrap();
        for doc in [&saved, &listed[0], &fetched] {
            let json = serde_json::to_string(doc).unwrap();
            assert!(!json.contains("hunter2"), "plaintext leaked: {json}");
            assert!(!json.contains("sealed"), "a sealed blob leaked: {json}");
            assert!(json.contains("\"hasSecret\":true"), "{json}");
        }
        // The core still has it, sealed, and can open it for a run.
        let stored = state.with_store(|s| s.get_document(&saved.id)).unwrap();
        let DocumentBody::Workflow(body) = &stored.body else { panic!("not a workflow") };
        let NodeConfig::Email(email) = &body.nodes[0].config else { panic!("not an email node") };
        assert_eq!(secrets::reveal(&state, &email.password).unwrap().as_deref(), Some("hunter2"));
    }

    #[tokio::test]
    async fn a_run_reads_the_saved_document_not_the_callers() {
        let state = AppState::new(Store::open_in_memory().unwrap(), Box::new(MemoryKeyProvider::default()));
        let runs = WorkflowRuns::default();
        struct Quiet;
        impl WorkflowHost for Quiet {
            fn emit(&self, _event: WorkflowEvent) {}
            fn notify(&self, _title: &str, _body: &str) -> AppResult<()> {
                Ok(())
            }
        }
        // An empty workflow has nothing to run.
        let empty: Document = serde_json::from_value(serde_json::json!({
            "id": "", "kind": "workflow", "connectionId": null, "name": "Empty", "tags": [], "createdAt": "", "updatedAt": "",
            "body": { "kind": "workflow", "data": {} }
        }))
        .unwrap();
        let saved = crate::services::documents::save(&state, &empty).unwrap();
        let options = RunOptions { run_id: "r1".into(), target: None, trigger_node: None, trigger: RunTrigger::Manual, step_delay_ms: 0 };
        let err = run_saved(&state, &Quiet, &runs, &saved.id, options).await.err();
        assert!(matches!(err, Some(AppError::InvalidInput { .. })));
        let options = RunOptions { run_id: "r2".into(), target: None, trigger_node: None, trigger: RunTrigger::Manual, step_delay_ms: 0 };
        assert!(matches!(run_saved(&state, &Quiet, &runs, "nope", options).await.err(), Some(AppError::NotFound { .. })));
    }

    #[test]
    fn a_run_id_can_only_be_live_once_and_stops_by_id() {
        let runs = WorkflowRuns::default();
        let token = runs.begin("r1", "w1").unwrap();
        assert!(runs.begin("r1", "w1").is_err());
        assert!(runs.is_busy("w1") && !runs.is_busy("w2"));
        assert!(runs.cancel("r1") && token.is_cancelled());
        runs.end("r1");
        assert!(!runs.is_busy("w1"));
        assert!(!runs.cancel("r1"), "stopping a finished run is a no-op");
        assert!(runs.begin("r1", "w1").is_ok(), "the id is free again");
    }
}
