// SOT: shell-meta-oracle, sqlplus-describe, sqlplus-show-commands, oracle-structure-commands

use super::args::{self, eq_ci, split_word, string_literal, Part, Quote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand, Step};
use crate::error::{AppError, AppResult};

// ============================================================================
// ORACLE (SQL*Plus)
//
// WHAT:  The structure commands of SQL*Plus: `DESC[RIBE] [schema.]object`,
//        `SHOW USER | CON_NAME | CON_ID | PDBS | PARAMETERS [text] | RELEASE |
//        ERRORS | SGA | RECYCLEBIN`, and `HELP`. `SHOW TABLES` is added as a
//        convenience (SQL*Plus has no such command; SQLcl users expect one).
// WHY:   Oracle people describe an object with `DESC emp`, not with a query on
//        ALL_TAB_COLUMNS; the server answers `DESC` with ORA-00900.
// HOW:   Each command becomes dictionary queries (ALL_OBJECTS, ALL_TAB_COLUMNS,
//        ALL_ARGUMENTS, V$PARAMETER…) that the adapter's `run_script` executes
//        as plain SQL (src-tauri/src/integrations/oracle.rs, `run_script` /
//        `normalize_statement`: one statement, no trailing `;`). Names fold to
//        upper case unless quoted, as in the database; user text only ever
//        appears inside a quoted literal. The current schema is read with
//        SYS_CONTEXT('USERENV','CURRENT_SCHEMA'), so the editor's schema picker
//        (ALTER SESSION SET CURRENT_SCHEMA, prepended to the run) is honoured.
//        SQL*Plus commands end at their line, so the dialect declares `DESC`,
//        `DESCRIBE`, `SHOW` and `HELP` as line words.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: None,
    strict: false,
    help: &["HELP", "?"],
    cut: Cut::Statements { line_words: &["DESC", "DESCRIBE", "SHOW", "HELP", "?"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("DESCRIBE", "DESC[RIBE] [schema.]object", "Columns of a table or view, or arguments of a procedure, function or package"),
    ShellCommand::shell("DESC", "DESC[RIBE] [schema.]object", "Short form of DESCRIBE"),
    ShellCommand::shell("SHOW USER", "SHOW USER", "The connected user and the current schema"),
    ShellCommand::shell("SHOW CON_NAME", "SHOW CON_NAME", "Name of the container (CDB or PDB) the session is in"),
    ShellCommand::shell("SHOW CON_ID", "SHOW CON_ID", "Id of the container the session is in"),
    ShellCommand::shell("SHOW PDBS", "SHOW PDBS", "Pluggable databases of the container database"),
    ShellCommand::shell("SHOW PARAMETERS", "SHOW PARAMETER[S] [text]", "Initialisation parameters whose name contains the text"),
    ShellCommand::shell("SHOW RELEASE", "SHOW REL[EASE]", "Versions of the database and its components"),
    ShellCommand::shell("SHOW ERRORS", "SHOW ERR[ORS] [type [schema.]name]", "Compilation errors of the current schema, or of one stored object"),
    ShellCommand::shell("SHOW SGA", "SHOW SGA", "Sizes of the System Global Area"),
    ShellCommand::shell("SHOW RECYCLEBIN", "SHOW RECYCLEBIN", "Objects in the recycle bin"),
    ShellCommand::shell("SHOW TABLES", "SHOW TABLES", "Tables of the current schema (a convenience, SQL*Plus has no such command)"),
    ShellCommand::shell("HELP", "HELP | ?", "List the commands this console understands"),
    ShellCommand::native("SELECT * FROM TAB", "SELECT * FROM TAB", "Tables, views and synonyms of the current schema"),
    ShellCommand::native("SELECT * FROM USER_TABLES", "SELECT * FROM USER_TABLES", "Tables the current user owns"),
    ShellCommand::native("SELECT * FROM USER_TAB_COLUMNS", "SELECT * FROM USER_TAB_COLUMNS WHERE table_name = 'T'", "Columns of the tables and views the current user owns"),
    ShellCommand::native("SELECT * FROM USER_OBJECTS", "SELECT * FROM USER_OBJECTS", "Every object the current user owns"),
    ShellCommand::native("SELECT * FROM USER_INDEXES", "SELECT * FROM USER_INDEXES", "Indexes the current user owns"),
    ShellCommand::native("SELECT * FROM USER_CONSTRAINTS", "SELECT * FROM USER_CONSTRAINTS", "Constraints on the current user's tables"),
    ShellCommand::native("SELECT * FROM ALL_USERS", "SELECT * FROM ALL_USERS", "Users (schemas) visible to the current user"),
    ShellCommand::native("SELECT * FROM V$VERSION", "SELECT * FROM V$VERSION", "Version banner of the database"),
];

/// The schema a statement runs against when the user named none.
const CURRENT_SCHEMA: &str = "SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA')";

fn lit(raw: &str) -> String {
    string_literal(raw, Quote::Ansi)
}

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    if eq_ci(head, "HELP") || text == "?" {
        return Some(Ok(help_plan(&DIALECT)));
    }
    if eq_ci(head, "DESC") || eq_ci(head, "DESCRIBE") {
        return Some(describe(rest));
    }
    if eq_ci(head, "SHOW") {
        return Some(show(rest));
    }
    None
}

// ---------------------------------------------------------------------------
// DESCRIBE
// ---------------------------------------------------------------------------

/// A name as the dictionary stores it: upper case unless quoted.
struct Target {
    owner: Option<String>,
    name: String,
}

fn target(raw: &str) -> Result<Target, String> {
    let parts = args::qualified(raw)?;
    if parts.iter().any(|p| !p.quoted && p.text.contains(char::is_whitespace)) {
        return Err("Give one object name, e.g. DESCRIBE hr.employees.".to_string());
    }
    let fold = |p: &Part| p.folded_upper();
    match parts.as_slice() {
        [name] => Ok(Target { owner: None, name: fold(name) }),
        [owner, name] => Ok(Target { owner: Some(fold(owner)), name: fold(name) }),
        _ => Err("Give the object as name or schema.name.".to_string()),
    }
}

fn describe(rest: &str) -> AppResult<Plan> {
    let rest = rest.trim();
    if rest.is_empty() {
        return Err(AppError::invalid_input("DESCRIBE needs an object name, e.g. DESCRIBE emp or DESCRIBE hr.employees."));
    }
    let target = target(rest).map_err(AppError::invalid_input)?;
    let name = lit(&target.name);
    // The schema a name is looked up in: the one the user typed, else the current one.
    let owner = target.owner.as_deref().map_or_else(|| CURRENT_SCHEMA.to_string(), lit);
    // Where an unqualified name may also live: a public synonym.
    let owners = match &target.owner {
        Some(owner) => lit(owner),
        None => format!("{CURRENT_SCHEMA}, 'PUBLIC'"),
    };
    let object_owner_filter = match &target.owner {
        Some(owner) => format!("o.owner = {}", lit(owner)),
        None => format!("(o.owner = {CURRENT_SCHEMA} OR (o.owner = 'PUBLIC' AND o.object_type = 'SYNONYM'))"),
    };

    let header = format!(
        "SELECT o.owner AS \"Owner\", o.object_name AS \"Object\", o.object_type AS \"Type\", o.status AS \"Status\" \
         FROM all_objects o WHERE o.object_name = {name} AND o.object_type NOT IN ('PACKAGE BODY', 'TYPE BODY') AND {object_owner_filter} \
         ORDER BY o.owner, o.object_type"
    );
    let columns = format!(
        "SELECT c.column_name AS \"Name\", CASE WHEN c.nullable = 'N' THEN 'NOT NULL' END AS \"Null?\", {TYPE_EXPR} AS \"Type\" \
         FROM all_tab_columns c WHERE (c.owner, c.table_name) IN (\
         SELECT {owner}, {name} FROM dual \
         UNION ALL SELECT s.table_owner, s.table_name FROM all_synonyms s WHERE s.owner IN ({owners}) AND s.synonym_name = {name}) \
         ORDER BY c.owner, c.table_name, c.column_id"
    );
    let arguments = format!(
        "SELECT a.argument_name AS \"Argument Name\", a.data_type AS \"Type\", a.in_out AS \"In/Out\", a.defaulted AS \"Default?\" \
         FROM all_arguments a WHERE a.owner = {owner} AND a.object_name = {name} AND a.package_name IS NULL AND a.data_level = 0 \
         ORDER BY a.overload, a.position"
    );
    let members = format!(
        "SELECT a.object_name AS \"Subprogram\", a.overload AS \"Overload\", a.argument_name AS \"Argument Name\", a.data_type AS \"Type\", \
         a.in_out AS \"In/Out\", a.defaulted AS \"Default?\" \
         FROM all_arguments a WHERE a.owner = {owner} AND a.package_name = {name} AND a.data_level = 0 \
         ORDER BY a.object_name, a.overload, a.position"
    );
    let missing = format!("ORA-04043: object {} does not exist", target.name);
    Ok(Plan { steps: vec![Step::Exists(header, missing), Step::Section(columns), Step::Section(arguments), Step::Section(members)] })
}

/// SQL*Plus's rendering of a column's type: `VARCHAR2(30 CHAR)`, `NUMBER(10,2)`, `RAW(16)`, `DATE`.
const TYPE_EXPR: &str = "CASE \
    WHEN c.data_type IN ('VARCHAR2', 'CHAR') THEN c.data_type || '(' || c.char_length || CASE WHEN c.char_used = 'C' THEN ' CHAR' ELSE '' END || ')' \
    WHEN c.data_type IN ('NVARCHAR2', 'NCHAR') THEN c.data_type || '(' || c.char_length || ')' \
    WHEN c.data_type = 'NUMBER' THEN CASE \
        WHEN c.data_precision IS NULL AND c.data_scale IS NULL THEN 'NUMBER' \
        WHEN c.data_precision IS NULL THEN 'NUMBER(*,' || c.data_scale || ')' \
        WHEN c.data_scale IS NULL OR c.data_scale = 0 THEN 'NUMBER(' || c.data_precision || ')' \
        ELSE 'NUMBER(' || c.data_precision || ',' || c.data_scale || ')' END \
    WHEN c.data_type = 'FLOAT' THEN 'FLOAT(' || c.data_precision || ')' \
    WHEN c.data_type IN ('RAW', 'UROWID') THEN c.data_type || '(' || c.data_length || ')' \
    ELSE c.data_type END";

// ---------------------------------------------------------------------------
// SHOW
// ---------------------------------------------------------------------------

const SHOW_OPTIONS: &str = "USER, CON_NAME, CON_ID, PDBS, PARAMETERS, RELEASE, ERRORS, SGA, RECYCLEBIN, TABLES";

fn show(rest: &str) -> AppResult<Plan> {
    let (option, tail) = split_word(rest);
    let option = option.to_ascii_uppercase();
    let nothing_more = |plan: Plan| -> AppResult<Plan> {
        if tail.is_empty() {
            Ok(plan)
        } else {
            Err(AppError::invalid_input(format!("SHOW {option} takes no arguments.")))
        }
    };
    match option.as_str() {
        "" => Err(AppError::invalid_input(format!("SHOW what? DB Free supports: {SHOW_OPTIONS}."))),
        "USER" => nothing_more(Plan::native(format!(
            "SELECT USER AS \"User\", {CURRENT_SCHEMA} AS \"Current schema\" FROM dual"
        ))),
        "CON_NAME" => nothing_more(Plan::native("SELECT SYS_CONTEXT('USERENV', 'CON_NAME') AS \"CON_NAME\" FROM dual")),
        "CON_ID" => nothing_more(Plan::native("SELECT SYS_CONTEXT('USERENV', 'CON_ID') AS \"CON_ID\" FROM dual")),
        "PDBS" => nothing_more(Plan::native(
            "SELECT con_id AS \"CON_ID\", name AS \"CON_NAME\", open_mode AS \"OPEN MODE\", restricted AS \"RESTRICTED\" FROM v$pdbs ORDER BY con_id",
        )),
        "PARAMETERS" | "PARAMETER" | "PARAM" | "PARA" => Ok(parameters(tail)),
        "RELEASE" | "REL" => nothing_more(Plan::native("SELECT product AS \"Product\", version AS \"Version\" FROM product_component_version")),
        "ERRORS" | "ERR" | "ERROR" => errors(tail),
        "SGA" => nothing_more(Plan::native("SELECT name AS \"Name\", value AS \"Value\" FROM v$sga ORDER BY name")),
        "RECYCLEBIN" | "RECYC" => nothing_more(Plan::native(
            "SELECT original_name AS \"Original name\", object_name AS \"Recyclebin name\", type AS \"Type\", droptime AS \"Drop time\" FROM recyclebin ORDER BY droptime",
        )),
        "TABLES" => nothing_more(Plan::native(format!(
            "SELECT t.table_name AS \"Table\", t.tablespace_name AS \"Tablespace\", t.num_rows AS \"Rows\" \
             FROM all_tables t WHERE t.owner = {CURRENT_SCHEMA} ORDER BY t.table_name"
        ))),
        other => Err(AppError::invalid_input(format!(
            "SHOW {other} is a SQL*Plus client setting or is not supported here. DB Free supports: {SHOW_OPTIONS}."
        ))),
    }
}

// WHAT:  `SHOW PARAMETERS text`: SQL*Plus lists the parameters whose name contains `text`, any case.
// HOW:   DISPLAY_VALUE is the user-friendly form of VALUE (`256K`, not `262144`), which is how
//        SQL*Plus prints sizes.
fn parameters(text: &str) -> Plan {
    let filter = if text.is_empty() { String::new() } else { format!(" WHERE LOWER(p.name) LIKE {}", lit(&format!("%{}%", text.to_lowercase()))) };
    Plan::native(format!(
        "SELECT p.name AS \"NAME\", DECODE(p.type, 1, 'boolean', 2, 'string', 3, 'integer', 4, 'parameter file', 5, 'reserved', 6, 'big integer', 'unknown') AS \"TYPE\", \
         p.display_value AS \"VALUE\" FROM v$parameter p{filter} ORDER BY p.name"
    ))
}

/// The object types `SHOW ERRORS` accepts.
const ERROR_TYPES: &[&str] = &["FUNCTION", "PROCEDURE", "PACKAGE", "PACKAGE BODY", "TRIGGER", "VIEW", "TYPE", "TYPE BODY", "DIMENSION", "JAVA SOURCE", "JAVA CLASS"];

// WHAT:  `SHOW ERRORS` for the current schema, or `SHOW ERRORS PACKAGE BODY hr.pkg` for one object.
// WHY:   SQL*Plus remembers the last object it compiled; this editor does not, so the
//        bare form lists every error in the schema rather than guessing which one.
fn errors(tail: &str) -> AppResult<Plan> {
    let base = "SELECT e.name AS \"Name\", e.type AS \"Type\", e.line AS \"Line\", e.position AS \"Position\", e.text AS \"Error\" FROM all_errors e WHERE ";
    let order = " ORDER BY e.name, e.type, e.sequence";
    if tail.is_empty() {
        return Ok(Plan::native(format!("{base}e.owner = {CURRENT_SCHEMA}{order}")));
    }
    // A quoted name may hold blanks, so the words are cut the way a shell cuts them.
    let words = args::raw_args(tail).map_err(AppError::invalid_input)?;
    let Some((raw_name, type_words)) = words.split_last() else {
        return Ok(Plan::native(format!("{base}e.owner = {CURRENT_SCHEMA}{order}")));
    };
    let kind = type_words.join(" ").to_ascii_uppercase();
    if !ERROR_TYPES.contains(&kind.as_str()) {
        return Err(AppError::invalid_input(format!(
            "SHOW ERRORS takes an object type and a name, e.g. SHOW ERRORS PROCEDURE hr.raise_salary. Types: {}.",
            ERROR_TYPES.join(", ")
        )));
    }
    let named = target(raw_name).map_err(AppError::invalid_input)?;
    let owner = named.owner.as_deref().map_or_else(|| CURRENT_SCHEMA.to_string(), lit);
    Ok(Plan::native(format!("{base}e.owner = {owner} AND e.type = {} AND e.name = {}{order}", lit(&kind), lit(&named.name))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::destructive::{classify, StatementKind};
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Oracle), text)
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
        for text in ["desc emp", "DESCRIBE emp;", "Desc hr.employees", "show user", "SHOW USER;", "show con_name", "show con_id", "show pdbs", "show parameters", "show parameter db_", "show release", "show rel", "show errors", "show err", "show sga", "show recyclebin", "show tables", "help", "HELP;", "?", "help index"] {
            assert!(matches!(planned(text), Some(Ok(_))), "{text}");
        }
    }

    #[test]
    fn native_statements_are_not_intercepted() {
        for text in ["SELECT * FROM TAB", "select 1 from dual", "describe_me()", "SHOWER", "helpful", "DESCRIPTION"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn describe_resolves_unquoted_names_to_upper_case_and_the_current_schema() {
        let sql = statements("desc emp");
        assert_eq!(sql.len(), 4);
        assert!(sql[0].contains("o.object_name = 'EMP'") && sql[0].contains("SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA')"), "{}", sql[0]);
        assert!(sql[0].contains("o.owner = 'PUBLIC'"), "an unqualified name may be a public synonym");
        assert!(sql[1].contains("FROM all_tab_columns c") && sql[1].contains("ORDER BY c.owner, c.table_name, c.column_id"));
        assert!(sql[1].contains("NUMBER(") && sql[1].contains("' CHAR'"));
        assert!(sql[2].contains("FROM all_arguments a") && sql[2].contains("a.package_name IS NULL"));
        assert!(sql[3].contains("a.package_name = 'EMP'"));
    }

    #[test]
    fn describe_honours_a_schema_and_quoted_case() {
        let sql = statements("DESCRIBE hr.\"Mixed Case\"");
        assert!(sql[0].contains("o.owner = 'HR'") && sql[0].contains("o.object_name = 'Mixed Case'"), "{}", sql[0]);
        assert!(!sql[0].contains("PUBLIC"), "a qualified name is not looked up as a public synonym");
        assert!(sql[1].contains("SELECT 'HR', 'Mixed Case' FROM dual"));
    }

    #[test]
    fn describe_of_a_missing_object_says_what_sql_plus_says() {
        let Some(Ok(plan)) = planned("desc nothing_here") else { panic!("not planned") };
        assert!(matches!(&plan.steps[0], Step::Exists(_, message) if message == "ORA-04043: object NOTHING_HERE does not exist"));
        assert!(matches!(&plan.steps[1], Step::Section(_)));
    }

    #[test]
    fn describe_needs_exactly_one_name() {
        assert!(error("desc").contains("needs an object name"));
        assert!(error("desc a b").contains("one object name"));
        assert!(error("desc a.b.c").contains("name or schema.name"));
        assert!(error("desc a..b").contains("not a valid name"));
        assert!(error("describe \"open").contains("never closed"));
    }

    #[test]
    fn user_text_stays_inside_a_literal() {
        let sql = statements("desc \"x' OR 1=1 --\"");
        assert!(sql[0].contains("o.object_name = 'x'' OR 1=1 --'"), "{}", sql[0]);
        let params = statements("show parameters x' || (select 1 from dual) || '");
        assert!(params[0].contains("LIKE '%x'' || (select 1 from dual) || ''%'"), "{}", params[0]);
        let backslash = statements("show parameters a\\");
        assert!(backslash[0].contains("LIKE '%a\\%'"), "a backslash is an ordinary character in Oracle: {}", backslash[0]);
        let errors = statements("show errors procedure hr.\"p'; drop table t; --\"");
        assert!(errors[0].contains("e.name = 'p''; drop table t; --'"), "{}", errors[0]);
    }

    #[test]
    fn show_parameters_matches_a_substring_in_any_case() {
        let sql = statements("SHOW PARAMETERS DB_Block");
        assert!(sql[0].contains("FROM v$parameter p WHERE LOWER(p.name) LIKE '%db_block%'"), "{}", sql[0]);
        assert!(sql[0].contains("DECODE(p.type, 1, 'boolean'"));
        assert!(sql[0].contains("p.display_value AS \"VALUE\""), "sizes print as 256K, as in SQL*Plus: {}", sql[0]);
        assert!(!statements("show parameters")[0].contains("WHERE"));
    }

    #[test]
    fn show_errors_reads_the_current_schema_or_one_object() {
        assert!(statements("show errors")[0].contains("e.owner = SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA')"));
        let one = statements("show errors package body pkg_util");
        assert!(one[0].contains("e.type = 'PACKAGE BODY'") && one[0].contains("e.name = 'PKG_UTIL'"), "{}", one[0]);
        assert!(error("show errors widget w").contains("object type"));
    }

    #[test]
    fn show_rejects_what_it_cannot_answer_and_lists_what_it_can() {
        assert!(error("show").contains("SHOW what?"));
        assert!(error("show linesize").contains("client setting"));
        assert!(error("show user now").contains("takes no arguments"));
    }

    #[test]
    fn every_statement_is_a_read() {
        for text in ["desc hr.emp", "show user", "show con_name", "show con_id", "show pdbs", "show parameters x", "show release", "show errors", "show errors view v", "show sga", "show recyclebin", "show tables"] {
            for sql in statements(text) {
                let kinds: Vec<StatementKind> = classify(&sql).into_iter().map(|s| s.kind).collect();
                assert_eq!(kinds, vec![StatementKind::Read], "{text}: {sql}");
                assert!(!sql.contains(';'), "{text}: the adapter takes one statement without a terminator");
            }
        }
    }

    #[test]
    fn line_commands_are_cut_without_a_semicolon() {
        let found = segments(Engine::Oracle, "desc emp\nselect * from dual;\nshow user");
        assert_eq!(
            found,
            vec![Segment::Meta("desc emp".to_string()), Segment::Native("select * from dual".to_string()), Segment::Meta("show user".to_string())]
        );
    }

    #[test]
    fn help_lists_every_command() {
        let Some(Ok(plan)) = planned("help") else { panic!("no help") };
        assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()));
    }
}
