// SOT: driver-error-position, syntax-error-locate, error-position-utf16

use crate::error::AppError;

// WHAT:  Works out where in a statement the database complained, for engines
//        that say so only inside the message text.
// WHY:   The editor underlines the failing spot. Postgres reports the position
//        as a structured field (the adapter fills it); MySQL, ClickHouse,
//        SQLite, MSSQL, DuckDB and BigQuery spell it as prose ("near 'FORM' at
//        line 2", "failed at position 15", "[1:10]"). Parsing that prose in one
//        place keeps every adapter free of it.
// HOW:   `describe` is called by the query service on every driver error with
//        the exact text that was sent. It returns the same error with
//        `position` set: 1-based, counted in UTF-16 code units, which is how
//        JavaScript and CodeMirror index a string, so the UI adds the
//        statement's start offset and underlines without converting.
// WHERE: src-tauri/src/services/query.rs (execute), src/features/editor/SqlEditor.tsx

/// `err` with its position pinned inside `sql`.
///
/// `prefix_chars` is how many characters were sent in front of `sql` (the
/// engine's "use this schema" statement). A position the adapter reported is
/// relative to everything sent, so the prefix comes off; one read from the
/// message is already relative to `sql`.
pub fn describe(err: AppError, sql: &str, prefix_chars: u32) -> AppError {
    match err {
        AppError::Driver { message, position, hint, detail } => {
            let chars: Vec<char> = sql.chars().collect();
            let in_chars = match position {
                Some(reported) => reported.checked_sub(prefix_chars).filter(|p| *p > 0),
                None => locate(&message, &chars),
            };
            let position = in_chars.and_then(|p| to_utf16(&chars, p));
            AppError::Driver { message, position, hint, detail }
        }
        other => other,
    }
}

/// 1-based character offset the message points at, if it says.
fn locate(message: &str, sql: &[char]) -> Option<u32> {
    if let Some(n) = number_after(message, "at position ") {
        return u32::try_from(n).ok().filter(|p| *p > 0);
    }
    if let Some(found) = bracketed_line_col(message) {
        return line_col(sql, found.0, found.1);
    }
    if let Some(found) = parenthesised_line_col(message) {
        return line_col(sql, found.0, found.1);
    }
    if let Some(found) = caret_line(message) {
        return line_col(sql, found.0, found.1);
    }
    let (snippet, line) = near_snippet(message)?;
    snippet_position(sql, &snippet, line)
}

/// The digits right after `marker`, case-insensitively.
fn number_after(text: &str, marker: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let at = lower.find(marker)? + marker.len();
    let digits: String = text[at..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// BigQuery: `... at [1:10]`.
fn bracketed_line_col(message: &str) -> Option<(usize, usize)> {
    let open = message.rfind(" at [")? + " at [".len();
    let inner = &message[open..];
    let close = inner.find(']')?;
    let (line, col) = inner[..close].split_once(':')?;
    Some((line.trim().parse().ok()?, col.trim().parse().ok()?))
}

/// ClickHouse: `(line 1, col 10)`.
fn parenthesised_line_col(message: &str) -> Option<(usize, usize)> {
    let lower = message.to_ascii_lowercase();
    let open = lower.find("(line ")? + "(line ".len();
    let rest = &lower[open..];
    let line: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let after = &rest[line.len()..];
    let col_at = after.find("col ")? + "col ".len();
    let col: String = after[col_at..].chars().take_while(char::is_ascii_digit).collect();
    Some((line.parse().ok()?, col.parse().ok()?))
}

/// DuckDB / Postgres-style excerpt: a `LINE 2: text` row followed by a row
/// whose only mark is the `^` under the failing column.
fn caret_line(message: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = message.lines().collect();
    for (i, row) in lines.iter().enumerate() {
        let Some(rest) = row.strip_prefix("LINE ") else { continue };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let line: usize = digits.parse().ok()?;
        let gutter = "LINE ".len() + digits.len() + ": ".len();
        let caret = lines.get(i + 1)?;
        let at = caret.chars().position(|c| c == '^')?;
        return Some((line, at.checked_sub(gutter)? + 1));
    }
    None
}

/// MySQL `near '…' at line N`, SQLite `near "…"`, MSSQL `near the keyword '…'`,
/// DuckDB `at or near "…"`: the quoted text and the line when the message has one.
fn near_snippet(message: &str) -> Option<(String, Option<usize>)> {
    let lower = message.to_ascii_lowercase();
    let mut from = near_at(&lower)? + "near ".len();
    for filler in ["the keyword ", "the identifier ", "the "] {
        if lower[from..].starts_with(filler) {
            from += filler.len();
            break;
        }
    }
    let rest = &message[from..];
    let quote = rest.chars().next().filter(|c| matches!(c, '\'' | '"' | '`'))?;
    let body = &rest[quote.len_utf8()..];
    // MySQL closes with `' at line N`; the snippet itself may hold quotes.
    let tail = format!("{quote} at line ");
    if let Some(end) = body.rfind(&tail) {
        let digits: String = body[end + tail.len()..].chars().take_while(char::is_ascii_digit).collect();
        return Some((body[..end].to_string(), digits.parse().ok()));
    }
    let end = body.find(quote)?;
    Some((body[..end].to_string(), None))
}

/// Index of a standalone `near ` (at the start of the text or after whitespace).
fn near_at(lower: &str) -> Option<usize> {
    lower
        .match_indices("near ")
        .map(|(at, _)| at)
        .find(|at| *at == 0 || lower[..*at].ends_with(char::is_whitespace))
}

/// Where `snippet` starts in `sql`, searching from line `line` (1-based) on.
/// An empty snippet means "end of input".
fn snippet_position(sql: &[char], snippet: &str, line: Option<usize>) -> Option<u32> {
    let from = line.map_or(0, |n| line_start(sql, n));
    if snippet.is_empty() {
        return u32::try_from(sql.len() + 1).ok();
    }
    let needle: Vec<char> = snippet.chars().collect();
    let found = find_chars(sql, &needle, from, false)
        .or_else(|| find_chars(sql, &needle, from, true))
        .or_else(|| if from > 0 { find_chars(sql, &needle, 0, true) } else { None });
    match found {
        Some(at) => u32::try_from(at + 1).ok(),
        // A line was named but the snippet was reworded: the line's own start is still near.
        None => line.and_then(|n| u32::try_from(line_start(sql, n) + 1).ok()),
    }
}

fn find_chars(haystack: &[char], needle: &[char], from: usize, ignore_case: bool) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let same = |a: char, b: char| if ignore_case { a.to_lowercase().eq(b.to_lowercase()) } else { a == b };
    (from..=haystack.len() - needle.len()).find(|&start| needle.iter().enumerate().all(|(i, n)| same(haystack[start + i], *n)))
}

/// Index of the first character of 1-based line `line`.
fn line_start(sql: &[char], line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    let mut seen = 1;
    for (i, c) in sql.iter().enumerate() {
        if *c == '\n' {
            seen += 1;
            if seen == line {
                return i + 1;
            }
        }
    }
    sql.len()
}

/// 1-based character offset of (`line`, `col`), both 1-based.
fn line_col(sql: &[char], line: usize, col: usize) -> Option<u32> {
    if line == 0 || col == 0 {
        return None;
    }
    let start = line_start(sql, line);
    u32::try_from((start + col).min(sql.len() + 1)).ok()
}

/// A 1-based character offset as a 1-based UTF-16 offset.
fn to_utf16(sql: &[char], position: u32) -> Option<u32> {
    let chars_before = usize::try_from(position).ok()?.checked_sub(1)?.min(sql.len());
    let units: usize = sql[..chars_before].iter().map(|c| c.len_utf16()).sum();
    u32::try_from(units + 1).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    #[test]
    fn mysql_near_points_at_the_snippet_on_its_line() {
        let message = "You have an error in your SQL syntax; check the manual that corresponds to your MySQL server version for the right syntax to use near 'FORM users' at line 1";
        assert_eq!(locate(message, &chars("SELECT * FORM users")), Some(10));
        let second = "You have an error in your SQL syntax; use near 'FORM users' at line 2";
        assert_eq!(locate(second, &chars("SELECT 1;\nSELECT * FORM users")), Some(20));
    }

    #[test]
    fn mysql_end_of_input_points_past_the_last_character() {
        let message = "You have an error in your SQL syntax; use near '' at line 1";
        assert_eq!(locate(message, &chars("SELECT * FROM")), Some(14));
    }

    #[test]
    fn clickhouse_reports_a_position_directly() {
        let message = "Code: 62. DB::Exception: Syntax error: failed at position 10 (FORM) (line 1, col 10): FORM users. Expected one of: ...";
        assert_eq!(locate(message, &chars("SELECT * FORM users")), Some(10));
    }

    #[test]
    fn clickhouse_line_and_column_work_without_a_position() {
        let message = "DB::Exception: Syntax error (line 2, col 3): x";
        assert_eq!(locate(message, &chars("SELECT 1\n  FORM")), Some(12));
    }

    #[test]
    fn sqlite_near_finds_the_token() {
        assert_eq!(locate("near \"FORM\": syntax error", &chars("SELECT * FORM users")), Some(10));
    }

    #[test]
    fn mssql_near_the_keyword_finds_the_token() {
        assert_eq!(locate("Incorrect syntax near the keyword 'FROM'.", &chars("SELECT a, FROM t")), Some(11));
        assert_eq!(locate("Incorrect syntax near 'FORM'.", &chars("SELECT * FORM t")), Some(10));
    }

    #[test]
    fn bigquery_brackets_are_line_and_column() {
        let message = "Syntax error: Unexpected identifier \"FORM\" at [2:3]";
        assert_eq!(locate(message, &chars("SELECT 1\n  FORM")), Some(12));
    }

    #[test]
    fn duckdb_caret_excerpt_gives_the_column() {
        let message = format!("Parser Error: syntax error at or near \"FORM\"\nLINE 1: SELECT * FORM users\n{}^", " ".repeat(17));
        assert_eq!(locate(&message, &chars("SELECT * FORM users")), Some(10));
    }

    #[test]
    fn a_message_without_a_pointer_has_no_position() {
        assert_eq!(locate("relation \"users\" does not exist", &chars("SELECT * FROM users")), None);
    }

    #[test]
    fn describe_removes_the_schema_prefix_from_a_reported_position() {
        let prefix = "SET search_path = x;\n";
        let prefix_chars = u32::try_from(prefix.chars().count()).unwrap_or(0);
        let err = AppError::driver_with("syntax error", Some(prefix_chars + 10), Some("try FROM".into()), None);
        let AppError::Driver { position, hint, .. } = describe(err, "SELECT * FORM users", prefix_chars) else {
            panic!("a driver error stays a driver error");
        };
        assert_eq!(position, Some(10));
        assert_eq!(hint.as_deref(), Some("try FROM"));
    }

    #[test]
    fn describe_counts_in_utf16_units() {
        // The emoji is two UTF-16 units, so everything after it moves one place.
        let sql = "SELECT '😀' FORM t";
        let err = AppError::driver("You have an error near 'FORM t' at line 1");
        let AppError::Driver { position, .. } = describe(err, sql, 0) else {
            panic!("a driver error stays a driver error");
        };
        assert_eq!(position, Some(13));
    }

    #[test]
    fn describe_leaves_other_errors_alone() {
        let err = AppError::timeout("slow");
        assert_eq!(describe(err.clone(), "SELECT 1", 0), err);
    }

    #[test]
    fn a_position_past_the_prefix_only_is_dropped() {
        let err = AppError::driver_with("boom", Some(3), None, None);
        let AppError::Driver { position, .. } = describe(err, "SELECT 1", 20) else {
            panic!("a driver error stays a driver error");
        };
        assert_eq!(position, None);
    }
}
