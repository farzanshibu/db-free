// SOT: shell-meta-hbase, hbase-shell-structure-commands, hbase-rest-passthrough-reads

use super::args::{split_word, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// HBASE
//
// WHAT:  The hbase shell's structure commands, each answered from the REST
//        gateway (Stargate) the adapter talks to: `list_namespace`,
//        `list_namespace_tables 'ns'`, `describe_namespace 'ns'`,
//        `describe 'table'`, `status`, `version`, and `help`.
// WHY:   In `hbase shell` these are the way to see what a cluster holds. The
//        adapter's tab already runs `list`, `get` and `scan` in the shell's own
//        syntax, plus a JSON `{"method","path","body"}` request that reaches any
//        REST endpoint (`hbase::parse_command`). The structure commands are
//        exactly the GET endpoints behind that passthrough, so they are
//        translated to it rather than re-implemented.
// HOW:   Each command becomes one GET request document. Names are checked against
//        what HBase allows in a table or namespace name (letters, digits, `_`; a
//        table also `- .` after its first character, and one `ns:` prefix) and then
//        percent-encoded into the path, so a quote, a slash or a `..` in an
//        argument can only fail the check. Arguments are
//        read the way the shell's Ruby does: `'t'`, `"t"` or a bare word.
//        `exists`, `is_enabled` and `whoami` have no REST endpoint and are not
//        claimed; `list 'regex'` is refused rather than silently listing every
//        table, because the gateway cannot filter by regular expression.
//        `status` is the cluster summary (`/status/cluster`); the shell's
//        `'simple'`/`'detailed'` forms are not available over REST.
// WHERE: src-tauri/src/integrations/hbase.rs (`parse_command`, the
//        `Command::Passthrough` branch of `execute`; the same paths back its
//        object explorer), src-tauri/src/integrations/shell_meta/mod.rs
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::shell("list_namespace", "list_namespace", "List the namespaces"),
    ShellCommand::shell("describe_namespace", "describe_namespace 'namespace'", "Properties of a namespace"),
    ShellCommand::shell("list_namespace_tables", "list_namespace_tables 'namespace'", "The tables of a namespace"),
    ShellCommand::shell("describe", "describe 'table'", "A table's schema: column families and their settings"),
    ShellCommand::shell("status", "status", "Cluster summary: servers, regions, load"),
    ShellCommand::shell("version", "version", "The HBase version"),
    ShellCommand::native("list", "list", "Every table (the regular-expression filter is not available over REST)"),
    ShellCommand::native("scan", "scan 'table'", "Rows of a table"),
    ShellCommand::native("get", "get 'table', 'rowkey'", "One row"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let text = strip_terminator(text);
    let (head, rest) = split_word(text);
    let rest_missing = rest.is_empty();
    match head.to_ascii_lowercase().as_str() {
        "help" if rest_missing => Some(Ok(help_plan(&DIALECT))),
        "list_namespace" => Some(no_arguments("list_namespace", rest).map(|()| get("/namespaces"))),
        "status" => Some(no_arguments("status", rest).map(|()| get("/status/cluster"))),
        "version" => Some(no_arguments("version", rest).map(|()| get("/version/cluster"))),
        "describe_namespace" => Some(namespace("describe_namespace", rest).map(|ns| get(&format!("/namespaces/{}", pct(&ns))))),
        "list_namespace_tables" => Some(namespace("list_namespace_tables", rest).map(|ns| get(&format!("/namespaces/{}/tables", pct(&ns))))),
        "describe" => Some(table("describe", rest).map(|t| get(&format!("/{}/schema", pct(&t))))),
        // The adapter lists every table whatever follows `list`; say so instead of pretending to filter.
        "list" if !rest_missing => Some(Err(AppError::invalid_input(
            "The REST gateway cannot filter tables by regular expression, so `list` takes no argument here. Run `list` for every table, or `list_namespace_tables 'ns'` for one namespace.",
        ))),
        _ => None,
    }
}

/// One GET request document for the adapter's passthrough.
fn get(path: &str) -> Plan {
    Plan::native(serde_json::json!({ "method": "GET", "path": path }).to_string())
}

fn no_arguments(command: &str, rest: &str) -> AppResult<()> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(AppError::invalid_input(format!("`{command}` takes no argument over the REST gateway.")))
    }
}

fn namespace(command: &str, rest: &str) -> AppResult<String> {
    let name = one_name(command, rest, "namespace")?;
    if name.contains(':') {
        return Err(AppError::invalid_input(format!("`{command}` takes a namespace, not a table: {name:?}.")));
    }
    // HBase's own rule for a namespace (`TableName.isLegalNamespaceName`): letters, digits and `_`.
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AppError::invalid_input(format!("{name:?} is not a valid namespace name: HBase allows letters, digits and `_`.")));
    }
    Ok(name)
}

fn table(command: &str, rest: &str) -> AppResult<String> {
    one_name(command, rest, "table")
}

// WHAT:  The single name argument of a command, as the shell's Ruby would read it:
//        `'t'`, `"t"` or a bare word, with an optional trailing comma.
fn one_name(command: &str, rest: &str, what: &str) -> AppResult<String> {
    let usage = || AppError::invalid_input(format!("Usage: {command} '{what}'"));
    let word = rest.trim().trim_end_matches(',').trim();
    let unquoted = match (word.chars().next(), word.chars().last()) {
        (Some(open @ ('\'' | '"')), Some(close)) if open == close && word.len() >= 2 => &word[1..word.len() - 1],
        _ => word,
    };
    if unquoted.is_empty() || unquoted.contains(char::is_whitespace) {
        return Err(usage());
    }
    if !unquoted.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')) {
        return Err(AppError::invalid_input(format!("{unquoted:?} is not a valid {what} name: HBase allows {}.", if what == "table" { "letters, digits, `_`, `-` and `.` (and `namespace:table`)" } else { "letters, digits and `_`" })));
    }
    // Each part (`ns`, `table`) starts with a letter, digit or `_`, as HBase requires. It also keeps
    // `.` and `..` out: the URL parser would resolve them away from the endpoint they sit in.
    let parts: Vec<&str> = unquoted.split(':').collect();
    let starts_well = |part: &str| part.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    if parts.len() > 2 || !parts.iter().copied().all(starts_well) {
        return Err(AppError::invalid_input(format!("{unquoted:?} is not a valid {what} name: each part starts with a letter, digit or `_`, and a table has at most one `namespace:` prefix.")));
    }
    Ok(unquoted.to_string())
}

// WHAT:  A path segment the way the adapter encodes one (`hbase::pct`): everything but
//        the unreserved characters and the `ns:table` colon is escaped.
fn pct(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b':') {
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
    use crate::integrations::shell_meta::{CommandOrigin, Step};
    use crate::model::Engine;
    use serde_json::{json, Value};

    fn run(text: &str) -> Option<AppResult<Plan>> {
        plan(&Ctx::bare(Engine::Hbase), text)
    }

    fn request(text: &str) -> Value {
        match run(text) {
            Some(Ok(plan)) => match plan.steps.as_slice() {
                [Step::Native(statement)] => serde_json::from_str(statement).unwrap_or(Value::Null),
                other => panic!("{text}: {other:?}"),
            },
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn help_lists_every_command() {
        let Some(Ok(plan)) = run("help") else { panic!("help") };
        assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()));
        assert!(run("help me").is_none());
    }

    #[test]
    fn structure_commands_become_get_requests() {
        assert_eq!(request("list_namespace"), json!({"method": "GET", "path": "/namespaces"}));
        assert_eq!(request("LIST_NAMESPACE;"), json!({"method": "GET", "path": "/namespaces"}));
        assert_eq!(request("status"), json!({"method": "GET", "path": "/status/cluster"}));
        assert_eq!(request("version"), json!({"method": "GET", "path": "/version/cluster"}));
        assert_eq!(request("describe_namespace 'sales'"), json!({"method": "GET", "path": "/namespaces/sales"}));
        assert_eq!(request("list_namespace_tables \"sales\""), json!({"method": "GET", "path": "/namespaces/sales/tables"}));
        assert_eq!(request("describe 'users'"), json!({"method": "GET", "path": "/users/schema"}));
        assert_eq!(request("DESCRIBE users"), json!({"method": "GET", "path": "/users/schema"}));
        assert_eq!(request("describe 'sales:orders'"), json!({"method": "GET", "path": "/sales:orders/schema"}));
        assert_eq!(request("describe 'my-table.v2',"), json!({"method": "GET", "path": "/my-table.v2/schema"}));
    }

    #[test]
    fn a_name_the_cluster_would_not_accept_is_refused_not_encoded_into_a_path() {
        for text in [
            "describe 'a/b'",
            "describe '../status/cluster'",
            "describe 'a b'",
            "describe 'a\"b'",
            "describe 'x?y=1'",
            "describe 't1', 't2'",
            "describe",
            "describe ''",
            "describe_namespace 'a:b'",
            "list_namespace_tables",
            "describe 'é'",
            "describe 'a%2Fb'",
            "describe 'a\nb'",
        ] {
            let found = run(text);
            assert!(matches!(found, Some(Err(AppError::InvalidInput { .. }))), "{text:?}: {found:?}");
        }
        assert_eq!(pct("a/b c"), "a%2Fb%20c");
    }

    // A dot segment would be resolved by the URL parser into another endpoint (`/namespaces/..` is `/`).
    #[test]
    fn dot_segments_and_malformed_qualified_names_never_reach_a_path() {
        for text in [
            "describe '..'",
            "describe '.'",
            "describe '.x'",
            "describe '-x'",
            "describe 'ns:.x'",
            "describe 'ns:'",
            "describe ':t'",
            "describe 'a:b:c'",
            "describe_namespace '..'",
            "describe_namespace '.'",
            "list_namespace_tables '..'",
            "describe_namespace 'a-b'",
            "describe_namespace 'a.b'",
        ] {
            let found = run(text);
            assert!(matches!(found, Some(Err(AppError::InvalidInput { .. }))), "{text:?}: {found:?}");
        }
        assert_eq!(request("describe 'hbase:meta'"), json!({"method": "GET", "path": "/hbase:meta/schema"}));
        assert_eq!(request("describe '_t-1.b'"), json!({"method": "GET", "path": "/_t-1.b/schema"}));
        assert_eq!(request("describe_namespace 'ns_1'"), json!({"method": "GET", "path": "/namespaces/ns_1"}));
        assert_eq!(pct("ns:t"), "ns:t");
    }

    #[test]
    fn commands_without_arguments_refuse_extras() {
        for text in ["status 'detailed'", "version 1", "list_namespace foo"] {
            assert!(matches!(run(text), Some(Err(AppError::InvalidInput { .. }))), "{text}");
        }
    }

    #[test]
    fn list_with_a_pattern_is_refused_and_a_plain_list_is_left_to_the_adapter() {
        assert!(run("list").is_none());
        assert!(run("LIST").is_none());
        let found = run("list 'user.*'");
        assert!(matches!(&found, Some(Err(AppError::InvalidInput { message })) if message.contains("regular expression")), "{found:?}");
    }

    #[test]
    fn native_commands_are_not_intercepted() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(run(command.name).is_none(), "{}", command.name);
        }
        for text in ["get 'users', 'r1'", "scan 'users'", "put 't', 'r', 'f:q', 'v'", "{\"list\": 1}", "describe_table 't'"] {
            assert!(run(text).is_none(), "{text}");
        }
    }

    #[test]
    fn every_request_is_a_get_the_adapter_reads_as_a_passthrough() {
        for text in ["list_namespace", "status", "version", "describe_namespace 'n'", "list_namespace_tables 'n'", "describe 't'"] {
            let doc = request(text);
            assert_eq!(doc["method"], "GET", "{text}");
            assert!(doc["path"].as_str().is_some_and(|p| p.starts_with('/')), "{text}");
            assert!(doc.get("body").is_none(), "{text}");
        }
    }
}
