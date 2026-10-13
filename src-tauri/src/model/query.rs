// SOT: result-set, statement-result, query-outcome, table-page, page-query, sort-rule, filter-rule, filter-op, history-entry, history-origin, saved-query, editor-buffer, statement-span, statement-intent

use crate::model::schema::ColumnInfo;
use crate::model::value::Value;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
}

// WHAT:  Rows returned by one statement. `truncated` is set when the row cap hit.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResultSet {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
}

impl ResultSet {
    pub fn row_count(&self) -> u64 {
        self.rows.len() as u64
    }
}

// WHAT:  One statement's place inside the editor's text, and what running it does.
// WHY:   PRD §4.3 — the gutter ▶ and Run at cursor execute one statement out of a
//        script, so the UI has to be told where each one starts and ends. It reads
//        the block's own tokenizer rather than shipping a second one.
// HOW:   `start` / `end` are UTF-16 code-unit offsets: the units a JavaScript string
//        and a CodeMirror position are counted in, so `sql.slice(start, end)` is the
//        statement exactly.
// WHERE: src-tauri/src/guard/destructive.rs (spans), src/features/editor/SqlEditor.tsx
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StatementSpan {
    pub start: u32,
    pub end: u32,
    pub intent: StatementIntent,
}

// WHAT:  What the block would do with a statement: run it, refuse it under a
//        read-only lock, or ask the user to confirm it first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StatementIntent {
    Read,
    Write,
    Destructive,
}

// WHAT:  Comparison operators the GUI filter builder offers (PRD §4.2).
// HOW:   `needs_value()` tells the UI which operators take an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FilterOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    In,
    IsNull,
    IsNotNull,
}

impl FilterOp {
    pub fn needs_value(self) -> bool {
        !matches!(self, FilterOp::IsNull | FilterOp::IsNotNull)
    }
}

// WHAT:  How a table's filter rules combine: every rule must hold (the default), or
//        any one of them.
// WHY:   `FilterRule`/`PageQuery` are built by dozens of adapters, so the mode rides
//        beside the query (`TablePageRequest`) instead of inside it.
// WHERE: src-tauri/src/integrations/sql.rs (`with_filter_mode`), src/features/grid/FilterPopover.tsx
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FilterMode {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FilterRule {
    pub column: String,
    pub op: FilterOp,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SortRule {
    pub column: String,
    pub desc: bool,
}

// WHAT:  Everything the browser needs to ask for one page: sort, filters, window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PageQuery {
    pub sort: Vec<SortRule>,
    pub filters: Vec<FilterRule>,
    pub offset: u64,
    pub limit: u32,
}

// WHAT:  What the grid needs to know about a column beyond `ColumnInfo`: the values
//        an enum / SET column allows, and whether the database fills it in itself.
// WHY:   `ColumnInfo` is built by every adapter, so these facts travel in their own
//        list (one entry per column, matched by name) that only the SQL adapters
//        that know them fill in.
// HOW:   `generated` and `auto_increment` columns are read-only in the grid; a
//        column that is NOT NULL with no default, no auto-increment and no
//        generation is `REQUIRED` on insert (derived in the UI).
// WHERE: src-tauri/src/integrations/mod.rs (`column_traits`), src/lib/columnTraits.ts
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnTraits {
    pub name: String,
    /// Allowed values of an ENUM / SET column (or an enum array). Empty otherwise.
    pub enum_values: Vec<String>,
    /// Several of `enum_values` may be held at once: MySQL SET, an enum array.
    pub multi: bool,
    pub has_default: bool,
    /// The default as the database spells it (`now()`, `0`, …), when it is readable.
    pub default_expr: Option<String>,
    /// The database computes the value (generated column, identity ALWAYS): never written.
    pub generated: bool,
    pub auto_increment: bool,
}

// WHAT:  A cell whose value was cut short for the page and where the full length
//        stands. The grid shows the prefix and fetches the rest by key on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TruncatedCell {
    pub row: u32,
    pub col: u32,
    /// The real size: characters for text and JSON, bytes for binary.
    pub size: u64,
}

// WHAT:  One page of a table for the grid.
// HOW:   `total` is exact when filters are applied (a count query runs) or the table
//        is small enough to count; otherwise the engine's cheap estimate.
//        `total_exact` says which. `column_traits` is empty when the engine cannot say.
//        `truncated` lists the cells cut to the requested size (see `truncate_over`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TablePage {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<Value>>,
    pub offset: u64,
    pub total: Option<i64>,
    pub total_exact: bool,
    pub column_traits: Vec<ColumnTraits>,
    pub truncated: Vec<TruncatedCell>,
    /// Rows can be inserted, updated and deleted through this connection's dialect.
    pub editable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum StatementResult {
    #[serde(rename_all = "camelCase")]
    Rows { result: ResultSet },
    #[serde(rename_all = "camelCase")]
    Affected { rows_affected: u64 },
}

// WHAT:  Everything one Run produced: a result per statement, how many rows the
//        script would return in full, and the wall time the block measured.
// HOW:   `total_rows` equals the rows in hand whenever nothing hit the row cap.
//        Once the cap hits it is the counted total when the engine can be asked
//        for one, and None when it cannot — so the UI can say "1,000 of 84,213"
//        or "1,000 (capped)" but never present a cap as the whole answer.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QueryOutcome {
    pub statements: Vec<StatementResult>,
    pub total_rows: Option<u64>,
    pub elapsed_ms: u64,
}

impl QueryOutcome {
    /// Rows returned plus rows affected — what the history log records.
    pub fn row_count(&self) -> u64 {
        self.statements
            .iter()
            .map(|s| match s {
                StatementResult::Rows { result } => result.row_count(),
                StatementResult::Affected { rows_affected } => *rows_affected,
            })
            .sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HistoryStatus {
    Ok,
    Error,
}

impl HistoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            HistoryStatus::Ok => "ok",
            HistoryStatus::Error => "error",
        }
    }

    pub fn parse(raw: &str) -> HistoryStatus {
        if raw == "ok" {
            HistoryStatus::Ok
        } else {
            HistoryStatus::Error
        }
    }
}

// WHAT:  Who issued a logged statement: the user (editor) or the app itself
//        (table pages, catalog probes). The history tab filters on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HistoryOrigin {
    User,
    System,
}

impl HistoryOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            HistoryOrigin::User => "user",
            HistoryOrigin::System => "system",
        }
    }

    pub fn parse(raw: &str) -> HistoryOrigin {
        if raw == "system" {
            HistoryOrigin::System
        } else {
            HistoryOrigin::User
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryEntry {
    pub id: i64,
    pub connection_id: String,
    pub sql: String,
    pub status: HistoryStatus,
    pub origin: HistoryOrigin,
    pub error: Option<String>,
    pub elapsed_ms: u64,
    pub row_count: Option<u64>,
    pub executed_at: String,
    /// Upper-cased verb of the first statement (`SELECT`, `UPDATE`, `CREATE`…).
    pub statement_type: Option<String>,
    /// `schema.table` the first statement names, when it names one.
    pub table_name: Option<String>,
    /// The strongest intent in the script: Read, Write or Destructive.
    pub operation: Option<StatementIntent>,
    /// The database user the statement ran as.
    pub user: Option<String>,
    /// Typed in the query editor (as opposed to sent by a dashboard, workflow, import…).
    pub from_editor: bool,
}

// WHAT:  An editor tab's unsaved text, persisted so restarts never lose work.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditorBuffer {
    pub id: String,
    pub connection_id: Option<String>,
    pub title: String,
    pub content: String,
    pub updated_at: String,
}

// WHAT:  A named, reusable query. `connection_id` None = available everywhere.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedQuery {
    pub id: String,
    pub connection_id: Option<String>,
    pub name: String,
    pub sql: String,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}
