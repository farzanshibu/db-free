// SOT: sql-dialect-catalog, sql-keyword-list, sql-function-list, formatter-dialect-map, comment-prefix
import type { SqlLanguage } from "sql-formatter";
import type { Engine, Family } from "@/lib/bindings";
import { ENGINE_FACTS } from "@/lib/bindings/EngineFacts.gen";

// WHAT:  What the editor knows about each SQL dialect: the keywords and
//        functions it offers while you type, the language the formatter should
//        use, and how a comment starts.
// WHY:   One generic keyword list suggested `ILIKE` to MySQL users and
//        `TOP` to Postgres users. The dialect decides the vocabulary.
// HOW:   Every adapter family maps to a flavour through a `satisfies` record,
//        so a family added in Rust fails `tsc` here until it is classified.
//        A flavour's list is added on top of the ANSI core, never instead of it.
// WHERE: src/features/editor/completion.ts (suggestions),
//        src/features/editor/formatQuery.ts (Format), src/features/editor/QueryPane.tsx

export type SqlFlavor =
  | "ansi"
  | "postgres"
  | "mysql"
  | "sqlite"
  | "mssql"
  | "oracle"
  | "clickhouse"
  | "duckdb"
  | "bigquery"
  | "snowflake"
  | "cql"
  | "partiql";

interface FamilyDialect {
  /// null: the family does not speak SQL (documents, key-value, graph…).
  flavor: SqlFlavor | null;
  /// sql-formatter's name for the dialect; null leaves the text alone.
  formatter: SqlLanguage | null;
  /// Line comment opener for the family's language.
  comment: "--" | "//" | "#";
}

const sql = (flavor: SqlFlavor, formatter: SqlLanguage | null, comment: FamilyDialect["comment"] = "--"): FamilyDialect => ({ flavor, formatter, comment });
const other = (comment: FamilyDialect["comment"] = "//"): FamilyDialect => ({ flavor: null, formatter: null, comment });

export const FAMILY_DIALECTS = {
  postgres: sql("postgres", "postgresql"),
  mysql: sql("mysql", "mysql"),
  mssql: sql("mssql", "transactsql"),
  oracle: sql("oracle", "plsql"),
  sqlite: sql("sqlite", "sqlite"),
  duckdb: sql("duckdb", "duckdb"),
  rocksdb: other("#"),
  clickhouse: sql("clickhouse", "clickhouse"),
  redis: other("#"),
  memcached: other("#"),
  mongodb: other("//"),
  couchdb: other("//"),
  firestore: other("//"),
  dynamodb: sql("partiql", "sql"),
  cassandra: sql("cql", "sql"),
  hbase: other("#"),
  neo4j: other("//"),
  tigergraph: other("//"),
  influxdb: sql("ansi", null),
  prometheus: other("#"),
  qdrant: other("//"),
  milvus: other("//"),
  weaviate: other("#"),
  pinecone: other("//"),
  chroma: other("//"),
  elasticsearch: other("//"),
  meilisearch: other("//"),
  typesense: other("//"),
  arangodb: other("//"),
  surrealdb: sql("ansi", null),
  orientdb: sql("ansi", "sql"),
  druid: sql("ansi", "sql"),
  snowflake: sql("snowflake", "snowflake"),
  bigquery: sql("bigquery", "bigquery"),
  libsql: sql("sqlite", "sqlite"),
  val_town: sql("sqlite", "sqlite"),
  cloudflare_d1: sql("sqlite", "sqlite"),
  immudb: sql("ansi", "sql"),
  qldb: sql("partiql", "sql"),
  kafka: other("//"),
  rabbitmq: other("//"),
  convex: other("//"),
  objectdb: sql("ansi", null),
  sparql: other("#"),
  basex: other("#"),
  existdb: other("#"),
  spacetimedb: sql("ansi", "sql"),
  s3: other("#"),
} satisfies Record<Family, FamilyDialect>;

export function dialectOf(engine: Engine): FamilyDialect {
  return FAMILY_DIALECTS[ENGINE_FACTS[engine].family];
}

/// The formatter language for an engine, or null when Format should not touch the text.
export function formatterLanguage(engine: Engine): SqlLanguage | null {
  return dialectOf(engine).formatter;
}

/// How a line comment starts in the engine's language, for "keep the AI prompt as a comment".
export function commentPrefix(engine: Engine): string {
  return dialectOf(engine).comment;
}

export interface SqlCatalog {
  keywords: readonly string[];
  functions: readonly string[];
}

const words = (text: string): string[] => text.split(/\s+/).filter((w) => w.length > 0);

// ---- the shared core -------------------------------------------------------

const ANSI_KEYWORDS = words(`
  SELECT FROM WHERE JOIN LEFT RIGHT INNER OUTER FULL CROSS NATURAL ON USING GROUP BY ORDER HAVING LIMIT OFFSET
  DISTINCT AS AND OR NOT NULL IS IN BETWEEN LIKE EXISTS ANY ALL SOME CASE WHEN THEN ELSE END UNION INTERSECT EXCEPT
  INSERT INTO VALUES UPDATE SET DELETE CREATE TABLE VIEW INDEX ALTER DROP ADD COLUMN CONSTRAINT PRIMARY KEY FOREIGN
  REFERENCES UNIQUE CHECK DEFAULT WITH RECURSIVE ASC DESC TRUE FALSE OVER PARTITION ROWS RANGE UNBOUNDED PRECEDING
  FOLLOWING CURRENT ROW FETCH FIRST NEXT ONLY TRUNCATE BEGIN COMMIT ROLLBACK SAVEPOINT GRANT REVOKE EXPLAIN IF CASCADE
  RESTRICT TEMPORARY TEMP SCHEMA DATABASE RENAME TO LATERAL FILTER WINDOW
`);

const ANSI_FUNCTIONS = words(`
  count sum avg min max coalesce nullif cast abs ceil ceiling floor round sqrt power mod length lower upper trim ltrim
  rtrim substring replace concat position current_date current_time current_timestamp extract row_number rank
  dense_rank lag lead first_value last_value ntile greatest least
`);

// ---- dialects --------------------------------------------------------------

const FLAVORS: Record<SqlFlavor, SqlCatalog> = {
  ansi: { keywords: [], functions: [] },
  postgres: {
    keywords: words(`
      ILIKE RETURNING CONFLICT DO NOTHING MATERIALIZED VACUUM ANALYZE REINDEX COPY LISTEN NOTIFY ONLY SIMILAR TO
      SERIAL BIGSERIAL TABLESPACE EXTENSION SEQUENCE TRIGGER FUNCTION PROCEDURE LANGUAGE PLPGSQL DECLARE
      GENERATED ALWAYS IDENTITY INHERITS UNLOGGED OWNER ENABLE DISABLE REPLACE
    `),
    functions: words(`
      now clock_timestamp statement_timestamp date_trunc date_part age to_char to_date to_timestamp to_number
      generate_series unnest array_agg string_agg json_agg jsonb_agg json_build_object jsonb_build_object
      jsonb_set jsonb_extract_path jsonb_array_elements jsonb_each jsonb_object_keys json_object_agg
      row_to_json to_json to_jsonb regexp_replace regexp_matches regexp_split_to_table split_part
      string_to_array array_length array_append array_cat array_position array_remove cardinality
      left right initcap lpad rpad repeat reverse md5 encode decode gen_random_uuid uuid_generate_v4
      random setseed pg_sleep pg_size_pretty pg_total_relation_size pg_relation_size pg_database_size
      pg_typeof pg_get_viewdef pg_get_indexdef pg_get_constraintdef current_database current_schema
      current_user session_user version bool_and bool_or every percentile_cont percentile_disc mode
      width_bucket exists_any to_tsvector to_tsquery plainto_tsquery ts_rank
    `),
  },
  mysql: {
    keywords: words(`
      AUTO_INCREMENT ENGINE CHARSET COLLATE UNSIGNED ZEROFILL SHOW DESCRIBE USE REPLACE IGNORE DUPLICATE KEY
      STRAIGHT_JOIN SQL_CALC_FOUND_ROWS FORCE LOCK UNLOCK TABLES OPTIMIZE ANALYZE FLUSH DELIMITER RLIKE REGEXP
      XOR DIV STORED VIRTUAL GENERATED COMMENT PARTITION SUBPARTITION TRIGGER PROCEDURE FUNCTION EVENT
    `),
    functions: words(`
      now curdate curtime sysdate date_format str_to_date date_add date_sub datediff timestampdiff adddate subdate
      from_unixtime unix_timestamp year month day hour minute second dayofweek dayofmonth dayofyear weekofyear
      ifnull if group_concat concat_ws find_in_set field elt substring_index locate instr char_length
      lpad rpad reverse left right format json_extract json_unquote json_object json_array json_arrayagg
      json_objectagg json_contains json_set json_length json_keys json_table uuid uuid_short last_insert_id
      found_rows row_count database schema user version connection_id rand md5 sha1 sha2 aes_encrypt
      aes_decrypt inet_aton inet_ntoa bit_count bit_and bit_or bit_xor std stddev variance regexp_like
      regexp_replace regexp_substr
    `),
  },
  sqlite: {
    keywords: words(`
      AUTOINCREMENT PRAGMA VACUUM ATTACH DETACH REINDEX GLOB MATCH REGEXP WITHOUT ROWID STRICT REPLACE
      CONFLICT ABORT FAIL IGNORE INDEXED VIRTUAL TRIGGER RETURNING UPSERT DO NOTHING
    `),
    functions: words(`
      date time datetime julianday strftime unixepoch ifnull iif typeof instr printf glob changes total_changes
      last_insert_rowid random randomblob zeroblob hex unhex quote group_concat json json_array json_extract
      json_insert json_object json_patch json_remove json_replace json_set json_type json_valid json_group_array
      json_group_object json_each json_tree sqlite_version sqlite_source_id likelihood likely unlikely load_extension
      lower upper substr trim ltrim rtrim replace length abs round max min total
    `),
  },
  mssql: {
    keywords: words(`
      TOP PERCENT IDENTITY NVARCHAR VARCHAR NCHAR GO EXEC EXECUTE PROC PROCEDURE DECLARE PRINT RAISERROR THROW TRY
      CATCH OUTPUT INSERTED DELETED MERGE MATCHED WITHOUT NOLOCK READUNCOMMITTED ROWLOCK HOLDLOCK TRIGGER NONCLUSTERED
      CLUSTERED PIVOT UNPIVOT APPLY OPENJSON OPENROWSET IIF TRAN TRANSACTION WAITFOR DELAY USE
    `),
    functions: words(`
      getdate getutcdate sysdatetime sysdatetimeoffset dateadd datediff datename datepart eomonth format
      isnull iif len charindex patindex stuff string_agg string_split concat_ws isnumeric isdate try_cast
      try_convert convert parse try_parse newid newsequentialid scope_identity ident_current checksum
      row_number object_id db_name schema_name suser_name user_name host_name error_message error_number
      json_value json_query json_modify isjson openjson hashbytes app_name db_id
    `),
  },
  oracle: {
    keywords: words(`
      ROWNUM ROWID DUAL MINUS CONNECT START PRIOR LEVEL NOCYCLE SEQUENCE NEXTVAL CURRVAL SYNONYM TRIGGER PACKAGE BODY DESC DESCRIBE SHOW
      PROCEDURE FUNCTION DECLARE EXCEPTION LOOP EXIT RETURN CURSOR OPEN CLOSE MERGE MATCHED VARCHAR2 NUMBER PLS_INTEGER
      NVL TABLESPACE QUOTA PURGE FLASHBACK MODEL PIVOT UNPIVOT KEEP DENSE_RANK
    `),
    functions: words(`
      sysdate systimestamp current_date current_timestamp add_months months_between last_day next_day trunc round
      to_char to_date to_number to_timestamp nvl nvl2 decode listagg regexp_like regexp_replace regexp_substr
      regexp_instr substr instr lpad rpad initcap translate sys_guid user uid sys_context userenv dbms_random
      ora_hash json_value json_query json_table xmlagg xmlelement
    `),
  },
  clickhouse: {
    keywords: words(`
      FORMAT SETTINGS FINAL PREWHERE SAMPLE ARRAY ENGINE MERGETREE ORDER PARTITION TTL MATERIALIZED OPTIMIZE ATTACH
      DETACH SYSTEM PROJECTION TOTALS WITH INTO OUTFILE GLOBAL ANY ASOF SEMI ANTI LIMIT BY CODEC ALIAS DICTIONARY
      DISTRIBUTED DATABASE KILL QUERY
    `),
    functions: words(`
      toDate toDateTime toDateTime64 toStartOfDay toStartOfHour toStartOfMonth toStartOfWeek toYYYYMM toYYYYMMDD
      toUInt32 toUInt64 toInt32 toInt64 toFloat64 toString toTypeName now today yesterday formatDateTime parseDateTimeBestEffort
      dateDiff dateAdd dateSub uniq uniqExact uniqCombined quantile quantiles median argMax argMin groupArray
      groupUniqArray groupBitAnd sumIf countIf avgIf anyLast any arrayJoin arrayMap arrayFilter arrayExists
      arraySort arrayReverse arrayConcat has hasAll hasAny length empty notEmpty splitByChar splitByString
      extract match multiMatchAny JSONExtract JSONExtractString JSONExtractInt JSONExtractRaw JSONHas
      cityHash64 sipHash64 rand generateUUIDv4 isNull isNotNull ifNull nullIf multiIf if lowerUTF8 upperUTF8
      position positionCaseInsensitive replaceAll replaceRegexpAll
    `),
  },
  duckdb: {
    keywords: words(`
      PRAGMA INSTALL LOAD EXPORT IMPORT COPY ATTACH DETACH SUMMARIZE DESCRIBE QUALIFY SAMPLE USING PIVOT UNPIVOT
      RETURNING EXCLUDE REPLACE COLUMNS STRUCT MAP LIST UNION ILIKE SIMILAR GLOB MACRO SEQUENCE
    `),
    functions: words(`
      now today current_date list_value list_agg list_aggregate list_transform list_filter list_sort unnest
      range generate_series array_agg string_agg first last any_value arg_min arg_max histogram median mode
      quantile_cont quantile_disc approx_count_distinct regexp_matches regexp_replace regexp_extract
      string_split split_part strftime strptime date_trunc date_part date_diff date_add epoch epoch_ms
      read_csv read_csv_auto read_parquet read_json read_json_auto parquet_scan json_extract json_extract_string
      struct_pack struct_extract map map_keys map_values map_entries to_json from_json typeof try_cast
      random uuid hash md5 sha256 format printf
    `),
  },
  bigquery: {
    keywords: words(`
      QUALIFY UNNEST STRUCT ARRAY OFFSET ORDINAL SAFE_OFFSET SAFE_ORDINAL PARTITION CLUSTER OPTIONS EXCEPT REPLACE
      TABLESAMPLE SYSTEM WINDOW DECLARE ASSERT EXECUTE IMMEDIATE MERGE MATCHED CREATE OR REPLACE FOR SYSTEM_TIME
      OF DEFINE
    `),
    functions: words(`
      current_datetime current_timestamp timestamp_trunc timestamp_add timestamp_sub timestamp_diff date_trunc
      date_add date_sub date_diff datetime_trunc format_date format_timestamp parse_date parse_timestamp
      extract array_agg array_length array_concat array_to_string generate_array generate_date_array
      string_agg safe_cast safe_divide ifnull if countif any_value approx_count_distinct approx_quantiles
      json_extract json_extract_scalar json_value json_query to_json_string parse_json regexp_contains
      regexp_extract regexp_replace split starts_with ends_with farm_fingerprint generate_uuid session_user
    `),
  },
  snowflake: {
    keywords: words(`
      QUALIFY FLATTEN LATERAL SAMPLE TABLESAMPLE PIVOT UNPIVOT COPY STAGE WAREHOUSE USE ROLE CLONE TIME TRAVEL AT
      BEFORE VARIANT OBJECT ARRAY MATCH_RECOGNIZE ILIKE RLIKE SHOW DESCRIBE CALL TRANSIENT STREAM TASK PIPE
      SEQUENCE MERGE MATCHED
    `),
    functions: words(`
      current_timestamp current_date dateadd datediff date_trunc to_date to_timestamp to_varchar to_number
      try_to_number try_to_date iff nvl nvl2 zeroifnull nullifzero listagg array_agg object_construct parse_json
      get_path flatten split_part regexp_replace regexp_substr regexp_like regexp_count strtok_to_array
      array_size array_contains array_slice array_to_string hash md5 sha2 uuid_string random generator
      current_warehouse current_role current_user current_database current_schema
    `),
  },
  cql: {
    keywords: words(`
      KEYSPACE KEYSPACES TABLES USE DESCRIBE DESC SHOW ALLOW FILTERING TTL WRITETIME TOKEN IF NOT EXISTS CLUSTERING ORDER PRIMARY KEY
      STATIC FROZEN COUNTER LIST SET MAP TUPLE TRUNCATE BATCH APPLY USING TIMESTAMP CONTAINS ROLE PERMISSION PERMISSIONS
      MATERIALIZED VIEW REPLICATION STRATEGY DURABLE_WRITES JSON DISTINCT
    `),
    functions: words(`
      now uuid toTimestamp toDate toUnixTimestamp minTimeuuid maxTimeuuid token ttl writetime blobAsText textAsBlob
      count sum avg min max cast json_extract
    `),
  },
  partiql: {
    keywords: words(`
      MISSING VALUE UNPIVOT PIVOT TUPLE BAG LIST SEXP CONTAINS BEGINS_WITH ATTRIBUTE_EXISTS ATTRIBUTE_TYPE SIZE
    `),
    functions: words(`
      attribute_exists attribute_not_exists attribute_type begins_with contains size exists missing
    `),
  },
};

const cache = new Map<SqlFlavor, SqlCatalog>();

// WHAT:  The keyword and function lists for a flavour: the ANSI core plus its own.
export function catalogOfFlavor(flavor: SqlFlavor): SqlCatalog {
  const known = cache.get(flavor);
  if (known !== undefined) return known;
  const own = FLAVORS[flavor];
  const built: SqlCatalog = {
    keywords: [...new Set([...ANSI_KEYWORDS, ...own.keywords.map((k) => k.toUpperCase())])],
    functions: [...new Set([...ANSI_FUNCTIONS, ...own.functions])],
  };
  cache.set(flavor, built);
  return built;
}

/// The catalogue for an engine, or null when its language is not SQL.
export function catalogFor(engine: Engine): SqlCatalog | null {
  const flavor = dialectOf(engine).flavor;
  return flavor === null ? null : catalogOfFlavor(flavor);
}
