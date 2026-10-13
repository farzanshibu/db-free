// SOT: batch-run-model, batch-event-stream, history-page-model

use crate::error::AppError;
use crate::model::{HistoryEntry, QueryOutcome, StatementIntent};
use serde::Serialize;
use ts_rs::TS;

// WHAT:  How one statement of a batch ended.
// WHY:   A script is not one pass/fail: statement 3 can fail while 1, 2 and 4 are
//        fine. The result tabs show each one's own state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BatchStatus {
    Success,
    Error,
    /// Never started: an earlier statement failed with stop-on-error on, or the run was stopped.
    Skipped,
    /// Was running when the user pressed Stop.
    Cancelled,
}

// WHAT:  One statement as the batch cut it out of the script, before it runs.
// HOW:   `start` / `end` are UTF-16 offsets into the script that was sent (the
//        units a JavaScript string and a CodeMirror position count in), so the UI
//        maps an error's position back into the editor with plain arithmetic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BatchStatementPlan {
    pub index: u32,
    pub start: u32,
    pub end: u32,
    pub sql: String,
    pub intent: StatementIntent,
}

// WHAT:  What running one statement produced.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BatchStatementResult {
    pub index: u32,
    pub status: BatchStatus,
    pub elapsed_ms: u64,
    pub outcome: Option<QueryOutcome>,
    /// The failure behind an `Error` status; `Cancelled` carries the cancel notice.
    pub error: Option<AppError>,
}

// WHAT:  Counts for the status line once a batch is over.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BatchSummary {
    pub total: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub skipped: u32,
    pub cancelled: u32,
    pub elapsed_ms: u64,
}

impl BatchSummary {
    // WHAT:  Tallies finished statements; `total` counts the plan, so statements
    //        that never got a result still show as skipped.
    pub fn of(total: usize, results: &[BatchStatementResult], elapsed_ms: u64) -> BatchSummary {
        let count = |status: BatchStatus| u32::try_from(results.iter().filter(|r| r.status == status).count()).unwrap_or(u32::MAX);
        let total = u32::try_from(total).unwrap_or(u32::MAX);
        let accounted = count(BatchStatus::Success) + count(BatchStatus::Error) + count(BatchStatus::Cancelled) + count(BatchStatus::Skipped);
        BatchSummary {
            total,
            succeeded: count(BatchStatus::Success),
            failed: count(BatchStatus::Error),
            skipped: count(BatchStatus::Skipped) + total.saturating_sub(accounted),
            cancelled: count(BatchStatus::Cancelled),
            elapsed_ms,
        }
    }
}

// WHAT:  The progress stream of one batch run, in order: the plan, then a
//        started / finished pair per statement, then the summary.
// WHY:   The result tabs fill in as statements finish instead of after the whole
//        script. The events carry the outcomes themselves, so the command's own
//        return value is only the summary and no row travels twice.
// WHERE: src-tauri/src/commands/query.rs (execute_batch), src/lib/ipc.ts (executeBatch)
#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum BatchEvent {
    #[serde(rename_all = "camelCase")]
    Planned { statements: Vec<BatchStatementPlan> },
    #[serde(rename_all = "camelCase")]
    Started { index: u32 },
    #[serde(rename_all = "camelCase")]
    Finished { result: BatchStatementResult },
    #[serde(rename_all = "camelCase")]
    Done { summary: BatchSummary },
}

// WHAT:  One page of the query log and how many rows the filter matches in all.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    pub total: u64,
}

// WHAT:  A `.sql` file read for the editor.
// WHY:   The webview cannot read files; Rust reads the one the user picked.
// HOW:   `lossy` is set when the bytes were not valid text in their encoding and
//        some characters had to be replaced, so the UI can say so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenedSqlFile {
    /// The file name without its extension: the new tab's title.
    pub name: String,
    pub sql: String,
    pub lossy: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(index: u32, status: BatchStatus) -> BatchStatementResult {
        BatchStatementResult { index, status, elapsed_ms: 1, outcome: None, error: None }
    }

    #[test]
    fn a_summary_counts_every_planned_statement_even_those_that_never_ran() {
        let results = [result(0, BatchStatus::Success), result(1, BatchStatus::Error)];
        let summary = BatchSummary::of(4, &results, 9);
        assert_eq!(summary, BatchSummary { total: 4, succeeded: 1, failed: 1, skipped: 2, cancelled: 0, elapsed_ms: 9 });
    }

    #[test]
    fn explicit_skips_and_cancels_are_not_counted_twice() {
        let results = [result(0, BatchStatus::Cancelled), result(1, BatchStatus::Skipped), result(2, BatchStatus::Skipped)];
        let summary = BatchSummary::of(3, &results, 0);
        assert_eq!((summary.cancelled, summary.skipped, summary.total), (1, 2, 3));
    }

    #[test]
    fn events_are_tagged_for_the_ui() {
        let event = BatchEvent::Started { index: 2 };
        let json = serde_json::to_value(&event).unwrap_or_default();
        assert_eq!(json, serde_json::json!({ "type": "started", "index": 2 }));
    }
}
