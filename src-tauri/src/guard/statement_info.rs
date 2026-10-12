// SOT: statement-info, statement-type-label, statement-table-name, history-statement-details

// WHAT:  The two facts the query log keeps about a statement besides its text:
//        what kind it is (SELECT, UPDATE, CREATE…) and which table it touches.
// WHY:   The log viewer filters and opens by them. They are read once, here,
//        when the block logs the run — not re-derived by every reader of the log.
// HOW:   A word scanner that skips comments and string literals, so a table
//        name in a comment or a `FROM` inside a literal never counts. The kind
//        is the first word (a leading WITH looks through the CTEs to the real
//        verb); the table is the first identifier after the verb's own marker
//        (FROM, INTO, UPDATE, TABLE…). Non-SQL commands degrade sensibly: a
//        Mongo command document is its first key (`{"find": "users"}` -> FIND
//        on `users`), a Redis command is its first word.
// WHERE: src-tauri/src/guard/mod.rs (step 10), src-tauri/src/store/history.rs

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatementInfo {
    /// Upper-cased verb: `SELECT`, `INSERT`, `CREATE`… Empty when there is none.
    pub kind: String,
    /// `schema.table` or `table`, unquoted, when the statement names one.
    pub table: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    /// A quoted identifier, already unquoted.
    Quoted(String),
    Dot,
    Open,
    Close,
    /// Any other punctuation, one character.
    Other(char),
}

pub fn describe(sql: &str) -> StatementInfo {
    let trimmed = sql.trim_start();
    if trimmed.starts_with('{') {
        return describe_document(trimmed);
    }
    let tokens = tokenize(sql);
    let Some(Token::Word(first)) = tokens.first() else {
        return StatementInfo::default();
    };
    let with = first.eq_ignore_ascii_case("WITH");
    let kind = if with { verb_after_ctes(&tokens).unwrap_or_else(|| first.to_ascii_uppercase()) } else { first.to_ascii_uppercase() };
    let table = table_of(&kind, &tokens, with);
    StatementInfo { kind, table }
}

// WHAT:  `{"find": "users", ...}` -> FIND on `users`.
// HOW:   Reads the first key and, when it is a string, its value by hand: the
//        command's verb is whichever key comes first in the text, and a parsed
//        map does not promise to keep that order.
fn describe_document(text: &str) -> StatementInfo {
    let chars: Vec<char> = text.chars().collect();
    let Some((key, after_key)) = json_string(&chars, skip_spaces(&chars, 1)) else {
        return StatementInfo { kind: "COMMAND".to_string(), table: None };
    };
    let colon = skip_spaces(&chars, after_key);
    let table = if chars.get(colon).copied() == Some(':') {
        json_string(&chars, skip_spaces(&chars, colon + 1)).map(|(value, _)| value).filter(|t| !t.is_empty())
    } else {
        None
    };
    StatementInfo { kind: key.to_ascii_uppercase(), table }
}

fn skip_spaces(chars: &[char], mut at: usize) -> usize {
    while chars.get(at).is_some_and(|c| c.is_whitespace()) {
        at += 1;
    }
    at
}

/// The JSON string starting at `at` (a `"`), and the index just past it.
fn json_string(chars: &[char], at: usize) -> Option<(String, usize)> {
    if chars.get(at).copied() != Some('"') {
        return None;
    }
    let mut out = String::new();
    let mut i = at + 1;
    while i < chars.len() {
        match chars[i] {
            '"' => return Some((out, i + 1)),
            '\\' => {
                i += 1;
                if let Some(c) = chars.get(i) {
                    out.push(*c);
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    None
}

fn tokenize(sql: &str) -> Vec<Token> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            c if c.is_whitespace() => i += 1,
            '-' if next == Some('-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1).copied() == Some('/')) {
                    i += 1;
                }
                i += 2;
            }
            '\'' => i = skip_quoted(&chars, i, '\''),
            '"' | '`' => {
                let end = skip_quoted(&chars, i, c);
                let inner_end = if end > i + 1 && chars.get(end - 1).copied() == Some(c) { end - 1 } else { end };
                out.push(Token::Quoted(chars[(i + 1).min(inner_end)..inner_end].iter().collect()));
                i = end;
            }
            '[' => {
                // T-SQL bracket identifier.
                let mut j = i + 1;
                while j < chars.len() && chars[j] != ']' {
                    j += 1;
                }
                out.push(Token::Quoted(chars[i + 1..j.min(chars.len())].iter().collect()));
                i = j + 1;
            }
            '.' => {
                out.push(Token::Dot);
                i += 1;
            }
            '(' => {
                out.push(Token::Open);
                i += 1;
            }
            ')' => {
                out.push(Token::Close);
                i += 1;
            }
            c if c.is_alphanumeric() || c == '_' || c == '$' || c == '@' || c == '#' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '$' | '@' | '#')) {
                    i += 1;
                }
                out.push(Token::Word(chars[start..i].iter().collect()));
            }
            other => {
                out.push(Token::Other(other));
                i += 1;
            }
        }
    }
    out
}

/// Index just past the closing quote; a doubled quote is an escaped one.
fn skip_quoted(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == quote {
            if chars.get(i + 1).copied() == Some(quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

fn is_word(token: Option<&Token>, keyword: &str) -> bool {
    matches!(token, Some(Token::Word(w)) if w.eq_ignore_ascii_case(keyword))
}

/// The verb of `WITH a AS (...), b AS (...) <verb> ...`: the first word at
/// depth 0 after the CTE list that is not part of it.
fn verb_after_ctes(tokens: &[Token]) -> Option<String> {
    let mut depth = 0_i32;
    let mut after_close = false;
    for token in tokens.iter().skip(1) {
        match token {
            Token::Open => depth += 1,
            Token::Close => {
                depth -= 1;
                after_close = depth == 0;
            }
            Token::Word(w) if depth == 0 && after_close => {
                let upper = w.to_ascii_uppercase();
                if matches!(upper.as_str(), "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "REPLACE") {
                    return Some(upper);
                }
            }
            Token::Other(',') if depth == 0 => after_close = false,
            _ => {}
        }
    }
    None
}

fn table_of(kind: &str, tokens: &[Token], through_ctes: bool) -> Option<String> {
    // Where the name starts: after the verb's own marker word.
    let start = match kind {
        // A CTE query reads from the CTE bodies first, so the first FROM anywhere is the one that names a real table.
        "SELECT" | "DELETE" => marker_position(tokens, "FROM", through_ctes),
        "INSERT" | "REPLACE" | "MERGE" => marker_position(tokens, "INTO", through_ctes),
        "UPDATE" | "TRUNCATE" => Some(0),
        "CREATE" | "ALTER" | "DROP" => object_marker(tokens),
        _ => None,
    }?;
    name_after(tokens, start + 1)
}

/// Position of the first `keyword`; at parenthesis depth 0 only unless `anywhere`
/// (so `EXTRACT(year FROM d)` in a select list is not mistaken for the table).
fn marker_position(tokens: &[Token], keyword: &str, anywhere: bool) -> Option<usize> {
    let mut depth = 0_i32;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Open => depth += 1,
            Token::Close => depth -= 1,
            Token::Word(w) if w.eq_ignore_ascii_case(keyword) && (anywhere || depth == 0) => return Some(i),
            _ => {}
        }
    }
    None
}

/// For CREATE / ALTER / DROP: the position of the TABLE / VIEW word (an INDEX is
/// described by the table it is `ON`).
fn object_marker(tokens: &[Token]) -> Option<usize> {
    let position = tokens.iter().position(|t| matches!(t, Token::Word(w) if matches!(w.to_ascii_uppercase().as_str(), "TABLE" | "VIEW" | "INDEX" | "COLLECTION")))?;
    if is_word(tokens.get(position), "INDEX") {
        return tokens.iter().skip(position).position(|t| is_word(Some(t), "ON")).map(|on| position + on);
    }
    Some(position)
}

/// The (possibly schema-qualified) identifier at or after `from`, skipping the
/// `IF [NOT] EXISTS`, `ONLY`, `TABLE` and `CONCURRENTLY` noise words.
fn name_after(tokens: &[Token], from: usize) -> Option<String> {
    let mut i = from;
    while let Some(Token::Word(w)) = tokens.get(i) {
        if matches!(w.to_ascii_uppercase().as_str(), "IF" | "NOT" | "EXISTS" | "ONLY" | "TABLE" | "CONCURRENTLY" | "TEMPORARY" | "TEMP") {
            i += 1;
        } else {
            break;
        }
    }
    let mut parts: Vec<String> = Vec::new();
    loop {
        match tokens.get(i) {
            Some(Token::Word(w)) if parts.len() < 3 => parts.push(w.clone()),
            Some(Token::Quoted(q)) if parts.len() < 3 => parts.push(q.clone()),
            _ => break,
        }
        if matches!(tokens.get(i + 1), Some(Token::Dot)) {
            i += 2;
        } else {
            break;
        }
    }
    // `database.schema.table` keeps its last two parts; a derived table
    // `(SELECT …)` has no name.
    let keep = parts.len().min(2);
    let name = parts[parts.len() - keep..].join(".");
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(sql: &str) -> (String, Option<String>) {
        let d = describe(sql);
        (d.kind, d.table)
    }

    #[test]
    fn reads_the_kind_and_table_of_common_statements() {
        assert_eq!(info("SELECT * FROM users WHERE id = 1"), ("SELECT".into(), Some("users".into())));
        assert_eq!(info("  select a from public.\"Orders\" o"), ("SELECT".into(), Some("public.Orders".into())));
        assert_eq!(info("INSERT INTO t (a) VALUES (1)"), ("INSERT".into(), Some("t".into())));
        assert_eq!(info("UPDATE ONLY s.t SET a = 1"), ("UPDATE".into(), Some("s.t".into())));
        assert_eq!(info("DELETE FROM `db`.`items` WHERE 1"), ("DELETE".into(), Some("db.items".into())));
        assert_eq!(info("TRUNCATE TABLE logs"), ("TRUNCATE".into(), Some("logs".into())));
    }

    #[test]
    fn ddl_names_the_object_it_acts_on() {
        assert_eq!(info("CREATE TABLE IF NOT EXISTS app.events (id int)"), ("CREATE".into(), Some("app.events".into())));
        assert_eq!(info("DROP VIEW IF EXISTS v_sales"), ("DROP".into(), Some("v_sales".into())));
        assert_eq!(info("ALTER TABLE t ADD COLUMN c int"), ("ALTER".into(), Some("t".into())));
        assert_eq!(info("CREATE UNIQUE INDEX ix ON accounts (email)"), ("CREATE".into(), Some("accounts".into())));
        assert_eq!(info("CREATE OR REPLACE FUNCTION f() RETURNS int AS $$ SELECT 1 $$"), ("CREATE".into(), None));
    }

    #[test]
    fn a_with_clause_looks_through_to_the_real_verb() {
        let (kind, table) = info("WITH a AS (SELECT 1 FROM x), b AS (SELECT 2) SELECT * FROM a JOIN b ON true");
        assert_eq!(kind, "SELECT");
        assert_eq!(table.as_deref(), Some("x"), "the first FROM in the text is the one the log reports");
        assert_eq!(info("WITH d AS (SELECT 1) DELETE FROM t").0, "DELETE");
    }

    #[test]
    fn comments_and_literals_are_not_read() {
        assert_eq!(info("-- FROM ghost\nSELECT 'FROM nope' FROM real_table"), ("SELECT".into(), Some("real_table".into())));
        assert_eq!(info("/* UPDATE x */ SELECT 1").0, "SELECT");
    }

    #[test]
    fn statements_with_no_table_report_none() {
        assert_eq!(info("SELECT 1"), ("SELECT".into(), None));
        assert_eq!(info("SELECT * FROM (SELECT 1) t").1, None);
        assert_eq!(info("BEGIN"), ("BEGIN".into(), None));
        assert_eq!(info(""), (String::new(), None));
    }

    #[test]
    fn command_documents_and_plain_commands() {
        assert_eq!(info(r#"{"find": "users", "filter": {}}"#), ("FIND".into(), Some("users".into())));
        assert_eq!(info(r#"{"ping": 1}"#), ("PING".into(), None));
        assert_eq!(info("GET session:42"), ("GET".into(), None));
    }

    #[test]
    fn bracket_identifiers_are_unwrapped() {
        assert_eq!(info("SELECT * FROM [dbo].[Order Items]"), ("SELECT".into(), Some("dbo.Order Items".into())));
    }
}
