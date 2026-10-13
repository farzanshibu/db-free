// SOT: workflow-commands, ipc-run-workflow, ipc-cancel-workflow, workflow-event-host, workflow-schedule-tick, ipc-workflow-schedules, ipc-regex-test, ipc-last-notifier-config, workflow-runner

use crate::error::{AppError, AppResult};
use crate::guard;
use crate::model::documents::WorkflowRunReport;
use crate::model::workflow::{NotifierKind, RegexMode, RegexTestResult, RunTrigger, WorkflowEvent};
use crate::model::{NodeConfig, WorkflowScheduleStatus};
use crate::services;
use crate::services::workflows::{Fire, RunOptions, WorkflowHost, WorkflowRuns};
use crate::state::AppState;
use serde::Deserialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
use ts_rs::TS;

// WHAT:  The IPC surface for workflows: run (a node, or the whole graph), stop, the
//        schedule list, the regex live test, and "the last email / Slack setup".
// WHY:   The engine runs in the core so a schedule can fire with every window
//        closed. This layer is the only one that may hold an `AppHandle`: it turns
//        engine events into app events and `notify` into a desktop notification.
// HOW:   A run reports node by node on the app event `workflow:event` (filter on
//        `runId`); the command's own result is the final report. Each node's SQL
//        still passes through `guard::statement` inside the service.
// WHERE: src-tauri/src/services/workflows (engine), src/lib/ipc.ts (onWorkflowEvent)

/// Event the canvas listens on while a run is live.
const WORKFLOW_EVENT: &str = "workflow:event";

// WHAT:  The engine's window on the app.
struct TauriHost {
    app: AppHandle,
}

impl WorkflowHost for TauriHost {
    // A dropped frame costs a status update, never the run.
    fn emit(&self, event: WorkflowEvent) {
        let _ = self.app.emit(WORKFLOW_EVENT, event);
    }

    fn notify(&self, title: &str, body: &str) -> AppResult<()> {
        self.app
            .notification()
            .builder()
            .title(if title.is_empty() { "Workflow" } else { title })
            .body(body)
            .show()
            .map_err(|e| AppError::driver(format!("Could not show the notification: {e}")))
    }
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunWorkflowRequest {
    pub id: String,
    /// Minted by the caller so it can filter events and stop the run; one is made when absent.
    #[serde(default)]
    #[ts(optional)]
    pub run_id: Option<String>,
    /// Run just this node and everything it needs; absent runs the whole workflow.
    #[serde(default)]
    #[ts(optional)]
    pub node_id: Option<String>,
    /// Pause between levels so a person can watch the data move.
    #[serde(default)]
    #[ts(optional)]
    pub step_delay_ms: Option<u32>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CancelWorkflowRequest {
    pub run_id: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TestRegexRequest {
    pub pattern: String,
    pub flags: String,
    pub mode: RegexMode,
    pub replacement: String,
    pub sample: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LastNodeConfigRequest {
    pub kind: NotifierKind,
}

#[tauri::command]
pub async fn run_workflow(
    app: AppHandle,
    state: State<'_, AppState>,
    runs: State<'_, WorkflowRuns>,
    req: RunWorkflowRequest,
) -> AppResult<WorkflowRunReport> {
    guard::local("run_workflow", async {
        let host = TauriHost { app };
        let options = RunOptions {
            run_id: req.run_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            target: req.node_id.clone(),
            trigger_node: None,
            trigger: RunTrigger::Manual,
            step_delay_ms: u64::from(req.step_delay_ms.unwrap_or(0)),
        };
        services::workflows::run_saved(&state, &host, &runs, &req.id, options).await
    })
    .await
}

// WHAT:  Stops a run. Answers whether it was still live: Stop and completion race.
#[tauri::command]
pub async fn cancel_workflow(runs: State<'_, WorkflowRuns>, req: CancelWorkflowRequest) -> AppResult<bool> {
    guard::local("cancel_workflow", async { Ok(runs.cancel(&req.run_id)) }).await
}

// WHAT:  Settings → Schedules: every Schedule node, its workflow and connection, mode and next fire.
#[tauri::command]
pub async fn list_workflow_schedules(state: State<'_, AppState>) -> AppResult<Vec<WorkflowScheduleStatus>> {
    guard::local("list_workflow_schedules", async { services::workflows::list_schedules(&state) }).await
}

#[tauri::command]
pub async fn test_workflow_regex(req: TestRegexRequest) -> AppResult<RegexTestResult> {
    guard::local("test_workflow_regex", async {
        Ok(services::workflows::test_regex(&req.pattern, &req.flags, req.mode, &req.replacement, &req.sample))
    })
    .await
}

// WHAT:  The most recently saved Email / Slack setup, for a new node to start from.
//        Secrets stay sealed on this side; the node carries `inheritLast` instead.
#[tauri::command]
pub async fn workflow_last_node_config(state: State<'_, AppState>, req: LastNodeConfigRequest) -> AppResult<Option<NodeConfig>> {
    guard::local("workflow_last_node_config", async { services::workflows::last_node_config(&state, req.kind) }).await
}

// WHAT:  Starts the schedule clock: once a second, run whatever Schedule nodes are due.
// WHY:   Called once from `setup`. It lives as long as the app, so a schedule fires
//        with the workflow tab (or every window) closed.
// HOW:   Each fire runs on its own task, so a slow workflow never delays the clock.
pub fn start_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let Some(state) = app.try_state::<AppState>() else { continue };
            for fire in services::workflows::due_fires(&state) {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { run_fire(app, fire).await });
            }
        }
    });
}

async fn run_fire(app: AppHandle, fire: Fire) {
    let (Some(state), Some(runs)) = (app.try_state::<AppState>(), app.try_state::<WorkflowRuns>()) else {
        return;
    };
    let host = TauriHost { app: app.clone() };
    match services::workflows::run_fire(&state, &host, &runs, &fire).await {
        Ok(Some(report)) => log::info!("scheduled workflow {} finished: {:?}", fire.workflow_id, report.status),
        Ok(None) => {}
        Err(err) => log::warn!("scheduled workflow {} could not run: {err}", fire.workflow_id),
    }
}
