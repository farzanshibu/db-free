// SOT: shell-meta-duckdb, duckdb-dot-commands, duckdb-catalog-queries

use super::args::{string_literal, Quote};
use super::sqlite::{parse, Command};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// DUCKDB: the duckdb shell's dot-commands
//
// WHAT:  `.tables`, `.schema`, `.indexes`, `.databases` and `.help`, answered
//        from DuckDB's own catalog functions.
// WHY:   DuckDB's command-line shell is a fork of sqlite3's and keeps its
//        dot-commands. DuckDB has no `sqlite_master` with the real catalog, but
//        `duckdb_tables()`, `duckdb_views()`, `duckdb_indexes()` and friends
//        carry more than it ever did (schemas, attached databases, the SQL).
// HOW:   The words and options are read by the SQLite dialect's parser; only the
//        statements differ. The argument is a LIKE pattern, passed as one literal.
// WHERE: src-tauri/src/integrations/shell_meta/sqlite.rs (`parse`)
// ============================================================================

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell(".tables", ".tables [PATTERN]", "List tables and views whose name matches the LIKE pattern"),
    ShellCommand::shell(".schema", ".schema [PATTERN]", "Show the CREATE statements of the tables, views, indexes and sequences matching the pattern"),
    ShellCommand::shell(".indexes", ".indexes [TABLE]", "List indexes, with their table and expressions"),
    ShellCommand::shell(".databases", ".databases", "List the attached databases and their files"),
    ShellCommand::shell(".help", ".help", "List the commands this console understands"),
];

pub const DIALECT: Dialect = Dialect { prefix: Some("."), strict: true, help: &[".help"], cut: Cut::Statements { line_words: &[] }, commands: COMMANDS, plan };

fn lit(raw: &str) -> String {
    string_literal(raw, Quote::Ansi)
}

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let command = match parse(text)? {
        Ok(command) => command,
        Err(message) => return Some(Err(AppError::invalid_input(message))),
    };
    Some(Ok(match command {
        Command::Help => help_plan(&DIALECT),
        Command::Tables(pattern) => Plan::native(tables(pattern.as_deref())),
        Command::Schema { pattern, full: false, .. } => Plan::native(schema(pattern.as_deref())),
        Command::Indexes(pattern) => Plan::native(indexes(pattern.as_deref())),
        Command::Databases => Plan::native("SELECT database_name AS \"name\", path AS \"file\", type AS \"type\", readonly AS \"readonly\" FROM duckdb_databases() WHERE NOT internal ORDER BY 1"),
        Command::Schema { full: true, .. } => return Some(Err(AppError::invalid_input(".fullschema is not available in DuckDB; use .schema."))),
        Command::DbInfo => return Some(Err(AppError::invalid_input(".dbinfo is not available in DuckDB; try PRAGMA database_size."))),
    }))
}

fn like(column: &str, pattern: Option<&str>) -> String {
    pattern.map_or(String::new(), |p| format!(" AND {column} LIKE {}", lit(p)))
}

// WHAT:  Tables and views of every attached database, the user-made ones only. A
//        name is qualified only where the shell would qualify it: outside `main`,
//        or in a database other than the one in use.
fn tables(pattern: Option<&str>) -> String {
    format!(
        "SELECT CASE WHEN database_name <> current_database() THEN database_name || '.' || schema_name || '.' || name WHEN schema_name <> 'main' THEN schema_name || '.' || name ELSE name END AS \"name\", type AS \"type\"\nFROM (\n  SELECT database_name, schema_name, table_name AS name, 'table' AS type FROM duckdb_tables() WHERE NOT internal{}\n  UNION ALL\n  SELECT database_name, schema_name, view_name AS name, 'view' AS type FROM duckdb_views() WHERE NOT internal{}\n)\nORDER BY 1",
        like("table_name", pattern),
        like("view_name", pattern)
    )
}

fn schema(pattern: Option<&str>) -> String {
    format!(
        "SELECT sql || ';' AS \"sql\"\nFROM (\n  SELECT database_name, schema_name, table_name AS name, sql, 0 AS part FROM duckdb_tables() WHERE NOT internal{}\n  UNION ALL\n  SELECT database_name, schema_name, view_name, sql, 1 FROM duckdb_views() WHERE NOT internal{}\n  UNION ALL\n  SELECT database_name, schema_name, index_name, sql, 2 FROM duckdb_indexes() WHERE sql IS NOT NULL{}\n  UNION ALL\n  SELECT database_name, schema_name, sequence_name, sql, 3 FROM duckdb_sequences() WHERE TRUE{}\n)\nWHERE sql IS NOT NULL\nORDER BY part, database_name, schema_name, name",
        like("table_name", pattern),
        like("view_name", pattern),
        like("table_name", pattern),
        like("sequence_name", pattern)
    )
}

fn indexes(table: Option<&str>) -> String {
    format!(
        "SELECT index_name AS \"index\", table_name AS \"table\", schema_name AS \"schema\", expressions AS \"expressions\", CASE WHEN is_unique THEN 'yes' ELSE 'no' END AS \"unique\", CASE WHEN is_primary THEN 'yes' ELSE 'no' END AS \"primary\", sql AS \"sql\"\nFROM duckdb_indexes()\nWHERE TRUE{}\nORDER BY 2, 1",
        like("table_name", table)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::plan as plan_for;
    use crate::model::Engine;

    fn first(text: &str) -> String {
        match plan_for(&Ctx::bare(Engine::Duckdb), text) {
            Some(Ok(plan)) => plan.statements().next().unwrap_or_default().to_string(),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn dot_commands_become_catalog_function_queries() {
        let tables = first(".tables");
        assert!(tables.contains("duckdb_tables()") && tables.contains("duckdb_views()") && tables.contains("WHERE NOT internal"), "{tables}");
        assert!(first(".tables ord%").contains("table_name LIKE 'ord%'") && first(".tables ord%").contains("view_name LIKE 'ord%'"));
        assert!(first(".schema users").contains("duckdb_indexes()") && first(".schema users").contains("sql || ';'"));
        assert!(first(".indexes t").contains("table_name LIKE 't'"));
        assert!(first(".databases").contains("duckdb_databases()"));
    }

    #[test]
    fn a_pattern_cannot_leave_its_literal() {
        let sql = first(".tables 'x''); drop table t; --'");
        assert!(sql.matches("LIKE 'x''); drop table t; --'").count() == 2, "{sql}");
        assert!(crate::guard::destructive::classify(&sql).iter().all(|s| s.kind == crate::guard::destructive::StatementKind::Read));
    }

    #[test]
    fn sqlite_only_commands_are_refused_with_a_pointer() {
        for text in [".fullschema", ".dbinfo"] {
            assert!(matches!(plan_for(&Ctx::bare(Engine::Duckdb), text), Some(Err(_))), "{text}");
        }
    }
}
