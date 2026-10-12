// SOT: agent-diagram-builder, ai-diagram-schema, diagram-draft-validation, diagram-layered-layout, diagram-from-schema

use super::{choice, clean_name, flag, fresh_id, list, plural, properties, required, slug, text, Draft};
use crate::error::{AppError, AppResult};
use crate::guard::destructive::StatementKind;
use crate::guard::SessionCtx;
use crate::model::{
    ConnectionColor, DiagramBody, DiagramColumn, DiagramColumnPair, DiagramNote, DiagramRelation, DiagramTable, Document,
    DocumentBody, DocumentKind, ReferentialAction, TableKind, TableRef,
};
use crate::services;
use crate::services::agent::provider::{tool, ToolSpec};
use serde_json::{json, Value as Json};
use std::collections::{HashMap, HashSet};

// WHAT:  `create_diagram`: what the model is told a schema diagram is, the check
//        that turns its answer into a real `DiagramBody`, the table layout, and
//        the diagram built from the live catalogue ("Open in designer").
// WHY:   Relations name tables and columns by text; one that points nowhere would
//        draw as a dangling edge, so each endpoint is resolved (ignoring case)
//        against what the diagram really holds. The designer's cards are a fixed
//        size, so the layout can be computed without drawing anything.
// HOW:   Tables are laid out in columns by foreign-key depth (a table that
//        references nothing on the left, the tables that point at it to its
//        right), the way the ER diagram's own layout does. On an update, what
//        the user did to the diagram by hand — positions, colours, line styles,
//        notes — survives wherever the model did not say otherwise.
// WHERE: ./mod.rs (the shared create/update flow), src/features/diagrams (the designer)

const MAX_TABLES: usize = 40;
const MAX_COLUMNS: usize = 80;
const MAX_RELATIONS: usize = 120;
const MAX_NOTES: usize = 10;
const MAX_NAME: usize = 120;
/// Tables a diagram built from the catalogue may hold.
const MAX_SCHEMA_TABLES: usize = 60;

// Card geometry (src/features/diagrams/metrics.ts).
const TABLE_WIDTH: f64 = 260.0;
const HEADER_HEIGHT: f64 = 30.0;
const ROW_HEIGHT: f64 = 22.0;
const EMPTY_HEIGHT: f64 = 24.0;
const BORDER: f64 = 2.0;
/// Room between two cards for the relation lines that run between them.
const COLUMN_GAP: f64 = 110.0;
const CARD_GAP: f64 = 56.0;
const ORIGIN: f64 = 60.0;
/// A stack of cards taller than this continues in the next column.
const MAX_STACK_HEIGHT: f64 = 1_400.0;
const NOTE_HEIGHT: f64 = 120.0;

const COLORS: [&str; 9] = ["blue", "teal", "green", "amber", "orange", "red", "pink", "violet", "gray"];
const ACTIONS: [&str; 5] = ["no_action", "restrict", "cascade", "set_null", "set_default"];

// ---------------------------------------------------------------------------
// What the model is told
// ---------------------------------------------------------------------------

pub fn diagram_tool(engine: &str) -> ToolSpec {
    tool(
        "create_diagram",
        &format!(
            "Design a schema diagram — tables with their columns and the relations between them — and save \
             it for the user to open in the designer, where they can edit it and generate the SQL. Use it to \
             propose a new schema, document an existing one, or sketch a change. Use {engine} column types. \
             The user reviews and approves before it is saved. To change a diagram the user @-mentioned, or \
             whose document id they quoted, pass its documentId; the new tables REPLACE the old ones (keep a \
             table's name to keep its position and colour; notes are kept when you leave them out). Nothing \
             is changed in the database. A relation is a foreign key: fromTable.fromColumn references \
             toTable.toColumn; put the key on the table that holds it."
        ),
        json!({
            "type": "object",
            "properties": properties(vec![
                ("name", json!({ "type": "string", "description": "Diagram name." })),
                ("documentId", json!({ "type": "string", "description": "Id of an existing diagram to replace, from an @mention or an id the user quoted. Omit to create a new one." })),
                ("tables", json!({ "type": "array", "minItems": 1, "maxItems": MAX_TABLES, "items": table_schema(engine) })),
                ("relations", json!({ "type": "array", "maxItems": MAX_RELATIONS, "items": relation_schema() })),
                ("notes", json!({
                    "type": "array",
                    "maxItems": MAX_NOTES,
                    "description": "Sticky notes explaining a design decision. They replace the existing notes.",
                    "items": {
                        "type": "object",
                        "properties": properties(vec![
                            ("text", json!({ "type": "string" })),
                            ("color", json!({ "type": "string", "enum": COLORS })),
                        ]),
                        "required": ["text"],
                        "additionalProperties": false
                    }
                })),
            ]),
            "required": ["name", "tables"],
            "additionalProperties": false
        }),
    )
}

fn table_schema(engine: &str) -> Json {
    json!({
        "type": "object",
        "properties": properties(vec![
            ("name", json!({ "type": "string" })),
            ("schema", json!({ "type": "string", "description": "Schema the table lives in. Omit for the connection's default." })),
            ("comment", json!({ "type": "string", "description": "What the table is for." })),
            ("color", json!({ "type": "string", "enum": COLORS, "description": "Header colour, to group related tables." })),
            ("columns", json!({ "type": "array", "minItems": 1, "maxItems": MAX_COLUMNS, "items": column_schema(engine) })),
        ]),
        "required": ["name", "columns"],
        "additionalProperties": false
    })
}

fn column_schema(engine: &str) -> Json {
    json!({
        "type": "object",
        "properties": properties(vec![
            ("name", json!({ "type": "string" })),
            ("type", json!({ "type": "string", "description": format!("A {engine} column type.") })),
            ("primaryKey", json!({ "type": "boolean" })),
            ("nullable", json!({ "type": "boolean", "description": "Defaults to true; primary keys are never nullable." })),
            ("unique", json!({ "type": "boolean" })),
            ("autoIncrement", json!({ "type": "boolean" })),
            ("defaultValue", json!({ "type": "string", "description": "A SQL expression, written after DEFAULT as it is." })),
            ("comment", json!({ "type": "string" })),
        ]),
        "required": ["name", "type"],
        "additionalProperties": false
    })
}

fn relation_schema() -> Json {
    json!({
        "type": "object",
        "properties": properties(vec![
            ("fromTable", json!({ "type": "string", "description": "The table that holds the foreign key (schema.table when two schemas share a name)." })),
            ("fromColumn", json!({ "type": "string" })),
            ("toTable", json!({ "type": "string", "description": "The table it references." })),
            ("toColumn", json!({ "type": "string" })),
            ("extraColumns", json!({
                "type": "array",
                "description": "More column pairs of a composite key.",
                "items": {
                    "type": "object",
                    "properties": properties(vec![
                        ("fromColumn", json!({ "type": "string" })),
                        ("toColumn", json!({ "type": "string" })),
                    ]),
                    "required": ["fromColumn", "toColumn"],
                    "additionalProperties": false
                }
            })),
            ("name", json!({ "type": "string", "description": "Constraint name. Generated when omitted." })),
            ("onDelete", json!({ "type": "string", "enum": ACTIONS })),
            ("onUpdate", json!({ "type": "string", "enum": ACTIONS })),
        ]),
        "required": ["fromTable", "fromColumn", "toTable", "toColumn"],
        "additionalProperties": false
    })
}

// ---------------------------------------------------------------------------
// What the model's answer becomes
// ---------------------------------------------------------------------------

// WHAT:  A diagram proposal → a validated `DiagramBody`, laid out in columns.
pub(super) fn parse(args: &Json, previous: Option<&DocumentBody>) -> AppResult<Draft> {
    let what = "A diagram";
    let name = clean_name(args, what)?;
    let raw_tables = list(args, "tables", what, 1, MAX_TABLES)?;
    let before = match previous {
        Some(DocumentBody::Diagram(body)) => Some(body),
        _ => None,
    };

    let mut tables: Vec<DiagramTable> = Vec::with_capacity(raw_tables.len());
    let mut ids: HashSet<String> = HashSet::new();
    for raw in raw_tables {
        let table = parse_table(raw, &mut ids, before)?;
        tables.push(table);
    }

    let relations = parse_relations(args, &tables, before)?;
    arrange(&mut tables, &relations, before);

    let notes = match args.get("notes") {
        Some(_) => parse_notes(args, &tables)?,
        None => before.map(|body| body.notes.clone()).unwrap_or_default(),
    };

    let previews: Vec<String> = tables
        .iter()
        .map(|table| {
            let names: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).take(8).collect();
            let more = if table.columns.len() > 8 { ", …" } else { "" };
            let shown = match &table.schema {
                Some(schema) => format!("{schema}.{}", table.name),
                None => table.name.clone(),
            };
            format!("{shown} ({}{more})", names.join(", "))
        })
        .collect();
    let mut summary = format!("{} · {}", plural(tables.len(), "table", "tables"), plural(relations.len(), "relation", "relations"));
    if !notes.is_empty() && args.get("notes").is_some() {
        summary.push_str(&format!(" · {}", plural(notes.len(), "note", "notes")));
    }
    let body = DiagramBody {
        tables,
        relations,
        notes,
        grid_size: before.and_then(|b| b.grid_size),
        notes_visible: before.and_then(|b| b.notes_visible),
    };
    Ok(Draft { name, body: DocumentBody::Diagram(body), summary, previews, worst: StatementKind::Read, caveats: Vec::new() })
}

fn parse_table(raw: &Json, ids: &mut HashSet<String>, before: Option<&DiagramBody>) -> AppResult<DiagramTable> {
    let table_name = required(raw, "name", "Every table")?;
    let what = format!("Table \"{table_name}\"");
    let schema = text(raw, "schema").map(str::to_string);
    let columns_raw = list(raw, "columns", &what, 1, MAX_COLUMNS)?;

    let key = match &schema {
        Some(schema) => format!("{schema}.{table_name}"),
        None => table_name.to_string(),
    };
    let base = slug(&key);
    let mut id = base.clone();
    let mut n = 2;
    while !ids.insert(id.clone()) {
        id = format!("{base}-{n}");
        n += 1;
    }

    let mut seen = HashSet::new();
    let mut columns = Vec::with_capacity(columns_raw.len());
    for column in columns_raw {
        let column_name = required(column, "name", &format!("Every column of \"{table_name}\""))?;
        if !seen.insert(column_name.to_lowercase()) {
            return Err(AppError::invalid_input(format!("{what} has the column \"{column_name}\" twice.")));
        }
        let data_type = required(column, "type", &format!("Column \"{table_name}.{column_name}\""))?;
        let primary_key = flag(column, "primaryKey").unwrap_or(false);
        columns.push(DiagramColumn {
            name: column_name.to_string(),
            data_type: data_type.to_string(),
            primary_key,
            nullable: !primary_key && flag(column, "nullable").unwrap_or(true),
            unique: flag(column, "unique").unwrap_or(false),
            auto_increment: flag(column, "autoIncrement").unwrap_or(false),
            default_value: text(column, "defaultValue").map(str::to_string),
            comment: text(column, "comment").map(str::to_string),
        });
    }

    // What the user did to this table by hand (colour, a comment) outlives an edit that does not mention it.
    let earlier = before.and_then(|body| body.tables.iter().find(|t| t.id == id));
    let color = match choice(raw, "color", &COLORS, &what)? {
        Some(word) => ConnectionColor::parse(word),
        None => earlier.and_then(|t| t.color),
    };
    let comment = text(raw, "comment").map(str::to_string).or_else(|| earlier.and_then(|t| t.comment.clone()));
    Ok(DiagramTable { id, name: table_name.to_string(), schema, columns, color, comment, ..DiagramTable::default() })
}

/// Which table a relation's text names: `schema.table` exactly, else a bare name that only one table has.
fn find_table<'a>(tables: &'a [DiagramTable], reference: &str) -> AppResult<&'a DiagramTable> {
    let wanted = reference.trim().to_lowercase();
    let qualified = |table: &DiagramTable| match &table.schema {
        Some(schema) => format!("{schema}.{}", table.name).to_lowercase(),
        None => table.name.to_lowercase(),
    };
    if let Some(exact) = tables.iter().find(|table| qualified(table) == wanted) {
        return Ok(exact);
    }
    let bare: Vec<&DiagramTable> = tables.iter().filter(|table| table.name.to_lowercase() == wanted).collect();
    match bare.as_slice() {
        [only] => Ok(only),
        [] => Err(AppError::invalid_input(format!("A relation names the table \"{reference}\", which is not in the diagram."))),
        _ => Err(AppError::invalid_input(format!(
            "A relation names \"{reference}\", which exists in several schemas. Write it as schema.table."
        ))),
    }
}

/// A column's exact spelling in a table, ignoring case.
fn find_column<'a>(table: &'a DiagramTable, column: &str) -> AppResult<&'a str> {
    table
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .find(|name| name.eq_ignore_ascii_case(column.trim()))
        .ok_or_else(|| {
            AppError::invalid_input(format!("A relation names the column \"{}.{column}\", which the table does not have.", table.name))
        })
}

fn action(raw: &Json, key: &str, what: &str) -> AppResult<Option<ReferentialAction>> {
    let Some(word) = text(raw, key) else { return Ok(None) };
    let normal = word.to_lowercase().replace([' ', '-'], "_");
    if !ACTIONS.contains(&normal.as_str()) {
        return Err(AppError::invalid_input(format!("{what}: \"{word}\" is not a referential action. Use {}.", ACTIONS.join(", "))));
    }
    serde_json::from_value::<ReferentialAction>(Json::String(normal)).map(Some).map_err(|err| AppError::invalid_input(format!("{what}: {err}")))
}

fn parse_relations(args: &Json, tables: &[DiagramTable], before: Option<&DiagramBody>) -> AppResult<Vec<DiagramRelation>> {
    let Some(items) = args.get("relations").and_then(Json::as_array) else { return Ok(Vec::new()) };
    if items.len() > MAX_RELATIONS {
        return Err(AppError::invalid_input(format!("A diagram may have at most {MAX_RELATIONS} relations.")));
    }
    let mut out: Vec<DiagramRelation> = Vec::with_capacity(items.len());
    for item in items {
        let from = find_table(tables, required(item, "fromTable", "Every relation")?)?;
        let to = find_table(tables, required(item, "toTable", "Every relation")?)?;
        let what = format!("Relation {} → {}", from.name, to.name);
        let from_column = find_column(from, required(item, "fromColumn", &what)?)?.to_string();
        let to_column = find_column(to, required(item, "toColumn", &what)?)?.to_string();

        let mut extra_columns = Vec::new();
        if let Some(pairs) = item.get("extraColumns").and_then(Json::as_array) {
            for pair in pairs {
                extra_columns.push(DiagramColumnPair {
                    from_column: find_column(from, required(pair, "fromColumn", &what)?)?.to_string(),
                    to_column: find_column(to, required(pair, "toColumn", &what)?)?.to_string(),
                });
            }
        }
        if out.iter().any(|r| r.from_table == from.id && r.to_table == to.id && r.from_column == from_column && r.to_column == to_column) {
            return Err(AppError::invalid_input(format!("{what} ({from_column} → {to_column}) is listed twice.")));
        }

        let mut relation = DiagramRelation {
            id: fresh_id("rel"),
            from_table: from.id.clone(),
            from_column,
            to_table: to.id.clone(),
            to_column,
            extra_columns,
            name: text(item, "name").map(|n| n.chars().take(MAX_NAME).collect()),
            on_delete: action(item, "onDelete", &what)?,
            on_update: action(item, "onUpdate", &what)?,
            ..DiagramRelation::default()
        };
        // The same foreign key as before keeps the look the user gave its line.
        let earlier = before.and_then(|body| {
            body.relations.iter().find(|r| {
                r.from_table == relation.from_table && r.to_table == relation.to_table && r.from_column == relation.from_column && r.to_column == relation.to_column
            })
        });
        if let Some(old) = earlier {
            relation.id = old.id.clone();
            relation.color = old.color;
            relation.width = old.width;
            relation.from_side = old.from_side;
            relation.to_side = old.to_side;
            relation.waypoints = old.waypoints.clone();
            relation.name = relation.name.or_else(|| old.name.clone());
            relation.on_delete = relation.on_delete.or(old.on_delete);
            relation.on_update = relation.on_update.or(old.on_update);
        }
        out.push(relation);
    }
    Ok(out)
}

fn parse_notes(args: &Json, tables: &[DiagramTable]) -> AppResult<Vec<DiagramNote>> {
    let Some(items) = args.get("notes").and_then(Json::as_array) else { return Ok(Vec::new()) };
    if items.len() > MAX_NOTES {
        return Err(AppError::invalid_input(format!("A diagram may have at most {MAX_NOTES} notes.")));
    }
    // Notes sit in a column to the right of the tables, top to bottom.
    let right = tables.iter().map(|t| t.x).fold(ORIGIN, f64::max) + TABLE_WIDTH + COLUMN_GAP;
    let top = tables.iter().map(|t| t.y).fold(f64::MAX, f64::min).min(ORIGIN);
    let mut notes = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let body = required(item, "text", "Every note")?;
        let color = choice(item, "color", &COLORS, "A note")?.and_then(ConnectionColor::parse);
        let y = top + f64::from(u32::try_from(index).unwrap_or(0)) * (NOTE_HEIGHT + 24.0);
        notes.push(
            serde_json::from_value::<DiagramNote>(json!({ "id": fresh_id("note"), "x": right, "y": y, "text": body, "color": color }))
                .map_err(|err| AppError::invalid_input(format!("A note is not valid: {err}")))?,
        );
    }
    Ok(notes)
}

// ------------------------------------------------------------------- layout

/// Height of a table card (src/features/diagrams/metrics.ts `tableHeight`).
fn card_height(columns: usize) -> f64 {
    if columns == 0 {
        return HEADER_HEIGHT + EMPTY_HEIGHT + BORDER;
    }
    HEADER_HEIGHT + f64::from(u32::try_from(columns).unwrap_or(u32::MAX)) * ROW_HEIGHT + BORDER
}

/// How many references deep each table sits: 0 for one that references nothing,
/// otherwise one more than the deepest table it references. A loop (a table that
/// refers to itself, or two that refer to each other) is cut where it closes.
fn depths(count: usize, links: &[(usize, usize)]) -> Vec<usize> {
    let mut references: Vec<Vec<usize>> = vec![Vec::new(); count];
    for &(from, to) in links {
        if from != to && from < count && to < count && !references[from].contains(&to) {
            references[from].push(to);
        }
    }
    fn visit(index: usize, references: &[Vec<usize>], depth: &mut [Option<usize>], trail: &mut Vec<usize>) -> usize {
        if let Some(known) = depth[index] {
            return known;
        }
        if trail.contains(&index) {
            return 0;
        }
        trail.push(index);
        let deepest = references[index].iter().map(|&other| visit(other, references, depth, trail) + 1).max().unwrap_or(0);
        trail.pop();
        depth[index] = Some(deepest);
        deepest
    }
    let mut depth: Vec<Option<usize>> = vec![None; count];
    (0..count).map(|index| visit(index, &references, &mut depth, &mut Vec::new())).collect()
}

// WHAT:  A position for every card: columns by reference depth, each column a
//        stack of cards, a stack that grows too tall continuing in the next column.
// WHY:   Foreign keys then run from the right-hand cards to the left-hand ones
//        and rarely cross a column they do not need to.
fn layout(columns_per_card: &[usize], links: &[(usize, usize)]) -> Vec<(f64, f64)> {
    let depth = depths(columns_per_card.len(), links);
    let deepest = depth.iter().copied().max().unwrap_or(0);
    let mut spots = vec![(ORIGIN, ORIGIN); columns_per_card.len()];
    let mut column = 0_u32;
    for level in 0..=deepest {
        let mut y = ORIGIN;
        let mut started = false;
        for (index, _) in depth.iter().enumerate().filter(|(_, d)| **d == level) {
            let height = card_height(columns_per_card[index]);
            if started && y + height > ORIGIN + MAX_STACK_HEIGHT {
                column += 1;
                y = ORIGIN;
            }
            spots[index] = (ORIGIN + f64::from(column) * (TABLE_WIDTH + COLUMN_GAP), y);
            y += height + CARD_GAP;
            started = true;
        }
        if started {
            column += 1;
        }
    }
    spots
}

// WHAT:  Give the tables their positions: where they were, when the edit only
//        touched tables that already existed, otherwise a fresh layout of all.
// WHY:   Moving a table the user placed is the most annoying thing an edit can do;
//        but a new table dropped at the origin would land on top of another.
fn arrange(tables: &mut [DiagramTable], relations: &[DiagramRelation], before: Option<&DiagramBody>) {
    if let Some(old) = before {
        if tables.iter().all(|table| old.tables.iter().any(|t| t.id == table.id)) {
            for table in tables.iter_mut() {
                if let Some(earlier) = old.tables.iter().find(|t| t.id == table.id) {
                    table.x = earlier.x;
                    table.y = earlier.y;
                }
            }
            return;
        }
    }
    let index: HashMap<&str, usize> = tables.iter().enumerate().map(|(i, t)| (t.id.as_str(), i)).collect();
    let links: Vec<(usize, usize)> = relations
        .iter()
        .filter_map(|r| Some((*index.get(r.from_table.as_str())?, *index.get(r.to_table.as_str())?)))
        .collect();
    let sizes: Vec<usize> = tables.iter().map(|t| t.columns.len()).collect();
    for (table, (x, y)) in tables.iter_mut().zip(layout(&sizes, &links)) {
        table.x = x;
        table.y = y;
    }
}

// ---------------------------------------------------------------------------
// A diagram from the real schema
// ---------------------------------------------------------------------------

// WHAT:  The catalogue's tables, columns and foreign keys as a diagram.
// WHY:   "Open in designer" on a relationship graph the assistant drew: the graph
//        carries table names only, and the designer needs columns and relations.
// HOW:   `wanted` limits it to those tables (bare or schema-qualified names, any
//        case); empty means the whole database, capped. Views are left out. Each
//        table keeps its schema, as the ER diagram's "Edit in designer" does.
pub async fn diagram_from_catalog(ctx: &SessionCtx, wanted: &[String]) -> AppResult<DiagramBody> {
    let catalog = services::schema::catalog(ctx).await?;
    let wanted: HashSet<String> = wanted.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect();
    let qualified = |schema: &Option<String>, name: &str| match schema {
        Some(s) if !s.is_empty() => format!("{s}.{name}"),
        _ => name.to_string(),
    };

    let mut picked: Vec<(Option<String>, String)> = Vec::new();
    for schema in &catalog.schemas {
        for table in &schema.tables {
            if table.kind != TableKind::Table {
                continue;
            }
            let full = qualified(&table.schema, &table.name).to_lowercase();
            if wanted.is_empty() || wanted.contains(&full) || wanted.contains(&table.name.to_lowercase()) {
                picked.push((table.schema.clone(), table.name.clone()));
            }
        }
    }
    if picked.is_empty() {
        return Err(AppError::not_found("None of those tables exist in this database."));
    }
    picked.truncate(MAX_SCHEMA_TABLES);

    let mut tables: Vec<DiagramTable> = Vec::with_capacity(picked.len());
    let mut id_of: HashMap<String, String> = HashMap::new();
    for (schema, name) in &picked {
        let full = qualified(schema, name);
        let columns = services::schema::columns(ctx, &TableRef { schema: schema.clone(), name: name.clone() }).await.unwrap_or_default();
        let id = slug(&full);
        id_of.insert(full.to_lowercase(), id.clone());
        tables.push(DiagramTable {
            id,
            name: name.clone(),
            schema: schema.clone().filter(|s| !s.is_empty()),
            columns: columns
                .iter()
                .map(|c| DiagramColumn {
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                    primary_key: c.primary_key,
                    nullable: c.nullable && !c.primary_key,
                    ..DiagramColumn::default()
                })
                .collect(),
            ..DiagramTable::default()
        });
    }

    let mut relations: Vec<DiagramRelation> = Vec::new();
    for fk in services::schema::foreign_keys(ctx).await.unwrap_or_default() {
        if relations.len() >= MAX_RELATIONS {
            break;
        }
        let from = id_of.get(&qualified(&fk.from_schema, &fk.from_table).to_lowercase());
        let to = id_of.get(&qualified(&fk.to_schema, &fk.to_table).to_lowercase());
        let (Some(from), Some(to)) = (from, to) else { continue };
        let mut pairs = fk.from_columns.iter().zip(&fk.to_columns).map(|(f, t)| DiagramColumnPair { from_column: f.clone(), to_column: t.clone() });
        let Some(first) = pairs.next() else { continue };
        relations.push(DiagramRelation {
            id: fresh_id("fk"),
            from_table: from.clone(),
            from_column: first.from_column,
            to_table: to.clone(),
            to_column: first.to_column,
            extra_columns: pairs.collect(),
            name: Some(fk.name.clone()).filter(|n| !n.is_empty()),
            ..DiagramRelation::default()
        });
    }

    arrange(&mut tables, &relations, None);
    Ok(DiagramBody { tables, relations, ..DiagramBody::default() })
}

// WHAT:  Save a diagram built from the schema as a new document for this connection.
pub fn save_diagram(state: &crate::state::AppState, ctx: &SessionCtx, name: &str, body: DiagramBody) -> AppResult<Document> {
    let name = name.trim();
    let name: String = if name.is_empty() { "Schema diagram".to_string() } else { name.chars().take(MAX_NAME).collect() };
    let document = super::new_document(DocumentKind::Diagram, Some(ctx.connection.id.clone()), &name, DocumentBody::Diagram(body))?;
    services::documents::save(state, &document)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> Json {
        json!({
            "name": "Shop",
            "tables": [
                { "name": "Customers", "columns": [
                    { "name": "id", "type": "integer", "primaryKey": true },
                    { "name": "email", "type": "text", "nullable": false, "unique": true } ] },
                { "name": "Orders", "columns": [
                    { "name": "id", "type": "integer", "primaryKey": true, "autoIncrement": true },
                    { "name": "customer_id", "type": "integer" },
                    { "name": "placed_at", "type": "timestamp", "defaultValue": "now()", "comment": "UTC" } ] },
            ],
            "relations": [
                { "fromTable": "orders", "fromColumn": "CUSTOMER_ID", "toTable": "customers", "toColumn": "id", "onDelete": "set null", "onUpdate": "cascade" }
            ]
        })
    }

    fn built(args: &Json, previous: Option<&DocumentBody>) -> (Draft, DiagramBody) {
        let draft = parse(args, previous).unwrap_or_else(|e| panic!("{e}"));
        let body = match &draft.body {
            DocumentBody::Diagram(body) => body.clone(),
            _ => panic!("not a diagram"),
        };
        (draft, body)
    }

    #[test]
    fn a_diagram_resolves_relations_to_table_ids_and_puts_the_referenced_table_on_the_left() {
        let (draft, body) = built(&args(), None);
        assert_eq!(draft.summary, "2 tables · 1 relation");
        assert_eq!(draft.worst, StatementKind::Read);
        assert_eq!(body.tables[0].id, "customers");
        assert!(!body.tables[0].columns[0].nullable, "a primary key is never nullable");
        assert!(body.tables[0].columns[1].unique && !body.tables[0].columns[1].nullable);
        assert!(body.tables[1].columns[0].auto_increment);
        assert_eq!(body.tables[1].columns[2].default_value.as_deref(), Some("now()"));
        assert!(body.tables[1].x > body.tables[0].x, "Orders references Customers, so it sits to the right");
        let relation = &body.relations[0];
        assert_eq!((relation.from_table.as_str(), relation.to_table.as_str()), ("orders", "customers"));
        assert_eq!(relation.from_column, "customer_id", "the column keeps the table's own spelling");
        assert_eq!((relation.on_delete, relation.on_update), (Some(ReferentialAction::SetNull), Some(ReferentialAction::Cascade)));
        assert!(relation.id.starts_with("rel-"));
    }

    #[test]
    fn a_diagram_rejects_relations_that_point_nowhere() {
        let refused = |edit: &dyn Fn(&mut Json)| {
            let mut a = args();
            edit(&mut a);
            parse(&a, None).is_err()
        };
        assert!(refused(&|a| a["relations"][0]["toTable"] = json!("Invoices")));
        assert!(refused(&|a| a["relations"][0]["fromColumn"] = json!("nope")));
        assert!(refused(&|a| a["relations"][0]["onDelete"] = json!("explode")));
        assert!(refused(&|a| a["tables"][0]["columns"][1]["name"] = json!("ID")), "duplicate column, ignoring case");
        assert!(refused(&|a| a["tables"][0]["color"] = json!("chartreuse")));
        assert!(refused(&|a| a["relations"] = json!([a["relations"][0].clone(), a["relations"][0].clone()])), "the same key twice");
    }

    #[test]
    fn tables_in_different_schemas_can_share_a_name_and_must_be_qualified() {
        let a = json!({ "name": "d", "tables": [
            { "name": "item", "schema": "a", "columns": [{ "name": "id", "type": "int", "primaryKey": true }] },
            { "name": "item", "schema": "b", "columns": [{ "name": "id", "type": "int", "primaryKey": true }, { "name": "a_id", "type": "int" }] },
        ], "relations": [{ "fromTable": "b.item", "fromColumn": "a_id", "toTable": "a.item", "toColumn": "id" }] });
        let (_, body) = built(&a, None);
        assert_eq!(body.tables.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["a-item", "b-item"]);
        assert_eq!((body.relations[0].from_table.as_str(), body.relations[0].to_table.as_str()), ("b-item", "a-item"));
        let ambiguous = json!({ "name": "d", "tables": a["tables"], "relations": [{ "fromTable": "item", "fromColumn": "a_id", "toTable": "a.item", "toColumn": "id" }] });
        assert!(parse(&ambiguous, None).err().map(|e| e.message().to_string()).unwrap_or_default().contains("several schemas"));
    }

    #[test]
    fn identical_names_get_distinct_ids() {
        let tables: Vec<Json> = (0..4).map(|_| json!({ "name": "Item", "columns": [{ "name": "id", "type": "int" }] })).collect();
        let (_, body) = built(&json!({ "name": "d", "tables": tables }), None);
        let ids: Vec<&str> = body.tables.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["item", "item-2", "item-3", "item-4"]);
    }

    #[test]
    fn composite_keys_and_names_are_kept() {
        let a = json!({ "name": "d", "tables": [
            { "name": "a", "columns": [{ "name": "x", "type": "int", "primaryKey": true }, { "name": "y", "type": "int", "primaryKey": true }] },
            { "name": "b", "columns": [{ "name": "ax", "type": "int" }, { "name": "ay", "type": "int" }] },
        ], "relations": [{ "fromTable": "b", "fromColumn": "ax", "toTable": "a", "toColumn": "x", "name": "fk_b_a",
            "extraColumns": [{ "fromColumn": "ay", "toColumn": "y" }] }] });
        let (_, body) = built(&a, None);
        assert_eq!(body.relations[0].name.as_deref(), Some("fk_b_a"));
        assert_eq!(body.relations[0].extra_columns, vec![DiagramColumnPair { from_column: "ay".into(), to_column: "y".into() }]);
        let bad = json!({ "name": "d", "tables": a["tables"], "relations": [{ "fromTable": "b", "fromColumn": "ax", "toTable": "a", "toColumn": "x",
            "extraColumns": [{ "fromColumn": "ay", "toColumn": "nope" }] }] });
        assert!(parse(&bad, None).is_err());
    }

    #[test]
    fn notes_are_placed_to_the_right_and_replace_the_old_ones_only_when_given() {
        let mut with_notes = args();
        with_notes["notes"] = json!([{ "text": "Orders are never deleted", "color": "amber" }, { "text": "second" }]);
        let (draft, body) = built(&with_notes, None);
        assert_eq!(body.notes.len(), 2);
        assert!(draft.summary.ends_with("2 notes"), "{}", draft.summary);
        let rightmost = body.tables.iter().map(|t| t.x).fold(0.0, f64::max);
        assert!(body.notes[0].x > rightmost + TABLE_WIDTH);
        assert!(body.notes[1].y > body.notes[0].y);
        assert_eq!(body.notes[0].color, Some(ConnectionColor::Amber));

        let previous = DocumentBody::Diagram(body);
        let (_, kept) = built(&args(), Some(&previous));
        assert_eq!(kept.notes.len(), 2, "notes the model did not mention stay");
        let mut cleared = args();
        cleared["notes"] = json!([]);
        let (_, emptied) = built(&cleared, Some(&previous));
        assert!(emptied.notes.is_empty());
    }

    #[test]
    fn an_edit_keeps_positions_colours_and_line_styles_the_user_chose() {
        let (_, first) = built(&args(), None);
        let mut hand = first.clone();
        hand.tables[0].x = 999.0;
        hand.tables[0].y = 777.0;
        hand.tables[0].color = Some(ConnectionColor::Violet);
        hand.relations[0].color = Some(ConnectionColor::Red);
        hand.relations[0].waypoints = vec![crate::model::DiagramPoint { x: 5.0, y: 6.0 }];
        hand.grid_size = Some(8);
        let previous = DocumentBody::Diagram(hand);

        let mut edit = args();
        if let Some(columns) = edit["tables"][1]["columns"].as_array_mut() {
            columns.push(json!({ "name": "total", "type": "numeric" }));
        }
        let (_, body) = built(&edit, Some(&previous));
        assert_eq!((body.tables[0].x, body.tables[0].y), (999.0, 777.0), "only existing tables: nothing moves");
        assert_eq!(body.tables[0].color, Some(ConnectionColor::Violet));
        assert_eq!(body.relations[0].color, Some(ConnectionColor::Red));
        assert_eq!(body.relations[0].waypoints.len(), 1);
        assert_eq!(body.relations[0].id, first.relations[0].id, "the same key keeps its id");
        assert_eq!(body.grid_size, Some(8));
        assert_eq!(body.tables[1].columns.len(), 4);

        // A new table means a fresh layout: it would otherwise land on top of another.
        let mut grown = edit.clone();
        if let Some(tables) = grown["tables"].as_array_mut() {
            tables.push(json!({ "name": "Items", "columns": [{ "name": "id", "type": "int" }] }));
        }
        let (_, relaid) = built(&grown, Some(&previous));
        assert_ne!((relaid.tables[0].x, relaid.tables[0].y), (999.0, 777.0));
        assert_eq!(relaid.tables[0].color, Some(ConnectionColor::Violet), "styling still survives");
    }

    #[test]
    fn layout_puts_referenced_tables_left_and_wraps_tall_stacks() {
        // 0 references nothing; 1 and 2 reference 0; 3 references 1.
        let spots = layout(&[3, 3, 3, 3], &[(1, 0), (2, 0), (3, 1)]);
        assert_eq!(spots[0].0, ORIGIN);
        assert!(spots[1].0 > spots[0].0 && spots[1].0 == spots[2].0);
        assert!(spots[3].0 > spots[1].0);
        assert!(spots[2].1 > spots[1].1, "cards in a column stack downwards");

        // Unrelated tables: one level, wrapped into several columns once the stack is tall enough.
        let many = layout(&[12; 14], &[]);
        let columns: HashSet<i64> = many.iter().map(|(x, _)| *x as i64).collect();
        assert!(columns.len() > 1, "{columns:?}");
        assert!(many.iter().all(|(_, y)| *y <= ORIGIN + MAX_STACK_HEIGHT));
    }

    #[test]
    fn depths_survive_loops() {
        assert_eq!(depths(2, &[(0, 1), (1, 0)]).len(), 2);
        assert_eq!(depths(1, &[(0, 0)]), vec![0], "a self reference adds no depth");
        assert_eq!(depths(3, &[(2, 1), (1, 0)]), vec![0, 1, 2]);
        assert_eq!(depths(0, &[]), Vec::<usize>::new());
    }

    #[test]
    fn card_height_matches_the_designer() {
        assert_eq!(card_height(0), 56.0);
        assert_eq!(card_height(3), 30.0 + 66.0 + 2.0);
    }

    #[test]
    fn counts_are_limited() {
        let big: Vec<Json> = (0..=MAX_TABLES).map(|i| json!({ "name": format!("t{i}"), "columns": [{ "name": "id", "type": "int" }] })).collect();
        assert!(parse(&json!({ "name": "d", "tables": big }), None).is_err());
        let wide: Vec<Json> = (0..=MAX_COLUMNS).map(|i| json!({ "name": format!("c{i}"), "type": "int" })).collect();
        assert!(parse(&json!({ "name": "d", "tables": [{ "name": "t", "columns": wide }] }), None).is_err());
    }
}
