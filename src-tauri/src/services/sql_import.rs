// SOT: sql-import, sql-splitter, sql-dump-reader, sql-file-scan, mysql-delimiter, dollar-quote-splitter, go-batch-separator, plsql-slash-terminator

use crate::error::{AppError, AppResult};
use crate::guard::destructive::{classify, StatementKind};
use crate::model::{Engine, Family, SqlFileInfo};
use std::collections::VecDeque;
use std::path::Path;
use tokio::io::AsyncReadExt;

// ============================================================================
// SQL FILE IMPORT
//
// WHAT:  Reads a .sql script of any size a chunk at a time and cuts it into
//        statements the way the engine's own client would: quotes, comments,
//        dollar quotes, MySQL's DELIMITER, SQL Server's GO, Oracle's `/`.
// WHY:   The editor's splitter (guard/destructive.rs) answers "where does this
//        statement end" for a script that is already in a text box. A dump is
//        gigabytes, full of stored-routine bodies with `;` inside them, and
//        must never be held whole in memory — and cutting it in the wrong place
//        would run half a statement.
// HOW:   `SqlSplitter` is a push parser: feed it text in any chunking and it
//        yields the same statements. State (open quote, comment depth, the
//        current delimiter…) survives between chunks; the last few characters
//        are held back so a two-character token never straddles a boundary.
//        `SqlFile` wraps it around an async file reader and decodes UTF-8
//        incrementally. `scan` walks a whole file once, with the same splitter
//        and the guard's classifier, so the confirmation dialog's counts are
//        what the run will meet.
// WHERE: src-tauri/src/commands/transfer.rs (import_sql_file, inspect_sql_file)
// ============================================================================

/// Characters held back between chunks: longer than any token looked ahead for
/// (a DELIMITER line, a dollar-quote tag, a `GO` line).
const HOLD: usize = 160;
const MAX_STATEMENT_CHARS: usize = 16 * 1024 * 1024;
const READ_CHUNK: usize = 64 * 1024;
const HEAD_BYTES: usize = 2 * 1024;
const MAX_DELIMITER: usize = 16;
const EXAMPLES: usize = 5;

/// How a script is cut. One per kind of lexical rules, not per engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDialect {
    /// Backslash escapes, `#` comments, `-- ` comments, backticks, `DELIMITER`.
    Mysql,
    /// Dollar quotes, `E'…'` escapes, nested block comments.
    Postgres,
    /// `[bracketed]` identifiers, nested block comments, `GO` batches.
    Mssql,
    /// A PL/SQL block ends at a line holding only `/`.
    Oracle,
    /// Standard SQL: quotes, `--`, `/* */`.
    Standard,
}

pub fn dialect_of(engine: Engine) -> SplitDialect {
    match engine.family() {
        Family::Mysql => SplitDialect::Mysql,
        Family::Postgres | Family::Duckdb => SplitDialect::Postgres,
        Family::Mssql => SplitDialect::Mssql,
        Family::Oracle => SplitDialect::Oracle,
        _ => SplitDialect::Standard,
    }
}

/// One statement and the line of the file it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlStatement {
    pub text: String,
    pub line: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comment {
    None,
    Line,
    Block,
}

pub struct SqlSplitter {
    dialect: SplitDialect,
    /// Characters held back between chunks (see `HOLD`).
    hold: usize,
    /// Characters not processed yet: the held-back tail plus the newest chunk.
    carry: Vec<char>,
    current: String,
    has_sql: bool,
    quote: Option<char>,
    /// The open quote is a Postgres `E'…'` string, which reads backslash escapes.
    e_string: bool,
    bracket: bool,
    comment: Comment,
    block_depth: u32,
    dollar: Option<Vec<char>>,
    delimiter: Vec<char>,
    /// Only whitespace since the last newline.
    line_start: bool,
    plsql: bool,
    line: u64,
    start_line: u64,
    prev: char,
    prev2: char,
}

impl SqlSplitter {
    pub fn new(dialect: SplitDialect) -> SqlSplitter {
        SqlSplitter {
            dialect,
            hold: HOLD,
            carry: Vec::new(),
            current: String::new(),
            has_sql: false,
            quote: None,
            e_string: false,
            bracket: false,
            comment: Comment::None,
            block_depth: 0,
            dollar: None,
            delimiter: vec![';'],
            line_start: true,
            plsql: false,
            line: 1,
            start_line: 1,
            prev: '\0',
            prev2: '\0',
        }
    }

    // WHAT:  Adds text; returns the statements it completed.
    pub fn feed(&mut self, chunk: &str) -> AppResult<Vec<SqlStatement>> {
        self.carry.extend(chunk.chars());
        self.run(false)
    }

    // WHAT:  Ends the script: the last statement (which needs no delimiter), or
    //        an error when the text ends inside a quote or comment.
    pub fn finish(&mut self) -> AppResult<Vec<SqlStatement>> {
        let mut out = self.run(true)?;
        if self.quote.is_some() || self.bracket || self.dollar.is_some() || self.comment == Comment::Block {
            return Err(AppError::invalid_input(format!(
                "The script ends inside a quoted value or comment that began before line {}. Check for a missing quote.",
                self.line
            )));
        }
        self.take_statement(&mut out);
        Ok(out)
    }

    fn nested_comments(&self) -> bool {
        matches!(self.dialect, SplitDialect::Postgres | SplitDialect::Mssql)
    }

    /// Does a backslash escape inside the open quote?
    fn backslash(&self, quote: char) -> bool {
        match self.dialect {
            SplitDialect::Mysql => quote != '`',
            SplitDialect::Postgres => self.e_string,
            _ => false,
        }
    }

    fn consume(&mut self, chars: &[char]) {
        for &c in chars {
            if c == '\n' {
                self.line += 1;
            }
            self.prev2 = self.prev;
            self.prev = c;
        }
    }

    fn append(&mut self, chars: &[char]) -> AppResult<()> {
        if self.current.len() + chars.len() > MAX_STATEMENT_CHARS {
            return Err(AppError::invalid_input(format!(
                "A SQL statement near line {} is over {} MiB. Export the dump with smaller INSERT batches.",
                self.start_line,
                MAX_STATEMENT_CHARS / 1024 / 1024
            )));
        }
        self.current.extend(chars.iter());
        Ok(())
    }

    fn take_statement(&mut self, out: &mut Vec<SqlStatement>) {
        let text = std::mem::take(&mut self.current);
        let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if self.has_sql && !trimmed.is_empty() {
            out.push(SqlStatement { text: trimmed.to_string(), line: self.start_line });
        }
        self.has_sql = false;
        self.plsql = false;
    }

    fn mark_sql(&mut self, rest: &[char]) {
        if !self.has_sql {
            self.has_sql = true;
            self.start_line = self.line;
            if self.dialect == SplitDialect::Oracle {
                self.plsql = starts_plsql(rest);
            }
        }
        self.line_start = false;
    }

    fn run(&mut self, last: bool) -> AppResult<Vec<SqlStatement>> {
        let buf = std::mem::take(&mut self.carry);
        let len = buf.len();
        let limit = if last { len } else { len.saturating_sub(self.hold) };
        let mut out: Vec<SqlStatement> = Vec::new();
        let mut i = 0_usize;
        // Start of the text not yet appended to `current`.
        let mut seg = 0_usize;
        let result = self.scan_buffer(&buf, limit, last, &mut i, &mut seg, &mut out);
        match result {
            Ok(()) => {
                self.append(&buf[seg..i])?;
                self.carry = buf[i..].to_vec();
                Ok(out)
            }
            Err(err) => Err(err),
        }
    }

    fn scan_buffer(&mut self, buf: &[char], limit: usize, last: bool, i: &mut usize, seg: &mut usize, out: &mut Vec<SqlStatement>) -> AppResult<()> {
        let len = buf.len();
        while *i < limit {
            let at = *i;
            let c = buf[at];
            let next = buf.get(at + 1).copied().unwrap_or('\0');

            match self.comment {
                Comment::Line => {
                    self.consume(&buf[at..=at]);
                    if c == '\n' {
                        self.comment = Comment::None;
                        self.line_start = true;
                    }
                    *i += 1;
                    continue;
                }
                Comment::Block => {
                    if c == '*' && next == '/' {
                        self.consume(&buf[at..at + 2]);
                        *i += 2;
                        self.block_depth = self.block_depth.saturating_sub(1);
                        if self.block_depth == 0 {
                            self.comment = Comment::None;
                        }
                    } else if c == '/' && next == '*' && self.nested_comments() {
                        self.consume(&buf[at..at + 2]);
                        *i += 2;
                        self.block_depth += 1;
                    } else {
                        self.consume(&buf[at..=at]);
                        *i += 1;
                        if c == '\n' {
                            self.line_start = true;
                        }
                    }
                    continue;
                }
                Comment::None => {}
            }

            if let Some(quote) = self.quote {
                if c == '\\' && self.backslash(quote) {
                    let n = if at + 1 < len { 2 } else { 1 };
                    self.consume(&buf[at..at + n]);
                    *i += n;
                    continue;
                }
                if c == quote {
                    if next == quote {
                        self.consume(&buf[at..at + 2]);
                        *i += 2;
                        continue;
                    }
                    self.quote = None;
                    self.e_string = false;
                }
                self.consume(&buf[at..=at]);
                *i += 1;
                continue;
            }
            if self.bracket {
                if c == ']' {
                    if next == ']' {
                        self.consume(&buf[at..at + 2]);
                        *i += 2;
                        continue;
                    }
                    self.bracket = false;
                }
                self.consume(&buf[at..=at]);
                *i += 1;
                continue;
            }
            if let Some(closing) = self.dollar.as_ref().map(|tag| buf[at..].starts_with(tag).then_some(tag.len())) {
                match closing {
                    Some(n) => {
                        self.dollar = None;
                        self.consume(&buf[at..at + n]);
                        *i += n;
                    }
                    None => {
                        self.consume(&buf[at..=at]);
                        *i += 1;
                    }
                }
                continue;
            }

            // ---- directives that own a whole line --------------------------
            if self.line_start {
                if self.dialect == SplitDialect::Mysql && !self.has_sql {
                    if let Some((used, token)) = parse_delimiter(&buf[at..], last)? {
                        self.append(&buf[*seg..at])?;
                        self.consume(&buf[at..at + used]);
                        *i += used;
                        *seg = *i;
                        self.delimiter = token;
                        self.line_start = true;
                        continue;
                    }
                }
                if self.dialect == SplitDialect::Mssql {
                    if let Some(used) = parse_go(&buf[at..], last) {
                        self.append(&buf[*seg..at])?;
                        self.take_statement(out);
                        self.consume(&buf[at..at + used]);
                        *i += used;
                        *seg = *i;
                        self.line_start = true;
                        continue;
                    }
                }
                if self.dialect == SplitDialect::Oracle {
                    if let Some(used) = parse_slash(&buf[at..], last) {
                        self.append(&buf[*seg..at])?;
                        self.take_statement(out);
                        self.consume(&buf[at..at + used]);
                        *i += used;
                        *seg = *i;
                        self.line_start = true;
                        continue;
                    }
                }
            }

            // ---- the statement delimiter -----------------------------------
            if !self.plsql && buf[at..].starts_with(&self.delimiter) {
                let n = self.delimiter.len();
                self.append(&buf[*seg..at])?;
                self.take_statement(out);
                self.consume(&buf[at..at + n]);
                *i += n;
                *seg = *i;
                continue;
            }

            // ---- comments --------------------------------------------------
            let mysql = self.dialect == SplitDialect::Mysql;
            if c == '-' && next == '-' {
                let after = buf.get(at + 2).copied();
                // MySQL needs whitespace after `--`; `5--3` is arithmetic there.
                if !mysql || after.is_some_and(char::is_whitespace) || (after.is_none() && last) {
                    self.comment = Comment::Line;
                    self.consume(&buf[at..at + 2]);
                    *i += 2;
                    continue;
                }
            }
            if c == '#' && mysql {
                self.comment = Comment::Line;
                self.consume(&buf[at..=at]);
                *i += 1;
                continue;
            }
            if c == '/' && next == '*' {
                let conditional = mysql && (buf.get(at + 2) == Some(&'!') || (buf.get(at + 2) == Some(&'M') && buf.get(at + 3) == Some(&'!')));
                if conditional {
                    // `/*! … */` runs on MySQL: the statement is real code.
                    self.mark_sql(&buf[at..]);
                }
                self.comment = Comment::Block;
                self.block_depth = 1;
                self.consume(&buf[at..at + 2]);
                *i += 2;
                continue;
            }

            // ---- quotes ----------------------------------------------------
            let backtick = c == '`' && !matches!(self.dialect, SplitDialect::Postgres | SplitDialect::Mssql);
            if c == '\'' || c == '"' || backtick {
                self.mark_sql(&buf[at..]);
                self.e_string = self.dialect == SplitDialect::Postgres && c == '\'' && matches!(self.prev, 'E' | 'e') && !(self.prev2.is_alphanumeric() || self.prev2 == '_');
                self.quote = Some(c);
                self.consume(&buf[at..=at]);
                *i += 1;
                continue;
            }
            if c == '[' && self.dialect == SplitDialect::Mssql {
                self.mark_sql(&buf[at..]);
                self.bracket = true;
                self.consume(&buf[at..=at]);
                *i += 1;
                continue;
            }
            if c == '$' && self.dialect == SplitDialect::Postgres && !(self.prev.is_alphanumeric() || self.prev == '_') {
                if let Some(tag) = dollar_tag(&buf[at..]) {
                    self.mark_sql(&buf[at..]);
                    let n = tag.len();
                    self.consume(&buf[at..at + n]);
                    self.dollar = Some(tag);
                    *i += n;
                    continue;
                }
            }

            // ---- anything else ---------------------------------------------
            if !c.is_whitespace() && c != '\u{feff}' {
                self.mark_sql(&buf[at..]);
            }
            if c == '\n' {
                self.line_start = true;
            }
            self.consume(&buf[at..=at]);
            *i += 1;
        }
        Ok(())
    }
}

/// `$tag$` or `$$` at the start of `rest`.
fn dollar_tag(rest: &[char]) -> Option<Vec<char>> {
    let mut j = 1;
    while j < rest.len() && (rest[j].is_alphanumeric() || rest[j] == '_') {
        j += 1;
    }
    if rest.get(j) != Some(&'$') {
        return None;
    }
    // A tag cannot start with a digit (`$1$` is not one).
    if j > 1 && rest[1].is_ascii_digit() {
        return None;
    }
    Some(rest[..=j].to_vec())
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Characters up to and including the line's end, if the rest of the line holds
/// only blanks from `from`; None when text follows.
fn rest_of_line_blank(rest: &[char], from: usize, last: bool) -> Option<usize> {
    let mut j = from;
    while j < rest.len() && is_blank(rest[j]) {
        j += 1;
    }
    match rest.get(j) {
        Some('\n') => Some(j + 1),
        Some('\r') if rest.get(j + 1) == Some(&'\n') => Some(j + 2),
        None if last => Some(j),
        _ => None,
    }
}

fn matches_word(rest: &[char], word: &str) -> bool {
    let mut chars = rest.iter();
    word.chars().all(|w| chars.next().is_some_and(|c| c.eq_ignore_ascii_case(&w)))
}

// WHAT:  `DELIMITER <token>` as a whole line → (chars it spans, the new delimiter).
fn parse_delimiter(rest: &[char], last: bool) -> AppResult<Option<(usize, Vec<char>)>> {
    const WORD: &str = "DELIMITER";
    if !matches_word(rest, WORD) || !rest.get(WORD.len()).is_some_and(|c| is_blank(*c)) {
        return Ok(None);
    }
    let mut j = WORD.len();
    while j < rest.len() && is_blank(rest[j]) {
        j += 1;
    }
    let start = j;
    while j < rest.len() && !rest[j].is_whitespace() {
        j += 1;
    }
    let token: Vec<char> = rest[start..j].to_vec();
    if token.is_empty() {
        return Ok(None);
    }
    let Some(used) = rest_of_line_blank(rest, j, last) else {
        return Ok(None);
    };
    if token.len() > MAX_DELIMITER {
        return Err(AppError::invalid_input(format!("A DELIMITER is longer than {MAX_DELIMITER} characters.")));
    }
    Ok(Some((used, token)))
}

// WHAT:  `GO` (optionally `GO 5`) as a whole line → chars it spans.
fn parse_go(rest: &[char], last: bool) -> Option<usize> {
    if !matches_word(rest, "GO") {
        return None;
    }
    let mut j = 2;
    if rest.get(j).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
        return None;
    }
    while j < rest.len() && is_blank(rest[j]) {
        j += 1;
    }
    while j < rest.len() && rest[j].is_ascii_digit() {
        j += 1;
    }
    rest_of_line_blank(rest, j, last)
}

// WHAT:  A lone `/` line (Oracle ends a PL/SQL block with it).
fn parse_slash(rest: &[char], last: bool) -> Option<usize> {
    if rest.first() != Some(&'/') {
        return None;
    }
    rest_of_line_blank(rest, 1, last)
}

// WHAT:  Does the statement starting at `rest` open a PL/SQL block, one that can
//        hold `;` and so ends only at a `/` line?
fn starts_plsql(rest: &[char]) -> bool {
    let words: Vec<String> = rest
        .iter()
        .take(160)
        .collect::<String>()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .take(6)
        .map(str::to_ascii_uppercase)
        .collect();
    let mut it = words.iter().map(String::as_str).peekable();
    match it.next() {
        Some("DECLARE" | "BEGIN") => true,
        Some("CREATE") => {
            if it.peek() == Some(&"OR") {
                it.next();
                it.next();
            }
            while matches!(it.peek(), Some(&"EDITIONABLE" | &"NONEDITIONABLE")) {
                it.next();
            }
            match it.next() {
                Some("FUNCTION" | "PROCEDURE" | "PACKAGE" | "TRIGGER") => true,
                Some("TYPE") => it.next() == Some("BODY"),
                _ => false,
            }
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Reading a file
// ---------------------------------------------------------------------------

/// A .sql file read a chunk at a time, yielding statements.
pub struct SqlFile {
    file: tokio::fs::File,
    splitter: SqlSplitter,
    queue: VecDeque<SqlStatement>,
    /// Bytes of a UTF-8 character cut by the end of the last chunk.
    partial: Vec<u8>,
    first_chunk: bool,
    done: bool,
    pub size: u64,
    pub bytes_read: u64,
}

impl SqlFile {
    pub async fn open(path: &Path, dialect: SplitDialect) -> AppResult<SqlFile> {
        let file = tokio::fs::File::open(path).await.map_err(AppError::invalid_input_from)?;
        let meta = file.metadata().await.map_err(AppError::invalid_input_from)?;
        if !meta.is_file() {
            return Err(AppError::invalid_input("Choose a regular SQL file."));
        }
        Ok(SqlFile { file, splitter: SqlSplitter::new(dialect), queue: VecDeque::new(), partial: Vec::new(), first_chunk: true, done: false, size: meta.len(), bytes_read: 0 })
    }

    // WHAT:  The next statement, or None at the end of the file.
    pub async fn next(&mut self) -> AppResult<Option<SqlStatement>> {
        loop {
            if let Some(statement) = self.queue.pop_front() {
                return Ok(Some(statement));
            }
            if self.done {
                return Ok(None);
            }
            let mut chunk = vec![0_u8; READ_CHUNK];
            let n = self.file.read(&mut chunk).await.map_err(AppError::invalid_input_from)?;
            if n == 0 {
                self.done = true;
                if !self.partial.is_empty() {
                    return Err(AppError::invalid_input("The file ends in the middle of a UTF-8 character. Large SQL files must be valid UTF-8."));
                }
                self.queue.extend(self.splitter.finish()?);
                continue;
            }
            chunk.truncate(n);
            self.bytes_read += n as u64;
            if self.first_chunk {
                self.first_chunk = false;
                if chunk.starts_with(&[0xFF, 0xFE]) || chunk.starts_with(&[0xFE, 0xFF]) {
                    return Err(AppError::invalid_input("This file is UTF-16. Convert it to UTF-8 before importing."));
                }
                if chunk.starts_with(b"\xEF\xBB\xBF") {
                    chunk.drain(..3);
                }
            }
            let text = self.decode(chunk)?;
            self.queue.extend(self.splitter.feed(&text)?);
        }
    }

    // WHAT:  Bytes → text, keeping a character cut by the chunk edge for the next call.
    fn decode(&mut self, chunk: Vec<u8>) -> AppResult<String> {
        let mut bytes = std::mem::take(&mut self.partial);
        bytes.extend(chunk);
        match std::str::from_utf8(&bytes) {
            Ok(text) => Ok(text.to_string()),
            Err(err) if err.error_len().is_none() => {
                let valid = err.valid_up_to();
                self.partial = bytes[valid..].to_vec();
                Ok(String::from_utf8_lossy(&bytes[..valid]).into_owned())
            }
            Err(_) => Err(AppError::invalid_input("The file is not valid UTF-8. Convert it to UTF-8 before importing.")),
        }
    }
}

/// `SET sql_mode = '…NO_BACKSLASH_ESCAPES…'` changes how every later string is
/// read; the splitter's rules would then cut statements wrongly.
pub fn changes_escaping(statement: &str) -> bool {
    let upper = statement.to_ascii_uppercase();
    upper.contains("NO_BACKSLASH_ESCAPES") && upper.trim_start().starts_with("SET")
}

pub const ESCAPING_PROBLEM: &str = "This dump sets NO_BACKSLASH_ESCAPES, which changes how strings are read. Import it with the mysql command-line client instead.";

/// The statement without the comments in front of it (a dump labels each one).
fn skip_comments(statement: &str) -> &str {
    let mut rest = statement.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("--").or_else(|| rest.strip_prefix('#')) {
            rest = after.split_once('\n').map_or("", |(_, tail)| tail).trim_start();
        } else if rest.starts_with("/*") && !rest.starts_with("/*!") {
            rest = rest.split_once("*/").map_or("", |(_, tail)| tail).trim_start();
        } else {
            return rest;
        }
    }
}

/// The first word of a statement, uppercased, with leading comments skipped.
fn leading_word(statement: &str) -> String {
    skip_comments(statement).split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or("").to_ascii_uppercase()
}

/// Can the guard's classifier call this statement destructive? Only these
/// leaders can be; skipping the rest keeps a scan of a huge INSERT dump cheap.
fn may_be_destructive(statement: &str) -> bool {
    matches!(leading_word(statement).as_str(), "DROP" | "TRUNCATE" | "DELETE" | "UPDATE" | "ALTER" | "WITH")
}

pub fn is_destructive(statement: &str) -> bool {
    may_be_destructive(statement) && classify(statement).iter().any(|s| s.kind == StatementKind::Destructive)
}

fn shorten(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 120 {
        format!("{}…", flat.chars().take(119).collect::<String>())
    } else {
        flat
    }
}

pub fn shorten_statement(text: &str) -> String {
    shorten(text)
}

fn modified_ms(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

// WHAT:  Refuses a file that is not the one the user inspected.
// WHY:   The destructive-statement confirmation covers a particular file; the
//        same path edited in between would run statements nobody confirmed.
pub async fn verify_unchanged(path: &Path, expected_size: u64, expected_modified_ms: u64) -> AppResult<()> {
    let metadata = tokio::fs::metadata(path).await.map_err(AppError::invalid_input_from)?;
    if metadata.len() != expected_size || modified_ms(&metadata) != expected_modified_ms {
        return Err(AppError::invalid_input("The SQL file changed after it was inspected. Choose it again before importing."));
    }
    Ok(())
}

// WHAT:  One pass over a whole file: how many statements, how many the guard
//        will ask to confirm, the first few of those, the file's head.
// WHY:   The dialog needs the numbers before the user decides, and a count made
//        with another splitter would disagree with the run.
pub async fn scan(path: &Path, engine: Engine) -> AppResult<SqlFileInfo> {
    let metadata = tokio::fs::metadata(path).await.map_err(AppError::invalid_input_from)?;
    let modified_ms = modified_ms(&metadata);
    let mut head = vec![0_u8; HEAD_BYTES];
    let read = {
        let mut file = tokio::fs::File::open(path).await.map_err(AppError::invalid_input_from)?;
        file.read(&mut head).await.map_err(AppError::invalid_input_from)?
    };
    head.truncate(read);
    let head_text = String::from_utf8_lossy(&head).trim_start_matches('\u{feff}').to_string();

    let mut info = SqlFileInfo { size: metadata.len(), modified_ms, statements: 0, destructive: 0, destructive_examples: Vec::new(), head: head_text, problem: None };
    let mut file = SqlFile::open(path, dialect_of(engine)).await?;
    loop {
        match file.next().await {
            Ok(Some(statement)) => {
                if changes_escaping(&statement.text) {
                    info.problem = Some(ESCAPING_PROBLEM.to_string());
                    break;
                }
                info.statements += 1;
                if is_destructive(&statement.text) {
                    info.destructive += 1;
                    if info.destructive_examples.len() < EXAMPLES {
                        info.destructive_examples.push(shorten(skip_comments(&statement.text)));
                    }
                }
            }
            Ok(None) => break,
            Err(err) => {
                info.problem = Some(err.message().to_string());
                break;
            }
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(dialect: SplitDialect, script: &str) -> Vec<String> {
        let mut splitter = SqlSplitter::new(dialect);
        let mut out = splitter.feed(script).unwrap_or_else(|e| panic!("{e}"));
        out.extend(splitter.finish().unwrap_or_else(|e| panic!("{e}")));
        out.into_iter().map(|s| s.text).collect()
    }

    /// The same script fed in pieces of `size` characters.
    fn split_in(dialect: SplitDialect, script: &str, size: usize) -> Vec<SqlStatement> {
        let mut splitter = SqlSplitter::new(dialect);
        let mut out = Vec::new();
        let chars: Vec<char> = script.chars().collect();
        for piece in chars.chunks(size) {
            out.extend(splitter.feed(&piece.iter().collect::<String>()).unwrap_or_else(|e| panic!("{e}")));
        }
        out.extend(splitter.finish().unwrap_or_else(|e| panic!("{e}")));
        out
    }

    #[test]
    fn quotes_and_comments_hide_semicolons() {
        let got = split(SplitDialect::Standard, "select 1; select 'a;b', \"c;d\", `e;f`; -- trailing; comment\nselect 2 /* x; y */ ; select 3");
        assert_eq!(got, vec!["select 1", "select 'a;b', \"c;d\", `e;f`", "-- trailing; comment\nselect 2 /* x; y */", "select 3"]);
        assert_eq!(split(SplitDialect::Standard, "select 'it''s;'; select 2"), vec!["select 'it''s;'", "select 2"]);
        assert!(split(SplitDialect::Standard, "-- only a comment\n/* and another */\n  ;;; ").is_empty(), "comments alone are not statements");
    }

    #[test]
    fn mysql_reads_backslashes_hash_comments_and_dash_dash_rules() {
        assert_eq!(split(SplitDialect::Mysql, "insert into t values ('it\\'s;'); select 1"), vec!["insert into t values ('it\\'s;')", "select 1"]);
        assert_eq!(split(SplitDialect::Standard, "select 'a\\'; select 2"), vec!["select 'a\\'", "select 2"], "standard SQL has no backslash escape");
        assert_eq!(split(SplitDialect::Mysql, "# note; here\nselect 1; select 5--3; select 2"), vec!["# note; here\nselect 1", "select 5--3", "select 2"]);
        assert_eq!(split(SplitDialect::Mysql, "select 1 -- why; not\n; select 2"), vec!["select 1 -- why; not", "select 2"]);
    }

    #[test]
    fn mysql_conditional_comments_are_statements() {
        let got = split(SplitDialect::Mysql, "/*!40101 SET NAMES utf8 */;\n/* plain */;\nDROP TABLE IF EXISTS `t`;");
        assert_eq!(got, vec!["/*!40101 SET NAMES utf8 */", "DROP TABLE IF EXISTS `t`"]);
    }

    #[test]
    fn mysql_delimiter_changes_the_terminator() {
        let script = "SELECT 1;\nDELIMITER ;;\nCREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END;;\nDELIMITER ;\nSELECT 3;\n";
        assert_eq!(split(SplitDialect::Mysql, script), vec!["SELECT 1", "CREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END", "SELECT 3"]);
        let two = "delimiter $$\nSELECT 1$$\nSELECT 2$$\nDELIMITER ;\nSELECT 3;";
        assert_eq!(split(SplitDialect::Mysql, two), vec!["SELECT 1", "SELECT 2", "SELECT 3"]);
        // Not a directive when text follows, or off the dialect.
        assert_eq!(split(SplitDialect::Mysql, "SELECT delimiter ;"), vec!["SELECT delimiter"]);
        assert_eq!(split(SplitDialect::Standard, "DELIMITER ;;\nSELECT 1;;"), vec!["DELIMITER", "SELECT 1"]);
    }

    #[test]
    fn postgres_dollar_quotes_and_nested_comments() {
        let script = "create function f() returns int as $$ begin return 1; end $$ language plpgsql; select $tag$ a;b $tag$; /* a /* b */ c; */ select 2;";
        assert_eq!(
            split(SplitDialect::Postgres, script),
            vec!["create function f() returns int as $$ begin return 1; end $$ language plpgsql", "select $tag$ a;b $tag$", "/* a /* b */ c; */ select 2"]
        );
        assert_eq!(split(SplitDialect::Postgres, "select E'a\\';b'; select 'a\\'; select 2"), vec!["select E'a\\';b'", "select 'a\\'", "select 2"], "only E strings read backslashes");
        assert_eq!(split(SplitDialect::Postgres, "select $1, $2; select 3"), vec!["select $1, $2", "select 3"], "parameters are not dollar quotes");
    }

    #[test]
    fn sql_server_go_ends_a_batch() {
        let got = split(SplitDialect::Mssql, "select 1\nGO\nselect [a;b]\ngo 3\nselect 3\n");
        assert_eq!(got, vec!["select 1", "select [a;b]", "select 3"]);
        assert_eq!(split(SplitDialect::Mssql, "select 'GO'\nGOTO x\n"), vec!["select 'GO'\nGOTO x"], "GO is a whole line");
    }

    #[test]
    fn oracle_blocks_end_at_a_slash_line() {
        let script = "BEGIN\n  NULL;\n  NULL;\nEND;\n/\nSELECT 1 FROM dual;\nCREATE OR REPLACE PROCEDURE p AS BEGIN NULL; END;\n/\nCREATE TABLE t (a int);\n";
        assert_eq!(
            split(SplitDialect::Oracle, script),
            vec!["BEGIN\n  NULL;\n  NULL;\nEND;", "SELECT 1 FROM dual", "CREATE OR REPLACE PROCEDURE p AS BEGIN NULL; END;", "CREATE TABLE t (a int)"]
        );
        assert!(starts_plsql(&"CREATE OR REPLACE EDITIONABLE PACKAGE BODY x".chars().collect::<Vec<_>>()));
        assert!(!starts_plsql(&"CREATE TABLE x".chars().collect::<Vec<_>>()));
        assert!(!starts_plsql(&"CREATE TYPE x AS OBJECT".chars().collect::<Vec<_>>()));
        assert!(starts_plsql(&"CREATE TYPE BODY x".chars().collect::<Vec<_>>()));
    }

    /// `script` fed in pieces of `size`, holding back only `hold` characters so
    /// the chunk edges really fall inside tokens.
    fn split_holding(dialect: SplitDialect, script: &str, size: usize, hold: usize) -> Vec<SqlStatement> {
        let mut splitter = SqlSplitter::new(dialect);
        splitter.hold = hold;
        let mut out = Vec::new();
        let chars: Vec<char> = script.chars().collect();
        for piece in chars.chunks(size) {
            out.extend(splitter.feed(&piece.iter().collect::<String>()).unwrap_or_else(|e| panic!("{e}")));
        }
        out.extend(splitter.finish().unwrap_or_else(|e| panic!("{e}")));
        out
    }

    #[test]
    fn chunking_never_changes_the_answer() {
        let one = "SELECT 'a;b'; -- c;\nDELIMITER ;;\nCREATE PROCEDURE p() BEGIN SELECT 1; END;;\nDELIMITER ;\n/*!40101 SET x=1 */;\nINSERT INTO t VALUES ('it\\'s;');\nselect 5--3;\n/* long\nblock; */ select 6;\n";
        let script = one.repeat(12);
        let whole = split_in(SplitDialect::Mysql, &script, usize::MAX);
        assert_eq!(whole.len(), 72, "{}", whole.len());
        for size in [1, 2, 3, 5, 7, 31, 200] {
            assert_eq!(split_in(SplitDialect::Mysql, &script, size), whole, "pieces of {size}");
            assert_eq!(split_holding(SplitDialect::Mysql, &script, size, 40), whole, "pieces of {size}, 40 held back");
        }
        let pg = "create function f() returns int as $a$ begin return 1; end $a$ language plpgsql;\nselect E'x\\';y'; /* a /* b */ c */ select 2;\n".repeat(10);
        let whole = split_in(SplitDialect::Postgres, &pg, usize::MAX);
        assert_eq!(whole.len(), 30);
        for size in [1, 2, 4, 9, 40] {
            assert_eq!(split_in(SplitDialect::Postgres, &pg, size), whole, "pieces of {size}");
            assert_eq!(split_holding(SplitDialect::Postgres, &pg, size, 30), whole, "pieces of {size}, 30 held back");
        }
        let long = format!("select '{}'; select 2;", "x;".repeat(500));
        assert_eq!(split_in(SplitDialect::Standard, &long, 13).len(), 2, "a chunk edge inside a long literal");
        assert_eq!(split_holding(SplitDialect::Standard, &long, 13, 4).len(), 2);
    }

    #[test]
    fn statements_know_their_line() {
        let got = split_in(SplitDialect::Standard, "select 1;\n\n-- c\nselect 2;\nselect\n3;", usize::MAX);
        assert_eq!(got.iter().map(|s| s.line).collect::<Vec<_>>(), vec![1, 4, 5]);
        let got = split_in(SplitDialect::Standard, "select 1;\n\n-- c\nselect 2;\nselect\n3;", 3);
        assert_eq!(got.iter().map(|s| s.line).collect::<Vec<_>>(), vec![1, 4, 5], "in small pieces too");
    }

    #[test]
    fn an_unterminated_quote_or_comment_is_an_error() {
        for (dialect, script) in [(SplitDialect::Standard, "select 'oops"), (SplitDialect::Standard, "select 1 /* never closed"), (SplitDialect::Postgres, "select $$ never closed")] {
            let mut splitter = SqlSplitter::new(dialect);
            let fed = splitter.feed(script);
            assert!(fed.is_ok());
            assert!(splitter.finish().is_err(), "{script}");
        }
        let mut splitter = SqlSplitter::new(SplitDialect::Mysql);
        assert!(splitter.feed("DELIMITER aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n").is_err() || splitter.finish().is_err());
    }

    #[test]
    fn byte_order_marks_and_blank_input_are_ignored() {
        assert_eq!(split(SplitDialect::Standard, "\u{feff}select 1;"), vec!["select 1"]);
        assert!(split(SplitDialect::Standard, "   \n  ").is_empty());
    }

    #[test]
    fn destructive_statements_are_found_cheaply() {
        assert!(is_destructive("DROP TABLE t"));
        assert!(is_destructive("-- c\ntruncate t"));
        assert!(is_destructive("DELETE FROM t"));
        assert!(!is_destructive("DELETE FROM t WHERE id = 1"));
        assert!(!is_destructive("INSERT INTO t VALUES (1)"));
        assert!(!is_destructive("CREATE TABLE t (a int)"));
        assert_eq!(leading_word("/* x */ -- y\n  Select 1"), "SELECT");
        assert!(changes_escaping("SET sql_mode='NO_BACKSLASH_ESCAPES'"));
        assert!(!changes_escaping("INSERT INTO t VALUES ('NO_BACKSLASH_ESCAPES')"));
        assert_eq!(shorten_statement(&format!("insert into t values ({})", "1, ".repeat(100))).chars().count(), 120);
    }

    #[test]
    fn dialects_follow_the_family() {
        assert_eq!(dialect_of(Engine::Mariadb), SplitDialect::Mysql);
        assert_eq!(dialect_of(Engine::Cockroachdb), SplitDialect::Postgres);
        assert_eq!(dialect_of(Engine::Mssql), SplitDialect::Mssql);
        assert_eq!(dialect_of(Engine::Oracle), SplitDialect::Oracle);
        assert_eq!(dialect_of(Engine::Sqlite), SplitDialect::Standard);
    }

    #[tokio::test]
    async fn a_file_is_read_in_chunks_and_scanned() {
        let dir = std::env::temp_dir().join(format!("db-free-sqlimport-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let path = dir.join("dump.sql");
        // Multi-byte characters straddle the 64 KiB chunk edge.
        let mut script = String::from("\u{feff}-- dump\nDROP TABLE IF EXISTS t;\nCREATE TABLE t (a text);\n");
        for i in 0..4_000 {
            script.push_str(&format!("INSERT INTO t VALUES ('é世界-{i};');\n"));
        }
        script.push_str("DELETE FROM t;\nDELETE FROM t WHERE a = 'x';\n");
        std::fs::write(&path, script).unwrap_or_else(|e| panic!("{e}"));
        let info = scan(&path, Engine::Mysql).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(info.statements, 4_004);
        assert_eq!(info.destructive, 2, "DROP and a bare DELETE");
        assert_eq!(info.destructive_examples, vec!["DROP TABLE IF EXISTS t", "DELETE FROM t"]);
        assert!(info.problem.is_none(), "{:?}", info.problem);
        assert!(info.head.starts_with("-- dump"));
        let mut file = SqlFile::open(&path, SplitDialect::Mysql).await.unwrap_or_else(|e| panic!("{e}"));
        let mut n = 0;
        let mut last = String::new();
        while let Some(statement) = file.next().await.unwrap_or_else(|e| panic!("{e}")) {
            n += 1;
            last = statement.text;
        }
        assert_eq!(n, 4_004);
        assert_eq!(last, "DELETE FROM t WHERE a = 'x'");
        assert_eq!(file.bytes_read, file.size);

        let bad = dir.join("bad.sql");
        std::fs::write(&bad, b"select 1;\n\xff\xfe select 2;").unwrap_or_else(|e| panic!("{e}"));
        let info = scan(&bad, Engine::Mysql).await.unwrap_or_else(|e| panic!("{e}"));
        assert!(info.problem.is_some(), "invalid UTF-8 is a problem, not a panic");
        let escaping = dir.join("esc.sql");
        std::fs::write(&escaping, b"SET sql_mode = 'NO_BACKSLASH_ESCAPES';\nselect 1;").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(scan(&escaping, Engine::Mysql).await.unwrap_or_else(|e| panic!("{e}")).problem.as_deref(), Some(ESCAPING_PROBLEM));
        let info = scan(&path, Engine::Mysql).await.unwrap_or_else(|e| panic!("{e}"));
        assert!(verify_unchanged(&path, info.size, info.modified_ms).await.is_ok());
        assert!(verify_unchanged(&path, info.size + 1, info.modified_ms).await.is_err(), "a changed size is refused");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
