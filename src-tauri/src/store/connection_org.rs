// SOT: connection-tags-table, connection-folders-table, connection-membership, connection-org-persistence

use crate::error::{AppError, AppResult};
use crate::model::connection::MAX_FOLDER_LEN;
use crate::model::connection_org::MAX_CONNECTION_TAG_NAME_LEN;
use crate::model::{ConnectionFolder, ConnectionOrg, ConnectionSummary, ConnectionTag, ConnectionTagColor};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

// ============================================================================
// WHAT:  Tags and folders for the connections page, and which connection is in
//        which. Folders and tags are entities; a connection holds any number of
//        each through a link table.
// WHY:   The old single free-text `folder` column could not express an empty
//        folder, a connection in two folders, a rename that touches one row, or
//        a coloured label.
// HOW:   Name uniqueness is case-insensitive and enforced twice: a pre-check
//        gives the form a readable message, the UNIQUE NOCASE index is the
//        backstop against two saves racing. Deleting a folder or tag removes
//        only the links (ON DELETE CASCADE); connections are never touched.
//        A save of a connection replaces its links with exactly what it names
//        (`replace_membership`); unknown ids are skipped, so a stale form that
//        still holds a deleted tag cannot fail the save.
// WHERE: src-tauri/src/store/connections.rs (save / list), migrations/connections.rs (schema)
// ============================================================================

fn tag_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConnectionTag> {
    let color_raw: String = row.get(2)?;
    Ok(ConnectionTag {
        id: row.get(0)?,
        name: row.get(1)?,
        color: ConnectionTagColor::parse(&color_raw).unwrap_or(ConnectionTagColor::Gray),
        is_preset: row.get::<_, i64>(3)? != 0,
        sort_order: i32::try_from(row.get::<_, i64>(4)?).unwrap_or(i32::MAX),
        connection_count: u32::try_from(row.get::<_, i64>(5)?).unwrap_or(0),
    })
}

const TAG_SELECT: &str = "SELECT t.id, t.name, t.color, t.is_preset, t.sort_order, \
                          (SELECT COUNT(*) FROM connection_tag_links l WHERE l.tag_id = t.id) \
                          FROM connection_tags t";

fn folder_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConnectionFolder> {
    Ok(ConnectionFolder {
        id: row.get(0)?,
        name: row.get(1)?,
        sort_order: i32::try_from(row.get::<_, i64>(2)?).unwrap_or(i32::MAX),
        connection_count: u32::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
    })
}

const FOLDER_SELECT: &str = "SELECT f.id, f.name, f.sort_order, \
                             (SELECT COUNT(*) FROM connection_folder_members m WHERE m.folder_id = f.id) \
                             FROM connection_folders f";

fn clean_name(raw: &str, limit: usize, noun: &str) -> AppResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::invalid_input(format!("{noun} name is required.")));
    }
    if name.chars().count() > limit {
        return Err(AppError::invalid_input(format!("{noun} name is too long (max {limit}).")));
    }
    Ok(name.to_string())
}

impl Store {
    pub fn list_connection_org(&self) -> AppResult<ConnectionOrg> {
        let conn = self.conn();
        let tags = {
            let sql = format!("{TAG_SELECT} ORDER BY t.sort_order, t.name COLLATE NOCASE");
            let mut stmt = conn.prepare(&sql).map_err(AppError::store)?;
            let rows = stmt.query_map([], tag_from_row).map_err(AppError::store)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(AppError::store)?
        };
        let folders = {
            let sql = format!("{FOLDER_SELECT} ORDER BY f.sort_order, f.name COLLATE NOCASE");
            let mut stmt = conn.prepare(&sql).map_err(AppError::store)?;
            let rows = stmt.query_map([], folder_from_row).map_err(AppError::store)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(AppError::store)?
        };
        Ok(ConnectionOrg { tags, folders })
    }

    fn tag_by_id(&self, id: &str) -> AppResult<ConnectionTag> {
        let sql = format!("{TAG_SELECT} WHERE t.id = ?1");
        self.conn()
            .query_row(&sql, params![id], tag_from_row)
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::not_found(format!("Tag {id} does not exist.")))
    }

    fn tag_id_named(&self, name: &str, except: Option<&str>) -> AppResult<Option<String>> {
        self.conn()
            .query_row(
                "SELECT id FROM connection_tags WHERE name = ?1 COLLATE NOCASE AND id != ?2",
                params![name, except.unwrap_or("")],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::store)
    }

    // WHAT:  Creates a tag, or renames / recolours the one with `id`.
    // HOW:   A name already used by another tag (any case) is refused with a
    //        message that names it.
    pub fn save_connection_tag(&self, id: Option<&str>, name: &str, color: ConnectionTagColor) -> AppResult<ConnectionTag> {
        let name = clean_name(name, MAX_CONNECTION_TAG_NAME_LEN, "Tag")?;
        if self.tag_id_named(&name, id)?.is_some() {
            return Err(AppError::invalid_input(format!("A tag named \"{name}\" already exists.")));
        }
        let saved_id = match id {
            Some(id) => {
                let changed = self
                    .conn()
                    .execute("UPDATE connection_tags SET name = ?2, color = ?3 WHERE id = ?1", params![id, name, color.as_str()])
                    .map_err(AppError::store)?;
                if changed == 0 {
                    return Err(AppError::not_found(format!("Tag {id} does not exist.")));
                }
                id.to_string()
            }
            None => {
                let new_id = uuid::Uuid::new_v4().to_string();
                let next: i64 = self
                    .conn()
                    .query_row("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM connection_tags", [], |row| row.get(0))
                    .map_err(AppError::store)?;
                self.conn()
                    .execute(
                        "INSERT INTO connection_tags (id, name, color, is_preset, sort_order, created_at) VALUES (?1, ?2, ?3, 0, ?4, ?5)",
                        params![new_id, name, color.as_str(), next, now_rfc3339()],
                    )
                    .map_err(AppError::store)?;
                new_id
            }
        };
        self.tag_by_id(&saved_id)
    }

    pub fn delete_connection_tag(&self, id: &str) -> AppResult<()> {
        let changed = self.conn().execute("DELETE FROM connection_tags WHERE id = ?1", params![id]).map_err(AppError::store)?;
        if changed == 0 {
            return Err(AppError::not_found(format!("Tag {id} does not exist.")));
        }
        Ok(())
    }

    fn folder_by_id(&self, id: &str) -> AppResult<ConnectionFolder> {
        let sql = format!("{FOLDER_SELECT} WHERE f.id = ?1");
        self.conn()
            .query_row(&sql, params![id], folder_from_row)
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::not_found(format!("Folder {id} does not exist.")))
    }

    // WHAT:  Creates a folder, or renames the one with `id`. Names are unique in any case.
    pub fn save_connection_folder(&self, id: Option<&str>, name: &str) -> AppResult<ConnectionFolder> {
        let name = clean_name(name, MAX_FOLDER_LEN, "Folder")?;
        let clash: Option<String> = self
            .conn()
            .query_row(
                "SELECT id FROM connection_folders WHERE name = ?1 COLLATE NOCASE AND id != ?2",
                params![name, id.unwrap_or("")],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::store)?;
        if clash.is_some() {
            return Err(AppError::invalid_input(format!("A folder named \"{name}\" already exists.")));
        }
        let saved_id = match id {
            Some(id) => {
                let changed = self
                    .conn()
                    .execute("UPDATE connection_folders SET name = ?2 WHERE id = ?1", params![id, name])
                    .map_err(AppError::store)?;
                if changed == 0 {
                    return Err(AppError::not_found(format!("Folder {id} does not exist.")));
                }
                id.to_string()
            }
            None => insert_folder(self.conn(), &name)?,
        };
        self.folder_by_id(&saved_id)
    }

    pub fn delete_connection_folder(&self, id: &str) -> AppResult<()> {
        let changed = self.conn().execute("DELETE FROM connection_folders WHERE id = ?1", params![id]).map_err(AppError::store)?;
        if changed == 0 {
            return Err(AppError::not_found(format!("Folder {id} does not exist.")));
        }
        Ok(())
    }

    // WHAT:  The folder called `name` (any case), created when missing.
    // WHY:   Import files and the legacy single-folder input name folders; both
    //        mean "this folder, make it if it is not there yet".
    pub fn ensure_connection_folder(&self, name: &str) -> AppResult<String> {
        ensure_folder(self.conn(), name)
    }

    // WHAT:  The tag called `name` (any case), created with `color` when missing.
    pub fn ensure_connection_tag(&self, name: &str, color: ConnectionTagColor) -> AppResult<String> {
        let name = clean_name(name, MAX_CONNECTION_TAG_NAME_LEN, "Tag")?;
        if let Some(existing) = self.tag_id_named(&name, None)? {
            return Ok(existing);
        }
        Ok(self.save_connection_tag(None, &name, color)?.id)
    }

    // WHAT:  Replaces a connection's tags and / or folders; `None` leaves that side alone.
    pub fn set_connection_membership(&self, connection_id: &str, tag_ids: Option<&[String]>, folder_ids: Option<&[String]>) -> AppResult<()> {
        let exists: bool = self
            .conn()
            .query_row("SELECT 1 FROM connections WHERE id = ?1", params![connection_id], |_| Ok(true))
            .optional()
            .map_err(AppError::store)?
            .unwrap_or(false);
        if !exists {
            return Err(AppError::not_found(format!("Connection {connection_id} does not exist.")));
        }
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        replace_membership(&tx, connection_id, tag_ids, folder_ids)?;
        tx.commit().map_err(AppError::store)
    }
}

fn insert_folder(conn: &Connection, name: &str) -> AppResult<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let next: i64 = conn
        .query_row("SELECT COALESCE(MAX(sort_order), -1) + 1 FROM connection_folders", [], |row| row.get(0))
        .map_err(AppError::store)?;
    conn.execute(
        "INSERT INTO connection_folders (id, name, sort_order, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, name, next, now_rfc3339()],
    )
    .map_err(AppError::store)?;
    Ok(id)
}

// WHAT:  The folder id for `name`, creating the folder when none matches.
pub(crate) fn ensure_folder(conn: &Connection, name: &str) -> AppResult<String> {
    let name = clean_name(name, MAX_FOLDER_LEN, "Folder")?;
    let existing: Option<String> = conn
        .query_row("SELECT id FROM connection_folders WHERE name = ?1 COLLATE NOCASE", params![name], |row| row.get(0))
        .optional()
        .map_err(AppError::store)?;
    match existing {
        Some(id) => Ok(id),
        None => insert_folder(conn, &name),
    }
}

// WHAT:  Writes a connection's tag and folder links, replacing the old ones.
// HOW:   Ids that no longer exist are skipped by the `SELECT … WHERE id = ?`
//        shape of the insert, never a constraint error.
pub(crate) fn replace_membership(conn: &Connection, connection_id: &str, tag_ids: Option<&[String]>, folder_ids: Option<&[String]>) -> AppResult<()> {
    if let Some(tag_ids) = tag_ids {
        conn.execute("DELETE FROM connection_tag_links WHERE connection_id = ?1", params![connection_id]).map_err(AppError::store)?;
        for tag_id in tag_ids {
            conn.execute(
                "INSERT OR IGNORE INTO connection_tag_links (connection_id, tag_id) SELECT ?1, id FROM connection_tags WHERE id = ?2",
                params![connection_id, tag_id],
            )
            .map_err(AppError::store)?;
        }
    }
    if let Some(folder_ids) = folder_ids {
        conn.execute("DELETE FROM connection_folder_members WHERE connection_id = ?1", params![connection_id]).map_err(AppError::store)?;
        for folder_id in folder_ids {
            conn.execute(
                "INSERT OR IGNORE INTO connection_folder_members (connection_id, folder_id) SELECT ?1, id FROM connection_folders WHERE id = ?2",
                params![connection_id, folder_id],
            )
            .map_err(AppError::store)?;
        }
    }
    Ok(())
}

// WHAT:  Fills `extra.tag_ids`, `extra.folder_ids` and the derived `folder`
//        (the first folder's name) on a batch of summaries.
// HOW:   Two queries for the whole batch, not two per connection; tags come back
//        in palette order, folders in sort order, so the "primary" folder is stable.
pub(crate) fn attach_membership(conn: &Connection, summaries: &mut [ConnectionSummary]) -> AppResult<()> {
    if summaries.is_empty() {
        return Ok(());
    }
    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT l.connection_id, l.tag_id FROM connection_tag_links l \
                 JOIN connection_tags t ON t.id = l.tag_id ORDER BY t.sort_order, t.name COLLATE NOCASE",
            )
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(AppError::store)?;
        for row in rows {
            let (connection_id, tag_id) = row.map_err(AppError::store)?;
            tags.entry(connection_id).or_default().push(tag_id);
        }
    }
    let mut folders: HashMap<String, Vec<(String, String)>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT m.connection_id, f.id, f.name FROM connection_folder_members m \
                 JOIN connection_folders f ON f.id = m.folder_id ORDER BY f.sort_order, f.name COLLATE NOCASE",
            )
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .map_err(AppError::store)?;
        for row in rows {
            let (connection_id, folder_id, name) = row.map_err(AppError::store)?;
            folders.entry(connection_id).or_default().push((folder_id, name));
        }
    }
    for summary in summaries.iter_mut() {
        summary.extra.tag_ids = tags.remove(&summary.id).unwrap_or_default();
        let memberships = folders.remove(&summary.id).unwrap_or_default();
        summary.folder = memberships.first().map(|(_, name)| name.clone());
        summary.extra.folder_ids = memberships.into_iter().map(|(id, _)| id).collect();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionExtras, ConnectionInput, Engine, Environment, SshTunnel, SslMode};

    fn store() -> Store {
        Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"))
    }

    fn input(name: &str) -> ConnectionInput {
        ConnectionInput {
            name: name.into(),
            engine: Engine::Sqlite,
            environment: Environment::Local,
            read_only: false,
            host: None,
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: Some("/tmp/a.db".into()),
            ssl_mode: SslMode::Disable,
            ssh: SshTunnel::default(),
            ssh_secret: None,
            folder: None,
            color: None,
            favorite: false,
            auth: Default::default(),
            auth_secrets: Default::default(),
            extra: ConnectionExtras::default(),
        }
    }

    #[test]
    fn five_presets_are_seeded_in_order() {
        let org = store().list_connection_org().unwrap_or_else(|e| panic!("{e}"));
        let names: Vec<&str> = org.tags.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Production", "Staging", "Development", "Testing", "Local"]);
        assert!(org.tags.iter().all(|t| t.is_preset));
        assert!(org.folders.is_empty());
    }

    #[test]
    fn tag_names_are_unique_in_any_case_and_bounded() {
        let store = store();
        let blue = store.save_connection_tag(None, "  Client A ", ConnectionTagColor::Blue).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(blue.name, "Client A");
        assert_eq!(blue.sort_order, 5, "after the five presets");
        let clash = store.save_connection_tag(None, "client a", ConnectionTagColor::Red);
        assert!(clash.is_err_and(|e| e.to_string().contains("already exists")));
        // Recolouring the same tag under its own name is not a clash.
        let recoloured = store.save_connection_tag(Some(&blue.id), "Client A", ConnectionTagColor::Teal).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(recoloured.color, ConnectionTagColor::Teal);
        assert!(store.save_connection_tag(None, "   ", ConnectionTagColor::Red).is_err());
        assert!(store.save_connection_tag(None, &"x".repeat(MAX_CONNECTION_TAG_NAME_LEN + 1), ConnectionTagColor::Red).is_err());
        store.delete_connection_tag(&blue.id).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(store.delete_connection_tag(&blue.id), Err(AppError::NotFound { .. })));
    }

    #[test]
    fn folders_can_be_empty_renamed_and_deleted_without_touching_connections() {
        let store = store();
        let folder = store.save_connection_folder(None, "Clients").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(folder.connection_count, 0, "an empty folder exists");
        assert!(store.save_connection_folder(None, "CLIENTS").is_err());
        let created = store.insert_connection(&input("a"), None).unwrap_or_else(|e| panic!("{e}"));
        store.set_connection_membership(&created.id, None, Some(std::slice::from_ref(&folder.id))).unwrap_or_else(|e| panic!("{e}"));
        let renamed = store.save_connection_folder(Some(&folder.id), "Customers").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(renamed.connection_count, 1);
        assert_eq!(store.get_connection(&created.id).unwrap_or_else(|e| panic!("{e}")).folder.as_deref(), Some("Customers"));
        store.delete_connection_folder(&folder.id).unwrap_or_else(|e| panic!("{e}"));
        let after = store.get_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(after.folder, None);
        assert!(after.extra.folder_ids.is_empty());
    }

    #[test]
    fn a_connection_can_hold_several_tags_and_folders_and_unknown_ids_are_skipped() {
        let store = store();
        let a = store.save_connection_folder(None, "A").unwrap_or_else(|e| panic!("{e}"));
        let b = store.save_connection_folder(None, "B").unwrap_or_else(|e| panic!("{e}"));
        let created = store.insert_connection(&input("multi"), None).unwrap_or_else(|e| panic!("{e}"));
        let tags = vec!["preset-production".to_string(), "preset-local".to_string(), "deleted-tag".to_string()];
        store.set_connection_membership(&created.id, Some(&tags), Some(&[b.id.clone(), a.id.clone()])).unwrap_or_else(|e| panic!("{e}"));
        let got = store.get_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(got.extra.tag_ids, ["preset-production", "preset-local"], "palette order; the stale id is dropped");
        assert_eq!(got.extra.folder_ids, [a.id.clone(), b.id.clone()], "sort order, not the order named");
        assert_eq!(got.folder.as_deref(), Some("A"));
        // `None` leaves a side alone.
        store.set_connection_membership(&created.id, Some(&[]), None).unwrap_or_else(|e| panic!("{e}"));
        let kept = store.get_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert!(kept.extra.tag_ids.is_empty());
        assert_eq!(kept.extra.folder_ids.len(), 2);
        let org = store.list_connection_org().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(org.folders.iter().find(|f| f.id == a.id).map(|f| f.connection_count), Some(1));
        assert!(store.set_connection_membership("missing", None, None).is_err());
    }

    #[test]
    fn ensure_finds_by_name_in_any_case_and_creates_otherwise() {
        let store = store();
        let first = store.ensure_connection_folder("Prod").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.ensure_connection_folder("  prod ").unwrap_or_default(), first);
        let tag = store.ensure_connection_tag("production", ConnectionTagColor::Blue).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(tag, "preset-production", "the preset is reused, not duplicated");
    }
}
