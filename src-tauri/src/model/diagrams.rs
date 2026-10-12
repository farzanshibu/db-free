// SOT: diagram-shapes, diagram-note-model, diagram-customization-model, document-version-model, diagram-ddl-model, diagram-export-model

use super::connection::ConnectionColor;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  The small shapes the schema designer and the ER diagram share: sides,
//        points, notes, line widths and FK actions, plus the request/response
//        shapes of the diagram commands (customizations, versions, DDL, export).
// WHY:   `DiagramBody` and friends live in documents.rs because they ride inside
//        a `Document`; everything the diagram commands exchange on its own lives
//        here so documents.rs stays a list of document bodies.
// WHERE: src-tauri/src/model/documents.rs (DiagramBody), src-tauri/src/commands/diagrams.rs,
//        src/features/diagrams

/// A side of a table card: where a relation attaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiagramSide {
    Top,
    Right,
    Bottom,
    Left,
}

/// The four stroke widths the designer offers for a relation line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiagramLineWidth {
    Thin,
    Normal,
    Thick,
    Heavy,
}

/// The referential actions SQL knows for `ON DELETE` / `ON UPDATE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ReferentialAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ReferentialAction {
    /// The clause spelling (`SET NULL`), shared by every dialect that supports it.
    pub fn sql(self) -> &'static str {
        match self {
            ReferentialAction::NoAction => "NO ACTION",
            ReferentialAction::Restrict => "RESTRICT",
            ReferentialAction::Cascade => "CASCADE",
            ReferentialAction::SetNull => "SET NULL",
            ReferentialAction::SetDefault => "SET DEFAULT",
        }
    }
}

/// A point on the canvas, in flow coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramPoint {
    pub x: f64,
    pub y: f64,
}

/// One more column pair of a composite foreign key (the first pair is
/// `DiagramRelation.from_column` / `to_column`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramColumnPair {
    pub from_column: String,
    pub to_column: String,
}

fn default_note_width() -> f64 {
    220.0
}

fn default_note_height() -> f64 {
    120.0
}

// WHAT:  A sticky note on the designer canvas.
// WHY:   Notes explain a diagram to whoever opens it next; they never reach SQL.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramNote {
    pub id: String,
    pub x: f64,
    pub y: f64,
    #[serde(default = "default_note_width")]
    pub width: f64,
    #[serde(default = "default_note_height")]
    pub height: f64,
    #[serde(default)]
    pub text: String,
    /// A palette key, drawn through the `--color-conn-*` tokens. None = amber.
    #[serde(default)]
    pub color: Option<ConnectionColor>,
    /// Text size in px. None = 12.
    #[serde(default)]
    pub font_size: Option<u32>,
    /// Left out of PNG / SVG exports.
    #[serde(default)]
    pub hide_on_export: bool,
}

// ------------------------------------------------------------ customizations

/// What a stored customization belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiagramCustomKind {
    /// A table card on an ER diagram: position and colour. Key: `schema.table`.
    Node,
    /// A relation line: bend points. Key: `from->to:constraint`.
    Edge,
    /// The pan / zoom of the whole diagram. Key: `main`.
    Viewport,
}

impl DiagramCustomKind {
    pub const ALL: [DiagramCustomKind; 3] = [DiagramCustomKind::Node, DiagramCustomKind::Edge, DiagramCustomKind::Viewport];

    pub fn as_str(self) -> &'static str {
        match self {
            DiagramCustomKind::Node => "node",
            DiagramCustomKind::Edge => "edge",
            DiagramCustomKind::Viewport => "viewport",
        }
    }

    pub fn parse(raw: &str) -> Option<DiagramCustomKind> {
        DiagramCustomKind::ALL.into_iter().find(|k| k.as_str() == raw)
    }
}

// WHAT:  The payload of one customization. A flat bag of optionals so one typed
//        shape serves all three kinds; each kind reads only the fields it owns.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramCustom {
    /// Node: top-left corner. Viewport: pan offset.
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    /// Viewport: zoom factor.
    #[serde(default)]
    pub zoom: Option<f64>,
    /// Node: header colour.
    #[serde(default)]
    pub color: Option<ConnectionColor>,
    /// Edge: bend points in order from the source to the target.
    #[serde(default)]
    pub waypoints: Vec<DiagramPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramCustomization {
    pub kind: DiagramCustomKind,
    pub entity_key: String,
    pub custom: DiagramCustom,
}

/// Names one customization without its payload (for removals).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramCustomizationKey {
    pub kind: DiagramCustomKind,
    pub entity_key: String,
}

// ------------------------------------------------------------------ versions

// WHAT:  One saved state of a document, without its body.
// WHY:   The history popover lists up to a few dozen of these; sending the bodies
//        too would ship every snapshot of a large diagram just to draw a list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentVersionSummary {
    pub document_id: String,
    pub version: u32,
    /// Set for a checkpoint the user named; automatic versions have none.
    pub name: Option<String>,
    pub created_at: String,
    /// "3 tables · 2 relations · 1 note" — what the body holds, in words.
    pub summary: String,
}

// ----------------------------------------------------------------- ddl/export

// WHAT:  Generated SQL plus whatever the dialect could not express.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagramDdl {
    pub sql: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiagramExportFormat {
    Png,
    Svg,
}

impl DiagramExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            DiagramExportFormat::Png => "png",
            DiagramExportFormat::Svg => "svg",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiagramBody, DocumentBody};

    // A diagram saved by the first release: no schema, colour, notes or FK actions.
    const FIRST_RELEASE: &str = r#"{"kind":"diagram","data":{
        "tables":[{"id":"t1","name":"users","x":10,"y":20,"columns":[
            {"name":"id","dataType":"integer","primaryKey":true,"nullable":false},
            {"name":"email","dataType":"text"}]}],
        "relations":[{"id":"r1","fromTable":"t1","fromColumn":"id","toTable":"t1","toColumn":"id"}]}}"#;

    #[test]
    fn diagrams_saved_before_these_fields_still_load_with_defaults() {
        let body: DocumentBody = serde_json::from_str(FIRST_RELEASE).unwrap_or_else(|e| panic!("{e}"));
        let DocumentBody::Diagram(d) = body else { panic!("not a diagram") };
        let table = &d.tables[0];
        assert_eq!(table.schema, None);
        assert_eq!(table.color, None);
        assert!(table.columns[1].nullable, "a column without `nullable` is nullable");
        assert!(!table.columns[0].unique && !table.columns[0].auto_increment);
        let rel = &d.relations[0];
        assert!(rel.extra_columns.is_empty() && rel.waypoints.is_empty());
        assert!(rel.on_delete.is_none() && rel.from_side.is_none());
        assert!(d.notes.is_empty() && d.grid_size.is_none() && d.notes_visible.is_none());
    }

    #[test]
    fn empty_extras_are_not_written_and_new_fields_round_trip() {
        let mut body: DiagramBody = serde_json::from_str::<DocumentBody>(FIRST_RELEASE)
            .ok()
            .and_then(|b| if let DocumentBody::Diagram(d) = b { Some(d) } else { None })
            .unwrap_or_default();
        let json = serde_json::to_value(&body).unwrap_or_default();
        assert!(json.get("notes").is_none() && json.get("gridSize").is_none(), "{json}");

        body.grid_size = Some(24);
        body.notes.push(DiagramNote {
            id: "n1".into(),
            x: 1.0,
            y: 2.0,
            width: 200.0,
            height: 80.0,
            text: "hi".into(),
            color: Some(ConnectionColor::Teal),
            font_size: Some(14),
            hide_on_export: true,
        });
        if let Some(rel) = body.relations.first_mut() {
            rel.on_delete = Some(ReferentialAction::SetNull);
            rel.from_side = Some(DiagramSide::Bottom);
            rel.width = Some(DiagramLineWidth::Heavy);
            rel.extra_columns.push(DiagramColumnPair { from_column: "a".into(), to_column: "b".into() });
        }
        let text = serde_json::to_string(&body).unwrap_or_default();
        let back: DiagramBody = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.grid_size, Some(24));
        assert_eq!(back.notes[0].color, Some(ConnectionColor::Teal));
        assert_eq!(back.relations[0].on_delete, Some(ReferentialAction::SetNull));
        assert!(text.contains("\"onDelete\":\"set_null\"") && text.contains("\"fromSide\":\"bottom\""), "{text}");
        assert_eq!(back.relations[0].pairs(), vec![("id", "id"), ("a", "b")]);
    }

    #[test]
    fn actions_spell_as_sql() {
        assert_eq!(ReferentialAction::SetDefault.sql(), "SET DEFAULT");
        assert_eq!(ReferentialAction::NoAction.sql(), "NO ACTION");
    }

    #[test]
    fn custom_kinds_round_trip_through_their_storage_names() {
        for kind in DiagramCustomKind::ALL {
            assert_eq!(DiagramCustomKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(DiagramCustomKind::parse("nope"), None);
        assert_eq!(DiagramExportFormat::Png.extension(), "png");
    }
}
