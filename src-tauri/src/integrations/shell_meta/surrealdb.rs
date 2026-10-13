// SOT: shell-meta-surrealdb, surrealdb-structure-statements

use super::args::{eq_ci, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// SURREALDB
//
// WHAT:  `help`: a listing of the `INFO FOR …` statements SurrealQL answers about its
//        own structure.
// WHY:   `surreal sql` has no meta-commands of its own: structure is asked with
//        `INFO FOR`, a SurrealQL statement the server already runs, so nothing is
//        translated.
// HOW:   Every entry is `ShellCommand::native`: the adapter sends the script as typed
//        (src-tauri/src/integrations/surrealdb.rs, `execute` posts it to `/sql`).
//        Only `help` is recognised, and only as the whole statement.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: None,
    strict: false,
    help: &["help"],
    cut: Cut::Statements { line_words: &["help"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help", "List the structure statements SurrealDB answers"),
    ShellCommand::native("INFO FOR ROOT", "INFO FOR ROOT", "Namespaces, users and the server's root definitions"),
    ShellCommand::native("INFO FOR NS", "INFO FOR NS", "Databases, users and accesses of the current namespace"),
    ShellCommand::native("INFO FOR DB", "INFO FOR DB", "Tables, functions, params, users and analyzers of the current database"),
    ShellCommand::native("INFO FOR TABLE", "INFO FOR TABLE table", "Fields, indexes, events and live queries of a table"),
    ShellCommand::native("INFO FOR USER", "INFO FOR USER name ON DATABASE", "One user's definition"),
    ShellCommand::native("SELECT * FROM table", "SELECT * FROM table LIMIT 10", "Records of a table"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    // Only the bare word: `help` followed by anything else is not a command.
    (rest.is_empty() && eq_ci(head, "help")).then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment, Step};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Surrealdb), text)
    }

    #[test]
    fn help_lists_every_command_in_any_case_with_or_without_a_terminator() {
        for text in ["help", "HELP", "Help;"] {
            let Some(Ok(plan)) = planned(text) else { panic!("{text} is not understood") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn native_statements_are_left_for_the_database() {
        for text in ["INFO FOR ROOT", "INFO FOR NS", "INFO FOR DB", "INFO FOR TABLE", "help me", "helper"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn help_is_cut_at_its_line() {
        let found = segments(Engine::Surrealdb, "help\nINFO FOR DB");
        assert_eq!(found, vec![Segment::Meta("help".to_string()), Segment::Native("INFO FOR DB".to_string())]);
    }
}
