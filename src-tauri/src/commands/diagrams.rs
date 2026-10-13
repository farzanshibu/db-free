// SOT: diagram-commands, ipc-diagram-customizations, ipc-document-versions, ipc-diagram-ddl, ipc-diagram-export

use crate::error::AppResult;
use crate::guard;
use crate::model::{
    DiagramBody, DiagramCustomization, DiagramCustomizationKey, DiagramDdl, DiagramExportFormat, Document, DocumentVersionSummary,
};
use crate::services;
use crate::state::AppState;
use serde::Deserialize;
use std::path::Path;
use tauri::State;
use ts_rs::TS;

// WHAT:  The commands behind the ER diagram and the schema designer that are not
//        a plain document save: customizations, version history, SQL, image export.
// WHY:   All of them touch only the app's own store or a file the user picked, so
//        each takes the local lane of the block (`guard::local`).
// WHERE: src/features/diagrams, src/lib/ipc.ts

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramScopeRequest {
    pub connection_id: String,
    /// None = the diagram of every schema.
    pub schema: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveDiagramCustomizationsRequest {
    pub connection_id: String,
    pub schema: Option<String>,
    pub upserts: Vec<DiagramCustomization>,
    pub removals: Vec<DiagramCustomizationKey>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutosaveDocumentRequest {
    pub document: Document,
    /// Also file the saved body as an automatic version.
    pub take_version: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckpointDocumentRequest {
    pub document: Document,
    /// Blank = "Checkpoint".
    pub name: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentHistoryRequest {
    pub document_id: String,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentVersionRequest {
    pub document_id: String,
    pub version: u32,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramDdlRequest {
    /// The diagram's connection; its engine picks the dialect. None = ANSI.
    pub connection_id: Option<String>,
    pub body: DiagramBody,
    /// Only this table (and the keys it owns). None = the whole diagram.
    pub table_id: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveDiagramExportRequest {
    pub path: String,
    pub format: DiagramExportFormat,
    /// PNG: base64 of the file. SVG: the markup.
    pub data: String,
}

#[tauri::command]
pub async fn list_diagram_customizations(state: State<'_, AppState>, req: DiagramScopeRequest) -> AppResult<Vec<DiagramCustomization>> {
    guard::local("list_diagram_customizations", async {
        services::diagrams::list_customizations(&state, &req.connection_id, req.schema.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn save_diagram_customizations(state: State<'_, AppState>, req: SaveDiagramCustomizationsRequest) -> AppResult<()> {
    guard::local("save_diagram_customizations", async {
        services::diagrams::save_customizations(&state, &req.connection_id, req.schema.as_deref(), &req.upserts, &req.removals)
    })
    .await
}

// WHAT:  Saves a document and, when asked, files the saved body as an automatic version.
#[tauri::command]
pub async fn autosave_document(state: State<'_, AppState>, req: AutosaveDocumentRequest) -> AppResult<Document> {
    guard::local("autosave_document", async { services::document_versions::autosave(&state, &req.document, req.take_version) }).await
}

#[tauri::command]
pub async fn checkpoint_document(state: State<'_, AppState>, req: CheckpointDocumentRequest) -> AppResult<DocumentVersionSummary> {
    guard::local("checkpoint_document", async { services::document_versions::checkpoint(&state, &req.document, &req.name) }).await
}

#[tauri::command]
pub async fn list_document_versions(state: State<'_, AppState>, req: DocumentHistoryRequest) -> AppResult<Vec<DocumentVersionSummary>> {
    guard::local("list_document_versions", async { services::document_versions::list(&state, &req.document_id) }).await
}

#[tauri::command]
pub async fn restore_document_version(state: State<'_, AppState>, req: DocumentVersionRequest) -> AppResult<Document> {
    guard::local("restore_document_version", async { services::document_versions::restore(&state, &req.document_id, req.version) }).await
}

#[tauri::command]
pub async fn delete_document_version(state: State<'_, AppState>, req: DocumentVersionRequest) -> AppResult<()> {
    guard::local("delete_document_version", async { services::document_versions::delete(&state, &req.document_id, req.version) }).await
}

// WHAT:  The SQL for a diagram (or one table of it) in its connection's dialect.
#[tauri::command]
pub async fn diagram_ddl(state: State<'_, AppState>, req: DiagramDdlRequest) -> AppResult<DiagramDdl> {
    guard::local("diagram_ddl", async {
        Ok(services::diagrams::ddl(&state, req.connection_id.as_deref(), &req.body, req.table_id.as_deref()))
    })
    .await
}

// WHAT:  Writes an exported diagram image to the file the user picked.
#[tauri::command]
pub async fn save_diagram_export(req: SaveDiagramExportRequest) -> AppResult<String> {
    guard::local("save_diagram_export", async move { services::diagrams::save_export(Path::new(&req.path), req.format, &req.data) }).await
}
