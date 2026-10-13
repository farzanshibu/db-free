// SOT: shell-meta-orientdb, orientdb-console-list-info, orientdb-metadata-schema-queries

use super::args::{eq_ci, split_word, string_literal, unquote, Quote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand, Step};
use crate::error::{AppError, AppResult};

// ============================================================================
// ORIENTDB (the console)
//
// WHAT:  The structure commands of the OrientDB console: `LIST CLASSES`,
//        `LIST INDEXES`, `LIST USERS`, `INFO CLASS <class>`,
//        `INFO PROPERTY <class>.<property>`, and `help`.
// WHY:   The console answers `LIST` and `INFO` itself; the server's SQL does not
//        know them. They are the OrientDB user's way of asking what a database
//        holds.
// HOW:   Each command becomes OrientDB SQL over the schema documents the server
//        keeps: `metadata:schema` (classes and their properties) and
//        `metadata:indexmanager` (indexes), plus the `OUser` class. The adapter
//        sends the text to `POST /command/{db}/sql`
//        (src-tauri/src/integrations/orientdb.rs, `execute` and `command`); it
//        uses the same `expand(indexes) FROM metadata:indexmanager` and `OUser`
//        queries for its explorer. Class lookups are case-insensitive, as
//        OrientDB's are, property lookups are exact. Names are quoted as
//        OrientDB string literals (backslash escapes), never spliced in raw.
//        `LIST CLUSTERS`, `LIST DATABASES` and bare `INFO` are the console's
//        too, but are answered by REST calls SQL cannot make, so they say so.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: None,
    strict: false,
    help: &["help"],
    cut: Cut::Statements { line_words: &["list", "info", "help"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("LIST CLASSES", "LIST CLASSES", "List the classes with their super classes and clusters"),
    ShellCommand::shell("LIST INDEXES", "LIST INDEXES", "List the indexes"),
    ShellCommand::shell("LIST USERS", "LIST USERS", "List the database users and their roles"),
    ShellCommand::shell("INFO CLASS", "INFO CLASS class", "A class: its definition, properties and indexes"),
    ShellCommand::shell("INFO PROPERTY", "INFO PROPERTY class.property", "One property of a class"),
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::native("SELECT FROM V", "SELECT FROM V [WHERE ...]", "Vertices of a graph, or records of any class"),
    ShellCommand::native("SELECT expand(classes) FROM metadata:schema", "SELECT expand(classes) FROM metadata:schema", "The classes, as the schema document keeps them"),
    ShellCommand::native("SELECT expand(indexes) FROM metadata:indexmanager", "SELECT expand(indexes) FROM metadata:indexmanager", "The indexes, as the index manager keeps them"),
    ShellCommand::native("SELECT FROM OUser", "SELECT name, status, roles.name FROM OUser", "Database users"),
];

fn lit(raw: &str) -> String {
    string_literal(raw, Quote::Backslash)
}

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    if eq_ci(head, "help") && rest.is_empty() {
        return Some(Ok(help_plan(&DIALECT)));
    }
    if eq_ci(head, "list") {
        return Some(list(rest));
    }
    if eq_ci(head, "info") {
        return Some(info(rest));
    }
    None
}

fn list(rest: &str) -> AppResult<Plan> {
    match rest.to_ascii_uppercase().as_str() {
        "CLASSES" => Ok(Plan::native(
            "SELECT name, superClasses, abstract, defaultClusterId, clusterIds FROM (SELECT expand(classes) FROM metadata:schema) ORDER BY name",
        )),
        "INDEXES" => Ok(Plan::native("SELECT expand(indexes) FROM metadata:indexmanager")),
        "USERS" => Ok(Plan::native("SELECT name, status, roles.name AS roles FROM OUser")),
        "CLUSTERS" | "DATABASES" | "SERVERS" | "STORAGES" => Err(AppError::invalid_input(format!(
            "LIST {} is answered by the server's REST API, which SQL cannot reach; the sidebar shows the classes and databases. DB Free reads LIST CLASSES, INDEXES and USERS.",
            rest.to_ascii_uppercase()
        ))),
        _ => Err(AppError::invalid_input("LIST what? DB Free reads LIST CLASSES, LIST INDEXES and LIST USERS.")),
    }
}

fn info(rest: &str) -> AppResult<Plan> {
    let (what, tail) = split_word(rest);
    match what.to_ascii_uppercase().as_str() {
        "CLASS" => class(&unquote(tail)),
        "PROPERTY" => property(&unquote(tail)),
        _ => Err(AppError::invalid_input("INFO what? DB Free reads INFO CLASS <class> and INFO PROPERTY <class>.<property>.")),
    }
}

/// The class documents of the schema, one row each.
const CLASSES: &str = "(SELECT expand(classes) FROM metadata:schema)";

/// A class or property name as a single token: no blanks, no quotes of its own.
fn checked(name: &str, what: &str) -> AppResult<()> {
    if name.is_empty() || name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(AppError::invalid_input(format!("Give the {what} as one name, e.g. INFO {} Person.", what.to_ascii_uppercase())));
    }
    Ok(())
}

// WHAT:  `INFO CLASS Person`: the class, its properties, and the indexes named after it.
// HOW:   Class names are case-insensitive in OrientDB, so the match is on the lower-cased name.
//        An index is named `Class.field` unless it was given another name, which is what the
//        index section matches; indexes under other names are in LIST INDEXES.
fn class(name: &str) -> AppResult<Plan> {
    checked(name, "class")?;
    let lower = name.to_lowercase();
    let wanted = lit(&lower);
    let prefix = lit(&format!("{lower}."));
    let prefix_len = prefix_length(&lower);
    let header = format!(
        "SELECT name, superClasses, abstract, strictMode, defaultClusterId, clusterIds FROM {CLASSES} WHERE name.toLowerCase() = {wanted}"
    );
    let properties = format!("SELECT expand(properties) FROM {CLASSES} WHERE name.toLowerCase() = {wanted}");
    let indexes = format!(
        "SELECT name, type, algorithm, indexDefinition FROM (SELECT expand(indexes) FROM metadata:indexmanager) WHERE name.toLowerCase().left({prefix_len}) = {prefix}"
    );
    Ok(Plan {
        steps: vec![
            Step::Exists(header, format!("Class '{name}' not found.")),
            Step::Section(properties),
            Step::Section(indexes),
        ],
    })
}

/// Characters of `lower.` — what `left(n)` takes off an index name to compare with the prefix.
fn prefix_length(lower: &str) -> usize {
    lower.chars().count() + 1
}

// WHAT:  `INFO PROPERTY Person.name`: one property of a class.
fn property(target: &str) -> AppResult<Plan> {
    checked(target, "property")?;
    let Some((class, property)) = target.split_once('.').filter(|(c, p)| !c.is_empty() && !p.is_empty()) else {
        return Err(AppError::invalid_input("Give the property as class.property, e.g. INFO PROPERTY Person.name."));
    };
    let statement = format!(
        "SELECT name, type, mandatory, notNull, readonly, linkedClass, linkedType, min, max, collate, defaultValue, regexp \
         FROM (SELECT expand(properties) FROM {CLASSES} WHERE name.toLowerCase() = {}) WHERE name = {}",
        lit(&class.to_lowercase()),
        lit(property)
    );
    Ok(Plan { steps: vec![Step::Exists(statement, format!("Property '{target}' not found."))] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard::destructive::{classify, StatementKind};
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Orientdb), text)
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
        for text in ["list classes", "LIST INDEXES;", "List Users", "info class Person", "INFO PROPERTY Person.name;", "help", "HELP;"] {
            assert!(matches!(planned(text), Some(Ok(_))), "{text}: {:?}", planned(text));
        }
    }

    #[test]
    fn orientdb_sql_is_not_intercepted() {
        for text in ["SELECT FROM V", "select expand(classes) from metadata:schema", "INFORMATION", "listing", "help me", "CREATE CLASS X"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn listings_read_the_schema_documents() {
        assert!(statements("list classes")[0].contains("FROM (SELECT expand(classes) FROM metadata:schema) ORDER BY name"));
        assert_eq!(statements("LIST INDEXES"), ["SELECT expand(indexes) FROM metadata:indexmanager"]);
        assert_eq!(statements("list users"), ["SELECT name, status, roles.name AS roles FROM OUser"]);
    }

    #[test]
    fn rest_only_listings_say_where_to_look() {
        for text in ["list clusters", "list databases"] {
            assert!(error(text).contains("REST API"), "{text}");
        }
        assert!(error("list").contains("LIST what?"));
        assert!(error("list tables").contains("LIST what?"));
    }

    #[test]
    fn info_class_reads_the_class_its_properties_and_its_indexes() {
        let Some(Ok(plan)) = planned("INFO CLASS Person") else { panic!("not planned") };
        assert_eq!(plan.steps.len(), 3);
        assert!(matches!(&plan.steps[0], Step::Exists(sql, message)
            if sql.contains("WHERE name.toLowerCase() = 'person'") && message == "Class 'Person' not found."));
        let sql: Vec<&str> = plan.statements().collect();
        assert!(sql[1].starts_with("SELECT expand(properties) FROM (SELECT expand(classes) FROM metadata:schema) WHERE name.toLowerCase() = 'person'"));
        assert!(sql[2].contains("name.toLowerCase().left(7) = 'person.'"), "{}", sql[2]);
        assert!(plan.steps[1..].iter().all(|s| matches!(s, Step::Section(_))));
    }

    #[test]
    fn info_property_reads_one_property_exactly() {
        let Some(Ok(plan)) = planned("info property Person.firstName") else { panic!("not planned") };
        assert!(matches!(&plan.steps[..], [Step::Exists(sql, message)]
            if sql.contains("WHERE name.toLowerCase() = 'person') WHERE name = 'firstName'") && message == "Property 'Person.firstName' not found."));
        assert!(error("info property Person").contains("class.property"));
        assert!(error("info property .name").contains("class.property"));
        assert!(error("info property Person.").contains("class.property"));
    }

    #[test]
    fn bad_arguments_say_how_to_fix_them() {
        assert!(error("info").contains("INFO what?"));
        assert!(error("info server").contains("INFO what?"));
        assert!(error("info class").contains("one name"));
        assert!(error("info class a b").contains("one name"));
    }

    #[test]
    fn user_text_stays_inside_a_literal() {
        assert!(matches!(planned("info class \"x' OR 1=1 --\""), Some(Err(_))), "blanks are refused before any SQL is written");
        let quote = statements("info class x'y");
        assert!(quote[0].contains("name.toLowerCase() = 'x\\'y'"), "{}", quote[0]);
        let backslash = statements("info class x\\");
        assert!(backslash[0].contains("= 'x\\\\'"), "{}", backslash[0]);
        let property = statements("info property A'.b'c");
        assert!(property[0].contains("name.toLowerCase() = 'a\\'') WHERE name = 'b\\'c'"), "{}", property[0]);
    }

    #[test]
    fn a_quoted_class_name_loses_its_quotes_and_its_case() {
        assert!(statements("info class \"Person\"")[0].contains("WHERE name.toLowerCase() = 'person'"));
        assert!(statements("info property 'Person.firstName'")[0].contains("WHERE name = 'firstName'"));
    }

    #[test]
    fn every_statement_is_a_read() {
        for text in ["list classes", "list indexes", "list users", "info class Person", "info property Person.name"] {
            for sql in statements(text) {
                let kinds: Vec<StatementKind> = classify(&sql).into_iter().map(|s| s.kind).collect();
                assert_eq!(kinds, vec![StatementKind::Read], "{text}: {sql}");
            }
        }
    }

    #[test]
    fn line_commands_are_cut_without_a_semicolon() {
        let found = segments(Engine::Orientdb, "LIST CLASSES\nSELECT FROM Person;\nINFO CLASS Person");
        assert_eq!(
            found,
            vec![
                Segment::Meta("LIST CLASSES".to_string()),
                Segment::Native("SELECT FROM Person".to_string()),
                Segment::Meta("INFO CLASS Person".to_string()),
            ]
        );
    }

    #[test]
    fn help_lists_every_command() {
        let Some(Ok(plan)) = planned("help") else { panic!("no help") };
        assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()));
    }
}
