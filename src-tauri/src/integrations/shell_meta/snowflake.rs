// SOT: shell-meta-snowflake, snowsql-help, snowflake-structure-statements

use super::args::{eq_ci, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// SNOWFLAKE
//
// WHAT:  `help` and SnowSQL's `!help`: a listing of the statements Snowflake
//        answers about its own structure (`SHOW …`, `DESCRIBE …`, `GET_DDL`,
//        `INFORMATION_SCHEMA`).
// WHY:   SnowSQL's `!` commands (`!source`, `!set`, `!print`, `!queries`…) run
//        scripts and manage the client; none of them lists objects. What a
//        Snowflake user types for structure is `SHOW TABLES`, which the SQL API
//        already runs, so nothing is translated.
// HOW:   Every structure entry is `ShellCommand::native`: the adapter sends the
//        statement as typed (src-tauri/src/integrations/snowflake.rs, `execute`
//        trims one trailing `;` and posts the text to the SQL API). Only `help`
//        and `!help` are recognised, and only as the whole statement.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: None,
    strict: false,
    help: &["help", "!help"],
    cut: Cut::Statements { line_words: &["help", "!help"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | !help", "List the structure statements Snowflake answers"),
    ShellCommand::shell("!help", "!help", "List the structure statements Snowflake answers (SnowSQL's own !help lists client commands instead)"),
    ShellCommand::native("SHOW DATABASES", "SHOW DATABASES [LIKE '%pattern%']", "List databases"),
    ShellCommand::native("SHOW SCHEMAS", "SHOW SCHEMAS [IN DATABASE db]", "List schemas"),
    ShellCommand::native("SHOW TABLES", "SHOW TABLES [LIKE '%pattern%'] [IN schema]", "List tables"),
    ShellCommand::native("SHOW VIEWS", "SHOW VIEWS [IN schema]", "List views"),
    ShellCommand::native("SHOW COLUMNS", "SHOW COLUMNS IN TABLE table", "List the columns of a table"),
    ShellCommand::native("SHOW PRIMARY KEYS", "SHOW PRIMARY KEYS IN TABLE table", "Primary key of a table"),
    ShellCommand::native("SHOW OBJECTS", "SHOW OBJECTS [IN schema]", "Tables, views and other objects of a schema"),
    ShellCommand::native("DESCRIBE TABLE", "DESCRIBE TABLE table", "Columns, types and defaults of a table"),
    ShellCommand::native("SELECT GET_DDL", "SELECT GET_DDL('TABLE', 'table')", "The statement that creates an object"),
    ShellCommand::native("SHOW WAREHOUSES", "SHOW WAREHOUSES", "List warehouses"),
    ShellCommand::native("SHOW STAGES", "SHOW STAGES [IN schema]", "List stages"),
    ShellCommand::native("SHOW GRANTS", "SHOW GRANTS [TO ROLE role]", "List privileges"),
    ShellCommand::native("SHOW ROLES", "SHOW ROLES", "List roles"),
    ShellCommand::native("SHOW USERS", "SHOW USERS", "List users"),
    ShellCommand::native("SELECT * FROM INFORMATION_SCHEMA.TABLES", "SELECT * FROM INFORMATION_SCHEMA.TABLES", "Tables and views of the current database"),
    ShellCommand::native("SELECT * FROM INFORMATION_SCHEMA.COLUMNS", "SELECT * FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'T'", "Columns of the tables and views"),
    ShellCommand::native("SELECT CURRENT_DATABASE()", "SELECT CURRENT_DATABASE(), CURRENT_SCHEMA(), CURRENT_WAREHOUSE(), CURRENT_ROLE()", "Where this session is"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    (rest.is_empty() && (eq_ci(head, "help") || eq_ci(head, "!help"))).then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment, Step};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Snowflake), text)
    }

    #[test]
    fn help_and_snowsqls_help_list_the_structure_statements() {
        for text in ["help", "HELP;", "!help", "!HELP"] {
            let Some(Ok(plan)) = planned(text) else { panic!("{text} is not understood") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn show_and_describe_are_left_for_snowflake() {
        for text in ["SHOW TABLES", "show databases like '%x%'", "DESCRIBE TABLE t", "SELECT GET_DDL('TABLE','t')", "help me", "!source file.sql", "!set x=1"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn help_commands_are_cut_at_their_line() {
        let found = segments(Engine::Snowflake, "!help\nSHOW TABLES;");
        assert_eq!(found, vec![Segment::Meta("!help".to_string()), Segment::Native("SHOW TABLES".to_string())]);
    }
}
