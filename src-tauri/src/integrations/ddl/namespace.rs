// SOT: create-schema-sql, rename-schema-sql, drop-schema-sql, create-database-sql, drop-database-sql

use super::dialect::{check_name, Cx, Dialect};
use super::mongo;
use super::profile::namespace_refusal;
use crate::error::{AppError, AppResult};
use crate::model::{NamespaceKind, NamespaceOp, NamespaceSpec};

// WHAT:  CREATE / RENAME / DROP for schemas and databases.
// WHY:   The sidebar offers these on the database picker and on schema headers.
//        Engines disagree wildly (MySQL has no RENAME DATABASE, SQL Server has no
//        schema rename, MongoDB creates databases lazily), so each refusal is
//        written once in the profile and returned here as the error.
// HOW:   The profile's `namespace_refusal` decides whether an operation exists;
//        this module only spells the ones that do.
// WHERE: src-tauri/src/integrations/ddl/profile.rs

pub fn namespace(cx: &Cx, spec: &NamespaceSpec) -> AppResult<Vec<String>> {
    check_name(spec.kind.noun(), &spec.name)?;
    let op_name = match &spec.op {
        NamespaceOp::Create { .. } => "create",
        NamespaceOp::Rename { .. } => "rename",
        NamespaceOp::Drop { .. } => "drop",
    };
    if cx.dialect == Dialect::Mongo {
        return mongo::namespace(spec);
    }
    if let Some(reason) = namespace_refusal(cx.dialect, spec.kind, op_name) {
        return Err(AppError::invalid_input(reason));
    }
    if let NamespaceOp::Rename { to } = &spec.op {
        check_name(spec.kind.noun(), to)?;
        if to == &spec.name {
            return Err(AppError::invalid_input("Pick a different name."));
        }
    }
    let name = cx.q(&spec.name);
    let noun = match (cx.dialect, spec.kind) {
        (Dialect::Cassandra, _) => "KEYSPACE",
        (_, NamespaceKind::Database) => "DATABASE",
        (_, NamespaceKind::Schema) => "SCHEMA",
    };
    let statements = match (&spec.op, cx.dialect) {
        // ---- create ----
        (NamespaceOp::Create { owner, if_not_exists }, Dialect::Mssql) => {
            let owner_sql = match (spec.kind, owner.as_deref().map(str::trim).filter(|o| !o.is_empty())) {
                (NamespaceKind::Schema, Some(o)) => format!(" AUTHORIZATION {}", cx.q(o)),
                _ => String::new(),
            };
            let create = format!("CREATE {noun} {name}{owner_sql}");
            if *if_not_exists {
                // CREATE SCHEMA must be alone in its batch, so the guarded form runs it dynamically.
                let probe = match spec.kind {
                    NamespaceKind::Schema => format!("SCHEMA_ID(N{}) IS NULL", cx.lit(&spec.name)),
                    NamespaceKind::Database => format!("DB_ID(N{}) IS NULL", cx.lit(&spec.name)),
                };
                vec![format!("IF {probe} EXEC(N{})", cx.lit(&create))]
            } else {
                vec![create]
            }
        }
        (NamespaceOp::Create { if_not_exists, .. }, Dialect::Cassandra) => {
            vec![format!(
                "CREATE KEYSPACE {}{name} WITH replication = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
                if *if_not_exists { "IF NOT EXISTS " } else { "" }
            )]
        }
        (NamespaceOp::Create { owner, if_not_exists }, _) => {
            let owner = owner.as_deref().map(str::trim).filter(|o| !o.is_empty());
            let guard = if *if_not_exists && !(cx.dialect == Dialect::Postgres && spec.kind == NamespaceKind::Database) { "IF NOT EXISTS " } else { "" };
            let owner_sql = match (cx.dialect, spec.kind, owner) {
                (Dialect::Postgres, NamespaceKind::Schema, Some(o)) => format!(" AUTHORIZATION {}", cx.q(o)),
                (Dialect::Postgres, NamespaceKind::Database, Some(o)) => format!(" OWNER {}", cx.q(o)),
                _ => String::new(),
            };
            vec![format!("CREATE {noun} {guard}{name}{owner_sql}")]
        }
        // ---- rename ----
        (NamespaceOp::Rename { to }, Dialect::Mssql) => vec![format!("ALTER DATABASE {name} MODIFY NAME = {}", cx.q(to))],
        (NamespaceOp::Rename { to }, Dialect::Clickhouse) => vec![format!("RENAME DATABASE {name} TO {}", cx.q(to))],
        (NamespaceOp::Rename { to }, _) => vec![format!("ALTER {noun} {name} RENAME TO {}", cx.q(to))],
        // ---- drop ----
        (NamespaceOp::Drop { cascade, if_exists }, dialect) => {
            let guard = if *if_exists { "IF EXISTS " } else { "" };
            let tail = match (dialect, spec.kind) {
                (Dialect::Postgres | Dialect::Duckdb | Dialect::Snowflake | Dialect::Bigquery, NamespaceKind::Schema) if *cascade => " CASCADE",
                _ => "",
            };
            vec![format!("DROP {noun} {guard}{name}{tail}")]
        }
    };
    Ok(statements)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Engine;

    fn ns(kind: NamespaceKind, name: &str, op: NamespaceOp) -> NamespaceSpec {
        NamespaceSpec { kind, name: name.into(), op }
    }

    fn run(engine: Engine, spec: &NamespaceSpec) -> String {
        super::super::namespace(engine, spec).map(|s| s.join(";")).unwrap_or_else(|e| format!("ERR {e}"))
    }

    fn create(owner: Option<&str>, if_not_exists: bool) -> NamespaceOp {
        NamespaceOp::Create { owner: owner.map(String::from), if_not_exists }
    }

    #[test]
    fn postgres_schemas_and_databases() {
        assert_eq!(run(Engine::Postgres, &ns(NamespaceKind::Schema, "billing", create(Some("app_owner"), true))), "CREATE SCHEMA IF NOT EXISTS \"billing\" AUTHORIZATION \"app_owner\"");
        assert_eq!(run(Engine::Postgres, &ns(NamespaceKind::Schema, "billing", NamespaceOp::Rename { to: "invoices".into() })), "ALTER SCHEMA \"billing\" RENAME TO \"invoices\"");
        assert_eq!(run(Engine::Postgres, &ns(NamespaceKind::Schema, "billing", NamespaceOp::Drop { cascade: true, if_exists: true })), "DROP SCHEMA IF EXISTS \"billing\" CASCADE");
        assert_eq!(run(Engine::Postgres, &ns(NamespaceKind::Database, "analytics", create(Some("bi"), true))), "CREATE DATABASE \"analytics\" OWNER \"bi\"", "PostgreSQL has no CREATE DATABASE IF NOT EXISTS");
        assert_eq!(run(Engine::Postgres, &ns(NamespaceKind::Database, "analytics", NamespaceOp::Rename { to: "bi".into() })), "ALTER DATABASE \"analytics\" RENAME TO \"bi\"");
        assert_eq!(run(Engine::Postgres, &ns(NamespaceKind::Database, "analytics", NamespaceOp::Drop { cascade: true, if_exists: false })), "DROP DATABASE \"analytics\"");
        assert!(run(Engine::Postgres, &ns(NamespaceKind::Schema, "same", NamespaceOp::Rename { to: "same".into() })).starts_with("ERR"));
    }

    #[test]
    fn mysql_cannot_rename_a_database_and_says_so() {
        assert_eq!(run(Engine::Mysql, &ns(NamespaceKind::Database, "shop", create(Some("ignored"), true))), "CREATE DATABASE IF NOT EXISTS `shop`");
        assert_eq!(run(Engine::Mariadb, &ns(NamespaceKind::Database, "shop", NamespaceOp::Drop { cascade: true, if_exists: true })), "DROP DATABASE IF EXISTS `shop`");
        let refused = run(Engine::Mysql, &ns(NamespaceKind::Database, "shop", NamespaceOp::Rename { to: "x".into() }));
        assert!(refused.starts_with("ERR MySQL has no RENAME DATABASE"), "{refused}");
        assert!(run(Engine::Mysql, &ns(NamespaceKind::Schema, "shop", create(None, false))).starts_with("ERR MySQL calls a schema a database"));
    }

    #[test]
    fn sql_server_guards_create_and_explains_schema_rename() {
        assert_eq!(run(Engine::Mssql, &ns(NamespaceKind::Schema, "sales", create(Some("dbo"), false))), "CREATE SCHEMA [sales] AUTHORIZATION [dbo]");
        assert_eq!(run(Engine::Mssql, &ns(NamespaceKind::Schema, "sales", create(None, true))), "IF SCHEMA_ID(N'sales') IS NULL EXEC(N'CREATE SCHEMA [sales]')");
        assert_eq!(run(Engine::Mssql, &ns(NamespaceKind::Database, "crm", create(None, true))), "IF DB_ID(N'crm') IS NULL EXEC(N'CREATE DATABASE [crm]')");
        assert_eq!(run(Engine::Mssql, &ns(NamespaceKind::Database, "crm", NamespaceOp::Rename { to: "crm2".into() })), "ALTER DATABASE [crm] MODIFY NAME = [crm2]");
        assert!(run(Engine::Mssql, &ns(NamespaceKind::Schema, "sales", NamespaceOp::Rename { to: "x".into() })).contains("TRANSFER"));
        assert_eq!(run(Engine::Mssql, &ns(NamespaceKind::Schema, "sales", NamespaceOp::Drop { cascade: true, if_exists: true })), "DROP SCHEMA IF EXISTS [sales]");
    }

    #[test]
    fn the_rest_of_the_engines() {
        assert_eq!(run(Engine::Duckdb, &ns(NamespaceKind::Schema, "staging", create(None, true))), "CREATE SCHEMA IF NOT EXISTS \"staging\"");
        assert!(run(Engine::Duckdb, &ns(NamespaceKind::Schema, "staging", NamespaceOp::Rename { to: "x".into() })).starts_with("ERR DuckDB has no ALTER SCHEMA"));
        assert_eq!(run(Engine::Clickhouse, &ns(NamespaceKind::Database, "logs", NamespaceOp::Rename { to: "audit".into() })), "RENAME DATABASE \"logs\" TO \"audit\"");
        assert_eq!(run(Engine::Snowflake, &ns(NamespaceKind::Schema, "raw", NamespaceOp::Rename { to: "bronze".into() })), "ALTER SCHEMA \"raw\" RENAME TO \"bronze\"");
        assert_eq!(run(Engine::Snowflake, &ns(NamespaceKind::Database, "lake", NamespaceOp::Drop { cascade: true, if_exists: false })), "DROP DATABASE \"lake\"");
        assert_eq!(run(Engine::Bigquery, &ns(NamespaceKind::Schema, "ds", NamespaceOp::Drop { cascade: true, if_exists: true })), "DROP SCHEMA IF EXISTS `ds` CASCADE");
        assert_eq!(
            run(Engine::Cassandra, &ns(NamespaceKind::Database, "ks", create(None, true))),
            "CREATE KEYSPACE IF NOT EXISTS \"ks\" WITH replication = {'class': 'SimpleStrategy', 'replication_factor': 1}"
        );
        assert_eq!(run(Engine::Scylladb, &ns(NamespaceKind::Database, "ks", NamespaceOp::Drop { cascade: false, if_exists: true })), "DROP KEYSPACE IF EXISTS \"ks\"");
        assert!(run(Engine::Sqlite, &ns(NamespaceKind::Database, "x", create(None, false))).starts_with("ERR"));
        assert!(run(Engine::Oracle, &ns(NamespaceKind::Schema, "x", create(None, false))).starts_with("ERR"));
        assert!(run(Engine::Mongodb, &ns(NamespaceKind::Database, "x", create(None, false))).starts_with("ERR MongoDB creates a database"));
        assert_eq!(run(Engine::Mongodb, &ns(NamespaceKind::Database, "x", NamespaceOp::Drop { cascade: false, if_exists: false })), r#"{"dropDatabase":1,"$db":"x"}"#);
        assert!(run(Engine::Postgres, &ns(NamespaceKind::Schema, "", create(None, false))).starts_with("ERR"));
    }
}
