// SOT: shell-meta-mssql, sqlserver-structure-procedures, sqlcmd-help

use super::args::{eq_ci, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// SQL SERVER
//
// WHAT:  A `help` / `\?` listing of the statements SQL Server answers about its
//        own structure: the `sp_*` system procedures and the INFORMATION_SCHEMA /
//        `sys.*` catalog views.
// WHY:   `sqlcmd` has no structure commands (its colon commands run files and
//        set variables), and SSMS does the browsing in a tree. What a SQL Server
//        user types is `sp_help orders`, which the server already runs, so
//        nothing is translated here: inventing `\dt` aliases would teach syntax
//        SQL Server does not have. The listing makes the native ones findable.
// HOW:   Every entry is `ShellCommand::native`: the adapter sends them to the
//        server as typed (src-tauri/src/integrations/mssql.rs, `execute` runs each
//        statement of `split_statements` with `simple_query`, so `EXEC` of a
//        procedure and a plain `sp_help t` batch both work). Only `help` and `\?`
//        are recognised, and only when they are the whole statement.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: None,
    strict: false,
    help: &["help", "\\?"],
    cut: Cut::Statements { line_words: &["help"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | \\?", "List the structure statements SQL Server answers"),
    ShellCommand::shell("\\?", "\\?", "List the structure statements SQL Server answers"),
    ShellCommand::native("sp_databases", "EXEC sp_databases", "List the databases of the server"),
    ShellCommand::native("sp_helpdb", "EXEC sp_helpdb [database]", "Databases with size and owner, or one database's files"),
    ShellCommand::native("sp_tables", "EXEC sp_tables [@table_name = 'pattern']", "List tables and views the current database exposes"),
    ShellCommand::native("sp_help", "EXEC sp_help [object]", "Columns, indexes and constraints of an object, or every object of the database"),
    ShellCommand::native("sp_columns", "EXEC sp_columns @table_name = 'table'", "Columns of a table or view"),
    ShellCommand::native("sp_helpindex", "EXEC sp_helpindex 'table'", "Indexes of a table"),
    ShellCommand::native("sp_helpconstraint", "EXEC sp_helpconstraint 'table'", "Constraints of a table"),
    ShellCommand::native("sp_helptext", "EXEC sp_helptext 'object'", "The definition of a view, procedure, function or trigger"),
    ShellCommand::native("sp_fkeys", "EXEC sp_fkeys @pktable_name = 'table'", "Foreign keys that reference a table"),
    ShellCommand::native("sp_pkeys", "EXEC sp_pkeys @table_name = 'table'", "Primary key of a table"),
    ShellCommand::native("sp_stored_procedures", "EXEC sp_stored_procedures", "List the stored procedures"),
    ShellCommand::native("sp_spaceused", "EXEC sp_spaceused ['table']", "Space used by the database or a table"),
    ShellCommand::native("sp_who", "EXEC sp_who", "Current sessions"),
    ShellCommand::native("SELECT name FROM sys.databases", "SELECT name, state_desc, recovery_model_desc FROM sys.databases", "Databases of the server"),
    ShellCommand::native("SELECT * FROM INFORMATION_SCHEMA.TABLES", "SELECT * FROM INFORMATION_SCHEMA.TABLES", "Tables and views of the current database"),
    ShellCommand::native("SELECT * FROM INFORMATION_SCHEMA.COLUMNS", "SELECT * FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 't'", "Columns of the tables and views"),
    ShellCommand::native("SELECT * FROM sys.schemas", "SELECT * FROM sys.schemas", "Schemas of the current database"),
    ShellCommand::native("SELECT * FROM sys.objects", "SELECT name, type_desc FROM sys.objects WHERE is_ms_shipped = 0", "Every user object of the current database"),
    ShellCommand::native("SELECT @@VERSION", "SELECT @@VERSION", "Version of the server"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    // Only the bare word: `help` followed by anything else is not a command.
    (rest.is_empty() && (eq_ci(head, "help") || head == "\\?")).then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment, Step};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Mssql), text)
    }

    #[test]
    fn help_and_the_question_mark_list_the_native_procedures() {
        for text in ["help", "HELP", "help;", "\\?", "\\?;"] {
            let Some(Ok(plan)) = planned(text) else { panic!("{text} is not understood") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn the_listing_names_the_procedures_people_type() {
        let Some(Ok(plan)) = planned("help") else { panic!("no help") };
        let Step::Table(table) = &plan.steps[0] else { panic!("not a table") };
        for name in ["sp_help", "sp_columns", "sp_tables", "sp_helpindex", "sp_helpdb", "sp_databases", "sp_spaceused", "sp_who"] {
            assert!(table.rows.iter().any(|row| row[0] == name && row[3] == "run by the database"), "{name}");
        }
    }

    #[test]
    fn native_statements_and_longer_text_are_left_for_the_server() {
        for text in [
            "sp_help orders", "EXEC sp_help 'orders'", "sp_columns @table_name = 'orders'", "SELECT name FROM sys.databases", "SELECT @@VERSION", "help me", "helper",
            "\\dt", "\\? now",
        ] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn help_is_cut_at_its_line() {
        let found = segments(Engine::Mssql, "help\nSELECT 1;");
        assert_eq!(found, vec![Segment::Meta("help".to_string()), Segment::Native("SELECT 1".to_string())]);
    }

    // A backslash line ends at its line without being a line word: the block's splitter cuts it there.
    #[test]
    fn the_question_mark_command_is_cut_at_its_line_too() {
        let found = segments(Engine::Mssql, "\\?\nSELECT 1;");
        assert_eq!(found, vec![Segment::Meta("\\?".to_string()), Segment::Native("SELECT 1".to_string())]);
    }
}
