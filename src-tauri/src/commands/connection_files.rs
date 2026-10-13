// SOT: connection-file-commands, ipc-connection-export, ipc-connection-import, ipc-open-database-file, ipc-ssh-config, ipc-demo-connection

use crate::error::{AppError, AppResult};
use crate::guard;
use crate::model::{ConnectionExportReport, ConnectionImportReport, ConnectionSummary, SshConfigHost};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::{AppHandle, Manager, State};
use ts_rs::TS;

// ============================================================================
// WHAT:  IPC for connections that move through files: export / import, a
//        database file opened from the OS or a drop, the sample database, and
//        the hosts of ~/.ssh/config.
// WHY:   Paths are chosen by the user in a native dialog and handed here; the
//        file is read and written in Rust, so a decrypted password in an export
//        never exists in the webview.
// WHERE: src/features/connections/ConnectionTransferDialogs.tsx, ConnectionEvents.tsx
// ============================================================================

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportConnectionsRequest {
    pub ids: Vec<String>,
    /// Off by default: without it the file carries no secret at all.
    pub include_passwords: bool,
    pub path: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportConnectionsRequest {
    pub path: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenDatabaseFileRequest {
    pub path: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveTransientRequest {
    pub id: String,
    /// New name for the saved connection; None keeps the file's name.
    pub name: Option<String>,
}

#[tauri::command]
pub async fn export_connections(state: State<'_, AppState>, req: ExportConnectionsRequest) -> AppResult<ConnectionExportReport> {
    guard::local("export_connections", async {
        services::connection_transfer::export(&state, &req.ids, req.include_passwords, &req.path)
    })
    .await
}

#[tauri::command]
pub async fn import_connections(state: State<'_, AppState>, req: ImportConnectionsRequest) -> AppResult<ConnectionImportReport> {
    guard::local("import_connections", async { services::connection_transfer::import(&state, &req.path) }).await
}

#[tauri::command]
pub async fn open_database_file(state: State<'_, AppState>, req: OpenDatabaseFileRequest) -> AppResult<ConnectionSummary> {
    guard::local("open_database_file", async {
        services::connection_files::open_database_file(&state, std::path::Path::new(req.path.trim()))
    })
    .await
}

#[tauri::command]
pub async fn save_transient_connection(state: State<'_, AppState>, req: SaveTransientRequest) -> AppResult<ConnectionSummary> {
    guard::local("save_transient_connection", async {
        services::connection_files::save_transient(&state, &req.id, req.name.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn list_ssh_config_hosts() -> AppResult<Vec<SshConfigHost>> {
    guard::local("list_ssh_config_hosts", async { services::connection_files::ssh_config_hosts() }).await
}

// WHAT:  The sample e-commerce database as a saved connection ("Try the demo").
#[tauri::command]
pub async fn create_demo_connection(app: AppHandle, state: State<'_, AppState>) -> AppResult<ConnectionSummary> {
    let data_dir = app.path().app_data_dir().map_err(|e| AppError::internal(format!("no app data directory: {e}")))?;
    guard::local("create_demo_connection", async { services::connection_files::create_demo(&state, &data_dir) }).await
}
