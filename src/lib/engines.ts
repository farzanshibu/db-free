// SOT: engine-registry, engine-labels, engine-categories, engine-presets, connection-string-parser, form-field-labels
import type { ConnectionExtras, ConnectionInput, Engine, EngineKind, Family, FormKind, SshTunnel, SslMode } from "./bindings";
import { ENGINE_FACTS } from "./bindings/EngineFacts.gen";
import { defaultAuth, emptyAuthSecrets } from "./cloudAuth";
import { isSecretParam, paramValue, splitUri } from "./connectionUri";
import type { IconName } from "./icons";

// WHAT:  UI metadata per engine, keyed by the Rust `Engine` enum.
//
// `kind`, `form` and `defaultPort` are decided by Rust (`Engine::facts()`), so
// this object is checked against the generated `ENGINE_FACTS`: change a port or
// a category on one side only and `tsc` fails here rather than the app opening
// a connection dialog with the wrong fields.
// WHY:   Adding an engine in Rust makes this object fail `satisfies` until the
//        entry exists — the registry pattern, not a hand-written list.
//        `kind` and `form` mirror `Engine::kind()` / `Engine::form()` in Rust so
//        the picker can group and the form can render without an IPC round-trip.
// WHERE: src-tauri/src/model/connection.rs (Engine), src-tauri/src/integrations/mod.rs
export interface EngineMeta {
  label: string;
  kind: EngineKind;
  form: FormKind;
  defaultPort: number | null;
  defaultDatabase: string;
  defaultUser: string;
  icon: IconName;
  /// Short description on the picker card.
  hint: string;
  /// Label for the editor tab when the engine does not speak SQL.
  commandLanguage: string;
  /// URL schemes that map to this engine when a connection string is pasted.
  schemes: readonly string[];
  /// Default SSL mode for hosted services.
  sslMode?: SslMode;
  /// Overrides for the generic field labels (see `fieldLabels`).
  fields?: Partial<FieldLabels>;
  /// Placeholder shown for the host / URL field.
  hostPlaceholder?: string;
}

// WHAT:  One line of help shown in the connection form for an engine.
// WHY:   Hosted services fail in ways the error message does not explain (a
//        Supabase direct host that only speaks IPv6, a Turso token that is not
//        the password). Saying it before the first attempt saves the round trip.
export interface EngineNotice {
  tone: "info" | "warning";
  text: string;
}

// WHAT:  Labels for the connection form; engines override what a field means
//        (Cloudflare "host" is an account id, DynamoDB "host" is a region…).
export interface FieldLabels {
  host: string;
  port: string;
  database: string;
  username: string;
  password: string;
  filePath: string;
}

const SQL = "SQL";

// WHAT:  Per-form-kind defaults, so each entry only states what is unusual.
// WHY:   Generic in `T` (not `=> EngineMeta`) so the literal `kind` / `form` /
//        `defaultPort` survive into the object and can be checked against the
//        Rust-generated ENGINE_FACTS below.
type Base = Partial<EngineMeta> & Pick<EngineMeta, "label" | "kind" | "hint" | "schemes">;

const server = <T extends Base>(o: T) => ({
  form: "server" as const,
  defaultPort: null,
  defaultDatabase: "",
  defaultUser: "",
  icon: "database" as const,
  commandLanguage: SQL,
  ...o,
});
const http = <T extends Base>(o: T) => ({
  form: "http_token" as const,
  defaultPort: null,
  defaultDatabase: "",
  defaultUser: "",
  icon: "database" as const,
  commandLanguage: "Query",
  fields: { host: "Server URL", password: "API key / token" },
  ...o,
});
const meta = <T extends EngineMeta>(o: T) => o;
const file = <T extends Base>(o: T) => ({
  form: "file" as const,
  defaultPort: null,
  defaultDatabase: "",
  defaultUser: "",
  icon: "file" as const,
  commandLanguage: SQL,
  ...o,
});

export const ENGINES = {
  // ---- Relational / SQL
  postgres: server({ label: "PostgreSQL", kind: "relational", defaultPort: 5432, defaultUser: "postgres", hint: "Relational · SQL", schemes: ["postgres", "postgresql"] }),
  mysql: server({ label: "MySQL", kind: "relational", defaultPort: 3306, defaultUser: "root", hint: "Relational · SQL", schemes: ["mysql"] }),
  mariadb: server({ label: "MariaDB", kind: "relational", defaultPort: 3306, defaultUser: "root", hint: "Relational · SQL", schemes: ["mariadb"] }),
  mssql: server({ label: "SQL Server", kind: "relational", defaultPort: 1433, defaultDatabase: "master", defaultUser: "sa", hint: "Relational · T-SQL", schemes: ["mssql", "sqlserver", "tds"] }),
  oracle: server({ label: "Oracle Database", kind: "relational", defaultPort: 1521, defaultDatabase: "FREEPDB1", defaultUser: "system", hint: "Relational · PL/SQL", schemes: ["oracle", "oracledb"], fields: { database: "Service name / SID" } }),
  supabase: server({ label: "Supabase", kind: "relational", defaultPort: 5432, defaultDatabase: "postgres", defaultUser: "postgres", hint: "Postgres · managed cloud", schemes: ["supabase"], sslMode: "require" }),
  neon: server({ label: "Neon", kind: "relational", defaultPort: 5432, defaultDatabase: "neondb", hint: "Postgres · serverless cloud", schemes: ["neon"], sslMode: "require" }),
  planetscale: server({ label: "PlanetScale", kind: "relational", defaultPort: 3306, hint: "MySQL · serverless cloud", schemes: ["planetscale", "psdb"], sslMode: "verify_full" }),
  planetscale_postgres: server({ label: "PlanetScale Postgres", kind: "relational", defaultPort: 5432, defaultDatabase: "postgres", hint: "Postgres · serverless cloud", schemes: ["planetscale-postgres"], sslMode: "verify_full" }),

  // ---- Document
  mongodb: server({ label: "MongoDB", kind: "document", defaultPort: 27017, defaultDatabase: "test", icon: "braces", hint: "Documents · JSON commands", commandLanguage: "Command", schemes: ["mongodb", "mongodb+srv"] }),
  couchdb: http({ label: "CouchDB", kind: "document", defaultPort: 5984, defaultUser: "admin", icon: "braces", hint: "Documents · Mango over HTTP", commandLanguage: "Mango", schemes: ["couchdb", "couch"], fields: { host: "Server URL", username: "User", password: "Password", database: "Database" } }),
  firestore: meta({ label: "Firestore", kind: "document", form: "gcp", defaultPort: null, defaultDatabase: "", defaultUser: "", icon: "braces", hint: "Documents · Google Cloud", commandLanguage: "Query", schemes: ["firestore"], fields: { database: "Project ID", username: "Database ID", password: "Service-account JSON" } }),
  convex: http({ label: "Convex", kind: "document", defaultPort: null, icon: "braces", hint: "Serverless backend · functions", commandLanguage: "Query", schemes: ["convex"], fields: { host: "Deployment URL", password: "Deploy key", database: "Deployment name" }, hostPlaceholder: "https://your-app.convex.cloud" }),

  // ---- Key-value
  redis: server({ label: "Redis", kind: "key_value", defaultPort: 6379, defaultDatabase: "0", icon: "hash", hint: "Key-value · commands", commandLanguage: "Command", schemes: ["redis", "rediss"] }),
  valkey: server({ label: "Valkey", kind: "key_value", defaultPort: 6379, defaultDatabase: "0", icon: "hash", hint: "Key-value · Redis-compatible", commandLanguage: "Command", schemes: ["valkey", "valkeys"] }),
  dynamodb: meta({ label: "DynamoDB", kind: "key_value", form: "aws", defaultPort: null, defaultDatabase: "", defaultUser: "", icon: "hash", hint: "Key-value · AWS PartiQL", commandLanguage: "PartiQL", schemes: ["dynamodb"], fields: { host: "Region", username: "Access key ID", password: "Secret access key", database: "Endpoint override" }, hostPlaceholder: "us-east-1" }),

  // ---- Wide-column
  cassandra: server({ label: "Cassandra", kind: "wide_column", defaultPort: 9042, icon: "columns", hint: "Wide-column · CQL", commandLanguage: "CQL", schemes: ["cassandra", "cql"], fields: { database: "Keyspace" } }),
  scylladb: server({ label: "ScyllaDB", kind: "wide_column", defaultPort: 9042, icon: "columns", hint: "Wide-column · CQL", commandLanguage: "CQL", schemes: ["scylla", "scylladb"], fields: { database: "Keyspace" } }),
  hbase: http({ label: "HBase", kind: "wide_column", defaultPort: 8080, icon: "columns", hint: "Wide-column · REST gateway", commandLanguage: "Command", schemes: ["hbase"], fields: { host: "REST gateway URL", database: "Namespace" } }),

  // ---- Graph
  neo4j: server({ label: "Neo4j", kind: "graph", defaultPort: 7687, defaultDatabase: "neo4j", defaultUser: "neo4j", icon: "link", hint: "Graph · Cypher", commandLanguage: "Cypher", schemes: ["neo4j", "neo4j+s", "bolt", "bolt+s"] }),
  memgraph: server({ label: "Memgraph", kind: "graph", defaultPort: 7687, defaultDatabase: "memgraph", icon: "link", hint: "Graph · Cypher (Bolt)", commandLanguage: "Cypher", schemes: ["memgraph"] }),
  tigergraph: http({ label: "TigerGraph", kind: "graph", defaultPort: 14240, icon: "link", hint: "Graph · GSQL over REST", commandLanguage: "GSQL", schemes: ["tigergraph"], fields: { database: "Graph name", username: "User", password: "Password / token" } }),

  // ---- Time-series
  timescaledb: server({ label: "TimescaleDB", kind: "time_series", defaultPort: 5432, defaultUser: "postgres", icon: "calendar", hint: "Time-series · Postgres SQL", schemes: ["timescale", "timescaledb"] }),
  influxdb: http({ label: "InfluxDB", kind: "time_series", defaultPort: 8086, icon: "calendar", hint: "Time-series · InfluxQL / Flux", commandLanguage: "InfluxQL", schemes: ["influx", "influxdb"], fields: { database: "Bucket / database", username: "Org", password: "API token" } }),
  victoriametrics: http({ label: "VictoriaMetrics", kind: "time_series", defaultPort: 8428, icon: "calendar", hint: "Time-series · PromQL / MetricsQL", commandLanguage: "PromQL", schemes: ["victoriametrics", "vm"], fields: { username: "User", password: "Password" } }),
  prometheus: http({ label: "Prometheus", kind: "time_series", defaultPort: 9090, icon: "calendar", hint: "Time-series · PromQL", commandLanguage: "PromQL", schemes: ["prometheus", "prom"], fields: { username: "User", password: "Password" } }),
  questdb: server({ label: "QuestDB", kind: "time_series", defaultPort: 8812, defaultDatabase: "qdb", defaultUser: "admin", icon: "calendar", hint: "Time-series · SQL (PG wire)", schemes: ["questdb"] }),

  // ---- Vector
  qdrant: http({ label: "Qdrant", kind: "vector", defaultPort: 6333, icon: "binary", hint: "Vectors · REST", commandLanguage: "Query", schemes: ["qdrant"], fields: { password: "API key" } }),
  milvus: http({ label: "Milvus", kind: "vector", defaultPort: 19530, defaultDatabase: "default", icon: "binary", hint: "Vectors · REST v2", commandLanguage: "Query", schemes: ["milvus", "zilliz"], fields: { database: "Database", username: "User", password: "Password / token" } }),
  weaviate: http({ label: "Weaviate", kind: "graph_vector", defaultPort: 8080, icon: "binary", hint: "Vectors + graph · REST / GraphQL", commandLanguage: "GraphQL", schemes: ["weaviate"], fields: { password: "API key" } }),
  pinecone: http({ label: "Pinecone", kind: "vector", defaultPort: null, icon: "binary", hint: "Vectors · managed cloud", commandLanguage: "Query", schemes: ["pinecone"], fields: { host: "Index host (optional)", password: "API key", database: "Index name" }, hostPlaceholder: "index-abc123.svc.us-east-1.pinecone.io" }),
  chroma: http({ label: "Chroma", kind: "vector", defaultPort: 8000, defaultDatabase: "default_database", icon: "binary", hint: "Vectors · REST", commandLanguage: "Query", schemes: ["chroma"], fields: { database: "Database", username: "Tenant", password: "Auth token" } }),
  pgvector: server({ label: "pgvector", kind: "vector", defaultPort: 5432, defaultUser: "postgres", icon: "binary", hint: "Vectors · Postgres extension", schemes: ["pgvector"] }),

  // ---- Search / full-text
  elasticsearch: http({ label: "Elasticsearch", kind: "search", defaultPort: 9200, defaultUser: "elastic", icon: "search", hint: "Search · Query DSL / ES|QL", commandLanguage: "Query DSL", schemes: ["elasticsearch", "elastic", "es"], fields: { username: "User", password: "Password / API key" } }),
  opensearch: http({ label: "OpenSearch", kind: "search", defaultPort: 9200, defaultUser: "admin", icon: "search", hint: "Search · Query DSL / SQL", commandLanguage: "Query DSL", schemes: ["opensearch"], fields: { username: "User", password: "Password" } }),
  meilisearch: http({ label: "Meilisearch", kind: "search", defaultPort: 7700, icon: "search", hint: "Search · REST", commandLanguage: "Search", schemes: ["meilisearch", "meili"], fields: { password: "Master / API key" } }),
  typesense: http({ label: "Typesense", kind: "search", defaultPort: 8108, icon: "search", hint: "Search · REST", commandLanguage: "Search", schemes: ["typesense"], fields: { password: "API key" } }),

  // ---- Multi-model
  arangodb: http({ label: "ArangoDB", kind: "graph_vector", defaultPort: 8529, defaultDatabase: "_system", defaultUser: "root", icon: "braces", hint: "Multi-model · AQL", commandLanguage: "AQL", schemes: ["arangodb", "arango"], fields: { database: "Database", username: "User", password: "Password" } }),
  surrealdb: http({ label: "SurrealDB", kind: "multi_model", defaultPort: 8000, defaultDatabase: "test", defaultUser: "root", icon: "braces", hint: "Multi-model · SurrealQL", commandLanguage: "SurrealQL", schemes: ["surrealdb", "surreal", "ws", "wss"], fields: { database: "Namespace/Database (ns/db)", username: "User", password: "Password" } }),
  orientdb: http({ label: "OrientDB", kind: "multi_model", defaultPort: 2480, defaultDatabase: "demodb", defaultUser: "root", icon: "braces", hint: "Multi-model · SQL / Gremlin", schemes: ["orientdb", "orient"], fields: { database: "Database", username: "User", password: "Password" } }),

  // ---- Spatial
  postgis: server({ label: "PostGIS", kind: "spatial", defaultPort: 5432, defaultUser: "postgres", icon: "folder", hint: "Spatial · Postgres SQL", schemes: ["postgis"] }),
  spatialite: file({ label: "SpatiaLite", kind: "spatial", icon: "folder", hint: "Spatial · SQLite file", schemes: ["spatialite"] }),

  // ---- In-memory
  memcached: server({ label: "Memcached", kind: "in_memory", defaultPort: 11211, icon: "hash", hint: "In-memory · text protocol", commandLanguage: "Command", schemes: ["memcached", "memcache"] }),
  dragonfly: server({ label: "Dragonfly", kind: "in_memory", defaultPort: 6379, defaultDatabase: "0", icon: "hash", hint: "In-memory · Redis-compatible", commandLanguage: "Command", schemes: ["dragonfly"] }),

  // ---- Columnar / OLAP
  clickhouse: server({ label: "ClickHouse", kind: "analytical", defaultPort: 8123, defaultDatabase: "default", defaultUser: "default", icon: "columns", hint: "Analytical · SQL over HTTP", schemes: ["clickhouse"] }),
  duckdb: file({ label: "DuckDB", kind: "analytical", icon: "columns", hint: "Analytical · embedded SQL", schemes: ["duckdb"] }),
  druid: http({ label: "Apache Druid", kind: "analytical", defaultPort: 8888, icon: "columns", hint: "Analytical · Druid SQL", commandLanguage: SQL, schemes: ["druid"], fields: { host: "Router URL", username: "User", password: "Password" } }),
  snowflake: http({ label: "Snowflake", kind: "analytical", defaultPort: null, icon: "columns", hint: "Analytical · cloud SQL", commandLanguage: SQL, schemes: ["snowflake"], fields: { host: "Account identifier", database: "Database.Schema", username: "User", password: "Password or PAT" }, hostPlaceholder: "xy12345.us-east-1" }),
  bigquery: meta({ label: "BigQuery", kind: "analytical", form: "gcp", defaultPort: null, defaultDatabase: "", defaultUser: "", icon: "columns", hint: "Analytical · GoogleSQL", commandLanguage: SQL, schemes: ["bigquery", "bq"], fields: { database: "Project ID", username: "Dataset (optional)", password: "Service-account JSON" } }),

  // ---- NewSQL / distributed SQL
  cockroachdb: server({ label: "CockroachDB", kind: "new_sql", defaultPort: 26257, defaultDatabase: "defaultdb", defaultUser: "root", hint: "Distributed SQL · PG wire", schemes: ["cockroachdb", "cockroach", "crdb"] }),
  tidb: server({ label: "TiDB", kind: "new_sql", defaultPort: 4000, defaultDatabase: "test", defaultUser: "root", hint: "Distributed SQL · MySQL wire", schemes: ["tidb"] }),
  yugabytedb: server({ label: "YugabyteDB", kind: "new_sql", defaultPort: 5433, defaultDatabase: "yugabyte", defaultUser: "yugabyte", hint: "Distributed SQL · PG wire", schemes: ["yugabyte", "yugabytedb"] }),
  spacetimedb: http({ label: "SpacetimeDB", kind: "new_sql", hint: "Module database · HTTP SQL", commandLanguage: SQL, schemes: ["spacetimedb", "spacetime"], fields: { host: "Host URL", database: "Module name or identity", password: "Identity token (optional)" }, hostPlaceholder: "http://localhost:3000" }),

  // ---- Embedded
  sqlite: file({ label: "SQLite", kind: "embedded", hint: "Embedded file · SQL", schemes: ["sqlite", "file"] }),
  rocksdb: file({ label: "RocksDB", kind: "embedded", icon: "hash", hint: "Embedded key-value · directory", commandLanguage: "Command", schemes: ["rocksdb"], fields: { filePath: "Database directory" } }),
  libsql: http({ label: "LibSQL / Turso", kind: "embedded", hint: "Serverless SQLite · HTTP", commandLanguage: SQL, schemes: ["libsql", "turso"], fields: { host: "Database URL", password: "Auth token" }, hostPlaceholder: "libsql://your-db.turso.io" }),
  val_town: http({ label: "Val Town", kind: "embedded", defaultDatabase: "main", hint: "Serverless SQLite · HTTP", commandLanguage: SQL, schemes: ["valtown", "val_town"], fields: { host: "API base (optional)", password: "API token", database: "Database name" }, hostPlaceholder: "https://api.val.town" }),
  s3: meta({ label: "Amazon S3", kind: "object", form: "aws", defaultPort: null, defaultDatabase: "", defaultUser: "", icon: "database", hint: "Object storage · buckets and keys", commandLanguage: "Command", schemes: ["s3"], fields: { host: "Region", username: "Access key ID", password: "Secret access key", database: "Endpoint override" }, hostPlaceholder: "us-east-1" }),
  minio: meta({ label: "MinIO", kind: "object", form: "aws", defaultPort: null, defaultDatabase: "", defaultUser: "", icon: "database", hint: "S3-compatible storage · self-hosted", commandLanguage: "Command", schemes: ["minio"], fields: { host: "Region", username: "Access key", password: "Secret key", database: "Endpoint URL" }, hostPlaceholder: "us-east-1" }),
  cloudflare_r2: meta({ label: "Cloudflare R2", kind: "object", form: "aws", defaultPort: null, defaultDatabase: "", defaultUser: "", icon: "database", hint: "S3-compatible storage · zero egress", commandLanguage: "Command", schemes: ["r2", "cloudflare_r2"], fields: { host: "Region", username: "Access key ID", password: "Secret access key", database: "Endpoint URL" }, hostPlaceholder: "auto" }),
  cloudflare_d1: http({ label: "Cloudflare D1", kind: "embedded", hint: "Serverless SQLite · REST", commandLanguage: SQL, schemes: ["cloudflare", "d1"], fields: { host: "Account ID", database: "Database ID", password: "API token" }, hostPlaceholder: "Account ID (32 hex characters)" }),

  // ---- Ledger
  immudb: http({ label: "immudb", kind: "ledger", defaultPort: 8080, defaultDatabase: "defaultdb", defaultUser: "immudb", hint: "Immutable ledger · SQL / KV", commandLanguage: SQL, schemes: ["immudb"], fields: { host: "Web API URL (port 8080)", database: "Database", username: "User", password: "Password" } }),
  qldb: meta({ label: "Amazon QLDB", kind: "ledger", form: "aws", defaultPort: null, defaultDatabase: "", defaultUser: "", icon: "database", hint: "Immutable ledger · PartiQL", commandLanguage: "PartiQL", schemes: ["qldb"], fields: { host: "Region", username: "Access key ID", password: "Secret access key", database: "Ledger name" }, hostPlaceholder: "us-east-1" }),

  // ---- Event / streaming
  kafka: server({ label: "Apache Kafka", kind: "streaming", defaultPort: 9092, icon: "rows", hint: "Event streaming · topics", commandLanguage: "Consume", schemes: ["kafka"], fields: { host: "Bootstrap servers", username: "SASL user", password: "SASL password", database: "Topic filter (optional)" } }),
  redpanda: server({ label: "Redpanda", kind: "streaming", defaultPort: 9092, icon: "rows", hint: "Event streaming · Kafka API", commandLanguage: "Consume", schemes: ["redpanda"], fields: { host: "Bootstrap servers", username: "SASL user", password: "SASL password", database: "Topic filter (optional)" } }),
  rabbitmq: server({ label: "RabbitMQ", kind: "streaming", defaultPort: 15672, icon: "flow", hint: "Message broker · queues", commandLanguage: "Consume", schemes: ["rabbitmq", "amqp", "amqps"], fields: { host: "Management host", username: "User", password: "Password", database: "Vhost" } }),

  // ---- Object
  objectdb: http({ label: "ObjectDB", kind: "object", defaultPort: 6136, hint: "Object database · JPQL", commandLanguage: "JPQL", schemes: ["objectdb"], fields: { database: "Database file", username: "User", password: "Password" } }),

  // ---- Hierarchical / network (through their SQL gateways)
  ibm_ims: server({ label: "IBM IMS", kind: "hierarchical", defaultPort: 5432, hint: "Hierarchical · via SQL gateway", schemes: ["ims"], fields: { host: "Gateway host", database: "PSB / database" } }),
  raima_rdm: server({ label: "Raima RDM", kind: "network", defaultPort: 5432, hint: "Network model · via SQL gateway", schemes: ["raima", "rdm"], fields: { host: "Gateway host" } }),

  // ---- XML
  basex: http({ label: "BaseX", kind: "xml", defaultPort: 8984, defaultUser: "admin", icon: "text", hint: "XML · XQuery over REST", commandLanguage: "XQuery", schemes: ["basex"], fields: { database: "Database", username: "User", password: "Password" } }),
  existdb: http({ label: "eXist-db", kind: "xml", defaultPort: 8080, defaultUser: "admin", icon: "text", hint: "XML · XQuery over REST", commandLanguage: "XQuery", schemes: ["existdb", "exist"], fields: { database: "Collection path", username: "User", password: "Password" } }),

  // ---- RDF / triple stores
  apache_jena: http({ label: "Apache Jena", kind: "rdf", defaultPort: 3030, icon: "link", hint: "RDF · SPARQL (Fuseki)", commandLanguage: "SPARQL", schemes: ["jena", "fuseki"], fields: { database: "Dataset", username: "User", password: "Password" } }),
  graphdb: http({ label: "GraphDB", kind: "rdf", defaultPort: 7200, icon: "link", hint: "RDF · SPARQL", commandLanguage: "SPARQL", schemes: ["graphdb"], fields: { database: "Repository", username: "User", password: "Password" } }),
  stardog: http({ label: "Stardog", kind: "rdf", defaultPort: 5820, defaultUser: "admin", icon: "link", hint: "RDF · SPARQL", commandLanguage: "SPARQL", schemes: ["stardog"], fields: { database: "Database", username: "User", password: "Password" } }),
  blazegraph: http({ label: "Blazegraph", kind: "rdf", defaultPort: 9999, defaultDatabase: "kb", icon: "link", hint: "RDF · SPARQL", commandLanguage: "SPARQL", schemes: ["blazegraph"], fields: { database: "Namespace" } }),
  virtuoso: http({ label: "Virtuoso", kind: "rdf", defaultPort: 8890, icon: "link", hint: "RDF · SPARQL", commandLanguage: "SPARQL", schemes: ["virtuoso"], fields: { database: "Default graph (optional)", username: "User", password: "Password" } }),
} satisfies Record<Engine, EngineMeta>;

// WHAT:  Picker order = the category table order, then the engine order above.
export const CATEGORIES: readonly { kind: EngineKind; label: string; blurb: string }[] = [
  { kind: "relational", label: "Relational / SQL", blurb: "Structured tables, transactions" },
  { kind: "document", label: "Document", blurb: "JSON / document data" },
  { kind: "key_value", label: "Key-Value", blurb: "Fast key → value lookups" },
  { kind: "wide_column", label: "Wide-Column", blurb: "Massive distributed datasets" },
  { kind: "graph", label: "Graph", blurb: "Relationships / networks" },
  { kind: "time_series", label: "Time-Series", blurb: "Metrics / events over time" },
  { kind: "vector", label: "Vector", blurb: "Embeddings, similarity search, AI" },
  { kind: "search", label: "Search / Full-Text", blurb: "Text search, logs" },
  { kind: "multi_model", label: "Multi-Model", blurb: "Multiple data models in one DB" },
  { kind: "spatial", label: "Spatial / Geospatial", blurb: "Location / GIS data" },
  { kind: "in_memory", label: "In-Memory", blurb: "Extremely low latency" },
  { kind: "analytical", label: "Columnar / OLAP", blurb: "Analytics and aggregations" },
  { kind: "new_sql", label: "NewSQL / Distributed SQL", blurb: "SQL + horizontal scaling" },
  { kind: "embedded", label: "Embedded", blurb: "Local application storage" },
  { kind: "ledger", label: "Ledger", blurb: "Immutable / auditable records" },
  { kind: "streaming", label: "Event / Streaming", blurb: "Event persistence & streaming" },
  { kind: "object", label: "Object Database", blurb: "Object-oriented persistence" },
  { kind: "hierarchical", label: "Hierarchical", blurb: "Tree-like data" },
  { kind: "network", label: "Network Database", blurb: "Complex navigational relationships" },
  { kind: "xml", label: "XML Database", blurb: "XML documents" },
  { kind: "graph_vector", label: "Graph + Vector", blurb: "Knowledge graphs + AI search" },
  { kind: "rdf", label: "RDF / Triple Store", blurb: "Semantic web / knowledge graphs" },
];

// WHAT:  Object.keys with the key union kept (the object is `satisfies Record<Engine, …>`).
// WHAT:  The widened view every consumer reads.
// WHY:   `ENGINES` deliberately keeps literal `kind` / `form` / `defaultPort`
//        types so it can be checked against the Rust-generated ENGINE_FACTS.
//        Switches and lookups want the full unions, not one engine's literal.
const REGISTRY: Record<Engine, EngineMeta> = ENGINES;

// WHAT:  The Rust core owns `kind`, `form` and `defaultPort`; the entries above
//        restate them for the picker and the connection form.
// WHY:   A disagreement is silent — a dialog with the wrong fields, or a wrong
//        prefilled port, noticed only when a user tries to connect.
// HOW:   `pnpm bindings` generates EngineFacts.gen.ts from `Engine::facts()`,
//        and the `engine-facts` rule in scripts/guardrail.py (part of
//        `pnpm check`) fails the build if any engine drifts. The helper spreads
//        widen literal types, so this is checked there rather than by `tsc`.

function engineKeys(): Engine[] {
  return Object.keys(REGISTRY).filter((k): k is Engine => k in REGISTRY);
}

export const ENGINE_ORDER: Engine[] = engineKeys();

export function engineMeta(engine: Engine): EngineMeta {
  return REGISTRY[engine];
}

export function enginesOfKind(kind: EngineKind): Engine[] {
  return ENGINE_ORDER.filter((e) => REGISTRY[e].kind === kind);
}

// WHAT:  Engines whose "tables" are keys with a per-key inspector (KeyTab) rather than a grid.
export function isKeyValueEngine(engine: Engine): boolean {
  return engine === "redis" || engine === "valkey" || engine === "dragonfly" || engine === "memcached" || engine === "rocksdb";
}

// WHAT:  S3-compatible object storage (buckets + keys): rows support byte-exact download.
export function isObjectStorageEngine(engine: Engine): boolean {
  return engine === "s3" || engine === "minio" || engine === "cloudflare_r2";
}

// WHAT:  What the sidebar calls the things in the catalogue.
export function collectionNoun(engine: Engine): string {
  if (isKeyValueEngine(engine)) return "Keys";
  switch (REGISTRY[engine].kind) {
    case "document":
    case "multi_model":
    case "vector":
    case "graph_vector":
      return "Collections";
    case "search":
      return "Indexes";
    case "graph":
      return "Labels";
    case "time_series":
      return engine === "timescaledb" || engine === "questdb" ? "Tables" : "Measurements";
    case "streaming":
      return engine === "rabbitmq" ? "Queues" : "Topics";
    case "rdf":
      return "Graphs";
    case "xml":
      return "Collections";
    default:
      return "Tables";
  }
}

// WHAT:  ER diagrams need foreign keys, which only relational-style engines expose.
export function supportsErd(engine: Engine): boolean {
  return REGISTRY[engine].commandLanguage === "SQL" || REGISTRY[engine].commandLanguage === "CQL";
}

export function categoryLabel(kind: EngineKind): string {
  return CATEGORIES.find((c) => c.kind === kind)?.label ?? kind;
}

const DEFAULT_FIELD_LABELS: FieldLabels = {
  host: "Host",
  port: "Port",
  database: "Database",
  username: "User",
  password: "Password",
  filePath: "Database file",
};

export function fieldLabels(engine: Engine): FieldLabels {
  return { ...DEFAULT_FIELD_LABELS, ...(REGISTRY[engine].fields ?? {}) };
}

// WHAT:  One line of help for the connection form.
// WHY:   Hosted services fail in ways the error message does not explain (a
//        Supabase direct host that only speaks IPv6, a Turso token that is not a
//        password). Saying it before the first attempt saves the round trip.
export interface EngineNotice {
  tone: "info" | "warning";
  text: string;
}

// Kept apart from `ENGINES` so each of those entries stays one line, which is
// the shape scripts/guardrail.py reads when it checks them against Rust.
const ENGINE_NOTICES: Partial<Record<Engine, EngineNotice>> = {
  supabase: {
    tone: "warning",
    text: "Direct connections (db.<project>.supabase.co) are IPv6-only. On an IPv4 network use the Session pooler host from the dashboard's Connect panel (aws-0-<region>.pooler.supabase.com) with the user postgres.<project-ref>.",
  },
  neon: {
    tone: "info",
    text: "Compute endpoints suspend when idle, so the first connection can take a few seconds. Use the pooled host (…-pooler.….neon.tech) when many clients connect at once.",
  },
  planetscale: {
    tone: "info",
    text: "Use the host, user and password from your branch's Connect dialog (…connect.psdb.cloud). Passwords belong to a branch, and TLS is required and verified.",
  },
  planetscale_postgres: {
    tone: "info",
    text: "Use the host, user and password from your database's Connect dialog (…psdb.cloud). TLS is required and verified.",
  },
  clickhouse: {
    tone: "info",
    text: "DB Free uses ClickHouse's HTTP interface: port 8123, or 8443 with TLS on (ClickHouse Cloud). The native port 9000 is not spoken.",
  },
  libsql: {
    tone: "info",
    text: "The password field takes an auth token, not a password: create one with `turso db tokens create <database>`.",
  },
  cloudflare_d1: {
    tone: "info",
    text: "Host is your Cloudflare account ID, Database is the D1 database ID (a UUID, not its name), and the token needs D1 edit permission.",
  },
  mongodb: {
    tone: "info",
    text: "Atlas clusters use a mongodb+srv:// address: switch on \"Use SRV record\" and enter the cluster name without a port. Users are usually created in the admin database.",
  },
};

export function engineNotice(engine: Engine): EngineNotice | null {
  return ENGINE_NOTICES[engine] ?? null;
}

// WHAT:  Hosted services that are one of the engines above with fixed settings.
export interface EnginePreset {
  id: string;
  label: string;
  engine: Engine;
  sslMode: SslMode;
  hint: string;
  host?: string;
}

export const PRESETS: readonly EnginePreset[] = [];

// WHAT:  Engines listed in the picker that this build does not ship an adapter for yet.
export const COMING_SOON: readonly { label: string; hint: string }[] = [];

// WHAT:  SSH tunnel settings of a new connection: off, port 22, password auth.
// WHERE: src-tauri/src/model/connection.rs (`impl Default for SshTunnel`)
export function blankSsh(): SshTunnel {
  return { enabled: false, host: null, port: 22, user: null, auth: "password", keyPath: null, hostKey: null };
}

// WHAT:  Per-connection extras of a new connection: no URI parameters, no TLS files, no tags or folders.
// WHERE: src-tauri/src/model/connection_org.rs (`ConnectionExtras`)
export function blankExtras(): ConnectionExtras {
  return { uriParams: null, srv: false, tls: { caPath: null, certPath: null, keyPath: null }, tagIds: [], folderIds: [], transient: false, tunnelOriginHost: null };
}

export function blankInput(engine: Engine, preset?: EnginePreset): ConnectionInput {
  const meta = engineMeta(engine);
  const isFile = meta.form === "file";
  return {
    name: preset ? `${preset.label} database` : "",
    engine,
    environment: "none",
    readOnly: false,
    host: isFile ? null : (preset?.host ?? (meta.form === "server" ? "localhost" : meta.form === "http_token" && meta.defaultPort !== null ? "localhost" : meta.form === "aws" ? "us-east-1" : "")),
    port: meta.defaultPort,
    database: isFile ? null : meta.defaultDatabase,
    username: isFile ? null : meta.defaultUser,
    password: null,
    filePath: isFile ? "" : null,
    sslMode: preset?.sslMode ?? meta.sslMode ?? "prefer",
    ssh: blankSsh(),
    sshSecret: null,
    folder: null,
    color: null,
    favorite: false,
    // MongoDB users live in `admin` unless the connection says otherwise; stating it
    // keeps a form-built connection signing in the way it always has.
    extra: { ...blankExtras(), uriParams: engine === "mongodb" ? "authSource=admin" : null },
    auth: defaultAuth("password"),
    authSecrets: emptyAuthSecrets(),
  };
}

// ============================================================================
// Connection strings
// ============================================================================

export function engineFamily(engine: Engine): Family {
  return ENGINE_FACTS[engine].family;
}

// WHAT:  Whether two engines are spoken to by the same adapter (a `postgres://`
//        string is fine in a TimescaleDB or CockroachDB form).
export function sharesAdapter(a: Engine, b: Engine): boolean {
  return engineFamily(a) === engineFamily(b);
}

/// Hosts that only ever serve TLS. `verify_full` where the provider's certificates chain to a public CA.
const CLOUD_HOSTS: readonly { suffix: string; ssl: SslMode }[] = [
  { suffix: ".supabase.co", ssl: "require" },
  { suffix: ".supabase.com", ssl: "require" },
  { suffix: ".neon.tech", ssl: "require" },
  { suffix: ".psdb.cloud", ssl: "verify_full" },
  { suffix: ".turso.io", ssl: "require" },
  { suffix: ".rds.amazonaws.com", ssl: "require" },
  { suffix: ".redshift.amazonaws.com", ssl: "require" },
  { suffix: ".database.azure.com", ssl: "require" },
  { suffix: ".database.windows.net", ssl: "require" },
  { suffix: ".postgres.database.azure.com", ssl: "require" },
  { suffix: ".mongodb.net", ssl: "require" },
  { suffix: ".cockroachlabs.cloud", ssl: "require" },
  { suffix: ".tsdb.cloud", ssl: "require" },
];

function hostName(host: string): string {
  // `user@` never reaches here; strip a port and a trailing dot, lower-case.
  const noPort = host.startsWith("[") ? host : (host.split(":")[0] ?? host);
  return noPort.toLowerCase().replace(/\.$/, "");
}

// WHAT:  The TLS posture a cloud host calls for (Supabase, Neon, RDS, Azure, Atlas…), or null.
// WHY:   These hosts reject plaintext; defaulting to `prefer` would work by luck
//        of the fallback and `disable` would fail with a confusing reset.
export function cloudHostSslMode(host: string | null): SslMode | null {
  if (host === null) return null;
  const name = hostName(host);
  return CLOUD_HOSTS.find((c) => name.endsWith(c.suffix))?.ssl ?? null;
}

// WHAT:  The engine a pasted connection string really is, going by its host.
// WHY:   `postgresql://…@db.abc.supabase.co` is Supabase, not generic Postgres:
//        picking the preset keeps the notice, TLS default and icon right.
//        Only the generic engines are refined; an explicit scheme such as
//        `timescale://` is never overridden.
export function engineForHost(engine: Engine, host: string | null): Engine {
  if (host === null) return engine;
  const name = hostName(host);
  if (engine === "postgres") {
    if (name.endsWith(".supabase.co") || name.endsWith(".supabase.com")) return "supabase";
    if (name.endsWith(".neon.tech")) return "neon";
    if (name.endsWith(".psdb.cloud")) return "planetscale_postgres";
  }
  if (engine === "mysql" && name.endsWith(".psdb.cloud")) return "planetscale";
  return engine;
}

export type TlsFileSupport = "none" | "ca" | "mutual" | "bundle";

// WHAT:  Which certificate files the engine's adapter reads.
// WHY:   A field that is saved and then ignored is worse than no field: the form
//        offers exactly what `connect` applies. Postgres and MySQL take a CA plus a
//        client certificate and key, MongoDB a CA plus one PEM holding both, SQL
//        Server a CA only.
export function tlsFileSupport(engine: Engine): TlsFileSupport {
  if (REGISTRY[engine].form !== "server") return "none";
  switch (engineFamily(engine)) {
    case "postgres":
    case "mysql":
      return "mutual";
    case "mongodb":
      return "bundle";
    case "mssql":
      return "ca";
    default:
      return "none";
  }
}

// WHAT:  Whether the engine's adapter reads `extra.uriParams`.
// WHY:   Same rule as the TLS files: a field that is saved and then ignored is
//        worse than no field. MongoDB takes every parameter (`authSource`,
//        `replicaSet`, …) and PostgreSQL's family reads `application_name` and
//        `options`; the other adapters take their settings from the fields.
export function usesUriParams(engine: Engine): boolean {
  const family = engineFamily(engine);
  return REGISTRY[engine].form === "server" && (family === "mongodb" || family === "postgres");
}

/// MongoDB's `mongodb+srv://` scheme: the host is a DNS name that resolves to the members.
export function supportsSrv(engine: Engine): boolean {
  return engine === "mongodb";
}

function sslFromValue(value: string | null): SslMode | null {
  switch ((value ?? "").toLowerCase()) {
    case "require":
    case "required":
    case "true":
    case "1":
      return "require";
    case "verify-ca":
    case "verify_ca":
      return "verify_ca";
    case "verify-full":
    case "verify_full":
    case "verify_identity":
      return "verify_full";
    case "disable":
    case "disabled":
    case "false":
    case "0":
      return "disable";
    case "prefer":
    case "preferred":
    case "allow":
      return "prefer";
    default:
      return null;
  }
}

/// Parameters the form turns into settings of its own, so they are not kept as raw options.
const SSL_PARAMS: readonly string[] = ["sslmode", "ssl-mode", "ssl", "tls", "secure"];
/// Parameters naming certificate files, mapped onto the TLS file fields.
const TLS_FILE_PARAMS: readonly { key: string; field: "caPath" | "certPath" | "keyPath" }[] = [
  { key: "sslrootcert", field: "caPath" },
  { key: "sslcert", field: "certPath" },
  { key: "sslkey", field: "keyPath" },
  { key: "ssl-ca", field: "caPath" },
  { key: "ssl-cert", field: "certPath" },
  { key: "ssl-key", field: "keyPath" },
  { key: "tlscafile", field: "caPath" },
  { key: "tlscertificatekeyfile", field: "certPath" },
];

export interface ParsedConnection {
  engine: Engine;
  input: ConnectionInput;
  /// Things the user should know were changed on the way in (a dropped password parameter…).
  notes: string[];
}

// WHAT:  Parses `scheme://user:pass@host:port/db?sslmode=…` into a ConnectionInput.
// WHY:   DB Manager's "paste a connection string to auto-detect" affordance, kept
//        whole: query parameters the form has no field for (`authSource`,
//        `replicaSet`, `application_name`, `pgbouncer`…) survive as `extra.uriParams`
//        and reach the driver, a cloud host picks the matching preset and TLS
//        default, and a password parameter is refused rather than stored in the clear.
export function parseConnectionStringDetailed(raw: string): ParsedConnection | null {
  const text = raw.trim();
  if (text.length === 0) return null;
  const parts = splitUri(text);
  if (!parts) return null;
  const schemeEngine = ENGINE_ORDER.find((e) => REGISTRY[e].schemes.includes(parts.scheme));
  if (!schemeEngine) return null;
  const engine = engineForHost(schemeEngine, parts.host);
  const meta = REGISTRY[engine];
  const base = blankInput(engine);
  const notes: string[] = [];

  if (meta.form === "file") {
    const withoutScheme = text.slice(text.indexOf("://") + 3);
    const path = decodeURIComponent(withoutScheme.split("?")[0] ?? withoutScheme);
    return { engine, input: { ...base, name: path.split("/").pop() ?? meta.label, filePath: path }, notes };
  }

  const tlsFiles = tlsFileSupport(engine) !== "none";
  let sslMode: SslMode | null = null;
  const tls = { ...base.extra.tls };
  const kept: string[] = [];
  let droppedSecret = false;
  for (const pair of parts.params) {
    const eq = pair.indexOf("=");
    const key = (eq >= 0 ? pair.slice(0, eq) : pair).toLowerCase();
    if (SSL_PARAMS.includes(key)) {
      sslMode = sslFromValue(paramValue([pair], key)) ?? sslMode;
      continue;
    }
    const file = tlsFiles ? TLS_FILE_PARAMS.find((p) => p.key === key) : undefined;
    if (file) {
      tls[file.field] = paramValue([pair], key);
      continue;
    }
    if (isSecretParam(key)) {
      droppedSecret = true;
      continue;
    }
    kept.push(pair);
  }
  if (droppedSecret) notes.push("A password parameter in the URI was not kept: options are stored unencrypted. Use the password field.");

  const secureScheme = ["rediss", "valkeys", "neo4j+s", "bolt+s", "wss", "amqps"].includes(parts.scheme);
  const srv = parts.scheme === "mongodb+srv";
  const resolvedSsl: SslMode =
    sslMode ?? (secureScheme || srv ? "require" : (cloudHostSslMode(parts.host) ?? meta.sslMode ?? "prefer"));

  const database = parts.database ?? "";
  const host = parts.host ?? "";
  return {
    engine,
    input: {
      ...base,
      name: database.length > 0 ? database : host.length > 0 ? host : meta.label,
      host: host.length > 0 ? host : base.host,
      port: srv ? null : (parts.port ?? base.port),
      database: database.length > 0 ? database : base.database,
      username: parts.username ?? base.username,
      password: parts.password,
      sslMode: resolvedSsl,
      extra: { ...base.extra, uriParams: kept.length > 0 ? kept.join("&") : null, srv, tls },
    },
    notes,
  };
}

export function parseConnectionString(raw: string): ConnectionInput | null {
  return parseConnectionStringDetailed(raw)?.input ?? null;
}

// WHAT:  The message when a pasted string belongs to a different engine than the form.
// HOW:   Engines on the same adapter are interchangeable here, so a Postgres URL
//        pasted into a CockroachDB form is accepted.
export function connectionStringMismatch(current: Engine, parsed: Engine): string | null {
  if (current === parsed || sharesAdapter(current, parsed)) return null;
  return `This looks like a ${REGISTRY[parsed].label} connection string, but this form is for ${REGISTRY[current].label}.`;
}
