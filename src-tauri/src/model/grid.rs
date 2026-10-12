// SOT: fk-remap, user-defined-foreign-key, grid-preferences-model

use crate::model::TableRef;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  A foreign key the user declared (or redirected) for one column.
// WHY:   Many databases have no FOREIGN KEY constraints (or point a column at a view or
//        a differently named table); the grid's peek, follow and pickers still
//        want to know where `orders.user_id` leads.
// HOW:   Keyed by the source column; saving again replaces it. It outranks the
//        catalog's constraint for that column and the name-based inference.
// WHERE: src-tauri/src/store/fk_remaps.rs, src/features/grid/fkRemaps.ts
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FkRemap {
    pub id: String,
    pub connection_id: String,
    pub from: TableRef,
    pub from_column: String,
    pub to: TableRef,
    pub to_column: String,
    pub created_at: String,
}
