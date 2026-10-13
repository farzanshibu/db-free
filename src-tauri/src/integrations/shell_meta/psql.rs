// SOT: shell-meta-psql, psql-backslash-commands, psql-name-patterns, postgres-catalog-queries

use super::args::{eq_ci, raw_args, split_word, string_literal, Quote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand, Step, Table};
use crate::error::{AppError, AppResult};

// ============================================================================
// POSTGRESQL FAMILY: psql's backslash commands
//
// WHAT:  The structure commands of `psql` — `\l`, `\dn`, `\dt`, `\d users`,
//        `\df`, `\du`, `\conninfo`… — answered with the catalog queries psql
//        itself sends, for PostgreSQL and every server that speaks its catalog
//        (Supabase, Neon, Timescale, CockroachDB, YugabyteDB, PostGIS…).
// WHY:   psql users type `\dt`; the server would answer a syntax error. A
//        translated command shows the same facts the shell would, in the result
//        grid, where they can be sorted, filtered and exported.
// HOW:   A command is its name plus modifiers (`S` includes system objects, `+`
//        adds columns) and at most one pattern argument. Patterns follow psql's
//        rules (`*` any run, `?` one character, `schema.name`, double quotes keep
//        case and make everything literal, otherwise names fold to lower case)
//        and become anchored regular expressions matched with `~` — each one a
//        single string literal, so nothing the user types can leave it.
//        `\d name` runs several queries (columns, indexes, constraints, foreign
//        keys, triggers…); each is one result tab, and tabs the object has
//        nothing for are dropped. Without a pattern the lists show what the
//        search path makes visible, like psql, so the editor's schema picker
//        (which sets the search path) narrows them the same way.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (dispatch),
//        src-tauri/src/services/query.rs (runs the plan)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: Some("\\"),
    strict: true,
    help: &["\\?", "help"],
    cut: Cut::Statements { line_words: &[] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("\\?", "\\?", "List the commands this console understands"),
    ShellCommand::shell("\\l", "\\l[+] [PATTERN]", "List databases"),
    ShellCommand::shell("\\dn", "\\dn[S+] [PATTERN]", "List schemas"),
    ShellCommand::shell("\\d", "\\d[S+] [NAME]", "Describe a table, view, sequence or index; without a name, list relations"),
    ShellCommand::shell("\\dt", "\\dt[S+] [PATTERN]", "List tables"),
    ShellCommand::shell("\\dv", "\\dv[S+] [PATTERN]", "List views"),
    ShellCommand::shell("\\dm", "\\dm[S+] [PATTERN]", "List materialized views"),
    ShellCommand::shell("\\di", "\\di[S+] [PATTERN]", "List indexes"),
    ShellCommand::shell("\\ds", "\\ds[S+] [PATTERN]", "List sequences"),
    ShellCommand::shell("\\dE", "\\dE[S+] [PATTERN]", "List foreign tables"),
    ShellCommand::shell("\\df", "\\df[anptw][S+] [PATTERN]", "List functions (a aggregate, n normal, p procedure, t trigger, w window)"),
    ShellCommand::shell("\\dT", "\\dT[S+] [PATTERN]", "List data types"),
    ShellCommand::shell("\\dD", "\\dD[S+] [PATTERN]", "List domains"),
    ShellCommand::shell("\\du", "\\du[S+] [PATTERN]", "List roles"),
    ShellCommand::shell("\\dg", "\\dg[S+] [PATTERN]", "List roles (same as \\du)"),
    ShellCommand::shell("\\dx", "\\dx[+] [PATTERN]", "List installed extensions; with + also their objects"),
    ShellCommand::shell("\\db", "\\db[+] [PATTERN]", "List tablespaces"),
    ShellCommand::shell("\\dL", "\\dL[S+] [PATTERN]", "List procedural languages"),
    ShellCommand::shell("\\dp", "\\dp [PATTERN]", "List access privileges of tables, views and sequences"),
    ShellCommand::shell("\\z", "\\z [PATTERN]", "List access privileges (same as \\dp)"),
    ShellCommand::shell("\\dy", "\\dy[+] [PATTERN]", "List event triggers"),
    ShellCommand::shell("\\dRp", "\\dRp[+] [PATTERN]", "List logical replication publications"),
    ShellCommand::shell("\\dRs", "\\dRs[+] [PATTERN]", "List logical replication subscriptions"),
    ShellCommand::shell("\\des", "\\des[+] [PATTERN]", "List foreign servers"),
    ShellCommand::shell("\\dew", "\\dew[+] [PATTERN]", "List foreign-data wrappers"),
    ShellCommand::shell("\\det", "\\det[+] [PATTERN]", "List foreign tables with their servers"),
    ShellCommand::shell("\\dO", "\\dO[S+] [PATTERN]", "List collations"),
    ShellCommand::shell("\\dA", "\\dA[+] [PATTERN]", "List access methods"),
    ShellCommand::shell("\\dconfig", "\\dconfig[+] [PATTERN]", "List server configuration parameters"),
    ShellCommand::shell("\\sf", "\\sf FUNCTION", "Show the definition of a function or procedure"),
    ShellCommand::shell("\\sv", "\\sv VIEW", "Show the definition of a view"),
    ShellCommand::shell("\\conninfo", "\\conninfo", "Show the connection and the server behind it"),
    ShellCommand::shell("\\encoding", "\\encoding", "Show the client and server encoding"),
];

fn plan(ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    if eq_ci(head, "help") && rest.is_empty() {
        return Some(Ok(help_plan(&DIALECT)));
    }
    let command = head.strip_prefix('\\')?;
    run(ctx, command, rest)
}

/// What a command name carries besides its name: `S` (system objects) and `+` (more columns).
#[derive(Debug, Clone, Copy, Default)]
struct Mods {
    system: bool,
    plus: bool,
}

fn modifiers(command: &str) -> (&str, Mods) {
    let base = command.trim_end_matches(['S', '+']);
    let tail = &command[base.len()..];
    (base, Mods { system: tail.contains('S'), plus: tail.contains('+') })
}

/// A command psql has, read from its name; the letters are the object kinds of `\dtvm` and `\dfa`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Help,
    Databases,
    Schemas,
    Describe,
    Relations(String),
    Functions(String),
    Roles,
    Extensions,
    Types,
    Domains,
    Tablespaces,
    Languages,
    Privileges,
    EventTriggers,
    Publications,
    Subscriptions,
    ForeignServers,
    ForeignWrappers,
    ForeignTables,
    Collations,
    AccessMethods,
    Settings,
    FunctionSource,
    ViewSource,
    ConnectionInfo,
    Encoding,
}

fn command_named(name: &str) -> Option<Command> {
    Some(match name {
        "?" => Command::Help,
        "l" | "list" => Command::Databases,
        "dn" => Command::Schemas,
        "d" => Command::Describe,
        "du" | "dg" => Command::Roles,
        "dx" => Command::Extensions,
        "dT" => Command::Types,
        "dD" => Command::Domains,
        "db" => Command::Tablespaces,
        "dL" => Command::Languages,
        "dp" | "z" => Command::Privileges,
        "dy" => Command::EventTriggers,
        "dRp" => Command::Publications,
        "dRs" => Command::Subscriptions,
        "des" => Command::ForeignServers,
        "dew" => Command::ForeignWrappers,
        "det" => Command::ForeignTables,
        "dO" => Command::Collations,
        "dA" => Command::AccessMethods,
        "dconfig" => Command::Settings,
        "sf" => Command::FunctionSource,
        "sv" => Command::ViewSource,
        "conninfo" => Command::ConnectionInfo,
        "encoding" => Command::Encoding,
        other => {
            // `\dt`, `\dv`, `\dtvm`… list relations; `\df`, `\dfa`, `\dfpn`… list functions.
            let letters = other.strip_prefix('d')?;
            if !letters.is_empty() && letters.chars().all(|c| "tvmisE".contains(c)) {
                Command::Relations(letters.to_string())
            } else {
                Command::Functions(letters.strip_prefix('f')?.to_string())
            }
        }
    })
}

fn run(ctx: &Ctx, command: &str, rest: &str) -> Option<AppResult<Plan>> {
    let (name, mods) = modifiers(command);
    let command = command_named(name)?;
    // `\sf name(integer, text)` takes the rest of the line, as psql does; the others take one pattern.
    let argument = if matches!(command, Command::FunctionSource | Command::ViewSource) {
        Ok(Some(rest.trim().to_string()).filter(|r| !r.is_empty()))
    } else {
        one_argument(rest)
    };
    let argument = match argument {
        Ok(argument) => argument,
        Err(message) => return Some(Err(AppError::invalid_input(message))),
    };
    let arg = argument.as_deref();
    let built = match command {
        Command::Help => Ok(help_plan(&DIALECT)),
        Command::Databases => databases(arg, mods).map(Plan::native),
        Command::Schemas => schemas(arg, mods).map(Plan::native),
        Command::Describe => match arg {
            Some(pattern) => describe(pattern, mods),
            None => relations("tvmsE", None, mods).map(Plan::native),
        },
        Command::Relations(letters) => relations(&letters, arg, mods).map(Plan::native),
        Command::Functions(letters) => functions(&letters, arg, mods).map(Plan::native),
        Command::Roles => roles(arg, mods).map(Plan::native),
        Command::Extensions => extensions(arg, mods),
        Command::Types => types(arg, mods, false).map(Plan::native),
        Command::Domains => types(arg, mods, true).map(Plan::native),
        Command::Tablespaces => tablespaces(arg, mods).map(Plan::native),
        Command::Languages => languages(arg, mods).map(Plan::native),
        Command::Privileges => privileges(arg).map(Plan::native),
        Command::EventTriggers => event_triggers(arg, mods).map(Plan::native),
        Command::Publications => publications(arg, mods).map(Plan::native),
        Command::Subscriptions => subscriptions(arg, mods).map(Plan::native),
        Command::ForeignServers => foreign_servers(arg, mods).map(Plan::native),
        Command::ForeignWrappers => foreign_wrappers(arg, mods).map(Plan::native),
        Command::ForeignTables => foreign_tables(arg, mods).map(Plan::native),
        Command::Collations => collations(arg, mods).map(Plan::native),
        Command::AccessMethods => access_methods(arg, mods).map(Plan::native),
        Command::Settings => settings(arg, mods).map(Plan::native),
        Command::FunctionSource => function_source(arg),
        Command::ViewSource => view_source(arg),
        Command::ConnectionInfo => Ok(connection_info(ctx)),
        Command::Encoding => encoding(arg),
    };
    Some(built.map_err(AppError::invalid_input))
}

/// psql takes one pattern; a second argument is a mistake worth saying so about.
fn one_argument(rest: &str) -> Result<Option<String>, String> {
    let mut args = raw_args(rest)?;
    match args.len() {
        0 => Ok(None),
        1 => Ok(args.pop()),
        _ => Err("A psql command takes one pattern; quote it if it contains spaces.".to_string()),
    }
}

fn lit(raw: &str) -> String {
    string_literal(raw, Quote::PgEscape)
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

/// A psql name pattern split at its dots: each part is a regular-expression body.
#[derive(Debug, Default, PartialEq, Eq)]
struct Pattern {
    database: Option<String>,
    schema: Option<String>,
    name: Option<String>,
}

// WHAT:  psql's pattern language as regular expressions, one per dotted part.
// HOW:   Unquoted: `*` is `.*`, `?` is `.`, upper case folds to lower, `$` and
//        anything else pass through (so `[0-9]` and `a|b` work, as in psql).
//        Quoted: every character is literal. `""` inside quotes is a quote.
//        More dots than `max_parts` allows is an error, like psql's.
fn pattern(raw: &str, max_parts: usize) -> Result<Pattern, String> {
    let mut parts: Vec<String> = vec![String::new()];
    let mut quoted = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        let current = parts.last_mut().ok_or_else(|| "internal: no pattern part".to_string())?;
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if !quoted && c.is_ascii_uppercase() => current.push(c.to_ascii_lowercase()),
            '*' if !quoted => current.push_str(".*"),
            '?' if !quoted => current.push('.'),
            '.' if !quoted => {
                if parts.len() >= max_parts {
                    return Err(format!("Too many dotted names in \"{raw}\"; at most {max_parts} are allowed here."));
                }
                parts.push(String::new());
            }
            '$' => current.push_str("\\$"),
            c => {
                if quoted && "|*+?()[]{}.^$\\".contains(c) {
                    current.push('\\');
                }
                current.push(c);
            }
        }
    }
    let mut parts = parts.into_iter();
    let found = match (parts.next(), parts.next(), parts.next()) {
        (Some(name), None, _) => Pattern { name: Some(name), ..Pattern::default() },
        (Some(schema), Some(name), None) => Pattern { schema: Some(schema), name: Some(name), ..Pattern::default() },
        (Some(database), Some(schema), Some(name)) => Pattern { database: Some(database), schema: Some(schema), name: Some(name) },
        _ => Pattern::default(),
    };
    Ok(found)
}

/// `column ~ '^(body)$'`, or nothing when the body matches everything.
fn anchored(column: &str, body: &str) -> Option<String> {
    (body != ".*").then(|| format!("{column} ~ {}", lit(&format!("^({body})$"))))
}

// WHAT:  The WHERE conditions a pattern adds: the database, the schema, the name,
//        and — when the pattern names no schema — psql's "visible on the search path".
fn conditions(raw: Option<&str>, max_parts: usize, schema_column: Option<&str>, name_column: &str, visible: Option<&str>) -> Result<Vec<String>, String> {
    let found = match raw {
        Some(raw) => pattern(raw, max_parts)?,
        None => Pattern::default(),
    };
    let mut out = Vec::new();
    if let Some(database) = &found.database {
        out.extend(anchored("pg_catalog.current_database()", database));
    }
    if let Some(name) = &found.name {
        out.extend(anchored(name_column, name));
    }
    match (&found.schema, schema_column) {
        (Some(schema), Some(column)) => out.extend(anchored(column, schema)),
        (Some(_), None) => return Err("This command does not take a schema-qualified name.".to_string()),
        (None, _) => out.extend(visible.map(str::to_string)),
    }
    Ok(out)
}

fn where_clause(parts: &[String]) -> String {
    if parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}\n", parts.join("\n  AND "))
    }
}

const NOT_SYSTEM: &str = "n.nspname <> 'pg_catalog'\n  AND n.nspname !~ '^pg_toast'\n  AND n.nspname <> 'information_schema'";

const RELKIND_LABEL: &str = "CASE c.relkind WHEN 'r' THEN 'table' WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized view' WHEN 'i' THEN 'index' WHEN 'S' THEN 'sequence' WHEN 't' THEN 'TOAST table' WHEN 'f' THEN 'foreign table' WHEN 'p' THEN 'partitioned table' WHEN 'I' THEN 'partitioned index' WHEN 'c' THEN 'composite type' END";

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

fn databases(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let conds = conditions(arg, 1, None, "d.datname", None)?;
    let verbose = if mods.plus {
        ",\n  CASE WHEN pg_catalog.has_database_privilege(d.datname, 'CONNECT') THEN pg_catalog.pg_size_pretty(pg_catalog.pg_database_size(d.datname)) ELSE 'No Access' END AS \"Size\",\n  t.spcname AS \"Tablespace\",\n  pg_catalog.shobj_description(d.oid, 'pg_database') AS \"Description\""
    } else {
        ""
    };
    let join = if mods.plus { "  JOIN pg_catalog.pg_tablespace t ON d.dattablespace = t.oid\n" } else { "" };
    Ok(format!(
        "SELECT d.datname AS \"Name\",\n  pg_catalog.pg_get_userbyid(d.datdba) AS \"Owner\",\n  pg_catalog.pg_encoding_to_char(d.encoding) AS \"Encoding\",\n  d.datcollate AS \"Collate\",\n  d.datctype AS \"Ctype\",\n  pg_catalog.array_to_string(d.datacl, E'\\n') AS \"Access privileges\"{verbose}\nFROM pg_catalog.pg_database d\n{join}{}ORDER BY 1",
        where_clause(&conds)
    ))
}

fn schemas(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let mut conds = Vec::new();
    if !mods.system && arg.is_none() {
        conds.push("n.nspname !~ '^pg_'\n  AND n.nspname <> 'information_schema'".to_string());
    }
    conds.extend(conditions(arg, 1, None, "n.nspname", None)?);
    let verbose = if mods.plus {
        ",\n  pg_catalog.array_to_string(n.nspacl, E'\\n') AS \"Access privileges\",\n  pg_catalog.obj_description(n.oid, 'pg_namespace') AS \"Description\""
    } else {
        ""
    };
    Ok(format!(
        "SELECT n.nspname AS \"Name\",\n  pg_catalog.pg_get_userbyid(n.nspowner) AS \"Owner\"{verbose}\nFROM pg_catalog.pg_namespace n\n{}ORDER BY 1",
        where_clause(&conds)
    ))
}

// WHAT:  `\dt`, `\dv`, `\dm`, `\di`, `\ds`, `\dE` and their combinations (`\dtv`),
//        and `\d` with no name (tables, views, materialized views, sequences, foreign tables).
fn relations(letters: &str, arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let mut kinds: Vec<&str> = Vec::new();
    for letter in letters.chars() {
        let add: &[&str] = match letter {
            't' => &["r", "p"],
            'v' => &["v"],
            'm' => &["m"],
            'i' => &["I", "i"],
            's' => &["S"],
            'E' => &["f"],
            _ => &[],
        };
        for kind in add {
            if !kinds.contains(kind) {
                kinds.push(kind);
            }
        }
    }
    let indexes = kinds.contains(&"i");
    let mut conds = vec![format!("c.relkind IN ({},'')", kinds.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(","))];
    if !mods.system && arg.is_none() {
        conds.push(NOT_SYSTEM.to_string());
    }
    conds.extend(conditions(arg, 3, Some("n.nspname"), "c.relname", Some("pg_catalog.pg_table_is_visible(c.oid)"))?);
    let table_column = if indexes { ",\n  c2.relname AS \"Table\"" } else { "" };
    let verbose = if mods.plus {
        ",\n  CASE c.relpersistence WHEN 'p' THEN 'permanent' WHEN 't' THEN 'temporary' WHEN 'u' THEN 'unlogged' END AS \"Persistence\",\n  am.amname AS \"Access method\",\n  pg_catalog.pg_size_pretty(pg_catalog.pg_table_size(c.oid)) AS \"Size\",\n  pg_catalog.obj_description(c.oid, 'pg_class') AS \"Description\""
    } else {
        ""
    };
    let joins = format!(
        "{}{}",
        if indexes { "  LEFT JOIN pg_catalog.pg_index i ON i.indexrelid = c.oid\n  LEFT JOIN pg_catalog.pg_class c2 ON i.indrelid = c2.oid\n" } else { "" },
        if mods.plus { "  LEFT JOIN pg_catalog.pg_am am ON am.oid = c.relam\n" } else { "" }
    );
    Ok(format!(
        "SELECT n.nspname AS \"Schema\",\n  c.relname AS \"Name\",\n  {RELKIND_LABEL} AS \"Type\",\n  pg_catalog.pg_get_userbyid(c.relowner) AS \"Owner\"{table_column}{verbose}\nFROM pg_catalog.pg_class c\n  LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\n{joins}{}ORDER BY 1, 2",
        where_clause(&conds)
    ))
}

fn functions(letters: &str, arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let mut kinds: Vec<&str> = Vec::new();
    for letter in letters.chars() {
        let kind = match letter {
            'a' => "p.prokind = 'a'",
            'n' => "(p.prokind = 'f' AND p.prorettype <> 'pg_catalog.trigger'::pg_catalog.regtype)",
            'p' => "p.prokind = 'p'",
            't' => "(p.prokind = 'f' AND p.prorettype = 'pg_catalog.trigger'::pg_catalog.regtype)",
            'w' => "p.prokind = 'w'",
            other => return Err(format!("\\df does not know the function kind \"{other}\"; use a, n, p, t or w.")),
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    let mut conds = Vec::new();
    if !kinds.is_empty() {
        conds.push(format!("({})", kinds.join(" OR ")));
    }
    if !mods.system && arg.is_none() {
        conds.push("n.nspname <> 'pg_catalog'\n  AND n.nspname <> 'information_schema'".to_string());
    }
    conds.extend(conditions(arg, 2, Some("n.nspname"), "p.proname", Some("pg_catalog.pg_function_is_visible(p.oid)"))?);
    let verbose = if mods.plus {
        ",\n  CASE p.provolatile WHEN 'i' THEN 'immutable' WHEN 's' THEN 'stable' WHEN 'v' THEN 'volatile' END AS \"Volatility\",\n  pg_catalog.pg_get_userbyid(p.proowner) AS \"Owner\",\n  l.lanname AS \"Language\",\n  p.prosrc AS \"Source code\",\n  pg_catalog.obj_description(p.oid, 'pg_proc') AS \"Description\""
    } else {
        ""
    };
    let join = if mods.plus { "  LEFT JOIN pg_catalog.pg_language l ON l.oid = p.prolang\n" } else { "" };
    Ok(format!(
        "SELECT n.nspname AS \"Schema\",\n  p.proname AS \"Name\",\n  pg_catalog.pg_get_function_result(p.oid) AS \"Result data type\",\n  pg_catalog.pg_get_function_arguments(p.oid) AS \"Argument data types\",\n  CASE p.prokind WHEN 'a' THEN 'agg' WHEN 'w' THEN 'window' WHEN 'p' THEN 'proc' ELSE 'func' END AS \"Type\"{verbose}\nFROM pg_catalog.pg_proc p\n  LEFT JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace\n{join}{}ORDER BY 1, 2, 4",
        where_clause(&conds)
    ))
}

fn roles(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let mut conds = Vec::new();
    if !mods.system && arg.is_none() {
        conds.push("r.rolname !~ '^pg_'".to_string());
    }
    conds.extend(conditions(arg, 1, None, "r.rolname", None)?);
    let verbose = if mods.plus { ",\n  pg_catalog.shobj_description(r.oid, 'pg_authid') AS \"Description\"" } else { "" };
    Ok(format!(
        "SELECT r.rolname AS \"Role name\",\n  pg_catalog.concat_ws(', ',\n    CASE WHEN r.rolsuper THEN 'Superuser' END,\n    CASE WHEN NOT r.rolinherit THEN 'No inheritance' END,\n    CASE WHEN r.rolcreaterole THEN 'Create role' END,\n    CASE WHEN r.rolcreatedb THEN 'Create DB' END,\n    CASE WHEN NOT r.rolcanlogin THEN 'Cannot login' END,\n    CASE WHEN r.rolreplication THEN 'Replication' END,\n    CASE WHEN r.rolbypassrls THEN 'Bypass RLS' END,\n    CASE WHEN r.rolconnlimit >= 0 THEN r.rolconnlimit::pg_catalog.text || ' connections' END) AS \"Attributes\",\n  pg_catalog.array_to_string(ARRAY(SELECT b.rolname FROM pg_catalog.pg_auth_members m JOIN pg_catalog.pg_roles b ON m.roleid = b.oid WHERE m.member = r.oid ORDER BY 1), ', ') AS \"Member of\"{verbose}\nFROM pg_catalog.pg_roles r\n{}ORDER BY 1",
        where_clause(&conds)
    ))
}

// WHAT:  `\dx`: the extensions; with `+`, the objects each one owns.
fn extensions(arg: Option<&str>, mods: Mods) -> Result<Plan, String> {
    let conds = conditions(arg, 1, None, "e.extname", None)?;
    if mods.plus {
        return Ok(Plan::native(format!(
            "SELECT e.extname AS \"Extension\",\n  pg_catalog.pg_describe_object(d.classid, d.objid, 0) AS \"Object description\"\nFROM pg_catalog.pg_depend d\n  JOIN pg_catalog.pg_extension e ON d.refobjid = e.oid\nWHERE d.refclassid = 'pg_catalog.pg_extension'::pg_catalog.regclass\n  AND d.deptype = 'e'\n{}ORDER BY 1, 2",
            conds.iter().map(|c| format!("  AND {c}\n")).collect::<String>()
        )));
    }
    Ok(Plan::native(format!(
        "SELECT e.extname AS \"Name\",\n  e.extversion AS \"Version\",\n  n.nspname AS \"Schema\",\n  c.description AS \"Description\"\nFROM pg_catalog.pg_extension e\n  LEFT JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace\n  LEFT JOIN pg_catalog.pg_description c ON c.objoid = e.oid AND c.classoid = 'pg_catalog.pg_extension'::pg_catalog.regclass\n{}ORDER BY 1",
        where_clause(&conds)
    )))
}

// WHAT:  `\dT` (data types) and `\dD` (domains).
fn types(arg: Option<&str>, mods: Mods, domains: bool) -> Result<String, String> {
    let mut conds = Vec::new();
    if domains {
        conds.push("t.typtype = 'd'".to_string());
    } else {
        conds.push("(t.typrelid = 0 OR (SELECT c.relkind = 'c' FROM pg_catalog.pg_class c WHERE c.oid = t.typrelid))\n  AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_type el WHERE el.oid = t.typelem AND el.typarray = t.oid)".to_string());
    }
    if !mods.system && arg.is_none() {
        conds.push("n.nspname <> 'pg_catalog'\n  AND n.nspname <> 'information_schema'".to_string());
    }
    conds.extend(conditions(arg, 2, Some("n.nspname"), "t.typname", Some("pg_catalog.pg_type_is_visible(t.oid)"))?);
    let body = if domains {
        "pg_catalog.format_type(t.typbasetype, t.typtypmod) AS \"Type\",\n  (SELECT c.collname FROM pg_catalog.pg_collation c, pg_catalog.pg_type bt WHERE c.oid = t.typcollation AND bt.oid = t.typbasetype AND t.typcollation <> bt.typcollation) AS \"Collation\",\n  CASE WHEN t.typnotnull THEN 'not null' END AS \"Nullable\",\n  t.typdefault AS \"Default\",\n  pg_catalog.array_to_string(ARRAY(SELECT pg_catalog.pg_get_constraintdef(r.oid, true) FROM pg_catalog.pg_constraint r WHERE t.oid = r.contypid), ' ') AS \"Check\""
    } else {
        "pg_catalog.format_type(t.oid, NULL) AS \"Name\",\n  CASE t.typtype WHEN 'b' THEN 'base' WHEN 'c' THEN 'composite' WHEN 'd' THEN 'domain' WHEN 'e' THEN 'enum' WHEN 'p' THEN 'pseudo' WHEN 'r' THEN 'range' WHEN 'm' THEN 'multirange' END AS \"Kind\",\n  CASE WHEN t.typtype = 'e' THEN (SELECT pg_catalog.array_to_string(ARRAY(SELECT e.enumlabel FROM pg_catalog.pg_enum e WHERE e.enumtypid = t.oid ORDER BY e.enumsortorder), E'\\n')) END AS \"Elements\""
    };
    let owner = if mods.plus { ",\n  pg_catalog.pg_get_userbyid(t.typowner) AS \"Owner\",\n  pg_catalog.obj_description(t.oid, 'pg_type') AS \"Description\"" } else { "" };
    Ok(format!(
        "SELECT n.nspname AS \"Schema\",\n  {}{body}{owner}\nFROM pg_catalog.pg_type t\n  LEFT JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace\n{}ORDER BY 1, 2",
        if domains { "t.typname AS \"Name\",\n  " } else { "" },
        where_clause(&conds)
    ))
}

fn tablespaces(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let conds = conditions(arg, 1, None, "spcname", None)?;
    let verbose = if mods.plus {
        ",\n  pg_catalog.array_to_string(spcacl, E'\\n') AS \"Access privileges\",\n  pg_catalog.pg_size_pretty(pg_catalog.pg_tablespace_size(oid)) AS \"Size\",\n  pg_catalog.shobj_description(oid, 'pg_tablespace') AS \"Description\""
    } else {
        ""
    };
    Ok(format!(
        "SELECT spcname AS \"Name\",\n  pg_catalog.pg_get_userbyid(spcowner) AS \"Owner\",\n  pg_catalog.pg_tablespace_location(oid) AS \"Location\"{verbose}\nFROM pg_catalog.pg_tablespace\n{}ORDER BY 1",
        where_clause(&conds)
    ))
}

fn languages(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let mut conds = Vec::new();
    if !mods.system && arg.is_none() {
        conds.push("l.lanplcallfoid != 0".to_string());
    }
    conds.extend(conditions(arg, 1, None, "l.lanname", None)?);
    let verbose = if mods.plus { ",\n  pg_catalog.array_to_string(l.lanacl, E'\\n') AS \"Access privileges\",\n  pg_catalog.obj_description(l.oid, 'pg_language') AS \"Description\"" } else { "" };
    Ok(format!(
        "SELECT l.lanname AS \"Name\",\n  pg_catalog.pg_get_userbyid(l.lanowner) AS \"Owner\",\n  l.lanpltrusted AS \"Trusted\"{verbose}\nFROM pg_catalog.pg_language l\n{}ORDER BY 1",
        where_clause(&conds)
    ))
}

// WHAT:  `\dp` / `\z`: who may do what to which relation.
fn privileges(arg: Option<&str>) -> Result<String, String> {
    let mut conds = vec!["c.relkind IN ('r','v','m','S','f','p')".to_string()];
    if arg.is_none() {
        conds.push(NOT_SYSTEM.to_string());
    }
    conds.extend(conditions(arg, 3, Some("n.nspname"), "c.relname", Some("pg_catalog.pg_table_is_visible(c.oid)"))?);
    Ok(format!(
        "SELECT n.nspname AS \"Schema\",\n  c.relname AS \"Name\",\n  {RELKIND_LABEL} AS \"Type\",\n  pg_catalog.array_to_string(c.relacl, E'\\n') AS \"Access privileges\",\n  pg_catalog.array_to_string(ARRAY(SELECT a.attname || E':\\n  ' || pg_catalog.array_to_string(a.attacl, E'\\n  ') FROM pg_catalog.pg_attribute a WHERE a.attrelid = c.oid AND NOT a.attisdropped AND a.attacl IS NOT NULL ORDER BY a.attnum), E'\\n') AS \"Column privileges\"\nFROM pg_catalog.pg_class c\n  LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\n{}ORDER BY 1, 2",
        where_clause(&conds)
    ))
}

fn event_triggers(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let conds = conditions(arg, 1, None, "e.evtname", None)?;
    let verbose = if mods.plus { ",\n  e.evttags AS \"Tags\",\n  pg_catalog.obj_description(e.oid, 'pg_event_trigger') AS \"Description\"" } else { "" };
    Ok(format!(
        "SELECT e.evtname AS \"Name\",\n  e.evtevent AS \"Event\",\n  pg_catalog.pg_get_userbyid(e.evtowner) AS \"Owner\",\n  CASE e.evtenabled WHEN 'O' THEN 'enabled' WHEN 'R' THEN 'replica' WHEN 'A' THEN 'always' WHEN 'D' THEN 'disabled' END AS \"Enabled\",\n  e.evtfoid::pg_catalog.regproc AS \"Function\"{verbose}\nFROM pg_catalog.pg_event_trigger e\n{}ORDER BY 1",
        where_clause(&conds)
    ))
}

fn publications(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let conds = conditions(arg, 1, None, "p.pubname", None)?;
    let verbose = if mods.plus { ",\n  p.pubviaroot AS \"Via root\"" } else { "" };
    Ok(format!(
        "SELECT p.pubname AS \"Name\",\n  pg_catalog.pg_get_userbyid(p.pubowner) AS \"Owner\",\n  p.puballtables AS \"All tables\",\n  p.pubinsert AS \"Inserts\",\n  p.pubupdate AS \"Updates\",\n  p.pubdelete AS \"Deletes\",\n  p.pubtruncate AS \"Truncates\"{verbose}\nFROM pg_catalog.pg_publication p\n{}ORDER BY 1",
        where_clause(&conds)
    ))
}

fn subscriptions(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let conds = conditions(arg, 1, None, "s.subname", None)?;
    let verbose = if mods.plus { ",\n  s.subpublications AS \"Publication\",\n  s.subsynccommit AS \"Synchronous commit\",\n  s.subconninfo AS \"Conninfo\"" } else { "" };
    let mut all = vec!["s.subdbid = (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname = pg_catalog.current_database())".to_string()];
    all.extend(conds);
    Ok(format!(
        "SELECT s.subname AS \"Name\",\n  pg_catalog.pg_get_userbyid(s.subowner) AS \"Owner\",\n  s.subenabled AS \"Enabled\"{verbose}\nFROM pg_catalog.pg_subscription s\n{}ORDER BY 1",
        where_clause(&all)
    ))
}

fn foreign_servers(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let conds = conditions(arg, 1, None, "s.srvname", None)?;
    let verbose = if mods.plus { ",\n  s.srvtype AS \"Type\",\n  s.srvversion AS \"Version\",\n  pg_catalog.array_to_string(s.srvoptions, ', ') AS \"FDW options\"" } else { "" };
    Ok(format!(
        "SELECT s.srvname AS \"Name\",\n  pg_catalog.pg_get_userbyid(s.srvowner) AS \"Owner\",\n  f.fdwname AS \"Foreign-data wrapper\"{verbose}\nFROM pg_catalog.pg_foreign_server s\n  JOIN pg_catalog.pg_foreign_data_wrapper f ON f.oid = s.srvfdw\n{}ORDER BY 1",
        where_clause(&conds)
    ))
}

fn foreign_wrappers(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let conds = conditions(arg, 1, None, "fdw.fdwname", None)?;
    let verbose = if mods.plus { ",\n  pg_catalog.array_to_string(fdw.fdwoptions, ', ') AS \"FDW options\"" } else { "" };
    Ok(format!(
        "SELECT fdw.fdwname AS \"Name\",\n  pg_catalog.pg_get_userbyid(fdw.fdwowner) AS \"Owner\",\n  fdw.fdwhandler::pg_catalog.regproc AS \"Handler\",\n  fdw.fdwvalidator::pg_catalog.regproc AS \"Validator\"{verbose}\nFROM pg_catalog.pg_foreign_data_wrapper fdw\n{}ORDER BY 1",
        where_clause(&conds)
    ))
}

fn foreign_tables(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let conds = conditions(arg, 2, Some("n.nspname"), "c.relname", Some("pg_catalog.pg_table_is_visible(c.oid)"))?;
    let verbose = if mods.plus { ",\n  pg_catalog.array_to_string(ft.ftoptions, ', ') AS \"FDW options\"" } else { "" };
    Ok(format!(
        "SELECT n.nspname AS \"Schema\",\n  c.relname AS \"Table\",\n  s.srvname AS \"Server\"{verbose}\nFROM pg_catalog.pg_foreign_table ft\n  JOIN pg_catalog.pg_class c ON c.oid = ft.ftrelid\n  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\n  JOIN pg_catalog.pg_foreign_server s ON s.oid = ft.ftserver\n{}ORDER BY 1, 2",
        where_clause(&conds)
    ))
}

fn collations(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let mut conds = vec!["c.collencoding IN (-1, (SELECT d.encoding FROM pg_catalog.pg_database d WHERE d.datname = pg_catalog.current_database()))".to_string()];
    if !mods.system && arg.is_none() {
        conds.push("n.nspname <> 'pg_catalog'\n  AND n.nspname <> 'information_schema'".to_string());
    }
    conds.extend(conditions(arg, 2, Some("n.nspname"), "c.collname", Some("pg_catalog.pg_collation_is_visible(c.oid)"))?);
    let verbose = if mods.plus { ",\n  pg_catalog.obj_description(c.oid, 'pg_collation') AS \"Description\"" } else { "" };
    Ok(format!(
        "SELECT n.nspname AS \"Schema\",\n  c.collname AS \"Name\",\n  c.collcollate AS \"Collate\",\n  c.collctype AS \"Ctype\"{verbose}\nFROM pg_catalog.pg_collation c\n  JOIN pg_catalog.pg_namespace n ON n.oid = c.collnamespace\n{}ORDER BY 1, 2",
        where_clause(&conds)
    ))
}

fn access_methods(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let conds = conditions(arg, 1, None, "a.amname", None)?;
    let verbose = if mods.plus { ",\n  a.amhandler::pg_catalog.regproc AS \"Handler\",\n  pg_catalog.obj_description(a.oid, 'pg_am') AS \"Description\"" } else { "" };
    Ok(format!(
        "SELECT a.amname AS \"Name\",\n  CASE a.amtype WHEN 'i' THEN 'Index' WHEN 't' THEN 'Table' END AS \"Type\"{verbose}\nFROM pg_catalog.pg_am a\n{}ORDER BY 1",
        where_clause(&conds)
    ))
}

// WHAT:  `\dconfig`: the server's settings, without the internal ones.
fn settings(arg: Option<&str>, mods: Mods) -> Result<String, String> {
    let mut conds = vec!["s.context <> 'internal'".to_string()];
    conds.extend(conditions(arg, 1, None, "s.name", None)?);
    let verbose = if mods.plus { ",\n  s.vartype AS \"Type\",\n  s.context AS \"Context\",\n  s.short_desc AS \"Description\"" } else { "" };
    Ok(format!(
        "SELECT s.name AS \"Parameter\",\n  s.setting AS \"Value\"{verbose}\nFROM pg_catalog.pg_settings s\n{}ORDER BY 1",
        where_clause(&conds)
    ))
}

// ---------------------------------------------------------------------------
// \d NAME
// ---------------------------------------------------------------------------

// WHAT:  The relations a name or pattern picks, as a subquery of their OIDs.
fn relation_oids(pattern: &str) -> Result<String, String> {
    let mut conds = vec!["c.relkind IN ('r','p','v','m','i','I','S','f','c')".to_string()];
    conds.extend(conditions(Some(pattern), 3, Some("n.nspname"), "c.relname", Some("pg_catalog.pg_table_is_visible(c.oid)"))?);
    Ok(format!(
        "SELECT c.oid FROM pg_catalog.pg_class c LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace {}",
        where_clause(&conds).trim_end().replace('\n', " ")
    ))
}

// WHAT:  `\d NAME`: an overview, then the columns, indexes, constraints, foreign
//        keys that point here and triggers of every relation the name matches —
//        and for views, sequences and indexes what is particular to them.
// HOW:   Every query selects "Table" first (schema.name), so a pattern that matches
//        several relations stays readable. The overview must find something; the
//        sections are dropped when the relation has none, like psql's.
fn describe(pattern: &str, mods: Mods) -> Result<Plan, String> {
    let oids = relation_oids(pattern)?;
    let qualified = "n.nspname || '.' || c.relname";
    let overview_verbose = if mods.plus {
        ",\n  pg_catalog.pg_size_pretty(pg_catalog.pg_total_relation_size(c.oid)) AS \"Total size\",\n  pg_catalog.array_to_string(c.reloptions, ', ') AS \"Options\",\n  pg_catalog.obj_description(c.oid, 'pg_class') AS \"Description\""
    } else {
        ""
    };
    let column_verbose = if mods.plus {
        ",\n  CASE a.attstorage WHEN 'p' THEN 'plain' WHEN 'e' THEN 'external' WHEN 'm' THEN 'main' WHEN 'x' THEN 'extended' END AS \"Storage\",\n  pg_catalog.col_description(a.attrelid, a.attnum) AS \"Description\""
    } else {
        ""
    };
    let overview = format!(
        "SELECT n.nspname AS \"Schema\",\n  c.relname AS \"Name\",\n  {RELKIND_LABEL} AS \"Type\",\n  pg_catalog.pg_get_userbyid(c.relowner) AS \"Owner\",\n  CASE c.relpersistence WHEN 'p' THEN 'permanent' WHEN 't' THEN 'temporary' WHEN 'u' THEN 'unlogged' END AS \"Persistence\",\n  CASE WHEN c.reltuples < 0 THEN NULL ELSE c.reltuples::pg_catalog.int8 END AS \"Estimated rows\"{overview_verbose}\nFROM pg_catalog.pg_class c\n  LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\nWHERE c.oid IN ({oids})\nORDER BY 1, 2"
    );
    let columns = format!(
        "SELECT {qualified} AS \"Table\",\n  a.attname AS \"Column\",\n  pg_catalog.format_type(a.atttypid, a.atttypmod) AS \"Type\",\n  (SELECT co.collname FROM pg_catalog.pg_collation co, pg_catalog.pg_type t WHERE co.oid = a.attcollation AND t.oid = a.atttypid AND a.attcollation <> t.typcollation) AS \"Collation\",\n  CASE WHEN a.attnotnull THEN 'not null' ELSE '' END AS \"Nullable\",\n  CASE WHEN a.attidentity = 'a' THEN 'generated always as identity'\n       WHEN a.attidentity = 'd' THEN 'generated by default as identity'\n       WHEN a.attgenerated = 's' THEN 'generated always as (' || pg_catalog.pg_get_expr(d.adbin, d.adrelid, true) || ') stored'\n       ELSE pg_catalog.pg_get_expr(d.adbin, d.adrelid, true) END AS \"Default\"{column_verbose}\nFROM pg_catalog.pg_attribute a\n  JOIN pg_catalog.pg_class c ON c.oid = a.attrelid\n  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\n  LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum AND a.atthasdef\nWHERE a.attnum > 0\n  AND NOT a.attisdropped\n  AND a.attrelid IN ({oids})\nORDER BY n.nspname, c.relname, a.attnum"
    );
    let indexes = format!(
        "SELECT {qualified} AS \"Table\",\n  c2.relname AS \"Index\",\n  CASE WHEN i.indisprimary THEN 'primary key' WHEN i.indisunique THEN 'unique' ELSE '' END AS \"Kind\",\n  pg_catalog.pg_get_indexdef(i.indexrelid, 0, true) AS \"Definition\",\n  CASE WHEN i.indisvalid THEN '' ELSE 'invalid' END AS \"State\"\nFROM pg_catalog.pg_index i\n  JOIN pg_catalog.pg_class c ON c.oid = i.indrelid\n  JOIN pg_catalog.pg_class c2 ON c2.oid = i.indexrelid\n  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\nWHERE i.indrelid IN ({oids})\nORDER BY 1, i.indisprimary DESC, 2"
    );
    let constraints = format!(
        "SELECT {qualified} AS \"Table\",\n  r.conname AS \"Constraint\",\n  CASE r.contype WHEN 'c' THEN 'check' WHEN 'f' THEN 'foreign key' WHEN 't' THEN 'trigger' ELSE r.contype::pg_catalog.text END AS \"Type\",\n  pg_catalog.pg_get_constraintdef(r.oid, true) AS \"Definition\"\nFROM pg_catalog.pg_constraint r\n  JOIN pg_catalog.pg_class c ON c.oid = r.conrelid\n  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\nWHERE r.contype IN ('c','f','t')\n  AND r.conrelid IN ({oids})\nORDER BY 1, 2"
    );
    let referenced = format!(
        "SELECT {qualified} AS \"Table\",\n  r.conrelid::pg_catalog.regclass::pg_catalog.text AS \"Referenced by\",\n  r.conname AS \"Constraint\",\n  pg_catalog.pg_get_constraintdef(r.oid, true) AS \"Definition\"\nFROM pg_catalog.pg_constraint r\n  JOIN pg_catalog.pg_class c ON c.oid = r.confrelid\n  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\nWHERE r.contype = 'f'\n  AND r.confrelid IN ({oids})\nORDER BY 1, 2, 3"
    );
    let triggers = format!(
        "SELECT {qualified} AS \"Table\",\n  t.tgname AS \"Trigger\",\n  pg_catalog.pg_get_triggerdef(t.oid, true) AS \"Definition\",\n  CASE t.tgenabled WHEN 'O' THEN 'enabled' WHEN 'D' THEN 'disabled' WHEN 'R' THEN 'replica' WHEN 'A' THEN 'always' END AS \"Enabled\"\nFROM pg_catalog.pg_trigger t\n  JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid\n  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\nWHERE NOT t.tgisinternal\n  AND t.tgrelid IN ({oids})\nORDER BY 1, 2"
    );
    let missing = format!("Did not find any relation named \"{pattern}\".");
    let mut plan = Plan::default()
        .then(Step::Exists(overview, missing))
        .then(Step::Native(columns))
        .then(Step::Section(indexes))
        .then(Step::Section(constraints))
        .then(Step::Section(referenced))
        .then(Step::Section(triggers));
    // Particular to one kind: a sequence's parameters, an index's definition,
    // and (with +) a view's query. Each is dropped when the match has none.
    plan = plan.then(Step::Section(format!(
        "SELECT {qualified} AS \"Sequence\",\n  s.data_type::pg_catalog.text AS \"Type\",\n  s.start_value AS \"Start\",\n  s.min_value AS \"Minimum\",\n  s.max_value AS \"Maximum\",\n  s.increment_by AS \"Increment\",\n  s.cycle AS \"Cycles\",\n  s.cache_size AS \"Cache\",\n  s.last_value AS \"Last value\"\nFROM pg_catalog.pg_sequences s\n  JOIN pg_catalog.pg_namespace n ON n.nspname = s.schemaname\n  JOIN pg_catalog.pg_class c ON c.relnamespace = n.oid AND c.relname = s.sequencename\nWHERE c.oid IN ({oids})\nORDER BY 1"
    )));
    plan = plan.then(Step::Section(format!(
        "SELECT {qualified} AS \"Index\",\n  i.indrelid::pg_catalog.regclass::pg_catalog.text AS \"On table\",\n  pg_catalog.pg_get_indexdef(c.oid, 0, true) AS \"Definition\"\nFROM pg_catalog.pg_index i\n  JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid\n  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\nWHERE c.oid IN ({oids})\nORDER BY 1"
    )));
    if mods.plus {
        plan = plan.then(Step::Section(format!(
            "SELECT {qualified} AS \"View\",\n  pg_catalog.pg_get_viewdef(c.oid, true) AS \"Definition\"\nFROM pg_catalog.pg_class c\n  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\nWHERE c.relkind IN ('v','m')\n  AND c.oid IN ({oids})\nORDER BY 1"
        )));
    }
    Ok(plan)
}

// ---------------------------------------------------------------------------
// \sf, \sv
// ---------------------------------------------------------------------------

fn function_source(arg: Option<&str>) -> Result<Plan, String> {
    let Some(name) = arg else {
        return Err("\\sf needs a function name, e.g. \\sf add_one or \\sf add_one(integer).".to_string());
    };
    // `name(types)` names one function exactly; a bare name matches every overload.
    if name.contains('(') {
        return Ok(Plan::native(format!("SELECT pg_catalog.pg_get_functiondef({}::pg_catalog.regprocedure) AS \"Definition\"", lit(name))));
    }
    let mut conds = vec!["p.prokind IN ('f','p','a','w')".to_string()];
    conds.extend(conditions(Some(name), 2, Some("n.nspname"), "p.proname", Some("pg_catalog.pg_function_is_visible(p.oid)"))?);
    Ok(Plan::native(format!(
        "SELECT pg_catalog.pg_get_functiondef(p.oid) AS \"Definition\"\nFROM pg_catalog.pg_proc p\n  LEFT JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace\n{}ORDER BY n.nspname, p.proname, p.oid",
        where_clause(&conds)
    )))
}

fn view_source(arg: Option<&str>) -> Result<Plan, String> {
    let Some(name) = arg else {
        return Err("\\sv needs a view name, e.g. \\sv big_orders.".to_string());
    };
    let mut conds = vec!["c.relkind = 'v'".to_string()];
    conds.extend(conditions(Some(name), 2, Some("n.nspname"), "c.relname", Some("pg_catalog.pg_table_is_visible(c.oid)"))?);
    Ok(Plan::native(format!(
        "SELECT 'CREATE OR REPLACE VIEW ' || c.oid::pg_catalog.regclass::pg_catalog.text || ' AS' || E'\\n' || pg_catalog.pg_get_viewdef(c.oid, true) AS \"Definition\"\nFROM pg_catalog.pg_class c\n  LEFT JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\n{}ORDER BY n.nspname, c.relname",
        where_clause(&conds)
    )))
}

// ---------------------------------------------------------------------------
// \conninfo, \encoding
// ---------------------------------------------------------------------------

// WHAT:  `\conninfo`: the saved connection's facts, then what the server says about the session.
fn connection_info(ctx: &Ctx) -> Plan {
    let mut rows: Vec<Vec<String>> = Vec::new();
    if let Some(connection) = ctx.connection {
        let show = |value: Option<&str>| value.filter(|v| !v.is_empty()).unwrap_or("(default)").to_string();
        rows.push(vec!["Connection".to_string(), connection.name.clone()]);
        rows.push(vec!["Host".to_string(), show(connection.host.as_deref())]);
        rows.push(vec!["Port".to_string(), connection.port.map_or_else(|| "(default)".to_string(), |p| p.to_string())]);
        rows.push(vec!["User".to_string(), show(connection.username.as_deref())]);
        rows.push(vec!["Database".to_string(), show(connection.database.as_deref())]);
    }
    let session = Step::Native("SELECT pg_catalog.current_database() AS \"Database\",\n  CURRENT_USER AS \"User\",\n  pg_catalog.current_setting('server_version') AS \"Server version\",\n  pg_catalog.pg_backend_pid() AS \"Backend PID\"".to_string());
    if rows.is_empty() {
        Plan { steps: vec![session] }
    } else {
        Plan::table(Table::new(&["Parameter", "Value"], rows)).then(session)
    }
}

fn encoding(arg: Option<&str>) -> Result<Plan, String> {
    if arg.is_some() {
        return Err("DB Free shows the encoding but does not change it: \\encoding takes no argument.".to_string());
    }
    Ok(Plan::native("SELECT pg_catalog.current_setting('client_encoding') AS \"Client encoding\",\n  pg_catalog.current_setting('server_encoding') AS \"Server encoding\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::destructive::{classify, StatementKind};
    use crate::integrations::shell_meta::{plan as plan_for, Segment};
    use crate::model::Engine;

    fn planned(text: &str) -> Plan {
        match plan_for(&Ctx::bare(Engine::Postgres), text) {
            Some(Ok(plan)) => plan,
            other => panic!("{text}: {other:?}"),
        }
    }

    fn sql(text: &str) -> String {
        planned(text).statements().next().unwrap_or_default().to_string()
    }

    fn refusal(text: &str) -> String {
        match plan_for(&Ctx::bare(Engine::Postgres), text) {
            Some(Err(e)) => e.message().to_string(),
            other => panic!("{text}: expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn patterns_follow_psqls_rules() {
        let p = |raw: &str, max: usize| pattern(raw, max).unwrap_or_default();
        assert_eq!(p("users", 3), Pattern { name: Some("users".into()), ..Pattern::default() });
        assert_eq!(p("Public.Us*", 3), Pattern { schema: Some("public".into()), name: Some("us.*".into()), database: None });
        assert_eq!(p("a?c", 3).name.as_deref(), Some("a.c"));
        assert_eq!(p("\"Mixed.Case\".\"T\"", 3), Pattern { schema: Some("Mixed\\.Case".into()), name: Some("T".into()), database: None });
        assert_eq!(p("db.s.t", 3), Pattern { database: Some("db".into()), schema: Some("s".into()), name: Some("t".into()) });
        assert_eq!(p("\"a*b\"", 1).name.as_deref(), Some("a\\*b"), "quotes make wildcards literal");
        assert_eq!(p("\"say \"\"hi\"\"\"", 1).name.as_deref(), Some("say \"hi\""));
        assert_eq!(p("a$b", 1).name.as_deref(), Some("a\\$b"));
        assert_eq!(p("[0-9]+|x", 1).name.as_deref(), Some("[0-9]+|x"), "unquoted regular-expression syntax passes through");
        assert!(pattern("a.b.c.d", 3).is_err());
        assert!(pattern("a.b", 1).is_err(), "a role pattern has no schema part");
    }

    #[test]
    fn a_pattern_is_one_string_literal_nothing_can_leave() {
        let statement = sql("\\dt \"x'; DROP TABLE users; --\"");
        assert!(statement.contains("c.relname ~ E'^(x''; DROP TABLE users; --)$'"), "{statement}");
        assert!(classify(&statement).iter().all(|s| s.kind == StatementKind::Read), "{statement}");
        let backslash = sql("\\dt \"a\\\"");
        assert!(backslash.contains("E'^(a\\\\\\\\)$'"), "a backslash is escaped twice, once for the pattern and once for the literal: {backslash}");
        let schema = sql("\\dn \"it's\"");
        assert!(schema.contains("n.nspname ~ E'^(it''s)$'"), "{schema}");
        assert!(refusal("\\dt a b").contains("one pattern"));
        assert!(refusal("\\dt 'open").contains("never closed"));
    }

    #[test]
    fn lists_follow_psql_system_and_visibility_rules() {
        let tables = sql("\\dt");
        assert!(tables.contains("c.relkind IN ('r','p','')") && tables.contains("n.nspname <> 'pg_catalog'") && tables.contains("pg_catalog.pg_table_is_visible(c.oid)"), "{tables}");
        let system = sql("\\dtS");
        assert!(!system.contains("n.nspname <> 'pg_catalog'") && system.contains("pg_table_is_visible"), "{system}");
        let pattern = sql("\\dt public.*");
        assert!(pattern.contains("n.nspname ~ E'^(public)$'") && !pattern.contains("c.relname ~") && !pattern.contains("pg_table_is_visible") && !pattern.contains("<> 'pg_catalog'"), "a pattern shows what it names: {pattern}");
        let mixed = sql("\\dtvm");
        assert!(mixed.contains("IN ('r','p','v','m','')"), "{mixed}");
        assert!(sql("\\di").contains("c2.relname AS \"Table\"") && sql("\\di").contains("'I','i'"));
        assert!(sql("\\ds").contains("IN ('S','')") && sql("\\dE").contains("IN ('f','')"));
        let plain = sql("\\d");
        assert!(plain.contains("'r','p','v','m','S','f'"), "\\d alone lists relations: {plain}");
        assert!(sql("\\dt+").contains("pg_table_size") && sql("\\dt+").contains("\"Description\""));
        assert!(sql("\\l").contains("pg_database d") && sql("\\l+").contains("pg_database_size"));
        assert!(sql("\\list").contains("pg_database d"));
    }

    #[test]
    fn schemas_roles_functions_and_the_rest() {
        let schemas = sql("\\dn");
        assert!(schemas.contains("n.nspname !~ '^pg_'") && !sql("\\dnS").contains("!~ '^pg_'") && !sql("\\dn pg_*").contains("!~ '^pg_'"), "{schemas}");
        assert!(sql("\\du").contains("pg_roles r") && sql("\\dg").contains("pg_roles r") && sql("\\du").contains("!~ '^pg_'"));
        assert!(sql("\\du+").contains("shobj_description"));
        let functions = sql("\\df");
        assert!(functions.contains("pg_get_function_arguments") && functions.contains("pg_function_is_visible") && functions.contains("n.nspname <> 'pg_catalog'"), "{functions}");
        let kinds = sql("\\dfpa");
        assert!(kinds.contains("p.prokind = 'p'") && kinds.contains("p.prokind = 'a'") && kinds.contains(" OR "), "{kinds}");
        assert!(sql("\\df+").contains("l.lanname"));
        assert!(refusal("\\dfz").contains("function kind"));
        assert!(sql("\\dx").contains("pg_extension e"));
        assert!(sql("\\dx+").contains("pg_depend d") && sql("\\dx+ postgis").contains("e.extname ~ E'^(postgis)$'"));
        assert!(sql("\\dT").contains("pg_type t") && sql("\\dD").contains("t.typtype = 'd'"));
        assert!(sql("\\db").contains("pg_tablespace") && sql("\\dL").contains("pg_language") && sql("\\dp").contains("relacl") && sql("\\z").contains("relacl"));
        assert!(sql("\\dy").contains("pg_event_trigger") && sql("\\dRp").contains("pg_publication") && sql("\\dRs").contains("pg_subscription"));
        assert!(sql("\\des").contains("pg_foreign_server") && sql("\\dew").contains("pg_foreign_data_wrapper") && sql("\\det").contains("pg_foreign_table"));
        assert!(sql("\\dO").contains("pg_collation") && sql("\\dA").contains("pg_am") && sql("\\dconfig").contains("pg_settings"));
        assert!(sql("\\dconfig+ work*").contains("s.name ~ E'^(work.*)$'"));
    }

    #[test]
    fn describe_runs_one_query_per_section_and_drops_the_empty_ones() {
        let plan = planned("\\d users");
        let kinds: Vec<&str> = plan
            .steps
            .iter()
            .map(|step| match step {
                Step::Exists(..) => "exists",
                Step::Native(_) => "native",
                Step::Section(_) => "section",
                Step::Table(_) => "table",
            })
            .collect();
        assert_eq!(kinds, ["exists", "native", "section", "section", "section", "section", "section", "section"]);
        let Step::Exists(overview, missing) = &plan.steps[0] else { panic!("no overview") };
        assert!(overview.contains("c.relname ~ E'^(users)$'") && overview.contains("pg_table_is_visible"), "{overview}");
        assert_eq!(missing, "Did not find any relation named \"users\".");
        assert!(plan.statements().all(|s| s.contains("c.relname ~ E'^(users)$'")), "every section selects the same relations");
        let verbose = planned("\\d+ users");
        assert!(verbose.statements().any(|s| s.contains("pg_get_viewdef")) && verbose.statements().any(|s| s.contains("\"Storage\"")));
        assert!(!plan.statements().any(|s| s.contains("pg_get_viewdef")), "a view's query is shown by \\d+ only, like psql");
        let qualified = planned("\\d public.\"Users\"");
        assert!(qualified.statements().next().is_some_and(|s| s.contains("n.nspname ~ E'^(public)$'") && s.contains("c.relname ~ E'^(Users)$'")));
        assert!(!qualified.statements().next().is_some_and(|s| s.contains("pg_table_is_visible")));
    }

    #[test]
    fn source_commands_take_the_rest_of_the_line() {
        assert!(sql("\\sf add_one").contains("pg_get_functiondef(p.oid)") && sql("\\sf add_one").contains("p.proname ~ E'^(add_one)$'"));
        let exact = sql("\\sf add_one(integer, text)");
        assert!(exact.contains("E'add_one(integer, text)'::pg_catalog.regprocedure"), "{exact}");
        assert!(sql("\\sv big_orders").contains("pg_get_viewdef(c.oid, true)") && sql("\\sv big_orders").contains("c.relkind = 'v'"));
        assert!(refusal("\\sf").contains("function name") && refusal("\\sv").contains("view name"));
        let hostile = sql("\\sf f(x')'); drop table t; --");
        assert!(hostile.contains("E'f(x'')''); drop table t; --'::pg_catalog.regprocedure"), "{hostile}");
    }

    #[test]
    fn connection_and_encoding() {
        let bare = planned("\\conninfo");
        assert!(bare.statements().next().is_some_and(|s| s.contains("current_database()") && s.contains("pg_backend_pid()")));
        let encoding = sql("\\encoding");
        assert!(encoding.contains("client_encoding") && encoding.contains("server_encoding"));
        assert!(refusal("\\encoding latin1").contains("does not change"));
    }

    #[test]
    fn help_and_unknown_commands() {
        for text in ["\\?", "help", "HELP;"] {
            assert!(matches!(planned(text).steps.as_slice(), [Step::Table(t)] if t.rows.len() == COMMANDS.len()), "{text}");
        }
        for text in ["\\copy t from stdin", "\\x", "\\timing", "\\c other", "\\i file.sql", "\\!ls"] {
            assert!(refusal(text).contains("does not know"), "{text}");
        }
        assert!(plan_for(&Ctx::bare(Engine::Postgres), "SELECT 1").is_none());
        assert!(plan_for(&Ctx::bare(Engine::Postgres), "helper").is_none());
    }

    #[test]
    fn every_statement_is_a_read_for_every_command_and_modifier() {
        for command in COMMANDS {
            for text in [command.name.to_string(), format!("{}+", command.name), format!("{}S", command.name), format!("{} some_name", command.name), format!("{}+ a*.b?", command.name)] {
                let Some(Ok(plan)) = plan_for(&Ctx::bare(Engine::Postgres), &text) else { continue };
                for statement in plan.statements() {
                    assert!(classify(statement).iter().all(|s| s.kind == StatementKind::Read), "{text}: {statement}");
                    assert_eq!(classify(statement).len(), 1, "{text}: one statement each");
                }
            }
        }
    }

    #[test]
    fn scripts_mix_commands_and_sql_in_order() {
        let found = crate::integrations::shell_meta::segments(Engine::Supabase, "\\dt\nselect 1;\n\\d users");
        assert_eq!(found, vec![Segment::Meta("\\dt".into()), Segment::Native("select 1".into()), Segment::Meta("\\d users".into())]);
        assert!(plan_for(&Ctx::bare(Engine::Cockroachdb), "\\dt").is_some(), "every engine of the family reads the same commands");
    }
}

// ----------------------------------------------------------------------------
// Live checks against a real PostgreSQL
//
// WHAT:  Runs every statement the commands plan against a server and checks the
//        answers against a fixture schema.
// WHY:   The catalog queries are text; only a server proves each one parses and
//        returns what psql would.
// HOW:   Skipped unless DB_FREE_PG_HOST is set (with DB_FREE_PG_PORT / USER /
//        PASSWORD / DB). Point it at a scratch database: the test creates the
//        schema `shellcmd_probe` (tables, a view, a materialized view, a sequence,
//        functions, a trigger, an enum, a domain), reads it with every command,
//        and drops the schema again. Nothing outside that schema is touched.
// ----------------------------------------------------------------------------
#[cfg(test)]
mod live {
    use super::*;
    use crate::integrations::shell_meta::plan as plan_for;
    use crate::integrations::Integration;
    use crate::model::{ConnectionInput, ConnectionSummary, Engine, ResolvedConnection, StatementResult, Value};
    use std::sync::Arc;

    async fn server() -> Option<Arc<dyn Integration>> {
        let host = std::env::var("DB_FREE_PG_HOST").ok()?;
        let input: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "live", "engine": "postgres", "environment": "local", "readOnly": false,
            "host": host, "port": std::env::var("DB_FREE_PG_PORT").ok().and_then(|p| p.parse::<u16>().ok()),
            "database": std::env::var("DB_FREE_PG_DB").ok(), "username": std::env::var("DB_FREE_PG_USER").ok(),
            "password": null, "filePath": null, "sslMode": "prefer",
            "ssh": serde_json::to_value(crate::model::SshTunnel::default()).ok()?,
            "sshSecret": null, "folder": null, "color": null, "favorite": false, "extra": {}
        }))
        .ok()?;
        let resolved = ResolvedConnection { summary: ConnectionSummary::draft(&input, true), secret: std::env::var("DB_FREE_PG_PASSWORD").ok() };
        crate::integrations::postgres::connect(&resolved).await.ok()
    }

    /// The schema the live checks read: a little of everything a `\d` has a section for.
    const FIXTURE: &str = "DROP SCHEMA IF EXISTS shellcmd_probe CASCADE; CREATE SCHEMA shellcmd_probe;
        CREATE TYPE shellcmd_probe.mood AS ENUM ('sad','ok','happy');
        CREATE DOMAIN shellcmd_probe.posint AS integer CHECK (VALUE > 0);
        CREATE TABLE shellcmd_probe.users (id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY, email text NOT NULL UNIQUE, mood shellcmd_probe.mood DEFAULT 'ok', score shellcmd_probe.posint, created timestamptz DEFAULT now(), full_name text GENERATED ALWAYS AS (email || '!') STORED, CHECK (char_length(email) > 3));
        CREATE TABLE shellcmd_probe.orders (id bigserial PRIMARY KEY, user_id integer NOT NULL REFERENCES shellcmd_probe.users(id) ON DELETE CASCADE, total numeric(10,2) DEFAULT 0);
        CREATE INDEX orders_user_idx ON shellcmd_probe.orders (user_id, total DESC) WHERE total > 0;
        CREATE VIEW shellcmd_probe.big_orders AS SELECT * FROM shellcmd_probe.orders WHERE total > 100;
        CREATE MATERIALIZED VIEW shellcmd_probe.order_totals AS SELECT user_id, sum(total) AS total FROM shellcmd_probe.orders GROUP BY user_id;
        CREATE SEQUENCE shellcmd_probe.ticket_seq START 100 INCREMENT 5;
        CREATE FUNCTION shellcmd_probe.add_one(a integer) RETURNS integer LANGUAGE sql IMMUTABLE AS 'select a + 1';
        CREATE PROCEDURE shellcmd_probe.do_nothing() LANGUAGE sql AS 'select 1';
        CREATE FUNCTION shellcmd_probe.touch() RETURNS trigger LANGUAGE plpgsql AS 'begin return new; end';
        CREATE TRIGGER users_touch BEFORE UPDATE ON shellcmd_probe.users FOR EACH ROW EXECUTE FUNCTION shellcmd_probe.touch();
        COMMENT ON TABLE shellcmd_probe.users IS 'People who log in';";

    /// Every cell as text, per result set.
    fn tables(results: &[StatementResult]) -> Vec<Vec<Vec<String>>> {
        results
            .iter()
            .filter_map(|r| match r {
                StatementResult::Rows { result } => Some(
                    result
                        .rows
                        .iter()
                        .map(|row| {
                            row.iter()
                                .map(|v| match v {
                                    Value::Text(t) | Value::Decimal(t) | Value::DateTime(t) => t.clone(),
                                    Value::Int(i) => i.to_string(),
                                    Value::Bool(b) => b.to_string(),
                                    Value::Null => String::new(),
                                    other => format!("{other:?}"),
                                })
                                .collect()
                        })
                        .collect(),
                ),
                StatementResult::Affected { .. } => None,
            })
            .collect()
    }

    async fn run(db: &Arc<dyn Integration>, text: &str) -> Result<Vec<Vec<Vec<String>>>, String> {
        let Some(Ok(plan)) = plan_for(&Ctx::bare(Engine::Postgres), text) else { return Err(format!("{text} is not planned")) };
        let mut out = Vec::new();
        for step in &plan.steps {
            match step {
                Step::Table(t) => out.extend(tables(&[t.result()])),
                Step::Native(sql) => out.extend(tables(&db.execute(sql, 500).await.map_err(|e| format!("{text}: {e}\n{sql}"))?)),
                Step::Section(sql) => out.extend(tables(&db.execute(sql, 500).await.map_err(|e| format!("{text}: {e}\n{sql}"))?).into_iter().filter(|t| !t.is_empty())),
                Step::Exists(sql, missing) => {
                    let found = tables(&db.execute(sql, 500).await.map_err(|e| format!("{text}: {e}\n{sql}"))?);
                    if found.iter().all(|t| t.is_empty()) {
                        return Err(missing.clone());
                    }
                    out.extend(found);
                }
            }
        }
        Ok(out)
    }

    fn column(table: &[Vec<String>], at: usize) -> Vec<&str> {
        table.iter().filter_map(|row| row.get(at).map(String::as_str)).collect()
    }

    // One test, so the fixture is created once and dropped once however the runner schedules.
    #[tokio::test]
    async fn psql_commands_on_a_real_server() {
        let Some(db) = server().await else { return };
        db.execute(FIXTURE, 10).await.unwrap_or_else(|e| panic!("fixture: {e}"));
        every_command_runs(&db).await;
        the_fixture_is_described_like_psql_would(&db).await;
        db.execute("DROP SCHEMA shellcmd_probe CASCADE", 10).await.unwrap_or_else(|e| panic!("cleanup: {e}"));
    }

    async fn every_command_runs(db: &Arc<dyn Integration>) {
        for text in [
            "\\l", "\\l+", "\\l post*", "\\dn", "\\dn+", "\\dnS", "\\dn shell*", "\\d", "\\dS", "\\d+", "\\dt", "\\dt+", "\\dtS", "\\dt shellcmd_probe.*", "\\dtvmsE shellcmd_probe.*",
            "\\dv shellcmd_probe.*", "\\dm+ shellcmd_probe.*", "\\di+ shellcmd_probe.*", "\\ds shellcmd_probe.*", "\\dE", "\\df", "\\df+ shellcmd_probe.*", "\\dfp shellcmd_probe.*",
            "\\dfa", "\\dfw", "\\dft shellcmd_probe.*", "\\dfn shellcmd_probe.*", "\\du", "\\du+", "\\duS", "\\dg", "\\dx", "\\dx+", "\\dT", "\\dT+ shellcmd_probe.*", "\\dD shellcmd_probe.*",
            "\\dD+", "\\db", "\\db+", "\\dL", "\\dL+", "\\dp shellcmd_probe.*", "\\z shellcmd_probe.*", "\\dy", "\\dy+", "\\dRp", "\\dRp+", "\\dRs", "\\dRs+", "\\des", "\\des+", "\\dew",
            "\\dew+", "\\det", "\\det+", "\\dO", "\\dO+", "\\dA", "\\dA+", "\\dconfig", "\\dconfig+ work*", "\\sf shellcmd_probe.add_one", "\\sf shellcmd_probe.add_one(integer)",
            "\\sv shellcmd_probe.big_orders", "\\conninfo", "\\encoding", "\\d shellcmd_probe.users", "\\d+ shellcmd_probe.users", "\\d shellcmd_probe.orders", "\\d shellcmd_probe.big_orders",
            "\\d+ shellcmd_probe.big_orders", "\\d shellcmd_probe.order_totals", "\\d shellcmd_probe.ticket_seq", "\\d shellcmd_probe.orders_user_idx", "\\d shellcmd_probe.*", "\\?",
        ] {
            run(db, text).await.unwrap_or_else(|e| panic!("{e}"));
        }
    }

    async fn the_fixture_is_described_like_psql_would(db: &Arc<dyn Integration>) {
        let tables_listed = run(db, "\\dt shellcmd_probe.*").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(column(&tables_listed[0], 1), ["orders", "users"]);
        let relations = run(db, "\\dtvmsE shellcmd_probe.*").await.unwrap_or_else(|e| panic!("{e}"));
        let mut names = column(&relations[0], 1);
        names.sort_unstable();
        assert_eq!(names, ["big_orders", "order_totals", "orders", "orders_id_seq", "ticket_seq", "users", "users_id_seq"]);
        let views = run(db, "\\dv shellcmd_probe.*").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(column(&views[0], 1), ["big_orders"]);
        let functions = run(db, "\\df shellcmd_probe.*").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(column(&functions[0], 1).contains(&"add_one") && column(&functions[0], 4).contains(&"proc"), "{functions:?}");
        let procedures = run(db, "\\dfp shellcmd_probe.*").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(column(&procedures[0], 1), ["do_nothing"]);

        let users = run(db, "\\d shellcmd_probe.users").await.unwrap_or_else(|e| panic!("{e}"));
        // overview, columns, indexes, constraints (the check), triggers; "referenced by" has orders.
        assert_eq!(users[0][0][1], "users");
        assert_eq!(column(&users[1], 1), ["id", "email", "mood", "score", "created", "full_name"]);
        assert_eq!(users[1][0][5], "generated always as identity");
        assert!(users[1][5][5].starts_with("generated always as ("), "{:?}", users[1][5]);
        assert_eq!(column(&users[2], 1), ["users_pkey", "users_email_key"]);
        assert!(users.iter().any(|t| t.iter().any(|r| r.get(2).is_some_and(|c| c == "orders_user_id_fkey"))), "referenced by orders: {users:?}");
        assert!(users.iter().any(|t| t.iter().any(|r| r.get(1).is_some_and(|c| c == "users_touch"))), "the trigger is listed");

        let orders = run(db, "\\d shellcmd_probe.orders").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(orders.iter().any(|t| t.iter().any(|r| r.get(3).is_some_and(|d| d.starts_with("FOREIGN KEY (user_id) REFERENCES shellcmd_probe.users(id)")))));
        let missing = run(db, "\\d shellcmd_probe.nope").await;
        assert_eq!(missing.err().as_deref(), Some("Did not find any relation named \"shellcmd_probe.nope\"."));

        let seq = run(db, "\\d shellcmd_probe.ticket_seq").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(seq.iter().any(|t| t.first().is_some_and(|r| r.get(2) == Some(&"100".to_string()) && r.get(5) == Some(&"5".to_string()))), "start 100 increment 5: {seq:?}");
        let source = run(db, "\\sf shellcmd_probe.add_one(integer)").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(source[0][0][0].contains("CREATE OR REPLACE FUNCTION shellcmd_probe.add_one"), "{source:?}");
        let view_source = run(db, "\\sv shellcmd_probe.big_orders").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(view_source[0][0][0].starts_with("CREATE OR REPLACE VIEW shellcmd_probe.big_orders AS"), "{view_source:?}");

        let types = run(db, "\\dT shellcmd_probe.*").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(types[0].iter().any(|r| r[1].ends_with("mood") && r[3] == "sad\nok\nhappy"), "{types:?}");
        let domains = run(db, "\\dD shellcmd_probe.*").await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(column(&domains[0], 1), ["posint"]);
        let databases = run(db, "\\l").await.unwrap_or_else(|e| panic!("{e}"));
        assert!(column(&databases[0], 0).contains(&"postgres"));
    }
}
