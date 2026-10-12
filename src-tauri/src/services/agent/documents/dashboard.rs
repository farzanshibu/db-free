// SOT: agent-dashboard-builder, ai-dashboard-schema, dashboard-draft-validation, dashboard-grid-placement, dashboard-variable-draft

use super::{choice, clamped, clean_name, flag, fresh_id, list, plural, preview, properties, required, text, Draft};
use crate::error::{AppError, AppResult};
use crate::guard::destructive::{classify, StatementKind};
use crate::model::{
    ConditionOp, DashboardBody, DashboardVariable, DocumentBody, LegendMode, MetricFormat, ValueFormat, Widget, WidgetCondition,
    WidgetKind,
};
use crate::services::agent::provider::{tool, ToolSpec};
use serde::de::DeserializeOwned;
use serde_json::{json, Value as Json};
use std::collections::HashMap;

// WHAT:  `create_dashboard`: what the model is told a dashboard is, and the
//        check that turns its answer into a real `DashboardBody`.
// WHY:   Widget queries run on every refresh, unattended, so anything that is
//        not a plain read is refused here, before the user is even asked. The
//        grid geometry (12 columns of 60 px rows, `layoutVersion` 1) is the
//        dashboard editor's; a body without it would be re-laid out as an old
//        flow layout, so this file writes it.
// WHERE: ./mod.rs (the shared create/update flow), src/features/dashboards (the editor)

const MAX_WIDGETS: usize = 24;
const MAX_VARIABLES: usize = 10;
const MAX_OPTIONS: usize = 100;
const MAX_CONDITIONS: usize = 6;
const MAX_TITLE: usize = 120;
const GRID_COLUMNS: u32 = 12;
const MIN_CELLS: u32 = 2;
const MAX_ROWS: u32 = 24;
/// `DashboardBody.layout_version` of the 12-column, 60 px-row grid.
const LAYOUT_VERSION: u32 = 1;

/// Widget kinds the assistant may build. Image and GIF tiles need an outside URL, which is the user's to choose.
const WIDGET_KINDS: [&str; 11] = ["area", "line", "bar", "pie", "sankey", "table", "metric", "sparkline", "map", "progress", "text"];
const SIZES: [&str; 4] = ["small", "medium", "large", "full"];
const REFRESH_CHOICES: [u32; 6] = [0, 5, 10, 30, 60, 300];
const VARIABLE_TYPES: [&str; 6] = ["date_range", "text", "number", "select", "multi_select", "boolean"];
/// The tile colour registry (`src/features/dashboards/tiles.ts`).
const SWATCHES: [&str; 14] = [
    "slate", "red", "orange", "amber", "green", "emerald", "teal", "cyan", "blue", "indigo", "violet", "purple", "pink", "rose",
];
const CONDITION_OPS: [&str; 9] = ["equals", "not_equals", "gt", "gte", "lt", "lte", "contains", "is_null", "is_not_null"];

// ---------------------------------------------------------------------------
// What the model is told
// ---------------------------------------------------------------------------

pub fn dashboard_tool(language: &str) -> ToolSpec {
    tool(
        "create_dashboard",
        &format!(
            "Build a dashboard — a grid of tiles, each backed by one read-only {language} query — and \
             save it for the user. The user reviews it and approves before anything is saved. To change \
             a dashboard the user @-mentioned, or whose document id they quoted, pass its documentId \
             instead of creating another; the new widgets REPLACE the old ones, so include every widget \
             you want to keep (variables, title and refresh are kept when you leave them out). Write each \
             query to return exactly the shape its tile draws: metric = one row whose first numeric \
             column is the headline and whose optional second numeric column is the value to compare \
             with; line / area / bar / sparkline = a label column (a date, a category) then one or more \
             numeric columns; pie = a label and a value; progress = a value and, optionally, a target; \
             sankey = source, target, value; map = latitude and longitude columns (lat, lon) and an \
             optional label; table = any columns. Queries may only read data. Prefer 4 to 8 tiles: \
             headline metrics first, then charts, then a table.\n\
             Variables: declare a named value the user can change in the dashboard's toolbar and use \
             it in a query as {{{{name}}}}, written bare where a literal belongs (WHERE region = \
             {{{{region}}}}, never inside quotes): text and select give one string, number a number, \
             multi_select a comma list for IN ({{{{regions}}}}), boolean true/false, and date_range gives \
             {{{{period.start}}}} and {{{{period.end}}}} (or {{{{period}}}} after BETWEEN). Every value is \
             quoted for the database for you. Give every variable a sensible starting value (an empty one \
             reads as NULL and matches nothing). Only use a variable for a filter the user would really change."
        ),
        json!({
            "type": "object",
            "properties": properties(vec![
                ("name", json!({ "type": "string", "description": "Dashboard name." })),
                ("documentId", json!({ "type": "string", "description": "Id of an existing dashboard to replace, from an @mention or an id the user quoted. Omit to create a new one." })),
                ("title", json!({ "type": "string", "description": "Optional heading drawn above the grid. The name is usually enough." })),
                ("refreshSeconds", json!({ "type": "integer", "enum": REFRESH_CHOICES, "description": "Auto-refresh interval in seconds; 0 for manual (the default)." })),
                ("variables", json!({ "type": "array", "maxItems": MAX_VARIABLES, "items": variable_schema() })),
                ("widgets", json!({ "type": "array", "minItems": 1, "maxItems": MAX_WIDGETS, "items": widget_schema(language) })),
            ]),
            "required": ["name", "widgets"],
            "additionalProperties": false
        }),
    )
}

fn variable_schema() -> Json {
    json!({
        "type": "object",
        "properties": properties(vec![
            ("name", json!({ "type": "string", "description": "Letters, digits and underscores; used as {{name}}." })),
            ("type", json!({ "type": "string", "enum": VARIABLE_TYPES, "description": "Defaults to text." })),
            ("value", json!({ "type": "string", "description": "text / select: the value. number: the number as text. boolean: \"true\" or \"false\"." })),
            ("values", json!({ "type": "array", "items": { "type": "string" }, "description": "multi_select: the chosen options." })),
            ("options", json!({ "type": "array", "items": { "type": "string" }, "description": "select / multi_select: the choices." })),
            ("start", json!({ "type": "string", "description": "date_range: first day, YYYY-MM-DD. Give a real range: an empty bound reads as NULL and matches nothing." })),
            ("end", json!({ "type": "string", "description": "date_range: last day, YYYY-MM-DD." })),
        ]),
        "required": ["name"],
        "additionalProperties": false
    })
}

fn condition_schema() -> Json {
    json!({
        "type": "object",
        "properties": properties(vec![
            ("op", json!({ "type": "string", "enum": CONDITION_OPS })),
            ("value", json!({ "type": "string" })),
            ("content", json!({ "type": "string", "description": "text: the text shown while the rule matches." })),
            ("bg", json!({ "type": "string", "enum": SWATCHES, "description": "The background colour while the rule matches." })),
        ]),
        "required": ["op"],
        "additionalProperties": false
    })
}

fn widget_schema(language: &str) -> Json {
    json!({
        "type": "object",
        "properties": properties(vec![
            ("title", json!({ "type": "string" })),
            ("kind", json!({ "type": "string", "enum": WIDGET_KINDS })),
            ("sql", json!({ "type": "string", "description": format!("A single read-only {language} statement. Omit for a text tile with no data.") })),
            ("size", json!({ "type": "string", "enum": SIZES, "description": "Footprint on the 12-column grid. Defaults by kind." })),
            ("w", json!({ "type": "integer", "minimum": MIN_CELLS, "maximum": GRID_COLUMNS, "description": "Width in grid columns (of 12). Overrides size." })),
            ("h", json!({ "type": "integer", "minimum": MIN_CELLS, "maximum": MAX_ROWS, "description": "Height in 60 px rows. Overrides size." })),
            ("x", json!({ "type": "integer", "minimum": 0, "description": "Column. Give x and y on every widget to keep an existing layout; otherwise tiles are placed left to right." })),
            ("y", json!({ "type": "integer", "minimum": 0, "description": "Row." })),
            ("color", json!({ "type": "string", "enum": SWATCHES, "description": "The tile's accent colour. Defaults to the chart palette." })),
            ("background", json!({ "type": "string", "enum": SWATCHES, "description": "metric / text: a gradient background in that colour." })),
            ("xLabel", json!({ "type": "string" })),
            ("yLabel", json!({ "type": "string" })),
            ("horizontal", json!({ "type": "boolean", "description": "bar: draw horizontal bars." })),
            ("stacked", json!({ "type": "boolean", "description": "area: stack the series. Defaults to true." })),
            ("legend", json!({ "type": "string", "enum": ["auto", "show", "hide"], "description": "Charts: when the legend is drawn." })),
            ("valueFormat", json!({ "type": "string", "enum": ["number", "compact", "currency", "percent"], "description": "Charts: how values read on axes and in tooltips." })),
            ("format", json!({ "type": "string", "enum": ["number", "decimal", "currency", "percent"], "description": "metric: how the headline is written." })),
            ("currency", json!({ "type": "string", "description": "ISO 4217 code behind the currency formats, e.g. EUR. Defaults to USD." })),
            ("showChange", json!({ "type": "boolean", "description": "metric: show the change from the compared value." })),
            ("colorByChange", json!({ "type": "boolean", "description": "metric: tint green or red by the sign of the change." })),
            ("maxValue", json!({ "type": "number", "description": "progress: the value that means 100%." })),
            ("showPercent", json!({ "type": "boolean", "description": "progress: show the percentage. Defaults to true." })),
            ("showValues", json!({ "type": "boolean", "description": "progress: show value / maximum." })),
            ("pulse", json!({ "type": "boolean", "description": "map: pulse the markers." })),
            ("text", json!({ "type": "string", "description": "text: the body, plain lines (markdown is not rendered). {{column}} inserts a value from the first row of its query." })),
            ("conditions", json!({
                "type": "array",
                "maxItems": MAX_CONDITIONS,
                "description": "metric / text: rules checked against the first cell of the first row; the last match wins.",
                "items": condition_schema()
            })),
        ]),
        "required": ["title", "kind"],
        "additionalProperties": false
    })
}

// ---------------------------------------------------------------------------
// What the model's answer becomes
// ---------------------------------------------------------------------------

/// A widget as validated, before it is given a spot on the grid.
struct Tile {
    widget: Widget,
    w: u32,
    h: u32,
    at: Option<(u32, u32)>,
}

// WHAT:  A dashboard proposal → a validated `DraftBody`.
// HOW:   Variables and the grid are settled first (a query may only name a
//        declared variable), then each widget is checked on its own and placed.
//        On an update, whatever the model left out — variables, title, refresh —
//        is carried over from the saved dashboard.
pub(super) fn parse(args: &Json, previous: Option<&DocumentBody>) -> AppResult<Draft> {
    let what = "A dashboard";
    let name = clean_name(args, what)?;
    let raw_widgets = list(args, "widgets", what, 1, MAX_WIDGETS)?;
    let before = match previous {
        Some(DocumentBody::Dashboard(body)) => Some(body),
        _ => None,
    };

    let variables = if args.get("variables").is_some() {
        parse_variables(args)?
    } else {
        before.map(|body| body.variables.clone()).unwrap_or_default()
    };
    let declared: HashMap<&str, &DashboardVariable> = variables.iter().map(|variable| (variable.name(), variable)).collect();

    let mut sized = Vec::with_capacity(raw_widgets.len());
    let mut previews = Vec::with_capacity(raw_widgets.len());
    for (index, raw) in raw_widgets.iter().enumerate() {
        let tile = parse_widget(raw, index, &declared)?;
        previews.push(if tile.widget.kind == WidgetKind::Text && tile.widget.sql.is_empty() {
            format!("{} (text)", tile.widget.title)
        } else {
            format!("{} ({}): {}", tile.widget.title, kind_name(tile.widget.kind), preview(&tile.widget.sql))
        });
        sized.push(tile);
    }

    let spots = place(&sized);
    let widgets: Vec<Widget> = sized
        .into_iter()
        .zip(spots)
        .map(|(tile, (x, y))| Widget { x, y, w: tile.w, h: tile.h, ..tile.widget })
        .collect();

    let refresh_seconds = match args.get("refreshSeconds").and_then(Json::as_u64) {
        Some(requested) => snap_refresh(requested),
        None => before.map_or(0, |body| body.refresh_seconds),
    };
    let title = match text(args, "title") {
        Some(heading) => Some(heading.chars().take(MAX_TITLE).collect::<String>()),
        None => before.and_then(|body| body.title.clone()),
    };

    let mut summary = plural(widgets.len(), "widget", "widgets");
    if !variables.is_empty() {
        summary.push_str(&format!(" · {}", plural(variables.len(), "variable", "variables")));
    }
    let body = DashboardBody { widgets, variables, refresh_seconds, title, layout_version: Some(LAYOUT_VERSION) };
    Ok(Draft { name, body: DocumentBody::Dashboard(body), summary, previews, worst: StatementKind::Read, caveats: Vec::new() })
}

fn kind_name(kind: WidgetKind) -> String {
    serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// An enum read from its wire spelling; the caller has already checked the word is one of the allowed.
fn wire<T: DeserializeOwned>(word: &str, what: &str) -> AppResult<T> {
    serde_json::from_value::<T>(Json::String(word.to_string())).map_err(|err| AppError::invalid_input(format!("{what}: {err}")))
}

fn snap_refresh(requested: u64) -> u32 {
    REFRESH_CHOICES
        .iter()
        .copied()
        .min_by_key(|choice| u64::from(*choice).abs_diff(requested))
        .unwrap_or(0)
}

// ------------------------------------------------------------------ widgets

fn parse_widget(raw: &Json, index: usize, declared: &HashMap<&str, &DashboardVariable>) -> AppResult<Tile> {
    let title = required(raw, "title", "Every widget")?;
    let what = format!("Widget \"{title}\"");
    let kind_word = required(raw, "kind", &what)?.to_lowercase();
    // Dashboards saved before the rename spell a metric "number"; the model may too.
    let kind_word = if kind_word == "number" { "metric".to_string() } else { kind_word };
    if !WIDGET_KINDS.contains(&kind_word.as_str()) {
        return Err(AppError::invalid_input(format!("{what}: \"{kind_word}\" is not a widget kind. Use {}.", WIDGET_KINDS.join(", "))));
    }
    let kind: WidgetKind = wire(&kind_word, &what)?;

    let sql = text(raw, "sql").unwrap_or_default().to_string();
    if kind == WidgetKind::Text {
        if text(raw, "text").is_none() {
            return Err(AppError::invalid_input(format!("Text widget \"{title}\" needs a \"text\" body.")));
        }
    } else if sql.is_empty() {
        return Err(AppError::invalid_input(format!("{what} needs a non-empty \"sql\".")));
    }
    if !sql.is_empty() {
        check_read_only(title, &sql)?;
        check_placeholders(title, &sql, declared)?;
    }

    let mut widget = Widget {
        id: fresh_id("ai"),
        title: title.to_string(),
        kind,
        sql,
        tint: Some(match text(raw, "color") {
            Some(_) => swatch(raw, "color", &what)?.unwrap_or_default(),
            None => format!("series-{}", (index % 7) + 1),
        }),
        ..Widget::default()
    };

    widget.bg = swatch(raw, "background", &what)?;
    widget.x_label = text(raw, "xLabel").map(str::to_string);
    widget.y_label = text(raw, "yLabel").map(str::to_string);
    widget.text = text(raw, "text").map(str::to_string);
    widget.horizontal = flag(raw, "horizontal").unwrap_or(false);
    widget.stacked = flag(raw, "stacked").unwrap_or(kind == WidgetKind::Area);
    widget.show_change = flag(raw, "showChange").unwrap_or(false);
    widget.color_by_change = flag(raw, "colorByChange").unwrap_or(false);
    widget.show_percent = flag(raw, "showPercent").unwrap_or(true);
    widget.show_values = flag(raw, "showValues").unwrap_or(false);
    widget.pulse = flag(raw, "pulse").unwrap_or(false);
    widget.max_value = raw.get("maxValue").and_then(Json::as_f64).filter(|v| v.is_finite() && *v > 0.0);
    if let Some(word) = choice(raw, "legend", &["auto", "show", "hide"], &what)? {
        widget.legend_mode = wire::<LegendMode>(word, &what)?;
    }
    if let Some(word) = choice(raw, "valueFormat", &["number", "compact", "currency", "percent"], &what)? {
        widget.value_format = wire::<ValueFormat>(word, &what)?;
    }
    if let Some(word) = choice(raw, "format", &["number", "decimal", "currency", "percent"], &what)? {
        widget.format = wire::<MetricFormat>(word, &what)?;
    }
    if let Some(code) = text(raw, "currency") {
        let code = code.to_uppercase();
        if code.len() != 3 || !code.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(AppError::invalid_input(format!("{what}: \"{code}\" is not an ISO 4217 currency code (like USD or EUR).")));
        }
        widget.currency = code;
    }
    widget.conditions = parse_conditions(raw, &what)?;

    let (w, h) = footprint(kind, choice(raw, "size", &SIZES, &what)?, clamped(raw, "w", MIN_CELLS, GRID_COLUMNS), clamped(raw, "h", MIN_CELLS, MAX_ROWS));
    let at = match (raw.get("x").and_then(Json::as_u64), raw.get("y").and_then(Json::as_u64)) {
        (Some(x), Some(y)) => Some((u32::try_from(x).unwrap_or(0).min(GRID_COLUMNS - w), u32::try_from(y).unwrap_or(0))),
        _ => None,
    };
    Ok(Tile { widget, w, h, at })
}

/// A colour name from the tile registry (or the legacy `series-N` accent), or None when absent.
fn swatch(raw: &Json, key: &str, what: &str) -> AppResult<Option<String>> {
    let Some(word) = text(raw, key) else { return Ok(None) };
    let lower = word.to_lowercase();
    let series = lower.strip_prefix("series-").and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=7).contains(&n));
    if SWATCHES.contains(&lower.as_str()) || (key == "color" && series) {
        Ok(Some(lower))
    } else {
        Err(AppError::invalid_input(format!("{what}: \"{word}\" is not a {key}. Use {}.", SWATCHES.join(", "))))
    }
}

fn parse_conditions(raw: &Json, what: &str) -> AppResult<Vec<WidgetCondition>> {
    let Some(items) = raw.get("conditions").and_then(Json::as_array) else { return Ok(Vec::new()) };
    if items.len() > MAX_CONDITIONS {
        return Err(AppError::invalid_input(format!("{what} may have at most {MAX_CONDITIONS} conditions.")));
    }
    items
        .iter()
        .map(|item| {
            let op_word = choice(item, "op", &CONDITION_OPS, what)?
                .ok_or_else(|| AppError::invalid_input(format!("{what}: every condition needs an \"op\". Use {}.", CONDITION_OPS.join(", "))))?;
            let op: ConditionOp = wire(op_word, what)?;
            let value = item.get("value").and_then(Json::as_str).unwrap_or_default().to_string();
            if value.trim().is_empty() && !matches!(op, ConditionOp::IsNull | ConditionOp::IsNotNull) {
                return Err(AppError::invalid_input(format!("{what}: a \"{op_word}\" condition needs a \"value\".")));
            }
            Ok(WidgetCondition {
                op,
                value,
                content: item.get("content").and_then(Json::as_str).unwrap_or_default().to_string(),
                bg: swatch(item, "bg", what)?,
            })
        })
        .collect()
}

// WHAT:  A widget's query must be exactly one read.
// WHY:   It runs on every refresh with nobody watching.
fn check_read_only(title: &str, sql: &str) -> AppResult<()> {
    let statements = classify(sql);
    if statements.len() != 1 {
        return Err(AppError::invalid_input(format!(
            "Widget \"{title}\": give exactly one statement per widget (found {}).",
            statements.len()
        )));
    }
    if let Some(offender) = statements.iter().find(|s| s.kind != StatementKind::Read) {
        return Err(AppError::invalid_input(format!(
            "Widget \"{title}\": a dashboard query may only read data, and this one would change it: {}. Rewrite it as a SELECT.",
            preview(&offender.text)
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------- variables

fn parse_variables(args: &Json) -> AppResult<Vec<DashboardVariable>> {
    let Some(items) = args.get("variables").and_then(Json::as_array) else { return Ok(Vec::new()) };
    if items.len() > MAX_VARIABLES {
        return Err(AppError::invalid_input(format!("A dashboard may have at most {MAX_VARIABLES} variables.")));
    }
    let mut out: Vec<DashboardVariable> = Vec::new();
    for item in items {
        let variable = parse_variable(item)?;
        if out.iter().any(|seen| seen.name() == variable.name()) {
            return Err(AppError::invalid_input(format!("Variable \"{}\" is declared twice.", variable.name())));
        }
        out.push(variable);
    }
    Ok(out)
}

fn strings(item: &Json, key: &str) -> Vec<String> {
    item.get(key)
        .and_then(Json::as_array)
        .map(|list| list.iter().filter_map(Json::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

fn parse_variable(item: &Json) -> AppResult<DashboardVariable> {
    let name = required(item, "name", "Every variable")?;
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err(AppError::invalid_input(format!("Variable \"{name}\" must be letters, digits and underscores, and not start with a digit.")));
    }
    let what = format!("Variable \"{name}\"");
    let kind = choice(item, "type", &VARIABLE_TYPES, &what)?.unwrap_or("text");
    let value = item.get("value").and_then(Json::as_str).unwrap_or_default().trim().to_string();

    let typed = match kind {
        "date_range" => {
            let start = day(item, "start", &what)?;
            let end = day(item, "end", &what)?;
            if !start.is_empty() && !end.is_empty() && start > end {
                return Err(AppError::invalid_input(format!("{what}: the range starts after it ends.")));
            }
            json!({ "type": kind, "name": name, "start": start, "end": end })
        }
        "number" => {
            let number = if value.is_empty() {
                Json::Null
            } else {
                let parsed = value
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| AppError::invalid_input(format!("{what}: \"{value}\" is not a number.")))?;
                json!(parsed)
            };
            json!({ "type": kind, "name": name, "value": number })
        }
        "select" | "multi_select" => {
            let mut options: Vec<String> = Vec::new();
            for option in strings(item, "options") {
                if !options.contains(&option) {
                    options.push(option);
                }
            }
            if options.is_empty() {
                return Err(AppError::invalid_input(format!("{what} is a {kind}: give it the \"options\" to choose from.")));
            }
            if options.len() > MAX_OPTIONS {
                return Err(AppError::invalid_input(format!("{what} may have at most {MAX_OPTIONS} options.")));
            }
            if kind == "select" {
                if !value.is_empty() && !options.contains(&value) {
                    return Err(AppError::invalid_input(format!("{what}: \"{value}\" is not one of its options.")));
                }
                // A select with nothing chosen would hand its queries NULL; start on the first choice.
                let chosen = if value.is_empty() { options[0].clone() } else { value };
                json!({ "type": kind, "name": name, "value": chosen, "options": options })
            } else {
                // Nothing said means everything chosen, so the dashboard opens with data in it.
                let values = if item.get("values").is_some() { strings(item, "values") } else { options.clone() };
                if let Some(stray) = values.iter().find(|v| !options.contains(v)) {
                    return Err(AppError::invalid_input(format!("{what}: \"{stray}\" is not one of its options.")));
                }
                json!({ "type": kind, "name": name, "values": values, "options": options })
            }
        }
        "boolean" => {
            let on = match value.to_lowercase().as_str() {
                "" | "false" | "0" | "no" => false,
                "true" | "1" | "yes" => true,
                other => return Err(AppError::invalid_input(format!("{what}: \"{other}\" is not true or false."))),
            };
            json!({ "type": kind, "name": name, "value": on })
        }
        _ => json!({ "type": "text", "name": name, "value": value }),
    };
    serde_json::from_value::<DashboardVariable>(typed).map_err(|err| AppError::invalid_input(format!("{what} is not valid: {err}")))
}

/// A date bound: empty, `YYYY-MM-DD`, or a day with a time.
fn day(item: &Json, key: &str, what: &str) -> AppResult<String> {
    let value = item.get(key).and_then(Json::as_str).unwrap_or_default().trim().to_string();
    if value.is_empty() || is_day(&value) {
        return Ok(value);
    }
    Err(AppError::invalid_input(format!("{what}: \"{value}\" is not a date. Write it as YYYY-MM-DD.")))
}

fn is_day(value: &str) -> bool {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
        || ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"]
            .iter()
            .any(|format| chrono::NaiveDateTime::parse_from_str(value, format).is_ok())
}

/// A `{{name}}`, `{{name.start}}` or `{{name.end}}` token in a statement.
#[derive(Debug, PartialEq, Eq)]
struct Token {
    name: String,
    bound: Option<String>,
}

/// Every variable-shaped token in a statement. Other `{{…}}` spans (a JSON
/// document, an expression) are not variables and are left alone.
fn tokens(sql: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut rest = sql;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let inner = after[..end].trim();
        let (name, bound) = match inner.split_once('.') {
            Some((name, bound)) => (name.trim(), Some(bound.trim().to_string())),
            None => (inner, None),
        };
        let identifier = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if identifier && bound.as_deref().is_none_or(|b| b.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
            let token = Token { name: name.to_string(), bound };
            if !out.contains(&token) {
                out.push(token);
            }
        }
        rest = &after[end + 2..];
    }
    out
}

fn check_placeholders(title: &str, sql: &str, declared: &HashMap<&str, &DashboardVariable>) -> AppResult<()> {
    for token in tokens(sql) {
        let Some(variable) = declared.get(token.name.as_str()) else {
            return Err(AppError::invalid_input(format!(
                "Widget \"{title}\" uses {{{{{}}}}} but no variable of that name is declared. Add it to \"variables\" or write the value into the query.",
                token.name
            )));
        };
        if let Some(bound) = &token.bound {
            let is_range = matches!(variable, DashboardVariable::DateRange { .. });
            if !is_range || (bound != "start" && bound != "end") {
                return Err(AppError::invalid_input(format!(
                    "Widget \"{title}\": {{{{{}.{bound}}}}} is only valid for a date_range variable, as .start or .end.",
                    token.name
                )));
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------- layout

/// Width and height on the 12-column, 60 px-row grid. Explicit cells win, then
/// a size word, then what the kind needs.
fn footprint(kind: WidgetKind, size: Option<&str>, w: Option<u32>, h: Option<u32>) -> (u32, u32) {
    let (kw, kh) = match size {
        Some("small") => (3, 3),
        Some("medium") => (6, 5),
        Some("large") => (8, 6),
        Some("full") => (GRID_COLUMNS, 6),
        _ => match kind {
            WidgetKind::Metric | WidgetKind::Sparkline | WidgetKind::Progress => (3, 3),
            WidgetKind::Text => (6, 3),
            WidgetKind::Table | WidgetKind::Sankey => (GRID_COLUMNS, 6),
            _ => (6, 5),
        },
    };
    (w.unwrap_or(kw).clamp(MIN_CELLS, GRID_COLUMNS), h.unwrap_or(kh).clamp(MIN_CELLS, MAX_ROWS))
}

/// Left-to-right, top-to-bottom packing: a widget that does not fit wraps to a new row.
#[derive(Default)]
struct GridCursor {
    x: u32,
    y: u32,
    row_height: u32,
}

impl GridCursor {
    fn place(&mut self, w: u32, h: u32) -> (u32, u32) {
        let w = w.min(GRID_COLUMNS);
        if self.x + w > GRID_COLUMNS {
            self.y += self.row_height;
            self.x = 0;
            self.row_height = 0;
        }
        let spot = (self.x, self.y);
        self.x += w;
        self.row_height = self.row_height.max(h);
        spot
    }
}

// WHAT:  Where each widget goes.
// WHY:   The model's own coordinates are honoured only when it gave every widget
//        a spot — that is an edit of an existing layout. Mixing its coordinates
//        with automatic ones would leave tiles on top of each other. (The editor
//        also untangles overlaps when it opens a dashboard.)
fn place(sized: &[Tile]) -> Vec<(u32, u32)> {
    if sized.iter().all(|tile| tile.at.is_some()) {
        return sized.iter().filter_map(|tile| tile.at).collect();
    }
    let mut cursor = GridCursor::default();
    sized.iter().map(|tile| cursor.place(tile.w, tile.h)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(args: &Json) -> Draft {
        parse(args, None).unwrap_or_else(|e| panic!("{e}"))
    }

    fn body(draft: &Draft) -> DashboardBody {
        match &draft.body {
            DocumentBody::Dashboard(body) => body.clone(),
            _ => panic!("not a dashboard"),
        }
    }

    fn metric(title: &str, sql: &str) -> Json {
        json!({ "title": title, "kind": "metric", "sql": sql })
    }

    #[test]
    fn a_dashboard_is_written_on_the_current_grid_and_laid_out_in_rows() {
        let draft = draft(&json!({
            "name": "Sales",
            "widgets": [
                metric("Orders", "SELECT count(*) FROM orders"),
                metric("Revenue", "SELECT sum(total) FROM orders"),
                { "title": "Per day", "kind": "line", "sql": "SELECT day, n FROM daily" },
                { "title": "By region", "kind": "pie", "sql": "SELECT region, n FROM r" },
                { "title": "Recent", "kind": "table", "sql": "SELECT * FROM orders LIMIT 10" },
            ]
        }));
        assert_eq!(draft.summary, "5 widgets");
        assert_eq!(draft.worst, StatementKind::Read);
        let body = body(&draft);
        assert_eq!(body.layout_version, Some(1), "without it the editor re-lays the tiles out as the old flow layout");
        let cell = |i: usize| (body.widgets[i].x, body.widgets[i].y, body.widgets[i].w, body.widgets[i].h);
        assert_eq!(cell(0), (0, 0, 3, 3));
        assert_eq!(cell(1), (3, 0, 3, 3));
        // The line chart is 6 wide: 6 columns are used, 6 remain, so it still fits on row one.
        assert_eq!(cell(2), (6, 0, 6, 5));
        // The pie no longer fits and wraps below the tallest widget of the row.
        assert_eq!(cell(3), (0, 5, 6, 5));
        // The table needs the whole width, so it starts a new row again.
        assert_eq!(cell(4), (0, 10, 12, 6));
        assert!(body.widgets[0].id.starts_with("ai-"));
        assert_ne!(body.widgets[0].id, body.widgets[1].id);
        assert_eq!(body.widgets[0].tint.as_deref(), Some("series-1"));
        assert_eq!(body.widgets[2].tint.as_deref(), Some("series-3"));
    }

    #[test]
    fn dashboard_queries_must_be_single_reads() {
        let write = json!({ "name": "x", "widgets": [metric("Bad", "DELETE FROM orders WHERE id = 1")] });
        let err = parse(&write, None).err().map(|e| e.message().to_string()).unwrap_or_default();
        assert!(err.contains("only read"), "{err}");

        let many = json!({ "name": "x", "widgets": [metric("Two", "SELECT 1; SELECT 2")] });
        assert!(parse(&many, None).is_err());

        // A read that merely mentions a write keyword in a string is still a read.
        let fine = json!({ "name": "x", "widgets": [metric("Ok", "SELECT 'DELETE FROM t' AS note")] });
        assert!(parse(&fine, None).is_ok());
    }

    #[test]
    fn dashboard_input_is_validated() {
        assert!(parse(&json!({ "widgets": [] }), None).is_err(), "name");
        assert!(parse(&json!({ "name": "x", "widgets": [] }), None).is_err(), "no widgets");
        let unknown = json!({ "name": "x", "widgets": [{ "title": "t", "kind": "gauge", "sql": "SELECT 1" }] });
        assert!(parse(&unknown, None).is_err());
        let image = json!({ "name": "x", "widgets": [{ "title": "t", "kind": "image", "sql": "SELECT 1" }] });
        assert!(parse(&image, None).is_err(), "image tiles need an outside URL");
        let no_sql = json!({ "name": "x", "widgets": [{ "title": "t", "kind": "line" }] });
        assert!(parse(&no_sql, None).is_err());
        let text_only = json!({ "name": "x", "widgets": [{ "title": "Notes", "kind": "text", "text": "Hello" }] });
        assert!(parse(&text_only, None).is_ok());
        let text_empty = json!({ "name": "x", "widgets": [{ "title": "Notes", "kind": "text" }] });
        assert!(parse(&text_empty, None).is_err());
        let too_many: Vec<Json> = (0..=MAX_WIDGETS).map(|i| metric(&format!("w{i}"), "SELECT 1")).collect();
        assert!(parse(&json!({ "name": "x", "widgets": too_many }), None).is_err());
        // The old spelling of a metric is still understood.
        let legacy = json!({ "name": "x", "widgets": [{ "title": "n", "kind": "number", "sql": "SELECT 1" }] });
        assert_eq!(body(&draft(&legacy)).widgets[0].kind, WidgetKind::Metric);
    }

    #[test]
    fn widget_options_reach_the_widget_and_bad_ones_are_refused() {
        let args = json!({ "name": "x", "widgets": [{
            "title": "Revenue", "kind": "metric", "sql": "SELECT 1, 2",
            "format": "currency", "currency": "eur", "showChange": true, "colorByChange": true,
            "background": "indigo", "color": "teal",
            "conditions": [{ "op": "lt", "value": "100", "bg": "red" }, { "op": "is_null" }]
        }, {
            "title": "Trend", "kind": "area", "sql": "SELECT d, n FROM t",
            "legend": "hide", "valueFormat": "compact", "stacked": false, "size": "large"
        }]});
        let widgets = body(&draft(&args)).widgets;
        assert_eq!(widgets[0].format, MetricFormat::Currency);
        assert_eq!(widgets[0].currency, "EUR");
        assert!(widgets[0].show_change && widgets[0].color_by_change);
        assert_eq!((widgets[0].bg.as_deref(), widgets[0].tint.as_deref()), (Some("indigo"), Some("teal")));
        assert_eq!(widgets[0].conditions.len(), 2);
        assert_eq!(widgets[0].conditions[0].bg.as_deref(), Some("red"));
        assert_eq!(widgets[1].legend_mode, LegendMode::Hide);
        assert_eq!(widgets[1].value_format, ValueFormat::Compact);
        assert!(!widgets[1].stacked);
        assert_eq!((widgets[1].w, widgets[1].h), (8, 6));

        let bad = |widget: Json| parse(&json!({ "name": "x", "widgets": [widget] }), None).is_err();
        assert!(bad(json!({ "title": "t", "kind": "metric", "sql": "SELECT 1", "currency": "dollars" })));
        assert!(bad(json!({ "title": "t", "kind": "metric", "sql": "SELECT 1", "format": "hex" })));
        assert!(bad(json!({ "title": "t", "kind": "metric", "sql": "SELECT 1", "color": "chartreuse" })));
        assert!(bad(json!({ "title": "t", "kind": "metric", "sql": "SELECT 1", "conditions": [{ "op": "gt" }] })));
        assert!(bad(json!({ "title": "t", "kind": "metric", "sql": "SELECT 1", "conditions": [{ "op": "matches", "value": "1" }] })));
    }

    #[test]
    fn an_area_chart_stacks_unless_told_not_to() {
        let area = json!({ "name": "x", "widgets": [{ "title": "t", "kind": "area", "sql": "SELECT d, n FROM t" }] });
        assert!(body(&draft(&area)).widgets[0].stacked);
        let line = json!({ "name": "x", "widgets": [{ "title": "t", "kind": "line", "sql": "SELECT d, n FROM t" }] });
        assert!(!body(&draft(&line)).widgets[0].stacked);
    }

    #[test]
    fn explicit_cells_are_kept_only_when_every_widget_has_a_spot() {
        let spot = |title: &str, x: u64, y: u64| json!({ "title": title, "kind": "metric", "sql": "SELECT 1", "x": x, "y": y, "w": 4, "h": 3 });
        let all = body(&draft(&json!({ "name": "x", "widgets": [spot("a", 8, 2), spot("b", 0, 0)] })));
        assert_eq!((all.widgets[0].x, all.widgets[0].y, all.widgets[0].w), (8, 2, 4));
        assert_eq!((all.widgets[1].x, all.widgets[1].y), (0, 0));
        let wide = body(&draft(&json!({ "name": "x", "widgets": [spot("a", 11, 0)] })));
        assert_eq!(wide.widgets[0].x, 8, "a tile is pulled back inside the 12 columns");

        let mixed = body(&draft(&json!({ "name": "x", "widgets": [spot("a", 8, 2), metric("b", "SELECT 1")] })));
        assert_eq!((mixed.widgets[0].x, mixed.widgets[0].y), (0, 0), "a partial layout is ignored, not mixed with the automatic one");
    }

    #[test]
    fn variables_are_typed_validated_and_must_be_declared_before_a_query_uses_them() {
        let missing = json!({ "name": "x", "widgets": [metric("t", "SELECT {{since}}")] });
        let err = parse(&missing, None).err().map(|e| e.message().to_string()).unwrap_or_default();
        assert!(err.contains("since"), "{err}");

        let declared = json!({
            "name": "x",
            "variables": [
                { "name": "period", "type": "date_range", "start": "2026-01-01", "end": "2026-01-31" },
                { "name": "region", "type": "select", "options": ["eu", "us"] },
                { "name": "tags", "type": "multi_select", "options": ["a", "b"], "values": ["a"] },
                { "name": "limit", "type": "number", "value": "25" },
                { "name": "active", "type": "boolean", "value": "true" },
                { "name": "note", "value": "hello" },
            ],
            "widgets": [metric("t", "SELECT count(*) FROM o WHERE d BETWEEN {{period}} AND x AND created >= {{period.start}} AND r = {{region}} AND t IN ({{tags}}) LIMIT {{limit}}")]
        });
        let draft = draft(&declared);
        assert_eq!(draft.summary, "1 widget · 6 variables");
        let variables = body(&draft).variables;
        assert_eq!(variables[0], DashboardVariable::DateRange { name: "period".into(), start: "2026-01-01".into(), end: "2026-01-31".into() });
        assert_eq!(variables[1], DashboardVariable::Select { name: "region".into(), value: "eu".into(), options: vec!["eu".into(), "us".into()] });
        assert_eq!(variables[2], DashboardVariable::MultiSelect { name: "tags".into(), values: vec!["a".into()], options: vec!["a".into(), "b".into()] });
        assert_eq!(variables[3], DashboardVariable::Number { name: "limit".into(), value: Some(25.0) });
        assert_eq!(variables[4], DashboardVariable::Boolean { name: "active".into(), value: true });
        assert_eq!(variables[5], DashboardVariable::Text { name: "note".into(), value: "hello".into() }, "an untyped variable is text");
    }

    #[test]
    fn a_multi_select_with_no_values_starts_with_everything_chosen() {
        let args = json!({ "name": "x", "variables": [{ "name": "tags", "type": "multi_select", "options": ["a", "b"] }], "widgets": [metric("t", "SELECT 1 WHERE x IN ({{tags}})")] });
        assert_eq!(
            body(&draft(&args)).variables,
            vec![DashboardVariable::MultiSelect { name: "tags".into(), values: vec!["a".into(), "b".into()], options: vec!["a".into(), "b".into()] }]
        );
        let none = json!({ "name": "x", "variables": [{ "name": "tags", "type": "multi_select", "options": ["a"], "values": [] }], "widgets": [metric("t", "SELECT 1")] });
        assert_eq!(
            body(&draft(&none)).variables,
            vec![DashboardVariable::MultiSelect { name: "tags".into(), values: Vec::new(), options: vec!["a".into()] }],
            "an explicit empty choice is respected"
        );
    }

    #[test]
    fn bad_variables_are_refused() {
        let with = |variable: Json| parse(&json!({ "name": "x", "variables": [variable], "widgets": [{ "title": "t", "kind": "text", "text": "a" }] }), None);
        assert!(with(json!({ "name": "1bad" })).is_err());
        assert!(with(json!({ "name": "ok", "type": "color" })).is_err());
        assert!(with(json!({ "name": "ok", "type": "number", "value": "many" })).is_err());
        assert!(with(json!({ "name": "ok", "type": "select" })).is_err(), "a select needs options");
        assert!(with(json!({ "name": "ok", "type": "select", "options": ["a"], "value": "b" })).is_err());
        assert!(with(json!({ "name": "ok", "type": "multi_select", "options": ["a"], "values": ["z"] })).is_err());
        assert!(with(json!({ "name": "ok", "type": "date_range", "start": "next week" })).is_err());
        assert!(with(json!({ "name": "ok", "type": "date_range", "start": "2026-02-01", "end": "2026-01-01" })).is_err());
        assert!(with(json!({ "name": "ok", "type": "boolean", "value": "maybe" })).is_err());
        let twice = json!({ "name": "x", "variables": [{ "name": "a" }, { "name": "a" }], "widgets": [{ "title": "t", "kind": "text", "text": "a" }] });
        assert!(parse(&twice, None).is_err());
    }

    #[test]
    fn a_bound_is_only_valid_on_a_date_range() {
        let on_text = json!({ "name": "x", "variables": [{ "name": "r" }], "widgets": [metric("t", "SELECT {{r.start}}")] });
        assert!(parse(&on_text, None).is_err());
        let wrong_bound = json!({ "name": "x", "variables": [{ "name": "p", "type": "date_range" }], "widgets": [metric("t", "SELECT {{p.middle}}")] });
        assert!(parse(&wrong_bound, None).is_err());
        let ok = json!({ "name": "x", "variables": [{ "name": "p", "type": "date_range" }], "widgets": [metric("t", "SELECT {{ p.end }}")] });
        assert!(parse(&ok, None).is_ok());
    }

    #[test]
    fn tokens_are_found_once_each_and_other_braces_are_not_variables() {
        let names: Vec<String> = tokens("a {{x}} b {{ y.start }} c {{x}}").into_iter().map(|t| format!("{}{}", t.name, t.bound.map(|b| format!(".{b}")).unwrap_or_default())).collect();
        assert_eq!(names, vec!["x", "y.start"]);
        assert!(tokens("no braces {{unclosed").is_empty());
        assert!(tokens(r#"{"$match": {{"a": 1}}}"#).is_empty(), "a JSON document is not a variable");
        assert!(tokens("{{i * 100}}").is_empty(), "an expression is not a variable");
    }

    #[test]
    fn an_update_keeps_what_the_model_left_out() {
        let saved = DashboardBody {
            variables: vec![DashboardVariable::Text { name: "region".into(), value: "eu".into() }],
            refresh_seconds: 30,
            title: Some("Sales overview".into()),
            layout_version: Some(1),
            ..DashboardBody::default()
        };
        let previous = DocumentBody::Dashboard(saved);
        let args = json!({ "name": "Sales", "widgets": [metric("t", "SELECT count(*) FROM o WHERE r = {{region}}")] });
        let kept = match parse(&args, Some(&previous)).unwrap_or_else(|e| panic!("{e}")).body {
            DocumentBody::Dashboard(body) => body,
            _ => panic!("not a dashboard"),
        };
        assert_eq!(kept.variables.len(), 1, "the saved variables stay, so a query can still use them");
        assert_eq!((kept.refresh_seconds, kept.title.as_deref()), (30, Some("Sales overview")));

        let changed = json!({ "name": "Sales", "variables": [], "refreshSeconds": 7, "title": "New", "widgets": [metric("t", "SELECT 1")] });
        let replaced = match parse(&changed, Some(&previous)).unwrap_or_else(|e| panic!("{e}")).body {
            DocumentBody::Dashboard(body) => body,
            _ => panic!("not a dashboard"),
        };
        assert!(replaced.variables.is_empty(), "an explicit empty list removes them");
        assert_eq!(replaced.refresh_seconds, 5, "snapped to the nearest choice the editor offers");
        assert_eq!(replaced.title.as_deref(), Some("New"));
    }

    #[test]
    fn refresh_snaps_to_what_the_editor_offers() {
        assert_eq!(snap_refresh(0), 0);
        assert_eq!(snap_refresh(7), 5);
        assert_eq!(snap_refresh(45), 30);
        assert_eq!(snap_refresh(100_000), 300);
    }

    #[test]
    fn grid_cursor_never_overflows_the_row() {
        let mut cursor = GridCursor::default();
        assert_eq!(cursor.place(12, 4), (0, 0));
        assert_eq!(cursor.place(3, 2), (0, 4));
        assert_eq!(cursor.place(20, 2), (0, 6), "an over-wide widget is clamped, not placed off-grid");
    }
}
