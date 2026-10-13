// SOT: ddl-dialect, ddl-quoting, ddl-engine-to-dialect, ddl-transactional-engines

use crate::error::{AppError, AppResult};
use crate::integrations::sql::quote_literal;
use crate::integrations::{qualified_name_for, quote_ident_for};
use crate::model::{Engine, Family, TableRef};

// WHAT:  The statement languages the schema forms can write, and the engines that
//        speak each of them.
// WHY:   Forty-odd engines share eleven dialects (Supabase, Neon, Cockroach and
//        Timescale are all `Postgres`; MariaDB, TiDB and PlanetScale are `Mysql`).
//        The builders switch on the dialect, never on the engine, so a new
//        wire-compatible engine needs no change here.
// HOW:   `dialect_of` is the only mapping from `Engine` to `Dialect`; an engine it
//        answers None for is one the forms do not drive (key-value stores, search
//        engines, graph databases…) and every builder refuses it by name.
// WHERE: src-tauri/src/integrations/ddl/mod.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Mysql,
    Mssql,
    Oracle,
    Sqlite,
    Duckdb,
    Clickhouse,
    Snowflake,
    Bigquery,
    Cassandra,
    Mongo,
}

impl Dialect {
    pub fn label(self) -> &'static str {
        match self {
            Dialect::Postgres => "PostgreSQL",
            Dialect::Mysql => "MySQL",
            Dialect::Mssql => "SQL Server",
            Dialect::Oracle => "Oracle",
            Dialect::Sqlite => "SQLite",
            Dialect::Duckdb => "DuckDB",
            Dialect::Clickhouse => "ClickHouse",
            Dialect::Snowflake => "Snowflake",
            Dialect::Bigquery => "BigQuery",
            Dialect::Cassandra => "Cassandra",
            Dialect::Mongo => "MongoDB",
        }
    }
}

pub fn dialect_of(engine: Engine) -> Option<Dialect> {
    // QuestDB rides the Postgres wire protocol but has no keys, no ALTER COLUMN
    // and a designated timestamp: the generic Postgres statements would be wrong.
    if engine == Engine::Questdb {
        return None;
    }
    match engine.family() {
        Family::Postgres => Some(Dialect::Postgres),
        Family::Mysql => Some(Dialect::Mysql),
        Family::Mssql => Some(Dialect::Mssql),
        Family::Oracle => Some(Dialect::Oracle),
        Family::Sqlite | Family::Libsql | Family::ValTown | Family::CloudflareD1 => Some(Dialect::Sqlite),
        Family::Duckdb => Some(Dialect::Duckdb),
        Family::Clickhouse => Some(Dialect::Clickhouse),
        Family::Snowflake => Some(Dialect::Snowflake),
        Family::Bigquery => Some(Dialect::Bigquery),
        Family::Cassandra => Some(Dialect::Cassandra),
        Family::Mongodb => Some(Dialect::Mongo),
        _ => None,
    }
}

/// DDL can be rolled back as part of a transaction on this engine, so a script of
/// several statements is wrapped in one. Cloudflare D1 and Val Town run each
/// request on their own and take no `BEGIN`.
pub fn transactional_ddl(engine: Engine) -> bool {
    match dialect_of(engine) {
        Some(Dialect::Postgres | Dialect::Mssql | Dialect::Duckdb) => true,
        Some(Dialect::Sqlite) => !matches!(engine, Engine::CloudflareD1 | Engine::ValTown),
        _ => false,
    }
}

// WHAT:  An engine plus its dialect: what every builder carries around.
#[derive(Debug, Clone, Copy)]
pub struct Cx {
    pub engine: Engine,
    pub dialect: Dialect,
}

impl Cx {
    pub fn new(engine: Engine) -> AppResult<Cx> {
        match dialect_of(engine) {
            Some(dialect) => Ok(Cx { engine, dialect }),
            None => Err(AppError::invalid_input(format!(
                "{} is not driven through the schema forms; use its own console to change its structure.",
                engine.label()
            ))),
        }
    }

    /// An identifier quoted the engine's way, exactly as typed.
    pub fn q(&self, ident: &str) -> String {
        quote_ident_for(self.engine, ident)
    }

    /// `schema.table` (or just `table`), quoted.
    pub fn table(&self, table: &TableRef) -> String {
        qualified_name_for(self.engine, table)
    }

    /// A string literal.
    pub fn lit(&self, text: &str) -> String {
        quote_literal(text)
    }

    /// A refusal that names the engine: "DuckDB cannot add a foreign key to…".
    pub fn cannot(&self, what: &str) -> AppError {
        AppError::invalid_input(format!("{} cannot {what}.", self.dialect.label()))
    }

    /// A comma-separated, quoted column list.
    pub fn columns(&self, names: &[String]) -> String {
        names.iter().map(|n| self.q(n)).collect::<Vec<_>>().join(", ")
    }
}

/// The table with its schema dropped to `None` and its name swapped: the reference
/// to a table after a rename, or to the scratch table of a rebuild.
pub fn renamed(table: &TableRef, name: &str) -> TableRef {
    TableRef { schema: table.schema.clone().filter(|s| !s.is_empty()), name: name.to_string() }
}

/// A generated constraint / index / sequence name, kept inside the shortest
/// identifier limit the supported engines share (MySQL: 64).
pub fn derived_name(parts: &[&str]) -> String {
    let joined = parts.join("_");
    if joined.chars().count() <= 60 {
        return joined;
    }
    joined.chars().take(60).collect()
}

/// Whether two names are the same identifier for duplicate checks (engines differ
/// in case sensitivity; refusing case-insensitive duplicates is the safe superset).
pub fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Names a user may give a table, column, schema or database. The builders quote
/// every identifier, so this only refuses what no engine accepts.
pub fn check_name(kind: &str, name: &str) -> AppResult<()> {
    if name.trim().is_empty() {
        return Err(AppError::invalid_input(format!("A {kind} name is required.")));
    }
    if name != name.trim() {
        return Err(AppError::invalid_input(format!("The {kind} name \"{name}\" starts or ends with a space.")));
    }
    if name.chars().any(char::is_control) {
        return Err(AppError::invalid_input(format!("The {kind} name \"{name}\" contains a control character.")));
    }
    if name.chars().count() > 128 {
        return Err(AppError::invalid_input(format!("The {kind} name \"{}…\" is longer than 128 characters.", name.chars().take(20).collect::<String>())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_map_to_their_wire_dialect() {
        assert_eq!(dialect_of(Engine::Supabase), Some(Dialect::Postgres));
        assert_eq!(dialect_of(Engine::Cockroachdb), Some(Dialect::Postgres));
        assert_eq!(dialect_of(Engine::Mariadb), Some(Dialect::Mysql));
        assert_eq!(dialect_of(Engine::Tidb), Some(Dialect::Mysql));
        assert_eq!(dialect_of(Engine::CloudflareD1), Some(Dialect::Sqlite));
        assert_eq!(dialect_of(Engine::Libsql), Some(Dialect::Sqlite));
        assert_eq!(dialect_of(Engine::Scylladb), Some(Dialect::Cassandra));
        assert_eq!(dialect_of(Engine::Mongodb), Some(Dialect::Mongo));
        assert_eq!(dialect_of(Engine::Redis), None);
        assert_eq!(dialect_of(Engine::Questdb), None);
        assert!(Cx::new(Engine::Redis).is_err());
    }

    #[test]
    fn only_transactional_ddl_engines_get_a_wrapper() {
        assert!(transactional_ddl(Engine::Postgres));
        assert!(transactional_ddl(Engine::Sqlite));
        assert!(transactional_ddl(Engine::Libsql));
        assert!(!transactional_ddl(Engine::CloudflareD1));
        assert!(!transactional_ddl(Engine::ValTown));
        assert!(!transactional_ddl(Engine::Mysql));
        assert!(!transactional_ddl(Engine::Oracle));
    }

    #[test]
    fn identifiers_quote_per_dialect_and_names_are_checked() {
        let table = TableRef { schema: Some("app".into()), name: "us\"ers".into() };
        assert_eq!(Cx::new(Engine::Postgres).map(|c| c.table(&table)).unwrap_or_default(), "\"app\".\"us\"\"ers\"");
        assert_eq!(Cx::new(Engine::Mysql).map(|c| c.q("a`b")).unwrap_or_default(), "`a``b`");
        assert_eq!(Cx::new(Engine::Mssql).map(|c| c.q("a]b")).unwrap_or_default(), "[a]]b]");
        assert!(check_name("table", "").is_err());
        assert!(check_name("table", " x").is_err());
        assert!(check_name("table", "a\nb").is_err());
        assert!(check_name("table", "orders").is_ok());
        assert!(derived_name(&["a_very_long_table_name_that_goes_on", "and_on_and_on_and_on_and_on", "key"]).chars().count() <= 60);
    }
}
