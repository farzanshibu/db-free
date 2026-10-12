// SOT: ai-migrations, store-migration-steps

// WHAT:  Saved AI chats: this feature's store migration steps, in order.
// HOW:   Append a new step to change the schema; never edit a released one.
// WHERE: src-tauri/src/store/migrations/mod.rs (applies every list)
pub const STEPS: &[&str] = &[
    // 1 — chats and their messages. A message keeps its tool calls and
    //     artifacts as JSON in `payload`, so a restored chat shows exactly what
    //     the run produced. Messages go with their chat (cascade); chats go with
    //     their connection through the trigger rather than a foreign key, because
    //     a foreign key would make any later rebuild of `connections` (drop and
    //     recreate) delete every chat as a side effect. A unit test in
    //     store/ai_chats.rs pins the behaviour.
    "CREATE TABLE ai_chats (
        id TEXT PRIMARY KEY,
        connection_id TEXT NOT NULL,
        title TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE INDEX ai_chats_connection_idx ON ai_chats (connection_id, updated_at DESC);
    CREATE TABLE ai_chat_messages (
        id TEXT PRIMARY KEY,
        chat_id TEXT NOT NULL REFERENCES ai_chats(id) ON DELETE CASCADE,
        seq INTEGER NOT NULL,
        role TEXT NOT NULL,
        text TEXT NOT NULL,
        payload TEXT NOT NULL,
        created_at TEXT NOT NULL
    );
    CREATE INDEX ai_chat_messages_chat_idx ON ai_chat_messages (chat_id, seq);
    CREATE TRIGGER ai_chats_connection_deleted AFTER DELETE ON connections
    BEGIN
        DELETE FROM ai_chats WHERE connection_id = OLD.id;
    END;",
];
