// SOT: shell-meta-qdrant, qdrant-console-help

use super::args::eq_ci;
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// QDRANT
//
// WHAT:  The structure commands of Qdrant's console: a `help` listing of the
//        shorthands and REST envelopes that show collections, aliases, snapshots
//        and the cluster.
// WHY:   Qdrant is reached over REST (and gRPC) with client libraries only; it has
//        no shell and so no meta-commands to translate. The structure calls are
//        native to the console and `help` is how a user finds them.
// HOW:   Only `help` and `?` are intercepted (the console's text is a one-word
//        shorthand or a JSON body, never a bare `help`). `parse_command` in
//        integrations/qdrant.rs reads `COLLECTIONS`, `INFO <c>`, `SCROLL <c> [n]`
//        and JSON; `{"method": "GET", "path": "/aliases"}` is its raw passthrough.
// WHERE: src-tauri/src/integrations/qdrant.rs (`parse_command`, `run`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | ?", "List the calls that show the structure of this server"),
    ShellCommand::native("COLLECTIONS", "COLLECTIONS", "Every collection"),
    ShellCommand::native("INFO", "INFO <collection>", "Configuration, vector parameters and point counts of a collection"),
    ShellCommand::native("SCROLL", "SCROLL <collection> [n]", "The first points of a collection, to see their payload fields"),
    ShellCommand::native("{\"method\": \"GET\", \"path\": \"/aliases\"}", "{\"method\": \"GET\", \"path\": \"/aliases\"}", "Collection aliases"),
    ShellCommand::native("{\"method\": \"GET\", \"path\": \"/collections/<name>/snapshots\"}", "{\"method\": \"GET\", \"path\": \"/collections/<name>/snapshots\"}", "Snapshots of a collection"),
    ShellCommand::native("{\"method\": \"GET\", \"path\": \"/collections/<name>/cluster\"}", "{\"method\": \"GET\", \"path\": \"/collections/<name>/cluster\"}", "Shard placement of a collection"),
    ShellCommand::native("{\"method\": \"GET\", \"path\": \"/cluster\"}", "{\"method\": \"GET\", \"path\": \"/cluster\"}", "Cluster status and peers"),
    ShellCommand::native("{\"method\": \"GET\", \"path\": \"/telemetry\"}", "{\"method\": \"GET\", \"path\": \"/telemetry\"}", "Version, collections and request statistics"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    DIALECT.help.iter().any(|h| eq_ci(h, text)).then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::super::{plan, CommandOrigin, Ctx, Step};
    use crate::model::Engine;

    #[test]
    fn help_is_translated_in_any_case_and_lists_every_command() {
        for word in ["help", "Help", "help;", "?"] {
            let found = plan(&Ctx::bare(Engine::Qdrant), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}: {found:?}");
        }
    }

    #[test]
    fn natives_and_json_bodies_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Qdrant), native.name).is_none(), "{}", native.name);
        }
        for text in ["INFO books", "SCROLL books 5", "{\"collection\": \"books\", \"search\": {\"vector\": [0.1]}}", "helpers"] {
            assert!(plan(&Ctx::bare(Engine::Qdrant), text).is_none(), "{text}");
        }
    }
}
