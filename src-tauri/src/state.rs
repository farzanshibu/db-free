// SOT: app-state, sessions-registry, shared-handles, master-key-cache, agent-chat-registry

use crate::adapters::crypto::MasterKey;
use crate::adapters::keyring::{KeyLookup, KeyProvider};
use crate::integrations::Integration;
use crate::model::CredentialStatus;
use crate::services::agent::{AgentChat, RunControl};
use crate::services::schema_cache::SchemaCache;
use crate::error::{AppError, AppResult};
use crate::store::Store;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

// WHAT:  Everything a command can reach: the local store, live integration sessions,
//        and the master key provider.
// WHY:   One managed struct so the block (guard) resolves connections and
//        sessions from a single place.
// HOW:   Store ops are sub-millisecond and never held across an await.
//        Sessions are keyed by connection id; one integration per connection.
// WHERE: src-tauri/src/guard/mod.rs (the consumer), src-tauri/src/lib.rs (setup)
pub struct AppState {
    store: Arc<Mutex<Store>>,
    sessions: RwLock<HashMap<String, Arc<dyn Integration>>>,
    keys: Box<dyn KeyProvider>,
    master_key: OnceLock<MasterKey>,
    /// Conversation memory, one per open chat. Held here rather than shipped
    /// from the UI each message so the cached prompt prefix stays identical.
    agent_chats: RwLock<HashMap<String, Arc<AgentChat>>>,
    /// Which connection each in-memory chat is about (chat id -> connection id), so
    /// dropping a connection can drop its chats whatever their ids look like.
    chat_connections: Mutex<HashMap<String, String>>,
    /// Turns that are running right now, so the UI can answer a permission
    /// prompt or stop one. Keyed by run id; removed when the run ends.
    agent_runs: RwLock<HashMap<String, Arc<RunControl>>>,
    /// Editor runs that can be stopped right now (`cancel_query`).
    query_runs: QueryRuns,
    /// The catalog / columns / foreign keys per connection, persisted in the
    /// store and served first after a restart (services/schema_cache.rs).
    schema_cache: SchemaCache,
}

// WHAT:  Outcome of looking for the master key: a usable key, or the number of
//        sealed secrets that a missing key left unreadable.
enum Resolved {
    Key(MasterKey),
    Lost(u32),
}

fn credentials_lost_message(sealed: u32) -> String {
    format!(
        "Credentials lost: the OS keychain no longer holds DB Free's encryption key, so {sealed} saved password{} can't be decrypted. \
         Clear them from the Connections page, then enter each password again.",
        if sealed == 1 { "" } else { "s" }
    )
}

// WHAT:  The Stop button's registry: one cancellation token per in-flight run.
// WHY:   A query otherwise runs until the block's timeout. The UI names its run
//        (`ExecuteQueryRequest::run_id`) so a second command can reach it.
// HOW:   The block registers the id before the handler starts and finishes it
//        when the handler returns, win or lose; `cancel` trips the token the
//        block is racing. A cancel for an id that is not running (finished, or
//        never started) is a no-op rather than an error: Stop and completion
//        race by nature.
// WHERE: src-tauri/src/guard/mod.rs (step 9), src-tauri/src/commands/query.rs (cancel_query)
#[derive(Default)]
pub struct QueryRuns {
    runs: Mutex<HashMap<String, CancellationToken>>,
}

impl QueryRuns {
    /// Token the block races the handler against. Re-registering an id hands
    /// back a fresh token; the old run can no longer be stopped by that name.
    pub fn register(&self, run_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        if let Ok(mut runs) = self.runs.lock() {
            runs.insert(run_id.to_string(), token.clone());
        }
        token
    }

    /// True when a running run was told to stop.
    pub fn cancel(&self, run_id: &str) -> bool {
        let token = self.runs.lock().ok().and_then(|runs| runs.get(run_id).cloned());
        match token {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    pub fn finish(&self, run_id: &str) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.remove(run_id);
        }
    }

    pub fn is_running(&self, run_id: &str) -> bool {
        self.runs.lock().map(|runs| runs.contains_key(run_id)).unwrap_or(false)
    }
}

impl AppState {
    pub fn new(store: Store, keys: Box<dyn KeyProvider>) -> AppState {
        AppState {
            store: Arc::new(Mutex::new(store)),
            sessions: RwLock::new(HashMap::new()),
            keys,
            master_key: OnceLock::new(),
            agent_chats: RwLock::new(HashMap::new()),
            chat_connections: Mutex::new(HashMap::new()),
            agent_runs: RwLock::new(HashMap::new()),
            query_runs: QueryRuns::default(),
            schema_cache: SchemaCache::default(),
        }
    }

    pub fn query_runs(&self) -> &QueryRuns {
        &self.query_runs
    }

    pub fn schema_cache(&self) -> &SchemaCache {
        &self.schema_cache
    }

    // WHAT:  A shared handle on the store for work that outlives the command that
    //        started it (the schema cache's background refresh).
    // WHY:   `with_store` borrows `&self`, which a spawned task cannot hold.
    // HOW:   Same rule as `with_store`: lock, do a sub-millisecond operation,
    //        release; never across an await.
    pub fn store_handle(&self) -> Arc<Mutex<Store>> {
        Arc::clone(&self.store)
    }

    pub fn with_store<T>(&self, f: impl FnOnce(&Store) -> AppResult<T>) -> AppResult<T> {
        let guard = self
            .store
            .lock()
            .map_err(|_| AppError::internal("local store lock poisoned"))?;
        f(&guard)
    }

    pub fn master_key(&self) -> AppResult<&MasterKey> {
        if let Some(key) = self.master_key.get() {
            return Ok(key);
        }
        match self.resolve_master_key()? {
            Resolved::Key(key) => Ok(self.master_key.get_or_init(|| key)),
            Resolved::Lost(sealed) => Err(AppError::keyring(credentials_lost_message(sealed))),
        }
    }

    // WHAT:  Finds the master key, creating one only when that cannot orphan anything.
    // WHY:   A missing keychain entry used to mint a fresh key silently; every
    //        password sealed under the old one then failed to decrypt, with no
    //        hint why. Here a missing key with sealed secrets present is reported
    //        as lost, and an unreadable keychain (locked, prompt denied) is an
    //        error to retry, never grounds for a new key.
    // WHERE: src-tauri/src/adapters/keyring.rs (KeyLookup), src-tauri/src/store/connections.rs (sealed_secret_count)
    fn resolve_master_key(&self) -> AppResult<Resolved> {
        let sealed = || self.with_store(|store| store.sealed_secret_count());
        match self.keys.lookup()? {
            KeyLookup::Found(key) => Ok(Resolved::Key(key)),
            KeyLookup::Missing => match sealed()? {
                0 => Ok(Resolved::Key(self.keys.create()?)),
                lost => Ok(Resolved::Lost(lost)),
            },
            KeyLookup::Unavailable(reason) => match sealed()? {
                0 => Ok(Resolved::Key(self.keys.create()?)),
                _ => Err(AppError::keyring(format!(
                    "The OS keychain could not be read ({reason}). Saved passwords stay locked until it can; unlock it and try again."
                ))),
            },
        }
    }

    // WHAT:  Where the master key lives and whether saved passwords can still be opened.
    // WHY:   The connections page shows a standing warning while the key sits in
    //        a file, and an actionable banner when it is gone.
    pub fn credential_status(&self) -> AppResult<CredentialStatus> {
        let (lost, lost_secrets) = match self.master_key.get() {
            Some(_) => (false, 0),
            None => match self.resolve_master_key()? {
                Resolved::Key(key) => {
                    let _ = self.master_key.set(key);
                    (false, 0)
                }
                Resolved::Lost(sealed) => (true, sealed),
            },
        };
        Ok(CredentialStatus { storage: self.keys.storage(), key_path: self.keys.key_path().map(|p| p.to_string_lossy().into_owned()), lost, lost_secrets })
    }

    // WHAT:  Gives up on sealed secrets whose key is gone: clears them and starts a new key.
    // WHY:   They can never be opened again; leaving them makes every save and
    //        connect fail. Refused while the key is intact, so this cannot be
    //        used to wipe working credentials.
    pub fn reset_credentials(&self) -> AppResult<CredentialStatus> {
        if !self.credential_status()?.lost {
            return Err(AppError::invalid_input("Saved passwords are intact; there is nothing to reset."));
        }
        self.with_store(|store| store.clear_sealed_secrets())?;
        let key = self.keys.create()?;
        let _ = self.master_key.set(key);
        self.credential_status()
    }

    pub async fn session(&self, connection_id: &str) -> Option<Arc<dyn Integration>> {
        self.sessions.read().await.get(connection_id).cloned()
    }

    pub async fn session_ids(&self) -> Vec<String> {
        self.sessions.read().await.keys().cloned().collect()
    }

    pub async fn insert_session(&self, connection_id: String, integration: Arc<dyn Integration>) {
        let previous = self.sessions.write().await.insert(connection_id, integration);
        if let Some(old) = previous {
            old.close().await;
        }
    }

    pub async fn remove_session(&self, connection_id: &str) -> Option<Arc<dyn Integration>> {
        let removed = self.sessions.write().await.remove(connection_id);
        if let Some(integration) = &removed {
            integration.close().await;
        }
        // A chat is about a database. Dropping the connection drops the memory
        // of it too, rather than leaving a transcript that describes a schema
        // the next connection may not have. The chats are found by the owner they
        // were opened with (`agent_chat_for`), not by what their ids look like.
        let owned: Vec<String> = match self.chat_connections.lock() {
            Ok(mut owners) => {
                let ids: Vec<String> = owners.iter().filter(|(_, owner)| owner.as_str() == connection_id).map(|(id, _)| id.clone()).collect();
                owners.retain(|_, owner| owner.as_str() != connection_id);
                ids
            }
            Err(_) => Vec::new(),
        };
        if !owned.is_empty() {
            self.agent_chats.write().await.retain(|id, _| !owned.contains(id));
        }
        removed
    }

    /// The chat with this id, created on first use, remembered as being about
    /// `connection_id` so `remove_session` can drop it with the connection.
    pub async fn agent_chat_for(&self, chat_id: &str, connection_id: &str) -> Arc<AgentChat> {
        if let Ok(mut owners) = self.chat_connections.lock() {
            owners.insert(chat_id.to_string(), connection_id.to_string());
        }
        self.agent_chat(chat_id).await
    }

    /// The chat with this id, created on first use.
    pub async fn agent_chat(&self, chat_id: &str) -> Arc<AgentChat> {
        if let Some(chat) = self.agent_chats.read().await.get(chat_id) {
            return Arc::clone(chat);
        }
        let mut chats = self.agent_chats.write().await;
        Arc::clone(chats.entry(chat_id.to_string()).or_default())
    }

    pub async fn register_run(&self, run_id: String, control: Arc<RunControl>) {
        self.agent_runs.write().await.insert(run_id, control);
    }

    pub async fn finish_run(&self, run_id: &str) {
        self.agent_runs.write().await.remove(run_id);
    }

    pub async fn run_control(&self, run_id: &str) -> Option<Arc<RunControl>> {
        self.agent_runs.read().await.get(run_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_trips_the_registered_token_only() {
        let runs = QueryRuns::default();
        let a = runs.register("a");
        let b = runs.register("b");
        assert!(runs.cancel("a"));
        assert!(a.is_cancelled());
        assert!(!b.is_cancelled(), "Stop on one tab must not stop another");
    }

    #[test]
    fn cancel_after_finish_is_a_no_op() {
        let runs = QueryRuns::default();
        let token = runs.register("a");
        assert!(runs.is_running("a"));
        runs.finish("a");
        assert!(!runs.is_running("a"));
        assert!(!runs.cancel("a"), "a finished run has nothing to stop");
        assert!(!token.is_cancelled());
        assert!(!runs.cancel("never-started"));
    }

    // The ids of saved chats are UUIDs; what ties a chat to its connection is the
    // owner it was opened with, so closing one connection drops only its chats.
    #[tokio::test]
    async fn dropping_a_connection_drops_the_chats_opened_on_it() {
        let state = AppState::new(
            Store::open_in_memory().unwrap_or_else(|e| panic!("{e}")),
            Box::new(crate::adapters::keyring::MemoryKeyProvider::default()),
        );
        let on_a = state.agent_chat_for("6f1c2d3e-0000-4000-8000-000000000001", "conn-a").await;
        let also_a = state.agent_chat_for("6f1c2d3e-0000-4000-8000-000000000002", "conn-a").await;
        let on_b = state.agent_chat_for("6f1c2d3e-0000-4000-8000-000000000003", "conn-b").await;

        assert!(state.remove_session("conn-a").await.is_none(), "no live session was registered");
        let again = |id: &str| {
            let state = &state;
            let id = id.to_string();
            async move { state.agent_chat(&id).await }
        };
        assert!(!Arc::ptr_eq(&on_a, &again("6f1c2d3e-0000-4000-8000-000000000001").await), "conn-a's chat was forgotten");
        assert!(!Arc::ptr_eq(&also_a, &again("6f1c2d3e-0000-4000-8000-000000000002").await), "and so was its other one");
        assert!(Arc::ptr_eq(&on_b, &again("6f1c2d3e-0000-4000-8000-000000000003").await), "another connection's chat is untouched");
    }

    fn state_with_sealed_secret_and_no_key() -> AppState {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        store.set_setting("ai_key", "sealed-under-a-key-that-is-gone").unwrap_or_else(|e| panic!("{e}"));
        AppState::new(store, Box::new(crate::adapters::keyring::MemoryKeyProvider::default()))
    }

    // WHAT:  B8 — a missing key must not be replaced while it would orphan sealed secrets.
    #[test]
    fn a_missing_key_with_sealed_secrets_is_lost_not_regenerated() {
        let state = state_with_sealed_secret_and_no_key();
        let err = state.master_key().err().map(|e| e.to_string()).unwrap_or_default();
        assert!(err.starts_with("Credentials lost"), "{err}");
        let status = state.credential_status().unwrap_or_else(|e| panic!("{e}"));
        assert!(status.lost);
        assert_eq!(status.lost_secrets, 1);
        // Asking again does not change the answer: nothing minted a key in between.
        assert!(state.master_key().is_err());
    }

    #[test]
    fn resetting_lost_credentials_clears_them_and_starts_a_new_key() {
        let state = state_with_sealed_secret_and_no_key();
        let after = state.reset_credentials().unwrap_or_else(|e| panic!("{e}"));
        assert!(!after.lost);
        assert!(state.master_key().is_ok());
        assert_eq!(state.with_store(|s| s.sealed_secret_count()).unwrap_or(99), 0);
        // With the key intact there is nothing to reset: working credentials are never wiped.
        assert!(state.reset_credentials().is_err());
    }

    #[test]
    fn a_fresh_install_creates_a_key_on_first_use() {
        let state = AppState::new(
            Store::open_in_memory().unwrap_or_else(|e| panic!("{e}")),
            Box::new(crate::adapters::keyring::MemoryKeyProvider::default()),
        );
        assert!(state.master_key().is_ok());
        let status = state.credential_status().unwrap_or_else(|e| panic!("{e}"));
        assert!(!status.lost);
    }
}
