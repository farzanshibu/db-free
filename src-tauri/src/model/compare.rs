// SOT: compare-direction, diff-status, schema-diff, table-diff, column-diff, column-change, foreign-key-diff

use crate::model::connection::Engine;
use crate::model::schema::{ColumnInfo, ForeignKey, TableRef};
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
