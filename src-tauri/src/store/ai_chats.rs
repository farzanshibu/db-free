// SOT: ai-chats-table, ai-chat-messages, chat-persistence, chat-history

use crate::error::{AppError, AppResult};
use crate::model::{AgentTurn, AiChatMessage, AiChatRole, AiChatSummary, ChatMention};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

// WHAT:  Saved assistant chats: one row per chat, one per message, per connection.
// WHY:   A conversation with the database outlives the tab. The chat list and the
//        messages live here; the model's own memory (the transcript it was
//        shown) is rebuilt from the text of these rows when a chat resumes.
// HOW:   A message's structured parts — mentions, the whole finished turn with
//        its tool calls and artifacts, a failure — are one JSON `payload`, so
//        the shapes can grow without a migration per field. Chats are removed
//        with their connection by the trigger in store/migrations/ai.rs.
// WHERE: src-tauri/src/services/ai_chats.rs (the only caller)

/// The JSON that rides in `ai_chat_messages.payload`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Payload {
    #[serde(default)]
    mentions: Vec<ChatMention>,
    #[serde(default)]
    turn: Option<AgentTurn>,
    #[serde(default)]
    error: Option<String>,
}

const CHAT_COLUMNS: &str = "c.id, c.connection_id, c.title, c.created_at, c.updated_at, \
     (SELECT COUNT(*) FROM ai_chat_messages m WHERE m.chat_id = c.id)";

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<AiChatSummary> {
    let count: i64 = row.get(5)?;
    Ok(AiChatSummary {
        id: row.get(0)?,
        connection_id: row.get(1)?,
        title: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        message_count: u32::try_from(count).unwrap_or(u32::MAX),
    })
}

fn message_from_row(row: &Row<'_>) -> rusqlite::Result<AiChatMessage> {
    let role_raw: String = row.get(1)?;
    let payload_raw: String = row.get(3)?;
    // A payload this build cannot read (written by a newer one) still shows its text.
    let payload: Payload = serde_json::from_str(&payload_raw).unwrap_or_default();
    Ok(AiChatMessage {
        id: row.get(0)?,
        role: AiChatRole::parse(&role_raw).unwrap_or(AiChatRole::Assistant),
        text: row.get(2)?,
        created_at: row.get(4)?,
        mentions: payload.mentions,
        turn: payload.turn,
        error: payload.error,
    })
}

impl Store {
    /// A connection's chats, most recently active first.
    pub fn list_ai_chats(&self, connection_id: &str) -> AppResult<Vec<AiChatSummary>> {
        let sql = format!(
            "SELECT {CHAT_COLUMNS} FROM ai_chats c WHERE c.connection_id = ?1 ORDER BY c.updated_at DESC, c.rowid DESC"
        );
        let mut stmt = self.conn().prepare(&sql).map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![connection_id], summary_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    pub fn find_ai_chat(&self, id: &str) -> AppResult<Option<AiChatSummary>> {
        let sql = format!("SELECT {CHAT_COLUMNS} FROM ai_chats c WHERE c.id = ?1");
        self.conn().query_row(&sql, params![id], summary_from_row).optional().map_err(AppError::store)
    }

    // WHAT:  Creates the chat the first time a message is sent in it.
    // WHY:   The UI mints chat ids, so "New chat" costs nothing and an abandoned
    //        one leaves no empty row behind.
    // HOW:   An existing chat is returned as it is. One that belongs to another
    //        connection is refused: a chat id is not a way into someone else's.
    pub fn ensure_ai_chat(&self, id: &str, connection_id: &str, title: &str) -> AppResult<AiChatSummary> {
        if let Some(existing) = self.find_ai_chat(id)? {
            if existing.connection_id != connection_id {
                return Err(AppError::invalid_input("That chat belongs to a different connection."));
            }
            return Ok(existing);
        }
        let now = now_rfc3339();
        self.conn()
            .execute(
                "INSERT INTO ai_chats (id, connection_id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![id, connection_id, title, now],
            )
            .map_err(AppError::store)?;
        self.find_ai_chat(id)?.ok_or_else(|| AppError::store("the chat vanished after it was created"))
    }

    // WHAT:  Appends a message and bumps the chat's activity time.
    // HOW:   `seq` is the next number in the chat; the id makes a repeated
    //        append of the same message an update rather than a duplicate.
    pub fn append_ai_message(&self, chat_id: &str, message: &AiChatMessage) -> AppResult<()> {
        let payload = serde_json::to_string(&Payload {
            mentions: message.mentions.clone(),
            turn: message.turn.clone(),
            error: message.error.clone(),
        })
        .map_err(AppError::store)?;
        let next: i64 = self
            .conn()
            .query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM ai_chat_messages WHERE chat_id = ?1", params![chat_id], |row| row.get(0))
            .map_err(AppError::store)?;
        self.conn()
            .execute(
                "INSERT INTO ai_chat_messages (id, chat_id, seq, role, text, payload, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(id) DO UPDATE SET text = excluded.text, payload = excluded.payload",
                params![message.id, chat_id, next, message.role.as_str(), message.text, payload, message.created_at],
            )
            .map_err(AppError::store)?;
        self.conn()
            .execute("UPDATE ai_chats SET updated_at = ?2 WHERE id = ?1", params![chat_id, now_rfc3339()])
            .map_err(AppError::store)?;
        Ok(())
    }

    pub fn list_ai_messages(&self, chat_id: &str) -> AppResult<Vec<AiChatMessage>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id, role, text, payload, created_at FROM ai_chat_messages WHERE chat_id = ?1 ORDER BY seq")
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![chat_id], message_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    pub fn rename_ai_chat(&self, id: &str, title: &str) -> AppResult<()> {
        let changed = self
            .conn()
            .execute("UPDATE ai_chats SET title = ?2 WHERE id = ?1", params![id, title])
            .map_err(AppError::store)?;
        if changed == 0 {
            return Err(AppError::not_found("That chat no longer exists."));
        }
        Ok(())
    }

    /// Deleting a chat that is already gone is not an error: the goal state holds.
    pub fn delete_ai_chat(&self, id: &str) -> AppResult<()> {
        self.conn().execute("DELETE FROM ai_chats WHERE id = ?1", params![id]).map_err(AppError::store)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentStop, AgentUsage};

    fn store_with_connection(id: &str) -> Store {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        store
            .conn()
            .execute(
                "INSERT INTO connections (id, name, engine, environment, created_at, updated_at) VALUES (?1, 'c', 'sqlite', 'local', 'now', 'now')",
                params![id],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        store
    }

    fn message(id: &str, role: AiChatRole, text: &str) -> AiChatMessage {
        AiChatMessage {
            id: id.into(),
            role,
            text: text.into(),
            created_at: now_rfc3339(),
            mentions: Vec::new(),
            turn: None,
            error: None,
        }
    }

    #[test]
    fn chats_and_messages_round_trip_in_order() {
        let store = store_with_connection("c1");
        let chat = store.ensure_ai_chat("chat-1", "c1", "Top customers").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(chat.title, "Top customers");
        assert_eq!(chat.message_count, 0);

        store.append_ai_message("chat-1", &message("u-1", AiChatRole::User, "who buys most?")).unwrap_or_else(|e| panic!("{e}"));
        let mut reply = message("a-1", AiChatRole::Assistant, "Acme.");
        reply.turn = Some(AgentTurn {
            run_id: "r1".into(),
            model: "m".into(),
            text: "Acme.".into(),
            sql: Some("select 1".into()),
            tool_calls: Vec::new(),
            artifacts: Vec::new(),
            usage: AgentUsage::default(),
            stop: AgentStop::EndTurn,
        });
        store.append_ai_message("chat-1", &reply).unwrap_or_else(|e| panic!("{e}"));

        let messages = store.list_ai_messages("chat-1").unwrap_or_default();
        assert_eq!(messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["u-1", "a-1"]);
        assert_eq!(messages[1].turn.as_ref().and_then(|t| t.sql.as_deref()), Some("select 1"));
        assert_eq!(store.find_ai_chat("chat-1").unwrap_or_default().map(|c| c.message_count), Some(2));
    }

    #[test]
    fn appending_the_same_message_twice_keeps_one_row() {
        let store = store_with_connection("c1");
        store.ensure_ai_chat("chat-1", "c1", "t").unwrap_or_else(|e| panic!("{e}"));
        store.append_ai_message("chat-1", &message("u-1", AiChatRole::User, "one")).unwrap_or_else(|e| panic!("{e}"));
        store.append_ai_message("chat-1", &message("u-1", AiChatRole::User, "one, edited")).unwrap_or_else(|e| panic!("{e}"));
        let messages = store.list_ai_messages("chat-1").unwrap_or_default();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "one, edited");
    }

    #[test]
    fn a_chat_id_cannot_reach_into_another_connection() {
        let store = store_with_connection("c1");
        store.ensure_ai_chat("chat-1", "c1", "t").unwrap_or_else(|e| panic!("{e}"));
        let err = store.ensure_ai_chat("chat-1", "c2", "t").err();
        assert!(matches!(err, Some(AppError::InvalidInput { .. })));
        // The same connection gets the same chat back, with its original title.
        let again = store.ensure_ai_chat("chat-1", "c1", "other title").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.title, "t");
    }

    #[test]
    fn list_is_per_connection_and_most_recent_first() {
        let store = store_with_connection("c1");
        store.ensure_ai_chat("old", "c1", "old").unwrap_or_else(|e| panic!("{e}"));
        store.ensure_ai_chat("other", "c2", "other").unwrap_or_else(|e| panic!("{e}"));
        store.ensure_ai_chat("new", "c1", "new").unwrap_or_else(|e| panic!("{e}"));
        // Activity in the older chat moves it to the top.
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.append_ai_message("old", &message("u-1", AiChatRole::User, "hi")).unwrap_or_else(|e| panic!("{e}"));
        let ids: Vec<String> = store.list_ai_chats("c1").unwrap_or_default().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["old".to_string(), "new".to_string()]);
    }

    #[test]
    fn rename_and_delete() {
        let store = store_with_connection("c1");
        store.ensure_ai_chat("chat-1", "c1", "before").unwrap_or_else(|e| panic!("{e}"));
        store.append_ai_message("chat-1", &message("u-1", AiChatRole::User, "hi")).unwrap_or_else(|e| panic!("{e}"));
        store.rename_ai_chat("chat-1", "after").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.find_ai_chat("chat-1").unwrap_or_default().map(|c| c.title), Some("after".into()));
        assert!(matches!(store.rename_ai_chat("missing", "x").err(), Some(AppError::NotFound { .. })));

        store.delete_ai_chat("chat-1").unwrap_or_else(|e| panic!("{e}"));
        assert!(store.find_ai_chat("chat-1").unwrap_or_default().is_none());
        assert!(store.list_ai_messages("chat-1").unwrap_or_default().is_empty(), "messages go with their chat");
        store.delete_ai_chat("chat-1").unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn deleting_a_connection_deletes_its_chats() {
        let store = store_with_connection("c1");
        store.ensure_ai_chat("chat-1", "c1", "t").unwrap_or_else(|e| panic!("{e}"));
        store.append_ai_message("chat-1", &message("u-1", AiChatRole::User, "hi")).unwrap_or_else(|e| panic!("{e}"));
        store.ensure_ai_chat("keep", "c2", "t").unwrap_or_else(|e| panic!("{e}"));

        store.delete_connection("c1").unwrap_or_else(|e| panic!("{e}"));
        assert!(store.list_ai_chats("c1").unwrap_or_default().is_empty());
        assert!(store.list_ai_messages("chat-1").unwrap_or_default().is_empty());
        assert_eq!(store.list_ai_chats("c2").unwrap_or_default().len(), 1);
    }
}
