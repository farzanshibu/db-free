// SOT: connection-hygiene, session-hazard-scan, pinned-sessions, dead-session-detection, pooler-detection, session-key

use crate::error::{AppError, AppResult};
use crate::guard::destructive::split_statements;
use crate::model::StatementResult;
use async_trait::async_trait;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ============================================================================
// CONNECTION HYGIENE AND QUERY-TAB SESSIONS
//
// WHAT:  The vendor-free half of the engine-robustness work shared by the
//        pooled SQL adapters (Postgres / MySQL family, MSSQL, SQLite):
//          1. `scan`        what a script can leave behind on the connection it
//                           ran on (an open or aborted transaction, SET / USE /
//                           temp tables / advisory locks);
//          2. `Pins`        one dedicated connection per (connection, query tab),
//                           beside the pool, with idle expiry and dead-session
//                           detection;
//          3. `is_connection_lost`, `wants_simple_protocol`, `valid_session_key`.
// WHY:   sqlx 0.8 hands a connection back to the pool after only a ping, and the
//        transaction status it tracks is private: a script that did `BEGIN` and
//        then failed (or was stopped, or simply never committed) returns its
//        connection to the pool inside that transaction, and the next unrelated
//        request (a grid page, the sidebar) runs inside it, or fails with
//        "current transaction is aborted". The same goes for `SET search_path`
//        and `USE db`, which silently move every later query on that connection.
//        The editor wants the opposite for its own tab: SET, temp tables and
//        BEGIN should persist from one Run to the next.
// HOW:   The adapters scan each script before running it. A clean script costs
//        nothing extra; a script that touched transaction or session state gets
//        the connection rolled back / reset before it is released, or discarded
//        if that fails or the run was dropped half-way (Stop, timeout). Query
//        tabs that name a session key bypass the pool entirely and run on their
//        own pinned connection, where that state is the point.
// WHERE: src-tauri/src/integrations/{postgres,mysql,mssql,sqlite}.rs,
//        src-tauri/src/guard/mod.rs (`statement_in_session` sets the key)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Mysql,
    Mssql,
}

// WHAT:  What a script can leave on the connection it ran on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Hazard {
    /// The script contains transaction control (BEGIN, SAVEPOINT, SET autocommit…),
    /// so a failure or a Stop can leave the connection mid-transaction.
    pub transaction: bool,
    /// Read in order, the script finishes with a transaction still open.
    pub left_open: bool,
    /// Session state a rollback does not undo (SET, temp tables, locks, user variables).
    pub state: bool,
    /// The default database was switched (`USE db`), which the adapter can switch back.
    pub database_switch: bool,
}

impl Hazard {
    pub fn any(self) -> bool {
        self.transaction || self.state || self.database_switch
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    Word(String),
    Assign,
}

// WHAT:  Upper-cased words of one statement with literals and comments removed,
//        plus `@user_variable` names and the `:=` operator MySQL assigns with.
// HOW:   Strings, quoted identifiers, `--` / `/* */` comments, `#` comments (MySQL)
//        and Postgres dollar-quotes are skipped whole, so `'BEGIN'` in a literal
//        never counts as a BEGIN.
fn lex(statement: &str, hash_comments: bool) -> Vec<Tok> {
    let chars: Vec<char> = statement.chars().collect();
    let len = chars.len();
    let mut out = Vec::new();
    let mut i = 0_usize;
    while i < len {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '\'' | '"' | '`' => {
                i += 1;
                while i < len {
                    if chars[i] == c {
                        if chars.get(i + 1).copied() == Some(c) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            '-' if next == Some('-') => skip_line(&chars, &mut i),
            '#' if hash_comments => skip_line(&chars, &mut i),
            '/' if next == Some('*') => {
                i += 2;
                while i < len && !(chars[i] == '*' && chars.get(i + 1).copied() == Some('/')) {
                    i += 1;
                }
                i += 2;
            }
            '$' => {
                let mut j = i + 1;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if chars.get(j).copied() == Some('$') {
                    let tag: Vec<char> = chars[i..=j].to_vec();
                    i = j + 1;
                    while i < len && !chars[i..].starts_with(&tag) {
                        i += 1;
                    }
                    i += tag.len();
                } else {
                    i += 1;
                }
            }
            ':' if next == Some('=') => {
                out.push(Tok::Assign);
                i += 2;
            }
            '@' => {
                let mut word = String::from("@");
                i += 1;
                while i < len && chars[i] == '@' {
                    word.push('@');
                    i += 1;
                }
                while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                    word.push(chars[i].to_ascii_uppercase());
                    i += 1;
                }
                out.push(Tok::Word(word));
            }
            c if c.is_alphanumeric() || c == '_' => {
                let mut word = String::new();
                while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    word.push(chars[i].to_ascii_uppercase());
                    i += 1;
                }
                out.push(Tok::Word(word));
            }
            _ => i += 1,
        }
    }
    out
}

fn skip_line(chars: &[char], i: &mut usize) {
    while *i < chars.len() && chars[*i] != '\n' {
        *i += 1;
    }
}

/// Functions whose effect outlives the statement on the same connection.
const PG_SESSION_FUNCTIONS: &[&str] = &[
    "SET_CONFIG",
    "PG_ADVISORY_LOCK",
    "PG_ADVISORY_LOCK_SHARED",
    "PG_TRY_ADVISORY_LOCK",
    "PG_TRY_ADVISORY_LOCK_SHARED",
];

// WHAT:  Reads a script and answers what it can leave behind (see `Hazard`).
// HOW:   Statement by statement, in order, so `BEGIN; …; COMMIT` reads as balanced
//        and `BEGIN; …` as left open. Unknown statements are clean: a false
//        positive costs one reset round trip, a false negative is the bug this
//        module exists to prevent, so anything that might change session state
//        is listed.
pub fn scan(sql: &str, dialect: Dialect) -> Hazard {
    let mut hazard = Hazard::default();
    for statement in split_statements(sql) {
        let toks = lex(&statement, dialect == Dialect::Mysql);
        let words: Vec<&str> = toks
            .iter()
            .filter_map(|t| match t {
                Tok::Word(w) => Some(w.as_str()),
                Tok::Assign => None,
            })
            .collect();
        if words.is_empty() {
            continue;
        }
        match dialect {
            Dialect::Postgres => scan_postgres(&mut hazard, &words),
            Dialect::Mysql => scan_mysql(&mut hazard, &words, &toks),
            Dialect::Mssql => scan_mssql(&mut hazard, &words),
        }
    }
    hazard
}

fn has(words: &[&str], word: &str) -> bool {
    words.contains(&word)
}

/// `AND CHAIN` makes COMMIT / ROLLBACK start the next transaction at once.
fn chains(words: &[&str]) -> bool {
    has(words, "CHAIN") && !has(words, "NO")
}

fn scan_postgres(h: &mut Hazard, w: &[&str]) {
    let first = w.first().copied().unwrap_or_default();
    let second = w.get(1).copied().unwrap_or_default();
    match first {
        "BEGIN" | "START" => {
            h.transaction = true;
            h.left_open = true;
        }
        "COMMIT" | "END" | "ABORT" => h.left_open = chains(w),
        "ROLLBACK" => {
            if second != "TO" && !(has(w, "TO") && has(w, "SAVEPOINT")) {
                h.left_open = chains(w);
            }
        }
        "SAVEPOINT" | "RELEASE" | "LOCK" => h.transaction = true,
        "SET" => match second {
            "LOCAL" | "TRANSACTION" | "CONSTRAINTS" => h.transaction = true,
            _ => h.state = true,
        },
        "RESET" | "LISTEN" | "PREPARE" | "LOAD" => h.state = true,
        "DECLARE" => {
            if has(w, "HOLD") && !has(w, "WITHOUT") {
                h.state = true;
            } else {
                h.transaction = true;
            }
        }
        "CREATE" if w.iter().take(4).any(|t| *t == "TEMP" || *t == "TEMPORARY") => h.state = true,
        _ => {}
    }
    if first != "SET" {
        if w.iter().any(|t| PG_SESSION_FUNCTIONS.contains(t)) || (has(w, "INTO") && (has(w, "TEMP") || has(w, "TEMPORARY"))) {
            h.state = true;
        }
        if has(w, "PG_ADVISORY_XACT_LOCK") || has(w, "PG_TRY_ADVISORY_XACT_LOCK") {
            h.transaction = true;
        }
    }
}

fn scan_mysql(h: &mut Hazard, w: &[&str], toks: &[Tok]) {
    let first = w.first().copied().unwrap_or_default();
    let second = w.get(1).copied().unwrap_or_default();
    match first {
        "BEGIN" => {
            h.transaction = true;
            h.left_open = true;
        }
        "START" if second == "TRANSACTION" => {
            h.transaction = true;
            h.left_open = true;
        }
        "COMMIT" => h.left_open = chains(w),
        "ROLLBACK" => {
            if second != "TO" && !(has(w, "TO") && has(w, "SAVEPOINT")) {
                h.left_open = chains(w);
            }
        }
        "SAVEPOINT" | "RELEASE" => h.transaction = true,
        "XA" => {
            h.transaction = true;
            h.left_open = true;
        }
        "SET" => match second {
            "GLOBAL" | "PERSIST" | "PERSIST_ONLY" => {}
            _ => {
                h.state = true;
                if w.iter().any(|t| t.ends_with("AUTOCOMMIT")) {
                    h.transaction = true;
                    h.left_open = true;
                }
            }
        },
        "USE" => h.database_switch = true,
        "LOCK" | "PREPARE" | "HANDLER" | "DEALLOCATE" => h.state = true,
        "FLUSH" if has(w, "LOCK") => h.state = true,
        "CREATE" if w.iter().take(3).any(|t| *t == "TEMPORARY") => h.state = true,
        _ => {}
    }
    if first != "SET" {
        if has(w, "GET_LOCK") {
            h.state = true;
        }
        // `@x := …` and `… INTO @x` set a variable that outlives the statement.
        let mut seen_into = false;
        for (index, tok) in toks.iter().enumerate() {
            if let Tok::Word(word) = tok {
                if word == "INTO" {
                    seen_into = true;
                }
                if word.starts_with('@') && !word.starts_with("@@") && (seen_into || toks.get(index + 1) == Some(&Tok::Assign)) {
                    h.state = true;
                }
            }
        }
    }
}

fn scan_mssql(h: &mut Hazard, w: &[&str]) {
    let first = w.first().copied().unwrap_or_default();
    let second = w.get(1).copied().unwrap_or_default();
    match first {
        // `BEGIN TRY` / `BEGIN CATCH` / a bare `BEGIN` are blocks, not transactions.
        "BEGIN" if second.starts_with("TRAN") || second == "DISTRIBUTED" => {
            h.transaction = true;
            h.left_open = true;
        }
        "COMMIT" | "ROLLBACK" => h.left_open = false,
        "SAVE" => h.transaction = true,
        "SET" if has(w, "IMPLICIT_TRANSACTIONS") => {
            h.transaction = true;
            h.left_open = true;
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Dead-session detection
// ---------------------------------------------------------------------------

/// What an adapter knows about a failed call, in driver-neutral terms.
#[derive(Debug, Clone, Copy, Default)]
pub struct ErrorFacts<'a> {
    /// The transport failed (socket error, TLS, protocol desync), not the SQL.
    pub transport: bool,
    /// SQLSTATE, when the server sent one.
    pub sqlstate: Option<&'a str>,
    /// Vendor error number (MySQL / MariaDB), when the server sent one.
    pub number: Option<u32>,
    pub message: &'a str,
}

// WHAT:  True when the connection itself is gone rather than the statement being wrong.
// WHY:   A dead pinned session has to be replaced; a mistyped query must not
//        throw away the session (and its temp tables, SETs and open transaction).
// HOW:   Transport errors, SQLSTATE class 08 (connection exception), Postgres
//        shutdown / idle-timeout codes (57P01-03, 25P03, 25P04), MySQL's "gone away"
//        family, and the phrases drivers use when a socket is reset or closed.
pub fn is_connection_lost(facts: ErrorFacts<'_>) -> bool {
    if facts.transport {
        return true;
    }
    if let Some(state) = facts.sqlstate {
        if state.starts_with("08") || matches!(state, "57P01" | "57P02" | "57P03" | "25P03" | "25P04") {
            return true;
        }
    }
    // 2006 server has gone away, 2013 lost connection, 2055 lost connection at
    // read, 1053 shutdown in progress, 1927 connection was killed, 4031 idle timeout.
    if matches!(facts.number, Some(2006 | 2013 | 2055 | 1053 | 1927 | 4031)) {
        return true;
    }
    let message = facts.message.to_ascii_lowercase();
    [
        "connection reset",
        "broken pipe",
        "connection closed",
        "connection was closed",
        "connection aborted",
        "server closed the connection",
        "terminating connection",
        "unexpected eof",
        "server has gone away",
        "lost connection",
        "econnreset",
        "pool is closed",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

// ---------------------------------------------------------------------------
// Transaction poolers
// ---------------------------------------------------------------------------

// WHAT:  True when the server end looks like a transaction pooler (PgBouncer,
//        the Supabase / Neon poolers), where server-side prepared statements
//        cannot be reused across requests.
// WHY:   sqlx caches each prepared statement by name on the connection it was
//        parsed on; behind a pooler the next request lands on another server
//        connection and fails with "prepared statement does not exist".
//        Capacity 0 makes every call parse its own unnamed statement.
// HOW:   An explicit `pgbouncer=true` option, the well-known pooler ports
//        (6543 Supabase, 6432 PgBouncer's default) or a host that names a
//        pooler. `options` are the connection string's extra `key=value` pairs.
pub fn wants_simple_protocol(host: &str, port: u16, options: &[(&str, &str)]) -> bool {
    let flagged = options.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("pgbouncer") && matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
    });
    let host = host.to_ascii_lowercase();
    flagged || matches!(port, 6543 | 6432) || host.contains("pooler") || host.contains("pgbouncer")
}

// ---------------------------------------------------------------------------
// Session keys
// ---------------------------------------------------------------------------

const MAX_KEY_LEN: usize = 128;

/// A key names one query tab (the UI sends the tab id). Bounded and printable,
/// since it is a map key held for the life of the connection.
pub fn valid_session_key(key: &str) -> bool {
    !key.trim().is_empty() && key.chars().count() <= MAX_KEY_LEN && !key.chars().any(char::is_control)
}

tokio::task_local! {
    /// The query tab the current request belongs to; set by the block around
    /// the handler of a statement request that named one.
    static SESSION_KEY: Option<String>;
}

// WHAT:  Runs `fut` with `key` visible to `current_session_key()` inside it.
// WHY:   Same trick as `with_run_id`: the trait's `execute` is shared by every
//        adapter and service, and a task-local carries the tab to the two
//        adapters that can pin a connection without widening that signature.
// WHERE: src-tauri/src/guard/mod.rs (`statement_in_session`)
pub async fn with_session_key<F: Future>(key: Option<String>, fut: F) -> F::Output {
    SESSION_KEY.scope(key, fut).await
}

pub fn current_session_key() -> Option<String> {
    SESSION_KEY.try_with(Clone::clone).ok().flatten()
}

// ---------------------------------------------------------------------------
// Pinned sessions
// ---------------------------------------------------------------------------

// WHAT:  One dedicated connection, as the registry sees it.
#[async_trait]
pub trait PinnedConn: Send {
    /// Runs the script on this connection and nowhere else.
    async fn run(&mut self, sql: &str, max_rows: usize) -> AppResult<Vec<StatementResult>>;
    /// Cheap round trip that says whether the server still answers. Must work
    /// in any transaction state, including an aborted one.
    async fn alive(&mut self) -> bool;
    /// The last `run` failed because the connection died, not because the SQL was wrong.
    fn lost(&self) -> bool;
    async fn close(&mut self);
}

// WHAT:  Opens a fresh dedicated connection with the adapter's own options.
#[async_trait]
pub trait PinOpener: Send + Sync {
    async fn open(&self) -> AppResult<Box<dyn PinnedConn>>;
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A session idle this long is closed.
    pub idle: Duration,
    /// A session idle at least this long is probed before the next script is sent.
    pub probe_after: Duration,
    /// How often the reaper looks for idle sessions.
    pub sweep_every: Duration,
    /// Dedicated connections one database connection may hold.
    pub max: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { idle: Duration::from_secs(30 * 60), probe_after: Duration::from_secs(20), sweep_every: Duration::from_secs(60), max: 16 }
    }
}

/// Said once when a session that held state was found dead before a script was sent.
pub const SESSION_EXPIRED: &str = "This query tab's database session was closed by the server, so its settings, temporary tables and any open transaction are gone. Nothing was run; run again to continue on a new session.";

fn lost_error(original: &AppError) -> AppError {
    AppError::driver(format!(
        "The connection of this query tab's session was lost ({}). The script may or may not have been applied. Run again to reconnect; session settings, temporary tables and any open transaction are gone.",
        original.message().trim_end_matches('.')
    ))
}

struct SlotState {
    conn: Option<Box<dyn PinnedConn>>,
    last_used: Instant,
    /// A script on this session set state worth telling the user about if it vanishes.
    stateful: bool,
    /// The connection died while nobody was running on it and the user has not been told.
    unannounced_loss: bool,
}

struct Slot {
    state: tokio::sync::Mutex<SlotState>,
    touched: Mutex<Instant>,
}

impl Slot {
    fn new() -> Slot {
        Slot {
            state: tokio::sync::Mutex::new(SlotState { conn: None, last_used: Instant::now(), stateful: false, unannounced_loss: false }),
            touched: Mutex::new(Instant::now()),
        }
    }

    fn touch(&self) {
        if let Ok(mut at) = self.touched.lock() {
            *at = Instant::now();
        }
    }

    fn touched(&self) -> Instant {
        self.touched.lock().map(|at| *at).unwrap_or_else(|_| Instant::now())
    }
}

struct PinsInner {
    opener: Arc<dyn PinOpener>,
    dialect: Dialect,
    limits: Limits,
    slots: Mutex<HashMap<String, Arc<Slot>>>,
}

// WHAT:  The registry of pinned connections for one database connection.
// WHY:   A query tab expects `SET search_path`, `CREATE TEMP TABLE` and `BEGIN`
//        to still be there on its next Run, which a pool cannot promise.
// HOW:   Created lazily on a tab's first Run, kept until the tab closes
//        (`close`), the connection closes (`close_all`) or the session sits idle
//        past `Limits::idle`. Before a script goes to a session that has been
//        idle, the connection is probed; a dead one is replaced, silently while
//        the session held nothing worth keeping and with one explicit error when
//        it did. A script that fails because the connection died is never
//        retried: it may have been applied, so the user decides.
// WHERE: src-tauri/src/integrations/{postgres,mysql}.rs
pub struct Pins {
    inner: Arc<PinsInner>,
}

impl Pins {
    pub fn new(opener: Arc<dyn PinOpener>, dialect: Dialect) -> Pins {
        Pins::with_limits(opener, dialect, Limits::default())
    }

    pub fn with_limits(opener: Arc<dyn PinOpener>, dialect: Dialect, limits: Limits) -> Pins {
        let inner = Arc::new(PinsInner { opener, dialect, limits, slots: Mutex::new(HashMap::new()) });
        if tokio::runtime::Handle::try_current().is_ok() {
            let weak = Arc::downgrade(&inner);
            let period = limits.sweep_every;
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(period).await;
                    let Some(inner) = weak.upgrade() else { break };
                    inner.sweep().await;
                }
            });
        }
        Pins { inner }
    }

    // WHAT:  Runs `sql` on the session named `key`, opening it first when needed.
    pub async fn run(&self, key: &str, sql: &str, max_rows: usize) -> AppResult<Vec<StatementResult>> {
        let slot = self.inner.slot(key)?;
        let mut state = slot.state.lock().await;

        // A session that sat idle may have been dropped by the server or a
        // firewall. Finding out before sending is what makes replacing it safe.
        let probe_due = state.last_used.elapsed() >= self.inner.limits.probe_after;
        if probe_due {
            if let Some(conn) = state.conn.as_mut() {
                if !conn.alive().await {
                    conn.close().await;
                    state.conn = None;
                    state.unannounced_loss = true;
                }
            }
        }
        if state.conn.is_none() {
            if state.unannounced_loss && state.stateful {
                state.unannounced_loss = false;
                state.stateful = false;
                return Err(AppError::driver(SESSION_EXPIRED));
            }
            state.unannounced_loss = false;
            state.stateful = false;
            state.conn = Some(self.inner.opener.open().await?);
        }
        if scan(sql, self.inner.dialect).any() {
            state.stateful = true;
        }

        let Some(conn) = state.conn.as_mut() else {
            return Err(AppError::internal("the session connection vanished"));
        };
        let result = conn.run(sql, max_rows).await;
        let lost = conn.lost();
        state.last_used = Instant::now();
        slot.touch();
        if lost {
            if let Some(mut dead) = state.conn.take() {
                dead.close().await;
            }
            // The error below tells the user; the next run reconnects without repeating it.
            state.unannounced_loss = false;
            state.stateful = false;
            return Err(match &result {
                Err(original) => lost_error(original),
                Ok(_) => lost_error(&AppError::driver("connection closed")),
            });
        }
        result
    }

    /// Closes the session of one tab. A script still running on it finishes first.
    pub async fn close(&self, key: &str) {
        let removed = self.inner.slots.lock().ok().and_then(|mut slots| slots.remove(key));
        if let Some(slot) = removed {
            retire(slot);
        }
    }

    /// Closes every session (the connection is closing). Sessions busy with a
    /// script are given a moment, then closed in the background.
    pub async fn close_all(&self) {
        let slots: Vec<Arc<Slot>> = match self.inner.slots.lock() {
            Ok(mut slots) => slots.drain().map(|(_, slot)| slot).collect(),
            Err(_) => return,
        };
        for slot in slots {
            match tokio::time::timeout(Duration::from_secs(2), slot.state.lock()).await {
                Ok(mut state) => {
                    if let Some(mut conn) = state.conn.take() {
                        conn.close().await;
                    }
                }
                Err(_) => retire(Arc::clone(&slot)),
            }
        }
    }

    /// Sessions currently holding a connection (for tests and diagnostics).
    pub fn open_sessions(&self) -> usize {
        let slots: Vec<Arc<Slot>> = self.inner.slots.lock().map(|slots| slots.values().cloned().collect()).unwrap_or_default();
        slots.iter().filter(|slot| slot.state.try_lock().map(|state| state.conn.is_some()).unwrap_or(true)).count()
    }

    /// Closes sessions idle past the limit. The reaper calls it on a timer.
    pub async fn sweep(&self) {
        self.inner.sweep().await;
    }
}

impl PinsInner {
    fn slot(&self, key: &str) -> AppResult<Arc<Slot>> {
        let mut slots = self.slots.lock().map_err(|_| AppError::internal("the session table is poisoned"))?;
        if let Some(slot) = slots.get(key) {
            slot.touch();
            return Ok(Arc::clone(slot));
        }
        if slots.len() >= self.limits.max {
            // Make room by retiring the least recently used session nobody is running on.
            let victim = slots
                .iter()
                .filter(|(_, slot)| slot.state.try_lock().is_ok())
                .min_by_key(|(_, slot)| slot.touched())
                .map(|(key, _)| key.clone());
            match victim.and_then(|key| slots.remove(&key)) {
                Some(old) => retire(old),
                None => {
                    return Err(AppError::invalid_input(format!(
                        "{} query sessions are running at once. Wait for one to finish or close a query tab.",
                        self.limits.max
                    )))
                }
            }
        }
        let slot = Arc::new(Slot::new());
        slots.insert(key.to_string(), Arc::clone(&slot));
        Ok(slot)
    }

    async fn sweep(&self) {
        let slots: Vec<Arc<Slot>> = match self.slots.lock() {
            Ok(slots) => slots.values().cloned().collect(),
            Err(_) => return,
        };
        for slot in slots {
            let Ok(mut state) = slot.state.try_lock() else { continue };
            if state.conn.is_some() && state.last_used.elapsed() >= self.limits.idle {
                if let Some(mut conn) = state.conn.take() {
                    conn.close().await;
                }
                state.unannounced_loss = true;
            }
        }
    }
}

// WHAT:  Closes a removed session's connection once whatever is running on it ends.
fn retire(slot: Arc<Slot>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        let mut state = slot.state.lock().await;
        if let Some(mut conn) = state.conn.take() {
            conn.close().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn pg(sql: &str) -> Hazard {
        scan(sql, Dialect::Postgres)
    }
    fn my(sql: &str) -> Hazard {
        scan(sql, Dialect::Mysql)
    }
    fn ms(sql: &str) -> Hazard {
        scan(sql, Dialect::Mssql)
    }

    #[test]
    fn plain_reads_and_writes_leave_nothing_behind() {
        for sql in ["SELECT 1", "select * from t where a = 'BEGIN'", "INSERT INTO t VALUES (1); UPDATE t SET a = 2 WHERE id = 1", "-- BEGIN\nSELECT 1", "/* SET x */ SELECT 2"] {
            assert_eq!(pg(sql), Hazard::default(), "{sql}");
            assert_eq!(my(sql), Hazard::default(), "{sql}");
        }
        assert_eq!(pg("DO $$ BEGIN PERFORM 1; END $$"), Hazard::default(), "a PL/pgSQL block is not a transaction");
        assert_eq!(pg("SELECT 'SET search_path TO x'"), Hazard::default());
    }

    #[test]
    fn a_balanced_transaction_is_flagged_but_not_left_open() {
        let h = pg("BEGIN; UPDATE t SET a = 1 WHERE id = 1; COMMIT;");
        assert!(h.transaction && !h.left_open && !h.state);
        assert!(pg("BEGIN; ROLLBACK").transaction);
        assert!(!pg("BEGIN; ROLLBACK").left_open);
    }

    #[test]
    fn an_unfinished_transaction_is_left_open() {
        assert!(pg("BEGIN; INSERT INTO t VALUES (1)").left_open);
        assert!(pg("START TRANSACTION; SELECT 1").left_open);
        assert!(pg("COMMIT AND CHAIN").left_open, "AND CHAIN starts the next one");
        assert!(!pg("COMMIT AND NO CHAIN").left_open);
        assert!(pg("BEGIN; SAVEPOINT a; ROLLBACK TO SAVEPOINT a").left_open, "rolling back to a savepoint keeps the transaction");
        assert!(!pg("BEGIN; ROLLBACK TO a; COMMIT").left_open);
    }

    #[test]
    fn postgres_session_state_is_recognised() {
        for sql in [
            "SET search_path TO app",
            "SET SESSION AUTHORIZATION bob",
            "RESET ALL",
            "CREATE TEMP TABLE x (a int)",
            "create temporary table x as select 1",
            "SELECT set_config('app.user', 'x', false)",
            "SELECT pg_advisory_lock(1)",
            "LISTEN jobs",
            "PREPARE p AS SELECT 1",
            "SELECT * INTO TEMP t FROM u",
            "DECLARE c CURSOR WITH HOLD FOR SELECT 1",
        ] {
            assert!(pg(sql).state, "{sql}");
        }
        assert!(!pg("SET LOCAL statement_timeout = 5").state, "SET LOCAL ends with the transaction");
        assert!(pg("SET LOCAL statement_timeout = 5").transaction);
        assert!(!pg("CREATE TABLE x (a int)").state);
        assert!(pg("DECLARE c CURSOR FOR SELECT 1").transaction, "a plain cursor needs a transaction");
    }

    #[test]
    fn mysql_state_and_transactions() {
        assert!(my("SET @a = 1").state);
        assert!(my("SET NAMES utf8mb4").state);
        assert!(my("SET SESSION sql_mode = ''").state);
        assert!(my("SELECT @n := 5").state);
        assert!(my("SELECT 1 INTO @x").state);
        assert!(my("SELECT GET_LOCK('a', 1)").state);
        assert!(my("CREATE TEMPORARY TABLE t (a int)").state);
        assert!(my("LOCK TABLES t WRITE").state);
        assert!(!my("SET GLOBAL max_connections = 10").any(), "server-wide, not this connection");
        assert!(!my("SELECT @@version").any(), "reading a system variable is not an assignment");
        assert!(!my("START SLAVE").any());
        let auto = my("SET autocommit = 0");
        assert!(auto.state && auto.transaction && auto.left_open);
        let tx = my("START TRANSACTION; UPDATE t SET a = 1 WHERE id = 1");
        assert!(tx.transaction && tx.left_open);
        assert!(!my("BEGIN; UPDATE t SET a = 1 WHERE id = 1; COMMIT").left_open);
    }

    #[test]
    fn mysql_use_is_restorable_not_state() {
        let h = my("USE other; SELECT * FROM t");
        assert!(h.database_switch && !h.state && !h.transaction);
        assert!(h.any());
        assert!(!pg("USE other").any(), "Postgres has no USE");
    }

    #[test]
    fn mssql_only_tracks_transactions() {
        assert!(ms("BEGIN TRAN; INSERT INTO t VALUES (1)").left_open);
        assert!(ms("BEGIN TRANSACTION; COMMIT TRAN").transaction);
        assert!(!ms("BEGIN TRANSACTION; COMMIT TRAN").left_open);
        assert!(!ms("BEGIN TRY SELECT 1 END TRY").any(), "a TRY block is not a transaction");
        assert!(ms("SET IMPLICIT_TRANSACTIONS ON").left_open);
        assert!(ms("SAVE TRAN a").transaction);
        assert!(!ms("SET NOCOUNT ON").any());
    }

    #[test]
    fn lost_connections_are_told_apart_from_bad_sql() {
        let facts = |sqlstate: Option<&'static str>, number: Option<u32>, message: &'static str, transport: bool| {
            is_connection_lost(ErrorFacts { transport, sqlstate, number, message })
        };
        assert!(facts(None, None, "x", true));
        assert!(facts(Some("08006"), None, "connection failure", false));
        assert!(facts(Some("57P01"), None, "terminating connection due to administrator command", false));
        assert!(facts(Some("57P03"), None, "the database system is starting up", false));
        assert!(facts(Some("25P03"), None, "idle-in-transaction timeout", false));
        assert!(facts(None, Some(2006), "MySQL server has gone away", false));
        assert!(facts(None, Some(2013), "Lost connection to MySQL server during query", false));
        assert!(facts(None, None, "Connection reset by peer (os error 54)", false));
        assert!(facts(None, None, "server closed the connection unexpectedly", false));
        assert!(!facts(Some("42601"), None, "syntax error at or near \"selec\"", false));
        assert!(!facts(Some("57014"), None, "canceling statement due to user request", false), "a Stop is not a dead session");
        assert!(!facts(Some("25P02"), None, "current transaction is aborted", false), "an aborted transaction is alive");
        assert!(!facts(None, Some(1064), "You have an error in your SQL syntax", false));
    }

    #[test]
    fn poolers_are_detected_by_option_port_and_host() {
        assert!(wants_simple_protocol("db.example.com", 5432, &[("pgbouncer", "true")]));
        assert!(wants_simple_protocol("db.example.com", 5432, &[("PgBouncer", "1")]));
        assert!(!wants_simple_protocol("db.example.com", 5432, &[("pgbouncer", "false")]));
        assert!(wants_simple_protocol("db.example.com", 6543, &[]), "the Supabase transaction pooler port");
        assert!(wants_simple_protocol("127.0.0.1", 6432, &[]), "PgBouncer's default port");
        assert!(wants_simple_protocol("aws-0-eu-west-1.pooler.supabase.com", 5432, &[]));
        assert!(wants_simple_protocol("ep-cool-darkness-123456-pooler.us-east-2.aws.neon.tech", 5432, &[]));
        assert!(!wants_simple_protocol("localhost", 5432, &[]));
        assert!(!wants_simple_protocol("db.internal", 5432, &[("sslmode", "require")]));
    }

    #[test]
    fn session_keys_are_bounded_and_printable() {
        assert!(valid_session_key("query:abc:1700000000000-1"));
        assert!(!valid_session_key(""));
        assert!(!valid_session_key("   "));
        assert!(!valid_session_key("a\nb"));
        assert!(!valid_session_key(&"k".repeat(MAX_KEY_LEN + 1)));
        assert!(valid_session_key(&"k".repeat(MAX_KEY_LEN)));
    }

    #[tokio::test]
    async fn the_session_key_travels_with_the_task() {
        assert_eq!(current_session_key(), None);
        let seen = with_session_key(Some("tab-1".into()), async { current_session_key() }).await;
        assert_eq!(seen.as_deref(), Some("tab-1"));
        assert_eq!(current_session_key(), None, "the scope ends with the future");
    }

    // ---- registry ----------------------------------------------------------

    #[derive(Default)]
    struct Shared {
        opened: AtomicUsize,
        closed: AtomicUsize,
        /// Every open connection answers `alive` with this.
        answering: AtomicBool,
        /// The next `run` dies mid-flight.
        die_on_run: AtomicBool,
        fail_open: AtomicBool,
    }

    struct MockConn {
        id: usize,
        shared: Arc<Shared>,
        lost: bool,
    }

    #[async_trait]
    impl PinnedConn for MockConn {
        async fn run(&mut self, sql: &str, _max_rows: usize) -> AppResult<Vec<StatementResult>> {
            self.lost = false;
            if self.shared.die_on_run.swap(false, Ordering::SeqCst) {
                self.lost = true;
                return Err(AppError::driver("Connection reset by peer"));
            }
            if sql.contains("bad") {
                return Err(AppError::driver("syntax error"));
            }
            Ok(vec![StatementResult::Affected { rows_affected: self.id as u64 }])
        }
        async fn alive(&mut self) -> bool {
            self.shared.answering.load(Ordering::SeqCst)
        }
        fn lost(&self) -> bool {
            self.lost
        }
        async fn close(&mut self) {
            self.shared.closed.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct MockOpener(Arc<Shared>);

    #[async_trait]
    impl PinOpener for MockOpener {
        async fn open(&self) -> AppResult<Box<dyn PinnedConn>> {
            if self.0.fail_open.load(Ordering::SeqCst) {
                return Err(AppError::driver("could not connect"));
            }
            let id = self.0.opened.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(Box::new(MockConn { id, shared: Arc::clone(&self.0), lost: false }))
        }
    }

    fn pins(limits: Limits) -> (Pins, Arc<Shared>) {
        let shared = Arc::new(Shared::default());
        shared.answering.store(true, Ordering::SeqCst);
        let pins = Pins::with_limits(Arc::new(MockOpener(Arc::clone(&shared))), Dialect::Postgres, limits);
        (pins, shared)
    }

    fn eager() -> Limits {
        Limits { probe_after: Duration::ZERO, ..Limits::default() }
    }

    async fn id_of(pins: &Pins, key: &str, sql: &str) -> AppResult<u64> {
        match pins.run(key, sql, 10).await?.first() {
            Some(StatementResult::Affected { rows_affected }) => Ok(*rows_affected),
            other => Err(AppError::internal(format!("unexpected {other:?}"))),
        }
    }

    #[tokio::test]
    async fn one_tab_keeps_one_connection_and_tabs_do_not_share() {
        let (pins, shared) = pins(Limits::default());
        let a1 = id_of(&pins, "tab-a", "SET search_path TO x").await.unwrap_or(0);
        let a2 = id_of(&pins, "tab-a", "SELECT 1").await.unwrap_or(0);
        let b = id_of(&pins, "tab-b", "SELECT 1").await.unwrap_or(0);
        assert_eq!(a1, a2, "a tab's runs share one connection, so SET and BEGIN persist");
        assert_ne!(a1, b, "another tab has its own");
        assert_eq!(shared.opened.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_script_error_keeps_the_session() {
        let (pins, shared) = pins(eager());
        let first = id_of(&pins, "t", "SELECT 1").await.unwrap_or(0);
        assert!(id_of(&pins, "t", "bad sql").await.is_err());
        assert_eq!(id_of(&pins, "t", "SELECT 2").await.unwrap_or(0), first, "a mistyped query must not cost the tab its session");
        assert_eq!(shared.opened.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn closing_a_tab_closes_its_connection() {
        let (pins, shared) = pins(Limits::default());
        id_of(&pins, "t", "SELECT 1").await.unwrap_or(0);
        pins.close("t").await;
        for _ in 0..100 {
            if shared.closed.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(shared.closed.load(Ordering::SeqCst), 1);
        assert_eq!(id_of(&pins, "t", "SELECT 1").await.unwrap_or(0), 2, "the next run opens a new session");
        pins.close("never-opened").await;
    }

    #[tokio::test]
    async fn a_connection_that_dies_mid_run_is_reported_and_replaced_next_time_never_retried() {
        let (pins, shared) = pins(Limits::default());
        id_of(&pins, "t", "SELECT 1").await.unwrap_or(0);
        shared.die_on_run.store(true, Ordering::SeqCst);
        let err = id_of(&pins, "t", "UPDATE t SET a = 1 WHERE id = 1").await.err();
        let message = err.map(|e| e.message().to_string()).unwrap_or_default();
        assert!(message.contains("may or may not have been applied"), "{message}");
        assert!(message.contains("Connection reset by peer"), "{message}");
        assert_eq!(shared.opened.load(Ordering::SeqCst), 1, "the failed write was not retried on a new connection");
        assert_eq!(id_of(&pins, "t", "SELECT 1").await.unwrap_or(0), 2, "reconnected on next use");
        assert_eq!(shared.closed.load(Ordering::SeqCst), 1, "the dead connection was closed");
    }

    #[tokio::test]
    async fn a_dead_idle_session_is_replaced_silently_when_it_held_nothing() {
        let (pins, shared) = pins(eager());
        id_of(&pins, "t", "SELECT 1").await.unwrap_or(0);
        shared.answering.store(false, Ordering::SeqCst);
        // The replacement answers the next probe.
        let probe_then_answer = async {
            let id = id_of(&pins, "t", "SELECT 1").await;
            shared.answering.store(true, Ordering::SeqCst);
            id
        };
        assert_eq!(probe_then_answer.await.unwrap_or(0), 2, "a clean session is replaced without a word");
    }

    #[tokio::test]
    async fn a_dead_idle_session_that_held_state_says_so_once() {
        let (pins, shared) = pins(eager());
        id_of(&pins, "t", "CREATE TEMP TABLE scratch (a int)").await.unwrap_or(0);
        shared.answering.store(false, Ordering::SeqCst);
        let err = id_of(&pins, "t", "SELECT * FROM scratch").await.err();
        assert_eq!(err.map(|e| e.message().to_string()).as_deref(), Some(SESSION_EXPIRED));
        assert_eq!(shared.opened.load(Ordering::SeqCst), 1, "nothing was sent anywhere");
        shared.answering.store(true, Ordering::SeqCst);
        assert_eq!(id_of(&pins, "t", "SELECT 1").await.unwrap_or(0), 2, "the second run goes through on a new session");
        assert_eq!(id_of(&pins, "t", "SELECT 1").await.unwrap_or(0), 2);
    }

    #[tokio::test]
    async fn a_failed_open_leaves_the_tab_able_to_try_again() {
        let (pins, shared) = pins(Limits::default());
        shared.fail_open.store(true, Ordering::SeqCst);
        assert!(id_of(&pins, "t", "SELECT 1").await.is_err());
        shared.fail_open.store(false, Ordering::SeqCst);
        assert_eq!(id_of(&pins, "t", "SELECT 1").await.unwrap_or(0), 1);
    }

    #[tokio::test]
    async fn the_least_recently_used_session_makes_room() {
        let (pins, shared) = pins(Limits { max: 2, ..Limits::default() });
        id_of(&pins, "a", "SELECT 1").await.unwrap_or(0);
        tokio::time::sleep(Duration::from_millis(5)).await;
        id_of(&pins, "b", "SELECT 1").await.unwrap_or(0);
        tokio::time::sleep(Duration::from_millis(5)).await;
        id_of(&pins, "a", "SELECT 1").await.unwrap_or(0);
        tokio::time::sleep(Duration::from_millis(5)).await;
        id_of(&pins, "c", "SELECT 1").await.unwrap_or(0);
        for _ in 0..100 {
            if shared.closed.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(shared.closed.load(Ordering::SeqCst), 1, "b was the stalest");
        assert_eq!(id_of(&pins, "a", "SELECT 1").await.unwrap_or(0), 1, "a kept its connection");
    }

    #[tokio::test]
    async fn sweep_closes_sessions_idle_past_the_limit() {
        let (pins, shared) = pins(Limits { idle: Duration::ZERO, ..Limits::default() });
        id_of(&pins, "t", "SELECT 1").await.unwrap_or(0);
        assert_eq!(pins.open_sessions(), 1);
        pins.sweep().await;
        assert_eq!(shared.closed.load(Ordering::SeqCst), 1);
        assert_eq!(pins.open_sessions(), 0);
        assert_eq!(id_of(&pins, "t", "SELECT 1").await.unwrap_or(0), 2);
    }

    #[tokio::test]
    async fn close_all_closes_everything() {
        let (pins, shared) = pins(Limits::default());
        id_of(&pins, "a", "SELECT 1").await.unwrap_or(0);
        id_of(&pins, "b", "SELECT 1").await.unwrap_or(0);
        pins.close_all().await;
        assert_eq!(shared.closed.load(Ordering::SeqCst), 2);
        assert_eq!(pins.open_sessions(), 0);
    }
}
