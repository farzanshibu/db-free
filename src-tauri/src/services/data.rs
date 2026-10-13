// SOT: data-service, table-paging, primary-key-ordering, filtered-count, row-count-strategy, value-truncation, rows-by-key, filter-any-fallback

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::integrations::http::local;
use crate::integrations::qualified_name_for;
use crate::integrations::sql::{honours_filter_mode, order_clause, validate_columns, where_clause, with_filter_mode, with_filter_mode_sync};
use crate::model::{ColumnInfo, ColumnTraits, FilterMode, PageQuery, ResultSet, RowSqlKind, SortRule, StatementResult, TablePage, TableRef, TruncatedCell, Value};
use std::time::Duration;

/// Tables the engine reports as larger than this keep their cheap estimate: a
/// `COUNT(*)` over them would cost more than the page the user asked for.
pub const EXACT_COUNT_BELOW: i64 = 500_000;

/// How long an unfiltered `COUNT(*)` may run before the estimate (or "unknown") will do.
const COUNT_BUDGET: Duration = Duration::from_secs(5);

/// Most keys one `rows_by_key` call looks up.
pub const MAX_KEYS: usize = 500;

// WHAT:  The per-request knobs of a table page that are not part of the query itself.
// WHY:   `PageQuery` is built by every adapter; these ride beside it.
#[derive(Debug, Clone, Copy, Default)]
pub struct PageOptions {
    /// Whether the filter rules combine with AND (default) or OR.
    pub filter_mode: FilterMode,
    /// The caller already knows the total (it is paging through a result it counted).
    pub skip_count: bool,
    /// Cut text, JSON and binary cells longer than this many characters.
    pub truncate_over: Option<usize>,
}

pub async fn table_page(ctx: &SessionCtx, table: &TableRef, query: &PageQuery) -> AppResult<TablePage> {
    table_page_with(ctx, table, query, PageOptions::default()).await
}

// WHAT:  One page of a table with sort + filters, ordered deterministically.
// WHY:   Offset paging without a stable order duplicates/skips rows, so the
//        primary key is appended when the caller gave no sort.
// HOW:   Columns come from the catalog (an empty table still shows headers);
//        sort/filter columns are validated against them before any SQL is built.
//        The total is a `COUNT(*)` whenever that is cheap — filtered, or a table the
//        engine reports under EXACT_COUNT_BELOW rows (or cannot size) — and the
//        engine's estimate beyond, flagged inexact. It is computed for every page,
//        not just the first, unless the caller says it already has it.
// WHERE: src/features/grid/TableTab.tsx (consumer)
pub async fn table_page_with(ctx: &SessionCtx, table: &TableRef, query: &PageQuery, options: PageOptions) -> AppResult<TablePage> {
    let columns = ctx.integration.columns(table).await?;
    if columns.is_empty() {
        return Err(AppError::not_found(format!("Table \"{}\" has no columns or does not exist.", table.name)));
    }
    validate_columns(&columns, &query.sort, &query.filters)?;

    let mut sort = query.sort.clone();
    for pk in columns.iter().filter(|c| c.primary_key) {
        if !sort.iter().any(|s| s.column == pk.name) {
            sort.push(SortRule { column: pk.name.clone(), desc: false });
        }
    }
    let effective = PageQuery { sort, filters: query.filters.clone(), offset: query.offset, limit: query.limit };
    // Rules joined with OR that the adapter cannot say in its own language are applied here,
    // on a window of rows, instead of being answered as if they were joined with AND.
    let scan_any = options.filter_mode == FilterMode::Any && query.filters.len() > 1 && !honours_filter_mode(ctx.connection.engine.family());
    let (mut result, scanned) = if scan_any {
        let (set, total, complete) = scan_for_any(ctx, table, &effective).await?;
        (set, Some((Some(total), complete)))
    } else {
        (with_filter_mode(options.filter_mode, ctx.integration.fetch_page(table, &effective)).await?, None)
    };

    let (total, total_exact) = if options.skip_count {
        (None, false)
    } else if let Some(found) = scanned {
        found
    } else {
        row_total(ctx, table, query, options.filter_mode).await?
    };
    let truncated = match options.truncate_over {
        Some(limit) => truncate_cells(&mut result.rows, limit),
        None => Vec::new(),
    };
    let adapter_traits = match ctx.integration.column_traits(table).await {
        Ok(traits) => traits,
        Err(err) => {
            log::debug!("column traits unavailable for {}: {err}", table.name);
            Vec::new()
        }
    };
    let column_traits = merge_traits(&columns, adapter_traits);

    Ok(TablePage {
        columns,
        rows: result.rows,
        offset: query.offset,
        total,
        total_exact,
        column_traits,
        truncated,
        editable: crate::services::changes::supports_edits(ctx.connection.engine),
    })
}

/// Most rows the any-mode fallback reads before it stops and calls its count a lower bound.
pub const ANY_SCAN_ROWS: u32 = 10_000;

// WHAT:  A page of rows matching ANY of the filter rules, for an adapter that cannot say OR.
// WHY:   A MongoDB or Elasticsearch filter is translated rule by rule into that
//        engine's query language and the translation only knows AND. Rather than
//        answering "all" while the toolbar says "any", the first ANY_SCAN_ROWS rows
//        (in the requested order) are read and filtered here.
// HOW:   Returns the page, how many rows matched in all, and whether the scan saw the
//        whole table (no => the count is only a lower bound and shown as inexact).
async fn scan_for_any(ctx: &SessionCtx, table: &TableRef, query: &PageQuery) -> AppResult<(ResultSet, i64, bool)> {
    let window = PageQuery { sort: query.sort.clone(), filters: Vec::new(), offset: 0, limit: ANY_SCAN_ROWS };
    let mut set = ctx.integration.fetch_page(table, &window).await?;
    let complete = set.rows.len() < ANY_SCAN_ROWS as usize;
    let names: Vec<String> = set.columns.iter().map(|c| c.name.clone()).collect();
    let (rows, total) = any_page(&names, std::mem::take(&mut set.rows), query);
    set.rows = rows;
    Ok((set, total, complete))
}

// WHAT:  The rows that satisfy at least one rule, sliced to the page, and how many matched.
pub fn any_page(names: &[String], rows: Vec<Vec<Value>>, query: &PageQuery) -> (Vec<Vec<Value>>, i64) {
    let matching = with_filter_mode_sync(FilterMode::Any, || local::apply_filters(names, rows, &query.filters));
    let total = i64::try_from(matching.len()).unwrap_or(i64::MAX);
    let page = matching.into_iter().skip(query.offset as usize).take(query.limit as usize).collect();
    (page, total)
}

// WHAT:  How many rows the table (or the filtered view of it) has, and whether the
//        number is exact.
// HOW:   Filters always count exactly (that is what makes "3 of 120" true). Without
//        filters, an SQL engine's estimate decides: past EXACT_COUNT_BELOW it is
//        used as is; below it, or when there is none, a COUNT(*) replaces it — a
//        small table's statistics lag behind its real size. A count that overruns
//        its budget falls back to the estimate. Engines that are not SQL keep
//        whatever their cheap estimate says.
async fn row_total(ctx: &SessionCtx, table: &TableRef, query: &PageQuery, mode: FilterMode) -> AppResult<(Option<i64>, bool)> {
    if !query.filters.is_empty() {
        let n = with_filter_mode(mode, ctx.integration.count(table, &query.filters)).await?;
        return Ok((Some(n), true));
    }
    let caps = ctx.integration.capabilities();
    let estimate = ctx.integration.row_estimate(table).await.unwrap_or(None);
    if !caps.sql {
        return Ok(match estimate {
            Some(n) => (Some(n), caps.exact_estimate),
            None => (None, false),
        });
    }
    if let Some(n) = estimate {
        if caps.exact_estimate {
            return Ok((Some(n), true));
        }
        if n >= EXACT_COUNT_BELOW {
            return Ok((Some(n), false));
        }
    }
    match tokio::time::timeout(COUNT_BUDGET, ctx.integration.count(table, &[])).await {
        Ok(Ok(n)) => Ok((Some(n), true)),
        Ok(Err(err)) => {
            log::debug!("row count failed for {}: {err}", table.name);
            Ok((estimate, false))
        }
        Err(_) => Ok((estimate, false)),
    }
}

// WHAT:  Cuts long text / JSON / binary cells to `limit` characters and lists them.
// WHY:   One 5 MB cell makes a page of 50 rows 5 MB of JSON for the webview to parse
//        and paint. The grid shows the start, and fetches the real value by key when
//        a cell is opened or edited (`rows_by_key`).
// HOW:   JSON becomes the start of its serialised text; binary keeps a whole number
//        of base64 groups. `size` is the real length: characters, or bytes for binary.
pub fn truncate_cells(rows: &mut [Vec<Value>], limit: usize) -> Vec<TruncatedCell> {
    let mut out = Vec::new();
    for (r, row) in rows.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            if let Some(size) = shorten(cell, limit) {
                out.push(TruncatedCell { row: u32::try_from(r).unwrap_or(u32::MAX), col: u32::try_from(c).unwrap_or(u32::MAX), size });
            }
        }
    }
    out
}

fn shorten(cell: &mut Value, limit: usize) -> Option<u64> {
    let (cut, size) = match cell {
        Value::Text(text) | Value::Unsupported(text) => cut_text(text, limit)?,
        Value::Json(json) => {
            let text = json.to_string();
            let (prefix, size) = cut_text(&text, limit)?;
            *cell = Value::Text(prefix);
            return Some(size);
        }
        Value::Bytes(b64) => {
            if b64.len() <= limit {
                return None;
            }
            let padding = b64.chars().rev().take_while(|c| *c == '=').count();
            let bytes = (b64.len() / 4 * 3).saturating_sub(padding);
            (b64.chars().take(limit / 4 * 4).collect(), bytes as u64)
        }
        _ => return None,
    };
    match cell {
        Value::Text(text) | Value::Unsupported(text) | Value::Bytes(text) => *text = cut,
        _ => {}
    }
    Some(size)
}

/// The first `limit` characters of `text` and its full length in characters, or None
/// when it already fits.
fn cut_text(text: &str, limit: usize) -> Option<(String, u64)> {
    let mut boundaries = text.char_indices().map(|(i, _)| i);
    let cut = boundaries.nth(limit)?;
    let size = text.chars().count() as u64;
    Some((text[..cut].to_string(), size))
}

// WHAT:  Whole rows (every column, nothing truncated) for a list of keys.
// WHY:   The grid holds shortened cells and a page that may be stale; the inspector,
//        the editor and a commit's refresh read the rows themselves by primary key.
// HOW:   The statement is the Select that `rows_as_sql` writes, so the key literals
//        are the ones Commit uses. SQL engines only.
pub async fn rows_by_key(ctx: &SessionCtx, table: &TableRef, key_columns: &[String], keys: &[Vec<Value>]) -> AppResult<ResultSet> {
    if key_columns.is_empty() {
        return Err(AppError::invalid_input("This table has no primary key, so rows cannot be looked up by key."));
    }
    if keys.is_empty() {
        return Ok(ResultSet { columns: Vec::new(), rows: Vec::new(), truncated: false });
    }
    if keys.len() > MAX_KEYS {
        return Err(AppError::invalid_input(format!("At most {MAX_KEYS} rows can be fetched at once.")));
    }
    let sql = rows_by_key_sql(ctx, table, key_columns, keys)?;
    let results = ctx.integration.execute(&sql, keys.len().saturating_add(1)).await?;
    results
        .into_iter()
        .find_map(|r| match r {
            StatementResult::Rows { result } => Some(result),
            StatementResult::Affected { .. } => None,
        })
        .ok_or_else(|| AppError::internal("The lookup returned no result set."))
}

/// The SELECT `rows_by_key` runs, also what the history log records.
pub fn rows_by_key_sql(ctx: &SessionCtx, table: &TableRef, key_columns: &[String], keys: &[Vec<Value>]) -> AppResult<String> {
    crate::services::changes::rows_as_sql(ctx.connection.engine, table, RowSqlKind::Select, key_columns, key_columns, keys)
}

// WHAT:  One `ColumnTraits` per column, whatever the adapter could or could not say.
// HOW:   The adapter's entry (defaults, generated, auto-increment, enum labels) wins;
//        a column it did not describe gets an empty one. Enum / SET values the
//        adapter did not supply are read from the column type: MySQL spells them
//        `enum('a','b')`, ClickHouse `Enum8('a' = 1)`, DuckDB `ENUM('a', 'b')`.
pub fn merge_traits(columns: &[ColumnInfo], adapter: Vec<ColumnTraits>) -> Vec<ColumnTraits> {
    columns
        .iter()
        .map(|column| {
            let mut traits = adapter.iter().find(|t| t.name == column.name).cloned().unwrap_or_else(|| ColumnTraits { name: column.name.clone(), ..ColumnTraits::default() });
            if traits.enum_values.is_empty() {
                if let Some((values, multi)) = enum_values_from_type(&column.data_type) {
                    traits.enum_values = values;
                    traits.multi = multi;
                }
            }
            traits
        })
        .collect()
}

// WHAT:  The labels of an ENUM / SET column type, and whether several may be held at once.
// HOW:   Reads the single-quoted strings inside the first parenthesis after the
//        keyword (`''` and `\'` are escaped quotes); `Nullable(Enum8(…))` and
//        `LowCardinality` wrappers are looked through. None for any other type.
pub fn enum_values_from_type(data_type: &str) -> Option<(Vec<String>, bool)> {
    let lower = data_type.to_ascii_lowercase();
    let (keyword, multi) = ["enum8(", "enum16(", "enum(", "set("]
        .into_iter()
        .find_map(|k| lower.find(k).filter(|at| *at == 0 || !lower[..*at].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_')).map(|at| (at + k.len(), k == "set(")))?;
    let mut values = Vec::new();
    let mut chars = data_type[keyword..].chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ')' => break,
            '\'' => {
                let mut label = String::new();
                while let Some(d) = chars.next() {
                    match d {
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                label.push(escaped);
                            }
                        }
                        '\'' if chars.peek() == Some(&'\'') => {
                            chars.next();
                            label.push('\'');
                        }
                        '\'' => break,
                        other => label.push(other),
                    }
                }
                values.push(label);
            }
            _ => {}
        }
    }
    if values.is_empty() {
        None
    } else {
        Some((values, multi))
    }
}

// WHAT:  The SELECT the adapter ran for a page, for the System history log.
pub fn describe_page_sql(ctx: &SessionCtx, table: &TableRef, query: &PageQuery, mode: FilterMode) -> String {
    let engine = ctx.connection.engine;
    with_filter_mode_sync(mode, || {
        format!(
            "SELECT * FROM {}{}{} LIMIT {} OFFSET {}",
            qualified_name_for(engine, table),
            where_clause(engine, &query.filters),
            order_clause(engine, &query.sort),
            query.limit,
            query.offset
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_is_cut_at_a_character_boundary_and_listed() {
        let mut rows = vec![vec![Value::Int(1), Value::Text("héllo wörld".into())], vec![Value::Int(2), Value::Text("ok".into())]];
        let cut = truncate_cells(&mut rows, 5);
        assert_eq!(rows[0][1], Value::Text("héllo".into()));
        assert_eq!(rows[1][1], Value::Text("ok".into()));
        assert_eq!(cut, vec![TruncatedCell { row: 0, col: 1, size: 11 }]);
    }

    #[test]
    fn json_becomes_the_start_of_its_text() {
        let mut rows = vec![vec![Value::Json(serde_json::json!({"key": "a long enough value"}))]];
        let cut = truncate_cells(&mut rows, 8);
        assert_eq!(rows[0][0], Value::Text("{\"key\":\"".into()));
        assert_eq!(cut.len(), 1);
        let mut small = vec![vec![Value::Json(serde_json::json!({"a": 1}))]];
        assert!(truncate_cells(&mut small, 100).is_empty());
        assert!(matches!(small[0][0], Value::Json(_)), "a small document stays a document");
    }

    #[test]
    fn binary_keeps_whole_base64_groups_and_reports_its_byte_size() {
        // 12 bytes -> 16 base64 characters.
        let mut rows = vec![vec![Value::Bytes("AAECAwQFBgcICQoL".into())]];
        let cut = truncate_cells(&mut rows, 10);
        assert_eq!(rows[0][0], Value::Bytes("AAECAwQF".into()));
        assert_eq!(cut, vec![TruncatedCell { row: 0, col: 0, size: 12 }]);
        // Padding is not data.
        let mut padded = vec![vec![Value::Bytes("AAECAw==".into())]];
        assert_eq!(truncate_cells(&mut padded, 4)[0].size, 4);
    }

    #[test]
    fn enum_labels_are_read_from_the_column_type() {
        assert_eq!(enum_values_from_type("enum('a','b','it''s')"), Some((vec!["a".into(), "b".into(), "it's".into()], false)));
        assert_eq!(enum_values_from_type("set('x','y')"), Some((vec!["x".into(), "y".into()], true)));
        assert_eq!(enum_values_from_type("Nullable(Enum8('on' = 1, 'off' = 2))"), Some((vec!["on".into(), "off".into()], false)));
        assert_eq!(enum_values_from_type("ENUM('a', 'b')"), Some((vec!["a".into(), "b".into()], false)));
        assert_eq!(enum_values_from_type("varchar(255)"), None);
        assert_eq!(enum_values_from_type("timestamp"), None);
        assert_eq!(enum_values_from_type("offset(1)"), None, "`set(` inside another word is not a SET type");
    }

    #[test]
    fn every_column_gets_a_traits_entry() {
        let column = |name: &str, ty: &str| ColumnInfo { name: name.into(), data_type: ty.into(), nullable: true, primary_key: false, ordinal: 0 };
        let adapter = vec![ColumnTraits { name: "id".into(), auto_increment: true, has_default: true, ..ColumnTraits::default() }];
        let merged = merge_traits(&[column("id", "int"), column("mood", "enum('a','b')")], adapter);
        assert_eq!(merged.len(), 2);
        assert!(merged[0].auto_increment && merged[0].enum_values.is_empty());
        assert_eq!(merged[1].enum_values, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn any_mode_keeps_rows_matching_at_least_one_rule() {
        use crate::model::{FilterOp, FilterRule};
        let names = vec!["k".to_string(), "n".to_string()];
        let row = |k: &str, n: i64| vec![Value::Text(k.into()), Value::Int(n)];
        let rows = vec![row("alpha", 1), row("beta", 2), row("gamma", 3), row("delta", 4)];
        let rule = |column: &str, op, value: &str| FilterRule { column: column.into(), op, value: value.into() };
        let query = PageQuery { sort: vec![], filters: vec![rule("k", FilterOp::Eq, "alpha"), rule("n", FilterOp::Gte, "3")], offset: 1, limit: 5 };
        let (page, total) = any_page(&names, rows, &query);
        assert_eq!(total, 3, "alpha, gamma and delta match one rule each");
        assert_eq!(page, vec![row("gamma", 3), row("delta", 4)], "the page starts at the offset");
    }

    #[test]
    fn numbers_and_dates_are_never_cut() {
        let mut rows = vec![vec![Value::Int(1), Value::Decimal("123456789012345678901234567890".into()), Value::DateTime("2024-01-01 00:00:00".into())]];
        assert!(truncate_cells(&mut rows, 3).is_empty());
    }
}
