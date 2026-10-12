// SOT: ddl-spec, table-spec, column-spec, foreign-key-spec, index-spec, alter-spec, duplicate-spec, remove-spec, namespace-spec, ddl-preview, ddl-profile

use super::schema::{TableKind, TableRef};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ============================================================================
// SCHEMA FORMS: THE TYPED DESCRIPTION OF A TABLE AND OF A CHANGE TO IT
//
// WHAT:  The shapes the Create / Edit / Duplicate / Drop / Namespace forms speak.
//        The UI fills a `TableSpec`; the adapter's DDL builders turn it into the
//        engine's own statements (`integrations/ddl`), the UI previews them, and
//        the user runs them through `execute_query` (so the read-only lock and the
//        destructive confirmation apply as they do for any other script).
// WHY:   SQL is never assembled in the webview. Quoting, type spelling,
//        auto-increment, defaults and ALTER syntax differ per dialect, so one typed
//        description is written once and rendered per engine, with tests.
// HOW:   `ColumnSpec::original_name` / `ForeignKeySpec::original_name` give a
//        column or constraint an identity across an edit: a spec whose name is
//        Some(old) is the live `old`; None is new; a live one nobody refers to was
//        removed. The edit diff is computed from (original, desired) in Rust.
// WHERE: src-tauri/src/integrations/ddl/, src/features/schema/
// ============================================================================

/// What a new column's DEFAULT is. `None` writes no clause at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ColumnDefault {
    None,
    Null,
    /// The current timestamp, spelled the way the dialect spells it.
    Now,
    /// The empty string.
    Empty,
    /// A literal or expression written out verbatim (the user owns its syntax).
    Custom { expression: String },
}

/// A foreign key's ON DELETE / ON UPDATE action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FkAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl FkAction {
    pub const ALL: [FkAction; 5] = [FkAction::NoAction, FkAction::Restrict, FkAction::Cascade, FkAction::SetNull, FkAction::SetDefault];

    pub fn sql(self) -> &'static str {
        match self {
            FkAction::NoAction => "NO ACTION",
            FkAction::Restrict => "RESTRICT",
            FkAction::Cascade => "CASCADE",
            FkAction::SetNull => "SET NULL",
            FkAction::SetDefault => "SET DEFAULT",
        }
    }

    /// Reads the spelling a catalog reports ("CASCADE", "set null", "SET_NULL", "r"…).
    pub fn parse(raw: &str) -> FkAction {
        match raw.trim().to_ascii_uppercase().replace('_', " ").as_str() {
            "CASCADE" | "C" => FkAction::Cascade,
            "RESTRICT" | "R" => FkAction::Restrict,
            "SET NULL" | "N" => FkAction::SetNull,
            "SET DEFAULT" | "D" => FkAction::SetDefault,
            _ => FkAction::NoAction,
        }
    }
}

/// One column of a table being created, duplicated or edited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ColumnSpec {
    pub name: String,
    /// The name this column has in the live table: Some on an edit of an
    /// existing column (so a rename is told from a drop + add), None for a new one.
    pub original_name: Option<String>,
    /// Base type as the picker lists it (`varchar`, `numeric`, `UInt64`…); a type
    /// the picker does not know (a user type, `citext`) is written out as it is.
    pub data_type: String,
    /// `varchar(255)`: the length parameter.
    pub length: Option<u32>,
    /// `numeric(10, 2)`: precision; also the single parameter of `datetime(3)`.
    pub precision: Option<u32>,
    pub scale: Option<u32>,
    /// ClickHouse only: `LowCardinality` or `Array`, applied around the type.
    pub wrapper: Option<String>,
    /// MySQL `enum`/`set`, ClickHouse `Enum8`/`Enum16`: the allowed values.
    pub enum_values: Vec<String>,
    pub nullable: bool,
    pub primary_key: bool,
    pub unique: bool,
    /// The live constraint or index that makes `unique` true; needed to drop it.
    pub unique_name: Option<String>,
    pub auto_increment: bool,
    /// MySQL `UNSIGNED`.
    pub unsigned: bool,
    pub default: ColumnDefault,
}

/// One foreign-key constraint (composite keys list several columns).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ForeignKeySpec {
    /// Constraint name; None lets the engine (or the builder) pick one.
    pub name: Option<String>,
    /// The live constraint's name on an edit; None for a new one.
    pub original_name: Option<String>,
    pub columns: Vec<String>,
    pub ref_table: TableRef,
    pub ref_columns: Vec<String>,
    pub on_delete: FkAction,
    pub on_update: FkAction,
}

/// A secondary index (or a multi-column unique constraint). Read from the live
/// table so that a SQLite rebuild and "duplicate with indexes" can keep them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IndexSpec {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

/// A whole table: what Create writes, what Edit loads and sends back changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TableSpec {
    pub table: TableRef,
    pub columns: Vec<ColumnSpec>,
    /// The live primary-key constraint's name, when the engine names it and it is known.
    pub primary_key_name: Option<String>,
    pub foreign_keys: Vec<ForeignKeySpec>,
    pub indexes: Vec<IndexSpec>,
    /// ClickHouse `ENGINE` (`MergeTree`…); None for every other dialect.
    pub engine: Option<String>,
    /// ClickHouse `ORDER BY` columns; empty falls back to the primary key.
    pub order_by: Vec<String>,
    pub if_not_exists: bool,
}

/// Edit table: the live table as loaded, and the table as the form leaves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AlterTableSpec {
    pub original: TableSpec,
    pub desired: TableSpec,
}

/// Duplicate table: which copy to make of `source`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateSpec {
    pub source: TableRef,
    pub target: TableRef,
    pub include_data: bool,
    pub include_indexes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RemoveAction {
    /// Empty the table, keep its structure.
    Truncate,
    /// Remove the table or view itself.
    Drop,
}

/// Truncate or drop a table, or drop a view (`kind` picks DROP TABLE / DROP VIEW).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoveSpec {
    pub table: TableRef,
    pub kind: TableKind,
    pub action: RemoveAction,
    /// CASCADE where the dialect has it (Postgres, Oracle, Snowflake…).
    pub cascade: bool,
    pub if_exists: bool,
}

/// What a namespace is called in this engine (Postgres schema, MySQL database…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NamespaceKind {
    Schema,
    Database,
}

impl NamespaceKind {
    pub fn noun(self) -> &'static str {
        match self {
            NamespaceKind::Schema => "schema",
            NamespaceKind::Database => "database",
        }
    }
}

/// What the forms can do with one kind of namespace on this engine. A refused
/// operation says why in `note`, so the menu can explain instead of just greying out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NamespaceCaps {
    pub kind: NamespaceKind,
    pub create: bool,
    pub rename: bool,
    pub drop: bool,
    /// Create accepts an owner (Postgres AUTHORIZATION).
    pub owner: bool,
    /// Why an operation is unavailable, or what to know before using one.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum NamespaceOp {
    /// `owner` is Postgres' AUTHORIZATION; ignored by dialects without one.
    #[serde(rename_all = "camelCase")]
    Create { owner: Option<String>, if_not_exists: bool },
    Rename { to: String },
    #[serde(rename_all = "camelCase")]
    Drop { cascade: bool, if_exists: bool },
}

/// Create / rename / drop a database or schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NamespaceSpec {
    pub kind: NamespaceKind,
    pub name: String,
    pub op: NamespaceOp,
}

/// One schema form submission, whatever it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(export)]
pub enum DdlOperation {
    CreateTable { spec: TableSpec },
    AlterTable { spec: AlterTableSpec },
    DuplicateTable { spec: DuplicateSpec },
    RemoveTable { spec: RemoveSpec },
    Namespace { spec: NamespaceSpec },
}

/// The statements a form would run, for the user to read before running them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DdlPreview {
    pub statements: Vec<String>,
    /// The statements as one runnable script (transaction wrapper included where
    /// the engine's DDL is transactional); this is what `execute_query` is given.
    pub script: String,
    /// The block would ask for confirmation (a DROP, TRUNCATE or ALTER … DROP in
    /// the script); the form must collect it before passing `confirmDestructive`.
    pub destructive: bool,
    /// Set when the form must make the user type this name to continue (dropping
    /// or emptying a table, dropping a schema or database, dropping columns).
    pub confirm_name: Option<String>,
    /// Caveats worth reading before running (implicit commit, table rebuild…).
    pub notes: Vec<String>,
}

// ---- what an engine's forms offer ------------------------------------------------

/// How many parameters a type takes, which decides the extra inputs the form shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TypeParams {
    None,
    /// `varchar(255)`
    Length,
    /// `datetime2(3)`, `Decimal64(4)`: one precision.
    Precision,
    /// `numeric(10, 2)`
    PrecisionScale,
    /// `enum('a', 'b')`
    Values,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DdlType {
    pub name: String,
    pub params: TypeParams,
    /// An auto-increment column may have this type.
    pub auto_increment: bool,
    /// `UNSIGNED` applies (MySQL integers).
    pub unsigned: bool,
}

/// What the engine can do through the forms; every flag the UI uses to enable,
/// disable or explain a control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DdlSupport {
    pub create_table: bool,
    pub alter_table: bool,
    pub rename_table: bool,
    pub drop_column: bool,
    pub rename_column: bool,
    pub change_type: bool,
    pub change_nullability: bool,
    pub change_default: bool,
    pub change_unique: bool,
    pub change_primary_key: bool,
    /// Foreign keys can be declared when creating a table.
    pub foreign_keys: bool,
    /// Foreign keys can be added to / dropped from an existing table.
    pub alter_foreign_keys: bool,
    /// An edit the engine cannot ALTER is carried out by rebuilding the table (SQLite).
    pub rebuilds: bool,
    pub duplicate_table: bool,
    pub truncate_table: bool,
    pub drop_table: bool,
    pub auto_increment: bool,
    /// ClickHouse: the form asks for an ENGINE and an ORDER BY.
    pub table_engine: bool,
}

impl DdlSupport {
    /// Nothing: an engine the forms do not speak to.
    pub const NONE: DdlSupport = DdlSupport {
        create_table: false,
        alter_table: false,
        rename_table: false,
        drop_column: false,
        rename_column: false,
        change_type: false,
        change_nullability: false,
        change_default: false,
        change_unique: false,
        change_primary_key: false,
        foreign_keys: false,
        alter_foreign_keys: false,
        rebuilds: false,
        duplicate_table: false,
        truncate_table: false,
        drop_table: false,
        auto_increment: false,
        table_engine: false,
    };
}

/// The per-engine catalogue the forms are built from: type picker, switches and
/// the caveats to show. Static per engine (no session round trip beyond the call).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DdlProfile {
    /// The forms speak this engine's language at all.
    pub supported: bool,
    /// Short name of the dialect the statements are written in ("PostgreSQL", "MongoDB"…).
    pub dialect: String,
    /// "Table" or "Collection": what the engine calls the thing the forms edit.
    pub noun: String,
    /// The databases / schemas the forms can create, rename or drop, with what
    /// each can do. Empty when the engine has none (a SQLite file) or the forms
    /// cannot reach them.
    pub namespaces: Vec<NamespaceCaps>,
    pub types: Vec<DdlType>,
    /// Type offered for a new column.
    pub default_type: String,
    /// Type offered for the first (primary-key) column of a new table.
    pub key_type: String,
    /// ClickHouse: types that wrap another (`LowCardinality`, `Array`).
    pub wrappers: Vec<String>,
    /// ClickHouse: table engines offered.
    pub engines: Vec<String>,
    pub support: DdlSupport,
    /// Caveats the forms show (`MySQL commits DDL immediately`…).
    pub notes: Vec<String>,
}

impl DdlProfile {
    /// An engine the forms cannot drive; `notes` says why.
    pub fn unsupported(dialect: &str, note: &str) -> DdlProfile {
        DdlProfile {
            supported: false,
            dialect: dialect.to_string(),
            noun: "Table".to_string(),
            namespaces: Vec::new(),
            types: Vec::new(),
            default_type: String::new(),
            key_type: String::new(),
            wrappers: Vec::new(),
            engines: Vec::new(),
            support: DdlSupport::NONE,
            notes: vec![note.to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fk_actions_round_trip_through_sql_and_catalog_spellings() {
        for action in FkAction::ALL {
            assert_eq!(FkAction::parse(action.sql()), action);
        }
        assert_eq!(FkAction::parse("set_null"), FkAction::SetNull);
        assert_eq!(FkAction::parse("c"), FkAction::Cascade);
        assert_eq!(FkAction::parse("whatever"), FkAction::NoAction);
    }

    #[test]
    fn defaults_serialise_as_tagged_objects() {
        let json = serde_json::to_string(&ColumnDefault::Custom { expression: "1".into() }).unwrap_or_default();
        assert_eq!(json, r#"{"kind":"custom","expression":"1"}"#);
        assert_eq!(serde_json::to_string(&ColumnDefault::Now).unwrap_or_default(), r#"{"kind":"now"}"#);
    }

    #[test]
    fn operations_serialise_with_an_op_tag() {
        let op = DdlOperation::Namespace {
            spec: NamespaceSpec { kind: NamespaceKind::Schema, name: "s".into(), op: NamespaceOp::Drop { cascade: true, if_exists: false } },
        };
        let json = serde_json::to_string(&op).unwrap_or_default();
        assert!(json.starts_with(r#"{"op":"namespace","spec":{"#), "{json}");
        assert!(json.contains(r#""op":"drop","cascade":true,"ifExists":false"#), "{json}");
    }
}
