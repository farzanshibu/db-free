// SOT: shell-meta-cassandra, cqlsh-describe, cqlsh-show-commands, system-schema-structure-queries

use super::args::{self, eq_ci, keyword, split_word, string_literal, Part, Quote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand, Step, Table};
use crate::error::{AppError, AppResult};

// ============================================================================
// CASSANDRA / SCYLLADB (cqlsh)
//
// WHAT:  The structure commands of cqlsh: `DESCRIBE KEYSPACES | KEYSPACE [ks] |
//        TABLES | TABLE t | TYPES | TYPE t | INDEX i | MATERIALIZED VIEW v |
//        FUNCTIONS | FUNCTION f | AGGREGATES | AGGREGATE a | CLUSTER | SCHEMA`,
//        `DESC` for short, a bare `DESCRIBE name`, `SHOW VERSION | HOST |
//        SESSION <uuid>`, and `HELP`.
// WHY:   cqlsh answers `DESCRIBE` itself, from the driver's schema metadata; the
//        server rejects it as a CQL syntax error.
// HOW:   Each command reads the `system_schema.*` tables and `system.local` /
//        `system.peers`, which every Cassandra and ScyllaDB node serves as plain
//        CQL. The adapter's `execute` (src-tauri/src/integrations/cassandra.rs,
//        `split_statements` then `query_unpaged`) runs each statement as typed.
//        cqlsh prints the CQL that would recreate an object; a plan is only
//        queries, so the same facts come back as rows (columns with their
//        partition / clustering role, indexes, the table options). Names fold
//        to lower case unless quoted, like CQL identifiers; user text only
//        appears inside a literal. `ks.name` names a keyspace; otherwise the
//        editor's keyspace picker, then the connection's keyspace, then (for
//        listings) every keyspace.
//        CQL has no `NOT IN`, so the listings include the system keyspaces.
//        cqlsh commands end at their line, so `DESC`, `DESCRIBE`, `SHOW` and
//        `HELP` are line words.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: None,
    strict: false,
    help: &["HELP"],
    cut: Cut::Statements { line_words: &["DESC", "DESCRIBE", "SHOW", "HELP"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("DESCRIBE KEYSPACES", "DESC[RIBE] KEYSPACES", "Every keyspace with its replication"),
    ShellCommand::shell("DESCRIBE KEYSPACE", "DESC[RIBE] KEYSPACE [keyspace]", "One keyspace and what it holds: tables, views, types, indexes, functions"),
    ShellCommand::shell("DESCRIBE TABLES", "DESC[RIBE] TABLES", "Tables of the current keyspace, or of every keyspace"),
    ShellCommand::shell("DESCRIBE TABLE", "DESC[RIBE] TABLE [keyspace.]table", "Columns (with partition and clustering roles), indexes and options of a table"),
    ShellCommand::shell("DESCRIBE TYPES", "DESC[RIBE] TYPES", "User-defined types"),
    ShellCommand::shell("DESCRIBE TYPE", "DESC[RIBE] TYPE [keyspace.]type", "The fields of a user-defined type"),
    ShellCommand::shell("DESCRIBE INDEX", "DESC[RIBE] INDEX [keyspace.]index", "A secondary index"),
    ShellCommand::shell("DESCRIBE MATERIALIZED VIEW", "DESC[RIBE] MATERIALIZED VIEW [keyspace.]view", "A materialized view with its columns"),
    ShellCommand::shell("DESCRIBE FUNCTIONS", "DESC[RIBE] FUNCTIONS", "User-defined functions"),
    ShellCommand::shell("DESCRIBE FUNCTION", "DESC[RIBE] FUNCTION [keyspace.]function", "A user-defined function with its body"),
    ShellCommand::shell("DESCRIBE AGGREGATES", "DESC[RIBE] AGGREGATES", "User-defined aggregates"),
    ShellCommand::shell("DESCRIBE AGGREGATE", "DESC[RIBE] AGGREGATE [keyspace.]aggregate", "A user-defined aggregate"),
    ShellCommand::shell("DESCRIBE CLUSTER", "DESC[RIBE] CLUSTER", "Cluster name, partitioner and the nodes"),
    ShellCommand::shell("DESCRIBE SCHEMA", "DESC[RIBE] [FULL] SCHEMA", "Every keyspace, table, view, type, index and function (system keyspaces included)"),
    ShellCommand::shell("DESCRIBE", "DESC[RIBE] [keyspace.]name", "A keyspace or a table by name"),
    ShellCommand::shell("DESC", "DESC[RIBE] ...", "Short form of DESCRIBE"),
    ShellCommand::shell("SHOW VERSION", "SHOW VERSION", "Cassandra, CQL and native protocol versions"),
    ShellCommand::shell("SHOW HOST", "SHOW HOST", "The host and port DB Free is connected to, and the cluster it belongs to"),
    ShellCommand::shell("SHOW SESSION", "SHOW SESSION <uuid>", "A tracing session and its events"),
    ShellCommand::shell("HELP", "HELP", "List the commands this console understands"),
    ShellCommand::native("SELECT * FROM system_schema.keyspaces", "SELECT * FROM system_schema.keyspaces", "Keyspaces with their replication settings"),
    ShellCommand::native("SELECT * FROM system_schema.tables", "SELECT * FROM system_schema.tables WHERE keyspace_name = 'ks'", "Tables and their options"),
    ShellCommand::native("SELECT * FROM system_schema.columns", "SELECT * FROM system_schema.columns WHERE keyspace_name = 'ks' AND table_name = 't'", "Columns of a table"),
    ShellCommand::native("SELECT * FROM system.local", "SELECT * FROM system.local", "This node: cluster name, version, data center and rack"),
    ShellCommand::native("SELECT * FROM system.peers", "SELECT * FROM system.peers", "The other nodes of the cluster"),
];

fn lit(raw: &str) -> String {
    string_literal(raw, Quote::Ansi)
}

fn plan(ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    if eq_ci(head, "HELP") {
        return Some(Ok(help_plan(&DIALECT)));
    }
    if eq_ci(head, "DESC") || eq_ci(head, "DESCRIBE") {
        return Some(describe(ctx, rest));
    }
    if eq_ci(head, "SHOW") {
        return Some(show(ctx, rest));
    }
    None
}

// ---------------------------------------------------------------------------
// Names and scope
// ---------------------------------------------------------------------------

/// `[keyspace.]name`, folded the way CQL folds identifiers.
struct Name {
    keyspace: Option<String>,
    name: String,
}

fn lower(part: &Part) -> String {
    part.folded_lower()
}

fn name_of(raw: &str, what: &str) -> AppResult<Name> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::invalid_input(format!("Give the {what} to describe, e.g. DESCRIBE {} ks.name.", what.to_ascii_uppercase())));
    }
    let parts = args::qualified(raw).map_err(AppError::invalid_input)?;
    if parts.iter().any(|p| !p.quoted && p.text.contains(char::is_whitespace)) {
        return Err(AppError::invalid_input(format!("Give one {what} name, e.g. ks.name.")));
    }
    match parts.as_slice() {
        [name] => Ok(Name { keyspace: None, name: lower(name) }),
        [keyspace, name] => Ok(Name { keyspace: Some(lower(keyspace)), name: lower(name) }),
        _ => Err(AppError::invalid_input(format!("Give the {what} as name or keyspace.name."))),
    }
}

// WHAT:  The keyspace a command without one defaults to: the editor's picker, then the connection's.
fn current_keyspace(ctx: &Ctx) -> Option<String> {
    let picked = ctx.namespace.map(str::trim).filter(|k| !k.is_empty());
    let connected = ctx.connection.and_then(|c| c.database.as_deref()).map(str::trim).filter(|k| !k.is_empty());
    picked.or(connected).map(str::to_string)
}

/// ` WHERE keyspace_name = 'ks'` for a listing, or nothing when no keyspace is in scope.
fn keyspace_filter(keyspace: Option<&str>) -> String {
    keyspace.map_or_else(String::new, |k| format!(" WHERE keyspace_name = {}", lit(k)))
}

// ---------------------------------------------------------------------------
// DESCRIBE
// ---------------------------------------------------------------------------

fn describe(ctx: &Ctx, rest: &str) -> AppResult<Plan> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Err(AppError::invalid_input(
            "DESCRIBE what? Try DESCRIBE KEYSPACES, TABLES, TABLE name, TYPES, INDEX name, MATERIALIZED VIEW name, FUNCTIONS, AGGREGATES, CLUSTER or SCHEMA.",
        ));
    }
    let scope = current_keyspace(ctx);
    let scope = scope.as_deref();
    let (word, tail) = split_word(rest);
    match word.to_ascii_uppercase().as_str() {
        "KEYSPACES" => no_more(tail, "KEYSPACES").map(|()| keyspaces()),
        "KEYSPACE" => keyspace(ctx, tail),
        "TABLES" => no_more(tail, "TABLES").map(|()| tables(scope)),
        "TABLE" => table(scope, tail),
        "TYPES" => no_more(tail, "TYPES").map(|()| types(scope)),
        "TYPE" => user_type(scope, tail),
        "INDEX" => index(scope, tail),
        "MATERIALIZED" => match keyword(tail, "VIEW") {
            Some(name) => view(scope, name),
            None => Err(AppError::invalid_input("Say DESCRIBE MATERIALIZED VIEW name.")),
        },
        "FUNCTIONS" => no_more(tail, "FUNCTIONS").map(|()| routines(scope, Routine::Function)),
        "FUNCTION" => routine(scope, tail, Routine::Function),
        "AGGREGATES" => no_more(tail, "AGGREGATES").map(|()| routines(scope, Routine::Aggregate)),
        "AGGREGATE" => routine(scope, tail, Routine::Aggregate),
        "CLUSTER" => no_more(tail, "CLUSTER").map(|()| cluster()),
        "SCHEMA" => no_more(tail, "SCHEMA").map(|()| schema()),
        "FULL" => match keyword(tail, "SCHEMA") {
            Some(more) => no_more(more, "FULL SCHEMA").map(|()| schema()),
            None => Err(AppError::invalid_input("Say DESCRIBE FULL SCHEMA.")),
        },
        _ => by_name(scope, rest),
    }
}

fn no_more(tail: &str, what: &str) -> AppResult<()> {
    if tail.trim().is_empty() {
        Ok(())
    } else {
        Err(AppError::invalid_input(format!("DESCRIBE {what} takes no arguments.")))
    }
}

fn keyspaces() -> Plan {
    Plan::native("SELECT keyspace_name, durable_writes, replication FROM system_schema.keyspaces")
}

// WHAT:  A keyspace with everything in it: its row, then tables, views, types, indexes,
//        functions and aggregates, each listed only if the keyspace has some.
fn keyspace(ctx: &Ctx, tail: &str) -> AppResult<Plan> {
    let named = if tail.trim().is_empty() {
        current_keyspace(ctx).ok_or_else(|| AppError::invalid_input("DESCRIBE KEYSPACE needs a keyspace name when none is selected."))?
    } else {
        name_of(tail, "keyspace").and_then(|n| match n.keyspace {
            None => Ok(n.name),
            Some(_) => Err(AppError::invalid_input("A keyspace name has no dots.")),
        })?
    };
    let k = lit(&named);
    let missing = format!("Keyspace '{named}' not found.");
    Ok(Plan { steps: vec![
        Step::Exists(format!("SELECT keyspace_name, durable_writes, replication FROM system_schema.keyspaces WHERE keyspace_name = {k}"), missing),
        Step::Section(format!("SELECT table_name, comment, default_time_to_live, gc_grace_seconds FROM system_schema.tables WHERE keyspace_name = {k}")),
        Step::Section(format!("SELECT view_name, base_table_name, where_clause FROM system_schema.views WHERE keyspace_name = {k}")),
        Step::Section(format!("SELECT type_name, field_names, field_types FROM system_schema.types WHERE keyspace_name = {k}")),
        Step::Section(format!("SELECT table_name, index_name, kind, options FROM system_schema.indexes WHERE keyspace_name = {k}")),
        Step::Section(format!("SELECT function_name, argument_types, return_type, language FROM system_schema.functions WHERE keyspace_name = {k}")),
        Step::Section(format!("SELECT aggregate_name, argument_types, return_type, state_func, final_func FROM system_schema.aggregates WHERE keyspace_name = {k}")),
    ] })
}

fn tables(scope: Option<&str>) -> Plan {
    Plan::native(format!("SELECT keyspace_name, table_name, comment FROM system_schema.tables{}", keyspace_filter(scope)))
}

// WHAT:  The keyspace a name lives in: its own prefix, else the scope. None = every keyspace.
fn home(named: &Name, scope: Option<&str>) -> Option<String> {
    named.keyspace.clone().or_else(|| scope.map(str::to_string))
}

/// ` WHERE keyspace_name = 'ks' AND <column> = 'name'`, or just the name when no keyspace is known.
fn where_named(home: Option<&str>, column: &str, name: &str) -> String {
    match home {
        Some(k) => format!(" WHERE keyspace_name = {} AND {column} = {}", lit(k), lit(name)),
        None => format!(" WHERE {column} = {}", lit(name)),
    }
}

// WHAT:  `ALLOW FILTERING` for a lookup CQL cannot serve from the key alone.
// HOW:   `key_prefix` says the restricted columns are a leading part of the table's primary key
//        (keyspace, then the name). Without a keyspace the name is searched across all of them.
fn filtering(home: Option<&str>, key_prefix: bool) -> &'static str {
    if home.is_some() && key_prefix {
        ""
    } else {
        " ALLOW FILTERING"
    }
}

fn table(scope: Option<&str>, tail: &str) -> AppResult<Plan> {
    let named = name_of(tail, "table")?;
    Ok(table_plan(&named, scope))
}

// WHAT:  A table: its row, its columns, and its indexes.
// HOW:   `kind` and `position` carry the roles cqlsh groups columns by (partition key, clustering,
//        static, regular). CQL can only ORDER BY a clustering column inside one partition, so the
//        rows come back in the table's own order (by column name), not in cqlsh's.
fn table_plan(named: &Name, scope: Option<&str>) -> Plan {
    let home = home(named, scope);
    let home = home.as_deref();
    let missing = format!("Table '{}' not found.", named.name);
    let mut steps = vec![Step::Exists(
        format!(
            "SELECT keyspace_name, table_name, comment, default_time_to_live, gc_grace_seconds, bloom_filter_fp_chance, caching, compaction, compression \
             FROM system_schema.tables{}{}",
            where_named(home, "table_name", &named.name),
            filtering(home, true)
        ),
        missing,
    )];
    steps.push(Step::Section(format!(
        "SELECT keyspace_name, table_name, column_name, kind, position, clustering_order, type FROM system_schema.columns{}{}",
        where_named(home, "table_name", &named.name),
        filtering(home, true)
    )));
    steps.push(Step::Section(format!(
        "SELECT keyspace_name, table_name, index_name, kind, options FROM system_schema.indexes{}{}",
        where_named(home, "table_name", &named.name),
        filtering(home, true)
    )));
    Plan { steps }
}

fn types(scope: Option<&str>) -> Plan {
    Plan::native(format!("SELECT keyspace_name, type_name, field_names, field_types FROM system_schema.types{}", keyspace_filter(scope)))
}

fn user_type(scope: Option<&str>, tail: &str) -> AppResult<Plan> {
    let named = name_of(tail, "type")?;
    let home = home(&named, scope);
    let home = home.as_deref();
    Ok(Plan::native(format!(
        "SELECT keyspace_name, type_name, field_names, field_types FROM system_schema.types{}{}",
        where_named(home, "type_name", &named.name),
        filtering(home, true)
    )))
}

fn index(scope: Option<&str>, tail: &str) -> AppResult<Plan> {
    let named = name_of(tail, "index")?;
    let home = home(&named, scope);
    let home = home.as_deref();
    // The index name is the third key column, after the table: a lookup by name alone is a filter.
    let statement = format!(
        "SELECT keyspace_name, table_name, index_name, kind, options FROM system_schema.indexes{}{}",
        where_named(home, "index_name", &named.name),
        filtering(home, false)
    );
    Ok(Plan { steps: vec![Step::Exists(statement, format!("Index '{}' not found.", named.name))] })
}

fn view(scope: Option<&str>, tail: &str) -> AppResult<Plan> {
    let named = name_of(tail, "materialized view")?;
    let home = home(&named, scope);
    let home = home.as_deref();
    let missing = format!("Materialized view '{}' not found.", named.name);
    let header = format!(
        "SELECT keyspace_name, view_name, base_table_name, include_all_columns, where_clause, default_time_to_live, gc_grace_seconds FROM system_schema.views{}{}",
        where_named(home, "view_name", &named.name),
        filtering(home, true)
    );
    let columns = format!(
        "SELECT keyspace_name, table_name, column_name, kind, position, clustering_order, type FROM system_schema.columns{}{}",
        where_named(home, "table_name", &named.name),
        filtering(home, true)
    );
    Ok(Plan { steps: vec![Step::Exists(header, missing), Step::Section(columns)] })
}

#[derive(Clone, Copy)]
enum Routine {
    Function,
    Aggregate,
}

impl Routine {
    fn table(self) -> &'static str {
        match self {
            Routine::Function => "functions",
            Routine::Aggregate => "aggregates",
        }
    }
    fn name_column(self) -> &'static str {
        match self {
            Routine::Function => "function_name",
            Routine::Aggregate => "aggregate_name",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Routine::Function => "function",
            Routine::Aggregate => "aggregate",
        }
    }
    fn listing_columns(self) -> &'static str {
        match self {
            Routine::Function => "keyspace_name, function_name, argument_types, return_type, language",
            Routine::Aggregate => "keyspace_name, aggregate_name, argument_types, return_type, state_func, final_func",
        }
    }
    fn detail_columns(self) -> &'static str {
        match self {
            Routine::Function => "keyspace_name, function_name, argument_names, argument_types, return_type, language, called_on_null_input, body",
            Routine::Aggregate => "keyspace_name, aggregate_name, argument_types, return_type, state_type, state_func, final_func, initcond",
        }
    }
}

fn routines(scope: Option<&str>, kind: Routine) -> Plan {
    Plan::native(format!("SELECT {} FROM system_schema.{}{}", kind.listing_columns(), kind.table(), keyspace_filter(scope)))
}

fn routine(scope: Option<&str>, tail: &str, kind: Routine) -> AppResult<Plan> {
    let named = name_of(tail, kind.label())?;
    let home = home(&named, scope);
    let home = home.as_deref();
    Ok(Plan::native(format!(
        "SELECT {} FROM system_schema.{}{}{}",
        kind.detail_columns(),
        kind.table(),
        where_named(home, kind.name_column(), &named.name),
        filtering(home, true)
    )))
}

// WHAT:  `DESCRIBE name`: cqlsh looks the name up as a keyspace, then as a table of the current one.
// HOW:   Both lookups are sections, so only the one that exists is shown. A dotted name is a table.
fn by_name(scope: Option<&str>, raw: &str) -> AppResult<Plan> {
    let named = name_of(raw, "name")?;
    if named.keyspace.is_some() {
        return Ok(table_plan(&named, scope));
    }
    let mut steps = vec![Step::Section(format!(
        "SELECT keyspace_name, durable_writes, replication FROM system_schema.keyspaces WHERE keyspace_name = {}",
        lit(&named.name)
    ))];
    steps.extend(table_plan(&named, scope).steps.into_iter().map(|step| match step {
        Step::Exists(statement, _) => Step::Section(statement),
        other => other,
    }));
    Ok(Plan { steps })
}

fn cluster() -> Plan {
    Plan {
        steps: vec![
            Step::Native("SELECT cluster_name, partitioner, release_version, data_center, rack FROM system.local".to_string()),
            Step::Section("SELECT peer, data_center, rack, release_version FROM system.peers".to_string()),
        ],
    }
}

fn schema() -> Plan {
    Plan {
        steps: vec![
            Step::Native("SELECT keyspace_name, durable_writes, replication FROM system_schema.keyspaces".to_string()),
            Step::Section("SELECT keyspace_name, table_name, comment FROM system_schema.tables".to_string()),
            Step::Section("SELECT keyspace_name, view_name, base_table_name FROM system_schema.views".to_string()),
            Step::Section("SELECT keyspace_name, type_name, field_names, field_types FROM system_schema.types".to_string()),
            Step::Section("SELECT keyspace_name, table_name, index_name, kind, options FROM system_schema.indexes".to_string()),
            Step::Section("SELECT keyspace_name, function_name, argument_types, return_type FROM system_schema.functions".to_string()),
            Step::Section("SELECT keyspace_name, aggregate_name, argument_types, return_type FROM system_schema.aggregates".to_string()),
        ],
    }
}

// ---------------------------------------------------------------------------
// SHOW
// ---------------------------------------------------------------------------

fn show(ctx: &Ctx, rest: &str) -> AppResult<Plan> {
    let (option, tail) = split_word(rest);
    match option.to_ascii_uppercase().as_str() {
        "" => Err(AppError::invalid_input("SHOW what? DB Free supports SHOW VERSION, SHOW HOST and SHOW SESSION <uuid>.")),
        "VERSION" => no_args(tail, "VERSION").map(|()| {
            Plan::native("SELECT release_version, cql_version, native_protocol_version, cluster_name FROM system.local")
        }),
        "HOST" => no_args(tail, "HOST").map(|()| host(ctx)),
        "SESSION" => session(tail),
        other => Err(AppError::invalid_input(format!("SHOW {other} is not supported here. DB Free supports SHOW VERSION, SHOW HOST and SHOW SESSION <uuid>."))),
    }
}

fn no_args(tail: &str, what: &str) -> AppResult<()> {
    if tail.trim().is_empty() {
        Ok(())
    } else {
        Err(AppError::invalid_input(format!("SHOW {what} takes no arguments.")))
    }
}

// WHAT:  "Connected to <cluster> at host:port": the saved connection's address, then the node's view.
fn host(ctx: &Ctx) -> Plan {
    let (address, port) = match ctx.connection {
        Some(c) => (c.host.clone().unwrap_or_default(), c.port.map(|p| p.to_string()).unwrap_or_default()),
        None => (String::new(), String::new()),
    };
    Plan {
        steps: vec![
            Step::Table(Table::new(&["Host", "Port"], vec![vec![address, port]])),
            Step::Native("SELECT cluster_name, data_center, rack FROM system.local".to_string()),
        ],
    }
}

// WHAT:  `SHOW SESSION <uuid>`: the tracing session and its events.
// HOW:   The id is a bare CQL uuid, so it is checked to be one rather than quoted.
fn session(tail: &str) -> AppResult<Plan> {
    let id = tail.trim();
    if !is_uuid(id) {
        return Err(AppError::invalid_input("SHOW SESSION takes a tracing session id, e.g. SHOW SESSION 3cd6d9f0-1f1c-11ef-9262-0242ac120002."));
    }
    Ok(Plan {
        steps: vec![
            Step::Exists(
                format!("SELECT session_id, command, client, coordinator, duration, parameters, request, started_at FROM system_traces.sessions WHERE session_id = {id}"),
                format!("Session {id} not found."),
            ),
            Step::Section(format!("SELECT activity, source, source_elapsed, thread FROM system_traces.events WHERE session_id = {id}")),
        ],
    })
}

fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8_usize, 4, 4, 4, 12]).all(|(group, len)| group.len() == len && group.chars().all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::destructive::{classify, StatementKind};
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Cassandra), text)
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
    fn every_command_is_recognised_in_any_case_with_or_without_a_terminator() {
        for text in [
            "describe keyspaces", "DESC KEYSPACES;", "Describe Keyspace app", "desc tables", "DESCRIBE TABLE app.users", "describe types", "describe type app.address",
            "describe index app.users_idx", "describe materialized view app.by_email", "describe functions", "describe function app.f", "describe aggregates",
            "describe aggregate app.a", "describe cluster", "describe schema", "describe full schema", "describe users", "show version", "SHOW HOST;", "help",
            "show session 3cd6d9f0-1f1c-11ef-9262-0242ac120002",
        ] {
            assert!(matches!(planned(text), Some(Ok(_))), "{text}: {:?}", planned(text));
        }
    }

    #[test]
    fn native_statements_are_not_intercepted() {
        for text in ["SELECT * FROM system_schema.keyspaces", "select 1", "USE app", "CREATE KEYSPACE x WITH replication = {}", "DESCRIBE_ME", "showtime"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn describe_needs_an_object() {
        assert!(error("describe").contains("DESCRIBE what?"));
        assert!(error("describe table").contains("table to describe"));
        assert!(error("describe tables now").contains("takes no arguments"));
        assert!(error("describe materialized users").contains("MATERIALIZED VIEW"));
        assert!(error("describe table a.b.c").contains("keyspace.name"));
        assert!(error("describe table a b").contains("one table name"));
        assert!(error("describe full").contains("FULL SCHEMA"));
        assert!(error("describe keyspace a.b").contains("no dots"));
    }

    #[test]
    fn listings_cover_every_keyspace_until_one_is_in_scope() {
        assert_eq!(statements("describe tables"), ["SELECT keyspace_name, table_name, comment FROM system_schema.tables"]);
        let scoped = run_plan(&Ctx::new(Engine::Cassandra, None, Some("App")), "describe tables");
        let Some(Ok(plan)) = scoped else { panic!("not planned") };
        assert_eq!(
            plan.statements().collect::<Vec<_>>(),
            ["SELECT keyspace_name, table_name, comment FROM system_schema.tables WHERE keyspace_name = 'App'"],
            "the picker's keyspace is used as typed, it is a real name"
        );
        assert!(statements("describe types")[0].starts_with("SELECT keyspace_name, type_name"));
        assert!(statements("describe functions")[0].contains("FROM system_schema.functions"));
        assert!(statements("describe aggregates")[0].contains("FROM system_schema.aggregates"));
        assert!(statements("describe keyspaces")[0].contains("replication"));
    }

    #[test]
    fn the_pickers_keyspace_is_quoted_like_any_other_text() {
        let picked = run_plan(&Ctx::new(Engine::Cassandra, None, Some("a'b\\")), "describe tables");
        let Some(Ok(plan)) = picked else { panic!("not planned") };
        assert_eq!(plan.statements().collect::<Vec<_>>(), ["SELECT keyspace_name, table_name, comment FROM system_schema.tables WHERE keyspace_name = 'a''b\\'"]);
    }

    #[test]
    fn describe_table_reads_the_table_its_columns_and_its_indexes() {
        let sql = statements("DESC TABLE App.Users");
        assert_eq!(sql.len(), 3);
        assert!(sql[0].contains("FROM system_schema.tables WHERE keyspace_name = 'app' AND table_name = 'users'"), "{}", sql[0]);
        assert!(!sql[0].contains("ALLOW FILTERING") && !sql[1].contains("ALLOW FILTERING") && !sql[2].contains("ALLOW FILTERING"), "keyspace and table are a key prefix");
        assert!(sql[1].contains("FROM system_schema.columns WHERE keyspace_name = 'app' AND table_name = 'users'"), "{}", sql[1]);
        assert!(sql[1].contains("kind, position, clustering_order, type"));
        assert!(sql[2].contains("FROM system_schema.indexes WHERE keyspace_name = 'app' AND table_name = 'users'"), "{}", sql[2]);
        let Some(Ok(plan)) = planned("describe table users") else { panic!("not planned") };
        assert!(matches!(&plan.steps[0], Step::Exists(_, message) if message == "Table 'users' not found."));
        assert!(matches!(&plan.steps[1], Step::Section(_)) && matches!(&plan.steps[2], Step::Section(_)));
    }

    #[test]
    fn quoted_names_keep_their_case_and_unquoted_ones_fold() {
        let sql = statements("describe table \"MyKs\".\"MyTable\"");
        assert!(sql[0].contains("keyspace_name = 'MyKs' AND table_name = 'MyTable'"), "{}", sql[0]);
        let folded = statements("describe table MyKs.MyTable");
        assert!(folded[0].contains("keyspace_name = 'myks' AND table_name = 'mytable'"), "{}", folded[0]);
    }

    #[test]
    fn the_connections_keyspace_is_the_default_scope() {
        let connection = crate::model::ConnectionSummary::draft(
            &serde_json::from_value(serde_json::json!({
                "name": "c", "engine": "cassandra", "environment": "local", "readOnly": false, "host": "node1", "port": 9042, "database": "shop",
                "username": null, "password": null, "filePath": null, "sslMode": "disable",
                "ssh": serde_json::to_value(crate::model::SshTunnel::default()).unwrap_or_default(),
                "sshSecret": null, "folder": null, "color": null, "favorite": false, "extra": {}
            }))
            .unwrap_or_else(|e| panic!("{e}")),
            false,
        );
        let ctx = Ctx::new(Engine::Cassandra, Some(&connection), None);
        let Some(Ok(plan)) = run_plan(&ctx, "describe table orders") else { panic!("not planned") };
        assert!(plan.statements().next().is_some_and(|s| s.contains("keyspace_name = 'shop' AND table_name = 'orders'")));
        let Some(Ok(host)) = run_plan(&ctx, "show host") else { panic!("not planned") };
        assert!(matches!(&host.steps[0], Step::Table(t) if t.rows == vec![vec!["node1".to_string(), "9042".to_string()]]));
    }

    #[test]
    fn names_without_a_keyspace_are_searched_with_allow_filtering() {
        let sql = statements("describe table users");
        assert!(sql[0].contains(" WHERE table_name = 'users' ALLOW FILTERING"), "{}", sql[0]);
        let index = statements("describe index by_email");
        assert!(index[0].contains("WHERE index_name = 'by_email' ALLOW FILTERING"), "{}", index[0]);
        let in_ks = statements("describe index shop.by_email");
        assert!(in_ks[0].contains("WHERE keyspace_name = 'shop' AND index_name = 'by_email' ALLOW FILTERING"), "{}", in_ks[0]);
        let ty = statements("describe type shop.address");
        assert!(ty[0].ends_with("WHERE keyspace_name = 'shop' AND type_name = 'address'"), "a partition and clustering key restriction needs no filtering: {}", ty[0]);
    }

    #[test]
    fn describe_keyspace_lists_what_it_holds() {
        let Some(Ok(plan)) = planned("describe keyspace \"Shop\"") else { panic!("not planned") };
        assert_eq!(plan.steps.len(), 7);
        assert!(matches!(&plan.steps[0], Step::Exists(sql, message) if sql.contains("WHERE keyspace_name = 'Shop'") && message.contains("Shop")));
        assert!(plan.steps[1..].iter().all(|s| matches!(s, Step::Section(sql) if sql.contains("keyspace_name = 'Shop'"))));
        assert!(error("describe keyspace").contains("needs a keyspace name"));
        let Some(Ok(scoped)) = run_plan(&Ctx::new(Engine::Scylladb, None, Some("shop")), "describe keyspace") else { panic!("not planned") };
        assert!(scoped.statements().next().is_some_and(|s| s.contains("'shop'")));
    }

    #[test]
    fn a_bare_name_is_a_keyspace_or_a_table() {
        let Some(Ok(plan)) = planned("describe shop") else { panic!("not planned") };
        assert!(plan.steps.iter().all(|s| matches!(s, Step::Section(_))), "nothing is demanded, only what exists is shown");
        let sql: Vec<&str> = plan.statements().collect();
        assert!(sql[0].contains("FROM system_schema.keyspaces WHERE keyspace_name = 'shop'"));
        assert!(sql[1].contains("FROM system_schema.tables"));
        let Some(Ok(dotted)) = planned("describe shop.orders") else { panic!("not planned") };
        assert!(matches!(&dotted.steps[0], Step::Exists(..)), "a dotted name can only be a table");
    }

    #[test]
    fn materialized_views_functions_and_aggregates_are_read_by_name() {
        let view = statements("describe materialized view shop.by_email");
        assert!(view[0].contains("FROM system_schema.views WHERE keyspace_name = 'shop' AND view_name = 'by_email'"), "{}", view[0]);
        assert!(view[1].contains("FROM system_schema.columns WHERE keyspace_name = 'shop' AND table_name = 'by_email'"), "{}", view[1]);
        let function = statements("describe function shop.fmt");
        assert!(function[0].contains("FROM system_schema.functions WHERE keyspace_name = 'shop' AND function_name = 'fmt'") && function[0].contains("body"));
        let aggregate = statements("describe aggregate shop.total");
        assert!(aggregate[0].contains("FROM system_schema.aggregates WHERE keyspace_name = 'shop' AND aggregate_name = 'total'"));
    }

    #[test]
    fn cluster_and_schema_walk_the_system_tables() {
        let cluster = statements("describe cluster");
        assert!(cluster[0].contains("cluster_name, partitioner") && cluster[1].contains("system.peers"));
        let schema = statements("describe full schema");
        assert_eq!(schema, statements("describe schema"));
        assert!(schema.iter().any(|s| s.contains("system_schema.aggregates")) && schema.iter().any(|s| s.contains("system_schema.indexes")));
    }

    #[test]
    fn show_session_accepts_only_a_uuid_and_inlines_nothing_else() {
        let sql = statements("show session 3cd6d9f0-1f1c-11ef-9262-0242ac120002");
        assert!(sql[0].ends_with("WHERE session_id = 3cd6d9f0-1f1c-11ef-9262-0242ac120002") && sql[1].contains("system_traces.events"));
        assert!(error("show session 1 OR 1=1").contains("tracing session id"));
        assert!(error("show session 3cd6d9f0-1f1c-11ef-9262-0242ac12000g").contains("tracing session id"));
        assert!(error("show session").contains("tracing session id"));
        assert!(is_uuid("00000000-0000-0000-0000-000000000000") && !is_uuid("0000000-0000-0000-0000-000000000000"));
    }

    #[test]
    fn show_version_and_unknown_options() {
        assert!(statements("show version")[0].contains("release_version, cql_version, native_protocol_version"));
        assert!(error("show").contains("SHOW what?"));
        assert!(error("show tables").contains("not supported"));
        assert!(error("show version now").contains("takes no arguments"));
    }

    #[test]
    fn user_text_stays_inside_a_literal() {
        let sql = statements("describe table \"ks'; DROP KEYSPACE x; --\".\"t'\"");
        assert!(sql[0].contains("keyspace_name = 'ks''; DROP KEYSPACE x; --' AND table_name = 't'''"), "{}", sql[0]);
        let backslash = statements("describe table \"a\\\".\"b\"");
        assert!(backslash[0].contains("keyspace_name = 'a\\'"), "a backslash is an ordinary character in CQL: {}", backslash[0]);
    }

    #[test]
    fn every_statement_is_a_read() {
        for text in [
            "describe keyspaces", "describe keyspace app", "describe tables", "describe table app.t", "describe types", "describe type app.t", "describe index app.i",
            "describe materialized view app.v", "describe functions", "describe function app.f", "describe aggregates", "describe aggregate app.a", "describe cluster",
            "describe schema", "describe app", "describe app.t", "show version", "show host", "show session 3cd6d9f0-1f1c-11ef-9262-0242ac120002",
        ] {
            for sql in statements(text) {
                let kinds: Vec<StatementKind> = classify(&sql).into_iter().map(|s| s.kind).collect();
                assert_eq!(kinds, vec![StatementKind::Read], "{text}: {sql}");
            }
        }
    }

    #[test]
    fn line_commands_are_cut_without_a_semicolon() {
        let found = segments(Engine::Scylladb, "DESCRIBE KEYSPACES\nSELECT * FROM app.users;\nSHOW VERSION");
        assert_eq!(
            found,
            vec![
                Segment::Meta("DESCRIBE KEYSPACES".to_string()),
                Segment::Native("SELECT * FROM app.users".to_string()),
                Segment::Meta("SHOW VERSION".to_string()),
            ]
        );
    }

    #[test]
    fn help_lists_every_command() {
        let Some(Ok(plan)) = planned("help") else { panic!("no help") };
        assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()));
    }
}
