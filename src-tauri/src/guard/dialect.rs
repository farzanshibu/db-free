// SOT: query-dialect, per-engine-classifier, native-statement-split, redis-classifier, mongo-classifier, cypher-classifier, console-classifier

// WHAT:  The language a connection's query editor speaks, and how the block
//        reads it: where one statement ends, and whether it reads, writes or
//        destroys.
// WHY:   The block used to read every engine's script as SQL. `GET user:1`,
//        `{"find": "users"}` and `GET /logs/_search` all start with a word SQL
//        does not know, so they classified as writes: a read-only Redis, Mongo
//        or Elasticsearch connection refused plain reads, and a Redis script was
//        cut on `;` instead of per line. Each connection is now read in its own
//        engine's language and nothing else.
// HOW:   `Engine` → `Family` → `Dialect`. Each dialect owns a splitter that cuts
//        exactly where that family's adapter does (so the gutter ▶ sends one
//        statement the adapter accepts) and a classifier over that language's own
//        verbs. Anything a classifier does not recognise is a Write: fail closed,
//        as the SQL classifier always has.
// WHERE: src-tauri/src/guard/mod.rs (steps 4–6), commands/query.rs (split_script),
//        services/agent/tools.rs (agent gate), src/lib/queryLanguage.ts (editor)

use super::destructive::{self, ClassifiedStatement, StatementKind};
use crate::model::{Engine, Family, StatementIntent, StatementSpan};
use serde_json::Value as Json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// SQL and its relatives (CQL, PartiQL, SurrealQL, ClickHouse…): `;`-separated.
    Sql,
    /// Cypher (Neo4j, Memgraph): `;`-separated, its own write keywords.
    Cypher,
    /// Redis / Valkey / Dragonfly: one command per line.
    Redis,
    /// MongoDB: JSON command documents or `find`/`count` shorthands, blank-line separated.
    Mongo,
    /// Memcached: one command per line; a storage command takes the next line as its value.
    Memcached,
    /// One shorthand command per line (RocksDB, immudb KV).
    Lines,
    /// Blank-line separated commands, each a shorthand or a JSON body
    /// (CouchDB, Firestore, Meilisearch, Typesense, Prometheus, DynamoDB…).
    Blocks,
    /// Elasticsearch / OpenSearch console: `METHOD /path` with an optional JSON body.
    Console,
    /// JSON bodies one per balanced object, shorthands one per line (Kafka, RabbitMQ, Convex).
    Stream,
    /// The whole buffer is one command (AQL, XQuery, SPARQL, GraphQL, GSQL, Flux…).
    Whole,
}

pub fn dialect_of(engine: Engine) -> Dialect {
    match engine.family() {
        Family::Postgres
        | Family::Mysql
        | Family::Mssql
        | Family::Oracle
        | Family::Sqlite
        | Family::Duckdb
        | Family::Clickhouse
        | Family::Cassandra
        | Family::Snowflake
        | Family::Bigquery
        | Family::Libsql
        | Family::ValTown
        | Family::CloudflareD1
        | Family::Qldb
        | Family::Surrealdb
        | Family::Druid
        | Family::Dynamodb => Dialect::Sql,
        Family::Neo4j => Dialect::Cypher,
        Family::Redis => Dialect::Redis,
        Family::Mongodb => Dialect::Mongo,
        Family::Memcached => Dialect::Memcached,
        Family::Rocksdb => Dialect::Lines,
        Family::Couchdb | Family::Firestore | Family::Meilisearch | Family::Typesense | Family::Prometheus => Dialect::Blocks,
        Family::Elasticsearch => Dialect::Console,
        Family::Kafka | Family::Rabbitmq | Family::Convex => Dialect::Stream,
        Family::Hbase
        | Family::Tigergraph
        | Family::Influxdb
        | Family::Qdrant
        | Family::Milvus
        | Family::Weaviate
        | Family::Pinecone
        | Family::Chroma
        | Family::Arangodb
        | Family::Orientdb
        | Family::Spacetimedb
        | Family::S3
        | Family::Immudb
        | Family::Objectdb
        | Family::Sparql
        | Family::Basex
        | Family::Existdb => Dialect::Whole,
    }
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Every statement in `text`, read in `engine`'s own language.
pub fn classify(engine: Engine, text: &str) -> Vec<ClassifiedStatement> {
    let dialect = dialect_of(engine);
    if dialect == Dialect::Sql {
        return destructive::classify(text);
    }
    let chars: Vec<char> = text.chars().collect();
    ranges(dialect, &chars)
        .into_iter()
        .filter_map(|(from, to)| {
            let statement: String = chars[from..to].iter().collect();
            let statement = statement.trim().to_string();
            if statement.is_empty() {
                return None;
            }
            let (kind, reason) = classify_one(engine, dialect, &statement);
            Some(ClassifiedStatement { text: statement, kind, reason })
        })
        .collect()
}

// WHAT:  Where each statement starts and ends, in UTF-16 units (CodeMirror's
//        positions), with what running it would mean.
pub fn spans(engine: Engine, text: &str) -> Vec<StatementSpan> {
    let dialect = dialect_of(engine);
    if dialect == Dialect::Sql {
        return destructive::spans(text);
    }
    let chars: Vec<char> = text.chars().collect();
    let mut utf16: Vec<u32> = Vec::with_capacity(chars.len() + 1);
    let mut total: u32 = 0;
    utf16.push(0);
    for c in &chars {
        total = total.saturating_add(if c.len_utf16() == 2 { 2 } else { 1 });
        utf16.push(total);
    }
    let at = |i: usize| utf16.get(i).copied().unwrap_or(total);
    ranges(dialect, &chars)
        .into_iter()
        .filter_map(|(mut from, mut to)| {
            while from < to && chars[from].is_whitespace() {
                from += 1;
            }
            while to > from && chars[to - 1].is_whitespace() {
                to -= 1;
            }
            if from == to {
                return None;
            }
            let statement: String = chars[from..to].iter().collect();
            let (kind, _) = classify_one(engine, dialect, &statement);
            Some(StatementSpan { start: at(from), end: at(to), intent: intent_of(kind) })
        })
        .collect()
}

fn intent_of(kind: StatementKind) -> StatementIntent {
    match kind {
        StatementKind::Read => StatementIntent::Read,
        StatementKind::Write => StatementIntent::Write,
        StatementKind::Destructive => StatementIntent::Destructive,
    }
}

// ---------------------------------------------------------------------------
// Splitting: char ranges, the way each family's adapter cuts its input
// ---------------------------------------------------------------------------

fn ranges(dialect: Dialect, chars: &[char]) -> Vec<(usize, usize)> {
    match dialect {
        // Cypher shares SQL's quoting and `;` separator; the SQL scanner already
        // skips strings, backticks and comments.
        Dialect::Sql | Dialect::Cypher => sql_ranges(chars),
        Dialect::Redis | Dialect::Lines => line_ranges(chars, false),
        Dialect::Memcached => line_ranges(chars, true),
        Dialect::Mongo | Dialect::Blocks => block_ranges(chars),
        Dialect::Console => console_ranges(chars),
        Dialect::Stream => stream_ranges(chars),
        Dialect::Whole => {
            if chars.iter().any(|c| !c.is_whitespace()) {
                vec![(0, chars.len())]
            } else {
                Vec::new()
            }
        }
    }
}

fn sql_ranges(chars: &[char]) -> Vec<(usize, usize)> {
    let text: String = chars.iter().collect();
    // `destructive::spans` reports UTF-16 offsets; map back through char counts.
    let pieces = destructive::split_statements(&text);
    let mut out = Vec::with_capacity(pieces.len());
    let mut cursor = 0_usize;
    for piece in pieces {
        let len = piece.chars().count();
        out.push((cursor, (cursor + len).min(chars.len())));
        // Every piece but the tail was followed by its `;` separator.
        cursor += len + 1;
    }
    out
}

/// (start, end) of every line, end excluding the newline.
fn lines(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            out.push((start, i));
            start = i + 1;
        }
    }
    if start <= chars.len() {
        out.push((start, chars.len()));
    }
    out
}

fn line_text(chars: &[char], (from, to): (usize, usize)) -> String {
    chars[from..to].iter().collect::<String>().trim().to_string()
}

fn is_comment_line(text: &str) -> bool {
    text.starts_with('#') || (text.starts_with("--") && !text.starts_with("--cf"))
}

fn line_ranges(chars: &[char], storage_takes_next: bool) -> Vec<(usize, usize)> {
    let all = lines(chars);
    let mut out = Vec::new();
    let mut i = 0;
    while i < all.len() {
        let range = all[i];
        let text = line_text(chars, range);
        i += 1;
        if text.is_empty() || is_comment_line(&text) {
            continue;
        }
        let verb = first_word(&text).to_ascii_lowercase();
        if storage_takes_next && matches!(verb.as_str(), "set" | "add" | "replace" | "append" | "prepend" | "cas") {
            if let Some(next) = all.get(i) {
                out.push((range.0, next.1));
                i += 1;
                continue;
            }
        }
        out.push(range);
    }
    out
}

fn block_ranges(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    for range in lines(chars) {
        if line_text(chars, range).is_empty() {
            if let Some(block) = current.take() {
                out.push(block);
            }
        } else {
            current = Some(match current {
                Some((from, _)) => (from, range.1),
                None => range,
            });
        }
    }
    if let Some(block) = current {
        out.push(block);
    }
    out
}

const REST_VERBS: [&str; 6] = ["GET", "POST", "PUT", "DELETE", "HEAD", "PATCH"];

fn is_rest_line(text: &str) -> bool {
    let mut words = text.split_whitespace();
    let verb = words.next().unwrap_or_default().to_ascii_uppercase();
    REST_VERBS.contains(&verb.as_str()) && words.next().is_some_and(|p| p.starts_with('/') || p.starts_with('_') || p.chars().next().is_some_and(char::is_alphanumeric))
}

// WHAT:  `METHOD /path` starts a request and the JSON under it is its body;
//        a bare JSON body (a search) runs on its own, ended by a blank line.
fn console_ranges(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut current: Option<(usize, usize)> = None;
    for range in lines(chars) {
        let text = line_text(chars, range);
        if is_rest_line(&text) {
            if let Some(block) = current.take() {
                out.push(block);
            }
            current = Some(range);
            continue;
        }
        if text.is_empty() {
            // A blank line inside a request's JSON body does not end it; one
            // after a complete body (or after a bodiless request) does.
            if let Some((from, to)) = current {
                let so_far: String = chars[from..to].iter().collect();
                if balanced(&so_far) {
                    out.push((from, to));
                    current = None;
                }
            }
            continue;
        }
        current = Some(match current {
            Some((from, _)) => (from, range.1),
            None => range,
        });
    }
    if let Some(block) = current {
        out.push(block);
    }
    out
}

/// Braces and brackets outside strings all closed.
fn balanced(text: &str) -> bool {
    let mut depth = 0_i32;
    let mut in_str = false;
    let mut escaped = false;
    for c in text.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth <= 0 && !in_str
}

// WHAT:  The Kafka / RabbitMQ / Convex console: a JSON body runs from its `{`
//        to the matching `}`, a shorthand is one line.
fn stream_ranges(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= len {
            break;
        }
        let start = i;
        if chars[i] == '{' {
            let mut depth = 0_i32;
            let mut in_str = false;
            let mut escaped = false;
            while i < len {
                let c = chars[i];
                i += 1;
                if in_str {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        in_str = false;
                    }
                    continue;
                }
                match c {
                    '"' => in_str = true,
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        } else {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
        }
        let text: String = chars[start..i].iter().collect();
        if !is_comment_line(text.trim()) {
            out.push((start, i));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Classification, one language at a time
// ---------------------------------------------------------------------------

type Verdict = (StatementKind, Option<String>);

const READ: Verdict = (StatementKind::Read, None);
const WRITE: Verdict = (StatementKind::Write, None);

fn destroy(reason: &str) -> Verdict {
    (StatementKind::Destructive, Some(reason.to_string()))
}

fn first_word(text: &str) -> &str {
    text.split_whitespace().next().unwrap_or_default()
}

fn words_upper(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '$'))
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_uppercase)
        .collect()
}

fn classify_one(engine: Engine, dialect: Dialect, statement: &str) -> Verdict {
    match dialect {
        Dialect::Sql => {
            let mut all = destructive::classify(statement);
            match all.pop() {
                Some(one) => (one.kind, one.reason),
                None => READ,
            }
        }
        Dialect::Cypher => cypher(statement),
        Dialect::Redis => redis(statement),
        Dialect::Mongo => mongo(statement),
        Dialect::Memcached => memcached(statement),
        Dialect::Lines => keyed_lines(statement),
        Dialect::Blocks | Dialect::Console | Dialect::Stream | Dialect::Whole => native(engine, statement),
    }
}

// --- Cypher ----------------------------------------------------------------

const CYPHER_WRITES: &[&str] = &["CREATE", "MERGE", "SET", "REMOVE", "DELETE", "DETACH", "LOAD", "ALTER", "GRANT", "REVOKE", "DENY", "FOREACH"];

fn cypher(statement: &str) -> Verdict {
    let words = words_upper(&strip_quoted(statement));
    let has = |kw: &str| words.iter().any(|w| w == kw);
    if words.first().is_some_and(|w| w == "DROP") {
        return destroy("DROP removes the object and its data.");
    }
    if has("DELETE") && !has("WHERE") && !has("LIMIT") && !words.iter().any(|w| w.contains('{')) {
        return destroy("DELETE without a WHERE clause removes every matched node.");
    }
    if words.first().is_some_and(|w| w == "STOP" || w == "TERMINATE") {
        return WRITE;
    }
    if CYPHER_WRITES.iter().any(|kw| has(kw)) {
        return WRITE;
    }
    READ
}

/// The statement with quoted strings and comments blanked, so a keyword inside
/// a literal (`{name: 'Delete me'}`) never counts.
fn strip_quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == '\\' {
                    chars.next();
                } else if c == q {
                    quote = None;
                }
                out.push(' ');
            }
            None => match c {
                '\'' | '"' | '`' => {
                    quote = Some(c);
                    out.push(' ');
                }
                '/' if chars.peek() == Some(&'/') => {
                    for n in chars.by_ref() {
                        if n == '\n' {
                            break;
                        }
                    }
                    out.push('\n');
                }
                _ => out.push(c),
            },
        }
    }
    out
}

// --- Redis -----------------------------------------------------------------

const REDIS_READS: &[&str] = &[
    "GET", "MGET", "GETRANGE", "SUBSTR", "STRLEN", "EXISTS", "TYPE", "TTL", "PTTL", "EXPIRETIME", "PEXPIRETIME",
    "KEYS", "SCAN", "RANDOMKEY", "DBSIZE", "DUMP", "TOUCH", "LCS",
    "HGET", "HMGET", "HGETALL", "HKEYS", "HVALS", "HLEN", "HEXISTS", "HSTRLEN", "HSCAN", "HRANDFIELD", "HTTL",
    "LRANGE", "LLEN", "LINDEX", "LPOS",
    "SMEMBERS", "SCARD", "SISMEMBER", "SMISMEMBER", "SRANDMEMBER", "SSCAN", "SINTER", "SINTERCARD", "SUNION", "SDIFF",
    "ZRANGE", "ZRANGEBYSCORE", "ZRANGEBYLEX", "ZREVRANGE", "ZREVRANGEBYSCORE", "ZREVRANGEBYLEX", "ZCARD", "ZSCORE",
    "ZMSCORE", "ZRANK", "ZREVRANK", "ZCOUNT", "ZLEXCOUNT", "ZSCAN", "ZRANDMEMBER", "ZINTER", "ZUNION", "ZDIFF", "ZINTERCARD",
    "XRANGE", "XREVRANGE", "XLEN", "XREAD", "XINFO", "XPENDING",
    "GETBIT", "BITCOUNT", "BITPOS", "BITFIELD_RO", "PFCOUNT",
    "GEOPOS", "GEODIST", "GEOHASH", "GEORADIUS_RO", "GEORADIUSBYMEMBER_RO", "GEOSEARCH",
    "INFO", "PING", "ECHO", "TIME", "LASTSAVE", "ROLE", "LOLWUT", "COMMAND", "HELLO", "READONLY",
    "SORT_RO", "EVAL_RO", "EVALSHA_RO", "FCALL_RO", "PUBSUB", "WAIT",
    "JSON.GET", "JSON.MGET", "JSON.TYPE", "JSON.STRLEN", "JSON.ARRLEN", "JSON.OBJKEYS", "JSON.OBJLEN", "JSON.RESP",
    "FT.SEARCH", "FT.AGGREGATE", "FT.INFO", "FT._LIST", "FT.EXPLAIN", "FT.PROFILE", "FT.TAGVALS",
    "TS.GET", "TS.MGET", "TS.RANGE", "TS.REVRANGE", "TS.MRANGE", "TS.MREVRANGE", "TS.INFO", "TS.QUERYINDEX",
    "BF.EXISTS", "BF.MEXISTS", "BF.INFO", "CF.EXISTS", "CF.COUNT", "CF.INFO", "CMS.QUERY", "CMS.INFO", "TOPK.QUERY", "TOPK.LIST",
];

/// `CONTAINER SUBCOMMAND` pairs that only read.
const REDIS_READ_SUBCOMMANDS: &[(&str, &[&str])] = &[
    ("CONFIG", &["GET"]),
    ("CLIENT", &["LIST", "INFO", "GETNAME", "ID", "TRACKINGINFO", "GETREDIR"]),
    ("SLOWLOG", &["GET", "LEN"]),
    ("CLUSTER", &["INFO", "NODES", "SLOTS", "SHARDS", "MYID", "KEYSLOT", "COUNTKEYSINSLOT", "GETKEYSINSLOT", "LINKS", "REPLICAS"]),
    ("MEMORY", &["USAGE", "STATS", "DOCTOR", "MALLOC-STATS"]),
    ("OBJECT", &["ENCODING", "FREQ", "IDLETIME", "REFCOUNT", "HELP"]),
    ("ACL", &["LIST", "USERS", "WHOAMI", "GETUSER", "CAT", "LOG"]),
    ("FUNCTION", &["LIST", "DUMP", "STATS"]),
    ("SCRIPT", &["EXISTS"]),
    ("MODULE", &["LIST"]),
    ("LATENCY", &["LATEST", "HISTORY", "DOCTOR", "GRAPH", "HISTOGRAM"]),
    ("XINFO", &["STREAM", "GROUPS", "CONSUMERS"]),
];

const REDIS_DESTRUCTIVE: &[(&str, &str)] = &[
    ("FLUSHALL", "FLUSHALL deletes every key in every database."),
    ("FLUSHDB", "FLUSHDB deletes every key in this database."),
    ("SWAPDB", "SWAPDB exchanges two whole databases."),
    ("SHUTDOWN", "SHUTDOWN stops the server."),
    ("DEBUG", "DEBUG can crash or block the server."),
];

fn redis(statement: &str) -> Verdict {
    let mut words = statement.split_whitespace().map(|w| w.trim_matches('"').to_ascii_uppercase());
    let Some(verb) = words.next() else {
        return READ;
    };
    if let Some((_, reason)) = REDIS_DESTRUCTIVE.iter().find(|(v, _)| *v == verb) {
        return destroy(reason);
    }
    let sub = words.next().unwrap_or_default();
    if (verb == "SCRIPT" || verb == "FUNCTION") && sub == "FLUSH" {
        return destroy("FLUSH removes every loaded script or function.");
    }
    if REDIS_READS.contains(&verb.as_str()) {
        return READ;
    }
    if REDIS_READ_SUBCOMMANDS.iter().any(|(container, subs)| *container == verb && subs.contains(&sub.as_str())) {
        return READ;
    }
    WRITE
}

// --- MongoDB ---------------------------------------------------------------

const MONGO_READS: &[&str] = &[
    "find", "count", "distinct", "aggregate", "getMore", "explain", "listCollections", "listDatabases",
    "listIndexes", "collStats", "dbStats", "dataSize", "serverStatus", "buildInfo", "hello", "isMaster",
    "ping", "hostInfo", "connectionStatus", "currentOp", "getParameter", "getCmdLineOpts", "top",
    "replSetGetStatus", "replSetGetConfig", "usersInfo", "rolesInfo", "listCommands", "getLog",
    "connPoolStats", "shardConnPoolStats", "listShards", "balancerStatus", "validate", "profile",
    "getDefaultRWConcern", "lockInfo", "whatsmyuri", "features", "$currentOp",
];

const MONGO_DESTRUCTIVE: &[(&str, &str)] = &[
    ("drop", "drop removes the collection and every document in it."),
    ("dropDatabase", "dropDatabase removes the database and everything in it."),
    ("dropIndexes", "dropIndexes removes indexes from the collection."),
    ("dropUser", "dropUser removes a user."),
    ("dropAllUsersFromDatabase", "dropAllUsersFromDatabase removes every user."),
    ("dropRole", "dropRole removes a role."),
    ("dropAllRolesFromDatabase", "dropAllRolesFromDatabase removes every role."),
    ("shutdown", "shutdown stops the server."),
];

fn mongo(statement: &str) -> Verdict {
    let text = statement.trim();
    if !text.starts_with('{') {
        // `find <collection> {filter}` / `count <collection> {filter}`; anything
        // else is an error the adapter will report, so nothing runs.
        let verb = first_word(text).to_ascii_lowercase();
        return if verb == "find" || verb == "count" { READ } else { WRITE };
    }
    let Ok(Json::Object(doc)) = serde_json::from_str::<Json>(text) else {
        return WRITE;
    };
    // The command is the document's first key (`$db` is a routing key, not a
    // command). Read from the text itself: a JSON map need not keep key order.
    let Some(command) = top_level_keys(text).into_iter().find(|k| k.as_str() != "$db") else {
        return READ;
    };
    let command = &command;
    if let Some((_, reason)) = MONGO_DESTRUCTIVE.iter().find(|(c, _)| c == command) {
        return destroy(reason);
    }
    if command == "aggregate" {
        let writes = doc
            .get("pipeline")
            .and_then(Json::as_array)
            .is_some_and(|stages| stages.iter().any(|s| s.get("$out").is_some() || s.get("$merge").is_some()));
        return if writes { WRITE } else { READ };
    }
    if command == "delete" {
        let wipes = doc.get("deletes").and_then(Json::as_array).is_some_and(|deletes| {
            deletes.iter().any(|d| d.get("q").and_then(Json::as_object).is_none_or(serde_json::Map::is_empty))
        });
        return if wipes { destroy("delete with an empty filter removes every document.") } else { WRITE };
    }
    if command == "update" {
        let all = doc.get("updates").and_then(Json::as_array).is_some_and(|updates| {
            updates.iter().any(|u| {
                u.get("q").and_then(Json::as_object).is_none_or(serde_json::Map::is_empty) && u.get("multi").and_then(Json::as_bool) == Some(true)
            })
        });
        return if all { destroy("update with an empty filter and multi: true rewrites every document.") } else { WRITE };
    }
    if MONGO_READS.contains(&command.as_str()) {
        return READ;
    }
    WRITE
}

/// Keys of the outermost JSON object, in the order they are written.
fn top_level_keys(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut depth = 0_i32;
    let mut chars = text.chars().peekable();
    // At depth 1, a string is a key when it is followed by `:`.
    while let Some(c) = chars.next() {
        match c {
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            '"' => {
                let mut s = String::new();
                while let Some(n) = chars.next() {
                    match n {
                        '\\' => {
                            if let Some(e) = chars.next() {
                                s.push(e);
                            }
                        }
                        '"' => break,
                        _ => s.push(n),
                    }
                }
                if depth == 1 {
                    while chars.peek().is_some_and(|n| n.is_whitespace()) {
                        chars.next();
                    }
                    if chars.peek() == Some(&':') {
                        keys.push(s);
                    }
                }
            }
            _ => {}
        }
    }
    keys
}

// --- Memcached / RocksDB / line shorthands ---------------------------------

fn memcached(statement: &str) -> Verdict {
    match first_word(statement).to_ascii_lowercase().as_str() {
        "get" | "gets" | "gat" | "gats" | "stats" | "version" | "mg" | "mn" | "me" | "lru_crawler" => READ,
        "flush_all" => destroy("flush_all invalidates every item in the cache."),
        "shutdown" => destroy("shutdown stops the server."),
        _ => WRITE,
    }
}

fn keyed_lines(statement: &str) -> Verdict {
    let mut text = statement.trim();
    // RocksDB: `--cf <name> GET key` addresses a column family.
    if let Some(rest) = text.strip_prefix("--cf") {
        text = rest.trim_start();
        text = text.split_once(char::is_whitespace).map_or("", |(_, rest)| rest.trim_start());
    }
    match first_word(text).to_ascii_uppercase().as_str() {
        "GET" | "SCAN" | "KEYS" | "CF" | "CFS" | "COLUMNFAMILIES" | "HISTORY" | "VERIFIED" => READ,
        "DROP" => destroy("DROP removes the column family and its data."),
        _ => WRITE,
    }
}

// --- Everything that speaks its own command / REST / query language --------

/// Leading words that only read, across the shorthand consoles the adapters
/// accept (`COLLECTIONS`, `SCROLL c 10`, `CONSUME topic`, `BUCKETS`, `SEARCH i q`…).
const NATIVE_READS: &[&str] = &[
    "GET", "HEAD", "SCAN", "KEYS", "LIST", "LS", "SEARCH", "QUERY", "FIND", "COUNT", "DESCRIBE", "DESC", "INFO",
    "STATS", "STATUS", "SHOW", "SELECT", "MATCH", "CONSUME", "PEEK", "TOPICS", "QUEUES", "EXCHANGES", "BUCKETS",
    "INDEXES", "INDICES", "COLLECTIONS", "CLASSES", "SCHEMA", "OBJECTS", "TABLES", "ALL", "HISTORY", "VERIFIED",
    "SCROLL", "FETCH", "EXPLAIN", "PING", "VERSION", "HEALTH", "ASK", "CONSTRUCT", "FROM", "RANGE", "INSTANT",
    "SERIES", "LABELS", "TARGETS", "METRICS", "VALUES", "GROUPS", "DATABASES", "MEASUREMENTS", "FUNCTIONS",
    "VERTICES", "EDGES", "INTERPRET", "SIMILAR", "NEAR", "TRAVERSE", "PROFILE",
];

const NATIVE_WRITES: &[&str] = &[
    "INSERT", "UPDATE", "UPSERT", "REPLACE", "REMOVE", "DELETE", "PUT", "POST", "PATCH", "SET", "CREATE",
    "ALTER", "RENAME", "PUBLISH", "PRODUCE", "SEND", "MUTATION", "LOAD", "CLEAR", "ADD", "DEFINE", "RELATE",
    "INSTALL", "GRANT", "REVOKE", "MOVE", "COPY", "MERGE", "TRUNCATE", "DROP", "PURGE", "WITH", "IMPORT",
];

/// HTTP paths a POST still only reads through (search and count endpoints).
const READ_PATHS: &[&str] = &[
    "_search", "_msearch", "_count", "_mget", "_explain", "_validate", "_field_caps", "_sql", "_eql", "_esql",
    "_rank_eval", "_terms_enum", "_analyze", "_find", "_all_docs", "_explain", "/search", "multi_search",
    "/query", "/scroll", "/count", "/recommend", "/discover", "/similar", "/facet-search", "/graphql",
];

fn native(engine: Engine, statement: &str) -> Verdict {
    let text = statement.trim();
    if text.starts_with('{') || text.starts_with('[') {
        return json_command(engine, text);
    }
    let words = words_upper(&strip_quoted(text));
    let Some(verb) = words.iter().find(|w| !matches!(w.as_str(), "PREFIX" | "BASE" | "IMPORT" | "DECLARE" | "XQUERY" | "VERSION")) else {
        return READ;
    };
    let verb = verb.as_str();
    let has = |kw: &str| words.iter().any(|w| w == kw);

    // A REST line (`DELETE /index`, `POST /i/_search`).
    if REST_VERBS.contains(&verb) && text.split_whitespace().nth(1).is_some_and(|p| p.starts_with('/') || p.starts_with('_') || p.contains('/')) {
        let path = text.split_whitespace().nth(1).unwrap_or_default().to_ascii_lowercase();
        return rest(verb, &path);
    }

    match engine.family() {
        // PromQL / MetricsQL only query.
        Family::Prometheus => return READ,
        Family::Convex => return if verb == "QUERY" || verb == "FUNCTIONS" || verb == "TABLES" { READ } else { WRITE },
        // HBase shell: get / scan / list read; put / delete / truncate / drop do not.
        Family::Hbase => {
            return match text.split_whitespace().next().unwrap_or_default().to_ascii_lowercase().as_str() {
                "get" | "scan" | "list" | "describe" | "count" | "exists" | "status" => READ,
                "drop" | "truncate" | "deleteall" => destroy("This removes a whole table or row."),
                _ => WRITE,
            };
        }
        Family::Sparql => {
            return if ["INSERT", "DELETE", "LOAD", "CLEAR", "CREATE", "DROP", "COPY", "MOVE", "ADD"].iter().any(|kw| has(kw)) {
                if has("DROP") || has("CLEAR") { destroy("DROP / CLEAR removes a whole graph.") } else { WRITE }
            } else {
                READ
            };
        }
        Family::Basex | Family::Existdb => {
            let lower = text.to_ascii_lowercase();
            let updating = ["insert node", "delete node", "replace node", "replace value", "rename node", "db:create", "db:drop", "db:delete", "db:add", "db:put", "db:replace", "db:rename", "xmldb:store", "xmldb:remove", "xmldb:create-collection", "file:write", "update insert", "update delete", "update replace", "update value", "update rename"];
            return if lower.contains("db:drop") || lower.contains("xmldb:remove") {
                destroy("This removes a whole database or collection.")
            } else if updating.iter().any(|u| lower.contains(u)) {
                WRITE
            } else {
                READ
            };
        }
        Family::Arangodb => {
            return if ["INSERT", "UPDATE", "REPLACE", "REMOVE", "UPSERT"].iter().any(|kw| has(kw)) { WRITE } else { READ };
        }
        Family::Influxdb => {
            if text.contains("|>") || text.starts_with("from(") || text.starts_with("buckets(") || text.starts_with("import") {
                return if text.contains("to(") || text.contains("experimental.to") { WRITE } else { READ };
            }
        }
        Family::Weaviate => {
            if text.starts_with('{') || verb == "GET" || verb == "AGGREGATE" || verb == "EXPLORE" || verb == "QUERY" {
                return READ;
            }
            if verb == "MUTATION" {
                return WRITE;
            }
        }
        Family::Tigergraph => {
            if verb == "INTERPRET" {
                return if ["INSERT", "DELETE", "UPDATE"].iter().any(|kw| has(kw)) { WRITE } else { READ };
            }
            // `RUN QUERY name(...)` runs an installed query that may write.
            if verb == "RUN" {
                return WRITE;
            }
        }
        Family::Orientdb => {
            if text.trim_start().starts_with("g.") {
                let lower = text.to_ascii_lowercase();
                return if ["addv", "adde", "property(", "drop()", "remove"].iter().any(|k| lower.contains(k)) { WRITE } else { READ };
            }
            let mut all = destructive::classify(text);
            return all.pop().map_or(READ, |one| (one.kind, one.reason));
        }
        Family::Objectdb | Family::Spacetimedb | Family::Immudb if !NATIVE_READS.contains(&verb) || verb == "SELECT" => {
            let mut all = destructive::classify(text);
            return all.pop().map_or(READ, |one| (one.kind, one.reason));
        }
        _ => {}
    }

    if verb == "DROP" || verb == "TRUNCATE" || verb == "PURGE" || verb == "FLUSH" {
        return destroy("This removes the object and everything in it.");
    }
    if NATIVE_READS.contains(&verb) {
        // A read-shaped lead with a write later (`FROM x INSERT …`) is a write.
        return if words.iter().skip(1).any(|w| matches!(w.as_str(), "INSERT" | "UPSERT" | "REMOVE" | "MUTATION")) { WRITE } else { READ };
    }
    if NATIVE_WRITES.contains(&verb) {
        return WRITE;
    }
    WRITE
}

fn rest(method: &str, path: &str) -> Verdict {
    match method {
        "GET" | "HEAD" => READ,
        "DELETE" => {
            // Deleting one document is a write; deleting an index, a database or
            // by query wipes data wholesale.
            let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            if path.contains("_delete_by_query") || segments.len() <= 1 || segments.iter().all(|s| s.starts_with('_') || segments.len() == 1) {
                destroy("This DELETE removes a whole index, database or every matching document.")
            } else {
                WRITE
            }
        }
        _ => {
            if READ_PATHS.iter().any(|p| path.contains(p)) {
                READ
            } else {
                WRITE
            }
        }
    }
}

/// A JSON command body (search bodies, `{"method": …, "path": …}` passthroughs,
/// DynamoDB `{"Operation": …}`, Kafka/RabbitMQ consume/publish bodies…).
fn json_command(engine: Engine, text: &str) -> Verdict {
    let Ok(value) = serde_json::from_str::<Json>(text) else {
        // Not JSON: GraphQL (`{ Get { … } }`) and similar brace-led languages.
        return if text.contains("mutation") { WRITE } else if engine.family() == Family::Weaviate { READ } else { WRITE };
    };
    let Some(obj) = value.as_object() else {
        return WRITE;
    };
    // Convex names the call and its kind: `{"function": "m:list", "kind": "query"}`.
    if engine.family() == Family::Convex {
        let kind = obj.get("kind").and_then(Json::as_str).unwrap_or("query").to_ascii_lowercase();
        return if kind == "query" { READ } else { WRITE };
    }
    if engine.family() == Family::Kafka && obj.contains_key("delete") {
        return destroy("This deletes a topic and every message in it.");
    }
    let field = |k: &str| obj.get(k).and_then(Json::as_str).map(str::to_ascii_uppercase);
    if let (Some(method), Some(path)) = (field("method"), obj.get("path").and_then(Json::as_str)) {
        return rest(method.as_str(), &path.to_ascii_lowercase());
    }
    if let Some(path) = obj.get("path").and_then(Json::as_str) {
        // A path with no method is a GET (CouchDB, Druid passthroughs).
        let _ = path;
        return READ;
    }
    if let Some(op) = field("operation").or_else(|| field("op")).or_else(|| field("action")).or_else(|| field("command")) {
        let op = op.as_str();
        if op.starts_with("DELETETABLE") || op.starts_with("DROP") || op == "PURGE" || op == "DELETE_COLLECTION" || op == "DELETEINDEX" {
            return destroy("This removes a whole table, collection or index.");
        }
        let reads = ["GET", "SCAN", "QUERY", "LIST", "DESCRIBE", "SEARCH", "COUNT", "CONSUME", "FETCH", "BATCHGET", "INFO", "STATS", "PEEK"];
        return if reads.iter().any(|r| op.starts_with(r)) { READ } else { WRITE };
    }
    const WRITE_KEYS: &[&str] = &[
        "documents", "delete", "deletes", "import", "upsert", "insert", "update", "updates", "vectors", "points",
        "publish", "produce", "messages", "records", "mutation", "args", "set", "put", "embeddings", "objects",
    ];
    if WRITE_KEYS.iter().any(|k| obj.contains_key(*k)) {
        return WRITE;
    }
    if engine.family() == Family::Arangodb || engine.family() == Family::Objectdb {
        if let Some(query) = obj.get("query").and_then(Json::as_str) {
            return native(engine, query);
        }
    }
    // Anything else is a search / filter / structured query body.
    READ
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(engine: Engine, text: &str) -> Vec<StatementKind> {
        classify(engine, text).into_iter().map(|s| s.kind).collect()
    }

    #[test]
    fn redis_reads_are_reads_and_runs_one_command_per_line() {
        assert_eq!(kinds(Engine::Redis, "GET user:1\nHGETALL session:9\nscan 0 match a*"), vec![StatementKind::Read; 3]);
        assert_eq!(kinds(Engine::Redis, "SET a 1\nDEL a"), vec![StatementKind::Write, StatementKind::Write]);
        assert_eq!(kinds(Engine::Valkey, "FLUSHALL"), vec![StatementKind::Destructive]);
        assert_eq!(kinds(Engine::Redis, "CONFIG GET maxmemory\nCONFIG SET maxmemory 1"), vec![StatementKind::Read, StatementKind::Write]);
        // `;` is not a separator in Redis; a value may hold one.
        assert_eq!(classify(Engine::Redis, "SET k \"a;b\"").len(), 1);
        assert_eq!(kinds(Engine::Redis, "# comment\n\nPING"), vec![StatementKind::Read]);
    }

    #[test]
    fn mongo_reads_json_commands() {
        assert_eq!(kinds(Engine::Mongodb, r#"{"find": "users", "filter": {}}"#), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Mongodb, "find users {\"age\": 3}"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Mongodb, r#"{"aggregate": "u", "pipeline": [{"$match": {}}], "cursor": {}}"#), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Mongodb, r#"{"aggregate": "u", "pipeline": [{"$out": "x"}], "cursor": {}}"#), vec![StatementKind::Write]);
        assert_eq!(kinds(Engine::Mongodb, r#"{"insert": "u", "documents": [{"a": 1}]}"#), vec![StatementKind::Write]);
        assert_eq!(kinds(Engine::Mongodb, r#"{"drop": "u"}"#), vec![StatementKind::Destructive]);
        assert_eq!(kinds(Engine::Mongodb, r#"{"delete": "u", "deletes": [{"q": {}, "limit": 0}]}"#), vec![StatementKind::Destructive]);
        assert_eq!(kinds(Engine::Mongodb, r#"{"delete": "u", "deletes": [{"q": {"a": 1}, "limit": 1}]}"#), vec![StatementKind::Write]);
        // Blank lines separate commands, exactly as the adapter splits them.
        assert_eq!(kinds(Engine::Mongodb, "{\"find\": \"a\"}\n\n{\"count\": \"b\"}"), vec![StatementKind::Read, StatementKind::Read]);
        // A `;` inside a JSON string is not a boundary.
        assert_eq!(classify(Engine::Mongodb, r#"{"find": "a", "filter": {"x": "1;2"}}"#).len(), 1);
    }

    #[test]
    fn cypher_reads_and_writes() {
        assert_eq!(kinds(Engine::Neo4j, "MATCH (n:Person) RETURN n LIMIT 5"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Memgraph, "CREATE (n:Person {name: 'a'})"), vec![StatementKind::Write]);
        assert_eq!(kinds(Engine::Neo4j, "MATCH (n) DETACH DELETE n"), vec![StatementKind::Destructive]);
        assert_eq!(kinds(Engine::Neo4j, "MATCH (n {name: 'Delete me'}) RETURN n"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Neo4j, "MATCH (n) RETURN n; CREATE (m)"), vec![StatementKind::Read, StatementKind::Write]);
    }

    #[test]
    fn console_and_search_engines() {
        assert_eq!(kinds(Engine::Elasticsearch, "GET /logs/_search\n{\n  \"query\": {\"match_all\": {}}\n}"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Opensearch, "POST /logs/_search\n{\"size\": 1}"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Elasticsearch, "PUT /logs/_doc/1\n{\"a\": 1}"), vec![StatementKind::Write]);
        assert_eq!(kinds(Engine::Elasticsearch, "DELETE /logs"), vec![StatementKind::Destructive]);
        assert_eq!(kinds(Engine::Elasticsearch, "GET /a/_count\nGET /b/_count"), vec![StatementKind::Read, StatementKind::Read]);
        assert_eq!(kinds(Engine::Meilisearch, "SEARCH movies matrix"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Typesense, r#"{"collection": "c", "documents": [{"id": "1"}]}"#), vec![StatementKind::Write]);
        assert_eq!(kinds(Engine::Qdrant, "SCROLL points 10"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Prometheus, "rate(http_requests_total[5m])"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Couchdb, r#"{"selector": {"type": "user"}}"#), vec![StatementKind::Read]);
    }

    #[test]
    fn other_native_languages() {
        assert_eq!(kinds(Engine::Arangodb, "FOR u IN users FILTER u.age > 3 RETURN u"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Arangodb, "FOR u IN users REMOVE u IN users"), vec![StatementKind::Write]);
        assert_eq!(kinds(Engine::Graphdb, "PREFIX ex: <http://e/>\nSELECT * WHERE { ?s ?p ?o } LIMIT 10"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Stardog, "INSERT DATA { <a> <b> <c> }"), vec![StatementKind::Write]);
        assert_eq!(kinds(Engine::Kafka, "TOPICS\nCONSUME orders 10"), vec![StatementKind::Read, StatementKind::Read]);
        assert_eq!(kinds(Engine::Rabbitmq, "PUBLISH ex key hello"), vec![StatementKind::Write]);
        assert_eq!(kinds(Engine::S3, "BUCKETS"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Minio, "DELETE bucket key"), vec![StatementKind::Write]);
        assert_eq!(kinds(Engine::Memcached, "get a\nset a 0 0 1\nx\nflush_all"), vec![StatementKind::Read, StatementKind::Write, StatementKind::Destructive]);
        assert_eq!(kinds(Engine::Rocksdb, "--cf meta GET k\nPUT k v"), vec![StatementKind::Read, StatementKind::Write]);
        assert_eq!(kinds(Engine::Basex, "for $b in //book return $b/title"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Influxdb, "from(bucket: \"b\") |> range(start: -1h)"), vec![StatementKind::Read]);
        assert_eq!(kinds(Engine::Weaviate, "{ Get { Article { title } } }"), vec![StatementKind::Read]);
    }

    #[test]
    fn sql_engines_keep_the_sql_classifier() {
        assert_eq!(kinds(Engine::Postgres, "SELECT 1; DROP TABLE t"), vec![StatementKind::Read, StatementKind::Destructive]);
        assert_eq!(kinds(Engine::Cassandra, "SELECT * FROM ks.t; TRUNCATE ks.t"), vec![StatementKind::Read, StatementKind::Destructive]);
    }

    #[test]
    fn every_engine_has_a_dialect_and_spans_point_inside_the_text() {
        for engine in Engine::ALL {
            let text = "GET a\n\nGET b";
            for span in spans(engine, text) {
                assert!(span.start < span.end && span.end as usize <= text.len(), "{engine:?}");
            }
        }
    }

    #[test]
    fn redis_spans_are_lines() {
        let found = spans(Engine::Redis, "GET a\n\nSET b 1\n");
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].start, found[0].end), (0, 5));
        assert_eq!((found[1].start, found[1].end), (7, 14));
        assert_eq!(found[1].intent, StatementIntent::Write);
    }
}
