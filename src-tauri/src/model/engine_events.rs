// SOT: schema-cache-updated-event

use serde::Serialize;
use ts_rs::TS;

// WHAT:  The event `schema-cache:updated`: a connection's persisted schema cache
//        was refreshed in the background and what it holds is no longer what
//        `load_catalog` answered with.
// WHY:   After a restart the sidebar is drawn from the cached catalog at once and
//        the live one is fetched behind it; this tells the UI when to redraw.
// WHERE: src-tauri/src/services/schema_cache.rs (raises it), src-tauri/src/lib.rs
//        (emits it), src/stores/sessions.ts (listens)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SchemaCacheUpdated {
    pub connection_id: String,
}
