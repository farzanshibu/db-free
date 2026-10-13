// SOT: table-tags-table, table-tag-persistence, table-tag-links, tag-collapse-state

use crate::error::{AppError, AppResult};
use crate::model::{ConnectionColor, TableTag, TableTagInput, TableTagLink, TagState, MAX_TAG_NAME_LEN};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, OptionalExtension, Row};

// WHAT:  Persistence for the sidebar's table tags: the tags of a (connection,
//        database, schema) scope, which tables carry them, and which groups are
//        folded.
// WHY:   Tags are the app's own metadata about the user's tables; they live in the
//        local store and never touch the user's database.
// HOW:   One tag row per scope; a link row per (tag, table); the unique index on
//        lower(name) backs the duplicate-name rule. Deleting a tag or a connection
//        cascades to its links.
// WHERE: src-tauri/src/services/tags.rs, src-tauri/src/store/migrations/schema.rs

const COLUMNS: &str = "id, connection_id, database, schema, name, color, position, collapsed";

fn from_row(row: &Row<'_>) -> rusqlite::Result<TableTag> {
    let color: String = row.get(5)?;
    let position: i64 = row.get(6)?;
    let collapsed: i64 = row.get(7)?;
    Ok(TableTag {
        id: row.get(0)?,
        connection_id: row.get(1)?,
        database: row.get(2)?,
        schema: row.get(3)?,
        name: row.get(4)?,
        color: ConnectionColor::parse(&color).unwrap_or(ConnectionColor::Gray),
        position: u32::try_from(position).unwrap_or(0),
        collapsed: collapsed != 0,
    })
}

/// Collapses runs of whitespace and trims, so "  Billing  " and "Billing" collide.
pub fn clean_tag_name(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Store {
    /// Every tag, link and folded "Untagged" group of one connection + database.
    pub fn list_tag_state(&self, connection_id: &str, database: &str) -> AppResult<TagState> {
        let sql = format!(
            "SELECT {COLUMNS} FROM table_tags WHERE connection_id = ?1 AND database = ?2 ORDER BY schema, position, lower(name)"
        );
        let mut stmt = self.conn().prepare(&sql).map_err(AppError::store)?;
        let tags = stmt
            .query_map(params![connection_id, database], from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;

        let mut stmt = self
            .conn()
            .prepare(
                "SELECT l.tag_id, l.table_name FROM table_tag_links l JOIN table_tags t ON t.id = l.tag_id \
                 WHERE t.connection_id = ?1 AND t.database = ?2 ORDER BY l.table_name",
            )
            .map_err(AppError::store)?;
        let links = stmt
            .query_map(params![connection_id, database], |row| Ok(TableTagLink { tag_id: row.get(0)?, table: row.get(1)? }))
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;

        let mut stmt = self
            .conn()
            .prepare("SELECT schema FROM table_tag_scopes WHERE connection_id = ?1 AND database = ?2 AND untagged_collapsed = 1 ORDER BY schema")
            .map_err(AppError::store)?;
        let untagged_collapsed = stmt
            .query_map(params![connection_id, database], |row| row.get::<_, String>(0))
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;

        Ok(TagState { tags, links, untagged_collapsed })
    }

    pub fn get_tag(&self, id: &str) -> AppResult<TableTag> {
        let sql = format!("SELECT {COLUMNS} FROM table_tags WHERE id = ?1");
        self.conn()
            .query_row(&sql, params![id], from_row)
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::not_found("That tag no longer exists."))
    }

    // WHAT:  Creates a tag (`id` None) or renames / recolours one, refusing a name
    //        another tag of the same scope already has (case-insensitively).
    pub fn save_tag(&self, input: &TableTagInput) -> AppResult<TableTag> {
        let name = clean_tag_name(&input.name);
        if name.is_empty() {
            return Err(AppError::invalid_input("A tag needs a name."));
        }
        if name.chars().count() > MAX_TAG_NAME_LEN {
            return Err(AppError::invalid_input(format!("A tag name is at most {MAX_TAG_NAME_LEN} characters.")));
        }
        if name.eq_ignore_ascii_case("untagged") {
            return Err(AppError::invalid_input("\"Untagged\" is the fixed group of tables without a tag; pick another name."));
        }
        let taken: Option<String> = self
            .conn()
            .query_row(
                "SELECT id FROM table_tags WHERE connection_id = ?1 AND database = ?2 AND schema = ?3 AND lower(name) = lower(?4)",
                params![input.connection_id, input.database, input.schema, name],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::store)?;
        if let Some(existing) = taken {
            if input.id.as_deref() != Some(existing.as_str()) {
                return Err(AppError::invalid_input(format!("A tag named \"{name}\" already exists here.")));
            }
        }
        match &input.id {
            Some(id) => {
                let changed = self
                    .conn()
                    .execute("UPDATE table_tags SET name = ?2, color = ?3 WHERE id = ?1", params![id, name, input.color.as_str()])
                    .map_err(AppError::store)?;
                if changed == 0 {
                    return Err(AppError::not_found("That tag no longer exists."));
                }
                self.get_tag(id)
            }
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                let next: i64 = self
                    .conn()
                    .query_row(
                        "SELECT COALESCE(MAX(position) + 1, 0) FROM table_tags WHERE connection_id = ?1 AND database = ?2 AND schema = ?3",
                        params![input.connection_id, input.database, input.schema],
                        |row| row.get(0),
                    )
                    .map_err(AppError::store)?;
                self.conn()
                    .execute(
                        "INSERT INTO table_tags (id, connection_id, database, schema, name, color, position, collapsed, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
                        params![id, input.connection_id, input.database, input.schema, name, input.color.as_str(), next, now_rfc3339()],
                    )
                    .map_err(AppError::store)?;
                self.get_tag(&id)
            }
        }
    }

    pub fn delete_tag(&self, id: &str) -> AppResult<()> {
        self.conn().execute("DELETE FROM table_tags WHERE id = ?1", params![id]).map_err(AppError::store)?;
        Ok(())
    }

    // WHAT:  Rewrites `position` so the scope's tags follow `ids`; a tag the list
    //        leaves out keeps its place after the named ones.
    pub fn reorder_tags(&self, connection_id: &str, database: &str, schema: &str, ids: &[String]) -> AppResult<()> {
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        let mut stmt = tx
            .prepare("SELECT id FROM table_tags WHERE connection_id = ?1 AND database = ?2 AND schema = ?3 ORDER BY position, lower(name)")
            .map_err(AppError::store)?;
        let current = stmt
            .query_map(params![connection_id, database, schema], |row| row.get::<_, String>(0))
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        drop(stmt);
        let mut order: Vec<&String> = ids.iter().filter(|id| current.contains(id)).collect();
        for id in &current {
            if !order.contains(&id) {
                order.push(id);
            }
        }
        for (position, id) in order.iter().enumerate() {
            let position = i64::try_from(position).unwrap_or(i64::MAX);
            tx.execute("UPDATE table_tags SET position = ?2 WHERE id = ?1", params![id, position]).map_err(AppError::store)?;
        }
        tx.commit().map_err(AppError::store)
    }

    /// Adds `table` to the tag (`tagged`) or takes it off.
    pub fn tag_table(&self, tag_id: &str, table: &str, tagged: bool) -> AppResult<()> {
        if table.trim().is_empty() {
            return Err(AppError::invalid_input("A table name is required."));
        }
        if tagged {
            self.get_tag(tag_id)?;
            self.conn()
                .execute("INSERT OR IGNORE INTO table_tag_links (tag_id, table_name) VALUES (?1, ?2)", params![tag_id, table])
                .map_err(AppError::store)?;
        } else {
            self.conn()
                .execute("DELETE FROM table_tag_links WHERE tag_id = ?1 AND table_name = ?2", params![tag_id, table])
                .map_err(AppError::store)?;
        }
        Ok(())
    }

    // WHAT:  Folds or unfolds a tag's group, or (`tag_id` None) the scope's fixed
    //        "Untagged" group.
    pub fn set_tag_collapsed(&self, connection_id: &str, database: &str, schema: &str, tag_id: Option<&str>, collapsed: bool) -> AppResult<()> {
        let flag = i64::from(collapsed);
        match tag_id {
            Some(id) => {
                self.conn().execute("UPDATE table_tags SET collapsed = ?2 WHERE id = ?1", params![id, flag]).map_err(AppError::store)?;
            }
            None => {
                self.conn()
                    .execute(
                        "INSERT INTO table_tag_scopes (connection_id, database, schema, untagged_collapsed) VALUES (?1, ?2, ?3, ?4) \
                         ON CONFLICT(connection_id, database, schema) DO UPDATE SET untagged_collapsed = excluded.untagged_collapsed",
                        params![connection_id, database, schema, flag],
                    )
                    .map_err(AppError::store)?;
            }
        }
        Ok(())
    }

    // WHAT:  Follows a table through a rename (`to` Some) or forgets it after a drop
    //        (`to` None), so a tag does not point at a table that is gone.
    pub fn move_tagged_table(&self, connection_id: &str, database: &str, schema: &str, from: &str, to: Option<&str>) -> AppResult<()> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id FROM table_tags WHERE connection_id = ?1 AND database = ?2 AND schema = ?3")
            .map_err(AppError::store)?;
        let ids = stmt
            .query_map(params![connection_id, database, schema], |row| row.get::<_, String>(0))
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        drop(stmt);
        for id in ids {
            match to {
                Some(target) => {
                    // OR REPLACE: renaming onto a name that is already tagged keeps one link.
                    self.conn()
                        .execute("UPDATE OR REPLACE table_tag_links SET table_name = ?3 WHERE tag_id = ?1 AND table_name = ?2", params![id, from, target])
                        .map_err(AppError::store)?;
                }
                None => {
                    self.conn()
                        .execute("DELETE FROM table_tag_links WHERE tag_id = ?1 AND table_name = ?2", params![id, from])
                        .map_err(AppError::store)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_connection() -> (Store, String) {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let id = "conn-1".to_string();
        store
            .conn()
            .execute(
                "INSERT INTO connections (id, name, engine, environment, created_at, updated_at) VALUES (?1, 'c', 'postgres', 'local', 'now', 'now')",
                params![id],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        (store, id)
    }

    fn input(connection_id: &str, schema: &str, name: &str) -> TableTagInput {
        TableTagInput {
            id: None,
            connection_id: connection_id.into(),
            database: "app".into(),
            schema: schema.into(),
            name: name.into(),
            color: ConnectionColor::Blue,
        }
    }

    #[test]
    fn names_are_unique_per_scope_ignoring_case_and_spacing() {
        let (store, conn) = store_with_connection();
        let first = store.save_tag(&input(&conn, "public", "Billing")).unwrap_or_else(|e| panic!("{e}"));
        let dup = store.save_tag(&input(&conn, "public", "  billing ")).err();
        assert!(matches!(dup, Some(AppError::InvalidInput { .. })), "{dup:?}");
        // The same name in another schema is another tag.
        assert!(store.save_tag(&input(&conn, "audit", "Billing")).is_ok());
        // Editing a tag may keep its own name.
        let renamed = store
            .save_tag(&TableTagInput { id: Some(first.id.clone()), color: ConnectionColor::Red, ..input(&conn, "public", "BILLING") })
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((renamed.name.as_str(), renamed.color), ("BILLING", ConnectionColor::Red));
        assert!(store.save_tag(&input(&conn, "public", "untagged")).is_err(), "reserved");
        assert!(store.save_tag(&input(&conn, "public", "   ")).is_err());
    }

    #[test]
    fn a_table_carries_several_tags_and_links_follow_renames_and_drops() {
        let (store, conn) = store_with_connection();
        let a = store.save_tag(&input(&conn, "public", "A")).unwrap_or_else(|e| panic!("{e}"));
        let b = store.save_tag(&input(&conn, "public", "B")).unwrap_or_else(|e| panic!("{e}"));
        store.tag_table(&a.id, "orders", true).unwrap_or_else(|e| panic!("{e}"));
        store.tag_table(&b.id, "orders", true).unwrap_or_else(|e| panic!("{e}"));
        store.tag_table(&b.id, "orders", true).unwrap_or_else(|e| panic!("{e}"));
        let state = store.list_tag_state(&conn, "app").unwrap_or_default();
        assert_eq!(state.links.len(), 2);

        store.move_tagged_table(&conn, "app", "public", "orders", Some("orders_v2")).unwrap_or_else(|e| panic!("{e}"));
        let state = store.list_tag_state(&conn, "app").unwrap_or_default();
        assert!(state.links.iter().all(|l| l.table == "orders_v2"));

        store.tag_table(&a.id, "orders_v2", false).unwrap_or_else(|e| panic!("{e}"));
        store.move_tagged_table(&conn, "app", "public", "orders_v2", None).unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_tag_state(&conn, "app").unwrap_or_default().links.is_empty());
        assert!(store.tag_table("missing", "t", true).is_err());
    }

    #[test]
    fn order_and_collapse_persist_and_deletes_cascade() {
        let (store, conn) = store_with_connection();
        let a = store.save_tag(&input(&conn, "public", "A")).unwrap_or_else(|e| panic!("{e}"));
        let b = store.save_tag(&input(&conn, "public", "B")).unwrap_or_else(|e| panic!("{e}"));
        let c = store.save_tag(&input(&conn, "public", "C")).unwrap_or_else(|e| panic!("{e}"));
        store.reorder_tags(&conn, "app", "public", &[c.id.clone(), a.id.clone()]).unwrap_or_else(|e| panic!("{e}"));
        let names: Vec<String> = store.list_tag_state(&conn, "app").unwrap_or_default().tags.into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["C", "A", "B"], "unnamed tags keep their place after the named ones");

        store.set_tag_collapsed(&conn, "app", "public", Some(&a.id), true).unwrap_or_else(|e| panic!("{e}"));
        store.set_tag_collapsed(&conn, "app", "public", None, true).unwrap_or_else(|e| panic!("{e}"));
        let state = store.list_tag_state(&conn, "app").unwrap_or_default();
        assert!(state.tags.iter().any(|t| t.id == a.id && t.collapsed));
        assert_eq!(state.untagged_collapsed, vec!["public".to_string()]);
        store.set_tag_collapsed(&conn, "app", "public", None, false).unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_tag_state(&conn, "app").unwrap_or_default().untagged_collapsed.is_empty());

        store.tag_table(&b.id, "t", true).unwrap_or_else(|e| panic!("{e}"));
        store.delete_tag(&b.id).unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_tag_state(&conn, "app").unwrap_or_default().links.is_empty(), "links go with their tag");
        // Another database on the same connection has its own tags.
        assert!(store.list_tag_state(&conn, "other").unwrap_or_default().tags.is_empty());
    }
}
