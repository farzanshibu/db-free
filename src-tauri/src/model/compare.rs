// SOT: compare-direction, diff-status, schema-diff, table-diff, column-diff, column-change, foreign-key-diff, data-compare, row-diff, row-status

use crate::model::connection::Engine;
use crate::model::schema::{ColumnInfo, ForeignKey, TableRef};
use crate::model::value::Value;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ============================================================================
// COMPARE MODEL
//
// WHAT:  Shapes shared by schema compare and data compare: which side is the
//        source of truth, and what differs between the two sides.
// WHY:   Both features answer "what would it take to make B look like A" —
//        a diff the UI can browse plus a script the user can run — so they
//        share the direction and status vocabulary.
// HOW:   Every diff keeps both sides (`left` / `right`) so the UI can draw
//        them side by side; `status` is read in the chosen direction:
//        Added = present in the source, missing from the target (the script
//        creates it), Removed = only in the target (the script drops it).
// WHERE: src-tauri/src/services/{schema_diff,data_compare}.rs,
//        src/features/compare/{SchemaCompareTab,DataCompareTab}.tsx
// ============================================================================

// WHAT:  Which side the script changes. LeftToRight = the right side is made to
//        match the left (left is the source, right the target).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CompareDirection {
    LeftToRight,
    RightToLeft,
}

impl CompareDirection {
    /// True when the left side is the source of truth.
    pub fn left_is_source(self) -> bool {
        self == CompareDirection::LeftToRight
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiffStatus {
    /// In the source only: the script adds it to the target.
    Added,
    /// In the target only: the script removes it.
    Removed,
    Changed,
    Identical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ColumnChange {
    Type,
    Nullable,
    PrimaryKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnDiff {
    pub name: String,
    pub status: DiffStatus,
    pub left: Option<ColumnInfo>,
    pub right: Option<ColumnInfo>,
    pub changes: Vec<ColumnChange>,
}

// WHAT:  A foreign key present on one side only (keys are matched by columns
//        and referenced table, not by constraint name, which engines generate).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForeignKeyDiff {
    pub status: DiffStatus,
    pub left: Option<ForeignKey>,
    pub right: Option<ForeignKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableDiff {
    pub name: String,
    pub status: DiffStatus,
    pub left: Option<TableRef>,
    pub right: Option<TableRef>,
    pub columns: Vec<ColumnDiff>,
    pub foreign_keys: Vec<ForeignKeyDiff>,
}

// WHAT:  The whole schema comparison: every table (identical ones too, so the
//        UI can count them), and the migration script in the target's dialect.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SchemaDiff {
    pub direction: CompareDirection,
    pub left_engine: Engine,
    pub right_engine: Engine,
    pub tables: Vec<TableDiff>,
    pub script: String,
    /// Caveats worth reading before running the script (cross-engine compare…).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RowStatus {
    OnlyLeft,
    OnlyRight,
    Different,
}

// WHAT:  One row that is not identical on both sides. `left` / `right` hold
//        the compared columns in `DataCompare::columns` order.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RowDiff {
    pub status: RowStatus,
    pub key: Vec<Value>,
    pub left: Option<Vec<Value>>,
    pub right: Option<Vec<Value>>,
    /// Columns whose values differ (Different rows only).
    pub changed: Vec<String>,
}

// WHAT:  Row-level comparison of two tables keyed by `key_columns`.
// HOW:   Counts cover every row read; `rows` stops at a display cap
//        (`rows_truncated`). A side that hit `max_rows` is `*_capped`: rows
//        past the cap were not read, so its only-left / only-right counts
//        can be overstated near the end of the key range.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataCompare {
    pub direction: CompareDirection,
    /// Columns present on both sides, in the left table's order.
    pub columns: Vec<ColumnInfo>,
    pub key_columns: Vec<String>,
    pub left_only_columns: Vec<String>,
    pub right_only_columns: Vec<String>,
    pub rows: Vec<RowDiff>,
    pub rows_truncated: bool,
    pub only_left: u64,
    pub only_right: u64,
    pub different: u64,
    pub identical: u64,
    pub left_rows: u64,
    pub right_rows: u64,
    pub left_capped: bool,
    pub right_capped: bool,
    /// INSERT / UPDATE / DELETE statements that make the target match the
    /// source. None when not asked for or the target speaks no SQL.
    pub script: Option<String>,
    pub notes: Vec<String>,
}
