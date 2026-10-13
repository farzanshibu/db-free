// SOT: shell-meta-convex, convex-function-console-help

use super::args::{eq_ci, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// CONVEX
//
// WHAT:  A `help` listing for the Convex tab, which is a function console: it
//        runs a deployed query, mutation or action by its path.
// WHY:   Convex has no structure commands to translate. `npx convex data` and
//        `npx convex function-spec` read tables and function signatures through
//        the deployment's admin API, which is not part of the HTTP functions API
//        the adapter uses (`/api/query`, `/api/mutation`, `/api/action`;
//        `convex::parse_command`), and the adapter says as much: no table or
//        function listing exists over that API. So the only honest structure
//        help is how to call the functions that return it, which the user's
//        deployment defines.
// HOW:   `help` returns the table built from `COMMANDS`; the function-call
//        syntaxes are listed as native and never intercepted.
// WHERE: src-tauri/src/integrations/convex.rs (`parse_command`, `execute`,
//        `NO_TABLES`), src-tauri/src/integrations/shell_meta/mod.rs
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help", "List the commands this console understands (Convex lists no tables or functions over HTTP)"),
    ShellCommand::native("QUERY", "QUERY module:function [{\"arg\": value}]", "Run a query function and show what it returns"),
    ShellCommand::native(r#"{"function":"<module:function>"}"#, r#"{"function":"module:function","args":{},"kind":"query"}"#, "Run a function from a JSON body; kind is query, mutation or action"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    eq_ci(strip_terminator(text), "help").then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::convex::parse_command;
    use crate::integrations::shell_meta::{CommandOrigin, Step};
    use crate::model::Engine;

    fn run(text: &str) -> Option<AppResult<Plan>> {
        plan(&Ctx::bare(Engine::Convex), text)
    }

    #[test]
    fn help_lists_every_command() {
        for text in ["help", "HELP", "help;"] {
            let Some(Ok(plan)) = run(text) else { panic!("{text}") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn function_calls_are_not_intercepted() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(run(command.name).is_none(), "{}", command.name);
        }
        // A function named help is a function: only the bare word is the command.
        for text in ["QUERY help:list", "help:list", "helpers:all", r#"{"function": "help:list"}"#] {
            assert!(run(text).is_none(), "{text}");
        }
    }

    #[test]
    fn the_listed_call_forms_are_ones_the_adapter_parses() {
        assert!(parse_command("QUERY messages:list").is_ok());
        assert!(parse_command(r#"{"function":"messages:list","args":{},"kind":"query"}"#).is_ok());
        assert!(parse_command(COMMANDS[2].name).is_ok(), "the template runs as is once the name is filled in");
    }
}
