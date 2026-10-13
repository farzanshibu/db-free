// SOT: history-service, history-listing, history-paging, history-retention

use crate::error::AppResult;
use crate::guard::statement_info;
use crate::model::{HistoryEntry, HistoryOrigin, HistoryPage, HistoryStatus, StatementIntent};
use crate::state::AppState;
use crate::store::history::{HistoryFilter, NewHistoryEntry};

pub fn list(state: &AppState, connection_id: Option<&str>, origin: Option<HistoryOrigin>, limit: u32) -> AppResult<Vec<HistoryEntry>> {
    state.with_store(|store| store.list_history(connection_id, origin, limit))
}

// WHAT:  One window of the log and the filtered total, for the log viewer's pager.
pub fn page(
    state: &AppState,
    connection_id: Option<&str>,
    origin: Option<HistoryOrigin>,
    from_editor: Option<bool>,
    limit: u32,
    offset: u32,
) -> AppResult<HistoryPage> {
    state.with_store(|store| {
        let filter = HistoryFilter { connection_id, origin, from_editor, limit, offset };
        let (entries, total) = store.history_page(&filter)?;
        Ok(HistoryPage { entries, total })
    })
}

// WHAT:  The editor's recent statements, each text once.
pub fn recent_editor(state: &AppState, connection_id: &str, limit: u32) -> AppResult<Vec<HistoryEntry>> {
    state.with_store(|store| store.recent_editor_queries(connection_id, limit))
}

pub fn clear(state: &AppState, connection_id: Option<&str>) -> AppResult<u64> {
    state.with_store(|store| store.clear_history(connection_id))
}

// WHAT:  Deletes log rows older than the retention setting. 0 days keeps everything.
// WHY:   The log grows with every statement and every table page; left alone it
//        would be the largest thing in the app's database.
// WHERE: src-tauri/src/lib.rs (setup runs it once at startup)
pub fn prune(state: &AppState, retention_days: u32) -> AppResult<u64> {
    if retention_days == 0 {
        return Ok(0);
    }
    state.with_store(|store| store.delete_history_older_than(retention_days))
}

// WHAT:  Startup housekeeping: applies the saved retention setting.
// HOW:   A failure is logged, never raised: the app must start with an old log.
pub fn prune_on_start(state: &AppState) {
    let days = match crate::services::settings::get(state) {
        Ok(settings) => settings.log_retention_days,
        Err(err) => {
            log::warn!("history retention skipped, settings unreadable: {err}");
            return;
        }
    };
    match prune(state, days) {
        Ok(0) => {}
        Ok(removed) => log::info!("history retention removed {removed} entries older than {days} days"),
        Err(err) => log::warn!("history retention failed: {err}"),
    }
}

// WHAT:  Logs a statement the app issued on the user's behalf (table pages).
pub fn record_system(state: &AppState, connection_id: &str, sql: &str, ok: Result<u64, &str>, elapsed_ms: u64) {
    let info = statement_info::describe(sql);
    let entry = NewHistoryEntry {
        connection_id,
        sql,
        status: if ok.is_ok() { HistoryStatus::Ok } else { HistoryStatus::Error },
        origin: HistoryOrigin::System,
        error: ok.err(),
        elapsed_ms,
        row_count: ok.ok(),
        statement_type: Some(info.kind.as_str()).filter(|kind| !kind.is_empty()),
        table_name: info.table.as_deref(),
        intent: Some(StatementIntent::Read),
        user: None,
        from_editor: false,
    };
    if let Err(err) = state.with_store(|store| store.insert_history(&entry)) {
        log::warn!("system history log failed: {err}");
    }
}
