// SOT: shell-meta-milvus, milvus-cli-structure-commands, zilliz-structure-commands

use super::args::{eq_ci, raw_args, split_word, unquote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};
use serde_json::{json, Value as Json};

// ============================================================================
// MILVUS (and Zilliz Cloud)
//
// WHAT:  The structure commands of `milvus_cli`, Milvus's interactive shell:
//        `list collections`, `list databases`, `list users`, `list roles`,
//        `list indexes -c <collection>`, `list partitions -c <collection>`,
//        `show collection -c <collection>`, `show index -c <collection> -in <index>`
//        and `show partition -c <collection> -p <partition>` (the current names of
//        the three describing commands) with their older spelling `describe ...`.
// WHY:   People who run Milvus know `milvus_cli`; its listing and describe
//        commands are exactly the questions the object explorer answers, so the
//        query tab answers them too instead of failing with "Unknown command".
// HOW:   Each command becomes the console's raw envelope `{"path": ..., "body": ...}`
//        built with `serde_json`, so a name is always a JSON string and can never
//        change the request. `parse_command` in integrations/milvus.rs turns the
//        envelope into `Command::Raw`, which `call` posts under `/v2/vectordb/` with
//        the connection's `dbName`; every path used here is a `*/list`, `*/describe`
//        or `*/get_stats` read, none of which `Command::is_mutation` flags.
//        `list collections` uses the console's own `COLLECTIONS` shorthand, which
//        shows the names as a table. `describe collection` etc. are claimed only
//        when a flag follows, so the native `DESCRIBE <collection>` of a collection
//        that happens to be called `collection` is untouched; `show collection` etc.
//        have no native meaning and are claimed whenever the kind is one of the three
//        (the other `show` commands of the CLI are sent on as typed).
// WHERE: src-tauri/src/integrations/milvus.rs (`parse_command`, `call`, `run`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | ?", "List the commands this console understands"),
    ShellCommand::shell("list collections", "list collections", "Every collection of the connected database"),
    ShellCommand::shell("list databases", "list databases", "Every database of the server"),
    ShellCommand::shell("list users", "list users", "Database users"),
    ShellCommand::shell("list roles", "list roles", "Roles"),
    ShellCommand::shell("list indexes -c", "list indexes -c <collection>", "Indexes of a collection"),
    ShellCommand::shell("list partitions -c", "list partitions -c <collection>", "Partitions of a collection"),
    ShellCommand::shell("show collection -c", "show collection -c <collection>", "Schema, fields and properties of a collection"),
    ShellCommand::shell("show index -c", "show index -c <collection> -in <index>", "Type, metric and parameters of an index"),
    ShellCommand::shell("show partition -c", "show partition -c <collection> -p <partition>", "Statistics of a partition"),
    ShellCommand::shell("describe collection -c", "describe collection -c <collection>", "Older spelling of show collection"),
    ShellCommand::shell("describe index -c", "describe index -c <collection> -in <index>", "Older spelling of show index"),
    ShellCommand::shell("describe partition -c", "describe partition -c <collection> -p <partition>", "Older spelling of show partition"),
    ShellCommand::native("COLLECTIONS", "COLLECTIONS", "Every collection"),
    ShellCommand::native("DESCRIBE", "DESCRIBE <collection>", "Schema, fields and properties of a collection"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    if DIALECT.help.iter().any(|h| eq_ci(h, text)) {
        return Some(Ok(help_plan(&DIALECT)));
    }
    let (head, rest) = split_word(text);
    if eq_ci(head, "list") {
        return Some(list(rest));
    }
    let describing = ["collection", "index", "partition"];
    if eq_ci(head, "describe") {
        let (kind, flags) = split_word(rest);
        // Only `describe <kind> -c ...`; a bare `describe collection` is the native shorthand.
        if flags.starts_with('-') && describing.iter().any(|k| eq_ci(k, kind)) {
            return Some(describe("describe", &kind.to_ascii_lowercase(), flags));
        }
    }
    if eq_ci(head, "show") {
        let (kind, flags) = split_word(rest);
        if describing.iter().any(|k| eq_ci(k, kind)) {
            return Some(describe("show", &kind.to_ascii_lowercase(), flags));
        }
    }
    None
}

/// The console's raw envelope for one management call.
fn call(path: &str, body: Json) -> Plan {
    Plan::native(json!({ "path": path, "body": body }).to_string())
}

fn list(rest: &str) -> AppResult<Plan> {
    let (what, flags) = split_word(rest);
    match what.to_ascii_lowercase().as_str() {
        "collections" => no_flags(flags, "list collections").map(|()| Plan::native("COLLECTIONS")),
        "databases" => no_flags(flags, "list databases").map(|()| call("databases/list", json!({}))),
        "users" => no_flags(flags, "list users").map(|()| call("users/list", json!({}))),
        "roles" => no_flags(flags, "list roles").map(|()| call("roles/list", json!({}))),
        "indexes" => {
            let collection = collection_only(flags, "list indexes")?;
            Ok(call("indexes/list", json!({ "collectionName": collection })))
        }
        "partitions" => {
            let collection = collection_only(flags, "list partitions")?;
            Ok(call("partitions/list", json!({ "collectionName": collection })))
        }
        _ => Err(AppError::invalid_input("Say what to list: collections, databases, users, roles, indexes -c <collection> or partitions -c <collection>.")),
    }
}

// WHAT:  `show|describe <kind> -c <collection> ...` (the CLI renamed `describe` to `show`; both are read).
fn describe(verb: &str, kind: &str, flags: &str) -> AppResult<Plan> {
    let mut tokens = tokens_of(flags)?;
    let collection = take_flag(&mut tokens, &["-c", "--collection-name", "--collection"])?
        .ok_or_else(|| AppError::invalid_input(format!("{verb} {kind} needs the collection: -c <collection>.")))?;
    match kind {
        "collection" => {
            leftovers(&tokens, &format!("{verb} collection"))?;
            Ok(call("collections/describe", json!({ "collectionName": collection })))
        }
        "index" => {
            let index = take_flag(&mut tokens, &["-in", "--index-name", "--index"])?
                .ok_or_else(|| AppError::invalid_input(format!("{verb} index needs the index: -in <index>.")))?;
            leftovers(&tokens, &format!("{verb} index"))?;
            Ok(call("indexes/describe", json!({ "collectionName": collection, "indexName": index })))
        }
        _ => {
            let partition = take_flag(&mut tokens, &["-p", "--partition", "--partition-name"])?
                .ok_or_else(|| AppError::invalid_input(format!("{verb} partition needs the partition: -p <partition>.")))?;
            leftovers(&tokens, &format!("{verb} partition"))?;
            Ok(call("partitions/get_stats", json!({ "collectionName": collection, "partitionName": partition })))
        }
    }
}

fn no_flags(rest: &str, command: &str) -> AppResult<()> {
    leftovers(&tokens_of(rest)?, command)
}

fn collection_only(flags: &str, command: &str) -> AppResult<String> {
    let mut tokens = tokens_of(flags)?;
    let collection = take_flag(&mut tokens, &["-c", "--collection-name", "--collection"])?
        .ok_or_else(|| AppError::invalid_input(format!("{command} needs the collection: -c <collection>.")))?;
    leftovers(&tokens, command)?;
    Ok(collection)
}

fn tokens_of(rest: &str) -> AppResult<Vec<String>> {
    raw_args(rest).map_err(AppError::invalid_input)
}

fn leftovers(tokens: &[String], command: &str) -> AppResult<()> {
    match tokens.first() {
        None => Ok(()),
        Some(extra) => Err(AppError::invalid_input(format!("{command} does not take {extra}."))),
    }
}

// WHAT:  Removes `-c value` / `--collection-name=value` from the arguments and returns the value.
fn take_flag(tokens: &mut Vec<String>, names: &[&str]) -> AppResult<Option<String>> {
    let Some(at) = tokens.iter().position(|t| names.iter().any(|n| flag_name(t) == *n)) else {
        return Ok(None);
    };
    let token = tokens.remove(at);
    let value = match token.split_once('=') {
        Some((_, inline)) if token.starts_with("--") => unquote(inline),
        _ if at < tokens.len() => unquote(&tokens.remove(at)),
        _ => return Err(AppError::invalid_input(format!("{} needs a value.", flag_name(&token)))),
    };
    if value.is_empty() {
        return Err(AppError::invalid_input(format!("{} needs a value.", flag_name(&token))));
    }
    Ok(Some(value))
}

fn flag_name(token: &str) -> &str {
    if token.starts_with("--") { token.split('=').next().unwrap_or(token) } else { token }
}

#[cfg(test)]
mod tests {
    use super::super::{plan, CommandOrigin, Ctx, Step};
    use crate::model::Engine;
    use serde_json::{json, Value};

    fn statements(text: &str) -> Vec<String> {
        match plan(&Ctx::bare(Engine::Milvus), text) {
            Some(Ok(p)) => p.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn envelope(text: &str) -> Value {
        let found = statements(text);
        assert_eq!(found.len(), 1, "{text}");
        serde_json::from_str(&found[0]).unwrap_or(Value::Null)
    }

    fn refused(text: &str) -> String {
        match plan(&Ctx::bare(Engine::Milvus), text) {
            Some(Err(e)) => e.message().to_string(),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn help_lists_every_command() {
        for word in ["help", "?", "HELP;"] {
            let found = plan(&Ctx::bare(Engine::Milvus), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}");
        }
    }

    #[test]
    fn listings_become_the_consoles_own_calls() {
        assert_eq!(statements("list collections"), ["COLLECTIONS"]);
        assert_eq!(statements("LIST   Collections;"), ["COLLECTIONS"]);
        assert_eq!(envelope("list databases"), json!({ "path": "databases/list", "body": {} }));
        assert_eq!(envelope("list users"), json!({ "path": "users/list", "body": {} }));
        assert_eq!(envelope("list roles"), json!({ "path": "roles/list", "body": {} }));
        assert_eq!(envelope("list indexes -c books"), json!({ "path": "indexes/list", "body": { "collectionName": "books" } }));
        assert_eq!(envelope("list partitions --collection-name=books"), json!({ "path": "partitions/list", "body": { "collectionName": "books" } }));
    }

    #[test]
    fn describe_commands_name_what_they_describe() {
        assert_eq!(envelope("describe collection -c books"), json!({ "path": "collections/describe", "body": { "collectionName": "books" } }));
        assert_eq!(envelope("describe index -c books -in idx_vec"), json!({ "path": "indexes/describe", "body": { "collectionName": "books", "indexName": "idx_vec" } }));
        assert_eq!(envelope("Describe Index --index-name idx -c books"), json!({ "path": "indexes/describe", "body": { "collectionName": "books", "indexName": "idx" } }));
        assert_eq!(envelope("describe partition -c books -p _default"), json!({ "path": "partitions/get_stats", "body": { "collectionName": "books", "partitionName": "_default" } }));
    }

    #[test]
    fn show_is_the_current_spelling_of_describe() {
        assert_eq!(envelope("show collection -c books"), envelope("describe collection -c books"));
        assert_eq!(envelope("SHOW INDEX -c books -in idx_vec"), envelope("describe index -c books -in idx_vec"));
        assert_eq!(envelope("show partition -c books --partition _default"), json!({ "path": "partitions/get_stats", "body": { "collectionName": "books", "partitionName": "_default" } }));
        // Without its flag the old spelling is the native DESCRIBE, but `show` has no native meaning.
        assert!(refused("show collection").contains("show collection needs the collection"));
        assert!(refused("show index -c books").contains("show index needs the index"));
        // The other `show` commands of the CLI are not translated.
        for text in ["show collection_stats -c books", "show loading_progress -c books", "show database", "show"] {
            assert!(plan(&Ctx::bare(Engine::Milvus), text).is_none(), "{text}");
        }
    }

    #[test]
    fn every_call_is_a_read_the_adapter_does_not_gate() {
        for text in ["list databases", "list users", "list roles", "list indexes -c a", "list partitions -c a", "describe collection -c a", "describe index -c a -in b", "describe partition -c a -p b", "show collection -c a", "show index -c a -in b", "show partition -c a -p b"] {
            let path = envelope(text)["path"].as_str().unwrap_or_default().to_ascii_lowercase();
            for write in ["/insert", "/upsert", "/delete", "/create", "/drop", "/alter", "/load", "/release", "/rename", "/flush", "/compact", "/grant", "/revoke"] {
                assert!(!path.ends_with(write), "{text}: {path}");
            }
        }
    }

    #[test]
    fn hostile_names_stay_inside_their_json_string() {
        let name = "x\",\"collectionName\":\"y\\\"}{";
        let found = envelope(&format!("describe collection -c '{}'", name.replace('\'', "''")));
        assert_eq!(found["body"]["collectionName"], json!(name));
        assert_eq!(found["body"].as_object().map(|o| o.len()), Some(1));
        let quoted = envelope("list indexes -c \"a b\"");
        assert_eq!(quoted["body"]["collectionName"], json!("a b"));
    }

    #[test]
    fn missing_or_extra_arguments_say_how_to_fix_them() {
        assert!(refused("list indexes").contains("-c"));
        assert!(refused("list indexes -c").contains("needs a value"));
        assert!(refused("describe index -c books").contains("-in"));
        assert!(refused("describe partition -c books").contains("-p"));
        assert!(refused("describe collection -c").contains("needs a value"));
        assert!(refused("list collections now").contains("does not take"));
        assert!(refused("list widgets").contains("Say what to list"));
        assert!(refused("describe collection -c 'a").contains("quote"));
        assert!(refused("describe collection -x 1").contains("-c"));
    }

    #[test]
    fn native_shorthands_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Milvus), native.name).is_none(), "{}", native.name);
        }
        for text in ["DESCRIBE books", "describe collection", "describe index", "QUERY books id > 3", "{\"collection\": \"books\", \"query\": {}}", "COLLECTIONS"] {
            assert!(plan(&Ctx::bare(Engine::Milvus), text).is_none(), "{text}");
        }
    }
}
