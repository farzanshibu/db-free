// SOT: home-service, pins-service, recents-service, home-entity-validation

use crate::error::{AppError, AppResult};
use crate::model::{HomeEntity, HomeItemKind, HomePin, HomeRecent};
use crate::state::AppState;

/// Longest label stored; the Home cards truncate far sooner.
const MAX_LABEL: usize = 200;

// WHAT:  Checks a pin / recent request and tidies it: ids present, a label to show,
//        and the table fields only where they mean something.
// WHY:   The row outlives the item's name and is shown on the Home screen, so a
//        blank label or a table pin with no table would be unreadable there.
pub fn clean_entity(entity: &HomeEntity) -> AppResult<HomeEntity> {
    let item_id = entity.item_id.trim();
    let connection_id = entity.connection_id.trim();
    if item_id.is_empty() || connection_id.is_empty() {
        return Err(AppError::invalid_input("Nothing to pin: the item or its connection is missing."));
    }
    let mut label: String = entity.label.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        return Err(AppError::invalid_input("An item needs a name to appear on Home."));
    }
    if label.chars().count() > MAX_LABEL {
        label = label.chars().take(MAX_LABEL).collect();
    }
    let table_name = entity.table_name.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    if entity.kind == HomeItemKind::Table && table_name.is_none() {
        return Err(AppError::invalid_input("A table pin needs the table's name."));
    }
    let is_table = entity.kind == HomeItemKind::Table;
    Ok(HomeEntity {
        kind: entity.kind,
        item_id: item_id.to_string(),
        label,
        connection_id: connection_id.to_string(),
        schema_name: if is_table { entity.schema_name.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string) } else { None },
        table_name: if is_table { table_name } else { None },
    })
}

pub fn pins(state: &AppState) -> AppResult<Vec<HomePin>> {
    state.with_store(|store| store.list_pins())
}

pub fn pin(state: &AppState, entity: &HomeEntity) -> AppResult<()> {
    let entity = clean_entity(entity)?;
    state.with_store(|store| store.pin_item(&entity))
}

pub fn unpin(state: &AppState, kind: HomeItemKind, item_id: &str) -> AppResult<()> {
    state.with_store(|store| store.unpin_item(kind, item_id))
}

pub fn reorder(state: &AppState, ids: &[String]) -> AppResult<()> {
    state.with_store(|store| store.reorder_pins(ids))
}

pub fn recents(state: &AppState) -> AppResult<Vec<HomeRecent>> {
    state.with_store(|store| store.list_recents())
}

pub fn record(state: &AppState, entity: &HomeEntity) -> AppResult<()> {
    let entity = clean_entity(entity)?;
    state.with_store(|store| store.record_recent(&entity))
}

pub fn clear_recents(state: &AppState) -> AppResult<()> {
    state.with_store(|store| store.clear_recents())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(kind: HomeItemKind) -> HomeEntity {
        HomeEntity {
            kind,
            item_id: " q1 ".into(),
            label: "  Top   customers \n".into(),
            connection_id: "c1".into(),
            schema_name: Some(" public ".into()),
            table_name: Some(" users ".into()),
        }
    }

    #[test]
    fn labels_are_tidied_and_table_fields_kept_only_for_tables() {
        let table = clean_entity(&entity(HomeItemKind::Table)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(table.item_id, "q1");
        assert_eq!(table.label, "Top customers");
        assert_eq!((table.schema_name.as_deref(), table.table_name.as_deref()), (Some("public"), Some("users")));
        let query = clean_entity(&entity(HomeItemKind::SavedQuery)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((query.schema_name, query.table_name), (None, None));
    }

    #[test]
    fn incomplete_entities_are_refused() {
        assert!(clean_entity(&HomeEntity { item_id: "  ".into(), ..entity(HomeItemKind::Dashboard) }).is_err());
        assert!(clean_entity(&HomeEntity { connection_id: String::new(), ..entity(HomeItemKind::Dashboard) }).is_err());
        assert!(clean_entity(&HomeEntity { label: " \n ".into(), ..entity(HomeItemKind::Dashboard) }).is_err());
        assert!(clean_entity(&HomeEntity { table_name: None, ..entity(HomeItemKind::Table) }).is_err());
        let long = clean_entity(&HomeEntity { label: "x".repeat(500), ..entity(HomeItemKind::Workflow) }).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(long.label.chars().count(), MAX_LABEL);
    }
}
