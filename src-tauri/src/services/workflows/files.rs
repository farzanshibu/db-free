// SOT: workflow-file-formats, read-file-node, save-file-node, delimited-text, ndjson, markdown-table, sql-insert-file

use super::data::{cell_text, Dataset};
use crate::error::{AppError, AppResult};
use crate::model::workflow::{FileFormat, SaveFormat};
use crate::model::{ColumnMeta, Engine, TableRef, Value};
use crate::services::changes;
use std::path::Path;

// ============================================================================
// FILE FORMATS FOR THE READ / SAVE NODES
//
// WHAT:  Reads CSV, TSV, JSON and NDJSON into a dataset; writes a dataset as
//        CSV, TSV, JSON, NDJSON, a Markdown table or SQL INSERT statements.
// WHY:   Workflows move rows between files and databases, in memory, so these
//        are text-in/text-out and the node adds only the disk I/O.
// HOW:   Delimited text follows RFC 4180: quoted fields may hold the delimiter,
//        doubled quotes and line breaks. An empty field reads as NULL. A JSON
//        file may be an array of objects or one object; NDJSON is an object per line.
// WHERE: src-tauri/src/services/workflows/nodes/io.rs
// ============================================================================

const SQL_INSERT_BATCH: usize = 100;

pub fn parse(content: &str, format: FileFormat) -> AppResult<Dataset> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    match format {
        FileFormat::Csv => parse_delimited(content, ','),
        FileFormat::Tsv => parse_delimited(content, '\t'),
        FileFormat::Json => parse_json(content),
        FileFormat::Ndjson => parse_ndjson(content),
    }
}

fn parse_json(content: &str) -> AppResult<Dataset> {
    let parsed: serde_json::Value = serde_json::from_str(content).map_err(|e| AppError::invalid_input(format!("Invalid JSON: {e}")))?;
    match parsed {
        serde_json::Value::Array(items) => Ok(Dataset::from_json_rows(&items)),
        serde_json::Value::Object(_) => Ok(Dataset::from_json_rows(&[parsed])),
        _ => Err(AppError::invalid_input("JSON root must be an array of objects or a single object.")),
    }
}

fn parse_ndjson(content: &str) -> AppResult<Dataset> {
    let mut items = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let item: serde_json::Value =
            serde_json::from_str(line).map_err(|e| AppError::invalid_input(format!("Invalid NDJSON on line {}: {e}", index + 1)))?;
        items.push(item);
    }
    Ok(Dataset::from_json_rows(&items))
}

// WHAT:  Splits delimited text into records of fields.
// HOW:   A small state machine: inside quotes everything is literal except a
//        doubled quote; outside, the delimiter ends a field and a line break ends
//        a record (blank records are skipped).
fn split_records(content: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut field_started = false;
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if !field_started => {
                in_quotes = true;
                field_started = true;
            }
            c if c == delimiter => {
                record.push(std::mem::take(&mut field));
                field_started = false;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                if field_started || !field.is_empty() || !record.is_empty() {
                    record.push(std::mem::take(&mut field));
                    records.push(std::mem::take(&mut record));
                }
                field_started = false;
            }
            other => {
                field.push(other);
                field_started = true;
            }
        }
    }
    if field_started || !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

fn parse_delimited(content: &str, delimiter: char) -> AppResult<Dataset> {
    let mut records = split_records(content, delimiter).into_iter();
    let Some(header) = records.next() else {
        return Ok(Dataset::empty());
    };
    let mut names: Vec<String> = Vec::with_capacity(header.len());
    for (index, raw) in header.iter().enumerate() {
        let name = raw.trim();
        let name = if name.is_empty() { format!("column{}", index + 1) } else { name.to_string() };
        names.push(name);
    }
    let rows = records
        .map(|record| {
            (0..names.len())
                .map(|i| match record.get(i) {
                    Some(text) if !text.is_empty() => Value::Text(text.clone()),
                    _ => Value::Null,
                })
                .collect()
        })
        .collect();
    let columns = names.into_iter().map(|name| ColumnMeta { name, type_name: "any".to_string() }).collect();
    Ok(Dataset { columns, rows })
}

// ---------------------------------------------------------------- writing
/// The file's content for a dataset. `table` names the table of the SQL format.
pub fn render(data: &Dataset, format: SaveFormat, table: &str) -> AppResult<String> {
    Ok(match format {
        SaveFormat::Csv => render_delimited(data, ','),
        SaveFormat::Tsv => render_delimited(data, '\t'),
        SaveFormat::Json => {
            let rows = serde_json::Value::Array(data.to_json_rows());
            let mut text = serde_json::to_string_pretty(&rows).map_err(AppError::internal)?;
            text.push('\n');
            text
        }
        SaveFormat::Ndjson => {
            let mut text = String::new();
            for row in data.to_json_rows() {
                text.push_str(&serde_json::to_string(&row).map_err(AppError::internal)?);
                text.push('\n');
            }
            text
        }
        SaveFormat::Markdown => render_markdown(data),
        SaveFormat::Sql => render_sql(data, table),
    })
}

fn quote_field(text: &str, delimiter: char) -> String {
    if text.contains(delimiter) || text.contains(['"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn render_delimited(data: &Dataset, delimiter: char) -> String {
    let sep = delimiter.to_string();
    let mut out = String::new();
    out.push_str(&data.columns.iter().map(|c| quote_field(&c.name, delimiter)).collect::<Vec<_>>().join(&sep));
    out.push('\n');
    for row in &data.rows {
        out.push_str(&row.iter().map(|v| quote_field(&cell_text(v), delimiter)).collect::<Vec<_>>().join(&sep));
        out.push('\n');
    }
    out
}

fn render_markdown(data: &Dataset) -> String {
    let cell = |text: String| text.replace('|', "\\|").replace(['\n', '\r'], " ");
    let names: Vec<String> = data.columns.iter().map(|c| cell(c.name.clone())).collect();
    let mut out = format!("| {} |\n| {} |\n", names.join(" | "), names.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
    for row in &data.rows {
        out.push_str(&format!("| {} |\n", row.iter().map(|v| cell(cell_text(v))).collect::<Vec<_>>().join(" | ")));
    }
    out
}

fn render_sql(data: &Dataset, table: &str) -> String {
    let table_ref = TableRef { schema: None, name: if table.trim().is_empty() { "rows".to_string() } else { table.trim().to_string() } };
    let columns: Vec<String> = data.columns.iter().map(|c| c.name.clone()).collect();
    if columns.is_empty() || data.rows.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for statement in changes::insert_batches(Engine::Postgres, &table_ref, &columns, &data.rows, SQL_INSERT_BATCH) {
        out.push_str(&statement);
        out.push_str(";\n");
    }
    out
}

/// The table a SQL file inserts into when the node names none: the file's stem.
pub fn table_from_path(path: &str) -> String {
    Path::new(path).file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or("rows").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[&[&str]]) -> Dataset {
        let columns = rows[0].iter().map(|n| ColumnMeta { name: (*n).into(), type_name: "any".into() }).collect();
        let body = rows[1..].iter().map(|r| r.iter().map(|c| Value::Text((*c).into())).collect()).collect();
        Dataset { columns, rows: body }
    }

    #[test]
    fn csv_reads_quotes_and_line_breaks() {
        let csv = "\u{feff}id,note,empty\r\n1,\"hello, \"\"world\"\"\",\r\n2,\"two\nlines\",x\r\n\r\n3,plain,";
        let ds = parse(csv, FileFormat::Csv).unwrap();
        assert_eq!(ds.column_names(), vec!["id", "note", "empty"]);
        assert_eq!(ds.rows.len(), 3);
        assert_eq!(ds.rows[0][1], Value::Text("hello, \"world\"".into()));
        assert_eq!(ds.rows[0][2], Value::Null, "an empty field reads as NULL");
        assert_eq!(ds.rows[1][1], Value::Text("two\nlines".into()));
        assert_eq!(ds.rows[2][0], Value::Text("3".into()));
        assert_eq!(ds.rows[2][2], Value::Null);
    }

    #[test]
    fn short_rows_pad_with_null_and_blank_headers_are_named() {
        let ds = parse("a,,c\n1\n", FileFormat::Csv).unwrap();
        assert_eq!(ds.column_names(), vec!["a", "column2", "c"]);
        assert_eq!(ds.rows[0], vec![Value::Text("1".into()), Value::Null, Value::Null]);
        assert!(parse("", FileFormat::Csv).unwrap().columns.is_empty());
    }

    #[test]
    fn tsv_round_trips_through_the_writer() {
        let ds = text(&[&["a", "b"], &["x\ty", "line1\nline2"], &["\"q\"", "plain"]]);
        let out = render(&ds, SaveFormat::Tsv, "").unwrap();
        let back = parse(&out, FileFormat::Tsv).unwrap();
        assert_eq!(back.column_names(), vec!["a", "b"]);
        assert_eq!(back.rows, ds.rows);
    }

    #[test]
    fn csv_writer_quotes_only_when_needed() {
        let ds = text(&[&["a", "b,c"], &["1", "say \"hi\""], &["", "x"]]);
        assert_eq!(render(&ds, SaveFormat::Csv, "").unwrap(), "a,\"b,c\"\n1,\"say \"\"hi\"\"\"\n,x\n");
    }

    #[test]
    fn json_and_ndjson_read_objects_and_report_bad_lines() {
        let ds = parse("[{\"a\":1},{\"a\":2,\"b\":\"x\"}]", FileFormat::Json).unwrap();
        assert_eq!(ds.column_names(), vec!["a", "b"]);
        assert_eq!(parse("{\"a\":1}", FileFormat::Json).unwrap().rows.len(), 1);
        assert!(parse("42", FileFormat::Json).is_err());
        assert!(parse("{nope", FileFormat::Json).is_err());
        let nd = parse("{\"a\":1}\n\n{\"a\":2}\n", FileFormat::Ndjson).unwrap();
        assert_eq!(nd.rows.len(), 2);
        let err = parse("{\"a\":1}\nnot json\n", FileFormat::Ndjson).err().map(|e| e.message().to_string());
        assert!(err.is_some_and(|m| m.contains("line 2")));
    }

    #[test]
    fn json_formats_keep_types() {
        let ds = Dataset {
            columns: vec![ColumnMeta { name: "n".into(), type_name: "integer".into() }, ColumnMeta { name: "s".into(), type_name: "string".into() }],
            rows: vec![vec![Value::Int(1), Value::Text("a".into())], vec![Value::Null, Value::Text("b".into())]],
        };
        let nd = render(&ds, SaveFormat::Ndjson, "").unwrap();
        assert_eq!(nd, "{\"n\":1,\"s\":\"a\"}\n{\"n\":null,\"s\":\"b\"}\n");
        let pretty = render(&ds, SaveFormat::Json, "").unwrap();
        assert!(pretty.starts_with("[\n  {\n    \"n\": 1"));
        assert_eq!(parse(&pretty, FileFormat::Json).unwrap().rows[0][0], Value::Int(1));
    }

    #[test]
    fn markdown_and_sql_outputs() {
        let ds = text(&[&["id", "name"], &["1", "O'Brien | Co"]]);
        assert_eq!(render(&ds, SaveFormat::Markdown, "").unwrap(), "| id | name |\n| --- | --- |\n| 1 | O'Brien \\| Co |\n");
        let sql = render(&ds, SaveFormat::Sql, "people").unwrap();
        assert_eq!(sql, "INSERT INTO \"people\" (\"id\", \"name\") VALUES\n('1', 'O''Brien | Co');\n");
        assert_eq!(table_from_path("/tmp/out/users.sql"), "users");
        assert_eq!(table_from_path(""), "rows");
        assert_eq!(render(&Dataset::empty(), SaveFormat::Sql, "t").unwrap(), "");
    }
}
