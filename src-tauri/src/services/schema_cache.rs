// SOT: schema-cache-service, persisted-schema-cache, in-flight-dedupe, stale-while-revalidate, schema-invalidation

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::model::{ColumnInfo, ForeignKey, SchemaCacheUpdated, SchemaCatalog, TableRef};
use crate::store::Store;
use futures::future::{BoxFuture, FutureExt, Shared};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

// ============================================================================
// SCHEMA CACHE
//
// WHAT:  The catalog, every table's columns and the foreign keys, kept in the
//        app store per (connection, database) and served from there on the first
//        request after launch, with the live answer fetched behind it.
// WHY:   `load_catalog` on a database with thousands of tables takes seconds and
//        every launch paid it before the sidebar drew. DB Pro keeps the same
//        cache; this one also reconciles: the UI is told when the live answer
//        differs from what it was shown.
// HOW:   Three layers, cheapest first:
//          1. `recent`   an answer fetched in the last few seconds is reused, so a
//                        refresh that follows another (the background revalidation,
//                        then the redraw it triggers) costs one query, not two;
//          2. the store  only for a key not yet answered live since launch. It is
//                        returned at once and a revalidation starts behind it;
//          3. live       the adapter, with concurrent callers for the same key
//                        sharing one in-flight call, a timeout, and the result
//                        written back to the store.
//        `force` skips 1 and 2. A script that changes the catalog (`guard`,
//        step 14) calls `invalidate`, which also fences off a fetch that started
//        before the change so it cannot write a stale answer back.
// WHERE: src-tauri/src/services/schema.rs (callers), src-tauri/src/state.rs (owner),
//        src-tauri/src/store/schema_cache.rs (persistence), src-tauri/src/lib.rs (event)
// ============================================================================

/// How long a live answer is reused without asking again.
const FRESH_FOR: Duration = Duration::from_secs(5);
/// Longest a catalog load may take before the waiting callers get an error.
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
const SEP: char = '\u{1f}';

type Payload = Arc<String>;
type Flight = Shared<BoxFuture<'static, Result<Payload, AppError>>>;
type Notifier = Box<dyn Fn(SchemaCacheUpdated) + Send + Sync>;

#[derive(Clone)]
struct Ids {
    key: String,
    connection_id: String,
    namespace: String,
    scope: String,
}

impl Ids {
    fn new(connection_id: &str, namespace: &str, scope: &str) -> Ids {
        Ids {
            key: format!("{connection_id}{SEP}{namespace}{SEP}{scope}"),
            connection_id: connection_id.to_string(),
            namespace: namespace.to_string(),
            scope: scope.to_string(),
        }
    }
}

fn connection_prefix(connection_id: &str) -> String {
    format!("{connection_id}{SEP}")
}

#[derive(Default)]
struct Inner {
    /// Calls in flight, so concurrent requests for one key share a single query.
    flights: Mutex<HashMap<String, (u64, Flight)>>,
    next_flight: AtomicU64,
    /// Live answers from the last few seconds.
    recent: Mutex<HashMap<String, (Instant, Payload)>>,
    /// Keys answered live at least once since launch: only the others may be
    /// answered from the store.
    live: Mutex<HashSet<String>>,
    /// Bumped by `invalidate`; a fetch that began under an older value is not stored.
    epochs: Mutex<HashMap<String, u64>>,
    notifier: OnceLock<Notifier>,
}

impl Inner {
    fn epoch(&self, connection_id: &str) -> u64 {
        self.epochs.lock().ok().and_then(|epochs| epochs.get(connection_id).copied()).unwrap_or(0)
    }

    fn remember(&self, key: &str, payload: &Payload) {
        if let Ok(mut recent) = self.recent.lock() {
            recent.insert(key.to_string(), (Instant::now(), Arc::clone(payload)));
        }
        if let Ok(mut live) = self.live.lock() {
            live.insert(key.to_string());
        }
    }

    fn finish(&self, key: &str, id: u64) {
        if let Ok(mut flights) = self.flights.lock() {
            if flights.get(key).is_some_and(|(current, _)| *current == id) {
                flights.remove(key);
            }
        }
    }

    fn notify(&self, connection_id: &str) {
        if let Some(notifier) = self.notifier.get() {
            notifier(SchemaCacheUpdated { connection_id: connection_id.to_string() });
        }
    }
}

// WHAT:  The cache's in-memory half; the app store holds the persistent one.
#[derive(Clone, Default)]
pub struct SchemaCache {
    inner: Arc<Inner>,
}

impl SchemaCache {
    /// Called once at startup with whatever delivers `schema-cache:updated` to the UI.
    pub fn set_notifier(&self, notifier: impl Fn(SchemaCacheUpdated) + Send + Sync + 'static) {
        let _ = self.inner.notifier.set(Box::new(notifier));
    }

    pub async fn catalog(&self, store: &Arc<Mutex<Store>>, ctx: &SessionCtx, force: bool) -> AppResult<SchemaCatalog> {
        let integration = Arc::clone(&ctx.integration);
        self.load(store, ctx, "catalog", force, move || async move { integration.catalog().await }).await
    }

    pub async fn columns(&self, store: &Arc<Mutex<Store>>, ctx: &SessionCtx, table: &TableRef, force: bool) -> AppResult<Vec<ColumnInfo>> {
        let integration = Arc::clone(&ctx.integration);
        let wanted = table.clone();
        let scope = format!("columns:{}.{}", table.schema.as_deref().unwrap_or_default(), table.name);
        self.load(store, ctx, &scope, force, move || async move { integration.columns(&wanted).await }).await
    }

    pub async fn foreign_keys(&self, store: &Arc<Mutex<Store>>, ctx: &SessionCtx, force: bool) -> AppResult<Vec<ForeignKey>> {
        let integration = Arc::clone(&ctx.integration);
        self.load(store, ctx, "foreign_keys", force, move || async move { integration.foreign_keys().await }).await
    }

    // WHAT:  Forgets everything cached for a connection: memory, in-flight calls and the store.
    // WHY:   The catalog just changed (DDL ran), so none of it can be trusted.
    pub fn invalidate(&self, store: &Arc<Mutex<Store>>, connection_id: &str) {
        let prefix = connection_prefix(connection_id);
        if let Ok(mut epochs) = self.inner.epochs.lock() {
            *epochs.entry(connection_id.to_string()).or_insert(0) += 1;
        }
        if let Ok(mut flights) = self.inner.flights.lock() {
            flights.retain(|key, _| !key.starts_with(&prefix));
        }
        if let Ok(mut recent) = self.inner.recent.lock() {
            recent.retain(|key, _| !key.starts_with(&prefix));
        }
        match store.lock() {
            Ok(store) => {
                if let Err(err) = store.clear_schema_cache(connection_id) {
                    log::warn!("schema cache clear failed: {err}");
                }
            }
            Err(_) => log::warn!("local store lock poisoned"),
        }
    }

    async fn load<T, F, Fut>(&self, store: &Arc<Mutex<Store>>, ctx: &SessionCtx, scope: &str, force: bool, fetch: F) -> AppResult<T>
    where
        T: Serialize + DeserializeOwned + Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = AppResult<T>> + Send + 'static,
    {
        let namespace = ctx.integration.current_database().unwrap_or_default();
        let ids = Ids::new(&ctx.connection.id, &namespace, scope);
        self.load_by_ids(store, ids, force, fetch).await
    }

    async fn load_by_ids<T, F, Fut>(&self, store: &Arc<Mutex<Store>>, ids: Ids, force: bool, fetch: F) -> AppResult<T>
    where
        T: Serialize + DeserializeOwned + Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = AppResult<T>> + Send + 'static,
    {
        if !force {
            if let Some(payload) = self.recent(&ids.key) {
                if let Ok(value) = serde_json::from_str::<T>(&payload) {
                    return Ok(value);
                }
            }
            // First ask since launch: show what the last launch saw, then reconcile.
            if !self.is_live(&ids.key) {
                if let Some(stored) = read_stored(store, &ids) {
                    if let Ok(value) = serde_json::from_str::<T>(&stored) {
                        let flight = self.start(Arc::clone(store), ids.clone(), fetch);
                        self.revalidate(flight, ids.connection_id, Arc::new(stored));
                        return Ok(value);
                    }
                }
            }
        }
        let payload = self.start(Arc::clone(store), ids, fetch).await?;
        serde_json::from_str::<T>(&payload).map_err(AppError::internal)
    }

    fn recent(&self, key: &str) -> Option<Payload> {
        let recent = self.inner.recent.lock().ok()?;
        let (at, payload) = recent.get(key)?;
        (at.elapsed() < FRESH_FOR).then(|| Arc::clone(payload))
    }

    fn is_live(&self, key: &str) -> bool {
        self.inner.live.lock().map(|live| live.contains(key)).unwrap_or(true)
    }

    // WHAT:  The one live call for `ids`, shared by everyone who asks while it runs.
    fn start<T, F, Fut>(&self, store: Arc<Mutex<Store>>, ids: Ids, fetch: F) -> Flight
    where
        T: Serialize + Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = AppResult<T>> + Send + 'static,
    {
        let Ok(mut flights) = self.inner.flights.lock() else {
            return async { Err(AppError::internal("the schema cache is poisoned")) }.boxed().shared();
        };
        if let Some((_, running)) = flights.get(&ids.key) {
            return running.clone();
        }
        let inner = Arc::clone(&self.inner);
        let id = inner.next_flight.fetch_add(1, Ordering::Relaxed);
        let epoch = inner.epoch(&ids.connection_id);
        let key = ids.key.clone();
        let flight: Flight = async move {
            let fetched = tokio::time::timeout(FETCH_TIMEOUT, async {
                let value = fetch().await?;
                serde_json::to_string(&value).map(Arc::new).map_err(AppError::internal)
            })
            .await
            .unwrap_or_else(|_| Err(AppError::timeout("Loading the schema timed out.")));
            if let Ok(payload) = &fetched {
                // A script that changed the catalog while this ran makes the answer
                // possibly stale: the callers waiting on it get it, the cache does not.
                if inner.epoch(&ids.connection_id) == epoch {
                    persist(&store, &ids, payload);
                    inner.remember(&ids.key, payload);
                }
            }
            inner.finish(&ids.key, id);
            fetched
        }
        .boxed()
        .shared();
        flights.insert(key, (id, flight.clone()));
        flight
    }

    // WHAT:  Lets the live fetch run behind an answer served from the store, and
    //        tells the UI if it came back different.
    fn revalidate(&self, flight: Flight, connection_id: String, served: Payload) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            match flight.await {
                Ok(fresh) if *fresh != *served => inner.notify(&connection_id),
                Ok(_) => {}
                Err(err) => log::debug!("schema cache refresh failed: {err}"),
            }
        });
    }
}

fn read_stored(store: &Arc<Mutex<Store>>, ids: &Ids) -> Option<String> {
    let store = store.lock().ok()?;
    match store.get_schema_cache(&ids.connection_id, &ids.namespace, &ids.scope) {
        Ok(entry) => entry.map(|e| e.payload),
        Err(err) => {
            log::warn!("schema cache read failed: {err}");
            None
        }
    }
}

fn persist(store: &Arc<Mutex<Store>>, ids: &Ids, payload: &str) {
    let now = chrono::Utc::now().timestamp_millis();
    match store.lock() {
        Ok(store) => {
            if let Err(err) = store.put_schema_cache(&ids.connection_id, &ids.namespace, &ids.scope, payload, now) {
                log::warn!("schema cache write failed: {err}");
            }
        }
        Err(_) => log::warn!("local store lock poisoned"),
    }
}

// WHAT:  True when running `sql` can change what the catalog lists.
// WHY:   The cache is only as good as its invalidation: a table created in the
//        editor has to show up in the sidebar, not after the next launch.
// HOW:   By leading keyword, comments skipped: CREATE, ALTER, DROP, RENAME,
//        COMMENT, ATTACH, DETACH. Over-matching only costs a reload.
pub fn changes_catalog(sql: &str) -> bool {
    crate::guard::destructive::split_statements(sql)
        .iter()
        .filter_map(|statement| leading_keyword(statement))
        .any(|word| matches!(word.as_str(), "CREATE" | "ALTER" | "DROP" | "RENAME" | "COMMENT" | "ATTACH" | "DETACH"))
}

fn leading_keyword(statement: &str) -> Option<String> {
    let chars: Vec<char> = statement.chars().collect();
    let mut i = 0_usize;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && next == Some('-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1).copied() == Some('/')) {
                i += 1;
            }
            i += 2;
        } else {
            break;
        }
    }
    let word: String = chars.iter().skip(i).take_while(|c| c.is_alphabetic()).collect();
    (!word.is_empty()).then(|| word.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    // A bare connection row: the cache only needs the foreign key to resolve.
    fn store() -> Arc<Mutex<Store>> {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        store
            .conn()
            .execute_batch("INSERT INTO connections (id, name, engine, environment, created_at, updated_at) VALUES ('c1', 'c', 'sqlite', 'local', 'x', 'x')")
            .unwrap_or_else(|e| panic!("{e}"));
        Arc::new(Mutex::new(store))
    }

    fn ids(scope: &str) -> Ids {
        Ids::new("c1", "main", scope)
    }

    fn stored(store: &Arc<Mutex<Store>>, scope: &str) -> Option<String> {
        read_stored(store, &ids(scope))
    }

    /// A fetch that counts its calls and answers with the n-th number.
    fn counting(calls: &Arc<AtomicUsize>) -> impl FnOnce() -> BoxFuture<'static, AppResult<Vec<u32>>> + Send + 'static {
        let calls = Arc::clone(calls);
        move || {
            async move {
                let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                Ok(vec![u32::try_from(n).unwrap_or(0)])
            }
            .boxed()
        }
    }

    #[test]
    fn catalog_changes_are_recognised_by_their_leading_keyword() {
        assert!(changes_catalog("CREATE TABLE t (a int)"));
        assert!(changes_catalog("  -- add it\n  alter table t add column b int"));
        assert!(changes_catalog("/* x */ DROP VIEW v"));
        assert!(changes_catalog("SELECT 1; RENAME TABLE a TO b"));
        assert!(changes_catalog("COMMENT ON TABLE t IS 'x'"));
        assert!(!changes_catalog("SELECT 'CREATE TABLE x'"));
        assert!(!changes_catalog("INSERT INTO t VALUES (1)"));
        assert!(!changes_catalog("UPDATE t SET a = 1 WHERE b = 2"));
        assert!(!changes_catalog("BEGIN; DELETE FROM t WHERE a = 1; COMMIT"));
        assert!(!changes_catalog(""));
    }

    #[tokio::test]
    async fn a_live_answer_is_stored_and_reused_for_a_few_seconds() {
        let cache = SchemaCache::default();
        let store = store();
        let calls = Arc::new(AtomicUsize::new(0));
        let first: Vec<u32> = cache.load_by_ids(&store, ids("catalog"), false, counting(&calls)).await.unwrap_or_default();
        let second: Vec<u32> = cache.load_by_ids(&store, ids("catalog"), false, counting(&calls)).await.unwrap_or_default();
        assert_eq!((first, second), (vec![1], vec![1]), "the second ask reused the first answer");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(stored(&store, "catalog").as_deref(), Some("[1]"), "and it reached the store");
    }

    #[tokio::test]
    async fn force_goes_to_the_adapter_and_replaces_what_was_stored() {
        let cache = SchemaCache::default();
        let store = store();
        let calls = Arc::new(AtomicUsize::new(0));
        let _: Vec<u32> = cache.load_by_ids(&store, ids("catalog"), false, counting(&calls)).await.unwrap_or_default();
        let forced: Vec<u32> = cache.load_by_ids(&store, ids("catalog"), true, counting(&calls)).await.unwrap_or_default();
        assert_eq!(forced, vec![2]);
        assert_eq!(stored(&store, "catalog").as_deref(), Some("[2]"));
    }

    #[tokio::test]
    async fn concurrent_asks_share_one_call() {
        let cache = SchemaCache::default();
        let store = store();
        let calls = Arc::new(AtomicUsize::new(0));
        let slow = |calls: &Arc<AtomicUsize>| {
            let calls = Arc::clone(calls);
            move || {
                async move {
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                    Ok::<Vec<u32>, AppError>(vec![u32::try_from(n).unwrap_or(0)])
                }
                .boxed()
            }
        };
        let (a, b, c) = tokio::join!(
            cache.load_by_ids::<Vec<u32>, _, _>(&store, ids("catalog"), true, slow(&calls)),
            cache.load_by_ids::<Vec<u32>, _, _>(&store, ids("catalog"), true, slow(&calls)),
            cache.load_by_ids::<Vec<u32>, _, _>(&store, ids("catalog"), true, slow(&calls)),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1, "three callers, one query");
        assert_eq!((a.unwrap_or_default(), b.unwrap_or_default(), c.unwrap_or_default()), (vec![1], vec![1], vec![1]));
        assert!(cache.inner.flights.lock().map(|f| f.is_empty()).unwrap_or(false), "the finished flight is forgotten");
    }

    #[tokio::test]
    async fn a_failed_call_is_not_cached() {
        let cache = SchemaCache::default();
        let store = store();
        let failing = || async { Err::<Vec<u32>, AppError>(AppError::driver("boom")) }.boxed();
        assert!(cache.load_by_ids::<Vec<u32>, _, _>(&store, ids("catalog"), false, failing).await.is_err());
        assert_eq!(stored(&store, "catalog"), None);
        let calls = Arc::new(AtomicUsize::new(0));
        let retried: Vec<u32> = cache.load_by_ids(&store, ids("catalog"), false, counting(&calls)).await.unwrap_or_default();
        assert_eq!(retried, vec![1], "the next ask tries again");
    }

    #[tokio::test]
    async fn after_a_restart_the_stored_answer_comes_first_and_the_live_one_follows() {
        let store = store();
        let before = SchemaCache::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let _: Vec<u32> = before.load_by_ids(&store, ids("catalog"), false, counting(&calls)).await.unwrap_or_default();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // "Restart": a fresh in-memory cache over the same store.
        let after = SchemaCache::default();
        let told = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = Arc::clone(&told);
        after.set_notifier(move |event| {
            if let Ok(mut told) = sink.lock() {
                told.push(event.connection_id);
            }
        });
        let cold: Vec<u32> = after.load_by_ids(&store, ids("catalog"), false, counting(&calls)).await.unwrap_or_default();
        assert_eq!(cold, vec![1], "answered from the store without waiting for the adapter");

        for _ in 0..200 {
            if stored(&store, "catalog").as_deref() == Some("[2]") && told.lock().map(|t| !t.is_empty()).unwrap_or(false) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(stored(&store, "catalog").as_deref(), Some("[2]"), "the live answer replaced the stored one");
        assert_eq!(told.lock().map(|t| t.clone()).unwrap_or_default(), vec!["c1".to_string()], "and the UI was told it changed");

        let warm: Vec<u32> = after.load_by_ids(&store, ids("catalog"), false, counting(&calls)).await.unwrap_or_default();
        assert_eq!(warm, vec![2], "from then on the live answer is served");
    }

    #[tokio::test]
    async fn an_unchanged_catalog_does_not_ping_the_ui() {
        let store = store();
        let steady = |calls: &Arc<AtomicUsize>| {
            let calls = Arc::clone(calls);
            move || {
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<Vec<u32>, AppError>(vec![7])
                }
                .boxed()
            }
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let _: Vec<u32> = SchemaCache::default().load_by_ids(&store, ids("catalog"), false, steady(&calls)).await.unwrap_or_default();
        let after = SchemaCache::default();
        let told = Arc::new(AtomicUsize::new(0));
        let sink = Arc::clone(&told);
        after.set_notifier(move |_| {
            sink.fetch_add(1, Ordering::SeqCst);
        });
        let _: Vec<u32> = after.load_by_ids(&store, ids("catalog"), false, steady(&calls)).await.unwrap_or_default();
        for _ in 0..200 {
            if calls.load(Ordering::SeqCst) == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2, "the revalidation ran");
        assert_eq!(told.load(Ordering::SeqCst), 0, "and found nothing new");
    }

    #[tokio::test]
    async fn invalidate_forgets_everything_and_fences_off_a_fetch_in_flight() {
        let cache = SchemaCache::default();
        let store = store();
        let calls = Arc::new(AtomicUsize::new(0));
        let _: Vec<u32> = cache.load_by_ids(&store, ids("catalog"), false, counting(&calls)).await.unwrap_or_default();
        cache.invalidate(&store, "c1");
        assert_eq!(stored(&store, "catalog"), None, "the store was cleared");
        let again: Vec<u32> = cache.load_by_ids(&store, ids("catalog"), false, counting(&calls)).await.unwrap_or_default();
        assert_eq!(again, vec![2], "the next ask is live");

        // A fetch that started before the DDL must not write its (stale) answer back.
        let slow = {
            let calls = Arc::clone(&calls);
            move || {
                async move {
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<Vec<u32>, AppError>(vec![99])
                }
                .boxed()
            }
        };
        let running = cache.load_by_ids::<Vec<u32>, _, _>(&store, ids("columns:main.t"), true, slow);
        let ddl = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cache.invalidate(&store, "c1");
        };
        let (answer, ()) = tokio::join!(running, ddl);
        assert_eq!(answer.unwrap_or_default(), vec![99], "the caller still gets its answer");
        assert_eq!(stored(&store, "columns:main.t"), None, "but it was not cached");
    }

    #[tokio::test]
    async fn databases_and_connections_do_not_share_entries() {
        let cache = SchemaCache::default();
        let store = store();
        let calls = Arc::new(AtomicUsize::new(0));
        let main: Vec<u32> = cache.load_by_ids(&store, Ids::new("c1", "main", "catalog"), false, counting(&calls)).await.unwrap_or_default();
        let other: Vec<u32> = cache.load_by_ids(&store, Ids::new("c1", "other", "catalog"), false, counting(&calls)).await.unwrap_or_default();
        assert_eq!((main, other), (vec![1], vec![2]));
    }
}
