// SOT: saved-queries-service

use crate::error::{AppError, AppResult};
use crate::model::{LibraryScope, SavedQuery, SavedQueryVersion};
use crate::services::library::copy_name;
use crate::state::AppState;

/// Longest saved-query name accepted; the sidebar truncates long ones anyway.
const MAX_NAME: usize = 200;

pub fn list(state: &AppState) -> AppResult<Vec<SavedQuery>> {
    state.with_store(|store| store.list_saved_queries())
}

fn clean_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::invalid_input("A saved query needs a name."));
    }
    if name.chars().count() > MAX_NAME {
        return Err(AppError::invalid_input(format!("A saved query name can be at most {MAX_NAME} characters.")));
    }
    Ok(name.to_string())
}

fn name_clash(name: &str) -> AppError {
    AppError::invalid_input(format!("A saved query named “{name}” already exists in this folder."))
}

// WHAT:  Saves a query (new when `id` is empty), optionally into a folder.
// WHY:   Every save path — the save dialog, ⌘S over a linked tab, a rename — ends
//        here, so the duplicate-name rule and the version history apply to all of them.
// HOW:   `folder_id` None keeps the folder a saved query is already in (top level
//        for a new one). The name must be unique among the queries of the same
//        connection scope and folder, unless this save leaves an existing query's
//        name and place unchanged — an old duplicate must stay editable.
pub fn save(state: &AppState, query: &SavedQuery, folder_id: Option<&str>) -> AppResult<SavedQuery> {
    let name = clean_name(&query.name)?;
    state.with_store(|store| {
        if let Some(folder) = folder_id {
            let target = store.get_folder(folder)?;
            if target.scope != LibraryScope::SavedQuery {
                return Err(AppError::invalid_input("That folder belongs to a different list."));
            }
        }
        let existing = if query.id.is_empty() { None } else { store.get_saved_query(&query.id).ok() };
        let current_folder = match &existing {
            Some(found) => store.folder_of(LibraryScope::SavedQuery, &found.id)?,
            None => None,
        };
        let target_folder = folder_id.map(str::to_string).or_else(|| current_folder.clone());
        let unchanged = existing.as_ref().is_some_and(|found| {
            found.name.trim().to_lowercase() == name.to_lowercase() && found.connection_id == query.connection_id && target_folder == current_folder
        });
        if !unchanged {
            let except = existing.as_ref().map(|found| found.id.as_str());
            if store.saved_query_name_taken(query.connection_id.as_deref(), target_folder.as_deref(), &name, except)? {
                return Err(name_clash(&name));
            }
        }
        let saved = store.upsert_saved_query(&SavedQuery { name, ..query.clone() })?;
        if let Some(folder) = folder_id {
            store.place_item(LibraryScope::SavedQuery, &saved.id, Some(folder))?;
        }
        Ok(saved)
    })
}

pub fn rename(state: &AppState, id: &str, name: &str) -> AppResult<SavedQuery> {
    let name = clean_name(name)?;
    state.with_store(|store| {
        let current = store.get_saved_query(id)?;
        let folder = store.folder_of(LibraryScope::SavedQuery, id)?;
        if store.saved_query_name_taken(current.connection_id.as_deref(), folder.as_deref(), &name, Some(id))? {
            return Err(name_clash(&name));
        }
        store.rename_saved_query(id, &name)
    })
}

// WHAT:  Copies a saved query, in the same folder. `target_connection_id` files the
//        copy under another connection ("Copy to connection"); None keeps it here.
// HOW:   Without an explicit `name` a same-connection copy is "X (copy)"; a copy
//        under another connection keeps the name unless that connection already
//        has one by it. History is not copied: the copy starts at version 1.
pub fn duplicate(state: &AppState, id: &str, target_connection_id: Option<&str>, name: Option<&str>) -> AppResult<SavedQuery> {
    let explicit = name.map(clean_name).transpose()?;
    state.with_store(|store| {
        let original = store.get_saved_query(id)?;
        let folder = store.folder_of(LibraryScope::SavedQuery, id)?;
        let cross = target_connection_id.is_some_and(|target| Some(target) != original.connection_id.as_deref());
        if cross {
            if let Some(target) = target_connection_id {
                if !store.connection_exists(target)? {
                    return Err(AppError::not_found("The connection to copy to no longer exists."));
                }
            }
        }
        let connection_id = if cross { target_connection_id.map(str::to_string) } else { original.connection_id.clone() };
        let taken = store.saved_query_names_in(connection_id.as_deref(), folder.as_deref())?;
        let name = match explicit {
            Some(chosen) => {
                if taken.iter().any(|t| t.trim().to_lowercase() == chosen.to_lowercase()) {
                    return Err(name_clash(&chosen));
                }
                chosen
            }
            None if cross && !taken.iter().any(|t| t.trim().to_lowercase() == original.name.trim().to_lowercase()) => original.name.clone(),
            None => copy_name(&original.name, &taken),
        };
        let copy = store.upsert_saved_query(&SavedQuery {
            id: String::new(),
            connection_id,
            name,
            sql: original.sql,
            tags: original.tags,
            created_at: String::new(),
            updated_at: String::new(),
        })?;
        if let Some(folder) = folder {
            store.place_item(LibraryScope::SavedQuery, &copy.id, Some(&folder))?;
        }
        Ok(copy)
    })
}

pub fn versions(state: &AppState, id: &str) -> AppResult<Vec<SavedQueryVersion>> {
    state.with_store(|store| store.list_saved_query_versions(id))
}

pub fn restore_version(state: &AppState, version_id: &str) -> AppResult<SavedQuery> {
    state.with_store(|store| store.restore_saved_query_version(version_id))
}

pub fn delete(state: &AppState, id: &str) -> AppResult<()> {
    state.with_store(|store| store.delete_saved_query(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::store::Store;

    fn state() -> AppState {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        AppState::new(store, Box::new(MemoryKeyProvider::default()))
    }

    fn query(name: &str, sql: &str, connection: Option<&str>) -> SavedQuery {
        SavedQuery {
            id: String::new(),
            connection_id: connection.map(String::from),
            name: name.into(),
            sql: sql.into(),
            tags: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn folder(state: &AppState, name: &str) -> String {
        state.with_store(|s| s.create_folder(LibraryScope::SavedQuery, None, name)).unwrap_or_else(|e| panic!("{e}")).id
    }

    #[test]
    fn a_name_must_be_unique_within_its_folder() {
        let state = state();
        let reports = folder(&state, "Reports");
        let first = save(&state, &query("Revenue", "select 1", None), Some(&reports)).unwrap_or_else(|e| panic!("{e}"));
        // Same name, same folder: refused. Same name at the top level: fine.
        assert!(save(&state, &query("revenue", "select 2", None), Some(&reports)).is_err());
        assert!(save(&state, &query("Revenue", "select 2", None), None).is_ok());
        // Overwriting the query itself (same id) is not a clash, and keeps its folder.
        let edited = save(&state, &SavedQuery { sql: "select 3".into(), ..first.clone() }, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(edited.sql, "select 3");
        let placed = state.with_store(|s| s.folder_of(LibraryScope::SavedQuery, &first.id)).unwrap_or_default();
        assert_eq!(placed.as_deref(), Some(reports.as_str()));
        assert!(save(&state, &query("  ", "select 1", None), None).is_err());
    }

    #[test]
    fn an_old_duplicate_stays_editable() {
        let state = state();
        // Two queries that already share a name (saved before the rule existed).
        let a = state.with_store(|s| s.upsert_saved_query(&query("Same", "select 1", None))).unwrap_or_else(|e| panic!("{e}"));
        state.with_store(|s| s.upsert_saved_query(&query("Same", "select 2", None))).unwrap_or_else(|e| panic!("{e}"));
        assert!(save(&state, &SavedQuery { sql: "select 9".into(), ..a }, None).is_ok());
    }

    #[test]
    fn rename_and_duplicate_respect_the_name_rule() {
        let state = state();
        let a = save(&state, &query("A", "select 1", None), None).unwrap_or_else(|e| panic!("{e}"));
        save(&state, &query("B", "select 2", None), None).unwrap_or_else(|e| panic!("{e}"));
        assert!(rename(&state, &a.id, "b").is_err());
        assert_eq!(rename(&state, &a.id, "A2").unwrap_or_else(|e| panic!("{e}")).name, "A2");
        let copy = duplicate(&state, &a.id, None, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(copy.name, "A2 (copy)");
        assert_eq!(copy.sql, "select 1");
        assert!(duplicate(&state, &a.id, None, Some("B")).is_err());
    }

    #[test]
    fn copying_to_another_connection_files_it_under_that_connection() {
        let state = state();
        state.with_store(|s| s.insert_test_connection("c1")).unwrap_or_else(|e| panic!("{e}"));
        state.with_store(|s| s.insert_test_connection("c2")).unwrap_or_else(|e| panic!("{e}"));
        let reports = folder(&state, "Reports");
        let original = save(&state, &query("Revenue", "select 1", Some("c1")), Some(&reports)).unwrap_or_else(|e| panic!("{e}"));
        let copy = duplicate(&state, &original.id, Some("c2"), None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((copy.name.as_str(), copy.connection_id.as_deref()), ("Revenue", Some("c2")));
        let placed = state.with_store(|s| s.folder_of(LibraryScope::SavedQuery, &copy.id)).unwrap_or_default();
        assert_eq!(placed.as_deref(), Some(reports.as_str()));
        assert!(duplicate(&state, &original.id, Some("ghost"), None).is_err());
        // The copy starts its own history.
        assert_eq!(versions(&state, &copy.id).unwrap_or_default().len(), 1);
    }

    #[test]
    fn saving_changed_sql_adds_a_version_and_restoring_goes_back() {
        let state = state();
        let q = save(&state, &query("Q", "select 1", None), None).unwrap_or_else(|e| panic!("{e}"));
        save(&state, &SavedQuery { sql: "select 2".into(), ..q.clone() }, None).unwrap_or_else(|e| panic!("{e}"));
        let listed = versions(&state, &q.id).unwrap_or_default();
        assert_eq!(listed.len(), 2);
        let oldest = listed.last().unwrap_or_else(|| panic!("no versions"));
        let restored = restore_version(&state, &oldest.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(restored.sql, "select 1");
        assert_eq!(versions(&state, &q.id).unwrap_or_default().len(), 3);
    }
}
