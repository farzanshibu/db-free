// SOT: create-table-sql, ddl-column-definition, ddl-default-value, ddl-foreign-key-sql, ddl-index-sql, ddl-spec-validation

use super::dialect::{check_name, derived_name, same_name, Cx, Dialect};
use super::types::{auto_increment_allowed, render_type};
use crate::error::{AppError, AppResult};
use crate::model::{ColumnDefault, ColumnSpec, FkAction, ForeignKeySpec, IndexSpec, TableRef, TableSpec};

// WHAT:  CREATE TABLE from a `TableSpec`, and the pieces every other builder
//        reuses: one column's definition, a DEFAULT clause, a FOREIGN KEY
//        clause, a CREATE INDEX statement and the spec validation.
// WHY:   Create, the SQLite rebuild and Duplicate all write the same column and
//        constraint syntax. Writing it once means the three can never disagree
//        about, say, how a `serial` column or an inline AUTOINCREMENT is spelled.
// HOW:   The CREATE TABLE is laid out one definition per line so the preview in
//        the form reads like hand-written DDL. Anything the dialect cannot
//        express is refused with a message naming the engine and the column,
//        never silently dropped.
// WHERE: src-tauri/src/integrations/ddl/{alter,duplicate}.rs

/// The statements that create a table, split so a rebuild can place its own steps
/// between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    /// Statements that must run first (DuckDB sequences).
    pub prelude: Vec<String>,
    pub table: String,
    pub indexes: Vec<String>,
}

pub fn create_table(cx: &Cx, spec: &TableSpec) -> AppResult<Vec<String>> {
    if cx.dialect == Dialect::Mongo {
        return super::mongo::create_collection(spec);
    }
    let created = build_create(cx, spec)?;
    let mut out = created.prelude;
    out.push(created.table);
    out.extend(created.indexes);
    Ok(out)
}

// ---- validation ----------------------------------------------------------------------

pub fn validate(cx: &Cx, spec: &TableSpec) -> AppResult<()> {
    check_name(if cx.dialect == Dialect::Mongo { "collection" } else { "table" }, &spec.table.name)?;
    if cx.dialect == Dialect::Mongo {
        return Ok(());
    }
    if spec.columns.is_empty() {
        return Err(AppError::invalid_input("A table needs at least one column."));
    }
    for (i, column) in spec.columns.iter().enumerate() {
        check_name("column", &column.name)?;
        if spec.columns[..i].iter().any(|other| same_name(&other.name, &column.name)) {
            return Err(AppError::invalid_input(format!("Column \"{}\" is listed twice.", column.name)));
        }
        if column.default != ColumnDefault::None && cx.dialect == Dialect::Cassandra {
            return Err(AppError::invalid_input(format!("Column \"{}\": Cassandra columns have no DEFAULT.", column.name)));
        }
        if column.unique && matches!(cx.dialect, Dialect::Clickhouse | Dialect::Bigquery | Dialect::Cassandra) {
            return Err(AppError::invalid_input(format!("Column \"{}\": {} does not enforce UNIQUE.", column.name, cx.dialect.label())));
        }
        if let ColumnDefault::Custom { expression } = &column.default {
            if expression.trim().is_empty() {
                return Err(AppError::invalid_input(format!("Column \"{}\": the custom default is empty.", column.name)));
            }
        }
    }
    let auto: Vec<&ColumnSpec> = spec.columns.iter().filter(|c| c.auto_increment).collect();
    if !auto.is_empty() {
        if matches!(cx.dialect, Dialect::Clickhouse | Dialect::Bigquery | Dialect::Cassandra) {
            return Err(AppError::invalid_input(format!("{} has no auto-increment columns.", cx.dialect.label())));
        }
        for column in &auto {
            if !auto_increment_allowed(cx.dialect, &column.data_type) {
                return Err(AppError::invalid_input(format!(
                    "Column \"{}\": auto-increment needs an integer type, not {}.",
                    column.name, column.data_type
                )));
            }
        }
        if auto.len() > 1 && cx.dialect != Dialect::Postgres {
            return Err(AppError::invalid_input(format!("{} allows one auto-increment column per table.", cx.dialect.label())));
        }
        let pk_count = spec.columns.iter().filter(|c| c.primary_key).count();
        for column in &auto {
            if cx.dialect == Dialect::Mysql && !column.primary_key && !column.unique {
                return Err(AppError::invalid_input(format!(
                    "Column \"{}\": MySQL needs an auto-increment column to be a primary key or unique.",
                    column.name
                )));
            }
            if cx.dialect == Dialect::Sqlite && !(column.primary_key && pk_count == 1) {
                return Err(AppError::invalid_input(format!(
                    "Column \"{}\": SQLite auto-increment needs the column to be the table's only primary key (it cannot be part of a composite key).",
                    column.name
                )));
            }
        }
    }
    if cx.dialect == Dialect::Cassandra && !spec.columns.iter().any(|c| c.primary_key) {
        return Err(AppError::invalid_input("A Cassandra table needs a primary key: mark at least one column."));
    }
    validate_keys(cx, spec)?;
    validate_engine(cx, spec)
}

fn has_column(spec: &TableSpec, name: &str) -> bool {
    spec.columns.iter().any(|c| same_name(&c.name, name))
}

fn validate_keys(cx: &Cx, spec: &TableSpec) -> AppResult<()> {
    if !spec.foreign_keys.is_empty() && matches!(cx.dialect, Dialect::Clickhouse | Dialect::Cassandra) {
        return Err(AppError::invalid_input(format!("{} has no foreign keys.", cx.dialect.label())));
    }
    for fk in &spec.foreign_keys {
        if fk.columns.is_empty() {
            return Err(AppError::invalid_input("A foreign key needs at least one column."));
        }
        if fk.columns.len() != fk.ref_columns.len() {
            return Err(AppError::invalid_input(format!(
                "A foreign key on ({}) must reference the same number of columns, not {}.",
                fk.columns.join(", "),
                fk.ref_columns.len()
            )));
        }
        check_name("referenced table", &fk.ref_table.name)?;
        for column in &fk.columns {
            if !has_column(spec, column) {
                return Err(AppError::invalid_input(format!("The foreign key refers to column \"{column}\", which the table does not have.")));
            }
        }
        for column in &fk.ref_columns {
            check_name("referenced column", column)?;
        }
    }
    for index in &spec.indexes {
        check_name("index", &index.name)?;
        if index.columns.is_empty() {
            return Err(AppError::invalid_input(format!("Index \"{}\" has no columns.", index.name)));
        }
        for column in &index.columns {
            if !has_column(spec, column) {
                return Err(AppError::invalid_input(format!("Index \"{}\" refers to column \"{column}\", which the table does not have.", index.name)));
            }
        }
    }
    Ok(())
}

fn validate_engine(cx: &Cx, spec: &TableSpec) -> AppResult<()> {
    if cx.dialect != Dialect::Clickhouse {
        return Ok(());
    }
    if let Some(engine) = spec.engine.as_deref() {
        let name = engine.split('(').next().unwrap_or_default().trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(AppError::invalid_input(format!("\"{engine}\" is not a ClickHouse table engine.")));
        }
    }
    for column in &spec.order_by {
        if !has_column(spec, column) {
            return Err(AppError::invalid_input(format!("ORDER BY refers to column \"{column}\", which the table does not have.")));
        }
    }
    Ok(())
}

// ---- DEFAULT ---------------------------------------------------------------------------

fn now_sql(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Postgres | Dialect::Duckdb | Dialect::Clickhouse => "now()",
        Dialect::Mssql => "GETDATE()",
        Dialect::Snowflake | Dialect::Bigquery => "CURRENT_TIMESTAMP()",
        Dialect::Mysql | Dialect::Oracle | Dialect::Sqlite | Dialect::Cassandra | Dialect::Mongo => "CURRENT_TIMESTAMP",
    }
}

/// SQLite accepts a bare literal or keyword as a default; anything else must be
/// parenthesised.
fn sqlite_default_is_literal(expression: &str) -> bool {
    let e = expression.trim();
    let upper = e.to_ascii_uppercase();
    if matches!(upper.as_str(), "NULL" | "TRUE" | "FALSE" | "CURRENT_TIME" | "CURRENT_DATE" | "CURRENT_TIMESTAMP") {
        return true;
    }
    if e.len() >= 2 && e.starts_with('(') && e.ends_with(')') {
        return true;
    }
    if (e.starts_with('\'') && e.ends_with('\'') && e.len() >= 2) || ((upper.starts_with("X'")) && e.ends_with('\'')) {
        return true;
    }
    let digits = e.strip_prefix(['-', '+']).unwrap_or(e);
    !digits.is_empty()
        && digits.chars().filter(|c| *c == '.').count() <= 1
        && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        && digits.chars().any(|c| c.is_ascii_digit())
}

/// What follows the word DEFAULT, or None for "write no clause".
pub fn default_value(cx: &Cx, column: &ColumnSpec) -> AppResult<Option<String>> {
    let nullable = column.nullable && !column.primary_key;
    Ok(match &column.default {
        ColumnDefault::None => None,
        // `DEFAULT NULL` on a NOT NULL column is rejected by MySQL and means nothing elsewhere.
        ColumnDefault::Null => nullable.then(|| "NULL".to_string()),
        ColumnDefault::Now => Some(now_sql(cx.dialect).to_string()),
        ColumnDefault::Empty => Some("''".to_string()),
        ColumnDefault::Custom { expression } => {
            let expression = expression.trim();
            if expression.is_empty() {
                return Err(AppError::invalid_input(format!("Column \"{}\": the custom default is empty.", column.name)));
            }
            Some(if cx.dialect == Dialect::Sqlite && !sqlite_default_is_literal(expression) { format!("({expression})") } else { expression.to_string() })
        }
    })
}

// ---- one column ------------------------------------------------------------------------

/// How a column definition is being used.
#[derive(Debug, Clone, Copy)]
pub struct ColumnUse<'a> {
    /// The table the column belongs to (DuckDB names its sequence after it).
    pub table: &'a TableRef,
    /// The column is the table's only primary-key column.
    pub sole_primary_key: bool,
    /// ADD COLUMN: the primary key is added separately, and DuckDB cannot add
    /// constraints inline.
    pub adding: bool,
}

/// The sequence DuckDB uses for an auto-increment column.
pub fn duckdb_sequence(table: &TableRef, column: &str) -> TableRef {
    TableRef { schema: table.schema.clone().filter(|s| !s.is_empty()), name: derived_name(&[&table.name, column, "seq"]) }
}

fn postgres_serial(base: &str) -> &'static str {
    match base.trim().to_ascii_lowercase().as_str() {
        "smallint" => "smallserial",
        "bigint" => "bigserial",
        _ => "serial",
    }
}

pub fn column_definition(cx: &Cx, column: &ColumnSpec, usage: ColumnUse<'_>) -> AppResult<String> {
    let name = cx.q(&column.name);
    let pk_here = column.primary_key && !usage.adding;
    let not_null = !(column.nullable && !column.primary_key);
    let mut ty = render_type(cx.dialect, column)?;
    let default = if column.auto_increment && cx.dialect != Dialect::Duckdb { None } else { default_value(cx, column)? };
    // UNIQUE beside a lone primary key is redundant.
    let unique = column.unique && !(pk_here && usage.sole_primary_key);

    let mut parts: Vec<String> = vec![name];
    match cx.dialect {
        Dialect::Postgres => {
            if column.auto_increment {
                ty = postgres_serial(&column.data_type).to_string();
            }
            parts.push(ty);
            if not_null && !column.auto_increment {
                parts.push("NOT NULL".into());
            }
            if let Some(d) = default {
                parts.push(format!("DEFAULT {d}"));
            }
            if unique {
                parts.push("UNIQUE".into());
            }
        }
        Dialect::Mysql => {
            parts.push(ty);
            parts.push(if not_null { "NOT NULL" } else { "NULL" }.into());
            if let Some(d) = default {
                parts.push(format!("DEFAULT {d}"));
            }
            if column.auto_increment {
                parts.push("AUTO_INCREMENT".into());
            }
            if unique {
                parts.push("UNIQUE".into());
            }
        }
        Dialect::Mssql => {
            parts.push(ty);
            if column.auto_increment {
                parts.push("IDENTITY(1,1)".into());
            }
            parts.push(if not_null { "NOT NULL" } else { "NULL" }.into());
            if let Some(d) = default {
                parts.push(format!("DEFAULT {d}"));
            }
            if unique {
                parts.push("UNIQUE".into());
            }
        }
        Dialect::Oracle => {
            parts.push(ty);
            if let Some(d) = default {
                parts.push(format!("DEFAULT {d}"));
            }
            if column.auto_increment {
                parts.push("GENERATED BY DEFAULT AS IDENTITY".into());
            }
            if not_null && !column.auto_increment {
                parts.push("NOT NULL".into());
            }
            if unique {
                parts.push("UNIQUE".into());
            }
        }
        Dialect::Sqlite => {
            parts.push(ty);
            if column.auto_increment {
                parts.push("PRIMARY KEY AUTOINCREMENT".into());
            }
            if not_null {
                parts.push("NOT NULL".into());
            }
            if let Some(d) = default {
                parts.push(format!("DEFAULT {d}"));
            }
            if unique {
                parts.push("UNIQUE".into());
            }
        }
        Dialect::Duckdb => {
            parts.push(ty);
            if column.auto_increment {
                let sequence = cx.table(&duckdb_sequence(usage.table, &column.name));
                parts.push(format!("DEFAULT nextval({})", cx.lit(&sequence)));
            } else if let Some(d) = default {
                parts.push(format!("DEFAULT {d}"));
            }
            // DuckDB cannot add a column that carries constraints; nullability
            // follows as its own ALTER.
            if not_null && !usage.adding {
                parts.push("NOT NULL".into());
            }
            if unique && !usage.adding {
                parts.push("UNIQUE".into());
            }
        }
        Dialect::Clickhouse => {
            parts.push(ty);
            if let Some(d) = default {
                parts.push(format!("DEFAULT {d}"));
            }
        }
        Dialect::Snowflake => {
            parts.push(ty);
            if column.auto_increment {
                parts.push("AUTOINCREMENT".into());
            } else if let Some(d) = default {
                parts.push(format!("DEFAULT {d}"));
            }
            if not_null {
                parts.push("NOT NULL".into());
            }
            if unique {
                parts.push("UNIQUE".into());
            }
        }
        Dialect::Bigquery => {
            parts.push(ty);
            if let Some(d) = default {
                parts.push(format!("DEFAULT {d}"));
            }
            if not_null {
                parts.push("NOT NULL".into());
            }
        }
        Dialect::Cassandra => parts.push(ty),
        Dialect::Mongo => return Err(AppError::invalid_input("MongoDB collections have no columns.")),
    }
    Ok(parts.join(" "))
}

// ---- constraints and indexes -----------------------------------------------------------

fn fk_action_clause(cx: &Cx, keyword: &str, action: FkAction) -> AppResult<Option<String>> {
    let action = match (cx.dialect, action) {
        (_, FkAction::NoAction) => return Ok(None),
        // SQL Server has no RESTRICT; its NO ACTION is checked immediately, which is what RESTRICT means.
        (Dialect::Mssql, FkAction::Restrict) => return Ok(None),
        (Dialect::Mysql, FkAction::SetDefault) => {
            return Err(AppError::invalid_input("MySQL (InnoDB) rejects ON DELETE / ON UPDATE SET DEFAULT; use SET NULL, CASCADE, RESTRICT or NO ACTION."));
        }
        (Dialect::Oracle, FkAction::Restrict) if keyword == "DELETE" => return Ok(None),
        (Dialect::Oracle, FkAction::Cascade | FkAction::SetNull) if keyword == "DELETE" => action,
        (Dialect::Oracle, _) => return Err(AppError::invalid_input("Oracle supports only ON DELETE CASCADE and ON DELETE SET NULL, and has no ON UPDATE action.")),
        (Dialect::Duckdb, FkAction::Restrict) => return Ok(None),
        (Dialect::Duckdb, _) => return Err(AppError::invalid_input("DuckDB does not support ON DELETE / ON UPDATE actions on foreign keys.")),
        (Dialect::Bigquery, _) => return Err(AppError::invalid_input("BigQuery foreign keys have no ON DELETE / ON UPDATE actions.")),
        _ => action,
    };
    Ok(Some(format!("ON {keyword} {}", action.sql())))
}

/// The generated name of a foreign key the form left unnamed.
pub fn auto_fk_name(table: &str, fk: &ForeignKeySpec) -> String {
    let columns = fk.columns.join("_");
    derived_name(&[table, &columns, "fkey"])
}

/// `[CONSTRAINT name] FOREIGN KEY (a, b) REFERENCES ref (x, y) [ON DELETE …] [ON UPDATE …]`
pub fn foreign_key_clause(cx: &Cx, table_name: &str, fk: &ForeignKeySpec, named: bool) -> AppResult<String> {
    let mut out = String::new();
    if named {
        let name = fk.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| auto_fk_name(table_name, fk));
        out.push_str(&format!("CONSTRAINT {} ", cx.q(&name)));
    }
    // SQLite's REFERENCES takes a bare table name.
    let target = if cx.dialect == Dialect::Sqlite { cx.q(&fk.ref_table.name) } else { cx.table(&fk.ref_table) };
    out.push_str(&format!("FOREIGN KEY ({}) REFERENCES {} ({})", cx.columns(&fk.columns), target, cx.columns(&fk.ref_columns)));
    for clause in [fk_action_clause(cx, "DELETE", fk.on_delete)?, fk_action_clause(cx, "UPDATE", fk.on_update)?].into_iter().flatten() {
        out.push(' ');
        out.push_str(&clause);
    }
    if cx.dialect == Dialect::Bigquery {
        out.push_str(" NOT ENFORCED");
    }
    Ok(out)
}

pub fn primary_key_clause(cx: &Cx, columns: &[&str]) -> String {
    let list = columns.iter().map(|c| cx.q(c)).collect::<Vec<_>>().join(", ");
    match cx.dialect {
        Dialect::Bigquery => format!("PRIMARY KEY ({list}) NOT ENFORCED"),
        _ => format!("PRIMARY KEY ({list})"),
    }
}

/// `CREATE [UNIQUE] INDEX name ON table (cols)`; None for engines without them.
pub fn index_statement(cx: &Cx, table: &TableRef, index: &IndexSpec) -> Option<String> {
    if matches!(cx.dialect, Dialect::Clickhouse | Dialect::Bigquery | Dialect::Cassandra | Dialect::Snowflake | Dialect::Mongo) {
        return None;
    }
    // PostgreSQL, SQLite and DuckDB create an index in the table's schema; MySQL,
    // SQL Server and Oracle name it on its own and attach it with ON.
    let name = match cx.dialect {
        Dialect::Postgres | Dialect::Sqlite | Dialect::Duckdb | Dialect::Oracle => {
            cx.table(&TableRef { schema: table.schema.clone().filter(|s| !s.is_empty()), name: index.name.clone() })
        }
        _ => cx.q(&index.name),
    };
    Some(format!(
        "CREATE {}INDEX {} ON {} ({})",
        if index.unique { "UNIQUE " } else { "" },
        name,
        cx.table(table),
        cx.columns(&index.columns)
    ))
}

// ---- the statement -----------------------------------------------------------------------

fn primary_key_columns(spec: &TableSpec) -> Vec<&ColumnSpec> {
    spec.columns.iter().filter(|c| c.primary_key).collect()
}

pub fn build_create(cx: &Cx, spec: &TableSpec) -> AppResult<Created> {
    validate(cx, spec)?;
    let table = &spec.table;
    let pk = primary_key_columns(spec);
    let sole = pk.len() == 1;
    let mut prelude = Vec::new();
    let mut lines: Vec<String> = Vec::new();

    for column in &spec.columns {
        if column.auto_increment && cx.dialect == Dialect::Duckdb {
            let sequence = cx.table(&duckdb_sequence(table, &column.name));
            prelude.push(format!("CREATE SEQUENCE {}{}", if spec.if_not_exists { "IF NOT EXISTS " } else { "" }, sequence));
        }
        lines.push(column_definition(cx, column, ColumnUse { table, sole_primary_key: sole, adding: false })?);
    }

    // SQLite's AUTOINCREMENT is spelled inside the column, so the key clause would repeat it.
    let key_inline = cx.dialect == Dialect::Sqlite && pk.iter().any(|c| c.auto_increment);
    if !pk.is_empty() && !key_inline && cx.dialect != Dialect::Clickhouse {
        lines.push(primary_key_clause(cx, &pk.iter().map(|c| c.name.as_str()).collect::<Vec<_>>()));
    }
    for fk in &spec.foreign_keys {
        lines.push(foreign_key_clause(cx, &table.name, fk, true)?);
    }

    let mut head = String::from("CREATE TABLE ");
    let mut guard = String::new();
    if spec.if_not_exists {
        match cx.dialect {
            Dialect::Mssql => {
                guard = format!("IF OBJECT_ID(N{}, N'U') IS NULL\n", cx.lit(&cx.table(table)));
            }
            Dialect::Oracle => {
                return Err(AppError::invalid_input("Oracle before 23ai has no CREATE TABLE IF NOT EXISTS; untick \"if not exists\"."));
            }
            _ => head.push_str("IF NOT EXISTS "),
        }
    }
    head.push_str(&cx.table(table));
    let body = lines.iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join(",\n");
    let mut sql = format!("{guard}{head} (\n{body}\n)");
    if cx.dialect == Dialect::Clickhouse {
        sql.push_str(&clickhouse_tail(cx, spec, &pk));
    }

    let indexes = spec.indexes.iter().filter_map(|i| index_statement(cx, table, i)).collect();
    Ok(Created { prelude, table: sql, indexes })
}

/// `ENGINE = … ORDER BY … PRIMARY KEY …`
fn clickhouse_tail(cx: &Cx, spec: &TableSpec, pk: &[&ColumnSpec]) -> String {
    let engine = spec.engine.as_deref().map(str::trim).filter(|e| !e.is_empty()).unwrap_or("MergeTree");
    let name = engine.split('(').next().unwrap_or(engine);
    let mut out = format!("\nENGINE = {engine}");
    if name.ends_with("MergeTree") {
        let order: Vec<&str> = if spec.order_by.is_empty() { pk.iter().map(|c| c.name.as_str()).collect() } else { spec.order_by.iter().map(String::as_str).collect() };
        if order.is_empty() {
            out.push_str("\nORDER BY tuple()");
        } else {
            out.push_str(&format!("\nORDER BY ({})", order.iter().map(|c| cx.q(c)).collect::<Vec<_>>().join(", ")));
        }
        if !pk.is_empty() {
            out.push_str(&format!("\nPRIMARY KEY ({})", pk.iter().map(|c| cx.q(&c.name)).collect::<Vec<_>>().join(", ")));
        }
    }
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::integrations::ddl::types::tests::col;
    use crate::model::Engine;

    pub fn table(name: &str) -> TableRef {
        TableRef { schema: None, name: name.into() }
    }

    pub fn spec(engine_table: TableRef, columns: Vec<ColumnSpec>) -> TableSpec {
        TableSpec {
            table: engine_table,
            columns,
            primary_key_name: None,
            foreign_keys: Vec::new(),
            indexes: Vec::new(),
            engine: None,
            order_by: Vec::new(),
            if_not_exists: false,
        }
    }

    pub fn pk(name: &str, ty: &str) -> ColumnSpec {
        ColumnSpec { primary_key: true, nullable: false, ..col(name, ty) }
    }

    pub fn fk(columns: &[&str], ref_table: &str, ref_columns: &[&str]) -> ForeignKeySpec {
        ForeignKeySpec {
            name: None,
            original_name: None,
            columns: columns.iter().map(|c| (*c).to_string()).collect(),
            ref_table: table(ref_table),
            ref_columns: ref_columns.iter().map(|c| (*c).to_string()).collect(),
            on_delete: FkAction::NoAction,
            on_update: FkAction::NoAction,
        }
    }

    fn create(engine: Engine, spec: &TableSpec) -> String {
        super::super::create_table(engine, spec).map(|s| s.join(";\n")).unwrap_or_else(|e| format!("ERR {e}"))
    }

    fn users() -> TableSpec {
        let mut id = pk("id", "integer");
        id.auto_increment = true;
        let mut email = col("email", "varchar");
        email.length = Some(255);
        email.nullable = false;
        email.unique = true;
        let mut created = col("created_at", "timestamp");
        created.default = ColumnDefault::Now;
        created.nullable = false;
        let mut note = col("note", "text");
        note.default = ColumnDefault::Empty;
        spec(TableRef { schema: Some("app".into()), name: "users".into() }, vec![id, email, created, note])
    }

    #[test]
    fn postgres_create_quotes_everything_and_uses_serial() {
        assert_eq!(
            create(Engine::Postgres, &users()),
            "CREATE TABLE \"app\".\"users\" (\n  \"id\" serial,\n  \"email\" varchar(255) NOT NULL UNIQUE,\n  \"created_at\" timestamp NOT NULL DEFAULT now(),\n  \"note\" text DEFAULT '',\n  PRIMARY KEY (\"id\")\n)"
        );
        let mut big = users();
        big.columns[0].data_type = "bigint".into();
        big.if_not_exists = true;
        assert!(create(Engine::Supabase, &big).starts_with("CREATE TABLE IF NOT EXISTS \"app\".\"users\" (\n  \"id\" bigserial,"));
    }

    #[test]
    fn mysql_create_uses_backticks_auto_increment_and_explicit_null() {
        let mut spec = users();
        spec.table.schema = None;
        spec.columns[0].data_type = "int".into();
        spec.columns[0].unsigned = true;
        assert_eq!(
            create(Engine::Mariadb, &spec),
            "CREATE TABLE `users` (\n  `id` int UNSIGNED NOT NULL AUTO_INCREMENT,\n  `email` varchar(255) NOT NULL UNIQUE,\n  `created_at` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,\n  `note` text NULL DEFAULT '',\n  PRIMARY KEY (`id`)\n)"
        );
        // Auto-increment must be a key.
        let mut loose = spec.clone();
        loose.columns[0].primary_key = false;
        assert!(create(Engine::Mysql, &loose).starts_with("ERR"));
    }

    #[test]
    fn sqlite_spells_autoincrement_inline_and_parenthesises_expression_defaults() {
        let mut spec = users();
        spec.table.schema = None;
        spec.columns[0].data_type = "INTEGER".into();
        spec.columns[1].data_type = "TEXT".into();
        spec.columns[1].length = None;
        spec.columns[2].data_type = "DATETIME".into();
        spec.columns[3].default = ColumnDefault::Custom { expression: "lower(hex(randomblob(4)))".into() };
        assert_eq!(
            create(Engine::Sqlite, &spec),
            "CREATE TABLE \"users\" (\n  \"id\" INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,\n  \"email\" TEXT NOT NULL UNIQUE,\n  \"created_at\" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,\n  \"note\" text DEFAULT (lower(hex(randomblob(4))))\n)"
        );
        // A composite key cannot carry AUTOINCREMENT.
        spec.columns[1].primary_key = true;
        assert!(create(Engine::Sqlite, &spec).starts_with("ERR"));
        assert!(sqlite_default_is_literal("42") && sqlite_default_is_literal("-1.5") && sqlite_default_is_literal("'x'"));
        assert!(sqlite_default_is_literal("(1+1)") && sqlite_default_is_literal("current_date") && !sqlite_default_is_literal("abs(1)"));
    }

    #[test]
    fn mssql_uses_identity_and_guards_if_not_exists() {
        let mut spec = users();
        spec.table.schema = Some("dbo".into());
        spec.columns[0].data_type = "int".into();
        spec.columns[1].data_type = "nvarchar".into();
        spec.columns[2].data_type = "datetime2".into();
        spec.if_not_exists = true;
        assert_eq!(
            create(Engine::Mssql, &spec),
            "IF OBJECT_ID(N'[dbo].[users]', N'U') IS NULL\nCREATE TABLE [dbo].[users] (\n  [id] int IDENTITY(1,1) NOT NULL,\n  [email] nvarchar(255) NOT NULL UNIQUE,\n  [created_at] datetime2 NOT NULL DEFAULT GETDATE(),\n  [note] text NULL DEFAULT '',\n  PRIMARY KEY ([id])\n)"
        );
    }

    #[test]
    fn composite_keys_foreign_keys_and_actions() {
        let mut order_items = spec(
            table("order_items"),
            vec![pk("order_id", "integer"), pk("line", "integer"), {
                let mut c = col("sku", "varchar");
                c.length = Some(40);
                c.nullable = false;
                c
            }],
        );
        let mut order_fk = fk(&["order_id"], "orders", &["id"]);
        order_fk.on_delete = FkAction::Cascade;
        order_fk.on_update = FkAction::Restrict;
        order_fk.name = Some("fk_items_order".into());
        order_items.foreign_keys = vec![order_fk, fk(&["sku"], "products", &["sku"])];
        assert_eq!(
            create(Engine::Postgres, &order_items),
            "CREATE TABLE \"order_items\" (\n  \"order_id\" integer NOT NULL,\n  \"line\" integer NOT NULL,\n  \"sku\" varchar(40) NOT NULL,\n  PRIMARY KEY (\"order_id\", \"line\"),\n  CONSTRAINT \"fk_items_order\" FOREIGN KEY (\"order_id\") REFERENCES \"orders\" (\"id\") ON DELETE CASCADE ON UPDATE RESTRICT,\n  CONSTRAINT \"order_items_sku_fkey\" FOREIGN KEY (\"sku\") REFERENCES \"products\" (\"sku\")\n)"
        );
        // MySQL cannot SET DEFAULT; Oracle has no ON UPDATE; SQLite writes a bare parent name.
        let mut bad = order_items.clone();
        bad.foreign_keys[0].on_delete = FkAction::SetDefault;
        assert!(create(Engine::Mysql, &bad).starts_with("ERR"));
        bad.foreign_keys[0].on_delete = FkAction::Cascade;
        assert!(create(Engine::Oracle, &bad).starts_with("ERR"), "ON UPDATE RESTRICT has no Oracle form");
        let mut lite = order_items.clone();
        lite.foreign_keys[0].ref_table.schema = Some("main".into());
        assert!(create(Engine::Sqlite, &lite).contains("REFERENCES \"orders\" (\"id\") ON DELETE CASCADE ON UPDATE RESTRICT"));
        // Arity and unknown columns are refused before any SQL exists.
        let mut skew = order_items.clone();
        skew.foreign_keys[0].ref_columns.push("extra".into());
        assert!(create(Engine::Postgres, &skew).starts_with("ERR"));
        let mut ghost = order_items.clone();
        ghost.foreign_keys[0].columns = vec!["nope".into()];
        assert!(create(Engine::Postgres, &ghost).starts_with("ERR"));
    }

    #[test]
    fn indexes_follow_the_table_and_are_skipped_where_the_engine_has_none() {
        let mut t = users();
        t.table.schema = None;
        t.indexes = vec![IndexSpec { name: "users_note_idx".into(), columns: vec!["note".into(), "email".into()], unique: true }];
        let statements = super::super::create_table(Engine::Postgres, &t).unwrap_or_default();
        assert_eq!(statements.last().map(String::as_str), Some("CREATE UNIQUE INDEX \"users_note_idx\" ON \"users\" (\"note\", \"email\")"));
        let mysql = super::super::create_table(Engine::Mysql, &t).unwrap_or_default();
        assert_eq!(mysql.last().map(String::as_str), Some("CREATE UNIQUE INDEX `users_note_idx` ON `users` (`note`, `email`)"));
        let mut ch = spec(table("e"), vec![pk("id", "UInt64")]);
        ch.indexes = vec![IndexSpec { name: "i".into(), columns: vec!["id".into()], unique: false }];
        assert_eq!(super::super::create_table(Engine::Clickhouse, &ch).map(|s| s.len()).unwrap_or(0), 1);
    }

    #[test]
    fn clickhouse_gets_engine_order_by_and_nullable_types() {
        let mut events = spec(
            TableRef { schema: Some("analytics".into()), name: "events".into() },
            vec![pk("id", "UInt64"), {
                let mut c = col("name", "String");
                c.wrapper = Some("LowCardinality".into());
                c
            }, {
                let mut c = col("at", "DateTime64");
                c.precision = Some(3);
                c.nullable = false;
                c.default = ColumnDefault::Now;
                c
            }],
        );
        events.if_not_exists = true;
        assert_eq!(
            create(Engine::Clickhouse, &events),
            "CREATE TABLE IF NOT EXISTS \"analytics\".\"events\" (\n  \"id\" UInt64,\n  \"name\" LowCardinality(Nullable(String)),\n  \"at\" DateTime64(3) DEFAULT now()\n)\nENGINE = MergeTree\nORDER BY (\"id\")\nPRIMARY KEY (\"id\")"
        );
        events.engine = Some("ReplacingMergeTree".into());
        events.order_by = vec!["at".into(), "id".into()];
        assert!(create(Engine::Clickhouse, &events).ends_with("ENGINE = ReplacingMergeTree\nORDER BY (\"at\", \"id\")\nPRIMARY KEY (\"id\")"));
        events.columns[0].primary_key = false;
        events.order_by.clear();
        assert!(create(Engine::Clickhouse, &events).ends_with("ENGINE = ReplacingMergeTree\nORDER BY tuple()"));
        events.engine = Some("Memory".into());
        assert!(create(Engine::Clickhouse, &events).ends_with("\n)\nENGINE = Memory"));
        events.engine = Some("Mem ory; DROP".into());
        assert!(create(Engine::Clickhouse, &events).starts_with("ERR"));
        events.engine = None;
        events.order_by = vec!["ghost".into()];
        assert!(create(Engine::Clickhouse, &events).starts_with("ERR"));
        // Constraints ClickHouse cannot honour are refused, not dropped.
        let mut unique = spec(table("t"), vec![{
            let mut c = col("a", "String");
            c.unique = true;
            c
        }]);
        assert!(create(Engine::Clickhouse, &unique).starts_with("ERR"));
        unique.columns[0].unique = false;
        unique.columns[0].auto_increment = true;
        assert!(create(Engine::Clickhouse, &unique).starts_with("ERR"));
    }

    #[test]
    fn duckdb_auto_increment_goes_through_a_sequence() {
        let t = spec(table("people"), vec![{
            let mut c = pk("id", "INTEGER");
            c.auto_increment = true;
            c
        }, col("name", "VARCHAR")]);
        let statements = super::super::create_table(Engine::Duckdb, &t).unwrap_or_default();
        assert_eq!(statements.first().map(String::as_str), Some("CREATE SEQUENCE \"people_id_seq\""));
        assert_eq!(
            statements.get(1).map(String::as_str),
            Some("CREATE TABLE \"people\" (\n  \"id\" INTEGER DEFAULT nextval('\"people_id_seq\"') NOT NULL,\n  \"name\" VARCHAR,\n  PRIMARY KEY (\"id\")\n)")
        );
    }

    #[test]
    fn other_dialects_write_their_own_keys_and_identity() {
        let mut t = spec(table("t"), vec![{
            let mut c = pk("id", "NUMBER");
            c.auto_increment = true;
            c
        }, {
            let mut c = col("name", "VARCHAR2");
            c.length = Some(80);
            c.default = ColumnDefault::Now;
            c
        }]);
        t.columns[1].data_type = "VARCHAR2".into();
        assert_eq!(
            create(Engine::Oracle, &t),
            "CREATE TABLE \"t\" (\n  \"id\" NUMBER GENERATED BY DEFAULT AS IDENTITY,\n  \"name\" VARCHAR2(80) DEFAULT CURRENT_TIMESTAMP,\n  PRIMARY KEY (\"id\")\n)"
        );
        t.if_not_exists = true;
        assert!(create(Engine::Oracle, &t).starts_with("ERR"));

        let mut sf = spec(table("t"), vec![pk("id", "NUMBER"), col("at", "TIMESTAMP_NTZ")]);
        sf.columns[0].auto_increment = true;
        sf.columns[1].default = ColumnDefault::Now;
        assert_eq!(
            create(Engine::Snowflake, &sf),
            "CREATE TABLE \"t\" (\n  \"id\" NUMBER AUTOINCREMENT NOT NULL,\n  \"at\" TIMESTAMP_NTZ DEFAULT CURRENT_TIMESTAMP(),\n  PRIMARY KEY (\"id\")\n)"
        );

        let bq = spec(table("t"), vec![pk("id", "INT64"), {
            let mut c = col("name", "STRING");
            c.default = ColumnDefault::Custom { expression: "'x'".into() };
            c.nullable = false;
            c
        }]);
        assert_eq!(create(Engine::Bigquery, &bq), "CREATE TABLE `t` (\n  `id` INT64 NOT NULL,\n  `name` STRING DEFAULT 'x' NOT NULL,\n  PRIMARY KEY (`id`) NOT ENFORCED\n)");

        let cql = spec(TableRef { schema: Some("ks".into()), name: "t".into() }, vec![pk("id", "uuid"), col("name", "text")]);
        assert_eq!(create(Engine::Cassandra, &cql), "CREATE TABLE \"ks\".\"t\" (\n  \"id\" uuid,\n  \"name\" text,\n  PRIMARY KEY (\"id\")\n)");
        let mut no_pk = cql.clone();
        no_pk.columns[0].primary_key = false;
        assert!(create(Engine::Cassandra, &no_pk).starts_with("ERR"));
    }

    #[test]
    fn validation_catches_what_the_database_would_reject_late() {
        let mut t = users();
        t.columns.clear();
        assert!(create(Engine::Postgres, &t).starts_with("ERR"));
        let mut twice = users();
        twice.columns[1].name = "ID".into();
        assert!(create(Engine::Postgres, &twice).starts_with("ERR"), "duplicates ignore case");
        let mut text_auto = users();
        text_auto.columns[0].data_type = "text".into();
        assert!(create(Engine::Postgres, &text_auto).starts_with("ERR"));
        let mut empty = users();
        empty.table.name = " ".into();
        assert!(create(Engine::Postgres, &empty).starts_with("ERR"));
        let mut blank_default = users();
        blank_default.columns[3].default = ColumnDefault::Custom { expression: "  ".into() };
        assert!(create(Engine::Postgres, &blank_default).starts_with("ERR"));
        assert!(create(Engine::Redis, &users()).starts_with("ERR"));
        // DEFAULT NULL on a NOT NULL column is dropped rather than sent to MySQL.
        let mut null_default = users();
        null_default.columns[1].default = ColumnDefault::Null;
        assert!(!create(Engine::Mysql, &null_default).contains("DEFAULT NULL"));
    }
}
