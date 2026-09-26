// SOT: the-block, guard, request-pipeline, read-only-lock, destructive-guard, query-history-log, page-bounds

use crate::error::{AppError, AppResult};
use crate::model::{ConnectionSummary, HistoryOrigin, HistoryStatus, QueryOutcome};
use crate::state::AppState;
use crate::store::history::NewHistoryEntry;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub mod destructive;

use destructive::{classify, StatementKind};

// ============================================================================
// THE BLOCK
//
// WHAT:  Every Tauri command passes through one of the three entry points below.
// WHY:   Concerns that apply to every request live here once — timing, connection
//        resolution, session lookup, read-only enforcement, destructive-statement
//        confirmation, timeouts, bounds and the history log — so no feature has to
//        restate them and none can quietly drop one.
// HOW:   Steps are numbered and ordered cheapest-rejection-first. Add a concern by
//        inserting a step; never by creating a second guard.
// WHERE: scripts/guardrail.py fails the build if a command skips `guard::`.
//
//   #  step                         rejects when
//   1  request setup                — starts the timer
//   2  connection resolution        connection id unknown
//   3  session lookup               connection not connected
//   4  statement classification     — labels each statement Read/Write/Destructive
//   5  read-only gate               connection is read-only and a Write/Destructive exists
//   6  destructive gate             Destructive present and caller did not confirm
//   7  bounds                       page limit / row cap outside range (clamped, not rejected; an
//                                   absent row cap is "No limit", not a default)
//   8  timeout                      handler exceeds the request's deadline
//   9  cancellation                 the user pressed Stop on this run (`cancel_query`)
//  10  history log                  — records SQL on success and error
//  11  timing enrichment            — elapsed_ms attached to the outcome
//  12  native-tool gate             backup / restore with the engine's own tools
//                                   (pg_dump, mysqldump, …): connection unknown;
//                                   a restore on a read-only connection; a restore
//                                   the caller did not confirm
// ============================================================================

pub const MAX_PAGE_LIMIT: u32 = 1_000;
/// Ceiling for a row cap the caller named. "No limit" is a separate answer, not
/// a bigger number — see `clamp_result_rows`.
pub const MAX_RESULT_ROWS: u32 = 1_000_000;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a stopped run gets to unwind after the adapter's server-side
/// cancel before the block drops it anyway (see step 9).
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);

pub struct SessionCtx {
    pub connection: ConnectionSummary,
    pub integration: Arc<dyn crate::integrations::Integration>,
    pub started: Instant,
}

impl SessionCtx {
    pub fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

// WHAT:  Store-only requests (connections CRUD, buffers, history reads).
pub async fn local<T>(op: &'static str, fut: impl Future<Output = AppResult<T>>) -> AppResult<T> {
    let started = Instant::now(); // 1
    let result = fut.await;
    log::debug!("{op} finished in {:?} ok={}", started.elapsed(), result.is_ok());
    result
}

// WHAT:  Requests against a live database session that run no user-authored SQL.
pub async fn session<T, F, Fut>(state: &AppState, connection_id: &str, handler: F) -> AppResult<T>
where
    F: FnOnce(SessionCtx) -> Fut,
    Fut: Future<Output = AppResult<T>>,
{
    let ctx = resolve(state, connection_id).await?; // 1–3
    tokio::time::timeout(DEFAULT_TIMEOUT, handler(ctx)) // 8
        .await
        .map_err(|_| AppError::timeout("The request timed out."))?
}

pub struct StatementRequest<'a> {
    pub connection_id: &'a str,
    pub sql: &'a str,
    pub confirm_destructive: bool,
    /// Names the run so `cancel_query` can stop it (step 9). None = not stoppable.
    pub run_id: Option<&'a str>,
}

// WHAT:  Requests that execute user-authored SQL. Adds steps 4–6 and 9–11.
pub async fn statement<F, Fut>(
    state: &AppState,
    req: StatementRequest<'_>,
    handler: F,
) -> AppResult<QueryOutcome>
where
    F: FnOnce(SessionCtx) -> Fut,
    Fut: Future<Output = AppResult<QueryOutcome>>,
{
    let ctx = resolve(state, req.connection_id).await?; // 1–3
    let trimmed = req.sql.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_input("Nothing to run."));
    }

    // 4 — classification
    let statements = classify(trimmed);

    // 5 — read-only gate
    if ctx.connection.read_only {
        if let Some(offender) = statements.iter().find(|s| s.kind != StatementKind::Read) {
            return Err(AppError::read_only(format!(
                "This connection is read-only. Blocked: {}",
                preview(&offender.text)
            )));
        }
    }

    // 6 — destructive gate
    let destructive: Vec<String> = statements
        .iter()
        .filter(|s| s.kind == StatementKind::Destructive)
        .map(|s| match &s.reason {
            Some(reason) => format!("{} — {}", preview(&s.text), reason),
            None => preview(&s.text),
        })
        .collect();
    if !destructive.is_empty() && !req.confirm_destructive {
        return Err(AppError::DestructiveConfirmationRequired {
            message: "This script contains destructive statements.".to_string(),
            statements: destructive,
        });
    }

    // 8 — timeout
    let started = ctx.started;
    let connection_id = ctx.connection.id.clone();
    let integration = Arc::clone(&ctx.integration);
    let run_id = req.run_id.map(str::to_string);
    let work = tokio::time::timeout(
        DEFAULT_TIMEOUT,
        crate::integrations::with_run_id(run_id.clone(), handler(ctx)),
    );

    // 9 — cancellation
    // WHAT:  Races the handler against the run's Stop token.
    // WHY:   Without it a runaway query holds the tab (and the server) until the
    //        timeout. Dropping the future alone abandons only the client side, so
    //        the adapter is asked to cancel on the server too.
    // HOW:   The handler is pinned outside the race so it survives it: after a
    //        Stop it gets CANCEL_GRACE to unwind the server-side cancel cleanly
    //        (a pooled connection comes back idle, not mid-statement) before it
    //        is dropped regardless.
    // WHERE: src-tauri/src/state.rs (QueryRuns), integrations/mod.rs (`cancel`)
    let result = match &run_id {
        None => work.await,
        Some(id) => {
            let token = state.query_runs().register(id);
            tokio::pin!(work);
            let outcome = tokio::select! {
                done = &mut work => done,
                () = token.cancelled() => {
                    integration.cancel(id).await;
                    let _ = tokio::time::timeout(CANCEL_GRACE, &mut work).await;
                    Ok(Err(AppError::cancelled("Cancelled by user")))
                }
            };
            state.query_runs().finish(id);
            outcome
        }
    }
    .map_err(|_| AppError::timeout("The query timed out."))
    .and_then(|inner| inner);

    // 10 — history log (success and error alike)
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (status, error, row_count) = match &result {
        Ok(outcome) => (HistoryStatus::Ok, None, Some(outcome.row_count())),
        Err(err) => (HistoryStatus::Error, Some(err.message().to_string()), None),
    };
    let logged = state.with_store(|store| {
        store.insert_history(&NewHistoryEntry {
            connection_id: &connection_id,
            sql: trimmed,
            status,
            origin: HistoryOrigin::User,
            error: error.as_deref(),
            elapsed_ms,
            row_count,
        })
    });
    if let Err(err) = logged {
        log::warn!("history log failed: {err}");
    }

    // 11 — timing enrichment
    result.map(|mut outcome| {
        outcome.elapsed_ms = elapsed_ms;
        outcome
    })
}

// 7 — bounds. Clamping (not rejecting) keeps the grid usable if a caller over-asks.
pub fn clamp_page_limit(limit: u32) -> u32 {
    limit.clamp(1, MAX_PAGE_LIMIT)
}

// WHAT:  The row cap one Run honours. `None` is the editor's "No limit": the
//        caller asked for the whole result and gets it, so this is a ceiling to
//        clamp to rather than a value to invent.
pub fn clamp_result_rows(max_rows: Option<u32>) -> usize {
    match max_rows {
        Some(rows) => rows.clamp(1, MAX_RESULT_ROWS) as usize,
        None => usize::MAX,
    }
}

// WHAT:  What a native-tool run does to the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAccess {
    /// A backup: reads only.
    Read,
    /// A restore: replaces data. `confirmed` is the user's explicit yes.
    Replace { confirmed: bool },
}

// WHAT:  Step 12 — the gate for backup / restore runs.
// WHY:   A restore runs no SQL the classifier could read (it is pg_restore or
//        mysql fed a file), yet it is the most destructive thing the app can
//        do. It gets the same two answers as a destructive statement: never on
//        a read-only connection, never without an explicit confirmation — and
//        the confirmation error names the target, so the UI can say exactly
//        what is about to be overwritten.
// HOW:   Called inside `guard::local` by the backup commands. It resolves the
//        saved connection (step 2) but not a session: the tools open their own
//        connections, and file engines must be restored while disconnected.
// WHERE: src-tauri/src/commands/backup.rs, src-tauri/src/integrations/native_tools.rs
pub fn native_tool(state: &AppState, connection_id: &str, access: ToolAccess) -> AppResult<ConnectionSummary> {
    let connection = state.with_store(|store| store.get_connection(connection_id))?; // 2
    if let ToolAccess::Replace { confirmed } = access {
        let target = connection
            .database
            .as_deref()
            .or(connection.file_path.as_deref())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or("the default database");
        if connection.read_only {
            return Err(AppError::read_only(format!(
                "\"{}\" is read-only. Restoring would overwrite {target}.",
                connection.name
            )));
        }
        if !confirmed {
            return Err(AppError::DestructiveConfirmationRequired {
                message: format!("Restoring replaces data in \"{}\".", connection.name),
                statements: vec![format!("restore into {target}")],
            });
        }
    }
    Ok(connection)
}

async fn resolve(state: &AppState, connection_id: &str) -> AppResult<SessionCtx> {
    let started = Instant::now(); // 1
    let connection = state.with_store(|store| store.get_connection(connection_id))?; // 2
    let integration = state
        .session(connection_id)
        .await
        .ok_or_else(|| AppError::not_connected(format!("Not connected to \"{}\".", connection.name)))?; // 3
    Ok(SessionCtx { connection, integration, started })
}

fn preview(sql: &str) -> String {
    let flat: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 80 {
        let head: String = flat.chars().take(77).collect();
        format!("{head}...")
    } else {
        flat
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::{ConnectionInput, Engine, Environment, SslMode};
    use crate::store::Store;

    async fn state_with_sqlite(read_only: bool) -> (AppState, String) {
        let dir = std::env::temp_dir().join(format!("db-free-guard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let path = dir.join("g.db").to_string_lossy().into_owned();
        // A zero-byte file is a valid empty SQLite database; read-only open needs it to exist.
        std::fs::File::create(&path).unwrap_or_else(|e| panic!("{e}"));
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let input = ConnectionInput {
            name: "guarded".into(),
            engine: Engine::Sqlite,
            environment: Environment::Production,
            read_only,
            host: None,
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: Some(path),
            ssl_mode: SslMode::Disable,
            ssh: crate::model::SshTunnel::default(),
            ssh_secret: None,
            folder: None,
            color: None,
            favorite: false,
        };
        let summary = store.insert_connection(&input, None).unwrap_or_else(|e| panic!("{e}"));
        let state = AppState::new(store, Box::new(MemoryKeyProvider::default()));
        let resolved = crate::model::ResolvedConnection { summary: summary.clone(), secret: None };
        let integration = crate::integrations::connect(&resolved).await.unwrap_or_else(|e| panic!("{e}"));
        state.insert_session(summary.id.clone(), integration).await;
        (state, summary.id)
    }

    async fn run(state: &AppState, id: &str, sql: &str, confirm: bool) -> AppResult<QueryOutcome> {
        statement(
            state,
            StatementRequest { connection_id: id, sql, confirm_destructive: confirm, run_id: None },
            |ctx| async move {
                let statements = ctx.integration.execute(sql, 10).await?;
                Ok(QueryOutcome { statements, total_rows: None, elapsed_ms: 0 })
            },
        )
        .await
    }

    #[tokio::test]
    async fn stop_cancels_a_running_statement_and_logs_it() {
        let (state, id) = state_with_sqlite(false).await;
        // Counts to a billion: long enough that only a cancel ends it in time.
        let sql = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000000000) SELECT count(*) FROM n";
        let running = statement(
            &state,
            StatementRequest { connection_id: &id, sql, confirm_destructive: false, run_id: Some("run-1") },
            |ctx| async move {
                let statements = ctx.integration.execute(sql, 10).await?;
                Ok(QueryOutcome { statements, total_rows: None, elapsed_ms: 0 })
            },
        );
        let stop = async {
            while !state.query_runs().is_running("run-1") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(state.query_runs().cancel("run-1"));
        };
        let started = Instant::now();
        let (result, ()) = tokio::join!(running, stop);
        assert!(matches!(result, Err(AppError::Cancelled { .. })), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(10), "the stop did not end the run");
        assert!(!state.query_runs().is_running("run-1"), "a finished run leaves the registry");
        let history = state.with_store(|s| s.list_history(Some(&id), None, 10)).unwrap_or_default();
        assert_eq!(history.first().and_then(|h| h.error.clone()).as_deref(), Some("Cancelled by user"));
    }

    #[tokio::test]
    async fn unknown_connection_is_not_found() {
        let (state, _) = state_with_sqlite(false).await;
        let err = session(&state, "nope", |_| async { Ok(()) }).await.err();
        assert!(matches!(err, Some(AppError::NotFound { .. })));
    }

    #[tokio::test]
    async fn read_only_blocks_writes_and_logs_history() {
        let (state, id) = state_with_sqlite(true).await;
        assert!(run(&state, &id, "SELECT 1", false).await.is_ok());
        let err = run(&state, &id, "CREATE TABLE t (a int)", false).await.err();
        assert!(matches!(err, Some(AppError::ReadOnly { .. })));
        let history = state.with_store(|s| s.list_history(Some(&id), None, 10)).unwrap_or_default();
        assert_eq!(history.len(), 1, "read-only rejections happen before execution and are not logged");
    }

    #[tokio::test]
    async fn destructive_requires_confirmation() {
        let (state, id) = state_with_sqlite(false).await;
        run(&state, &id, "CREATE TABLE t (a int)", false).await.unwrap_or_else(|e| panic!("{e}"));
        let err = run(&state, &id, "DELETE FROM t", false).await.err();
        assert!(matches!(err, Some(AppError::DestructiveConfirmationRequired { ref statements, .. }) if statements.len() == 1));
        assert!(run(&state, &id, "DELETE FROM t", true).await.is_ok());
    }

    #[tokio::test]
    async fn errors_are_logged_with_status_error() {
        let (state, id) = state_with_sqlite(false).await;
        assert!(run(&state, &id, "SELECT * FROM missing_table", false).await.is_err());
        let history = state.with_store(|s| s.list_history(Some(&id), None, 10)).unwrap_or_default();
        assert_eq!(history.first().map(|h| h.status), Some(HistoryStatus::Error));
    }

    #[tokio::test]
    async fn native_restore_is_gated() {
        let (state, id) = state_with_sqlite(false).await;
        assert!(native_tool(&state, &id, ToolAccess::Read).is_ok());
        let err = native_tool(&state, &id, ToolAccess::Replace { confirmed: false }).err();
        assert!(matches!(err, Some(AppError::DestructiveConfirmationRequired { ref statements, .. }) if statements.len() == 1));
        assert!(native_tool(&state, &id, ToolAccess::Replace { confirmed: true }).is_ok());
        assert!(matches!(native_tool(&state, "nope", ToolAccess::Read).err(), Some(AppError::NotFound { .. })));

        let (locked, locked_id) = state_with_sqlite(true).await;
        assert!(native_tool(&locked, &locked_id, ToolAccess::Read).is_ok(), "a backup is a read");
        let err = native_tool(&locked, &locked_id, ToolAccess::Replace { confirmed: true }).err();
        assert!(matches!(err, Some(AppError::ReadOnly { .. })), "confirming does not unlock a read-only connection");
    }

    #[test]
    fn bounds_clamp() {
        assert_eq!(clamp_page_limit(0), 1);
        assert_eq!(clamp_page_limit(5_000), MAX_PAGE_LIMIT);
        assert_eq!(clamp_result_rows(None), usize::MAX, "None is \"No limit\", not a default");
        assert_eq!(clamp_result_rows(Some(0)), 1);
        assert_eq!(clamp_result_rows(Some(u32::MAX)), MAX_RESULT_ROWS as usize);
    }
}
