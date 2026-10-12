// SOT: ddl-type-catalog, ddl-type-picker, ddl-parse-type, ddl-render-type

use super::dialect::Dialect;
use crate::error::{AppError, AppResult};
use crate::model::{ColumnSpec, DdlType, TypeParams};

// WHAT:  The column types each dialect's pickers offer, how a type a catalogue
//        reports ("character varying(255)", "int(10) unsigned", "Nullable(String)")
//        becomes the picker's base type plus parameters, and how a spec is spelled
//        back as SQL.
// WHY:   An unchanged column must render byte-for-byte the same on both sides of an
//        edit, or every Save would ALTER columns nobody touched. Parsing
//        normalises the catalogue's spelling onto the picker's names once, here.
// HOW:   Rendering is driven by which fields are set (length, precision/scale,
//        enum values), never by the catalogue, so a type the picker does not know
//        round-trips untouched. A type containing `(` or `[` is a custom type
//        and is written out verbatim.
// WHERE: src-tauri/src/integrations/ddl/{create,alter,describe}.rs

const N: TypeParams = TypeParams::None;
const L: TypeParams = TypeParams::Length;
const P: TypeParams = TypeParams::Precision;
const PS: TypeParams = TypeParams::PrecisionScale;
const V: TypeParams = TypeParams::Values;

type Row = (&'static str, TypeParams);

const POSTGRES: &[Row] = &[
    ("bigint", N),
    ("bit", L),
    ("boolean", N),
    ("box", N),
    ("bytea", N),
    ("char", L),
    ("cidr", N),
    ("circle", N),
    ("date", N),
    ("decimal", PS),
    ("double precision", N),
    ("inet", N),
    ("integer", N),
    ("interval", N),
    ("json", N),
    ("jsonb", N),
    ("line", N),
    ("lseg", N),
    ("macaddr", N),
    ("money", N),
    ("numeric", PS),
    ("path", N),
    ("point", N),
    ("polygon", N),
    ("real", N),
    ("smallint", N),
    ("text", N),
    ("time", P),
    ("timestamp", P),
    ("timestamptz", P),
    ("timetz", P),
    ("tsvector", N),
    ("uuid", N),
    ("varchar", L),
    ("xml", N),
];

const MYSQL: &[Row] = &[
    ("bigint", N),
    ("binary", L),
    ("blob", N),
    ("boolean", N),
    ("char", L),
    ("date", N),
    ("datetime", P),
    ("decimal", PS),
    ("double", N),
    ("enum", V),
    ("float", N),
    ("int", N),
    ("json", N),
    ("longblob", N),
    ("longtext", N),
    ("mediumblob", N),
    ("mediumint", N),
    ("mediumtext", N),
    ("numeric", PS),
    ("set", V),
    ("smallint", N),
    ("text", N),
    ("time", P),
    ("timestamp", P),
    ("tinyblob", N),
    ("tinyint", N),
    ("tinytext", N),
    ("varbinary", L),
    ("varchar", L),
    ("year", N),
];

const SQLITE: &[Row] = &[
    ("BLOB", N),
    ("BOOLEAN", N),
    ("DATE", N),
    ("DATETIME", N),
    ("DECIMAL", PS),
    ("INTEGER", N),
    ("NUMERIC", N),
    ("REAL", N),
    ("TEXT", N),
    ("VARCHAR", L),
];

const MSSQL: &[Row] = &[
    ("bigint", N),
    ("binary", L),
    ("bit", N),
    ("char", L),
    ("date", N),
    ("datetime", N),
    ("datetime2", P),
    ("datetimeoffset", P),
    ("decimal", PS),
    ("float", P),
    ("geography", N),
    ("geometry", N),
    ("hierarchyid", N),
    ("image", N),
    ("int", N),
    ("json", N),
    ("money", N),
    ("nchar", L),
    ("ntext", N),
    ("numeric", PS),
    ("nvarchar", L),
    ("nvarchar(max)", N),
    ("real", N),
    ("smalldatetime", N),
    ("smallint", N),
    ("smallmoney", N),
    ("sql_variant", N),
    ("text", N),
    ("time", P),
    ("timestamp", N),
    ("tinyint", N),
    ("uniqueidentifier", N),
    ("varbinary", L),
    ("varbinary(max)", N),
    ("varchar", L),
    ("varchar(max)", N),
    ("xml", N),
];

const ORACLE: &[Row] = &[
    ("BINARY_DOUBLE", N),
    ("BINARY_FLOAT", N),
    ("BLOB", N),
    ("CHAR", L),
    ("CLOB", N),
    ("DATE", N),
    ("FLOAT", P),
    ("INTEGER", N),
    ("JSON", N),
    ("LONG", N),
    ("NCHAR", L),
    ("NCLOB", N),
    ("NUMBER", PS),
    ("NVARCHAR2", L),
    ("RAW", L),
    ("ROWID", N),
    ("SMALLINT", N),
    ("TIMESTAMP", P),
    ("TIMESTAMP WITH LOCAL TIME ZONE", P),
    ("TIMESTAMP WITH TIME ZONE", P),
    ("UROWID", L),
    ("VARCHAR2", L),
    ("XMLTYPE", N),
];

const DUCKDB: &[Row] = &[
    ("BIGINT", N),
    ("BLOB", N),
    ("BOOLEAN", N),
    ("DATE", N),
    ("DECIMAL", PS),
    ("DOUBLE", N),
    ("FLOAT", N),
    ("HUGEINT", N),
    ("INTEGER", N),
    ("INTERVAL", N),
    ("JSON", N),
    ("REAL", N),
    ("SMALLINT", N),
    ("TEXT", N),
    ("TIME", N),
    ("TIMESTAMP", N),
    ("TIMESTAMPTZ", N),
    ("TINYINT", N),
    ("UBIGINT", N),
    ("UHUGEINT", N),
    ("UINTEGER", N),
    ("USMALLINT", N),
    ("UTINYINT", N),
    ("UUID", N),
    ("VARCHAR", L),
];

// ClickHouse's structured types (Tuple, Map, Nested, AggregateFunction, Variant…)
// need their element types spelled out; they are entered as a custom type
// (`Map(String, UInt64)`) rather than offered here half-defined.
const CLICKHOUSE: &[Row] = &[
    ("BFloat16", N),
    ("Bool", N),
    ("Date", N),
    ("Date32", N),
    ("DateTime", N),
    ("DateTime64", P),
    ("Decimal", PS),
    ("Decimal128", P),
    ("Decimal256", P),
    ("Decimal32", P),
    ("Decimal64", P),
    ("Enum16", V),
    ("Enum8", V),
    ("FixedString", L),
    ("Float32", N),
    ("Float64", N),
    ("Int128", N),
    ("Int16", N),
    ("Int256", N),
    ("Int32", N),
    ("Int64", N),
    ("Int8", N),
    ("IPv4", N),
    ("IPv6", N),
    ("JSON", N),
    ("MultiPolygon", N),
    ("Point", N),
    ("Polygon", N),
    ("Ring", N),
    ("String", N),
    ("UInt128", N),
    ("UInt16", N),
    ("UInt256", N),
    ("UInt32", N),
    ("UInt64", N),
    ("UInt8", N),
    ("UUID", N),
];

const SNOWFLAKE: &[Row] = &[
    ("ARRAY", N),
    ("BIGINT", N),
    ("BINARY", L),
    ("BOOLEAN", N),
    ("CHAR", L),
    ("DATE", N),
    ("DECIMAL", PS),
    ("DOUBLE", N),
    ("FLOAT", N),
    ("GEOGRAPHY", N),
    ("GEOMETRY", N),
    ("INTEGER", N),
    ("NUMBER", PS),
    ("NUMERIC", PS),
    ("OBJECT", N),
    ("REAL", N),
    ("SMALLINT", N),
    ("STRING", L),
    ("TEXT", L),
    ("TIME", P),
    ("TIMESTAMP_LTZ", P),
    ("TIMESTAMP_NTZ", P),
    ("TIMESTAMP_TZ", P),
    ("TINYINT", N),
    ("VARBINARY", L),
    ("VARCHAR", L),
    ("VARIANT", N),
];

// BigQuery's ARRAY<…> and STRUCT<…> are entered as a custom type.
const BIGQUERY: &[Row] = &[
    ("BIGNUMERIC", PS),
    ("BOOL", N),
    ("BYTES", L),
    ("DATE", N),
    ("DATETIME", N),
    ("FLOAT64", N),
    ("GEOGRAPHY", N),
    ("INT64", N),
    ("INTERVAL", N),
    ("JSON", N),
    ("NUMERIC", PS),
    ("STRING", L),
    ("TIME", N),
    ("TIMESTAMP", N),
];

// CQL collections (list<text>, map<text, int>) are entered as a custom type.
const CASSANDRA: &[Row] = &[
    ("ascii", N),
    ("bigint", N),
    ("blob", N),
    ("boolean", N),
    ("counter", N),
    ("date", N),
    ("decimal", N),
    ("double", N),
    ("duration", N),
    ("float", N),
    ("inet", N),
    ("int", N),
    ("smallint", N),
    ("text", N),
    ("time", N),
    ("timestamp", N),
    ("timeuuid", N),
    ("tinyint", N),
    ("uuid", N),
    ("varchar", N),
    ("varint", N),
];

fn rows(dialect: Dialect) -> &'static [Row] {
    match dialect {
        Dialect::Postgres => POSTGRES,
        Dialect::Mysql => MYSQL,
        Dialect::Mssql => MSSQL,
        Dialect::Oracle => ORACLE,
        Dialect::Sqlite => SQLITE,
        Dialect::Duckdb => DUCKDB,
        Dialect::Clickhouse => CLICKHOUSE,
        Dialect::Snowflake => SNOWFLAKE,
        Dialect::Bigquery => BIGQUERY,
        Dialect::Cassandra => CASSANDRA,
        Dialect::Mongo => &[],
    }
}

/// Integer types an auto-increment column may have, by dialect (lower case).
fn auto_increment_types(dialect: Dialect) -> &'static [&'static str] {
    match dialect {
        Dialect::Postgres => &["smallint", "integer", "bigint"],
        Dialect::Mysql => &["tinyint", "smallint", "mediumint", "int", "bigint"],
        Dialect::Mssql => &["tinyint", "smallint", "int", "bigint"],
        Dialect::Oracle => &["number", "integer", "smallint"],
        Dialect::Sqlite => &["integer"],
        Dialect::Duckdb => &["tinyint", "smallint", "integer", "bigint", "hugeint"],
        Dialect::Snowflake => &["number", "integer", "bigint", "smallint", "tinyint"],
        Dialect::Clickhouse | Dialect::Bigquery | Dialect::Cassandra | Dialect::Mongo => &[],
    }
}

/// MySQL integer types that accept `UNSIGNED`.
const MYSQL_UNSIGNED: &[&str] = &["tinyint", "smallint", "mediumint", "int", "bigint", "decimal", "numeric", "float", "double"];

pub fn auto_increment_allowed(dialect: Dialect, base: &str) -> bool {
    let lower = base.trim().to_ascii_lowercase();
    // `integer` on MySQL, `int4` on PostgreSQL… are the same type under another name.
    let canonical = alias(dialect, &lower).map(str::to_ascii_lowercase);
    let allowed = auto_increment_types(dialect);
    allowed.contains(&lower.as_str()) || canonical.is_some_and(|c| allowed.contains(&c.as_str()))
}

pub fn catalog(dialect: Dialect) -> Vec<DdlType> {
    rows(dialect)
        .iter()
        .map(|(name, params)| DdlType {
            name: (*name).to_string(),
            params: *params,
            auto_increment: auto_increment_allowed(dialect, name),
            unsigned: dialect == Dialect::Mysql && MYSQL_UNSIGNED.contains(name),
        })
        .collect()
}

fn lookup(dialect: Dialect, name: &str) -> Option<&'static Row> {
    rows(dialect).iter().find(|(n, _)| n.eq_ignore_ascii_case(name))
}

/// The picker's base type for the first column of a new table / any new column.
pub fn default_types(dialect: Dialect) -> (&'static str, &'static str) {
    // (new column, key column)
    match dialect {
        Dialect::Postgres => ("varchar", "integer"),
        Dialect::Mysql => ("varchar", "int"),
        Dialect::Mssql => ("nvarchar", "int"),
        Dialect::Oracle => ("VARCHAR2", "NUMBER"),
        Dialect::Sqlite => ("TEXT", "INTEGER"),
        Dialect::Duckdb => ("VARCHAR", "INTEGER"),
        Dialect::Clickhouse => ("String", "UInt64"),
        Dialect::Snowflake => ("VARCHAR", "NUMBER"),
        Dialect::Bigquery => ("STRING", "INT64"),
        Dialect::Cassandra => ("text", "uuid"),
        Dialect::Mongo => ("", ""),
    }
}

// ---- parsing -----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedType {
    pub base: String,
    pub length: Option<u32>,
    pub precision: Option<u32>,
    pub scale: Option<u32>,
    pub unsigned: bool,
    pub enum_values: Vec<String>,
    pub wrapper: Option<String>,
    /// ClickHouse `Nullable(...)`.
    pub nullable: bool,
}

impl ParsedType {
    fn verbatim(text: &str) -> ParsedType {
        ParsedType { base: text.to_string(), ..ParsedType::default() }
    }
}

fn alias(dialect: Dialect, lower: &str) -> Option<&'static str> {
    match (dialect, lower) {
        (Dialect::Postgres, "character varying") => Some("varchar"),
        (Dialect::Postgres, "character" | "bpchar") => Some("char"),
        (Dialect::Postgres, "int" | "int4") => Some("integer"),
        (Dialect::Postgres, "int8") => Some("bigint"),
        (Dialect::Postgres, "int2") => Some("smallint"),
        (Dialect::Postgres, "float8") => Some("double precision"),
        (Dialect::Postgres, "float4") => Some("real"),
        (Dialect::Postgres, "bool") => Some("boolean"),
        (Dialect::Postgres, "timestamp without time zone") => Some("timestamp"),
        (Dialect::Postgres, "timestamp with time zone") => Some("timestamptz"),
        (Dialect::Postgres, "time without time zone") => Some("time"),
        (Dialect::Postgres, "time with time zone") => Some("timetz"),
        (Dialect::Mysql, "integer") => Some("int"),
        (Dialect::Mysql, "dec" | "fixed") => Some("decimal"),
        (Dialect::Mysql, "bool") => Some("boolean"),
        (Dialect::Mssql, "integer") => Some("int"),
        (Dialect::Mssql, "double precision") => Some("float"),
        (Dialect::Duckdb, "int" | "int4" | "signed") => Some("INTEGER"),
        (Dialect::Duckdb, "int8" | "long") => Some("BIGINT"),
        (Dialect::Duckdb, "int2" | "short") => Some("SMALLINT"),
        (Dialect::Duckdb, "bool" | "logical") => Some("BOOLEAN"),
        (Dialect::Duckdb, "string" | "bpchar" | "char") => Some("VARCHAR"),
        (Dialect::Duckdb, "float4") => Some("REAL"),
        (Dialect::Duckdb, "float8") => Some("DOUBLE"),
        (Dialect::Snowflake, "fixed") => Some("NUMBER"),
        _ => None,
    }
}

/// The catalogue's spelling of `name`, or the name as given (upper-cased for the
/// engines whose type names are conventionally upper case).
fn canonical(dialect: Dialect, name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let lower = alias(dialect, &lower).map_or(lower, str::to_ascii_lowercase);
    if let Some((spelled, _)) = lookup(dialect, &lower) {
        return (*spelled).to_string();
    }
    match dialect {
        Dialect::Sqlite | Dialect::Oracle => name.to_ascii_uppercase(),
        _ => name.to_string(),
    }
}

/// Index of the `)` matching the `(` at `open`, skipping quoted strings.
fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0_i32;
    let mut quote: Option<char> = None;
    for (i, ch) in text.char_indices().skip_while(|(i, _)| *i < open) {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' | '`' => quote = Some(ch),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            },
        }
    }
    None
}

/// Peels ClickHouse `Nullable(…)`, `LowCardinality(…)` and `Array(…)` into
/// (inner type, wrapper, nullable). Shapes the form cannot express
/// (`Array(LowCardinality(…))`) come back None and are kept verbatim.
fn peel_clickhouse(text: &str) -> Option<(String, Option<String>, bool)> {
    let mut rest = text.trim().to_string();
    let mut layers: Vec<String> = Vec::new();
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim().to_string();
        if !matches!(name.as_str(), "Nullable" | "LowCardinality" | "Array") {
            break;
        }
        let close = matching_paren(&rest, open)?;
        if !rest[close + 1..].trim().is_empty() {
            break;
        }
        let inner = rest[open + 1..close].trim().to_string();
        layers.push(name);
        rest = inner;
    }
    let shape: Vec<&str> = layers.iter().map(String::as_str).collect();
    match shape.as_slice() {
        [] => Some((rest, None, false)),
        ["Nullable"] => Some((rest, None, true)),
        ["LowCardinality"] => Some((rest, Some("LowCardinality".into()), false)),
        ["Array"] => Some((rest, Some("Array".into()), false)),
        ["LowCardinality", "Nullable"] => Some((rest, Some("LowCardinality".into()), true)),
        ["Array", "Nullable"] => Some((rest, Some("Array".into()), true)),
        _ => None,
    }
}

/// Quoted labels of an `enum('a','b')` argument list: `''` is an escaped quote.
/// A ClickHouse `'a' = 1` numbering is accepted only when it is 1, 2, 3….
fn enum_labels(args: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = args.chars().collect();
    let mut labels = Vec::new();
    let mut numbers: Vec<i64> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == ',') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        if chars[i] != '\'' {
            return None;
        }
        i += 1;
        let mut label = String::new();
        loop {
            let ch = *chars.get(i)?;
            if ch == '\'' {
                if chars.get(i + 1) == Some(&'\'') {
                    label.push('\'');
                    i += 2;
                    continue;
                }
                i += 1;
                break;
            }
            label.push(ch);
            i += 1;
        }
        labels.push(label);
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if chars.get(i) == Some(&'=') {
            i += 1;
            let start = i;
            while i < chars.len() && (chars[i].is_whitespace() || chars[i].is_ascii_digit() || chars[i] == '-') {
                i += 1;
            }
            let digits: String = chars[start..i].iter().collect();
            numbers.push(digits.trim().parse().ok()?);
        }
    }
    if !numbers.is_empty() && (numbers.len() != labels.len() || numbers.iter().enumerate().any(|(n, v)| *v != i64::try_from(n + 1).unwrap_or(0))) {
        return None;
    }
    Some(labels)
}

fn numbers(args: &str, dialect: Dialect) -> Option<Vec<u32>> {
    let mut out = Vec::new();
    for part in args.split(',') {
        let mut part = part.trim();
        if dialect == Dialect::Oracle {
            // VARCHAR2(255 CHAR) / VARCHAR2(255 BYTE)
            part = part.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c.is_whitespace());
        }
        out.push(part.parse::<u32>().ok()?);
    }
    Some(out)
}

pub fn parse_type(dialect: Dialect, raw: &str) -> ParsedType {
    let raw = raw.trim();
    if raw.is_empty() {
        return ParsedType::default();
    }
    let mut parsed = ParsedType::default();
    let mut text = raw.to_string();

    if dialect == Dialect::Clickhouse {
        match peel_clickhouse(&text) {
            Some((inner, wrapper, nullable)) => {
                text = inner;
                parsed.wrapper = wrapper;
                parsed.nullable = nullable;
            }
            None => return ParsedType::verbatim(raw),
        }
    }
    if dialect == Dialect::Mysql {
        loop {
            let lower = text.to_ascii_lowercase();
            if let Some(stripped) = lower.strip_suffix(" unsigned") {
                parsed.unsigned = true;
                text.truncate(stripped.len());
            } else if let Some(stripped) = lower.strip_suffix(" zerofill") {
                text.truncate(stripped.len());
            } else {
                break;
            }
        }
    }
    if text.contains('[') {
        // PostgreSQL arrays (`integer[]`) and BigQuery-style collections are custom types.
        parsed.base = text;
        return parsed;
    }

    let (name, args, tail) = match text.find('(') {
        Some(open) => match matching_paren(&text, open) {
            Some(close) => (text[..open].trim().to_string(), Some(text[open + 1..close].to_string()), text[close + 1..].trim().to_string()),
            None => return ParsedType::verbatim(raw),
        },
        None => (text.trim().to_string(), None, String::new()),
    };

    let mut spelled = name.clone();
    // `timestamp(3) with time zone`: the zone words trail the parameters.
    let tail_lower = tail.to_ascii_lowercase();
    if dialect == Dialect::Postgres {
        if tail_lower == "with time zone" || tail_lower == "without time zone" {
            spelled = format!("{name} {tail_lower}");
        } else if !tail.is_empty() {
            return ParsedType::verbatim(raw);
        }
    } else if dialect == Dialect::Oracle && tail_lower.starts_with("with ") {
        spelled = format!("{name} {tail_lower}");
    } else if !tail.is_empty() {
        return ParsedType::verbatim(raw);
    }
    let base = canonical(dialect, &spelled);
    let class = lookup(dialect, &base).map(|(_, p)| *p);
    parsed.base = base.clone();

    let Some(args) = args else {
        return parsed;
    };

    if class == Some(V) || matches!(base.as_str(), "Enum8" | "Enum16" | "enum" | "set") {
        return match enum_labels(&args) {
            Some(labels) => {
                parsed.enum_values = labels;
                parsed
            }
            None => {
                // Explicitly numbered ClickHouse enums keep their numbers: custom type.
                let mut verbatim = ParsedType::verbatim(&text);
                verbatim.wrapper = parsed.wrapper;
                verbatim.nullable = parsed.nullable;
                verbatim
            }
        };
    }

    let Some(values) = numbers(&args, dialect) else {
        let mut verbatim = ParsedType::verbatim(&text);
        verbatim.wrapper = parsed.wrapper;
        verbatim.nullable = parsed.nullable;
        verbatim.unsigned = parsed.unsigned;
        return verbatim;
    };
    // MySQL's display width (`int(11)`) means nothing; `tinyint(1)` is its boolean idiom.
    if dialect == Dialect::Mysql && matches!(base.as_str(), "tinyint" | "smallint" | "mediumint" | "int" | "bigint") {
        if base == "tinyint" && values == [1] {
            parsed.length = Some(1);
        }
        return parsed;
    }
    match (class, values.as_slice()) {
        (Some(L), [len]) => parsed.length = Some(*len),
        (_, [p]) => parsed.precision = Some(*p),
        (_, [p, s]) => {
            parsed.precision = Some(*p);
            parsed.scale = Some(*s);
        }
        _ => {
            let mut verbatim = ParsedType::verbatim(&text);
            verbatim.wrapper = parsed.wrapper;
            verbatim.nullable = parsed.nullable;
            return verbatim;
        }
    }
    parsed
}

// ---- rendering ---------------------------------------------------------------------

/// The length a type needs when the form left it blank (MySQL has no unbounded
/// `varchar`; SQL Server's bare `varchar` means length 1).
fn required_length(dialect: Dialect, lower: &str) -> Option<u32> {
    match (dialect, lower) {
        (Dialect::Mysql, "varchar" | "varbinary") => Some(255),
        (Dialect::Mysql, "char" | "binary") => Some(1),
        (Dialect::Mssql, "varchar" | "nvarchar" | "varbinary") => Some(255),
        (Dialect::Oracle, "varchar2" | "nvarchar2" | "raw") => Some(255),
        _ => None,
    }
}

fn quote_label(label: &str) -> String {
    format!("'{}'", label.replace('\'', "''"))
}

/// Oracle spells the parameters before the zone words: `TIMESTAMP(6) WITH TIME ZONE`.
fn place_params(dialect: Dialect, base: &str, params: &str) -> String {
    if dialect == Dialect::Oracle {
        if let Some(rest) = base.strip_prefix("TIMESTAMP ") {
            return format!("TIMESTAMP{params} {rest}");
        }
    }
    format!("{base}{params}")
}

/// The type exactly as it goes after the column name (no NULL / DEFAULT).
pub fn render_type(dialect: Dialect, column: &ColumnSpec) -> AppResult<String> {
    let base = column.data_type.trim();
    if base.is_empty() {
        return Err(AppError::invalid_input(format!("Pick a type for column \"{}\".", column.name)));
    }
    if column.wrapper.is_some() && dialect != Dialect::Clickhouse {
        return Err(AppError::invalid_input(format!("Column \"{}\": only ClickHouse columns can be wrapped in LowCardinality or Array.", column.name)));
    }
    let custom = base.contains('(') || base.contains('[');
    let lower = base.to_ascii_lowercase();
    let mut params = String::new();
    if !custom {
        if !column.enum_values.is_empty() {
            let labels: Vec<String> = if dialect == Dialect::Clickhouse {
                column.enum_values.iter().enumerate().map(|(n, v)| format!("{} = {}", quote_label(v), n + 1)).collect()
            } else {
                column.enum_values.iter().map(|v| quote_label(v)).collect()
            };
            params = format!("({})", labels.join(", "));
        } else if matches!(lower.as_str(), "enum" | "set" | "enum8" | "enum16") {
            return Err(AppError::invalid_input(format!("Column \"{}\": list the values of the {base} type.", column.name)));
        } else if let Some(length) = column.length {
            params = format!("({length})");
        } else if let Some(precision) = column.precision {
            params = match column.scale {
                Some(scale) => format!("({precision}, {scale})"),
                None => format!("({precision})"),
            };
        } else if let Some(length) = required_length(dialect, &lower) {
            params = format!("({length})");
        } else if dialect == Dialect::Clickhouse && lower == "fixedstring" {
            return Err(AppError::invalid_input(format!("Column \"{}\": FixedString needs a length.", column.name)));
        }
    }
    let mut out = place_params(dialect, base, &params);
    if dialect == Dialect::Mysql && column.unsigned {
        out.push_str(" UNSIGNED");
    }
    if dialect == Dialect::Clickhouse {
        if column.nullable {
            out = format!("Nullable({out})");
        }
        if let Some(wrapper) = column.wrapper.as_deref() {
            if !matches!(wrapper, "LowCardinality" | "Array") {
                return Err(AppError::invalid_input(format!("Column \"{}\": ClickHouse wrappers are LowCardinality and Array, not \"{wrapper}\".", column.name)));
            }
            out = format!("{wrapper}({out})");
        }
    }
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::model::ColumnDefault;

    pub fn col(name: &str, data_type: &str) -> ColumnSpec {
        ColumnSpec {
            name: name.into(),
            original_name: None,
            data_type: data_type.into(),
            length: None,
            precision: None,
            scale: None,
            wrapper: None,
            enum_values: Vec::new(),
            nullable: true,
            primary_key: false,
            unique: false,
            unique_name: None,
            auto_increment: false,
            unsigned: false,
            default: ColumnDefault::None,
        }
    }

    fn from_parsed(p: &ParsedType) -> ColumnSpec {
        ColumnSpec {
            data_type: p.base.clone(),
            length: p.length,
            precision: p.precision,
            scale: p.scale,
            unsigned: p.unsigned,
            enum_values: p.enum_values.clone(),
            wrapper: p.wrapper.clone(),
            nullable: p.nullable,
            ..col("c", "")
        }
    }

    /// parse then render must reproduce the catalogue's own spelling.
    fn round(dialect: Dialect, raw: &str) -> String {
        render_type(dialect, &from_parsed(&parse_type(dialect, raw))).unwrap_or_else(|e| e.to_string())
    }

    #[test]
    fn postgres_catalogue_spellings_land_on_the_pickers_names() {
        assert_eq!(round(Dialect::Postgres, "character varying(255)"), "varchar(255)");
        assert_eq!(round(Dialect::Postgres, "timestamp without time zone"), "timestamp");
        assert_eq!(round(Dialect::Postgres, "timestamp(3) with time zone"), "timestamptz(3)");
        assert_eq!(round(Dialect::Postgres, "time with time zone"), "timetz");
        assert_eq!(round(Dialect::Postgres, "numeric(10,2)"), "numeric(10, 2)");
        assert_eq!(round(Dialect::Postgres, "numeric(10)"), "numeric(10)");
        assert_eq!(round(Dialect::Postgres, "int4"), "integer");
        assert_eq!(round(Dialect::Postgres, "double precision"), "double precision");
        assert_eq!(round(Dialect::Postgres, "text[]"), "text[]");
        assert_eq!(round(Dialect::Postgres, "citext"), "citext");
        assert_eq!(round(Dialect::Postgres, "geometry(Point,4326)"), "geometry(Point,4326)");
        let parsed = parse_type(Dialect::Postgres, "character varying(40)");
        assert_eq!((parsed.base.as_str(), parsed.length), ("varchar", Some(40)));
    }

    #[test]
    fn mysql_drops_display_width_and_reads_unsigned_and_enums() {
        assert_eq!(round(Dialect::Mysql, "int(11)"), "int");
        assert_eq!(round(Dialect::Mysql, "int(10) unsigned"), "int UNSIGNED");
        assert_eq!(round(Dialect::Mysql, "bigint unsigned zerofill"), "bigint UNSIGNED");
        assert_eq!(round(Dialect::Mysql, "tinyint(1)"), "tinyint(1)");
        assert_eq!(round(Dialect::Mysql, "varchar(255)"), "varchar(255)");
        assert_eq!(round(Dialect::Mysql, "decimal(10,2)"), "decimal(10, 2)");
        assert_eq!(round(Dialect::Mysql, "enum('a','b c','it''s')"), "enum('a', 'b c', 'it''s')");
        assert_eq!(parse_type(Dialect::Mysql, "set('x','y')").enum_values, vec!["x".to_string(), "y".to_string()]);
        // A bare varchar must not reach MySQL without a length.
        assert_eq!(round(Dialect::Mysql, "varchar"), "varchar(255)");
    }

    #[test]
    fn clickhouse_wrappers_and_nullable_are_peeled_and_rebuilt() {
        let p = parse_type(Dialect::Clickhouse, "LowCardinality(Nullable(String))");
        assert_eq!((p.base.as_str(), p.wrapper.as_deref(), p.nullable), ("String", Some("LowCardinality"), true));
        assert_eq!(round(Dialect::Clickhouse, "LowCardinality(Nullable(String))"), "LowCardinality(Nullable(String))");
        assert_eq!(round(Dialect::Clickhouse, "Array(String)"), "Array(String)");
        assert_eq!(round(Dialect::Clickhouse, "Nullable(DateTime64(3))"), "Nullable(DateTime64(3))");
        assert_eq!(round(Dialect::Clickhouse, "Decimal(10, 2)"), "Decimal(10, 2)");
        assert_eq!(round(Dialect::Clickhouse, "FixedString(8)"), "FixedString(8)");
        assert_eq!(round(Dialect::Clickhouse, "Enum8('a' = 1, 'b' = 2)"), "Enum8('a' = 1, 'b' = 2)");
        // Custom numbering and structured types stay exactly as written.
        assert_eq!(round(Dialect::Clickhouse, "Enum8('a' = 5, 'b' = 7)"), "Enum8('a' = 5, 'b' = 7)");
        assert_eq!(round(Dialect::Clickhouse, "Map(String, UInt64)"), "Map(String, UInt64)");
        assert_eq!(round(Dialect::Clickhouse, "Array(LowCardinality(String))"), "Array(LowCardinality(String))");
        assert!(render_type(Dialect::Clickhouse, &col("c", "FixedString")).is_err());
    }

    #[test]
    fn other_dialects_normalise_case_and_aliases() {
        assert_eq!(round(Dialect::Sqlite, "varchar(20)"), "VARCHAR(20)");
        assert_eq!(round(Dialect::Sqlite, "integer"), "INTEGER");
        assert_eq!(round(Dialect::Duckdb, "INT"), "INTEGER");
        assert_eq!(round(Dialect::Duckdb, "DECIMAL(18,3)"), "DECIMAL(18, 3)");
        assert_eq!(round(Dialect::Oracle, "VARCHAR2(255 CHAR)"), "VARCHAR2(255)");
        assert_eq!(round(Dialect::Oracle, "NUMBER(10,2)"), "NUMBER(10, 2)");
        assert_eq!(round(Dialect::Oracle, "TIMESTAMP(6) WITH TIME ZONE"), "TIMESTAMP(6) WITH TIME ZONE");
        assert_eq!(round(Dialect::Mssql, "nvarchar(max)"), "nvarchar(max)");
        assert_eq!(round(Dialect::Mssql, "varchar"), "varchar(255)");
        assert_eq!(round(Dialect::Snowflake, "NUMBER(38,0)"), "NUMBER(38, 0)");
        assert_eq!(round(Dialect::Cassandra, "list<text>"), "list<text>");
    }

    #[test]
    fn catalogues_flag_what_each_type_can_do() {
        let pg = catalog(Dialect::Postgres);
        let find = |name: &str| pg.iter().find(|t| t.name == name).cloned();
        assert_eq!(find("varchar").map(|t| t.params), Some(TypeParams::Length));
        assert_eq!(find("numeric").map(|t| t.params), Some(TypeParams::PrecisionScale));
        assert_eq!(find("integer").map(|t| t.auto_increment), Some(true));
        assert_eq!(find("text").map(|t| t.auto_increment), Some(false));
        let my = catalog(Dialect::Mysql);
        assert!(my.iter().any(|t| t.name == "int" && t.unsigned && t.auto_increment));
        assert!(my.iter().any(|t| t.name == "enum" && t.params == TypeParams::Values));
        assert!(catalog(Dialect::Clickhouse).iter().all(|t| !t.auto_increment));
        assert!(catalog(Dialect::Mongo).is_empty());
        for dialect in [Dialect::Postgres, Dialect::Mysql, Dialect::Mssql, Dialect::Oracle, Dialect::Sqlite, Dialect::Duckdb, Dialect::Clickhouse, Dialect::Snowflake, Dialect::Bigquery, Dialect::Cassandra] {
            let names: Vec<String> = catalog(dialect).iter().map(|t| t.name.to_ascii_lowercase()).collect();
            let mut unique = names.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), names.len(), "{dialect:?} lists a type twice");
            let (column, key) = default_types(dialect);
            assert!(names.contains(&column.to_ascii_lowercase()) && names.contains(&key.to_ascii_lowercase()), "{dialect:?} default types");
        }
    }

    #[test]
    fn wrappers_outside_clickhouse_and_unlisted_enums_are_refused() {
        let mut c = col("c", "text");
        c.wrapper = Some("Array".into());
        assert!(render_type(Dialect::Postgres, &c).is_err());
        assert!(render_type(Dialect::Mysql, &col("c", "enum")).is_err());
        assert!(render_type(Dialect::Mysql, &col("c", " ")).is_err());
    }
}
