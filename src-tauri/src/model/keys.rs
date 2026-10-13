// SOT: redis-key-scan, redis-key-page, redis-key-entry, redis-key-type, redis-key-info, key-browser-model

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  The data type a Redis key holds, as the key browser names it.
// WHY:   The tree draws an icon per type and the key tab picks an editor per
//        type, so both need a closed set rather than the server's free-form
//        `TYPE` reply (modules add their own: `ReJSON-RL`, `MBbloom--`…).
// HOW:   `Json` is RedisJSON (`TYPE` answers `ReJSON-RL`); anything this app
//        has no editor for is `Other` and opens read-only.
// WHERE: src-tauri/src/integrations/redis.rs (producer), src/features/keys
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RedisKeyType {
    String,
    Hash,
    List,
    Set,
    Zset,
    Stream,
    Json,
    Other,
}

impl RedisKeyType {
    // WHAT:  `TYPE` reply → type. A missing key (`none`) has no type.
    pub fn parse(raw: &str) -> Option<RedisKeyType> {
        match raw {
            "none" => None,
            "string" => Some(RedisKeyType::String),
            "hash" => Some(RedisKeyType::Hash),
            "list" => Some(RedisKeyType::List),
            "set" => Some(RedisKeyType::Set),
            "zset" => Some(RedisKeyType::Zset),
            "stream" => Some(RedisKeyType::Stream),
            "ReJSON-RL" => Some(RedisKeyType::Json),
            _ => Some(RedisKeyType::Other),
        }
    }
}

// WHAT:  One step of the key browser's cursor-paged `SCAN`.
// WHY:   The sidebar lists a keyspace of any size a batch at a time, and the
//        search box asks the server for matches instead of filtering a capped
//        list on the client.
// HOW:   `cursor` is Redis's own, opaque, and "0" both starts a scan and ends
//        one. `search` is plain text: the adapter turns it into `*text*` (and
//        escapes it) unless it already holds a glob character. `count` is the
//        SCAN COUNT hint per round trip; `limit` is how many keys one page
//        should gather before returning, so a sparse `MATCH` still fills a page.
// WHERE: src/features/keys/keyBrowser.ts, src-tauri/src/integrations/redis.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RedisKeyScan {
    pub search: Option<String>,
    pub cursor: String,
    pub count: u32,
    pub limit: u32,
    /// Attach a short value preview to string keys (one pipeline per page).
    pub previews: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RedisKeyEntry {
    pub name: String,
    pub kind: RedisKeyType,
    /// First 80 characters of a string key's value, whitespace collapsed; for a
    /// collection, its size ("12 fields"). None unless previews were asked for.
    pub preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RedisKeyPage {
    pub keys: Vec<RedisKeyEntry>,
    /// Pass back as `RedisKeyScan::cursor` for the next page.
    pub cursor: String,
    /// The scan visited the whole keyspace; there is no next page.
    pub done: bool,
    /// `DBSIZE`: how many keys the database holds, for "N of ~M" progress.
    pub total: Option<i64>,
}

// WHAT:  Everything the key tab shows next to a key's name.
// WHY:   Fetched in one pipeline so opening a key adds one round trip, not a
//        chain of them; each optional figure is None on a server that lacks the
//        command (Redis < 4 has no MEMORY USAGE; an LFU policy has no IDLETIME).
// HOW:   `ttl_ms` follows PTTL: -1 no expiry, -2 no such key. `length` is the
//        element count (STRLEN, HLEN, LLEN, SCARD, ZCARD, XLEN).
// WHERE: src/features/keys/KeyTab.tsx, src-tauri/src/integrations/redis.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RedisKeyInfo {
    pub kind: RedisKeyType,
    /// The server's own `TYPE` reply (`ReJSON-RL`, `TSDB-TYPE`…), for display.
    pub type_name: String,
    pub ttl_ms: i64,
    pub memory_bytes: Option<i64>,
    pub encoding: Option<String>,
    pub idle_seconds: Option<i64>,
    pub length: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_replies_map_to_the_closed_set() {
        assert_eq!(RedisKeyType::parse("string"), Some(RedisKeyType::String));
        assert_eq!(RedisKeyType::parse("zset"), Some(RedisKeyType::Zset));
        assert_eq!(RedisKeyType::parse("ReJSON-RL"), Some(RedisKeyType::Json));
        assert_eq!(RedisKeyType::parse("TSDB-TYPE"), Some(RedisKeyType::Other));
        assert_eq!(RedisKeyType::parse("none"), None);
    }
}
