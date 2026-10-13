// SOT: editor-migrations, store-migration-steps

// WHAT:  Query log details: this feature's store migration steps, in order.
// WHY:   The log viewer filters by statement type and table, shows who ran a
//        statement, and tells editor runs from the app's own. Rows written
//        before these columns existed keep NULL details; the ones the user typed
//        (origin 'user') count as editor runs, so the editor's history panel
//        does not lose them.
// HOW:   Append a new step to change the schema; never edit a released one.
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list)
pub const STEPS: &[&str] = &[
    "ALTER TABLE query_history ADD COLUMN statement_type TEXT;
     ALTER TABLE query_history ADD COLUMN table_name TEXT;
     ALTER TABLE query_history ADD COLUMN operation TEXT;
     ALTER TABLE query_history ADD COLUMN db_user TEXT;
     ALTER TABLE query_history ADD COLUMN from_editor INTEGER NOT NULL DEFAULT 0;
     UPDATE query_history SET from_editor = 1 WHERE origin = 'user';",
    "CREATE INDEX IF NOT EXISTS query_history_time_idx ON query_history (executed_at);
     CREATE INDEX IF NOT EXISTS query_history_editor_idx ON query_history (connection_id, from_editor, id DESC);",
];
