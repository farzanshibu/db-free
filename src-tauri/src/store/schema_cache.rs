// SOT: schema-cache-store, persisted-catalog, persisted-columns

use crate::error::{AppError, AppResult};
use crate::store::Store;
use rusqlite::{params, OptionalExtension};

// WHAT:  One cached answer from an adapter, as it was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedSchema {
    /// The JSON the adapter's answer serialised to.
    pub payload: String,
    /// Unix milliseconds the answer was fetched.
    pub fetched_at: i64,
}

// WHAT:  The app-store half of the schema cache: the catalog and the column lists
//        an adapter returned, kept per (connection, database) so the sidebar can
//        draw at once after a restart while the live answer is fetched.
// WHY:   `load_catalog` on a database with thousands of tables takes seconds, and
//        every launch paid it before the tree appeared.
// HOW:   `scope` is `catalog` or `columns:<schema>.<table>`; `namespace` is the
//        database the session was on, since one connection can switch database.
//        Rows cascade with the connection.
// WHERE: src-tauri/src/services/schema_cache.rs (the only caller)
impl Store {
    pub fn get_schema_cache(&self, connection_id: &str, namespace: &str, scope: &str) -> AppResult<Option<CachedSchema>> {
        self.conn()
            .query_row(
                "SELECT payload, fetched_at FROM schema_cache WHERE connection_id = ?1 AND namespace = ?2 AND scope = ?3",
                params![connection_id, namespace, scope],
                |row| Ok(CachedSchema { payload: row.get(0)?, fetched_at: row.get(1)? }),
            )
            .optional()
            .map_err(AppError::store)
    }

    pub fn put_schema_cache(&self, connection_id: &str, namespace: &str, scope: &str, payload: &str, fetched_at: i64) -> AppResult<()> {
        self.conn()
            .execute(
                "INSERT INTO schema_cache (connection_id, namespace, scope, payload, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(connection_id, namespace, scope) DO UPDATE SET payload = excluded.payload, fetched_at = excluded.fetched_at",
                params![connection_id, namespace, scope, payload, fetched_at],
            )
            .map_err(AppError::store)?;
        Ok(())
    }

    /// Forgets everything cached for one connection (every database, every scope).
    pub fn clear_schema_cache(&self, connection_id: &str) -> AppResult<usize> {
        self.conn()
            .execute("DELETE FROM schema_cache WHERE connection_id = ?1", params![connection_id])
            .map_err(AppError::store)
    }

    /// Entries cached for one connection (diagnostics and tests).
    pub fn count_schema_cache(&self, connection_id: &str) -> AppResult<i64> {
        self.conn()
            .query_row("SELECT COUNT(*) FROM schema_cache WHERE connection_id = ?1", params![connection_id], |row| row.get(0))
            .map_err(AppError::store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A bare connection row: the cache only needs the foreign key to resolve, and
    // going through `ConnectionInput` would couple this test to every field other
    // features add to it.
    fn store_with_connection() -> (Store, String) {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let id = "conn-1".to_string();
        store
            .conn()
            .execute(
                "INSERT INTO connections (id, name, engine, environment, created_at, updated_at) VALUES (?1, 'cached', 'sqlite', 'local', 'x', 'x')",
                params![id],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        (store, id)
    }

    #[test]
    fn an_entry_round_trips_and_is_replaced_in_place() {
        let (store, id) = store_with_connection();
        assert_eq!(store.get_schema_cache(&id, "app", "catalog").unwrap_or(None), None);
        store.put_schema_cache(&id, "app", "catalog", "{\"schemas\":[]}", 10).unwrap_or_else(|e| panic!("{e}"));
        store.put_schema_cache(&id, "app", "catalog", "{\"schemas\":[1]}", 20).unwrap_or_else(|e| panic!("{e}"));
        let got = store.get_schema_cache(&id, "app", "catalog").unwrap_or(None);
        assert_eq!(got, Some(CachedSchema { payload: "{\"schemas\":[1]}".into(), fetched_at: 20 }));
        assert_eq!(store.count_schema_cache(&id).unwrap_or(0), 1, "same key, one row");
    }

    #[test]
    fn databases_and_scopes_are_kept_apart() {
        let (store, id) = store_with_connection();
        store.put_schema_cache(&id, "a", "catalog", "A", 1).unwrap_or_else(|e| panic!("{e}"));
        store.put_schema_cache(&id, "b", "catalog", "B", 1).unwrap_or_else(|e| panic!("{e}"));
        store.put_schema_cache(&id, "a", "columns:public.t", "C", 1).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.get_schema_cache(&id, "a", "catalog").unwrap_or(None).map(|c| c.payload), Some("A".into()));
        assert_eq!(store.get_schema_cache(&id, "b", "catalog").unwrap_or(None).map(|c| c.payload), Some("B".into()));
        assert_eq!(store.count_schema_cache(&id).unwrap_or(0), 3);
        assert_eq!(store.clear_schema_cache(&id).unwrap_or(0), 3);
        assert_eq!(store.count_schema_cache(&id).unwrap_or(1), 0);
    }

    #[test]
    fn deleting_the_connection_takes_its_cache_with_it() {
        let (store, id) = store_with_connection();
        store.put_schema_cache(&id, "a", "catalog", "A", 1).unwrap_or_else(|e| panic!("{e}"));
        store.conn().execute("DELETE FROM connections WHERE id = ?1", params![id]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.count_schema_cache(&id).unwrap_or(1), 0);
    }
}
