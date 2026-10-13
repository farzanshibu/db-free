// SOT: shell-meta-sqlite, sqlite3-dot-commands, sqlite-catalog-queries

use super::args::{raw_args, split_word, string_literal, unquote, Quote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};
use crate::integrations::quote_ident;
use crate::model::Family;

// ============================================================================
// SQLITE FAMILY: the sqlite3 shell's dot-commands
//
// WHAT:  `.tables`, `.schema`, `.indexes`, `.databases`, `.fullschema`, `.dbinfo`
//        and `.help`, for SQLite, SpatiaLite, libSQL/Turso, Cloudflare D1 and
//        Val Town; the same words (translated to DuckDB's own catalog) for DuckDB.
// WHY:   The sqlite3 command-line shell is how most people look at a SQLite
//        file. Its dot-commands are not SQL — the engine would reject `.tables` —
//        but each is a question the catalog (`sqlite_master`, the PRAGMA table
//        functions) answers.
// HOW:   The first word picks the command; options (`--nosys`, `--indent`) and one
//        argument follow. The argument is a LIKE pattern, as in sqlite3, and is
//        passed as one string literal. A schema prefix (`aux.users`) is quoted as
//        an identifier. What the hosted engines (D1, Val Town) do not allow
//        (`PRAGMA database_list`, `.dbinfo`) is left out of their command list.
// WHERE: src-tauri/src/integrations/shell_meta/duckdb.rs (reuses `parse`),
//        src-tauri/src/integrations/shell_meta/mod.rs (dispatch)
// ============================================================================

const SHARED: [ShellCommand; 3] = [
    ShellCommand::shell(".tables", ".tables [PATTERN]", "List tables and views whose name matches the LIKE pattern"),
    ShellCommand::shell(".schema", ".schema [--nosys] [PATTERN]", "Show the CREATE statements of the tables, views, indexes and triggers matching the pattern"),
    ShellCommand::shell(".indexes", ".indexes [TABLE]", "List indexes, with their table and columns"),
];

const SQLITE_COMMANDS: &[ShellCommand] = &[
    SHARED[0],
    SHARED[1],
    SHARED[2],
    ShellCommand::shell(".databases", ".databases", "List the attached databases and their files"),
    ShellCommand::shell(".fullschema", ".fullschema", "Show the whole schema, internal tables included"),
    ShellCommand::shell(".dbinfo", ".dbinfo", "Show the database file's page size, encoding, versions and object counts"),
    ShellCommand::shell(".help", ".help", "List the commands this console understands"),
];

const LIBSQL_COMMANDS: &[ShellCommand] = &[
    SHARED[0],
    SHARED[1],
    SHARED[2],
    ShellCommand::shell(".databases", ".databases", "List the attached databases and their files"),
    ShellCommand::shell(".fullschema", ".fullschema", "Show the whole schema, internal tables included"),
    ShellCommand::shell(".help", ".help", "List the commands this console understands"),
];

const HOSTED_COMMANDS: &[ShellCommand] = &[SHARED[0], SHARED[1], SHARED[2], ShellCommand::shell(".help", ".help", "List the commands this console understands")];

pub const SQLITE: Dialect = Dialect { prefix: Some("."), strict: true, help: &[".help"], cut: Cut::Statements { line_words: &[] }, commands: SQLITE_COMMANDS, plan };
pub const LIBSQL: Dialect = Dialect { commands: LIBSQL_COMMANDS, ..SQLITE };
pub const HOSTED: Dialect = Dialect { commands: HOSTED_COMMANDS, ..SQLITE };

/// A dot-command, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Command {
    Tables(Option<String>),
    Schema { pattern: Option<String>, nosys: bool, full: bool },
    Indexes(Option<String>),
    Databases,
    DbInfo,
    Help,
}

// WHAT:  One dot-command line as a `Command`; None when the word is not one of ours.
// HOW:   Options come first (`--nosys`, `--indent`, accepted and ignored: the result
//        grid does its own layout), then at most one pattern. Quotes around the
//        pattern are removed, as sqlite3 does.
pub(super) fn parse(text: &str) -> Option<Result<Command, String>> {
    let (head, rest) = split_word(text);
    let word = head.strip_prefix('.')?.to_ascii_lowercase();
    let wanted = matches!(word.as_str(), "tables" | "schema" | "indexes" | "databases" | "fullschema" | "dbinfo" | "help");
    if !wanted {
        return None;
    }
    let raw = match raw_args(rest) {
        Ok(raw) => raw,
        Err(message) => return Some(Err(message)),
    };
    let mut nosys = false;
    let mut patterns: Vec<String> = Vec::new();
    for arg in raw {
        match arg.as_str() {
            "--nosys" | "-nosys" if word == "schema" || word == "fullschema" => nosys = true,
            "--indent" | "-indent" if word == "schema" || word == "fullschema" => {}
            flag if flag.starts_with('-') && flag.len() > 1 && !flag.starts_with("-\"") => {
                return Some(Err(format!("Unknown option {flag} for .{word}.")));
            }
            other => patterns.push(unquote(other)),
        }
    }
    let takes_pattern = matches!(word.as_str(), "tables" | "schema" | "indexes");
    if (!takes_pattern && !patterns.is_empty()) || patterns.len() > 1 {
        return Some(Err(if takes_pattern { format!(".{word} takes one pattern; quote it if it contains spaces.") } else { format!(".{word} takes no arguments.") }));
    }
    let pattern = patterns.pop();
    Some(Ok(match word.as_str() {
        "tables" => Command::Tables(pattern),
        "schema" => Command::Schema { pattern, nosys, full: false },
        "fullschema" => Command::Schema { pattern: None, nosys, full: true },
        "indexes" => Command::Indexes(pattern),
        "databases" => Command::Databases,
        "dbinfo" => Command::DbInfo,
        _ => Command::Help,
    }))
}

fn dialect_of(family: Family) -> &'static Dialect {
    match family {
        Family::Sqlite => &SQLITE,
        Family::Libsql => &LIBSQL,
        _ => &HOSTED,
    }
}

fn plan(ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let family = ctx.family();
    let dialect = dialect_of(family);
    let command = match parse(text)? {
        Ok(command) => command,
        Err(message) => return Some(Err(AppError::invalid_input(message))),
    };
    // A command the engine's shell has but this engine's SQL cannot answer is said so, not sent.
    let listed = |name: &str| dialect.commands.iter().any(|c| c.name == name);
    let unavailable = |name: &str| Some(Err(AppError::invalid_input(format!("{name} is not available over this connection."))));
    Some(Ok(match command {
        Command::Help => help_plan(dialect),
        Command::Tables(pattern) => Plan::native(tables(pattern.as_deref(), family)),
        Command::Schema { pattern, nosys, full } => {
            if full && !listed(".fullschema") {
                return unavailable(".fullschema");
            }
            Plan::native(schema(pattern.as_deref(), nosys))
        }
        Command::Indexes(pattern) => Plan::native(indexes(pattern.as_deref())),
        Command::Databases => {
            if !listed(".databases") {
                return unavailable(".databases");
            }
            Plan::native("SELECT seq AS \"seq\", name AS \"name\", file AS \"file\" FROM pragma_database_list ORDER BY seq")
        }
        Command::DbInfo => {
            if !listed(".dbinfo") {
                return unavailable(".dbinfo");
            }
            Plan::native(dbinfo())
        }
    }))
}

fn lit(raw: &str) -> String {
    string_literal(raw, Quote::Ansi)
}

/// sqlite3 hides its own bookkeeping tables (`sqlite_sequence`, `sqlite_stat1`…) from `.tables`.
const NOT_INTERNAL: &str = "name NOT LIKE 'sqlite\\_%' ESCAPE '\\'";

fn tables(pattern: Option<&str>, family: Family) -> String {
    let mut conds = vec!["type IN ('table', 'view')".to_string(), NOT_INTERNAL.to_string()];
    if family == Family::CloudflareD1 {
        conds.push("name NOT LIKE '\\_cf\\_%' ESCAPE '\\'".to_string());
    }
    if let Some(pattern) = pattern {
        conds.push(format!("name LIKE {}", lit(pattern)));
    }
    let filter = conds.join(" AND ");
    let temp = if family == Family::CloudflareD1 { String::new() } else { format!("\nUNION ALL\nSELECT 'temp.' || name, type FROM sqlite_temp_master WHERE {filter}") };
    format!("SELECT name AS \"name\", type AS \"type\" FROM sqlite_master WHERE {filter}{temp}\nORDER BY 1")
}

// WHAT:  `.schema [PATTERN]`: the CREATE statements of the objects whose table matches.
// HOW:   Like sqlite3: `lower(tbl_name) LIKE lower(pattern)`, ordered as they were created,
//        each statement ending in `;`. `aux.users` reads the attached database `aux`.
//        The internal tables are included unless `--nosys`, as in sqlite3.
fn schema(pattern: Option<&str>, nosys: bool) -> String {
    let (database, table) = match pattern {
        Some(pattern) => match pattern.split_once('.') {
            Some((db, rest)) if !rest.is_empty() && !db.is_empty() && db.chars().all(|c| c.is_alphanumeric() || c == '_') => (Some(db), Some(rest)),
            _ => (None, Some(pattern)),
        },
        None => (None, None),
    };
    let master = |schema: Option<&str>, table: &str| match schema {
        Some(db) => format!("{}.{table}", quote_ident(db)),
        None => table.to_string(),
    };
    let mut conds = vec!["sql IS NOT NULL".to_string()];
    if nosys {
        conds.push(NOT_INTERNAL.to_string());
    }
    if let Some(table) = table {
        conds.push(format!("lower(tbl_name) LIKE lower({}) ESCAPE '\\'", lit(table)));
    }
    let filter = conds.join(" AND ");
    let main = master(database, "sqlite_master");
    if database.is_none() {
        format!("SELECT sql || ';' AS \"sql\" FROM (SELECT sql, tbl_name, name, rowid AS x, 0 AS part FROM {main} UNION ALL SELECT sql, tbl_name, name, rowid, 1 FROM sqlite_temp_master) WHERE {filter} ORDER BY part, x")
    } else {
        format!("SELECT sql || ';' AS \"sql\" FROM {main} WHERE {filter} ORDER BY rowid")
    }
}

fn indexes(table: Option<&str>) -> String {
    let filter = table.map_or(String::new(), |t| format!(" AND m.tbl_name LIKE {}", lit(t)));
    format!(
        "SELECT m.name AS \"index\", m.tbl_name AS \"table\", (SELECT group_concat(i.name, ', ') FROM pragma_index_info(m.name) i) AS \"columns\", (SELECT CASE l.\"unique\" WHEN 1 THEN 'yes' ELSE 'no' END FROM pragma_index_list(m.tbl_name) l WHERE l.name = m.name) AS \"unique\", m.sql AS \"sql\" FROM sqlite_master m WHERE m.type = 'index'{filter}\nORDER BY 2, 1"
    )
}

fn dbinfo() -> String {
    let rows = [
        ("database page size", "SELECT page_size FROM pragma_page_size"),
        ("database page count", "SELECT page_count FROM pragma_page_count"),
        ("freelist page count", "SELECT freelist_count FROM pragma_freelist_count"),
        ("schema cookie", "SELECT schema_version FROM pragma_schema_version"),
        ("text encoding", "SELECT encoding FROM pragma_encoding"),
        ("auto vacuum", "SELECT auto_vacuum FROM pragma_auto_vacuum"),
        ("journal mode", "SELECT journal_mode FROM pragma_journal_mode"),
        ("user version", "SELECT user_version FROM pragma_user_version"),
        ("application id", "SELECT application_id FROM pragma_application_id"),
        ("software version", "SELECT sqlite_version()"),
        ("number of tables", "SELECT count(*) FROM sqlite_master WHERE type = 'table'"),
        ("number of indexes", "SELECT count(*) FROM sqlite_master WHERE type = 'index'"),
        ("number of views", "SELECT count(*) FROM sqlite_master WHERE type = 'view'"),
        ("number of triggers", "SELECT count(*) FROM sqlite_master WHERE type = 'trigger'"),
    ];
    let selects: Vec<String> = rows
        .iter()
        .enumerate()
        .map(|(n, (key, value))| format!("SELECT {} AS \"key\", CAST(({value}) AS TEXT) AS \"value\", {n} AS \"n\"", lit(key)))
        .collect();
    format!("SELECT \"key\", \"value\" FROM (\n{}\n) ORDER BY \"n\"", selects.join("\nUNION ALL\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{plan as plan_for, Step};
    use crate::model::Engine;

    fn statements(engine: Engine, text: &str) -> Vec<String> {
        match plan_for(&Ctx::bare(engine), text) {
            Some(Ok(plan)) => plan.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn first(engine: Engine, text: &str) -> String {
        statements(engine, text).into_iter().next().unwrap_or_default()
    }

    fn refused(engine: Engine, text: &str) -> String {
        match plan_for(&Ctx::bare(engine), text) {
            Some(Err(e)) => e.message().to_string(),
            other => panic!("{text}: expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn commands_are_read_with_their_options_and_pattern() {
        assert_eq!(parse(".tables"), Some(Ok(Command::Tables(None))));
        assert_eq!(parse(".TABLES  us%"), Some(Ok(Command::Tables(Some("us%".into())))));
        assert_eq!(parse(".tables 'my table'"), Some(Ok(Command::Tables(Some("my table".into())))));
        assert_eq!(parse(".schema --nosys --indent users"), Some(Ok(Command::Schema { pattern: Some("users".into()), nosys: true, full: false })));
        assert_eq!(parse(".fullschema --indent"), Some(Ok(Command::Schema { pattern: None, nosys: false, full: true })));
        assert_eq!(parse(".indexes users"), Some(Ok(Command::Indexes(Some("users".into())))));
        assert_eq!(parse(".databases"), Some(Ok(Command::Databases)));
        assert_eq!(parse(".help"), Some(Ok(Command::Help)));
        assert_eq!(parse(".mode csv"), None, "other dot-commands are not structure commands");
        assert_eq!(parse("select 1"), None);
        assert!(matches!(parse(".tables a b"), Some(Err(_))));
        assert!(matches!(parse(".databases x"), Some(Err(_))));
        assert!(matches!(parse(".schema --bogus"), Some(Err(m)) if m.contains("--bogus")));
        assert!(matches!(parse(".tables 'open"), Some(Err(_))));
    }

    #[test]
    fn tables_filter_internal_tables_and_take_a_like_pattern() {
        let sql = first(Engine::Sqlite, ".tables");
        assert!(sql.contains("FROM sqlite_master") && sql.contains("sqlite_temp_master"), "{sql}");
        assert!(sql.contains("name NOT LIKE 'sqlite\\_%' ESCAPE '\\'"), "{sql}");
        let with_pattern = first(Engine::Sqlite, ".tables us%");
        assert!(with_pattern.contains("name LIKE 'us%'"), "{with_pattern}");
        let d1 = first(Engine::CloudflareD1, ".tables");
        assert!(d1.contains("_cf\\_%") && !d1.contains("sqlite_temp_master"), "D1 hides its internal tables and has no temp schema: {d1}");
    }

    #[test]
    fn a_hostile_pattern_stays_inside_its_literal() {
        let sql = first(Engine::Sqlite, ".tables 'x''; DROP TABLE users; --'");
        assert!(sql.contains("name LIKE 'x''; DROP TABLE users; --'"), "{sql}");
        assert!(matches!(plan_for(&Ctx::bare(Engine::Sqlite), ".tables x'; DROP TABLE users; --"), Some(Err(_))), "an unclosed quote is refused, as in sqlite3");
        assert!(crate::guard::destructive::classify(&sql).iter().all(|s| s.kind == crate::guard::destructive::StatementKind::Read));
        // Only a plain name before the dot is a schema; anything else stays part of the pattern.
        let schema = first(Engine::Sqlite, ".schema \"a'b\".x");
        assert!(schema.contains("lower('\"a''b\".x')") && !schema.contains("\"a'b\".sqlite_master"), "{schema}");
        let evil = first(Engine::Sqlite, ".schema 'aux\"; select 1; --.t'");
        assert!(evil.contains("lower('aux\"; select 1; --.t')") && !evil.contains(".sqlite_master WHERE"), "{evil}");
        let attached = first(Engine::Sqlite, ".schema aux.t");
        assert!(attached.contains("\"aux\".sqlite_master"), "{attached}");
    }

    #[test]
    fn schema_orders_by_creation_and_can_skip_the_internal_tables() {
        let all = first(Engine::Sqlite, ".schema");
        assert!(all.contains("sql || ';'") && all.contains("ORDER BY part, x") && !all.contains("NOT LIKE"), "{all}");
        let user = first(Engine::Sqlite, ".schema --nosys users");
        assert!(user.contains("NOT LIKE 'sqlite\\_%'") && user.contains("lower(tbl_name) LIKE lower('users')"), "{user}");
        let attached = first(Engine::Sqlite, ".schema aux.users");
        assert!(attached.contains("\"aux\".sqlite_master") && attached.contains("lower('users')"), "{attached}");
    }

    #[test]
    fn indexes_databases_and_dbinfo() {
        let idx = first(Engine::Sqlite, ".indexes users");
        assert!(idx.contains("pragma_index_info(m.name)") && idx.contains("m.tbl_name LIKE 'users'"), "{idx}");
        assert!(first(Engine::Libsql, ".databases").contains("pragma_database_list"));
        let info = first(Engine::Sqlite, ".dbinfo");
        assert!(info.contains("pragma_page_size") && info.contains("sqlite_version()") && info.contains("ORDER BY \"n\""), "{info}");
    }

    #[test]
    fn what_a_hosted_engine_cannot_do_is_refused_not_sent() {
        for engine in [Engine::CloudflareD1, Engine::ValTown] {
            assert!(refused(engine, ".databases").contains("not available"));
            assert!(refused(engine, ".dbinfo").contains("not available"));
            assert!(refused(engine, ".fullschema").contains("not available"));
        }
        assert!(refused(Engine::Libsql, ".dbinfo").contains("not available"));
        assert!(!statements(Engine::Libsql, ".fullschema").is_empty());
    }

    #[test]
    fn every_statement_is_a_read_and_help_lists_the_family_commands() {
        use crate::guard::destructive::{classify, StatementKind};
        for engine in [Engine::Sqlite, Engine::Spatialite, Engine::Libsql, Engine::CloudflareD1, Engine::ValTown] {
            let dialect = dialect_of(engine.family());
            for command in dialect.commands.iter().filter(|c| c.name != ".help") {
                for text in [command.name.to_string(), format!("{} some%", command.name)] {
                    if text.contains(' ') && matches!(command.name, ".databases" | ".fullschema" | ".dbinfo") {
                        continue;
                    }
                    for sql in statements(engine, &text) {
                        assert!(classify(&sql).iter().all(|s| s.kind == StatementKind::Read), "{engine:?} {text}: {sql}");
                    }
                }
            }
            let Some(Ok(help)) = plan_for(&Ctx::bare(engine), ".help") else { panic!("no help") };
            assert!(matches!(help.steps.as_slice(), [Step::Table(t)] if t.rows.len() == dialect.commands.len()));
        }
    }
}
