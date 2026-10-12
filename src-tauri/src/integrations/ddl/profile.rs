// SOT: ddl-profile-per-engine, ddl-support-flags, ddl-namespace-capabilities, ddl-engine-notes

use super::dialect::{dialect_of, Dialect};
use super::types::{catalog, default_types};
use crate::model::{DdlProfile, DdlSupport, Engine, NamespaceCaps, NamespaceKind};

// WHAT:  The catalogue the schema forms are built from for one engine: its types,
//        what ALTER can and cannot do, which databases / schemas can be created,
//        renamed or dropped, and the caveats worth showing.
// WHY:   The UI never asks `if engine === "sqlite"`. It reads these flags to
//        enable, disable or explain a control, and the builders consult the same
//        flags to refuse what the engine cannot do, so a greyed-out button and an
//        error message always say the same thing.
// HOW:   Static per dialect. A refused namespace operation carries its reason in
//        `NamespaceCaps::note`; the namespace builder returns that text as its
//        error.
// WHERE: src-tauri/src/integrations/ddl/{namespace,alter}.rs, src/features/schema/

/// All flags off except the ones named: keeps each dialect's table readable.
fn support(f: impl FnOnce(&mut DdlSupport)) -> DdlSupport {
    let mut s = DdlSupport::NONE;
    f(&mut s);
    s
}

fn full_alter(s: &mut DdlSupport) {
    s.create_table = true;
    s.alter_table = true;
    s.rename_table = true;
    s.drop_column = true;
    s.rename_column = true;
    s.change_type = true;
    s.change_nullability = true;
    s.change_default = true;
    s.change_unique = true;
    s.change_primary_key = true;
    s.foreign_keys = true;
    s.alter_foreign_keys = true;
    s.duplicate_table = true;
    s.truncate_table = true;
    s.drop_table = true;
    s.auto_increment = true;
}

pub fn support_of(dialect: Dialect) -> DdlSupport {
    match dialect {
        Dialect::Postgres | Dialect::Mysql | Dialect::Mssql | Dialect::Oracle | Dialect::Snowflake => support(full_alter),
        Dialect::Sqlite => support(|s| {
            full_alter(s);
            s.rebuilds = true;
        }),
        Dialect::Duckdb => support(|s| {
            full_alter(s);
            s.change_unique = false;
            s.change_primary_key = false;
            s.alter_foreign_keys = false;
        }),
        Dialect::Clickhouse => support(|s| {
            s.create_table = true;
            s.alter_table = true;
            s.rename_table = true;
            s.drop_column = true;
            s.rename_column = true;
            s.change_type = true;
            s.change_nullability = true;
            s.change_default = true;
            s.duplicate_table = true;
            s.truncate_table = true;
            s.drop_table = true;
            s.table_engine = true;
        }),
        Dialect::Bigquery => support(|s| {
            s.create_table = true;
            s.alter_table = true;
            s.rename_table = true;
            s.drop_column = true;
            s.rename_column = true;
            s.change_type = true;
            s.change_nullability = true;
            s.change_default = true;
            s.foreign_keys = true;
            s.duplicate_table = true;
            s.truncate_table = true;
            s.drop_table = true;
        }),
        Dialect::Cassandra => support(|s| {
            s.create_table = true;
            s.alter_table = true;
            s.drop_column = true;
            s.truncate_table = true;
            s.drop_table = true;
        }),
        Dialect::Mongo => support(|s| {
            s.create_table = true;
            s.alter_table = true;
            s.rename_table = true;
            s.duplicate_table = true;
            s.truncate_table = true;
            s.drop_table = true;
        }),
    }
}

fn caps(kind: NamespaceKind, create: bool, rename: bool, drop: bool, owner: bool, note: Option<&str>) -> NamespaceCaps {
    NamespaceCaps { kind, create, rename, drop, owner, note: note.map(String::from) }
}

/// What the forms can do with each kind of namespace the dialect has.
pub fn namespace_caps(dialect: Dialect) -> Vec<NamespaceCaps> {
    use NamespaceKind::{Database, Schema};
    match dialect {
        Dialect::Postgres => vec![
            caps(Schema, true, true, true, true, None),
            caps(Database, true, true, true, true, Some("PostgreSQL cannot rename or drop the database you are connected to, and CREATE DATABASE has no IF NOT EXISTS; connect to another database first.")),
        ],
        Dialect::Mysql => vec![caps(
            Database,
            true,
            false,
            true,
            false,
            Some("MySQL has no RENAME DATABASE; create a new database, move the tables with RENAME TABLE, then drop the old one. A schema is a database in MySQL."),
        )],
        Dialect::Mssql => vec![
            caps(Schema, true, false, true, true, Some("SQL Server cannot rename a schema; create a new one and ALTER SCHEMA … TRANSFER the objects. A schema must be empty to be dropped.")),
            caps(Database, true, true, true, false, Some("SQL Server cannot rename or drop the database you are connected to; connect to another one first.")),
        ],
        Dialect::Oracle => Vec::new(),
        Dialect::Sqlite => Vec::new(),
        Dialect::Duckdb => vec![caps(Schema, true, false, true, false, Some("DuckDB has no ALTER SCHEMA … RENAME; create a new schema and move the tables."))],
        Dialect::Clickhouse => vec![caps(Database, true, true, true, false, Some("RENAME DATABASE works only on databases using the Atomic engine."))],
        Dialect::Snowflake => vec![caps(Schema, true, true, true, false, None), caps(Database, true, true, true, false, None)],
        Dialect::Bigquery => vec![caps(
            Schema,
            true,
            false,
            true,
            false,
            Some("A BigQuery dataset is a schema. Datasets cannot be renamed; copy the tables into a new dataset."),
        )],
        Dialect::Cassandra => vec![caps(
            Database,
            true,
            false,
            true,
            false,
            Some("A Cassandra keyspace is a database. Keyspaces cannot be renamed, and new ones use SimpleStrategy with replication factor 1; alter the replication in cqlsh for production."),
        )],
        Dialect::Mongo => vec![caps(
            Database,
            false,
            false,
            true,
            false,
            Some("MongoDB creates a database when its first collection is created, and cannot rename one; copy its collections into a new database instead."),
        )],
    }
}

/// Why `op` ("create" / "rename" / "drop") on `kind` is unavailable, or None when it is allowed.
pub fn namespace_refusal(dialect: Dialect, kind: NamespaceKind, op: &str) -> Option<String> {
    let all = namespace_caps(dialect);
    let Some(found) = all.iter().find(|c| c.kind == kind) else {
        return Some(match (dialect, kind) {
            (Dialect::Mysql, NamespaceKind::Schema) => "MySQL calls a schema a database; use the database actions.".to_string(),
            (Dialect::Mongo, _) => "MongoDB only has databases.".to_string(),
            _ => format!("{} has no {}s the schema forms can manage.", dialect.label(), kind.noun()),
        });
    };
    let allowed = match op {
        "create" => found.create,
        "rename" => found.rename,
        _ => found.drop,
    };
    if allowed {
        None
    } else {
        Some(found.note.clone().unwrap_or_else(|| format!("{} cannot {op} a {}.", dialect.label(), kind.noun())))
    }
}

fn notes(dialect: Dialect) -> Vec<String> {
    let implicit_commit = |name: &str| format!("{name} commits each DDL statement immediately: a failure part-way leaves the earlier statements applied.");
    match dialect {
        Dialect::Postgres => vec!["Statements run in one transaction: if any fails, none is applied.".into()],
        Dialect::Mysql => vec![implicit_commit("MySQL")],
        Dialect::Mssql => vec!["Statements run in one transaction: if any fails, none is applied.".into()],
        Dialect::Oracle => vec![implicit_commit("Oracle")],
        Dialect::Sqlite => vec![
            "SQLite renames columns and adds or drops unconstrained ones in place. Any other change rebuilds the table: its rows are copied into a new table, the old one is dropped and the new one renamed. Triggers on the table are not recreated; views that use it may need recreating.".into(),
        ],
        Dialect::Duckdb => vec!["DuckDB cannot add or drop UNIQUE, PRIMARY KEY or FOREIGN KEY constraints on an existing table; define them when creating it.".into()],
        Dialect::Clickhouse => vec![
            "ClickHouse does not enforce primary keys, unique constraints or foreign keys; ENGINE and ORDER BY cannot be changed after creation. Nullable columns are written as Nullable(type).".into(),
            implicit_commit("ClickHouse"),
        ],
        Dialect::Snowflake => vec![
            "Snowflake records primary, unique and foreign keys but does not enforce them.".into(),
            implicit_commit("Snowflake"),
        ],
        Dialect::Bigquery => vec![
            "BigQuery primary and foreign keys are not enforced. A column's type can only be widened (INT64 to NUMERIC, for example) and NOT NULL can only be dropped.".into(),
            implicit_commit("BigQuery"),
        ],
        Dialect::Cassandra => vec![
            "Cassandra tables are defined by their primary key (partition and clustering columns); a column can be added or dropped but not retyped, and only primary-key columns can be renamed.".into(),
            implicit_commit("Cassandra"),
        ],
        Dialect::Mongo => vec!["MongoDB collections have no fixed columns, and commands run immediately and cannot be undone.".into()],
    }
}

pub fn profile(engine: Engine) -> DdlProfile {
    let Some(dialect) = dialect_of(engine) else {
        let reason = if engine == Engine::Questdb {
            "QuestDB speaks the PostgreSQL protocol but has its own CREATE TABLE (designated timestamp, no keys); use the query editor.".to_string()
        } else {
            format!("{} is not driven through the schema forms; use its own console to change its structure.", engine.label())
        };
        return DdlProfile::unsupported(engine.label(), &reason);
    };
    let (default_type, key_type) = default_types(dialect);
    let is_clickhouse = dialect == Dialect::Clickhouse;
    DdlProfile {
        supported: true,
        dialect: dialect.label().to_string(),
        noun: if dialect == Dialect::Mongo { "Collection" } else { "Table" }.to_string(),
        namespaces: namespace_caps(dialect),
        types: catalog(dialect),
        default_type: default_type.to_string(),
        key_type: key_type.to_string(),
        wrappers: if is_clickhouse { vec!["LowCardinality".to_string(), "Array".to_string()] } else { Vec::new() },
        engines: if is_clickhouse {
            [
                "MergeTree",
                "ReplacingMergeTree",
                "SummingMergeTree",
                "AggregatingMergeTree",
                "CollapsingMergeTree",
                "VersionedCollapsingMergeTree",
                "Log",
                "TinyLog",
                "StripeLog",
                "Memory",
                "Null",
            ]
            .iter()
            .map(|e| (*e).to_string())
            .collect()
        } else {
            Vec::new()
        },
        support: support_of(dialect),
        notes: notes(dialect),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_engine_has_types_and_unsupported_ones_say_why() {
        let pg = profile(Engine::Neon);
        assert!(pg.supported && pg.support.change_unique && !pg.support.rebuilds && pg.types.len() > 20);
        assert_eq!(pg.default_type, "varchar");
        let sqlite = profile(Engine::Sqlite);
        assert!(sqlite.support.rebuilds && sqlite.support.change_primary_key && sqlite.namespaces.is_empty());
        let ch = profile(Engine::Clickhouse);
        assert!(ch.support.table_engine && !ch.support.foreign_keys && ch.engines.contains(&"MergeTree".to_string()) && ch.wrappers.len() == 2);
        let mongo = profile(Engine::Mongodb);
        assert_eq!((mongo.noun.as_str(), mongo.types.len()), ("Collection", 0));
        assert!(mongo.support.rename_table && !mongo.support.drop_column);
        let redis = profile(Engine::Redis);
        assert!(!redis.supported && !redis.notes.is_empty() && redis.types.is_empty());
        assert!(!profile(Engine::Questdb).supported);
        let duck = profile(Engine::Duckdb);
        assert!(!duck.support.change_unique && duck.support.foreign_keys && !duck.support.alter_foreign_keys);
        for engine in Engine::ALL {
            let p = profile(engine);
            assert_eq!(p.supported, !p.types.is_empty() || dialect_of(engine) == Some(Dialect::Mongo), "{engine:?}");
        }
    }

    #[test]
    fn namespace_refusals_carry_the_reason_the_menu_shows() {
        assert!(namespace_refusal(Dialect::Postgres, NamespaceKind::Schema, "rename").is_none());
        let mysql_rename = namespace_refusal(Dialect::Mysql, NamespaceKind::Database, "rename").unwrap_or_default();
        assert!(mysql_rename.starts_with("MySQL has no RENAME DATABASE"), "{mysql_rename}");
        assert_eq!(namespace_refusal(Dialect::Mysql, NamespaceKind::Schema, "create").as_deref(), Some("MySQL calls a schema a database; use the database actions."));
        assert!(namespace_refusal(Dialect::Sqlite, NamespaceKind::Database, "create").is_some());
        assert!(namespace_refusal(Dialect::Mongo, NamespaceKind::Database, "create").is_some());
        assert!(namespace_refusal(Dialect::Mongo, NamespaceKind::Database, "drop").is_none());
        assert!(namespace_refusal(Dialect::Mssql, NamespaceKind::Schema, "rename").unwrap_or_default().contains("TRANSFER"));
    }
}
