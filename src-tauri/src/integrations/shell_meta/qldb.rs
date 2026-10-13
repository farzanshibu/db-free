// SOT: shell-meta-qldb, qldb-shell-show-tables, partiql-user-tables

use super::args::{eq_ci, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// AMAZON QLDB (the QLDB shell)
//
// WHAT:  `SHOW TABLES`, the QLDB shell's own command, and `help`.
// WHY:   The shell answers `SHOW TABLES` itself, by querying the ledger's
//        catalog; PartiQL has no such statement, so the server would reject it.
// HOW:   `SHOW TABLES` becomes the query the shell runs on
//        `information_schema.user_tables`, the only catalog QLDB exposes, sent
//        through the adapter's `execute` (src-tauri/src/integrations/qldb.rs:
//        statements from `split_statements`, run in a read transaction that is
//        aborted afterwards). Only active tables are listed, as the shell does.
//        A table's indexes are in the `indexes` column of that same view.
//        QLDB has no `ORDER BY`, so the rows come in the ledger's order.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: None,
    strict: false,
    help: &["help"],
    cut: Cut::Statements { line_words: &["help", "show"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("SHOW TABLES", "SHOW TABLES", "List the active tables of the ledger, with their ids and indexes"),
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::native("SELECT * FROM information_schema.user_tables", "SELECT * FROM information_schema.user_tables", "Every table of the ledger, active or not, with its indexes"),
    ShellCommand::native("SELECT * FROM history(table)", "SELECT * FROM history(table) AS h WHERE h.metadata.id = 'id'", "Every revision of a document"),
    ShellCommand::native("SELECT * FROM table", "SELECT * FROM table [WHERE ...]", "Current documents of a table"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    if eq_ci(head, "help") && rest.is_empty() {
        return Some(Ok(help_plan(&DIALECT)));
    }
    if !eq_ci(head, "show") {
        return None;
    }
    // `SHOW` is not PartiQL, so every use is the shell's: say what is supported.
    Some(if eq_ci(rest, "tables") {
        Ok(Plan::native("SELECT name, tableId, status, indexes FROM information_schema.user_tables WHERE status = 'ACTIVE'"))
    } else {
        Err(AppError::invalid_input("DB Free reads SHOW TABLES. Query information_schema.user_tables for more."))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment, Step};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Qldb), text)
    }

    #[test]
    fn show_tables_reads_the_active_tables_of_the_catalog() {
        for text in ["SHOW TABLES", "show tables;", "Show   Tables"] {
            let Some(Ok(plan)) = planned(text) else { panic!("{text} is not understood") };
            assert_eq!(
                plan.statements().collect::<Vec<_>>(),
                ["SELECT name, tableId, status, indexes FROM information_schema.user_tables WHERE status = 'ACTIVE'"],
                "{text}"
            );
        }
    }

    #[test]
    fn other_show_forms_are_refused_with_a_pointer() {
        for text in ["SHOW", "SHOW DATABASES", "show tables now"] {
            let Some(Err(err)) = planned(text) else { panic!("{text} should be refused") };
            assert!(err.message().contains("SHOW TABLES"), "{text}: {err}");
        }
    }

    #[test]
    fn help_lists_every_command() {
        for text in ["help", "HELP;"] {
            let Some(Ok(plan)) = planned(text) else { panic!("{text} is not understood") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn partiql_is_left_for_the_ledger() {
        for text in ["SELECT * FROM information_schema.user_tables", "SELECT * FROM history(vehicles)", "SELECT * FROM vehicles", "help me", "showing"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn the_statement_is_a_read_the_adapter_accepts_without_a_terminator() {
        let Some(Ok(plan)) = planned("show tables") else { panic!("not planned") };
        let statement = plan.statements().next().unwrap_or_default();
        assert!(statement.starts_with("SELECT ") && !statement.contains(';'));
        assert_eq!(crate::guard::destructive::classify(statement).len(), 1);
        assert!(crate::guard::destructive::classify(statement).iter().all(|s| s.kind == crate::guard::destructive::StatementKind::Read));
    }

    #[test]
    fn line_commands_are_cut_without_a_semicolon() {
        let found = segments(Engine::Qldb, "SHOW TABLES\nSELECT * FROM vehicles;\nhelp");
        assert_eq!(
            found,
            vec![Segment::Meta("SHOW TABLES".to_string()), Segment::Native("SELECT * FROM vehicles".to_string()), Segment::Meta("help".to_string())]
        );
    }
}
