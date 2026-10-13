// SOT: grid-migrations, store-migration-steps

// WHAT:  Per-table grid preferences: this feature's store migration steps, in order.
// HOW:   Append a new step to change the schema; never edit a released one.
//        1. user-defined foreign keys (`FkRemap`): one row per source column, gone
//           with its connection.
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list)
pub const STEPS: &[&str] = &[
    "CREATE TABLE fk_remaps (
        id TEXT PRIMARY KEY,
        connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
        from_schema TEXT,
        from_table TEXT NOT NULL,
        from_column TEXT NOT NULL,
        to_schema TEXT,
        to_table TEXT NOT NULL,
        to_column TEXT NOT NULL,
        created_at TEXT NOT NULL
    );
    CREATE UNIQUE INDEX idx_fk_remaps_source ON fk_remaps (connection_id, COALESCE(from_schema, ''), from_table, from_column);",
];
