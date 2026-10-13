// SOT: shell-meta-objectdb, objectdb-structure-statements

use super::args::{eq_ci, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// OBJECTDB
//
// WHAT:  `help`: a listing of the JPQL that shows what an entity holds. JPQL has no
//        catalog, so the listing says where the structure is.
// WHY:   ObjectDB's explorer is a GUI; the gateway lists entities and their fields over
//        its own endpoints, which the app shows in the sidebar.
// HOW:   Only `help` is recognised, as the whole statement. The native entries are JPQL
//        the gateway runs as typed (src-tauri/src/integrations/objectdb.rs,
//        `execute` through `parse_execute_input`).
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
    ShellCommand::shell("help", "help", "List the structure statements ObjectDB answers"),
    ShellCommand::native("SELECT e FROM Entity e", "SELECT e FROM Entity e [WHERE ...]", "Objects of an entity; the entities and their fields are in the sidebar"),
    ShellCommand::native("SELECT COUNT(e) FROM Entity e", "SELECT COUNT(e) FROM Entity e", "How many objects an entity holds"),
    ShellCommand::native("SELECT DISTINCT e.field FROM Entity e", "SELECT DISTINCT e.field FROM Entity e", "The values a field takes"),
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
        run_plan(&Ctx::bare(Engine::Objectdb), text)
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
        for text in ["SELECT e FROM Entity e", "SELECT COUNT(e) FROM Entity e", "SELECT DISTINCT e.field FROM Entity e", "help me", "helper"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn help_is_cut_at_its_line() {
        let found = segments(Engine::Objectdb, "help\nSELECT e FROM Entity e");
        assert_eq!(found, vec![Segment::Meta("help".to_string()), Segment::Native("SELECT e FROM Entity e".to_string())]);
    }
}
