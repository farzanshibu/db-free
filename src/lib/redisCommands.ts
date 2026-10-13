// SOT: redis-command-catalog, redis-completion, redis-argument-hints, redis-command-quoting
//
// WHAT:  Every Redis command a console user is likely to type (about 200), each
//        with its argument hint and a one-line description, plus the helpers the
//        editor's completion and the key editors use.
// WHY:   The Redis console is a plain text editor; without a catalog it cannot
//        suggest `ZRANGEBYSCORE`, show `key min max [WITHSCORES] [LIMIT offset count]`
//        as the cursor reaches the third argument, or tell `CONFIG GET` from
//        `CONFIG SET`. The key editors build their write commands here too, so
//        the quoting the Rust tokenizer reads back lives in one place.
// HOW:   A flat list of `[name, arguments, description, group]`; sub-commands
//        (`CONFIG GET`) are their own entries, keyed by the two words.
// WHERE: src/features/editor/SqlEditor.tsx (completion), src/features/keys (writes),
//        src-tauri/src/integrations/redis.rs (`tokenize` reads what is written here)

import type { Engine } from "./bindings";

// WHAT:  The engines the Redis adapter serves: wire-compatible products share its
//        key browser, key editors and command catalog.
// WHERE: src-tauri/src/model/connection.rs (Family::Redis)
export function isRedisEngine(engine: Engine): boolean {
  return engine === "redis" || engine === "valkey" || engine === "dragonfly";
}

export type RedisCommandGroup =
  | "string"
  | "key"
  | "hash"
  | "list"
  | "set"
  | "zset"
  | "stream"
  | "pubsub"
  | "transaction"
  | "scripting"
  | "server"
  | "connection"
  | "cluster"
  | "geo"
  | "hyperloglog"
  | "bitmap"
  | "json";

export interface RedisCommandSpec {
  /// `GET`, or `CONFIG GET` for a sub-command.
  name: string;
  /// Argument syntax as Redis documents it: `key [EX seconds]`.
  args: string;
  summary: string;
  group: RedisCommandGroup;
}

type Row = readonly [name: string, args: string, summary: string, group: RedisCommandGroup];

const ROWS: readonly Row[] = [
  // ---- strings
  ["APPEND", "key value", "Append a value to a string key.", "string"],
  ["DECR", "key", "Decrement the integer value of a key by one.", "string"],
  ["DECRBY", "key decrement", "Decrement the integer value of a key by a number.", "string"],
  ["GET", "key", "Get the value of a key.", "string"],
  ["GETDEL", "key", "Get the value of a key and delete it.", "string"],
  ["GETEX", "key [EX seconds | PX milliseconds | EXAT unix-time | PXAT unix-time-ms | PERSIST]", "Get the value of a key and change its expiry.", "string"],
  ["GETRANGE", "key start end", "Get a substring of a string value.", "string"],
  ["GETSET", "key value", "Set a key and return its old value.", "string"],
  ["INCR", "key", "Increment the integer value of a key by one.", "string"],
  ["INCRBY", "key increment", "Increment the integer value of a key by a number.", "string"],
  ["INCRBYFLOAT", "key increment", "Increment the float value of a key by a number.", "string"],
  ["LCS", "key1 key2 [LEN] [IDX] [MINMATCHLEN len] [WITHMATCHLEN]", "Find the longest common substring of two strings.", "string"],
  ["MGET", "key [key ...]", "Get the values of several keys.", "string"],
  ["MSET", "key value [key value ...]", "Set several keys to several values.", "string"],
  ["MSETNX", "key value [key value ...]", "Set several keys only if none of them exists.", "string"],
  ["PSETEX", "key milliseconds value", "Set a key with an expiry in milliseconds.", "string"],
  ["SET", "key value [NX | XX] [GET] [EX seconds | PX milliseconds | EXAT unix-time | PXAT unix-time-ms | KEEPTTL]", "Set the string value of a key.", "string"],
  ["SETEX", "key seconds value", "Set a key with an expiry in seconds.", "string"],
  ["SETNX", "key value", "Set a key only if it does not exist.", "string"],
  ["SETRANGE", "key offset value", "Overwrite part of a string at an offset.", "string"],
  ["STRLEN", "key", "Get the length of a string value.", "string"],
  // ---- keys
  ["COPY", "source destination [DB destination-db] [REPLACE]", "Copy a key.", "key"],
  ["DEL", "key [key ...]", "Delete one or more keys.", "key"],
  ["DUMP", "key", "Serialize the value stored at a key.", "key"],
  ["EXISTS", "key [key ...]", "Count how many of the keys exist.", "key"],
  ["EXPIRE", "key seconds [NX | XX | GT | LT]", "Set a key's time to live in seconds.", "key"],
  ["EXPIREAT", "key unix-time-seconds [NX | XX | GT | LT]", "Expire a key at a Unix timestamp.", "key"],
  ["EXPIRETIME", "key", "Get the expiry of a key as a Unix timestamp.", "key"],
  ["KEYS", "pattern", "Find all keys matching a pattern (blocks the server on big databases).", "key"],
  ["MOVE", "key db", "Move a key to another database.", "key"],
  ["OBJECT ENCODING", "key", "Show the internal encoding of a key's value.", "key"],
  ["OBJECT FREQ", "key", "Show the access frequency of a key (LFU policies).", "key"],
  ["OBJECT IDLETIME", "key", "Show seconds since a key was last accessed.", "key"],
  ["OBJECT REFCOUNT", "key", "Show how many references a value has.", "key"],
  ["PERSIST", "key", "Remove a key's expiry.", "key"],
  ["PEXPIRE", "key milliseconds [NX | XX | GT | LT]", "Set a key's time to live in milliseconds.", "key"],
  ["PEXPIREAT", "key unix-time-ms [NX | XX | GT | LT]", "Expire a key at a Unix timestamp in milliseconds.", "key"],
  ["PEXPIRETIME", "key", "Get the expiry of a key in Unix milliseconds.", "key"],
  ["PTTL", "key", "Get a key's time to live in milliseconds.", "key"],
  ["RANDOMKEY", "", "Return a random key.", "key"],
  ["RENAME", "key newkey", "Rename a key (overwrites the target).", "key"],
  ["RENAMENX", "key newkey", "Rename a key only if the new name is free.", "key"],
  ["RESTORE", "key ttl serialized-value [REPLACE] [ABSTTL] [IDLETIME seconds] [FREQ frequency]", "Create a key from a serialized value.", "key"],
  ["SCAN", "cursor [MATCH pattern] [COUNT count] [TYPE type]", "Iterate the keyspace a step at a time.", "key"],
  ["SORT", "key [BY pattern] [LIMIT offset count] [GET pattern ...] [ASC | DESC] [ALPHA] [STORE destination]", "Sort a list, set or sorted set.", "key"],
  ["SORT_RO", "key [BY pattern] [LIMIT offset count] [GET pattern ...] [ASC | DESC] [ALPHA]", "Read-only variant of SORT.", "key"],
  ["TOUCH", "key [key ...]", "Update the last access time of keys.", "key"],
  ["TTL", "key", "Get a key's time to live in seconds.", "key"],
  ["TYPE", "key", "Get the data type of a key.", "key"],
  ["UNLINK", "key [key ...]", "Delete keys in a background thread.", "key"],
  ["WAIT", "numreplicas timeout", "Wait for writes to reach replicas.", "key"],
  // ---- hashes
  ["HDEL", "key field [field ...]", "Delete fields from a hash.", "hash"],
  ["HEXISTS", "key field", "Check whether a hash has a field.", "hash"],
  ["HGET", "key field", "Get the value of a hash field.", "hash"],
  ["HGETALL", "key", "Get every field and value of a hash.", "hash"],
  ["HINCRBY", "key field increment", "Increment an integer hash field.", "hash"],
  ["HINCRBYFLOAT", "key field increment", "Increment a float hash field.", "hash"],
  ["HKEYS", "key", "Get all field names of a hash.", "hash"],
  ["HLEN", "key", "Count the fields of a hash.", "hash"],
  ["HMGET", "key field [field ...]", "Get several hash fields.", "hash"],
  ["HMSET", "key field value [field value ...]", "Set several hash fields (use HSET).", "hash"],
  ["HRANDFIELD", "key [count [WITHVALUES]]", "Return random fields of a hash.", "hash"],
  ["HSCAN", "key cursor [MATCH pattern] [COUNT count] [NOVALUES]", "Iterate the fields of a hash.", "hash"],
  ["HSET", "key field value [field value ...]", "Set hash fields.", "hash"],
  ["HSETNX", "key field value", "Set a hash field only if it does not exist.", "hash"],
  ["HSTRLEN", "key field", "Get the length of a hash field's value.", "hash"],
  ["HVALS", "key", "Get all values of a hash.", "hash"],
  // ---- lists
  ["BLMOVE", "source destination LEFT | RIGHT LEFT | RIGHT timeout", "Move an element between lists, blocking.", "list"],
  ["BLMPOP", "timeout numkeys key [key ...] LEFT | RIGHT [COUNT count]", "Pop from the first non-empty list, blocking.", "list"],
  ["BLPOP", "key [key ...] timeout", "Pop the first element of a list, blocking.", "list"],
  ["BRPOP", "key [key ...] timeout", "Pop the last element of a list, blocking.", "list"],
  ["BRPOPLPUSH", "source destination timeout", "Pop from one list and push to another, blocking.", "list"],
  ["LINDEX", "key index", "Get a list element by index.", "list"],
  ["LINSERT", "key BEFORE | AFTER pivot element", "Insert an element before or after another.", "list"],
  ["LLEN", "key", "Count the elements of a list.", "list"],
  ["LMOVE", "source destination LEFT | RIGHT LEFT | RIGHT", "Move an element between lists.", "list"],
  ["LMPOP", "numkeys key [key ...] LEFT | RIGHT [COUNT count]", "Pop from the first non-empty list.", "list"],
  ["LPOP", "key [count]", "Remove and return the first elements of a list.", "list"],
  ["LPOS", "key element [RANK rank] [COUNT num-matches] [MAXLEN len]", "Find the index of an element.", "list"],
  ["LPUSH", "key element [element ...]", "Prepend elements to a list.", "list"],
  ["LPUSHX", "key element [element ...]", "Prepend to a list only if it exists.", "list"],
  ["LRANGE", "key start stop", "Get a range of list elements.", "list"],
  ["LREM", "key count element", "Remove elements equal to a value.", "list"],
  ["LSET", "key index element", "Set a list element by index.", "list"],
  ["LTRIM", "key start stop", "Keep only a range of a list.", "list"],
  ["RPOP", "key [count]", "Remove and return the last elements of a list.", "list"],
  ["RPOPLPUSH", "source destination", "Pop from one list and push to another.", "list"],
  ["RPUSH", "key element [element ...]", "Append elements to a list.", "list"],
  ["RPUSHX", "key element [element ...]", "Append to a list only if it exists.", "list"],
  // ---- sets
  ["SADD", "key member [member ...]", "Add members to a set.", "set"],
  ["SCARD", "key", "Count the members of a set.", "set"],
  ["SDIFF", "key [key ...]", "Subtract sets.", "set"],
  ["SDIFFSTORE", "destination key [key ...]", "Subtract sets and store the result.", "set"],
  ["SINTER", "key [key ...]", "Intersect sets.", "set"],
  ["SINTERCARD", "numkeys key [key ...] [LIMIT limit]", "Count the members of an intersection.", "set"],
  ["SINTERSTORE", "destination key [key ...]", "Intersect sets and store the result.", "set"],
  ["SISMEMBER", "key member", "Check whether a value is in a set.", "set"],
  ["SMEMBERS", "key", "Get all members of a set.", "set"],
  ["SMISMEMBER", "key member [member ...]", "Check several values against a set.", "set"],
  ["SMOVE", "source destination member", "Move a member between sets.", "set"],
  ["SPOP", "key [count]", "Remove and return random members of a set.", "set"],
  ["SRANDMEMBER", "key [count]", "Return random members of a set.", "set"],
  ["SREM", "key member [member ...]", "Remove members from a set.", "set"],
  ["SSCAN", "key cursor [MATCH pattern] [COUNT count]", "Iterate the members of a set.", "set"],
  ["SUNION", "key [key ...]", "Union of sets.", "set"],
  ["SUNIONSTORE", "destination key [key ...]", "Union sets and store the result.", "set"],
  // ---- sorted sets
  ["BZPOPMAX", "key [key ...] timeout", "Pop the highest-scored member, blocking.", "zset"],
  ["BZPOPMIN", "key [key ...] timeout", "Pop the lowest-scored member, blocking.", "zset"],
  ["ZADD", "key [NX | XX] [GT | LT] [CH] [INCR] score member [score member ...]", "Add or update sorted-set members.", "zset"],
  ["ZCARD", "key", "Count the members of a sorted set.", "zset"],
  ["ZCOUNT", "key min max", "Count members within a score range.", "zset"],
  ["ZDIFF", "numkeys key [key ...] [WITHSCORES]", "Subtract sorted sets.", "zset"],
  ["ZINCRBY", "key increment member", "Increment a member's score.", "zset"],
  ["ZINTER", "numkeys key [key ...] [WEIGHTS weight ...] [AGGREGATE SUM | MIN | MAX] [WITHSCORES]", "Intersect sorted sets.", "zset"],
  ["ZINTERSTORE", "destination numkeys key [key ...] [WEIGHTS weight ...] [AGGREGATE SUM | MIN | MAX]", "Intersect sorted sets and store the result.", "zset"],
  ["ZLEXCOUNT", "key min max", "Count members within a lexicographic range.", "zset"],
  ["ZMSCORE", "key member [member ...]", "Get the scores of several members.", "zset"],
  ["ZPOPMAX", "key [count]", "Remove and return the highest-scored members.", "zset"],
  ["ZPOPMIN", "key [count]", "Remove and return the lowest-scored members.", "zset"],
  ["ZRANDMEMBER", "key [count [WITHSCORES]]", "Return random members of a sorted set.", "zset"],
  ["ZRANGE", "key start stop [BYSCORE | BYLEX] [REV] [LIMIT offset count] [WITHSCORES]", "Get a range of members by index, score or lex.", "zset"],
  ["ZRANGEBYLEX", "key min max [LIMIT offset count]", "Get members by lexicographic range.", "zset"],
  ["ZRANGEBYSCORE", "key min max [WITHSCORES] [LIMIT offset count]", "Get members by score range.", "zset"],
  ["ZRANGESTORE", "dst src min max [BYSCORE | BYLEX] [REV] [LIMIT offset count]", "Store a range of members in another key.", "zset"],
  ["ZRANK", "key member [WITHSCORE]", "Get a member's rank, lowest score first.", "zset"],
  ["ZREM", "key member [member ...]", "Remove members from a sorted set.", "zset"],
  ["ZREMRANGEBYLEX", "key min max", "Remove members by lexicographic range.", "zset"],
  ["ZREMRANGEBYRANK", "key start stop", "Remove members by rank.", "zset"],
  ["ZREMRANGEBYSCORE", "key min max", "Remove members by score range.", "zset"],
  ["ZREVRANGE", "key start stop [WITHSCORES]", "Get a range of members, highest score first.", "zset"],
  ["ZREVRANGEBYLEX", "key max min [LIMIT offset count]", "Get members by lex range, reversed.", "zset"],
  ["ZREVRANGEBYSCORE", "key max min [WITHSCORES] [LIMIT offset count]", "Get members by score range, highest first.", "zset"],
  ["ZREVRANK", "key member [WITHSCORE]", "Get a member's rank, highest score first.", "zset"],
  ["ZSCAN", "key cursor [MATCH pattern] [COUNT count]", "Iterate the members of a sorted set.", "zset"],
  ["ZSCORE", "key member", "Get a member's score.", "zset"],
  ["ZUNION", "numkeys key [key ...] [WEIGHTS weight ...] [AGGREGATE SUM | MIN | MAX] [WITHSCORES]", "Union of sorted sets.", "zset"],
  ["ZUNIONSTORE", "destination numkeys key [key ...] [WEIGHTS weight ...] [AGGREGATE SUM | MIN | MAX]", "Union sorted sets and store the result.", "zset"],
  // ---- streams
  ["XACK", "key group id [id ...]", "Acknowledge stream messages.", "stream"],
  ["XADD", "key [NOMKSTREAM] [MAXLEN | MINID [= | ~] threshold [LIMIT count]] * | id field value [field value ...]", "Append an entry to a stream.", "stream"],
  ["XAUTOCLAIM", "key group consumer min-idle-time start [COUNT count] [JUSTID]", "Claim idle pending messages automatically.", "stream"],
  ["XCLAIM", "key group consumer min-idle-time id [id ...]", "Claim pending messages.", "stream"],
  ["XDEL", "key id [id ...]", "Delete stream entries.", "stream"],
  ["XGROUP CREATE", "key group id | $ [MKSTREAM] [ENTRIESREAD entries-read]", "Create a consumer group.", "stream"],
  ["XGROUP CREATECONSUMER", "key group consumer", "Create a consumer in a group.", "stream"],
  ["XGROUP DELCONSUMER", "key group consumer", "Delete a consumer from a group.", "stream"],
  ["XGROUP DESTROY", "key group", "Destroy a consumer group.", "stream"],
  ["XGROUP SETID", "key group id | $ [ENTRIESREAD entries-read]", "Set a group's last delivered id.", "stream"],
  ["XINFO CONSUMERS", "key group", "List a group's consumers.", "stream"],
  ["XINFO GROUPS", "key", "List a stream's consumer groups.", "stream"],
  ["XINFO STREAM", "key [FULL [COUNT count]]", "Describe a stream.", "stream"],
  ["XLEN", "key", "Count the entries of a stream.", "stream"],
  ["XPENDING", "key group [[IDLE min-idle-time] start end count [consumer]]", "Inspect pending messages of a group.", "stream"],
  ["XRANGE", "key start end [COUNT count]", "Get stream entries in an id range.", "stream"],
  ["XREAD", "[COUNT count] [BLOCK milliseconds] STREAMS key [key ...] id [id ...]", "Read entries from streams.", "stream"],
  ["XREADGROUP", "GROUP group consumer [COUNT count] [BLOCK milliseconds] [NOACK] STREAMS key [key ...] id [id ...]", "Read entries as a group consumer.", "stream"],
  ["XREVRANGE", "key end start [COUNT count]", "Get stream entries in reverse.", "stream"],
  ["XSETID", "key last-id [ENTRIESADDED entries-added] [MAXDELETEDID max-deleted-id]", "Set a stream's last id.", "stream"],
  ["XTRIM", "key MAXLEN | MINID [= | ~] threshold [LIMIT count]", "Trim a stream.", "stream"],
  // ---- pub/sub
  ["PSUBSCRIBE", "pattern [pattern ...]", "Listen to channels matching patterns.", "pubsub"],
  ["PUBLISH", "channel message", "Post a message to a channel.", "pubsub"],
  ["PUBSUB CHANNELS", "[pattern]", "List active channels.", "pubsub"],
  ["PUBSUB NUMPAT", "", "Count pattern subscriptions.", "pubsub"],
  ["PUBSUB NUMSUB", "[channel [channel ...]]", "Count subscribers of channels.", "pubsub"],
  ["PUNSUBSCRIBE", "[pattern [pattern ...]]", "Stop listening to patterns.", "pubsub"],
  ["SPUBLISH", "shardchannel message", "Post a message to a shard channel.", "pubsub"],
  ["SSUBSCRIBE", "shardchannel [shardchannel ...]", "Listen to shard channels.", "pubsub"],
  ["SUBSCRIBE", "channel [channel ...]", "Listen to channels.", "pubsub"],
  ["UNSUBSCRIBE", "[channel [channel ...]]", "Stop listening to channels.", "pubsub"],
  // ---- transactions
  ["DISCARD", "", "Discard the queued commands of a transaction.", "transaction"],
  ["EXEC", "", "Run the queued commands of a transaction.", "transaction"],
  ["MULTI", "", "Start a transaction.", "transaction"],
  ["UNWATCH", "", "Forget all watched keys.", "transaction"],
  ["WATCH", "key [key ...]", "Watch keys for changes before a transaction.", "transaction"],
  // ---- scripting
  ["EVAL", "script numkeys [key [key ...]] [arg [arg ...]]", "Run a Lua script.", "scripting"],
  ["EVAL_RO", "script numkeys [key [key ...]] [arg [arg ...]]", "Run a read-only Lua script.", "scripting"],
  ["EVALSHA", "sha1 numkeys [key [key ...]] [arg [arg ...]]", "Run a cached Lua script by digest.", "scripting"],
  ["EVALSHA_RO", "sha1 numkeys [key [key ...]] [arg [arg ...]]", "Run a cached read-only script by digest.", "scripting"],
  ["FCALL", "function numkeys [key [key ...]] [arg [arg ...]]", "Call a function.", "scripting"],
  ["FCALL_RO", "function numkeys [key [key ...]] [arg [arg ...]]", "Call a read-only function.", "scripting"],
  ["FUNCTION DELETE", "library-name", "Delete a function library.", "scripting"],
  ["FUNCTION FLUSH", "[ASYNC | SYNC]", "Delete every function library.", "scripting"],
  ["FUNCTION LIST", "[LIBRARYNAME pattern] [WITHCODE]", "List function libraries.", "scripting"],
  ["FUNCTION LOAD", "[REPLACE] function-code", "Load a function library.", "scripting"],
  ["SCRIPT EXISTS", "sha1 [sha1 ...]", "Check whether scripts are cached.", "scripting"],
  ["SCRIPT FLUSH", "[ASYNC | SYNC]", "Clear the script cache.", "scripting"],
  ["SCRIPT LOAD", "script", "Cache a script and return its digest.", "scripting"],
  // ---- server
  ["ACL CAT", "[category]", "List ACL categories or the commands in one.", "server"],
  ["ACL DELUSER", "username [username ...]", "Delete users.", "server"],
  ["ACL GETUSER", "username", "Show a user's rules.", "server"],
  ["ACL LIST", "", "List users and their rules.", "server"],
  ["ACL LOG", "[count | RESET]", "Show recent ACL denials.", "server"],
  ["ACL SETUSER", "username [rule [rule ...]]", "Create or change a user.", "server"],
  ["ACL USERS", "", "List usernames.", "server"],
  ["ACL WHOAMI", "", "Show the current user.", "server"],
  ["BGREWRITEAOF", "", "Rewrite the append-only file in the background.", "server"],
  ["BGSAVE", "[SCHEDULE]", "Save a snapshot in the background.", "server"],
  ["CLIENT GETNAME", "", "Get the connection's name.", "server"],
  ["CLIENT ID", "", "Get the connection's id.", "server"],
  ["CLIENT INFO", "", "Describe the current connection.", "server"],
  ["CLIENT KILL", "ID client-id | ADDR ip:port | USER username | TYPE type", "Close connections.", "server"],
  ["CLIENT LIST", "[TYPE normal | master | replica | pubsub] [ID client-id ...]", "List connections.", "server"],
  ["CLIENT PAUSE", "timeout [WRITE | ALL]", "Pause command processing.", "server"],
  ["CLIENT SETNAME", "connection-name", "Name the connection.", "server"],
  ["CLIENT UNPAUSE", "", "Resume command processing.", "server"],
  ["COMMAND", "", "List every command the server knows.", "server"],
  ["COMMAND COUNT", "", "Count the commands the server knows.", "server"],
  ["COMMAND DOCS", "[command-name ...]", "Show documentation of commands.", "server"],
  ["COMMAND INFO", "[command-name ...]", "Show details of commands.", "server"],
  ["CONFIG GET", "parameter [parameter ...]", "Read configuration parameters.", "server"],
  ["CONFIG RESETSTAT", "", "Reset the statistics INFO reports.", "server"],
  ["CONFIG REWRITE", "", "Write the running configuration to redis.conf.", "server"],
  ["CONFIG SET", "parameter value [parameter value ...]", "Change configuration parameters.", "server"],
  ["DBSIZE", "", "Count the keys of the current database.", "server"],
  ["FLUSHALL", "[ASYNC | SYNC]", "Delete every key of every database.", "server"],
  ["FLUSHDB", "[ASYNC | SYNC]", "Delete every key of the current database.", "server"],
  ["INFO", "[section [section ...]]", "Server information and statistics.", "server"],
  ["LASTSAVE", "", "Unix time of the last successful save.", "server"],
  ["LATENCY LATEST", "", "Latest latency spikes per event.", "server"],
  ["LOLWUT", "[VERSION version]", "Display some computer art and the Redis version.", "server"],
  ["MEMORY DOCTOR", "", "Memory problems report.", "server"],
  ["MEMORY STATS", "", "Memory usage details.", "server"],
  ["MEMORY USAGE", "key [SAMPLES count]", "Bytes a key and its value use.", "server"],
  ["MODULE LIST", "", "List loaded modules.", "server"],
  ["MONITOR", "", "Stream every command the server receives.", "server"],
  ["REPLICAOF", "host port | NO ONE", "Make this server a replica of another.", "server"],
  ["ROLE", "", "Show the replication role.", "server"],
  ["SAVE", "", "Save a snapshot, blocking.", "server"],
  ["SHUTDOWN", "[NOSAVE | SAVE] [NOW] [FORCE]", "Stop the server.", "server"],
  ["SLOWLOG GET", "[count]", "Read the slow log.", "server"],
  ["SLOWLOG LEN", "", "Count slow log entries.", "server"],
  ["SLOWLOG RESET", "", "Clear the slow log.", "server"],
  ["SWAPDB", "index1 index2", "Swap two databases.", "server"],
  ["TIME", "", "Server time as seconds and microseconds.", "server"],
  // ---- connection
  ["AUTH", "[username] password", "Authenticate to the server.", "connection"],
  ["ECHO", "message", "Echo a message.", "connection"],
  ["HELLO", "[protover [AUTH username password] [SETNAME clientname]]", "Switch protocol version and authenticate.", "connection"],
  ["PING", "[message]", "Check the connection.", "connection"],
  ["QUIT", "", "Close the connection.", "connection"],
  ["RESET", "", "Reset the connection's state.", "connection"],
  ["SELECT", "index", "Change the selected database.", "connection"],
  // ---- cluster
  ["CLUSTER INFO", "", "Cluster state and statistics.", "cluster"],
  ["CLUSTER KEYSLOT", "key", "Hash slot of a key.", "cluster"],
  ["CLUSTER MYID", "", "This node's id.", "cluster"],
  ["CLUSTER NODES", "", "The cluster's nodes.", "cluster"],
  ["CLUSTER SHARDS", "", "The cluster's shards.", "cluster"],
  ["CLUSTER SLOTS", "", "Slot-to-node mapping.", "cluster"],
  ["READONLY", "", "Allow reads on a replica.", "cluster"],
  ["READWRITE", "", "Undo READONLY.", "cluster"],
  // ---- geo
  ["GEOADD", "key [NX | XX] [CH] longitude latitude member [longitude latitude member ...]", "Add geospatial members.", "geo"],
  ["GEODIST", "key member1 member2 [M | KM | FT | MI]", "Distance between two members.", "geo"],
  ["GEOHASH", "key member [member ...]", "Geohash strings of members.", "geo"],
  ["GEOPOS", "key member [member ...]", "Coordinates of members.", "geo"],
  ["GEORADIUS", "key longitude latitude radius M | KM | FT | MI [WITHCOORD] [WITHDIST] [COUNT count] [ASC | DESC]", "Members within a radius of a point.", "geo"],
  ["GEORADIUSBYMEMBER", "key member radius M | KM | FT | MI [WITHCOORD] [WITHDIST] [COUNT count] [ASC | DESC]", "Members within a radius of a member.", "geo"],
  ["GEOSEARCH", "key FROMMEMBER member | FROMLONLAT longitude latitude BYRADIUS radius unit | BYBOX width height unit [ASC | DESC] [COUNT count [ANY]] [WITHCOORD] [WITHDIST] [WITHHASH]", "Search members in an area.", "geo"],
  ["GEOSEARCHSTORE", "destination source FROMMEMBER member | FROMLONLAT longitude latitude BYRADIUS radius unit | BYBOX width height unit [STOREDIST]", "Store a geo search.", "geo"],
  // ---- hyperloglog
  ["PFADD", "key [element [element ...]]", "Add elements to a HyperLogLog.", "hyperloglog"],
  ["PFCOUNT", "key [key ...]", "Estimate the cardinality.", "hyperloglog"],
  ["PFMERGE", "destkey [sourcekey [sourcekey ...]]", "Merge HyperLogLogs.", "hyperloglog"],
  // ---- bitmaps
  ["BITCOUNT", "key [start end [BYTE | BIT]]", "Count set bits.", "bitmap"],
  ["BITFIELD", "key [GET encoding offset | SET encoding offset value | INCRBY encoding offset increment | OVERFLOW WRAP | SAT | FAIL ...]", "Operate on integer fields of a string.", "bitmap"],
  ["BITOP", "AND | OR | XOR | NOT destkey key [key ...]", "Bitwise operation between strings.", "bitmap"],
  ["BITPOS", "key bit [start [end [BYTE | BIT]]]", "Position of the first 0 or 1 bit.", "bitmap"],
  ["GETBIT", "key offset", "Read a bit.", "bitmap"],
  ["SETBIT", "key offset value", "Write a bit.", "bitmap"],
  // ---- RedisJSON
  ["JSON.ARRAPPEND", "key path value [value ...]", "Append values to an array.", "json"],
  ["JSON.ARRINDEX", "key path value [start [stop]]", "Index of a value in an array.", "json"],
  ["JSON.ARRINSERT", "key path index value [value ...]", "Insert values into an array.", "json"],
  ["JSON.ARRLEN", "key [path]", "Length of an array.", "json"],
  ["JSON.ARRPOP", "key [path [index]]", "Pop an element of an array.", "json"],
  ["JSON.ARRTRIM", "key path start stop", "Trim an array.", "json"],
  ["JSON.CLEAR", "key [path]", "Clear containers and zero numbers.", "json"],
  ["JSON.DEL", "key [path]", "Delete a value.", "json"],
  ["JSON.GET", "key [path [path ...]]", "Read a JSON document.", "json"],
  ["JSON.MGET", "key [key ...] path", "Read the same path from several keys.", "json"],
  ["JSON.NUMINCRBY", "key path value", "Add to a number.", "json"],
  ["JSON.OBJKEYS", "key [path]", "Keys of an object.", "json"],
  ["JSON.OBJLEN", "key [path]", "Number of keys of an object.", "json"],
  ["JSON.SET", "key path value [NX | XX]", "Write a JSON value.", "json"],
  ["JSON.STRAPPEND", "key [path] value", "Append to a string value.", "json"],
  ["JSON.STRLEN", "key [path]", "Length of a string value.", "json"],
  ["JSON.TOGGLE", "key path", "Toggle a boolean.", "json"],
  ["JSON.TYPE", "key [path]", "Type of a value.", "json"],
];

export const REDIS_COMMANDS: readonly RedisCommandSpec[] = ROWS.map(([name, args, summary, group]) => ({ name, args, summary, group }));

const BY_NAME: ReadonlyMap<string, RedisCommandSpec> = new Map(REDIS_COMMANDS.map((c) => [c.name, c]));

/// Commands that have sub-commands (`CONFIG GET`), so a first word alone is not a command.
const PARENTS: ReadonlySet<string> = new Set(REDIS_COMMANDS.filter((c) => c.name.includes(" ")).map((c) => c.name.split(" ")[0] ?? ""));

export function redisCommand(name: string): RedisCommandSpec | undefined {
  return BY_NAME.get(name.toUpperCase());
}

// WHAT:  The words of one command line, honouring "double" and 'single' quotes the
//        way the Rust tokenizer does; `open` says the last word is unfinished.
function words(line: string): { words: string[]; open: boolean } {
  const out: string[] = [];
  let current = "";
  let quote: string | null = null;
  let inWord = false;
  for (let i = 0; i < line.length; i += 1) {
    const c = line.charAt(i);
    if (quote !== null) {
      if (c === "\\" && quote === '"') {
        current += c + line.charAt(i + 1);
        i += 1;
      } else {
        current += c;
        if (c === quote) quote = null;
      }
    } else if (c === '"' || c === "'") {
      quote = c;
      inWord = true;
      current += c;
    } else if (/\s/.test(c)) {
      if (inWord) {
        out.push(current);
        current = "";
        inWord = false;
      }
    } else {
      inWord = true;
      current += c;
    }
  }
  if (inWord) out.push(current);
  return { words: out, open: inWord };
}

export interface RedisCommandContext {
  spec: RedisCommandSpec;
  /// How many words of the line belong to the command's name (1, or 2 for `CONFIG GET`).
  nameWords: number;
  /// Zero-based index of the argument the caret is on; -1 while still on the name.
  argument: number;
}

// WHAT:  Which command the caret is inside and which argument it is on.
// WHY:   The completion list shows the syntax of the command being typed.
export function commandContext(lineBeforeCaret: string): RedisCommandContext | null {
  const { words: parts, open } = words(lineBeforeCaret.replace(/^\s+/, ""));
  const first = parts[0];
  if (first === undefined) return null;
  const two = parts.length > 1 ? redisCommand(`${first} ${parts[1] ?? ""}`) : undefined;
  const spec = two ?? redisCommand(first);
  if (spec === undefined) return null;
  const nameWords = spec.name.split(" ").length;
  if (parts.length < nameWords || (parts.length === nameWords && open)) return { spec, nameWords, argument: -1 };
  return { spec, nameWords, argument: parts.length - nameWords - (open ? 1 : 0) };
}

export interface RedisCompletion {
  label: string;
  detail: string;
  info: string;
  group: RedisCommandGroup;
}

// WHAT:  Command names that start with what has been typed so far on this line.
// WHY:   Only the first word (or, after a parent such as CONFIG, the second) is
//        a command; arguments are keys and values and get no suggestions.
// HOW:   Prefix matches only, shortest first, so `GET` lists before `GETEX`.
export function redisCompletions(lineBeforeCaret: string, limit = 12): RedisCompletion[] {
  const line = lineBeforeCaret.replace(/^\s+/, "");
  const { words: parts, open } = words(line);
  if (parts.length === 0 || (!open && parts.length > 0 && !PARENTS.has((parts[0] ?? "").toUpperCase()))) return [];
  const typed = (open ? parts : [...parts, ""]).join(" ").toUpperCase();
  const wordCount = typed.split(" ").length;
  if (wordCount > 2) return [];
  if (wordCount === 2 && !PARENTS.has(typed.split(" ")[0] ?? "")) return [];
  return REDIS_COMMANDS.filter((c) => c.name.startsWith(typed))
    .filter((c) => (wordCount === 1 ? true : c.name.split(" ").length === 2))
    .sort((a, b) => a.name.length - b.name.length || a.name.localeCompare(b.name))
    .slice(0, limit)
    .map((c) => ({ label: c.name, detail: c.args, info: c.summary, group: c.group }));
}

// WHAT:  The completion the editor shows for the line up to the caret, and where
//        in that line the text to replace begins (the command's first letter).
export function redisCompletionAt(lineBeforeCaret: string): { from: number; items: RedisCompletion[] } | null {
  const items = redisCompletions(lineBeforeCaret);
  if (items.length === 0) return null;
  return { from: lineBeforeCaret.length - lineBeforeCaret.trimStart().length, items };
}

// WHAT:  A key or value written so the Rust tokenizer reads it back verbatim.
// WHY:   Every write the key editors make is a command line run through
//        `execute_query`, so a value with a space, a quote, a backslash or a
//        newline has to survive the round trip. Control characters are written
//        as escapes; other text (including non-ASCII) is left as it is.
export function quoteRedisArg(raw: string): string {
  if (raw.length > 0 && !/[\s"'\\\u0000-\u001f\u007f]/.test(raw)) return raw;
  let out = '"';
  for (const c of raw) {
    switch (c) {
      case '"':
        out += '\\"';
        break;
      case "\\":
        out += "\\\\";
        break;
      case "\n":
        out += "\\n";
        break;
      case "\r":
        out += "\\r";
        break;
      case "\t":
        out += "\\t";
        break;
      default: {
        const code = c.codePointAt(0) ?? 0;
        out += code < 0x20 || code === 0x7f ? `\\x${code.toString(16).padStart(2, "0")}` : c;
      }
    }
  }
  return `${out}"`;
}

// WHAT:  A command line from its words, each quoted as needed.
export function redisLine(...parts: readonly string[]): string {
  return parts.map(quoteRedisArg).join(" ");
}
