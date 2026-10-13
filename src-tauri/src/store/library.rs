// SOT: library-folders-store, library-placements-store, folder-tree-persistence

use crate::error::{AppError, AppResult};
use crate::model::{LibraryFolder, LibraryLayout, LibraryPlacement, LibraryScope};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, OptionalExtension, Row};

/// Longest folder name accepted; the sidebars truncate long ones anyway.
pub const MAX_FOLDER_NAME: usize = 120;

const FOLDER_COLUMNS: &str = "id, scope, parent_id, name, sort_order, created_at, updated_at";

fn parse_scope(index: usize, raw: &str) -> rusqlite::Result<LibraryScope> {
    LibraryScope::parse(raw)
        .ok_or_else(|| rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, format!("unknown library scope {raw}").into()))
}

fn folder_from_row(row: &Row<'_>) -> rusqlite::Result<LibraryFolder> {
    let scope_raw: String = row.get(1)?;
    Ok(LibraryFolder {
        id: row.get(0)?,
        scope: parse_scope(1, &scope_raw)?,
        parent_id: row.get(2)?,
        name: row.get(3)?,
        sort_order: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn placement_from_row(row: &Row<'_>) -> rusqlite::Result<LibraryPlacement> {
    let scope_raw: String = row.get(0)?;
    Ok(LibraryPlacement { scope: parse_scope(0, &scope_raw)?, item_id: row.get(1)?, folder_id: row.get(2)? })
}

// WHAT:  A folder name that is safe to store: trimmed, non-empty, not absurdly long.
pub fn clean_folder_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::invalid_input("A folder needs a name."));
    }
    if name.chars().count() > MAX_FOLDER_NAME {
        return Err(AppError::invalid_input(format!("A folder name can be at most {MAX_FOLDER_NAME} characters.")));
    }
    Ok(name.to_string())
}

impl Store {
    // WHAT:  Every folder and every placement, for the sidebars to join with their item lists.
    pub fn list_library(&self) -> AppResult<LibraryLayout> {
        let conn = self.conn();
        let mut folders_stmt = conn
            .prepare(&format!("SELECT {FOLDER_COLUMNS} FROM library_folders ORDER BY scope, sort_order, lower(name)"))
            .map_err(AppError::store)?;
        let folders = folders_stmt
            .query_map([], folder_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        let mut placements_stmt = conn
            .prepare("SELECT scope, item_id, folder_id FROM library_placements")
            .map_err(AppError::store)?;
        let placements = placements_stmt
            .query_map([], placement_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(LibraryLayout { folders, placements })
    }

    pub fn get_folder(&self, id: &str) -> AppResult<LibraryFolder> {
        self.conn()
            .query_row(&format!("SELECT {FOLDER_COLUMNS} FROM library_folders WHERE id = ?1"), params![id], folder_from_row)
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::not_found(format!("Folder {id} does not exist.")))
    }

    // WHAT:  True when a sibling of the same scope and parent already has this
    //        name (case-insensitive), ignoring `except`.
    fn folder_name_taken(&self, scope: LibraryScope, parent_id: Option<&str>, name: &str, except: Option<&str>) -> AppResult<bool> {
        let found: Option<i64> = self
            .conn()
            .query_row(
                "SELECT 1 FROM library_folders WHERE scope = ?1 AND parent_id IS ?2 AND lower(name) = lower(?3) AND id IS NOT ?4 LIMIT 1",
                params![scope.as_str(), parent_id, name, except],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::store)?;
        Ok(found.is_some())
    }

    // WHAT:  A parent must exist and belong to the same scope as the folder going under it.
    fn require_parent(&self, scope: LibraryScope, parent_id: Option<&str>) -> AppResult<()> {
        let Some(parent_id) = parent_id else { return Ok(()) };
        let parent = self.get_folder(parent_id)?;
        if parent.scope != scope {
            return Err(AppError::invalid_input("A folder can only go inside a folder from the same list."));
        }
        Ok(())
    }

    pub fn create_folder(&self, scope: LibraryScope, parent_id: Option<&str>, name: &str) -> AppResult<LibraryFolder> {
        let name = clean_folder_name(name)?;
        self.require_parent(scope, parent_id)?;
        if self.folder_name_taken(scope, parent_id, &name, None)? {
            return Err(AppError::invalid_input(format!("A folder named “{name}” already exists here.")));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let next_order: i64 = self
            .conn()
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM library_folders WHERE scope = ?1 AND parent_id IS ?2",
                params![scope.as_str(), parent_id],
                |row| row.get(0),
            )
            .map_err(AppError::store)?;
        self.conn()
            .execute(
                "INSERT INTO library_folders (id, scope, parent_id, name, sort_order, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![id, scope.as_str(), parent_id, name, next_order, now],
            )
            .map_err(AppError::store)?;
        self.get_folder(&id)
    }

    pub fn rename_folder(&self, id: &str, name: &str) -> AppResult<LibraryFolder> {
        let name = clean_folder_name(name)?;
        let folder = self.get_folder(id)?;
        if self.folder_name_taken(folder.scope, folder.parent_id.as_deref(), &name, Some(id))? {
            return Err(AppError::invalid_input(format!("A folder named “{name}” already exists here.")));
        }
        self.conn()
            .execute("UPDATE library_folders SET name = ?2, updated_at = ?3 WHERE id = ?1", params![id, name, now_rfc3339()])
            .map_err(AppError::store)?;
        self.get_folder(id)
    }

    // WHAT:  True when `candidate` is `folder` itself or lies anywhere beneath it.
    // WHY:   Moving a folder into its own subtree would detach the whole branch.
    fn is_self_or_descendant(&self, folder_id: &str, candidate: &str) -> AppResult<bool> {
        let mut current = Some(candidate.to_string());
        // The depth bound stops a corrupt parent cycle from looping forever.
        for _ in 0..256 {
            let Some(id) = current else { return Ok(false) };
            if id == folder_id {
                return Ok(true);
            }
            current = self
                .conn()
                .query_row("SELECT parent_id FROM library_folders WHERE id = ?1", params![id], |row| row.get::<_, Option<String>>(0))
                .optional()
                .map_err(AppError::store)?
                .flatten();
        }
        Ok(true)
    }

    pub fn move_folder(&self, id: &str, parent_id: Option<&str>) -> AppResult<LibraryFolder> {
        let folder = self.get_folder(id)?;
        self.require_parent(folder.scope, parent_id)?;
        if let Some(target) = parent_id {
            if self.is_self_or_descendant(id, target)? {
                return Err(AppError::invalid_input("A folder cannot be moved into itself or one of its own folders."));
            }
        }
        if folder.parent_id.as_deref() != parent_id && self.folder_name_taken(folder.scope, parent_id, &folder.name, Some(id))? {
            return Err(AppError::invalid_input(format!("A folder named “{}” already exists there.", folder.name)));
        }
        let next_order: i64 = self
            .conn()
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM library_folders WHERE scope = ?1 AND parent_id IS ?2",
                params![folder.scope.as_str(), parent_id],
                |row| row.get(0),
            )
            .map_err(AppError::store)?;
        self.conn()
            .execute(
                "UPDATE library_folders SET parent_id = ?2, sort_order = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, parent_id, next_order, now_rfc3339()],
            )
            .map_err(AppError::store)?;
        self.get_folder(id)
    }

    // WHAT:  Deletes a folder without deleting anything in it: its subfolders and
    //        items move up to the folder's own parent (the top level for a root folder).
    // WHY:   Removing a folder must never destroy a dashboard or a saved query as a
    //        side effect; deleting an item is its own, confirmed action.
    pub fn delete_folder(&self, id: &str) -> AppResult<()> {
        let folder = self.get_folder(id)?;
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        tx.execute("UPDATE library_folders SET parent_id = ?2 WHERE parent_id = ?1", params![id, folder.parent_id]).map_err(AppError::store)?;
        match folder.parent_id.as_deref() {
            Some(parent) => {
                tx.execute("UPDATE library_placements SET folder_id = ?2 WHERE folder_id = ?1", params![id, parent]).map_err(AppError::store)?;
            }
            None => {
                tx.execute("DELETE FROM library_placements WHERE folder_id = ?1", params![id]).map_err(AppError::store)?;
            }
        }
        tx.execute("DELETE FROM library_folders WHERE id = ?1", params![id]).map_err(AppError::store)?;
        tx.commit().map_err(AppError::store)
    }

    // WHAT:  Files an item into a folder, or takes it out (None = top level).
    pub fn place_item(&self, scope: LibraryScope, item_id: &str, folder_id: Option<&str>) -> AppResult<()> {
        match folder_id {
            Some(folder_id) => {
                let folder = self.get_folder(folder_id)?;
                if folder.scope != scope {
                    return Err(AppError::invalid_input("That folder belongs to a different list."));
                }
                self.conn()
                    .execute(
                        "INSERT INTO library_placements (scope, item_id, folder_id) VALUES (?1, ?2, ?3) \
                         ON CONFLICT(scope, item_id) DO UPDATE SET folder_id = excluded.folder_id",
                        params![scope.as_str(), item_id, folder_id],
                    )
                    .map_err(AppError::store)?;
            }
            None => self.clear_placement(scope, item_id)?,
        }
        Ok(())
    }

    pub fn folder_of(&self, scope: LibraryScope, item_id: &str) -> AppResult<Option<String>> {
        self.conn()
            .query_row(
                "SELECT folder_id FROM library_placements WHERE scope = ?1 AND item_id = ?2",
                params![scope.as_str(), item_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::store)
    }

    // WHAT:  True when a saved connection with this id exists.
    // WHY:   "Copy to connection" must refuse a target that was deleted since the
    //        list was drawn, instead of filing a copy under a ghost.
    pub fn connection_exists(&self, id: &str) -> AppResult<bool> {
        let found: Option<i64> = self
            .conn()
            .query_row("SELECT 1 FROM connections WHERE id = ?1", params![id], |row| row.get(0))
            .optional()
            .map_err(AppError::store)?;
        Ok(found.is_some())
    }

    // WHAT:  Forgets where an item was filed; called when the item itself is deleted.
    pub fn clear_placement(&self, scope: LibraryScope, item_id: &str) -> AppResult<()> {
        self.conn()
            .execute("DELETE FROM library_placements WHERE scope = ?1 AND item_id = ?2", params![scope.as_str(), item_id])
            .map_err(AppError::store)?;
        Ok(())
    }
}

// WHAT:  A bare connection row, for tests that need a connection to point at.
// WHY:   Service tests cannot name `rusqlite` (only the store may), and the pin,
//        copy-to-connection and prune rules all look connections up.
#[cfg(test)]
impl Store {
    pub fn insert_test_connection(&self, id: &str) -> AppResult<()> {
        self.conn()
            .execute(
                "INSERT INTO connections (id, name, engine, environment, created_at, updated_at) VALUES (?1, ?1, 'sqlite', 'none', 'now', 'now')",
                params![id],
            )
            .map_err(AppError::store)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn folders_nest_rename_and_reject_duplicates() {
        let store = store();
        let reports = store.create_folder(LibraryScope::Dashboard, None, "  Reports ").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(reports.name, "Reports");
        let nested = store.create_folder(LibraryScope::Dashboard, Some(&reports.id), "Weekly").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(nested.parent_id.as_deref(), Some(reports.id.as_str()));
        // The same name is fine under a different parent or in another scope.
        assert!(store.create_folder(LibraryScope::Dashboard, None, "Weekly").is_ok());
        assert!(store.create_folder(LibraryScope::Workflow, None, "reports").is_ok());
        // Not under the same parent, in any letter case.
        assert!(store.create_folder(LibraryScope::Dashboard, None, "REPORTS").is_err());
        assert!(store.create_folder(LibraryScope::Dashboard, None, "   ").is_err());
        let renamed = store.rename_folder(&nested.id, "Monthly").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(renamed.name, "Monthly");
        assert!(store.rename_folder(&nested.id, "Monthly").is_ok(), "renaming to its own name is a no-op");
        // A parent from another scope is refused.
        assert!(store.create_folder(LibraryScope::Diagram, Some(&reports.id), "Nope").is_err());
    }

    #[test]
    fn moving_a_folder_into_its_own_subtree_is_refused() {
        let store = store();
        let a = store.create_folder(LibraryScope::SavedQuery, None, "A").unwrap_or_else(|e| panic!("{e}"));
        let b = store.create_folder(LibraryScope::SavedQuery, Some(&a.id), "B").unwrap_or_else(|e| panic!("{e}"));
        let c = store.create_folder(LibraryScope::SavedQuery, Some(&b.id), "C").unwrap_or_else(|e| panic!("{e}"));
        assert!(store.move_folder(&a.id, Some(&c.id)).is_err());
        assert!(store.move_folder(&a.id, Some(&a.id)).is_err());
        let moved = store.move_folder(&c.id, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(moved.parent_id, None);
        let back = store.move_folder(&c.id, Some(&a.id)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.parent_id.as_deref(), Some(a.id.as_str()));
    }

    #[test]
    fn items_are_filed_and_survive_their_folder_being_deleted() {
        let store = store();
        let outer = store.create_folder(LibraryScope::Workflow, None, "Ops").unwrap_or_else(|e| panic!("{e}"));
        let inner = store.create_folder(LibraryScope::Workflow, Some(&outer.id), "Nightly").unwrap_or_else(|e| panic!("{e}"));
        store.place_item(LibraryScope::Workflow, "wf-1", Some(&inner.id)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.folder_of(LibraryScope::Workflow, "wf-1").unwrap_or_default().as_deref(), Some(inner.id.as_str()));
        // Another list's folder is not a valid target.
        assert!(store.place_item(LibraryScope::Dashboard, "d-1", Some(&inner.id)).is_err());
        // Deleting the inner folder lifts its item to the outer one.
        store.delete_folder(&inner.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.folder_of(LibraryScope::Workflow, "wf-1").unwrap_or_default().as_deref(), Some(outer.id.as_str()));
        // Deleting a root folder lifts it to the top level.
        store.delete_folder(&outer.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.folder_of(LibraryScope::Workflow, "wf-1").unwrap_or_default(), None);
        let layout = store.list_library().unwrap_or_default();
        assert!(layout.folders.is_empty() && layout.placements.is_empty());
    }

    #[test]
    fn deleting_a_folder_keeps_its_subfolders() {
        let store = store();
        let a = store.create_folder(LibraryScope::Diagram, None, "A").unwrap_or_else(|e| panic!("{e}"));
        let b = store.create_folder(LibraryScope::Diagram, Some(&a.id), "B").unwrap_or_else(|e| panic!("{e}"));
        store.delete_folder(&a.id).unwrap_or_else(|e| panic!("{e}"));
        let kept = store.get_folder(&b.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(kept.parent_id, None);
    }

    #[test]
    fn placing_at_the_top_level_clears_the_placement() {
        let store = store();
        let f = store.create_folder(LibraryScope::SavedQuery, None, "F").unwrap_or_else(|e| panic!("{e}"));
        store.place_item(LibraryScope::SavedQuery, "q-1", Some(&f.id)).unwrap_or_else(|e| panic!("{e}"));
        store.place_item(LibraryScope::SavedQuery, "q-1", None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.folder_of(LibraryScope::SavedQuery, "q-1").unwrap_or_default(), None);
    }
}
