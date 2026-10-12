// SOT: shell-meta-dynamodb, aws-cli-dynamodb-structure, dynamodb-console-shorthands

use super::args::{self, eq_ci, split_word, unquote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// AMAZON DYNAMODB (the AWS CLI)
//
// WHAT:  `aws dynamodb list-tables`, `aws dynamodb describe-table --table-name T`,
//        `aws dynamodb describe-limits`, `aws dynamodb describe-time-to-live
//        --table-name T`, and `help`.
// WHY:   DynamoDB has no shell of its own: people list and describe tables with
//        the AWS CLI, whose commands are the nearest thing to meta-commands.
//        PartiQL cannot ask these questions.
// HOW:   Each command becomes what the adapter's console already understands
//        (src-tauri/src/integrations/dynamodb.rs, `parse_command`): `TABLES` and
//        `DESCRIBE <table>` shorthands, and `{"Operation", "Params"}` JSON for the
//        two operations without one. All four are read operations the adapter
//        lets through on a read-only connection (`is_write_op`). A table name is
//        checked against DynamoDB's own rule (letters, digits, `_`, `-`, `.`),
//        and the JSON is built with serde, so nothing the user types can change
//        the command. Flags other than `--table-name` are refused rather than
//        ignored; `--region` and `--endpoint-url` belong to the connection.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: None,
    strict: false,
    help: &["help"],
    cut: Cut::Statements { line_words: &["aws", "help"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("aws dynamodb list-tables", "aws dynamodb list-tables", "List the tables of the region"),
    ShellCommand::shell("aws dynamodb describe-table", "aws dynamodb describe-table --table-name TABLE", "Key schema, attributes, indexes, throughput and size of a table"),
    ShellCommand::shell("aws dynamodb describe-limits", "aws dynamodb describe-limits", "Account and table throughput limits of the region"),
    ShellCommand::shell("aws dynamodb describe-time-to-live", "aws dynamodb describe-time-to-live --table-name TABLE", "The time-to-live setting of a table"),
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::native("TABLES", "TABLES", "List the tables of the region"),
    ShellCommand::native("DESCRIBE", "DESCRIBE table", "Describe a table"),
    ShellCommand::native("SELECT * FROM table", "SELECT * FROM \"table\" [WHERE ...]", "Items of a table, in PartiQL"),
    ShellCommand::native("{\"Operation\": \"Scan\"}", "{\"Operation\": \"Scan\", \"Params\": {\"TableName\": \"table\", \"Limit\": 25}}", "Any read operation of the DynamoDB API, as JSON"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    if eq_ci(head, "help") && rest.is_empty() {
        return Some(Ok(help_plan(&DIALECT)));
    }
    eq_ci(head, "aws").then(|| aws(rest))
}

fn aws(rest: &str) -> AppResult<Plan> {
    let words = args::raw_args(rest).map_err(AppError::invalid_input)?;
    let words: Vec<String> = words.iter().map(|w| unquote(w)).collect();
    match words.split_first() {
        Some((service, tail)) if service.eq_ignore_ascii_case("dynamodb") => dynamodb(tail),
        _ => Err(AppError::invalid_input("Only `aws dynamodb ...` commands are read here, e.g. aws dynamodb list-tables.")),
    }
}

const SUPPORTED: &str = "list-tables, describe-table, describe-limits and describe-time-to-live";

fn dynamodb(words: &[String]) -> AppResult<Plan> {
    let Some((operation, flags)) = words.split_first() else {
        return Err(AppError::invalid_input(format!("aws dynamodb what? DB Free reads {SUPPORTED}.")));
    };
    match operation.to_ascii_lowercase().as_str() {
        "list-tables" => no_flags(flags, operation).map(|()| Plan::native("TABLES")),
        "describe-limits" => no_flags(flags, operation).map(|()| Plan::native(operation_json("DescribeLimits", None))),
        "describe-table" => {
            let table = table_name(flags, operation)?;
            Ok(Plan::native(format!("DESCRIBE {table}")))
        }
        "describe-time-to-live" => {
            let table = table_name(flags, operation)?;
            Ok(Plan::native(operation_json("DescribeTimeToLive", Some(&table))))
        }
        other => Err(AppError::invalid_input(format!("aws dynamodb {other} is not supported here. DB Free reads {SUPPORTED}; query items with PartiQL."))),
    }
}

fn no_flags(flags: &[String], operation: &str) -> AppResult<()> {
    match flags.first() {
        None => Ok(()),
        Some(flag) => Err(AppError::invalid_input(format!("aws dynamodb {operation} takes no arguments here; found {flag}."))),
    }
}

// WHAT:  The value of `--table-name`, in either `--table-name T` or `--table-name=T` form.
fn table_name(flags: &[String], operation: &str) -> AppResult<String> {
    let usage = || AppError::invalid_input(format!("aws dynamodb {operation} needs --table-name TABLE."));
    let (first, more) = flags.split_first().ok_or_else(usage)?;
    let (flag, value) = match first.split_once('=') {
        Some((flag, value)) => (flag, Some(value.to_string())),
        None => (first.as_str(), None),
    };
    if flag != "--table-name" {
        return Err(AppError::invalid_input(format!("aws dynamodb {operation} reads --table-name only; found {flag}.")));
    }
    let (name, rest) = match value {
        Some(value) => (value, more),
        None => {
            let (name, rest) = more.split_first().ok_or_else(usage)?;
            (name.clone(), rest)
        }
    };
    if let Some(extra) = rest.first() {
        return Err(AppError::invalid_input(format!("aws dynamodb {operation} reads --table-name only; found {extra}.")));
    }
    valid_table(&name)?;
    Ok(name)
}

// WHAT:  DynamoDB's naming rule: 3 to 255 characters of letters, digits, `_`, `-` and `.`.
fn valid_table(name: &str) -> AppResult<()> {
    let len = name.chars().count();
    if (3..=255).contains(&len) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        Ok(())
    } else {
        Err(AppError::invalid_input(format!(
            "\"{name}\" is not a DynamoDB table name: 3 to 255 characters, letters, digits, underscore, hyphen and dot."
        )))
    }
}

// WHAT:  The adapter's JSON console form of an operation, written by serde so no text can break out of it.
fn operation_json(operation: &str, table: Option<&str>) -> String {
    let params = match table {
        Some(table) => serde_json::json!({ "TableName": table }),
        None => serde_json::json!({}),
    };
    serde_json::json!({ "Operation": operation, "Params": params }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment, Step};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Dynamodb), text)
    }

    fn statements(text: &str) -> Vec<String> {
        match planned(text) {
            Some(Ok(plan)) => plan.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn error(text: &str) -> String {
        match planned(text) {
            Some(Err(err)) => err.message().to_string(),
            other => panic!("{text}: expected an error, got {other:?}"),
        }
    }

    #[test]
    fn the_cli_commands_become_the_consoles_own_forms() {
        assert_eq!(statements("aws dynamodb list-tables"), ["TABLES"]);
        assert_eq!(statements("AWS DynamoDB List-Tables;"), ["TABLES"]);
        assert_eq!(statements("aws dynamodb describe-table --table-name Orders"), ["DESCRIBE Orders"]);
        assert_eq!(statements("aws dynamodb describe-table --table-name=Orders"), ["DESCRIBE Orders"]);
        assert_eq!(statements("aws dynamodb describe-table --table-name \"Orders.v2\""), ["DESCRIBE Orders.v2"]);
        assert_eq!(statements("aws dynamodb describe-limits"), ["{\"Operation\":\"DescribeLimits\",\"Params\":{}}"]);
        assert_eq!(
            statements("aws dynamodb describe-time-to-live --table-name Orders"),
            ["{\"Operation\":\"DescribeTimeToLive\",\"Params\":{\"TableName\":\"Orders\"}}"]
        );
    }

    #[test]
    fn the_generated_json_is_what_the_adapter_parses() {
        for text in ["aws dynamodb describe-limits", "aws dynamodb describe-time-to-live --table-name Orders"] {
            let sql = statements(text).remove(0);
            let parsed: serde_json::Value = serde_json::from_str(&sql).unwrap_or_default();
            assert!(parsed.get("Operation").and_then(|o| o.as_str()).is_some_and(|o| o.starts_with("Describe")), "{sql}");
            assert!(parsed.get("Params").is_some_and(serde_json::Value::is_object), "{sql}");
        }
    }

    #[test]
    fn bad_arguments_say_how_to_fix_them() {
        assert!(error("aws").contains("Only `aws dynamodb"));
        assert!(error("aws s3 ls").contains("Only `aws dynamodb"));
        assert!(error("aws dynamodb").contains("aws dynamodb what?"));
        assert!(error("aws dynamodb scan --table-name t").contains("not supported"));
        assert!(error("aws dynamodb describe-table").contains("needs --table-name"));
        assert!(error("aws dynamodb describe-table --table-name").contains("needs --table-name"));
        assert!(error("aws dynamodb describe-table --region us-east-1").contains("reads --table-name only"));
        assert!(error("aws dynamodb describe-table --table-name Orders --region x").contains("reads --table-name only"));
        assert!(error("aws dynamodb list-tables --max-items 5").contains("takes no arguments"));
        assert!(error("aws dynamodb describe-table --table-name ab").contains("not a DynamoDB table name"));
    }

    #[test]
    fn a_table_name_cannot_carry_anything_but_a_name() {
        for hostile in ["Orders;DROP", "a b c", "t\\\"x", "a{b}", "Orders\\n", "../etc", "\"x\" OR 1=1"] {
            let text = format!("aws dynamodb describe-table --table-name '{hostile}'");
            assert!(matches!(planned(&text), Some(Err(_))), "{hostile}: {:?}", planned(&text));
            let json = format!("aws dynamodb describe-time-to-live --table-name '{hostile}'");
            assert!(matches!(planned(&json), Some(Err(_))), "{hostile}");
        }
        // Whatever passes validation is also safe inside the JSON string.
        let sql = statements("aws dynamodb describe-time-to-live --table-name my-table_1.v2").remove(0);
        assert!(sql.contains("\"TableName\":\"my-table_1.v2\""));
    }

    #[test]
    fn native_statements_are_left_for_the_adapter() {
        for text in ["TABLES", "DESCRIBE Orders", "SELECT * FROM \"Orders\"", "{\"Operation\": \"Scan\", \"Params\": {}}", "help me", "awsome"] {
            assert!(planned(text).is_none(), "{text}");
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
    fn aws_commands_are_cut_at_their_line() {
        let found = segments(Engine::Dynamodb, "aws dynamodb list-tables\nSELECT * FROM \"Orders\";");
        assert_eq!(found, vec![Segment::Meta("aws dynamodb list-tables".to_string()), Segment::Native("SELECT * FROM \"Orders\"".to_string())]);
    }
}
