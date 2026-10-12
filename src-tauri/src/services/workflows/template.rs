// SOT: workflow-template, template-placeholders, template-arithmetic, sql-safe-substitution, rows-formatters, date-tokens

use super::data::{cell_number, cell_text, format_number, Dataset};
use crate::error::{AppError, AppResult};
use crate::model::{Engine, Family, Value};
use crate::services::changes;
use chrono::{DateTime, Utc};

// ============================================================================
// WORKFLOW TEMPLATES
//
// WHAT:  `{{column}}`, `{{a * 2 + 1}}`, `{{rows.table}}` and `{{date.today}}`
//        placeholders inside a node's text fields, filled from the rows that
//        reach the node.
// WHY:   The one place that decides what a placeholder means, so a Query, an
//        HTTP call and an email read the same syntax the same way.
// HOW:   `{{name}}` reads the first input row; `{{rows…}}` formats the whole
//        input (at most 50 rows); arithmetic (`+ - * / %`, parentheses, unary
//        minus) works on numeric columns; `{{date.now|today|time|unix}}` is the
//        current UTC time. Text fields render a missing value as an empty
//        string. SQL never receives pasted text: `render_sql` writes a literal
//        quoted for the engine, or escapes the value in place when the
//        placeholder already sits inside a quoted string or identifier.
// WHERE: src-tauri/src/services/workflows/nodes
// ============================================================================

/// Most rows `{{rows.*}}` prints; a footer counts the rest.
pub const MAX_ROWS_IN_TEXT: usize = 50;

#[derive(Debug, Clone, PartialEq)]
enum Piece<'a> {
    Text(&'a str),
    Expr(&'a str),
}

// WHAT:  Splits a template into literal text and `{{expressions}}`.
// HOW:   Same rule as the original tool: `{{`, one or more characters that are
//        not `}`, then `}}`. Anything else (an unterminated `{{`) is literal.
fn scan(template: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let bytes = template.as_bytes();
    let mut literal_start = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let content_start = i + 2;
            if let Some(rel) = template[content_start..].find('}') {
                let content_end = content_start + rel;
                if rel > 0 && bytes.get(content_end + 1) == Some(&b'}') {
                    if literal_start < i {
                        pieces.push(Piece::Text(&template[literal_start..i]));
                    }
                    pieces.push(Piece::Expr(&template[content_start..content_end]));
                    i = content_end + 2;
                    literal_start = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    if literal_start < template.len() {
        pieces.push(Piece::Text(&template[literal_start..]));
    }
    pieces
}

/// True when the template holds at least one `{{…}}` placeholder.
pub fn has_placeholders(template: &str) -> bool {
    scan(template).iter().any(|p| matches!(p, Piece::Expr(_)))
}

/// Fills a text field. A placeholder that cannot be resolved renders empty.
pub fn render_text(template: &str, data: &Dataset) -> String {
    render_text_at(template, data, Utc::now())
}

fn render_text_at(template: &str, data: &Dataset, now: DateTime<Utc>) -> String {
    let mut out = String::with_capacity(template.len());
    for piece in scan(template) {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Expr(expr) => {
                if let Ok(value) = eval(expr.trim(), data, now) {
                    out.push_str(&cell_text(&value));
                }
            }
        }
    }
    out
}

// WHAT:  Evaluates one placeholder body to a value.
// HOW:   Reserved names first (`rows…`, `date.…`), then an exact column name, then
//        arithmetic over numeric columns.
fn eval(expr: &str, data: &Dataset, now: DateTime<Utc>) -> Result<Value, String> {
    match expr {
        "rows" | "rows.table" => return Ok(Value::Text(rows_table(data))),
        "rows.json" => return Ok(Value::Text(rows_json(data))),
        "rows.list" => return Ok(Value::Text(rows_list(data))),
        "rows.html" => return Ok(Value::Text(rows_html(data))),
        "rows.count" => return Ok(Value::Int(i64::try_from(data.rows.len()).unwrap_or(i64::MAX))),
        "date.now" => return Ok(Value::DateTime(now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))),
        "date.today" => return Ok(Value::DateTime(now.format("%Y-%m-%d").to_string())),
        "date.time" => return Ok(Value::DateTime(now.format("%H:%M:%S").to_string())),
        "date.unix" => return Ok(Value::Int(now.timestamp())),
        _ => {}
    }
    if let Some(index) = data.index_of(expr) {
        return Ok(data.rows.first().and_then(|row| row.get(index)).cloned().unwrap_or(Value::Null));
    }
    if is_identifier(expr) {
        return Ok(Value::Null);
    }
    let result = evaluate_arithmetic(expr, data)?;
    Ok(number_value(result))
}

fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        Value::Int(n as i64)
    } else {
        Value::Float(n)
    }
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ---------------------------------------------------------------- arithmetic
struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    data: &'a Dataset,
}

// WHAT:  `+ - * / %`, parentheses, unary minus, numbers and column names.
// WHY:   `LIMIT 100 OFFSET {{i * 100}}` needs arithmetic, not a script engine.
pub fn evaluate_arithmetic(expr: &str, data: &Dataset) -> Result<f64, String> {
    let mut parser = Parser { chars: expr.chars().collect(), pos: 0, data };
    let value = parser.expression()?;
    parser.skip_space();
    if parser.pos < parser.chars.len() {
        return Err(format!("unexpected \"{}\"", parser.chars[parser.pos]));
    }
    if value.is_finite() {
        Ok(value)
    } else {
        Err("the result is not a finite number".to_string())
    }
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.chars.get(self.pos).is_some_and(|c| c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, ch: char) -> bool {
        self.skip_space();
        if self.chars.get(self.pos) == Some(&ch) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expression(&mut self) -> Result<f64, String> {
        let mut left = self.term()?;
        loop {
            if self.eat('+') {
                left += self.term()?;
            } else if self.eat('-') {
                left -= self.term()?;
            } else {
                return Ok(left);
            }
        }
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut left = self.factor()?;
        loop {
            if self.eat('*') {
                left *= self.factor()?;
            } else if self.eat('/') {
                let right = self.factor()?;
                if right == 0.0 {
                    return Err("division by zero".to_string());
                }
                left /= right;
            } else if self.eat('%') {
                let right = self.factor()?;
                if right == 0.0 {
                    return Err("division by zero".to_string());
                }
                left %= right;
            } else {
                return Ok(left);
            }
        }
    }

    fn factor(&mut self) -> Result<f64, String> {
        self.skip_space();
        if self.eat('(') {
            let value = self.expression()?;
            if !self.eat(')') {
                return Err("expected )".to_string());
            }
            return Ok(value);
        }
        if self.eat('-') {
            return Ok(-self.factor()?);
        }
        let start = self.pos;
        match self.chars.get(self.pos) {
            Some(c) if c.is_ascii_digit() => {
                while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
                if self.chars.get(self.pos) == Some(&'.') && self.chars.get(self.pos + 1).is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                    while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_digit()) {
                        self.pos += 1;
                    }
                }
                let text: String = self.chars[start..self.pos].iter().collect();
                text.parse::<f64>().map_err(|_| format!("\"{text}\" is not a number"))
            }
            Some(c) if c.is_ascii_alphabetic() || *c == '_' => {
                while self.chars.get(self.pos).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_') {
                    self.pos += 1;
                }
                let name: String = self.chars[start..self.pos].iter().collect();
                self.data
                    .first_cell(&name)
                    .and_then(cell_number)
                    .ok_or_else(|| format!("\"{name}\" is not a number in the first input row"))
            }
            Some(c) => Err(format!("unexpected \"{c}\"")),
            None => Err("unexpected end of expression".to_string()),
        }
    }
}

// ---------------------------------------------------------------- rows formatters
fn visible_rows(data: &Dataset) -> (&[Vec<Value>], usize) {
    let shown = data.rows.len().min(MAX_ROWS_IN_TEXT);
    (&data.rows[..shown], data.rows.len() - shown)
}

fn table_cell(value: &Value) -> String {
    cell_text(value).replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn rows_table(data: &Dataset) -> String {
    if data.columns.is_empty() {
        return String::new();
    }
    let (rows, hidden) = visible_rows(data);
    let names: Vec<&str> = data.column_names();
    let mut lines = vec![format!("| {} |", names.join(" | ")), format!("| {} |", names.iter().map(|_| "---").collect::<Vec<_>>().join(" | "))];
    for row in rows {
        lines.push(format!("| {} |", row.iter().map(table_cell).collect::<Vec<_>>().join(" | ")));
    }
    let block = format!("```\n{}\n```", lines.join("\n"));
    if hidden > 0 {
        format!("{block}\n… and {hidden} more")
    } else {
        block
    }
}

fn rows_json(data: &Dataset) -> String {
    let (_, hidden) = visible_rows(data);
    let shown: Vec<serde_json::Value> = data.to_json_rows().into_iter().take(MAX_ROWS_IN_TEXT).collect();
    let pretty = serde_json::to_string_pretty(&shown).unwrap_or_else(|_| "[]".to_string());
    let block = format!("```\n{pretty}\n```");
    if hidden > 0 {
        format!("{block}\n… and {hidden} more")
    } else {
        block
    }
}

fn rows_list(data: &Dataset) -> String {
    if data.rows.is_empty() || data.columns.is_empty() {
        return String::new();
    }
    let (rows, hidden) = visible_rows(data);
    let lines: Vec<String> = rows.iter().map(|row| format!("• {}", row.iter().map(table_cell_plain).collect::<Vec<_>>().join(", "))).collect();
    let block = lines.join("\n");
    if hidden > 0 {
        format!("{block}\n… and {hidden} more")
    } else {
        block
    }
}

fn table_cell_plain(value: &Value) -> String {
    cell_text(value).replace(['\n', '\r'], " ")
}

pub fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn rows_html(data: &Dataset) -> String {
    if data.columns.is_empty() {
        return String::new();
    }
    let (rows, hidden) = visible_rows(data);
    let th = "style=\"text-align:left;padding:6px 10px;border-bottom:1px solid #ddd;background:#f6f6f6;font-weight:600\"";
    let td = "style=\"padding:6px 10px;border-bottom:1px solid #eee\"";
    let header: String = data.columns.iter().map(|c| format!("<th {th}>{}</th>", escape_html(&c.name))).collect();
    let body: String = rows
        .iter()
        .map(|row| format!("<tr>{}</tr>", row.iter().map(|v| format!("<td {td}>{}</td>", escape_html(&cell_text(v)))).collect::<String>()))
        .collect();
    let table = format!("<table style=\"border-collapse:collapse;font-family:sans-serif;font-size:13px\"><tr>{header}</tr>{body}</table>");
    if hidden > 0 {
        format!("{table}<p style=\"color:#666;font-size:12px;margin:6px 0 0\">… and {hidden} more</p>")
    } else {
        table
    }
}

// ---------------------------------------------------------------- SQL
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqlState {
    Code,
    Single,
    Double,
    Backtick,
    LineComment,
    BlockComment,
}

/// Engines whose string literals treat a backslash as an escape.
fn backslash_escapes(engine: Engine) -> bool {
    matches!(engine.family(), Family::Mysql | Family::Clickhouse | Family::Snowflake | Family::Bigquery)
}

// WHAT:  Fills a SQL template so a data value can never become SQL.
// WHY:   A value read from a row, a file or an API is data. Pasting it raw into a
//        statement is an injection hole; this is the only way workflow SQL is built.
// HOW:   The template is scanned like SQL (strings, quoted identifiers,
//        comments). A placeholder outside any quote is written as a literal for the
//        engine (`NULL`, a number, a quoted string); one inside quotes is escaped
//        in place, so both `WHERE id = {{id}}` and `WHERE name = '{{name}}'` work.
//        An expression that cannot be evaluated is an error, never a blank.
pub fn render_sql(template: &str, data: &Dataset, engine: Engine) -> AppResult<String> {
    render_sql_at(template, data, engine, Utc::now())
}

fn render_sql_at(template: &str, data: &Dataset, engine: Engine, now: DateTime<Utc>) -> AppResult<String> {
    let backslash = backslash_escapes(engine);
    let mut out = String::with_capacity(template.len());
    let mut state = SqlState::Code;
    for piece in scan(template) {
        match piece {
            Piece::Text(text) => {
                state = copy_sql_text(text, state, backslash, &mut out);
            }
            Piece::Expr(expr) => {
                let expr = expr.trim();
                let value = eval(expr, data, now).map_err(|reason| AppError::invalid_input(format!("Cannot evaluate {{{{{expr}}}}}: {reason}")))?;
                out.push_str(&splice(&value, state, engine, backslash));
            }
        }
    }
    Ok(out)
}

/// Copies literal SQL text through, tracking whether the end of it is inside a quote or comment.
fn copy_sql_text(text: &str, mut state: SqlState, backslash: bool, out: &mut String) -> SqlState {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        out.push(c);
        match state {
            SqlState::Code => match (c, next) {
                ('\'', _) => state = SqlState::Single,
                ('"', _) => state = SqlState::Double,
                ('`', _) => state = SqlState::Backtick,
                ('-', Some('-')) => {
                    out.push('-');
                    i += 1;
                    state = SqlState::LineComment;
                }
                ('/', Some('*')) => {
                    out.push('*');
                    i += 1;
                    state = SqlState::BlockComment;
                }
                _ => {}
            },
            SqlState::Single | SqlState::Double => {
                let quote = if state == SqlState::Single { '\'' } else { '"' };
                if c == '\\' && backslash {
                    if let Some(escaped) = next {
                        out.push(escaped);
                        i += 1;
                    }
                } else if c == quote {
                    if next == Some(quote) {
                        out.push(quote);
                        i += 1;
                    } else {
                        state = SqlState::Code;
                    }
                }
            }
            SqlState::Backtick => {
                if c == '`' {
                    if next == Some('`') {
                        out.push('`');
                        i += 1;
                    } else {
                        state = SqlState::Code;
                    }
                }
            }
            SqlState::LineComment => {
                if c == '\n' {
                    state = SqlState::Code;
                }
            }
            SqlState::BlockComment => {
                if c == '*' && next == Some('/') {
                    out.push('/');
                    i += 1;
                    state = SqlState::Code;
                }
            }
        }
        i += 1;
    }
    state
}

fn splice(value: &Value, state: SqlState, engine: Engine, backslash: bool) -> String {
    match state {
        SqlState::Code => sql_literal(value, engine, backslash),
        SqlState::Single => escape_string(&cell_text(value), '\'', backslash),
        SqlState::Double => escape_string(&cell_text(value), '"', backslash),
        SqlState::Backtick => cell_text(value).replace('`', "``"),
        SqlState::LineComment => cell_text(value).replace(['\r', '\n'], " "),
        SqlState::BlockComment => cell_text(value).replace("*/", "* /"),
    }
}

fn escape_string(text: &str, quote: char, backslash: bool) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for c in text.chars() {
        if c == quote {
            out.push(quote);
            out.push(quote);
        } else if c == '\\' && backslash {
            out.push_str("\\\\");
        } else {
            out.push(c);
        }
    }
    out
}

/// A value as a SQL literal for the engine.
pub fn sql_literal(value: &Value, engine: Engine, backslash: bool) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(_) | Value::Bytes(_) => changes::literal(engine, value),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            if f.is_finite() {
                format_number(*f)
            } else {
                "NULL".to_string()
            }
        }
        Value::Decimal(s) if s.trim().parse::<f64>().is_ok_and(f64::is_finite) => s.trim().to_string(),
        other => format!("'{}'", escape_string(&cell_text(other), '\'', backslash)),
    }
}

/// A value as a SQL literal, with the engine's own escaping rules.
pub fn literal_for(engine: Engine, value: &Value) -> String {
    sql_literal(value, engine, backslash_escapes(engine))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ColumnMeta;

    fn data(cols: &[&str], row: Vec<Value>) -> Dataset {
        Dataset { columns: cols.iter().map(|c| ColumnMeta { name: (*c).into(), type_name: "any".into() }).collect(), rows: vec![row] }
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-03-04T05:06:07Z").unwrap().with_timezone(&Utc)
    }

    #[test]
    fn scan_splits_text_and_expressions() {
        assert_eq!(scan("a {{x}} b {{ y }}"), vec![Piece::Text("a "), Piece::Expr("x"), Piece::Text(" b "), Piece::Expr(" y ")]);
        assert_eq!(scan("{{x"), vec![Piece::Text("{{x")]);
        assert_eq!(scan("{{}}"), vec![Piece::Text("{{}}")]);
        assert_eq!(scan("{{a}b}}"), vec![Piece::Text("{{a}b}}")]);
        assert_eq!(scan("{{{x}}}"), vec![Piece::Expr("{x"), Piece::Text("}")]);
        assert!(has_placeholders("hi {{name}}"));
        assert!(!has_placeholders("plain"));
    }

    #[test]
    fn text_fills_columns_and_leaves_missing_ones_blank() {
        let d = data(&["name", "n"], vec![Value::Text("Ada".into()), Value::Int(3)]);
        assert_eq!(render_text_at("Hi {{name}} x{{n}} [{{nope}}]", &d, now()), "Hi Ada x3 []");
    }

    #[test]
    fn arithmetic_uses_numeric_columns() {
        let d = data(&["i", "total", "price"], vec![Value::Int(2), Value::Int(5), Value::Text("1.5".into())]);
        assert_eq!(render_text_at("{{i * 100}}", &d, now()), "200");
        assert_eq!(render_text_at("{{(i + 1) * 10}}", &d, now()), "30");
        assert_eq!(render_text_at("{{-i + total % 3}}", &d, now()), "0");
        assert_eq!(render_text_at("{{price * 2}}", &d, now()), "3");
        assert_eq!(render_text_at("{{total / 2}}", &d, now()), "2.5");
        assert_eq!(render_text_at("[{{total / 0}}]", &d, now()), "[]", "a bad expression renders blank in text");
        assert_eq!(render_text_at("[{{1 +}}]", &d, now()), "[]");
    }

    #[test]
    fn date_tokens_are_utc() {
        let d = Dataset::empty();
        assert_eq!(render_text_at("{{date.today}} {{date.time}}", &d, now()), "2026-03-04 05:06:07");
        assert_eq!(render_text_at("{{date.unix}}", &d, now()), "1772600767");
        assert_eq!(render_text_at("{{date.now}}", &d, now()), "2026-03-04T05:06:07.000Z");
    }

    #[test]
    fn rows_formatters_cap_at_fifty() {
        let mut d = Dataset { columns: vec![ColumnMeta { name: "a".into(), type_name: "any".into() }], rows: Vec::new() };
        for i in 0..53 {
            d.rows.push(vec![Value::Int(i)]);
        }
        let table = render_text_at("{{rows.table}}", &d, now());
        assert!(table.starts_with("```\n| a |\n| --- |\n| 0 |"));
        assert!(table.ends_with("```\n… and 3 more"));
        assert_eq!(render_text_at("{{rows.count}}", &d, now()), "53");
        assert_eq!(render_text_at("{{rows}}", &d, now()), table);
        let list = render_text_at("{{rows.list}}", &d, now());
        assert!(list.starts_with("• 0\n• 1") && list.ends_with("… and 3 more"));
        let json = render_text_at("{{rows.json}}", &d, now());
        assert!(json.starts_with("```\n[\n  {\n    \"a\": 0") && json.contains("… and 3 more"));
        let html = render_text_at("{{rows.html}}", &d, now());
        assert!(html.starts_with("<table") && html.contains("<th ") && html.contains("… and 3 more</p>"));
    }

    #[test]
    fn html_rows_escape_cells() {
        let d = data(&["x"], vec![Value::Text("<b>&\"'".into())]);
        let html = render_text_at("{{rows.html}}", &d, now());
        assert!(html.contains("&lt;b&gt;&amp;&quot;&#39;"), "{html}");
        let table = render_text_at("{{rows.table}}", &data(&["x"], vec![Value::Text("a|b\nc".into())]), now());
        assert!(table.contains("| a\\|b c |"), "{table}");
    }

    #[test]
    fn sql_writes_literals_outside_quotes() {
        let d = data(&["id", "name", "gone", "ok"], vec![Value::Int(7), Value::Text("O'Brien".into()), Value::Null, Value::Bool(true)]);
        let sql = render_sql_at("SELECT * FROM t WHERE id = {{id}} AND n = {{name}} AND g = {{gone}} AND ok = {{ok}}", &d, Engine::Postgres, now()).unwrap();
        assert_eq!(sql, "SELECT * FROM t WHERE id = 7 AND n = 'O''Brien' AND g = NULL AND ok = TRUE");
        let sqlite = render_sql_at("SELECT {{ok}}", &d, Engine::Sqlite, now()).unwrap();
        assert_eq!(sqlite, "SELECT 1");
    }

    #[test]
    fn sql_escapes_in_place_inside_quotes() {
        let d = data(&["name"], vec![Value::Text("x'; DROP TABLE t; --".into())]);
        let sql = render_sql_at("SELECT * FROM t WHERE n = '{{name}}'", &d, Engine::Postgres, now()).unwrap();
        assert_eq!(sql, "SELECT * FROM t WHERE n = 'x''; DROP TABLE t; --'");
        // Stays inside the string: the statement count is still one.
        assert_eq!(sql.matches('\'').count(), 4);
    }

    #[test]
    fn mysql_escapes_backslashes_too() {
        let d = data(&["name"], vec![Value::Text("a\\' OR 1=1 -- ".into())]);
        let inside = render_sql_at("SELECT * FROM t WHERE n = '{{name}}'", &d, Engine::Mysql, now()).unwrap();
        assert_eq!(inside, "SELECT * FROM t WHERE n = 'a\\\\'' OR 1=1 -- '");
        let outside = render_sql_at("SELECT * FROM t WHERE n = {{name}}", &d, Engine::Mysql, now()).unwrap();
        assert_eq!(outside, "SELECT * FROM t WHERE n = 'a\\\\'' OR 1=1 -- '");
        // Postgres standard strings have no backslash escape.
        let pg = render_sql_at("SELECT {{name}}", &d, Engine::Postgres, now()).unwrap();
        assert_eq!(pg, "SELECT 'a\\'' OR 1=1 -- '");
    }

    #[test]
    fn sql_tracks_identifiers_and_comments() {
        let d = data(&["t", "c"], vec![Value::Text("we\"ird".into()), Value::Text("*/ DROP".into())]);
        let sql = render_sql_at("SELECT 1 FROM \"{{t}}\" /* {{c}} */ -- {{t}}\nWHERE 1=1", &d, Engine::Postgres, now()).unwrap();
        assert_eq!(sql, "SELECT 1 FROM \"we\"\"ird\" /* * / DROP */ -- we\"ird\nWHERE 1=1");
        let my = render_sql_at("SELECT 1 FROM `{{t}}`", &data(&["t"], vec![Value::Text("a`b".into())]), Engine::Mysql, now()).unwrap();
        assert_eq!(my, "SELECT 1 FROM `a``b`");
    }

    #[test]
    fn sql_quote_state_survives_doubled_quotes() {
        let d = data(&["v"], vec![Value::Int(5)]);
        let sql = render_sql_at("SELECT 'it''s {{v}}' , {{v}}", &d, Engine::Postgres, now()).unwrap();
        assert_eq!(sql, "SELECT 'it''s 5' , 5");
    }

    #[test]
    fn sql_arithmetic_and_errors() {
        let d = data(&["i"], vec![Value::Int(3)]);
        assert_eq!(render_sql_at("LIMIT 100 OFFSET {{i * 100}}", &d, Engine::Postgres, now()).unwrap(), "LIMIT 100 OFFSET 300");
        let err = render_sql_at("SELECT {{i / 0}}", &d, Engine::Postgres, now()).err();
        assert!(matches!(err, Some(AppError::InvalidInput { .. })));
        // A missing plain column is NULL, not an error.
        assert_eq!(render_sql_at("SELECT {{missing}}", &d, Engine::Postgres, now()).unwrap(), "SELECT NULL");
        assert_eq!(render_sql_at("SELECT {{date.today}}", &d, Engine::Postgres, now()).unwrap(), "SELECT '2026-03-04'");
    }

    #[test]
    fn exact_column_names_beat_arithmetic() {
        let d = data(&["first name", "a-b"], vec![Value::Text("Ada".into()), Value::Int(9)]);
        assert_eq!(render_text_at("{{first name}} {{a-b}}", &d, now()), "Ada 9");
    }
}
