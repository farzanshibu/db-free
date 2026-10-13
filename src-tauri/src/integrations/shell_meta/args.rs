// SOT: shell-meta-arguments, shell-meta-quoting, shell-meta-qualified-names, sql-string-literal

// WHAT:  The small text tools every dialect's meta-commands share: reading a
//        command's head and arguments, and putting what the user typed into the
//        translated statement safely.
// WHY:   A meta-command's argument is user text that lands inside a catalog query
//        (`\dt users`, `.schema 'a b'`, `DESC hr.employees`). Quoting it in one
//        place, by the target language's own rules, is what keeps a pattern
//        containing a quote or a backslash from changing the statement.
// HOW:   Pure functions over `&str`. Nothing here knows an engine: callers pick
//        the `Quote` style of the language they translate to.
// WHERE: src-tauri/src/integrations/shell_meta/*.rs (every dialect)

/// How a language spells a text literal; chosen by the dialect that builds the statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quote {
    /// `'it''s'`; backslash is an ordinary character (Oracle, SQL Server, SQLite, DuckDB, CQL, ANSI SQL).
    Ansi,
    /// `E'it''s\\'`; escapes are always on, whatever `standard_conforming_strings` says (PostgreSQL family).
    PgEscape,
    /// `'it\'s\\'`; backslash escapes the next character (OrientDB).
    Backslash,
    /// `'it\x27s\\'`; like `Backslash`, but the quote is a hex escape so no quote character
    /// ever sits inside the literal: the block's statement splitter, which does not read
    /// backslash escapes, then sees one literal and never cuts at a `;` inside it
    /// (ClickHouse, BigQuery).
    BackslashHex,
    /// `'it''s\\'`; the quote doubled and the backslash escaped (MySQL family, Snowflake).
    DoubledBackslash,
}

// WHAT:  `raw` as a single-quoted text literal of the given style.
// WHY:   The only way user text enters a statement. Control characters are
//        dropped: a NUL or a stray line break has no business in a catalog filter.
pub fn string_literal(raw: &str, style: Quote) -> String {
    let mut out = String::with_capacity(raw.len() + 3);
    if style == Quote::PgEscape {
        out.push('E');
    }
    out.push('\'');
    for c in raw.chars().filter(|c| !c.is_control()) {
        match (c, style) {
            ('\'', Quote::Ansi | Quote::DoubledBackslash | Quote::PgEscape) => out.push_str("''"),
            ('\'', Quote::Backslash) => out.push_str("\\'"),
            ('\'', Quote::BackslashHex) => out.push_str("\\x27"),
            ('\\', Quote::PgEscape | Quote::Backslash | Quote::BackslashHex | Quote::DoubledBackslash) => out.push_str("\\\\"),
            (c, _) => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// `text` without its trailing `;` and blanks: a shell command takes no terminator, but people type one.
pub fn strip_terminator(text: &str) -> &str {
    text.trim().trim_end_matches(|c: char| c == ';' || c.is_whitespace())
}

// WHAT:  `text` from its first character that is not blank or part of a leading comment.
// WHY:   A command can sit under a `-- note` or `/* note */` line, and the splitter keeps that comment with it.
pub fn code_start(text: &str) -> &str {
    let mut rest = text.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("--") {
            rest = after.find('\n').map_or("", |at| &after[at..]).trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |at| &after[at + 2..]).trim_start();
        } else {
            return rest;
        }
    }
}

// WHAT:  The first blank-delimited word of `text` and the rest, trimmed.
pub fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    match text.find(char::is_whitespace) {
        Some(at) => (&text[..at], text[at..].trim()),
        None => (text, ""),
    }
}

/// Case-insensitive equality for keywords.
pub fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

// WHAT:  The text after `keyword` when `text` starts with that whole word, any case.
// HOW:   `keyword("describe tables", "DESCRIBE")` is `Some("tables")`; `keyword("describes", "DESCRIBE")` is None.
pub fn keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let (head, rest) = split_word(text);
    eq_ci(head, keyword).then_some(rest)
}

// WHAT:  The text after a run of keywords (`SHOW`, `CREATE`) when `text` starts with all of them, any case.
pub fn keywords<'a>(text: &'a str, words: &[&str]) -> Option<&'a str> {
    let mut rest = text.trim();
    for word in words {
        rest = keyword(rest, word)?;
    }
    Some(rest)
}

// WHAT:  Cuts a command's arguments at unquoted blanks, keeping each as typed (quotes included).
// WHY:   Dialects differ on what a quote means: psql keeps `"Mixed"` case-sensitive,
//        sqlite3 strips the quotes. `unquote` turns a raw argument into its value.
// HOW:   Single and double quotes group, a doubled quote inside is a literal one.
//        An unterminated quote is an error, never a guess.
pub fn raw_args(rest: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                current.push(c);
                if c == q {
                    if chars.peek() == Some(&q) {
                        current.push(q);
                        chars.next();
                    } else {
                        quote = None;
                    }
                }
            }
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                current.push(c);
            }
            None if c.is_whitespace() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            None => current.push(c),
        }
    }
    if let Some(q) = quote {
        return Err(format!("A {q} quote is never closed."));
    }
    if !current.is_empty() {
        args.push(current);
    }
    Ok(args)
}

// WHAT:  An argument's value: surrounding quotes removed, doubled quotes undoubled.
pub fn unquote(raw: &str) -> String {
    let mut chars = raw.chars();
    match (chars.next(), raw.chars().last()) {
        (Some(open @ ('\'' | '"')), Some(close)) if open == close && raw.chars().count() >= 2 => {
            let inner: String = raw.chars().skip(1).take(raw.chars().count() - 2).collect();
            inner.replace(&format!("{open}{open}"), &open.to_string())
        }
        _ => raw.to_string(),
    }
}

/// One dot-separated piece of a name, with whether the user quoted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub text: String,
    pub quoted: bool,
}

// WHAT:  `schema.table`, `"Mixed"."Case"`, `db.schema.table` as its pieces.
// WHY:   Every dialect looks names up by their pieces; whether a piece was
//        quoted decides if its case is folded (Oracle upper, PostgreSQL lower).
// HOW:   Dots inside double quotes belong to the piece; `""` is a quote. An empty
//        piece (`a..b`, a leading or trailing dot) is an error.
pub fn qualified(raw: &str) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut quoted = false;
    let mut open = false;
    let mut chars = raw.trim().chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if open => {
                if chars.peek() == Some(&'"') {
                    text.push('"');
                    chars.next();
                } else {
                    open = false;
                }
            }
            '"' => {
                open = true;
                quoted = true;
            }
            '.' if !open => {
                if text.is_empty() {
                    return Err(format!("\"{raw}\" is not a valid name."));
                }
                parts.push(Part { text: std::mem::take(&mut text), quoted });
                quoted = false;
            }
            c => text.push(c),
        }
    }
    if open {
        return Err("A \" quote is never closed.".to_string());
    }
    if text.is_empty() {
        return Err(format!("\"{raw}\" is not a valid name."));
    }
    parts.push(Part { text, quoted });
    Ok(parts)
}

impl Part {
    /// The piece as the database stores it when unquoted names fold to lower case.
    pub fn folded_lower(&self) -> String {
        if self.quoted { self.text.clone() } else { self.text.to_lowercase() }
    }
    /// The piece as the database stores it when unquoted names fold to upper case.
    pub fn folded_upper(&self) -> String {
        if self.quoted { self.text.clone() } else { self.text.to_uppercase() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_cannot_break_out_of_their_quotes() {
        assert_eq!(string_literal("it's", Quote::Ansi), "'it''s'");
        assert_eq!(string_literal("a\\b'c", Quote::Ansi), "'a\\b''c'");
        assert_eq!(string_literal("a\\b'c", Quote::PgEscape), "E'a\\\\b''c'");
        assert_eq!(string_literal("a\\b'c", Quote::Backslash), "'a\\\\b\\'c'");
        assert_eq!(string_literal("a\\b'c;", Quote::BackslashHex), "'a\\\\b\\x27c;'", "no quote character is left inside the literal");
        assert_eq!(string_literal("a\\b'c", Quote::DoubledBackslash), "'a\\\\b''c'");
        assert_eq!(string_literal("a\nb\0c", Quote::Ansi), "'abc'", "control characters are dropped");
        // The hostile case: a trailing backslash must not escape the closing quote.
        assert_eq!(string_literal("x\\", Quote::PgEscape), "E'x\\\\'");
        assert_eq!(string_literal("x\\", Quote::Backslash), "'x\\\\'");
    }

    #[test]
    fn terminators_and_comments_come_off_the_front_and_back() {
        assert_eq!(strip_terminator("  \\dt ; ; "), "\\dt");
        assert_eq!(code_start("-- note\n  /* more */ \\dt"), "\\dt");
        assert_eq!(code_start("select 1"), "select 1");
        assert_eq!(code_start("-- only a comment"), "");
    }

    #[test]
    fn keywords_match_whole_words_in_any_case() {
        assert_eq!(keyword("Describe  tables", "DESCRIBE"), Some("tables"));
        assert_eq!(keyword("describes", "describe"), None);
        assert_eq!(keywords("show   CREATE table t", &["SHOW", "create"]), Some("table t"));
        assert_eq!(keywords("show", &["show", "user"]), None);
        assert_eq!(split_word("  \\dt+   public.*  "), ("\\dt+", "public.*"));
        assert_eq!(split_word("help"), ("help", ""));
    }

    #[test]
    fn arguments_split_at_blanks_outside_quotes() {
        assert_eq!(raw_args("a  'b c' \"d \"\"e\" f").unwrap_or_default(), ["a", "'b c'", "\"d \"\"e\"", "f"]);
        assert!(raw_args("a 'b").is_err());
        assert!(raw_args("   ").unwrap_or_default().is_empty());
        assert_eq!(unquote("'it''s'"), "it's");
        assert_eq!(unquote("\"a\"\"b\""), "a\"b");
        assert_eq!(unquote("plain"), "plain");
        assert_eq!(unquote("'"), "'");
    }

    #[test]
    fn names_split_at_unquoted_dots() {
        let names = qualified("Hr.\"Emp.Loyees\".x").unwrap_or_default();
        assert_eq!(names.iter().map(|p| (p.text.as_str(), p.quoted)).collect::<Vec<_>>(), [("Hr", false), ("Emp.Loyees", true), ("x", false)]);
        assert_eq!(names[0].folded_lower(), "hr");
        assert_eq!(names[0].folded_upper(), "HR");
        assert_eq!(names[1].folded_lower(), "Emp.Loyees");
        assert!(qualified("a..b").is_err());
        assert!(qualified(".a").is_err());
        assert!(qualified("a.").is_err());
        assert!(qualified("\"open").is_err());
        assert_eq!(qualified("\"say \"\"hi\"\"\"").unwrap_or_default()[0].text, "say \"hi\"");
    }
}
