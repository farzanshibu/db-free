// SOT: session-commands, ipc-close-session

use crate::error::AppResult;
use crate::guard;
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use ts_rs::TS;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CloseSessionRequest {
    pub connection_id: String,
    /// The key the tab's runs carried (`ExecuteQueryRequest::session_key`).
    pub session_key: String,
}

// WHAT:  Gives back the dedicated connection of a query tab that was closed.
// WHY:   Session lane: it runs no user-authored SQL, so there is nothing to classify.
// WHERE: src-tauri/src/services/sessions.rs, src/stores/sessions.ts
#[tauri::command]
pub async fn close_session(state: State<'_, AppState>, req: CloseSessionRequest) -> AppResult<()> {
    guard::session(&state, &req.connection_id, |ctx| async move { services::sessions::close(&ctx, &req.session_key).await }).await
}
