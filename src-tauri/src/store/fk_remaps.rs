// SOT: fk-remaps-store, user-defined-foreign-keys, fk-remap-persistence

use crate::error::{AppError, AppResult};
use crate::model::{FkRemap, TableRef};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, Row};

const COLUMNS: &str = "id, connection_id, from_schema, from_table, from_column, to_schema, to_table, to_column, created_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<FkRemap> {
    Ok(FkRemap {
        id: row.get(0)?,
        connection_id: row.get(1)?,
        from: TableRef { schema: row.get(2)?, name: row.get(3)? },
        from_column: row.get(4)?,
        to: TableRef { schema: row.get(5)?, name: row.get(6)? },
        to_column: row.get(7)?,
        created_at: row.get(8)?,
    })
}

impl Store {
    pub fn list_fk_remaps(&self, connection_id: &str) -> AppResult<Vec<FkRemap>> {
        let mut stmt = self
            .conn()
            .prepare(&format!("SELECT {COLUMNS} FROM fk_remaps WHERE connection_id = ?1 ORDER BY from_table, from_column"))
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![connection_id], from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    // WHAT:  Saves the remap for a source column, replacing the one it had.
    pub fn upsert_fk_remap(&self, remap: &FkRemap) -> AppResult<FkRemap> {
        let id = if remap.id.is_empty() { uuid::Uuid::new_v4().to_string() } else { remap.id.clone() };
        let now = now_rfc3339();
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        tx.execute(
            "DELETE FROM fk_remaps WHERE connection_id = ?1 AND COALESCE(from_schema, '') = COALESCE(?2, '') AND from_table = ?3 AND from_column = ?4",
            params![remap.connection_id, remap.from.schema, remap.from.name, remap.from_column],
        )
        .map_err(AppError::store)?;
        tx.execute(
            "INSERT INTO fk_remaps (id, connection_id, from_schema, from_table, from_column, to_schema, to_table, to_column, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, remap.connection_id, remap.from.schema, remap.from.name, remap.from_column, remap.to.schema, remap.to.name, remap.to_column, now],
        )
        .map_err(AppError::store)?;
        tx.commit().map_err(AppError::store)?;
        self.conn()
            .query_row(&format!("SELECT {COLUMNS} FROM fk_remaps WHERE id = ?1"), params![id], from_row)
            .map_err(AppError::store)
    }

    pub fn delete_fk_remap(&self, id: &str) -> AppResult<()> {
        self.conn().execute("DELETE FROM fk_remaps WHERE id = ?1", params![id]).map_err(AppError::store)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remap(connection: &str, from_column: &str, to_table: &str) -> FkRemap {
        FkRemap {
            id: String::new(),
            connection_id: connection.into(),
            from: TableRef { schema: Some("public".into()), name: "orders".into() },
            from_column: from_column.into(),
            to: TableRef { schema: None, name: to_table.into() },
            to_column: "id".into(),
            created_at: String::new(),
        }
    }

    #[test]
    fn saving_a_column_again_replaces_its_remap() {
        let store = Store::open_in_memory().unwrap();
        for id in ["c1", "other"] {
            store
                .conn()
                .execute(
                    "INSERT INTO connections (id, name, engine, environment, created_at, updated_at) VALUES (?1, 'c', 'postgres', 'local', '', '')",
                    params![id],
                )
                .unwrap();
        }
        let first = store.upsert_fk_remap(&remap("c1", "user_id", "users")).unwrap();
        let second = store.upsert_fk_remap(&remap("c1", "user_id", "accounts")).unwrap();
        store.upsert_fk_remap(&remap("c1", "owner_id", "users")).unwrap();
        let all = store.list_fk_remaps("c1").unwrap();
        assert_eq!(all.len(), 2, "one remap per source column");
        assert!(all.iter().any(|r| r.id == second.id && r.to.name == "accounts"));
        assert!(!all.iter().any(|r| r.id == first.id));
        store.delete_fk_remap(&second.id).unwrap();
        assert_eq!(store.list_fk_remaps("c1").unwrap().len(), 1);
        assert!(store.list_fk_remaps("other").unwrap().is_empty());
    }
}
