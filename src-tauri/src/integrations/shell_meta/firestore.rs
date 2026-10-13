// SOT: shell-meta-firestore, firestore-collections-help, firestore-structure-commands

use super::args::{eq_ci, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// CLOUD FIRESTORE
//
// WHAT:  A `help` listing of what the Firestore tab runs to describe a project:
//        `COLLECTIONS` for the root collections, `GET collection/id` for one
//        document, and a one-document peek into a collection.
// WHY:   Firestore has no query shell. Its tools are the console, the Admin SDKs
//        and the `gcloud firestore` / `firebase` command lines, whose structure
//        commands (`gcloud firestore indexes composite list`, `firestore
//        databases list`) call admin endpoints the adapter's `execute` does not
//        reach: it runs a structured query, the `{"collection": …}` sugar,
//        `COLLECTIONS`, `GET` and the set/delete documents
//        (`firestore::parse_command`). Claiming those CLI commands would be
//        inventing them, so only what the tab runs itself is listed.
// HOW:   `help` returns the table built from `COMMANDS`; nothing else is
//        translated, and `COLLECTIONS` stays native (it lists the root
//        collections through `listCollectionIds`).
// WHERE: src-tauri/src/integrations/firestore.rs (`parse_command`, `execute`),
//        src-tauri/src/integrations/shell_meta/mod.rs
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::native("COLLECTIONS", "COLLECTIONS", "The root collections of the database"),
    ShellCommand::native("GET", "GET collection/id", "One document by its path; a nested collection is collection/id/subcollection/id"),
    ShellCommand::native(r#"{"collection":"<name>","limit":1}"#, r#"{"collection":"<name>","where":[[field,op,value]],"orderBy":"field","limit":n}"#, "Peek into a collection; a path such as users/u1/orders reaches a sub-collection"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    eq_ci(strip_terminator(text), "help").then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{CommandOrigin, Step};
    use crate::model::Engine;

    fn run(text: &str) -> Option<AppResult<Plan>> {
        plan(&Ctx::bare(Engine::Firestore), text)
    }

    #[test]
    fn help_lists_every_command() {
        for text in ["help", "HELP", "help;"] {
            let Some(Ok(plan)) = run(text) else { panic!("{text}") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn the_adapters_commands_are_not_intercepted() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(run(command.name).is_none(), "{}", command.name);
        }
        for text in ["COLLECTIONS", "collections", "GET users/u1", "help users", r#"{"collection": "help"}"#] {
            assert!(run(text).is_none(), "{text}");
        }
    }

    #[test]
    fn the_peek_template_is_valid_json() {
        let peek = COMMANDS.iter().find(|c| c.name.starts_with('{'));
        let doc: serde_json::Value = peek.map(|c| serde_json::from_str(c.name).unwrap_or_default()).unwrap_or_default();
        assert_eq!(doc["limit"], 1);
        assert_eq!(doc["collection"], "<name>");
    }
}
