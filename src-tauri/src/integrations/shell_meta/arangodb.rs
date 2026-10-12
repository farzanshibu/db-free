// SOT: shell-meta-arangodb, arangosh-db-helpers, arangodb-aql-structure-reads

use super::args::{eq_ci, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// ARANGODB
//
// WHAT:  The arangosh `db` helpers that describe a database, translated to AQL:
//        `db._collections()`, `db._name()`, `db._version()`, and
//        `db.<collection>.count()` (also `db["name"].count()` and
//        `db._collection("name").count()`), plus `help`.
// WHY:   arangosh is a JavaScript shell, so its structure commands are method
//        calls on `db`. The adapter's tab speaks AQL (`arangodb::
//        parse_execute_input`: AQL text or `{"query","bindVars"}`), where the same
//        questions have functions: `COLLECTIONS()`, `CURRENT_DATABASE()`,
//        `VERSION()`, and a `COLLECT WITH COUNT` over `@@collection` for a count. The rest of the
//        helpers (`db._databases()`, `db._views()`, `db.<c>.getIndexes()`,
//        `db.<c>.properties()`) read catalog state that AQL cannot reach and
//        that the adapter has no HTTP call for from `execute`; they are
//        recognised so the answer is an explanation, not an AQL syntax error.
// HOW:   Matching is anchored and exact (arangosh is case-sensitive JavaScript);
//        nothing is evaluated. The collection name of a count is never part of
//        the query text: it travels as the `@c` collection bind parameter
//        (`FOR d IN @@c` in the adapter's own queries), so quoting cannot be
//        escaped. A name must be a JavaScript identifier or a quoted string
//        without control characters.
// WHERE: src-tauri/src/integrations/arangodb.rs (`execute`, `parse_execute_input`,
//        `is_write_aql`), src-tauri/src/integrations/shell_meta/mod.rs
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::shell("db._collections()", "db._collections()", "Every collection of the database (AQL: COLLECTIONS())"),
    ShellCommand::shell("db._name()", "db._name()", "The current database's name"),
    ShellCommand::shell("db._version()", "db._version()", "The server version"),
    ShellCommand::shell("db.collection.count()", "db.<collection>.count()", "Number of documents in a collection; also db[\"name\"] and db._collection(\"name\")"),
    ShellCommand::native("FOR c IN COLLECTIONS() RETURN c", "FOR c IN COLLECTIONS() RETURN c", "Collections of the database as AQL"),
    ShellCommand::native("RETURN CURRENT_DATABASE()", "RETURN CURRENT_DATABASE()", "The current database's name as AQL"),
    ShellCommand::native("RETURN LENGTH(collection)", "RETURN LENGTH(collection_name)", "Number of documents in a collection as AQL"),
    ShellCommand::native("RETURN VERSION()", "RETURN VERSION()", "The server version as AQL"),
];

/// The arangosh `db` helpers that read state AQL cannot see.
const UNAVAILABLE: &[(&str, &str)] = &[
    ("_databases", "AQL cannot list databases; the sidebar's database switcher lists them"),
    ("_views", "AQL cannot list views; the sidebar's Views section lists them"),
];
/// Collection methods that read state AQL cannot see.
const UNAVAILABLE_ON_COLLECTION: &[&str] = &["getIndexes", "indexes", "properties", "figures", "revision", "checksum"];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let text = strip_terminator(text);
    if eq_ci(text, "help") {
        return Some(Ok(help_plan(&DIALECT)));
    }
    let call = parse_call(text)?;
    Some(match call {
        Call::Method(name) => on_database(&name),
        Call::Collection { name, method } => on_collection(&name, &method),
    })
}

/// What an arangosh line after `db` names.
#[derive(Debug, PartialEq, Eq)]
enum Call {
    /// `db._something()`
    Method(String),
    /// `db.users.count()`, `db["users"].count()`, `db._collection("users").count()`
    Collection { name: String, method: String },
}

fn on_database(name: &str) -> AppResult<Plan> {
    match name {
        "_collections" => Ok(Plan::native("FOR c IN COLLECTIONS() SORT c.name RETURN c")),
        "_name" => Ok(Plan::native("RETURN CURRENT_DATABASE()")),
        "_version" => Ok(Plan::native("RETURN VERSION()")),
        other => match UNAVAILABLE.iter().find(|(n, _)| *n == other) {
            Some((n, why)) => Err(AppError::invalid_input(format!("db.{n}() is not available here: {why}."))),
            None => Err(AppError::invalid_input(format!("db.{other}() is not one of the helpers DB Free translates. Type help to list them, or write AQL."))),
        },
    }
}

fn on_collection(name: &str, method: &str) -> AppResult<Plan> {
    match method {
        "count" => Ok(Plan::native(serde_json::json!({ "query": "FOR d IN @@c COLLECT WITH COUNT INTO n RETURN n", "bindVars": { "@c": name } }).to_string())),
        other if UNAVAILABLE_ON_COLLECTION.contains(&other) => Err(AppError::invalid_input(format!(
            "db.<collection>.{other}() reads state AQL cannot see; the sidebar shows a collection's indexes and properties."
        ))),
        other => Err(AppError::invalid_input(format!("db.<collection>.{other}() is not one of the helpers DB Free translates. Type help to list them, or write AQL."))),
    }
}

// WHAT:  `db.NAME()` / `db.NAME.method()` / `db["NAME"].method()` /
//        `db._collection("NAME").method()`, or None when the line is something else.
// HOW:   A small cursor over the characters; only these shapes parse, and the line
//        must end right after the closing parenthesis.
fn parse_call(text: &str) -> Option<Call> {
    let mut cursor = Cursor::new(text);
    cursor.word("db")?;
    if cursor.eat('[') {
        let name = cursor.string()?;
        cursor.expect(']')?;
        return finish_collection(cursor, name);
    }
    cursor.expect('.')?;
    let ident = cursor.ident()?;
    // A helper called on `db`.
    if cursor.eat('(') {
        if ident == "_collection" {
            let name = cursor.string()?;
            cursor.expect(')')?;
            return finish_collection(cursor, name);
        }
        cursor.expect(')')?;
        return cursor.at_end().then_some(Call::Method(ident));
    }
    finish_collection(cursor, ident)
}

// WHAT:  The `.method()` that follows a collection.
fn finish_collection(mut cursor: Cursor, name: String) -> Option<Call> {
    cursor.expect('.')?;
    let method = cursor.ident()?;
    cursor.expect('(')?;
    cursor.expect(')')?;
    (cursor.at_end() && !name.is_empty()).then_some(Call::Collection { name, method })
}

struct Cursor {
    chars: Vec<char>,
    at: usize,
}

impl Cursor {
    fn new(text: &str) -> Cursor {
        Cursor { chars: text.chars().collect(), at: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_blanks(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_blanks();
        if self.peek() == Some(c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> Option<()> {
        self.eat(c).then_some(())
    }

    fn at_end(&mut self) -> bool {
        self.skip_blanks();
        self.at >= self.chars.len()
    }

    /// A whole word (not the start of a longer identifier).
    fn word(&mut self, word: &str) -> Option<()> {
        let ident = self.ident()?;
        (ident == word).then_some(())
    }

    /// A JavaScript identifier: letters, digits, `_` and `$`, not starting with a digit.
    fn ident(&mut self) -> Option<String> {
        self.skip_blanks();
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$') {
            self.at += 1;
        }
        let ident: String = self.chars[start..self.at].iter().collect();
        (!ident.is_empty() && !ident.starts_with(|c: char| c.is_ascii_digit())).then_some(ident)
    }

    /// A single- or double-quoted string; backslashes and control characters are not accepted.
    fn string(&mut self) -> Option<String> {
        self.skip_blanks();
        let quote = self.peek().filter(|c| matches!(c, '\'' | '"'))?;
        self.at += 1;
        let start = self.at;
        while let Some(c) = self.peek() {
            if c == quote {
                let value: String = self.chars[start..self.at].iter().collect();
                self.at += 1;
                return Some(value);
            }
            if c == '\\' || c.is_control() {
                return None;
            }
            self.at += 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{CommandOrigin, Step};
    use crate::model::Engine;
    use serde_json::{json, Value};

    fn run(text: &str) -> Option<AppResult<Plan>> {
        plan(&Ctx::bare(Engine::Arangodb), text)
    }

    fn statement(text: &str) -> String {
        match run(text) {
            Some(Ok(plan)) => match plan.steps.as_slice() {
                [Step::Native(statement)] => statement.clone(),
                other => panic!("{text}: {other:?}"),
            },
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn the_database_helpers_become_aql() {
        assert_eq!(statement("db._collections()"), "FOR c IN COLLECTIONS() SORT c.name RETURN c");
        assert_eq!(statement("db._collections();"), "FOR c IN COLLECTIONS() SORT c.name RETURN c");
        assert_eq!(statement("db . _name ( )"), "RETURN CURRENT_DATABASE()");
        assert_eq!(statement("db._version()"), "RETURN VERSION()");
    }

    #[test]
    fn a_count_carries_the_collection_as_a_bind_parameter() {
        let expected = json!({"query": "FOR d IN @@c COLLECT WITH COUNT INTO n RETURN n", "bindVars": {"@c": "users"}});
        for text in ["db.users.count()", "db[\"users\"].count()", "db['users'].count()", "db._collection(\"users\").count()", "db._collection('users').count();"] {
            let sent: Value = serde_json::from_str(&statement(text)).unwrap_or_default();
            assert_eq!(sent, expected, "{text}");
        }
        let system: Value = serde_json::from_str(&statement("db._users.count()")).unwrap_or_default();
        assert_eq!(system["bindVars"]["@c"], "_users", "a system collection is a collection");
    }

    #[test]
    fn a_hostile_collection_name_stays_in_the_bind_value() {
        for text in ["db[\"a\\\"b\"].count()", "db['x\ny'].count()"] {
            assert!(run(text).is_none(), "{text:?}: a string with a backslash or control character is not parsed");
        }
        let odd: Value = serde_json::from_str(&statement("db[\"a b'; REMOVE d IN users\"].count()")).unwrap_or_default();
        assert_eq!(odd["query"], "FOR d IN @@c COLLECT WITH COUNT INTO n RETURN n");
        assert_eq!(odd["bindVars"]["@c"], "a b'; REMOVE d IN users");
    }

    #[test]
    fn helpers_aql_cannot_answer_explain_themselves() {
        for text in ["db._databases()", "db._views()", "db.users.getIndexes()", "db.users.properties()", "db._frobnicate()", "db.users.truncate()"] {
            let found = run(text);
            assert!(matches!(found, Some(Err(AppError::InvalidInput { .. }))), "{text}: {found:?}");
        }
    }

    #[test]
    fn other_javascript_and_aql_are_left_alone() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(run(command.name).is_none(), "{}", command.name);
        }
        for text in [
            "db._collections().length",
            "db._collections()[0]",
            "db.users.count() + 1",
            "db.users.count",
            "db._query('RETURN 1')",
            "db._query(\"RETURN 1\").toArray()",
            "DB._collections()",
            "dbx.users.count()",
            "db.1x.count()",
            "db.users.count(1)",
            "FOR d IN users RETURN d",
            "db",
            "db.",
            "db[].count()",
        ] {
            let found = run(text);
            assert!(found.is_none() || matches!(found, Some(Err(_))), "{text}: {found:?}");
            assert!(!matches!(found, Some(Ok(_))), "{text} must not be translated");
        }
    }

    #[test]
    fn help_lists_every_command() {
        for text in ["help", "HELP", "help;"] {
            let Some(Ok(plan)) = run(text) else { panic!("{text}") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn nothing_emitted_writes() {
        for text in ["db._collections()", "db._name()", "db._version()", "db.users.count()"] {
            let sent = statement(text);
            let query = serde_json::from_str::<Value>(&sent).ok().and_then(|v| v["query"].as_str().map(str::to_string)).unwrap_or(sent);
            for word in ["INSERT", "UPDATE", "REPLACE", "REMOVE", "UPSERT"] {
                assert!(!query.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w.eq_ignore_ascii_case(word)), "{query}");
            }
        }
    }
}
