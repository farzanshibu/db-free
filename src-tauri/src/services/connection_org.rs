// SOT: connection-org-service, connection-tags-service, connection-folders-service, connection-membership-service

use crate::error::AppResult;
use crate::model::{ConnectionFolder, ConnectionOrg, ConnectionSummary, ConnectionTag, ConnectionTagColor};
use crate::state::AppState;

// ============================================================================
// WHAT:  Tags and folders of the connections page: list, create, rename,
//        recolour, delete, and assign them to a connection.
// WHY:   The commands stay one-liners; the rules (case-insensitive unique names,
//        bounded length, unknown ids skipped) live with the data in the store.
// HOW:   Deleting a tag or folder removes only its links, never a connection.
// WHERE: src-tauri/src/store/connection_org.rs, src-tauri/src/commands/connection_org.rs
// ============================================================================

pub fn list(state: &AppState) -> AppResult<ConnectionOrg> {
    state.with_store(|store| store.list_connection_org())
}

pub fn save_tag(state: &AppState, id: Option<&str>, name: &str, color: ConnectionTagColor) -> AppResult<ConnectionTag> {
    state.with_store(|store| store.save_connection_tag(id, name, color))
}

pub fn delete_tag(state: &AppState, id: &str) -> AppResult<()> {
    state.with_store(|store| store.delete_connection_tag(id))
}

pub fn save_folder(state: &AppState, id: Option<&str>, name: &str) -> AppResult<ConnectionFolder> {
    state.with_store(|store| store.save_connection_folder(id, name))
}

pub fn delete_folder(state: &AppState, id: &str) -> AppResult<()> {
    state.with_store(|store| store.delete_connection_folder(id))
}

// WHAT:  Replaces a connection's tags and / or folders without touching anything else.
// WHY:   The card menu files a connection under a folder or tags it with one
//        click; saving the whole connection for that would round-trip every
//        field and could reset a live session.
// HOW:   `None` leaves that side as it is. Returns the connection as it reads now.
pub fn set_membership(
    state: &AppState,
    connection_id: &str,
    tag_ids: Option<&[String]>,
    folder_ids: Option<&[String]>,
) -> AppResult<ConnectionSummary> {
    state.with_store(|store| {
        store.set_connection_membership(connection_id, tag_ids, folder_ids)?;
        store.get_connection(connection_id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::store::Store;

    fn state() -> AppState {
        AppState::new(Store::open_in_memory().unwrap_or_else(|e| panic!("{e}")), Box::new(MemoryKeyProvider::default()))
    }

    #[test]
    fn org_lists_presets_and_new_entries_in_creation_order() {
        let state = state();
        let tag = save_tag(&state, None, "Client A", ConnectionTagColor::Teal).unwrap_or_else(|e| panic!("{e}"));
        let folder = save_folder(&state, None, "Clients").unwrap_or_else(|e| panic!("{e}"));
        let org = list(&state).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(org.tags.len(), 6);
        assert_eq!(org.tags.last().map(|t| t.id.clone()), Some(tag.id.clone()));
        assert_eq!(org.folders.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), [folder.id.as_str()]);
        delete_tag(&state, &tag.id).unwrap_or_else(|e| panic!("{e}"));
        delete_folder(&state, &folder.id).unwrap_or_else(|e| panic!("{e}"));
        let after = list(&state).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((after.tags.len(), after.folders.len()), (5, 0));
    }
}
