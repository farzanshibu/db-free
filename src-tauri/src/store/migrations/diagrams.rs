// SOT: diagrams-migrations, store-migration-steps

// WHAT:  Diagram versions and customizations: this feature's store migration steps, in order.
// HOW:   Append a new step to change the schema; never edit a released one.
//        1. `diagram_customizations`: what the user changed on an ER diagram that the
//           database cannot say for them (card position, colour, bend points, pan / zoom),
//           keyed by connection + schema + kind + entity. `schema_name` is "" for the
//           all-schemas diagram so the key never holds a NULL.
//        2. `document_versions`: saved states of any document (a diagram today), numbered
//           per document. `name` is set for checkpoints the user named, which pruning
//           never touches.
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list)
pub const STEPS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS diagram_customizations (
        connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
        schema_name TEXT NOT NULL DEFAULT '',
        kind TEXT NOT NULL,
        entity_key TEXT NOT NULL,
        custom_json TEXT NOT NULL DEFAULT '{}',
        updated_at TEXT NOT NULL,
        PRIMARY KEY (connection_id, schema_name, kind, entity_key)
    );",
    "CREATE TABLE IF NOT EXISTS document_versions (
        document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
        version INTEGER NOT NULL,
        body TEXT NOT NULL,
        name TEXT,
        created_at TEXT NOT NULL,
        PRIMARY KEY (document_id, version)
    );",
];
