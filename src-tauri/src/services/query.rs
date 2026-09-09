// SOT: query-service, sql-execution, query-namespace, query-total-rows

use crate::error::AppResult;
use crate::guard::destructive::{classify, StatementKind};
use crate::guard::SessionCtx;
use crate::model::{QueryOutcome, StatementResult, Value};

/// The engine's own "make this the default schema" statement, and how many
/// statement results running it will add in front of the script's own.
type Prelude = Option<(String, usize)>;

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

    let statements = run(ctx, &prelude, sql, max_rows).await?;
    let total_rows = count_total(ctx, &prelude, sql, &statements).await;
    Ok(QueryOutcome { statements, total_rows, elapsed_ms: ctx.elapsed_ms() })
}

async fn run(ctx: &SessionCtx, prelude: &Prelude, sql: &str, max_rows: usize) -> AppResult<Vec<StatementResult>> {
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
