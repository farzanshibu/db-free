// SOT: shell-meta-mysql, mysql-client-commands

use super::args::{eq_ci, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand, Step, Table};
use crate::error::AppResult;

// ============================================================================
// MYSQL FAMILY: the mysql client's own commands
//
// WHAT:  `\s` / `status` (what the server and the session are), and the help
//        listing (`\?`, `\h`, `help`, `?`) for MySQL, MariaDB, TiDB and PlanetScale.
// WHY:   The mysql client has no `\d`-style structure commands: structure is asked
//        of the server with `SHOW …` and `DESCRIBE`, which the server runs itself.
//        So the only client-side structure command is `status`; the help names
//        the server's own statements so they can be found from the editor.
// HOW:   `status` is a local table of what the saved connection says plus one
//        SELECT over functions and variables every MySQL-protocol server answers.
//        The native statements are listed, not intercepted: they reach the server
//        as typed.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (dispatch)
// ============================================================================

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("\\s", "\\s  |  status", "Show the server and the session: version, user, database, character sets"),
    ShellCommand::shell("\\?", "\\?  |  \\h  |  help  |  ?", "List the commands this console understands"),
    ShellCommand::native("SHOW DATABASES", "SHOW DATABASES [LIKE 'pattern']", "List databases"),
    ShellCommand::native("SHOW TABLES", "SHOW [FULL] TABLES [FROM db] [LIKE 'pattern']", "List tables (FULL adds the table type)"),
    ShellCommand::native("DESCRIBE", "DESCRIBE table  |  SHOW COLUMNS FROM table", "Show a table's columns"),
    ShellCommand::native("SHOW CREATE TABLE", "SHOW CREATE TABLE table", "Show the statement that creates a table"),
    ShellCommand::native("SHOW CREATE VIEW", "SHOW CREATE VIEW view", "Show the statement that creates a view"),
    ShellCommand::native("SHOW INDEX", "SHOW INDEX FROM table", "List a table's indexes"),
    ShellCommand::native("SHOW TABLE STATUS", "SHOW TABLE STATUS [FROM db] [LIKE 'pattern']", "Show engine, row count and size per table"),
    ShellCommand::native("SHOW TRIGGERS", "SHOW TRIGGERS [FROM db]", "List triggers"),
    ShellCommand::native("SHOW PROCEDURE STATUS", "SHOW PROCEDURE STATUS [LIKE 'pattern']", "List stored procedures"),
    ShellCommand::native("SHOW FUNCTION STATUS", "SHOW FUNCTION STATUS [LIKE 'pattern']", "List stored functions"),
    ShellCommand::native("SHOW EVENTS", "SHOW EVENTS [FROM db]", "List scheduled events"),
    ShellCommand::native("SHOW GRANTS", "SHOW GRANTS [FOR user]", "Show the privileges of an account"),
    ShellCommand::native("SHOW PROCESSLIST", "SHOW [FULL] PROCESSLIST", "List the server's connections and what they run"),
    ShellCommand::native("SHOW VARIABLES", "SHOW [GLOBAL] VARIABLES [LIKE 'pattern']", "List server variables"),
    ShellCommand::native("SHOW STATUS", "SHOW [GLOBAL] STATUS [LIKE 'pattern']", "List server status counters"),
    ShellCommand::native("SHOW ENGINES", "SHOW ENGINES", "List storage engines"),
    ShellCommand::native("SHOW CHARACTER SET", "SHOW CHARACTER SET [LIKE 'pattern']", "List character sets"),
    ShellCommand::native("SHOW COLLATION", "SHOW COLLATION [LIKE 'pattern']", "List collations"),
];

pub const DIALECT: Dialect = Dialect {
    prefix: Some("\\"),
    strict: true,
    help: &["\\?", "\\h", "help", "?"],
    cut: Cut::Statements { line_words: &["status", "help"] },
    commands: COMMANDS,
    plan,
};

fn plan(ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    let status = eq_ci(head, "\\s") || eq_ci(head, "status");
    if status && rest.is_empty() {
        return Some(Ok(status_plan(ctx)));
    }
    let help = ["\\?", "\\h", "help", "?"].iter().any(|name| eq_ci(head, name));
    // `help 'contents'` is the server's own HELP statement and runs as typed.
    (help && rest.is_empty()).then(|| Ok(help_plan(&DIALECT)))
}

// WHAT:  `\s`: the connection as saved, then what the server reports for this session.
fn status_plan(ctx: &Ctx) -> Plan {
    let session = Step::Native(
        "SELECT CONNECTION_ID() AS `Connection id`, DATABASE() AS `Current database`, CURRENT_USER() AS `Current user`, VERSION() AS `Server version`, @@character_set_server AS `Server characterset`, @@character_set_database AS `Db characterset`, @@character_set_client AS `Client characterset`, @@character_set_connection AS `Conn. characterset`".to_string(),
    );
    let Some(connection) = ctx.connection else {
        return Plan { steps: vec![session] };
    };
    let show = |value: Option<&str>| value.filter(|v| !v.is_empty()).unwrap_or("(default)").to_string();
    let rows = vec![
        vec!["Connection".to_string(), connection.name.clone()],
        vec!["Host".to_string(), show(connection.host.as_deref())],
        vec!["Port".to_string(), connection.port.map_or_else(|| "(default)".to_string(), |p| p.to_string())],
        vec!["User".to_string(), show(connection.username.as_deref())],
        vec!["Database".to_string(), show(connection.database.as_deref())],
    ];
    Plan::table(Table::new(&["Parameter", "Value"], rows)).then(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{plan as plan_for, segments, Segment};
    use crate::model::Engine;

    #[test]
    fn status_is_a_session_query_in_every_spelling() {
        for text in ["\\s", "\\S", "status", "STATUS;", "  \\s  "] {
            let Some(Ok(plan)) = plan_for(&Ctx::bare(Engine::Mysql), text) else { panic!("{text} not understood") };
            let statements: Vec<&str> = plan.statements().collect();
            assert_eq!(statements.len(), 1, "{text}");
            assert!(statements[0].contains("CONNECTION_ID()") && statements[0].contains("@@character_set_server"), "{text}");
            assert!(crate::guard::destructive::classify(statements[0]).iter().all(|s| s.kind == crate::guard::destructive::StatementKind::Read));
        }
    }

    #[test]
    fn help_is_a_local_table_for_every_spelling_and_the_servers_help_statement_is_left_alone() {
        for text in ["\\?", "\\h", "help", "?"] {
            assert!(matches!(plan_for(&Ctx::bare(Engine::Mariadb), text), Some(Ok(p)) if matches!(p.steps.as_slice(), [Step::Table(_)])), "{text}");
        }
        assert!(plan_for(&Ctx::bare(Engine::Mysql), "HELP 'contents'").is_none(), "HELP with a topic is a server statement");
        assert!(plan_for(&Ctx::bare(Engine::Mysql), "SHOW TABLES").is_none());
        assert!(plan_for(&Ctx::bare(Engine::Mysql), "DESCRIBE users").is_none());
    }

    #[test]
    fn another_shells_commands_are_refused_with_a_pointer_to_help() {
        let Some(Err(refused)) = plan_for(&Ctx::bare(Engine::Mysql), "\\dt") else { panic!("expected a refusal") };
        assert!(refused.message().contains("\\dt") && refused.message().contains("\\?"), "{}", refused.message());
    }

    #[test]
    fn status_without_a_semicolon_ends_at_its_line() {
        assert_eq!(
            segments(Engine::Mysql, "status\nselect 1"),
            vec![Segment::Meta("status".to_string()), Segment::Native("select 1".to_string())]
        );
    }
}
