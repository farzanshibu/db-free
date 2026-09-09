// SOT: statement-classifier, destructive-detection, sql-splitter, read-write-intent

// WHAT:  Splits a script into statements and labels each Read / Write / Destructive.
// WHY:   The block needs intent without executing: read-only locks reject Write,
//        and Destructive statements need explicit confirmation from the user.
// HOW:   A small tokenizer skips strings, quoted identifiers, comments and
//        dollar-quotes, so a `;` or `WHERE` inside a literal never fools it.
//        Unknown leading keywords classify as Write — fail closed.
// WHERE: src-tauri/src/guard/mod.rs (consumer)

use crate::model::{StatementIntent, StatementSpan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementKind {
    Read,
    Write,
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedStatement {
    pub text: String,
    pub kind: StatementKind,
    pub reason: Option<String>,
}

pub fn classify(sql: &str) -> Vec<ClassifiedStatement> {
    split_statements(sql)
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .map(|text| {
            let words = top_level_words(&text);
            let (kind, reason) = classify_words(&words);
            ClassifiedStatement { text: text.trim().to_string(), kind, reason }
        })
        .collect()
}

pub fn split_statements(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    scan(&chars).into_iter().map(|(from, to)| chars[from..to].iter().collect()).collect()
}

// WHAT:  Where each statement starts and ends in the caller's own text, with the
//        intent running it would have.
// WHY:   PRD §4.3 — the editor runs one statement out of a script (the gutter ▶,
//        Run at cursor). It has to agree with the block about where a statement
//        begins and ends, so both read this tokenizer rather than the UI growing a
//        second one that drifts from it.
// HOW:   Offsets count UTF-16 code units, because that is how JavaScript indexes a
//        string and how CodeMirror numbers a position; char indices would slide as
//        soon as a literal holds an astral character.
// WHERE: src/features/editor/SqlEditor.tsx (gutter), src-tauri/src/commands/query.rs
pub fn spans(sql: &str) -> Vec<StatementSpan> {
    let chars: Vec<char> = sql.chars().collect();
    let mut utf16: Vec<u32> = Vec::with_capacity(chars.len() + 1);
    let mut total: u32 = 0;
    utf16.push(0);
    for c in &chars {
        total = total.saturating_add(if c.len_utf16() == 2 { 2 } else { 1 });
        utf16.push(total);
    }
    let at = |i: usize| utf16.get(i).copied().unwrap_or(total);
    scan(&chars)
        .into_iter()
        .filter_map(|(mut from, mut to)| {
            while from < to && chars[from].is_whitespace() {
                from += 1;
            }
            while to > from && chars[to - 1].is_whitespace() {
                to -= 1;
            }
            if from == to {
                return None;
            }
            let text: String = chars[from..to].iter().collect();
            let (kind, _) = classify_words(&top_level_words(&text));
            Some(StatementSpan { start: at(from), end: at(to), intent: intent_of(kind) })
        })
        .collect()
}

fn intent_of(kind: StatementKind) -> StatementIntent {
    match kind {
        StatementKind::Read => StatementIntent::Read,
        StatementKind::Write => StatementIntent::Write,
        StatementKind::Destructive => StatementIntent::Destructive,
    }
}

// WHAT:  Half-open char ranges of every `;`-separated chunk, the separator excluded.
// HOW:   Strings, quoted identifiers, line and block comments and dollar-quotes are
//        skipped whole, so a `;` inside one is never a boundary. A chunk is emitted
//        at every separator (an empty one included, which `classify` then drops) and
//        the tail only when it holds something other than whitespace.
fn scan(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let len = chars.len();
    let mut start = 0_usize;
    let mut i = 0_usize;
    while i < len {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '\'' | '"' | '`' => {
                let quote = c;
                i += 1;
                while i < len {
                    if chars[i] == quote {
                        if chars.get(i + 1).copied() == Some(quote) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            '-' if next == Some('-') => {
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                i += 2;
                while i < len {
                    if chars[i] == '*' && chars.get(i + 1).copied() == Some('/') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            '$' => {
                // Postgres dollar quoting: $$...$$ or $tag$...$tag$
                let mut j = i + 1;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if chars.get(j).copied() == Some('$') {
                    let tag = &chars[i..=j];
                    i = j + 1;
                    while i < len {
                        if chars[i..].starts_with(tag) {
                            i += tag.len();
                            break;
                        }
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            ';' => {
                out.push((start, i));
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    if chars[start..].iter().any(|c| !c.is_whitespace()) {
        out.push((start, len));
    }
    out
}

// WHAT:  Uppercased bare words at parenthesis depth 0, with literals and comments removed.
fn top_level_words(statement: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut depth: i32 = 0;
    let chars: Vec<char> = statement.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let flush = |word: &mut String, words: &mut Vec<String>, depth: i32| {
        if !word.is_empty() {
            if depth == 0 {
                words.push(word.to_ascii_uppercase());
            }
            word.clear();
        }
    };
    while i < len {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '\'' | '"' | '`' => {
                flush(&mut word, &mut words, depth);
                let quote = c;
                i += 1;
                while i < len {
                    if chars[i] == quote {
                        if chars.get(i + 1).copied() == Some(quote) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            '-' if next == Some('-') => {
                flush(&mut word, &mut words, depth);
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                flush(&mut word, &mut words, depth);
                i += 2;
                while i < len && !(chars[i] == '*' && chars.get(i + 1).copied() == Some('/')) {
                    i += 1;
                }
                i += 2;
            }
            '(' => {
                flush(&mut word, &mut words, depth);
                depth += 1;
                i += 1;
            }
            ')' => {
                flush(&mut word, &mut words, depth);
                depth -= 1;
                i += 1;
            }
            c if c.is_alphanumeric() || c == '_' => {
                word.push(c);
                i += 1;
            }
            _ => {
                flush(&mut word, &mut words, depth);
                i += 1;
            }
        }
    }
    flush(&mut word, &mut words, depth);
    words
}

const READ_LEADERS: &[&str] = &[
    "SELECT", "EXPLAIN", "SHOW", "VALUES", "TABLE", "DESCRIBE", "DESC", "BEGIN", "START", "COMMIT",
    "END", "ROLLBACK", "SAVEPOINT", "RELEASE", "SET", "RESET", "DISCARD", "LISTEN", "UNLISTEN",
];
const DML_WRITERS: &[&str] = &["INSERT", "UPDATE", "DELETE", "MERGE", "REPLACE", "UPSERT"];

fn classify_words(words: &[String]) -> (StatementKind, Option<String>) {
    let Some(first) = words.first().map(String::as_str) else {
        return (StatementKind::Read, None);
    };
    let has = |kw: &str| words.iter().any(|w| w == kw);
    match first {
        "DROP" => (StatementKind::Destructive, Some("DROP removes the object and its data.".into())),
        "TRUNCATE" => (StatementKind::Destructive, Some("TRUNCATE removes every row.".into())),
        "DELETE" if !has("WHERE") => {
            (StatementKind::Destructive, Some("DELETE without a WHERE clause removes every row.".into()))
        }
        "UPDATE" if !has("WHERE") => {
            (StatementKind::Destructive, Some("UPDATE without a WHERE clause rewrites every row.".into()))
        }
        "ALTER" if has("DROP") => {
            (StatementKind::Destructive, Some("ALTER ... DROP discards a column or constraint.".into()))
        }
        "WITH" => {
            if DML_WRITERS.iter().any(|kw| has(kw)) {
                (StatementKind::Write, None)
            } else {
                (StatementKind::Read, None)
            }
        }
        "PRAGMA" => {
            if words.len() > 2 {
                (StatementKind::Write, None)
            } else {
                (StatementKind::Read, None)
            }
        }
        kw if READ_LEADERS.contains(&kw) => (StatementKind::Read, None),
        _ => (StatementKind::Write, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(sql: &str) -> Vec<StatementKind> {
        classify(sql).into_iter().map(|s| s.kind).collect()
    }

    #[test]
    fn reads_are_reads() {
        assert_eq!(kinds("SELECT 1; select * from t where a = 'x;y'"), vec![StatementKind::Read, StatementKind::Read]);
        assert_eq!(kinds("WITH x AS (SELECT 1) SELECT * FROM x"), vec![StatementKind::Read]);
        assert_eq!(kinds("EXPLAIN ANALYZE SELECT 1"), vec![StatementKind::Read]);
        assert_eq!(kinds("PRAGMA table_info(users)"), vec![StatementKind::Read]);
    }

    #[test]
    fn writes_are_writes() {
        assert_eq!(kinds("INSERT INTO t VALUES (1)"), vec![StatementKind::Write]);
        assert_eq!(kinds("UPDATE t SET a = 1 WHERE id = 3"), vec![StatementKind::Write]);
        assert_eq!(kinds("DELETE FROM t WHERE id = 3"), vec![StatementKind::Write]);
        assert_eq!(kinds("WITH d AS (SELECT 1) DELETE FROM t WHERE id IN (SELECT * FROM d)"), vec![StatementKind::Write]);
        assert_eq!(kinds("CREATE TABLE t (a int)"), vec![StatementKind::Write]);
        assert_eq!(kinds("PRAGMA journal_mode = WAL"), vec![StatementKind::Write]);
        assert_eq!(kinds("frobnicate everything"), vec![StatementKind::Write]);
    }

    #[test]
    fn destructive_needs_confirmation() {
        assert_eq!(kinds("DROP TABLE t"), vec![StatementKind::Destructive]);
        assert_eq!(kinds("TRUNCATE t"), vec![StatementKind::Destructive]);
        assert_eq!(kinds("DELETE FROM t"), vec![StatementKind::Destructive]);
        assert_eq!(kinds("UPDATE t SET a = 1"), vec![StatementKind::Destructive]);
        assert_eq!(kinds("ALTER TABLE t DROP COLUMN a"), vec![StatementKind::Destructive]);
        assert_eq!(kinds("ALTER TABLE t ADD COLUMN a int"), vec![StatementKind::Write]);
    }

    #[test]
    fn where_inside_string_or_subquery_does_not_count() {
        assert_eq!(kinds("DELETE FROM t -- WHERE id = 1"), vec![StatementKind::Destructive]);
        assert_eq!(kinds("UPDATE t SET note = 'WHERE'"), vec![StatementKind::Destructive]);
        assert_eq!(kinds("DELETE FROM t USING (SELECT 1 WHERE true) s"), vec![StatementKind::Destructive]);
    }

    #[test]
    fn splitter_respects_quotes_comments_and_dollar_quotes() {
        let parts = split_statements("select ';'; /* ; */ select 2; -- ;\nselect $$a;b$$; select 4");
        assert_eq!(parts.len(), 4);
        assert!(parts.get(2).is_some_and(|s| s.trim().ends_with("select $$a;b$$")), "comment stays attached: {parts:?}");
    }

    /// What the editor will do with a span: slice its own string by UTF-16 units.
    fn sliced(script: &str, span: &StatementSpan) -> String {
        let units: Vec<u16> = script.encode_utf16().collect();
        units.get(span.start as usize..span.end as usize).map(String::from_utf16_lossy).unwrap_or_default()
    }

    // The editor slices its own text with these offsets, so a span has to cover the
    // statement exactly — no leading blank line, no trailing semicolon.
    #[test]
    fn spans_point_at_each_statement() {
        let script = "select 1;\n\ndelete from t;\n";
        let found = spans(script);
        assert_eq!(found.len(), 2);
        assert_eq!(found.first().map(|s| sliced(script, s)), Some("select 1".to_string()));
        assert_eq!(found.first().map(|s| s.intent), Some(StatementIntent::Read));
        assert_eq!(found.get(1).map(|s| sliced(script, s)), Some("delete from t".to_string()));
        assert_eq!(found.get(1).map(|s| s.intent), Some(StatementIntent::Destructive));
    }

    // A `;` inside a literal is not a boundary, and an astral character counts as
    // the two UTF-16 units JavaScript indexes it by.
    #[test]
    fn spans_count_utf16_units() {
        let script = "select '\u{1F600};' as e; select 2";
        let found = spans(script);
        assert_eq!(found.len(), 2);
        assert_eq!(found.get(1).map(|s| sliced(script, s)), Some("select 2".to_string()));
    }
}
