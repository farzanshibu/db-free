// SOT: ai-chats-service, chat-history-service, chat-title, chat-turn-persistence

use crate::error::{AppError, AppResult};
use crate::model::{AgentTurn, AiChatDetail, AiChatMessage, AiChatRole, AiChatSummary, ChatMention};
use crate::services::agent::HistoryTurn;
use crate::state::AppState;
use crate::store::now_rfc3339;

// WHAT:  Saved assistant chats: list, load, rename, delete, and the two writes
//        that bracket every turn.
// WHY:   A chat has to survive closing the tab and restarting the app, and a
//        resumed chat has to remember what was said. The UI mints a chat id and
//        the first message creates the row, so "New chat" is free.
// HOW:   `begin_turn` saves the user's message before the model runs (so a crash
//        mid-run still leaves the question) and returns the earlier text turns,
//        which the loop replays when the in-memory transcript is empty.
//        `finish_turn` saves the answer — or the failure.
// WHERE: src-tauri/src/commands/ai.rs (chat management), commands/agent.rs
//        (the two turn writes), src-tauri/src/store/ai_chats.rs

const TITLE_MAX_CHARS: usize = 50;
const RENAME_MAX_CHARS: usize = 120;
/// Earlier messages replayed to a resumed chat, newest kept.
const HISTORY_MESSAGES: usize = 24;
/// Characters of earlier conversation replayed, newest kept: the model has a
/// context window and a long answer from last week is not worth displacing today's.
const HISTORY_CHARS: usize = 24_000;

const DEFAULT_TITLE: &str = "New chat";

// WHAT:  The prompt without the "(document id …)" note other screens add when they
//        hand a dashboard, workflow or diagram to the chat.
// WHY:   The id tells the model which document to update; in a chat's title it is
//        noise ("Build the dashboard "Sales" (document id 3f2a9c…").
fn without_document_ids(prompt: &str) -> String {
    let mut out = String::with_capacity(prompt.len());
    let mut rest = prompt;
    while let Some(start) = rest.find("(document id ") {
        let Some(end) = rest[start..].find(')') else { break };
        out.push_str(&rest[..start]);
        rest = &rest[start + end + 1..];
        // "…"Sales" (document id x). Add…" must not become "…"Sales" . Add…".
        while out.ends_with(' ') && rest.starts_with(['.', ',', ';', ':', '!', '?']) {
            out.pop();
        }
    }
    out.push_str(rest);
    out
}

/// The first words of the first question, on one line.
pub fn title_from_prompt(prompt: &str) -> String {
    let flat = without_document_ids(prompt).split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return DEFAULT_TITLE.to_string();
    }
    if flat.chars().count() <= TITLE_MAX_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(TITLE_MAX_CHARS).collect();
    format!("{}…", head.trim_end())
}

pub fn list(state: &AppState, connection_id: &str) -> AppResult<Vec<AiChatSummary>> {
    state.with_store(|store| store.list_ai_chats(connection_id))
}

pub fn load(state: &AppState, chat_id: &str) -> AppResult<AiChatDetail> {
    state.with_store(|store| {
        let chat = store.find_ai_chat(chat_id)?.ok_or_else(|| AppError::not_found("That chat no longer exists."))?;
        let messages = store.list_ai_messages(chat_id)?;
        Ok(AiChatDetail { chat, messages })
    })
}

pub fn rename(state: &AppState, chat_id: &str, title: &str) -> AppResult<AiChatSummary> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err(AppError::invalid_input("A chat needs a title."));
    }
    let title: String = title.chars().take(RENAME_MAX_CHARS).collect();
    state.with_store(|store| {
        store.rename_ai_chat(chat_id, &title)?;
        store.find_ai_chat(chat_id)?.ok_or_else(|| AppError::not_found("That chat no longer exists."))
    })
}

pub fn delete(state: &AppState, chat_id: &str) -> AppResult<()> {
    state.with_store(|store| store.delete_ai_chat(chat_id))
}

// WHAT:  Record the question and return what the chat already said.
pub fn begin_turn(
    state: &AppState,
    chat_id: &str,
    connection_id: &str,
    run_id: &str,
    prompt: &str,
    mentions: &[ChatMention],
) -> AppResult<Vec<HistoryTurn>> {
    if chat_id.trim().is_empty() {
        return Err(AppError::invalid_input("A chat needs an id."));
    }
    state.with_store(|store| {
        store.ensure_ai_chat(chat_id, connection_id, &title_from_prompt(prompt))?;
        let earlier = store.list_ai_messages(chat_id)?;
        store.append_ai_message(
            chat_id,
            &AiChatMessage {
                id: format!("u-{run_id}"),
                role: AiChatRole::User,
                text: prompt.trim().to_string(),
                created_at: now_rfc3339(),
                mentions: mentions.to_vec(),
                turn: None,
                error: None,
            },
        )?;
        Ok(history_for_model(&earlier))
    })
}

// WHAT:  Pick a chat back up where a run stopped: what it already said, with no
//        new question recorded.
// WHY:   Continue after a step-limit stop asks the model to keep going, not the
//        user to repeat themselves, so the thread must not gain a message.
pub fn resume_turn(state: &AppState, chat_id: &str) -> AppResult<Vec<HistoryTurn>> {
    state.with_store(|store| {
        let earlier = store.list_ai_messages(chat_id)?;
        if earlier.is_empty() {
            return Err(AppError::not_found("There is no conversation to continue."));
        }
        Ok(history_for_model(&earlier))
    })
}

// WHAT:  Record the answer, or why there was none.
pub fn finish_turn(state: &AppState, chat_id: &str, run_id: &str, outcome: Result<&AgentTurn, &AppError>) -> AppResult<()> {
    let message = match outcome {
        Ok(turn) => AiChatMessage {
            id: format!("a-{run_id}"),
            role: AiChatRole::Assistant,
            text: turn.text.clone(),
            created_at: now_rfc3339(),
            mentions: Vec::new(),
            turn: Some(turn.clone()),
            error: None,
        },
        Err(err) => AiChatMessage {
            id: format!("a-{run_id}"),
            role: AiChatRole::Assistant,
            text: String::new(),
            created_at: now_rfc3339(),
            mentions: Vec::new(),
            turn: None,
            error: Some(err.message().to_string()),
        },
    };
    state.with_store(|store| store.append_ai_message(chat_id, &message))
}

// WHAT:  The text turns of a saved chat, trimmed to what is worth replaying.
// WHY:   Tool calls and their raw results are not replayed: the assistant's own
//        prose already carries what they found, and a replayed tool call without
//        its provider ids would be rejected anyway.
pub fn history_for_model(messages: &[AiChatMessage]) -> Vec<HistoryTurn> {
    let mut kept: Vec<HistoryTurn> = Vec::new();
    let mut chars = 0usize;
    for message in messages.iter().rev().take(HISTORY_MESSAGES) {
        let text = message.text.trim();
        if text.is_empty() {
            continue;
        }
        let size = text.chars().count();
        // The newest question and its answer are always kept, however long: a
        // replay that opened on a bare answer would be dropped whole below.
        if chars + size > HISTORY_CHARS && kept.iter().any(|turn| !turn.assistant) {
            break;
        }
        chars += size;
        kept.push(HistoryTurn { assistant: message.role == AiChatRole::Assistant, text: text.to_string() });
    }
    kept.reverse();
    // A transcript opens on a user message; an answer whose question fell off the
    // front would make some providers reject the whole request.
    while kept.first().is_some_and(|turn| turn.assistant) {
        kept.remove(0);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::{AgentStop, AgentUsage};
    use crate::store::Store;

    fn state() -> AppState {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        store
            .conn()
            .execute(
                "INSERT INTO connections (id, name, engine, environment, created_at, updated_at) VALUES ('c1', 'c', 'sqlite', 'local', 'now', 'now')",
                [],
            )
            .unwrap_or_else(|e| panic!("{e}"));
        AppState::new(store, Box::new(MemoryKeyProvider::default()))
    }

    fn turn(text: &str) -> AgentTurn {
        AgentTurn {
            run_id: "r".into(),
            model: "m".into(),
            text: text.into(),
            sql: None,
            tool_calls: Vec::new(),
            artifacts: Vec::new(),
            usage: AgentUsage::default(),
            stop: AgentStop::EndTurn,
        }
    }

    #[test]
    fn titles_are_the_first_fifty_characters_on_one_line() {
        assert_eq!(title_from_prompt("  top 10\n customers  by revenue "), "top 10 customers by revenue");
        assert_eq!(title_from_prompt(""), "New chat");
        let long = "a".repeat(80);
        let title = title_from_prompt(&long);
        assert_eq!(title.chars().count(), 51, "50 characters and an ellipsis");
        assert!(title.ends_with('…'));
        // Multi-byte text is cut on a character boundary, never inside one.
        assert_eq!(title_from_prompt(&"é".repeat(60)).chars().count(), 51);
    }

    #[test]
    fn the_document_id_a_screen_adds_is_not_part_of_the_title() {
        assert_eq!(
            title_from_prompt("Build the dashboard \"Sales\" (document id 3f2a9c10-aa). Add a revenue chart."),
            "Build the dashboard \"Sales\". Add a revenue chart."
        );
        assert_eq!(title_from_prompt("what is (document id broken"), "what is (document id broken", "an unclosed note is left alone");
        assert_eq!(title_from_prompt("(document id x)"), "New chat", "nothing left means the default title");
    }

    #[test]
    fn continuing_a_stopped_run_replays_the_chat_without_saving_a_new_question() {
        let state = state();
        begin_turn(&state, "chat-1", "c1", "r1", "audit every table", &[]).unwrap_or_else(|e| panic!("{e}"));
        let mut stopped = turn("checked 12 of 40 tables");
        stopped.stop = AgentStop::MaxSteps;
        finish_turn(&state, "chat-1", "r1", Ok(&stopped)).unwrap_or_else(|e| panic!("{e}"));

        let earlier = resume_turn(&state, "chat-1").unwrap_or_else(|e| panic!("{e}"));
        let texts: Vec<(&bool, &str)> = earlier.iter().map(|t| (&t.assistant, t.text.as_str())).collect();
        assert_eq!(texts, vec![(&false, "audit every table"), (&true, "checked 12 of 40 tables")]);
        assert_eq!(load(&state, "chat-1").unwrap_or_else(|e| panic!("{e}")).messages.len(), 2, "no message was added");
        assert!(resume_turn(&state, "nothing-here").is_err(), "there is nothing to continue in a chat that does not exist");
    }

    #[test]
    fn a_turn_is_saved_around_the_run_and_the_next_one_sees_it() {
        let state = state();
        let first = begin_turn(&state, "chat-1", "c1", "r1", "which table is biggest?", &[]).unwrap_or_else(|e| panic!("{e}"));
        assert!(first.is_empty(), "a new chat has no history");
        finish_turn(&state, "chat-1", "r1", Ok(&turn("orders, ~2M rows"))).unwrap_or_else(|e| panic!("{e}"));

        let second = begin_turn(&state, "chat-1", "c1", "r2", "and the smallest?", &[]).unwrap_or_else(|e| panic!("{e}"));
        let texts: Vec<(&bool, &str)> = second.iter().map(|t| (&t.assistant, t.text.as_str())).collect();
        assert_eq!(texts, vec![(&false, "which table is biggest?"), (&true, "orders, ~2M rows")]);

        let detail = load(&state, "chat-1").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(detail.chat.title, "which table is biggest?", "titled from the first question only");
        assert_eq!(detail.messages.len(), 3, "the unanswered second question is already saved");
    }

    #[test]
    fn a_failed_run_is_saved_with_its_error_and_not_replayed() {
        let state = state();
        begin_turn(&state, "chat-1", "c1", "r1", "hello", &[]).unwrap_or_else(|e| panic!("{e}"));
        finish_turn(&state, "chat-1", "r1", Err(&AppError::timeout("The provider timed out."))).unwrap_or_else(|e| panic!("{e}"));
        let detail = load(&state, "chat-1").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(detail.messages[1].error.as_deref(), Some("The provider timed out."));
        let next = begin_turn(&state, "chat-1", "c1", "r2", "try again", &[]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(next.len(), 1, "only the question; the failure carries no text to replay");
    }

    #[test]
    fn rename_validates_and_delete_is_idempotent() {
        let state = state();
        begin_turn(&state, "chat-1", "c1", "r1", "hello", &[]).unwrap_or_else(|e| panic!("{e}"));
        assert!(rename(&state, "chat-1", "   ").is_err());
        let renamed = rename(&state, "chat-1", "  Revenue   questions ").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(renamed.title, "Revenue questions");
        assert_eq!(list(&state, "c1").unwrap_or_default().len(), 1);
        delete(&state, "chat-1").unwrap_or_else(|e| panic!("{e}"));
        delete(&state, "chat-1").unwrap_or_else(|e| panic!("{e}"));
        assert!(load(&state, "chat-1").is_err());
        assert!(list(&state, "c1").unwrap_or_default().is_empty());
    }

    #[test]
    fn history_is_bounded_and_opens_on_a_user_message() {
        let message = |role: AiChatRole, text: &str| AiChatMessage {
            id: text.into(),
            role,
            text: text.into(),
            created_at: String::new(),
            mentions: Vec::new(),
            turn: None,
            error: None,
        };
        let mut many = Vec::new();
        for i in 0..40 {
            many.push(message(AiChatRole::User, &format!("q{i}")));
            many.push(message(AiChatRole::Assistant, &format!("a{i}")));
        }
        let history = history_for_model(&many);
        assert!(history.len() <= HISTORY_MESSAGES);
        assert!(!history[0].assistant, "starts with a question");
        assert_eq!(history.last().map(|t| t.text.as_str()), Some("a39"));

        let huge = vec![message(AiChatRole::User, &"x".repeat(HISTORY_CHARS + 10)), message(AiChatRole::Assistant, "ok")];
        let kept = history_for_model(&huge);
        assert_eq!(kept.len(), 2, "one oversized message is still kept when it is all there is");
    }
}
