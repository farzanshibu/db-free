// SOT: library-model, library-folder-model, saved-query-version-model, home-pin-model, home-recent-model

use crate::model::DocumentKind;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  The four kinds of thing a folder can hold: saved queries and the three
//        document kinds. Folder trees are separate per scope.
// WHY:   One folder implementation serves every sidebar list; the scope is what
//        keeps a "Reports" folder for dashboards apart from one for queries.
// WHERE: src-tauri/src/store/library.rs, src/features/documents/LibraryTree.tsx
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LibraryScope {
    SavedQuery,
    Dashboard,
    Workflow,
    Diagram,
}

impl LibraryScope {
    pub const ALL: [LibraryScope; 4] = [LibraryScope::SavedQuery, LibraryScope::Dashboard, LibraryScope::Workflow, LibraryScope::Diagram];

    pub fn as_str(self) -> &'static str {
        match self {
            LibraryScope::SavedQuery => "saved_query",
            LibraryScope::Dashboard => "dashboard",
            LibraryScope::Workflow => "workflow",
            LibraryScope::Diagram => "diagram",
        }
    }

    pub fn parse(raw: &str) -> Option<LibraryScope> {
        LibraryScope::ALL.into_iter().find(|s| s.as_str() == raw)
    }

    /// The scope a document of this kind lives in.
    pub fn of_document(kind: DocumentKind) -> LibraryScope {
        match kind {
            DocumentKind::Dashboard => LibraryScope::Dashboard,
            DocumentKind::Workflow => LibraryScope::Workflow,
            DocumentKind::Diagram => LibraryScope::Diagram,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryFolder {
    pub id: String,
    pub scope: LibraryScope,
    /// None = a top-level folder.
    pub parent_id: Option<String>,
    pub name: String,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

// WHAT:  "This item sits in that folder". An item without a placement is at the top level.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryPlacement {
    pub scope: LibraryScope,
    pub item_id: String,
    pub folder_id: String,
}

// WHAT:  Every folder and placement in one read; the sidebars join it with the
//        saved-query and document lists they already hold.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryLayout {
    pub folders: Vec<LibraryFolder>,
    pub placements: Vec<LibraryPlacement>,
}

// WHAT:  One earlier SQL text of a saved query. Newest first when listed.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedQueryVersion {
    pub id: String,
    pub saved_query_id: String,
    /// 1-based, counting up for as long as the query exists (old ones are pruned).
    pub version: u32,
    pub sql: String,
    pub created_at: String,
}

// WHAT:  What a Home pin or recent can point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HomeItemKind {
    Connection,
    Table,
    SavedQuery,
    Dashboard,
    Workflow,
    Diagram,
}

impl HomeItemKind {
    pub const ALL: [HomeItemKind; 6] = [
        HomeItemKind::Connection,
        HomeItemKind::Table,
        HomeItemKind::SavedQuery,
        HomeItemKind::Dashboard,
        HomeItemKind::Workflow,
        HomeItemKind::Diagram,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            HomeItemKind::Connection => "connection",
            HomeItemKind::Table => "table",
            HomeItemKind::SavedQuery => "saved_query",
            HomeItemKind::Dashboard => "dashboard",
            HomeItemKind::Workflow => "workflow",
            HomeItemKind::Diagram => "diagram",
        }
    }

    pub fn parse(raw: &str) -> Option<HomeItemKind> {
        HomeItemKind::ALL.into_iter().find(|k| k.as_str() == raw)
    }
}

// WHAT:  The thing a pin or recent opens: enough to reconnect and open it, plus
//        the label it had when it was recorded (the UI shows the live name when
//        the item still exists).
// HOW:   `item_id` is the connection / saved query / document id; for a table it
//        is `<connection id>:<schema.table>`. `connection_id` is the connection
//        to open it on (for a connection, its own id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HomeEntity {
    pub kind: HomeItemKind,
    pub item_id: String,
    pub label: String,
    pub connection_id: String,
    pub schema_name: Option<String>,
    pub table_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HomePin {
    pub id: String,
    pub entity: HomeEntity,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HomeRecent {
    pub id: String,
    pub entity: HomeEntity,
    pub opened_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_and_kind_names_round_trip() {
        for scope in LibraryScope::ALL {
            assert_eq!(LibraryScope::parse(scope.as_str()), Some(scope));
            assert_eq!(serde_json::to_string(&scope).unwrap_or_default(), format!("\"{}\"", scope.as_str()));
        }
        for kind in HomeItemKind::ALL {
            assert_eq!(HomeItemKind::parse(kind.as_str()), Some(kind));
            assert_eq!(serde_json::to_string(&kind).unwrap_or_default(), format!("\"{}\"", kind.as_str()));
        }
        assert_eq!(LibraryScope::parse("nope"), None);
    }

    #[test]
    fn documents_map_to_their_own_scope() {
        for kind in DocumentKind::ALL {
            assert_eq!(LibraryScope::of_document(kind).as_str(), kind.as_str());
            assert_eq!(HomeItemKind::parse(kind.as_str()).map(HomeItemKind::as_str), Some(kind.as_str()));
        }
    }
}
