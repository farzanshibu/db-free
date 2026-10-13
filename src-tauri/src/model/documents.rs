// SOT: document-model, dashboard-model, widget-model, workflow-model, diagram-model

use super::connection::ConnectionColor;
use super::diagrams::{DiagramColumnPair, DiagramLineWidth, DiagramNote, DiagramPoint, DiagramSide, ReferentialAction};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  JSON-bodied artefacts the user authors: dashboards, workflows, schema diagrams.
// WHY:   One envelope (`Document`) with a tagged body keeps persistence generic
//        while every body stays a typed, TS-exported shape.
// WHERE: src-tauri/src/store/documents.rs, src/features/{dashboards,workflows,diagrams}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DocumentKind {
    Dashboard,
    Workflow,
    Diagram,
}

impl DocumentKind {
    pub const ALL: [DocumentKind; 3] = [DocumentKind::Dashboard, DocumentKind::Workflow, DocumentKind::Diagram];

    pub fn as_str(self) -> &'static str {
        match self {
            DocumentKind::Dashboard => "dashboard",
            DocumentKind::Workflow => "workflow",
            DocumentKind::Diagram => "diagram",
        }
    }

    pub fn parse(raw: &str) -> Option<DocumentKind> {
        DocumentKind::ALL.into_iter().find(|k| k.as_str() == raw)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Document {
    pub id: String,
    pub kind: DocumentKind,
    pub connection_id: Option<String>,
    pub name: String,
    pub body: DocumentBody,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[ts(export)]
pub enum DocumentBody {
    Dashboard(DashboardBody),
    Workflow(WorkflowBody),
    Diagram(DiagramBody),
}

// ---------------------------------------------------------------- dashboards
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DashboardBody {
    pub widgets: Vec<Widget>,
    /// Typed variables. Dashboards saved before types existed stored plain
    /// `{name, value}` strings; `variables_compat` reads those as `Text`.
    #[serde(default, with = "variables_compat")]
    #[ts(as = "Vec<DashboardVariable>")]
    pub variables: Vec<DashboardVariable>,
    #[serde(default)]
    pub refresh_seconds: u32,
    /// Heading drawn over the grid; absent or empty = none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    /// Grid geometry the `x/y/w/h` of every widget are written in. Absent (0)
    /// is the first grid, 120 px rows; 1 is 12 columns of 60 px rows. The
    /// editor converts older dashboards on open and writes 1 when it saves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub layout_version: Option<u32>,
}

// WHAT:  Widget kinds (DB Manager parity). Dashboards saved before the rename
//        still spell `Metric` as "number"; `widget_kind_compat` on `Widget.kind`
//        accepts it (a `#[serde(alias)]` here would do the same, but ts-rs
//        cannot parse that attribute and warns on every build).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WidgetKind {
    Area,
    Line,
    Bar,
    Pie,
    Sankey,
    Table,
    Metric,
    Sparkline,
    Map,
    Progress,
    Text,
    Image,
    Gif,
}

/// Legacy spelling of `WidgetKind::Metric` in stored dashboards.
const LEGACY_METRIC_NAME: &str = "number";

// WHAT:  `Widget.kind` codec: writes the enum as-is, reads the enum plus the
//        legacy "number" spelling of `metric`. The TS shape stays `WidgetKind`
//        (`#[ts(as)]` on the field).
mod widget_kind_compat {
    use super::{WidgetKind, LEGACY_METRIC_NAME};
    use serde::de::IntoDeserializer;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(kind: &WidgetKind, serializer: S) -> Result<S::Ok, S::Error> {
        kind.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<WidgetKind, D::Error> {
        let name = String::deserialize(deserializer)?;
        let canonical = if name == LEGACY_METRIC_NAME { "metric" } else { name.as_str() };
        WidgetKind::deserialize(IntoDeserializer::<D::Error>::into_deserializer(canonical))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Widget {
    pub id: String,
    pub title: String,
    #[serde(with = "widget_kind_compat")]
    #[ts(as = "WidgetKind")]
    pub kind: WidgetKind,
    pub sql: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub tint: Option<String>,
    #[serde(default)]
    pub show_change: bool,
    /// Progress: the value that means 100 %.
    #[serde(default)]
    pub max_value: Option<f64>,
    /// Text widgets: the body (no SQL).
    #[serde(default)]
    pub text: Option<String>,
    /// Image / GIF widgets: the source URL.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub x_label: Option<String>,
    #[serde(default)]
    pub y_label: Option<String>,
    /// Bar: horizontal bars.
    #[serde(default)]
    pub horizontal: bool,
    /// Progress: show percentage / raw values.
    #[serde(default = "default_true")]
    pub show_percent: bool,
    #[serde(default)]
    pub show_values: bool,
    /// Map: pulse the markers.
    #[serde(default)]
    pub pulse: bool,
    /// Metric / Text / Image / GIF: evaluated against the first cell of the first row; last match wins.
    #[serde(default)]
    pub conditions: Vec<WidgetCondition>,
    /// Metric: how the value is written.
    #[serde(default)]
    pub format: MetricFormat,
    /// ISO 4217 code behind the Currency formats (Metric `format` and chart `valueFormat`).
    #[serde(default = "default_currency")]
    pub currency: String,
    /// Metric: tint the tile green or red by the sign of the change.
    #[serde(default)]
    pub color_by_change: bool,
    /// Metric / Text / Image / GIF: gradient background, a swatch name from the tile colour registry.
    #[serde(default)]
    pub bg: Option<String>,
    /// Area: stack the series on top of one another instead of overlapping them.
    #[serde(default)]
    pub stacked: bool,
    /// Area / Line / Bar / Pie: when the legend is drawn.
    #[serde(default)]
    pub legend_mode: LegendMode,
    /// Area / Line / Bar / Pie: how values read on axes and in tooltips.
    #[serde(default)]
    pub value_format: ValueFormat,
}

fn default_currency() -> String {
    "USD".to_string()
}

// WHAT:  A widget at its defaults: a line chart, 4 columns by 5 rows of the
//        60 px grid, every option off. Callers fill in what they mean.
// WHY:   Anything that builds dashboards in Rust (the agent) names only the
//        fields it cares about and takes `..Widget::default()` for the rest, so
//        a new option here never breaks a call site.
impl Default for Widget {
    fn default() -> Self {
        Widget {
            id: String::new(),
            title: String::new(),
            kind: WidgetKind::Line,
            sql: String::new(),
            x: 0,
            y: 0,
            w: 4,
            h: 5,
            tint: Some("series-1".to_string()),
            show_change: false,
            max_value: None,
            text: None,
            url: None,
            x_label: None,
            y_label: None,
            horizontal: false,
            show_percent: true,
            show_values: false,
            pulse: false,
            conditions: Vec::new(),
            format: MetricFormat::default(),
            currency: default_currency(),
            color_by_change: false,
            bg: None,
            stacked: false,
            legend_mode: LegendMode::default(),
            value_format: ValueFormat::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MetricFormat {
    #[default]
    Number,
    Decimal,
    Currency,
    Percent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LegendMode {
    /// Drawn when there are two or more series.
    #[default]
    Auto,
    Show,
    Hide,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ValueFormat {
    #[default]
    Number,
    /// 1.2K, 3.4M.
    Compact,
    Currency,
    Percent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ConditionOp {
    Equals,
    NotEquals,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WidgetCondition {
    pub op: ConditionOp,
    pub value: String,
    /// Text / image URL / tint name to use when the condition matches.
    pub content: String,
    /// Metric / Text: gradient background swatch used while the rule matches.
    #[serde(default)]
    pub bg: Option<String>,
}

// WHAT:  A dashboard variable: a name widget SQL reads as `{{name}}` plus the
//        value it currently holds, in the shape its control edits.
// WHY:   Every variable used to be a string pasted into the SQL as typed. A
//        date range, a list or a switch each need a control of their own and a
//        different literal in the statement, so the type travels with the value.
// HOW:   `date_range` holds ISO dates (`start` / `end`, "" while unset);
//        `select` / `multi_select` pick from `options`; `number` is null while
//        empty. The client turns each into a quoted SQL literal for the
//        connection's dialect (src/features/dashboards/variables.ts).
// WHERE: ./variables_compat (stored dashboards), src/features/dashboards/VariablesToolbar.tsx
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum DashboardVariable {
    DateRange {
        name: String,
        #[serde(default)]
        start: String,
        #[serde(default)]
        end: String,
    },
    Text {
        name: String,
        #[serde(default)]
        value: String,
    },
    Number {
        name: String,
        #[serde(default)]
        value: Option<f64>,
    },
    Select {
        name: String,
        #[serde(default)]
        value: String,
        #[serde(default)]
        options: Vec<String>,
    },
    MultiSelect {
        name: String,
        #[serde(default)]
        values: Vec<String>,
        #[serde(default)]
        options: Vec<String>,
    },
    Boolean {
        name: String,
        #[serde(default)]
        value: bool,
    },
}

impl DashboardVariable {
    pub fn name(&self) -> &str {
        match self {
            DashboardVariable::DateRange { name, .. }
            | DashboardVariable::Text { name, .. }
            | DashboardVariable::Number { name, .. }
            | DashboardVariable::Select { name, .. }
            | DashboardVariable::MultiSelect { name, .. }
            | DashboardVariable::Boolean { name, .. } => name,
        }
    }
}

// WHAT:  `DashboardBody.variables` codec: writes the typed list, reads it plus
//        the untyped `{name, value}` pairs older dashboards stored (they become
//        `Text`). The TS shape stays `Vec<DashboardVariable>` (`#[ts(as)]`).
// WHY:   Same pattern as `widget_kind_compat`: serde's own `from` / `alias`
//        attributes make ts-rs warn on every build, a field codec does not.
// HOW:   Each element is tried as a typed variable first (it carries `type`);
//        the legacy shape denies unknown fields, so a typed element with a
//        misspelt type is an error rather than a silent `Text`.
mod variables_compat {
    use super::DashboardVariable;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Typed(DashboardVariable),
        Legacy(LegacyVariable),
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct LegacyVariable {
        name: String,
        #[serde(default)]
        value: String,
    }

    pub fn serialize<S: Serializer>(variables: &[DashboardVariable], serializer: S) -> Result<S::Ok, S::Error> {
        variables.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<DashboardVariable>, D::Error> {
        let stored = Vec::<Stored>::deserialize(deserializer)?;
        Ok(stored
            .into_iter()
            .map(|s| match s {
                Stored::Typed(v) => v,
                Stored::Legacy(l) => DashboardVariable::Text { name: l.name, value: l.value },
            })
            .collect())
    }
}

// ---------------------------------------------------------------- workflows
use crate::model::workflow::{NodeRunState, RunStatus, WorkflowEdge, WorkflowNode};

// WHAT:  A workflow: nodes on a canvas joined by edges (see `model/workflow.rs`).
// WHY:   Documents saved before the graph existed hold a linear `steps` list
//        instead. Reading a body (a hand-written `Deserialize` in model/workflow.rs)
//        turns it into a chain of Query nodes, so the editor and the engine only
//        ever see a graph. Every field is optional on the wire so a body that only
//        names `steps` (an empty new workflow) is still valid.
// WHERE: src-tauri/src/services/workflows, src/features/workflows
#[derive(Debug, Clone, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowBody {
    /// Legacy linear step list; empty once the document has been saved as a graph.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<WorkflowStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<WorkflowNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<WorkflowEdge>,
    /// "Schedules active": Schedule nodes fire only while this is on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub schedules_active: bool,
}

/// One step of the pre-graph linear format; read only, to be upgraded.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowStep {
    pub id: String,
    pub name: String,
    /// None = the workflow's own connection.
    pub connection_id: Option<String>,
    pub sql: String,
    #[serde(default = "default_true")]
    pub stop_on_error: bool,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------- diagrams (designer)
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramBody {
    pub tables: Vec<DiagramTable>,
    pub relations: Vec<DiagramRelation>,
    /// Sticky notes. Diagrams saved before notes existed have none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<DiagramNote>,
    /// Snap grid in px; 0 turns snapping off. Absent = 16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub grid_size: Option<u32>,
    /// Show / hide every sticky note at once. Absent = shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub notes_visible: Option<bool>,
}

// WHAT:  One table card of a designed diagram. Every field past `columns` was
//        added after the first release, so each carries `#[serde(default)]`
//        and diagrams saved earlier still load.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramTable {
    pub id: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub columns: Vec<DiagramColumn>,
    /// Schema the table lives in; None = the connection's default.
    #[serde(default)]
    pub schema: Option<String>,
    /// Header colour: a palette key drawn through the `--color-conn-*` tokens.
    #[serde(default)]
    pub color: Option<ConnectionColor>,
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramColumn {
    pub name: String,
    pub data_type: String,
    #[serde(default)]
    pub primary_key: bool,
    #[serde(default = "default_true")]
    pub nullable: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub auto_increment: bool,
    /// A SQL expression, written into `DEFAULT …` as typed.
    #[serde(default)]
    pub default_value: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
}

impl Default for DiagramColumn {
    fn default() -> Self {
        DiagramColumn {
            name: String::new(),
            data_type: String::new(),
            primary_key: false,
            nullable: true,
            unique: false,
            auto_increment: false,
            default_value: None,
            comment: None,
        }
    }
}

// WHAT:  A foreign key from `from_table` to `to_table`. The first column pair is
//        `from_column` / `to_column` (the shape every earlier diagram has);
//        `extra_columns` holds the rest of a composite key. An empty
//        `from_column` / `to_column` means "drawn, columns not chosen yet".
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramRelation {
    pub id: String,
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    #[serde(default)]
    pub extra_columns: Vec<DiagramColumnPair>,
    /// Constraint name; None = generated when the SQL is written.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub on_delete: Option<ReferentialAction>,
    #[serde(default)]
    pub on_update: Option<ReferentialAction>,
    #[serde(default)]
    pub color: Option<ConnectionColor>,
    #[serde(default)]
    pub width: Option<DiagramLineWidth>,
    /// Which side of the source card the line leaves from; None = whichever faces the target.
    #[serde(default)]
    pub from_side: Option<DiagramSide>,
    #[serde(default)]
    pub to_side: Option<DiagramSide>,
    /// Bend points between the two cards.
    #[serde(default)]
    pub waypoints: Vec<DiagramPoint>,
}

impl DiagramRelation {
    /// Every (from, to) column pair, the first one included.
    pub fn pairs(&self) -> Vec<(&str, &str)> {
        let mut out = vec![(self.from_column.as_str(), self.to_column.as_str())];
        out.extend(self.extra_columns.iter().map(|p| (p.from_column.as_str(), p.to_column.as_str())));
        out
    }
}

// WHAT:  What one node did in a run (its last pass, if it ran more than once).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowStepResult {
    pub node_id: String,
    pub name: String,
    pub state: NodeRunState,
    pub elapsed_ms: u64,
    pub rows: Option<u64>,
    pub error: Option<String>,
    pub file_path: Option<String>,
}

// WHAT:  Outcome of a workflow run: how it ended and one entry per node that was reached.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkflowRunReport {
    pub run_id: String,
    pub workflow_id: String,
    pub status: RunStatus,
    pub elapsed_ms: u64,
    pub nodes: Vec<WorkflowStepResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget_json(kind: &str) -> String {
        format!(r#"{{"id":"w1","title":"Orders","kind":"{kind}","sql":"select 1","x":0,"y":0,"w":4,"h":2,"tint":null}}"#)
    }

    #[test]
    fn widget_kind_reads_legacy_number_as_metric() {
        let legacy: Widget = serde_json::from_str(&widget_json("number")).unwrap();
        assert_eq!(legacy.kind, WidgetKind::Metric);
        let current: Widget = serde_json::from_str(&widget_json("metric")).unwrap();
        assert_eq!(current.kind, WidgetKind::Metric);
    }

    #[test]
    fn widget_kind_writes_canonical_name() {
        let legacy: Widget = serde_json::from_str(&widget_json("number")).unwrap();
        let json = serde_json::to_value(&legacy).unwrap();
        assert_eq!(json["kind"], "metric");
    }

    #[test]
    fn widget_kind_rejects_unknown_names() {
        assert!(serde_json::from_str::<Widget>(&widget_json("gauge")).is_err());
    }

    #[test]
    fn widget_without_the_newer_options_loads_with_defaults() {
        let widget: Widget = serde_json::from_str(&widget_json("metric")).unwrap();
        assert_eq!(widget.format, MetricFormat::Number);
        assert_eq!(widget.currency, "USD");
        assert_eq!(widget.legend_mode, LegendMode::Auto);
        assert_eq!(widget.value_format, ValueFormat::Number);
        assert!(!widget.color_by_change && !widget.stacked);
        assert!(widget.bg.is_none());
    }

    #[test]
    fn condition_reads_the_null_operators_and_a_missing_background() {
        let cond: WidgetCondition = serde_json::from_str(r#"{"op":"is_not_null","value":"","content":"ok"}"#).unwrap();
        assert_eq!(cond.op, ConditionOp::IsNotNull);
        assert!(cond.bg.is_none());
        let cond: WidgetCondition = serde_json::from_str(r#"{"op":"is_null","value":"","content":"","bg":"red"}"#).unwrap();
        assert_eq!(cond.op, ConditionOp::IsNull);
        assert_eq!(cond.bg.as_deref(), Some("red"));
    }

    fn body_json(variables: &str) -> String {
        format!(r#"{{"widgets":[],"variables":{variables},"refreshSeconds":30}}"#)
    }

    #[test]
    fn untyped_variables_read_as_text() {
        let body: DashboardBody = serde_json::from_str(&body_json(r#"[{"name":"region","value":"eu"},{"name":"empty"}]"#)).unwrap();
        assert_eq!(
            body.variables,
            vec![
                DashboardVariable::Text { name: "region".into(), value: "eu".into() },
                DashboardVariable::Text { name: "empty".into(), value: String::new() },
            ]
        );
        assert!(body.title.is_none() && body.layout_version.is_none());
    }

    #[test]
    fn typed_variables_round_trip() {
        let variables = vec![
            DashboardVariable::DateRange { name: "period".into(), start: "2026-01-01".into(), end: "2026-01-31".into() },
            DashboardVariable::Number { name: "limit".into(), value: Some(25.0) },
            DashboardVariable::Select { name: "env".into(), value: "prod".into(), options: vec!["prod".into(), "dev".into()] },
            DashboardVariable::MultiSelect { name: "tags".into(), values: vec!["a".into()], options: vec!["a".into(), "b".into()] },
            DashboardVariable::Boolean { name: "active".into(), value: true },
        ];
        let body = DashboardBody { variables: variables.clone(), title: Some("Sales".into()), layout_version: Some(1), ..DashboardBody::default() };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["variables"][0]["type"], "date_range");
        assert_eq!(json["variables"][3]["type"], "multi_select");
        assert_eq!(json["title"], "Sales");
        assert_eq!(json["layoutVersion"], 1);
        let back: DashboardBody = serde_json::from_value(json).unwrap();
        assert_eq!(back.variables, variables);
    }

    #[test]
    fn variable_name_reads_every_variant() {
        let variable = DashboardVariable::MultiSelect { name: "tags".into(), values: Vec::new(), options: Vec::new() };
        assert_eq!(variable.name(), "tags");
    }

    #[test]
    fn typed_variable_with_an_unknown_type_is_rejected() {
        assert!(serde_json::from_str::<DashboardBody>(&body_json(r#"[{"type":"color","name":"c","value":"red"}]"#)).is_err());
    }

    #[test]
    fn empty_body_omits_the_optional_fields() {
        let json = serde_json::to_value(DashboardBody::default()).unwrap();
        assert!(json.get("title").is_none() && json.get("layoutVersion").is_none());
    }

    #[test]
    fn widget_default_is_a_plain_line_chart() {
        let widget = Widget::default();
        assert_eq!((widget.kind, widget.w, widget.h), (WidgetKind::Line, 4, 5));
        assert_eq!(widget.currency, "USD");
    }
}
