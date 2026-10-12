// SOT: shell-meta-bigquery, bq-ls-show, bigquery-information-schema-queries

use super::args::{self, eq_ci, split_word, string_literal, unquote, Quote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand, Step};
use crate::error::{AppError, AppResult};

// ============================================================================
// BIGQUERY (the `bq` tool)
//
// WHAT:  `bq ls` (datasets), `bq ls dataset` (tables), `bq show dataset.table`
//        (the table, its columns, nested fields and options) and `bq show
//        dataset` (the dataset), with the flags that matter to them:
//        `--location` and `--project_id`. Also `help`.
// WHY:   The `bq` command line is how BigQuery users list and describe things;
//        the service itself has only SQL, and answers `bq ls` with a syntax error.
// HOW:   Each command becomes `INFORMATION_SCHEMA` queries, which the adapter
//        runs through `jobs.query` (src-tauri/src/integrations/bigquery.rs,
//        `execute` sends each statement of `split_statements` as standard SQL).
//        Datasets are listed from `region-<location>.INFORMATION_SCHEMA.SCHEMATA`
//        because BigQuery only exposes that view per region: the default is `us`,
//        and `--location=eu` (or `europe-west1`…) asks another. Tables and
//        columns come from the dataset's own INFORMATION_SCHEMA, so they need no
//        region. Identifiers are backtick-quoted and text literals are quoted,
//        both with the same escapes (backslash, and the quote that delimits
//        them: a quoted identifier reads the escapes of a string literal, so a
//        backslash must be doubled or it would swallow the closing backtick).
//        A reference is `dataset.table`,
//        `project:dataset.table` or `project.dataset.table`.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: None,
    strict: false,
    help: &["help"],
    cut: Cut::Statements { line_words: &["bq", "help"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("bq ls", "bq [--location=REGION] [--project_id=PROJECT] ls [[project:]dataset]", "List the datasets of a region, or the tables of a dataset"),
    ShellCommand::shell("bq show", "bq [--location=REGION] [--project_id=PROJECT] show [project:]dataset[.table]", "Describe a dataset, or a table: its columns, nested fields and options"),
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::native("SELECT * FROM region-us.INFORMATION_SCHEMA.SCHEMATA", "SELECT * FROM `region-us`.INFORMATION_SCHEMA.SCHEMATA", "Datasets of a region"),
    ShellCommand::native("SELECT * FROM dataset.INFORMATION_SCHEMA.TABLES", "SELECT * FROM `dataset`.INFORMATION_SCHEMA.TABLES", "Tables and views of a dataset"),
    ShellCommand::native("SELECT * FROM dataset.INFORMATION_SCHEMA.COLUMNS", "SELECT * FROM `dataset`.INFORMATION_SCHEMA.COLUMNS WHERE table_name = 't'", "Columns of the tables of a dataset"),
    ShellCommand::native("SELECT * FROM dataset.INFORMATION_SCHEMA.COLUMN_FIELD_PATHS", "SELECT * FROM `dataset`.INFORMATION_SCHEMA.COLUMN_FIELD_PATHS WHERE table_name = 't'", "Columns with their nested fields"),
    ShellCommand::native("SELECT * FROM dataset.INFORMATION_SCHEMA.ROUTINES", "SELECT * FROM `dataset`.INFORMATION_SCHEMA.ROUTINES", "Functions and procedures of a dataset"),
    ShellCommand::native("SELECT * FROM dataset.INFORMATION_SCHEMA.PARTITIONS", "SELECT * FROM `dataset`.INFORMATION_SCHEMA.PARTITIONS WHERE table_name = 't'", "Partitions of a table"),
];

fn lit(raw: &str) -> String {
    string_literal(raw, Quote::BackslashHex)
}

// WHAT:  `raw` as a backtick-quoted identifier.
// WHY:   GoogleSQL gives quoted identifiers the escape sequences of string literals, so `\`
//        must be doubled as well as the backtick: `integrations::quote_ident_for` only
//        escapes the backtick, and a name ending in `\` (or holding `\``) would close the
//        identifier early. The backtick is written as the hex escape `\x60`, so no quote
//        character sits inside the name for the statement splitter (which does not read
//        escapes) to mistake for its end. Control characters are dropped, as in `args::string_literal`.
fn ident(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('`');
    for c in raw.chars().filter(|c| !c.is_control()) {
        match c {
            '`' => out.push_str("\\x60"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('`');
    out
}

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    if eq_ci(head, "help") && rest.is_empty() {
        return Some(Ok(help_plan(&DIALECT)));
    }
    eq_ci(head, "bq").then(|| bq(rest))
}

/// What `bq` was asked: flags that matter here, then the subcommand and its arguments.
struct Request {
    location: String,
    project: Option<String>,
    words: Vec<String>,
}

fn parse(rest: &str) -> AppResult<Request> {
    let tokens = args::raw_args(rest).map_err(AppError::invalid_input)?;
    let mut location = None;
    let mut project = None;
    let mut words = Vec::new();
    let mut tokens = tokens.into_iter().map(|t| unquote(&t)).peekable();
    while let Some(token) = tokens.next() {
        let Some(flag) = token.strip_prefix("--").or_else(|| token.strip_prefix('-')).filter(|_| token.starts_with('-')) else {
            words.push(token);
            continue;
        };
        let (name, inline) = match flag.split_once('=') {
            Some((name, value)) => (name.to_string(), Some(unquote(value))),
            None => (flag.to_string(), None),
        };
        let value = match inline {
            Some(value) => value,
            None => tokens.next_if(|next| !next.starts_with('-')).ok_or_else(|| AppError::invalid_input(format!("--{name} needs a value.")))?,
        };
        match name.as_str() {
            "location" => location = Some(value),
            "project_id" => project = Some(value),
            other => return Err(AppError::invalid_input(format!("bq --{other} is not supported here. DB Free reads --location and --project_id."))),
        }
    }
    let location = location.unwrap_or_else(|| "us".to_string());
    if location.is_empty() || !location.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(AppError::invalid_input("--location is a region such as us, eu or europe-west1."));
    }
    Ok(Request { location: location.to_ascii_lowercase(), project, words })
}

fn bq(rest: &str) -> AppResult<Plan> {
    let request = parse(rest)?;
    let Some((sub, args)) = request.words.split_first() else {
        return Err(AppError::invalid_input("bq what? DB Free reads `bq ls [dataset]` and `bq show dataset[.table]`."));
    };
    match sub.to_ascii_lowercase().as_str() {
        "ls" => list(&request, args),
        "show" => show(&request, args),
        other => Err(AppError::invalid_input(format!("bq {other} is not supported here. DB Free reads `bq ls [dataset]` and `bq show dataset[.table]`; type SQL to query."))),
    }
}

/// A reference as `bq` spells it, in pieces.
struct Reference {
    project: Option<String>,
    dataset: String,
    table: Option<String>,
}

fn reference(raw: &str, default_project: Option<&str>) -> AppResult<Reference> {
    let (project, path) = match raw.split_once(':') {
        Some((project, path)) => (Some(project.to_string()), path),
        None => (None, raw),
    };
    let pieces: Vec<&str> = path.split('.').collect();
    if pieces.iter().any(|p| p.is_empty()) || project.as_deref() == Some("") {
        return Err(AppError::invalid_input(format!("\"{raw}\" is not a dataset or table reference.")));
    }
    let (project, dataset, table) = match (project, pieces.as_slice()) {
        (project, [dataset]) => (project, (*dataset).to_string(), None),
        (project, [dataset, table]) => (project, (*dataset).to_string(), Some((*table).to_string())),
        (None, [project, dataset, table]) => (Some((*project).to_string()), (*dataset).to_string(), Some((*table).to_string())),
        _ => return Err(AppError::invalid_input(format!("\"{raw}\" is not a dataset or table reference; use [project:]dataset[.table]."))),
    };
    Ok(Reference { project: project.or_else(|| default_project.map(str::to_string)), dataset, table })
}

/// `` `project`.`dataset`.INFORMATION_SCHEMA.<view> ``, or without the project when none was named.
fn dataset_view(reference: &Reference, view: &str) -> String {
    match &reference.project {
        Some(project) => format!("{}.{}.INFORMATION_SCHEMA.{view}", ident(project), ident(&reference.dataset)),
        None => format!("{}.INFORMATION_SCHEMA.{view}", ident(&reference.dataset)),
    }
}

/// `` `project`.`region-us`.INFORMATION_SCHEMA.SCHEMATA ``, without the project when none is known.
fn region_schemata(location: &str, project: Option<&str>) -> String {
    let region = ident(&format!("region-{location}"));
    match project {
        Some(project) => format!("{}.{region}.INFORMATION_SCHEMA.SCHEMATA", ident(project)),
        None => format!("{region}.INFORMATION_SCHEMA.SCHEMATA"),
    }
}

fn list(request: &Request, args: &[String]) -> AppResult<Plan> {
    match args {
        [] => Ok(Plan::native(format!(
            "SELECT schema_name AS dataset_id, location, creation_time, last_modified_time FROM {} ORDER BY schema_name",
            region_schemata(&request.location, request.project.as_deref())
        ))),
        [dataset] => {
            let target = reference(dataset, request.project.as_deref())?;
            if target.table.is_some() {
                return Err(AppError::invalid_input("bq ls takes a dataset; use bq show dataset.table to describe a table."));
            }
            Ok(Plan::native(format!(
                "SELECT table_name, table_type, creation_time FROM {} ORDER BY table_name",
                dataset_view(&target, "TABLES")
            )))
        }
        _ => Err(AppError::invalid_input("bq ls takes at most one dataset.")),
    }
}

fn show(request: &Request, args: &[String]) -> AppResult<Plan> {
    let [raw] = args else {
        return Err(AppError::invalid_input("bq show takes one dataset or table, e.g. bq show mydataset.mytable."));
    };
    let target = reference(raw, request.project.as_deref())?;
    let Some(table) = &target.table else {
        // A dataset: its row in the region's SCHEMATA view, of the project the reference names.
        let statement = format!(
            "SELECT catalog_name, schema_name, schema_owner, location, creation_time, last_modified_time, ddl FROM {} WHERE schema_name = {}",
            region_schemata(&request.location, target.project.as_deref()),
            lit(&target.dataset)
        );
        let missing = format!("Not found: Dataset {} in region-{} (use --location for another region).", target.dataset, request.location);
        return Ok(Plan { steps: vec![Step::Exists(statement, missing)] });
    };
    let name = lit(table);
    let header = format!(
        "SELECT table_catalog, table_schema, table_name, table_type, creation_time, ddl FROM {} WHERE table_name = {name}",
        dataset_view(&target, "TABLES")
    );
    let columns = format!(
        "SELECT column_name, data_type, is_nullable, is_partitioning_column, clustering_ordinal_position, ordinal_position FROM {} \
         WHERE table_name = {name} ORDER BY ordinal_position",
        dataset_view(&target, "COLUMNS")
    );
    let nested = format!(
        "SELECT field_path, data_type, description FROM {} WHERE table_name = {name} AND STRPOS(field_path, '.') > 0 ORDER BY field_path",
        dataset_view(&target, "COLUMN_FIELD_PATHS")
    );
    let options = format!(
        "SELECT option_name, option_type, option_value FROM {} WHERE table_name = {name} ORDER BY option_name",
        dataset_view(&target, "TABLE_OPTIONS")
    );
    let missing = format!("Not found: Table {}.{}", target.dataset, table);
    Ok(Plan { steps: vec![Step::Exists(header, missing), Step::Section(columns), Step::Section(nested), Step::Section(options)] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::destructive::{classify, StatementKind};
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Bigquery), text)
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
    fn every_command_is_recognised_in_any_case_with_or_without_a_terminator() {
        for text in ["bq ls", "BQ LS;", "bq ls mydata", "bq show mydata.t", "bq show p:mydata.t", "bq show mydata", "bq --location=eu ls", "bq ls --location eu", "help", "HELP;"] {
            assert!(matches!(planned(text), Some(Ok(_))), "{text}: {:?}", planned(text));
        }
    }

    #[test]
    fn sql_is_not_intercepted() {
        for text in ["SELECT 1", "SELECT * FROM `region-us`.INFORMATION_SCHEMA.SCHEMATA", "select * from ds.INFORMATION_SCHEMA.TABLES", "bqx ls", "help me"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn ls_lists_datasets_of_a_region() {
        let sql = statements("bq ls");
        assert_eq!(sql.len(), 1);
        assert!(sql[0].contains("FROM `region-us`.INFORMATION_SCHEMA.SCHEMATA ORDER BY schema_name"), "{}", sql[0]);
        assert!(statements("bq --location=EU ls")[0].contains("`region-eu`.INFORMATION_SCHEMA.SCHEMATA"));
        assert!(statements("bq ls --location europe-west1")[0].contains("`region-europe-west1`"));
        assert!(statements("bq --project_id=my-proj ls")[0].contains("`my-proj`.`region-us`.INFORMATION_SCHEMA.SCHEMATA"));
    }

    #[test]
    fn ls_of_a_dataset_lists_its_tables() {
        let sql = statements("bq ls sales");
        assert_eq!(sql[0], "SELECT table_name, table_type, creation_time FROM `sales`.INFORMATION_SCHEMA.TABLES ORDER BY table_name");
        assert!(statements("bq ls my-proj:sales")[0].contains("FROM `my-proj`.`sales`.INFORMATION_SCHEMA.TABLES"));
        assert!(error("bq ls sales.t").contains("takes a dataset"));
        assert!(error("bq ls a b").contains("at most one"));
    }

    #[test]
    fn show_describes_a_table_from_its_datasets_information_schema() {
        let Some(Ok(plan)) = planned("bq show sales.orders") else { panic!("not planned") };
        assert_eq!(plan.steps.len(), 4);
        assert!(matches!(&plan.steps[0], Step::Exists(sql, message)
            if sql.contains("FROM `sales`.INFORMATION_SCHEMA.TABLES WHERE table_name = 'orders'") && message == "Not found: Table sales.orders"));
        let sql: Vec<&str> = plan.statements().collect();
        assert!(sql[1].contains("FROM `sales`.INFORMATION_SCHEMA.COLUMNS WHERE table_name = 'orders' ORDER BY ordinal_position"));
        assert!(sql[2].contains("COLUMN_FIELD_PATHS") && sql[2].contains("STRPOS(field_path, '.') > 0"));
        assert!(sql[3].contains("TABLE_OPTIONS"));
        assert!(plan.steps[1..].iter().all(|s| matches!(s, Step::Section(_))));
        assert!(statements("bq show my-proj:sales.orders")[0].contains("`my-proj`.`sales`.INFORMATION_SCHEMA.TABLES"));
        assert!(statements("bq show my-proj.sales.orders")[0].contains("`my-proj`.`sales`.INFORMATION_SCHEMA.TABLES"));
    }

    #[test]
    fn show_of_a_dataset_reads_its_row_in_the_region() {
        let Some(Ok(plan)) = planned("bq show --location=eu sales") else { panic!("not planned") };
        assert!(matches!(&plan.steps[..], [Step::Exists(sql, message)]
            if sql.contains("FROM `region-eu`.INFORMATION_SCHEMA.SCHEMATA WHERE schema_name = 'sales'") && message.contains("region-eu")));
    }

    #[test]
    fn bad_arguments_say_how_to_fix_them() {
        assert!(error("bq").contains("bq what?"));
        assert!(error("bq query 'select 1'").contains("not supported"));
        assert!(error("bq show").contains("one dataset or table"));
        assert!(error("bq show a.b.c.d").contains("not a dataset or table reference"));
        assert!(error("bq show a..b").contains("not a dataset or table reference"));
        assert!(error("bq show :a.b").contains("not a dataset or table reference"));
        assert!(error("bq --format=json ls").contains("--format is not supported"));
        assert!(error("bq ls --location").contains("--location needs a value"));
        assert!(error("bq --location=us;drop ls").contains("region"));
    }

    #[test]
    fn user_text_stays_inside_a_quoted_identifier_or_literal() {
        let sql = statements("bq show 'ds.t'' OR 1=1 --'");
        assert!(sql[0].contains("table_name = 't\\x27 OR 1=1 --'"), "{}", sql[0]);
        let ticks = statements("bq ls \"da`ta\"");
        assert!(ticks[0].contains("`da\\x60ta`.INFORMATION_SCHEMA.TABLES"), "a backtick cannot end the quoted name: {}", ticks[0]);
        let backslash = statements("bq show 'ds.a\\'");
        assert!(backslash[0].contains("table_name = 'a\\\\'"), "{}", backslash[0]);
    }

    // Quoted identifiers read the escapes of a string literal: a backslash that is not doubled
    // would turn the closing backtick into an escaped one, or `\\`` into "backslash, then close".
    #[test]
    fn a_backslash_cannot_close_or_reopen_a_quoted_identifier() {
        assert_eq!(statements("bq ls 'a\\'")[0], "SELECT table_name, table_type, creation_time FROM `a\\\\`.INFORMATION_SCHEMA.TABLES ORDER BY table_name");
        let hostile = statements("bq ls 'a\\`; DROP SCHEMA x; --'");
        assert!(hostile[0].contains("FROM `a\\\\\\x60; DROP SCHEMA x; --`.INFORMATION_SCHEMA.TABLES"), "{}", hostile[0]);
        assert!(statements("bq --project_id='p\\' ls")[0].contains("`p\\\\`.`region-us`.INFORMATION_SCHEMA.SCHEMATA"));
        assert_eq!(ident("a\nb\0c"), "`abc`", "control characters are dropped");
        assert!(statements("bq --project_id=\"my-proj\" ls")[0].contains("`my-proj`.`region-us`"), "a quoted flag value loses its quotes");
        assert!(error("bq --location=\"e u\" ls").contains("region"), "{}", error("bq --location=\"e u\" ls"));
    }

    #[test]
    fn a_dataset_reference_names_its_own_project() {
        let named = statements("bq show other:sales");
        assert!(named[0].contains("FROM `other`.`region-us`.INFORMATION_SCHEMA.SCHEMATA WHERE schema_name = 'sales'"), "{}", named[0]);
        let flag = statements("bq --project_id=mine show sales");
        assert!(flag[0].contains("FROM `mine`.`region-us`.INFORMATION_SCHEMA.SCHEMATA WHERE schema_name = 'sales'"), "{}", flag[0]);
        let both = statements("bq --project_id=mine --location=eu show other:sales");
        assert!(both[0].contains("FROM `other`.`region-eu`.INFORMATION_SCHEMA.SCHEMATA"), "the reference wins over the flag: {}", both[0]);
    }

    #[test]
    fn every_statement_is_a_read() {
        for text in ["bq ls", "bq ls sales", "bq show sales.orders", "bq show sales", "bq --location=eu show p:sales.t"] {
            for sql in statements(text) {
                let kinds: Vec<StatementKind> = classify(&sql).into_iter().map(|s| s.kind).collect();
                assert_eq!(kinds, vec![StatementKind::Read], "{text}: {sql}");
            }
        }
    }

    #[test]
    fn bq_commands_are_cut_at_their_line() {
        let found = segments(Engine::Bigquery, "bq ls\nSELECT 1;\nbq show d.t");
        assert_eq!(
            found,
            vec![Segment::Meta("bq ls".to_string()), Segment::Native("SELECT 1".to_string()), Segment::Meta("bq show d.t".to_string())]
        );
    }

    #[test]
    fn help_lists_every_command() {
        let Some(Ok(plan)) = planned("help") else { panic!("no help") };
        assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()));
    }
}
