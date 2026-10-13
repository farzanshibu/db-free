// SOT: table-tags-service, tag-state-loading, tag-table-assignment, tag-reorder

use crate::error::AppResult;
use crate::model::{TableTag, TableTagInput, TagState};
use crate::state::AppState;

// WHAT:  The use cases behind the sidebar's Tags view: read a connection's tags,
//        create / edit / delete one, pin a table to it, fold its group, reorder.
// WHY:   The rules (unique names per scope, reserved "Untagged", length) live in
//        the store next to the unique index that backs them; this layer is the one
//        place commands reach them, so a rule added later has one door to guard.
// WHERE: src-tauri/src/store/tags.rs, src-tauri/src/commands/tags.rs

pub fn state(app: &AppState, connection_id: &str, database: &str) -> AppResult<TagState> {
    app.with_store(|store| store.list_tag_state(connection_id, database))
}

pub fn save(app: &AppState, input: &TableTagInput) -> AppResult<TableTag> {
    app.with_store(|store| store.save_tag(input))
}

pub fn delete(app: &AppState, id: &str) -> AppResult<()> {
    app.with_store(|store| store.delete_tag(id))
}

pub fn reorder(app: &AppState, connection_id: &str, database: &str, schema: &str, ids: &[String]) -> AppResult<()> {
    app.with_store(|store| store.reorder_tags(connection_id, database, schema, ids))
}

pub fn tag_table(app: &AppState, tag_id: &str, table: &str, tagged: bool) -> AppResult<()> {
    app.with_store(|store| store.tag_table(tag_id, table, tagged))
}

pub fn set_collapsed(app: &AppState, connection_id: &str, database: &str, schema: &str, tag_id: Option<&str>, collapsed: bool) -> AppResult<()> {
    app.with_store(|store| store.set_tag_collapsed(connection_id, database, schema, tag_id, collapsed))
}

pub fn move_table(app: &AppState, connection_id: &str, database: &str, schema: &str, from: &str, to: Option<&str>) -> AppResult<()> {
    app.with_store(|store| store.move_tagged_table(connection_id, database, schema, from, to))
}
