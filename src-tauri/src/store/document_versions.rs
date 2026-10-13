// SOT: document-versions-table, document-history-store, diagram-version-persistence

use crate::error::{AppError, AppResult};
use crate::store::Store;
use rusqlite::{params, OptionalExtension, Row};

// WHAT:  One stored snapshot of a document. `body` is the `DocumentBody` JSON as
//        the documents table holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredVersion {
    pub document_id: String,
    pub version: u32,
    pub name: Option<String>,
    pub created_at: String,
    pub body: String,
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<StoredVersion> {
    Ok(StoredVersion {
        document_id: row.get(0)?,
        version: row.get(1)?,
        name: row.get(2)?,
        created_at: row.get(3)?,
        body: row.get(4)?,
    })
}

const COLUMNS: &str = "document_id, version, name, created_at, body";

// WHAT:  Primitives over the `document_versions` table: any document kind can
//        keep a history here, numbered 1, 2, 3… per document.
// WHY:   Rules (skip an unchanged save, keep the latest twenty automatic ones,
//        never prune a named checkpoint) are the service's; the store only
//        reads, appends and deletes rows.
// WHERE: src-tauri/src/services/document_versions.rs, src-tauri/src/store/migrations/diagrams.rs
impl Store {
    /// Newest first.
    pub fn list_document_versions(&self, document_id: &str) -> AppResult<Vec<StoredVersion>> {
        let sql = format!("SELECT {COLUMNS} FROM document_versions WHERE document_id = ?1 ORDER BY version DESC");
        let mut stmt = self.conn().prepare(&sql).map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![document_id], from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    pub fn latest_document_version(&self, document_id: &str) -> AppResult<Option<StoredVersion>> {
        let sql = format!("SELECT {COLUMNS} FROM document_versions WHERE document_id = ?1 ORDER BY version DESC LIMIT 1");
        self.conn().query_row(&sql, params![document_id], from_row).optional().map_err(AppError::store)
    }

    pub fn get_document_version(&self, document_id: &str, version: u32) -> AppResult<StoredVersion> {
        let sql = format!("SELECT {COLUMNS} FROM document_versions WHERE document_id = ?1 AND version = ?2");
        self.conn()
            .query_row(&sql, params![document_id, version], from_row)
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::not_found(format!("Version {version} of document {document_id} does not exist.")))
    }

    // WHAT:  Appends a snapshot as the next version number.
    pub fn insert_document_version(&self, document_id: &str, body: &str, name: Option<&str>, created_at: &str) -> AppResult<StoredVersion> {
        let next: u32 = self
            .conn()
            .query_row("SELECT COALESCE(MAX(version), 0) + 1 FROM document_versions WHERE document_id = ?1", params![document_id], |row| row.get(0))
            .map_err(AppError::store)?;
        self.conn()
            .execute(
                "INSERT INTO document_versions (document_id, version, body, name, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![document_id, next, body, name, created_at],
            )
            .map_err(AppError::store)?;
        self.get_document_version(document_id, next)
    }

    // WHAT:  Deletes the oldest unnamed versions beyond `keep`; returns how many went.
    pub fn prune_document_versions(&self, document_id: &str, keep: u32) -> AppResult<usize> {
        self.conn()
            .execute(
                "DELETE FROM document_versions
                 WHERE document_id = ?1 AND name IS NULL
                   AND version NOT IN (
                       SELECT version FROM document_versions
                       WHERE document_id = ?1 AND name IS NULL
                       ORDER BY version DESC LIMIT ?2
                   )",
                params![document_id, keep],
            )
            .map_err(AppError::store)
    }

    pub fn delete_document_version(&self, document_id: &str, version: u32) -> AppResult<()> {
        self.conn()
            .execute("DELETE FROM document_versions WHERE document_id = ?1 AND version = ?2", params![document_id, version])
            .map_err(AppError::store)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DashboardBody, Document, DocumentBody, DocumentKind};

    fn store_with_document() -> (Store, String) {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let doc = store
            .upsert_document(&Document {
                id: String::new(),
                kind: DocumentKind::Dashboard,
                connection_id: None,
                name: "d".into(),
                body: DocumentBody::Dashboard(DashboardBody::default()),
                tags: Vec::new(),
                created_at: String::new(),
                updated_at: String::new(),
            })
            .unwrap_or_else(|e| panic!("{e}"));
        (store, doc.id)
    }

    #[test]
    fn versions_are_numbered_per_document_and_listed_newest_first() {
        let (store, id) = store_with_document();
        for n in 0..3 {
            store.insert_document_version(&id, &format!("{{\"n\":{n}}}"), None, "t").unwrap_or_else(|e| panic!("{e}"));
        }
        let versions = store.list_document_versions(&id).unwrap_or_default();
        assert_eq!(versions.iter().map(|v| v.version).collect::<Vec<_>>(), vec![3, 2, 1]);
        assert_eq!(store.latest_document_version(&id).unwrap_or_default().map(|v| v.version), Some(3));
        assert!(store.get_document_version(&id, 9).is_err());
    }

    #[test]
    fn pruning_keeps_the_newest_unnamed_and_every_checkpoint() {
        let (store, id) = store_with_document();
        store.insert_document_version(&id, "{}", Some("before the refactor"), "t").unwrap_or_else(|e| panic!("{e}"));
        for _ in 0..5 {
            store.insert_document_version(&id, "{}", None, "t").unwrap_or_else(|e| panic!("{e}"));
        }
        let removed = store.prune_document_versions(&id, 2).unwrap_or_default();
        assert_eq!(removed, 3);
        let left = store.list_document_versions(&id).unwrap_or_default();
        assert_eq!(left.iter().map(|v| v.version).collect::<Vec<_>>(), vec![6, 5, 1]);
        assert_eq!(left.last().and_then(|v| v.name.clone()).as_deref(), Some("before the refactor"));
    }

    #[test]
    fn deleting_the_document_removes_its_history() {
        let (store, id) = store_with_document();
        store.insert_document_version(&id, "{}", None, "t").unwrap_or_else(|e| panic!("{e}"));
        store.delete_document(&id).unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_document_versions(&id).unwrap_or_default().is_empty());
    }

    #[test]
    fn a_deleted_version_leaves_a_gap_and_numbers_keep_growing() {
        let (store, id) = store_with_document();
        store.insert_document_version(&id, "{}", None, "t").unwrap_or_else(|e| panic!("{e}"));
        store.insert_document_version(&id, "{}", None, "t").unwrap_or_else(|e| panic!("{e}"));
        store.delete_document_version(&id, 1).unwrap_or_else(|e| panic!("{e}"));
        let next = store.insert_document_version(&id, "{}", None, "t").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(next.version, 3, "numbers continue from the highest remaining version");
        assert_eq!(store.list_document_versions(&id).unwrap_or_default().len(), 2);
    }
}
