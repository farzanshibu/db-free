// SOT: library-commands, ipc-saved-queries, ipc-documents, ipc-library-folders, ipc-saved-query-versions, ipc-home-pins, ipc-home-recents

use crate::error::AppResult;
use crate::guard;
use crate::model::{
    Document, DocumentKind, HomeEntity, HomeItemKind, HomePin, HomeRecent, LibraryFolder, LibraryLayout, LibraryScope, SavedQuery,
    SavedQueryVersion,
};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use tauri::State;
use ts_rs::TS;

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveQueryRequest {
    pub query: SavedQuery,
    /// File the query in this folder. Omitted: a saved query stays where it is
    /// (top level when new).
    #[serde(default)]
    #[ts(optional)]
    pub folder_id: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IdRequest {
    pub id: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ListDocumentsRequest {
    pub kind: DocumentKind,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveDocumentRequest {
    pub document: Document,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenameRequest {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateFolderRequest {
    pub scope: LibraryScope,
    pub parent_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MoveFolderRequest {
    pub id: String,
    /// None = the top level.
    pub parent_id: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MoveItemRequest {
    pub scope: LibraryScope,
    pub item_id: String,
    /// None = the top level.
    pub folder_id: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateDocumentRequest {
    pub id: String,
    /// Copy under this connection; None = keep the original's.
    pub target_connection_id: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateQueryRequest {
    pub id: String,
    /// Copy under this connection; None = keep the original's.
    pub target_connection_id: Option<String>,
    /// Name for the copy; None = "<name> (copy)".
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportQueriesRequest {
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HomeEntityRequest {
    pub entity: HomeEntity,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UnpinRequest {
    pub kind: HomeItemKind,
    pub item_id: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReorderPinsRequest {
    /// Pin ids in the order the board should show them.
    pub ids: Vec<String>,
}

#[tauri::command]
pub async fn list_saved_queries(state: State<'_, AppState>) -> AppResult<Vec<SavedQuery>> {
    guard::local("list_saved_queries", async { services::saved_queries::list(&state) }).await
}

#[tauri::command]
pub async fn save_saved_query(state: State<'_, AppState>, req: SaveQueryRequest) -> AppResult<SavedQuery> {
    guard::local("save_saved_query", async { services::saved_queries::save(&state, &req.query, req.folder_id.as_deref()) }).await
}

#[tauri::command]
pub async fn delete_saved_query(state: State<'_, AppState>, req: IdRequest) -> AppResult<()> {
    guard::local("delete_saved_query", async { services::saved_queries::delete(&state, &req.id) }).await
}

#[tauri::command]
pub async fn rename_saved_query(state: State<'_, AppState>, req: RenameRequest) -> AppResult<SavedQuery> {
    guard::local("rename_saved_query", async { services::saved_queries::rename(&state, &req.id, &req.name) }).await
}

#[tauri::command]
pub async fn duplicate_saved_query(state: State<'_, AppState>, req: DuplicateQueryRequest) -> AppResult<SavedQuery> {
    guard::local("duplicate_saved_query", async {
        services::saved_queries::duplicate(&state, &req.id, req.target_connection_id.as_deref(), req.name.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn list_saved_query_versions(state: State<'_, AppState>, req: IdRequest) -> AppResult<Vec<SavedQueryVersion>> {
    guard::local("list_saved_query_versions", async { services::saved_queries::versions(&state, &req.id) }).await
}

// WHAT:  `id` is the version's id, not the query's.
#[tauri::command]
pub async fn restore_saved_query_version(state: State<'_, AppState>, req: IdRequest) -> AppResult<SavedQuery> {
    guard::local("restore_saved_query_version", async { services::saved_queries::restore_version(&state, &req.id) }).await
}

// WHAT:  The text of one `.sql` file holding the chosen queries; the UI asks the
//        user where to save it and writes it with `save_sql_file`.
#[tauri::command]
pub async fn export_saved_queries(state: State<'_, AppState>, req: ExportQueriesRequest) -> AppResult<String> {
    guard::local("export_saved_queries", async { services::library::export_saved_queries(&state, &req.ids) }).await
}

#[tauri::command]
pub async fn list_documents(state: State<'_, AppState>, req: ListDocumentsRequest) -> AppResult<Vec<Document>> {
    guard::local("list_documents", async { services::documents::list(&state, req.kind) }).await
}

#[tauri::command]
pub async fn save_document(state: State<'_, AppState>, req: SaveDocumentRequest) -> AppResult<Document> {
    guard::local("save_document", async { services::documents::save(&state, &req.document) }).await
}

#[tauri::command]
pub async fn delete_document(state: State<'_, AppState>, req: IdRequest) -> AppResult<()> {
    guard::local("delete_document", async { services::documents::delete(&state, &req.id) }).await
}

#[tauri::command]
pub async fn rename_document(state: State<'_, AppState>, req: RenameRequest) -> AppResult<Document> {
    guard::local("rename_document", async { services::documents::rename(&state, &req.id, &req.name) }).await
}

#[tauri::command]
pub async fn duplicate_document(state: State<'_, AppState>, req: DuplicateDocumentRequest) -> AppResult<Document> {
    guard::local("duplicate_document", async { services::documents::duplicate(&state, &req.id, req.target_connection_id.as_deref()) }).await
}

#[tauri::command]
pub async fn list_library(state: State<'_, AppState>) -> AppResult<LibraryLayout> {
    guard::local("list_library", async { services::library::layout(&state) }).await
}

#[tauri::command]
pub async fn create_library_folder(state: State<'_, AppState>, req: CreateFolderRequest) -> AppResult<LibraryFolder> {
    guard::local("create_library_folder", async { services::library::create_folder(&state, req.scope, req.parent_id.as_deref(), &req.name) }).await
}

#[tauri::command]
pub async fn rename_library_folder(state: State<'_, AppState>, req: RenameRequest) -> AppResult<LibraryFolder> {
    guard::local("rename_library_folder", async { services::library::rename_folder(&state, &req.id, &req.name) }).await
}

#[tauri::command]
pub async fn move_library_folder(state: State<'_, AppState>, req: MoveFolderRequest) -> AppResult<LibraryFolder> {
    guard::local("move_library_folder", async { services::library::move_folder(&state, &req.id, req.parent_id.as_deref()) }).await
}

// WHAT:  Deleting a folder keeps what is in it: items and subfolders move up a level.
#[tauri::command]
pub async fn delete_library_folder(state: State<'_, AppState>, req: IdRequest) -> AppResult<()> {
    guard::local("delete_library_folder", async { services::library::delete_folder(&state, &req.id) }).await
}

#[tauri::command]
pub async fn move_library_item(state: State<'_, AppState>, req: MoveItemRequest) -> AppResult<()> {
    guard::local("move_library_item", async { services::library::move_item(&state, req.scope, &req.item_id, req.folder_id.as_deref()) }).await
}

#[tauri::command]
pub async fn list_pins(state: State<'_, AppState>) -> AppResult<Vec<HomePin>> {
    guard::local("list_pins", async { services::home::pins(&state) }).await
}

#[tauri::command]
pub async fn pin_item(state: State<'_, AppState>, req: HomeEntityRequest) -> AppResult<()> {
    guard::local("pin_item", async { services::home::pin(&state, &req.entity) }).await
}

#[tauri::command]
pub async fn unpin_item(state: State<'_, AppState>, req: UnpinRequest) -> AppResult<()> {
    guard::local("unpin_item", async { services::home::unpin(&state, req.kind, &req.item_id) }).await
}

#[tauri::command]
pub async fn reorder_pins(state: State<'_, AppState>, req: ReorderPinsRequest) -> AppResult<()> {
    guard::local("reorder_pins", async { services::home::reorder(&state, &req.ids) }).await
}

#[tauri::command]
pub async fn list_recents(state: State<'_, AppState>) -> AppResult<Vec<HomeRecent>> {
    guard::local("list_recents", async { services::home::recents(&state) }).await
}

#[tauri::command]
pub async fn record_recent(state: State<'_, AppState>, req: HomeEntityRequest) -> AppResult<()> {
    guard::local("record_recent", async { services::home::record(&state, &req.entity) }).await
}

#[tauri::command]
pub async fn clear_recents(state: State<'_, AppState>) -> AppResult<()> {
    guard::local("clear_recents", async { services::home::clear_recents(&state) }).await
}
