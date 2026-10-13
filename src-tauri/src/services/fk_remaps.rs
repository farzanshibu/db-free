// SOT: fk-remaps-service, user-defined-foreign-keys

use crate::error::{AppError, AppResult};
use crate::model::FkRemap;
use crate::state::AppState;

pub fn list(state: &AppState, connection_id: &str) -> AppResult<Vec<FkRemap>> {
    state.with_store(|store| store.list_fk_remaps(connection_id))
}

// WHAT:  Saves where a column's foreign key leads, replacing the column's earlier remap.
// HOW:   Every name must be non-empty; the target is not checked against the
//        catalog, because a remap may name a table in another schema the session
//        has not loaded — the grid's peek reports "not found" if it is wrong.
pub fn save(state: &AppState, remap: &FkRemap) -> AppResult<FkRemap> {
    let blank = |s: &str| s.trim().is_empty();
    if blank(&remap.connection_id) || blank(&remap.from.name) || blank(&remap.from_column) || blank(&remap.to.name) || blank(&remap.to_column) {
        return Err(AppError::invalid_input("A foreign key needs a source table and column and a target table and column."));
    }
    state.with_store(|store| store.upsert_fk_remap(remap))
}

pub fn delete(state: &AppState, id: &str) -> AppResult<()> {
    state.with_store(|store| store.delete_fk_remap(id))
}
