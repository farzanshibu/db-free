// SOT: documents-table, dashboards-persistence, workflows-persistence, diagrams-persistence

use crate::error::{AppError, AppResult};
use crate::model::{Document, DocumentBody, DocumentKind, LibraryScope};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, Row};

// WHAT:  One table for every JSON-bodied artefact (dashboard, workflow, diagram).
// WHY:   They share a lifecycle (name, tags, connection scope, timestamps); the
//        typed body is validated by serde on the way in and out.
fn from_row(row: &Row<'_>) -> rusqlite::Result<Document> {
    let kind_raw: String = row.get(1)?;
    let body_raw: String = row.get(4)?;
    let tags: String = row.get(5)?;
    let body: DocumentBody = serde_json::from_str(&body_raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Document {
        id: row.get(0)?,
        kind: DocumentKind::parse(&kind_raw).unwrap_or(DocumentKind::Dashboard),
        connection_id: row.get(2)?,
        name: row.get(3)?,
        body,
        tags: tags.split(',').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect(),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

const COLUMNS: &str = "id, kind, connection_id, name, body, tags, created_at, updated_at";

impl Store {
    pub fn list_documents(&self, kind: DocumentKind) -> AppResult<Vec<Document>> {
        let sql = format!("SELECT {COLUMNS} FROM documents WHERE kind = ?1 ORDER BY lower(name)");
        let mut stmt = self.conn().prepare(&sql).map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![kind.as_str()], from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    pub fn get_document(&self, id: &str) -> AppResult<Document> {
        let sql = format!("SELECT {COLUMNS} FROM documents WHERE id = ?1");
        self.conn()
            .query_row(&sql, params![id], from_row)
            .map_err(|_| AppError::not_found(format!("Document {id} does not exist.")))
    }

    // WHAT:  How many workflows hold a sealed secret (an SMTP password, a Slack webhook).
    // WHY:   Part of `sealed_secret_count`: if the master key is lost while workflows
    //        still hold sealed secrets, a new key would make them unreadable.
    pub fn workflow_sealed_secret_count(&self) -> AppResult<u32> {
        let total: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM documents WHERE kind = 'workflow' AND body LIKE '%\"sealed\":%'", [], |row| row.get(0))
            .map_err(AppError::store)?;
        Ok(u32::try_from(total).unwrap_or(u32::MAX))
    }

    // WHAT:  Forgets the sealed secrets of every workflow, after the master key was lost for good.
    pub fn clear_workflow_sealed_secrets(&self) -> AppResult<()> {
        for doc in self.list_documents(DocumentKind::Workflow)? {
            let DocumentBody::Workflow(mut body) = doc.body else { continue };
            if body.forget_secrets() == 0 {
                continue;
            }
            let json = crate::model::workflow::persisting(|| serde_json::to_string(&DocumentBody::Workflow(body))).map_err(AppError::store)?;
            self.conn().execute("UPDATE documents SET body = ?1 WHERE id = ?2", params![json, doc.id]).map_err(AppError::store)?;
        }
        Ok(())
    }

    pub fn upsert_document(&self, doc: &Document) -> AppResult<Document> {
        let now = now_rfc3339();
        let id = if doc.id.is_empty() { uuid::Uuid::new_v4().to_string() } else { doc.id.clone() };
        // Sealed workflow secrets are written here and nowhere else (model/workflow.rs, `persisting`).
        let body = crate::model::workflow::persisting(|| serde_json::to_string(&doc.body)).map_err(AppError::store)?;
        self.conn()
            .execute(
                "INSERT INTO documents (id, kind, connection_id, name, body, tags, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) \
                 ON CONFLICT(id) DO UPDATE SET connection_id = excluded.connection_id, name = excluded.name, \
                 body = excluded.body, tags = excluded.tags, updated_at = excluded.updated_at",
                params![id, doc.kind.as_str(), doc.connection_id, doc.name.trim(), body, doc.tags.join(","), now],
            )
            .map_err(AppError::store)?;
        self.get_document(&id)
    }

    // WHAT:  Renames without touching the body, so a rename from the sidebar cannot
    //        overwrite an edit the open editor has just saved.
    pub fn rename_document(&self, id: &str, name: &str) -> AppResult<Document> {
        let updated = self
            .conn()
            .execute("UPDATE documents SET name = ?2, updated_at = ?3 WHERE id = ?1", params![id, name.trim(), now_rfc3339()])
            .map_err(AppError::store)?;
        if updated == 0 {
            return Err(AppError::not_found(format!("Document {id} does not exist.")));
        }
        self.get_document(id)
    }

    // WHAT:  Deletes the document and forgets which folder it was filed in.
    pub fn delete_document(&self, id: &str) -> AppResult<()> {
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        for kind in DocumentKind::ALL {
            tx.execute(
                "DELETE FROM library_placements WHERE scope = ?1 AND item_id = ?2",
                params![LibraryScope::of_document(kind).as_str(), id],
            )
            .map_err(AppError::store)?;
        }
        tx.execute("DELETE FROM documents WHERE id = ?1", params![id]).map_err(AppError::store)?;
        tx.commit().map_err(AppError::store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DashboardBody, WorkflowBody, WorkflowStep};

    #[test]
    fn workflow_secrets_are_counted_and_can_be_forgotten() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.sealed_secret_count().unwrap_or(99), 0);
        let doc: Document = serde_json::from_value(serde_json::json!({
            "id": "", "kind": "workflow", "connectionId": null, "name": "Mail", "tags": [], "createdAt": "", "updatedAt": "",
            "body": { "kind": "workflow", "data": { "nodes": [{ "id": "m", "config": { "kind": "email", "password": { "sealed": "c2VhbGVk", "hasSecret": true } } }] } }
        }))
        .unwrap_or_else(|e| panic!("{e}"));
        let saved = store.upsert_document(&doc).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.workflow_sealed_secret_count().unwrap_or(99), 1);
        assert_eq!(store.sealed_secret_count().unwrap_or(99), 1, "the lost-master-key check sees workflow secrets");
        store.clear_sealed_secrets().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.sealed_secret_count().unwrap_or(99), 0);
        let again = store.get_document(&saved.id).unwrap_or_else(|e| panic!("{e}"));
        let json = serde_json::to_string(&again).unwrap_or_default();
        assert!(json.contains("\"hasSecret\":false"), "{json}");
    }

    #[test]
    fn documents_round_trip_by_kind() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let dash = store
            .upsert_document(&Document {
                id: String::new(),
                kind: DocumentKind::Dashboard,
                connection_id: None,
                name: "Dashboard 1".into(),
                body: DocumentBody::Dashboard(DashboardBody::default()),
                tags: Vec::new(),
                created_at: String::new(),
                updated_at: String::new(),
            })
            .unwrap_or_else(|e| panic!("{e}"));
        store
            .upsert_document(&Document {
                id: String::new(),
                kind: DocumentKind::Workflow,
                connection_id: None,
                name: "Workflow 1".into(),
                body: DocumentBody::Workflow(WorkflowBody {
                    steps: vec![WorkflowStep { id: "s1".into(), name: "count".into(), connection_id: None, sql: "select 1".into(), stop_on_error: true }],
                    ..WorkflowBody::default()
                }),
                tags: vec!["ops".into()],
                created_at: String::new(),
                updated_at: String::new(),
            })
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.list_documents(DocumentKind::Dashboard).unwrap_or_default().len(), 1);
        let flows = store.list_documents(DocumentKind::Workflow).unwrap_or_default();
        // A legacy `steps` list is read back as the graph it upgrades to (a chain of Query nodes).
        assert!(matches!(flows.first().map(|d| &d.body), Some(DocumentBody::Workflow(b)) if b.steps.is_empty() && b.nodes.len() == 1));
        store.delete_document(&dash.id).unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_documents(DocumentKind::Dashboard).unwrap_or_default().is_empty());
    }

    #[test]
    fn rename_keeps_the_body_and_delete_forgets_the_folder() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let doc = store
            .upsert_document(&Document {
                id: String::new(),
                kind: DocumentKind::Dashboard,
                connection_id: None,
                name: "Sales".into(),
                body: DocumentBody::Dashboard(DashboardBody::default()),
                tags: vec!["ops".into()],
                created_at: String::new(),
                updated_at: String::new(),
            })
            .unwrap_or_else(|e| panic!("{e}"));
        let folder = store.create_folder(LibraryScope::Dashboard, None, "Ops").unwrap_or_else(|e| panic!("{e}"));
        store.place_item(LibraryScope::Dashboard, &doc.id, Some(&folder.id)).unwrap_or_else(|e| panic!("{e}"));
        let renamed = store.rename_document(&doc.id, "  Sales overview ").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(renamed.name, "Sales overview");
        assert_eq!(renamed.tags, vec!["ops".to_string()]);
        assert!(matches!(&renamed.body, DocumentBody::Dashboard(_)));
        assert!(store.rename_document("missing", "x").is_err());
        store.delete_document(&doc.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.folder_of(LibraryScope::Dashboard, &doc.id).unwrap_or_default(), None);
    }
}
