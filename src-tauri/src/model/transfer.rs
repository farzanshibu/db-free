// SOT: export-model, import-model, transfer-format, ai-model, plan-node, plan-report

use crate::model::TableRef;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TransferFormat {
    Csv,
    Json,
    Sql,
}

impl TransferFormat {
    pub fn extension(self) -> &'static str {
        match self {
            TransferFormat::Csv => "csv",
            TransferFormat::Json => "json",
            TransferFormat::Sql => "sql",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportedFile {
    pub table: TableRef,
    pub path: String,
    pub rows: u64,
}

// WHAT:  What an export wrote: one entry per table (a bundle repeats its one
//        path), the bytes on disk, and the tables the row cap cut short.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportReport {
    pub files: Vec<ExportedFile>,
    pub elapsed_ms: u64,
    /// Size of what was written (a bundle's single file, or every file of a folder).
    pub bytes: u64,
    /// One line per table that hit the row cap and was cut short.
    pub warnings: Vec<String>,
}

// ============================================================================
// EXPORT OPTIONS
//
// WHAT:  How rows become text: the format and every knob the export dialog has.
// WHY:   One options value serves the folder / bundle export, its preview, and
//        (mirrored by src/lib/export.ts) the clipboard and download of rows
//        already on screen, so a file and its preview cannot disagree.
// HOW:   Knobs that do not apply to the chosen format are ignored (`delimiter`
//        outside CSV, `pretty` outside JSON…); the dialog hides them.
// WHERE: src-tauri/src/services/export.rs, src/lib/export.ts,
//        src/features/transfer/ExportDialog.tsx
// ============================================================================
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExportFormat {
    Csv,
    Tsv,
    Json,
    Ndjson,
    Markdown,
    Sql,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Tsv => "tsv",
            ExportFormat::Json => "json",
            ExportFormat::Ndjson => "ndjson",
            ExportFormat::Markdown => "md",
            ExportFormat::Sql => "sql",
        }
    }

    /// CSV and TSV: header row, delimiter, null text, quoting.
    pub fn is_delimited(self) -> bool {
        matches!(self, ExportFormat::Csv | ExportFormat::Tsv)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExportJsonShape {
    /// `[ {row}, {row} ]`
    Array,
    /// `{ "table": "users", "rows": [ {row} ] }`
    Envelope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// CSV / TSV / Markdown: write the column names first.
    pub header: bool,
    /// CSV: the one-character field separator (TSV is always a tab).
    pub delimiter: String,
    /// CSV / TSV / Markdown: the text a NULL becomes.
    pub null_text: String,
    /// Start the file with a UTF-8 byte-order mark (Excel wants one).
    pub bom: bool,
    pub json_shape: ExportJsonShape,
    /// JSON: indent with two spaces.
    pub pretty: bool,
    /// CSV / TSV: a cell starting with `=` `+` `-` `@` (or a tab / CR) is
    /// prefixed with `'` so a spreadsheet does not run it as a formula.
    pub quote_formulas: bool,
    /// SQL: the CREATE TABLE statement; JSON of several tables: the `__schema` block.
    pub include_schema: bool,
    /// SQL / JSON: the rows. Off writes the structure alone.
    pub include_data: bool,
    /// SQL: rows per multi-row INSERT.
    pub batch_rows: u32,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            format: ExportFormat::Csv,
            header: true,
            delimiter: ",".to_string(),
            null_text: String::new(),
            bom: false,
            json_shape: ExportJsonShape::Array,
            pretty: true,
            quote_formulas: true,
            include_schema: false,
            include_data: true,
            batch_rows: 100,
        }
    }
}

// WHAT:  Where an export goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExportLayout {
    /// One file at the chosen path: a ZIP of per-table files (CSV, TSV, NDJSON,
    /// Markdown, several tables), one combined JSON, or one combined SQL script.
    Bundle,
    /// One file per table inside the chosen directory.
    Folder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExportStage {
    /// Streaming a table's rows.
    Writing,
    /// Closing the archive and moving it into place.
    Finishing,
}

// WHAT:  A progress frame of a running export (sent over a Tauri channel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportProgress {
    pub stage: ExportStage,
    pub table: Option<String>,
    pub tables_done: u32,
    pub tables_total: u32,
    /// Rows written so far, over every table.
    pub rows: u64,
}

// WHAT:  The first rows of an export, formatted exactly as the file will be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportPreview {
    pub text: String,
    /// The preview stopped at its size cap; the file continues past it.
    pub truncated: bool,
    /// Rows shown, over every table.
    pub rows: u64,
    /// For a ZIP: the file names it will hold, in order. Empty otherwise.
    pub entries: Vec<String>,
}

// ============================================================================
// IMPORT
//
// WHAT:  The three ways data comes in: a CSV / JSON file into one table (with
//        column mapping), a multi-table JSON file as a checklist of operations,
//        and a .sql script run statement by statement.
// WHERE: src-tauri/src/services/transfer.rs, services/sql_import.rs,
//        src/features/transfer/TransferTab.tsx
// ============================================================================

// WHAT:  How a delimited file is read. `quote` is `"` or `'`; an empty
//        `null_text` makes an empty cell NULL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CsvImportOptions {
    pub delimiter: String,
    pub quote: String,
    /// The first row names the columns.
    pub header: bool,
    pub null_text: String,
}

impl Default for CsvImportOptions {
    fn default() -> Self {
        CsvImportOptions { delimiter: ",".to_string(), quote: "\"".to_string(), header: true, null_text: String::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ImportConflict {
    /// A row that collides with an existing one fails the import.
    Error,
    /// A colliding row is left as it is.
    Skip,
    /// A colliding row overwrites the existing one (an upsert on the primary key).
    Update,
}

// WHAT:  Which conflict modes this engine can express as one INSERT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportConflictSupport {
    pub skip: bool,
    pub update: bool,
}

// WHAT:  One target column and the file column that fills it (`source` is the
//        file column's position; None = Skip, the column gets its default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportMapping {
    pub target: String,
    pub source: Option<u32>,
}

// WHAT:  What the import dialog shows before anything is written.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportPreview {
    /// The file's columns: its header row, or "Column 1…" without one.
    pub headers: Vec<String>,
    /// The first 50 data rows as text; None is NULL.
    pub rows: Vec<Vec<Option<String>>>,
    /// Data rows in the whole file.
    pub row_count: u64,
    /// The target's columns (empty when no target was named).
    pub target_columns: Vec<crate::model::ColumnInfo>,
    /// One entry per target column, filled by name where the file has it.
    pub mapping: Vec<ImportMapping>,
    pub conflict: ImportConflictSupport,
}

// WHAT:  A progress frame of a running import (CSV / JSON into a table).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportProgress {
    pub rows_done: u64,
    pub rows_total: u64,
}

// WHAT:  What an import did. A file larger than one chunk is imported chunk by
//        chunk, each its own transaction, so a failure part-way leaves the
//        earlier chunks committed: `error` names the failure and `rows_done`
//        says how far it got.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportReport {
    /// Rows sent to the database and committed.
    pub rows_inserted: u64,
    /// What the server reported for them (an upsert or a skip makes this differ
    /// from `rows_inserted`); None when the engine reports no counts.
    pub rows_affected: Option<u64>,
    /// Rows in the file.
    pub rows_total: u64,
    pub statements: u64,
    pub elapsed_ms: u64,
    /// The import stopped: the message of the failure, or "Cancelled by user".
    pub error: Option<String>,
}

// WHAT:  What a JSON file holds: one table's rows (an array of objects) or
//        several tables keyed by name, with an optional `__schema` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum JsonFileShape {
    Rows,
    Tables,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ImportOperationKind {
    CreateTable,
    Insert,
}

// WHAT:  One step of a multi-table JSON import, shown as a checklist row.
// WHY:   A file of several tables can create tables the database lacks and fill
//        them; the user decides which steps run and where each lands.
// HOW:   `table` is the name in the file; the run request may override where it
//        lands. `exists` says the database already has a table of that name (a
//        CREATE for it is unticked by default). `sql` is the statement a
//        CREATE runs, or a summary line for an INSERT.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportOperation {
    pub id: String,
    pub kind: ImportOperationKind,
    pub table: String,
    pub columns: Vec<ImportColumn>,
    pub row_count: u64,
    pub exists: bool,
    pub sql: String,
}

// WHAT:  A JSON file read into its checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JsonImportPlan {
    pub shape: JsonFileShape,
    /// Empty for `Rows`.
    pub operations: Vec<ImportOperation>,
    /// Problems found while reading the file (entries that are neither rows nor schema).
    pub errors: Vec<String>,
    /// For `Rows`: the number of objects.
    pub row_count: u64,
}

// WHAT:  A ticked checklist row: which operation, and the table it lands in
//        (None = the name in the file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportOperationChoice {
    pub id: String,
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportOperationResult {
    pub id: String,
    pub ok: bool,
    pub message: String,
    pub rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JsonImportReport {
    pub results: Vec<ImportOperationResult>,
    pub elapsed_ms: u64,
}

// WHAT:  What the .sql import dialog shows before running a script.
// HOW:   Found by a streaming pass over the whole file with the same splitter
//        and classifier the run uses, so the counts are the run's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SqlFileInfo {
    pub size: u64,
    /// Last-modified time (milliseconds since the epoch). The run refuses a file
    /// whose size or time differs from what the user confirmed.
    pub modified_ms: u64,
    pub statements: u64,
    /// Statements the guard asks the user to confirm (DROP, TRUNCATE, DELETE without WHERE…).
    pub destructive: u64,
    /// The first few destructive statements, shortened.
    pub destructive_examples: Vec<String>,
    /// The first 2 KiB of the file.
    pub head: String,
    /// What the scan could not accept (a dump that changes the escaping rules, bad UTF-8…).
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SqlImportProgress {
    pub statements: u64,
    pub bytes_read: u64,
    pub bytes_total: u64,
}

// WHAT:  How far a .sql run got. Statements run one at a time with no wrapping
//        transaction, so everything before a failure stays committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SqlImportReport {
    pub statements: u64,
    pub elapsed_ms: u64,
    /// The failure that stopped the run, or "Cancelled by user".
    pub error: Option<String>,
    /// Line of the file the failing statement starts on.
    pub failed_line: Option<u64>,
    /// The failing statement, shortened.
    pub failed_statement: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiReply {
    pub sql: Option<String>,
    pub text: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlanReport {
    pub plan: String,
    pub explanation: Option<String>,
    /// The same plan as a tree, for engines whose adapter can read a
    /// structured EXPLAIN (Postgres, MySQL/MariaDB, SQLite). None elsewhere:
    /// the UI then shows only the text.
    #[serde(default)]
    pub plan_tree: Option<PlanNode>,
}

// WHAT:  One operator of an execution plan (scan, join, sort…) and its inputs.
// WHY:   A plan read as a tree, with a bar per node sized by cost, shows where
//        the time goes faster than the engine's indented text.
// HOW:   `cost` is the engine's own cumulative estimate for the subtree (its
//        units differ per engine, so only relative sizes mean anything);
//        `actual_ms` is filled only when the plan came from an EXPLAIN ANALYZE.
// WHERE: src-tauri/src/integrations/plan.rs (parsers), src/features/editor/PlanTree.tsx
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlanNode {
    pub label: String,
    pub detail: Option<String>,
    pub cost: Option<f64>,
    pub rows: Option<f64>,
    pub actual_ms: Option<f64>,
    pub children: Vec<PlanNode>,
}
