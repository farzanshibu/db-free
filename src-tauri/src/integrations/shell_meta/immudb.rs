// SOT: shell-meta-immudb, immudb-structure-statements

use super::args::{eq_ci, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// IMMUDB
//
// WHAT:  `help`: a listing of the statements immudb answers about its own structure
//        (the `TABLES()`, `COLUMNS()`, `INDEXES()`, `DATABASES()` and `USERS()` functions).
// WHY:   `immuclient` has no structure commands for the SQL engine, and the catalog is
//        already SQL, so nothing is translated; the listing makes the native
//        statements findable.
// HOW:   Every entry is `ShellCommand::native`: the adapter sends SQL as typed
//        (src-tauri/src/integrations/immudb.rs, `execute`: `parse_kv` for the
//        key-value verbs, then `split_sql` and `is_read_sql` for SQL). Only `help`
//        is recognised, and only as the whole statement.
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
    ShellCommand::shell("help", "help", "List the structure statements immudb answers"),
    ShellCommand::native("SELECT * FROM TABLES()", "SELECT * FROM TABLES()", "Tables of the current database"),
    ShellCommand::native("SELECT * FROM COLUMNS()", "SELECT * FROM COLUMNS('table')", "Columns of a table, with type, length and nullability"),
    ShellCommand::native("SELECT * FROM INDEXES()", "SELECT * FROM INDEXES('table')", "Indexes of a table"),
    ShellCommand::native("SELECT * FROM DATABASES()", "SELECT * FROM DATABASES()", "Databases of the server"),
    ShellCommand::native("SELECT * FROM USERS()", "SELECT * FROM USERS()", "Users and their permissions"),
    ShellCommand::native("SCAN", "SCAN [prefix]", "Key-value entries whose key starts with a prefix"),
    ShellCommand::native("HISTORY", "HISTORY key", "Every revision of a key"),
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
        run_plan(&Ctx::bare(Engine::Immudb), text)
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
        for text in ["SELECT * FROM TABLES()", "SELECT * FROM COLUMNS()", "SELECT * FROM INDEXES()", "SELECT * FROM DATABASES()", "help me", "helper"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn help_is_cut_at_its_line() {
        let found = segments(Engine::Immudb, "help\nSELECT 1");
        assert_eq!(found, vec![Segment::Meta("help".to_string()), Segment::Native("SELECT 1".to_string())]);
    }
}
