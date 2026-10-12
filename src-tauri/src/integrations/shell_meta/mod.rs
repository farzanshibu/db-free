// SOT: shell-meta-commands, shell-command-registry, shell-meta-segmenting, shell-meta-dispatch, shell-meta-help, shell-meta-ts-export

use crate::error::{AppError, AppResult};
use crate::guard::destructive::scan_ranges;
use crate::model::{ColumnMeta, ConnectionSummary, Engine, Family, ResultSet, StatementResult, Value};
use serde::Serialize;
use ts_rs::TS;

pub mod args;

mod arangodb;
mod basex;
mod bigquery;
mod cassandra;
mod chroma;
mod clickhouse;
mod couchdb;
mod convex;
mod druid;
mod duckdb;
mod dynamodb;
mod elasticsearch;
mod existdb;
mod firestore;
mod hbase;
mod immudb;
mod influxdb;
mod kafka;
mod meilisearch;
mod memcached;
mod milvus;
mod mongodb;
mod mssql;
mod mysql;
mod neo4j;
mod objectdb;
mod oracle;
mod orientdb;
mod pinecone;
mod prometheus;
mod psql;
mod qdrant;
mod qldb;
mod rabbitmq;
mod redis;
mod rocksdb;
mod s3;
mod snowflake;
mod spacetimedb;
mod sparql;
mod sqlite;
mod surrealdb;
mod tigergraph;
mod typesense;
mod weaviate;

// ============================================================================
// SHELL META-COMMANDS
//
// WHAT:  Lets every database take, in its query tab, the structure commands of
//        its own shell: `\dt` and `\d users` for PostgreSQL, `.tables` and
//        `.schema` for SQLite and DuckDB, `DESCRIBE KEYSPACES` for Cassandra,
//        `DESC emp` and `SHOW USER` for Oracle, `:schema` for Neo4j, `help`
//        wherever the engine has no such commands to translate.
// WHY:   The people who use these databases think in the shell's commands; the
//        server itself would answer `\dt` with a syntax error. A meta-command is
//        only ever a question about the catalog, so it can be answered by the
//        catalog queries the shell would run itself.
// HOW:   Each family has a `Dialect` (its own file): the commands it offers, the
//        prefix they share, and `plan`, which recognises one command (a single
//        statement or line, terminator removed) and returns what to run for it.
//        A `Plan` is steps in order: statements in the engine's own language,
//        built from fixed catalog text plus the user's arguments quoted by
//        `args::string_literal`, or tables computed locally (help, connection
//        info). `segments` cuts a script into the lines that are meta-commands and
//        the text that is not, at the block's own statement boundaries, so a
//        script mixing both runs in order and ordinary SQL is never touched.
//        This module only plans: it runs nothing, so the layers keep their
//        order (services run the steps through the engine, the guard has already
//        classified the command as a read).
// WHERE: src-tauri/src/services/query.rs (runs a plan), src-tauri/src/guard/language.rs
//        (classifies a command as a read), src/lib/shellCommands.ts (completion)
// ============================================================================

/// What running a command means for the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CommandOrigin {
    /// A command of the engine's own shell that DB Free translates into catalog queries.
    Shell,
    /// A statement the engine already runs itself, listed so that it can be found.
    Native,
}

// WHAT:  One command a family's shell offers, for completion and for `help`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShellCommand {
    /// What completion inserts, without arguments: `\dt`, `.tables`, `DESCRIBE TABLES`.
    pub name: &'static str,
    /// The full form as the shell documents it: `\dt[S+] [PATTERN]`.
    pub syntax: &'static str,
    /// One line, sentence case, no trailing full stop.
    pub summary: &'static str,
    pub origin: CommandOrigin,
}

impl ShellCommand {
    /// A shell command DB Free translates.
    pub const fn shell(name: &'static str, syntax: &'static str, summary: &'static str) -> ShellCommand {
        ShellCommand { name, syntax, summary, origin: CommandOrigin::Shell }
    }
    /// A statement the engine runs itself.
    pub const fn native(name: &'static str, syntax: &'static str, summary: &'static str) -> ShellCommand {
        ShellCommand { name, syntax, summary, origin: CommandOrigin::Native }
    }
}

// WHAT:  How a family's scripts are cut into commands.
#[derive(Debug, Clone, Copy)]
pub enum Cut {
    /// SQL-like: statements end at `;` (the block's splitter). `line_words` are the leading
    /// words of commands that end at their line instead (`DESC`, `SHOW`), as in SQL*Plus and cqlsh.
    Statements { line_words: &'static [&'static str] },
    /// One command per line (Redis, Cypher, REST consoles).
    Lines,
}

// WHAT:  Everything one family contributes.
#[derive(Clone, Copy)]
pub struct Dialect {
    /// What every command starts with (`\`, `.`, `:`); None for word commands (`DESCRIBE`, `show`).
    pub prefix: Option<&'static str>,
    /// An unrecognised command with the prefix is an error here rather than text sent on.
    /// True where the prefix cannot start a statement of the engine's language.
    pub strict: bool,
    /// The names that open the command list; the first is the one messages quote.
    pub help: &'static [&'static str],
    pub cut: Cut,
    pub commands: &'static [ShellCommand],
    /// Recognises one command: `text` is a single trimmed statement or line without its terminator.
    /// None: not a command, it runs as typed. Some(Err): it is one, but its arguments are wrong,
    /// and the message says how.
    pub plan: fn(&Ctx, &str) -> Option<AppResult<Plan>>,
}

/// What a dialect may need to know about the session it plans for.
pub struct Ctx<'a> {
    pub engine: Engine,
    /// The saved connection; None while only recognising a command (the guard).
    pub connection: Option<&'a ConnectionSummary>,
    /// The schema / keyspace / database the editor's picker is on, when it is.
    pub namespace: Option<&'a str>,
}

impl<'a> Ctx<'a> {
    pub fn new(engine: Engine, connection: Option<&'a ConnectionSummary>, namespace: Option<&'a str>) -> Ctx<'a> {
        Ctx { engine, connection, namespace }
    }

    /// A context for recognising commands, with no session behind it.
    pub fn bare(engine: Engine) -> Ctx<'static> {
        Ctx { engine, connection: None, namespace: None }
    }

    pub fn family(&self) -> Family {
        self.engine.family()
    }
}

/// A table computed locally: a help listing, a connection summary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(columns: &[&str], rows: Vec<Vec<String>>) -> Table {
        Table { columns: columns.iter().map(|c| (*c).to_string()).collect(), rows }
    }

    /// The table as the result the editor draws; every cell is text.
    pub fn result(&self) -> StatementResult {
        let columns = self.columns.iter().map(|name| ColumnMeta { name: name.clone(), type_name: "text".to_string() }).collect();
        let rows = self.rows.iter().map(|row| row.iter().map(|cell| Value::Text(cell.clone())).collect()).collect();
        StatementResult::Rows { result: ResultSet { columns, rows, truncated: false } }
    }
}

/// One thing to do for a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A statement in the engine's own language. Always a read.
    Native(String),
    /// Like `Native`, but a result with no rows is left out of the answer: a section
    /// the object does not have (the triggers of a table with none).
    Section(String),
    /// Like `Native`, but a result with no rows fails with the message: the object
    /// the command names does not exist (`Did not find any relation named "x"`).
    Exists(String, String),
    /// A table made here, with no round trip.
    Table(Table),
}

/// What a command runs, in order; each step contributes its own result tab(s).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    pub steps: Vec<Step>,
}

impl Plan {
    /// A plan of one statement.
    pub fn native(statement: impl Into<String>) -> Plan {
        Plan { steps: vec![Step::Native(statement.into())] }
    }

    /// A plan of several statements, run in this order.
    pub fn natives<I, S>(statements: I) -> Plan
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Plan { steps: statements.into_iter().map(|s| Step::Native(s.into())).collect() }
    }

    /// A plan that is a local table.
    pub fn table(table: Table) -> Plan {
        Plan { steps: vec![Step::Table(table)] }
    }

    /// Adds a step at the end.
    #[must_use]
    pub fn then(mut self, step: Step) -> Plan {
        self.steps.push(step);
        self
    }

    /// The statements the plan sends to the engine.
    pub fn statements(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().filter_map(|step| match step {
            Step::Native(statement) | Step::Section(statement) | Step::Exists(statement, _) => Some(statement.as_str()),
            Step::Table(_) => None,
        })
    }
}

// WHAT:  The `help` listing of a dialect: every command with its syntax and what it does.
pub fn help_plan(dialect: &Dialect) -> Plan {
    let rows = dialect
        .commands
        .iter()
        .map(|c| {
            let runs = match c.origin {
                CommandOrigin::Shell => "translated by DB Free",
                CommandOrigin::Native => "run by the database",
            };
            vec![c.name.to_string(), c.syntax.to_string(), c.summary.to_string(), runs.to_string()]
        })
        .collect();
    Plan::table(Table::new(&["Command", "Syntax", "Description", "Runs"], rows))
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

// WHAT:  The dialect of an adapter family. Exhaustive: a family added to the
//        crate fails to compile here until it says what its shell offers.
pub fn dialect(family: Family) -> &'static Dialect {
    match family {
        Family::Postgres => &psql::DIALECT,
        Family::Mysql => &mysql::DIALECT,
        Family::Mssql => &mssql::DIALECT,
        Family::Oracle => &self::oracle::DIALECT,
        Family::Sqlite => &sqlite::SQLITE,
        Family::Duckdb => &self::duckdb::DIALECT,
        Family::Rocksdb => &self::rocksdb::DIALECT,
        Family::Clickhouse => &clickhouse::DIALECT,
        Family::Redis => &self::redis::DIALECT,
        Family::Memcached => &memcached::DIALECT,
        Family::Mongodb => &self::mongodb::DIALECT,
        Family::Couchdb => &couchdb::DIALECT,
        Family::Firestore => &firestore::DIALECT,
        Family::Dynamodb => &dynamodb::DIALECT,
        Family::Cassandra => &cassandra::DIALECT,
        Family::Hbase => &hbase::DIALECT,
        Family::Neo4j => &neo4j::DIALECT,
        Family::Tigergraph => &tigergraph::DIALECT,
        Family::Influxdb => &influxdb::DIALECT,
        Family::Prometheus => &prometheus::DIALECT,
        Family::Qdrant => &qdrant::DIALECT,
        Family::Milvus => &milvus::DIALECT,
        Family::Weaviate => &weaviate::DIALECT,
        Family::Pinecone => &pinecone::DIALECT,
        Family::Chroma => &chroma::DIALECT,
        Family::Elasticsearch => &elasticsearch::DIALECT,
        Family::Meilisearch => &meilisearch::DIALECT,
        Family::Typesense => &typesense::DIALECT,
        Family::Arangodb => &arangodb::DIALECT,
        Family::Surrealdb => &surrealdb::DIALECT,
        Family::Orientdb => &orientdb::DIALECT,
        Family::Druid => &druid::DIALECT,
        Family::Snowflake => &snowflake::DIALECT,
        Family::Bigquery => &bigquery::DIALECT,
        Family::Libsql => &sqlite::LIBSQL,
        Family::ValTown => &sqlite::HOSTED,
        Family::CloudflareD1 => &sqlite::HOSTED,
        Family::Immudb => &immudb::DIALECT,
        Family::Qldb => &qldb::DIALECT,
        Family::Kafka => &kafka::DIALECT,
        Family::Rabbitmq => &rabbitmq::DIALECT,
        Family::Convex => &convex::DIALECT,
        Family::Objectdb => &objectdb::DIALECT,
        Family::Sparql => &sparql::DIALECT,
        Family::Basex => &basex::DIALECT,
        Family::Existdb => &existdb::DIALECT,
        Family::Spacetimedb => &spacetimedb::DIALECT,
        Family::S3 => &s3::DIALECT,
    }
}

// WHAT:  What to run for `text` if it is a meta-command of the engine's shell.
// HOW:   The terminator is removed, the family's dialect gets the first look, and
//        a command that carries the dialect's prefix but is not one of its
//        commands is an error (when the dialect is strict) rather than text sent
//        to the database to fail with a syntax error.
pub fn plan(ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let dialect = dialect(ctx.family());
    let text = args::strip_terminator(text);
    if text.is_empty() {
        return None;
    }
    if let Some(found) = (dialect.plan)(ctx, text) {
        return Some(found);
    }
    unknown_command(dialect, text).map(Err)
}

fn unknown_command(dialect: &Dialect, text: &str) -> Option<AppError> {
    let prefix = dialect.prefix.filter(|_| dialect.strict)?;
    let after = text.strip_prefix(prefix)?;
    let next = after.chars().next()?;
    if !(next.is_alphabetic() || matches!(next, '?' | '!' | '\\')) {
        return None;
    }
    let head = text.split_whitespace().next().unwrap_or(text);
    let help = dialect.help.first().copied().unwrap_or("help");
    Some(AppError::invalid_input(format!("DB Free does not know the command {head} for this database. {help} lists the commands it understands.")))
}

/// True when `text` is a meta-command of the engine's shell (or looks like one and is refused).
pub fn is_meta(engine: Engine, text: &str) -> bool {
    plan(&Ctx::bare(engine), text).is_some()
}

// ---------------------------------------------------------------------------
// Segments
// ---------------------------------------------------------------------------

/// A script is a run of commands for the shell layer and text for the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Text the database receives as typed.
    Native(String),
    /// One meta-command, trimmed.
    Meta(String),
}

// WHAT:  A script cut into meta-commands and the text between them.
// WHY:   A tab may hold `\dt` above a query, or a Redis `help` line among commands;
//        each run in order, the query exactly as typed.
// HOW:   A script with no meta-command comes back as one `Native` segment holding
//        the original text, so the common path is a no-op. Otherwise statements
//        are cut at the block's boundaries (`;`, or a line for shell commands),
//        and neighbours that are not commands are joined back from the original
//        characters, separators and comments included.
pub fn segments(engine: Engine, script: &str) -> Vec<Segment> {
    let dialect = dialect(engine.family());
    let ctx = Ctx::bare(engine);
    let cut = match dialect.cut {
        Cut::Statements { line_words } => cut_statements(&ctx, line_words, script),
        Cut::Lines => cut_lines(&ctx, script),
    };
    if cut.iter().any(|s| matches!(s, Segment::Meta(_))) {
        cut
    } else {
        vec![Segment::Native(script.to_string())]
    }
}

/// True when the script holds at least one meta-command.
pub fn has_meta(engine: Engine, script: &str) -> bool {
    segments(engine, script).iter().any(|s| matches!(s, Segment::Meta(_)))
}

fn cut_statements(ctx: &Ctx, line_words: &[&str], script: &str) -> Vec<Segment> {
    let (chars, ranges) = scan_ranges(script);
    let mut out = Vec::new();
    // The span of characters, from the first to the last, of the text that is not a command.
    let mut native: Option<(usize, usize)> = None;
    let flush = |native: &mut Option<(usize, usize)>, out: &mut Vec<Segment>| {
        if let Some((from, to)) = native.take() {
            out.push(Segment::Native(chars[from..to].iter().collect()));
        }
    };
    for (raw_from, raw_to) in ranges {
        let mut from = raw_from;
        let to = raw_to;
        while from < to && chars[from].is_whitespace() {
            from += 1;
        }
        let mut end = to;
        while end > from && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        // Each pass takes a command off the front of the range or keeps the range as text.
        while from < end {
            let text: String = chars[from..end].iter().collect();
            let code = args::code_start(&text);
            // A line command (`SHOW USER`) ends at its line, so it is cut off before the
            // whole range is asked: what follows it is the next statement, not its arguments.
            if let Some(consumed) = carve_line(ctx, line_words, code) {
                flush(&mut native, &mut out);
                let skipped = text.chars().count() - code.chars().count();
                out.push(Segment::Meta(args::strip_terminator(&code.chars().take(consumed.0).collect::<String>()).to_string()));
                from += skipped + consumed.1;
                while from < end && chars[from].is_whitespace() {
                    from += 1;
                }
                continue;
            }
            if is_command(ctx, code) {
                flush(&mut native, &mut out);
                out.push(Segment::Meta(args::strip_terminator(code).to_string()));
                break;
            }
            native = Some(match native {
                Some((first, _)) => (first, end),
                None => (from, end),
            });
            break;
        }
    }
    flush(&mut native, &mut out);
    out
}

// WHAT:  For a statement that starts with one of the dialect's line words and has more
//        lines below it, the length (in characters) of its first line and of that line plus
//        its break, when the first line is a command.
fn carve_line(ctx: &Ctx, line_words: &[&str], code: &str) -> Option<(usize, usize)> {
    let (head, _) = args::split_word(code);
    if !line_words.iter().any(|w| args::eq_ci(w, head)) {
        return None;
    }
    let newline = code.find('\n')?;
    let first = code[..newline].trim_end_matches('\r');
    if !is_command(ctx, first) {
        return None;
    }
    let chars = first.chars().count();
    Some((chars, code[..=newline].chars().count()))
}

fn cut_lines(ctx: &Ctx, script: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut native = String::new();
    let flush = |native: &mut String, out: &mut Vec<Segment>| {
        if !native.trim().is_empty() {
            out.push(Segment::Native(native.trim_end().to_string()));
        }
        native.clear();
    };
    for line in script.split('\n') {
        let trimmed = line.trim();
        if !trimmed.is_empty() && is_command(ctx, trimmed) {
            flush(&mut native, &mut out);
            out.push(Segment::Meta(args::strip_terminator(trimmed).to_string()));
        } else {
            native.push_str(line);
            native.push('\n');
        }
    }
    flush(&mut native, &mut out);
    out
}

fn is_command(ctx: &Ctx, text: &str) -> bool {
    !text.trim().is_empty() && plan(ctx, text).is_some()
}

// ---------------------------------------------------------------------------
// Registry for the UI
// ---------------------------------------------------------------------------

// WHAT:  A dialect as the editor's completion reads it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShellDialectInfo {
    pub prefix: Option<String>,
    pub help: Vec<String>,
    pub commands: Vec<ShellCommand>,
}

impl ShellDialectInfo {
    pub fn of(family: Family) -> ShellDialectInfo {
        let dialect = dialect(family);
        ShellDialectInfo {
            prefix: dialect.prefix.map(str::to_string),
            help: dialect.help.iter().map(|h| (*h).to_string()).collect(),
            commands: dialect.commands.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One engine per family, to run the same checks over all of them.
    fn families() -> Vec<(Family, Engine)> {
        let mut seen: Vec<(Family, Engine)> = Vec::new();
        for engine in Engine::ALL {
            if !seen.iter().any(|(family, _)| *family == engine.family()) {
                seen.push((engine.family(), engine));
            }
        }
        seen
    }

    #[test]
    fn every_family_has_a_dialect_with_a_working_help() {
        assert!(families().len() >= 48);
        for (family, engine) in families() {
            let dialect = dialect(family);
            assert!(!dialect.help.is_empty(), "{family:?} names no help command");
            assert!(!dialect.commands.is_empty(), "{family:?} lists no commands");
            for help in dialect.help {
                let found = plan(&Ctx::bare(engine), help);
                let Some(Ok(plan)) = found else { panic!("{family:?}: `{help}` is not understood: {found:?}") };
                assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() >= dialect.commands.len()), "{family:?}: `{help}` should list every command");
            }
        }
    }

    #[test]
    fn every_listed_shell_command_is_recognised_and_every_native_one_is_left_alone() {
        for (family, engine) in families() {
            let ctx = Ctx::bare(engine);
            let dialect = dialect(family);
            let mut seen = std::collections::HashSet::new();
            for command in dialect.commands {
                assert!(seen.insert(command.name), "{family:?} lists {} twice", command.name);
                assert!(!command.summary.is_empty() && !command.syntax.is_empty(), "{family:?}: {} is missing text", command.name);
                assert!(!command.summary.ends_with('.'), "{family:?}: {} summary ends with a full stop", command.name);
                let recognised = plan(&ctx, command.name).is_some();
                match command.origin {
                    CommandOrigin::Shell => assert!(recognised, "{family:?}: `{}` is listed but not recognised", command.name),
                    CommandOrigin::Native => assert!(!recognised || dialect.help.contains(&command.name), "{family:?}: `{}` is native but DB Free intercepts it", command.name),
                }
            }
        }
    }

    #[test]
    fn a_script_without_commands_is_returned_untouched() {
        let script = "select 1;\n-- c\nselect 2;\n";
        assert_eq!(segments(Engine::Postgres, script), vec![Segment::Native(script.to_string())]);
        assert_eq!(segments(Engine::Redis, "GET a\nGET b"), vec![Segment::Native("GET a\nGET b".to_string())]);
        assert!(!has_meta(Engine::Sqlite, "select '.tables'"));
    }

    #[test]
    fn commands_and_sql_keep_their_order() {
        let found = segments(Engine::Postgres, "\\dt public.*\nselect 1;\nselect 2;\n\\d users;\n-- done");
        assert_eq!(
            found,
            vec![
                Segment::Meta("\\dt public.*".to_string()),
                Segment::Native("select 1;\nselect 2".to_string()),
                Segment::Meta("\\d users".to_string()),
                Segment::Native("-- done".to_string()),
            ]
        );
        let sqlite = segments(Engine::Sqlite, ".tables\nselect 1");
        assert_eq!(sqlite, vec![Segment::Meta(".tables".to_string()), Segment::Native("select 1".to_string())]);
    }

    #[test]
    fn line_commands_end_at_their_line_without_a_semicolon() {
        let found = segments(Engine::Oracle, "SHOW USER\nselect * from dual;\nDESC emp\nDESC dept;");
        assert_eq!(
            found,
            vec![
                Segment::Meta("SHOW USER".to_string()),
                Segment::Native("select * from dual".to_string()),
                Segment::Meta("DESC emp".to_string()),
                Segment::Meta("DESC dept".to_string()),
            ]
        );
    }

    #[test]
    fn a_line_command_under_a_comment_is_still_a_command() {
        assert_eq!(segments(Engine::Postgres, "-- who\n\\conninfo"), vec![Segment::Meta("\\conninfo".to_string())]);
    }

    #[test]
    fn line_style_engines_cut_at_command_lines_only() {
        let found = segments(Engine::Redis, "GET a\nhelp\nGET b");
        assert_eq!(found, vec![Segment::Native("GET a".to_string()), Segment::Meta("help".to_string()), Segment::Native("GET b".to_string())]);
    }

    #[test]
    fn an_unknown_command_with_the_shells_prefix_is_refused_not_sent_on() {
        let refused = plan(&Ctx::bare(Engine::Postgres), "\\copy t from 'x'");
        assert!(matches!(refused, Some(Err(AppError::InvalidInput { .. }))), "{refused:?}");
        assert!(plan(&Ctx::bare(Engine::Postgres), "select 1").is_none());
        assert!(plan(&Ctx::bare(Engine::Sqlite), ".5").is_none());
        assert!(is_meta(Engine::Sqlite, ".frobnicate"), "refused, never sent");
        assert!(!is_meta(Engine::Sqlite, "\\dt"), "another shell's command is plain text to this engine");
    }

    // The command lists the editor completes from are generated, like the engine facts,
    // so a command added in Rust is offered in the editor without a second list to keep.
    #[test]
    fn export_shell_commands() {
        let Ok(dir) = std::env::var("TS_RS_EXPORT_DIR") else {
            return; // only runs under `pnpm bindings`
        };
        let mut out = String::from(
            "// This file is generated by the `export_shell_commands` test. Do not edit this file manually.\n\
             import type { ShellDialectInfo } from \"./ShellDialectInfo\";\n\
             import type { Family } from \"./Family\";\n\n\
             export const SHELL_DIALECTS = {\n",
        );
        for (family, _) in families() {
            let key = serde_json::to_string(&family).unwrap_or_default().trim_matches('"').to_string();
            let json = serde_json::to_string(&ShellDialectInfo::of(family)).unwrap_or_default();
            out.push_str(&format!("  {key}: {json},\n"));
        }
        out.push_str("} satisfies Record<Family, ShellDialectInfo>;\n");
        let path = std::path::Path::new(&dir).join("ShellCommands.gen.ts");
        std::fs::write(&path, out).unwrap_or_else(|e| panic!("{e}"));
    }
}
