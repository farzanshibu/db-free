// SOT: shell-meta-couchdb, couchdb-rest-structure-requests, couchdb-help

use super::args::{eq_ci, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// COUCHDB
//
// WHAT:  A `help` listing of the requests that describe a CouchDB server and
//        its databases, ready to paste: the list of databases, a database's
//        info, its design documents, indexes and security object, the running
//        tasks and the cluster membership.
// WHY:   CouchDB has no shell. Its structure is read over HTTP, with curl or in
//        Fauxton, so there is no command set to translate; a user opening a
//        query tab needs to be told which request answers which question. The
//        adapter already runs any such request as `{"method","path","body"}`
//        (`couchdb::parse_command`, the `Command::Passthrough` branch of
//        `execute`), and a GET is a read there: only PUT, POST and DELETE are
//        checked against the read-only lock.
// HOW:   Nothing is translated and nothing reaches the server for `help`: the
//        table is built from `COMMANDS`. The listed requests are native, so the
//        plan leaves them alone (`<db>` marks the part to fill in). `GET <id>`
//        and `ALL [n]` (the adapter's shorthands for a document and for
//        `_all_docs`) are listed with them.
// WHERE: src-tauri/src/integrations/couchdb.rs (`parse_command`, `execute`),
//        src-tauri/src/integrations/shell_meta/mod.rs
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help", "List the requests that describe this server"),
    ShellCommand::native(r#"{"method":"GET","path":"/_all_dbs"}"#, r#"{"method":"GET","path":"/_all_dbs"}"#, "Every database on the server"),
    ShellCommand::native(r#"{"method":"GET","path":"/"}"#, r#"{"method":"GET","path":"/"}"#, "The server's welcome message and version"),
    ShellCommand::native(r#"{"method":"GET","path":"/<db>"}"#, r#"{"method":"GET","path":"/<db>"}"#, "A database's document count, sizes and update sequence"),
    ShellCommand::native(r#"{"method":"GET","path":"/<db>/_design_docs"}"#, r#"{"method":"GET","path":"/<db>/_design_docs"}"#, "The design documents of a database"),
    ShellCommand::native(r#"{"method":"GET","path":"/<db>/_index"}"#, r#"{"method":"GET","path":"/<db>/_index"}"#, "The Mango indexes of a database"),
    ShellCommand::native(r#"{"method":"GET","path":"/<db>/_security"}"#, r#"{"method":"GET","path":"/<db>/_security"}"#, "A database's admins and members"),
    ShellCommand::native(r#"{"method":"GET","path":"/_active_tasks"}"#, r#"{"method":"GET","path":"/_active_tasks"}"#, "Running indexing, compaction and replication tasks"),
    ShellCommand::native(r#"{"method":"GET","path":"/_membership"}"#, r#"{"method":"GET","path":"/_membership"}"#, "The nodes of the cluster"),
    ShellCommand::native(r#"{"method":"GET","path":"/_scheduler/docs"}"#, r#"{"method":"GET","path":"/_scheduler/docs"}"#, "Replication jobs and their state"),
    ShellCommand::native("ALL", "ALL [n]", "The first n documents of the selected database"),
    ShellCommand::native("GET", "GET id", "One document of the selected database by its id"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    eq_ci(strip_terminator(text), "help").then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::couchdb::split_blank_lines;
    use crate::integrations::shell_meta::{CommandOrigin, Segment, Step};
    use crate::model::Engine;

    fn run(text: &str) -> Option<AppResult<Plan>> {
        plan(&Ctx::bare(Engine::Couchdb), text)
    }

    #[test]
    fn help_lists_every_request() {
        for text in ["help", "HELP", "help;"] {
            let Some(Ok(plan)) = run(text) else { panic!("{text}") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn requests_and_shorthands_are_not_intercepted() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(run(command.name).is_none(), "{}", command.name);
        }
        for text in ["GET help", "ALL 5", "help me", r#"{"selector": {"help": 1}}"#] {
            assert!(run(text).is_none(), "{text}");
        }
    }

    #[test]
    fn the_listed_requests_are_json_gets_with_a_path() {
        for command in COMMANDS.iter().filter(|c| c.name.starts_with('{')) {
            let doc: serde_json::Value = serde_json::from_str(command.name).unwrap_or_default();
            assert_eq!(doc["method"], "GET", "{}", command.name);
            assert!(doc["path"].as_str().is_some_and(|p| p.starts_with('/')), "{}", command.name);
            assert_eq!(command.name, command.syntax);
        }
    }

    #[test]
    fn a_help_line_among_requests_is_cut_out_of_the_block() {
        let script = "help\n{\"method\":\"GET\",\"path\":\"/_all_dbs\"}\n\nALL 3";
        let found = crate::integrations::shell_meta::segments(Engine::Couchdb, script);
        assert_eq!(found[0], Segment::Meta("help".to_string()));
        // What remains is what the adapter splits on blank lines as before.
        let Some(Segment::Native(rest)) = found.get(1) else { panic!("{found:?}") };
        assert_eq!(split_blank_lines(rest).len(), 2);
    }
}
