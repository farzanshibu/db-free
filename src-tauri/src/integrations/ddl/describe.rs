// SOT: describe-table, table-introspection, ddl-column-defaults-read, ddl-index-introspection, ddl-foreign-key-actions-read

use super::dialect::{same_name, Cx, Dialect};
use super::types::{parse_type, ParsedType};
use crate::error::AppResult;
use crate::integrations::sql::quote_literal;
use crate::integrations::Integration;
use crate::model::{
    ColumnDefault, ColumnInfo, ColumnSpec, Engine, FkAction, ForeignKey, ForeignKeySpec, IndexSpec, StatementResult, TableRef, TableSpec, Value,
};
use base64::Engine as _;

// WHAT:  Reads a live table into the `TableSpec` the Edit form loads: columns with
//        their full types, defaults, auto-increment and uniqueness, the primary
//        key, foreign keys with their actions, and secondary indexes.
// WHY:   An edit is a diff between what the table is and what the form leaves it
//        as, so "what the table is" has to include what ALTER needs to undo it: the
//        name of the constraint to drop, whether a default exists, the type with
//        its length.
// HOW:   The adapter's own `columns()` and `foreign_keys()` are the base (they
//        are tested against every engine). Catalogue queries then add what those
//        omit. Every query is best-effort: a user without rights on `pg_catalog`
//        still gets the form, with the base description. Row parsing is pure
//        (`Vec<Vec<Value>>` in, specs out) so it is unit-tested without a server.
// WHERE: src-tauri/src/integrations/ddl/mod.rs

type Rows = Vec<Vec<Value>>;

const MAX_ROWS: usize = 20_000;

// ---- reading cells ---------------------------------------------------------------------

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "1" } else { "0" }.to_string()),
        Value::Int(i) => Some(i.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Decimal(s) | Value::Text(s) | Value::DateTime(s) | Value::Unsupported(s) => Some(s.clone()),
        Value::Json(serde_json::Value::String(s)) => Some(s.clone()),
        Value::Json(other) => Some(other.to_string()),
        Value::Bytes(b64) => base64::engine::general_purpose::STANDARD.decode(b64).ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
    }
}

fn cell(row: &[Value], i: usize) -> Option<String> {
    row.get(i).and_then(text)
}

fn cell_or_empty(row: &[Value], i: usize) -> String {
    cell(row, i).unwrap_or_default()
}

fn flag(row: &[Value], i: usize) -> bool {
    match row.get(i) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Int(n)) => *n != 0,
        Some(other) => text(other).is_some_and(|t| matches!(t.trim().to_ascii_lowercase().as_str(), "1" | "t" | "true" | "y" | "yes")),
        None => false,
    }
}

fn number(row: &[Value], i: usize) -> Option<i64> {
    match row.get(i)? {
        Value::Int(n) => Some(*n),
        other => text(other)?.trim().parse().ok(),
    }
}

/// A list cell: a JSON array, or text such as `[a, b]` / `{a,b}`.
fn list(value: &Value) -> Vec<String> {
    match value {
        Value::Json(serde_json::Value::Array(items)) => items.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        other => text(other)
            .map(|t| {
                t.trim()
                    .trim_matches(|c| matches!(c, '[' | ']' | '{' | '}'))
                    .split(',')
                    .map(|p| p.trim().trim_matches(|c| matches!(c, '\'' | '"')).to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn lit(raw: &str) -> String {
    quote_literal(raw)
}

fn schema_of(table: &TableRef) -> Option<&str> {
    table.schema.as_deref().filter(|s| !s.is_empty())
}

// ---- defaults ----------------------------------------------------------------------------

fn strip_outer_parens(text: &str) -> &str {
    let mut t = text.trim();
    while t.starts_with('(') && t.ends_with(')') {
        let mut depth = 0_i32;
        let mut closes_at_end = true;
        for (i, ch) in t.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && i + 1 != t.len() {
                        closes_at_end = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !closes_at_end {
            break;
        }
        t = t[1..t.len() - 1].trim();
    }
    t
}

/// `'abc'::character varying` -> `'abc'`: PostgreSQL appends a cast to literals.
fn strip_literal_cast(text: &str) -> &str {
    let t = text.trim();
    if !t.starts_with('\'') {
        return t;
    }
    let bytes = t.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                i += 2;
                continue;
            }
            let rest = &t[i + 1..];
            return if rest.starts_with("::") { &t[..=i] } else { t };
        }
        i += 1;
    }
    t
}

fn is_now(upper: &str) -> bool {
    matches!(
        upper,
        "NOW()" | "CURRENT_TIMESTAMP" | "CURRENT_TIMESTAMP()" | "GETDATE()" | "SYSDATE" | "LOCALTIMESTAMP" | "TRANSACTION_TIMESTAMP()" | "STATEMENT_TIMESTAMP()"
    ) || (upper.starts_with("CURRENT_TIMESTAMP(") && upper.ends_with(')') && upper[18..upper.len() - 1].chars().all(|c| c.is_ascii_digit()))
}

/// A default as the catalogue prints it, back to the form's four presets.
pub fn parse_default(dialect: Dialect, raw: &str) -> ColumnDefault {
    let mut t = raw.trim();
    if dialect == Dialect::Mssql {
        t = strip_outer_parens(t);
    }
    if dialect == Dialect::Postgres {
        t = strip_literal_cast(t);
    }
    if t.is_empty() {
        return ColumnDefault::None;
    }
    let upper = t.to_ascii_uppercase();
    if upper == "NULL" || upper.starts_with("NULL::") {
        return ColumnDefault::Null;
    }
    if is_now(&upper) {
        return ColumnDefault::Now;
    }
    if t == "''" || (dialect == Dialect::Mssql && t == "N''") {
        return ColumnDefault::Empty;
    }
    ColumnDefault::Custom { expression: t.to_string() }
}

// ---- what the catalogue queries return ------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ColExtra {
    name: String,
    full_type: Option<String>,
    default: Option<ColumnDefault>,
    auto: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexRow {
    name: String,
    unique: bool,
    primary: bool,
    /// None for an expression column, which the form cannot represent.
    column: Option<String>,
    skip: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Extras {
    columns: Vec<ColExtra>,
    indexes: Vec<IndexRow>,
    foreign_keys: Option<Vec<ForeignKeySpec>>,
    primary_key_name: Option<String>,
    engine: Option<String>,
    order_by: Vec<String>,
}

fn find_extra<'a>(extras: &'a [ColExtra], name: &str) -> Option<&'a ColExtra> {
    extras.iter().find(|e| e.name == name)
}

/// Where each part of a foreign key sits in a catalogue row.
struct FkColumns {
    name: usize,
    from: usize,
    to: usize,
    ref_schema: usize,
    ref_table: usize,
    update: usize,
    delete: usize,
}

/// The layout every server-side foreign-key query here shares.
const FK_ROW: FkColumns = FkColumns { name: 0, from: 1, to: 2, ref_schema: 3, ref_table: 4, update: 5, delete: 6 };

/// Rows ordered by constraint then position -> one `ForeignKeySpec` per constraint.
fn group_foreign_keys(rows: &[Vec<Value>], at: &FkColumns) -> Vec<ForeignKeySpec> {
    let mut out: Vec<ForeignKeySpec> = Vec::new();
    for row in rows {
        let constraint = cell_or_empty(row, at.name);
        let existing = out.iter_mut().find(|f| f.original_name.as_deref() == Some(constraint.as_str()));
        match existing {
            Some(fk) => {
                fk.columns.push(cell_or_empty(row, at.from));
                fk.ref_columns.push(cell_or_empty(row, at.to));
            }
            None => out.push(ForeignKeySpec {
                name: Some(constraint.clone()),
                original_name: Some(constraint),
                columns: vec![cell_or_empty(row, at.from)],
                ref_table: TableRef { schema: cell(row, at.ref_schema).filter(|s| !s.is_empty()), name: cell_or_empty(row, at.ref_table) },
                ref_columns: vec![cell_or_empty(row, at.to)],
                on_update: FkAction::parse(&cell_or_empty(row, at.update)),
                on_delete: FkAction::parse(&cell_or_empty(row, at.delete)),
            }),
        }
    }
    out
}

// ---- PostgreSQL ------------------------------------------------------------------------------

fn pg_scope(table: &TableRef) -> String {
    let schema = schema_of(table).map_or_else(|| "n.nspname = current_schema()".to_string(), |s| format!("n.nspname = {}", lit(s)));
    format!("c.relname = {} AND {schema}", lit(&table.name))
}

fn pg_columns_sql(table: &TableRef) -> String {
    format!(
        "SELECT a.attname, format_type(a.atttypid, a.atttypmod), pg_get_expr(d.adbin, d.adrelid), a.attidentity::text \
         FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
         LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
         WHERE {} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
        pg_scope(table)
    )
}

fn pg_indexes_sql(table: &TableRef) -> String {
    format!(
        "SELECT i.relname, ix.indisunique::int, ix.indisprimary::int, a.attname, k.ord::int, (ix.indpred IS NOT NULL OR ix.indexprs IS NOT NULL)::int \
         FROM pg_index ix JOIN pg_class i ON i.oid = ix.indexrelid JOIN pg_class c ON c.oid = ix.indrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
         CROSS JOIN LATERAL unnest(ix.indkey::int2[]) WITH ORDINALITY AS k(attnum, ord) \
         LEFT JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = k.attnum \
         WHERE {} AND k.ord <= ix.indnkeyatts ORDER BY i.relname, k.ord",
        pg_scope(table)
    )
}

fn pg_primary_key_sql(table: &TableRef) -> String {
    format!(
        "SELECT con.conname FROM pg_constraint con JOIN pg_class c ON c.oid = con.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE con.contype = 'p' AND {}",
        pg_scope(table)
    )
}

fn pg_foreign_keys_sql(table: &TableRef) -> String {
    format!(
        "SELECT con.conname, a.attname, af.attname, nf.nspname, cf.relname, con.confupdtype::text, con.confdeltype::text \
         FROM pg_constraint con JOIN pg_class c ON c.oid = con.conrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
         JOIN pg_class cf ON cf.oid = con.confrelid JOIN pg_namespace nf ON nf.oid = cf.relnamespace \
         CROSS JOIN LATERAL unnest(con.conkey, con.confkey) WITH ORDINALITY AS k(from_att, to_att, ord) \
         JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.from_att \
         JOIN pg_attribute af ON af.attrelid = con.confrelid AND af.attnum = k.to_att \
         WHERE con.contype = 'f' AND {} ORDER BY con.conname, k.ord",
        pg_scope(table)
    )
}

fn pg_columns(rows: &Rows) -> Vec<ColExtra> {
    rows.iter()
        .map(|row| {
            let default = cell(row, 2);
            let serial = default.as_deref().is_some_and(|d| d.trim_start().to_ascii_lowercase().starts_with("nextval("));
            let identity = matches!(cell_or_empty(row, 3).trim(), "a" | "d");
            let auto = serial || identity;
            ColExtra {
                name: cell_or_empty(row, 0),
                full_type: cell(row, 1),
                default: if auto { Some(ColumnDefault::None) } else { default.map(|d| parse_default(Dialect::Postgres, &d)) },
                auto,
            }
        })
        .collect()
}

fn pg_indexes(rows: &Rows) -> Vec<IndexRow> {
    rows.iter()
        .map(|row| IndexRow { name: cell_or_empty(row, 0), unique: flag(row, 1), primary: flag(row, 2), column: cell(row, 3), skip: flag(row, 5) })
        .collect()
}

// ---- MySQL / MariaDB -----------------------------------------------------------------------

fn mysql_db(table: &TableRef) -> String {
    schema_of(table).map_or_else(|| "DATABASE()".to_string(), lit)
}

fn mysql_columns_sql(table: &TableRef) -> String {
    format!(
        "SELECT COLUMN_NAME, COLUMN_TYPE, COLUMN_DEFAULT, EXTRA FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = {} AND TABLE_NAME = {} ORDER BY ORDINAL_POSITION",
        mysql_db(table),
        lit(&table.name)
    )
}

fn mysql_indexes_sql(table: &TableRef) -> String {
    format!(
        "SELECT INDEX_NAME, NON_UNIQUE, COLUMN_NAME, SEQ_IN_INDEX, SUB_PART FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = {} AND TABLE_NAME = {} ORDER BY INDEX_NAME, SEQ_IN_INDEX",
        mysql_db(table),
        lit(&table.name)
    )
}

fn mysql_foreign_keys_sql(table: &TableRef) -> String {
    format!(
        "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_COLUMN_NAME, k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, r.UPDATE_RULE, r.DELETE_RULE \
         FROM information_schema.KEY_COLUMN_USAGE k \
         JOIN information_schema.REFERENTIAL_CONSTRAINTS r ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME \
         WHERE k.TABLE_SCHEMA = {} AND k.TABLE_NAME = {} AND k.REFERENCED_TABLE_NAME IS NOT NULL ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
        mysql_db(table),
        lit(&table.name)
    )
}

fn is_number(text: &str) -> bool {
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    !digits.is_empty() && digits.chars().filter(|c| *c == '.').count() <= 1 && digits.chars().all(|c| c.is_ascii_digit() || c == '.') && digits.chars().any(|c| c.is_ascii_digit())
}

/// MySQL 8 prints a literal default bare and an expression default (flagged in
/// EXTRA) unparenthesised; MariaDB prints both the way they must be written back.
fn mysql_default(engine: Engine, raw: &str, extra: &str) -> ColumnDefault {
    if engine == Engine::Mariadb {
        return parse_default(Dialect::Mysql, raw);
    }
    if extra.to_ascii_uppercase().contains("DEFAULT_GENERATED") {
        let upper = raw.trim().to_ascii_uppercase();
        if is_now(&upper) {
            return ColumnDefault::Now;
        }
        return ColumnDefault::Custom { expression: format!("({})", raw.trim()) };
    }
    if raw.is_empty() {
        return ColumnDefault::Empty;
    }
    if is_number(raw) {
        return ColumnDefault::Custom { expression: raw.to_string() };
    }
    ColumnDefault::Custom { expression: lit(raw) }
}

fn mysql_columns(engine: Engine, rows: &Rows) -> Vec<ColExtra> {
    rows.iter()
        .map(|row| {
            let extra = cell_or_empty(row, 3);
            ColExtra {
                name: cell_or_empty(row, 0),
                full_type: cell(row, 1),
                default: Some(cell(row, 2).map_or(ColumnDefault::None, |raw| mysql_default(engine, &raw, &extra))),
                auto: extra.to_ascii_lowercase().contains("auto_increment"),
            }
        })
        .collect()
}

fn mysql_indexes(rows: &Rows) -> Vec<IndexRow> {
    rows.iter()
        .map(|row| {
            let name = cell_or_empty(row, 0);
            IndexRow {
                primary: name == "PRIMARY",
                unique: !flag(row, 1),
                column: cell(row, 2),
                // A prefix index (`name(10)`) cannot be written back without its length.
                skip: cell(row, 4).is_some(),
                name,
            }
        })
        .collect()
}

// ---- SQLite family ---------------------------------------------------------------------------

fn sqlite_target(table: &TableRef, pragma: &str, argument: &str) -> String {
    match schema_of(table) {
        Some(schema) => format!("PRAGMA \"{}\".{pragma}(\"{}\")", schema.replace('"', "\"\""), argument.replace('"', "\"\"")),
        None => format!("PRAGMA {pragma}(\"{}\")", argument.replace('"', "\"\"")),
    }
}

fn sqlite_master_sql(table: &TableRef) -> String {
    let master = schema_of(table).map_or_else(|| "sqlite_master".to_string(), |s| format!("\"{}\".sqlite_master", s.replace('"', "\"\"")));
    format!("SELECT sql FROM {master} WHERE type = 'table' AND name = {}", lit(&table.name))
}

/// `table_info` rows (cid, name, type, notnull, dflt_value, pk) plus the CREATE text.
fn sqlite_columns(rows: &Rows, create_sql: &str) -> Vec<ColExtra> {
    let pk_count = rows.iter().filter(|r| number(r, 5).unwrap_or(0) > 0).count();
    let autoincrement = create_sql.to_ascii_uppercase().contains("AUTOINCREMENT");
    rows.iter()
        .map(|row| ColExtra {
            name: cell_or_empty(row, 1),
            full_type: cell(row, 2).filter(|t| !t.is_empty()),
            default: Some(cell(row, 4).map_or(ColumnDefault::None, |d| parse_default(Dialect::Sqlite, &d))),
            auto: autoincrement && pk_count == 1 && number(row, 5).unwrap_or(0) > 0,
        })
        .collect()
}

/// The parent column a foreign key on `from` names, when SQLite left `to` NULL (the
/// parent's primary key) and the adapter's own foreign-key list resolved it.
fn resolved_parent_column(base: &[ForeignKeySpec], from: &str) -> Option<String> {
    base.iter().find_map(|fk| fk.columns.iter().position(|c| c == from).and_then(|at| fk.ref_columns.get(at).cloned()))
}

/// `foreign_key_list` rows: id, seq, table, from, to, on_update, on_delete.
fn sqlite_foreign_keys(rows: &Rows, base: &[ForeignKeySpec]) -> Vec<ForeignKeySpec> {
    let mut grouped: Vec<(i64, ForeignKeySpec)> = Vec::new();
    for row in rows {
        let id = number(row, 0).unwrap_or(0);
        let from = cell_or_empty(row, 3);
        let to = cell(row, 4).or_else(|| resolved_parent_column(base, &from)).unwrap_or_default();
        match grouped.iter_mut().find(|(i, _)| *i == id) {
            Some((_, fk)) => {
                fk.columns.push(from);
                fk.ref_columns.push(to);
            }
            None => grouped.push((
                id,
                ForeignKeySpec {
                    name: None,
                    original_name: Some(format!("fk_{id}")),
                    columns: vec![from],
                    ref_table: TableRef { schema: None, name: cell_or_empty(row, 2) },
                    ref_columns: vec![to],
                    on_update: FkAction::parse(&cell_or_empty(row, 5)),
                    on_delete: FkAction::parse(&cell_or_empty(row, 6)),
                },
            )),
        }
    }
    grouped.into_iter().map(|(_, fk)| fk).filter(|fk| fk.ref_columns.iter().all(|c| !c.is_empty())).collect()
}

// ---- SQL Server --------------------------------------------------------------------------------

fn mssql_object(table: &TableRef) -> String {
    let qualified = match schema_of(table) {
        Some(schema) => format!("[{}].[{}]", schema.replace(']', "]]"), table.name.replace(']', "]]")),
        None => format!("[{}]", table.name.replace(']', "]]")),
    };
    format!("OBJECT_ID(N{})", lit(&qualified))
}

fn mssql_columns_sql(table: &TableRef) -> String {
    format!(
        "SELECT c.name, ty.name, c.max_length, c.precision, c.scale, c.is_identity, dc.definition, c.is_nullable \
         FROM sys.columns c JOIN sys.types ty ON ty.user_type_id = c.user_type_id \
         LEFT JOIN sys.default_constraints dc ON dc.object_id = c.default_object_id \
         WHERE c.object_id = {} ORDER BY c.column_id",
        mssql_object(table)
    )
}

fn mssql_indexes_sql(table: &TableRef) -> String {
    format!(
        "SELECT i.name, i.is_unique, i.is_primary_key, c.name, ic.key_ordinal, i.has_filter \
         FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
         JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
         WHERE i.object_id = {} AND i.type > 0 AND ic.is_included_column = 0 ORDER BY i.name, ic.key_ordinal",
        mssql_object(table)
    )
}

fn mssql_foreign_keys_sql(table: &TableRef) -> String {
    format!(
        "SELECT fk.name, pc.name, rc.name, rs.name, rt.name, fk.update_referential_action_desc, fk.delete_referential_action_desc \
         FROM sys.foreign_keys fk JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
         JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id \
         JOIN sys.tables rt ON rt.object_id = fkc.referenced_object_id JOIN sys.schemas rs ON rs.schema_id = rt.schema_id \
         JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id \
         WHERE fk.parent_object_id = {} ORDER BY fk.name, fkc.constraint_column_id",
        mssql_object(table)
    )
}

fn mssql_type(name: &str, max_length: i64, precision: i64, scale: i64) -> String {
    match name.to_ascii_lowercase().as_str() {
        "varchar" | "char" | "varbinary" | "binary" => {
            if max_length < 0 { format!("{name}(max)") } else { format!("{name}({max_length})") }
        }
        "nvarchar" | "nchar" => {
            if max_length < 0 { format!("{name}(max)") } else { format!("{name}({})", max_length / 2) }
        }
        "decimal" | "numeric" => format!("{name}({precision},{scale})"),
        "datetime2" | "datetimeoffset" | "time" if scale != 7 => format!("{name}({scale})"),
        "float" if precision != 53 => format!("float({precision})"),
        _ => name.to_string(),
    }
}

fn mssql_columns(rows: &Rows) -> Vec<ColExtra> {
    rows.iter()
        .map(|row| ColExtra {
            name: cell_or_empty(row, 0),
            full_type: Some(mssql_type(&cell_or_empty(row, 1), number(row, 2).unwrap_or(0), number(row, 3).unwrap_or(0), number(row, 4).unwrap_or(0))),
            default: Some(cell(row, 6).map_or(ColumnDefault::None, |d| parse_default(Dialect::Mssql, &d))),
            auto: flag(row, 5),
        })
        .collect()
}

fn mssql_indexes(rows: &Rows) -> Vec<IndexRow> {
    rows.iter()
        .map(|row| IndexRow { name: cell_or_empty(row, 0), unique: flag(row, 1), primary: flag(row, 2), column: cell(row, 3), skip: flag(row, 5) })
        .collect()
}

// ---- ClickHouse ----------------------------------------------------------------------------------

fn clickhouse_db(table: &TableRef) -> String {
    schema_of(table).map_or_else(|| "currentDatabase()".to_string(), lit)
}

fn clickhouse_columns_sql(table: &TableRef) -> String {
    format!(
        "SELECT name, type, default_kind, default_expression FROM system.columns WHERE database = {} AND table = {} ORDER BY position",
        clickhouse_db(table),
        lit(&table.name)
    )
}

fn clickhouse_table_sql(table: &TableRef) -> String {
    format!("SELECT engine, sorting_key FROM system.tables WHERE database = {} AND name = {}", clickhouse_db(table), lit(&table.name))
}

/// Splits `a, b, f(x, y)` at the top-level commas.
fn split_top_level(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0_i32;
    let mut current = String::new();
    for ch in text.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

fn clickhouse_columns(rows: &Rows) -> Vec<ColExtra> {
    rows.iter()
        .map(|row| {
            let kind = cell_or_empty(row, 2);
            let default = if kind.eq_ignore_ascii_case("DEFAULT") { cell(row, 3).map(|d| parse_default(Dialect::Clickhouse, &d)) } else { None };
            ColExtra { name: cell_or_empty(row, 0), full_type: cell(row, 1), default: Some(default.unwrap_or(ColumnDefault::None)), auto: false }
        })
        .collect()
}

// ---- information_schema engines ------------------------------------------------------------

fn duckdb_columns_sql(table: &TableRef) -> String {
    let schema = schema_of(table).map_or_else(|| "current_schema()".to_string(), lit);
    format!("SELECT column_name, data_type, column_default FROM information_schema.columns WHERE table_name = {} AND table_schema = {schema} ORDER BY ordinal_position", lit(&table.name))
}

fn duckdb_unique_sql(table: &TableRef) -> String {
    let schema = schema_of(table).map_or_else(|| "current_schema()".to_string(), lit);
    format!("SELECT constraint_column_names FROM duckdb_constraints() WHERE table_name = {} AND schema_name = {schema} AND constraint_type = 'UNIQUE'", lit(&table.name))
}

fn duckdb_indexes_sql(table: &TableRef) -> String {
    let schema = schema_of(table).map_or_else(|| "current_schema()".to_string(), lit);
    format!("SELECT index_name, is_unique, expressions FROM duckdb_indexes() WHERE table_name = {} AND schema_name = {schema}", lit(&table.name))
}

fn duckdb_columns(rows: &Rows) -> Vec<ColExtra> {
    rows.iter()
        .map(|row| {
            let default = cell(row, 2);
            let auto = default.as_deref().is_some_and(|d| d.trim_start().to_ascii_lowercase().starts_with("nextval("));
            ColExtra {
                name: cell_or_empty(row, 0),
                full_type: cell(row, 1),
                default: Some(if auto { ColumnDefault::None } else { default.map_or(ColumnDefault::None, |d| parse_default(Dialect::Duckdb, &d)) }),
                auto,
            }
        })
        .collect()
}

fn snowflake_columns_sql(table: &TableRef) -> String {
    let schema = schema_of(table).map_or_else(|| "CURRENT_SCHEMA()".to_string(), lit);
    format!(
        "SELECT COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, NUMERIC_PRECISION, NUMERIC_SCALE, COLUMN_DEFAULT, IS_IDENTITY \
         FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = {schema} AND TABLE_NAME = {} ORDER BY ORDINAL_POSITION",
        lit(&table.name)
    )
}

fn snowflake_columns(rows: &Rows) -> Vec<ColExtra> {
    rows.iter()
        .map(|row| {
            let ty = cell_or_empty(row, 1).to_ascii_uppercase();
            let full = match ty.as_str() {
                "TEXT" => match number(row, 2) {
                    Some(len) if len < 16_777_216 => format!("VARCHAR({len})"),
                    _ => "VARCHAR".to_string(),
                },
                "NUMBER" => format!("NUMBER({},{})", number(row, 3).unwrap_or(38), number(row, 4).unwrap_or(0)),
                other => other.to_string(),
            };
            let identity = cell_or_empty(row, 6).eq_ignore_ascii_case("YES");
            ColExtra {
                name: cell_or_empty(row, 0),
                full_type: Some(full),
                default: Some(if identity { ColumnDefault::None } else { cell(row, 5).map_or(ColumnDefault::None, |d| parse_default(Dialect::Snowflake, &d)) }),
                auto: identity,
            }
        })
        .collect()
}

fn bigquery_columns_sql(dataset: &str, table: &TableRef) -> String {
    format!(
        "SELECT column_name, data_type, column_default FROM `{}`.INFORMATION_SCHEMA.COLUMNS WHERE table_name = {} ORDER BY ordinal_position",
        dataset.replace('`', "\\`"),
        lit(&table.name)
    )
}

fn bigquery_columns(rows: &Rows) -> Vec<ColExtra> {
    rows.iter()
        .map(|row| ColExtra {
            name: cell_or_empty(row, 0),
            full_type: cell(row, 1),
            default: Some(cell(row, 2).filter(|d| !d.eq_ignore_ascii_case("NULL")).map_or(ColumnDefault::None, |d| parse_default(Dialect::Bigquery, &d))),
            auto: false,
        })
        .collect()
}

fn oracle_owner(table: &TableRef) -> String {
    schema_of(table).map_or_else(|| "SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA')".to_string(), lit)
}

fn oracle_columns_sql(table: &TableRef) -> String {
    format!(
        "SELECT column_name, data_type, data_length, data_precision, data_scale, char_length, data_default, identity_column \
         FROM all_tab_columns WHERE owner = {} AND table_name = {} ORDER BY column_id",
        oracle_owner(table),
        lit(&table.name)
    )
}

fn oracle_indexes_sql(table: &TableRef) -> String {
    format!(
        "SELECT i.index_name, i.uniqueness, c.column_name, c.column_position FROM all_indexes i \
         JOIN all_ind_columns c ON c.index_owner = i.owner AND c.index_name = i.index_name \
         WHERE i.table_owner = {owner} AND i.table_name = {name} AND i.index_type = 'NORMAL' \
         AND i.index_name NOT IN (SELECT index_name FROM all_constraints WHERE owner = {owner} AND table_name = {name} AND constraint_type = 'P' AND index_name IS NOT NULL) \
         ORDER BY i.index_name, c.column_position",
        owner = oracle_owner(table),
        name = lit(&table.name)
    )
}

fn oracle_primary_key_sql(table: &TableRef) -> String {
    format!("SELECT constraint_name FROM all_constraints WHERE constraint_type = 'P' AND owner = {} AND table_name = {}", oracle_owner(table), lit(&table.name))
}

fn oracle_columns(rows: &Rows) -> Vec<ColExtra> {
    rows.iter()
        .map(|row| {
            let ty = cell_or_empty(row, 1);
            let upper = ty.to_ascii_uppercase();
            let full = match upper.as_str() {
                "VARCHAR2" | "NVARCHAR2" | "CHAR" | "NCHAR" => format!("{upper}({})", number(row, 5).filter(|n| *n > 0).or_else(|| number(row, 2)).unwrap_or(1)),
                "RAW" => format!("RAW({})", number(row, 2).unwrap_or(1)),
                "NUMBER" => match (number(row, 3), number(row, 4)) {
                    (Some(p), Some(s)) if s != 0 => format!("NUMBER({p},{s})"),
                    (Some(p), _) => format!("NUMBER({p})"),
                    _ => "NUMBER".to_string(),
                },
                _ => ty,
            };
            let auto = cell_or_empty(row, 7).eq_ignore_ascii_case("YES");
            ColExtra {
                name: cell_or_empty(row, 0),
                full_type: Some(full),
                default: Some(if auto { ColumnDefault::None } else { cell(row, 6).map(|d| d.trim().to_string()).filter(|d| !d.is_empty()).map_or(ColumnDefault::None, |d| parse_default(Dialect::Oracle, &d)) }),
                auto,
            }
        })
        .collect()
}

fn oracle_indexes(rows: &Rows) -> Vec<IndexRow> {
    rows.iter()
        .map(|row| IndexRow {
            name: cell_or_empty(row, 0),
            unique: cell_or_empty(row, 1).eq_ignore_ascii_case("UNIQUE"),
            primary: false,
            column: cell(row, 2),
            skip: false,
        })
        .collect()
}

// ---- gathering ---------------------------------------------------------------------------------

async fn ask<I: Integration + ?Sized>(integration: &I, sql: &str) -> Option<Rows> {
    match integration.execute(sql, MAX_ROWS).await {
        Ok(results) => results.into_iter().find_map(|r| match r {
            StatementResult::Rows { result } => Some(result.rows),
            StatementResult::Affected { .. } => None,
        }),
        Err(err) => {
            log::debug!("describe: catalogue query failed ({err}): {sql}");
            None
        }
    }
}

async fn gather<I: Integration + ?Sized>(integration: &I, cx: &Cx, table: &TableRef, base_fks: &[ForeignKeySpec]) -> Extras {
    let mut extras = Extras::default();
    match cx.dialect {
        Dialect::Postgres => {
            if let Some(rows) = ask(integration, &pg_columns_sql(table)).await {
                extras.columns = pg_columns(&rows);
            }
            if let Some(rows) = ask(integration, &pg_indexes_sql(table)).await {
                extras.indexes = pg_indexes(&rows);
            }
            extras.primary_key_name = ask(integration, &pg_primary_key_sql(table)).await.and_then(|r| r.first().and_then(|row| cell(row, 0)));
            if let Some(rows) = ask(integration, &pg_foreign_keys_sql(table)).await {
                extras.foreign_keys = Some(group_foreign_keys(&rows, &FK_ROW));
            }
        }
        Dialect::Mysql => {
            if let Some(rows) = ask(integration, &mysql_columns_sql(table)).await {
                extras.columns = mysql_columns(cx.engine, &rows);
            }
            if let Some(rows) = ask(integration, &mysql_indexes_sql(table)).await {
                extras.indexes = mysql_indexes(&rows);
            }
            if let Some(rows) = ask(integration, &mysql_foreign_keys_sql(table)).await {
                extras.foreign_keys = Some(group_foreign_keys(&rows, &FK_ROW));
            }
        }
        Dialect::Sqlite => {
            let create_sql = ask(integration, &sqlite_master_sql(table)).await.and_then(|r| r.first().and_then(|row| cell(row, 0))).unwrap_or_default();
            if let Some(rows) = ask(integration, &sqlite_target(table, "table_info", &table.name)).await {
                extras.columns = sqlite_columns(&rows, &create_sql);
            }
            if let Some(rows) = ask(integration, &sqlite_target(table, "foreign_key_list", &table.name)).await {
                extras.foreign_keys = Some(sqlite_foreign_keys(&rows, base_fks));
            }
            if let Some(list_rows) = ask(integration, &sqlite_target(table, "index_list", &table.name)).await {
                // index_list: seq, name, unique, origin, partial
                for row in &list_rows {
                    let name = cell_or_empty(row, 1);
                    let origin = cell_or_empty(row, 3);
                    let partial = flag(row, 4);
                    let Some(columns) = ask(integration, &sqlite_target(table, "index_info", &name)).await else {
                        continue;
                    };
                    // index_info: seqno, cid, name (NULL for an expression)
                    for column in &columns {
                        extras.indexes.push(IndexRow {
                            name: name.clone(),
                            unique: flag(row, 2),
                            primary: origin == "pk",
                            column: cell(column, 2),
                            skip: partial,
                        });
                    }
                }
            }
        }
        Dialect::Mssql => {
            if let Some(rows) = ask(integration, &mssql_columns_sql(table)).await {
                extras.columns = mssql_columns(&rows);
            }
            if let Some(rows) = ask(integration, &mssql_indexes_sql(table)).await {
                extras.indexes = mssql_indexes(&rows);
                extras.primary_key_name = rows.iter().find(|r| flag(r, 2)).and_then(|r| cell(r, 0));
            }
            if let Some(rows) = ask(integration, &mssql_foreign_keys_sql(table)).await {
                extras.foreign_keys = Some(group_foreign_keys(&rows, &FK_ROW));
            }
        }
        Dialect::Clickhouse => {
            if let Some(rows) = ask(integration, &clickhouse_columns_sql(table)).await {
                extras.columns = clickhouse_columns(&rows);
            }
            if let Some(rows) = ask(integration, &clickhouse_table_sql(table)).await {
                if let Some(row) = rows.first() {
                    extras.engine = cell(row, 0).filter(|e| !e.is_empty());
                    let keys = split_top_level(&cell_or_empty(row, 1));
                    if keys.iter().all(|k| !k.contains('(')) {
                        extras.order_by = keys;
                    }
                }
            }
        }
        Dialect::Duckdb => {
            if let Some(rows) = ask(integration, &duckdb_columns_sql(table)).await {
                extras.columns = duckdb_columns(&rows);
            }
            if let Some(rows) = ask(integration, &duckdb_unique_sql(table)).await {
                for row in &rows {
                    let columns = row.first().map(list).unwrap_or_default();
                    for column in columns {
                        extras.indexes.push(IndexRow { name: format!("{}_{}_key", table.name, column), unique: true, primary: false, column: Some(column), skip: false });
                    }
                }
            }
            if let Some(rows) = ask(integration, &duckdb_indexes_sql(table)).await {
                for row in &rows {
                    let name = cell_or_empty(row, 0);
                    let expressions = row.get(2).map(list).unwrap_or_default();
                    let skip = expressions.iter().any(|e| e.contains('('));
                    for column in expressions {
                        extras.indexes.push(IndexRow { name: name.clone(), unique: flag(row, 1), primary: false, column: Some(column), skip });
                    }
                }
            }
        }
        Dialect::Snowflake => {
            if let Some(rows) = ask(integration, &snowflake_columns_sql(table)).await {
                extras.columns = snowflake_columns(&rows);
            }
        }
        Dialect::Bigquery => {
            if let Some(dataset) = schema_of(table) {
                if let Some(rows) = ask(integration, &bigquery_columns_sql(dataset, table)).await {
                    extras.columns = bigquery_columns(&rows);
                }
            }
        }
        Dialect::Oracle => {
            if let Some(rows) = ask(integration, &oracle_columns_sql(table)).await {
                extras.columns = oracle_columns(&rows);
            }
            if let Some(rows) = ask(integration, &oracle_indexes_sql(table)).await {
                extras.indexes = oracle_indexes(&rows);
            }
            extras.primary_key_name = ask(integration, &oracle_primary_key_sql(table)).await.and_then(|r| r.first().and_then(|row| cell(row, 0)));
        }
        Dialect::Cassandra | Dialect::Mongo => {}
    }
    extras
}

// ---- assembling the spec -----------------------------------------------------------------------

fn apply_parsed(column: &mut ColumnSpec, parsed: ParsedType, dialect: Dialect, reported_nullable: bool) {
    column.data_type = parsed.base;
    column.length = parsed.length;
    column.precision = parsed.precision;
    column.scale = parsed.scale;
    column.unsigned = parsed.unsigned;
    column.enum_values = parsed.enum_values;
    column.wrapper = parsed.wrapper;
    // ClickHouse writes nullability into the type; elsewhere the catalogue says.
    column.nullable = if dialect == Dialect::Clickhouse { parsed.nullable } else { reported_nullable };
}

fn base_columns(dialect: Dialect, infos: &[ColumnInfo]) -> Vec<ColumnSpec> {
    infos
        .iter()
        .map(|info| {
            let mut column = ColumnSpec {
                name: info.name.clone(),
                original_name: Some(info.name.clone()),
                data_type: String::new(),
                length: None,
                precision: None,
                scale: None,
                wrapper: None,
                enum_values: Vec::new(),
                nullable: info.nullable,
                primary_key: info.primary_key,
                unique: false,
                unique_name: None,
                auto_increment: false,
                unsigned: false,
                default: ColumnDefault::None,
            };
            apply_parsed(&mut column, parse_type(dialect, &info.data_type), dialect, info.nullable);
            column
        })
        .collect()
}

fn base_foreign_keys(table: &TableRef, all: &[ForeignKey]) -> Vec<ForeignKeySpec> {
    all.iter()
        .filter(|fk| same_name(&fk.from_table, &table.name) && (schema_of(table).is_none() || fk.from_schema.as_deref().filter(|s| !s.is_empty()) == schema_of(table)))
        .map(|fk| ForeignKeySpec {
            name: Some(fk.name.clone()),
            original_name: Some(fk.name.clone()),
            columns: fk.from_columns.clone(),
            ref_table: TableRef { schema: fk.to_schema.clone().filter(|s| !s.is_empty()), name: fk.to_table.clone() },
            ref_columns: fk.to_columns.clone(),
            on_delete: FkAction::NoAction,
            on_update: FkAction::NoAction,
        })
        .collect()
}

/// Folds the catalogue extras into the base description.
fn merge(cx: &Cx, spec: &mut TableSpec, extras: Extras) {
    for column in &mut spec.columns {
        let Some(extra) = find_extra(&extras.columns, &column.name) else {
            continue;
        };
        if let Some(full) = extra.full_type.as_deref() {
            let reported = column.nullable;
            apply_parsed(column, parse_type(cx.dialect, full), cx.dialect, reported);
        }
        if let Some(default) = &extra.default {
            column.default = default.clone();
        }
        column.auto_increment = extra.auto;
    }

    // Single-column unique indexes become the column's UNIQUE flag; every other
    // index stays an index.
    let mut names: Vec<String> = Vec::new();
    for row in &extras.indexes {
        if !names.contains(&row.name) {
            names.push(row.name.clone());
        }
    }
    for name in names {
        let rows: Vec<&IndexRow> = extras.indexes.iter().filter(|r| r.name == name).collect();
        if rows.iter().any(|r| r.primary) {
            continue;
        }
        let columns: Vec<String> = rows.iter().filter_map(|r| r.column.clone()).collect();
        if columns.len() != rows.len() || rows.iter().any(|r| r.skip) {
            continue;
        }
        let unique = rows.iter().any(|r| r.unique);
        if unique && columns.len() == 1 {
            if let Some(column) = spec.columns.iter_mut().find(|c| c.name == columns[0] && !c.unique) {
                column.unique = true;
                column.unique_name = Some(name);
                continue;
            }
        }
        spec.indexes.push(IndexSpec { name, columns, unique });
    }

    if let Some(fks) = extras.foreign_keys {
        spec.foreign_keys = fks;
    }
    spec.primary_key_name = extras.primary_key_name;
    if cx.dialect == Dialect::Clickhouse {
        spec.engine = extras.engine;
        spec.order_by = extras.order_by;
    }
}

pub async fn describe<I: Integration + ?Sized>(integration: &I, table: &TableRef) -> AppResult<TableSpec> {
    let cx = Cx::new(integration.engine())?;
    let mut spec = TableSpec {
        table: table.clone(),
        columns: Vec::new(),
        primary_key_name: None,
        foreign_keys: Vec::new(),
        indexes: Vec::new(),
        engine: None,
        order_by: Vec::new(),
        if_not_exists: false,
    };
    // A collection has no columns to describe.
    if cx.dialect == Dialect::Mongo {
        return Ok(spec);
    }
    spec.columns = base_columns(cx.dialect, &integration.columns(table).await?);

    // The adapter's foreign-key list is loaded up front only where it is the
    // answer (or, for SQLite, fills in the parent column PRAGMA leaves implicit).
    // Where a catalogue query answers with constraint names and actions, the
    // whole-database list is only fetched if that query failed.
    let adapter_first = !matches!(cx.dialect, Dialect::Postgres | Dialect::Mysql | Dialect::Mssql);
    let adapter_fks = if adapter_first { Some(base_foreign_keys(table, &integration.foreign_keys().await.unwrap_or_default())) } else { None };
    let extras = gather(integration, &cx, table, adapter_fks.as_deref().unwrap_or(&[])).await;
    let enriched = extras.foreign_keys.is_some();
    merge(&cx, &mut spec, extras);
    if !enriched {
        spec.foreign_keys = match adapter_fks {
            Some(fks) => fks,
            None => base_foreign_keys(table, &integration.foreign_keys().await.unwrap_or_default()),
        };
    }
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    fn i(n: i64) -> Value {
        Value::Int(n)
    }

    fn users() -> TableRef {
        TableRef { schema: Some("app".into()), name: "us'ers".into() }
    }

    #[test]
    fn defaults_map_back_to_the_forms_presets() {
        assert_eq!(parse_default(Dialect::Postgres, "now()"), ColumnDefault::Now);
        assert_eq!(parse_default(Dialect::Postgres, "CURRENT_TIMESTAMP"), ColumnDefault::Now);
        assert_eq!(parse_default(Dialect::Postgres, "NULL::character varying"), ColumnDefault::Null);
        assert_eq!(parse_default(Dialect::Postgres, "''::text"), ColumnDefault::Empty);
        assert_eq!(parse_default(Dialect::Postgres, "'draft'::character varying"), ColumnDefault::Custom { expression: "'draft'".into() });
        assert_eq!(parse_default(Dialect::Postgres, "0"), ColumnDefault::Custom { expression: "0".into() });
        assert_eq!(parse_default(Dialect::Postgres, "'it''s'::text"), ColumnDefault::Custom { expression: "'it''s'".into() });
        assert_eq!(parse_default(Dialect::Mssql, "((0))"), ColumnDefault::Custom { expression: "0".into() });
        assert_eq!(parse_default(Dialect::Mssql, "(getdate())"), ColumnDefault::Now);
        assert_eq!(parse_default(Dialect::Mssql, "('')"), ColumnDefault::Empty);
        assert_eq!(parse_default(Dialect::Mssql, "(CONVERT([int],(1)))"), ColumnDefault::Custom { expression: "CONVERT([int],(1))".into() });
        assert_eq!(parse_default(Dialect::Sqlite, "CURRENT_TIMESTAMP"), ColumnDefault::Now);
        assert_eq!(parse_default(Dialect::Sqlite, "'x'"), ColumnDefault::Custom { expression: "'x'".into() });
        assert_eq!(parse_default(Dialect::Clickhouse, "now()"), ColumnDefault::Now);
        assert_eq!(parse_default(Dialect::Postgres, "  "), ColumnDefault::None);
    }

    #[test]
    fn catalogue_queries_escape_the_table_name() {
        let sql = pg_columns_sql(&users());
        assert!(sql.contains("c.relname = 'us''ers'") && sql.contains("n.nspname = 'app'"), "{sql}");
        assert!(pg_columns_sql(&TableRef { schema: None, name: "t".into() }).contains("n.nspname = current_schema()"));
        assert!(mysql_columns_sql(&TableRef { schema: None, name: "t".into() }).contains("TABLE_SCHEMA = DATABASE()"));
        assert!(mssql_columns_sql(&users()).contains("OBJECT_ID(N'[app].[us''ers]')"));
        assert_eq!(sqlite_target(&users(), "table_info", "us\"ers"), "PRAGMA \"app\".table_info(\"us\"\"ers\")");
        assert!(oracle_indexes_sql(&users()).contains("table_name = 'us''ers'"));
        assert!(clickhouse_table_sql(&TableRef { schema: None, name: "t".into() }).contains("database = currentDatabase()"));
    }

    #[test]
    fn postgres_rows_become_columns_indexes_and_foreign_keys() {
        let columns = pg_columns(&vec![
            vec![t("id"), t("integer"), t("nextval('users_id_seq'::regclass)"), t("")],
            vec![t("uid"), t("bigint"), Value::Null, t("d")],
            vec![t("status"), t("character varying(20)"), t("'draft'::character varying"), t("")],
        ]);
        assert!(columns[0].auto && columns[0].default == Some(ColumnDefault::None));
        assert!(columns[1].auto);
        assert!(!columns[2].auto && columns[2].default == Some(ColumnDefault::Custom { expression: "'draft'".into() }));

        let indexes = pg_indexes(&vec![
            vec![t("users_pkey"), i(1), i(1), t("id"), i(1), i(0)],
            vec![t("users_email_key"), i(1), i(0), t("email"), i(1), i(0)],
            vec![t("users_ab"), i(0), i(0), t("a"), i(1), i(0)],
            vec![t("users_ab"), i(0), i(0), t("b"), i(2), i(0)],
            vec![t("users_lower"), i(0), i(0), Value::Null, i(1), i(1)],
        ]);
        assert_eq!(indexes.len(), 5);
        assert!(indexes[0].primary && indexes[1].unique && !indexes[2].unique && indexes[4].skip && indexes[4].column.is_none());

        let fks = group_foreign_keys(
            &[
                vec![t("fk_pair"), t("a"), t("x"), t("public"), t("parent"), t("c"), t("n")],
                vec![t("fk_pair"), t("b"), t("y"), t("public"), t("parent"), t("c"), t("n")],
                vec![t("fk_one"), t("c"), t("id"), t("public"), t("other"), t("a"), t("r")],
            ],
            &FK_ROW,
        );
        assert_eq!(fks.len(), 2);
        assert_eq!((fks[0].columns.clone(), fks[0].ref_columns.clone()), (vec!["a".to_string(), "b".to_string()], vec!["x".to_string(), "y".to_string()]));
        assert_eq!((fks[0].on_update, fks[0].on_delete), (FkAction::Cascade, FkAction::SetNull));
        assert_eq!((fks[1].on_update, fks[1].on_delete), (FkAction::NoAction, FkAction::Restrict));
        assert_eq!(fks[1].original_name.as_deref(), Some("fk_one"));
        assert_eq!(fks[1].ref_table, TableRef { schema: Some("public".into()), name: "other".into() });
    }

    #[test]
    fn mysql_defaults_follow_each_servers_printing_rules() {
        assert_eq!(mysql_default(Engine::Mysql, "", ""), ColumnDefault::Empty);
        assert_eq!(mysql_default(Engine::Mysql, "7", ""), ColumnDefault::Custom { expression: "7".into() });
        assert_eq!(mysql_default(Engine::Mysql, "it's", ""), ColumnDefault::Custom { expression: "'it''s'".into() });
        assert_eq!(mysql_default(Engine::Mysql, "CURRENT_TIMESTAMP", "DEFAULT_GENERATED"), ColumnDefault::Now);
        assert_eq!(mysql_default(Engine::Mysql, "uuid()", "DEFAULT_GENERATED"), ColumnDefault::Custom { expression: "(uuid())".into() });
        assert_eq!(mysql_default(Engine::Mariadb, "current_timestamp()", ""), ColumnDefault::Now);
        assert_eq!(mysql_default(Engine::Mariadb, "'abc'", ""), ColumnDefault::Custom { expression: "'abc'".into() });
        assert_eq!(mysql_default(Engine::Mariadb, "NULL", ""), ColumnDefault::Null);
        let columns = mysql_columns(Engine::Mysql, &vec![vec![t("id"), t("int(10) unsigned"), Value::Null, t("auto_increment")]]);
        assert!(columns[0].auto && columns[0].default == Some(ColumnDefault::None));
        let indexes = mysql_indexes(&vec![
            vec![t("PRIMARY"), i(0), t("id"), i(1), Value::Null],
            vec![t("email"), i(0), t("email"), i(1), Value::Null],
            vec![t("bio_idx"), i(1), t("bio"), i(1), i(10)],
        ]);
        assert!(indexes[0].primary && indexes[1].unique && !indexes[2].unique && indexes[2].skip);
    }

    #[test]
    fn sqlite_rows_become_defaults_autoincrement_and_foreign_keys() {
        let rows = vec![
            vec![i(0), t("id"), t("INTEGER"), i(1), Value::Null, i(1)],
            vec![i(1), t("at"), t("DATETIME"), i(0), t("CURRENT_TIMESTAMP"), i(0)],
        ];
        let plain = sqlite_columns(&rows, "CREATE TABLE t (id INTEGER PRIMARY KEY, at DATETIME)");
        assert!(!plain[0].auto && plain[1].default == Some(ColumnDefault::Now));
        let auto = sqlite_columns(&rows, "CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, at DATETIME)");
        assert!(auto[0].auto && !auto[1].auto);
        let fks = sqlite_foreign_keys(
            &vec![
                vec![i(0), i(0), t("parent"), t("a"), t("x"), t("CASCADE"), t("SET NULL")],
                vec![i(0), i(1), t("parent"), t("b"), t("y"), t("CASCADE"), t("SET NULL")],
                vec![i(1), i(0), t("owner"), t("o"), Value::Null, t("NO ACTION"), t("NO ACTION")],
            ],
            &[ForeignKeySpec {
                name: Some("fk".into()),
                original_name: Some("fk".into()),
                columns: vec!["o".into()],
                ref_table: TableRef { schema: None, name: "owner".into() },
                ref_columns: vec!["id".into()],
                on_delete: FkAction::NoAction,
                on_update: FkAction::NoAction,
            }],
        );
        assert_eq!(fks.len(), 2);
        assert_eq!(fks[0].columns, vec!["a".to_string(), "b".to_string()]);
        assert_eq!((fks[0].on_update, fks[0].on_delete, fks[0].original_name.as_deref()), (FkAction::Cascade, FkAction::SetNull, Some("fk_0")));
        assert_eq!(fks[1].ref_columns, vec!["id".to_string()], "an implicit parent key is read from the adapter's own answer");
        assert!(fks.iter().all(|f| f.name.is_none()));
    }

    #[test]
    fn sql_server_and_oracle_types_are_composed_from_their_parts() {
        assert_eq!(mssql_type("varchar", 255, 0, 0), "varchar(255)");
        assert_eq!(mssql_type("nvarchar", 100, 0, 0), "nvarchar(50)");
        assert_eq!(mssql_type("nvarchar", -1, 0, 0), "nvarchar(max)");
        assert_eq!(mssql_type("decimal", 9, 10, 2), "decimal(10,2)");
        assert_eq!(mssql_type("datetime2", 8, 27, 3), "datetime2(3)");
        assert_eq!(mssql_type("datetime2", 8, 27, 7), "datetime2");
        assert_eq!(mssql_type("float", 8, 53, 0), "float");
        assert_eq!(mssql_type("int", 4, 10, 0), "int");
        let columns = oracle_columns(&vec![
            vec![t("NAME"), t("VARCHAR2"), i(80), Value::Null, Value::Null, i(20), t("'x'  \n"), t("NO")],
            vec![t("ID"), t("NUMBER"), i(22), i(10), i(0), i(0), Value::Null, t("YES")],
            vec![t("AMOUNT"), t("NUMBER"), i(22), i(12), i(2), i(0), Value::Null, t("NO")],
        ]);
        assert_eq!(columns[0].full_type.as_deref(), Some("VARCHAR2(20)"));
        assert_eq!(columns[0].default, Some(ColumnDefault::Custom { expression: "'x'".into() }));
        assert!(columns[1].auto);
        assert_eq!((columns[1].full_type.as_deref(), columns[2].full_type.as_deref()), (Some("NUMBER(10)"), Some("NUMBER(12,2)")));
    }

    #[test]
    fn clickhouse_sorting_keys_split_at_top_level_commas_only() {
        assert_eq!(split_top_level("a, b"), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(split_top_level("toDate(at), id"), vec!["toDate(at)".to_string(), "id".to_string()]);
        assert!(split_top_level("").is_empty());
        let columns = clickhouse_columns(&vec![vec![t("n"), t("Nullable(String)"), t("DEFAULT"), t("'x'")], vec![t("m"), t("UInt8"), t("MATERIALIZED"), t("1")]]);
        assert_eq!(columns[0].default, Some(ColumnDefault::Custom { expression: "'x'".into() }));
        assert_eq!(columns[1].default, Some(ColumnDefault::None), "a materialised column has no DEFAULT to write back");
    }

    #[test]
    fn list_cells_read_json_arrays_and_bracketed_text() {
        assert_eq!(list(&Value::Json(serde_json::json!(["a", "b"]))), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(list(&t("[a, 'b']")), vec!["a".to_string(), "b".to_string()]);
        assert!(list(&Value::Null).is_empty());
    }

    #[test]
    fn merging_extras_makes_single_unique_indexes_column_flags() {
        use crate::integrations::ddl::types::tests::col;
        let cx = Cx::new(Engine::Postgres).unwrap_or_else(|e| panic!("{e}"));
        let named = |name: &str, ty: &str| ColumnSpec { original_name: Some(name.into()), ..col(name, ty) };
        let mut spec = TableSpec {
            table: users(),
            columns: vec![named("id", "integer"), named("email", "varchar"), named("a", "integer")],
            primary_key_name: None,
            foreign_keys: Vec::new(),
            indexes: Vec::new(),
            engine: None,
            order_by: Vec::new(),
            if_not_exists: false,
        };
        let extras = Extras {
            columns: vec![ColExtra { name: "email".into(), full_type: Some("character varying(255)".into()), default: Some(ColumnDefault::Empty), auto: false }],
            indexes: vec![
                IndexRow { name: "users_pkey".into(), unique: true, primary: true, column: Some("id".into()), skip: false },
                IndexRow { name: "users_email_key".into(), unique: true, primary: false, column: Some("email".into()), skip: false },
                IndexRow { name: "users_a_b".into(), unique: false, primary: false, column: Some("a".into()), skip: false },
                IndexRow { name: "users_a_b".into(), unique: false, primary: false, column: Some("id".into()), skip: false },
                IndexRow { name: "users_expr".into(), unique: false, primary: false, column: None, skip: true },
            ],
            primary_key_name: Some("users_pkey".into()),
            ..Extras::default()
        };
        merge(&cx, &mut spec, extras);
        let email = &spec.columns[1];
        assert_eq!((email.data_type.as_str(), email.length, email.unique, email.unique_name.as_deref()), ("varchar", Some(255), true, Some("users_email_key")));
        assert_eq!(email.default, ColumnDefault::Empty);
        assert_eq!(spec.indexes, vec![IndexSpec { name: "users_a_b".into(), columns: vec!["a".into(), "id".into()], unique: false }]);
        assert_eq!(spec.primary_key_name.as_deref(), Some("users_pkey"));
    }
}
