// SOT: workflow-transform-nodes, schedule-node, filter-node, gate-node, branch-node, repeat-node, delay-node, regex-node, javascript-node

use super::{or_stopped, NodeCtx, NodeFailure, NodeOutput, NodeResult};
use crate::integrations::js_sandbox::{self, ScriptErrorKind, ScriptOutcome, ScriptRequest, SCRIPT_TIME_LIMIT};
use crate::model::workflow::{
    BranchConfig, ConsoleLine, DelayConfig, DelayUnit, FilterConfig, GateConfig, JavascriptConfig, RegexConfig, SourceHandle,
};
use crate::model::Value;
use crate::services::workflows::data::Dataset;
use crate::services::workflows::{rules, text};
use chrono::Utc;
use std::time::Duration;

/// The longest a Delay node sleeps, whatever it is set to.
const MAX_DELAY: Duration = Duration::from_secs(7 * 24 * 3600);

// WHAT:  A trigger's output: one row naming the moment it fired.
pub fn schedule() -> NodeResult {
    let fired = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok(NodeOutput::data(Dataset::single_row(vec![("firedAt", Value::DateTime(fired))])))
}

pub fn filter(ctx: &NodeCtx<'_>, config: &FilterConfig) -> NodeResult {
    Ok(NodeOutput::data(rules::filter(&config.group, ctx.input)))
}

// WHAT:  Lets rows through only while the row count satisfies the rule.
// WHY:   "Only alert when something came back." A closed gate hands on an empty
//        result with the same columns and closes the path behind it, so the
//        notification nodes after it do not run at all.
pub fn gate(ctx: &NodeCtx<'_>, config: &GateConfig) -> NodeResult {
    let count = ctx.input.rows.len();
    if rules::gate_open(config.op, config.value, count) {
        return Ok(NodeOutput::data(ctx.input.clone()));
    }
    let mut out = NodeOutput::data(ctx.input.schema_only());
    out.closed = true;
    out.note = Some(format!("Gate closed: {count} row(s) did not satisfy the rule."));
    Ok(out)
}

pub fn branch(ctx: &NodeCtx<'_>, config: &BranchConfig) -> NodeResult {
    let (yes, no) = rules::split(&config.group, ctx.input);
    let note = format!("{} row(s) to True, {} to False", yes.rows.len(), no.rows.len());
    Ok(NodeOutput {
        rows: Some(ctx.input.row_count()),
        outputs: vec![(Some(SourceHandle::True), yes), (Some(SourceHandle::False), no)],
        note: Some(note),
        ..NodeOutput::default()
    })
}

// WHAT:  One pass of a Repeat: a single row `{ i, total }` with `i` from 0.
pub fn repeat(ctx: &NodeCtx<'_>) -> NodeResult {
    let (i, total) = ctx.repeat.unwrap_or((0, 1));
    Ok(NodeOutput::data(Dataset::single_row(vec![("i", Value::Int(i64::from(i))), ("total", Value::Int(i64::from(total)))])))
}

pub async fn delay(ctx: &NodeCtx<'_>, config: &DelayConfig) -> NodeResult {
    let factor = match config.unit {
        DelayUnit::Ms => 1.0,
        DelayUnit::S => 1_000.0,
        DelayUnit::M => 60_000.0,
    };
    let millis = if config.amount.is_finite() { (config.amount * factor).max(0.0) } else { 0.0 };
    let pause = Duration::from_secs_f64((millis / 1000.0).min(MAX_DELAY.as_secs_f64()));
    if !pause.is_zero() {
        or_stopped(ctx.cancel, tokio::time::sleep(pause)).await?;
    }
    Ok(NodeOutput::data(ctx.input.clone()))
}

pub fn regex(ctx: &NodeCtx<'_>, config: &RegexConfig) -> NodeResult {
    Ok(NodeOutput::data(text::apply(config, ctx.input)?))
}

// WHAT:  Runs the user's script over the input rows in the sandbox.
// HOW:   The script's `console` output is attached to the node whether it
//        succeeds or fails, so the inspector can show what it printed before it broke.
pub async fn javascript(ctx: &NodeCtx<'_>, config: &JavascriptConfig) -> NodeResult {
    if config.code.trim().is_empty() {
        return Ok(NodeOutput::data(ctx.input.clone()));
    }
    let rows_json = serde_json::to_string(&ctx.input.to_json_rows()).map_err(|e| NodeFailure::new(e.to_string()))?;
    let fields_json = ctx.input.to_json_fields().to_string();
    let request = ScriptRequest { code: config.code.clone(), rows_json, fields_json, time_limit: SCRIPT_TIME_LIMIT };
    let report = js_sandbox::run(request, ctx.cancel.clone()).await;
    let logs: Vec<ConsoleLine> = report.logs.into_iter().map(|l| ConsoleLine { level: l.level, text: l.text }).collect();
    match report.result {
        Ok(ScriptOutcome::Passthrough) => Ok(NodeOutput { logs, ..NodeOutput::data(ctx.input.clone()) }),
        Ok(ScriptOutcome::Rows { rows, fields }) => Ok(NodeOutput { logs, ..NodeOutput::data(Dataset::from_json_rows_ordered(&rows, &fields)) }),
        Err(err) => {
            let message = match err.kind {
                ScriptErrorKind::Syntax => format!("Syntax error: {}", err.message),
                ScriptErrorKind::Runtime => format!("Runtime error: {}", err.message),
                ScriptErrorKind::Stopped => return Err(NodeFailure { logs, ..NodeFailure::stopped() }),
                ScriptErrorKind::TimedOut => err.message,
            };
            Err(NodeFailure { message, stopped: false, logs })
        }
    }
}
