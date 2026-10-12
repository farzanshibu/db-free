// SOT: pooling-live-tests, pooled-connection-hygiene-tests, pinned-session-live-tests

use crate::integrations::pooling::with_session_key;
use crate::integrations::{current_run_id, with_run_id, Integration};
use crate::model::{ConnectionInput, ConnectionSummary, ResolvedConnection, StatementResult, Value};
use std::sync::Arc;
use std::time::Duration;

// WHAT:  Live checks of the engine-robustness work against a real server: a pooled
//        connection never goes back inside a failed or open transaction (B1), a
//        query tab's session keeps its state and survives its own errors (N1),
//        and a dropped session is replaced rather than reused (N1, dead-session
//        detection).
// WHY:   The unit tests prove the scanner and the registry; only a server proves
//        that the connection really comes back clean, because sqlx hides the
//        server's transaction status.
// HOW:   Skipped unless DB_FREE_PG_HOST is set (and DB_FREE_MYSQL_HOST for the
//        MySQL ones): DB_FREE_PG_HOST / PORT / USER / PASSWORD / DB. Nothing is
//        written: every check is a SET, a transaction that is rolled back, or a
//        pg_sleep.
// WHERE: src-tauri/src/integrations/{postgres,mysql}.rs, pooling.rs

fn input(engine: &str, host: String, port: Option<u16>, database: Option<String>, user: Option<String>) -> Option<ConnectionInput> {
    serde_json::from_value(serde_json::json!({
        "name": "live",
        "engine": engine,
        "environment": "local",
        "readOnly": false,
        "host": host,
        "port": port,
        "database": database,
        "username": user,
        "password": null,
        "filePath": null,
        "sslMode": "prefer",
        "ssh": serde_json::to_value(crate::model::SshTunnel::default()).ok()?,
        "sshSecret": null,
        "folder": null,
        "color": null,
        "favorite": false,
        "extra": {}
    }))
    .ok()
}

async fn live_postgres() -> Option<Arc<dyn Integration>> {
    let host = std::env::var("DB_FREE_PG_HOST").ok()?;
    let port = std::env::var("DB_FREE_PG_PORT").ok().and_then(|p| p.parse().ok());
    let input = input("postgres", host, port, std::env::var("DB_FREE_PG_DB").ok(), std::env::var("DB_FREE_PG_USER").ok())?;
    let resolved = ResolvedConnection { summary: ConnectionSummary::draft(&input, true), secret: std::env::var("DB_FREE_PG_PASSWORD").ok() };
    crate::integrations::postgres::connect(&resolved).await.ok()
}

async fn live_mysql() -> Option<Arc<dyn Integration>> {
    let host = std::env::var("DB_FREE_MYSQL_HOST").ok()?;
    let port = std::env::var("DB_FREE_MYSQL_PORT").ok().and_then(|p| p.parse().ok());
    let input = input("mysql", host, port, std::env::var("DB_FREE_MYSQL_DB").ok(), std::env::var("DB_FREE_MYSQL_USER").ok())?;
    let resolved = ResolvedConnection { summary: ConnectionSummary::draft(&input, true), secret: std::env::var("DB_FREE_MYSQL_PASSWORD").ok() };
    crate::integrations::mysql::connect(&resolved).await.ok()
}

/// The first cell of the last result set, as text.
async fn scalar(db: &Arc<dyn Integration>, sql: &str) -> Result<String, String> {
    let results = db.execute(sql, 10).await.map_err(|e| e.message().to_string())?;
    for result in results.iter().rev() {
        if let StatementResult::Rows { result } = result {
            return Ok(match result.rows.first().and_then(|r| r.first()) {
                Some(Value::Text(t)) | Some(Value::Decimal(t)) | Some(Value::DateTime(t)) => t.clone(),
                Some(Value::Int(i)) => i.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Null) | None => "NULL".to_string(),
                Some(other) => format!("{other:?}"),
            });
        }
    }
    Err("no rows".to_string())
}

/// `scalar` as the query tab named `tab`.
async fn scalar_in(db: &Arc<dyn Integration>, tab: &str, sql: &str) -> Result<String, String> {
    with_session_key(Some(tab.to_string()), scalar(db, sql)).await
}

// Postgres: `now()` is the transaction's start and `statement_timestamp()` the
// statement's, so they differ exactly when the statement runs inside a transaction
// that began earlier.
const PG_FRESH: &str = "SELECT now() = statement_timestamp()";

#[tokio::test]
async fn postgres_a_failed_transaction_never_reaches_the_next_request() {
    let Some(db) = live_postgres().await else { return };
    let failed = db.execute("BEGIN; SELECT 1/0;", 10).await;
    assert!(failed.is_err(), "the division fails inside the transaction");
    // Before the fix the very next request drew the same connection and died with
    // "current transaction is aborted"; with a pool of four, ten tries cover every connection.
    for round in 0..10 {
        assert_eq!(scalar(&db, "SELECT 1").await, Ok("1".to_string()), "request {round} ran on a clean connection");
    }
    db.close().await;
}

#[tokio::test]
async fn postgres_an_unfinished_transaction_is_rolled_back_not_pooled() {
    let Some(db) = live_postgres().await else { return };
    db.execute("BEGIN", 10).await.unwrap_or_else(|e| panic!("{e}"));
    for round in 0..10 {
        assert_eq!(scalar(&db, PG_FRESH).await, Ok("true".to_string()), "request {round} is outside the abandoned transaction");
    }
    // A balanced script is untouched by all this.
    assert_eq!(scalar(&db, "BEGIN; SELECT 41 + 1; COMMIT;").await, Ok("42".to_string()));
    db.close().await;
}

#[tokio::test]
async fn postgres_a_set_does_not_leak_between_requests() {
    let Some(db) = live_postgres().await else { return };
    let default = scalar(&db, "SHOW search_path").await.unwrap_or_default();
    db.execute("SET search_path TO pg_catalog", 10).await.unwrap_or_else(|e| panic!("{e}"));
    for round in 0..10 {
        assert_eq!(scalar(&db, "SHOW search_path").await, Ok(default.clone()), "request {round} sees the default search_path");
    }
    db.close().await;
}

// The reset a pooled connection gets after a SET (`DISCARD ALL`) also deallocates
// the server's prepared statements, which sqlx still counts as cached on that
// connection: its next parameterised query would Bind a name the server has forgotten.
// `ping` is one persistent statement, so every connection of the pool holds it,
// and a few rounds reach the connection that was reset. The pauses let the warm
// connection be back in the pool when the SET draws it (a fresh one has nothing
// cached and would hide the fault).
#[tokio::test]
async fn postgres_a_reset_does_not_break_the_cached_statements() {
    let Some(db) = live_postgres().await else { return };
    for round in 0..6 {
        db.ping().await.unwrap_or_else(|e| panic!("ping {round} before the reset: {e}"));
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    db.execute("SET search_path TO pg_catalog", 10).await.unwrap_or_else(|e| panic!("{e}"));
    tokio::time::sleep(Duration::from_millis(300)).await;
    for round in 0..8 {
        db.ping().await.unwrap_or_else(|e| panic!("ping {round} after the reset: {e}"));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    db.catalog().await.unwrap_or_else(|e| panic!("catalog after the reset: {e}"));
    db.close().await;
}

#[tokio::test]
async fn postgres_a_stopped_transaction_does_not_come_back_dirty() {
    let Some(db) = live_postgres().await else { return };
    // The future is dropped mid-run, as the block does on Stop or a timeout.
    let long = db.execute("BEGIN; SELECT pg_sleep(5);", 10);
    let _ = tokio::time::timeout(Duration::from_millis(400), long).await;
    for round in 0..10 {
        assert_eq!(scalar(&db, PG_FRESH).await, Ok("true".to_string()), "request {round} after the stop");
    }
    db.close().await;
}

#[tokio::test]
async fn postgres_a_tab_keeps_its_session_between_runs() {
    let Some(db) = live_postgres().await else { return };
    let default = scalar(&db, "SHOW search_path").await.unwrap_or_default();

    scalar_in(&db, "tab-a", "SET search_path TO pg_catalog").await.unwrap_or_default();
    assert_eq!(scalar_in(&db, "tab-a", "SHOW search_path").await, Ok("pg_catalog".to_string()), "SET persists in the tab");
    assert_eq!(scalar_in(&db, "tab-b", "SHOW search_path").await, Ok(default.clone()), "another tab has its own session");
    assert_eq!(scalar(&db, "SHOW search_path").await, Ok(default), "and so does the shared pool");

    let pid = scalar_in(&db, "tab-a", "SELECT pg_backend_pid()").await.unwrap_or_default();
    assert_eq!(scalar_in(&db, "tab-a", "SELECT pg_backend_pid()").await, Ok(pid.clone()), "one backend for the life of the tab");
    assert_ne!(scalar_in(&db, "tab-b", "SELECT pg_backend_pid()").await, Ok(pid.clone()));

    // BEGIN persists, and a failed statement leaves the transaction aborted until ROLLBACK, as in psql.
    scalar_in(&db, "tab-a", "BEGIN").await.unwrap_or_default();
    assert_eq!(scalar_in(&db, "tab-a", PG_FRESH).await, Ok("false".to_string()), "still inside the transaction");
    assert!(scalar_in(&db, "tab-a", "SELECT 1/0").await.is_err());
    let aborted = scalar_in(&db, "tab-a", "SELECT 1").await.err().unwrap_or_default();
    assert!(aborted.contains("aborted"), "{aborted}");
    scalar_in(&db, "tab-a", "ROLLBACK").await.unwrap_or_default();
    assert_eq!(scalar_in(&db, "tab-a", PG_FRESH).await, Ok("true".to_string()));
    assert_eq!(scalar_in(&db, "tab-a", "SELECT pg_backend_pid()").await, Ok(pid), "the error did not cost the tab its session");

    db.close().await;
}

#[tokio::test]
async fn postgres_closing_a_tab_closes_its_backend() {
    let Some(db) = live_postgres().await else { return };
    let pid = scalar_in(&db, "tab-c", "SELECT pg_backend_pid()").await.unwrap_or_default();
    let alive = |pid: String| format!("SELECT count(*) FROM pg_stat_activity WHERE pid = {pid}");
    assert_eq!(scalar(&db, &alive(pid.clone())).await, Ok("1".to_string()));
    db.close_session("tab-c").await;
    let mut gone = false;
    for _ in 0..40 {
        if scalar(&db, &alive(pid.clone())).await == Ok("0".to_string()) {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(gone, "backend {pid} should have been released with its tab");
    let fresh = scalar_in(&db, "tab-c", "SELECT pg_backend_pid()").await.unwrap_or_default();
    assert_ne!(fresh, pid, "the tab's next run opens a new session");
    db.close().await;
}

#[tokio::test]
async fn postgres_a_session_the_server_killed_is_reported_then_replaced() {
    let Some(db) = live_postgres().await else { return };
    scalar_in(&db, "tab-d", "SET search_path TO pg_catalog").await.unwrap_or_default();
    let pid = scalar_in(&db, "tab-d", "SELECT pg_backend_pid()").await.unwrap_or_default();
    assert_eq!(scalar(&db, &format!("SELECT pg_terminate_backend({pid})")).await, Ok("true".to_string()));
    tokio::time::sleep(Duration::from_millis(200)).await;

    let lost = scalar_in(&db, "tab-d", "SELECT 1").await.err().unwrap_or_default();
    assert!(lost.contains("lost") || lost.contains("closed"), "the user is told the session died: {lost}");
    let fresh = scalar_in(&db, "tab-d", "SELECT pg_backend_pid()").await.unwrap_or_default();
    assert!(!fresh.is_empty() && fresh != pid, "the next run reconnects on a new backend");
    db.close().await;
}

#[tokio::test]
async fn postgres_stop_cancels_the_pinned_backend_and_keeps_the_session() {
    let Some(db) = live_postgres().await else { return };
    let pid = scalar_in(&db, "tab-e", "SELECT pg_backend_pid()").await.unwrap_or_default();

    let running = {
        let db = Arc::clone(&db);
        tokio::spawn(async move {
            with_session_key(Some("tab-e".into()), with_run_id(Some("run-1".into()), async {
                assert_eq!(current_run_id().as_deref(), Some("run-1"));
                db.execute("SELECT pg_sleep(30)", 10).await
            }))
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    db.cancel("run-1").await;
    let outcome = tokio::time::timeout(Duration::from_secs(10), running).await;
    let stopped = matches!(outcome, Ok(Ok(Err(ref e))) if e.message().contains("cancel"));
    assert!(stopped, "the sleep was cancelled server-side: {outcome:?}");
    assert_eq!(scalar_in(&db, "tab-e", "SELECT pg_backend_pid()").await, Ok(pid), "a Stop is not a dead session");
    db.close().await;
}

#[tokio::test]
async fn postgres_behind_a_pooler_the_catalog_still_loads() {
    let Ok(port) = std::env::var("DB_FREE_PGBOUNCER_PORT") else { return };
    let Some(host) = std::env::var("DB_FREE_PG_HOST").ok() else { return };
    let Some(input) = input(
        "postgres",
        host,
        port.parse().ok(),
        std::env::var("DB_FREE_PGBOUNCER_DB").ok(),
        std::env::var("DB_FREE_PGBOUNCER_USER").ok(),
    ) else {
        return;
    };
    let resolved = ResolvedConnection { summary: ConnectionSummary::draft(&input, true), secret: std::env::var("DB_FREE_PGBOUNCER_PASSWORD").ok() };
    let db = crate::integrations::postgres::connect(&resolved).await.unwrap_or_else(|e| panic!("connect through the pooler: {e}"));
    for _ in 0..3 {
        let catalog = db.catalog().await.unwrap_or_else(|e| panic!("catalog through the pooler: {e}"));
        assert!(!catalog.schemas.is_empty());
    }
    db.close().await;
}

#[tokio::test]
async fn mysql_an_open_transaction_and_a_use_do_not_reach_the_next_request() {
    let Some(db) = live_mysql().await else { return };
    let home = scalar(&db, "SELECT DATABASE()").await.unwrap_or_default();
    let in_transaction = "SELECT COUNT(*) FROM information_schema.innodb_trx WHERE trx_mysql_thread_id = CONNECTION_ID()";
    db.execute("START TRANSACTION WITH CONSISTENT SNAPSHOT", 10).await.unwrap_or_else(|e| panic!("{e}"));
    for round in 0..10 {
        assert_eq!(scalar(&db, in_transaction).await, Ok("0".to_string()), "request {round} is outside the abandoned transaction");
    }
    db.execute("USE information_schema", 10).await.unwrap_or_else(|e| panic!("{e}"));
    for round in 0..10 {
        assert_eq!(scalar(&db, "SELECT DATABASE()").await, Ok(home.clone()), "request {round} is back in the home database");
    }
    db.close().await;
}

#[tokio::test]
async fn mysql_a_tab_keeps_user_variables_between_runs() {
    let Some(db) = live_mysql().await else { return };
    scalar_in(&db, "tab-a", "SET @dbfree_probe = 41").await.unwrap_or_default();
    assert_eq!(scalar_in(&db, "tab-a", "SELECT @dbfree_probe + 1").await, Ok("42".to_string()));
    assert_eq!(scalar_in(&db, "tab-b", "SELECT @dbfree_probe").await, Ok("NULL".to_string()), "another tab does not see it");
    assert_eq!(scalar(&db, "SELECT @dbfree_probe").await, Ok("NULL".to_string()), "nor does the pool");
    db.close().await;
}
