// SOT: backup-commands, ipc-backup, ipc-restore, backup-progress-event, backup-cancel, backup-run-registry

use crate::error::{AppError, AppResult};
use crate::guard::{self, ToolAccess};
use crate::model::{BackupEvent, BackupOptions, BackupReport, BackupSupport};
use crate::services;
use crate::services::backup::ToolSink;
use crate::state::AppState;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::oneshot;
use ts_rs::TS;

// WHAT:  The IPC surface for native backup / restore: which tools exist, run a
//        backup, run a restore, stop a run.
// WHY:   A dump takes minutes, so like the agent the reply is a stream of
//        events plus one final value; and like the agent this is the only
//        layer that may hold an `AppHandle`.
// HOW:   Runs are not bounded by the guard's timeout (a 40 GB dump is not a
//        hung request); the user's Cancel is the bound instead.
// WHERE: src-tauri/src/services/backup.rs, src/lib/ipc.ts (onBackupProgress)

/// Event the Backup / Restore dialog listens on while a run is live.
const BACKUP_EVENT: &str = "backup:progress";

// WHAT:  The cancel switch of every run that is in flight, by run id.
// HOW:   Managed by Tauri (src-tauri/src/lib.rs). The lock is only held to
//        insert or take a sender, never across an await.
#[derive(Default)]
pub struct BackupRuns(Mutex<HashMap<String, oneshot::Sender<()>>>);

impl BackupRuns {
    fn start(&self, run_id: &str) -> AppResult<oneshot::Receiver<()>> {
        let (tx, rx) = oneshot::channel();
        let mut runs = self.0.lock().map_err(|_| AppError::internal("backup run lock poisoned"))?;
        if runs.contains_key(run_id) {
            return Err(AppError::invalid_input("That run is already in progress."));
        }
        runs.insert(run_id.to_string(), tx);
        Ok(rx)
    }

    fn finish(&self, run_id: &str) {
        if let Ok(mut runs) = self.0.lock() {
            runs.remove(run_id);
        }
    }

    fn cancel(&self, run_id: &str) -> AppResult<()> {
        let sender = self.0.lock().map_err(|_| AppError::internal("backup run lock poisoned"))?.remove(run_id);
        if let Some(tx) = sender {
            let _ = tx.send(());
        }
        // A run that already ended needs no stopping; saying so would be noise.
        Ok(())
    }
}

// WHAT:  Forwards tool output to the window. A dropped frame costs a log line,
//        never the run — same rule as the updater and the agent.
struct WindowSink {
    app: AppHandle,
    run_id: String,
}

impl ToolSink for WindowSink {
    fn log(&self, line: &str) {
        let _ = self.app.emit(BACKUP_EVENT, BackupEvent::Log { run_id: self.run_id.clone(), line: line.to_string() });
    }

    fn progress(&self, bytes: u64, total: Option<u64>) {
        let _ = self.app.emit(BACKUP_EVENT, BackupEvent::Progress { run_id: self.run_id.clone(), bytes, total });
    }
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DetectToolsRequest {
    /// None lists every tool (Settings → Advanced).
    pub connection_id: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupRequest {
    pub connection_id: String,
    /// Minted by the caller so it can filter events and cancel.
    pub run_id: String,
    /// Absolute path picked in the native save dialog.
    pub path: String,
    pub options: BackupOptions,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RestoreRequest {
    pub connection_id: String,
    pub run_id: String,
    /// Absolute path picked in the native open dialog.
    pub path: String,
    pub options: BackupOptions,
    /// The user's explicit yes to overwriting the target (guard step 11).
    pub confirm_destructive: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupRunRequest {
    pub run_id: String,
}

#[tauri::command]
pub async fn detect_native_tools(state: State<'_, AppState>, req: DetectToolsRequest) -> AppResult<BackupSupport> {
    guard::local("detect_native_tools", async move {
        services::backup::support(&state, req.connection_id.as_deref()).await
    })
    .await
}

#[tauri::command]
pub async fn backup_database(
    app: AppHandle,
    state: State<'_, AppState>,
    runs: State<'_, BackupRuns>,
    req: BackupRequest,
) -> AppResult<BackupReport> {
    let cancel = runs.start(&req.run_id)?;
    let sink: Arc<dyn ToolSink> = Arc::new(WindowSink { app, run_id: req.run_id.clone() });
    let result = guard::local("backup_database", async {
        let connection = guard::native_tool(&state, &req.connection_id, ToolAccess::Read)?; // 11
        services::backup::backup(&state, &connection, &req.path, &req.options, sink, cancel).await
    })
    .await;
    runs.finish(&req.run_id);
    result
}

#[tauri::command]
pub async fn restore_database(
    app: AppHandle,
    state: State<'_, AppState>,
    runs: State<'_, BackupRuns>,
    req: RestoreRequest,
) -> AppResult<BackupReport> {
    // Gate first: a refused restore must not occupy a run slot.
    let access = ToolAccess::Replace { confirmed: req.confirm_destructive };
    let connection = guard::local("restore_database:gate", async { guard::native_tool(&state, &req.connection_id, access) }).await?; // 11
    let cancel = runs.start(&req.run_id)?;
    let sink: Arc<dyn ToolSink> = Arc::new(WindowSink { app, run_id: req.run_id.clone() });
    let result = guard::local("restore_database", async {
        services::backup::restore(&state, &connection, &req.path, &req.options, sink, cancel).await
    })
    .await;
    runs.finish(&req.run_id);
    result
}

/// Stop a running backup or restore. The tool is killed; a partial backup file is removed.
#[tauri::command]
pub async fn cancel_backup(runs: State<'_, BackupRuns>, req: BackupRunRequest) -> AppResult<()> {
    guard::local("cancel_backup", async move { runs.cancel(&req.run_id) }).await
}
