// SOT: schema-migrations, store-migration-steps

// WHAT:  Table tags: this feature's store migration steps, in order.
// HOW:   Append a new step to change the schema; never edit a released one.
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list)
pub const STEPS: &[&str] = &[
    // 1 - table tags, scoped to (connection, database, schema). The unique index
    //     makes "a tag named X already exists" a store-level fact, not only a check;
    //     links and per-scope state follow their tag / connection when it is deleted.
    "CREATE TABLE IF NOT EXISTS table_tags (
        id TEXT PRIMARY KEY,
        connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
        database TEXT NOT NULL DEFAULT '',
        schema TEXT NOT NULL DEFAULT '',
        name TEXT NOT NULL,
        color TEXT NOT NULL,
        position INTEGER NOT NULL DEFAULT 0,
        collapsed INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS table_tags_name_idx
        ON table_tags (connection_id, database, schema, lower(name));
    CREATE TABLE IF NOT EXISTS table_tag_links (
        tag_id TEXT NOT NULL REFERENCES table_tags(id) ON DELETE CASCADE,
        table_name TEXT NOT NULL,
        PRIMARY KEY (tag_id, table_name)
    );
    CREATE TABLE IF NOT EXISTS table_tag_scopes (
        connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
        database TEXT NOT NULL DEFAULT '',
        schema TEXT NOT NULL DEFAULT '',
        untagged_collapsed INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (connection_id, database, schema)
    );",
];

#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[test]
    fn the_tag_tables_exist_after_migrating() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        for table in ["table_tags", "table_tag_links", "table_tag_scopes"] {
            let found: i64 = store
                .conn()
                .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |row| row.get(0))
                .unwrap_or(0);
            assert_eq!(found, 1, "{table}");
        }
    }
}
