// SOT: query-service, sql-execution, query-namespace, query-total-rows, manual-transactions, batch-execution

use crate::error::{AppError, AppResult};
use crate::guard::destructive::{classify, spans, StatementKind};
use crate::guard::{self, SessionCtx, StatementRequest};
use crate::integrations::{error_detail, shell_meta};
use crate::model::{
    BatchEvent, BatchStatementPlan, BatchStatementResult, BatchStatus, BatchSummary, QueryOutcome, StatementIntent,
    StatementResult, Value,
};
use crate::services::shell_commands;
use crate::state::AppState;
use std::time::{Duration, Instant};

/// The engine's own "make this the default schema" statement, and how many
/// statement results running it will add in front of the script's own.
pub(crate) type Prelude = Option<(String, usize)>;

// WHAT:  Runs the editor's script and reports how many rows it really has.
// WHY:   PRD §4.3 — the editor's schema picker has to change what an unqualified
//        name resolves to, and a row cap that silently reads as the whole answer
//        is worse than no cap at all.
// HOW:   The adapter spells the namespace switch; it is prepended so it lands on
//        the same pooled connection as the script, and its results are dropped
//        before the UI sees them.
// WHERE: src-tauri/src/integrations/mod.rs (`use_namespace`), src-tauri/src/guard/mod.rs
pub async fn execute(ctx: &SessionCtx, sql: &str, max_rows: usize, namespace: Option<&str>) -> AppResult<QueryOutcome> {
    let prelude = namespace
        .map(str::trim)
        .filter(|ns| !ns.is_empty())
        .and_then(|ns| ctx.integration.use_namespace(ns));

    // What the adapter received in front of the script, so a position it reports
    // can be moved back onto the script the editor holds.
    let prefix_chars = prelude.as_ref().map_or(0, |(statement, _)| u32::try_from(statement.chars().count() + 2).unwrap_or(u32::MAX));
    // A script with shell commands (`\dt`, `.tables`, `DESC emp`…) runs those as the
    // catalog queries they stand for, and the rest as typed; any other script is untouched.
    let segments = shell_meta::segments(ctx.connection.engine, sql);
    if segments.iter().any(|segment| matches!(segment, shell_meta::Segment::Meta(_))) {
        let statements = shell_commands::execute(ctx, &prelude, &segments, namespace, max_rows).await?;
        let (rows, truncated) = shell_commands::rows_in_hand(&statements);
        return Ok(QueryOutcome { statements, total_rows: (!truncated).then_some(rows), elapsed_ms: ctx.elapsed_ms() });
    }
    let statements = run(ctx, &prelude, sql, max_rows).await.map_err(|err| error_detail::describe(err, sql, prefix_chars))?;
    let total_rows = count_total(ctx, &prelude, sql, &statements).await;
    Ok(QueryOutcome { statements, total_rows, elapsed_ms: ctx.elapsed_ms() })
}

// ============================================================================
// Batch runs
// ============================================================================

// WHAT:  What a batch run is asked to do.
// WHY:   A script is not all-or-nothing: the editor wants each statement's own
//        status, a choice between carrying on after a failure and stopping at it,
//        and a Stop that works between statements as well as inside one.
pub struct BatchRequest<'a> {
    pub connection_id: &'a str,
    pub sql: &'a str,
    pub confirm_destructive: bool,
    pub max_rows: usize,
    pub namespace: Option<&'a str>,
    /// Names the run so `cancel_query` can stop it.
    pub run_id: &'a str,
    pub stop_on_error: bool,
    /// Cut the script into statements (SQL); false sends the text as one (command languages).
    pub split: bool,
    /// The query tab this run belongs to; its statements share one pinned connection (step 13).
    pub session_key: Option<&'a str>,
}

/// One statement cut out of the script.
struct Planned {
    start: u32,
    end: u32,
    text: String,
    intent: StatementIntent,
}

// WHAT:  Runs a script statement by statement, each one through the block, and
//        streams progress to `emit` as it goes.
// WHY:   DB Pro-style multi-result runs: a failing statement must not hide the
//        results of the others, and each result tab needs its own status and time.
// HOW:   1. The whole script is checked first (read-only lock, destructive
//           confirmation), so a destructive statement in the middle stops the batch
//           before the statements above it have run.
//        2. Every statement then goes through `guard::statement` on its own: its
//           own history row, its own timeout, its own server-side cancel.
//        3. The run is registered under `run_id`. Stop trips that token: the
//           statement in flight is cancelled and the rest are reported skipped.
//        4. After a failure the loop continues, or stops when `stop_on_error`.
//        `emit` gets the plan, then Started / Finished per statement, then Done.
// WHERE: src-tauri/src/commands/query.rs (execute_batch), src-tauri/src/guard/mod.rs
pub async fn execute_batch(state: &AppState, req: &BatchRequest<'_>, emit: &(dyn Fn(BatchEvent) + Send + Sync)) -> AppResult<BatchSummary> {
    let script = req.sql.trim();
    if script.is_empty() {
        return Err(AppError::invalid_input("Nothing to run."));
    }
    let connection = state.with_store(|store| store.get_connection(req.connection_id))?;
    guard::script_gate(&connection, &guard::language::classify_for(connection.engine, script), req.confirm_destructive)?;
    if state.session(req.connection_id).await.is_none() {
        return Err(AppError::not_connected(format!("Not connected to \"{}\".", connection.name)));
    }

    let plan = plan_statements(req.sql, req.split);
    if plan.is_empty() {
        return Err(AppError::invalid_input("Nothing to run."));
    }
    emit(BatchEvent::Planned {
        statements: plan
            .iter()
            .enumerate()
            .map(|(index, p)| BatchStatementPlan { index: index_u32(index), start: p.start, end: p.end, sql: p.text.clone(), intent: p.intent })
            .collect(),
    });

    let started = Instant::now();
    let stop = state.query_runs().register(req.run_id);
    let mut results: Vec<BatchStatementResult> = Vec::with_capacity(plan.len());
    let mut halted = false;
    for (index, statement) in plan.iter().enumerate() {
        let index = index_u32(index);
        if halted || stop.is_cancelled() {
            let skipped = BatchStatementResult { index, status: BatchStatus::Skipped, elapsed_ms: 0, outcome: None, error: None };
            emit(BatchEvent::Finished { result: skipped.clone() });
            results.push(skipped);
            continue;
        }
        emit(BatchEvent::Started { index });
        let result = run_one(state, req, index, statement, &stop).await;
        halted = match result.status {
            BatchStatus::Error => req.stop_on_error,
            BatchStatus::Cancelled => true,
            BatchStatus::Success | BatchStatus::Skipped => false,
        };
        emit(BatchEvent::Finished { result: result.clone() });
        results.push(result);
    }
    state.query_runs().finish(req.run_id);

    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let summary = BatchSummary::of(plan.len(), &results, elapsed_ms);
    emit(BatchEvent::Done { summary: summary.clone() });
    Ok(summary)
}

// WHAT:  One statement of a batch, raced against the batch's Stop token.
async fn run_one(
    state: &AppState,
    req: &BatchRequest<'_>,
    index: u32,
    statement: &Planned,
    stop: &tokio_util::sync::CancellationToken,
) -> BatchStatementResult {
    let began = Instant::now();
    let run_id = format!("{}#{index}", req.run_id);
    let sql = statement.text.clone();
    let namespace = req.namespace.map(str::to_string);
    let max_rows = req.max_rows;
    let work = guard::statement_in_session(
        state,
        req.session_key,
        StatementRequest { connection_id: req.connection_id, sql: &statement.text, confirm_destructive: req.confirm_destructive, run_id: Some(&run_id) },
        |ctx| async move { execute(&ctx, &sql, max_rows, namespace.as_deref()).await },
    );
    tokio::pin!(work);
    let outcome = tokio::select! {
        done = &mut work => done,
        () = stop.cancelled() => {
            // The statement registers its own run id a moment after it starts, so a
            // Stop that lands in that gap is repeated until the statement ends.
            loop {
                state.query_runs().cancel(&run_id);
                if let Ok(done) = tokio::time::timeout(Duration::from_millis(50), &mut work).await {
                    break done;
                }
            }
        }
    };
    let elapsed_ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
    match outcome {
        Ok(outcome) => BatchStatementResult { index, status: BatchStatus::Success, elapsed_ms, outcome: Some(outcome), error: None },
        Err(err @ AppError::Cancelled { .. }) => BatchStatementResult { index, status: BatchStatus::Cancelled, elapsed_ms, outcome: None, error: Some(err) },
        Err(err) => BatchStatementResult { index, status: BatchStatus::Error, elapsed_ms, outcome: None, error: Some(err) },
    }
}

fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

// WHAT:  The statements of a script, each with where it sits in the text.
// HOW:   The block's own splitter (`spans`), so the batch cuts the script exactly
//        where the editor's gutter marks and the guard's classification do. Offsets
//        are UTF-16 units, sliced here the way JavaScript will slice them back.
//        Without `split` the trimmed text is one statement.
fn plan_statements(sql: &str, split: bool) -> Vec<Planned> {
    let units: Vec<u16> = sql.encode_utf16().collect();
    if !split {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let start = sql[..sql.len() - sql.trim_start().len()].encode_utf16().count();
        let end = start + trimmed.encode_utf16().count();
        let intent = guard::script_intent(&classify(trimmed)).unwrap_or(StatementIntent::Read);
        return vec![Planned { start: index_u32(start), end: index_u32(end), text: trimmed.to_string(), intent }];
    }
    spans(sql)
        .into_iter()
        .filter_map(|span| {
            let text = units.get(span.start as usize..span.end as usize).map(String::from_utf16_lossy)?;
            Some(Planned { start: span.start, end: span.end, text, intent: span.intent })
        })
        .collect()
}

pub(crate) async fn run(ctx: &SessionCtx, prelude: &Prelude, sql: &str, max_rows: usize) -> AppResult<Vec<StatementResult>> {
    let Some((statement, produced)) = prelude else {
        return ctx.integration.execute(sql, max_rows).await;
    };
    let mut out = ctx.integration.execute(&format!("{statement};\n{sql}"), max_rows).await?;
    let dropped = (*produced).min(out.len());
    out.drain(..dropped);
    Ok(out)
}

// WHAT:  How many rows the script would return without the row cap.
// HOW:   Nothing truncated -> the rows in hand are the total, for free. Once the
//        cap has hit, only a single SQL read can be counted, by wrapping it; an
//        engine that rejects the wrapper (an ORDER BY inside a derived table)
//        answers None rather than failing the query the user actually ran. The
//        count runs under the same namespace, or an unqualified name in it would
//        resolve somewhere the script's own did not.
async fn count_total(ctx: &SessionCtx, prelude: &Prelude, sql: &str, statements: &[StatementResult]) -> Option<u64> {
    let mut returned = 0_u64;
    let mut truncated = false;
    for statement in statements {
        if let StatementResult::Rows { result } = statement {
            returned += result.row_count();
            truncated |= result.truncated;
        }
    }
    if !truncated {
        return Some(returned);
    }
    if !ctx.integration.capabilities().sql {
        return None;
    }
    if !matches!(classify(sql).as_slice(), [only] if only.kind == StatementKind::Read) {
        return None;
    }
    match run(ctx, prelude, &count_wrapper(sql), 1).await {
        Ok(results) => results.first().and_then(first_count),
        Err(err) => {
            log::debug!("total-row count unavailable: {err}");
            None
        }
    }
}

/// One step of the editor's Manual transaction mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionStep {
    Begin,
    Commit,
    Rollback,
}

// WHAT:  Opens, commits or rolls back the session's manual transaction and
//        answers whether one is open afterwards.
// WHY:   The toolbar redraws from the answer, so it cannot drift from what the
//        adapter holds even when a step fails half-way (a failed COMMIT still
//        ends the transaction).
// WHERE: src-tauri/src/integrations/mod.rs (`begin_transaction` …)
pub async fn transaction(ctx: &SessionCtx, step: TransactionStep) -> AppResult<bool> {
    let integration = &ctx.integration;
    if !integration.manual_transactions() {
        return Err(crate::error::AppError::invalid_input("This engine has no manual transactions."));
    }
    let result = match step {
        TransactionStep::Begin => integration.begin_transaction().await,
        TransactionStep::Commit => integration.commit_transaction().await,
        TransactionStep::Rollback => integration.rollback_transaction().await,
    };
    result.map(|()| integration.in_transaction())
}

/// No `AS` before the alias: Oracle rejects it on a derived table.
fn count_wrapper(sql: &str) -> String {
    let inner = sql.trim().trim_end_matches(';').trim_end();
    format!("SELECT count(*) FROM ({inner}) db_free_total")
}

fn first_count(statement: &StatementResult) -> Option<u64> {
    let StatementResult::Rows { result } = statement else {
        return None;
    };
    match result.rows.first()?.first()? {
        Value::Int(n) => u64::try_from(*n).ok(),
        Value::Decimal(text) | Value::Text(text) => text.parse().ok(),
        Value::Json(json) => json.as_u64(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_wrapper_strips_the_terminator_so_the_derived_table_closes() {
        assert_eq!(count_wrapper("SELECT * FROM t;"), "SELECT count(*) FROM (SELECT * FROM t) db_free_total");
        assert_eq!(count_wrapper("  SELECT * FROM t ;; \n"), "SELECT count(*) FROM (SELECT * FROM t) db_free_total");
    }

    #[test]
    fn only_a_single_read_is_countable() {
        let read = |sql: &str| matches!(classify(sql).as_slice(), [only] if only.kind == StatementKind::Read);
        assert!(read("SELECT * FROM t"));
        assert!(!read("SELECT 1; SELECT 2"), "a script has no one total");
        assert!(!read("UPDATE t SET a = 1"), "a write returns no rows to count");
    }

    #[test]
    fn a_count_is_read_from_whatever_shape_the_adapter_decoded_it_into() {
        let set = |value: Value| StatementResult::Rows {
            result: crate::model::ResultSet { columns: Vec::new(), rows: vec![vec![value]], truncated: false },
        };
        assert_eq!(first_count(&set(Value::Int(42))), Some(42));
        assert_eq!(first_count(&set(Value::Text("42".into()))), Some(42));
        assert_eq!(first_count(&set(Value::Decimal("42".into()))), Some(42));
        assert_eq!(first_count(&set(Value::Json(serde_json::json!(42)))), Some(42));
        assert_eq!(first_count(&set(Value::Int(-1))), None, "a negative count is not a count");
        assert_eq!(first_count(&StatementResult::Affected { rows_affected: 9 }), None);
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::guard::tests::state_with_sqlite;
    use std::sync::Mutex;

    async fn batch(state: &AppState, id: &str, sql: &str, stop_on_error: bool, confirm: bool) -> (AppResult<BatchSummary>, Vec<BatchEvent>) {
        let events = Mutex::new(Vec::new());
        let emit = |event: BatchEvent| {
            if let Ok(mut log) = events.lock() {
                log.push(event);
            }
        };
        let req = BatchRequest { connection_id: id, sql, confirm_destructive: confirm, max_rows: 100, namespace: None, run_id: "run-x", stop_on_error, split: true, session_key: None };
        let result = execute_batch(state, &req, &emit).await;
        (result, events.into_inner().unwrap_or_default())
    }

    fn finished(events: &[BatchEvent]) -> Vec<(u32, BatchStatus)> {
        events
            .iter()
            .filter_map(|e| match e {
                BatchEvent::Finished { result } => Some((result.index, result.status)),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_failing_statement_does_not_hide_the_others() {
        let (state, id) = state_with_sqlite(false).await;
        let sql = "CREATE TABLE t (a int); INSERT INTO t VALUES (1); SELECT * FROM nope; SELECT a FROM t";
        let (summary, events) = batch(&state, &id, sql, false, false).await;
        let summary = summary.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((summary.total, summary.succeeded, summary.failed, summary.skipped), (4, 3, 1, 0));
        assert_eq!(
            finished(&events),
            [(0, BatchStatus::Success), (1, BatchStatus::Success), (2, BatchStatus::Error), (3, BatchStatus::Success)]
        );
        assert!(matches!(events.first(), Some(BatchEvent::Planned { statements }) if statements.len() == 4));
        assert!(matches!(events.last(), Some(BatchEvent::Done { .. })));
        let failure = events.iter().find_map(|e| match e {
            BatchEvent::Finished { result } if result.index == 2 => result.error.clone(),
            _ => None,
        });
        assert!(matches!(failure, Some(AppError::Driver { .. })), "{failure:?}");
        let rows = events.iter().find_map(|e| match e {
            BatchEvent::Finished { result } if result.index == 3 => result.outcome.as_ref().map(QueryOutcome::row_count),
            _ => None,
        });
        assert_eq!(rows, Some(1));
    }

    #[tokio::test]
    async fn every_statement_is_announced_before_it_finishes() {
        let (state, id) = state_with_sqlite(false).await;
        let (_, events) = batch(&state, &id, "SELECT 1; SELECT 2", false, false).await;
        let shape: Vec<String> = events
            .iter()
            .map(|e| match e {
                BatchEvent::Planned { .. } => "planned".to_string(),
                BatchEvent::Started { index } => format!("started {index}"),
                BatchEvent::Finished { result } => format!("finished {}", result.index),
                BatchEvent::Done { .. } => "done".to_string(),
            })
            .collect();
        assert_eq!(shape, ["planned", "started 0", "finished 0", "started 1", "finished 1", "done"]);
    }

    #[tokio::test]
    async fn stop_on_error_skips_what_is_left() {
        let (state, id) = state_with_sqlite(false).await;
        let (summary, events) = batch(&state, &id, "SELECT 1; SELECT * FROM nope; SELECT 3; SELECT 4", true, false).await;
        assert_eq!(finished(&events), [(0, BatchStatus::Success), (1, BatchStatus::Error), (2, BatchStatus::Skipped), (3, BatchStatus::Skipped)]);
        let summary = summary.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((summary.failed, summary.skipped), (1, 2));
    }

    #[tokio::test]
    async fn a_destructive_statement_stops_the_batch_before_anything_runs() {
        let (state, id) = state_with_sqlite(false).await;
        batch(&state, &id, "CREATE TABLE t (a int); INSERT INTO t VALUES (1)", false, false).await.0.unwrap_or_else(|e| panic!("{e}"));
        let (refused, events) = batch(&state, &id, "INSERT INTO t VALUES (2); DELETE FROM t", false, false).await;
        assert!(matches!(refused, Err(AppError::DestructiveConfirmationRequired { ref statements, .. }) if statements.len() == 1), "{refused:?}");
        assert!(events.is_empty(), "nothing is announced for a refused script");
        let (count, _) = batch(&state, &id, "SELECT count(*) FROM t", false, false).await;
        assert!(count.is_ok());
        let (confirmed, events) = batch(&state, &id, "INSERT INTO t VALUES (2); DELETE FROM t", false, true).await;
        assert_eq!(confirmed.map(|s| s.succeeded).unwrap_or_default(), 2);
        assert_eq!(finished(&events).len(), 2);
    }

    #[tokio::test]
    async fn a_read_only_connection_refuses_the_whole_batch() {
        let (state, id) = state_with_sqlite(true).await;
        let (refused, events) = batch(&state, &id, "SELECT 1; INSERT INTO t VALUES (1)", false, false).await;
        assert!(matches!(refused, Err(AppError::ReadOnly { .. })), "{refused:?}");
        assert!(events.is_empty());
    }

    #[tokio::test]
    async fn an_unknown_connection_and_an_empty_script_are_refused() {
        let (state, id) = state_with_sqlite(false).await;
        assert!(matches!(batch(&state, &id, "  \n ", false, false).await.0, Err(AppError::InvalidInput { .. })));
        assert!(matches!(batch(&state, "nope", "SELECT 1", false, false).await.0, Err(AppError::NotFound { .. })));
    }

    #[tokio::test]
    async fn stop_cancels_the_running_statement_and_skips_the_rest() {
        let (state, id) = state_with_sqlite(false).await;
        // Counts to a billion: long enough that only a cancel ends it in time.
        let long = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000000000) SELECT count(*) FROM n";
        let sql = format!("{long}; SELECT 2");
        let events = Mutex::new(Vec::new());
        let emit = |event: BatchEvent| {
            if let Ok(mut log) = events.lock() {
                log.push(event);
            }
        };
        let req = BatchRequest { connection_id: &id, sql: &sql, confirm_destructive: false, max_rows: 10, namespace: None, run_id: "run-stop", stop_on_error: false, split: true, session_key: None };
        let running = execute_batch(&state, &req, &emit);
        let stop = async {
            while !state.query_runs().is_running("run-stop#0") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(state.query_runs().cancel("run-stop"), "the batch itself is registered under its run id");
        };
        let began = Instant::now();
        let (summary, ()) = tokio::join!(running, stop);
        assert!(began.elapsed() < Duration::from_secs(10), "the stop did not end the run");
        let summary = summary.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((summary.cancelled, summary.skipped, summary.succeeded), (1, 1, 0));
        let log = events.into_inner().unwrap_or_default();
        assert_eq!(finished(&log), [(0, BatchStatus::Cancelled), (1, BatchStatus::Skipped)]);
        assert!(!state.query_runs().is_running("run-stop"), "a finished batch leaves the registry");
    }

    #[tokio::test]
    async fn a_failed_statement_reports_where_the_database_pointed() {
        let (state, id) = state_with_sqlite(false).await;
        let (_, events) = batch(&state, &id, "SELECT 1;\nSELECT * FORM t", false, false).await;
        let failure = events.iter().find_map(|e| match e {
            BatchEvent::Finished { result } if result.index == 1 => result.error.clone(),
            _ => None,
        });
        // The position is relative to the statement that was sent ("SELECT * FORM t"): FORM starts at 10.
        assert!(matches!(failure, Some(AppError::Driver { position: Some(10), .. })), "{failure:?}");
    }

    #[test]
    fn statements_are_planned_with_utf16_offsets() {
        let script = "select '\u{1F600}'; \n delete from t where a = 1";
        let plan = plan_statements(script, true);
        assert_eq!(plan.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["select '\u{1F600}'", "delete from t where a = 1"]);
        let units: Vec<u16> = script.encode_utf16().collect();
        for p in &plan {
            assert_eq!(units.get(p.start as usize..p.end as usize).map(String::from_utf16_lossy).as_deref(), Some(p.text.as_str()));
        }
        assert_eq!(plan.first().map(|p| p.intent), Some(StatementIntent::Read));
        assert_eq!(plan.get(1).map(|p| p.intent), Some(StatementIntent::Write));
    }

    #[test]
    fn an_unsplit_script_is_one_trimmed_statement() {
        let plan = plan_statements("  \n{ \"find\": \"users\" }  \n", false);
        assert_eq!(plan.len(), 1);
        let only = &plan[0];
        assert_eq!(only.text, "{ \"find\": \"users\" }");
        assert_eq!((only.start, only.end), (3, 3 + u32::try_from(only.text.encode_utf16().count()).unwrap_or(0)));
        assert!(plan_statements("   ", false).is_empty());
        assert!(plan_statements("  ;  ", true).is_empty());
    }
}
