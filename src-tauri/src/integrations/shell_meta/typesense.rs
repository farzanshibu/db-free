// SOT: shell-meta-typesense, typesense-console-help

use super::args::eq_ci;
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// TYPESENSE
//
// WHAT:  The structure commands of Typesense's console: a `help` listing of the
//        calls that show collections, schemas, aliases, keys and health.
// WHY:   Typesense is reached over REST only and has no shell, so there are no
//        meta-commands to translate; the structure calls are native to the
//        console and `help` is how a user finds them.
// HOW:   Only `help` and `?` are intercepted (a JSON body or a `GET /path` line can
//        never be a bare word). `parse_command` in integrations/typesense.rs reads
//        `COLLECTIONS`, `SEARCH <collection> <text>`, `GET|POST|... <path> [json]`
//        and JSON envelopes, so the entries below reach the server as typed.
// WHERE: src-tauri/src/integrations/typesense.rs (`parse_command`, `run_command`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | ?", "List the calls that show the structure of this server"),
    ShellCommand::native("COLLECTIONS", "COLLECTIONS", "Every collection with its document count and fields"),
    ShellCommand::native("GET /collections/<name>", "GET /collections/<name>", "The schema of one collection"),
    ShellCommand::native("GET /collections/<name>/overrides", "GET /collections/<name>/overrides", "Curation rules of a collection"),
    ShellCommand::native("GET /collections/<name>/synonyms", "GET /collections/<name>/synonyms", "Synonym sets of a collection"),
    ShellCommand::native("GET /aliases", "GET /aliases", "Collection aliases"),
    ShellCommand::native("GET /keys", "GET /keys", "API keys and what they allow"),
    ShellCommand::native("GET /stats.json", "GET /stats.json", "Request and latency statistics"),
    ShellCommand::native("GET /debug", "GET /debug", "Server version"),
    ShellCommand::native("GET /health", "GET /health", "Health check"),
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
        for word in ["help", "HELP", "help;", "?"] {
            let found = plan(&Ctx::bare(Engine::Typesense), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}: {found:?}");
        }
    }

    #[test]
    fn natives_and_json_bodies_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Typesense), native.name).is_none(), "{}", native.name);
        }
        for text in ["SEARCH books dune", "{\"collection\": \"books\", \"search\": {\"q\": \"*\"}}", "help me", "GET /collections?limit=3"] {
            assert!(plan(&Ctx::bare(Engine::Typesense), text).is_none(), "{text}");
        }
    }
}
