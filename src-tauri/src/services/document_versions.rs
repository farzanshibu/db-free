// SOT: document-versions-service, autosave-document, version-history-policy, checkpoints, restore-version

use crate::error::{AppError, AppResult};
use crate::model::{Document, DocumentBody, DocumentVersionSummary};
use crate::services;
use crate::state::AppState;
use crate::store::document_versions::StoredVersion;
use crate::store::now_rfc3339;

// WHAT:  The history of a document: automatic versions the editor takes while
//        you work, and named checkpoints you take on purpose.
// WHY:   An autosaving editor has no undo past the last keystroke. A short rolling
//        history of the document, plus checkpoints that are never thrown away,
//        gives the user somewhere to go back to.
// HOW:   Automatic versions skip a save that changes nothing and only the newest
//        `KEEP_AUTOMATIC` survive; a named checkpoint is always kept and always
//        written, even when it equals the latest version. Restoring first files
//        the state it replaces as a version, so a restore can itself be undone.
// WHERE: src-tauri/src/store/document_versions.rs (rows), src-tauri/src/commands/diagrams.rs

/// How many unnamed versions a document keeps.
pub const KEEP_AUTOMATIC: u32 = 20;
/// What a checkpoint is called when the user gives no name.
pub const DEFAULT_CHECKPOINT_NAME: &str = "Checkpoint";

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

// WHAT:  What a body holds, in words: "3 tables · 2 relations · 1 note".
pub fn summarize(body: &DocumentBody) -> String {
    match body {
        DocumentBody::Diagram(d) => [plural(d.tables.len(), "table"), plural(d.relations.len(), "relation"), plural(d.notes.len(), "note")].join(" · "),
        DocumentBody::Dashboard(d) => plural(d.widgets.len(), "widget"),
        DocumentBody::Workflow(w) if w.nodes.is_empty() => plural(w.steps.len(), "step"),
        DocumentBody::Workflow(w) => [plural(w.nodes.len(), "node"), plural(w.edges.len(), "connection")].join(" · "),
    }
}

fn summary_of(stored: &StoredVersion) -> DocumentVersionSummary {
    let summary = serde_json::from_str::<DocumentBody>(&stored.body).map_or_else(|_| "Snapshot".to_string(), |b| summarize(&b));
    DocumentVersionSummary {
        document_id: stored.document_id.clone(),
        version: stored.version,
        name: stored.name.clone(),
        created_at: stored.created_at.clone(),
        summary,
    }
}

// WHAT:  An automatic snapshot is pointless when the latest one already holds the
//        same body; a checkpoint is the user's explicit request and always lands.
fn is_redundant(latest_body: Option<&str>, new_body: &str, named: bool) -> bool {
    !named && latest_body == Some(new_body)
}

/// Newest first.
pub fn list(state: &AppState, document_id: &str) -> AppResult<Vec<DocumentVersionSummary>> {
    state.with_store(|store| Ok(store.list_document_versions(document_id)?.iter().map(summary_of).collect()))
}

// WHAT:  Files `body` as a version of the document. `name` None = automatic.
// RETURNS: None when an automatic snapshot would repeat the latest version.
pub fn snapshot(state: &AppState, document_id: &str, body: &DocumentBody, name: Option<&str>) -> AppResult<Option<DocumentVersionSummary>> {
    let json = serde_json::to_string(body).map_err(AppError::store)?;
    let name = name.map(|n| {
        let trimmed = n.trim();
        if trimmed.is_empty() {
            DEFAULT_CHECKPOINT_NAME
        } else {
            trimmed
        }
    });
    state.with_store(|store| {
        let latest = store.latest_document_version(document_id)?;
        if is_redundant(latest.as_ref().map(|v| v.body.as_str()), &json, name.is_some()) {
            return Ok(None);
        }
        let stored = store.insert_document_version(document_id, &json, name, &now_rfc3339())?;
        if name.is_none() {
            store.prune_document_versions(document_id, KEEP_AUTOMATIC)?;
        }
        Ok(Some(summary_of(&stored)))
    })
}

// WHAT:  Refuses to write a document that is not in the store.
// WHY:   The editor saves on a timer and once more when its tab closes. A document
//        deleted from the sidebar while its tab was still open would otherwise be
//        written back as a brand-new row by that last save and come back to life.
fn ensure_saved_once(state: &AppState, doc: &Document, unsaved_message: &str) -> AppResult<()> {
    if doc.id.is_empty() {
        return Err(AppError::invalid_input(unsaved_message));
    }
    services::documents::get(state, &doc.id).map(|_| ())
}

// WHAT:  Saves the document, then (when `take_version`) files it as an automatic
//        version. The editor calls it on a short debounce with `take_version`
//        false and on a longer one with it true.
pub fn autosave(state: &AppState, doc: &Document, take_version: bool) -> AppResult<Document> {
    ensure_saved_once(state, doc, "Save the document once before autosaving it.")?;
    let saved = services::documents::save(state, doc)?;
    if take_version {
        snapshot(state, &saved.id, &saved.body, None)?;
    }
    Ok(saved)
}

// WHAT:  Saves the document and files a named checkpoint of it.
pub fn checkpoint(state: &AppState, doc: &Document, name: &str) -> AppResult<DocumentVersionSummary> {
    ensure_saved_once(state, doc, "Save the document once before taking a checkpoint.")?;
    let saved = services::documents::save(state, doc)?;
    snapshot(state, &saved.id, &saved.body, Some(name))?.ok_or_else(|| AppError::internal("A checkpoint was not recorded."))
}

pub fn restore(state: &AppState, document_id: &str, version: u32) -> AppResult<Document> {
    let stored = state.with_store(|store| store.get_document_version(document_id, version))?;
    let body: DocumentBody =
        serde_json::from_str(&stored.body).map_err(|e| AppError::invalid_input(format!("Version {version} can no longer be read: {e}")))?;
    let current = services::documents::get(state, document_id)?;
    // The state being replaced is filed first, so a restore never loses work.
    snapshot(state, document_id, &current.body, None)?;
    let saved = services::documents::save(state, &Document { body, ..current })?;
    snapshot(state, document_id, &saved.body, None)?;
    Ok(saved)
}

pub fn delete(state: &AppState, document_id: &str, version: u32) -> AppResult<()> {
    state.with_store(|store| store.delete_document_version(document_id, version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::{DashboardBody, DiagramBody, DiagramNote, DiagramRelation, DiagramTable, DocumentKind};
    use crate::store::Store;

    fn state() -> AppState {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        AppState::new(store, Box::new(MemoryKeyProvider::default()))
    }

    fn diagram(tables: usize) -> DocumentBody {
        DocumentBody::Diagram(DiagramBody {
            tables: (0..tables).map(|i| DiagramTable { id: format!("t{i}"), name: format!("t{i}"), ..DiagramTable::default() }).collect(),
            ..DiagramBody::default()
        })
    }

    fn saved_diagram(state: &AppState, tables: usize) -> Document {
        let doc = Document {
            id: String::new(),
            kind: DocumentKind::Diagram,
            connection_id: None,
            name: "d".into(),
            body: diagram(tables),
            tags: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        services::documents::save(state, &doc).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn the_summary_names_each_count() {
        let body = DocumentBody::Diagram(DiagramBody {
            tables: vec![DiagramTable::default(), DiagramTable::default()],
            relations: vec![DiagramRelation::default()],
            notes: vec![],
            ..DiagramBody::default()
        });
        assert_eq!(summarize(&body), "2 tables · 1 relation · 0 notes");
        let one_note = DocumentBody::Diagram(DiagramBody {
            notes: vec![DiagramNote { id: "n".into(), x: 0.0, y: 0.0, width: 1.0, height: 1.0, text: String::new(), color: None, font_size: None, hide_on_export: false }],
            ..DiagramBody::default()
        });
        assert!(summarize(&one_note).ends_with("1 note"));
        assert_eq!(summarize(&DocumentBody::Dashboard(DashboardBody::default())), "0 widgets");
    }

    #[test]
    fn an_unchanged_automatic_snapshot_is_redundant_but_a_checkpoint_never_is() {
        assert!(is_redundant(Some("{}"), "{}", false));
        assert!(!is_redundant(Some("{}"), "{}", true));
        assert!(!is_redundant(Some("{}"), "{\"a\":1}", false));
        assert!(!is_redundant(None, "{}", false));
    }

    #[test]
    fn automatic_versions_skip_repeats_and_keep_the_newest_twenty() {
        let state = state();
        let doc = saved_diagram(&state, 0);
        for n in 0..30 {
            snapshot(&state, &doc.id, &diagram(n), None).unwrap_or_else(|e| panic!("{e}"));
        }
        assert!(snapshot(&state, &doc.id, &diagram(29), None).unwrap_or_default().is_none(), "the same body twice is one version");
        let versions = list(&state, &doc.id).unwrap_or_default();
        assert_eq!(versions.len(), usize::try_from(KEEP_AUTOMATIC).unwrap_or(20));
        assert_eq!(versions.first().map(|v| v.summary.clone()).as_deref(), Some("29 tables · 0 relations · 0 notes"));
    }

    #[test]
    fn checkpoints_survive_pruning_and_default_their_name() {
        let state = state();
        let doc = saved_diagram(&state, 1);
        let named = checkpoint(&state, &Document { name: "renamed".into(), ..doc.clone() }, "  ").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(named.name.as_deref(), Some(DEFAULT_CHECKPOINT_NAME));
        for n in 10..40 {
            snapshot(&state, &doc.id, &diagram(n), None).unwrap_or_else(|e| panic!("{e}"));
        }
        let versions = list(&state, &doc.id).unwrap_or_default();
        assert!(versions.iter().any(|v| v.name.is_some()), "the checkpoint outlived thirty automatic versions");
        assert_eq!(versions.len(), usize::try_from(KEEP_AUTOMATIC).unwrap_or(20) + 1);
    }

    #[test]
    fn restoring_files_the_current_state_first_and_returns_the_old_body() {
        let state = state();
        let doc = saved_diagram(&state, 1);
        snapshot(&state, &doc.id, &doc.body, None).unwrap_or_else(|e| panic!("{e}"));
        let edited = autosave(&state, &Document { body: diagram(5), ..doc.clone() }, false).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(summarize(&edited.body), "5 tables · 0 relations · 0 notes");
        let restored = restore(&state, &doc.id, 1).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(summarize(&restored.body), "1 table · 0 relations · 0 notes");
        let summaries: Vec<String> = list(&state, &doc.id).unwrap_or_default().into_iter().map(|v| v.summary).collect();
        assert!(summaries.iter().any(|s| s.starts_with("5 tables")), "the replaced state is a version: {summaries:?}");
        assert!(restore(&state, &doc.id, 99).is_err());
    }

    #[test]
    fn autosave_needs_a_saved_document() {
        let state = state();
        let unsaved = Document {
            id: String::new(),
            kind: DocumentKind::Diagram,
            connection_id: None,
            name: "d".into(),
            body: diagram(0),
            tags: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert!(autosave(&state, &unsaved, true).is_err());
        assert!(checkpoint(&state, &unsaved, "x").is_err());
    }

    #[test]
    fn a_deleted_document_is_not_written_back_by_a_late_autosave() {
        let state = state();
        let doc = saved_diagram(&state, 1);
        services::documents::delete(&state, &doc.id).unwrap_or_else(|e| panic!("{e}"));
        let err = autosave(&state, &Document { body: diagram(2), ..doc.clone() }, true).err();
        assert!(matches!(err, Some(AppError::NotFound { .. })), "{err:?}");
        assert!(checkpoint(&state, &doc, "late").is_err());
        assert!(services::documents::get(&state, &doc.id).is_err(), "the document stayed deleted");
    }

    #[test]
    fn a_workflow_is_summarised_by_its_nodes_once_it_has_any() {
        use crate::model::WorkflowBody;
        assert_eq!(summarize(&DocumentBody::Workflow(WorkflowBody::default())), "0 steps");
        let body: WorkflowBody = serde_json::from_value(serde_json::json!({
            "nodes": [{ "id": "a", "config": { "kind": "note" } }, { "id": "b", "config": { "kind": "note" } }],
            "edges": [{ "id": "e", "source": "a", "target": "b" }]
        }))
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(summarize(&DocumentBody::Workflow(body)), "2 nodes · 1 connection");
    }
}
