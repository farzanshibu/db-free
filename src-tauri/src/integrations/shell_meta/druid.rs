// SOT: shell-meta-druid, druid-structure-statements

use super::args::{eq_ci, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// APACHE DRUID
//
// WHAT:  `help`: a listing of the statements Druid answers about its own structure
//        (INFORMATION_SCHEMA, the `sys` tables, the coordinator's REST paths).
// WHY:   Druid has no shell with structure commands (`dsql` only runs SQL), and its
//        catalog is already plain SQL, so nothing is translated; the listing makes
//        the native statements findable.
// HOW:   Every entry is `ShellCommand::native`: the adapter sends SQL as typed, and a
//        `{"method", "path"}` object as a REST call to the router
//        (src-tauri/src/integrations/druid.rs, `execute`: `parse_rest_command`,
//        then `split_statements`). Only `help` is recognised, and only as the
//        whole statement.
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
    ShellCommand::shell("help", "help", "List the structure statements Druid answers"),
    ShellCommand::native("SELECT * FROM INFORMATION_SCHEMA.SCHEMATA", "SELECT * FROM INFORMATION_SCHEMA.SCHEMATA", "Schemas: druid, sys, INFORMATION_SCHEMA, lookup, view"),
    ShellCommand::native("SELECT * FROM INFORMATION_SCHEMA.TABLES", "SELECT * FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = 'druid'", "Datasources and system tables"),
    ShellCommand::native("SELECT * FROM INFORMATION_SCHEMA.COLUMNS", "SELECT * FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'datasource'", "Columns and types of a datasource"),
    ShellCommand::native("SELECT * FROM sys.segments", "SELECT * FROM sys.segments WHERE datasource = 'name'", "Segments with their size, rows and partitioning"),
    ShellCommand::native("SELECT * FROM sys.servers", "SELECT * FROM sys.servers", "Cluster servers and their roles"),
    ShellCommand::native("SELECT * FROM sys.server_segments", "SELECT * FROM sys.server_segments", "Which server holds which segment"),
    ShellCommand::native("SELECT * FROM sys.tasks", "SELECT * FROM sys.tasks", "Ingestion and maintenance tasks"),
    ShellCommand::native("SELECT * FROM sys.supervisors", "SELECT * FROM sys.supervisors", "Streaming ingestion supervisors"),
    ShellCommand::native("{\"method\": \"GET\", \"path\": \"/druid/coordinator/v1/datasources\"}", "{\"method\": \"GET\", \"path\": \"/druid/coordinator/v1/datasources\"}", "Datasource names, from the coordinator's REST API"),
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
        run_plan(&Ctx::bare(Engine::Druid), text)
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
        for text in ["SELECT * FROM INFORMATION_SCHEMA.SCHEMATA", "SELECT * FROM INFORMATION_SCHEMA.TABLES", "SELECT * FROM INFORMATION_SCHEMA.COLUMNS", "SELECT * FROM sys.segments", "help me", "helper"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn help_is_cut_at_its_line() {
        let found = segments(Engine::Druid, "help\nSELECT 1");
        assert_eq!(found, vec![Segment::Meta("help".to_string()), Segment::Native("SELECT 1".to_string())]);
    }
}
