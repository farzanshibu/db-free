// SOT: shell-meta-clickhouse, clickhouse-structure-aliases, glob-to-like-pattern

use super::args::{self, eq_ci, split_word, string_literal, Quote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand, Step};
use crate::error::{AppError, AppResult};

// ============================================================================
// CLICKHOUSE
//
// WHAT:  `\l`, `\dt [pattern]`, `\d [table]` and `\?` / `help`: a handful of
//        structure commands in the style of psql, plus a help listing of the
//        statements ClickHouse itself answers (`SHOW DATABASES`, `SHOW TABLES`,
//        `DESCRIBE TABLE`, `SHOW CREATE TABLE`, `EXISTS TABLE`).
// WHY:   `clickhouse-client` has no backslash commands of its own; they are
//        offered because people arrive from psql with `\dt` in their fingers,
//        and they are labelled as aliases in `help` so nobody mistakes them for
//        ClickHouse syntax. They are translated only when they start a statement,
//        which no ClickHouse statement does.
// HOW:   Each alias is one query on `system.databases` / `system.tables` /
//        `system.columns`, sent through the adapter's `execute`
//        (src-tauri/src/integrations/clickhouse.rs: `split_statements`, then
//        `run_statement`, one HTTP request per statement). Patterns use the
//        shell's wildcards: `*` any text, `?` one character, `db.table` to name
//        both; they become LIKE patterns whose own `%`, `_` and `\` are escaped.
//        Text literals escape backslash and quote (ClickHouse reads `\` in a
//        string as an escape), identifiers never reach SQL unquoted.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: Some("\\"),
    strict: true,
    help: &["\\?", "help"],
    cut: Cut::Statements { line_words: &["help"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("\\l", "\\l", "List databases (an alias in the style of psql)"),
    ShellCommand::shell("\\dt", "\\dt[+] [[database.]pattern]", "List tables of the current database, or those matching a pattern (an alias in the style of psql)"),
    ShellCommand::shell("\\d", "\\d [[database.]table]", "Describe a table's columns and keys; without a name, list tables (an alias in the style of psql)"),
    ShellCommand::shell("\\?", "\\?", "List the commands this console understands"),
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::native("SHOW DATABASES", "SHOW DATABASES [LIKE '%pattern%']", "List databases"),
    ShellCommand::native("SHOW TABLES", "SHOW TABLES [FROM database] [LIKE '%pattern%']", "List tables of a database"),
    ShellCommand::native("DESCRIBE TABLE", "DESCRIBE TABLE [database.]table", "Columns and types of a table"),
    ShellCommand::native("SHOW CREATE TABLE", "SHOW CREATE TABLE [database.]table", "The statement that creates a table"),
    ShellCommand::native("EXISTS TABLE", "EXISTS TABLE [database.]table", "Whether a table exists"),
    ShellCommand::native("SELECT * FROM system.tables", "SELECT * FROM system.tables WHERE database = 'db'", "Everything the server knows about its tables"),
    ShellCommand::native("SELECT * FROM system.parts", "SELECT * FROM system.parts WHERE active", "Data parts, sizes and partitions"),
];

fn lit(raw: &str) -> String {
    string_literal(raw, Quote::BackslashHex)
}

fn plan(ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    if eq_ci(text, "help") {
        return Some(Ok(help_plan(&DIALECT)));
    }
    let (head, rest) = split_word(text);
    let name = head.strip_prefix('\\')?;
    match name {
        "?" => Some(Ok(help_plan(&DIALECT))),
        "l" | "list" => Some(no_arguments(head, rest).map(|()| databases())),
        "dt" | "dt+" => Some(tables(ctx, head, rest)),
        "d" | "d+" => Some(describe(ctx, head, rest)),
        _ => None,
    }
}

fn no_arguments(head: &str, rest: &str) -> AppResult<()> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(AppError::invalid_input(format!("{head} takes no arguments.")))
    }
}

fn databases() -> Plan {
    Plan::native("SELECT name AS \"Database\", engine AS \"Engine\", data_path AS \"Data path\", comment AS \"Comment\" FROM system.databases ORDER BY name")
}

// WHAT:  The database a command without one looks at: the editor's picker, else the session's.
fn current_database(ctx: &Ctx) -> String {
    match ctx.namespace.map(str::trim).filter(|d| !d.is_empty()) {
        Some(picked) => lit(picked),
        None => "currentDatabase()".to_string(),
    }
}

// WHAT:  A shell wildcard pattern as the text of a LIKE pattern.
// HOW:   `*` becomes `%` and `?` becomes `_`; a literal `%`, `_` or `\` is escaped with a backslash,
//        which is how ClickHouse's LIKE reads an escape.
pub fn glob_to_like(glob: &str) -> String {
    let mut out = String::with_capacity(glob.len());
    for c in glob.chars() {
        match c {
            '*' => out.push('%'),
            '?' => out.push('_'),
            '%' | '_' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

// WHAT:  `\dt`: tables of one database, or of those matching `db.table` patterns.
fn tables(ctx: &Ctx, head: &str, rest: &str) -> AppResult<Plan> {
    let (database, name) = match pattern_parts(head, rest)? {
        None => (format!("database = {}", current_database(ctx)), None),
        Some((Some(db), name)) => (format!("database LIKE {}", lit(&glob_to_like(&db))), Some(name)),
        Some((None, name)) => (format!("database = {}", current_database(ctx)), Some(name)),
    };
    let name_filter = name.map_or_else(String::new, |n| format!(" AND name LIKE {}", lit(&glob_to_like(&n))));
    Ok(Plan::native(format!(
        "SELECT database AS \"Database\", name AS \"Name\", engine AS \"Engine\", total_rows AS \"Rows\", total_bytes AS \"Bytes\" \
         FROM system.tables WHERE {database} AND NOT is_temporary{name_filter} ORDER BY database, name"
    )))
}

/// `[db.]name` pieces of the one optional argument; None when there is none.
fn pattern_parts(head: &str, rest: &str) -> AppResult<Option<(Option<String>, String)>> {
    if rest.is_empty() {
        return Ok(None);
    }
    let parts = args::qualified(rest).map_err(AppError::invalid_input)?;
    if parts.iter().any(|p| !p.quoted && p.text.contains(char::is_whitespace)) {
        return Err(AppError::invalid_input(format!("{head} takes one pattern, e.g. {head} events_* or {head} analytics.events_*.")));
    }
    match parts.as_slice() {
        [name] => Ok(Some((None, name.text.clone()))),
        [database, name] => Ok(Some((Some(database.text.clone()), name.text.clone()))),
        _ => Err(AppError::invalid_input(format!("{head} takes name or database.name."))),
    }
}

// WHAT:  `\d table`: what the table is, then its columns with their roles in the keys.
fn describe(ctx: &Ctx, head: &str, rest: &str) -> AppResult<Plan> {
    let Some((database, name)) = pattern_parts(head, rest)? else {
        return tables(ctx, head, rest);
    };
    let database = database.map_or_else(|| current_database(ctx), |d| lit(&d));
    let name = lit(&name);
    let shown = rest.trim().trim_matches('"').to_string();
    let header = format!(
        "SELECT database AS \"Database\", name AS \"Name\", engine AS \"Engine\", total_rows AS \"Rows\", total_bytes AS \"Bytes\", \
         partition_key AS \"Partition key\", sorting_key AS \"Sorting key\", primary_key AS \"Primary key\", sampling_key AS \"Sampling key\", comment AS \"Comment\" \
         FROM system.tables WHERE database = {database} AND name = {name}"
    );
    let columns = format!(
        "SELECT name AS \"Column\", type AS \"Type\", default_kind AS \"Default kind\", default_expression AS \"Default\", comment AS \"Comment\", \
         compression_codec AS \"Codec\", is_in_partition_key AS \"In partition key\", is_in_sorting_key AS \"In sorting key\", is_in_primary_key AS \"In primary key\" \
         FROM system.columns WHERE database = {database} AND table = {name} ORDER BY position"
    );
    Ok(Plan { steps: vec![Step::Exists(header, format!("Did not find any table named \"{shown}\".")), Step::Section(columns)] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::destructive::{classify, StatementKind};
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Clickhouse), text)
    }

    fn statements(text: &str) -> Vec<String> {
        match planned(text) {
            Some(Ok(plan)) => plan.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn error(text: &str) -> String {
        match planned(text) {
            Some(Err(err)) => err.message().to_string(),
            other => panic!("{text}: expected an error, got {other:?}"),
        }
    }

    #[test]
    fn every_command_is_recognised_with_or_without_a_terminator() {
        for text in ["\\l", "\\l;", "\\list", "\\dt", "\\dt+", "\\dt events_*", "\\dt analytics.events;", "\\d", "\\d events", "\\d analytics.events", "\\?", "help", "HELP;"] {
            assert!(matches!(planned(text), Some(Ok(_))), "{text}: {:?}", planned(text));
        }
    }

    #[test]
    fn clickhouse_statements_are_not_intercepted() {
        for text in ["SHOW DATABASES", "SHOW TABLES FROM db", "DESCRIBE TABLE t", "SELECT 1", "EXISTS TABLE t", "helper"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn an_unknown_backslash_command_is_refused_not_sent_on() {
        assert!(error("\\copy t from x").contains("does not know the command \\copy"));
        assert!(error("\\x").contains("\\?"));
    }

    #[test]
    fn databases_come_from_system_databases() {
        let sql = statements("\\l");
        assert!(sql[0].contains("FROM system.databases ORDER BY name"), "{}", sql[0]);
        assert!(error("\\l extra").contains("takes no arguments"));
    }

    #[test]
    fn tables_default_to_the_current_database_or_the_pickers() {
        let sql = statements("\\dt");
        assert!(sql[0].contains("WHERE database = currentDatabase() AND NOT is_temporary ORDER BY database, name"), "{}", sql[0]);
        assert_eq!(statements("\\d"), sql, "\\d without a name lists tables");
        let Some(Ok(plan)) = run_plan(&Ctx::new(Engine::Clickhouse, None, Some("web")), "\\dt") else { panic!("not planned") };
        assert!(plan.statements().next().is_some_and(|s| s.contains("WHERE database = 'web' AND")));
    }

    #[test]
    fn patterns_use_shell_wildcards_and_escape_like() {
        let sql = statements("\\dt ev*_?");
        assert!(sql[0].contains("AND name LIKE 'ev%\\\\__'"), "{}", sql[0]);
        assert_eq!(glob_to_like("ev*_?"), "ev%\\__");
        assert_eq!(glob_to_like("100%"), "100\\%");
        assert_eq!(glob_to_like("a\\b"), "a\\\\b");
        let qualified = statements("\\dt web.ev*");
        assert!(qualified[0].contains("database LIKE 'web' AND NOT is_temporary AND name LIKE 'ev%'"), "{}", qualified[0]);
        assert!(error("\\dt a b").contains("takes one pattern"));
        assert!(error("\\dt a.b.c").contains("database.name"));
    }

    #[test]
    fn describe_reads_the_table_and_its_columns() {
        let Some(Ok(plan)) = planned("\\d web.events") else { panic!("not planned") };
        assert!(matches!(&plan.steps[0], Step::Exists(sql, message)
            if sql.contains("FROM system.tables WHERE database = 'web' AND name = 'events'") && message == "Did not find any table named \"web.events\"."));
        assert!(matches!(&plan.steps[1], Step::Section(sql)
            if sql.contains("FROM system.columns WHERE database = 'web' AND table = 'events' ORDER BY position") && sql.contains("is_in_primary_key")));
        let bare = statements("\\d events");
        assert!(bare[0].contains("database = currentDatabase() AND name = 'events'"), "{}", bare[0]);
    }

    #[test]
    fn user_text_stays_inside_a_literal() {
        let sql = statements("\\d \"t' OR 1=1 --\"");
        assert!(sql[0].contains("name = 't\\x27 OR 1=1 --'"), "{}", sql[0]);
        let backslash = statements("\\d \"a\\\\\"");
        assert!(backslash[0].contains("name = 'a\\\\\\\\'"), "a backslash is doubled so it cannot escape the closing quote: {}", backslash[0]);
        let pattern = statements("\\dt \"x'; DROP TABLE t; --\"");
        assert!(pattern[0].contains("AND name LIKE 'x\\x27; DROP TABLE t; --'"), "{}", pattern[0]);
    }

    #[test]
    fn the_pickers_database_is_quoted_like_any_other_text() {
        let picked = run_plan(&Ctx::new(Engine::Clickhouse, None, Some("we'ird\\db")), "\\dt");
        let Some(Ok(plan)) = picked else { panic!("not planned") };
        assert!(plan.statements().next().is_some_and(|s| s.contains("WHERE database = 'we\\x27ird\\\\db' AND")), "{:?}", plan.statements().next());
    }

    #[test]
    fn every_statement_is_a_read() {
        for text in ["\\l", "\\dt", "\\dt web.*", "\\d", "\\d web.events"] {
            for sql in statements(text) {
                let kinds: Vec<StatementKind> = classify(&sql).into_iter().map(|s| s.kind).collect();
                assert_eq!(kinds, vec![StatementKind::Read], "{text}: {sql}");
            }
        }
    }

    #[test]
    fn commands_and_sql_keep_their_order() {
        let found = segments(Engine::Clickhouse, "\\dt\nSELECT 1;\nhelp");
        assert_eq!(found, vec![Segment::Meta("\\dt".to_string()), Segment::Native("SELECT 1".to_string()), Segment::Meta("help".to_string())]);
    }

    #[test]
    fn help_lists_every_command() {
        for text in ["help", "\\?"] {
            let Some(Ok(plan)) = planned(text) else { panic!("no help") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()));
        }
    }
}
