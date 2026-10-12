// SOT: home-pins-store, home-recents-store, pinned-board-persistence

use crate::error::{AppError, AppResult};
use crate::model::{HomeEntity, HomeItemKind, HomePin, HomeRecent};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, Row};

/// How many recently opened items are kept; older ones fall off.
pub const RECENT_LIMIT: usize = 50;

fn entity_from_row(row: &Row<'_>, first: usize) -> rusqlite::Result<HomeEntity> {
    let kind_raw: String = row.get(first)?;
    let kind = HomeItemKind::parse(&kind_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(first, rusqlite::types::Type::Text, format!("unknown home item kind {kind_raw}").into())
    })?;
    Ok(HomeEntity {
        kind,
        item_id: row.get(first + 1)?,
        label: row.get(first + 2)?,
        connection_id: row.get(first + 3)?,
        schema_name: row.get(first + 4)?,
        table_name: row.get(first + 5)?,
    })
}

fn pin_from_row(row: &Row<'_>) -> rusqlite::Result<HomePin> {
    Ok(HomePin { id: row.get(0)?, entity: entity_from_row(row, 1)?, created_at: row.get(7)? })
}

fn recent_from_row(row: &Row<'_>) -> rusqlite::Result<HomeRecent> {
    Ok(HomeRecent { id: row.get(0)?, entity: entity_from_row(row, 1)?, opened_at: row.get(7)? })
}

impl Store {
    // WHAT:  Drops pins and recents whose item no longer exists.
    // WHY:   Deleting a connection, saved query or document happens in other
    //        modules; checking here, where the lists are read, keeps one rule
    //        instead of a cleanup call in every delete path. Tables cannot be
    //        checked (they live in the user's database), so only their
    //        connection is.
    fn prune_home(&self) -> AppResult<()> {
        let conn = self.conn();
        for table in ["home_pins", "home_recents"] {
            conn.execute_batch(&format!(
                "DELETE FROM {table} WHERE connection_id NOT IN (SELECT id FROM connections);
                 DELETE FROM {table} WHERE kind = 'saved_query' AND item_id NOT IN (SELECT id FROM saved_queries);
                 DELETE FROM {table} WHERE kind IN ('dashboard', 'workflow', 'diagram')
                     AND item_id NOT IN (SELECT id FROM documents WHERE documents.kind = {table}.kind);"
            ))
            .map_err(AppError::store)?;
        }
        Ok(())
    }

    pub fn list_pins(&self) -> AppResult<Vec<HomePin>> {
        self.prune_home()?;
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT id, kind, item_id, label, connection_id, schema_name, table_name, created_at \
                 FROM home_pins ORDER BY sort_order, created_at, rowid",
            )
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map([], pin_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    // WHAT:  Pins an item at the end of the board. Pinning something already
    //        pinned only refreshes its label.
    pub fn pin_item(&self, entity: &HomeEntity) -> AppResult<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let next_order: i64 = self
            .conn()
            .query_row("SELECT COALESCE(MAX(sort_order), 0) + 1 FROM home_pins", [], |row| row.get(0))
            .map_err(AppError::store)?;
        self.conn()
            .execute(
                "INSERT INTO home_pins (id, kind, item_id, label, connection_id, schema_name, table_name, sort_order, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
                 ON CONFLICT(kind, item_id) DO UPDATE SET label = excluded.label, connection_id = excluded.connection_id, \
                 schema_name = excluded.schema_name, table_name = excluded.table_name",
                params![
                    id,
                    entity.kind.as_str(),
                    entity.item_id,
                    entity.label,
                    entity.connection_id,
                    entity.schema_name,
                    entity.table_name,
                    next_order,
                    now_rfc3339()
                ],
            )
            .map_err(AppError::store)?;
        Ok(())
    }

    pub fn unpin_item(&self, kind: HomeItemKind, item_id: &str) -> AppResult<()> {
        self.conn()
            .execute("DELETE FROM home_pins WHERE kind = ?1 AND item_id = ?2", params![kind.as_str(), item_id])
            .map_err(AppError::store)?;
        Ok(())
    }

    // WHAT:  Stores the board order: `ids` first, in the order given. A pin the
    //        list leaves out (pinned meanwhile) keeps its place after them.
    pub fn reorder_pins(&self, ids: &[String]) -> AppResult<()> {
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        for (index, id) in ids.iter().enumerate() {
            let order = i64::try_from(index + 1).unwrap_or(i64::MAX);
            tx.execute("UPDATE home_pins SET sort_order = ?2 WHERE id = ?1", params![id, order]).map_err(AppError::store)?;
        }
        let after = i64::try_from(ids.len() + 1).unwrap_or(i64::MAX);
        let listed: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        let mut stmt = tx.prepare("SELECT id FROM home_pins ORDER BY sort_order, created_at, rowid").map_err(AppError::store)?;
        let mut rest = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        drop(stmt);
        rest.retain(|id| !listed.contains(id.as_str()));
        for (offset, id) in rest.iter().enumerate() {
            let order = after.saturating_add(i64::try_from(offset).unwrap_or(0));
            tx.execute("UPDATE home_pins SET sort_order = ?2 WHERE id = ?1", params![id, order]).map_err(AppError::store)?;
        }
        tx.commit().map_err(AppError::store)
    }

    pub fn list_recents(&self) -> AppResult<Vec<HomeRecent>> {
        self.prune_home()?;
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT id, kind, item_id, label, connection_id, schema_name, table_name, opened_at \
                 FROM home_recents ORDER BY seq DESC LIMIT ?1",
            )
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![i64::try_from(RECENT_LIMIT).unwrap_or(50)], recent_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    // WHAT:  Notes that an item was just opened: moves it to the front (adding it
    //        if new) and trims the list to RECENT_LIMIT.
    pub fn record_recent(&self, entity: &HomeEntity) -> AppResult<()> {
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        let next_seq: i64 = tx
            .query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM home_recents", [], |row| row.get(0))
            .map_err(AppError::store)?;
        tx.execute(
            "INSERT INTO home_recents (id, kind, item_id, label, connection_id, schema_name, table_name, seq, opened_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(kind, item_id) DO UPDATE SET label = excluded.label, connection_id = excluded.connection_id, \
             schema_name = excluded.schema_name, table_name = excluded.table_name, seq = excluded.seq, opened_at = excluded.opened_at",
            params![
                uuid::Uuid::new_v4().to_string(),
                entity.kind.as_str(),
                entity.item_id,
                entity.label,
                entity.connection_id,
                entity.schema_name,
                entity.table_name,
                next_seq,
                now_rfc3339()
            ],
        )
        .map_err(AppError::store)?;
        tx.execute(
            "DELETE FROM home_recents WHERE id NOT IN (SELECT id FROM home_recents ORDER BY seq DESC LIMIT ?1)",
            params![i64::try_from(RECENT_LIMIT).unwrap_or(50)],
        )
        .map_err(AppError::store)?;
        tx.commit().map_err(AppError::store)
    }

    pub fn clear_recents(&self) -> AppResult<()> {
        self.conn().execute("DELETE FROM home_recents", []).map_err(AppError::store)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DocumentKind;

    fn store() -> Store {
        Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"))
    }

    // The connection row a pin points at; without it the pin would be pruned.
    fn add_connection(store: &Store, id: &str) {
        store.insert_test_connection(id).unwrap_or_else(|e| panic!("{e}"));
    }

    fn entity(kind: HomeItemKind, item_id: &str, connection_id: &str) -> HomeEntity {
        HomeEntity { kind, item_id: item_id.into(), label: item_id.into(), connection_id: connection_id.into(), schema_name: None, table_name: None }
    }

    #[test]
    fn pins_keep_their_order_and_pinning_twice_is_one_pin() {
        let store = store();
        add_connection(&store, "c1");
        let table = |name: &str| HomeEntity {
            kind: HomeItemKind::Table,
            item_id: format!("c1:{name}"),
            label: name.into(),
            connection_id: "c1".into(),
            schema_name: None,
            table_name: Some(name.into()),
        };
        store.pin_item(&table("users")).unwrap_or_else(|e| panic!("{e}"));
        store.pin_item(&table("orders")).unwrap_or_else(|e| panic!("{e}"));
        store.pin_item(&table("users")).unwrap_or_else(|e| panic!("{e}"));
        let pins = store.list_pins().unwrap_or_default();
        let tables: Vec<&str> = pins.iter().filter(|p| p.entity.kind == HomeItemKind::Table).map(|p| p.entity.label.as_str()).collect();
        assert_eq!(tables, vec!["users", "orders"]);
        let ids: Vec<String> = pins.iter().rev().map(|p| p.id.clone()).collect();
        store.reorder_pins(&ids).unwrap_or_else(|e| panic!("{e}"));
        let reordered: Vec<String> = store.list_pins().unwrap_or_default().into_iter().map(|p| p.id).collect();
        assert_eq!(reordered, ids);
        store.unpin_item(HomeItemKind::Table, "c1:orders").unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_pins().unwrap_or_default().iter().all(|p| p.entity.item_id != "c1:orders"));
    }

    #[test]
    fn a_pin_missing_from_a_reorder_keeps_its_place_after_the_others() {
        let store = store();
        add_connection(&store, "c1");
        let table = |name: &str| HomeEntity {
            kind: HomeItemKind::Table,
            item_id: format!("c1:{name}"),
            label: name.into(),
            connection_id: "c1".into(),
            schema_name: None,
            table_name: Some(name.into()),
        };
        for name in ["a", "b", "c"] {
            store.pin_item(&table(name)).unwrap_or_else(|e| panic!("{e}"));
        }
        let pins = store.list_pins().unwrap_or_default();
        let (a, b, c) = (pins[0].id.clone(), pins[1].id.clone(), pins[2].id.clone());
        store.reorder_pins(&[c.clone(), a.clone()]).unwrap_or_else(|e| panic!("{e}"));
        let order: Vec<String> = store.list_pins().unwrap_or_default().into_iter().map(|p| p.id).collect();
        assert_eq!(order, vec![c, a, b]);
    }

    #[test]
    fn pins_of_deleted_items_disappear() {
        let store = store();
        add_connection(&store, "c1");
        store
            .upsert_document(&crate::model::Document {
                id: "d1".into(),
                kind: DocumentKind::Dashboard,
                connection_id: Some("c1".into()),
                name: "Sales".into(),
                body: crate::model::DocumentBody::Dashboard(crate::model::DashboardBody::default()),
                tags: Vec::new(),
                created_at: String::new(),
                updated_at: String::new(),
            })
            .unwrap_or_else(|e| panic!("{e}"));
        store.pin_item(&entity(HomeItemKind::Dashboard, "d1", "c1")).unwrap_or_else(|e| panic!("{e}"));
        store.pin_item(&entity(HomeItemKind::Dashboard, "gone", "c1")).unwrap_or_else(|e| panic!("{e}"));
        store.pin_item(&entity(HomeItemKind::Connection, "c1", "c1")).unwrap_or_else(|e| panic!("{e}"));
        store.pin_item(&entity(HomeItemKind::Connection, "c2", "c2")).unwrap_or_else(|e| panic!("{e}"));
        let kept: Vec<String> = store.list_pins().unwrap_or_default().into_iter().map(|p| p.entity.item_id).collect();
        assert_eq!(kept, vec!["d1".to_string(), "c1".to_string()]);
        // A workflow pin does not survive just because a dashboard has the same id.
        store.pin_item(&entity(HomeItemKind::Workflow, "d1", "c1")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.list_pins().unwrap_or_default().len(), 2);
    }

    #[test]
    fn recents_are_most_recent_first_deduplicated_and_capped() {
        let store = store();
        add_connection(&store, "c1");
        let table = |name: &str| HomeEntity {
            kind: HomeItemKind::Table,
            item_id: format!("c1:{name}"),
            label: name.into(),
            connection_id: "c1".into(),
            schema_name: None,
            table_name: Some(name.into()),
        };
        store.record_recent(&table("a")).unwrap_or_else(|e| panic!("{e}"));
        store.record_recent(&table("b")).unwrap_or_else(|e| panic!("{e}"));
        store.record_recent(&table("a")).unwrap_or_else(|e| panic!("{e}"));
        let labels: Vec<String> = store.list_recents().unwrap_or_default().into_iter().map(|r| r.entity.label).collect();
        assert_eq!(labels, vec!["a".to_string(), "b".to_string()]);
        for n in 0..(RECENT_LIMIT + 5) {
            store.record_recent(&table(&format!("t{n}"))).unwrap_or_else(|e| panic!("{e}"));
        }
        let recents = store.list_recents().unwrap_or_default();
        assert_eq!(recents.len(), RECENT_LIMIT);
        assert_eq!(recents[0].entity.label, format!("t{}", RECENT_LIMIT + 4));
        store.clear_recents().unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_recents().unwrap_or_default().is_empty());
    }
}
