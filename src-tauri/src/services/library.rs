// SOT: library-service, folders-service, copy-names, saved-query-export, folder-paths

use crate::error::{AppError, AppResult};
use crate::model::{LibraryFolder, LibraryLayout, LibraryScope, SavedQuery};
use crate::state::AppState;
use std::collections::HashMap;

pub fn layout(state: &AppState) -> AppResult<LibraryLayout> {
    state.with_store(|store| store.list_library())
}

pub fn create_folder(state: &AppState, scope: LibraryScope, parent_id: Option<&str>, name: &str) -> AppResult<LibraryFolder> {
    state.with_store(|store| store.create_folder(scope, parent_id, name))
}

pub fn rename_folder(state: &AppState, id: &str, name: &str) -> AppResult<LibraryFolder> {
    state.with_store(|store| store.rename_folder(id, name))
}

pub fn move_folder(state: &AppState, id: &str, parent_id: Option<&str>) -> AppResult<LibraryFolder> {
    state.with_store(|store| store.move_folder(id, parent_id))
}

pub fn delete_folder(state: &AppState, id: &str) -> AppResult<()> {
    state.with_store(|store| store.delete_folder(id))
}

// WHAT:  Files a saved query or document into a folder (None = top level).
// WHY:   Checks the item exists first so a stale id cannot leave a placement
//        row pointing at nothing.
pub fn move_item(state: &AppState, scope: LibraryScope, item_id: &str, folder_id: Option<&str>) -> AppResult<()> {
    state.with_store(|store| {
        match scope {
            LibraryScope::SavedQuery => {
                let query = store.get_saved_query(item_id)?;
                if store.saved_query_name_taken(query.connection_id.as_deref(), folder_id, &query.name, Some(item_id))? {
                    return Err(AppError::invalid_input(format!("A saved query named “{}” already exists in that folder.", query.name.trim())));
                }
            }
            LibraryScope::Dashboard | LibraryScope::Workflow | LibraryScope::Diagram => {
                let doc = store.get_document(item_id)?;
                if LibraryScope::of_document(doc.kind) != scope {
                    return Err(AppError::invalid_input("That item is not in this list."));
                }
            }
        }
        store.place_item(scope, item_id, folder_id)
    })
}

// ----------------------------------------------------------------------------
// Copy names
// ----------------------------------------------------------------------------

// WHAT:  The name a duplicate gets: "X (copy)", then "X (copy 2)", "X (copy 3)"…
//        — the first one `taken` does not already use.
// WHY:   Duplicating a copy should not stack "(copy) (copy)" suffixes.
// HOW:   A trailing " (copy)" / " (copy N)" on `base` is dropped first, so
//        duplicating "X (copy)" proposes "X (copy 2)".
pub fn copy_name(base: &str, taken: &[String]) -> String {
    let root = strip_copy_suffix(base.trim());
    let lowered: Vec<String> = taken.iter().map(|t| t.trim().to_lowercase()).collect();
    let is_taken = |candidate: &str| lowered.iter().any(|t| *t == candidate.to_lowercase());
    let first = format!("{root} (copy)");
    if !is_taken(&first) {
        return first;
    }
    (2u32..)
        .map(|n| format!("{root} (copy {n})"))
        .find(|candidate| !is_taken(candidate))
        .unwrap_or(first)
}

fn strip_copy_suffix(name: &str) -> &str {
    let Some(open) = name.rfind(" (copy") else { return name };
    let tail = &name[open + " (copy".len()..];
    let Some(inner) = tail.strip_suffix(')') else { return name };
    let plain = inner.is_empty();
    let numbered = inner.strip_prefix(' ').is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()));
    if (plain || numbered) && open > 0 {
        &name[..open]
    } else {
        name
    }
}

// ----------------------------------------------------------------------------
// Saved-query export
// ----------------------------------------------------------------------------

/// One saved query as the `.sql` export lists it.
pub struct ExportItem {
    pub name: String,
    /// "Reports / Weekly" — the folders above it, outermost first.
    pub folder_path: Option<String>,
    pub sql: String,
    pub updated_at: String,
}

// WHAT:  "A / B / C" for a folder, walking up its parents; None for the top level.
pub fn folder_path(folder_id: Option<&str>, folders: &[LibraryFolder]) -> Option<String> {
    let by_id: HashMap<&str, &LibraryFolder> = folders.iter().map(|f| (f.id.as_str(), f)).collect();
    let mut parts: Vec<&str> = Vec::new();
    let mut current = folder_id.and_then(|id| by_id.get(id).copied());
    // The bound stops a corrupt parent cycle from looping forever.
    while let Some(folder) = current {
        if parts.len() >= 64 {
            break;
        }
        parts.push(folder.name.as_str());
        current = folder.parent_id.as_deref().and_then(|id| by_id.get(id).copied());
    }
    if parts.is_empty() {
        None
    } else {
        parts.reverse();
        Some(parts.join(" / "))
    }
}

// WHAT:  One `.sql` file holding several saved queries, each under a `-- name:` header.
// WHY:   Handing a set of queries to a colleague or a repo: the headers keep names
//        and folders next to the text, and the file still runs as a script.
// HOW:   Sorted by folder path then name. A statement gets a closing `;` unless it
//        already has one or is JSON (a Mongo / search query), which `;` would break.
pub fn format_export(items: &[ExportItem], date: &str) -> String {
    let mut sorted: Vec<&ExportItem> = items.iter().collect();
    sorted.sort_by_key(|i| (i.folder_path.clone().unwrap_or_default().to_lowercase(), i.name.to_lowercase()));
    let noun = if sorted.len() == 1 { "query" } else { "queries" };
    let mut out = format!("-- DB Free saved queries: {} {noun}, exported {date}\n", sorted.len());
    for item in sorted {
        out.push('\n');
        out.push_str(&format!("-- name: {}\n", single_line(&item.name)));
        if let Some(path) = &item.folder_path {
            out.push_str(&format!("-- folder: {}\n", single_line(path)));
        }
        if let Some(day) = item.updated_at.get(..10) {
            out.push_str(&format!("-- updated: {day}\n"));
        }
        let body = item.sql.trim_end();
        out.push_str(body);
        if needs_terminator(body) {
            out.push(';');
        }
        out.push('\n');
    }
    out
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn needs_terminator(body: &str) -> bool {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return false;
    }
    let json = trimmed.starts_with('{') || trimmed.starts_with('[');
    !json && !trimmed.ends_with(';')
}

pub fn export_saved_queries(state: &AppState, ids: &[String]) -> AppResult<String> {
    if ids.is_empty() {
        return Err(AppError::invalid_input("Choose at least one query to export."));
    }
    let (queries, layout): (Vec<SavedQuery>, LibraryLayout) = state.with_store(|store| {
        let queries = ids.iter().map(|id| store.get_saved_query(id)).collect::<AppResult<Vec<_>>>()?;
        Ok((queries, store.list_library()?))
    })?;
    let folder_of: HashMap<&str, &str> = layout
        .placements
        .iter()
        .filter(|p| p.scope == LibraryScope::SavedQuery)
        .map(|p| (p.item_id.as_str(), p.folder_id.as_str()))
        .collect();
    let items: Vec<ExportItem> = queries
        .iter()
        .map(|q| ExportItem {
            name: q.name.clone(),
            folder_path: folder_path(folder_of.get(q.id.as_str()).copied(), &layout.folders),
            sql: q.sql.clone(),
            updated_at: q.updated_at.clone(),
        })
        .collect();
    Ok(format_export(&items, &chrono::Utc::now().format("%Y-%m-%d").to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn copies_are_numbered_and_never_stack_suffixes() {
        assert_eq!(copy_name("Sales", &[]), "Sales (copy)");
        assert_eq!(copy_name("Sales", &names(&["Sales (copy)"])), "Sales (copy 2)");
        assert_eq!(copy_name("Sales (copy)", &names(&["Sales", "Sales (copy)"])), "Sales (copy 2)");
        assert_eq!(copy_name("Sales (copy 2)", &names(&["Sales (copy)", "Sales (copy 2)"])), "Sales (copy 3)");
        assert_eq!(copy_name("Sales", &names(&["SALES (COPY)"])), "Sales (copy 2)", "comparison ignores letter case");
        assert_eq!(copy_name("  Sales  ", &[]), "Sales (copy)");
        // A name that merely contains the word keeps it.
        assert_eq!(copy_name("Copy of orders", &[]), "Copy of orders (copy)");
        assert_eq!(copy_name("Sales (copyright)", &[]), "Sales (copyright) (copy)");
    }

    fn folder(id: &str, parent: Option<&str>, name: &str) -> LibraryFolder {
        LibraryFolder {
            id: id.into(),
            scope: LibraryScope::SavedQuery,
            parent_id: parent.map(String::from),
            name: name.into(),
            sort_order: 0,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn folder_paths_walk_up_to_the_root() {
        let folders = vec![folder("a", None, "Reports"), folder("b", Some("a"), "Weekly"), folder("c", Some("b"), "Finance")];
        assert_eq!(folder_path(Some("c"), &folders).as_deref(), Some("Reports / Weekly / Finance"));
        assert_eq!(folder_path(Some("a"), &folders).as_deref(), Some("Reports"));
        assert_eq!(folder_path(None, &folders), None);
        assert_eq!(folder_path(Some("missing"), &folders), None);
        // A parent cycle terminates.
        let looped = vec![folder("x", Some("y"), "X"), folder("y", Some("x"), "Y")];
        assert!(folder_path(Some("x"), &looped).is_some());
    }

    #[test]
    fn the_export_lists_each_query_under_a_name_header() {
        let items = vec![
            ExportItem { name: "Weekly\nsignups".into(), folder_path: Some("Reports".into()), sql: "select 1\n\n".into(), updated_at: "2026-10-01T10:00:00.000Z".into() },
            ExportItem { name: "Alpha".into(), folder_path: None, sql: "select 2;".into(), updated_at: String::new() },
            ExportItem { name: "Docs".into(), folder_path: Some("Reports".into()), sql: "{\"find\":\"users\"}".into(), updated_at: String::new() },
        ];
        let out = format_export(&items, "2026-10-12");
        let expected = "-- DB Free saved queries: 3 queries, exported 2026-10-12\n\
                        \n\
                        -- name: Alpha\n\
                        select 2;\n\
                        \n\
                        -- name: Docs\n\
                        -- folder: Reports\n\
                        {\"find\":\"users\"}\n\
                        \n\
                        -- name: Weekly signups\n\
                        -- folder: Reports\n\
                        -- updated: 2026-10-01\n\
                        select 1;\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn a_single_query_export_uses_the_singular() {
        let items = vec![ExportItem { name: "Only".into(), folder_path: None, sql: "select 1".into(), updated_at: String::new() }];
        assert!(format_export(&items, "2026-10-12").starts_with("-- DB Free saved queries: 1 query, exported 2026-10-12\n"));
    }
}
