// SOT: shell-meta-spacetimedb, spacetimedb-structure-statements

use super::args::{eq_ci, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// SPACETIMEDB
//
// WHAT:  `help`: a listing of what the SQL endpoint answers. SpacetimeDB's SQL subset has
//        no catalog to query, so the listing says where the structure is.
// WHY:   The module schema (tables, columns, indexes, reducers) is read by the app over
//        the host's schema endpoint and shown in the sidebar; the `spacetime describe`
//        command of the CLI reads that same endpoint, and SQL cannot.
// HOW:   Only `help` is recognised, as the whole statement. The one native entry is
//        a `SELECT` of a table, which the host runs as typed
//        (src-tauri/src/integrations/spacetimedb.rs, `execute` posts the text to
//        `/v1/database/{db}/sql`).
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
    ShellCommand::shell("help", "help", "List the structure statements SpacetimeDB answers"),
    ShellCommand::native("SELECT * FROM table", "SELECT * FROM table [WHERE ...]", "Rows of a table; the tables and their columns are in the sidebar"),
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
        run_plan(&Ctx::bare(Engine::Spacetimedb), text)
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
        for text in ["SELECT * FROM table", "help me", "helper"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn help_is_cut_at_its_line() {
        let found = segments(Engine::Spacetimedb, "help\nSELECT * FROM t");
        assert_eq!(found, vec![Segment::Meta("help".to_string()), Segment::Native("SELECT * FROM t".to_string())]);
    }
}
