// SOT: table-tag, table-tag-link, tag-state, tag-input

use super::connection::ConnectionColor;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  A label the user pins to tables of one schema so the sidebar can group them.
// WHY:   A schema with a hundred tables is not browsable alphabetically; people
//        think in "billing", "audit", "scratch". Tags are the app's own metadata,
//        so they live in the local store and never touch the user's database.
// HOW:   Scoped to (connection, database, schema): the same table name in another
//        schema, or another database on the same server, is another table. The
//        colour is one of the connection palette's presets (the same nine keys).
// WHERE: src-tauri/src/store/tags.rs, src/features/shell/TablesPanel.tsx
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableTag {
    pub id: String,
    pub connection_id: String,
    /// The database the session is on; empty for engines without one.
    pub database: String,
    /// The schema the tag belongs to; empty for engines without schemas.
    pub schema: String,
    pub name: String,
    pub color: ConnectionColor,
    /// Display order within the scope (0 first).
    pub position: u32,
    /// The group is folded in the Tags view.
    pub collapsed: bool,
}

/// One table carrying one tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableTagLink {
    pub tag_id: String,
    pub table: String,
}

/// Everything the Tags view needs for one connection + database, every schema at once.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TagState {
    pub tags: Vec<TableTag>,
    pub links: Vec<TableTagLink>,
    /// Schemas whose fixed "Untagged" group is folded.
    pub untagged_collapsed: Vec<String>,
}

/// Create (`id` None) or edit a tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableTagInput {
    pub id: Option<String>,
    pub connection_id: String,
    pub database: String,
    pub schema: String,
    pub name: String,
    pub color: ConnectionColor,
}

/// Longest tag name accepted.
pub const MAX_TAG_NAME_LEN: usize = 40;
