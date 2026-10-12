// SOT: connection-org-commands, ipc-connection-tags, ipc-connection-folders

use crate::error::AppResult;
use crate::guard;
use crate::model::{ConnectionFolder, ConnectionOrg, ConnectionSummary, ConnectionTag, ConnectionTagColor};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use ts_rs::TS;

// ============================================================================
// WHAT:  IPC for the connections page's tags and folders.
// WHY:   Thin on purpose: input is bounded in the store, the block times each
//        call, and the service owns the rules.
// WHERE: src/features/connections/useConnectionOrg.ts
// ============================================================================

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveConnectionTagRequest {
    /// None creates a tag; Some renames / recolours it.
    pub id: Option<String>,
    pub name: String,
    pub color: ConnectionTagColor,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveConnectionFolderRequest {
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OrgIdRequest {
    pub id: String,
}

// WHAT:  Replace a connection's tags and / or folders. A side left null is left alone.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetConnectionOrgRequest {
    pub connection_id: String,
    pub tag_ids: Option<Vec<String>>,
    pub folder_ids: Option<Vec<String>>,
}

#[tauri::command]
pub async fn list_connection_org(state: State<'_, AppState>) -> AppResult<ConnectionOrg> {
    guard::local("list_connection_org", async { services::connection_org::list(&state) }).await
}

#[tauri::command]
pub async fn save_connection_tag(state: State<'_, AppState>, req: SaveConnectionTagRequest) -> AppResult<ConnectionTag> {
    guard::local("save_connection_tag", async {
        services::connection_org::save_tag(&state, req.id.as_deref(), &req.name, req.color)
    })
    .await
}

#[tauri::command]
pub async fn delete_connection_tag(state: State<'_, AppState>, req: OrgIdRequest) -> AppResult<()> {
    guard::local("delete_connection_tag", async { services::connection_org::delete_tag(&state, &req.id) }).await
}

#[tauri::command]
pub async fn save_connection_folder(state: State<'_, AppState>, req: SaveConnectionFolderRequest) -> AppResult<ConnectionFolder> {
    guard::local("save_connection_folder", async {
        services::connection_org::save_folder(&state, req.id.as_deref(), &req.name)
    })
    .await
}

#[tauri::command]
pub async fn delete_connection_folder(state: State<'_, AppState>, req: OrgIdRequest) -> AppResult<()> {
    guard::local("delete_connection_folder", async { services::connection_org::delete_folder(&state, &req.id) }).await
}

#[tauri::command]
pub async fn set_connection_org(state: State<'_, AppState>, req: SetConnectionOrgRequest) -> AppResult<ConnectionSummary> {
    guard::local("set_connection_org", async {
        services::connection_org::set_membership(&state, &req.connection_id, req.tag_ids.as_deref(), req.folder_ids.as_deref())
    })
    .await
}
