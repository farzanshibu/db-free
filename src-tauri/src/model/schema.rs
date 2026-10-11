// SOT: table-ref, column-info, schema-catalog, table-info, foreign-key

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  Identifies a table. `schema` is None for engines without namespaces (SQLite).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableRef {
    pub schema: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TableKind {
    Table,
    View,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableInfo {
    pub schema: Option<String>,
    pub name: String,
    pub kind: TableKind,
    pub row_estimate: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SchemaInfo {
    pub name: String,
    pub tables: Vec<TableInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SchemaCatalog {
    pub schemas: Vec<SchemaInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub ordinal: u32,
    // @parity wf
    // @end wf
    // @parity dash
    // @end dash
    // @parity diag
    // @end diag
    // @parity lib
    // @end lib
    // @parity grid
    // @end grid
    // @parity schema
    // @end schema
    // @parity editor
    // @end editor
    // @parity rx
    // @end rx
    // @parity conn
    // @end conn
    // @parity auth
    // @end auth
    // @parity shell
    // @end shell
    // @parity ai
    // @end ai
    // @parity engine
    // @end engine
}

// WHAT:  One foreign-key constraint, used to draw ER diagrams and FK traversal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForeignKey {
    pub name: String,
    pub from_schema: Option<String>,
    pub from_table: String,
    pub from_columns: Vec<String>,
    pub to_schema: Option<String>,
    pub to_table: String,
    pub to_columns: Vec<String>,
}
