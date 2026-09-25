// SOT: export-model, import-model, transfer-format, ai-model, plan-node, plan-report

use crate::model::TableRef;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TransferFormat {
    Csv,
    Json,
    Sql,
}

impl TransferFormat {
    pub fn extension(self) -> &'static str {
        match self {
            TransferFormat::Csv => "csv",
            TransferFormat::Json => "json",
            TransferFormat::Sql => "sql",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportedFile {
    pub table: TableRef,
    pub path: String,
    pub rows: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportReport {
    pub files: Vec<ExportedFile>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportReport {
    pub rows_inserted: u64,
    pub statements: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiReply {
    pub sql: Option<String>,
    pub text: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlanReport {
    pub plan: String,
    pub explanation: Option<String>,
    /// The same plan as a tree, for engines whose adapter can read a
    /// structured EXPLAIN (Postgres, MySQL/MariaDB, SQLite). None elsewhere:
    /// the UI then shows only the text.
    #[serde(default)]
    pub plan_tree: Option<PlanNode>,
}

// WHAT:  One operator of an execution plan (scan, join, sort…) and its inputs.
// WHY:   A plan read as a tree, with a bar per node sized by cost, shows where
//        the time goes faster than the engine's indented text.
// HOW:   `cost` is the engine's own cumulative estimate for the subtree (its
//        units differ per engine, so only relative sizes mean anything);
//        `actual_ms` is filled only when the plan came from an EXPLAIN ANALYZE.
// WHERE: src-tauri/src/integrations/plan.rs (parsers), src/features/editor/PlanTree.tsx
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlanNode {
    pub label: String,
    pub detail: Option<String>,
    pub cost: Option<f64>,
    pub rows: Option<f64>,
    pub actual_ms: Option<f64>,
    pub children: Vec<PlanNode>,
}
