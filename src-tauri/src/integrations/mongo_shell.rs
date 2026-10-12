// SOT: mongo-shell-parser, mongosh-translation, mongo-read-command-allowlist

use base64::Engine as _;
use serde_json::{json, Map, Value as Json};

// WHAT:  Translates the mongosh statements people actually type into the command
//        documents the MongoDB adapter already runs:
//        `db.users.find({ age: { $gt: 30 } }).sort({ age: -1 }).limit(10)`,
//        `db.orders.insertMany([...])`, `show collections`, `use sales`…
// WHY:   Mongo users think in the shell, not in `{ "find": "users", "filter": … }`.
//        Pasting a snippet from the docs, a Stack Overflow answer or a colleague
//        should just work, in the same console that takes raw command documents.
// HOW:   A hand-written recursive-descent reader for the JavaScript subset those
//        snippets use (relaxed object literals, single quotes, trailing commas,
//        comments, regex literals and the BSON constructors ObjectId, ISODate,
//        NumberLong…), producing extended JSON that the adapter turns into BSON.
//        Nothing is evaluated: a statement is parsed, never run as JavaScript.
//        This file is pure text work; the BSON conversion and the driver stay in
//        `mongodb.rs`, the only place that imports that crate.
// WHERE: src-tauri/src/integrations/mongodb.rs (`execute`)

/// One statement of a shell session, understood.
#[derive(Debug, Clone, PartialEq)]
pub enum Shell {
    ShowDatabases,
    ShowCollections,
    /// `use <db>`: the following statements of the same run target that database.
    Use(String),
    /// A command document, with `"$db"` set when the statement named another database.
    Command(Json),
}

/// True when `text` is shell syntax rather than a JSON command document.
pub fn is_shell(text: &str) -> bool {
    let t = code_start(text);
    t.starts_with("db.") || t.starts_with("db[") || starts_with_word(t, "show") || starts_with_word(t, "use")
}

/// `text` from its first character that is not whitespace or part of a comment.
fn code_start(text: &str) -> &str {
    let mut rest = text.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("//") {
            rest = after.find('\n').map_or("", |at| &after[at..]).trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |at| &after[at + 2..]).trim_start();
        } else {
            return rest;
        }
    }
}

fn starts_with_word(text: &str, word: &str) -> bool {
    text.strip_prefix(word).is_some_and(|rest| rest.chars().next().is_some_and(char::is_whitespace))
}

// ---------------------------------------------------------------------------
// Splitting a session into statements
// ---------------------------------------------------------------------------

// WHAT:  Cuts shell text into statements: at a top-level `;`, at a blank line, or
//        at a line break after a finished statement when the next line starts a
//        new one (`db.`, `show`, `use`). A call chain continued on the next line
//        (`.sort(...)` under `.find(...)`) stays one statement.
// HOW:   Brackets, strings, comments and regex literals are skipped whole, so a
//        `;` or a newline inside any of them is never a boundary.
pub fn split(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start = 0_usize;
    let mut i = 0_usize;
    let mut depth = 0_i32;
    let mut last: Option<char> = None;
    let push = |from: usize, to: usize, out: &mut Vec<String>| {
        let piece: String = chars[from..to.min(chars.len())].iter().collect();
        let piece = piece.trim();
        // A piece that is only a comment has nothing to run.
        if !code_start(piece).is_empty() {
            out.push(piece.to_string());
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '\'' | '"' | '`' => {
                i = skip_string(&chars, i);
                last = Some(c);
                continue;
            }
            '/' if next == Some('/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if next == Some('*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1).copied() == Some('/')) {
                    i += 1;
                }
                i += 2;
                continue;
            }
            '/' if regex_may_start(last) => {
                i = skip_regex(&chars, i);
                last = Some('/');
                continue;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ';' if depth <= 0 => {
                push(start, i, &mut out);
                i += 1;
                start = i;
                last = None;
                continue;
            }
            '\n' if depth <= 0 && last.is_some_and(|l| l == ')' || l.is_alphanumeric() || l == '_') => {
                let mut j = i + 1;
                let mut blank_line = false;
                while j < chars.len() && chars[j].is_whitespace() {
                    blank_line |= chars[j] == '\n';
                    j += 1;
                }
                let rest: String = chars[j..].iter().take(8).collect();
                if blank_line || is_shell(&rest) {
                    push(start, i, &mut out);
                    start = j;
                    last = None;
                    i = j;
                    continue;
                }
            }
            _ => {}
        }
        if !c.is_whitespace() {
            last = Some(c);
        }
        i += 1;
    }
    push(start, chars.len(), &mut out);
    out
}

/// Index just past the string that opens at `start` (a quote), honouring `\` escapes.
fn skip_string(chars: &[char], start: usize) -> usize {
    let quote = chars[start];
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

/// A `/` starts a regex literal where a value can start, never after a value.
fn regex_may_start(last: Option<char>) -> bool {
    last.is_none_or(|c| matches!(c, '(' | ',' | ':' | '[' | '{' | '=' | '!' | '&' | '|' | '?' | ';'))
}

/// Index just past the regex literal (and its flags) that opens at `start`.
fn skip_regex(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;
    let mut in_class = false;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                return i;
            }
            '\n' => return i,
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

// WHAT:  One shell statement -> what to run. `max_rows` bounds a `find` that has no `.limit()`.
pub fn parse(text: &str, max_rows: usize) -> Result<Shell, String> {
    let statement = code_start(text).trim_end().trim_end_matches(';').trim_end();
    let mut words = statement.split_whitespace();
    match words.next() {
        Some("show") => match words.next() {
            Some("dbs" | "databases") => Ok(Shell::ShowDatabases),
            Some("collections" | "tables") => Ok(Shell::ShowCollections),
            // The structure listings of mongosh beyond databases and collections, as the
            // read commands they run: users and roles of the current database, the last
            // profiler entries, and the server's log names (`show logs`) or one log.
            Some("users") => Ok(Shell::Command(json!({ "usersInfo": 1 }))),
            Some("roles") => Ok(Shell::Command(json!({ "rolesInfo": 1, "showBuiltinRoles": true }))),
            Some("profile") => Ok(Shell::Command(json!({ "find": "system.profile", "filter": {}, "sort": { "$natural": -1 }, "limit": 5 }))),
            Some("logs") => Ok(Shell::Command(json!({ "getLog": "*", "$db": "admin" }))),
            Some("log") => {
                let name = words.next().unwrap_or("global");
                Ok(Shell::Command(json!({ "getLog": name, "$db": "admin" })))
            }
            Some(other) => Err(format!("`show {other}` is not supported. Try `show dbs`, `show collections`, `show users`, `show roles`, `show profile` or `show logs`.")),
            None => Err("Say what to show: `show dbs`, `show collections`, `show users`, `show roles`, `show profile` or `show logs`.".to_string()),
        },
        Some("use") => {
            let name = statement["use".len()..].trim().trim_matches(['\'', '"', '`']);
            if name.is_empty() || name.contains(char::is_whitespace) {
                Err("`use` takes one database name, e.g. `use sales`.".to_string())
            } else {
                Ok(Shell::Use(name.to_string()))
            }
        }
        _ => parse_call(statement, max_rows),
    }
}

type Call = (String, Vec<Json>);

/// Methods that may follow a query: cursor modifiers, plus the ones that only shape the output.
const CURSOR_METHODS: &[&str] = &[
    "sort", "limit", "skip", "project", "projection", "hint", "collation", "maxTimeMS", "count", "size", "itcount", "toArray", "pretty",
    "batchSize", "noCursorTimeout", "allowDiskUse", "readPref", "comment",
];

fn parse_call(statement: &str, max_rows: usize) -> Result<Shell, String> {
    let mut p = Parser::new(statement);
    p.expect_word("db")?;
    let mut collection: Vec<String> = Vec::new();
    let mut target: Option<String> = None;
    let (method, args) = loop {
        p.skip_ws();
        match p.peek() {
            Some('.') => {
                p.bump();
                let name = p.ident().ok_or_else(|| p.error("Expected a name after `.`"))?;
                p.skip_ws();
                if p.peek() == Some('(') {
                    let args = p.args()?;
                    match name.as_str() {
                        "getCollection" => collection.push(single_string(&args, "getCollection")?),
                        "getSiblingDB" => target = Some(single_string(&args, "getSiblingDB")?),
                        _ => break (name, args),
                    }
                } else {
                    collection.push(name);
                }
            }
            Some('[') => {
                p.bump();
                p.skip_ws();
                collection.push(p.string()?);
                p.expect(']')?;
            }
            _ => return Err(p.error("Expected `.method(...)` after `db`")),
        }
    };
    let mut chain: Vec<Call> = Vec::new();
    loop {
        p.skip_ws();
        if p.peek() != Some('.') {
            break;
        }
        p.bump();
        let name = p.ident().ok_or_else(|| p.error("Expected a method name after `.`"))?;
        // Said before the arguments are read: `.forEach(printjson)` has no value to understand there.
        if !CURSOR_METHODS.contains(&name.as_str()) {
            return Err(format!("`.{name}()` after a query is not supported here. Supported: sort, limit, skip, project, hint, collation, maxTimeMS, count, toArray, pretty."));
        }
        p.skip_ws();
        chain.push((name, p.args()?));
    }
    p.skip_ws();
    if !p.at_end() {
        return Err(p.error("Unexpected text after the statement"));
    }
    let coll = (!collection.is_empty()).then(|| collection.join("."));
    let mut command = match coll.as_deref() {
        Some(c) => collection_command(c, &method, &args, &chain, max_rows)?,
        None => database_command(&method, &args, &chain)?,
    };
    if let (Some(db), Some(map)) = (target, command.as_object_mut()) {
        map.insert("$db".to_string(), Json::String(db));
    }
    Ok(Shell::Command(command))
}

fn single_string(args: &[Json], method: &str) -> Result<String, String> {
    match args {
        [Json::String(s)] if !s.is_empty() => Ok(s.clone()),
        _ => Err(format!("`{method}` takes one name, e.g. {method}(\"users\").")),
    }
}

// ---------------------------------------------------------------------------
// Method -> command document
// ---------------------------------------------------------------------------

/// The argument at `index` as an object; absent, `null` and `undefined` mean "none".
fn doc_arg(args: &[Json], index: usize, what: &str) -> Result<Option<Map<String, Json>>, String> {
    match args.get(index) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(map)) => Ok(Some(map.clone())),
        Some(_) => Err(format!("{what} must be an object, e.g. {{ field: value }}.")),
    }
}

fn required_doc(args: &[Json], index: usize, what: &str) -> Result<Map<String, Json>, String> {
    doc_arg(args, index, what)?.ok_or_else(|| format!("{what} is required."))
}

fn int_arg(args: &[Json], what: &str) -> Result<i64, String> {
    args.first().and_then(Json::as_i64).ok_or_else(|| format!("{what} takes a whole number."))
}

fn put(cmd: &mut Map<String, Json>, key: &str, value: Json) {
    cmd.insert(key.to_string(), value);
}

/// Copies the options a command understands from the options object into the command.
fn copy_options(cmd: &mut Map<String, Json>, options: &Option<Map<String, Json>>, keys: &[&str]) {
    if let Some(options) = options {
        for key in keys {
            if let Some(value) = options.get(*key) {
                cmd.insert((*key).to_string(), value.clone());
            }
        }
    }
}

fn collection_command(coll: &str, method: &str, args: &[Json], chain: &[Call], max_rows: usize) -> Result<Json, String> {
    let mut cmd = Map::new();
    match method {
        "find" => return find_command(coll, args, chain, max_rows, false),
        "findOne" => return find_command(coll, args, chain, max_rows, true),
        "aggregate" => {
            reject_unsupported_chain(chain, "aggregate", &["toArray", "pretty", "itcount"])?;
            let pipeline = match args.first() {
                Some(Json::Array(stages)) => Json::Array(stages.clone()),
                _ => return Err("aggregate() takes a pipeline array, e.g. [{ $match: { … } }].".to_string()),
            };
            put(&mut cmd, "aggregate", json!(coll));
            put(&mut cmd, "pipeline", pipeline);
            put(&mut cmd, "cursor", json!({}));
            let options = doc_arg(args, 1, "aggregate options")?;
            copy_options(&mut cmd, &options, &["allowDiskUse", "collation", "hint", "comment", "maxTimeMS", "readConcern", "let"]);
        }
        "countDocuments" => {
            put(&mut cmd, "count", json!(coll));
            put(&mut cmd, "query", Json::Object(doc_arg(args, 0, "The filter")?.unwrap_or_default()));
            copy_options(&mut cmd, &doc_arg(args, 1, "countDocuments options")?, &["limit", "skip", "hint", "maxTimeMS", "collation"]);
        }
        "estimatedDocumentCount" => put(&mut cmd, "count", json!(coll)),
        "distinct" => {
            let key = match args.first() {
                Some(Json::String(field)) if !field.is_empty() => field.clone(),
                _ => return Err("distinct() takes the field name first, e.g. distinct(\"country\").".to_string()),
            };
            put(&mut cmd, "distinct", json!(coll));
            put(&mut cmd, "key", json!(key));
            if let Some(query) = doc_arg(args, 1, "The filter")? {
                put(&mut cmd, "query", Json::Object(query));
            }
        }
        "insertOne" => {
            put(&mut cmd, "insert", json!(coll));
            put(&mut cmd, "documents", Json::Array(vec![Json::Object(required_doc(args, 0, "The document to insert")?)]));
            copy_options(&mut cmd, &doc_arg(args, 1, "insertOne options")?, &["writeConcern", "bypassDocumentValidation"]);
        }
        "insertMany" => {
            let documents = match args.first() {
                Some(Json::Array(docs)) if !docs.is_empty() && docs.iter().all(Json::is_object) => docs.clone(),
                _ => return Err("insertMany() takes a non-empty array of documents.".to_string()),
            };
            let options = doc_arg(args, 1, "insertMany options")?;
            put(&mut cmd, "insert", json!(coll));
            put(&mut cmd, "documents", Json::Array(documents));
            put(&mut cmd, "ordered", options.as_ref().and_then(|o| o.get("ordered")).cloned().unwrap_or(json!(true)));
            copy_options(&mut cmd, &options, &["writeConcern", "bypassDocumentValidation"]);
        }
        "updateOne" | "updateMany" => {
            let filter = required_doc(args, 0, "The filter")?;
            let update = match args.get(1) {
                Some(Json::Object(map)) if !map.is_empty() && map.keys().all(|k| k.starts_with('$')) => Json::Object(map.clone()),
                Some(Json::Array(stages)) => Json::Array(stages.clone()),
                _ => return Err(format!("{method}() needs an update with operators such as {{ $set: {{ … }} }}. Use replaceOne() to replace a whole document.")),
            };
            let options = doc_arg(args, 2, "update options")?;
            put(&mut cmd, "update", json!(coll));
            put(&mut cmd, "updates", Json::Array(vec![update_spec(filter, update, method == "updateMany", &options)]));
            copy_options(&mut cmd, &options, &["writeConcern", "bypassDocumentValidation"]);
        }
        "replaceOne" => {
            let filter = required_doc(args, 0, "The filter")?;
            let replacement = required_doc(args, 1, "The replacement document")?;
            if replacement.keys().any(|k| k.starts_with('$')) {
                return Err("replaceOne() takes a plain document, not update operators. Use updateOne() for $set and friends.".to_string());
            }
            let options = doc_arg(args, 2, "replaceOne options")?;
            put(&mut cmd, "update", json!(coll));
            put(&mut cmd, "updates", Json::Array(vec![update_spec(filter, Json::Object(replacement), false, &options)]));
        }
        "deleteOne" | "deleteMany" => {
            if args.is_empty() {
                return Err(format!("{method}() needs a filter. Pass {{}} to match every document."));
            }
            let filter = required_doc(args, 0, "The filter")?;
            let mut spec = Map::new();
            put(&mut spec, "q", Json::Object(filter));
            put(&mut spec, "limit", json!(i64::from(method == "deleteOne")));
            copy_options(&mut spec, &doc_arg(args, 1, "delete options")?, &["collation", "hint"]);
            put(&mut cmd, "delete", json!(coll));
            put(&mut cmd, "deletes", Json::Array(vec![Json::Object(spec)]));
        }
        "findOneAndUpdate" | "findOneAndReplace" | "findOneAndDelete" => {
            let delete = method == "findOneAndDelete";
            let options = doc_arg(args, if delete { 1 } else { 2 }, "options")?;
            put(&mut cmd, "findAndModify", json!(coll));
            put(&mut cmd, "query", Json::Object(required_doc(args, 0, "The filter")?));
            if delete {
                put(&mut cmd, "remove", json!(true));
            } else {
                let body = required_doc(args, 1, "The update")?;
                if method == "findOneAndUpdate" && !body.keys().all(|k| k.starts_with('$')) {
                    return Err("findOneAndUpdate() needs update operators such as { $set: { … } }.".to_string());
                }
                put(&mut cmd, "update", Json::Object(body));
                let after = options.as_ref().and_then(|o| o.get("returnDocument")).and_then(Json::as_str) == Some("after");
                put(&mut cmd, "new", json!(after));
                if let Some(upsert) = options.as_ref().and_then(|o| o.get("upsert")) {
                    put(&mut cmd, "upsert", upsert.clone());
                }
            }
            if let Some(sort) = options.as_ref().and_then(|o| o.get("sort")) {
                put(&mut cmd, "sort", sort.clone());
            }
            if let Some(fields) = options.as_ref().and_then(|o| o.get("projection")) {
                put(&mut cmd, "fields", fields.clone());
            }
        }
        "drop" => put(&mut cmd, "drop", json!(coll)),
        "getIndexes" => put(&mut cmd, "listIndexes", json!(coll)),
        "stats" => put(&mut cmd, "collStats", json!(coll)),
        "validate" => put(&mut cmd, "validate", json!(coll)),
        // `exists()` answers the collection's own entry in the listing, or nothing.
        "exists" => {
            put(&mut cmd, "listCollections", json!(1));
            put(&mut cmd, "filter", json!({ "name": coll }));
        }
        "createIndex" => {
            let keys = required_doc(args, 0, "The index keys")?;
            let options = doc_arg(args, 1, "index options")?;
            let name = options.as_ref().and_then(|o| o.get("name")).and_then(Json::as_str).map_or_else(|| index_name(&keys), str::to_string);
            let mut spec = options.unwrap_or_default();
            put(&mut spec, "key", Json::Object(keys));
            put(&mut spec, "name", json!(name));
            put(&mut cmd, "createIndexes", json!(coll));
            put(&mut cmd, "indexes", Json::Array(vec![Json::Object(spec)]));
        }
        "dropIndex" => {
            let index = match args.first() {
                Some(name @ (Json::String(_) | Json::Object(_))) => name.clone(),
                _ => return Err("dropIndex() takes an index name or its key pattern.".to_string()),
            };
            put(&mut cmd, "dropIndexes", json!(coll));
            put(&mut cmd, "index", index);
        }
        other => return Err(format!("`db.{coll}.{other}()` is not supported in this console. Run it as a command document, e.g. {{ \"{other}\": \"{coll}\" }}.")),
    }
    if !matches!(method, "aggregate") {
        reject_unsupported_chain(chain, method, &["toArray", "pretty"])?;
    }
    Ok(Json::Object(cmd))
}

fn update_spec(filter: Map<String, Json>, update: Json, multi: bool, options: &Option<Map<String, Json>>) -> Json {
    let mut spec = Map::new();
    put(&mut spec, "q", Json::Object(filter));
    put(&mut spec, "u", update);
    put(&mut spec, "multi", json!(multi));
    copy_options(&mut spec, options, &["upsert", "arrayFilters", "collation", "hint"]);
    Json::Object(spec)
}

/// `{ a: 1, b: -1 }` -> `a_1_b_-1`, how the server names an index when none is given.
fn index_name(keys: &Map<String, Json>) -> String {
    keys.iter()
        .map(|(field, direction)| format!("{field}_{}", direction.as_str().map_or_else(|| direction.to_string(), str::to_string)))
        .collect::<Vec<_>>()
        .join("_")
}

fn reject_unsupported_chain(chain: &[Call], method: &str, allowed: &[&str]) -> Result<(), String> {
    match chain.iter().find(|(name, _)| !allowed.contains(&name.as_str())) {
        Some((name, _)) => Err(format!("`.{name}()` after {method}() is not supported here.")),
        None => Ok(()),
    }
}

// WHAT:  `find` / `findOne` with the cursor modifiers chained after them.
// HOW:   Without a `.limit()` the cap is `max_rows + 1`, one more than will be
//        shown, so the adapter can tell the user the result was cut. `.count()`
//        turns the whole statement into a count of the same filter.
fn find_command(coll: &str, args: &[Json], chain: &[Call], max_rows: usize, one: bool) -> Result<Json, String> {
    let filter = doc_arg(args, 0, "The filter")?.unwrap_or_default();
    let mut cmd = Map::new();
    put(&mut cmd, "find", json!(coll));
    put(&mut cmd, "filter", Json::Object(filter.clone()));
    if let Some(projection) = doc_arg(args, 1, "The projection")? {
        put(&mut cmd, "projection", Json::Object(projection));
    }
    let mut limit: Option<i64> = None;
    let mut count = false;
    for (name, a) in chain {
        match name.as_str() {
            "sort" => put(&mut cmd, "sort", Json::Object(required_doc(a, 0, "The sort document")?)),
            "limit" => limit = Some(int_arg(a, ".limit()")?),
            "skip" => put(&mut cmd, "skip", json!(int_arg(a, ".skip()")?)),
            "project" | "projection" => put(&mut cmd, "projection", Json::Object(required_doc(a, 0, "The projection")?)),
            "hint" => put(&mut cmd, "hint", a.first().cloned().unwrap_or(Json::Null)),
            "collation" => put(&mut cmd, "collation", Json::Object(required_doc(a, 0, "The collation")?)),
            "maxTimeMS" => put(&mut cmd, "maxTimeMS", json!(int_arg(a, ".maxTimeMS()")?)),
            "count" | "size" | "itcount" => count = true,
            "toArray" | "pretty" | "batchSize" | "noCursorTimeout" | "allowDiskUse" | "readPref" | "comment" => {}
            other => return Err(format!("`.{other}()` after find() is not supported here. Supported: sort, limit, skip, project, hint, collation, count.")),
        }
    }
    if count {
        let mut counted = Map::new();
        put(&mut counted, "count", json!(coll));
        put(&mut counted, "query", Json::Object(filter));
        for key in ["skip", "hint", "collation", "maxTimeMS"] {
            if let Some(value) = cmd.get(key) {
                put(&mut counted, key, value.clone());
            }
        }
        if let Some(limit) = limit {
            put(&mut counted, "limit", json!(limit));
        }
        return Ok(Json::Object(counted));
    }
    if one {
        put(&mut cmd, "limit", json!(1));
        put(&mut cmd, "singleBatch", json!(true));
    } else {
        let capped = i64::try_from(max_rows).ok().and_then(|rows| rows.checked_add(1));
        match (limit, capped) {
            (Some(user), Some(cap)) if user > 0 => put(&mut cmd, "limit", json!(user.min(cap))),
            (Some(_), _) => {}
            (None, Some(cap)) => put(&mut cmd, "limit", json!(cap)),
            (None, None) => {}
        }
    }
    Ok(Json::Object(cmd))
}

fn database_command(method: &str, args: &[Json], chain: &[Call]) -> Result<Json, String> {
    reject_unsupported_chain(chain, method, &["toArray", "pretty"])?;
    let mut cmd = Map::new();
    match method {
        "getCollectionNames" => {
            put(&mut cmd, "listCollections", json!(1));
            put(&mut cmd, "nameOnly", json!(true));
        }
        "getCollectionInfos" => {
            put(&mut cmd, "listCollections", json!(1));
            if let Some(filter) = doc_arg(args, 0, "The filter")? {
                put(&mut cmd, "filter", Json::Object(filter));
            }
        }
        "stats" => put(&mut cmd, "dbStats", json!(1)),
        "getUsers" => {
            put(&mut cmd, "usersInfo", json!(1));
            // Options narrow or widen the answer (`filter`, `showPrivileges`…): never dropped silently.
            copy_options(&mut cmd, &doc_arg(args, 0, "The options")?, &["filter", "showCredentials", "showPrivileges", "showAuthenticationRestrictions"]);
        }
        "getRoles" => {
            put(&mut cmd, "rolesInfo", json!(1));
            copy_options(&mut cmd, &doc_arg(args, 0, "The options")?, &["showBuiltinRoles", "showPrivileges"]);
        }
        "hostInfo" | "currentOp" | "listCommands" => {
            // `currentOp` takes a filter (or `true` for idle operations too); answering without it would be a wrong answer.
            if method == "currentOp" && !args.is_empty() {
                return Err("db.currentOp() takes no arguments here. Filter with a command document, e.g. db.adminCommand({ currentOp: 1, active: true }).".to_string());
            }
            put(&mut cmd, method, json!(1));
            if method != "listCommands" {
                put(&mut cmd, "$db", json!("admin"));
            }
        }
        // The profiler's level, read without changing it (-1 asks, any other value sets).
        "getProfilingStatus" => put(&mut cmd, "profile", json!(-1)),
        "version" => put(&mut cmd, "buildInfo", json!(1)),
        "serverStatus" => put(&mut cmd, "serverStatus", json!(1)),
        "hello" | "isMaster" => put(&mut cmd, "hello", json!(1)),
        "dropDatabase" => put(&mut cmd, "dropDatabase", json!(1)),
        "createCollection" => {
            let name = match args.first() {
                Some(Json::String(name)) if !name.is_empty() => name.clone(),
                _ => return Err("createCollection() takes the collection name first.".to_string()),
            };
            put(&mut cmd, "create", json!(name));
            if let Some(options) = doc_arg(args, 1, "The collection options")? {
                cmd.extend(options);
            }
        }
        "runCommand" | "adminCommand" => {
            cmd = match args.first() {
                Some(Json::Object(doc)) if !doc.is_empty() => doc.clone(),
                Some(Json::String(name)) if !name.is_empty() => Map::from_iter([(name.clone(), json!(1))]),
                _ => return Err(format!("{method}() takes a command document, e.g. {{ ping: 1 }}.")),
            };
            if method == "adminCommand" {
                put(&mut cmd, "$db", json!("admin"));
            }
        }
        other => return Err(format!("`db.{other}()` is not supported in this console. Run it with db.runCommand({{ … }}).")),
    }
    Ok(Json::Object(cmd))
}

// ---------------------------------------------------------------------------
// Read-only enforcement
// ---------------------------------------------------------------------------

// WHAT:  Whether a command document only reads.
// WHY:   The SQL guard cannot read Mongo commands, so a read-only connection is
//        enforced here, by name: anything not on this list is refused. Unknown
//        commands fail closed.
// HOW:   The command is its first key. An aggregation that ends in `$out` or
//        `$merge` writes, and `explain` is as safe as the command it wraps.
pub fn is_read_command(command: &Json) -> bool {
    let Some(map) = command.as_object() else {
        return false;
    };
    let Some((name, value)) = map.iter().next() else {
        return false;
    };
    match name.as_str() {
        "aggregate" => !map.get("pipeline").and_then(Json::as_array).is_some_and(|stages| {
            stages.iter().any(|stage| stage.as_object().is_some_and(|s| s.contains_key("$out") || s.contains_key("$merge")))
        }),
        "explain" => is_read_command(value),
        // `profile: -1` reads the profiler's level; any other value changes it, and so may
        // `slowms` / `sampleRate` / `filter` beside it: only the bare form is a read.
        "profile" => value.as_i64() == Some(-1) && map.keys().all(|key| key == "profile" || key == "$db"),
        // `validate` only reads, except for the options that repair indexes, documents or multikey metadata.
        "validate" => !["repair", "fixMultikey"].iter().any(|key| map.get(*key).is_some_and(is_truthy)),
        _ => READ_COMMANDS.contains(&name.as_str()),
    }
}

fn is_truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(on) => *on,
        Json::Number(n) => n.as_f64() != Some(0.0),
        _ => true,
    }
}

const READ_COMMANDS: &[&str] = &[
    "find", "count", "distinct", "listCollections", "listDatabases", "listIndexes", "ping", "hello", "isMaster", "ismaster",
    "buildInfo", "buildinfo", "dbStats", "dbstats", "collStats", "collstats", "serverStatus", "hostInfo", "getParameter",
    "getCmdLineOpts", "getLog", "usersInfo", "rolesInfo", "connectionStatus", "currentOp", "top", "validate", "whatsmyuri",
    "replSetGetStatus", "replSetGetConfig", "listShards", "dataSize", "dbHash", "listCommands", "getMore", "killCursors",
    "balancerStatus", "isdbgrid", "lockInfo", "features",
];

// ---------------------------------------------------------------------------
// The JavaScript-ish value reader
// ---------------------------------------------------------------------------

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(text: &str) -> Parser {
        Parser { chars: text.chars().collect(), pos: 0 }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        self.pos += 1;
        c
    }

    // WHAT:  A message that says where the reader stopped.
    fn error(&self, message: &str) -> String {
        let near: String = self.chars.iter().skip(self.pos).take(24).collect();
        let near = near.trim();
        if near.is_empty() {
            format!("{message} (at the end of the statement).")
        } else {
            format!("{message} near `{near}`.")
        }
    }

    fn skip_ws(&mut self) {
        loop {
            match (self.peek(), self.peek_at(1)) {
                (Some(c), _) if c.is_whitespace() => self.pos += 1,
                (Some('/'), Some('/')) => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.pos += 1;
                    }
                }
                (Some('/'), Some('*')) => {
                    self.pos += 2;
                    while !self.at_end() && !(self.peek() == Some('*') && self.peek_at(1) == Some('/')) {
                        self.pos += 1;
                    }
                    self.pos += 2;
                }
                _ => return,
            }
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.error(&format!("Expected `{c}`")))
        }
    }

    fn expect_word(&mut self, word: &str) -> Result<(), String> {
        self.skip_ws();
        match self.ident() {
            Some(found) if found == word => Ok(()),
            _ => {
                self.pos = 0;
                Err(self.error(&format!("A statement starts with `{word}`")))
            }
        }
    }

    fn ident(&mut self) -> Option<String> {
        let start = self.pos;
        let first = self.peek()?;
        if !(first.is_alphabetic() || first == '_' || first == '$') {
            return None;
        }
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$') {
            self.pos += 1;
        }
        Some(self.chars[start..self.pos].iter().collect())
    }

    /// `( value, value, … )` — the parentheses included.
    fn args(&mut self) -> Result<Vec<Json>, String> {
        self.expect('(')?;
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(')') {
                self.pos += 1;
                return Ok(out);
            }
            out.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(')') => {}
                _ => return Err(self.error("Expected `,` or `)`")),
            }
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('\'' | '"' | '`') => Ok(Json::String(self.string()?)),
            Some('/') => self.regex(),
            Some(c) if c.is_ascii_digit() || matches!(c, '-' | '+' | '.') => self.number(),
            Some(c) if c.is_alphabetic() || c == '_' || c == '$' => self.word_value(),
            _ => Err(self.error("Expected a value")),
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect('{')?;
        let mut map = Map::new();
        loop {
            self.skip_ws();
            if self.peek() == Some('}') {
                self.pos += 1;
                return Ok(Json::Object(map));
            }
            let key = match self.peek() {
                Some('\'' | '"' | '`') => self.string()?,
                Some(c) if c.is_ascii_digit() => self.chars_while(|c| c.is_ascii_alphanumeric() || c == '_'),
                _ => self.ident().ok_or_else(|| self.error("Expected a field name"))?,
            };
            self.expect(':')?;
            let value = self.value()?;
            map.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some('}') => {}
                _ => return Err(self.error("Expected `,` or `}`")),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(']') {
                self.pos += 1;
                return Ok(Json::Array(items));
            }
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(']') => {}
                _ => return Err(self.error("Expected `,` or `]`")),
            }
        }
    }

    fn chars_while(&mut self, keep: impl Fn(char) -> bool) -> String {
        let start = self.pos;
        while self.peek().is_some_and(&keep) {
            self.pos += 1;
        }
        self.chars[start..self.pos].iter().collect()
    }

    /// A quoted string with JavaScript escapes.
    fn string(&mut self) -> Result<String, String> {
        self.skip_ws();
        let Some(quote @ ('\'' | '"' | '`')) = self.peek() else {
            return Err(self.error("Expected a string"));
        };
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err("The string is never closed.".to_string()),
                Some(c) if c == quote => return Ok(out),
                Some('\\') => match self.bump() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('b') => out.push('\u{8}'),
                    Some('f') => out.push('\u{c}'),
                    Some('0') => out.push('\0'),
                    Some('x') => out.push(self.hex_escape(2)?),
                    Some('u') => out.push(self.hex_escape(4)?),
                    Some(other) => out.push(other),
                    None => return Err("The string is never closed.".to_string()),
                },
                Some(c) => out.push(c),
            }
        }
    }

    fn hex_escape(&mut self, digits: usize) -> Result<char, String> {
        let hex: String = self.chars.iter().skip(self.pos).take(digits).collect();
        self.pos += digits;
        u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32).ok_or_else(|| format!("`\\{hex}` is not a valid escape."))
    }

    fn number(&mut self) -> Result<Json, String> {
        let text = self.chars_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'));
        let integral = !text.contains(['.', 'e', 'E']);
        if integral {
            if let Ok(n) = text.trim_start_matches('+').parse::<i64>() {
                return Ok(json!(n));
            }
        }
        text.trim_start_matches('+')
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Json::Number)
            .ok_or_else(|| format!("`{text}` is not a number."))
    }

    /// A regex literal `/pattern/flags` as extended JSON.
    fn regex(&mut self) -> Result<Json, String> {
        let start = self.pos;
        let end = skip_regex(&self.chars, start);
        let body: String = self.chars[start + 1..end].iter().collect();
        let Some(close) = body.rfind('/') else {
            return Err(self.error("The regular expression is never closed"));
        };
        self.pos = end;
        Ok(json!({ "$regularExpression": { "pattern": &body[..close], "options": &body[close + 1..] } }))
    }

    /// `true`, `null`, `ObjectId("…")`, `new Date(…)`…
    fn word_value(&mut self) -> Result<Json, String> {
        let name = self.ident().ok_or_else(|| self.error("Expected a value"))?;
        match name.as_str() {
            "true" => return Ok(json!(true)),
            "false" => return Ok(json!(false)),
            "null" | "undefined" => return Ok(Json::Null),
            "new" => {
                self.skip_ws();
                let constructor = self.ident().ok_or_else(|| self.error("Expected a constructor after `new`"))?;
                self.skip_ws();
                let args = self.args()?;
                return constructor_value(&constructor, &args);
            }
            _ => {}
        }
        self.skip_ws();
        if self.peek() == Some('(') {
            let args = self.args()?;
            return constructor_value(&name, &args);
        }
        match name.as_str() {
            "MinKey" => Ok(json!({ "$minKey": 1 })),
            "MaxKey" => Ok(json!({ "$maxKey": 1 })),
            other => Err(format!("`{other}` is not a value the console understands. Quote it as a string if it is text.")),
        }
    }
}

/// `ObjectId("…")`, `ISODate("…")`, `NumberLong("…")`… as extended JSON.
fn constructor_value(name: &str, args: &[Json]) -> Result<Json, String> {
    let text = |index: usize| args.get(index).and_then(Json::as_str);
    match name {
        "ObjectId" => match text(0) {
            Some(hex) if hex.len() == 24 && hex.chars().all(|c| c.is_ascii_hexdigit()) => Ok(json!({ "$oid": hex.to_ascii_lowercase() })),
            None if args.is_empty() => Ok(json!({ "$oid": new_object_id() })),
            _ => Err("ObjectId() takes 24 hexadecimal characters.".to_string()),
        },
        "ISODate" | "Date" => date_value(args),
        "NumberLong" | "Long" => {
            let digits = args.first().map(|a| a.as_i64().map(|n| n.to_string()).or_else(|| a.as_str().map(str::to_string)));
            match digits.flatten() {
                Some(d) if d.parse::<i64>().is_ok() => Ok(json!({ "$numberLong": d })),
                _ => Err("NumberLong() takes a whole number that fits in 64 bits.".to_string()),
            }
        }
        "NumberInt" | "Int32" => match args.first().and_then(|a| a.as_i64().or_else(|| a.as_str().and_then(|s| s.parse().ok()))) {
            Some(n) if i32::try_from(n).is_ok() => Ok(json!(n)),
            _ => Err("NumberInt() takes a whole number that fits in 32 bits.".to_string()),
        },
        "NumberDecimal" | "Decimal128" => match args.first() {
            Some(Json::String(d)) if !d.is_empty() => Ok(json!({ "$numberDecimal": d })),
            Some(Json::Number(n)) => Ok(json!({ "$numberDecimal": n.to_string() })),
            _ => Err("NumberDecimal() takes a decimal such as NumberDecimal(\"1.50\").".to_string()),
        },
        "Number" | "Double" => args.first().and_then(|a| a.as_f64().or_else(|| a.as_str().and_then(|s| s.parse().ok()))).and_then(serde_json::Number::from_f64).map(Json::Number).ok_or_else(|| "Number() takes a number.".to_string()),
        "UUID" => {
            let hex: String = text(0).unwrap_or_default().chars().filter(|c| *c != '-').collect();
            let bytes = decode_hex(&hex).filter(|b| b.len() == 16).ok_or_else(|| "UUID() takes a UUID such as UUID(\"123e4567-e89b-12d3-a456-426614174000\").".to_string())?;
            Ok(json!({ "$binary": { "base64": base64::engine::general_purpose::STANDARD.encode(bytes), "subType": "04" } }))
        }
        "BinData" => match (args.first().and_then(Json::as_i64), text(1)) {
            (Some(kind), Some(data)) if (0..=255).contains(&kind) => Ok(json!({ "$binary": { "base64": data, "subType": format!("{kind:02x}") } })),
            _ => Err("BinData() takes a subtype and a base64 string, e.g. BinData(0, \"aGk=\").".to_string()),
        },
        "Timestamp" => match (args.first().and_then(Json::as_u64), args.get(1).and_then(Json::as_u64)) {
            (Some(t), Some(i)) => Ok(json!({ "$timestamp": { "t": t, "i": i } })),
            _ => Err("Timestamp() takes seconds and an increment, e.g. Timestamp(1700000000, 1).".to_string()),
        },
        "RegExp" => match text(0) {
            Some(pattern) => Ok(json!({ "$regularExpression": { "pattern": pattern, "options": text(1).unwrap_or_default() } })),
            None => Err("RegExp() takes a pattern string.".to_string()),
        },
        "MinKey" => Ok(json!({ "$minKey": 1 })),
        "MaxKey" => Ok(json!({ "$maxKey": 1 })),
        other => Err(format!("`{other}(…)` is not supported in this console.")),
    }
}

fn date_value(args: &[Json]) -> Result<Json, String> {
    match args.first() {
        None => Ok(json!({ "$date": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true) })),
        Some(Json::Number(ms)) => match ms.as_i64() {
            Some(ms) => Ok(json!({ "$date": { "$numberLong": ms.to_string() } })),
            None => Err("A date from a number takes whole milliseconds since 1970.".to_string()),
        },
        Some(Json::String(text)) => parse_date(text).map(|iso| json!({ "$date": iso })).ok_or_else(|| format!("`{text}` is not a date. Use ISO 8601, e.g. ISODate(\"2024-05-01T12:00:00Z\").")),
        Some(_) => Err("A date takes an ISO 8601 string or milliseconds.".to_string()),
    }
}

/// ISO 8601 with or without a zone or a time; no zone means UTC.
fn parse_date(text: &str) -> Option<String> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
    let text = text.trim();
    let utc = DateTime::parse_from_rfc3339(text)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").ok().map(|n| n.and_utc()))
        .or_else(|| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f").ok().map(|n| n.and_utc()))
        .or_else(|| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)).map(|n| n.and_utc()))?;
    Some(utc.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect()
}

/// A fresh 12-byte id as 24 hex characters (4 bytes of time, 8 random).
fn new_object_id() -> String {
    let seconds = u32::try_from(chrono::Utc::now().timestamp()).unwrap_or_default();
    let random = uuid::Uuid::new_v4();
    let tail: String = random.as_bytes().iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{seconds:08x}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(text: &str) -> Json {
        match parse(text, 1000) {
            Ok(Shell::Command(c)) => c,
            other => panic!("expected a command, got {other:?} for {text}"),
        }
    }

    fn error(text: &str) -> String {
        parse(text, 1000).err().unwrap_or_default()
    }

    // The console reads a command's name from its first key, and `serde_json` only
    // keeps insertion order while some dependency enables `preserve_order`. If this
    // fails, enable that feature on `serde_json` in Cargo.toml.
    #[test]
    fn json_objects_keep_their_key_order() {
        let parsed: Json = serde_json::from_str(r#"{"find":"a","filter":{}}"#).unwrap_or_default();
        assert_eq!(parsed.as_object().and_then(|m| m.keys().next()).map(String::as_str), Some("find"));
    }

    #[test]
    fn recognises_shell_text_but_not_command_documents() {
        assert!(is_shell("db.users.find()"));
        assert!(is_shell("  show collections"));
        assert!(is_shell("use sales"));
        assert!(is_shell("db['odd name'].find()"));
        assert!(is_shell("// all users\ndb.users.find()"));
        assert!(!is_shell(r#"{ "find": "users" }"#));
        assert!(!is_shell("find users {}"));
        assert!(!is_shell("database.x"));
        assert!(!is_shell("users"));
    }

    #[test]
    fn find_carries_its_filter_projection_and_cursor_modifiers() {
        let c = command("db.users.find({ age: { $gt: 30 }, name: /^a/i }, { name: 1 }).sort({ age: -1 }).skip(5).limit(10)");
        assert_eq!(c["find"], "users");
        assert_eq!(c["filter"]["age"]["$gt"], 30);
        assert_eq!(c["filter"]["name"]["$regularExpression"]["pattern"], "^a");
        assert_eq!(c["filter"]["name"]["$regularExpression"]["options"], "i");
        assert_eq!(c["projection"]["name"], 1);
        assert_eq!(c["sort"]["age"], -1);
        assert_eq!(c["skip"], 5);
        assert_eq!(c["limit"], 10);
    }

    #[test]
    fn find_without_a_limit_asks_for_one_row_past_the_cap() {
        assert_eq!(command("db.users.find()")["limit"], 1001);
        assert_eq!(command("db.users.find().limit(50000)")["limit"], 1001, "the row cap still wins");
        assert!(command("db.users.find()").get("projection").is_none());
        let unlimited = parse("db.users.find()", usize::MAX);
        assert!(matches!(&unlimited, Ok(Shell::Command(c)) if c.get("limit").is_none()), "{unlimited:?}");
    }

    #[test]
    fn find_one_reads_a_single_document() {
        let c = command("db.users.findOne({ _id: ObjectId('507f1f77bcf86cd799439011') })");
        assert_eq!(c["limit"], 1);
        assert_eq!(c["singleBatch"], true);
        assert_eq!(c["filter"]["_id"]["$oid"], "507f1f77bcf86cd799439011");
    }

    #[test]
    fn a_counted_cursor_becomes_a_count() {
        let c = command("db.users.find({ active: true }).count()");
        assert_eq!(c["count"], "users");
        assert_eq!(c["query"]["active"], true);
        assert!(c.get("find").is_none());
        assert_eq!(command("db.users.countDocuments({ a: 1 })")["query"]["a"], 1);
        assert_eq!(command("db.users.estimatedDocumentCount()")["count"], "users");
    }

    #[test]
    fn aggregate_takes_a_pipeline_array() {
        let c = command("db.orders.aggregate([ { $match: { status: 'paid' } }, { $group: { _id: '$customer', total: { $sum: '$amount' } } } ], { allowDiskUse: true })");
        assert_eq!(c["aggregate"], "orders");
        assert_eq!(c["pipeline"].as_array().map(Vec::len), Some(2));
        assert_eq!(c["allowDiskUse"], true);
        assert!(error("db.orders.aggregate({ $match: {} })").contains("pipeline array"));
    }

    #[test]
    fn inserts_updates_and_deletes_become_write_commands() {
        let c = command("db.users.insertOne({ name: 'Ada', joined: ISODate('2024-05-01') })");
        assert_eq!(c["insert"], "users");
        assert_eq!(c["documents"][0]["joined"]["$date"], "2024-05-01T00:00:00.000Z");

        let c = command("db.users.insertMany([{ a: 1 }, { a: 2 }], { ordered: false })");
        assert_eq!(c["documents"].as_array().map(Vec::len), Some(2));
        assert_eq!(c["ordered"], false);

        let c = command("db.users.updateMany({ active: false }, { $set: { archived: true } }, { upsert: true })");
        assert_eq!(c["update"], "users");
        assert_eq!(c["updates"][0]["multi"], true);
        assert_eq!(c["updates"][0]["upsert"], true);
        assert_eq!(c["updates"][0]["u"]["$set"]["archived"], true);
        assert_eq!(command("db.users.updateOne({ a: 1 }, { $inc: { n: 1 } })")["updates"][0]["multi"], false);

        let c = command("db.users.deleteOne({ _id: 7 })");
        assert_eq!(c["deletes"][0]["limit"], 1);
        assert_eq!(command("db.users.deleteMany({ old: true })")["deletes"][0]["limit"], 0);
    }

    #[test]
    fn unsafe_shapes_are_refused_with_a_reason() {
        assert!(error("db.users.deleteMany()").contains("needs a filter"));
        assert!(error("db.users.updateOne({ a: 1 }, { b: 2 })").contains("update with operators"));
        assert!(error("db.users.replaceOne({ a: 1 }, { $set: { b: 2 } })").contains("plain document"));
        assert!(error("db.users.insertMany([])").contains("non-empty array"));
        assert!(error("db.users.frobnicate()").contains("not supported"));
        assert!(error("db.users.find().forEach(printjson)").contains("forEach"));
    }

    #[test]
    fn replace_and_find_and_modify() {
        let c = command("db.users.replaceOne({ _id: 1 }, { name: 'New' })");
        assert_eq!(c["updates"][0]["u"]["name"], "New");
        let c = command("db.users.findOneAndUpdate({ _id: 1 }, { $set: { a: 1 } }, { returnDocument: 'after', upsert: true, sort: { a: 1 } })");
        assert_eq!(c["findAndModify"], "users");
        assert_eq!(c["new"], true);
        assert_eq!(c["upsert"], true);
        assert_eq!(c["sort"]["a"], 1);
        assert_eq!(command("db.users.findOneAndDelete({ _id: 1 })")["remove"], true);
    }

    #[test]
    fn distinct_and_index_helpers() {
        let c = command("db.users.distinct('country', { active: true })");
        assert_eq!((c["distinct"].as_str(), c["key"].as_str()), (Some("users"), Some("country")));
        let c = command("db.users.createIndex({ email: 1, age: -1 }, { unique: true })");
        assert_eq!(c["indexes"][0]["name"], "email_1_age_-1");
        assert_eq!(c["indexes"][0]["unique"], true);
        assert_eq!(command("db.users.getIndexes()")["listIndexes"], "users");
        assert_eq!(command("db.users.dropIndex('email_1')")["index"], "email_1");
        assert_eq!(command("db.users.drop()")["drop"], "users");
    }

    #[test]
    fn collection_names_may_hold_dots_or_come_from_getcollection_and_brackets() {
        assert_eq!(command("db.system.profile.find()")["find"], "system.profile");
        assert_eq!(command("db.getCollection('my coll').find()")["find"], "my coll");
        assert_eq!(command("db['odd-name'].find()")["find"], "odd-name");
    }

    #[test]
    fn sibling_databases_route_the_command() {
        let c = command("db.getSiblingDB('reports').daily.find()");
        assert_eq!(c["$db"], "reports");
        assert_eq!(c["find"], "daily");
        assert_eq!(command("db.adminCommand({ listDatabases: 1 })")["$db"], "admin");
    }

    #[test]
    fn database_level_helpers() {
        assert_eq!(command("db.getCollectionNames()")["listCollections"], 1);
        assert_eq!(command("db.stats()")["dbStats"], 1);
        assert_eq!(command("db.version()")["buildInfo"], 1);
        assert_eq!(command("db.runCommand({ ping: 1 })")["ping"], 1);
        assert_eq!(command("db.runCommand('ping')")["ping"], 1);
        assert_eq!(command("db.createCollection('logs', { capped: true, size: 1048576 })")["capped"], true);
        assert!(error("db.nope()").contains("db.runCommand"));
    }

    #[test]
    fn show_and_use() {
        assert_eq!(parse("show dbs", 10), Ok(Shell::ShowDatabases));
        assert_eq!(parse("show databases;", 10), Ok(Shell::ShowDatabases));
        assert_eq!(parse("show collections", 10), Ok(Shell::ShowCollections));
        assert_eq!(parse("use sales", 10), Ok(Shell::Use("sales".to_string())));
        assert_eq!(parse("use 'my-db';", 10), Ok(Shell::Use("my-db".to_string())));
        assert!(error("show frobs").contains("not supported"));
        assert!(error("use").contains("one database name"));
    }

    #[test]
    fn values_follow_javascript_relaxed_syntax() {
        let c = command(
            "db.t.insertOne({ 'quoted key': 'it\\'s', unquoted: \"two\\nlines\", n: -1.5e2, big: NumberLong('9007199254740993'), \
             i: NumberInt(7), d: NumberDecimal('1.50'), t: true, nothing: null, undef: undefined, list: [1, 2, 3,], /* note */ trailing: 'comma', })",
        );
        let doc = &c["documents"][0];
        assert_eq!(doc["quoted key"], "it's");
        assert_eq!(doc["unquoted"], "two\nlines");
        assert_eq!(doc["n"], -150.0);
        assert_eq!(doc["big"]["$numberLong"], "9007199254740993");
        assert_eq!(doc["i"], 7);
        assert_eq!(doc["d"]["$numberDecimal"], "1.50");
        assert_eq!(doc["nothing"], Json::Null);
        assert_eq!(doc["undef"], Json::Null);
        assert_eq!(doc["list"].as_array().map(Vec::len), Some(3));
        assert_eq!(doc["trailing"], "comma");
    }

    #[test]
    fn bson_constructors_become_extended_json() {
        let c = command("db.t.insertOne({ a: new Date('2024-01-02T03:04:05Z'), b: new Date(1700000000000), u: UUID('123e4567-e89b-12d3-a456-426614174000'), ts: Timestamp(1700000000, 2), r: new RegExp('x+', 'g'), lo: MinKey(), hi: MaxKey() })");
        let doc = &c["documents"][0];
        assert_eq!(doc["a"]["$date"], "2024-01-02T03:04:05.000Z");
        assert_eq!(doc["b"]["$date"]["$numberLong"], "1700000000000");
        assert_eq!(doc["u"]["$binary"]["subType"], "04");
        assert_eq!(doc["u"]["$binary"]["base64"], "Ej5FZ+ibEtOkVkJmFBdAAA==");
        assert_eq!(doc["ts"]["$timestamp"]["i"], 2);
        assert_eq!(doc["r"]["$regularExpression"]["options"], "g");
        assert_eq!(doc["lo"]["$minKey"], 1);
        assert_eq!(doc["hi"]["$maxKey"], 1);
        assert!(error("db.t.insertOne({ a: ObjectId('xyz') })").contains("24 hexadecimal"));
        assert!(error("db.t.insertOne({ a: ISODate('yesterday') })").contains("not a date"));
        assert!(error("db.t.insertOne({ a: someVariable })").contains("someVariable"));
    }

    #[test]
    fn a_new_object_id_is_24_hex_characters() {
        let c = command("db.t.insertOne({ _id: ObjectId() })");
        let id = c["documents"][0]["_id"]["$oid"].as_str().unwrap_or_default().to_string();
        assert_eq!(id.len(), 24);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn comments_and_trailing_semicolons_are_ignored() {
        let c = command("db.users // all of them\n  .find( /* none */ )\n  .limit(3);");
        assert_eq!(c["find"], "users");
        assert_eq!(c["limit"], 3);
    }

    #[test]
    fn errors_say_where_the_reader_stopped() {
        assert!(error("db.users.find({ a: })").contains("near"));
        assert!(error("db.users.find({ a: 1 ").contains("at the end"));
        assert!(error("db.users.find() extra").contains("Unexpected text"));
        assert!(error("db.users.find('unclosed)").contains("never closed"));
        assert!(error("db").contains("Expected"));
    }

    #[test]
    fn split_cuts_at_semicolons_blank_lines_and_new_statements() {
        let text = "show dbs\nuse sales\ndb.a.find({ x: ';' })\n  .sort({ x: 1 });db.b.find()\n\ndb.c.find(\n  { y: 1 }\n)";
        assert_eq!(
            split(text),
            ["show dbs", "use sales", "db.a.find({ x: ';' })\n  .sort({ x: 1 })", "db.b.find()", "db.c.find(\n  { y: 1 }\n)"]
        );
    }

    #[test]
    fn split_keeps_a_chain_on_following_lines_together() {
        assert_eq!(split("db.a.find()\n  .limit(2)\n  .skip(1)"), ["db.a.find()\n  .limit(2)\n  .skip(1)"]);
        assert_eq!(split("db.a.find({ r: /a;b/ })"), ["db.a.find({ r: /a;b/ })"]);
        assert_eq!(split("// note; with semicolon\ndb.a.find()"), ["// note; with semicolon\ndb.a.find()"]);
    }

    #[test]
    fn the_shells_structure_listings_become_read_commands() {
        let shell = |text: &str| match parse(text, 1000) {
            Ok(Shell::Command(c)) => c,
            other => panic!("{text}: {other:?}"),
        };
        assert_eq!(shell("show users"), json!({ "usersInfo": 1 }));
        assert_eq!(shell("show roles")["rolesInfo"], 1);
        assert_eq!(shell("show profile")["find"], "system.profile");
        assert_eq!(shell("show profile")["limit"], 5);
        assert_eq!(shell("show logs"), json!({ "getLog": "*", "$db": "admin" }));
        assert_eq!(shell("show log")["getLog"], "global");
        assert_eq!(shell("show log startupWarnings")["getLog"], "startupWarnings");
        assert_eq!(command("db.getUsers()"), json!({ "usersInfo": 1 }));
        assert_eq!(command("db.getRoles({ showBuiltinRoles: true })")["showBuiltinRoles"], true);
        assert_eq!(command("db.hostInfo()"), json!({ "hostInfo": 1, "$db": "admin" }));
        assert_eq!(command("db.currentOp()")["currentOp"], 1);
        assert_eq!(command("db.getProfilingStatus()"), json!({ "profile": -1 }));
        assert_eq!(command("db.users.validate()")["validate"], "users");
        assert_eq!(command("db.users.exists()")["filter"], json!({ "name": "users" }));
        assert!(error("show frobs").contains("show users"));
        for text in ["show users", "show roles", "show profile", "show logs", "show log", "db.getUsers()", "db.getRoles()", "db.hostInfo()", "db.currentOp()", "db.getProfilingStatus()", "db.users.validate()", "db.users.exists()"] {
            let found = match parse(text, 1000) {
                Ok(Shell::Command(c)) => c,
                other => panic!("{text}: {other:?}"),
            };
            assert!(is_read_command(&found), "{text} only reads");
        }
        assert!(!is_read_command(&json!({ "profile": 2 })), "setting the profiler level writes");
        assert!(is_read_command(&json!({ "profile": -1, "$db": "app" })));
        assert!(!is_read_command(&json!({ "profile": -1, "slowms": 1 })), "slowms beside -1 may change the threshold");
        assert!(!is_read_command(&json!({ "profile": -1, "sampleRate": 0.5 })));
    }

    #[test]
    fn options_are_carried_or_refused_never_dropped() {
        assert_eq!(command("db.getUsers({ filter: { user: 'a' } })"), json!({ "usersInfo": 1, "filter": { "user": "a" } }));
        assert_eq!(command("db.getUsers({ showPrivileges: true })")["showPrivileges"], true);
        assert!(error("db.getUsers(5)").contains("object"));
        assert!(error("db.currentOp({ active: true })").contains("adminCommand"));
        assert!(error("db.currentOp(true)").contains("adminCommand"));
    }

    #[test]
    fn validate_reads_unless_it_repairs() {
        assert!(is_read_command(&json!({ "validate": "users" })));
        assert!(is_read_command(&json!({ "validate": "users", "full": true, "repair": false, "fixMultikey": 0 })));
        assert!(!is_read_command(&json!({ "validate": "users", "repair": true })));
        assert!(!is_read_command(&json!({ "validate": "users", "full": true, "fixMultikey": true })));
        assert!(!is_read_command(&json!({ "validate": "users", "repair": 1 })));
        assert!(!is_read_command(&command("db.runCommand({ validate: 'users', repair: true })")));
    }

    #[test]
    fn only_listed_commands_are_reads() {
        let read = |text: &str| is_read_command(&command(text));
        assert!(read("db.users.find()"));
        assert!(read("db.users.countDocuments({})"));
        assert!(read("db.orders.aggregate([{ $match: {} }])"));
        assert!(read("db.getCollectionNames()"));
        assert!(!read("db.orders.aggregate([{ $match: {} }, { $out: 'copy' }])"));
        assert!(!read("db.orders.aggregate([{ $merge: { into: 'copy' } }])"));
        assert!(!read("db.users.insertOne({ a: 1 })"));
        assert!(!read("db.users.deleteMany({})"));
        assert!(!read("db.users.drop()"));
        assert!(!read("db.runCommand({ shutdown: 1 })"));
        assert!(is_read_command(&json!({ "explain": { "find": "users" } })));
        assert!(!is_read_command(&json!({ "explain": { "delete": "users", "deletes": [] } })));
        assert!(!is_read_command(&json!({})));
    }
}
