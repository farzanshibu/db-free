// SOT: query-history-table, history-persistence, history-paging, history-retention

use crate::error::{AppError, AppResult};
use crate::model::{HistoryEntry, HistoryOrigin, HistoryStatus, StatementIntent};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, Row};

pub struct NewHistoryEntry<'a> {
    pub connection_id: &'a str,
    pub sql: &'a str,
    pub status: HistoryStatus,
    pub origin: HistoryOrigin,
    pub error: Option<&'a str>,
    pub elapsed_ms: u64,
    pub row_count: Option<u64>,
    pub statement_type: Option<&'a str>,
    pub table_name: Option<&'a str>,
    pub intent: Option<StatementIntent>,
    pub user: Option<&'a str>,
    pub from_editor: bool,
}

// WHAT:  Which slice of the log a reader wants.
// WHY:   The log viewer pages on the server (25 / 50 / 100 / 200 rows) and shows
//        the total, so the filters and the window travel together.
#[derive(Debug, Clone, Default)]
pub struct HistoryFilter<'a> {
    pub connection_id: Option<&'a str>,
    pub origin: Option<HistoryOrigin>,
    pub from_editor: Option<bool>,
    pub limit: u32,
    pub offset: u32,
}

const COLUMNS: &str = "id, connection_id, sql, status, error, elapsed_ms, row_count, executed_at, origin, \
                       statement_type, table_name, operation, db_user, from_editor";

const MAX_PAGE: u32 = 2_000;

fn intent_text(intent: StatementIntent) -> &'static str {
    match intent {
        StatementIntent::Read => "read",
        StatementIntent::Write => "write",
        StatementIntent::Destructive => "destructive",
    }
}

fn intent_from(raw: &str) -> Option<StatementIntent> {
    match raw {
        "read" => Some(StatementIntent::Read),
        "write" => Some(StatementIntent::Write),
        "destructive" => Some(StatementIntent::Destructive),
        _ => None,
    }
}

fn entry_from_row(row: &Row<'_>) -> rusqlite::Result<HistoryEntry> {
    let status_raw: String = row.get(3)?;
    let elapsed: i64 = row.get(5)?;
    let rows: Option<i64> = row.get(6)?;
    let origin_raw: String = row.get(8)?;
    let operation: Option<String> = row.get(11)?;
    let from_editor: i64 = row.get(13)?;
    Ok(HistoryEntry {
        id: row.get(0)?,
        connection_id: row.get(1)?,
        sql: row.get(2)?,
        status: HistoryStatus::parse(&status_raw),
        origin: HistoryOrigin::parse(&origin_raw),
        error: row.get(4)?,
        elapsed_ms: u64::try_from(elapsed).unwrap_or_default(),
        row_count: rows.and_then(|r| u64::try_from(r).ok()),
        executed_at: row.get(7)?,
        statement_type: row.get(9)?,
        table_name: row.get(10)?,
        operation: operation.as_deref().and_then(intent_from),
        user: row.get(12)?,
        from_editor: from_editor != 0,
    })
}

impl Store {
    pub fn insert_history(&self, entry: &NewHistoryEntry<'_>) -> AppResult<()> {
        self.conn()
            .execute(
                "INSERT INTO query_history (connection_id, sql, status, error, elapsed_ms, \
                 row_count, executed_at, origin, statement_type, table_name, operation, db_user, from_editor) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    entry.connection_id,
                    entry.sql,
                    entry.status.as_str(),
                    entry.error,
                    i64::try_from(entry.elapsed_ms).unwrap_or(i64::MAX),
                    entry.row_count.and_then(|r| i64::try_from(r).ok()),
                    now_rfc3339(),
                    entry.origin.as_str(),
                    entry.statement_type,
                    entry.table_name,
                    entry.intent.map(intent_text),
                    entry.user,
                    i64::from(entry.from_editor),
                ],
            )
            .map_err(AppError::store)?;
        Ok(())
    }

    pub fn list_history(&self, connection_id: Option<&str>, origin: Option<HistoryOrigin>, limit: u32) -> AppResult<Vec<HistoryEntry>> {
        let filter = HistoryFilter { connection_id, origin, from_editor: None, limit, offset: 0 };
        Ok(self.history_page(&filter)?.0)
    }

    // WHAT:  One window of the log, newest first, and how many rows match in all.
    // HOW:   The count and the page share one WHERE, so the pager's total is the
    //        filtered total and never drifts from the rows beneath it.
    pub fn history_page(&self, filter: &HistoryFilter<'_>) -> AppResult<(Vec<HistoryEntry>, u64)> {
        let limit = i64::from(filter.limit.clamp(1, MAX_PAGE));
        let offset = i64::from(filter.offset);
        let origin = filter.origin.map(HistoryOrigin::as_str);
        let editor = filter.from_editor.map(i64::from);
        const WHERE: &str = "(?1 IS NULL OR connection_id = ?1) AND (?2 IS NULL OR origin = ?2) AND (?3 IS NULL OR from_editor = ?3)";
        let total: i64 = self
            .conn()
            .query_row(&format!("SELECT COUNT(*) FROM query_history WHERE {WHERE}"), params![filter.connection_id, origin, editor], |row| row.get(0))
            .map_err(AppError::store)?;
        let mut stmt = self
            .conn()
            .prepare(&format!("SELECT {COLUMNS} FROM query_history WHERE {WHERE} ORDER BY id DESC LIMIT ?4 OFFSET ?5"))
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![filter.connection_id, origin, editor, limit, offset], entry_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok((rows, u64::try_from(total).unwrap_or_default()))
    }

    // WHAT:  The editor's recent statements, each text once, newest run first.
    // WHY:   The editor's History panel is a menu of things to re-run; the same
    //        query run twenty times is one line, not twenty.
    pub fn recent_editor_queries(&self, connection_id: &str, limit: u32) -> AppResult<Vec<HistoryEntry>> {
        let limit = i64::from(limit.clamp(1, MAX_PAGE));
        let mut stmt = self
            .conn()
            .prepare(&format!(
                "SELECT {COLUMNS} FROM query_history WHERE id IN (\
                   SELECT MAX(id) FROM query_history WHERE connection_id = ?1 AND from_editor = 1 GROUP BY TRIM(sql)\
                 ) ORDER BY id DESC LIMIT ?2"
            ))
            .map_err(AppError::store)?;
        let rows = stmt
            .query_map(params![connection_id, limit], entry_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    pub fn clear_history(&self, connection_id: Option<&str>) -> AppResult<u64> {
        let changed = self
            .conn()
            .execute("DELETE FROM query_history WHERE (?1 IS NULL OR connection_id = ?1)", params![connection_id])
            .map_err(AppError::store)?;
        Ok(changed as u64)
    }

    // WHAT:  Deletes log rows older than `days` days, across every connection.
    // HOW:   `executed_at` is written as UTC RFC 3339 with milliseconds, which
    //        sorts as text, so the cutoff is the same format and one comparison.
    pub fn delete_history_older_than(&self, days: u32) -> AppResult<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(i64::from(days))).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let changed = self
            .conn()
            .execute("DELETE FROM query_history WHERE executed_at < ?1", params![cutoff])
            .map_err(AppError::store)?;
        Ok(changed as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry<'a>(conn: &'a str, sql: &'a str, origin: HistoryOrigin, from_editor: bool) -> NewHistoryEntry<'a> {
        NewHistoryEntry {
            connection_id: conn,
            sql,
            status: HistoryStatus::Ok,
            origin,
            error: None,
            elapsed_ms: 5,
            row_count: Some(1),
            statement_type: Some("SELECT"),
            table_name: Some("public.t"),
            intent: Some(StatementIntent::Read),
            user: Some("postgres"),
            from_editor,
        }
    }

    fn seeded() -> Store {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        for (conn, sql) in [("a", "select 1"), ("b", "select 2"), ("a", "select 3")] {
            let origin = if sql == "select 2" { HistoryOrigin::System } else { HistoryOrigin::User };
            store.insert_history(&entry(conn, sql, origin, origin == HistoryOrigin::User)).unwrap_or_else(|e| panic!("{e}"));
        }
        store
    }

    #[test]
    fn history_is_newest_first_and_filterable() {
        let store = seeded();
        let all = store.list_history(None, None, 10).unwrap_or_default();
        assert_eq!(all.len(), 3);
        assert_eq!(all.first().map(|e| e.sql.as_str()), Some("select 3"));
        let only_a = store.list_history(Some("a"), None, 10).unwrap_or_default();
        assert_eq!(only_a.len(), 2);
        let system = store.list_history(None, Some(HistoryOrigin::System), 10).unwrap_or_default();
        assert_eq!(system.len(), 1);
        assert_eq!(store.clear_history(Some("a")).unwrap_or_default(), 2);
        assert_eq!(store.list_history(None, None, 10).unwrap_or_default().len(), 1);
    }

    #[test]
    fn the_details_round_trip() {
        let store = seeded();
        let first = store.list_history(Some("b"), None, 1).unwrap_or_default().into_iter().next();
        let first = first.unwrap_or_else(|| panic!("a row"));
        assert_eq!(first.statement_type.as_deref(), Some("SELECT"));
        assert_eq!(first.table_name.as_deref(), Some("public.t"));
        assert_eq!(first.operation, Some(StatementIntent::Read));
        assert_eq!(first.user.as_deref(), Some("postgres"));
        assert!(!first.from_editor);
    }

    #[test]
    fn a_page_reports_the_filtered_total_and_walks_by_offset() {
        let store = seeded();
        let page = |offset: u32, origin: Option<HistoryOrigin>| {
            store
                .history_page(&HistoryFilter { connection_id: None, origin, from_editor: None, limit: 2, offset })
                .unwrap_or_else(|e| panic!("{e}"))
        };
        let (rows, total) = page(0, None);
        assert_eq!((rows.len(), total), (2, 3));
        let (rows, total) = page(2, None);
        assert_eq!((rows.len(), total), (1, 3));
        assert_eq!(rows.first().map(|e| e.sql.as_str()), Some("select 1"));
        let (rows, total) = page(0, Some(HistoryOrigin::User));
        assert_eq!((rows.len(), total), (2, 2), "the total follows the filter");
    }

    #[test]
    fn recent_editor_queries_are_distinct_and_editor_only() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        for (sql, editor) in [("select 1", true), ("select 2", true), ("select 1 ", true), ("select 9", false)] {
            store.insert_history(&entry("a", sql, HistoryOrigin::User, editor)).unwrap_or_else(|e| panic!("{e}"));
        }
        let recent = store.recent_editor_queries("a", 10).unwrap_or_default();
        let texts: Vec<&str> = recent.iter().map(|e| e.sql.as_str()).collect();
        assert_eq!(texts, ["select 1 ", "select 2"], "newest first, trailing space counts as the same text, app queries left out");
    }

    #[test]
    fn retention_deletes_only_old_rows() {
        let store = seeded();
        store
            .conn()
            .execute("UPDATE query_history SET executed_at = '2000-01-01T00:00:00.000Z' WHERE sql = 'select 1'", [])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.delete_history_older_than(30).unwrap_or_default(), 1);
        assert_eq!(store.list_history(None, None, 10).unwrap_or_default().len(), 2);
        assert_eq!(store.delete_history_older_than(30).unwrap_or_default(), 0);
    }
}
