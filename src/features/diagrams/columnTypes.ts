// SOT: designer-column-types, type-picker-catalogue, per-engine-types
import type { Engine, Family } from "@/lib/bindings";
import { ENGINE_FACTS } from "@/lib/bindings/EngineFacts.gen";

// WHAT:  The column types the designer's picker suggests for a diagram's engine,
//        and the type a new primary key gets.
// WHY:   `text` is not a type in Oracle and `NUMBER` is not one in PostgreSQL; a
//        picker that offers one list teaches the wrong spelling for half the
//        engines. The list is a suggestion, not a whitelist (the field takes whatever is typed),
//        because engines have more types (and extensions) than a list can hold.
// HOW:   Keyed by adapter family, so every wire-compatible engine (Supabase, Neon,
//        MariaDB…) inherits its family's list. The designer works without a live
//        session, so this is static rather than fetched from the server.
// WHERE: src/features/diagrams/TableInspector.tsx, designerModel.ts

export interface TypeCatalog {
  types: readonly string[];
  /// Type of a new table's `id` column.
  idType: string;
  /// Type of a new column.
  textType: string;
  /// Whether the dialect can mark a column auto-increment (see diagram_ddl.rs).
  autoIncrement: boolean;
}

const POSTGRES: TypeCatalog = {
  types: [
    "smallint", "integer", "bigint", "serial", "bigserial", "numeric(10,2)", "real", "double precision", "boolean", "text", "varchar(255)",
    "char(1)", "uuid", "date", "time", "timestamp", "timestamptz", "interval", "json", "jsonb", "bytea", "inet", "text[]",
  ],
  idType: "integer",
  textType: "text",
  autoIncrement: true,
};

const MYSQL: TypeCatalog = {
  types: [
    "tinyint", "smallint", "mediumint", "int", "bigint", "decimal(10,2)", "float", "double", "boolean", "varchar(255)", "char(1)", "text",
    "mediumtext", "longtext", "json", "date", "time", "datetime", "timestamp", "year", "blob", "enum('a','b')",
  ],
  idType: "int",
  textType: "varchar(255)",
  autoIncrement: true,
};

const MSSQL: TypeCatalog = {
  types: [
    "bit", "tinyint", "smallint", "int", "bigint", "decimal(10,2)", "float", "real", "money", "nvarchar(255)", "varchar(255)", "nchar(1)",
    "nvarchar(max)", "uniqueidentifier", "date", "time", "datetime2", "datetimeoffset", "varbinary(max)",
  ],
  idType: "int",
  textType: "nvarchar(255)",
  autoIncrement: true,
};

const ORACLE: TypeCatalog = {
  types: [
    "NUMBER(10)", "NUMBER(19)", "NUMBER(10,2)", "BINARY_FLOAT", "BINARY_DOUBLE", "VARCHAR2(255)", "NVARCHAR2(255)", "CHAR(1)", "CLOB", "BLOB",
    "DATE", "TIMESTAMP", "TIMESTAMP WITH TIME ZONE", "RAW(16)",
  ],
  idType: "NUMBER(10)",
  textType: "VARCHAR2(255)",
  autoIncrement: true,
};

const SQLITE: TypeCatalog = {
  types: ["INTEGER", "TEXT", "REAL", "NUMERIC", "BLOB", "BOOLEAN", "DATETIME"],
  idType: "INTEGER",
  textType: "TEXT",
  autoIncrement: true,
};

const CLICKHOUSE: TypeCatalog = {
  types: [
    "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64", "String", "FixedString(16)", "Float32", "Float64", "Decimal(10,2)",
    "Bool", "Date", "DateTime", "DateTime64(3)", "UUID", "Array(String)", "LowCardinality(String)",
  ],
  idType: "UInt64",
  textType: "String",
  autoIncrement: false,
};

const DUCKDB: TypeCatalog = {
  types: ["SMALLINT", "INTEGER", "BIGINT", "DOUBLE", "DECIMAL(18,3)", "BOOLEAN", "VARCHAR", "UUID", "DATE", "TIME", "TIMESTAMP", "TIMESTAMPTZ", "BLOB", "JSON"],
  idType: "INTEGER",
  textType: "VARCHAR",
  autoIncrement: false,
};

const SNOWFLAKE: TypeCatalog = {
  types: [
    "NUMBER(38,0)", "NUMBER(10,2)", "FLOAT", "VARCHAR", "BOOLEAN", "DATE", "TIME", "TIMESTAMP_NTZ", "TIMESTAMP_TZ", "VARIANT", "OBJECT", "ARRAY", "BINARY",
  ],
  idType: "NUMBER(38,0)",
  textType: "VARCHAR",
  autoIncrement: true,
};

const BIGQUERY: TypeCatalog = {
  types: ["INT64", "FLOAT64", "NUMERIC", "BIGNUMERIC", "BOOL", "STRING", "BYTES", "DATE", "TIME", "DATETIME", "TIMESTAMP", "JSON", "ARRAY<STRING>"],
  idType: "INT64",
  textType: "STRING",
  autoIncrement: false,
};

const GENERIC: TypeCatalog = {
  types: ["integer", "bigint", "smallint", "numeric", "real", "text", "varchar(255)", "boolean", "date", "timestamp", "uuid", "json"],
  idType: "integer",
  textType: "text",
  autoIncrement: true,
};

function catalogOf(family: Family): TypeCatalog {
  switch (family) {
    case "postgres":
      return POSTGRES;
    case "mysql":
      return MYSQL;
    case "mssql":
      return MSSQL;
    case "oracle":
      return ORACLE;
    case "sqlite":
    case "libsql":
    case "val_town":
    case "cloudflare_d1":
      return SQLITE;
    case "clickhouse":
      return CLICKHOUSE;
    case "duckdb":
      return DUCKDB;
    case "snowflake":
      return SNOWFLAKE;
    case "bigquery":
      return BIGQUERY;
    default:
      return GENERIC;
  }
}

// WHAT:  The catalogue for a diagram's engine; ANSI-ish types when it has no connection.
export function typeCatalog(engine: Engine | null): TypeCatalog {
  return engine === null ? GENERIC : catalogOf(ENGINE_FACTS[engine].family);
}
