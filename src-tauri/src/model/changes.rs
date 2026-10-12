// SOT: staged-change, pending-changes-model, cell-edit, row-insert, row-delete, ddl-change, sql-token, commit-report, rows-as-sql

use crate::model::{TableRef, Value};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  A value the statement spells itself instead of quoting: the column's own
//        DEFAULT, or the current date / time.
// WHY:   `now()` typed into a date cell is not the text "now()"; quoting it would
//        store a string (or fail the cast). A fixed list, rendered per engine in
//        `services/changes.rs`, means the UI can never smuggle SQL through a value.
// WHERE: src-tauri/src/services/changes.rs (`token_sql`), src/lib/sqlTokens.ts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SqlToken {
    Default,
    Now,
    CurrentDate,
    CurrentTime,
    CurrentTimestamp,
}

// WHAT:  One column/value pair; the primary-key parts that address a row, or the
//        values of a new row.
// HOW:   `raw` set means `value` is only the stand-in the grid shows; the statement
//        uses the token unquoted. Omitted for every ordinary value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CellValue {
    pub column: String,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub raw: Option<SqlToken>,
}

// WHAT:  An edit queued in the Pending Changes panel (review mode) or applied at
//        once (direct mode). The SQL is generated on the Rust side so the UI
//        shows exactly what will run.
// HOW:   `Ddl` carries a finished statement (a column rename, a Redis `DEL`, an
//        index drop…) from a feature that builds its own; the commit runs it in
//        order with the row edits, and `destructive` makes the commit ask first.
// WHERE: src-tauri/src/services/changes.rs (SQL generation + commit)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum StagedChange {
    #[serde(rename_all = "camelCase")]
    Update {
        id: String,
        table: TableRef,
        key: Vec<CellValue>,
        column: String,
        old: Value,
        new: Value,
        /// The new value is this token, not `new` (which is only what the grid shows).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        new_raw: Option<SqlToken>,
    },
    #[serde(rename_all = "camelCase")]
    Insert { id: String, table: TableRef, values: Vec<CellValue> },
    #[serde(rename_all = "camelCase")]
    Delete { id: String, table: TableRef, key: Vec<CellValue> },
    #[serde(rename_all = "camelCase")]
    Ddl {
        id: String,
        /// What the statement is about; a schema-level statement leaves `name` empty.
        table: TableRef,
        statement: String,
        description: String,
        destructive: bool,
    },
}

impl StagedChange {
    pub fn id(&self) -> &str {
        match self {
            StagedChange::Update { id, .. }
            | StagedChange::Insert { id, .. }
            | StagedChange::Delete { id, .. }
            | StagedChange::Ddl { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangePreview {
    pub statements: Vec<String>,
    /// The full script including the transaction wrapper, exactly as it will run.
    pub script: String,
}

// WHAT:  The rows a statement handed back (`RETURNING *`): the state after the change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReturnedRows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

// WHAT:  What one staged change did.
// HOW:   `matched_nothing` is an update or delete that touched no row (already
//        changed or deleted elsewhere) — the commit still succeeded, but the
//        edit did not land, so the UI keeps it in the panel flagged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangeResult {
    pub id: String,
    pub statement: String,
    /// None when the engine did not report a count for this statement.
    pub rows_affected: Option<u64>,
    pub matched_nothing: bool,
    /// Present on engines with `RETURNING` for updates and inserts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub returned: Option<ReturnedRows>,
}

// WHAT:  The outcome of Commit: per-change results, and whether the commit was
//        one atomic transaction (false only on engines with no transactions).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommitReport {
    pub results: Vec<ChangeResult>,
    pub atomic: bool,
    pub elapsed_ms: u64,
}

// WHAT:  Which statement `rows_as_sql` writes for the rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RowSqlKind {
    Select,
    Insert,
    Update,
    Delete,
}
