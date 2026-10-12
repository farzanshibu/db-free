// SOT: diagram-customizations-table, er-diagram-persistence, diagram-layout-store

use crate::error::{AppError, AppResult};
use crate::model::{DiagramCustom, DiagramCustomKind, DiagramCustomization, DiagramCustomizationKey};
use crate::store::{now_rfc3339, Store};
use rusqlite::params;

// WHAT:  Per-(connection, schema) memory of what the user did to an ER diagram:
//        where each card sits, its colour, the bend points of each line, and the
//        pan / zoom of the view.
// WHY:   The ER diagram is derived from the database, so none of this can live
//        in the diagram itself; without a store the diagram re-lays itself out
//        every time it is opened.
// HOW:   One row per (connection, schema, kind, entity). `schema` is the empty
//        string for the all-schemas diagram so the key never holds a NULL. A row
//        whose payload becomes empty is deleted rather than stored as `{}`.
// WHERE: src-tauri/src/store/migrations/diagrams.rs (table), src-tauri/src/services/diagrams.rs
impl Store {
    pub fn list_diagram_customizations(&self, connection_id: &str, schema: &str) -> AppResult<Vec<DiagramCustomization>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT kind, entity_key, custom_json FROM diagram_customizations WHERE connection_id = ?1 AND schema_name = ?2 ORDER BY kind, entity_key")
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![connection_id, schema], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        // A row this build cannot read (an unknown kind from a newer build, a
        // payload that no longer parses) is skipped, not an error: the diagram
        // should still open, with that one thing back at its default.
        Ok(rows
            .into_iter()
            .filter_map(|(kind, entity_key, json)| {
                let kind = DiagramCustomKind::parse(&kind)?;
                let custom = serde_json::from_str::<DiagramCustom>(&json).ok()?;
                Some(DiagramCustomization { kind, entity_key, custom })
            })
            .collect())
    }

    // WHAT:  Applies upserts and removals in one transaction.
    pub fn save_diagram_customizations(
        &self,
        connection_id: &str,
        schema: &str,
        upserts: &[DiagramCustomization],
        removals: &[DiagramCustomizationKey],
    ) -> AppResult<()> {
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        let now = now_rfc3339();
        for item in upserts {
            if item.custom == DiagramCustom::default() {
                tx.execute(
                    "DELETE FROM diagram_customizations WHERE connection_id = ?1 AND schema_name = ?2 AND kind = ?3 AND entity_key = ?4",
                    params![connection_id, schema, item.kind.as_str(), item.entity_key],
                )
                .map_err(AppError::store)?;
                continue;
            }
            let json = serde_json::to_string(&item.custom).map_err(AppError::store)?;
            tx.execute(
                "INSERT INTO diagram_customizations (connection_id, schema_name, kind, entity_key, custom_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(connection_id, schema_name, kind, entity_key)
                 DO UPDATE SET custom_json = excluded.custom_json, updated_at = excluded.updated_at",
                params![connection_id, schema, item.kind.as_str(), item.entity_key, json, now],
            )
            .map_err(AppError::store)?;
        }
        for key in removals {
            tx.execute(
                "DELETE FROM diagram_customizations WHERE connection_id = ?1 AND schema_name = ?2 AND kind = ?3 AND entity_key = ?4",
                params![connection_id, schema, key.kind.as_str(), key.entity_key],
            )
            .map_err(AppError::store)?;
        }
        tx.commit().map_err(AppError::store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionColor, DiagramPoint};

    // The table references `connections`, so a row has to exist for the FK.
    fn store_with_connection() -> Store {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        store
            .conn()
            .execute(
                "INSERT INTO connections (id, name, engine, environment, created_at, updated_at) VALUES ('c1', 'Local', 'sqlite', 'none', 'now', 'now')",
                [],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        store
    }

    fn node(key: &str, x: f64, color: Option<ConnectionColor>) -> DiagramCustomization {
        DiagramCustomization {
            kind: DiagramCustomKind::Node,
            entity_key: key.into(),
            custom: DiagramCustom { x: Some(x), y: Some(10.0), color, ..DiagramCustom::default() },
        }
    }

    #[test]
    fn upserts_round_trip_and_replace() {
        let store = store_with_connection();
        store.save_diagram_customizations("c1", "public", &[node("public.users", 5.0, Some(ConnectionColor::Teal))], &[]).unwrap_or_else(|e| panic!("{e}"));
        store.save_diagram_customizations("c1", "public", &[node("public.users", 99.0, None)], &[]).unwrap_or_else(|e| panic!("{e}"));
        let rows = store.list_diagram_customizations("c1", "public").unwrap_or_default();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].custom.x, Some(99.0));
        assert_eq!(rows[0].custom.color, None);
    }

    #[test]
    fn schemas_are_separate_diagrams() {
        let store = store_with_connection();
        store.save_diagram_customizations("c1", "", &[node("users", 1.0, None)], &[]).unwrap_or_else(|e| panic!("{e}"));
        store.save_diagram_customizations("c1", "sales", &[node("orders", 2.0, None)], &[]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.list_diagram_customizations("c1", "").unwrap_or_default().len(), 1);
        assert_eq!(store.list_diagram_customizations("c1", "sales").unwrap_or_default()[0].entity_key, "orders");
    }

    #[test]
    fn an_empty_payload_and_a_removal_both_delete() {
        let store = store_with_connection();
        let edge = DiagramCustomization {
            kind: DiagramCustomKind::Edge,
            entity_key: "a->b:fk".into(),
            custom: DiagramCustom { waypoints: vec![DiagramPoint { x: 1.0, y: 2.0 }], ..DiagramCustom::default() },
        };
        store.save_diagram_customizations("c1", "", &[edge.clone(), node("t", 1.0, None)], &[]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.list_diagram_customizations("c1", "").unwrap_or_default().len(), 2);
        let cleared = DiagramCustomization { custom: DiagramCustom::default(), ..edge };
        store
            .save_diagram_customizations("c1", "", &[cleared], &[DiagramCustomizationKey { kind: DiagramCustomKind::Node, entity_key: "t".into() }])
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_diagram_customizations("c1", "").unwrap_or_default().is_empty());
    }

    #[test]
    fn deleting_the_connection_removes_its_customizations() {
        let store = store_with_connection();
        store.save_diagram_customizations("c1", "", &[node("t", 1.0, None)], &[]).unwrap_or_else(|e| panic!("{e}"));
        store.conn().execute("DELETE FROM connections WHERE id = 'c1'", []).unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_diagram_customizations("c1", "").unwrap_or_default().is_empty());
    }

    #[test]
    fn unreadable_rows_are_skipped() {
        let store = store_with_connection();
        store
            .conn()
            .execute(
                "INSERT INTO diagram_customizations (connection_id, schema_name, kind, entity_key, custom_json, updated_at) VALUES ('c1', '', 'node', 'bad', 'not json', 'now'), ('c1', '', 'future', 'x', '{}', 'now')",
                [],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_diagram_customizations("c1", "").unwrap_or_default().is_empty());
    }
}
