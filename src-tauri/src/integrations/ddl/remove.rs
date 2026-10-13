// SOT: truncate-table-sql, drop-table-sql, drop-view-sql

use super::dialect::{check_name, Cx, Dialect};
use super::mongo;
use crate::error::{AppError, AppResult};
use crate::model::{RemoveAction, RemoveSpec, TableKind};

// WHAT:  TRUNCATE and DROP for tables, and DROP for views.
// WHY:   "Empty" and "Delete" are the two destructive buttons in the sidebar; the
//        statement behind them differs per engine (SQLite has no TRUNCATE,
//        Oracle needs CASCADE CONSTRAINTS, ClickHouse and Snowflake take IF
//        EXISTS on TRUNCATE) and a view must never be sent DROP TABLE.
// HOW:   CASCADE is written only where the dialect has it; elsewhere the flag is
//        ignored, because the engine already refuses (or already ignores) it.
// WHERE: src-tauri/src/integrations/ddl/mod.rs

pub fn remove_table(cx: &Cx, spec: &RemoveSpec) -> AppResult<Vec<String>> {
    if cx.dialect == Dialect::Mongo {
        return mongo::remove(spec);
    }
    check_name("table", &spec.table.name)?;
    let target = cx.table(&spec.table);
    let is_view = spec.kind == TableKind::View;
    let if_exists = if spec.if_exists { "IF EXISTS " } else { "" };
    let statement = match spec.action {
        RemoveAction::Truncate => {
            if is_view {
                return Err(AppError::invalid_input("A view has no rows of its own to empty; drop it instead."));
            }
            match cx.dialect {
                Dialect::Sqlite => format!("DELETE FROM {target}"),
                Dialect::Postgres | Dialect::Oracle => format!("TRUNCATE TABLE {target}{}", if spec.cascade { " CASCADE" } else { "" }),
                Dialect::Clickhouse | Dialect::Snowflake => format!("TRUNCATE TABLE {if_exists}{target}"),
                _ => format!("TRUNCATE TABLE {target}"),
            }
        }
        RemoveAction::Drop => {
            if spec.if_exists && cx.dialect == Dialect::Oracle {
                return Err(AppError::invalid_input("Oracle before 23ai has no DROP … IF EXISTS; untick \"if exists\"."));
            }
            let noun = match (is_view, cx.dialect) {
                (false, _) => "TABLE",
                (true, Dialect::Cassandra) => "MATERIALIZED VIEW",
                (true, _) => "VIEW",
            };
            let tail = match cx.dialect {
                Dialect::Postgres | Dialect::Duckdb | Dialect::Snowflake if spec.cascade => " CASCADE",
                Dialect::Oracle if spec.cascade && !is_view => " CASCADE CONSTRAINTS",
                _ => "",
            };
            format!("DROP {noun} {if_exists}{target}{tail}")
        }
    };
    Ok(vec![statement])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Engine, TableRef};

    fn spec(action: RemoveAction, kind: TableKind) -> RemoveSpec {
        RemoveSpec { table: TableRef { schema: Some("app".into()), name: "events".into() }, kind, action, cascade: false, if_exists: false }
    }

    fn run(engine: Engine, spec: &RemoveSpec) -> String {
        super::super::remove_table(engine, spec).map(|s| s.join(";")).unwrap_or_else(|e| format!("ERR {e}"))
    }

    #[test]
    fn truncate_is_spelled_per_dialect() {
        let t = spec(RemoveAction::Truncate, TableKind::Table);
        assert_eq!(run(Engine::Postgres, &t), "TRUNCATE TABLE \"app\".\"events\"");
        assert_eq!(run(Engine::Mysql, &RemoveSpec { table: TableRef { schema: None, name: "e".into() }, ..t.clone() }), "TRUNCATE TABLE `e`");
        assert_eq!(run(Engine::Mssql, &t), "TRUNCATE TABLE [app].[events]");
        assert_eq!(run(Engine::Sqlite, &RemoveSpec { table: TableRef { schema: None, name: "e".into() }, ..t.clone() }), "DELETE FROM \"e\"");
        assert_eq!(run(Engine::Postgres, &RemoveSpec { cascade: true, ..t.clone() }), "TRUNCATE TABLE \"app\".\"events\" CASCADE");
        assert_eq!(run(Engine::Clickhouse, &RemoveSpec { if_exists: true, ..t.clone() }), "TRUNCATE TABLE IF EXISTS \"app\".\"events\"");
        assert_eq!(run(Engine::Cassandra, &t), "TRUNCATE TABLE \"app\".\"events\"");
        assert!(run(Engine::Postgres, &spec(RemoveAction::Truncate, TableKind::View)).starts_with("ERR"));
    }

    #[test]
    fn drop_picks_table_or_view_and_the_cascade_the_engine_has() {
        let table = spec(RemoveAction::Drop, TableKind::Table);
        let view = spec(RemoveAction::Drop, TableKind::View);
        assert_eq!(run(Engine::Postgres, &table), "DROP TABLE \"app\".\"events\"");
        assert_eq!(run(Engine::Postgres, &view), "DROP VIEW \"app\".\"events\"");
        assert_eq!(run(Engine::Postgres, &RemoveSpec { cascade: true, if_exists: true, ..view.clone() }), "DROP VIEW IF EXISTS \"app\".\"events\" CASCADE");
        assert_eq!(run(Engine::Mysql, &RemoveSpec { cascade: true, ..table.clone() }), "DROP TABLE `app`.`events`", "MySQL has no CASCADE");
        assert_eq!(run(Engine::Oracle, &RemoveSpec { cascade: true, ..table.clone() }), "DROP TABLE \"app\".\"events\" CASCADE CONSTRAINTS");
        assert!(run(Engine::Oracle, &RemoveSpec { if_exists: true, ..table.clone() }).starts_with("ERR"));
        assert_eq!(run(Engine::Bigquery, &view), "DROP VIEW `app`.`events`");
        assert_eq!(run(Engine::Cassandra, &view), "DROP MATERIALIZED VIEW \"app\".\"events\"");
        assert_eq!(run(Engine::CloudflareD1, &RemoveSpec { if_exists: true, ..table.clone() }), "DROP TABLE IF EXISTS \"app\".\"events\"");
        assert!(run(Engine::Redis, &table).starts_with("ERR"));
        assert_eq!(run(Engine::Mongodb, &table), r#"{"drop":"events","$db":"app"}"#);
    }
}
