// SOT: shell-meta-mongodb, mongosh-structure-commands

use super::args::{eq_ci, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// MONGODB: mongosh's structure commands
//
// WHAT:  `help`, and the listing of the structure commands the MongoDB console
//        already runs: `show dbs`, `show collections`, `show users`, `show roles`,
//        `show profile`, `show logs`, `use <db>`, and the `db.…()` helpers that
//        describe a database or a collection.
// WHY:   mongosh's structure commands are statements of its JavaScript shell. The
//        console reads them itself (`integrations::mongo_shell` turns each into
//        the command document it stands for and the adapter runs it), so unlike
//        the SQL shells there is nothing to translate here: the dialect only has
//        to answer `help` and tell the editor what the console understands.
// HOW:   Only `help` is planned (a local table). Every other entry is `Native`:
//        it reaches the console as typed, and is listed so it can be found.
// WHERE: src-tauri/src/integrations/mongo_shell.rs (the translation),
//        src-tauri/src/integrations/mongodb.rs (`execute`)
// ============================================================================

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::native("show dbs", "show dbs  |  show databases", "List the databases with their sizes"),
    ShellCommand::native("show collections", "show collections  |  show tables", "List the collections of the current database"),
    ShellCommand::native("show users", "show users", "List the users of the current database"),
    ShellCommand::native("show roles", "show roles", "List the roles of the current database, built-in ones included"),
    ShellCommand::native("show profile", "show profile", "Show the last five entries of the profiler"),
    ShellCommand::native("show logs", "show logs  |  show log [NAME]", "List the server's log names, or print one log"),
    ShellCommand::native("use", "use DATABASE", "Run the statements after it against another database"),
    ShellCommand::native("db.stats()", "db.stats()", "Show the current database's size and object counts"),
    ShellCommand::native("db.getCollectionNames()", "db.getCollectionNames()", "List the names of the collections"),
    ShellCommand::native("db.getCollectionInfos()", "db.getCollectionInfos([FILTER])", "List the collections with their options, types and UUIDs"),
    ShellCommand::native("db.COLLECTION.getIndexes()", "db.COLLECTION.getIndexes()", "List a collection's indexes"),
    ShellCommand::native("db.COLLECTION.stats()", "db.COLLECTION.stats()", "Show a collection's size, count and index sizes"),
    ShellCommand::native("db.COLLECTION.validate()", "db.COLLECTION.validate()", "Check a collection's structure and report on it"),
    ShellCommand::native("db.COLLECTION.exists()", "db.COLLECTION.exists()", "Return the collection's entry in the listing, or nothing"),
    ShellCommand::native("db.getUsers()", "db.getUsers()", "List the users of the current database"),
    ShellCommand::native("db.getRoles()", "db.getRoles([{ showBuiltinRoles: true }])", "List the roles of the current database"),
    ShellCommand::native("db.hostInfo()", "db.hostInfo()", "Show the host the server runs on"),
    ShellCommand::native("db.serverStatus()", "db.serverStatus()", "Show the server's counters and state"),
    ShellCommand::native("db.version()", "db.version()", "Show the server version"),
    ShellCommand::native("db.currentOp()", "db.currentOp()", "List the operations the server is running"),
    ShellCommand::native("db.getProfilingStatus()", "db.getProfilingStatus()", "Show the profiler's level and threshold"),
];

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help"], cut: Cut::Lines, commands: COMMANDS, plan };

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    eq_ci(strip_terminator(text), "help").then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::mongo_shell::{self, Shell};
    use crate::integrations::shell_meta::{plan as plan_for, segments, Segment};
    use crate::model::Engine;

    #[test]
    fn only_help_is_planned_and_the_shell_statements_reach_the_console() {
        let ctx = Ctx::bare(Engine::Mongodb);
        assert!(plan_for(&ctx, "help").is_some() && plan_for(&ctx, "HELP;").is_some());
        for text in ["show dbs", "show collections", "use app", "db.users.find({})", "{\"find\": \"users\"}"] {
            assert!(plan_for(&ctx, text).is_none(), "{text} belongs to the console");
        }
    }

    // Everything the help lists as a statement is a statement the console's own
    // translator reads, so the listing never promises what would then be refused.
    #[test]
    fn every_listed_statement_is_one_the_console_reads() {
        let samples = [
            "show dbs", "show databases", "show collections", "show tables", "show users", "show roles", "show profile", "show logs", "show log", "show log global",
            "use app", "db.stats()", "db.getCollectionNames()", "db.getCollectionInfos()", "db.users.getIndexes()", "db.users.stats()", "db.users.validate()",
            "db.users.exists()", "db.getUsers()", "db.getRoles()", "db.hostInfo()", "db.serverStatus()", "db.version()", "db.currentOp()", "db.getProfilingStatus()",
        ];
        for text in samples {
            assert!(mongo_shell::is_shell(text), "{text}");
            assert!(matches!(mongo_shell::parse(text, 100), Ok(Shell::ShowDatabases | Shell::ShowCollections | Shell::Use(_) | Shell::Command(_))), "{text}");
        }
        let listed: Vec<&str> = COMMANDS.iter().filter(|c| c.name.starts_with("show ")).map(|c| c.name).collect();
        for name in listed {
            assert!(samples.contains(&name), "{name} should be exercised above");
        }
    }

    #[test]
    fn help_among_statements_is_cut_out_and_the_rest_is_left_for_the_console() {
        assert_eq!(
            segments(Engine::Mongodb, "help\nshow collections"),
            vec![Segment::Meta("help".to_string()), Segment::Native("show collections".to_string())]
        );
        assert_eq!(segments(Engine::Mongodb, "db.users.find({ help: 1 })").len(), 1);
    }
}
