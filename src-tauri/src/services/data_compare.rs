// SOT: data-compare-service, row-compare, keyed-merge-join, sync-script, canonical-value-compare

use crate::error::{AppError, AppResult};
use crate::guard::{SessionCtx, MAX_PAGE_LIMIT};
use crate::integrations::{qualified_name_for, quote_ident_for};
use crate::model::{ColumnInfo, CompareDirection, DataCompare, Engine, PageQuery, RowDiff, RowStatus, SortRule, TableRef, Value};
use crate::services::changes::literal;
use std::cmp::Ordering;
use std::collections::BTreeMap;

// ============================================================================
// DATA COMPARE
//
// WHAT:  Compares the rows of two tables (on one connection or two) by a key,
//        and writes the INSERT / UPDATE / DELETE script that makes the target
//        table hold what the source holds.
// WHY:   "Is staging's reference data the same as production's" is otherwise
//        an export, a spreadsheet and a guess.
// HOW:   Each side is paged through `fetch_page` ordered by the key, up to
//        `max_rows` (a side that has more is reported as capped). Rows are
//        then merge-joined on a canonical form of the key rather than on the
//        engines' own order, because two engines (or two collations) can sort
//        the same keys differently and a merge over their raw order would
//        pair the wrong rows. Values compare in the same canonical form, so
//        1, 1.0 and DECIMAL '1.00' are equal across engines. `diff_rows` and
//        `sync_script` are pure and unit-tested; nothing here executes: the
//        script opens in a query tab and runs through the guard.
// WHERE: src-tauri/src/commands/compare.rs (caller), src-tauri/src/model/compare.rs,
//        src/features/compare/DataCompareTab.tsx
// ============================================================================

/// Differing rows sent to the UI; counts and the script still cover every row.
pub const MAX_REPORTED_ROWS: usize = 5_000;
/// Rows read per side when the caller names no cap.
pub const DEFAULT_MAX_ROWS: u32 = 100_000;

pub struct CompareOptions {
    /// Empty = the primary key (left's, else right's).
    pub key_columns: Vec<String>,
    pub max_rows: usize,
    pub direction: CompareDirection,
    pub include_script: bool,
}

// ---- canonical values (pure) -------------------------------------------------

// WHAT:  A value reduced to what it means, for equality and ordering across
//        engines: numbers by value (with their normalised text breaking ties
//        so i64s past f64 precision stay distinct), everything else by text.
#[derive(Debug, Clone)]
enum Canon {
    Null,
    Num { value: f64, text: String },
    Text(String),
}

impl Canon {
    fn rank(&self) -> u8 {
        match self {
            Canon::Null => 0,
            Canon::Num { .. } => 1,
            Canon::Text(_) => 2,
        }
    }
}

impl Ord for Canon {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Canon::Num { value: a, text: at }, Canon::Num { value: b, text: bt }) => a.total_cmp(b).then_with(|| at.cmp(bt)),
            (Canon::Text(a), Canon::Text(b)) => a.cmp(b),
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

impl PartialOrd for Canon {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Canon {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Canon {}

// WHAT:  "1.500" → "1.5", "-0.0" → "0", "+7" → "7". None when not a number.
fn normalize_number(raw: &str) -> Option<(f64, String)> {
    let trimmed = raw.trim().trim_start_matches('+');
    let value: f64 = trimmed.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let mut text = trimmed.to_string();
    if text.contains('.') && !text.contains(['e', 'E']) {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if value == 0.0 {
        text = "0".to_string();
    }
    Some((value, text))
}

fn canon(value: &Value) -> Canon {
    match value {
        Value::Null => Canon::Null,
        Value::Bool(b) => Canon::Num { value: if *b { 1.0 } else { 0.0 }, text: if *b { "1" } else { "0" }.to_string() },
        #[allow(clippy::cast_precision_loss)] // ordering only; `text` keeps exact equality
        Value::Int(i) => Canon::Num { value: *i as f64, text: i.to_string() },
        Value::Float(f) => match normalize_number(&f.to_string()) {
            Some((value, text)) => Canon::Num { value, text },
            None => Canon::Text(f.to_string()),
        },
        Value::Decimal(s) => match normalize_number(s) {
            Some((value, text)) => Canon::Num { value, text },
            None => Canon::Text(s.clone()),
        },
        Value::Text(s) | Value::DateTime(s) | Value::Unsupported(s) => Canon::Text(s.clone()),
        Value::Json(j) => Canon::Text(j.to_string()),
        Value::Bytes(b) => Canon::Text(format!("\u{0}bytes:{b}")),
    }
}

/// True when two cells hold the same value by the canonical rules above.
pub fn same_value(a: &Value, b: &Value) -> bool {
    canon(a) == canon(b)
}

// ---- row diff (pure) -----------------------------------------------------------

// WHAT:  One row cut down to the compared columns, with its key split out.
#[derive(Debug, Clone)]
pub struct KeyedRow {
    pub key: Vec<Value>,
    pub values: Vec<Value>,
}

#[derive(Debug, Default)]
pub struct RowComparison {
    pub rows: Vec<RowDiff>,
    pub only_left: u64,
    pub only_right: u64,
    pub different: u64,
    pub identical: u64,
    /// Rows whose key repeated an earlier row's on the same side (the later one is ignored).
    pub duplicate_keys: u64,
}

fn index(rows: Vec<KeyedRow>, duplicates: &mut u64) -> BTreeMap<Vec<Canon>, KeyedRow> {
    let mut map = BTreeMap::new();
    for row in rows {
        let key: Vec<Canon> = row.key.iter().map(canon).collect();
        if map.contains_key(&key) {
            *duplicates += 1;
        } else {
            map.insert(key, row);
        }
    }
    map
}

// WHAT:  Merge-joins both sides in canonical key order and classifies rows.
// HOW:   `columns` names the compared columns (`values` order); a Different
//        row lists the ones whose values differ.
pub fn diff_rows(left: Vec<KeyedRow>, right: Vec<KeyedRow>, columns: &[String]) -> RowComparison {
    let mut out = RowComparison::default();
    let left = index(left, &mut out.duplicate_keys);
    let right = index(right, &mut out.duplicate_keys);
    let mut l = left.into_iter().peekable();
    let mut r = right.into_iter().peekable();
    loop {
        let order = match (l.peek(), r.peek()) {
            (Some((lk, _)), Some((rk, _))) => lk.cmp(rk),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => break,
        };
        match order {
            Ordering::Less => {
                if let Some((_, row)) = l.next() {
                    out.only_left += 1;
                    out.rows.push(RowDiff { status: RowStatus::OnlyLeft, key: row.key, left: Some(row.values), right: None, changed: Vec::new() });
                }
            }
            Ordering::Greater => {
                if let Some((_, row)) = r.next() {
                    out.only_right += 1;
                    out.rows.push(RowDiff { status: RowStatus::OnlyRight, key: row.key, left: None, right: Some(row.values), changed: Vec::new() });
                }
            }
            Ordering::Equal => {
                if let (Some((_, lr)), Some((_, rr))) = (l.next(), r.next()) {
                    let changed: Vec<String> = columns
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| match (lr.values.get(*i), rr.values.get(*i)) {
                            (Some(a), Some(b)) => !same_value(a, b),
                            (a, b) => a.is_some() != b.is_some(),
                        })
                        .map(|(_, name)| name.clone())
                        .collect();
                    if changed.is_empty() {
                        out.identical += 1;
                    } else {
                        out.different += 1;
                        out.rows.push(RowDiff { status: RowStatus::Different, key: lr.key, left: Some(lr.values), right: Some(rr.values), changed });
                    }
                }
            }
        }
    }
    out
}

// ---- sync script (pure) --------------------------------------------------------

// WHAT:  Where the script writes and in which names: the target table, its
//        spelling of every compared column, and which of them form the key.
pub struct SyncTarget<'a> {
    pub engine: Engine,
    pub table: &'a TableRef,
    /// Target-side column names, in `DataCompare::columns` order.
    pub columns: &'a [String],
    /// Indices into `columns` of the key columns.
    pub key: &'a [usize],
}

fn predicate(target: &SyncTarget<'_>, values: &[Value]) -> String {
    target
        .key
        .iter()
        .filter_map(|&i| {
            let col = quote_ident_for(target.engine, target.columns.get(i)?);
            Some(match values.get(i) {
                None | Some(Value::Null) => format!("{col} IS NULL"),
                Some(v) => format!("{col} = {}", literal(target.engine, v)),
            })
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

// WHAT:  DELETE, then UPDATE, then INSERT — deleting first frees unique
//        values an insert may need.
// HOW:   Direction decides which side's values are written: the source's
//        values go into the target; rows only in the target are deleted.
pub fn sync_script(target: &SyncTarget<'_>, rows: &[RowDiff], direction: CompareDirection) -> String {
    let table = qualified_name_for(target.engine, target.table);
    let (mut deletes, mut updates, mut inserts) = (Vec::new(), Vec::new(), Vec::new());
    for row in rows {
        let (source, target_values) = if direction.left_is_source() { (&row.left, &row.right) } else { (&row.right, &row.left) };
        match (source, target_values) {
            (None, Some(existing)) => deletes.push(format!("DELETE FROM {table} WHERE {};", predicate(target, existing))),
            (Some(values), None) => {
                let cols = target.columns.iter().map(|c| quote_ident_for(target.engine, c)).collect::<Vec<_>>().join(", ");
                let vals = values.iter().map(|v| literal(target.engine, v)).collect::<Vec<_>>().join(", ");
                inserts.push(format!("INSERT INTO {table} ({cols}) VALUES ({vals});"));
            }
            (Some(values), Some(existing)) => {
                let sets = target
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !target.key.contains(i))
                    .filter(|(i, _)| match (values.get(*i), existing.get(*i)) {
                        (Some(a), Some(b)) => !same_value(a, b),
                        _ => false,
                    })
                    .filter_map(|(i, c)| Some(format!("{} = {}", quote_ident_for(target.engine, c), literal(target.engine, values.get(i)?))))
                    .collect::<Vec<_>>();
                if !sets.is_empty() {
                    updates.push(format!("UPDATE {table} SET {} WHERE {};", sets.join(", "), predicate(target, existing)));
                }
            }
            (None, None) => {}
        }
    }
    let mut lines = vec![
        format!("-- Sync generated by DB Free data compare: {} delete(s), {} update(s), {} insert(s).", deletes.len(), updates.len(), inserts.len()),
        "-- Review before running; the guard asks before DELETE and UPDATE statements.".to_string(),
    ];
    if deletes.is_empty() && updates.is_empty() && inserts.is_empty() {
        lines.push("-- The tables already match: nothing to sync.".to_string());
    }
    lines.extend(deletes);
    lines.extend(updates);
    lines.extend(inserts);
    let mut script = lines.join("\n");
    script.push('\n');
    script
}

// ---- orchestration ---------------------------------------------------------------

fn find_column<'a>(columns: &'a [ColumnInfo], name: &str) -> Option<&'a ColumnInfo> {
    columns.iter().find(|c| c.name == name).or_else(|| columns.iter().find(|c| c.name.to_lowercase() == name.to_lowercase()))
}

// WHAT:  The key, as (left name, right name) pairs: the caller's choice, or
//        the left table's primary key, or the right one's.
fn resolve_keys(left: &[ColumnInfo], right: &[ColumnInfo], requested: &[String]) -> AppResult<Vec<(String, String)>> {
    let pk = |cols: &[ColumnInfo]| {
        let mut keys: Vec<&ColumnInfo> = cols.iter().filter(|c| c.primary_key).collect();
        keys.sort_by_key(|c| c.ordinal);
        keys.into_iter().map(|c| c.name.clone()).collect::<Vec<_>>()
    };
    let names = if !requested.is_empty() {
        requested.to_vec()
    } else if !pk(left).is_empty() {
        pk(left)
    } else {
        pk(right)
    };
    if names.is_empty() {
        return Err(AppError::invalid_input("Neither table has a primary key: choose the key columns to match rows by."));
    }
    names
        .iter()
        .map(|name| match (find_column(left, name), find_column(right, name)) {
            (Some(l), Some(r)) => Ok((l.name.clone(), r.name.clone())),
            _ => Err(AppError::invalid_input(format!("Key column \"{name}\" must exist in both tables."))),
        })
        .collect()
}

// WHAT:  Every row of one side (up to `max_rows`), ordered by the key, each cut
//        to `wanted` columns by name. Returns the rows and whether it capped.
async fn read_side(ctx: &SessionCtx, table: &TableRef, sort: &[String], wanted: &[String], max_rows: usize) -> AppResult<(Vec<Vec<Value>>, bool)> {
    let sort: Vec<SortRule> = sort.iter().map(|c| SortRule { column: c.clone(), desc: false }).collect();
    let mut rows: Vec<Vec<Value>> = Vec::new();
    let mut offset: u64 = 0;
    // One row past the cap tells "exactly max_rows" apart from "more than that".
    let ceiling = max_rows.saturating_add(1);
    loop {
        let room = ceiling.saturating_sub(rows.len());
        let limit = u32::try_from(room).unwrap_or(MAX_PAGE_LIMIT).min(MAX_PAGE_LIMIT);
        if limit == 0 {
            break;
        }
        let page = ctx.integration.fetch_page(table, &PageQuery { sort: sort.clone(), filters: Vec::new(), offset, limit }).await?;
        let positions: Vec<Option<usize>> = wanted
            .iter()
            .map(|w| page.columns.iter().position(|c| c.name == *w).or_else(|| page.columns.iter().position(|c| c.name.to_lowercase() == w.to_lowercase())))
            .collect();
        let got = page.rows.len();
        for row in page.rows {
            rows.push(positions.iter().map(|p| p.and_then(|i| row.get(i).cloned()).unwrap_or(Value::Null)).collect());
        }
        offset += got as u64;
        if got < limit as usize {
            break;
        }
    }
    let capped = rows.len() > max_rows;
    rows.truncate(max_rows);
    Ok((rows, capped))
}

fn keyed(rows: Vec<Vec<Value>>, key: &[usize]) -> Vec<KeyedRow> {
    rows.into_iter()
        .map(|values| KeyedRow { key: key.iter().map(|&i| values.get(i).cloned().unwrap_or(Value::Null)).collect(), values })
        .collect()
}

pub async fn compare(left: &SessionCtx, left_table: &TableRef, right: &SessionCtx, right_table: &TableRef, options: &CompareOptions) -> AppResult<DataCompare> {
    let left_columns = left.integration.columns(left_table).await?;
    let right_columns = right.integration.columns(right_table).await?;
    if left_columns.is_empty() || right_columns.is_empty() {
        return Err(AppError::not_found("One of the tables has no columns or does not exist."));
    }
    let keys = resolve_keys(&left_columns, &right_columns, &options.key_columns)?;

    // Compared columns: every left column the right table also has, in left order.
    let mut ordered: Vec<&ColumnInfo> = left_columns.iter().collect();
    ordered.sort_by_key(|c| c.ordinal);
    let mut pairs: Vec<(ColumnInfo, String)> = Vec::new();
    let mut left_only = Vec::new();
    for c in ordered {
        match find_column(&right_columns, &c.name) {
            Some(r) => pairs.push((c.clone(), r.name.clone())),
            None => left_only.push(c.name.clone()),
        }
    }
    let right_only: Vec<String> = right_columns.iter().filter(|r| !pairs.iter().any(|(_, rn)| *rn == r.name)).map(|r| r.name.clone()).collect();
    let left_names: Vec<String> = pairs.iter().map(|(c, _)| c.name.clone()).collect();
    let right_names: Vec<String> = pairs.iter().map(|(_, r)| r.clone()).collect();
    let key_index: Vec<usize> = keys.iter().filter_map(|(l, _)| left_names.iter().position(|n| n == l)).collect();
    let left_keys: Vec<String> = keys.iter().map(|(l, _)| l.clone()).collect();
    let right_keys: Vec<String> = keys.iter().map(|(_, r)| r.clone()).collect();

    let (left_rows, left_capped) = read_side(left, left_table, &left_keys, &left_names, options.max_rows).await?;
    let (right_rows, right_capped) = read_side(right, right_table, &right_keys, &right_names, options.max_rows).await?;
    let (left_count, right_count) = (left_rows.len() as u64, right_rows.len() as u64);
    let mut comparison = diff_rows(keyed(left_rows, &key_index), keyed(right_rows, &key_index), &left_names);

    let mut notes = Vec::new();
    if left_capped || right_capped {
        notes.push(format!(
            "Only the first {} rows of {} were read (ordered by the key); rows past the cap are not compared.",
            options.max_rows,
            match (left_capped, right_capped) {
                (true, true) => "each side",
                (true, false) => "the left side",
                _ => "the right side",
            }
        ));
    }
    if comparison.duplicate_keys > 0 {
        notes.push(format!("{} row(s) repeat a key already seen on the same side and were skipped: the key does not identify rows uniquely.", comparison.duplicate_keys));
    }
    if !left_only.is_empty() || !right_only.is_empty() {
        notes.push("Columns present on one side only are not compared or synced.".to_string());
    }

    let (target_ctx, target_table, target_names) = if options.direction.left_is_source() { (right, right_table, &right_names) } else { (left, left_table, &left_names) };
    let script = if !options.include_script {
        None
    } else if target_ctx.integration.capabilities().sql {
        let target = SyncTarget { engine: target_ctx.connection.engine, table: target_table, columns: target_names, key: &key_index };
        Some(sync_script(&target, &comparison.rows, options.direction))
    } else {
        notes.push(format!("{} does not take SQL, so no sync script was written.", target_ctx.connection.engine.label()));
        None
    };

    let rows_truncated = comparison.rows.len() > MAX_REPORTED_ROWS;
    comparison.rows.truncate(MAX_REPORTED_ROWS);
    Ok(DataCompare {
        direction: options.direction,
        columns: pairs.into_iter().map(|(c, _)| c).collect(),
        key_columns: left_keys,
        left_only_columns: left_only,
        right_only_columns: right_only,
        rows: comparison.rows,
        rows_truncated,
        only_left: comparison.only_left,
        only_right: comparison.only_right,
        different: comparison.different,
        identical: comparison.identical,
        left_rows: left_count,
        right_rows: right_count,
        left_capped,
        right_capped,
        script,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: i64, name: &str, score: Value) -> KeyedRow {
        KeyedRow { key: vec![Value::Int(key)], values: vec![Value::Int(key), Value::Text(name.into()), score] }
    }

    fn cols() -> Vec<String> {
        vec!["id".into(), "name".into(), "score".into()]
    }

    #[test]
    fn canonical_values_compare_across_engines() {
        assert!(same_value(&Value::Int(1), &Value::Decimal("1.00".into())));
        assert!(same_value(&Value::Float(2.5), &Value::Decimal("2.50".into())));
        assert!(same_value(&Value::Bool(true), &Value::Int(1)));
        assert!(!same_value(&Value::Null, &Value::Text(String::new())));
        assert!(!same_value(&Value::Int(9_007_199_254_740_993), &Value::Int(9_007_199_254_740_992)), "past f64 precision");
        assert!(!same_value(&Value::Text("a".into()), &Value::Text("A".into())));
    }

    #[test]
    fn classifies_rows() {
        let left = vec![row(1, "a", Value::Int(10)), row(2, "b", Value::Int(20)), row(3, "c", Value::Null)];
        let right = vec![row(2, "b", Value::Decimal("20.0".into())), row(3, "C", Value::Null), row(4, "d", Value::Int(1))];
        let out = diff_rows(left, right, &cols());
        assert_eq!((out.only_left, out.only_right, out.different, out.identical), (1, 1, 1, 1));
        let diff = out.rows.iter().find(|r| r.status == RowStatus::Different).unwrap_or_else(|| panic!("no diff"));
        assert_eq!(diff.changed, vec!["name".to_string()]);
        assert_eq!(out.rows.iter().map(|r| r.status).collect::<Vec<_>>(), vec![RowStatus::OnlyLeft, RowStatus::Different, RowStatus::OnlyRight], "key order");
    }

    #[test]
    fn numeric_keys_order_by_value_not_text() {
        let left = vec![row(10, "x", Value::Null), row(9, "y", Value::Null)];
        let out = diff_rows(left, Vec::new(), &cols());
        assert_eq!(out.rows.first().map(|r| r.key.clone()), Some(vec![Value::Int(9)]));
    }

    #[test]
    fn keys_match_across_representations() {
        let left = vec![KeyedRow { key: vec![Value::Int(7)], values: vec![Value::Int(7)] }];
        let right = vec![KeyedRow { key: vec![Value::Decimal("7".into())], values: vec![Value::Decimal("7".into())] }];
        let out = diff_rows(left, right, &["id".to_string()]);
        assert_eq!(out.identical, 1);
    }

    #[test]
    fn duplicate_keys_are_counted() {
        let left = vec![row(1, "a", Value::Null), row(1, "b", Value::Null)];
        let out = diff_rows(left, vec![row(1, "a", Value::Null)], &cols());
        assert_eq!((out.duplicate_keys, out.identical), (1, 1));
    }

    #[test]
    fn sync_script_makes_target_match_source() {
        let left = vec![row(1, "a", Value::Int(10)), row(2, "b'b", Value::Int(20))];
        let right = vec![row(2, "old", Value::Int(20)), row(3, "gone", Value::Null)];
        let out = diff_rows(left, right, &cols());
        let table = TableRef { schema: Some("app".into()), name: "people".into() };
        let names = cols();
        let target = SyncTarget { engine: Engine::Postgres, table: &table, columns: &names, key: &[0] };
        let s = sync_script(&target, &out.rows, CompareDirection::LeftToRight);
        assert!(s.contains("DELETE FROM \"app\".\"people\" WHERE \"id\" = 3;"), "{s}");
        assert!(s.contains("UPDATE \"app\".\"people\" SET \"name\" = 'b''b' WHERE \"id\" = 2;"), "{s}");
        assert!(s.contains("INSERT INTO \"app\".\"people\" (\"id\", \"name\", \"score\") VALUES (1, 'a', 10);"), "{s}");
        let (d, u, i) = (s.find("DELETE").unwrap_or(0), s.find("UPDATE \"").unwrap_or(0), s.find("INSERT").unwrap_or(0));
        assert!(d < u && u < i, "{s}");

        // Reversed: the left side is the target now.
        let mysql = SyncTarget { engine: Engine::Mysql, table: &table, columns: &names, key: &[0] };
        let back = sync_script(&mysql, &out.rows, CompareDirection::RightToLeft);
        assert!(back.contains("DELETE FROM `app`.`people` WHERE `id` = 1;"), "{back}");
        assert!(back.contains("INSERT INTO `app`.`people` (`id`, `name`, `score`) VALUES (3, 'gone', NULL);"), "{back}");
        assert!(back.contains("UPDATE `app`.`people` SET `name` = 'old' WHERE `id` = 2;"), "{back}");
    }

    #[test]
    fn empty_diff_says_so() {
        let table = TableRef { schema: None, name: "t".into() };
        let names = cols();
        let target = SyncTarget { engine: Engine::Sqlite, table: &table, columns: &names, key: &[0] };
        assert!(sync_script(&target, &[], CompareDirection::LeftToRight).contains("nothing to sync"));
    }

    #[test]
    fn key_resolution() {
        let c = |n: &str, pk: bool, o: u32| ColumnInfo { name: n.into(), data_type: "int".into(), nullable: false, primary_key: pk, ordinal: o };
        let left = vec![c("id", true, 1), c("v", false, 2)];
        let right = vec![c("ID", false, 1), c("v", false, 2)];
        assert_eq!(resolve_keys(&left, &right, &[]).ok(), Some(vec![("id".to_string(), "ID".to_string())]));
        assert!(resolve_keys(&left, &right, &["nope".to_string()]).is_err());
        let bare = vec![c("a", false, 1)];
        assert!(resolve_keys(&bare, &bare, &[]).is_err());
    }

    // ---- end to end over two SQLite files ------------------------------------

    async fn sqlite_ctx(setup: &str) -> SessionCtx {
        use crate::model::{ConnectionInput, ConnectionSummary, Environment, ResolvedConnection, SslMode};
        let dir = std::env::temp_dir().join(format!("db-free-data-compare-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let path = dir.join("d.db").to_string_lossy().into_owned();
        let input = ConnectionInput {
            name: "side".into(),
            engine: Engine::Sqlite,
            environment: Environment::Local,
            read_only: false,
            host: None,
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: Some(path),
            ssl_mode: SslMode::Disable,
        };
        let summary = ConnectionSummary::draft(&input, false);
        let resolved = ResolvedConnection { summary: summary.clone(), secret: None };
        let integration = crate::integrations::connect(&resolved).await.unwrap_or_else(|e| panic!("{e}"));
        integration.execute(setup, 10).await.unwrap_or_else(|e| panic!("{e}"));
        SessionCtx { connection: summary, integration, started: std::time::Instant::now() }
    }

    #[tokio::test]
    async fn compares_and_syncs_two_sqlite_tables() {
        // More than one page (2,500 rows), so paging and the key order are
        // exercised; one set-based INSERT keeps the fixture fast.
        let seed = concat!(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, score REAL);",
            "INSERT INTO t WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2500) ",
            "SELECT i, 'n' || i, i + 0.5 FROM n;",
        )
        .to_string();
        let left = sqlite_ctx(&seed).await;
        let right = sqlite_ctx(&(seed.clone() + "DELETE FROM t WHERE id = 7; UPDATE t SET name = 'changed' WHERE id = 1500; INSERT INTO t VALUES (9000, 'extra', NULL);")).await;
        let table = TableRef { schema: None, name: "t".into() };
        let options = CompareOptions { key_columns: Vec::new(), max_rows: 100_000, direction: CompareDirection::LeftToRight, include_script: true };
        let out = compare(&left, &table, &right, &table, &options).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((out.only_left, out.only_right, out.different, out.identical), (1, 1, 1, 2_498));
        assert_eq!(out.key_columns, vec!["id".to_string()]);
        assert!(!out.left_capped && !out.right_capped);

        let script = out.script.unwrap_or_default();
        right.integration.execute(&script, 10).await.unwrap_or_else(|e| panic!("{e}\n{script}"));
        let again = compare(&left, &table, &right, &table, &options).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((again.only_left, again.only_right, again.different), (0, 0, 0), "the sync script converges");

        let capped = CompareOptions { max_rows: 1_000, ..options };
        let partial = compare(&left, &table, &right, &table, &capped).await.unwrap_or_else(|e| panic!("{e}"));
        assert!(partial.left_capped && partial.right_capped);
        assert_eq!(partial.left_rows, 1_000);
    }
}
