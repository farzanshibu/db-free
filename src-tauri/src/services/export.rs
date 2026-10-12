// SOT: export-service, export-tables, export-preview, export-bundle, export-zip, row-formatting, csv-json-sql-markdown-writers

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::model::{
    Engine, ExportFormat, ExportJsonShape, ExportLayout, ExportOptions, ExportPreview, ExportProgress, ExportReport,
    ExportStage, ExportedFile, PageQuery, SortRule, TableRef, Value,
};
use crate::services::changes;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

// ============================================================================
// EXPORT
//
// WHAT:  Streams tables into files: one file per table in a folder, or one file
//        (a ZIP of per-table files, one combined JSON, one combined SQL script)
//        at a chosen path; and renders the first rows of the same output as a
//        preview.
// WHY:   PRD §7 Phase 3 import/export. A million-row table never sits in memory:
//        rows are fetched a page at a time and written as they arrive. The
//        preview runs the very writers the file does, so what the dialog shows
//        is what the file will hold.
// HOW:   `RowWriter` turns pages of `Value` rows into one format's bytes. The
//        async layer around it fetches pages, checks the Stop token between
//        them, reports progress and writes to a temp file that is renamed into
//        place only when everything succeeded (a cancelled or failed export
//        leaves no half file behind).
// WHERE: src-tauri/src/commands/transfer.rs (callers), src/lib/export.ts (the
//        same formats for rows already on screen)
// ============================================================================

const PAGE: u32 = 1_000;
const PREVIEW_BYTES: usize = 1024 * 1024;
const BOM: &[u8] = b"\xEF\xBB\xBF";

type Sink<'a> = &'a mut (dyn Write + Send);
type Tick<'a> = &'a mut (dyn FnMut(ExportProgress) + Send);

fn io_error(err: impl std::fmt::Display) -> AppError {
    AppError::internal(err)
}

// ---------------------------------------------------------------------------
// Row formatting
// ---------------------------------------------------------------------------

/// How a table's JSON array sits in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonSlot {
    /// The file is this table: an array, or an envelope, as the options say.
    Standalone,
    /// A member of the combined `{ "table": [ … ] }` object (always an array,
    /// indented one level).
    Member,
}

/// A cell as plain text: what CSV, TSV and Markdown write.
pub fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Decimal(s) | Value::Text(s) | Value::Bytes(s) | Value::DateTime(s) | Value::Unsupported(s) => s.clone(),
        Value::Json(j) => j.to_string(),
    }
}

fn is_formula(text: &str) -> bool {
    matches!(text.chars().next(), Some('=' | '+' | '-' | '@' | '\t' | '\r'))
}

fn delimited_field(text: &str, delimiter: char, force_quote: bool) -> String {
    if force_quote || text.contains(delimiter) || text.contains(['"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

// WHAT:  One CSV / TSV cell.
// WHY:   An empty string and a NULL are different cells; with an empty NULL text
//        the empty string is written `""` so a reader can tell them apart. Only
//        text is checked for formula injection: a number such as -5 is a value,
//        not a formula, and quoting it would corrupt the column.
fn delimited_cell(value: &Value, delimiter: char, options: &ExportOptions) -> String {
    match value {
        Value::Null => delimited_field(&options.null_text, delimiter, false),
        Value::Text(s) if s.is_empty() && options.null_text.is_empty() => "\"\"".to_string(),
        Value::Text(s) if options.quote_formulas && is_formula(s) => delimited_field(&format!("'{s}"), delimiter, true),
        other => delimited_field(&cell_text(other), delimiter, false),
    }
}

fn markdown_cell(text: &str) -> String {
    text.replace('|', "\\|").replace("\r\n", "<br>").replace(['\n', '\r'], "<br>")
}

fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

/// `lines` after the first are indented by `by` spaces.
fn indent_tail(text: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    text.lines().enumerate().map(|(i, line)| if i == 0 { line.to_string() } else { format!("{pad}{line}") }).collect::<Vec<_>>().join("\n")
}

// WHAT:  A cell as JSON text. Numbers stay numbers, exact decimals, bytes (base64)
//        and dates stay strings, and a JSON cell is embedded as the document it is.
fn json_cell_text(value: &Value, pretty: bool, indent: usize) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => serde_json::Number::from_f64(*f).map(|n| n.to_string()).unwrap_or_else(|| "null".to_string()),
        Value::Decimal(s) | Value::Text(s) | Value::Bytes(s) | Value::DateTime(s) | Value::Unsupported(s) => json_string(s),
        Value::Json(j) => {
            if pretty {
                indent_tail(&serde_json::to_string_pretty(j).unwrap_or_else(|_| "null".to_string()), indent)
            } else {
                j.to_string()
            }
        }
    }
}

// WHAT:  One row as a JSON object with its keys in column order.
// WHY:   serde_json's map sorts keys, which would scramble the column order an
//        export is read in; the object is assembled by hand instead.
fn json_object(columns: &[String], row: &[Value], pretty: bool, indent: usize) -> String {
    if columns.is_empty() {
        return "{}".to_string();
    }
    if !pretty {
        let fields: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(i, name)| format!("{}:{}", json_string(name), json_cell_text(row.get(i).unwrap_or(&Value::Null), false, 0)))
            .collect();
        return format!("{{{}}}", fields.join(","));
    }
    let pad = " ".repeat(indent + 2);
    let fields: Vec<String> = columns
        .iter()
        .enumerate()
        .map(|(i, name)| format!("{pad}{}: {}", json_string(name), json_cell_text(row.get(i).unwrap_or(&Value::Null), true, indent + 2)))
        .collect();
    format!("{{\n{}\n{}}}", fields.join(",\n"), " ".repeat(indent))
}

// WHAT:  Writes one table in one format, in three calls: `begin`, `rows` (any
//        number of times), `finish`.
// WHY:   The export streams pages, and the preview feeds it a few rows: both go
//        through here so they cannot drift.
struct RowWriter<'a> {
    options: &'a ExportOptions,
    engine: Engine,
    table: &'a TableRef,
    columns: Vec<String>,
    slot: JsonSlot,
    /// CREATE statement for SQL, already fetched (it needs the session).
    ddl: Option<String>,
    written: u64,
}

impl<'a> RowWriter<'a> {
    fn new(options: &'a ExportOptions, engine: Engine, table: &'a TableRef, columns: Vec<String>, slot: JsonSlot, ddl: Option<String>) -> Self {
        RowWriter { options, engine, table, columns, slot, ddl, written: 0 }
    }

    fn delimiter(&self) -> char {
        match self.options.format {
            ExportFormat::Tsv => '\t',
            _ => self.options.delimiter.chars().next().unwrap_or(','),
        }
    }

    fn envelope(&self) -> bool {
        self.slot == JsonSlot::Standalone && self.options.json_shape == ExportJsonShape::Envelope
    }

    /// Spaces before each array item, and before the closing bracket.
    fn item_indent(&self) -> usize {
        match (self.slot, self.envelope()) {
            (_, true) => 4,
            (JsonSlot::Member, false) => 4,
            (JsonSlot::Standalone, false) => 2,
        }
    }

    fn close_indent(&self) -> usize {
        self.item_indent() - 2
    }

    fn begin(&mut self, out: &mut dyn Write) -> AppResult<()> {
        let text = match self.options.format {
            ExportFormat::Csv | ExportFormat::Tsv => {
                if self.options.header {
                    let d = self.delimiter();
                    let header: Vec<String> = self.columns.iter().map(|c| delimited_field(c, d, false)).collect();
                    format!("{}\n", header.join(&d.to_string()))
                } else {
                    String::new()
                }
            }
            ExportFormat::Markdown => {
                let header: Vec<String> = self.columns.iter().map(|c| markdown_cell(c)).collect();
                let rule: Vec<&str> = self.columns.iter().map(|_| "---").collect();
                format!("| {} |\n| {} |\n", header.join(" | "), rule.join(" | "))
            }
            ExportFormat::Json => {
                let pretty = self.options.pretty;
                if self.envelope() {
                    let table = json_string(&table_label(self.table));
                    if pretty {
                        format!("{{\n  \"table\": {table},\n  \"rows\": [")
                    } else {
                        format!("{{\"table\":{table},\"rows\":[")
                    }
                } else {
                    "[".to_string()
                }
            }
            ExportFormat::Ndjson => String::new(),
            ExportFormat::Sql => {
                let mut text = format!("-- Table: {}\n", table_label(self.table));
                if let Some(ddl) = &self.ddl {
                    let ddl = ddl.trim_end().trim_end_matches(';');
                    text.push_str(&format!("{ddl};\n\n"));
                }
                text
            }
        };
        out.write_all(text.as_bytes()).map_err(io_error)
    }

    fn rows(&mut self, out: &mut dyn Write, rows: &[Vec<Value>]) -> AppResult<()> {
        let mut text = String::new();
        match self.options.format {
            ExportFormat::Csv | ExportFormat::Tsv => {
                let d = self.delimiter();
                for row in rows {
                    let cells: Vec<String> = (0..self.columns.len()).map(|i| delimited_cell(row.get(i).unwrap_or(&Value::Null), d, self.options)).collect();
                    text.push_str(&cells.join(&d.to_string()));
                    text.push('\n');
                }
            }
            ExportFormat::Markdown => {
                for row in rows {
                    let cells: Vec<String> = (0..self.columns.len())
                        .map(|i| match row.get(i).unwrap_or(&Value::Null) {
                            Value::Null => markdown_cell(&self.options.null_text),
                            other => markdown_cell(&cell_text(other)),
                        })
                        .collect();
                    text.push_str(&format!("| {} |\n", cells.join(" | ")));
                }
            }
            ExportFormat::Json => {
                let pretty = self.options.pretty;
                let indent = self.item_indent();
                for (i, row) in rows.iter().enumerate() {
                    let object = json_object(&self.columns, row, pretty, if pretty { indent } else { 0 });
                    // Every row after the file's first one follows a comma.
                    let separator = if self.written == 0 && i == 0 { "" } else { "," };
                    if pretty {
                        text.push_str(&format!("{separator}\n{}{object}", " ".repeat(indent)));
                    } else {
                        text.push_str(&format!("{separator}\n{object}"));
                    }
                }
            }
            ExportFormat::Ndjson => {
                for row in rows {
                    text.push_str(&json_object(&self.columns, row, false, 0));
                    text.push('\n');
                }
            }
            ExportFormat::Sql => {
                for statement in changes::insert_batches(self.engine, self.table, &self.columns, rows, self.options.batch_rows.max(1) as usize) {
                    text.push_str(&statement);
                    text.push_str(";\n");
                }
            }
        }
        self.written += rows.len() as u64;
        out.write_all(text.as_bytes()).map_err(io_error)
    }

    fn finish(&mut self, out: &mut dyn Write) -> AppResult<()> {
        let text = match self.options.format {
            ExportFormat::Json => {
                let pretty = self.options.pretty;
                let close = " ".repeat(self.close_indent());
                let empty = self.written == 0;
                let array_end = if empty { "]".to_string() } else if pretty { format!("\n{close}]") } else { "\n]".to_string() };
                if self.envelope() {
                    if pretty {
                        format!("{array_end}\n}}\n")
                    } else {
                        format!("{array_end}}}\n")
                    }
                } else if self.slot == JsonSlot::Standalone {
                    format!("{array_end}\n")
                } else {
                    array_end
                }
            }
            ExportFormat::Sql => "\n".to_string(),
            ExportFormat::Csv | ExportFormat::Tsv | ExportFormat::Markdown | ExportFormat::Ndjson => String::new(),
        };
        out.write_all(text.as_bytes()).map_err(io_error)
    }
}

fn table_label(table: &TableRef) -> String {
    match &table.schema {
        Some(schema) if !schema.is_empty() => format!("{schema}.{}", table.name),
        _ => table.name.clone(),
    }
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

/// What one export produces at its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// One table, one file.
    Plain,
    /// Several tables as per-table files in a ZIP.
    Zip,
    /// Several tables (or one with its schema) as `{ "table": [rows] }`.
    CombinedJson,
    /// Every table's statements in one script.
    CombinedSql,
}

// WHAT:  The file shape a bundle export takes. Mirrored in
//        src/features/transfer (default file name and save-dialog filter).
pub fn container_of(options: &ExportOptions, table_count: usize) -> Container {
    match options.format {
        ExportFormat::Sql => Container::CombinedSql,
        ExportFormat::Json if table_count > 1 || options.include_schema => Container::CombinedJson,
        _ if table_count > 1 => Container::Zip,
        _ => Container::Plain,
    }
}

/// The extension of a bundle's single file.
pub fn bundle_extension(options: &ExportOptions, table_count: usize) -> &'static str {
    match container_of(options, table_count) {
        Container::Zip => "zip",
        Container::CombinedJson => "json",
        Container::CombinedSql => "sql",
        Container::Plain => options.format.extension(),
    }
}

fn safe_stem(table: &TableRef) -> String {
    let safe: String = table_label(table).chars().map(|c| if c.is_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' }).collect();
    let trimmed = safe.trim_start_matches('.');
    if trimmed.is_empty() { "table".to_string() } else { trimmed.to_string() }
}

fn folder_file(directory: &Path, table: &TableRef, extension: &str) -> PathBuf {
    directory.join(format!("{}.{extension}", safe_stem(table)))
}

// WHAT:  A name for each table's entry in a ZIP, unique within it.
fn entry_names(tables: &[TableRef], extension: &str) -> Vec<String> {
    let mut used: HashSet<String> = HashSet::new();
    tables
        .iter()
        .map(|table| {
            let stem = safe_stem(table);
            let mut name = format!("{stem}.{extension}");
            let mut n = 2;
            while !used.insert(name.clone()) {
                name = format!("{stem}_{n}.{extension}");
                n += 1;
            }
            name
        })
        .collect()
}

fn with_extension_if_missing(path: &Path, extension: &str) -> PathBuf {
    if path.extension().is_some() {
        path.to_path_buf()
    } else {
        path.with_extension(extension)
    }
}

fn part_path(target: &Path) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "export".to_string());
    target.with_file_name(format!(".{name}.{}.part", uuid::Uuid::new_v4().simple()))
}

// ---------------------------------------------------------------------------
// Fetching and writing one table
// ---------------------------------------------------------------------------

struct Run<'a> {
    ctx: &'a SessionCtx,
    options: &'a ExportOptions,
    /// Rows per table at most.
    limit: u64,
    cancel: &'a CancellationToken,
}

struct TableOutcome {
    rows: u64,
    capped: bool,
}

/// Where a table sits in the export, for the progress frames.
#[derive(Clone, Copy)]
struct Place {
    rows_before: u64,
    done: u32,
    total: u32,
}

// WHAT:  The CREATE statement for SQL output: the adapter's own DDL, else the
//        engine's create template built from the columns.
async fn schema_sql(run: &Run<'_>, table: &TableRef, columns: &[crate::model::ColumnInfo]) -> AppResult<Option<String>> {
    if let Some(ddl) = run.ctx.integration.ddl(table).await? {
        return Ok(Some(ddl));
    }
    Ok(run.ctx.integration.create_template(table, columns))
}

async fn write_table(run: &Run<'_>, table: &TableRef, slot: JsonSlot, out: Sink<'_>, place: Place, tick: Tick<'_>) -> AppResult<TableOutcome> {
    let columns = run.ctx.integration.columns(table).await?;
    if columns.is_empty() {
        return Err(AppError::not_found(format!("\"{}\" has no columns or does not exist.", table_label(table))));
    }
    let names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    let sort: Vec<SortRule> = columns.iter().filter(|c| c.primary_key).map(|c| SortRule { column: c.name.clone(), desc: false }).collect();
    let ddl = if run.options.format == ExportFormat::Sql && run.options.include_schema { schema_sql(run, table, &columns).await? } else { None };
    let mut writer = RowWriter::new(run.options, run.ctx.connection.engine, table, names, slot, ddl);
    writer.begin(&mut *out)?;
    let mut written: u64 = 0;
    let mut capped = false;
    if run.options.include_data {
        let mut offset: u64 = 0;
        loop {
            if run.cancel.is_cancelled() {
                return Err(AppError::cancelled("Cancelled by user"));
            }
            let want = run.limit.saturating_sub(written).min(u64::from(PAGE));
            if want == 0 {
                capped = true;
                break;
            }
            let query = PageQuery { sort: sort.clone(), filters: Vec::new(), offset, limit: u32::try_from(want).unwrap_or(PAGE) };
            let page = run.ctx.integration.fetch_page(table, &query).await?;
            let count = page.rows.len() as u64;
            writer.rows(&mut *out, &page.rows)?;
            written += count;
            offset += count;
            tick(ExportProgress { stage: ExportStage::Writing, table: Some(table_label(table)), tables_done: place.done, tables_total: place.total, rows: place.rows_before + written });
            if count < want {
                break;
            }
            if written >= run.limit {
                capped = true;
                break;
            }
        }
    }
    writer.finish(&mut *out)?;
    Ok(TableOutcome { rows: written, capped })
}

// WHAT:  The `{ "table": [{name, type}] }` object that follows `"__schema": ` in a
//        combined JSON file, in table order and column order.
// WHY:   Built by hand rather than through serde_json's map, which would sort the
//        tables alphabetically and no longer match the order of the data.
fn schema_json(tables: &[(String, Vec<(String, String)>)], pretty: bool) -> String {
    let column = |name: &str, kind: &str| format!("{{\"name\": {}, \"type\": {}}}", json_string(name), json_string(kind));
    if !pretty {
        let entries: Vec<String> = tables
            .iter()
            .map(|(table, columns)| {
                let described: Vec<String> = columns.iter().map(|(n, t)| format!("{{\"name\":{},\"type\":{}}}", json_string(n), json_string(t))).collect();
                format!("{}:[{}]", json_string(table), described.join(","))
            })
            .collect();
        return format!("{{{}}}", entries.join(","));
    }
    let mut out = String::from("{");
    for (i, (table, columns)) in tables.iter().enumerate() {
        out.push_str(if i == 0 { "\n" } else { ",\n" });
        out.push_str(&format!("    {}: [", json_string(table)));
        for (j, (name, kind)) in columns.iter().enumerate() {
            out.push_str(if j == 0 { "\n" } else { ",\n" });
            out.push_str(&format!("      {}", column(name, kind)));
        }
        out.push_str(if columns.is_empty() { "]" } else { "\n    ]" });
    }
    out.push_str(if tables.is_empty() { "}" } else { "\n  }" });
    out
}

async fn schema_block(run: &Run<'_>, tables: &[TableRef]) -> AppResult<String> {
    let mut described: Vec<(String, Vec<(String, String)>)> = Vec::with_capacity(tables.len());
    for table in tables {
        let columns = run.ctx.integration.columns(table).await?;
        described.push((table_label(table), columns.into_iter().map(|c| (c.name, c.data_type)).collect()));
    }
    Ok(schema_json(&described, run.options.pretty))
}

fn note_cap(report: &mut Vec<String>, table: &TableRef, rows: u64) {
    report.push(format!("{} stopped at the {rows}-row limit.", table_label(table)));
}

/// Everything one bundle write reports back.
struct Written {
    files: Vec<ExportedFile>,
    warnings: Vec<String>,
}

// WHAT:  Writes a Plain / CombinedJson / CombinedSql container to `out`.
async fn write_flat(
    run: &Run<'_>,
    container: Container,
    tables: &[TableRef],
    path: &str,
    out: Sink<'_>,
    tick: Tick<'_>,
) -> AppResult<Written> {
    let total = u32::try_from(tables.len()).unwrap_or(u32::MAX);
    let mut files = Vec::with_capacity(tables.len());
    let mut warnings = Vec::new();
    let mut rows_so_far: u64 = 0;
    if run.options.bom {
        out.write_all(BOM).map_err(io_error)?;
    }
    match container {
        Container::CombinedJson => {
            let pretty = run.options.pretty;
            out.write_all(b"{").map_err(io_error)?;
            let mut first = true;
            if run.options.include_schema {
                let block = schema_block(run, tables).await?;
                let lead = if pretty { "\n  " } else { "\n" };
                out.write_all(format!("{lead}\"__schema\": {block}").as_bytes()).map_err(io_error)?;
                first = false;
            }
            for (index, table) in tables.iter().enumerate() {
                let lead = if first { "" } else { "," };
                let indent = if pretty { "  " } else { "" };
                out.write_all(format!("{lead}\n{indent}{}: ", json_string(&table_label(table))).as_bytes()).map_err(io_error)?;
                first = false;
                let place = Place { rows_before: rows_so_far, done: u32::try_from(index).unwrap_or(0), total };
                let outcome = write_table(run, table, JsonSlot::Member, &mut *out, place, &mut *tick).await?;
                rows_so_far += outcome.rows;
                if outcome.capped {
                    note_cap(&mut warnings, table, run.limit);
                }
                files.push(ExportedFile { table: table.clone(), path: path.to_string(), rows: outcome.rows });
            }
            out.write_all(b"\n}\n").map_err(io_error)?;
        }
        Container::Plain | Container::CombinedSql | Container::Zip => {
            for (index, table) in tables.iter().enumerate() {
                let place = Place { rows_before: rows_so_far, done: u32::try_from(index).unwrap_or(0), total };
                let outcome = write_table(run, table, JsonSlot::Standalone, &mut *out, place, &mut *tick).await?;
                rows_so_far += outcome.rows;
                if outcome.capped {
                    note_cap(&mut warnings, table, run.limit);
                }
                files.push(ExportedFile { table: table.clone(), path: path.to_string(), rows: outcome.rows });
            }
        }
    }
    out.flush().map_err(io_error)?;
    Ok(Written { files, warnings })
}

async fn write_zip(run: &Run<'_>, tables: &[TableRef], path: &str, file: std::fs::File, tick: Tick<'_>) -> AppResult<Written> {
    let total = u32::try_from(tables.len()).unwrap_or(u32::MAX);
    let names = entry_names(tables, run.options.format.extension());
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let entry_options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    let mut files = Vec::with_capacity(tables.len());
    let mut warnings = Vec::new();
    let mut rows_so_far: u64 = 0;
    for (index, (table, name)) in tables.iter().zip(&names).enumerate() {
        zip.start_file(name.as_str(), entry_options).map_err(io_error)?;
        if run.options.bom {
            zip.write_all(BOM).map_err(io_error)?;
        }
        let place = Place { rows_before: rows_so_far, done: u32::try_from(index).unwrap_or(0), total };
        let outcome = write_table(run, table, JsonSlot::Standalone, &mut zip, place, &mut *tick).await?;
        rows_so_far += outcome.rows;
        if outcome.capped {
            note_cap(&mut warnings, table, run.limit);
        }
        files.push(ExportedFile { table: table.clone(), path: path.to_string(), rows: outcome.rows });
    }
    tick(ExportProgress { stage: ExportStage::Finishing, table: None, tables_done: total, tables_total: total, rows: rows_so_far });
    let mut inner = zip.finish().map_err(io_error)?;
    inner.flush().map_err(io_error)?;
    Ok(Written { files, warnings })
}

fn check_request(tables: &[TableRef], options: &ExportOptions) -> AppResult<()> {
    if tables.is_empty() {
        return Err(AppError::invalid_input("Select at least one table to export."));
    }
    if options.format == ExportFormat::Csv && options.delimiter.chars().count() != 1 {
        return Err(AppError::invalid_input("The CSV delimiter must be exactly one character."));
    }
    if options.format == ExportFormat::Sql && !options.include_schema && !options.include_data {
        return Err(AppError::invalid_input("Turn on the structure or the data: there is nothing else to write."));
    }
    Ok(())
}

/// One export: what to write, how, and where.
pub struct ExportJob<'a> {
    pub tables: &'a [TableRef],
    pub options: &'a ExportOptions,
    pub layout: ExportLayout,
    pub path: &'a Path,
    /// Rows per table at most. A runaway export cannot fill the disk; the tables
    /// it cut short are named in the report.
    pub limit: u64,
}

// WHAT:  Runs `job` (see `ExportLayout`), checking `cancel` between pages.
pub async fn export_tables(ctx: &SessionCtx, job: &ExportJob<'_>, cancel: &CancellationToken, tick: Tick<'_>) -> AppResult<ExportReport> {
    let (tables, options) = (job.tables, job.options);
    check_request(tables, options)?;
    let started = Instant::now();
    let run = Run { ctx, options, limit: job.limit.max(1), cancel };
    let (written, bytes) = match job.layout {
        ExportLayout::Folder => export_folder(&run, tables, job.path, tick).await?,
        ExportLayout::Bundle => export_bundle(&run, tables, job.path, tick).await?,
    };
    Ok(ExportReport {
        files: written.files,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        bytes,
        warnings: written.warnings,
    })
}

async fn export_folder(run: &Run<'_>, tables: &[TableRef], directory: &Path, tick: Tick<'_>) -> AppResult<(Written, u64)> {
    std::fs::create_dir_all(directory).map_err(io_error)?;
    let total = u32::try_from(tables.len()).unwrap_or(u32::MAX);
    let extension = run.options.format.extension();
    let mut written = Written { files: Vec::new(), warnings: Vec::new() };
    let mut bytes: u64 = 0;
    let mut rows_so_far: u64 = 0;
    for (index, table) in tables.iter().enumerate() {
        let target = folder_file(directory, table, extension);
        let part = part_path(&target);
        let result: AppResult<TableOutcome> = async {
            let file = std::fs::File::create(&part).map_err(io_error)?;
            let mut out = std::io::BufWriter::new(file);
            if run.options.bom {
                out.write_all(BOM).map_err(io_error)?;
            }
            let place = Place { rows_before: rows_so_far, done: u32::try_from(index).unwrap_or(0), total };
            let outcome = write_table(run, table, JsonSlot::Standalone, &mut out, place, &mut *tick).await?;
            out.flush().map_err(io_error)?;
            Ok(outcome)
        }
        .await;
        match result {
            Ok(outcome) => {
                std::fs::rename(&part, &target).map_err(io_error)?;
                bytes += std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                rows_so_far += outcome.rows;
                if outcome.capped {
                    note_cap(&mut written.warnings, table, run.limit);
                }
                written.files.push(ExportedFile { table: table.clone(), path: target.to_string_lossy().into_owned(), rows: outcome.rows });
            }
            Err(err) => {
                let _ = std::fs::remove_file(&part);
                return Err(err);
            }
        }
    }
    Ok((written, bytes))
}

async fn export_bundle(run: &Run<'_>, tables: &[TableRef], path: &Path, tick: Tick<'_>) -> AppResult<(Written, u64)> {
    let container = container_of(run.options, tables.len());
    let target = with_extension_if_missing(path, bundle_extension(run.options, tables.len()));
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(AppError::invalid_input(format!("{} is not a folder.", parent.display())));
        }
    }
    let part = part_path(&target);
    let shown = target.to_string_lossy().into_owned();
    let result: AppResult<Written> = async {
        let file = std::fs::File::create(&part).map_err(io_error)?;
        match container {
            Container::Zip => write_zip(run, tables, &shown, file, &mut *tick).await,
            other => {
                let mut out = std::io::BufWriter::new(file);
                write_flat(run, other, tables, &shown, &mut out, &mut *tick).await
            }
        }
    }
    .await;
    match result {
        Ok(written) => {
            std::fs::rename(&part, &target).map_err(io_error)?;
            let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            Ok((written, bytes))
        }
        Err(err) => {
            let _ = std::fs::remove_file(&part);
            Err(err)
        }
    }
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

/// A buffer that keeps the first `cap` bytes and notes that it dropped the rest.
struct Capped {
    buf: Vec<u8>,
    cap: usize,
    truncated: bool,
}

impl Write for Capped {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let room = self.cap.saturating_sub(self.buf.len());
        if data.len() > room {
            self.truncated = true;
        }
        self.buf.extend_from_slice(&data[..data.len().min(room)]);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// WHAT:  The first `rows_per_table` rows of each table, formatted as the export
//        will write them (a ZIP shows each entry under a `# File:` line).
pub async fn preview(ctx: &SessionCtx, tables: &[TableRef], options: &ExportOptions, rows_per_table: u64) -> AppResult<ExportPreview> {
    check_request(tables, options)?;
    let cancel = CancellationToken::new();
    let run = Run { ctx, options, limit: rows_per_table.max(1), cancel: &cancel };
    let container = container_of(options, tables.len());
    let mut sink = Capped { buf: Vec::new(), cap: PREVIEW_BYTES, truncated: false };
    let mut rows: u64 = 0;
    let mut entries: Vec<String> = Vec::new();
    let mut tick = |p: ExportProgress| rows = rows.max(p.rows);
    match container {
        Container::Zip => {
            let names = entry_names(tables, options.format.extension());
            let total = u32::try_from(tables.len()).unwrap_or(u32::MAX);
            let mut so_far: u64 = 0;
            for (index, (table, name)) in tables.iter().zip(&names).enumerate() {
                if sink.truncated {
                    break;
                }
                sink.write_all(format!("# File: {name}\n").as_bytes()).map_err(io_error)?;
                let place = Place { rows_before: so_far, done: u32::try_from(index).unwrap_or(0), total };
                let outcome = write_table(&run, table, JsonSlot::Standalone, &mut sink, place, &mut tick).await?;
                so_far += outcome.rows;
                sink.write_all(b"\n").map_err(io_error)?;
            }
            entries = names;
        }
        other => {
            write_flat(&run, other, tables, "", &mut sink, &mut tick).await?;
        }
    }
    let text = String::from_utf8_lossy(&sink.buf);
    Ok(ExportPreview { text: text.trim_start_matches('\u{feff}').to_string(), truncated: sink.truncated, rows, entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ExportOptions;

    fn table() -> TableRef {
        TableRef { schema: Some("public".into()), name: "users".into() }
    }

    fn cols() -> Vec<String> {
        vec!["id".into(), "name".into(), "meta".into()]
    }

    fn rows() -> Vec<Vec<Value>> {
        vec![
            vec![Value::Int(1), Value::Text("Ann \"A\"".into()), Value::Json(serde_json::json!({"a": [1, 2]}))],
            vec![Value::Int(2), Value::Null, Value::Null],
            vec![Value::Int(3), Value::Text(String::new()), Value::Null],
        ]
    }

    /// begin + rows + finish into a string.
    fn render(options: &ExportOptions, slot: JsonSlot, data: &[Vec<Value>]) -> String {
        let t = table();
        let mut writer = RowWriter::new(options, Engine::Postgres, &t, cols(), slot, None);
        let mut out: Vec<u8> = Vec::new();
        writer.begin(&mut out).unwrap_or_else(|e| panic!("{e}"));
        writer.rows(&mut out, data).unwrap_or_else(|e| panic!("{e}"));
        writer.finish(&mut out).unwrap_or_else(|e| panic!("{e}"));
        String::from_utf8(out).unwrap_or_default()
    }

    fn with(format: ExportFormat) -> ExportOptions {
        ExportOptions { format, ..ExportOptions::default() }
    }

    #[test]
    fn csv_quotes_and_distinguishes_empty_from_null() {
        let text = render(&with(ExportFormat::Csv), JsonSlot::Standalone, &rows());
        assert_eq!(text, "id,name,meta\n1,\"Ann \"\"A\"\"\",\"{\"\"a\"\":[1,2]}\"\n2,,\n3,\"\",\n");
        let custom = ExportOptions { delimiter: ";".into(), null_text: "NULL".into(), header: false, ..with(ExportFormat::Csv) };
        assert_eq!(render(&custom, JsonSlot::Standalone, &rows()[1..]), "2;NULL;NULL\n3;;NULL\n");
    }

    #[test]
    fn spreadsheet_formulas_are_defused_but_numbers_are_not() {
        let data = vec![vec![Value::Int(-5), Value::Text("=1+1".into()), Value::Text("@SUM(A1)".into())], vec![Value::Float(-1.5), Value::Text("-x".into()), Value::Text("fine".into())]];
        let text = render(&with(ExportFormat::Csv), JsonSlot::Standalone, &data);
        assert_eq!(text, "id,name,meta\n-5,\"'=1+1\",\"'@SUM(A1)\"\n-1.5,\"'-x\",fine\n");
        let raw = ExportOptions { quote_formulas: false, ..with(ExportFormat::Csv) };
        assert!(render(&raw, JsonSlot::Standalone, &data).contains("-5,=1+1,@SUM(A1)"));
    }

    #[test]
    fn tsv_uses_tabs() {
        let text = render(&with(ExportFormat::Tsv), JsonSlot::Standalone, &rows()[1..2]);
        assert_eq!(text, "id\tname\tmeta\n2\t\t\n");
    }

    #[test]
    fn json_keeps_column_order_and_both_shapes() {
        let compact = ExportOptions { pretty: false, ..with(ExportFormat::Json) };
        assert_eq!(render(&compact, JsonSlot::Standalone, &rows()[..2]), "[\n{\"id\":1,\"name\":\"Ann \\\"A\\\"\",\"meta\":{\"a\":[1,2]}},\n{\"id\":2,\"name\":null,\"meta\":null}\n]\n");
        let pretty = render(&with(ExportFormat::Json), JsonSlot::Standalone, &rows()[1..2]);
        assert_eq!(pretty, "[\n  {\n    \"id\": 2,\n    \"name\": null,\n    \"meta\": null\n  }\n]\n");
        let parsed: serde_json::Value = serde_json::from_str(&pretty).unwrap_or_default();
        assert_eq!(parsed[0]["id"], 2);
        let envelope = ExportOptions { json_shape: ExportJsonShape::Envelope, ..with(ExportFormat::Json) };
        let text = render(&envelope, JsonSlot::Standalone, &rows()[1..2]);
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        assert_eq!(parsed["table"], "public.users");
        assert_eq!(parsed["rows"][0]["id"], 2);
        assert!(text.starts_with("{\n  \"table\": \"public.users\",\n  \"rows\": [\n    {\n      \"id\": 2,"), "{text}");
        let empty = render(&envelope, JsonSlot::Standalone, &[]);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&empty).unwrap_or_default()["rows"], serde_json::json!([]));
        assert_eq!(render(&with(ExportFormat::Json), JsonSlot::Standalone, &[]), "[]\n");
    }

    #[test]
    fn json_members_nest_inside_a_combined_object() {
        let t = table();
        let options = with(ExportFormat::Json);
        let mut writer = RowWriter::new(&options, Engine::Postgres, &t, cols(), JsonSlot::Member, None);
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"{\n  \"users\": ");
        writer.begin(&mut out).unwrap_or_else(|e| panic!("{e}"));
        writer.rows(&mut out, &rows()[1..2]).unwrap_or_else(|e| panic!("{e}"));
        writer.rows(&mut out, &rows()[2..3]).unwrap_or_else(|e| panic!("{e}"));
        writer.finish(&mut out).unwrap_or_else(|e| panic!("{e}"));
        out.extend_from_slice(b"\n}\n");
        let text = String::from_utf8(out).unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
        assert_eq!(parsed["users"].as_array().map(Vec::len), Some(2));
        assert!(text.contains("\n  ]\n}"), "{text}");
    }

    #[test]
    fn json_rows_across_calls_are_comma_separated() {
        let options = ExportOptions { pretty: false, ..with(ExportFormat::Json) };
        let t = table();
        let mut writer = RowWriter::new(&options, Engine::Postgres, &t, cols(), JsonSlot::Standalone, None);
        let mut out: Vec<u8> = Vec::new();
        writer.begin(&mut out).unwrap_or_else(|e| panic!("{e}"));
        writer.rows(&mut out, &rows()[..1]).unwrap_or_else(|e| panic!("{e}"));
        writer.rows(&mut out, &rows()[1..]).unwrap_or_else(|e| panic!("{e}"));
        writer.finish(&mut out).unwrap_or_else(|e| panic!("{e}"));
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap_or_default();
        assert_eq!(parsed.as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn ndjson_is_one_object_per_line() {
        let text = render(&with(ExportFormat::Ndjson), JsonSlot::Standalone, &rows()[1..]);
        assert_eq!(text, "{\"id\":2,\"name\":null,\"meta\":null}\n{\"id\":3,\"name\":\"\",\"meta\":null}\n");
    }

    #[test]
    fn markdown_escapes_pipes_and_newlines() {
        let data = vec![vec![Value::Int(1), Value::Text("a|b\nc".into()), Value::Null]];
        let options = ExportOptions { null_text: "∅".into(), ..with(ExportFormat::Markdown) };
        assert_eq!(render(&options, JsonSlot::Standalone, &data), "| id | name | meta |\n| --- | --- | --- |\n| 1 | a\\|b<br>c | ∅ |\n");
    }

    #[test]
    fn sql_writes_a_comment_ddl_and_batched_inserts() {
        let t = table();
        let options = ExportOptions { batch_rows: 2, include_schema: true, ..with(ExportFormat::Sql) };
        let mut writer = RowWriter::new(&options, Engine::Postgres, &t, vec!["id".into(), "name".into()], JsonSlot::Standalone, Some("CREATE TABLE public.users (id int, name text);".into()));
        let mut out: Vec<u8> = Vec::new();
        writer.begin(&mut out).unwrap_or_else(|e| panic!("{e}"));
        let data: Vec<Vec<Value>> = (1..=3).map(|i| vec![Value::Int(i), Value::Text(format!("n'{i}"))]).collect();
        writer.rows(&mut out, &data).unwrap_or_else(|e| panic!("{e}"));
        writer.finish(&mut out).unwrap_or_else(|e| panic!("{e}"));
        let text = String::from_utf8(out).unwrap_or_default();
        assert!(text.starts_with("-- Table: public.users\nCREATE TABLE public.users (id int, name text);\n\nINSERT INTO \"public\".\"users\" (\"id\", \"name\") VALUES\n(1, 'n''1'),\n(2, 'n''2');\nINSERT INTO"), "{text}");
        assert_eq!(text.matches("INSERT INTO").count(), 2);
    }

    #[test]
    fn containers_follow_format_and_table_count() {
        let o = |format| ExportOptions { format, ..ExportOptions::default() };
        assert_eq!(container_of(&o(ExportFormat::Csv), 1), Container::Plain);
        assert_eq!(container_of(&o(ExportFormat::Csv), 3), Container::Zip);
        assert_eq!(container_of(&o(ExportFormat::Markdown), 2), Container::Zip);
        assert_eq!(container_of(&o(ExportFormat::Ndjson), 2), Container::Zip);
        assert_eq!(container_of(&o(ExportFormat::Json), 1), Container::Plain);
        assert_eq!(container_of(&o(ExportFormat::Json), 2), Container::CombinedJson);
        assert_eq!(container_of(&ExportOptions { include_schema: true, ..o(ExportFormat::Json) }, 1), Container::CombinedJson);
        assert_eq!(container_of(&o(ExportFormat::Sql), 1), Container::CombinedSql);
        assert_eq!(bundle_extension(&o(ExportFormat::Csv), 3), "zip");
        assert_eq!(bundle_extension(&o(ExportFormat::Markdown), 1), "md");
        assert_eq!(bundle_extension(&o(ExportFormat::Json), 4), "json");
        assert_eq!(bundle_extension(&o(ExportFormat::Sql), 4), "sql");
    }

    #[test]
    fn the_schema_block_keeps_table_and_column_order() {
        let tables = vec![
            ("users".to_string(), vec![("id".to_string(), "integer".to_string()), ("name".to_string(), "text".to_string())]),
            ("audit".to_string(), vec![]),
        ];
        let pretty = schema_json(&tables, true);
        let parsed: serde_json::Value = serde_json::from_str(&pretty).unwrap_or_else(|e| panic!("{e}: {pretty}"));
        assert_eq!(parsed["users"][1]["name"], "name");
        assert_eq!(parsed["audit"], serde_json::json!([]));
        assert!(pretty.find("users").unwrap_or(usize::MAX) < pretty.find("audit").unwrap_or(0), "file order, not alphabetical");
        assert!(pretty.starts_with("{\n    \"users\": [\n      {\"name\": \"id\", \"type\": \"integer\"},\n"), "{pretty}");
        let compact = schema_json(&tables, false);
        assert_eq!(compact, "{\"users\":[{\"name\":\"id\",\"type\":\"integer\"},{\"name\":\"name\",\"type\":\"text\"}],\"audit\":[]}");
        assert_eq!(schema_json(&[], true), "{}");
    }

    #[test]
    fn names_are_safe_and_unique() {
        let p = folder_file(Path::new("/tmp/x"), &TableRef { schema: Some("pub lic".into()), name: "us/ers".into() }, "csv");
        assert_eq!(p.file_name().and_then(|n| n.to_str()), Some("pub_lic.us_ers.csv"));
        let twins = vec![TableRef { schema: None, name: "a b".into() }, TableRef { schema: None, name: "a_b".into() }, TableRef { schema: None, name: "../etc".into() }];
        assert_eq!(entry_names(&twins, "csv"), vec!["a_b.csv", "a_b_2.csv", "_etc.csv"]);
        assert_eq!(with_extension_if_missing(Path::new("/tmp/out"), "zip"), PathBuf::from("/tmp/out.zip"));
        assert_eq!(with_extension_if_missing(Path::new("/tmp/out.dump"), "zip"), PathBuf::from("/tmp/out.dump"));
        let part = part_path(Path::new("/tmp/out.zip"));
        assert!(part.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(".out.zip.") && n.ends_with(".part")));
    }

    #[test]
    fn requests_are_checked() {
        let one = vec![table()];
        assert!(check_request(&[], &ExportOptions::default()).is_err());
        assert!(check_request(&one, &ExportOptions { delimiter: ";;".into(), ..ExportOptions::default() }).is_err());
        assert!(check_request(&one, &ExportOptions { include_data: false, ..with(ExportFormat::Sql) }).is_err());
        assert!(check_request(&one, &ExportOptions { include_data: false, include_schema: true, ..with(ExportFormat::Sql) }).is_ok());
    }

    #[test]
    fn the_capped_buffer_keeps_a_prefix() {
        let mut sink = Capped { buf: Vec::new(), cap: 4, truncated: false };
        sink.write_all(b"abcdef").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(sink.buf, b"abcd");
        assert!(sink.truncated);
    }
}
