// SOT: shell-meta-basex, basex-command-language, basex-xquery-equivalents

use super::args::{eq_ci, raw_args, split_word, unquote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// BASEX
//
// WHAT:  The structure commands of BaseX's own command language, as typed in its
//        console (`basex`) or the GUI's command mode: `LIST`, `LIST <database>
//        [<path>]`, `INFO`, `INFO DB`, `SHOW BACKUPS`, `SHOW USERS` and
//        `SHOW SESSIONS`.
// WHY:   BaseX has two languages, XQuery and a command language. This connection's
//        `execute` runs XQuery only (`POST /rest` with a `<query>` envelope, and
//        `GET <path>` for one resource), so a command typed as such would be
//        evaluated as a path expression and answer with nothing.
// HOW:   Each command becomes the XQuery function BaseX documents as its
//        counterpart: LIST -> `db:list-details()`, LIST db path ->
//        `db:list-details("db", "path")`, INFO -> `db:system()`, INFO DB ->
//        `db:info("db")`, SHOW BACKUPS -> `db:backups()`, SHOW USERS ->
//        `user:list-details()`, SHOW SESSIONS -> `admin:sessions()`. Names go in as
//        XQuery string literals (`"` doubled, `&` as `&amp;`, control characters
//        refused), so an argument can never leave its literal. All seven are reads;
//        `is_write_xquery` in integrations/basex.rs flags none of them.
//        `INFO DB` names the database the connection is on; a name after it is a DB
//        Free addition for connections that have none, since the console cannot OPEN
//        a database. `INFO INDEX`, `INFO STORAGE` and `SHOW DATABASES` have no
//        function that returns the same text and are refused with a pointer.
//        `INFO DATABASE` is BaseX's spelling of `INFO DB`. `SHOW DATABASES` was removed
//        from BaseX in 7.5 and is refused with a pointer to `LIST`.
//        The help word is `?` (and `\?`): a line holding only `help` is a valid
//        XQuery path step, and so is a line holding only `list` or `info`. The dialect
//        is therefore cut by statements, not by lines: a command is recognised at the
//        start of a statement (a script, or the text after a `;`), never in the middle
//        of a multi-line XQuery. A command word followed by an XQuery operator
//        (`info = 3`, `list | $x`) is an expression and is left alone.
// WHERE: src-tauri/src/integrations/basex.rs (`execute`, `is_write_xquery`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["?", "\\?"], cut: Cut::Statements { line_words: &["?", "\\?", "list", "info", "show"] }, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("?", "? | \\?", "List the commands this console understands"),
    ShellCommand::shell("LIST", "LIST [<database> [<path>]]", "The databases, or the resources of one database"),
    ShellCommand::shell("INFO", "INFO", "Global options and system information of the server"),
    ShellCommand::shell("INFO DB", "INFO DB [<database>]", "Properties of the connected database (a name is a DB Free addition)"),
    ShellCommand::shell("SHOW BACKUPS", "SHOW BACKUPS", "The database backups on the server"),
    ShellCommand::shell("SHOW USERS", "SHOW USERS", "Users with their permissions (all of them for an admin, otherwise only yourself)"),
    ShellCommand::shell("SHOW SESSIONS", "SHOW SESSIONS", "Client sessions open on the server (needs an admin)"),
    ShellCommand::native("db:list()", "db:list()", "XQuery: the names of the databases"),
    ShellCommand::native("db:list(\"<database>\")", "db:list(\"<database>\")", "XQuery: the resources of a database"),
    ShellCommand::native("db:info(\"<database>\")", "db:info(\"<database>\")", "XQuery: properties and indexes of a database"),
    ShellCommand::native("GET", "GET <path>", "One resource of the connected database"),
];

fn plan(ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    if DIALECT.help.iter().any(|h| eq_ci(h, text)) {
        return Some(Ok(help_plan(&DIALECT)));
    }
    let (head, rest) = split_word(text);
    // `info = 3` and `list | $x` are expressions over elements of those names.
    if rest.starts_with(|c: char| "=!<>|,+*()[]{}:@$%^&?/#".contains(c)) {
        return None;
    }
    let args = || raw_args(rest).map(|raw| raw.iter().map(|a| unquote(a)).collect::<Vec<_>>()).map_err(AppError::invalid_input);
    match head.to_ascii_lowercase().as_str() {
        "list" => Some(args().and_then(|a| list(&a))),
        "info" => Some(args().and_then(|a| info(ctx, &a))),
        "show" => Some(args().and_then(|a| show(&a))),
        _ => None,
    }
}

fn list(args: &[String]) -> AppResult<Plan> {
    match args {
        [] => Ok(Plan::native("db:list-details()")),
        [db] => Ok(Plan::native(format!("db:list-details({})", xquery_string(db)?))),
        [db, path] => Ok(Plan::native(format!("db:list-details({}, {})", xquery_string(db)?, xquery_string(path)?))),
        _ => Err(AppError::invalid_input("LIST takes a database and a path: LIST [<database> [<path>]].")),
    }
}

fn info(ctx: &Ctx, args: &[String]) -> AppResult<Plan> {
    match args {
        [] => Ok(Plan::native("db:system()")),
        [db, rest @ ..] if eq_ci(db, "db") || eq_ci(db, "database") => {
            let named = match rest {
                [] => ctx.connection.and_then(|c| c.database.as_deref()).map(str::trim).filter(|d| !d.is_empty()),
                [name] => Some(name.as_str()),
                _ => return Err(AppError::invalid_input("INFO DB takes at most a database name.")),
            };
            let name = named.ok_or_else(|| AppError::invalid_input("INFO DB reports the database the connection is on, and this connection has none. Name one: INFO DB <database>."))?;
            Ok(Plan::native(format!("db:info({})", xquery_string(name)?)))
        }
        _ => Err(AppError::invalid_input("INFO INDEX and INFO STORAGE are not translated. Supported: INFO, INFO DB [<database>]. db:info(\"<database>\") shows the indexes.")),
    }
}

fn show(args: &[String]) -> AppResult<Plan> {
    match args.iter().map(|a| a.to_ascii_lowercase()).collect::<Vec<_>>().as_slice() {
        [what] if what == "backups" => Ok(Plan::native("db:backups()")),
        [what] if what == "users" => Ok(Plan::native("user:list-details()")),
        [what] if what == "sessions" => Ok(Plan::native("admin:sessions()")),
        [what] if what == "databases" => Err(AppError::invalid_input("SHOW DATABASES no longer exists in BaseX (it was removed in 7.5). LIST shows every database; SHOW SESSIONS shows the open sessions.")),
        _ => Err(AppError::invalid_input("Supported: SHOW BACKUPS, SHOW USERS, SHOW SESSIONS.")),
    }
}

// WHAT:  `raw` as an XQuery string literal.
// WHY:   The only way an argument enters the query: inside `"..."` with `"` doubled
//        and `&` written as the predefined entity, which is all XQuery needs to
//        keep any other character literal. Control characters have no place in a name.
fn xquery_string(raw: &str) -> AppResult<String> {
    if raw.chars().any(char::is_control) {
        return Err(AppError::invalid_input("A name cannot hold line breaks or control characters."));
    }
    Ok(format!("\"{}\"", raw.replace('&', "&amp;").replace('"', "\"\"")))
}

#[cfg(test)]
mod tests {
    use super::super::{plan, segments, CommandOrigin, Ctx, Segment, Step};
    use crate::model::{ConnectionInput, ConnectionSummary, Engine};

    fn summary(database: Option<&str>) -> ConnectionSummary {
        let input: Option<ConnectionInput> = serde_json::from_value(serde_json::json!({
            "name": "x", "engine": "basex", "environment": "local", "readOnly": false, "host": "h", "port": null,
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

    fn statements_in(database: Option<&str>, text: &str) -> Vec<String> {
        let summary = summary(database);
        let ctx = Ctx::new(Engine::Basex, Some(&summary), None);
        match plan(&ctx, text) {
            Some(Ok(p)) => p.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn statements(text: &str) -> Vec<String> {
        statements_in(Some("factbook"), text)
    }

    fn refused_in(database: Option<&str>, text: &str) -> String {
        let summary = summary(database);
        match plan(&Ctx::new(Engine::Basex, Some(&summary), None), text) {
            Some(Err(e)) => e.message().to_string(),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn the_help_marks_list_every_command_and_a_bare_help_stays_xquery() {
        for word in ["?", "\\?", "?;"] {
            let found = plan(&Ctx::bare(Engine::Basex), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}");
        }
        assert!(plan(&Ctx::bare(Engine::Basex), "help").is_none(), "`help` is a valid path step");
    }

    #[test]
    fn list_becomes_db_list_details() {
        assert_eq!(statements("LIST"), ["db:list-details()"]);
        assert_eq!(statements("list;"), ["db:list-details()"]);
        assert_eq!(statements("LIST factbook"), ["db:list-details(\"factbook\")"]);
        assert_eq!(statements("list factbook countries/"), ["db:list-details(\"factbook\", \"countries/\")"]);
        assert_eq!(statements("LIST \"my db\" \"a b/c.xml\""), ["db:list-details(\"my db\", \"a b/c.xml\")"]);
    }

    #[test]
    fn info_commands_use_the_documented_functions() {
        assert_eq!(statements("INFO"), ["db:system()"]);
        assert_eq!(statements("info db"), ["db:info(\"factbook\")"]);
        assert_eq!(statements_in(None, "INFO DB other"), ["db:info(\"other\")"]);
        assert!(refused_in(None, "INFO DB").contains("has none"));
        assert!(refused_in(Some("  "), "INFO DB").contains("has none"));
        assert!(refused_in(Some("a"), "INFO INDEX").contains("not translated"));
        assert!(refused_in(Some("a"), "INFO DB a b").contains("at most"));
    }

    #[test]
    fn info_database_is_info_db() {
        assert_eq!(statements("INFO DATABASE"), ["db:info(\"factbook\")"]);
        assert_eq!(statements_in(None, "info database other"), ["db:info(\"other\")"]);
    }

    #[test]
    fn a_command_word_followed_by_an_operator_is_an_xquery_expression() {
        for text in ["info = 3", "list | $x", "show, 1", "info[1]", "list/name", "info != 'a'"] {
            assert!(plan(&Ctx::bare(Engine::Basex), text).is_none(), "{text}");
        }
    }

    #[test]
    fn a_command_is_cut_out_only_at_the_start_of_a_statement() {
        let query = "for $c in //country\nreturn\n  info";
        assert_eq!(segments(Engine::Basex, query), vec![Segment::Native(query.to_string())], "a line of a multi-line XQuery is never cut out");
        assert_eq!(segments(Engine::Basex, "LIST\nINFO"), vec![Segment::Meta("LIST".to_string()), Segment::Meta("INFO".to_string())]);
        assert_eq!(segments(Engine::Basex, "//item;\nSHOW USERS"), vec![Segment::Native("//item".to_string()), Segment::Meta("SHOW USERS".to_string())]);
        assert_eq!(segments(Engine::Basex, "?\nLIST factbook"), vec![Segment::Meta("?".to_string()), Segment::Meta("LIST factbook".to_string())]);
    }

    #[test]
    fn show_commands_use_the_documented_functions() {
        assert_eq!(statements("SHOW BACKUPS"), ["db:backups()"]);
        assert_eq!(statements("show users"), ["user:list-details()"]);
        assert_eq!(statements("Show Sessions;"), ["admin:sessions()"]);
        assert!(refused_in(Some("a"), "SHOW DATABASES").contains("LIST") && refused_in(Some("a"), "SHOW DATABASES").contains("7.5"));
        assert!(refused_in(Some("a"), "SHOW EVENTS").contains("Supported"));
        assert!(refused_in(Some("a"), "SHOW").contains("Supported"));
    }

    #[test]
    fn a_hostile_name_stays_inside_its_string_literal() {
        let found = statements("LIST 'x\") , db:delete(\"y' ");
        assert_eq!(found, ["db:list-details(\"x\"\") , db:delete(\"\"y\")"]);
        let amp = statements("LIST 'a&amp;b'");
        assert_eq!(amp, ["db:list-details(\"a&amp;amp;b\")"]);
        assert!(refused_in(Some("a"), "LIST 'x").contains("quote"));
        assert!(refused_in(Some("a"), "LIST a b c").contains("takes a database and a path"));
    }

    #[test]
    fn none_of_the_statements_is_a_write_for_the_adapter() {
        for text in ["LIST", "LIST a b", "INFO", "INFO DB a", "SHOW BACKUPS", "SHOW USERS", "SHOW SESSIONS"] {
            for statement in statements(text) {
                let upper = statement.to_uppercase();
                for write in ["DB:CREATE", "DB:ADD", "DB:DELETE", "DB:DROP", "DB:REPLACE", "DB:STORE", "DB:RENAME", "DB:PUT", "DB:OPTIMIZE", "DB:ALTER", "DB:COPY", "DB:RESTORE", "UPDATE:", "INSERT NODE", "DELETE NODE"] {
                    assert!(!upper.contains(write), "{statement}");
                }
            }
        }
    }

    #[test]
    fn xquery_and_the_adapters_own_commands_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Basex), native.name).is_none(), "{}", native.name);
        }
        for text in ["db:list()", "//country[@name = 'x']", "for $c in //country return $c/@name", "GET countries/a.xml", "listing", "information"] {
            assert!(plan(&Ctx::bare(Engine::Basex), text).is_none(), "{text}");
        }
    }
}
