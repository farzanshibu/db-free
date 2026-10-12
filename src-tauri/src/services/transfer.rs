// SOT: transfer-service, import-file, import-preview, import-mapping, import-script, csv-io, json-io, json-import-plan, upsert-sql, json-cell

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::integrations::{qualified_name_for, quote_ident_for};
use crate::model::{
    ColumnInfo, CsvImportOptions, Engine, Family, ImportColumn, ImportConflict, ImportConflictSupport, ImportMapping, ImportOperation,
    ImportOperationKind, ImportPreview, JsonFileShape, JsonImportPlan, TableKind, TableRef, TransferFormat, Value,
};
use crate::services::changes;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde::Deserialize;
use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

// ============================================================================
// IMPORT
//
// WHAT:  Reads a CSV or JSON file into typed rows, maps its columns onto a
//        table's, and turns the rows into the INSERT scripts the guard runs.
//        For a JSON file of several tables it reads the file into a checklist
//        of operations (CREATE TABLE, INSERT) the user ticks.
// WHY:   PRD §7 Phase 3 import. The file is read a chunk at a time (a CSV is
//        never held whole in memory), and every statement is generated here so
//        the UI never assembles SQL.
// HOW:   `ImportSource::open` + `next_rows` stream the rows; `chunk_script`
//        renders one chunk with the dialect's conflict clause and, for the first
//        chunk, the truncate statement. The command layer hands each script to
//        the guard, which classifies it (a TRUNCATE needs the user's confirm).
// WHERE: src-tauri/src/commands/transfer.rs, src/features/transfer/TransferTab.tsx
// ============================================================================

/// Rows per script: each chunk is one transaction, so a failure part-way leaves
/// the earlier chunks committed and says how far it got.
pub const IMPORT_CHUNK_ROWS: usize = 5_000;
/// Rows per multi-row INSERT inside a chunk.
const INSERT_BATCH: usize = 200;
/// Rows the preview shows.
pub const PREVIEW_ROWS: usize = 50;
/// A JSON import holds the whole file in memory; past this it is refused.
const MAX_JSON_BYTES: u64 = 512 * 1024 * 1024;

pub fn json_cell(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::from(*i),
        Value::Float(f) => serde_json::Value::from(*f),
        Value::Decimal(s) | Value::Text(s) | Value::Bytes(s) | Value::DateTime(s) | Value::Unsupported(s) => serde_json::Value::String(s.clone()),
        Value::Json(j) => j.clone(),
    }
}

fn value_from_json(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => n.as_i64().map(Value::Int).or_else(|| n.as_f64().map(Value::Float)).unwrap_or(Value::Null),
        serde_json::Value::String(s) => Value::Text(s.clone()),
        other => Value::Json(other.clone()),
    }
}

fn cell_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        other => Some(crate::services::export::cell_text(other)),
    }
}

// ---------------------------------------------------------------------------
// Reading files
// ---------------------------------------------------------------------------

// WHAT:  `,` / `;` / `|` / tab and `"` / `'` as the single bytes the csv reader wants.
fn one_byte(text: &str, what: &str) -> AppResult<u8> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii() => Ok(c as u8),
        _ => Err(AppError::invalid_input(format!("The {what} must be a single ASCII character."))),
    }
}

// WHAT:  A CSV reader over `path` with the user's delimiter and quote.
// HOW:   A UTF-8 byte-order mark is consumed first, or it would end up inside
//        the first column's name.
fn csv_reader(path: &Path, options: &CsvImportOptions) -> AppResult<csv::Reader<BufReader<File>>> {
    let delimiter = one_byte(&options.delimiter, "delimiter")?;
    let quote = one_byte(&options.quote, "quote character")?;
    let file = File::open(path).map_err(AppError::invalid_input_from)?;
    let mut input = BufReader::new(file);
    let has_bom = input.fill_buf().map_err(AppError::invalid_input_from)?.starts_with(b"\xEF\xBB\xBF");
    if has_bom {
        input.consume(3);
    }
    Ok(csv::ReaderBuilder::new().has_headers(false).flexible(true).delimiter(delimiter).quote(quote).from_reader(input))
}

/// A CSV record as cells; a cell equal to the NULL text is None.
fn record_cells(record: &csv::ByteRecord, null_text: &str) -> Vec<Option<String>> {
    record
        .iter()
        .map(|field| {
            let text = String::from_utf8_lossy(field);
            if text == null_text { None } else { Some(text.into_owned()) }
        })
        .collect()
}

/// Names for a file without a header row.
fn synthetic_headers(width: usize) -> Vec<String> {
    (1..=width).map(|i| format!("Column {i}")).collect()
}

// WHAT:  A JSON object that remembers the order its keys were written in.
// WHY:   serde_json's own map sorts keys, which would shuffle the columns of a
//        file the user wrote (or an export wrote) in a deliberate order.
type JsonRow = Vec<(String, serde_json::Value)>;

struct OrderedRow(JsonRow);

impl<'de> Deserialize<'de> for OrderedRow {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RowVisitor;
        impl<'de> Visitor<'de> for RowVisitor {
            type Value = OrderedRow;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<OrderedRow, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, serde_json::Value>()? {
                    entries.push(entry);
                }
                Ok(OrderedRow(entries))
            }
        }
        deserializer.deserialize_map(RowVisitor)
    }
}

/// One member of a multi-table file: an array of row objects, or something else.
#[derive(Deserialize)]
#[serde(untagged)]
enum Member {
    Rows(Vec<OrderedRow>),
    Other(serde_json::Value),
}

/// The top-level object of a multi-table file, members in file order.
struct OrderedMembers(Vec<(String, Member)>);

impl<'de> Deserialize<'de> for OrderedMembers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MembersVisitor;
        impl<'de> Visitor<'de> for MembersVisitor {
            type Value = OrderedMembers;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<OrderedMembers, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, Member>()? {
                    entries.push(entry);
                }
                Ok(OrderedMembers(entries))
            }
        }
        deserializer.deserialize_map(MembersVisitor)
    }
}

/// The two shapes a JSON import file can take.
enum JsonFile {
    Rows(Vec<OrderedRow>),
    Tables(OrderedMembers),
}

impl<'de> Deserialize<'de> for JsonFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FileVisitor;
        impl<'de> Visitor<'de> for FileVisitor {
            type Value = JsonFile;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an array of objects, or an object of tables")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<JsonFile, A::Error> {
                let mut rows = Vec::new();
                while let Some(row) = seq.next_element::<OrderedRow>()? {
                    rows.push(row);
                }
                Ok(JsonFile::Rows(rows))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JsonFile, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry::<String, Member>()? {
                    entries.push(entry);
                }
                Ok(JsonFile::Tables(OrderedMembers(entries)))
            }
        }
        deserializer.deserialize_any(FileVisitor)
    }
}

fn read_json_file(path: &Path) -> AppResult<JsonFile> {
    let size = std::fs::metadata(path).map_err(AppError::invalid_input_from)?.len();
    if size > MAX_JSON_BYTES {
        return Err(AppError::invalid_input("This JSON file is over 512 MiB. Split it, or import it as CSV."));
    }
    let file = File::open(path).map_err(AppError::invalid_input_from)?;
    let mut input = BufReader::new(file);
    if input.fill_buf().map_err(AppError::invalid_input_from)?.starts_with(b"\xEF\xBB\xBF") {
        input.consume(3);
    }
    serde_json::from_reader(input).map_err(|e| AppError::invalid_input(format!("This is not a JSON import file: {e}")))
}

// WHAT:  Columns of an array of row objects, in order of first appearance, and
//        the rows laid out under them.
fn lay_out(rows: &[OrderedRow]) -> (Vec<String>, Vec<Vec<Value>>) {
    let mut columns: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for row in rows {
        for (key, _) in &row.0 {
            if seen.insert(key.as_str()) {
                columns.push(key.clone());
            }
        }
    }
    let laid = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| row.0.iter().find(|(key, _)| key == column).map(|(_, v)| value_from_json(v)).unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    (columns, laid)
}

/// Where an import's rows come from, one chunk at a time.
pub struct ImportSource {
    /// The file's own columns: its header, or "Column 1…".
    pub headers: Vec<String>,
    inner: Inner,
}

enum Inner {
    Csv {
        reader: csv::Reader<BufReader<File>>,
        record: csv::ByteRecord,
        null_text: String,
        /// A first row read to learn the width of a headerless file, not yet handed out.
        pending: Option<Vec<Option<String>>>,
    },
    Rows(std::vec::IntoIter<Vec<Value>>),
}

impl ImportSource {
    // WHAT:  Opens `path` and reads its columns; no data row is consumed.
    pub fn open(path: &Path, format: TransferFormat, csv: &CsvImportOptions) -> AppResult<ImportSource> {
        match format {
            TransferFormat::Csv => {
                let mut reader = csv_reader(path, csv)?;
                let mut record = csv::ByteRecord::new();
                let first = reader.read_byte_record(&mut record).map_err(AppError::invalid_input_from)?;
                if !first {
                    return Err(AppError::invalid_input("The file is empty."));
                }
                let cells = record_cells(&record, "");
                if csv.header {
                    let headers = cells.into_iter().map(Option::unwrap_or_default).collect();
                    Ok(ImportSource { headers, inner: Inner::Csv { reader, record, null_text: csv.null_text.clone(), pending: None } })
                } else {
                    let headers = synthetic_headers(cells.len());
                    let pending = record_cells(&record, &csv.null_text);
                    Ok(ImportSource { headers, inner: Inner::Csv { reader, record, null_text: csv.null_text.clone(), pending: Some(pending) } })
                }
            }
            TransferFormat::Json => match read_json_file(path)? {
                JsonFile::Rows(rows) => {
                    let (headers, laid) = lay_out(&rows);
                    Ok(ImportSource { headers, inner: Inner::Rows(laid.into_iter()) })
                }
                JsonFile::Tables(_) => Err(AppError::invalid_input("This JSON file holds several tables. Use the multi-table import for it.")),
            },
            TransferFormat::Sql => Err(AppError::invalid_input("Run .sql files with the SQL import.")),
        }
    }

    // WHAT:  Up to `max` rows as typed values (a CSV cell is text, an empty or
    //        NULL-text cell is NULL). An empty result is the end of the file.
    pub fn next_rows(&mut self, max: usize) -> AppResult<Vec<Vec<Value>>> {
        let mut out: Vec<Vec<Value>> = Vec::new();
        let width = self.headers.len();
        match &mut self.inner {
            Inner::Rows(rows) => out.extend(rows.take(max)),
            Inner::Csv { reader, record, null_text, pending } => {
                if let Some(first) = pending.take() {
                    out.push(first.into_iter().map(|c| c.map_or(Value::Null, Value::Text)).collect());
                }
                while out.len() < max {
                    if !reader.read_byte_record(record).map_err(AppError::invalid_input_from)? {
                        break;
                    }
                    let cells = record_cells(record, null_text);
                    out.push((0..width).map(|i| cells.get(i).cloned().flatten().map_or(Value::Null, Value::Text)).collect());
                }
            }
        }
        Ok(out)
    }
}

/// A file as the import dialog first shows it.
pub struct FilePreview {
    pub headers: Vec<String>,
    /// The first rows as text; None is NULL.
    pub rows: Vec<Vec<Option<String>>>,
    /// Data rows in the whole file.
    pub total: u64,
}

// WHAT:  What the import dialog shows: the file's columns, its first 50 rows as
//        text, and how many rows the whole file holds.
pub fn preview_rows(path: &Path, format: TransferFormat, csv: &CsvImportOptions) -> AppResult<FilePreview> {
    let mut source = ImportSource::open(path, format, csv)?;
    let headers = source.headers.clone();
    let mut shown: Vec<Vec<Option<String>>> = Vec::new();
    let mut total: u64 = 0;
    loop {
        let batch = source.next_rows(1_000)?;
        if batch.is_empty() {
            break;
        }
        for row in batch {
            total += 1;
            if shown.len() < PREVIEW_ROWS {
                shown.push(row.iter().map(cell_text).collect());
            }
        }
    }
    Ok(FilePreview { headers, rows: shown, total })
}

// WHAT:  One entry per target column, filled by the file column of the same
//        name (case-insensitive). A file with no header row fills by position.
pub fn suggest_mapping(headers: &[String], target: &[ColumnInfo], positional: bool) -> Vec<ImportMapping> {
    target
        .iter()
        .enumerate()
        .map(|(i, column)| {
            let by_name = headers.iter().position(|h| h.trim().eq_ignore_ascii_case(&column.name));
            let source = if positional { (i < headers.len()).then_some(i) } else { by_name };
            ImportMapping { target: column.name.clone(), source: source.and_then(|s| u32::try_from(s).ok()) }
        })
        .collect()
}

pub fn make_preview(
    headers: Vec<String>,
    rows: Vec<Vec<Option<String>>>,
    row_count: u64,
    target_columns: Vec<ColumnInfo>,
    positional: bool,
    conflict: ImportConflictSupport,
) -> ImportPreview {
    let mapping = suggest_mapping(&headers, &target_columns, positional);
    ImportPreview { headers, rows, row_count, target_columns, mapping, conflict }
}

// WHAT:  The mapped columns (target names, in order) and, for each, which file
//        column fills it. Skipped targets are left out so their default applies.
pub fn mapped_columns(mapping: &[ImportMapping], target: &[ColumnInfo], file_width: usize) -> AppResult<(Vec<String>, Vec<usize>)> {
    let mut names = Vec::new();
    let mut picks = Vec::new();
    for entry in mapping {
        let Some(source) = entry.source else {
            continue;
        };
        let source = source as usize;
        if source >= file_width {
            return Err(AppError::invalid_input(format!("Column \"{}\" is mapped to a file column that does not exist.", entry.target)));
        }
        let column = target
            .iter()
            .find(|c| c.name == entry.target)
            .ok_or_else(|| AppError::invalid_input(format!("Column \"{}\" does not exist on the table.", entry.target)))?;
        if names.contains(&column.name) {
            return Err(AppError::invalid_input(format!("Column \"{}\" is mapped twice.", entry.target)));
        }
        names.push(column.name.clone());
        picks.push(source);
    }
    if names.is_empty() {
        return Err(AppError::invalid_input("Map at least one column, or there is nothing to import."));
    }
    Ok((names, picks))
}

pub fn pick_columns(rows: Vec<Vec<Value>>, picks: &[usize]) -> Vec<Vec<Value>> {
    rows.into_iter().map(|row| picks.iter().map(|&i| row.get(i).cloned().unwrap_or(Value::Null)).collect()).collect()
}

// ---------------------------------------------------------------------------
// Scripts
// ---------------------------------------------------------------------------

// WHAT:  Which conflict modes one INSERT can express on this engine.
pub fn conflict_support(engine: Engine) -> ImportConflictSupport {
    match engine.family() {
        Family::Postgres | Family::Duckdb | Family::Sqlite | Family::Libsql | Family::CloudflareD1 | Family::ValTown | Family::Mysql => {
            ImportConflictSupport { skip: true, update: true }
        }
        _ => ImportConflictSupport { skip: false, update: false },
    }
}

/// SQLite and friends have no TRUNCATE.
fn truncate_statement(engine: Engine, table: &TableRef) -> String {
    let name = qualified_name_for(engine, table);
    match engine.family() {
        // MySQL's TRUNCATE commits on its own and cannot be rolled back with the
        // import that follows; DELETE can.
        Family::Sqlite | Family::Libsql | Family::CloudflareD1 | Family::ValTown | Family::Mysql => format!("DELETE FROM {name}"),
        _ => format!("TRUNCATE TABLE {name}"),
    }
}

// WHAT:  The text appended to an INSERT for a conflict mode (and the prefix it
//        needs, for MySQL's INSERT IGNORE).
fn conflict_clause(engine: Engine, conflict: ImportConflict, columns: &[String], keys: &[String]) -> AppResult<(&'static str, String)> {
    if conflict == ImportConflict::Error {
        return Ok(("INSERT INTO", String::new()));
    }
    let support = conflict_support(engine);
    let supported = match conflict {
        ImportConflict::Skip => support.skip,
        _ => support.update,
    };
    if !supported {
        return Err(AppError::invalid_input("This engine cannot skip or update rows that already exist; import with \"fail\" instead."));
    }
    let ident = |name: &str| quote_ident_for(engine, name);
    match (engine.family(), conflict) {
        (Family::Mysql, ImportConflict::Skip) => Ok(("INSERT IGNORE INTO", String::new())),
        (Family::Mysql, _) => {
            if keys.is_empty() {
                return Err(AppError::invalid_input("Updating existing rows needs a primary key, and this table has none."));
            }
            let updates: Vec<String> = columns.iter().filter(|c| !keys.contains(c)).map(|c| format!("{0} = VALUES({0})", ident(c))).collect();
            // Every mapped column is part of the key: nothing to change, so the update is a no-op.
            let updates = if updates.is_empty() { vec![format!("{0} = {0}", ident(&keys[0]))] } else { updates };
            Ok(("INSERT INTO", format!("\nON DUPLICATE KEY UPDATE {}", updates.join(", "))))
        }
        (_, ImportConflict::Skip) => Ok(("INSERT INTO", "\nON CONFLICT DO NOTHING".to_string())),
        (_, _) => {
            if keys.is_empty() {
                return Err(AppError::invalid_input("Updating existing rows needs a primary key, and this table has none."));
            }
            let target = keys.iter().map(|k| ident(k)).collect::<Vec<_>>().join(", ");
            let updates: Vec<String> = columns.iter().filter(|c| !keys.contains(c)).map(|c| format!("{0} = EXCLUDED.{0}", ident(c))).collect();
            if updates.is_empty() {
                Ok(("INSERT INTO", format!("\nON CONFLICT ({target}) DO NOTHING")))
            } else {
                Ok(("INSERT INTO", format!("\nON CONFLICT ({target}) DO UPDATE SET {}", updates.join(", "))))
            }
        }
    }
}

/// How a chunk is wrapped so it commits or rolls back whole.
fn transaction_wrap(engine: Engine, transactional: bool) -> Option<(&'static str, &'static str)> {
    if !transactional {
        return None;
    }
    match engine.family() {
        Family::Mssql => Some(("BEGIN TRANSACTION;\n", "COMMIT TRANSACTION;")),
        // `BEGIN;` opens a PL/SQL block on Oracle.
        Family::Oracle => None,
        _ => Some(("BEGIN;\n", "COMMIT;")),
    }
}

/// What a chunk script is built from.
pub struct ChunkPlan<'a> {
    pub engine: Engine,
    pub table: &'a TableRef,
    pub columns: &'a [String],
    /// Primary-key columns of the table (for `ImportConflict::Update`).
    pub keys: &'a [String],
    pub conflict: ImportConflict,
    pub transactional: bool,
}

/// A chunk ready for the guard.
pub struct ChunkScript {
    /// What runs.
    pub script: String,
    /// What the guard classifies and the history records instead of the script.
    pub summary: String,
}

// WHAT:  One chunk of rows as a script, plus the short text the guard sees.
// WHY:   A 5,000-row script is hundreds of kilobytes; the history log would keep
//        every one. The summary has the same statement kinds as the script —
//        its INSERT (and TRUNCATE when `truncate` is set) — so the read-only
//        lock and the destructive confirmation judge exactly what will run.
pub fn chunk_script(plan: &ChunkPlan<'_>, rows: &[Vec<Value>], truncate: bool, label: &str) -> AppResult<ChunkScript> {
    let (prefix, suffix) = conflict_clause(plan.engine, plan.conflict, plan.columns, plan.keys)?;
    let wrap = transaction_wrap(plan.engine, plan.transactional);
    let mut script = String::new();
    // The label is a file name: keep it to one comment line.
    let label = label.replace(['\n', '\r'], " ");
    let mut summary = format!("-- Import {label}: {} row(s) into {}\n", rows.len(), qualified_name_for(plan.engine, plan.table));
    if let Some((begin, _)) = wrap {
        script.push_str(begin);
    }
    if truncate {
        let statement = truncate_statement(plan.engine, plan.table);
        script.push_str(&statement);
        script.push_str(";\n");
        summary.push_str(&statement);
        summary.push_str(";\n");
    }
    for batch in changes::insert_batches(plan.engine, plan.table, plan.columns, rows, INSERT_BATCH) {
        let statement = match batch.strip_prefix("INSERT INTO") {
            Some(rest) => format!("{prefix}{rest}{suffix}"),
            None => format!("{batch}{suffix}"),
        };
        script.push_str(&statement);
        script.push_str(";\n");
    }
    let columns = plan.columns.iter().map(|c| quote_ident_for(plan.engine, c)).collect::<Vec<_>>().join(", ");
    summary.push_str(&format!("{prefix} {} ({columns}) VALUES (…){suffix}", qualified_name_for(plan.engine, plan.table)));
    if let Some((_, commit)) = wrap {
        script.push_str(commit);
    }
    Ok(ChunkScript { script, summary })
}

/// The table's primary-key columns, in order.
pub fn key_columns(columns: &[ColumnInfo]) -> Vec<String> {
    columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect()
}

// ---------------------------------------------------------------------------
// What the session tells an import
// ---------------------------------------------------------------------------

/// The table an import writes into, as the session reports it.
pub struct Target {
    pub engine: Engine,
    pub columns: Vec<ColumnInfo>,
    pub keys: Vec<String>,
    pub transactional: bool,
}

// WHAT:  The target's columns, keys and transaction support.
// WHY:   Import builds INSERT statements, so it needs a SQL engine; a document
//        or key-value store is refused here with a plain message instead of
//        failing on the first statement.
pub async fn load_target(ctx: &SessionCtx, table: &TableRef) -> AppResult<Target> {
    let capabilities = ctx.integration.capabilities();
    if !capabilities.sql {
        return Err(AppError::invalid_input("Import writes INSERT statements, so it needs a SQL engine."));
    }
    let columns = ctx.integration.columns(table).await?;
    if columns.is_empty() {
        return Err(AppError::not_found(format!("Table \"{}\" has no columns or does not exist.", table.name)));
    }
    Ok(Target { engine: ctx.connection.engine, keys: key_columns(&columns), columns, transactional: capabilities.transactions })
}

// WHAT:  The tables the database has (views excluded) and whether it has namespaces.
pub async fn known_tables(ctx: &SessionCtx) -> AppResult<(Vec<TableRef>, bool)> {
    let catalog = ctx.integration.catalog().await?;
    let tables = catalog
        .schemas
        .into_iter()
        .flat_map(|schema| schema.tables)
        .filter(|t| t.kind == TableKind::Table)
        .map(|t| TableRef { schema: t.schema, name: t.name })
        .collect();
    Ok((tables, ctx.integration.capabilities().namespaces))
}

// WHAT:  The import dialog's first look at a file (see `ImportPreview`).
pub async fn preview(ctx: &SessionCtx, table: Option<&TableRef>, path: &Path, format: TransferFormat, csv: &CsvImportOptions) -> AppResult<ImportPreview> {
    let (owned_path, owned_csv) = (path.to_path_buf(), csv.clone());
    let FilePreview { headers, rows, total } = tokio::task::spawn_blocking(move || preview_rows(&owned_path, format, &owned_csv)).await.map_err(AppError::internal)??;
    let target = match table {
        Some(table) => ctx.integration.columns(table).await?,
        None => Vec::new(),
    };
    let positional = format == TransferFormat::Csv && !csv.header;
    Ok(make_preview(headers, rows, total, target, positional, conflict_support(ctx.connection.engine)))
}

// WHAT:  A JSON file read into its checklist (or, for a bare array, its row count).
pub async fn plan_json(ctx: &SessionCtx, path: &Path) -> AppResult<JsonImportPlan> {
    let owned = path.to_path_buf();
    let shape = json_shape(&owned)?;
    if shape == JsonFileShape::Rows {
        let counted = tokio::task::spawn_blocking(move || preview_rows(&owned, TransferFormat::Json, &CsvImportOptions::default())).await.map_err(AppError::internal)??;
        return Ok(make_plan(shape, Vec::new(), Vec::new(), counted.total));
    }
    let parsed = tokio::task::spawn_blocking(move || read_tables(&owned)).await.map_err(AppError::internal)??;
    let (known, namespaces) = known_tables(ctx).await?;
    let operations = plan_operations(ctx.connection.engine, namespaces, &parsed, &known);
    Ok(make_plan(shape, operations, parsed.errors, 0))
}

// ---------------------------------------------------------------------------
// Multi-table JSON
// ---------------------------------------------------------------------------

/// A multi-table JSON file, read.
pub struct ParsedTables {
    /// Table name → its column names and typed rows, in file order.
    pub tables: Vec<ParsedTable>,
    /// The `__schema` block: table name → columns.
    pub schema: Vec<(String, Vec<ImportColumn>)>,
    pub errors: Vec<String>,
}

pub struct ParsedTable {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// Types guessed from the first non-null value of each column.
    pub inferred: Vec<ImportColumn>,
}

// WHAT:  Reads `{ "__schema": {…}, "users": [rows], … }` (or a bare array, as one
//        unnamed table) in file order.
pub fn read_tables(path: &Path) -> AppResult<ParsedTables> {
    let members = match read_json_file(path)? {
        JsonFile::Tables(members) => members.0,
        JsonFile::Rows(rows) => vec![("rows".to_string(), Member::Rows(rows))],
    };
    let mut parsed = ParsedTables { tables: Vec::new(), schema: Vec::new(), errors: Vec::new() };
    for (name, member) in members {
        match member {
            Member::Other(value) if name == "__schema" => parsed.schema = schema_block(&value, &mut parsed.errors),
            Member::Rows(rows) if name != "__schema" => {
                let (columns, laid) = lay_out(&rows);
                let inferred = columns
                    .iter()
                    .enumerate()
                    .map(|(i, column)| ImportColumn {
                        name: column.clone(),
                        data_type: infer_kind(laid.iter().filter_map(|r| r.get(i)).find(|v| !matches!(v, Value::Null))).to_string(),
                    })
                    .collect();
                parsed.tables.push(ParsedTable { name, columns, rows: laid, inferred });
            }
            _ => parsed.errors.push(format!("\"{name}\" is neither an array of rows nor the __schema block, so it is skipped.")),
        }
    }
    if parsed.tables.is_empty() {
        parsed.errors.push("The file holds no table rows.".to_string());
    }
    Ok(parsed)
}

fn schema_block(value: &serde_json::Value, errors: &mut Vec<String>) -> Vec<(String, Vec<ImportColumn>)> {
    let Some(object) = value.as_object() else {
        errors.push("__schema is not an object, so it is ignored.".to_string());
        return Vec::new();
    };
    object
        .iter()
        .map(|(table, columns)| {
            let described = columns
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let name = item.get("name")?.as_str()?.to_string();
                            let data_type = item.get("type").and_then(serde_json::Value::as_str).unwrap_or("").to_string();
                            Some(ImportColumn { name, data_type })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (table.clone(), described)
        })
        .collect()
}

/// A coarse type class of a value, for naming a column's type.
fn infer_kind(value: Option<&Value>) -> &'static str {
    match value {
        Some(Value::Int(_)) => "int",
        Some(Value::Float(_)) => "float",
        Some(Value::Bool(_)) => "bool",
        Some(Value::Json(_)) => "json",
        _ => "text",
    }
}

// WHAT:  The engine's spelling of an inferred type class.
fn inferred_type(engine: Engine, class: &str) -> &'static str {
    match (engine.family(), class) {
        (Family::Postgres, "int") | (Family::Duckdb, "int") | (Family::Mysql, "int") => "BIGINT",
        (Family::Postgres, "float") | (Family::Duckdb, "float") => "DOUBLE PRECISION",
        (Family::Postgres, "json") => "JSONB",
        (Family::Mysql, "float") => "DOUBLE",
        (Family::Mysql, "json") => "JSON",
        (Family::Sqlite | Family::Libsql | Family::CloudflareD1 | Family::ValTown, "int") => "INTEGER",
        (Family::Sqlite | Family::Libsql | Family::CloudflareD1 | Family::ValTown, "float") => "REAL",
        (Family::Sqlite | Family::Libsql | Family::CloudflareD1 | Family::ValTown, "bool") => "BOOLEAN",
        (Family::Mssql, "int") => "bigint",
        (Family::Mssql, "float") => "float",
        (Family::Mssql, "bool") => "bit",
        (Family::Mssql, _) => "nvarchar(max)",
        (Family::Oracle, "int") => "NUMBER(19)",
        (Family::Oracle, "float") => "BINARY_DOUBLE",
        (Family::Oracle, "bool") => "NUMBER(1)",
        (Family::Oracle, _) => "CLOB",
        (Family::Clickhouse, "int") => "Int64",
        (Family::Clickhouse, "float") => "Float64",
        (Family::Clickhouse, "bool") => "Bool",
        (Family::Clickhouse, _) => "String",
        (_, "int") => "BIGINT",
        (_, "float") => "DOUBLE",
        (_, "bool") => "BOOLEAN",
        _ => "TEXT",
    }
}

/// MySQL wants a length on VARCHAR; a type copied from another engine often has none.
fn declared_type(engine: Engine, declared: &str, class: &str) -> String {
    let declared = declared.trim();
    if declared.is_empty() || matches!(declared, "int" | "float" | "bool" | "json" | "text") && declared == class {
        return inferred_type(engine, class).to_string();
    }
    if engine.family() == Family::Mysql && matches!(declared.to_ascii_uppercase().as_str(), "VARCHAR" | "CHAR" | "VARBINARY" | "BINARY") {
        return format!("{declared}(255)");
    }
    declared.to_string()
}

// WHAT:  CREATE TABLE for a table the file describes (its `__schema` columns, else
//        columns typed from the data).
pub fn create_table_sql(engine: Engine, target: &TableRef, columns: &[ImportColumn], inferred: &[ImportColumn]) -> AppResult<String> {
    let source = if columns.is_empty() { inferred } else { columns };
    if source.is_empty() {
        return Err(AppError::invalid_input(format!("\"{}\" has no columns to create.", target.name)));
    }
    let body = source
        .iter()
        .map(|c| {
            let class = inferred.iter().find(|i| i.name == c.name).map_or("text", |i| i.data_type.as_str());
            format!("  {} {}", quote_ident_for(engine, &c.name), declared_type(engine, &c.data_type, class))
        })
        .collect::<Vec<_>>()
        .join(",\n");
    Ok(format!("CREATE TABLE {} (\n{body}\n)", qualified_name_for(engine, target)))
}

// WHAT:  Where a table named in the file lands: a table the database already has
//        under that name, else `schema.table` split at the first dot when the
//        engine has namespaces, else the bare name.
pub fn resolve_target(engine_namespaces: bool, name: &str, known: &[TableRef]) -> TableRef {
    let label = |t: &TableRef| match &t.schema {
        Some(schema) if !schema.is_empty() => format!("{schema}.{}", t.name),
        _ => t.name.clone(),
    };
    if let Some(found) = known.iter().find(|t| label(t) == name).or_else(|| known.iter().find(|t| t.name == name)) {
        return found.clone();
    }
    match name.split_once('.') {
        Some((schema, table)) if engine_namespaces && !schema.is_empty() && !table.is_empty() => TableRef { schema: Some(schema.to_string()), name: table.to_string() },
        _ => TableRef { schema: None, name: name.to_string() },
    }
}

// WHAT:  The checklist for a multi-table file: per table, a CREATE (unticked by
//        default when the table exists) and an INSERT.
pub fn plan_operations(engine: Engine, namespaces: bool, parsed: &ParsedTables, known: &[TableRef]) -> Vec<ImportOperation> {
    let mut operations = Vec::new();
    for table in &parsed.tables {
        let target = resolve_target(namespaces, &table.name, known);
        let exists = known.iter().any(|t| t.name == target.name && t.schema == target.schema);
        let described: Vec<ImportColumn> = parsed.schema.iter().find(|(name, _)| name == &table.name).map(|(_, columns)| columns.clone()).unwrap_or_default();
        let columns = if described.is_empty() { table.inferred.clone() } else { described.clone() };
        if !described.is_empty() || !exists {
            let sql = create_table_sql(engine, &target, &described, &table.inferred).unwrap_or_else(|e| format!("-- {}", e.message()));
            operations.push(ImportOperation {
                id: format!("create:{}", table.name),
                kind: ImportOperationKind::CreateTable,
                table: table.name.clone(),
                columns: columns.clone(),
                row_count: 0,
                exists,
                sql,
            });
        }
        operations.push(ImportOperation {
            id: format!("insert:{}", table.name),
            kind: ImportOperationKind::Insert,
            table: table.name.clone(),
            columns,
            row_count: table.rows.len() as u64,
            exists,
            sql: format!("-- {} row(s) into {}", table.rows.len(), qualified_name_for(engine, &target)),
        });
    }
    operations
}

pub fn make_plan(shape: JsonFileShape, operations: Vec<ImportOperation>, errors: Vec<String>, row_count: u64) -> JsonImportPlan {
    JsonImportPlan { shape, operations, errors, row_count }
}

// WHAT:  Whether a JSON file is one table's rows or several tables.
pub fn json_shape(path: &Path) -> AppResult<JsonFileShape> {
    let file = File::open(path).map_err(AppError::invalid_input_from)?;
    let mut input = BufReader::new(file);
    let head = input.fill_buf().map_err(AppError::invalid_input_from)?;
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    match head.iter().find(|b| !b.is_ascii_whitespace()) {
        Some(b'[') => Ok(JsonFileShape::Rows),
        Some(b'{') => Ok(JsonFileShape::Tables),
        _ => Err(AppError::invalid_input("This is not a JSON import file: it must start with [ or {.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("db-free-transfer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir
    }

    fn write(dir: &Path, name: &str, text: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("{e}"));
        path
    }

    fn column(name: &str, pk: bool) -> ColumnInfo {
        ColumnInfo { name: name.into(), data_type: "text".into(), nullable: true, primary_key: pk, ordinal: 0 }
    }

    fn users() -> TableRef {
        TableRef { schema: Some("public".into()), name: "users".into() }
    }

    #[test]
    fn csv_options_drive_the_reader() {
        let dir = dir();
        let path = write(&dir, "a.csv", b"\xEF\xBB\xBFid;name\n1;'ann;b'\n2;NULL\n3;\n");
        let options = CsvImportOptions { delimiter: ";".into(), quote: "'".into(), header: true, null_text: "NULL".into() };
        let mut source = ImportSource::open(&path, TransferFormat::Csv, &options).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(source.headers, vec!["id", "name"], "the byte-order mark is not part of the first name");
        let rows = source.next_rows(10).unwrap_or_default();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec![Value::Text("1".into()), Value::Text("ann;b".into())]);
        assert_eq!(rows[1][1], Value::Null, "the NULL text is NULL");
        assert_eq!(rows[2][1], Value::Text(String::new()), "an empty cell is empty text when the NULL text is not empty");
        assert!(source.next_rows(10).unwrap_or_default().is_empty());
        let looked = preview_rows(&path, TransferFormat::Csv, &options).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((looked.headers.len(), looked.rows.len(), looked.total), (2, 3, 3));
        assert_eq!(looked.rows[1][1], None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_headerless_csv_keeps_its_first_row_and_fills_by_position() {
        let dir = dir();
        let path = write(&dir, "b.csv", b"1,ann\n2,bob\n");
        let options = CsvImportOptions { header: false, ..CsvImportOptions::default() };
        let mut source = ImportSource::open(&path, TransferFormat::Csv, &options).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(source.headers, vec!["Column 1", "Column 2"]);
        let rows = source.next_rows(10).unwrap_or_default();
        assert_eq!(rows.len(), 2, "the first line is data");
        let target = vec![column("id", true), column("name", false), column("extra", false)];
        let mapping = suggest_mapping(&source.headers, &target, true);
        assert_eq!(mapping.iter().map(|m| m.source).collect::<Vec<_>>(), vec![Some(0), Some(1), None]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mapping_is_by_name_and_validated() {
        let headers = vec!["Name".to_string(), "ID".to_string(), "junk".to_string()];
        let target = vec![column("id", true), column("name", false), column("age", false)];
        let mapping = suggest_mapping(&headers, &target, false);
        assert_eq!(mapping.iter().map(|m| m.source).collect::<Vec<_>>(), vec![Some(1), Some(0), None]);
        let (names, picks) = mapped_columns(&mapping, &target, headers.len()).unwrap_or_default();
        assert_eq!((names, picks), (vec!["id".to_string(), "name".to_string()], vec![1, 0]));
        let rows = vec![vec![Value::Text("ann".into()), Value::Text("7".into()), Value::Null]];
        assert_eq!(pick_columns(rows, &[1, 0]), vec![vec![Value::Text("7".into()), Value::Text("ann".into())]]);
        let none = vec![ImportMapping { target: "id".into(), source: None }];
        assert!(mapped_columns(&none, &target, 3).is_err(), "nothing mapped");
        let bad = vec![ImportMapping { target: "nope".into(), source: Some(0) }];
        assert!(mapped_columns(&bad, &target, 3).is_err());
        let far = vec![ImportMapping { target: "id".into(), source: Some(9) }];
        assert!(mapped_columns(&far, &target, 3).is_err());
        let twice = vec![ImportMapping { target: "id".into(), source: Some(0) }, ImportMapping { target: "id".into(), source: Some(1) }];
        assert!(mapped_columns(&twice, &target, 3).is_err());
    }

    #[test]
    fn json_rows_keep_column_order_and_types() {
        let dir = dir();
        let path = write(&dir, "a.json", br#"[{"name":"ann","id":1,"tags":["x"]},{"id":2,"extra":true}]"#);
        let mut source = ImportSource::open(&path, TransferFormat::Json, &CsvImportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(source.headers, vec!["name", "id", "tags", "extra"], "first appearance, not alphabetical");
        let rows = source.next_rows(10).unwrap_or_default();
        assert_eq!(rows[0][1], Value::Int(1));
        assert!(matches!(rows[0][2], Value::Json(_)));
        assert_eq!(rows[1][0], Value::Null);
        assert_eq!(rows[1][3], Value::Bool(true));
        let tables = write(&dir, "t.json", br#"{"a":[{"id":1}]}"#);
        assert!(ImportSource::open(&tables, TransferFormat::Json, &CsvImportOptions::default()).is_err(), "several tables need the multi-table import");
        assert_eq!(json_shape(&path).unwrap_or(JsonFileShape::Tables), JsonFileShape::Rows);
        assert_eq!(json_shape(&tables).unwrap_or(JsonFileShape::Rows), JsonFileShape::Tables);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scripts_carry_the_conflict_clause_of_each_dialect() {
        let rows = vec![vec![Value::Int(1), Value::Text("a".into())], vec![Value::Int(2), Value::Text("b".into())]];
        let columns = vec!["id".to_string(), "name".to_string()];
        let keys = vec!["id".to_string()];
        let table = users();
        let plan = |engine, conflict| ChunkPlan { engine, table: &table, columns: &columns, keys: &keys, conflict, transactional: true };

        let plain = chunk_script(&plan(Engine::Postgres, ImportConflict::Error), &rows, false, "a.csv").unwrap_or_else(|e| panic!("{e}"));
        assert!(plain.script.starts_with("BEGIN;\nINSERT INTO \"public\".\"users\" (\"id\", \"name\") VALUES\n(1, 'a'),\n(2, 'b');\nCOMMIT;"), "{}", plain.script);
        assert!(plain.summary.contains("-- Import a.csv: 2 row(s) into"), "{}", plain.summary);
        assert!(!plain.script.contains("TRUNCATE"));

        let skip = chunk_script(&plan(Engine::Postgres, ImportConflict::Skip), &rows, false, "a.csv").unwrap_or_else(|e| panic!("{e}"));
        assert!(skip.script.contains("(2, 'b')\nON CONFLICT DO NOTHING;"), "{}", skip.script);

        let update = chunk_script(&plan(Engine::Postgres, ImportConflict::Update), &rows, false, "a.csv").unwrap_or_else(|e| panic!("{e}"));
        assert!(update.script.contains("ON CONFLICT (\"id\") DO UPDATE SET \"name\" = EXCLUDED.\"name\";"), "{}", update.script);

        let mysql_skip = chunk_script(&plan(Engine::Mysql, ImportConflict::Skip), &rows, false, "a.csv").unwrap_or_else(|e| panic!("{e}"));
        assert!(mysql_skip.script.contains("INSERT IGNORE INTO `public`.`users` (`id`, `name`) VALUES"), "{}", mysql_skip.script);
        let mysql_update = chunk_script(&plan(Engine::Mysql, ImportConflict::Update), &rows, false, "a.csv").unwrap_or_else(|e| panic!("{e}"));
        assert!(mysql_update.script.contains("ON DUPLICATE KEY UPDATE `name` = VALUES(`name`);"), "{}", mysql_update.script);

        let sqlite = chunk_script(&plan(Engine::Sqlite, ImportConflict::Update), &rows, true, "a.csv").unwrap_or_else(|e| panic!("{e}"));
        assert!(sqlite.script.contains("DELETE FROM \"public\".\"users\";\nINSERT INTO"), "{}", sqlite.script);

        let truncated = chunk_script(&plan(Engine::Postgres, ImportConflict::Error), &rows, true, "a.csv").unwrap_or_else(|e| panic!("{e}"));
        assert!(truncated.script.starts_with("BEGIN;\nTRUNCATE TABLE \"public\".\"users\";\nINSERT INTO"), "{}", truncated.script);
        assert!(truncated.summary.contains("TRUNCATE TABLE \"public\".\"users\";\nINSERT INTO"), "the guard must see the TRUNCATE: {}", truncated.summary);

        let mssql = chunk_script(&plan(Engine::Mssql, ImportConflict::Error), &rows, false, "a.csv").unwrap_or_else(|e| panic!("{e}"));
        assert!(mssql.script.starts_with("BEGIN TRANSACTION;\n") && mssql.script.ends_with("COMMIT TRANSACTION;"));
        let oracle = chunk_script(&plan(Engine::Oracle, ImportConflict::Error), &rows, false, "a.csv").unwrap_or_else(|e| panic!("{e}"));
        assert!(!oracle.script.contains("BEGIN"), "BEGIN opens a PL/SQL block on Oracle");

        assert!(chunk_script(&plan(Engine::Mssql, ImportConflict::Skip), &rows, false, "a").is_err(), "no one-statement skip on SQL Server");
        let no_keys = ChunkPlan { engine: Engine::Postgres, table: &table, columns: &columns, keys: &[], conflict: ImportConflict::Update, transactional: true };
        assert!(chunk_script(&no_keys, &rows, false, "a").is_err(), "an upsert needs a key");
        assert!(conflict_support(Engine::Postgres).update && conflict_support(Engine::Mysql).skip && !conflict_support(Engine::Mongodb).skip);
    }

    #[test]
    fn the_summary_is_what_the_guard_judges() {
        let rows = vec![vec![Value::Int(1)]];
        let columns = vec!["id".to_string()];
        let table = users();
        let plan = ChunkPlan { engine: Engine::Postgres, table: &table, columns: &columns, keys: &[], conflict: ImportConflict::Error, transactional: true };
        let kinds = |truncate: bool| {
            let script = chunk_script(&plan, &rows, truncate, "x.csv").unwrap_or_else(|e| panic!("{e}"));
            crate::guard::destructive::classify(&script.summary).into_iter().map(|s| s.kind).collect::<Vec<_>>()
        };
        use crate::guard::destructive::StatementKind;
        assert_eq!(kinds(false), vec![StatementKind::Write]);
        assert_eq!(kinds(true), vec![StatementKind::Destructive, StatementKind::Write]);
    }

    #[test]
    fn tables_files_are_read_in_order_with_their_schema() {
        let dir = dir();
        let text = br#"{"__schema":{"users":[{"name":"id","type":"integer"},{"name":"email","type":"VARCHAR"}]},
                         "users":[{"id":1,"email":"a@x"},{"id":2,"email":null}],
                         "orders":[{"total":9.5,"paid":true}],
                         "weird": 5}"#;
        let path = write(&dir, "d.json", text);
        let parsed = read_tables(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["users", "orders"], "file order");
        assert_eq!(parsed.schema.len(), 1);
        assert_eq!(parsed.errors.len(), 1, "the odd member is reported: {:?}", parsed.errors);
        let orders = &parsed.tables[1];
        assert_eq!(orders.inferred.iter().map(|c| c.data_type.as_str()).collect::<Vec<_>>(), vec!["float", "bool"]);

        let known = vec![TableRef { schema: Some("public".into()), name: "orders".into() }];
        let ops = plan_operations(Engine::Postgres, true, &parsed, &known);
        let ids: Vec<&str> = ops.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, vec!["create:users", "insert:users", "insert:orders"], "orders exists and has no schema: only an insert");
        assert!(!ops[0].exists && ops[2].exists);
        assert_eq!(ops[1].row_count, 2);
        assert!(ops[0].sql.starts_with("CREATE TABLE \"users\" (\n  \"id\" integer,\n  \"email\" VARCHAR\n)"), "{}", ops[0].sql);
        let mysql = plan_operations(Engine::Mysql, false, &parsed, &[]);
        assert!(mysql[0].sql.contains("`email` VARCHAR(255)"), "{}", mysql[0].sql);
        let inferred = create_table_sql(Engine::Sqlite, &TableRef { schema: None, name: "orders".into() }, &[], &orders.inferred).unwrap_or_default();
        assert_eq!(inferred, "CREATE TABLE \"orders\" (\n  \"total\" REAL,\n  \"paid\" BOOLEAN\n)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_table_names_resolve_to_catalog_tables() {
        let known = vec![TableRef { schema: Some("public".into()), name: "users".into() }, TableRef { schema: None, name: "my.table".into() }];
        assert_eq!(resolve_target(true, "public.users", &known), known[0]);
        assert_eq!(resolve_target(true, "users", &known), known[0]);
        assert_eq!(resolve_target(false, "my.table", &known), known[1]);
        assert_eq!(resolve_target(true, "audit.log", &known), TableRef { schema: Some("audit".into()), name: "log".into() });
        assert_eq!(resolve_target(false, "audit.log", &known), TableRef { schema: None, name: "audit.log".into() });
    }

    #[test]
    fn delimiters_must_be_one_character() {
        let dir = dir();
        let path = write(&dir, "c.csv", b"a,b\n");
        let bad = CsvImportOptions { delimiter: ";;".into(), ..CsvImportOptions::default() };
        assert!(ImportSource::open(&path, TransferFormat::Csv, &bad).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
