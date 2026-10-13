// SOT: table-tag-commands, ipc-table-tags

use crate::commands::library::IdRequest;
use crate::error::AppResult;
use crate::guard;
use crate::model::{TableTag, TableTagInput, TagState};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use ts_rs::TS;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ListTagsRequest {
    pub connection_id: String,
    /// The database the session is on ("" for engines without one).
    pub database: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveTagRequest {
    pub tag: TableTagInput,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReorderTagsRequest {
    pub connection_id: String,
    pub database: String,
    pub schema: String,
    /// Tag ids in their new order.
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TagTableRequest {
    pub tag_id: String,
    /// The table's name within the tag's schema.
    pub table: String,
    /// True pins the table to the tag, false takes it off.
    pub tagged: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TagCollapseRequest {
    pub connection_id: String,
    pub database: String,
    pub schema: String,
    /// None folds the scope's fixed "Untagged" group.
    pub tag_id: Option<String>,
    pub collapsed: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MoveTaggedTableRequest {
    pub connection_id: String,
    pub database: String,
    pub schema: String,
    pub from: String,
    /// The new name after a rename; None after a drop.
    pub to: Option<String>,
}

#[tauri::command]
pub async fn list_table_tags(state: State<'_, AppState>, req: ListTagsRequest) -> AppResult<TagState> {
    guard::local("list_table_tags", async { services::tags::state(&state, &req.connection_id, &req.database) }).await
}

#[tauri::command]
pub async fn save_table_tag(state: State<'_, AppState>, req: SaveTagRequest) -> AppResult<TableTag> {
    guard::local("save_table_tag", async { services::tags::save(&state, &req.tag) }).await
}

#[tauri::command]
pub async fn delete_table_tag(state: State<'_, AppState>, req: IdRequest) -> AppResult<()> {
    guard::local("delete_table_tag", async { services::tags::delete(&state, &req.id) }).await
}

#[tauri::command]
pub async fn reorder_table_tags(state: State<'_, AppState>, req: ReorderTagsRequest) -> AppResult<()> {
    guard::local("reorder_table_tags", async {
        services::tags::reorder(&state, &req.connection_id, &req.database, &req.schema, &req.ids)
    })
    .await
}

#[tauri::command]
pub async fn tag_table(state: State<'_, AppState>, req: TagTableRequest) -> AppResult<()> {
    guard::local("tag_table", async { services::tags::tag_table(&state, &req.tag_id, &req.table, req.tagged) }).await
}

#[tauri::command]
pub async fn set_tag_collapsed(state: State<'_, AppState>, req: TagCollapseRequest) -> AppResult<()> {
    guard::local("set_tag_collapsed", async {
        services::tags::set_collapsed(&state, &req.connection_id, &req.database, &req.schema, req.tag_id.as_deref(), req.collapsed)
    })
    .await
}

#[tauri::command]
pub async fn move_tagged_table(state: State<'_, AppState>, req: MoveTaggedTableRequest) -> AppResult<()> {
    guard::local("move_tagged_table", async {
        services::tags::move_table(&state, &req.connection_id, &req.database, &req.schema, &req.from, req.to.as_deref())
    })
    .await
}
