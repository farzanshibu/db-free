// SOT: workflow-query-node, workflow-insert-node, guarded-workflow-statement, destructive-opt-in

use super::{NodeCtx, NodeFailure, NodeOutput, NodeResult};
use crate::error::{AppError, AppResult};
use crate::guard::{self, SessionCtx, StatementRequest};
use crate::integrations::{qualified_name_for, quote_ident_for};
use crate::model::workflow::{InsertConfig, QueryConfig};
use crate::model::{Engine, QueryOutcome, StatementResult, TableRef, Value};
use crate::services::workflows::data::Dataset;
use crate::services::workflows::template;
use crate::services::{connection, query};
use std::future::Future;
use std::sync::atomic::Ordering;

/// Rows a Query node keeps from one statement; a result cut here says so.
const ROW_LIMIT: u32 = guard::MAX_RESULT_ROWS;

fn connection_for(ctx: &NodeCtx<'_>, chosen: Option<&String>) -> Result<String, NodeFailure> {
    chosen
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .or(ctx.workflow_connection)
        .map(str::to_string)
        .ok_or_else(|| NodeFailure::new("No connection set for this node. Pick one in the inspector or set the workflow's connection."))
}

// WHAT:  Runs SQL through the guard on behalf of a node, and stops it on Stop.
// WHY:   Workflow SQL gets exactly the protection the editor's SQL gets: the
//        read-only lock, the destructive-statement confirmation, the timeout and
//        the history log. Nothing here can opt out of them; `confirm_destructive`
//        is the node's own saved opt-in and no one else's.
// HOW:   The statement registers under a name unique to this node and pass, so
//        Stop reaches the server (`cancel_query` semantics) and not only the client.
async fn guarded<F, Fut>(ctx: &NodeCtx<'_>, connection_id: &str, sql: &str, confirm_destructive: bool, handler: F) -> AppResult<QueryOutcome>
where
    F: FnOnce(SessionCtx) -> Fut,
    Fut: Future<Output = AppResult<QueryOutcome>>,
{
    let name = format!("wf:{}:{}:{}", ctx.run_id, ctx.node.id, ctx.sequence.fetch_add(1, Ordering::Relaxed));
    let statement = guard::statement(
        ctx.state,
        StatementRequest { connection_id, sql, confirm_destructive, run_id: Some(&name) },
        handler,
    );
    tokio::pin!(statement);
    tokio::select! {
        outcome = &mut statement => outcome,
        () = ctx.cancel.cancelled() => {
            if ctx.state.query_runs().cancel(&name) {
                // The block is racing its own token now; let it unwind and log the run.
                (&mut statement).await
            } else {
                Err(AppError::cancelled("Cancelled by user"))
            }
        }
    }
}

fn engine_of(ctx: &NodeCtx<'_>, connection_id: &str) -> Result<Engine, NodeFailure> {
    Ok(connection::engine_of(ctx.state, connection_id)?)
}

// WHAT:  The Query node: renders the SQL from the first input row, runs it, and
//        emits the last result set (or `{ rowsAffected }` for a write).
pub async fn query(ctx: &NodeCtx<'_>, config: &QueryConfig) -> NodeResult {
    let template_sql = config.sql.trim();
    if template_sql.is_empty() {
        return Err(NodeFailure::new("Query is empty."));
    }
    let connection_id = connection_for(ctx, config.connection_id.as_ref())?;
    let engine = engine_of(ctx, &connection_id)?;
    let sql = template::render_sql(template_sql, ctx.input, engine)?;
    let run_sql = sql.clone();
    let outcome = guarded(ctx, &connection_id, &sql, config.allow_destructive, |session| async move {
        query::execute(&session, &run_sql, guard::clamp_result_rows(Some(ROW_LIMIT)), None).await
    })
    .await
    .map_err(|err| destructive_hint(err, config.allow_destructive))?;
    let truncated = outcome.statements.iter().any(|s| matches!(s, StatementResult::Rows { result } if result.truncated));
    let mut output = match outcome.statements.into_iter().last() {
        Some(StatementResult::Rows { result }) => NodeOutput::data(Dataset::from_result_set(result)),
        Some(StatementResult::Affected { rows_affected }) => {
            let affected = i64::try_from(rows_affected).unwrap_or(i64::MAX);
            NodeOutput::data(Dataset::single_row(vec![("rowsAffected", Value::Int(affected))]))
        }
        None => NodeOutput::data(Dataset::empty()),
    };
    if truncated {
        output.note = Some(format!("Stopped at {ROW_LIMIT} rows."));
    }
    Ok(output)
}

fn destructive_hint(err: AppError, allowed: bool) -> NodeFailure {
    match err {
        AppError::DestructiveConfirmationRequired { statements, .. } if !allowed => NodeFailure::new(format!(
            "Blocked: this node would run a destructive statement ({}). Turn on \"Allow destructive statements\" in the node's settings to run it.",
            statements.join("; ")
        )),
        other => other.into(),
    }
}

// WHAT:  The Insert node: writes the input rows into a table, one row at a time.
// WHY:   Row by row so the first bad row is named ("Row 12: …") instead of one
//        opaque failure for a thousand-row batch.
// HOW:   Fields are matched to the table's columns by name (exact first, then
//        ignoring case); fields the table has no column for are skipped. Every
//        statement is built here with engine-quoted identifiers and literals. The
//        whole run is one guarded statement, so the read-only lock applies and
//        history records it once.
pub async fn insert(ctx: &NodeCtx<'_>, config: &InsertConfig) -> NodeResult {
    let table_name = config.table.trim();
    if table_name.is_empty() {
        return Err(NodeFailure::new("No target table set."));
    }
    if ctx.input_count == 0 {
        return Err(NodeFailure::new("No input: connect an upstream node first."));
    }
    let data = ctx.input;
    if data.rows.is_empty() {
        return Ok(NodeOutput::sink(0));
    }
    let connection_id = connection_for(ctx, config.connection_id.as_ref())?;
    let engine = engine_of(ctx, &connection_id)?;
    let schema = Some(config.schema.trim()).filter(|s| !s.is_empty()).map(str::to_string);
    let table = TableRef { schema, name: table_name.to_string() };

    // Columns the table really has; if they cannot be read, every field is tried.
    let known: Option<Vec<String>> = {
        let lookup = table.clone();
        guard::session(ctx.state, &connection_id, |session| async move { session.integration.columns(&lookup).await })
            .await
            .ok()
            .map(|columns| columns.into_iter().map(|c| c.name).collect::<Vec<_>>())
            .filter(|names| !names.is_empty())
    };
    let mapping = map_columns(&data.column_names(), known.as_deref());
    if mapping.is_empty() {
        return Err(NodeFailure::new(format!("Row 1: no fields match a column on \"{table_name}\".")));
    }
    let statements: Vec<String> = data.rows.iter().map(|row| insert_statement(engine, &table, &mapping, row)).collect();
    let first = statements.first().cloned().unwrap_or_default();
    let total = statements.len();
    let summary = format!("{first} /* workflow insert: {total} row(s) */");
    let outcome = guarded(ctx, &connection_id, &summary, false, |session| async move {
        let mut inserted: u64 = 0;
        for (index, statement) in statements.iter().enumerate() {
            session.integration.execute(statement, 1).await.map_err(|err| {
                let hint = if inserted > 0 { format!(" ({inserted} row(s) were inserted before it)") } else { String::new() };
                AppError::driver(format!("Row {}: {}{hint}", index + 1, err.message()))
            })?;
            inserted += 1;
        }
        Ok(QueryOutcome { statements: vec![StatementResult::Affected { rows_affected: inserted }], total_rows: None, elapsed_ms: 0 })
    })
    .await?;
    Ok(NodeOutput::sink(outcome.row_count()))
}

// WHAT:  Which input columns go where: `(input index, table column name)`.
fn map_columns(fields: &[&str], known: Option<&[String]>) -> Vec<(usize, String)> {
    fields
        .iter()
        .enumerate()
        .filter_map(|(index, field)| match known {
            None => Some((index, (*field).to_string())),
            Some(columns) => columns
                .iter()
                .find(|c| c.as_str() == *field)
                .or_else(|| columns.iter().find(|c| c.eq_ignore_ascii_case(field)))
                .map(|c| (index, c.clone())),
        })
        .collect()
}

fn insert_statement(engine: Engine, table: &TableRef, mapping: &[(usize, String)], row: &[Value]) -> String {
    let columns = mapping.iter().map(|(_, name)| quote_ident_for(engine, name)).collect::<Vec<_>>().join(", ");
    let values = mapping
        .iter()
        .map(|(index, _)| template::literal_for(engine, row.get(*index).unwrap_or(&Value::Null)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("INSERT INTO {} ({columns}) VALUES ({values})", qualified_name_for(engine, table))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_match_columns_by_name_then_ignoring_case() {
        let known = vec!["id".to_string(), "Name".to_string()];
        assert_eq!(map_columns(&["id", "name", "extra"], Some(&known)), vec![(0, "id".to_string()), (1, "Name".to_string())]);
        assert_eq!(map_columns(&["a", "b"], None), vec![(0, "a".to_string()), (1, "b".to_string())]);
        assert!(map_columns(&["zzz"], Some(&known)).is_empty());
    }

    #[test]
    fn insert_statements_quote_for_the_engine() {
        let table = TableRef { schema: Some("app".into()), name: "users".into() };
        let mapping = vec![(0, "id".to_string()), (1, "note".to_string())];
        let row = vec![Value::Int(7), Value::Text("it's \\ here".into())];
        assert_eq!(
            insert_statement(Engine::Postgres, &table, &mapping, &row),
            "INSERT INTO \"app\".\"users\" (\"id\", \"note\") VALUES (7, 'it''s \\ here')"
        );
        assert_eq!(
            insert_statement(Engine::Mysql, &table, &mapping, &row),
            "INSERT INTO `app`.`users` (`id`, `note`) VALUES (7, 'it''s \\\\ here')"
        );
        let short = vec![Value::Int(1)];
        assert!(insert_statement(Engine::Sqlite, &table, &mapping, &short).ends_with("VALUES (1, NULL)"));
    }

    #[test]
    fn destructive_blocks_explain_the_opt_in() {
        let err = AppError::DestructiveConfirmationRequired { message: "x".into(), statements: vec!["DROP TABLE t".into()] };
        let blocked = destructive_hint(err.clone(), false);
        assert!(blocked.message.contains("Allow destructive statements") && blocked.message.contains("DROP TABLE t"));
        assert!(!destructive_hint(AppError::cancelled("x"), false).message.is_empty());
    }
}
