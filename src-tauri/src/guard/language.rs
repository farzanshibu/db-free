// SOT: statement-language, language-aware-classification, redis-statement-classes, mongo-statement-classes

use super::destructive::{classify, ClassifiedStatement, StatementKind};
use crate::integrations::mongo_shell::{self, Shell};
use crate::integrations::redis as redis_adapter;
use crate::integrations::shell_meta::{self, Segment};
use crate::model::{Engine, Family};
use serde_json::Value as Json;

// ============================================================================
// LANGUAGE-AWARE CLASSIFICATION
//
// WHAT:  Labels each statement of a script Read / Write / Destructive in the
//        language its engine speaks: SQL by the block's own classifier, Redis by
//        command, MongoDB by command document.
// WHY:   `destructive::classify` reads SQL. Handed `GET key` it says Write (an
//        unknown leading word fails closed) and handed `SET key value` it says
//        Read (SET is a session statement in SQL), so a read-only Redis
//        connection refused every console read and let the SQL lock wave a write
//        through to the adapter, which alone stood between the two. The same went
//        for `db.users.find({})` on a read-only MongoDB connection.
// HOW:   The verdicts come from the adapters' own read lists (the Redis adapter's
//        `is_read_only_command`, `mongo_shell::is_read_command`), so the guard
//        and the adapter can never disagree about what reads. Anything the
//        classifier cannot parse is a Write, with the reason attached: failing
//        closed is the rule of the block, and nothing can run that the adapter
//        would not parse. The adapters still refuse writes on a read-only
//        connection themselves; this step makes the block's answer correct, it
//        does not replace theirs.
//        Other command languages (Cypher, AQL, Query DSL…) keep the SQL
//        classifier until they get a classifier of their own here.
//        The structure commands of an engine's own shell (`\dt`, `.tables`,
//        `DESC emp`, `:schema`, `help`) are reads in every language: they are
//        turned into catalog queries by `integrations::shell_meta`, which never
//        plans a write, so they pass a read-only lock instead of failing closed.
// WHERE: src-tauri/src/guard/mod.rs (`statement`, `script_gate` callers),
//        src-tauri/src/services/query.rs (`execute_batch`)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Sql,
    Redis,
    Mongo,
}

impl Language {
    pub fn of(engine: Engine) -> Language {
        match engine.family() {
            Family::Redis => Language::Redis,
            Family::Mongodb => Language::Mongo,
            _ => Language::Sql,
        }
    }
}

// WHAT:  `destructive::classify`, in the language of `engine`.
pub fn classify_for(engine: Engine, script: &str) -> Vec<ClassifiedStatement> {
    match Language::of(engine) {
        Language::Sql => classify_sql(engine, script),
        Language::Redis => classify_redis(engine, script),
        Language::Mongo => classify_mongo(engine, script),
    }
}

// WHAT:  SQL (and every command language the SQL classifier stands in for), with the
//        engine's shell commands told apart: each is a read, the text between them is
//        classified as before.
// HOW:   A script with no shell command is classified whole, untouched.
fn classify_sql(engine: Engine, script: &str) -> Vec<ClassifiedStatement> {
    let segments = shell_meta::segments(engine, script);
    if !segments.iter().any(|segment| matches!(segment, Segment::Meta(_))) {
        return classify(script);
    }
    segments
        .iter()
        .flat_map(|segment| match segment {
            Segment::Meta(text) => vec![statement(text, StatementKind::Read, None)],
            Segment::Native(text) => classify(text),
        })
        .collect()
}

fn statement(text: &str, kind: StatementKind, reason: Option<&str>) -> ClassifiedStatement {
    ClassifiedStatement { text: text.trim().to_string(), kind, reason: reason.map(str::to_string) }
}

fn unreadable(text: &str, why: &str) -> ClassifiedStatement {
    statement(text, StatementKind::Write, Some(why))
}

// ---------------------------------------------------------------------------
// Redis
// ---------------------------------------------------------------------------

// WHAT:  One command per line, as the Redis adapter reads a script.
// HOW:   Blank lines and `#` comments run nothing and are dropped. A line is
//        tokenised alone so a bad quote on one line is reported for that line.
fn classify_redis(engine: Engine, script: &str) -> Vec<ClassifiedStatement> {
    script
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            if shell_meta::is_meta(engine, line) {
                return statement(line, StatementKind::Read, None);
            }
            match redis_adapter::parse_commands(line) {
                Ok(commands) => match commands.first() {
                    Some(tokens) => redis_statement(line, tokens),
                    None => statement(line, StatementKind::Read, None),
                },
                Err(_) => unreadable(line, "it is not a command DB Free can read"),
            }
        })
        .collect()
}

fn redis_statement(line: &str, tokens: &[String]) -> ClassifiedStatement {
    let name = tokens.first().map(|t| t.to_ascii_uppercase()).unwrap_or_default();
    match name.as_str() {
        "FLUSHALL" => statement(line, StatementKind::Destructive, Some("FLUSHALL removes every key from every database.")),
        "FLUSHDB" => statement(line, StatementKind::Destructive, Some("FLUSHDB removes every key from the selected database.")),
        "SHUTDOWN" => statement(line, StatementKind::Destructive, Some("SHUTDOWN stops the server.")),
        _ if redis_adapter::is_read_only_command(tokens) => statement(line, StatementKind::Read, None),
        _ => statement(line, StatementKind::Write, None),
    }
}

// ---------------------------------------------------------------------------
// MongoDB
// ---------------------------------------------------------------------------

// WHAT:  A MongoDB console script: shell statements (`db.users.find({})`,
//        `show collections`, `use app`), JSON command documents, or the
//        `find <collection> {filter}` shorthand, cut the way the adapter cuts them.
fn classify_mongo(engine: Engine, script: &str) -> Vec<ClassifiedStatement> {
    shell_meta::segments(engine, script)
        .iter()
        .flat_map(|segment| match segment {
            Segment::Meta(text) => vec![statement(text, StatementKind::Read, None)],
            Segment::Native(text) => classify_mongo_text(text),
        })
        .collect()
}

fn classify_mongo_text(script: &str) -> Vec<ClassifiedStatement> {
    crate::integrations::mongodb::split_statements(script)
        .iter()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(mongo_statement)
        .collect()
}

fn mongo_statement(text: &str) -> ClassifiedStatement {
    if mongo_shell::is_shell(text) {
        return match mongo_shell::parse(text, 1) {
            Ok(Shell::ShowDatabases | Shell::ShowCollections | Shell::Use(_)) => statement(text, StatementKind::Read, None),
            Ok(Shell::Command(command)) => mongo_command(text, &command),
            Err(_) => unreadable(text, "it is not a statement DB Free can read"),
        };
    }
    if text.starts_with('{') {
        return match serde_json::from_str::<Json>(text) {
            Ok(command) if command.is_object() => mongo_command(text, &command),
            _ => unreadable(text, "it is not a JSON command document"),
        };
    }
    // The adapter's shorthands: `find <collection> {filter}` and `count <collection> {filter}`.
    let mut parts = text.split_whitespace();
    let verb = parts.next().unwrap_or_default().to_ascii_lowercase();
    if matches!(verb.as_str(), "find" | "count") && parts.next().is_some() {
        return statement(text, StatementKind::Read, None);
    }
    unreadable(text, "it is not a command DB Free can read")
}

fn mongo_command(text: &str, command: &Json) -> ClassifiedStatement {
    if mongo_shell::is_read_command(command) {
        return statement(text, StatementKind::Read, None);
    }
    match mongo_destruction(command) {
        Some(reason) => statement(text, StatementKind::Destructive, Some(reason)),
        None => statement(text, StatementKind::Write, None),
    }
}

// WHAT:  Why a write command cannot be undone, when it cannot: the MongoDB
//        counterparts of `DROP`, and of `DELETE` / `UPDATE` with no `WHERE`.
fn mongo_destruction(command: &Json) -> Option<&'static str> {
    let map = command.as_object()?;
    let (name, _) = map.iter().next()?;
    match name.as_str() {
        "drop" => Some("drop removes the collection and its documents."),
        "dropDatabase" => Some("dropDatabase removes the database and every collection in it."),
        "delete" if every_filter_is_empty(map.get("deletes"), "q") => Some("delete with an empty filter removes every document."),
        "update" if updates_everything(map.get("updates")) => Some("update with an empty filter rewrites every document."),
        _ => None,
    }
}

/// True when some entry of `specs` selects with an empty (or missing) filter.
fn every_filter_is_empty(specs: Option<&Json>, key: &str) -> bool {
    specs.and_then(Json::as_array).is_some_and(|list| list.iter().any(|spec| is_empty_filter(spec.get(key))))
}

/// An `update` spec that matches every document and writes to all of them.
fn updates_everything(specs: Option<&Json>) -> bool {
    specs
        .and_then(Json::as_array)
        .is_some_and(|list| list.iter().any(|spec| is_empty_filter(spec.get("q")) && spec.get("multi").and_then(Json::as_bool).unwrap_or(false)))
}

fn is_empty_filter(filter: Option<&Json>) -> bool {
    match filter {
        None | Some(Json::Null) => true,
        Some(Json::Object(map)) => map.is_empty(),
        Some(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(engine: Engine, script: &str) -> Vec<StatementKind> {
        classify_for(engine, script).into_iter().map(|s| s.kind).collect()
    }
    fn redis(script: &str) -> Vec<StatementKind> {
        kinds(Engine::Redis, script)
    }
    fn mongo(script: &str) -> Vec<StatementKind> {
        kinds(Engine::Mongodb, script)
    }

    use StatementKind::{Destructive, Read, Write};

    #[test]
    fn engines_pick_their_language() {
        assert_eq!(Language::of(Engine::Redis), Language::Redis);
        assert_eq!(Language::of(Engine::Valkey), Language::Redis, "wire-compatible products share the family's language");
        assert_eq!(Language::of(Engine::Dragonfly), Language::Redis);
        assert_eq!(Language::of(Engine::Mongodb), Language::Mongo);
        assert_eq!(Language::of(Engine::Postgres), Language::Sql);
        assert_eq!(Language::of(Engine::Sqlite), Language::Sql);
    }

    #[test]
    fn sql_engines_keep_the_sql_classifier() {
        for sql in ["SELECT 1", "DELETE FROM t", "UPDATE t SET a = 1 WHERE b = 2", "frobnicate"] {
            assert_eq!(classify_for(Engine::Postgres, sql), classify(sql), "{sql}");
        }
    }

    #[test]
    fn redis_reads_read_and_writes_write_which_sql_gets_backwards() {
        assert_eq!(classify("GET key")[0].kind, Write, "the SQL classifier fails closed on GET");
        assert_eq!(classify("SET key value")[0].kind, Read, "and calls SET a session statement");
        assert_eq!(redis("GET key"), vec![Read]);
        assert_eq!(redis("SET key value"), vec![Write]);
        assert_eq!(redis("get key\nttl key\nHGETALL h"), vec![Read, Read, Read]);
        assert_eq!(redis("DEL key"), vec![Write]);
        assert_eq!(redis("EXPIRE key 10"), vec![Write]);
        assert_eq!(redis("SCAN 0 MATCH user:* COUNT 100"), vec![Read]);
    }

    #[test]
    fn redis_sub_commands_decide_config_and_friends() {
        assert_eq!(redis("CONFIG GET maxmemory"), vec![Read]);
        assert_eq!(redis("CONFIG SET maxmemory 1gb"), vec![Write]);
        assert_eq!(redis("CLIENT LIST"), vec![Read]);
        assert_eq!(redis("CLIENT KILL ID 3"), vec![Write]);
        assert_eq!(redis("SORT list"), vec![Read]);
        assert_eq!(redis("SORT list STORE out"), vec![Write], "STORE writes");
        assert_eq!(redis("CONFIG"), vec![Write], "a sub-command command with no sub-command is refused");
    }

    #[test]
    fn redis_flushes_and_shutdown_need_confirmation() {
        for command in ["FLUSHALL", "flushall async", "FLUSHDB", "SHUTDOWN NOSAVE"] {
            let found = classify_for(Engine::Redis, command);
            assert_eq!(found[0].kind, Destructive, "{command}");
            assert!(found[0].reason.is_some(), "{command}");
        }
    }

    #[test]
    fn redis_scripts_are_read_a_line_at_a_time() {
        let found = classify_for(Engine::Redis, "# warm up\n\nGET a\n  SET b \"two words\"  \n");
        assert_eq!(found.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![Read, Write]);
        assert_eq!(found[1].text, "SET b \"two words\"", "the preview is the line as typed");
        assert!(classify_for(Engine::Redis, "").is_empty());
        assert!(classify_for(Engine::Redis, "# only a comment").is_empty());
    }

    #[test]
    fn redis_text_the_adapter_cannot_parse_fails_closed_with_a_reason() {
        let found = classify_for(Engine::Redis, "GET \"unterminated");
        assert_eq!(found[0].kind, Write);
        assert!(found[0].reason.as_deref().is_some_and(|r| r.contains("not a command")));
    }

    #[test]
    fn mongo_shell_reads_read() {
        assert_eq!(mongo("db.users.find({ age: { $gt: 3 } })"), vec![Read]);
        assert_eq!(mongo("db.users.find({}).sort({ a: 1 }).limit(5)"), vec![Read]);
        assert_eq!(mongo("db.users.countDocuments({})"), vec![Read]);
        assert_eq!(mongo("db.users.aggregate([{ $match: { a: 1 } }])"), vec![Read]);
        assert_eq!(mongo("show collections"), vec![Read]);
        assert_eq!(mongo("show dbs"), vec![Read]);
        assert_eq!(mongo("use app"), vec![Read]);
        assert_eq!(mongo("use app\ndb.users.find({})"), vec![Read, Read]);
    }

    #[test]
    fn mongo_shell_writes_write() {
        assert_eq!(mongo("db.users.insertOne({ a: 1 })"), vec![Write]);
        assert_eq!(mongo("db.users.updateOne({ a: 1 }, { $set: { b: 2 } })"), vec![Write]);
        assert_eq!(mongo("db.users.updateMany({ a: 1 }, { $set: { b: 2 } })"), vec![Write]);
        assert_eq!(mongo("db.users.deleteOne({ a: 1 })"), vec![Write]);
        assert_eq!(mongo("db.users.deleteMany({ old: true })"), vec![Write]);
        assert_eq!(mongo("db.users.aggregate([{ $match: {} }, { $out: \"copy\" }])"), vec![Write], "$out writes");
        assert_eq!(mongo("db.users.createIndex({ a: 1 })"), vec![Write]);
        assert_eq!(mongo("use app\ndb.users.insertOne({ a: 1 })"), vec![Read, Write]);
    }

    #[test]
    fn mongo_destructive_commands_need_confirmation() {
        for script in [
            "db.users.drop()",
            "db.dropDatabase()",
            "db.users.deleteMany({})",
            "db.users.updateMany({}, { $set: { archived: true } })",
            "{\"drop\": \"users\"}",
            "{\"delete\": \"users\", \"deletes\": [{\"q\": {}, \"limit\": 0}]}",
        ] {
            let found = classify_for(Engine::Mongodb, script);
            assert_eq!(found[0].kind, Destructive, "{script}");
            assert!(found[0].reason.is_some(), "{script}");
        }
        assert_eq!(mongo("db.users.updateOne({}, { $set: { a: 1 } })"), vec![Write], "one document is not everything");
    }

    #[test]
    fn mongo_json_documents_and_shorthands() {
        assert_eq!(mongo("{\"find\": \"users\", \"limit\": 20}"), vec![Read]);
        assert_eq!(mongo("{\"listCollections\": 1}"), vec![Read]);
        assert_eq!(mongo("{\"insert\": \"users\", \"documents\": [{\"a\": 1}]}"), vec![Write]);
        assert_eq!(mongo("{\"explain\": {\"find\": \"users\"}}"), vec![Read], "explain is as safe as what it wraps");
        assert_eq!(mongo("find users {\"a\": 1}"), vec![Read]);
        assert_eq!(mongo("count users"), vec![Read]);
        assert_eq!(mongo("{\"find\": \"a\"}\n\n{\"drop\": \"a\"}"), vec![Read, Destructive], "blank lines cut JSON blocks apart");
    }

    // The structure commands of an engine's own shell are reads in every language, so a
    // read-only connection answers `\dt` and `help`; whatever sits beside them is judged as before.
    #[test]
    fn shell_commands_are_reads_and_their_neighbours_keep_their_verdict() {
        assert_eq!(kinds(Engine::Postgres, "\\dt\nselect 1"), vec![Read, Read]);
        assert_eq!(kinds(Engine::Postgres, "\\dt\nDELETE FROM t"), vec![Read, Destructive]);
        assert_eq!(kinds(Engine::Supabase, "\\d users;\nUPDATE t SET a = 1 WHERE b = 2"), vec![Read, Write]);
        assert_eq!(kinds(Engine::Sqlite, ".tables\n.schema users\nINSERT INTO t VALUES (1)"), vec![Read, Read, Write]);
        assert_eq!(kinds(Engine::Duckdb, ".tables"), vec![Read]);
        assert_eq!(kinds(Engine::Mysql, "\\s"), vec![Read]);
        assert_eq!(kinds(Engine::Oracle, "SHOW USER\nDROP TABLE t"), vec![Read, Destructive]);
        assert_eq!(kinds(Engine::Oracle, "DESC emp;"), vec![Read]);
        assert_eq!(kinds(Engine::Cassandra, "DESCRIBE KEYSPACES\nTRUNCATE t"), vec![Read, Destructive]);
        assert_eq!(kinds(Engine::Neo4j, ":schema\nMATCH (n) DETACH DELETE n"), vec![Read, Write]);
        assert_eq!(redis("help\nGET a\nSET a 1"), vec![Read, Read, Write]);
        assert_eq!(redis("help INFO"), vec![Read]);
        assert_eq!(mongo("help"), vec![Read]);
        assert_eq!(mongo("help\ndb.users.insertOne({ a: 1 })"), vec![Read, Write]);
        assert_eq!(mongo("show users\nshow roles"), vec![Read, Read]);
        // A command the engine's shell does not have is still never sent: it is refused as a read.
        assert_eq!(kinds(Engine::Postgres, "\\copy t from stdin"), vec![Read]);
        // Another shell's command is just text to this engine, and fails closed like any unknown word.
        assert_eq!(kinds(Engine::Sqlite, "\\dt"), vec![Write]);
    }

    #[test]
    fn mongo_text_the_adapter_cannot_parse_fails_closed_with_a_reason() {
        for script in ["{not json", "[1, 2]", "drop users", "db.users.find(", "frobnicate"] {
            let found = classify_for(Engine::Mongodb, script);
            assert_eq!(found.first().map(|s| s.kind), Some(Write), "{script}");
            assert!(found.first().and_then(|s| s.reason.as_deref()).is_some(), "{script}");
        }
        assert!(classify_for(Engine::Mongodb, "  \n\n ").is_empty());
    }
}
