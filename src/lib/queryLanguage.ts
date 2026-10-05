// SOT: query-language, editor-language, per-engine-editor, native-query-examples, command-vocabulary
import type { Engine, Family } from "@/lib/bindings";
import { ENGINE_FACTS } from "@/lib/bindings/EngineFacts.gen";

// WHAT:  What the query editor speaks for the connection it is open on: its
//        syntax family, the words it completes, how a script is separated, and
//        a working example to start from.
// WHY:   The editor used to be a SQL editor for everyone: SQL keywords in the
//        suggestion list, SQL highlighting and `;` as the only separator. A
//        Redis or Mongo tab offered SELECT and gave no hint of what it accepts.
// HOW:   Keyed by adapter `Family`, so `satisfies Record<Family, …>` fails the
//        build until a new family says what its console speaks. The block's
//        splitter and classifier (src-tauri/src/guard/dialect.rs) use the same
//        per-family mapping; `separator` here only describes it.
// WHERE: src/features/editor/{SqlEditor,QueryPane}.tsx

export type Syntax = "sql" | "command" | "json" | "query";

export interface QueryLanguage {
  /// Shown on the toolbar and in the AI prompt ("Generate Redis command").
  name: string;
  syntax: Syntax;
  /// Words the completion list offers.
  keywords: readonly string[];
  /// How a script is split into statements, as the user should read it.
  separator: string;
  /// A runnable starting point, shown as the placeholder of an empty tab.
  example: string;
  /// Line comment prefix the highlighter greys out.
  comment: string;
}

const SQL_WORDS = [
  "SELECT", "FROM", "WHERE", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "FULL", "CROSS", "ON",
  "GROUP", "BY", "ORDER", "HAVING", "LIMIT", "OFFSET", "DISTINCT", "AS", "AND", "OR", "NOT",
  "NULL", "IS", "IN", "BETWEEN", "LIKE", "ILIKE", "EXISTS", "CASE", "WHEN", "THEN", "ELSE",
  "END", "UNION", "ALL", "INTERSECT", "EXCEPT", "INSERT", "INTO", "VALUES", "UPDATE", "SET",
  "DELETE", "CREATE", "TABLE", "VIEW", "INDEX", "ALTER", "DROP", "WITH", "ASC", "DESC",
  "COUNT", "SUM", "AVG", "MIN", "MAX", "TRUE", "FALSE", "CAST", "COALESCE", "RETURNING",
  "OVER", "PARTITION", "USING", "NATURAL",
];

const SEMI = "Statements end with ;";
const WHOLE = "The whole tab runs as one query";
const BLANK = "Separate commands with a blank line";
const LINE = "One command per line";

function sqlLike(name: string, example: string, extra: readonly string[] = []): QueryLanguage {
  return { name, syntax: "sql", keywords: [...SQL_WORDS, ...extra], separator: SEMI, example, comment: "--" };
}

function lang(name: string, syntax: Syntax, keywords: readonly string[], example: string, separator: string, comment: string): QueryLanguage {
  return { name, syntax, keywords, separator, example, comment };
}

const REDIS_WORDS = [
  "GET", "SET", "MGET", "MSET", "DEL", "UNLINK", "EXISTS", "EXPIRE", "TTL", "PERSIST", "TYPE", "KEYS", "SCAN", "RENAME",
  "INCR", "INCRBY", "DECR", "APPEND", "STRLEN", "GETRANGE", "SETEX", "SETNX",
  "HGET", "HSET", "HMGET", "HGETALL", "HDEL", "HKEYS", "HVALS", "HLEN", "HEXISTS", "HINCRBY", "HSCAN",
  "LPUSH", "RPUSH", "LPOP", "RPOP", "LRANGE", "LLEN", "LINDEX", "LREM", "LTRIM",
  "SADD", "SREM", "SMEMBERS", "SISMEMBER", "SCARD", "SINTER", "SUNION", "SDIFF", "SSCAN",
  "ZADD", "ZREM", "ZRANGE", "ZREVRANGE", "ZRANGEBYSCORE", "ZSCORE", "ZRANK", "ZCARD", "ZINCRBY", "ZSCAN",
  "XADD", "XRANGE", "XREVRANGE", "XLEN", "XREAD", "XINFO", "XDEL", "XTRIM",
  "PUBLISH", "PUBSUB", "INFO", "DBSIZE", "PING", "CONFIG", "CLIENT", "SLOWLOG", "MEMORY", "OBJECT", "SELECT",
  "FLUSHDB", "FLUSHALL", "JSON.GET", "JSON.SET", "FT.SEARCH", "FT.INFO", "TS.RANGE", "MATCH", "COUNT", "LIMIT", "WITHSCORES",
];

const MONGO_WORDS = [
  "find", "count", "aggregate", "distinct", "insert", "update", "delete", "findAndModify", "createIndexes",
  "listCollections", "listIndexes", "listDatabases", "collStats", "dbStats", "serverStatus", "explain", "drop",
  "filter", "projection", "sort", "limit", "skip", "pipeline", "cursor", "documents", "updates", "deletes", "multi", "upsert",
  "$match", "$group", "$project", "$sort", "$limit", "$skip", "$unwind", "$lookup", "$count", "$addFields", "$set",
  "$facet", "$sum", "$avg", "$min", "$max", "$push", "$first", "$last",
  "$eq", "$ne", "$gt", "$gte", "$lt", "$lte", "$in", "$nin", "$and", "$or", "$not", "$exists", "$regex", "$elemMatch", "$db",
];

const CYPHER_WORDS = [
  "MATCH", "OPTIONAL", "WHERE", "RETURN", "WITH", "UNWIND", "ORDER", "BY", "SKIP", "LIMIT", "DISTINCT", "AS",
  "CREATE", "MERGE", "SET", "REMOVE", "DELETE", "DETACH", "ON", "CALL", "YIELD", "UNION", "EXISTS", "COUNT",
  "AND", "OR", "NOT", "IN", "IS", "NULL", "CONTAINS", "STARTS", "ENDS", "CASE", "WHEN", "THEN", "ELSE", "END",
  "SHOW", "INDEXES", "CONSTRAINTS", "DATABASES", "PROFILE", "EXPLAIN", "labels", "type", "id", "properties", "collect",
];

const ES_WORDS = [
  "GET", "POST", "PUT", "DELETE", "HEAD", "_search", "_count", "_doc", "_mapping", "_cat/indices", "_bulk", "query", "match",
  "match_all", "term", "terms", "bool", "must", "should", "filter", "range", "aggs", "size", "from", "sort", "_source",
];

const XQUERY_WORDS = ["for", "let", "where", "order", "by", "return", "if", "then", "else", "declare", "function", "variable", "namespace", "doc", "collection", "count", "distinct-values", "contains"];

const LANGUAGES = {
  postgres: sqlLike("SQL", "SELECT *\nFROM my_table\nLIMIT 100;"),
  mysql: sqlLike("SQL", "SELECT *\nFROM my_table\nLIMIT 100;"),
  mssql: sqlLike("T-SQL", "SELECT TOP 100 *\nFROM dbo.my_table;", ["TOP", "GO", "NOLOCK"]),
  oracle: sqlLike("SQL / PL-SQL", "SELECT *\nFROM my_table\nFETCH FIRST 100 ROWS ONLY;", ["FETCH", "FIRST", "ROWS", "ONLY", "DUAL", "ROWNUM", "SYSDATE"]),
  sqlite: sqlLike("SQL", "SELECT *\nFROM my_table\nLIMIT 100;", ["PRAGMA"]),
  duckdb: sqlLike("SQL", "SELECT *\nFROM read_parquet('data.parquet')\nLIMIT 100;", ["SUMMARIZE", "DESCRIBE", "PIVOT", "QUALIFY"]),
  clickhouse: sqlLike("ClickHouse SQL", "SELECT *\nFROM my_table\nLIMIT 100;", ["FINAL", "PREWHERE", "SAMPLE", "FORMAT", "SETTINGS"]),
  cassandra: sqlLike("CQL", "SELECT *\nFROM my_keyspace.my_table\nLIMIT 100;", ["KEYSPACE", "ALLOW", "FILTERING", "TOKEN", "TTL", "WRITETIME", "USE"]),
  snowflake: sqlLike("Snowflake SQL", "SELECT *\nFROM my_table\nLIMIT 100;", ["QUALIFY", "FLATTEN", "LATERAL", "WAREHOUSE"]),
  bigquery: sqlLike("GoogleSQL", "SELECT *\nFROM `project.dataset.table`\nLIMIT 100;", ["STRUCT", "ARRAY", "UNNEST", "SAFE_CAST", "QUALIFY"]),
  libsql: sqlLike("SQL", "SELECT *\nFROM my_table\nLIMIT 100;", ["PRAGMA"]),
  val_town: sqlLike("SQL", "SELECT *\nFROM my_table\nLIMIT 100;", ["PRAGMA"]),
  cloudflare_d1: sqlLike("SQL", "SELECT *\nFROM my_table\nLIMIT 100;", ["PRAGMA"]),
  druid: sqlLike("Druid SQL", "SELECT *\nFROM my_datasource\nWHERE __time > CURRENT_TIMESTAMP - INTERVAL '1' DAY\nLIMIT 100;", ["__time", "TIME_FLOOR", "INTERVAL"]),
  dynamodb: sqlLike("PartiQL", "TABLES;\nSELECT * FROM \"MyTable\" WHERE pk = 'user#1';", ["TABLES", "DESCRIBE"]),
  qldb: sqlLike("PartiQL", "SELECT *\nFROM my_table\nWHERE id = 'a1';", ["HISTORY", "COMMITTED"]),
  surrealdb: sqlLike("SurrealQL", "SELECT * FROM person LIMIT 100;", ["DEFINE", "RELATE", "LIVE", "FETCH", "CONTENT", "MERGE", "RETURN", "INFO", "FOR"]),
  spacetimedb: { ...sqlLike("SQL", "SELECT * FROM my_table"), separator: WHOLE },
  immudb: { ...sqlLike("SQL / KV", "SELECT * FROM my_table LIMIT 100\n\n-- Or key-value: GET key / SET key value / SCAN prefix / HISTORY key", ["HISTORY", "VERIFIED", "SCAN", "GET"]), separator: WHOLE },
  orientdb: { ...sqlLike("OrientDB SQL / Gremlin", "SELECT FROM V LIMIT 100\n\n-- Or Gremlin: g.V().limit(10)", ["TRAVERSE", "EXPAND", "OUT", "IN", "BOTH", "V", "E"]), separator: WHOLE },
  objectdb: { ...sqlLike("JPQL", "SELECT e FROM Entity e"), separator: WHOLE },

  redis: lang("Redis command", "command", REDIS_WORDS, "# One command per line\nSCAN 0 MATCH user:* COUNT 100\nGET user:1\nHGETALL session:abc", LINE, "#"),
  memcached: lang("Memcached command", "command", ["get", "gets", "set", "add", "replace", "append", "prepend", "delete", "incr", "decr", "touch", "stats", "version", "flush_all"], "stats\nget my_key\n# Storage commands take the value on the next line\nset my_key 0 0 5\nhello", "One command per line; set/add take the value on the next line", "#"),
  rocksdb: lang("RocksDB command", "command", ["GET", "PUT", "DELETE", "SCAN", "KEYS", "CF", "--cf"], "CF\nSCAN user: 100\nGET user:1\n--cf meta GET version", LINE, "#"),
  hbase: lang("HBase shell", "command", ["list", "get", "scan", "put", "delete", "describe", "count", "LIMIT", "FILTER", "STARTROW", "STOPROW", "COLUMNS"], "scan 'my_table', {LIMIT => 10}", WHOLE, "#"),
  s3: lang("S3 command", "command", ["BUCKETS", "LIST", "GET", "HEAD", "DELETE"], "LIST my-bucket logs/", WHOLE, "#"),
  kafka: lang("Kafka console", "command", ["TOPICS", "CONSUME", "GROUPS", "produce", "topic", "key", "value", "partition", "headers", "delete"], "TOPICS\nCONSUME orders 20\n\n{\"produce\": {\"topic\": \"orders\", \"value\": \"hello\"}}", "One command per line; a JSON body runs on its own", "#"),
  rabbitmq: lang("RabbitMQ console", "command", ["QUEUES", "EXCHANGES", "CONSUME", "PEEK", "PUBLISH"], "QUEUES\nPEEK my_queue 10\nPUBLISH amq.direct my_key hello", "One command per line; a JSON body runs on its own", "#"),
  convex: lang("Convex function call", "command", ["QUERY", "MUTATION", "ACTION", "function", "args", "kind"], "QUERY messages:list {\"limit\": 20}\n\n{\"function\": \"messages:send\", \"args\": {\"body\": \"hi\"}, \"kind\": \"mutation\"}", "One call per line; a JSON body runs on its own", "#"),

  mongodb: lang("MongoDB command", "json", MONGO_WORDS, "{\"find\": \"users\", \"filter\": {\"age\": {\"$gte\": 21}}, \"limit\": 50}\n\n{\"aggregate\": \"orders\", \"pipeline\": [{\"$group\": {\"_id\": \"$status\", \"n\": {\"$sum\": 1}}}], \"cursor\": {}}\n\nfind users {\"active\": true}", BLANK, "//"),
  couchdb: lang("Mango query", "json", ["selector", "fields", "sort", "limit", "skip", "use_index", "GET", "ALL", "path", "method", "body", "$eq", "$gt", "$lt", "$in", "$regex", "$and", "$or", "$exists"], "{\"selector\": {\"type\": \"user\"}, \"limit\": 50}\n\nGET my_doc_id\n\nALL 100", BLANK, "//"),
  firestore: lang("Firestore structuredQuery", "json", ["COLLECTIONS", "GET", "from", "collectionId", "where", "fieldFilter", "field", "fieldPath", "op", "value", "orderBy", "limit", "EQUAL", "LESS_THAN", "GREATER_THAN", "ARRAY_CONTAINS", "IN"], "COLLECTIONS\n\n{\"from\": [{\"collectionId\": \"users\"}], \"limit\": 50}\n\nGET users/u1", BLANK, "//"),
  meilisearch: lang("Meilisearch", "json", ["INDEXES", "SEARCH", "GET", "POST", "PUT", "PATCH", "DELETE", "index", "q", "filter", "sort", "limit", "offset", "facets", "documents", "attributesToRetrieve"], "INDEXES\n\nSEARCH movies matrix\n\n{\"index\": \"movies\", \"q\": \"matrix\", \"limit\": 20}", BLANK, "//"),
  typesense: lang("Typesense", "json", ["COLLECTIONS", "SEARCH", "GET", "POST", "PUT", "PATCH", "DELETE", "collection", "q", "query_by", "filter_by", "sort_by", "per_page", "page", "documents"], "COLLECTIONS\n\n{\"collection\": \"books\", \"q\": \"harry\", \"query_by\": \"title\"}", BLANK, "//"),
  qdrant: lang("Qdrant", "json", ["COLLECTIONS", "INFO", "SCROLL", "collection", "vector", "filter", "must", "should", "key", "match", "limit", "with_payload", "points"], "SCROLL my_collection 20", WHOLE, "//"),
  milvus: lang("Milvus", "json", ["COLLECTIONS", "DESCRIBE", "INFO", "QUERY", "collectionName", "filter", "outputFields", "limit", "data", "annsField"], "QUERY my_collection id > 0", WHOLE, "//"),
  pinecone: lang("Pinecone", "json", ["INDEXES", "STATS", "DESCRIBE", "LIST", "index", "namespace", "vector", "topK", "filter", "includeMetadata", "includeValues", "vectors"], "LIST my-index 20", WHOLE, "//"),
  chroma: lang("Chroma", "json", ["COLLECTIONS", "COUNT", "GET", "collection", "where", "where_document", "query_embeddings", "query_texts", "n_results", "include", "ids"], "GET my_collection 20", WHOLE, "//"),
  elasticsearch: lang("Query DSL", "json", ES_WORDS, "GET /_cat/indices\n\nGET /my-index/_search\n{\n  \"query\": { \"match\": { \"title\": \"hello\" } },\n  \"size\": 20\n}", "Each METHOD /path starts a request; its JSON body follows", "//"),

  prometheus: lang("PromQL", "query", ["sum", "rate", "irate", "increase", "avg", "max", "min", "count", "by", "without", "on", "ignoring", "offset", "histogram_quantile", "topk", "bottomk", "label_replace", "absent", "delta", "deriv"], "up\n\nsum by (job) (rate(http_requests_total[5m]))", "Separate queries with a blank line", "#"),
  influxdb: lang("InfluxQL / Flux", "query", ["SELECT", "FROM", "WHERE", "GROUP", "BY", "time", "LIMIT", "SHOW", "MEASUREMENTS", "DATABASES", "FIELD", "KEYS", "TAG", "from", "range", "filter", "aggregateWindow", "mean", "yield", "bucket", "start", "stop"], "SHOW MEASUREMENTS", WHOLE, "--"),
  neo4j: lang("Cypher", "query", CYPHER_WORDS, "MATCH (n)\nRETURN labels(n) AS label, count(*) AS n\nORDER BY n DESC\nLIMIT 25;", SEMI, "//"),
  tigergraph: lang("GSQL", "query", ["INTERPRET", "QUERY", "RUN", "CREATE", "FOR", "GRAPH", "SELECT", "FROM", "WHERE", "ACCUM", "HAVING", "LIMIT", "PRINT", "SumAccum", "SetAccum", "MapAccum"], "INTERPRET QUERY () FOR GRAPH my_graph {\n  start = {Person.*};\n  result = SELECT s FROM start:s LIMIT 10;\n  PRINT result;\n}", WHOLE, "//"),
  weaviate: lang("GraphQL", "query", ["Get", "Aggregate", "Explore", "where", "nearText", "nearVector", "limit", "offset", "_additional", "id", "distance", "certainty", "concepts", "path", "operator", "valueText", "SCHEMA", "CLASSES", "OBJECTS"], "{\n  Get {\n    Article(limit: 10) {\n      title\n      _additional { id }\n    }\n  }\n}", WHOLE, "#"),
  arangodb: lang("AQL", "query", ["FOR", "IN", "FILTER", "RETURN", "SORT", "LIMIT", "LET", "COLLECT", "INTO", "WITH", "COUNT", "AGGREGATE", "INSERT", "UPDATE", "REPLACE", "REMOVE", "UPSERT", "OUTBOUND", "INBOUND", "ANY", "GRAPH", "SHORTEST_PATH", "DISTINCT", "LIKE", "AND", "OR", "NOT", "DOCUMENT", "LENGTH"], "FOR u IN users\n  FILTER u.active == true\n  SORT u.created DESC\n  LIMIT 50\n  RETURN u", WHOLE, "//"),
  sparql: lang("SPARQL", "query", ["PREFIX", "BASE", "SELECT", "CONSTRUCT", "ASK", "DESCRIBE", "WHERE", "FILTER", "OPTIONAL", "UNION", "GRAPH", "ORDER", "BY", "LIMIT", "OFFSET", "DISTINCT", "GROUP", "HAVING", "VALUES", "BIND", "INSERT", "DELETE", "DATA", "rdf:type", "rdfs:label"], "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\nSELECT ?s ?label\nWHERE { ?s rdfs:label ?label }\nLIMIT 25", WHOLE, "#"),
  basex: lang("XQuery", "query", [...XQUERY_WORDS, "db:open", "db:list"], "for $b in db:open('books')//book\nwhere $b/price > 20\nreturn $b/title", WHOLE, "(:"),
  existdb: lang("XQuery", "query", [...XQUERY_WORDS, "xmldb:get-child-collections"], "for $b in collection('/db/books')//book\nreturn $b/title", WHOLE, "(:"),
} satisfies Record<Family, QueryLanguage>;

export function queryLanguageOf(engine: Engine): QueryLanguage {
  return LANGUAGES[ENGINE_FACTS[engine].family];
}
