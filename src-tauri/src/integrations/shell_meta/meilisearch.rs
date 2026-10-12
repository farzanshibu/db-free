// SOT: shell-meta-meilisearch, meilisearch-console-help

use super::args::eq_ci;
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// MEILISEARCH
//
// WHAT:  The structure commands of Meilisearch's console: a `help` listing of the
//        calls that show indexes, their settings, statistics, tasks and keys.
// WHY:   Meilisearch is a REST-only server with no shell and no meta-commands, so
//        there is nothing to translate; the structure calls are native to the
//        console and `help` is how a user finds them.
// HOW:   Only `help` and `?` are intercepted (a JSON body or a `GET /path` line can
//        never be a bare word). The rest are `ShellCommand::native` entries:
//        `parse_command` in integrations/meilisearch.rs reads `INDEXES`, `SEARCH`,
//        `GET|POST|PUT|PATCH|DELETE <path> [json]` and JSON envelopes, so a `GET`
//        line reaches the REST API as typed.
// WHERE: src-tauri/src/integrations/meilisearch.rs (`parse_command`, `run_command`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | ?", "List the calls that show the structure of this server"),
    ShellCommand::native("INDEXES", "INDEXES", "Every index with its primary key and update time"),
    ShellCommand::native("GET /indexes/<uid>", "GET /indexes/<uid>", "One index: primary key, creation and update time"),
    ShellCommand::native("GET /indexes/<uid>/settings", "GET /indexes/<uid>/settings", "Searchable, filterable and sortable attributes, ranking rules"),
    ShellCommand::native("GET /indexes/<uid>/stats", "GET /indexes/<uid>/stats", "Document count and the field distribution of an index"),
    ShellCommand::native("GET /stats", "GET /stats", "Database size and the statistics of every index"),
    ShellCommand::native("GET /tasks", "GET /tasks", "The task queue"),
    ShellCommand::native("GET /keys", "GET /keys", "API keys and what they allow"),
    ShellCommand::native("GET /version", "GET /version", "Server version and commit"),
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
        for word in ["help", "HELP", "Help;", "?"] {
            let found = plan(&Ctx::bare(Engine::Meilisearch), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}: {found:?}");
        }
    }

    #[test]
    fn natives_and_json_bodies_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Meilisearch), native.name).is_none(), "{}", native.name);
        }
        for text in ["SEARCH movies batman", "{\"index\": \"movies\", \"q\": \"help\"}", "helpful", "GET /indexes?limit=5"] {
            assert!(plan(&Ctx::bare(Engine::Meilisearch), text).is_none(), "{text}");
        }
    }
}
