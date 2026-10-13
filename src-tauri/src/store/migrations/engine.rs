// SOT: engine-migrations, store-migration-steps

// WHAT:  Schema cache and engine state: this feature's store migration steps, in order.
// HOW:   Append a new step to change the schema; never edit a released one.
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list)
pub const STEPS: &[&str] = &[
    // 1 — the persisted schema cache (store/schema_cache.rs): one row per
    //     (connection, database, what was cached), holding the JSON the adapter
    //     returned and when. Cascades with the connection.
    "CREATE TABLE IF NOT EXISTS schema_cache (
        connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
        namespace TEXT NOT NULL,
        scope TEXT NOT NULL,
        payload TEXT NOT NULL,
        fetched_at INTEGER NOT NULL,
        PRIMARY KEY (connection_id, namespace, scope)
    );",
];
