// SOT: agent-documents, ai-document-builder, document-draft-validation, ai-create-update-flow

use crate::error::{AppError, AppResult};
use crate::guard::destructive::{classify, StatementKind};
use crate::model::{
    AgentArtifact, AgentAutonomy, Document, DocumentBody, DocumentKind, PermissionDecision, PermissionRequest,
    StatementIntent,
};
use crate::services;
use crate::services::agent::tools::{PermissionGate, ToolCtx, ToolRun};
use serde_json::{json, Value as Json};

mod dashboard;
mod diagram;
mod workflow;

pub use dashboard::dashboard_tool;
pub use diagram::{diagram_from_catalog, diagram_tool, save_diagram};
pub use workflow::workflow_tool;

// WHAT:  The agent tools that build something the user keeps — a dashboard, a
//        workflow, a schema diagram — and the code that checks what the model
//        wrote before it is saved.
// WHY:   The model proposes JSON; nothing it writes is trusted. Each kind has
//        one module that describes its shape to the model (`*_tool`) and
//        validates and builds the real document (`parse`), so the three are
//        easy to find and to change when the document models grow.
// HOW:   Every create/update passes the permission gate first and shows what it
//        will save (a dashboard's queries, a workflow's nodes). Saving goes
//        through the same `services::documents::save` the editors use, so a
//        document made here is indistinguishable from one made by hand. Bodies
//        are built as the real model structs (`..Default::default()` for the
//        fields that were not asked for), so a field the models gain later with
//        a default needs no change here. Updating a document replaces the part
//        the model rewrote and keeps the rest (variables, notes, schedules,
//        sealed secrets) — it never throws away what the model could not see.
// WHERE: src-tauri/src/services/agent/tools.rs (registers and dispatches),
//        src-tauri/src/model/documents.rs (the bodies built here)
//
// UPDATE AFTER MERGE: dashboards, workflows and diagrams keep growing options.
// `dashboard.rs`, `workflow.rs` and `diagram.rs` are the only code that knows
// those shapes — the schema text the model reads and the validation sit side by
// side in each, so a new option is one edit in one file.

const MAX_NAME: usize = 120;

/// A validated proposal, ready to show and to save.
pub(super) struct Draft {
    pub name: String,
    pub body: DocumentBody,
    /// "5 widgets · 1 variable".
    pub summary: String,
    /// One line per widget / node / table for the approval prompt.
    pub previews: Vec<String>,
    /// The most invasive statement the document would run. A dashboard is always `Read`.
    pub worst: StatementKind,
    /// Things the user should know before approving, and the model should relay
    /// afterwards: a schedule that stays paused, a login still to be entered.
    pub caveats: Vec<String>,
}

// ---------------------------------------------------------------------------
// The shared create/update flow
// ---------------------------------------------------------------------------

// WHAT:  Validate, ask, save — for any of the three kinds.
// WHY:   One path means the permission prompt and the read-only rule cannot be
//        skipped by adding a fourth kind.
pub async fn create(
    ctx: &ToolCtx<'_>,
    kind: DocumentKind,
    args: &Json,
    gate: &mut dyn PermissionGate,
) -> AppResult<ToolRun> {
    let existing = existing_document(ctx, kind, args)?;
    let previous = existing.as_ref().map(|doc| &doc.body);

    let draft = match kind {
        DocumentKind::Dashboard => dashboard::parse(args, previous)?,
        DocumentKind::Workflow => {
            let remembered = workflow::Remembered::load(ctx.state);
            workflow::parse(args, previous, &remembered)?
        }
        DocumentKind::Diagram => diagram::parse(args, previous)?,
    };

    let noun = kind.as_str();
    if draft.worst != StatementKind::Read {
        if ctx.autonomy == AgentAutonomy::ReadOnly {
            return Err(AppError::read_only(format!(
                "The assistant is set to read-only, so it cannot save a {noun} that changes data. \
                 Change it in Settings → AI, or give the user the statements to run themselves."
            )));
        }
        if ctx.session.connection.read_only {
            return Err(AppError::read_only(format!(
                "This connection is read-only, so a {noun} that changes data could never run here. \
                 Rewrite it to only read, or describe the changes for the user."
            )));
        }
    }

    let updating = existing.is_some();
    let verb = if updating { "Update" } else { "Create" };
    let request = PermissionRequest {
        call_id: ctx.call_id.clone(),
        tool: format!("create_{noun}"),
        title: format!("{verb} {noun} \"{}\"", draft.name),
        statement: None,
        statements: draft.previews.clone(),
        intent: intent_of(draft.worst),
        reason: reason_for(kind, &draft, updating),
        action_label: Some(format!("{verb} {noun}")),
    };
    if gate.ask(request).await == PermissionDecision::Deny {
        return Err(AppError::invalid_input(format!(
            "The user declined to save this {noun}. Do not try again; ask what they would change."
        )));
    }

    let document = match existing {
        Some(mut doc) => {
            // Keep what the user had: an AI rewrite of an open document can be
            // undone from its version history.
            services::document_versions::snapshot(ctx.state, &doc.id, &doc.body, None)?;
            doc.name = draft.name.clone();
            doc.body = draft.body.clone();
            doc
        }
        None => new_document(kind, Some(ctx.session.connection.id.clone()), &draft.name, draft.body.clone())?,
    };
    let saved = services::documents::save(ctx.state, &document)?;

    let artifact = AgentArtifact::Document {
        id: format!("doc-{}", ctx.call_id),
        title: saved.name.clone(),
        document_kind: kind,
        document_id: saved.id.clone(),
        connection_id: saved.connection_id.clone(),
        updated: updating,
        summary: draft.summary.clone(),
    };
    let mut content = format!(
        "{} {noun} \"{}\" ({}). The user sees a card with an Open button. Say in one or two \
         sentences what it holds; do not list every part again.",
        if updating { "Updated" } else { "Saved" },
        saved.name,
        draft.summary
    );
    for caveat in &draft.caveats {
        content.push_str("\nTell the user: ");
        content.push_str(caveat);
    }
    Ok(ToolRun::ok(draft.summary.clone(), content).with_artifact(artifact))
}

// WHAT:  The document `documentId` names, when it names one.
// WHY:   An update must be of the right kind and belong to this connection: an id
//        the model made up (or took from another connection's chat) must not
//        reach a different document.
fn existing_document(ctx: &ToolCtx<'_>, kind: DocumentKind, args: &Json) -> AppResult<Option<Document>> {
    let Some(id) = args.get("documentId").and_then(Json::as_str).map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    let doc = services::documents::get(ctx.state, id)?;
    if doc.kind != kind {
        return Err(AppError::invalid_input(format!(
            "\"{}\" is a {}, not a {}. Create a new {} or pass the id of a {}.",
            doc.name,
            doc.kind.as_str(),
            kind.as_str(),
            kind.as_str(),
            kind.as_str()
        )));
    }
    if doc.connection_id.as_deref().is_some_and(|owner| owner != ctx.session.connection.id) {
        return Err(AppError::invalid_input(format!(
            "\"{}\" belongs to a different connection. Create a new {} here instead.",
            doc.name,
            kind.as_str()
        )));
    }
    Ok(Some(doc))
}

fn intent_of(kind: StatementKind) -> StatementIntent {
    match kind {
        StatementKind::Read => StatementIntent::Read,
        StatementKind::Write => StatementIntent::Write,
        StatementKind::Destructive => StatementIntent::Destructive,
    }
}

fn reason_for(kind: DocumentKind, draft: &Draft, updating: bool) -> String {
    let verb = if updating { "replaces the saved" } else { "saves a new" };
    let mut reason = match kind {
        DocumentKind::Dashboard => format!(
            "This {verb} dashboard ({}). Its queries only read data and run each time the dashboard refreshes.",
            draft.summary
        ),
        DocumentKind::Workflow => match draft.worst {
            StatementKind::Read => format!("This {verb} workflow ({}). Nothing runs until you run it.", draft.summary),
            StatementKind::Write => format!(
                "This {verb} workflow ({}). Some nodes change data; they run when you run the workflow.",
                draft.summary
            ),
            StatementKind::Destructive => format!(
                "This {verb} workflow ({}). Some nodes delete or overwrite data; each needs your own \"Allow destructive statements\" tick in the editor before it can run.",
                draft.summary
            ),
        },
        DocumentKind::Diagram => format!("This {verb} schema diagram ({}). Nothing is changed in the database.", draft.summary),
    };
    for caveat in &draft.caveats {
        reason.push(' ');
        reason.push_str(caveat);
    }
    reason
}

// WHAT:  A new, unsaved document around a body.
// WHY:   Built through serde so a field the document model gains with a default
//        never breaks this file; the test below proves the round trip.
fn new_document(kind: DocumentKind, connection_id: Option<String>, name: &str, body: DocumentBody) -> AppResult<Document> {
    let body = serde_json::to_value(&body).map_err(AppError::internal)?;
    serde_json::from_value::<Document>(json!({
        "id": "",
        "kind": kind,
        "connectionId": connection_id,
        "name": name,
        "body": body,
        "tags": [],
        "createdAt": "",
        "updatedAt": "",
    }))
    .map_err(AppError::internal)
}

// ---------------------------------------------------------------------------
// Small shared helpers
// ---------------------------------------------------------------------------

fn text<'a>(value: &'a Json, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Json::as_str).map(str::trim).filter(|v| !v.is_empty())
}

fn required<'a>(value: &'a Json, key: &str, what: &str) -> AppResult<&'a str> {
    text(value, key).ok_or_else(|| AppError::invalid_input(format!("{what} needs a non-empty \"{key}\".")))
}

fn clean_name(args: &Json, what: &str) -> AppResult<String> {
    let name = required(args, "name", what)?;
    Ok(name.chars().take(MAX_NAME).collect())
}

fn list<'a>(args: &'a Json, key: &str, what: &str, min: usize, max: usize) -> AppResult<&'a Vec<Json>> {
    let items = args
        .get(key)
        .and_then(Json::as_array)
        .ok_or_else(|| AppError::invalid_input(format!("{what} needs a \"{key}\" array.")))?;
    if items.len() < min {
        return Err(AppError::invalid_input(format!(
            "{what} needs at least {min} entr{} in \"{key}\".",
            if min == 1 { "y" } else { "ies" }
        )));
    }
    if items.len() > max {
        return Err(AppError::invalid_input(format!(
            "{what} may have at most {max} entries in \"{key}\"; this has {}.",
            items.len()
        )));
    }
    Ok(items)
}

/// An optional whole number argument, clamped into `low..=high`.
fn clamped(value: &Json, key: &str, low: u32, high: u32) -> Option<u32> {
    let raw = value.get(key)?;
    let n = raw.as_u64().or_else(|| raw.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f.round() as u64))?;
    Some(u32::try_from(n.clamp(u64::from(low), u64::from(high))).unwrap_or(high))
}

fn flag(value: &Json, key: &str) -> Option<bool> {
    value.get(key).and_then(Json::as_bool)
}

/// One of a closed list of words, matched ignoring case; an error names the choices.
fn choice<'a>(value: &'a Json, key: &str, allowed: &'a [&'a str], what: &str) -> AppResult<Option<&'a str>> {
    let Some(raw) = text(value, key) else { return Ok(None) };
    let lower = raw.to_lowercase();
    match allowed.iter().find(|candidate| **candidate == lower) {
        Some(found) => Ok(Some(found)),
        None => Err(AppError::invalid_input(format!("{what}: \"{raw}\" is not a valid {key}. Use {}.", allowed.join(", ")))),
    }
}

// WHAT:  A JSON Schema `properties` object from a list of (name, schema) pairs.
// WHY:   `json!` expands once per entry and runs into the compiler's recursion
//        limit on a schema this wide; building the object from pieces keeps each
//        `json!` small.
fn properties(entries: Vec<(&str, Json)>) -> Json {
    Json::Object(entries.into_iter().map(|(name, schema)| (name.to_string(), schema)).collect())
}

/// A short, one-line excerpt of a statement for the approval prompt.
fn preview(sql: &str) -> String {
    let flat: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 110 {
        let head: String = flat.chars().take(107).collect();
        format!("{head}...")
    } else {
        flat
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn severity(kind: StatementKind) -> u8 {
    match kind {
        StatementKind::Read => 0,
        StatementKind::Write => 1,
        StatementKind::Destructive => 2,
    }
}

/// The more invasive of two statement kinds.
fn worse(a: StatementKind, b: StatementKind) -> StatementKind {
    if severity(b) > severity(a) { b } else { a }
}

/// The most invasive of a script's statements.
fn worst_of(sql: &str) -> StatementKind {
    classify(sql).iter().map(|s| s.kind).fold(StatementKind::Read, worse)
}

/// Lower-case identifier-ish id: "Order Items" → "order-items".
fn slug(raw: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in raw.chars() {
        if c.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.extend(c.to_lowercase());
        } else {
            pending_dash = true;
        }
    }
    if out.is_empty() { "item".to_string() } else { out }
}

/// An id no other part of any document has: `ai-3fa91c02`.
fn fresh_id(prefix: &str) -> String {
    let simple = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", simple.chars().take(8).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WorkflowBody;

    #[test]
    fn slugs_are_stable_and_never_empty() {
        assert_eq!(slug("Order Items"), "order-items");
        assert_eq!(slug("  public.Users "), "public-users");
        assert_eq!(slug("***"), "item");
    }

    #[test]
    fn a_new_document_round_trips_through_the_real_model() {
        let doc = new_document(DocumentKind::Workflow, Some("c1".into()), "w", DocumentBody::Workflow(WorkflowBody::default()))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(doc.kind, DocumentKind::Workflow);
        assert_eq!(doc.connection_id.as_deref(), Some("c1"));
        assert!(doc.id.is_empty(), "the store assigns the id");
    }

    #[test]
    fn statement_kinds_rank_by_how_much_they_change() {
        assert_eq!(worse(StatementKind::Read, StatementKind::Write), StatementKind::Write);
        assert_eq!(worse(StatementKind::Destructive, StatementKind::Write), StatementKind::Destructive);
        assert_eq!(worst_of("SELECT 1; DELETE FROM t"), StatementKind::Destructive);
        assert_eq!(worst_of("SELECT 1"), StatementKind::Read);
    }

    #[test]
    fn arguments_are_clamped_and_choices_are_closed() {
        let args = json!({ "w": 99, "h": -3, "mode": "Bar" });
        assert_eq!(clamped(&args, "w", 2, 12), Some(12));
        assert_eq!(clamped(&args, "h", 2, 24), None, "a negative number is not a size");
        assert_eq!(clamped(&args, "missing", 2, 12), None);
        assert_eq!(choice(&args, "mode", &["bar", "line"], "x").unwrap_or_default(), Some("bar"));
        assert!(choice(&json!({ "mode": "gauge" }), "mode", &["bar"], "x").is_err());
        assert!(fresh_id("ai").starts_with("ai-"));
        assert_ne!(fresh_id("ai"), fresh_id("ai"));
    }

    #[test]
    fn a_long_statement_is_shortened_for_the_prompt() {
        let long = format!("SELECT {}", "a, ".repeat(80));
        assert!(preview(&long).chars().count() <= 110);
        assert_eq!(preview("SELECT   1\n FROM t"), "SELECT 1 FROM t");
    }
}
