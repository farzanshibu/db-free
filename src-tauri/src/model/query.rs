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

// WHAT:  One page of a table for the grid.
// HOW:   `total` is exact when filters are applied (a count query runs), otherwise
//        the engine's cheap estimate; `total_exact` says which.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TablePage {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<Value>>,
    pub offset: u64,
    pub total: Option<i64>,
    pub total_exact: bool,
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
