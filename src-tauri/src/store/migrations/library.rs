// SOT: library-migrations, store-migration-steps

// WHAT:  Folders, versions, pins and recents: this feature's store migration steps, in order.
// HOW:   Append a new step to change the schema; never edit a released one.
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list)
pub const STEPS: &[&str] = &[
    // 1 — folders for saved queries, dashboards, workflows and diagrams, and
    //     which folder each item sits in. Items keep their own tables untouched:
    //     a placement row is the only thing that says "this item is in that
    //     folder", so no other feature's rows need a new column.
    "
    CREATE TABLE IF NOT EXISTS library_folders (
        id TEXT PRIMARY KEY,
        scope TEXT NOT NULL,
        parent_id TEXT,
        name TEXT NOT NULL,
        sort_order INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS library_folders_scope_idx ON library_folders (scope, parent_id);
    CREATE TABLE IF NOT EXISTS library_placements (
        scope TEXT NOT NULL,
        item_id TEXT NOT NULL,
        folder_id TEXT NOT NULL,
        PRIMARY KEY (scope, item_id)
    );
    CREATE INDEX IF NOT EXISTS library_placements_folder_idx ON library_placements (folder_id);
    ",
    // 2 — saved-query versions. Every query already saved gets its current SQL
    //     as version 1, so a restore has something to go back to the first time
    //     one of them is edited.
    "
    CREATE TABLE IF NOT EXISTS saved_query_versions (
        id TEXT PRIMARY KEY,
        saved_query_id TEXT NOT NULL,
        version INTEGER NOT NULL,
        sql TEXT NOT NULL,
        created_at TEXT NOT NULL
    );
    CREATE UNIQUE INDEX IF NOT EXISTS saved_query_versions_unique_idx ON saved_query_versions (saved_query_id, version);
    INSERT INTO saved_query_versions (id, saved_query_id, version, sql, created_at)
        SELECT lower(hex(randomblob(16))), id, 1, sql, updated_at FROM saved_queries;
    ",
    // 3 — the Home board: pinned items (ordered) and the recently opened ones.
    //     `item_id` is the id of the connection / saved query / document; for a
    //     table it is `<connection id>:<schema.table>`, which is unique across
    //     connections. Rows whose item has since been deleted are dropped when
    //     the lists are read (store/home.rs).
    "
    CREATE TABLE IF NOT EXISTS home_pins (
        id TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        item_id TEXT NOT NULL,
        label TEXT NOT NULL,
        connection_id TEXT NOT NULL,
        schema_name TEXT,
        table_name TEXT,
        sort_order INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL,
        UNIQUE (kind, item_id)
    );
    CREATE TABLE IF NOT EXISTS home_recents (
        id TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        item_id TEXT NOT NULL,
        label TEXT NOT NULL,
        connection_id TEXT NOT NULL,
        schema_name TEXT,
        table_name TEXT,
        seq INTEGER NOT NULL,
        opened_at TEXT NOT NULL,
        UNIQUE (kind, item_id)
    );
    CREATE INDEX IF NOT EXISTS home_recents_seq_idx ON home_recents (seq DESC);
    ",
];
