// SOT: shell-meta-chroma, chroma-console-help

use super::args::eq_ci;
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// CHROMA
//
// WHAT:  The structure commands of Chroma's console: a `help` listing of the
//        shorthands and REST envelopes that show collections, tenants and databases.
// WHY:   Chroma's `chroma` command line starts and browses a server but has no
//        structure commands of its own (collections are listed through the Python
//        and JavaScript clients), so there is nothing to translate. The structure
//        calls are native to the console and `help` is how a user finds them.
// HOW:   Only `help` and `?` are intercepted (the console's text is a shorthand
//        starting with a known verb, or JSON). `parse_command` in
//        integrations/chroma.rs reads `COLLECTIONS`, `COUNT <c>`, `GET <c> [n]` and
//        JSON; `{"method": "GET", "path": "/api/v2/..."}` is its raw passthrough.
// WHERE: src-tauri/src/integrations/chroma.rs (`parse_command`, `run`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | ?", "List the calls that show the structure of this server"),
    ShellCommand::native("COLLECTIONS", "COLLECTIONS", "Every collection of the connected tenant and database"),
    ShellCommand::native("COUNT", "COUNT <collection>", "Number of records in a collection"),
    ShellCommand::native("GET", "GET <collection> [n]", "The first records of a collection, to see their metadata keys"),
    ShellCommand::native("{\"method\": \"GET\", \"path\": \"/api/v2/tenants/<tenant>/databases\"}", "{\"method\": \"GET\", \"path\": \"/api/v2/tenants/<tenant>/databases\"}", "Databases of a tenant (Chroma v2 API)"),
    ShellCommand::native("{\"method\": \"GET\", \"path\": \"/api/v2/version\"}", "{\"method\": \"GET\", \"path\": \"/api/v2/version\"}", "Server version"),
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
            let found = plan(&Ctx::bare(Engine::Chroma), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}: {found:?}");
        }
    }

    #[test]
    fn natives_and_json_bodies_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Chroma), native.name).is_none(), "{}", native.name);
        }
        for text in ["COUNT notes", "GET notes 5", "{\"collection\": \"notes\", \"get\": {\"limit\": 3}}", "help notes"] {
            assert!(plan(&Ctx::bare(Engine::Chroma), text).is_none(), "{text}");
        }
    }
}
