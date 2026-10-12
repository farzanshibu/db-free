// SOT: saved-queries-table, saved-query-persistence, saved-query-versions

use crate::error::{AppError, AppResult};
use crate::model::{LibraryScope, SavedQuery, SavedQueryVersion};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, Connection, OptionalExtension, Row};

/// Versions kept per saved query; the oldest goes when a newer one lands.
pub const MAX_VERSIONS: i64 = 10;

const COLUMNS: &str = "id, connection_id, name, sql, tags, created_at, updated_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<SavedQuery> {
    let tags: String = row.get(4)?;
    Ok(SavedQuery {
        id: row.get(0)?,
        connection_id: row.get(1)?,
        name: row.get(2)?,
        sql: row.get(3)?,
        tags: tags.split(',').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect(),
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn version_from_row(row: &Row<'_>) -> rusqlite::Result<SavedQueryVersion> {
    let version: i64 = row.get(2)?;
    Ok(SavedQueryVersion {
        id: row.get(0)?,
        saved_query_id: row.get(1)?,
        version: u32::try_from(version).unwrap_or(u32::MAX),
        sql: row.get(3)?,
        created_at: row.get(4)?,
    })
}

// WHAT:  Adds a version row when the SQL of a saved query changed, and keeps the newest MAX_VERSIONS.
// WHY:   Every save goes through `upsert_saved_query`, so the editor's ⌘S, the save
//        dialog and a restore all get versions without knowing about them.
// HOW:   The first change to a query saved before versions existed also records the
//        text it had, so there is something to go back to. Saving identical SQL
//        (a rename, new tags) adds nothing.
fn record_version(conn: &Connection, saved_query_id: &str, previous: Option<(&str, &str)>, new_sql: &str, now: &str) -> AppResult<()> {
    let latest: Option<(i64, String)> = conn
        .query_row(
            "SELECT version, sql FROM saved_query_versions WHERE saved_query_id = ?1 ORDER BY version DESC LIMIT 1",
            params![saved_query_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(AppError::store)?;
    let insert = |version: i64, sql: &str, at: &str| -> AppResult<()> {
        conn.execute(
            "INSERT INTO saved_query_versions (id, saved_query_id, version, sql, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![uuid::Uuid::new_v4().to_string(), saved_query_id, version, sql, at],
        )
        .map_err(AppError::store)?;
        Ok(())
    };
    let newest = match latest {
        Some((version, sql)) => {
            if sql == new_sql {
                return Ok(());
            }
            insert(version + 1, new_sql, now)?;
            version + 1
        }
        None => match previous {
            Some((old_sql, old_at)) if old_sql != new_sql => {
                insert(1, old_sql, old_at)?;
                insert(2, new_sql, now)?;
                2
            }
            _ => {
                insert(1, new_sql, now)?;
                1
            }
        },
    };
    conn.execute(
        "DELETE FROM saved_query_versions WHERE saved_query_id = ?1 AND version <= ?2",
        params![saved_query_id, newest - MAX_VERSIONS],
    )
    .map_err(AppError::store)?;
    Ok(())
}

impl Store {
    pub fn list_saved_queries(&self) -> AppResult<Vec<SavedQuery>> {
        let mut stmt = self
            .conn()
            .prepare(&format!("SELECT {COLUMNS} FROM saved_queries ORDER BY lower(name)"))
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map([], from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    pub fn get_saved_query(&self, id: &str) -> AppResult<SavedQuery> {
        self.conn()
            .query_row(&format!("SELECT {COLUMNS} FROM saved_queries WHERE id = ?1"), params![id], from_row)
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::not_found(format!("Saved query {id} does not exist.")))
    }

    pub fn upsert_saved_query(&self, query: &SavedQuery) -> AppResult<SavedQuery> {
        let now = now_rfc3339();
        let id = if query.id.is_empty() { uuid::Uuid::new_v4().to_string() } else { query.id.clone() };
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        let previous: Option<(String, String)> = tx
            .query_row("SELECT sql, updated_at FROM saved_queries WHERE id = ?1", params![id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()
            .map_err(AppError::store)?;
        tx.execute(
            "INSERT INTO saved_queries (id, connection_id, name, sql, tags, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) \
             ON CONFLICT(id) DO UPDATE SET connection_id = excluded.connection_id, name = excluded.name, \
             sql = excluded.sql, tags = excluded.tags, updated_at = excluded.updated_at",
            params![id, query.connection_id, query.name.trim(), query.sql, query.tags.join(","), now],
        )
        .map_err(AppError::store)?;
        record_version(&tx, &id, previous.as_ref().map(|(sql, at)| (sql.as_str(), at.as_str())), &query.sql, &now)?;
        tx.commit().map_err(AppError::store)?;
        self.get_saved_query(&id)
    }

    // WHAT:  Renames without touching the SQL, so no version is added.
    pub fn rename_saved_query(&self, id: &str, name: &str) -> AppResult<SavedQuery> {
        let updated = self
            .conn()
            .execute("UPDATE saved_queries SET name = ?2, updated_at = ?3 WHERE id = ?1", params![id, name.trim(), now_rfc3339()])
            .map_err(AppError::store)?;
        if updated == 0 {
            return Err(AppError::not_found(format!("Saved query {id} does not exist.")));
        }
        self.get_saved_query(id)
    }

    pub fn delete_saved_query(&self, id: &str) -> AppResult<()> {
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        tx.execute("DELETE FROM saved_query_versions WHERE saved_query_id = ?1", params![id]).map_err(AppError::store)?;
        tx.execute(
            "DELETE FROM library_placements WHERE scope = ?1 AND item_id = ?2",
            params![LibraryScope::SavedQuery.as_str(), id],
        )
        .map_err(AppError::store)?;
        tx.execute("DELETE FROM saved_queries WHERE id = ?1", params![id]).map_err(AppError::store)?;
        tx.commit().map_err(AppError::store)
    }

    // WHAT:  True when another saved query with this name already exists in the
    //        same connection scope and folder (case-insensitive).
    // WHY:   Two "Daily revenue" entries side by side are indistinguishable in the
    //        sidebar; the save dialog asks for a different name instead.
    pub fn saved_query_name_taken(&self, connection_id: Option<&str>, folder_id: Option<&str>, name: &str, except_id: Option<&str>) -> AppResult<bool> {
        let found: Option<i64> = self
            .conn()
            .query_row(
                "SELECT 1 FROM saved_queries q \
                 LEFT JOIN library_placements p ON p.scope = ?5 AND p.item_id = q.id \
                 WHERE q.connection_id IS ?1 AND p.folder_id IS ?2 AND lower(trim(q.name)) = lower(trim(?3)) AND q.id IS NOT ?4 LIMIT 1",
                params![connection_id, folder_id, name, except_id, LibraryScope::SavedQuery.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::store)?;
        Ok(found.is_some())
    }

    // WHAT:  The names already used in one connection scope and folder, for numbering a copy.
    pub fn saved_query_names_in(&self, connection_id: Option<&str>, folder_id: Option<&str>) -> AppResult<Vec<String>> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT q.name FROM saved_queries q \
                 LEFT JOIN library_placements p ON p.scope = ?3 AND p.item_id = q.id \
                 WHERE q.connection_id IS ?1 AND p.folder_id IS ?2",
            )
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![connection_id, folder_id, LibraryScope::SavedQuery.as_str()], |row| row.get::<_, String>(0))
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    pub fn list_saved_query_versions(&self, saved_query_id: &str) -> AppResult<Vec<SavedQueryVersion>> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT id, saved_query_id, version, sql, created_at FROM saved_query_versions \
                 WHERE saved_query_id = ?1 ORDER BY version DESC",
            )
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![saved_query_id], version_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    pub fn get_saved_query_version(&self, version_id: &str) -> AppResult<SavedQueryVersion> {
        self.conn()
            .query_row(
                "SELECT id, saved_query_id, version, sql, created_at FROM saved_query_versions WHERE id = ?1",
                params![version_id],
                version_from_row,
            )
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::not_found(format!("Version {version_id} does not exist.")))
    }

    // WHAT:  Puts an earlier version's SQL back as the query's text. The restore is
    //        itself a new version, so it can be undone the same way.
    pub fn restore_saved_query_version(&self, version_id: &str) -> AppResult<SavedQuery> {
        let version = self.get_saved_query_version(version_id)?;
        let current = self.get_saved_query(&version.saved_query_id)?;
        self.upsert_saved_query(&SavedQuery { sql: version.sql, ..current })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(name: &str, sql: &str) -> SavedQuery {
        SavedQuery {
            id: String::new(),
            connection_id: None,
            name: name.into(),
            sql: sql.into(),
            tags: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn save_list_delete() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let saved = store
            .upsert_saved_query(&SavedQuery { tags: vec!["reports".into()], ..query("Top users", "select 1") })
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!saved.id.is_empty());
        assert_eq!(saved.tags, vec!["reports".to_string()]);
        let renamed = store
            .upsert_saved_query(&SavedQuery { name: "Top users v2".into(), ..saved.clone() })
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(renamed.id, saved.id);
        assert_eq!(store.list_saved_queries().unwrap_or_default().len(), 1);
        store.delete_saved_query(&saved.id).unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_saved_queries().unwrap_or_default().is_empty());
        assert!(store.list_saved_query_versions(&saved.id).unwrap_or_default().is_empty());
    }

    #[test]
    fn a_new_version_is_added_only_when_the_sql_changes() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let saved = store.upsert_saved_query(&query("Q", "select 1")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.list_saved_query_versions(&saved.id).unwrap_or_default().len(), 1);
        // Same SQL (a rename, new tags): nothing new.
        let again = store
            .upsert_saved_query(&SavedQuery { name: "Q2".into(), tags: vec!["x".into()], ..saved.clone() })
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.list_saved_query_versions(&saved.id).unwrap_or_default().len(), 1);
        store.rename_saved_query(&again.id, "Q3").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.list_saved_query_versions(&saved.id).unwrap_or_default().len(), 1);
        store.upsert_saved_query(&SavedQuery { sql: "select 2".into(), ..again }).unwrap_or_else(|e| panic!("{e}"));
        let versions = store.list_saved_query_versions(&saved.id).unwrap_or_default();
        assert_eq!(versions.iter().map(|v| (v.version, v.sql.as_str())).collect::<Vec<_>>(), vec![(2, "select 2"), (1, "select 1")]);
    }

    #[test]
    fn only_the_newest_ten_versions_are_kept() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let mut saved = store.upsert_saved_query(&query("Q", "select 0")).unwrap_or_else(|e| panic!("{e}"));
        for n in 1..=14 {
            saved = store.upsert_saved_query(&SavedQuery { sql: format!("select {n}"), ..saved }).unwrap_or_else(|e| panic!("{e}"));
        }
        let versions = store.list_saved_query_versions(&saved.id).unwrap_or_default();
        assert_eq!(versions.len(), MAX_VERSIONS as usize);
        assert_eq!(versions[0].version, 15);
        assert_eq!(versions[0].sql, "select 14");
        assert_eq!(versions.last().map(|v| v.version), Some(6));
    }

    #[test]
    fn restoring_a_version_makes_it_the_newest() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let saved = store.upsert_saved_query(&query("Q", "select 1")).unwrap_or_else(|e| panic!("{e}"));
        let edited = store.upsert_saved_query(&SavedQuery { sql: "select 2".into(), ..saved }).unwrap_or_else(|e| panic!("{e}"));
        let first = store
            .list_saved_query_versions(&edited.id)
            .unwrap_or_default()
            .into_iter()
            .find(|v| v.version == 1)
            .unwrap_or_else(|| panic!("version 1 missing"));
        let restored = store.restore_saved_query_version(&first.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(restored.sql, "select 1");
        let versions = store.list_saved_query_versions(&edited.id).unwrap_or_default();
        assert_eq!(versions.iter().map(|v| v.sql.as_str()).collect::<Vec<_>>(), vec!["select 1", "select 2", "select 1"]);
    }

    #[test]
    fn a_query_saved_before_versions_existed_keeps_its_old_text_on_first_edit() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        // A row as an older build wrote it: no version rows at all.
        store
            .conn()
            .execute(
                "INSERT INTO saved_queries (id, connection_id, name, sql, tags, created_at, updated_at) VALUES ('old', NULL, 'Old', 'select old', '', 't0', 't0')",
                [],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        store.conn().execute("DELETE FROM saved_query_versions", []).unwrap_or_else(|e| panic!("{e}"));
        let current = store.get_saved_query("old").unwrap_or_else(|e| panic!("{e}"));
        store.upsert_saved_query(&SavedQuery { sql: "select new".into(), ..current }).unwrap_or_else(|e| panic!("{e}"));
        let versions = store.list_saved_query_versions("old").unwrap_or_default();
        assert_eq!(versions.iter().map(|v| v.sql.as_str()).collect::<Vec<_>>(), vec!["select new", "select old"]);
    }

    #[test]
    fn duplicate_names_are_detected_per_connection_and_folder() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let folder = store.create_folder(LibraryScope::SavedQuery, None, "Reports").unwrap_or_else(|e| panic!("{e}"));
        let top = store.upsert_saved_query(&query("Daily revenue", "select 1")).unwrap_or_else(|e| panic!("{e}"));
        let filed = store.upsert_saved_query(&query("Weekly", "select 2")).unwrap_or_else(|e| panic!("{e}"));
        store.place_item(LibraryScope::SavedQuery, &filed.id, Some(&folder.id)).unwrap_or_else(|e| panic!("{e}"));
        assert!(store.saved_query_name_taken(None, None, "  daily REVENUE ", None).unwrap_or(false));
        assert!(!store.saved_query_name_taken(None, None, "Daily revenue", Some(&top.id)).unwrap_or(true), "a query does not clash with itself");
        assert!(!store.saved_query_name_taken(None, Some(&folder.id), "Daily revenue", None).unwrap_or(true), "another folder");
        assert!(store.saved_query_name_taken(None, Some(&folder.id), "Weekly", None).unwrap_or(false));
        assert!(!store.saved_query_name_taken(Some("c1"), None, "Daily revenue", None).unwrap_or(true), "another connection");
    }
}
