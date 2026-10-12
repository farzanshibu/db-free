// SOT: shell-meta-existdb, exist-client-shell-commands, exist-xmldb-equivalents

use super::args::{eq_ci, raw_args, split_word, unquote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// EXIST-DB
//
// WHAT:  The browsing command of eXist-db's command-line client in shell mode
//        (`client.sh -s`): `ls`, which lists the current collection.
// WHY:   People who work in that shell list a collection with `ls`. This connection
//        has no session state to `cd` through, but the question has an exact answer:
//        the connection is on one collection, and listing a collection is a pair of
//        `xmldb` calls.
// HOW:   `ls` becomes one XQuery sequence: the child collections (each with a
//        trailing `/`) followed by the child resources, from
//        `xmldb:get-child-collections` and `xmldb:get-child-resources`, which
//        integrations/existdb.rs runs through `?_query=` with the connection's
//        collection as context and returns one item per row. The shell's `ls` takes
//        no argument; the collection argument (absolute, or relative to the
//        connection's collection) is a DB Free addition, standing in for `cd`. With
//        none, the connection's own collection is listed, normalised as the adapter
//        does (`/db` by default). Names go in as XQuery string literals (`"` doubled,
//        `&` as `&amp;`, control characters refused). `cd`, `mkcol`, `rm`, `put` and the
//        rest of the shell change state or data and are not translated; the shell has
//        no `pwd`, so none is offered. A line holding only `ls` is a valid XQuery path
//        step, so the dialect is cut by statements, not by lines: `ls` is recognised at
//        the start of a statement (a script, or the text after a `;`), never in the
//        middle of a multi-line XQuery, and `ls` followed by an XQuery operator
//        (`ls = 3`) is an expression. The help word is `?` (and `\?`) for the same
//        reason `help` is not used.
// WHERE: src-tauri/src/integrations/existdb.rs (`execute`, `normalise_collection`, `is_write_xquery`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["?", "\\?"], cut: Cut::Statements { line_words: &["?", "\\?", "ls"] }, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("?", "? | \\?", "List the commands this console understands"),
    ShellCommand::shell("ls", "ls [<collection>]", "The sub-collections (with a trailing /) and resources of the connected collection (a collection argument is a DB Free addition)"),
    ShellCommand::native("xmldb:get-child-collections(\"/db\")", "xmldb:get-child-collections(\"<collection>\")", "XQuery: the sub-collections of a collection"),
    ShellCommand::native("xmldb:get-child-resources(\"/db\")", "xmldb:get-child-resources(\"<collection>\")", "XQuery: the resources of a collection"),
    ShellCommand::native("xmldb:collection-available(\"/db\")", "xmldb:collection-available(\"<collection>\")", "XQuery: whether a collection exists"),
    ShellCommand::native("system:get-version()", "system:get-version()", "XQuery: the server version"),
    ShellCommand::native("GET", "GET <path>", "One resource, relative to the connected collection or absolute"),
];

fn plan(ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    if DIALECT.help.iter().any(|h| eq_ci(h, text)) {
        return Some(Ok(help_plan(&DIALECT)));
    }
    let (head, rest) = split_word(text);
    // `ls = 3` is an expression over elements named `ls`.
    if eq_ci(head, "ls") && !rest.starts_with(|c: char| "=!<>|,+*()[]{}:@$%^&?#".contains(c)) {
        return Some(ls(ctx, rest));
    }
    None
}

/// The connection's collection, normalised the way `normalise_collection` does.
fn connected_collection(ctx: &Ctx) -> String {
    let raw = ctx.connection.and_then(|c| c.database.as_deref()).unwrap_or("/db");
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        "/db".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn ls(ctx: &Ctx, rest: &str) -> AppResult<Plan> {
    let args: Vec<String> = raw_args(rest).map_err(AppError::invalid_input)?.iter().map(|a| unquote(a)).collect();
    if let Some(flag) = args.iter().find(|a| a.starts_with('-')) {
        return Err(AppError::invalid_input(format!("ls does not take {flag}; it lists names.")));
    }
    let collection = match args.as_slice() {
        [] => connected_collection(ctx),
        [path] => resolve(&connected_collection(ctx), path)?,
        _ => return Err(AppError::invalid_input("ls takes one collection: ls [<collection>].")),
    };
    let path = xquery_string(&collection)?;
    Ok(Plan::native(format!(
        "(for $c in xmldb:get-child-collections({path}) order by $c return concat($c, \"/\"), for $r in xmldb:get-child-resources({path}) order by $r return $r)"
    )))
}

// WHAT:  A collection argument against the connected one: absolute as given, otherwise below it.
fn resolve(current: &str, path: &str) -> AppResult<String> {
    if path.split('/').any(|step| step == "..") {
        return Err(AppError::invalid_input("ls cannot step up with ..; name the collection from its root, e.g. ls /db/apps."));
    }
    let joined = if path.starts_with('/') { path.to_string() } else { format!("{}/{}", current.trim_end_matches('/'), path) };
    let trimmed = joined.trim_end_matches('/');
    Ok(if trimmed.is_empty() { "/".to_string() } else { trimmed.to_string() })
}

// WHAT:  `raw` as an XQuery string literal: `"` doubled, `&` as the predefined entity, no control characters.
fn xquery_string(raw: &str) -> AppResult<String> {
    if raw.chars().any(char::is_control) {
        return Err(AppError::invalid_input("A collection name cannot hold line breaks or control characters."));
    }
    Ok(format!("\"{}\"", raw.replace('&', "&amp;").replace('"', "\"\"")))
}

#[cfg(test)]
mod tests {
    use super::super::{plan, segments, CommandOrigin, Ctx, Segment, Step};
    use crate::model::{ConnectionInput, ConnectionSummary, Engine};

    fn summary(database: Option<&str>) -> ConnectionSummary {
        let input: Option<ConnectionInput> = serde_json::from_value(serde_json::json!({
            "name": "x", "engine": "existdb", "environment": "local", "readOnly": false, "host": "h", "port": null,
            "database": database, "username": null, "password": null, "filePath": null, "sslMode": "prefer",
            "ssh": serde_json::to_value(crate::model::SshTunnel::default()).unwrap_or_default(),
            "sshSecret": null, "folder": null, "color": null, "favorite": false, "extra": {}
        }))
        .ok();
        match input {
            Some(input) => ConnectionSummary::draft(&input, false),
            None => panic!("test connection did not build"),
        }
    }

    fn run_in(database: Option<&str>, text: &str) -> Option<crate::error::AppResult<super::super::Plan>> {
        let summary = summary(database);
        plan(&Ctx::new(Engine::Existdb, Some(&summary), None), text)
    }

    fn statements_in(database: Option<&str>, text: &str) -> Vec<String> {
        match run_in(database, text) {
            Some(Ok(p)) => p.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn listing(collection: &str) -> String {
        format!(
            "(for $c in xmldb:get-child-collections(\"{collection}\") order by $c return concat($c, \"/\"), for $r in xmldb:get-child-resources(\"{collection}\") order by $r return $r)"
        )
    }

    fn refused(text: &str) -> String {
        match run_in(Some("/db/apps"), text) {
            Some(Err(e)) => e.message().to_string(),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn the_help_marks_list_every_command_and_a_bare_help_stays_xquery() {
        for word in ["?", "\\?", "?;"] {
            let found = plan(&Ctx::bare(Engine::Existdb), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}");
        }
        assert!(plan(&Ctx::bare(Engine::Existdb), "help").is_none());
    }

    #[test]
    fn ls_lists_the_connected_collection_by_default() {
        assert_eq!(statements_in(Some("/db/apps"), "ls"), [listing("/db/apps")]);
        assert_eq!(statements_in(Some("db/apps/"), "LS;"), [listing("/db/apps")]);
        assert_eq!(statements_in(None, "ls"), [listing("/db")]);
        assert_eq!(statements_in(Some("   "), "ls"), [listing("/db")]);
    }

    #[test]
    fn ls_resolves_a_collection_argument() {
        assert_eq!(statements_in(Some("/db/apps"), "ls /db/system"), [listing("/db/system")]);
        assert_eq!(statements_in(Some("/db/apps"), "ls demo/data/"), [listing("/db/apps/demo/data")]);
        assert_eq!(statements_in(Some("/db/apps"), "ls \"my coll\""), [listing("/db/apps/my coll")]);
        assert_eq!(statements_in(Some("/db/apps"), "ls /"), [listing("/")]);
    }

    #[test]
    fn the_shell_has_no_pwd_so_it_stays_xquery() {
        for text in ["pwd", "PWD", "pwd /x"] {
            assert!(plan(&Ctx::bare(Engine::Existdb), text).is_none(), "{text}");
        }
        assert!(super::COMMANDS.iter().all(|c| c.name != "pwd"));
    }

    #[test]
    fn ls_followed_by_an_operator_is_an_expression() {
        for text in ["ls = 3", "ls | $x", "ls, 1", "ls[1]", "ls/name"] {
            assert!(plan(&Ctx::bare(Engine::Existdb), text).is_none(), "{text}");
        }
        assert!(plan(&Ctx::bare(Engine::Existdb), "ls /db/apps").is_some(), "an absolute collection is an argument");
    }

    #[test]
    fn ls_is_a_command_only_at_the_start_of_a_statement() {
        let query = "for $x in collection('/db/data')//root\nreturn\n  ls";
        assert_eq!(segments(Engine::Existdb, query), vec![Segment::Native(query.to_string())], "a line of a multi-line XQuery is never cut out");
        assert_eq!(segments(Engine::Existdb, "ls\n//item"), vec![Segment::Meta("ls".to_string()), Segment::Native("//item".to_string())]);
        assert_eq!(segments(Engine::Existdb, "//item;\nls /db/apps"), vec![Segment::Native("//item".to_string()), Segment::Meta("ls /db/apps".to_string())]);
        assert_eq!(segments(Engine::Existdb, "?\nls"), vec![Segment::Meta("?".to_string()), Segment::Meta("ls".to_string())]);
    }

    #[test]
    fn a_hostile_collection_stays_inside_its_string_literal() {
        let found = statements_in(Some("/db"), "ls '/db/x\") , xmldb:remove(\"/db'");
        assert_eq!(found, [listing("/db/x\"\") , xmldb:remove(\"\"/db")]);
        let amp = statements_in(Some("/db"), "ls '/db/a&amp;b'");
        assert_eq!(amp, [listing("/db/a&amp;amp;b")]);
        for statement in statements_in(Some("/db"), "ls") {
            for write in ["xmldb:store", "xmldb:remove", "xmldb:create-collection", "xmldb:move", "xmldb:rename", "xmldb:copy", "update insert", "update delete", "update replace"] {
                assert!(!statement.contains(write), "{statement}");
            }
        }
    }

    #[test]
    fn bad_arguments_say_how_to_fix_them() {
        assert!(refused("ls -l").contains("ls does not take -l"));
        assert!(refused("ls ../other").contains("name the collection"));
        assert!(refused("ls /db/a/../b").contains("name the collection"));
        assert!(refused("ls a b").contains("one collection"));
        assert!(refused("ls 'a").contains("quote"));
    }

    #[test]
    fn xquery_and_the_adapters_own_commands_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Existdb), native.name).is_none(), "{}", native.name);
        }
        for text in ["xmldb:get-child-collections(\"/db\")", "//item[@id = 1]", "GET data/a.xml", "lsx", "cd /db/apps", "mkcol x"] {
            assert!(plan(&Ctx::bare(Engine::Existdb), text).is_none(), "{text}");
        }
    }
}
