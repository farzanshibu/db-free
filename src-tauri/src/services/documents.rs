// SOT: documents-service, dashboards-service, workflows-service, diagrams-service

use crate::error::{AppError, AppResult};
use crate::model::{Document, DocumentBody, DocumentKind, LibraryScope};
use crate::services::library::copy_name;
use crate::state::AppState;

pub fn list(state: &AppState, kind: DocumentKind) -> AppResult<Vec<Document>> {
    state.with_store(|store| store.list_documents(kind))
}

pub fn get(state: &AppState, id: &str) -> AppResult<Document> {
    state.with_store(|store| store.get_document(id))
}

// WHAT:  Saves a document after checking the body matches its declared kind.
pub fn save(state: &AppState, doc: &Document) -> AppResult<Document> {
    if doc.name.trim().is_empty() {
        return Err(AppError::invalid_input("A name is required."));
    }
    let matches = matches!(
        (&doc.kind, &doc.body),
        (DocumentKind::Dashboard, DocumentBody::Dashboard(_))
            | (DocumentKind::Workflow, DocumentBody::Workflow(_))
            | (DocumentKind::Diagram, DocumentBody::Diagram(_))
    );
    if !matches {
        return Err(AppError::invalid_input("Document body does not match its kind."));
    }
    // Workflow secrets are sealed here, before the body reaches the store.
    let sealed;
    let doc = if matches!(doc.body, DocumentBody::Workflow(_)) {
        sealed = crate::services::workflows::seal_document(state, doc)?;
        &sealed
    } else {
        doc
    };
    let saved = state.with_store(|store| store.upsert_document(doc))?;
    crate::services::workflows::documents_changed();
    Ok(saved)
}

// WHAT:  Renames from the sidebar. Only the name changes: the body is left as the
//        open editor last saved it.
pub fn rename(state: &AppState, id: &str, name: &str) -> AppResult<Document> {
    if name.trim().is_empty() {
        return Err(AppError::invalid_input("A name is required."));
    }
    let renamed = state.with_store(|store| store.rename_document(id, name))?;
    // A workflow's name is shown wherever its schedules are listed.
    crate::services::workflows::documents_changed();
    Ok(renamed)
}

// WHAT:  Copies a document, in the same folder. `target_connection_id` files the copy
//        under another connection ("Copy to connection"); None keeps it here.
// HOW:   A same-connection copy is "X (copy)"; a copy under another connection keeps
//        the name unless that connection already has a document of this kind by it.
//        The body is copied as it is.
pub fn duplicate(state: &AppState, id: &str, target_connection_id: Option<&str>) -> AppResult<Document> {
    let copy = state.with_store(|store| {
        let original = store.get_document(id)?;
        let scope = LibraryScope::of_document(original.kind);
        let cross = target_connection_id.is_some_and(|target| Some(target) != original.connection_id.as_deref());
        if cross {
            if let Some(target) = target_connection_id {
                if !store.connection_exists(target)? {
                    return Err(AppError::not_found("The connection to copy to no longer exists."));
                }
            }
        }
        let connection_id = if cross { target_connection_id.map(str::to_string) } else { original.connection_id.clone() };
        let taken: Vec<String> = store
            .list_documents(original.kind)?
            .into_iter()
            .filter(|d| d.connection_id == connection_id)
            .map(|d| d.name)
            .collect();
        let name = if cross && !taken.iter().any(|t| t.trim().to_lowercase() == original.name.trim().to_lowercase()) {
            original.name.clone()
        } else {
            copy_name(&original.name, &taken)
        };
        let folder = store.folder_of(scope, id)?;
        let copy = store.upsert_document(&Document {
            id: String::new(),
            connection_id,
            name,
            created_at: String::new(),
            updated_at: String::new(),
            ..original
        })?;
        if let Some(folder) = folder {
            store.place_item(scope, &copy.id, Some(&folder))?;
        }
        Ok(copy)
    })?;
    // A copied workflow brings its schedules with it.
    crate::services::workflows::documents_changed();
    Ok(copy)
}

pub fn delete(state: &AppState, id: &str) -> AppResult<()> {
    state.with_store(|store| store.delete_document(id))?;
    crate::services::workflows::documents_changed(); // a deleted workflow's schedules stop
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::{DashboardBody, LibraryScope};
    use crate::store::Store;

    fn state() -> AppState {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        AppState::new(store, Box::new(MemoryKeyProvider::default()))
    }

    fn add_connection(state: &AppState, id: &str) {
        state.with_store(|store| store.insert_test_connection(id)).unwrap_or_else(|e| panic!("{e}"));
    }

    fn dashboard(name: &str, connection: Option<&str>) -> Document {
        Document {
            id: String::new(),
            kind: DocumentKind::Dashboard,
            connection_id: connection.map(String::from),
            name: name.into(),
            body: DocumentBody::Dashboard(DashboardBody::default()),
            tags: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn duplicates_are_numbered_and_keep_their_folder() {
        let state = state();
        let original = save(&state, &dashboard("Sales", None)).unwrap_or_else(|e| panic!("{e}"));
        let folder = state.with_store(|s| s.create_folder(LibraryScope::Dashboard, None, "Ops")).unwrap_or_else(|e| panic!("{e}"));
        state.with_store(|s| s.place_item(LibraryScope::Dashboard, &original.id, Some(&folder.id))).unwrap_or_else(|e| panic!("{e}"));
        let first = duplicate(&state, &original.id, None).unwrap_or_else(|e| panic!("{e}"));
        let second = duplicate(&state, &original.id, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((first.name.as_str(), second.name.as_str()), ("Sales (copy)", "Sales (copy 2)"));
        assert_ne!(first.id, original.id);
        let placed = state.with_store(|s| s.folder_of(LibraryScope::Dashboard, &first.id)).unwrap_or_default();
        assert_eq!(placed.as_deref(), Some(folder.id.as_str()));
    }

    #[test]
    fn copying_to_another_connection_keeps_the_name_unless_taken() {
        let state = state();
        add_connection(&state, "c1");
        add_connection(&state, "c2");
        let original = save(&state, &dashboard("Sales", Some("c1"))).unwrap_or_else(|e| panic!("{e}"));
        let moved = duplicate(&state, &original.id, Some("c2")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((moved.name.as_str(), moved.connection_id.as_deref()), ("Sales", Some("c2")));
        let again = duplicate(&state, &original.id, Some("c2")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.name, "Sales (copy)");
        assert!(duplicate(&state, &original.id, Some("ghost")).is_err());
    }

    #[test]
    fn rename_refuses_a_blank_name() {
        let state = state();
        let doc = save(&state, &dashboard("Sales", None)).unwrap_or_else(|e| panic!("{e}"));
        assert!(rename(&state, &doc.id, "   ").is_err());
        assert_eq!(rename(&state, &doc.id, " Revenue ").unwrap_or_else(|e| panic!("{e}")).name, "Revenue");
    }
}
