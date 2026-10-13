// SOT: shell-meta-tigergraph, gsql-shell-structure-commands, tigergraph-rest-schema-reads

use super::args::{split_word, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, Step, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// TIGERGRAPH
//
// WHAT:  The GSQL shell's catalog commands, answered from the servers the adapter
//        talks to: `LS`, `SHOW GRAPH <name>`, `SHOW VERTEX *`, `SHOW EDGE *`,
//        `SHOW QUERY *`, `SHOW USER`, `SHOW ROLE`, and `HELP`.
// WHY:   In the `gsql` shell these are how a schema is read. The adapter's tab
//        runs installed queries, interpreted GSQL queries and a
//        `{"method","path","body"}` request (`tigergraph::parse_command`); a
//        `SHOW` is none of those, and the GSQL server's text endpoint that does
//        run shell commands (`/gsqlserver/gsql/file`) is a POST that the
//        adapter's read-only check treats as a write, so it is not used. The
//        schema, the installed queries, the users and the roles each have a GET
//        endpoint that the object explorer already reads, so the commands map to
//        those.
// HOW:   Each command becomes GET request documents for the adapter's
//        passthrough (`/gsqlserver/...` goes to the GSQL port, `/restpp/...` to
//        REST++). The graph is the one the editor's picker is on, else the
//        connection's; a graph name typed after `SHOW GRAPH` is checked against
//        GSQL's identifier rule and percent-encoded. The schema endpoint returns
//        vertex and edge types together, so `SHOW VERTEX *` and `SHOW EDGE *`
//        both show that one document; `SHOW VERTEX <type>` is refused rather
//        than answered with the whole schema. `LS` is the schema plus the graph's
//        installed queries (an empty list of queries is left out).
// WHERE: src-tauri/src/integrations/tigergraph.rs (`parse_command`, `execute`,
//        `schema`, `endpoints`, `users`, `roles`), src-tauri/src/integrations/shell_meta/mod.rs
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::shell("ls", "ls", "The graph's types and installed queries"),
    ShellCommand::shell("show graph", "SHOW GRAPH graph_name", "The schema of a graph: vertex and edge types with their attributes"),
    ShellCommand::shell("show vertex", "SHOW VERTEX *", "The graph's schema (vertex and edge types are one document over REST)"),
    ShellCommand::shell("show edge", "SHOW EDGE *", "The graph's schema (vertex and edge types are one document over REST)"),
    ShellCommand::shell("show query", "SHOW QUERY *", "The graph's installed queries"),
    ShellCommand::shell("show user", "SHOW USER", "Database users (needs the GSQL server and an account that may list them)"),
    ShellCommand::shell("show role", "SHOW ROLE", "Roles of the graph (needs the GSQL server)"),
    ShellCommand::native("RUN QUERY", "RUN QUERY name(param: value)", "Run an installed query"),
    ShellCommand::native("INTERPRET QUERY", "INTERPRET QUERY () FOR GRAPH graph_name { ... }", "Run a GSQL query without installing it"),
    ShellCommand::native(r#"{"method":"GET","path":"/restpp/endpoints"}"#, r#"{"method":"GET","path":"/restpp/endpoints"}"#, "Every REST++ endpoint of the server"),
];

fn plan(ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let text = strip_terminator(text);
    let words: Vec<&str> = text.split_whitespace().collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
    let lower: Vec<&str> = lower.iter().map(String::as_str).collect();
    match lower.as_slice() {
        ["help"] => Some(Ok(help_plan(&DIALECT))),
        ["ls"] => Some(graph_of(ctx, None).map(|g| {
            Plan::native(get(&schema_path(&g))).then(Step::Section(get(&format!("/restpp/endpoints/{}?dynamic=true", pct(&g)))))
        })),
        ["show", "graph", ..] => {
            let (_, tail) = split_word(text);
            let (_, name) = split_word(tail);
            Some(show_graph(ctx, name))
        }
        ["show", "vertex", target] | ["show", "edge", target] => Some(match *target {
            "*" => graph_of(ctx, None).map(|g| Plan::native(get(&schema_path(&g)))),
            _ => Err(everything_only(words[1])),
        }),
        ["show", "query", target] => Some(match *target {
            "*" => graph_of(ctx, None).map(|g| Plan::native(get(&format!("/restpp/endpoints/{}?dynamic=true", pct(&g))))),
            _ => Err(everything_only("query")),
        }),
        ["show", "vertex" | "edge" | "query"] => Some(Err(everything_only(words[1]))),
        ["show", "user"] => Some(Ok(Plan::native(get("/gsqlserver/gsql/users")))),
        ["show", "role"] => Some(graph_of(ctx, None).map(|g| Plan::native(get(&format!("/gsqlserver/gsql/roles?graph={}", pct(&g)))))),
        _ => None,
    }
}

fn show_graph(ctx: &Ctx, name: &str) -> AppResult<Plan> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::invalid_input("Usage: SHOW GRAPH graph_name"));
    }
    graph_of(ctx, Some(name)).map(|g| Plan::native(get(&schema_path(&g))))
}

fn everything_only(what: &str) -> AppError {
    AppError::invalid_input(format!("Over REST the schema comes as one document for the whole graph, so only `SHOW {} *` is available.", what.to_ascii_uppercase()))
}

// WHAT:  The graph a command is about: the one named, else the editor picker's, else the connection's.
// WHY:   GSQL graph names are identifiers; anything else cannot be a graph and is not put in a path.
fn graph_of(ctx: &Ctx, named: Option<&str>) -> AppResult<String> {
    let chosen = named
        .or(ctx.namespace)
        .or_else(|| ctx.connection.and_then(|c| c.database.as_deref()))
        .map(str::trim)
        .filter(|g| !g.is_empty());
    match chosen {
        None if ctx.connection.is_none() => Ok(String::new()),
        None => Err(AppError::invalid_input("Name the graph: this connection has none selected. Use `SHOW GRAPH graph_name`.")),
        Some(graph) if is_identifier(graph) => Ok(graph.to_string()),
        Some(graph) => Err(AppError::invalid_input(format!("{graph:?} is not a graph name: GSQL names start with a letter or underscore and use letters, digits and underscores."))),
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn schema_path(graph: &str) -> String {
    format!("/gsqlserver/gsql/schema?graph={}", pct(graph))
}

/// One GET request document for the adapter's passthrough.
fn get(path: &str) -> String {
    serde_json::json!({ "method": "GET", "path": path }).to_string()
}

// WHAT:  A path segment the way the adapter encodes one (`tigergraph::pct`).
fn pct(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::CommandOrigin;
    use crate::model::{ConnectionInput, ConnectionSummary, Engine};
    use serde_json::{json, Value};

    fn connection(graph: Option<&str>) -> ConnectionSummary {
        let input: Option<ConnectionInput> = serde_json::from_value(json!({
            "name": "tg", "engine": "tigergraph", "environment": "local", "readOnly": false,
            "host": "localhost", "port": 9000, "database": graph, "username": "tigergraph", "password": null,
            "filePath": null, "sslMode": "disable", "ssh": serde_json::to_value(crate::model::SshTunnel::default()).ok(),
            "sshSecret": null, "folder": null, "color": null, "favorite": false, "extra": {}
        }))
        .ok();
        match input {
            Some(input) => ConnectionSummary::draft(&input, false),
            None => panic!("the test connection did not build"),
        }
    }

    fn run(text: &str, namespace: Option<&str>, summary: Option<&ConnectionSummary>) -> Option<AppResult<Plan>> {
        plan(&Ctx::new(Engine::Tigergraph, summary, namespace), text)
    }

    fn requests(text: &str, namespace: Option<&str>) -> Vec<Value> {
        match run(text, namespace, None) {
            Some(Ok(plan)) => plan.statements().map(|s| serde_json::from_str(s).unwrap_or(Value::Null)).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn schema_commands_read_the_graph_schema() {
        let schema = json!({"method": "GET", "path": "/gsqlserver/gsql/schema?graph=social"});
        assert_eq!(requests("SHOW GRAPH social", None), vec![schema.clone()]);
        assert_eq!(requests("show vertex *", Some("social")), vec![schema.clone()]);
        assert_eq!(requests("SHOW EDGE *;", Some("social")), vec![schema.clone()]);
        assert_eq!(requests("show graph other", Some("social")), vec![json!({"method": "GET", "path": "/gsqlserver/gsql/schema?graph=other"})]);
    }

    #[test]
    fn ls_is_the_schema_and_the_installed_queries_with_the_latter_optional() {
        let Some(Ok(plan)) = run("LS", Some("social"), None) else { panic!("ls") };
        assert_eq!(plan.steps.len(), 2);
        assert!(matches!(&plan.steps[0], Step::Native(s) if s.contains("/gsqlserver/gsql/schema?graph=social")));
        assert!(matches!(&plan.steps[1], Step::Section(s) if s.contains("/restpp/endpoints/social?dynamic=true")));
    }

    #[test]
    fn queries_users_and_roles_use_their_own_endpoints() {
        assert_eq!(requests("SHOW QUERY *", Some("g1")), vec![json!({"method": "GET", "path": "/restpp/endpoints/g1?dynamic=true"})]);
        assert_eq!(requests("show user", None), vec![json!({"method": "GET", "path": "/gsqlserver/gsql/users"})]);
        assert_eq!(requests("show role", Some("g1")), vec![json!({"method": "GET", "path": "/gsqlserver/gsql/roles?graph=g1"})]);
    }

    #[test]
    fn the_graph_defaults_to_the_connections_when_the_picker_is_not_set() {
        let summary = connection(Some("shop"));
        let found = run("ls", None, Some(&summary));
        let Some(Ok(plan)) = found else { panic!("{found:?}") };
        assert!(plan.statements().all(|s| s.contains("shop")), "{plan:?}");
        let bare = connection(None);
        assert!(matches!(run("ls", None, Some(&bare)), Some(Err(AppError::InvalidInput { .. }))), "no graph at all is an error, not an empty path");
        let named = connection(Some("shop"));
        let picked = run("show query *", Some("other"), Some(&named));
        assert!(matches!(&picked, Some(Ok(plan)) if plan.statements().all(|s| s.contains("other"))), "{picked:?}");
    }

    #[test]
    fn a_graph_name_that_is_not_an_identifier_is_refused() {
        for text in ["SHOW GRAPH a/b", "SHOW GRAPH ../x", "show graph a?b=1", "SHOW GRAPH \"x\"", "show graph a b", "show graph 1abc", "show graph"] {
            let found = run(text, None, None);
            assert!(matches!(found, Some(Err(AppError::InvalidInput { .. }))), "{text}: {found:?}");
        }
        let hostile = run("ls", Some("g/../../users"), None);
        assert!(matches!(hostile, Some(Err(AppError::InvalidInput { .. }))), "{hostile:?}");
    }

    #[test]
    fn a_single_type_is_refused_rather_than_answered_with_everything() {
        for text in ["SHOW VERTEX Person", "show edge KNOWS", "SHOW QUERY top_users", "show vertex", "show edge"] {
            assert!(matches!(run(text, Some("g"), None), Some(Err(AppError::InvalidInput { .. }))), "{text}");
        }
    }

    #[test]
    fn help_lists_every_command() {
        for text in ["help", "HELP", "help;"] {
            let Some(Ok(plan)) = run(text, None, None) else { panic!("{text}") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn the_adapters_own_commands_are_left_alone() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(run(command.name, None, None).is_none(), "{}", command.name);
        }
        for text in ["RUN QUERY top(k: 3)", "INTERPRET QUERY () FOR GRAPH g { PRINT 1; }", "SELECT s FROM Person:s", "show", "show tables", "lsx"] {
            assert!(run(text, Some("g"), None).is_none(), "{text}");
        }
    }

    #[test]
    fn every_request_is_a_get_without_a_body() {
        for text in ["ls", "show graph g", "show vertex *", "show edge *", "show query *", "show user", "show role"] {
            for doc in requests(text, Some("g")) {
                assert_eq!(doc["method"], "GET", "{text}");
                assert!(doc.get("body").is_none(), "{text}");
                let path = doc["path"].as_str().unwrap_or_default();
                assert!(path.starts_with("/gsqlserver/") || path.starts_with("/restpp/"), "{text}: {path}");
            }
        }
    }
}
