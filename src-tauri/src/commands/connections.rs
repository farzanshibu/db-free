// SOT: connection-commands, ipc-connections

use crate::error::AppResult;
use crate::guard;
use crate::integrations::SessionInfo;
use crate::model::{ConnectionFieldIssue, ConnectionInput, ConnectionSummary, CredentialStatus};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use ts_rs::TS;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveConnectionRequest {
    pub id: Option<String>,
    pub input: ConnectionInput,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionIdRequest {
    pub id: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectRequest {
    pub id: String,
    /// Session-only override of the saved database (sidebar switcher).
    pub database: Option<String>,
}

#[tauri::command]
pub async fn list_connections(state: State<'_, AppState>) -> AppResult<Vec<ConnectionSummary>> {
    guard::local("list_connections", async { services::connection::list(&state) }).await
}

#[tauri::command]
pub async fn save_connection(
    state: State<'_, AppState>,
    req: SaveConnectionRequest,
) -> AppResult<ConnectionSummary> {
    req.input.validate()?;
    guard::local("save_connection", async {
        services::connection::save(&state, req.id.as_deref(), &req.input)
    })
    .await
}

#[tauri::command]
pub async fn delete_connection(state: State<'_, AppState>, req: ConnectionIdRequest) -> AppResult<()> {
    guard::local("delete_connection", services::connection::delete(&state, &req.id)).await
}

// WHAT:  A copy of a saved connection ("name (copy)") with its sealed password,
//        SSH settings and tags / folders.
// WHERE: src/features/connections/ConnectionsPage.tsx (card menu)
#[tauri::command]
pub async fn duplicate_connection(state: State<'_, AppState>, req: ConnectionIdRequest) -> AppResult<ConnectionSummary> {
    guard::local("duplicate_connection", async { services::connection::duplicate(&state, &req.id) }).await
}

#[tauri::command]
pub async fn test_connection(state: State<'_, AppState>, req: SaveConnectionRequest) -> AppResult<()> {
    req.input.validate()?;
    guard::local(
        "test_connection",
        services::connection::test(&state, req.id.as_deref(), &req.input),
    )
    .await
}

// WHAT:  Every problem with a form's input, each tied to its field.
// WHY:   The form paints a message under the field that caused it. The rules live
//        in `ConnectionInput::check`, so the form cannot disagree with `save`.
#[tauri::command]
pub async fn validate_connection(req: SaveConnectionRequest) -> AppResult<Vec<ConnectionFieldIssue>> {
    guard::local("validate_connection", async { Ok(services::connection::validate(&req.input)) }).await
}

#[tauri::command]
pub async fn connect(state: State<'_, AppState>, req: ConnectRequest) -> AppResult<ConnectionSummary> {
    guard::local("connect", services::connection::connect(&state, &req.id, req.database.as_deref())).await
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>, req: ConnectionIdRequest) -> AppResult<()> {
    guard::local("disconnect", services::connection::disconnect(&state, &req.id)).await
}

#[tauri::command]
pub async fn active_sessions(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    guard::local("active_sessions", async { Ok(services::connection::active_sessions(&state).await) }).await
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionRequest {
    pub connection_id: String,
}

#[tauri::command]
pub async fn describe_session(state: State<'_, AppState>, req: SessionRequest) -> AppResult<SessionInfo> {
    guard::session(&state, &req.connection_id, |ctx| async move {
        services::connection::describe(&ctx).await
    })
    .await
}

// WHAT:  Round-trip to the database of a live session, in milliseconds.
// WHY:   The status bar polls it to show latency and to notice a dropped
//        session before the user's next query does.
// WHERE: src/features/shell/ConnectionHealth.tsx
#[tauri::command]
pub async fn ping_session(state: State<'_, AppState>, req: SessionRequest) -> AppResult<u64> {
    guard::session(&state, &req.connection_id, |ctx| async move { services::connection::ping(&ctx).await }).await
}

// WHAT:  Where the master key lives and whether saved passwords can still be opened.
// WHERE: src/features/connections/CredentialBanner.tsx
#[tauri::command]
pub async fn credential_status(state: State<'_, AppState>) -> AppResult<CredentialStatus> {
    guard::local("credential_status", async { services::connection::credential_status(&state) }).await
}

// WHAT:  Gives up on saved passwords whose key is gone: clears them and starts a new key.
// WHY:   Only valid while the status says `lost`; with the key intact it refuses,
//        so it cannot be used to wipe working credentials.
#[tauri::command]
pub async fn reset_credentials(state: State<'_, AppState>) -> AppResult<CredentialStatus> {
    guard::local("reset_credentials", async { services::connection::reset_credentials(&state) }).await
}
