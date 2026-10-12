// SOT: dynamodb-aggregates, partiql-select-parser, local-group-by, local-order-by

use crate::error::{AppError, AppResult};
use crate::integrations::http::json_to_value;
use crate::model::Value;
use serde_json::Value as Json;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

// ============================================================================
// DYNAMODB LOCAL AGGREGATES
//
// WHAT:  COUNT / SUM / AVG / MIN / MAX, GROUP BY, ORDER BY and LIMIT for a PartiQL
//        SELECT, computed here over the items DynamoDB returned.
// WHY:   DynamoDB's PartiQL has none of them (and ORDER BY only on a sort key
//        with the partition key fixed), so a statement DB Pro answers is a
//        validation error here. The data is reachable, so the work moves to the
//        client: the statement is rewritten to `SELECT * FROM … [WHERE …]`, the
//        adapter reads the matches (capped, never silently truncated), and this
//        module does the arithmetic.
// HOW:   `parse` splits the statement at its top-level clause keywords, reading
//        quotes and brackets so a `WHERE name = 'ORDER BY'` stays one clause.
//        It answers None for a statement DynamoDB can run itself. `evaluate`
//        takes plain JSON documents (attribute values already unwrapped) and
//        returns the result grid. SQL semantics: COUNT(col), SUM, AVG, MIN and MAX
//        skip missing and null values, and SUM / AVG of no values is NULL. NULLs
//        and missing attributes sort last whichever way the sort runs.
// WHERE: src-tauri/src/integrations/dynamodb.rs (`execute`)
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// An unquoted word or keyword, as written.
    Word(String),
    /// A "quoted identifier", unquoted.
    Ident(String),
    Str,
    Num(String),
    Sym(char),
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    start: usize,
    end: usize,
}

fn tokenize(sql: &str) -> Result<Vec<Token>, AppError> {
    let chars: Vec<(usize, char)> = sql.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0_usize;
    let end_of = |idx: usize| chars.get(idx).map(|(b, _)| *b).unwrap_or(sql.len());
    while i < chars.len() {
        let (start, c) = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && chars.get(i + 1).map(|(_, n)| *n) == Some('-') {
            while i < chars.len() && chars[i].1 != '\n' {
                i += 1;
            }
        } else if c == '\'' || c == '"' {
            let mut text = String::new();
            i += 1;
            let mut closed = false;
            while i < chars.len() {
                let ch = chars[i].1;
                if ch == c {
                    if chars.get(i + 1).map(|(_, n)| *n) == Some(c) {
                        text.push(c);
                        i += 2;
                        continue;
                    }
                    closed = true;
                    i += 1;
                    break;
                }
                text.push(ch);
                i += 1;
            }
            if !closed {
                return Err(AppError::invalid_input("The statement has an unterminated quote."));
            }
            out.push(Token { tok: if c == '"' { Tok::Ident(text) } else { Tok::Str }, start, end: end_of(i) });
        } else if c.is_ascii_digit() {
            let mut text = String::new();
            while i < chars.len() && (chars[i].1.is_ascii_digit() || chars[i].1 == '.' || chars[i].1 == 'e' || chars[i].1 == 'E') {
                text.push(chars[i].1);
                i += 1;
            }
            out.push(Token { tok: Tok::Num(text), start, end: end_of(i) });
        } else if c.is_alphabetic() || c == '_' {
            let mut text = String::new();
            while i < chars.len() && (chars[i].1.is_alphanumeric() || chars[i].1 == '_') {
                text.push(chars[i].1);
                i += 1;
            }
            out.push(Token { tok: Tok::Word(text), start, end: end_of(i) });
        } else {
            i += 1;
            out.push(Token { tok: Tok::Sym(c), start, end: end_of(i) });
        }
    }
    Ok(out)
}

fn is_word(token: &Token, word: &str) -> bool {
    matches!(&token.tok, Tok::Word(w) if w.eq_ignore_ascii_case(word))
}

fn is_sym(token: &Token, sym: char) -> bool {
    token.tok == Tok::Sym(sym)
}

/// Indices of the tokens at bracket depth 0.
fn top_level(tokens: &[Token]) -> Vec<bool> {
    let mut depth = 0_i32;
    tokens
        .iter()
        .map(|t| match t.tok {
            Tok::Sym('(' | '[' | '{') => {
                depth += 1;
                false
            }
            Tok::Sym(')' | ']' | '}') => {
                depth -= 1;
                false
            }
            _ => depth == 0,
        })
        .collect()
}

fn split_commas(tokens: &[Token]) -> Vec<&[Token]> {
    let top = top_level(tokens);
    let mut parts = Vec::new();
    let mut from = 0_usize;
    for (i, token) in tokens.iter().enumerate() {
        if top[i] && is_sym(token, ',') {
            parts.push(&tokens[from..i]);
            from = i + 1;
        }
    }
    parts.push(&tokens[from..]);
    parts
}

/// One step of an attribute path: `a.b[2]` is Key(a), Key(b), Index(2).
#[derive(Debug, Clone, PartialEq)]
enum Step {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq)]
struct Path {
    steps: Vec<Step>,
}

impl Path {
    fn name(&self) -> String {
        let mut out = String::new();
        for step in &self.steps {
            match step {
                Step::Key(k) if out.is_empty() => out.push_str(k),
                Step::Key(k) => {
                    out.push('.');
                    out.push_str(k);
                }
                Step::Index(i) => out.push_str(&format!("[{i}]")),
            }
        }
        out
    }

    fn get<'a>(&self, doc: &'a Json) -> Option<&'a Json> {
        let mut current = doc;
        for step in &self.steps {
            current = match step {
                Step::Key(k) => current.get(k)?,
                Step::Index(i) => current.get(*i)?,
            };
        }
        Some(current)
    }
}

fn parse_path(tokens: &[Token]) -> Option<Path> {
    let mut steps = Vec::new();
    let mut i = 0_usize;
    let key = |t: &Token| match &t.tok {
        Tok::Word(w) => Some(w.clone()),
        Tok::Ident(w) => Some(w.clone()),
        _ => None,
    };
    steps.push(Step::Key(key(tokens.first()?)?));
    i += 1;
    while i < tokens.len() {
        if is_sym(&tokens[i], '.') {
            steps.push(Step::Key(key(tokens.get(i + 1)?)?));
            i += 2;
        } else if is_sym(&tokens[i], '[') {
            let Tok::Num(n) = &tokens.get(i + 1)?.tok else { return None };
            if !is_sym(tokens.get(i + 2)?, ']') {
                return None;
            }
            steps.push(Step::Index(n.parse().ok()?));
            i += 3;
        } else {
            return None;
        }
    }
    Some(Path { steps })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Func {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl Func {
    fn parse(word: &str) -> Option<Func> {
        match word.to_ascii_uppercase().as_str() {
            "COUNT" => Some(Func::Count),
            "SUM" => Some(Func::Sum),
            "AVG" => Some(Func::Avg),
            "MIN" => Some(Func::Min),
            "MAX" => Some(Func::Max),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Func::Count => "COUNT",
            Func::Sum => "SUM",
            Func::Avg => "AVG",
            Func::Min => "MIN",
            Func::Max => "MAX",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Aggregate {
    func: Func,
    /// None = `*` (COUNT only).
    arg: Option<Path>,
    distinct: bool,
}

impl Aggregate {
    fn name(&self) -> String {
        let arg = self.arg.as_ref().map(Path::name).unwrap_or_else(|| "*".to_string());
        format!("{}({}{arg})", self.func.label(), if self.distinct { "DISTINCT " } else { "" })
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Star,
    Column { path: Path, alias: Option<String> },
    Aggregate { aggregate: Aggregate, alias: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
enum OrderTarget {
    Path(Path),
    Aggregate(Aggregate),
}

impl OrderTarget {
    fn name(&self) -> String {
        match self {
            OrderTarget::Path(p) => p.name(),
            OrderTarget::Aggregate(a) => a.name(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Order {
    target: OrderTarget,
    desc: bool,
}

// WHAT:  A SELECT split into what DynamoDB runs and what is computed here.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// What the adapter sends: `SELECT * FROM <table> [WHERE <condition>]`.
    pub base_statement: String,
    items: Vec<Item>,
    group_by: Vec<Path>,
    order_by: Vec<Order>,
    pub limit: Option<usize>,
    aggregated: bool,
}

impl Plan {
    /// Aggregates and GROUP BY need every matching item; a bare ORDER BY does too,
    /// while a bare LIMIT can stop reading as soon as it has enough.
    pub fn needs_every_item(&self) -> bool {
        self.aggregated || !self.order_by.is_empty()
    }

    /// DynamoDB can run an ORDER BY itself on a sort key; only that one is worth trying first.
    pub fn is_order_by_only(&self) -> bool {
        !self.aggregated && !self.order_by.is_empty() && self.limit.is_none()
    }
}

const CLAUSES: [&str; 5] = ["WHERE", "GROUP", "HAVING", "ORDER", "LIMIT"];

// WHAT:  Reads a statement; Some(plan) when it needs local work, None when
//        DynamoDB can run it as written (or it is not a SELECT at all).
// HOW:   An aggregate or GROUP BY always needs local work. ORDER BY and LIMIT
//        do too, but only for a plain column list: anything fancier is left to
//        DynamoDB, which will explain what it does not support.
pub fn parse(statement: &str) -> AppResult<Option<Plan>> {
    let text = statement.trim().trim_end_matches(';').trim();
    let tokens = tokenize(text)?;
    if !tokens.first().is_some_and(|t| is_word(t, "SELECT")) {
        return Ok(None);
    }
    let top = top_level(&tokens);
    let find = |from: usize, word: &str| (from..tokens.len()).find(|&i| top[i] && is_word(&tokens[i], word));
    let Some(from_at) = find(1, "FROM") else { return Ok(None) };

    // Where each clause after FROM starts, in the order they appear.
    let mut starts: Vec<(usize, &str)> = Vec::new();
    for clause in CLAUSES {
        if let Some(i) = find(from_at + 1, clause) {
            starts.push((i, clause));
        }
    }
    starts.sort_by_key(|(i, _)| *i);
    let table_end = starts.first().map(|(i, _)| *i).unwrap_or(tokens.len());
    let clause_tokens = |name: &str| -> Option<&[Token]> {
        let pos = starts.iter().position(|(_, n)| *n == name)?;
        let begin = starts[pos].0 + 1;
        let end = starts.get(pos + 1).map(|(i, _)| *i).unwrap_or(tokens.len());
        Some(&tokens[begin..end])
    };

    let select_tokens = &tokens[1..from_at];
    let items = parse_items(select_tokens);
    let aggregated_items = items.iter().any(|i| matches!(i, Some(Item::Aggregate { .. })));
    let group_tokens = clause_tokens("GROUP");
    let order_tokens = clause_tokens("ORDER");
    let limit_tokens = clause_tokens("LIMIT");
    let aggregated = aggregated_items || group_tokens.is_some();
    if !aggregated && order_tokens.is_none() && limit_tokens.is_none() {
        return Ok(None);
    }
    if clause_tokens("HAVING").is_some() {
        return Err(AppError::invalid_input("HAVING is not supported: DynamoDB PartiQL has no aggregates to filter on. Filter the grid after the query instead."));
    }

    let mut resolved = Vec::new();
    for (item, raw) in items.iter().zip(split_commas(select_tokens)) {
        match item {
            Some(item) => resolved.push(item.clone()),
            None if aggregated => {
                return Err(AppError::invalid_input(format!(
                    "\"{}\" can't be used with aggregates here. A select list may hold columns, *, COUNT, SUM, AVG, MIN and MAX.",
                    token_text(text, raw)
                )))
            }
            // A function or expression DynamoDB understands: leave the statement to it.
            None => return Ok(None),
        }
    }

    let group_by = match group_tokens {
        Some(tokens) => {
            let rest = match tokens.first() {
                Some(by) if is_word(by, "BY") => &tokens[1..],
                _ => return Err(AppError::invalid_input("GROUP must be followed by BY.")),
            };
            split_commas(rest)
                .into_iter()
                .map(|part| parse_path(part).ok_or_else(|| AppError::invalid_input(format!("GROUP BY takes columns; \"{}\" is not one.", token_text(text, part)))))
                .collect::<AppResult<Vec<_>>>()?
        }
        None => Vec::new(),
    };

    let order_by = match order_tokens {
        Some(tokens) => {
            let rest = match tokens.first() {
                Some(by) if is_word(by, "BY") => &tokens[1..],
                _ => return Err(AppError::invalid_input("ORDER must be followed by BY.")),
            };
            split_commas(rest).into_iter().map(|part| parse_order(text, part)).collect::<AppResult<Vec<_>>>()?
        }
        None => Vec::new(),
    };

    let limit = match limit_tokens {
        Some([Token { tok: Tok::Num(n), .. }]) => Some(n.parse::<usize>().map_err(|_| AppError::invalid_input("LIMIT takes a whole number."))?),
        Some(_) => return Err(AppError::invalid_input("LIMIT takes a whole number.")),
        None => None,
    };

    if aggregated {
        check_grouping(&resolved, &group_by)?;
    }

    let table_text = token_text(text, &tokens[from_at + 1..table_end]);
    if table_text.is_empty() {
        return Err(AppError::invalid_input("SELECT needs a table after FROM."));
    }
    let mut base_statement = format!("SELECT * FROM {table_text}");
    if let Some(condition) = clause_tokens("WHERE") {
        let condition = token_text(text, condition);
        if !condition.is_empty() {
            base_statement.push_str(" WHERE ");
            base_statement.push_str(&condition);
        }
    }
    Ok(Some(Plan { base_statement, items: resolved, group_by, order_by, limit, aggregated }))
}

fn token_text(text: &str, tokens: &[Token]) -> String {
    match (tokens.first(), tokens.last()) {
        (Some(first), Some(last)) => text.get(first.start..last.end).unwrap_or_default().trim().to_string(),
        _ => String::new(),
    }
}

fn parse_items(tokens: &[Token]) -> Vec<Option<Item>> {
    split_commas(tokens).into_iter().map(parse_item).collect()
}

fn parse_item(part: &[Token]) -> Option<Item> {
    if part.is_empty() {
        return None;
    }
    // `expr AS alias`
    let (body, alias) = match part {
        [rest @ .., as_tok, name] if is_word(as_tok, "AS") => {
            let alias = match &name.tok {
                Tok::Word(w) | Tok::Ident(w) => w.clone(),
                _ => return None,
            };
            (rest, Some(alias))
        }
        _ => (part, None),
    };
    if let [only] = body {
        if is_sym(only, '*') && alias.is_none() {
            return Some(Item::Star);
        }
    }
    if let Some(aggregate) = parse_aggregate(body) {
        return Some(Item::Aggregate { aggregate, alias });
    }
    parse_path(body).map(|path| Item::Column { path, alias })
}

fn parse_aggregate(tokens: &[Token]) -> Option<Aggregate> {
    let [name, open, args @ .., close] = tokens else { return None };
    let Tok::Word(word) = &name.tok else { return None };
    let func = Func::parse(word)?;
    if !is_sym(open, '(') || !is_sym(close, ')') {
        return None;
    }
    match args {
        [star] if is_sym(star, '*') && func == Func::Count => Some(Aggregate { func, arg: None, distinct: false }),
        [distinct, rest @ ..] if is_word(distinct, "DISTINCT") => parse_path(rest).map(|path| Aggregate { func, arg: Some(path), distinct: true }),
        rest => parse_path(rest).map(|path| Aggregate { func, arg: Some(path), distinct: false }),
    }
}

fn parse_order(text: &str, part: &[Token]) -> AppResult<Order> {
    let (body, desc) = match part.last() {
        Some(last) if is_word(last, "DESC") => (&part[..part.len() - 1], true),
        Some(last) if is_word(last, "ASC") => (&part[..part.len() - 1], false),
        _ => (part, false),
    };
    let target = match parse_aggregate(body) {
        Some(aggregate) => OrderTarget::Aggregate(aggregate),
        None => OrderTarget::Path(
            parse_path(body).ok_or_else(|| AppError::invalid_input(format!("ORDER BY takes columns; \"{}\" is not one.", token_text(text, part))))?,
        ),
    };
    Ok(Order { target, desc })
}

// WHAT:  Every plain column of an aggregated SELECT must be a GROUP BY column.
fn check_grouping(items: &[Item], group_by: &[Path]) -> AppResult<()> {
    for item in items {
        match item {
            Item::Star => return Err(AppError::invalid_input("SELECT * can't be combined with aggregates or GROUP BY.")),
            Item::Column { path, .. } if !group_by.contains(path) => {
                return Err(AppError::invalid_input(format!("\"{}\" must appear in GROUP BY or be used in an aggregate.", path.name())))
            }
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// A result grid: `(name, type)` per column and the rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub columns: Vec<(String, String)>,
    pub rows: Vec<Vec<Value>>,
}

fn kind_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "boolean",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "list",
        Json::Object(_) => "map",
    }
}

fn rank(value: &Json) -> u8 {
    match value {
        Json::Null => 0,
        Json::Bool(_) => 1,
        Json::Number(_) => 2,
        Json::String(_) => 3,
        Json::Array(_) => 4,
        Json::Object(_) => 5,
    }
}

// WHAT:  A total order over JSON values: numbers numerically, text lexically,
//        and values of different kinds by kind.
fn compare(a: &Json, b: &Json) -> Ordering {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => match (x.as_i64(), y.as_i64()) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => x.as_f64().unwrap_or(f64::NAN).partial_cmp(&y.as_f64().unwrap_or(f64::NAN)).unwrap_or(Ordering::Equal),
        },
        (Json::String(x), Json::String(y)) => x.cmp(y),
        (Json::Bool(x), Json::Bool(y)) => x.cmp(y),
        (Json::Array(_) | Json::Object(_), Json::Array(_) | Json::Object(_)) if rank(a) == rank(b) => a.to_string().cmp(&b.to_string()),
        _ => rank(a).cmp(&rank(b)),
    }
}

fn present(v: Option<&Json>) -> Option<&Json> {
    v.filter(|v| !v.is_null())
}

// WHAT:  The order two values sort in. NULLs and missing attributes sort last
//        whichever way the sort runs.
// WHY:   Items are schemaless, so a missing attribute is routine. Putting it first
//        on a descending sort would bury the "top 10" under items that have nothing
//        to rank.
fn compare_directed(a: Option<&Json>, b: Option<&Json>, desc: bool) -> Ordering {
    match (present(a), present(b)) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let ordering = compare(x, y);
            if desc {
                ordering.reverse()
            } else {
                ordering
            }
        }
    }
}

enum Acc {
    Count(u64),
    CountDistinct(HashSet<String>),
    Sum { ints: i128, floats: f64, any_float: bool, n: u64 },
    Avg { total: f64, n: u64 },
    Min(Option<Json>),
    Max(Option<Json>),
}

impl Acc {
    fn new(aggregate: &Aggregate) -> Acc {
        match (aggregate.func, aggregate.distinct) {
            (Func::Count, true) => Acc::CountDistinct(HashSet::new()),
            (Func::Count, false) => Acc::Count(0),
            (Func::Sum, _) => Acc::Sum { ints: 0, floats: 0.0, any_float: false, n: 0 },
            (Func::Avg, _) => Acc::Avg { total: 0.0, n: 0 },
            (Func::Min, _) => Acc::Min(None),
            (Func::Max, _) => Acc::Max(None),
        }
    }

    fn add(&mut self, aggregate: &Aggregate, doc: &Json) -> AppResult<()> {
        let value = aggregate.arg.as_ref().and_then(|path| path.get(doc)).filter(|v| !v.is_null());
        match self {
            // COUNT(*) counts items; COUNT(col) counts the items that have the column.
            Acc::Count(n) => {
                if aggregate.arg.is_none() || value.is_some() {
                    *n += 1;
                }
            }
            Acc::CountDistinct(seen) => {
                if let Some(v) = value {
                    seen.insert(v.to_string());
                }
            }
            Acc::Sum { ints, floats, any_float, n } => {
                if let Some(v) = value {
                    let number = numeric(aggregate, v)?;
                    match (number.as_i64(), number.as_u64()) {
                        (Some(i), _) => *ints += i128::from(i),
                        (None, Some(u)) => *ints += i128::from(u),
                        _ => {
                            *floats += number.as_f64().unwrap_or(0.0);
                            *any_float = true;
                        }
                    }
                    *n += 1;
                }
            }
            Acc::Avg { total, n } => {
                if let Some(v) = value {
                    *total += numeric(aggregate, v)?.as_f64().unwrap_or(0.0);
                    *n += 1;
                }
            }
            Acc::Min(best) => {
                if let Some(v) = value {
                    if best.as_ref().is_none_or(|b| compare(v, b) == Ordering::Less) {
                        *best = Some(v.clone());
                    }
                }
            }
            Acc::Max(best) => {
                if let Some(v) = value {
                    if best.as_ref().is_none_or(|b| compare(v, b) == Ordering::Greater) {
                        *best = Some(v.clone());
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> (Value, &'static str) {
        match self {
            Acc::Count(n) => (Value::Int(i64::try_from(n).unwrap_or(i64::MAX)), "number"),
            Acc::CountDistinct(seen) => (Value::Int(i64::try_from(seen.len()).unwrap_or(i64::MAX)), "number"),
            Acc::Sum { n: 0, .. } => (Value::Null, "number"),
            Acc::Sum { ints, floats, any_float, .. } => {
                if any_float {
                    (Value::Float(ints as f64 + floats), "number")
                } else {
                    match i64::try_from(ints) {
                        Ok(i) => (Value::Int(i), "number"),
                        Err(_) => (Value::Decimal(ints.to_string()), "number"),
                    }
                }
            }
            Acc::Avg { n: 0, .. } => (Value::Null, "number"),
            Acc::Avg { total, n } => (Value::Float(total / n as f64), "number"),
            Acc::Min(best) | Acc::Max(best) => match best {
                Some(v) => (json_to_value(&v), kind_name(&v)),
                None => (Value::Null, "null"),
            },
        }
    }
}

fn numeric<'a>(aggregate: &Aggregate, value: &'a Json) -> AppResult<&'a serde_json::Number> {
    match value {
        Json::Number(n) => Ok(n),
        other => Err(AppError::invalid_input(format!(
            "{} needs numbers, but {} holds {} in some items.",
            aggregate.name(),
            aggregate.arg.as_ref().map(Path::name).unwrap_or_default(),
            kind_name(other)
        ))),
    }
}

// WHAT:  Runs the plan over the documents and returns the grid.
// WHY:   `docs` are the items as plain JSON objects (attribute values unwrapped).
pub fn evaluate(plan: &Plan, docs: &[Json]) -> AppResult<Grid> {
    if plan.aggregated {
        evaluate_aggregated(plan, docs)
    } else {
        evaluate_plain(plan, docs)
    }
}

fn evaluate_plain(plan: &Plan, docs: &[Json]) -> AppResult<Grid> {
    let mut sorted: Vec<&Json> = docs.iter().collect();
    for order in plan.order_by.iter().rev() {
        let OrderTarget::Path(path) = &order.target else {
            return Err(AppError::invalid_input("ORDER BY with an aggregate needs an aggregate select list."));
        };
        sorted.sort_by(|a, b| compare_directed(path.get(a), path.get(b), order.desc));
    }
    if let Some(limit) = plan.limit {
        sorted.truncate(limit);
    }

    // Columns: `*` expands to every top-level attribute in first-seen order.
    let mut columns: Vec<(String, Option<Path>)> = Vec::new();
    for item in &plan.items {
        match item {
            Item::Star => {
                for doc in &sorted {
                    for key in doc.as_object().into_iter().flat_map(|o| o.keys()) {
                        if !columns.iter().any(|(name, _)| name == key) {
                            columns.push((key.clone(), Some(Path { steps: vec![Step::Key(key.clone())] })));
                        }
                    }
                }
            }
            Item::Column { path, alias } => columns.push((alias.clone().unwrap_or_else(|| path.name()), Some(path.clone()))),
            Item::Aggregate { .. } => return Err(AppError::invalid_input("Aggregates need an aggregate select list.")),
        }
    }
    let rows: Vec<Vec<Value>> = sorted
        .iter()
        .map(|doc| columns.iter().map(|(_, path)| path.as_ref().and_then(|p| p.get(doc)).map(json_to_value).unwrap_or(Value::Null)).collect())
        .collect();
    let types = columns
        .iter()
        .map(|(_, path)| {
            let seen = sorted.iter().filter_map(|doc| path.as_ref().and_then(|p| p.get(doc))).find(|v| !v.is_null());
            seen.map(kind_name).unwrap_or("null").to_string()
        })
        .collect::<Vec<_>>();
    Ok(Grid { columns: columns.into_iter().map(|(n, _)| n).zip(types).collect(), rows })
}

fn evaluate_aggregated(plan: &Plan, docs: &[Json]) -> AppResult<Grid> {
    // The aggregates of the select list, then any only ORDER BY mentions.
    let mut aggregates: Vec<Aggregate> = Vec::new();
    for item in &plan.items {
        if let Item::Aggregate { aggregate, .. } = item {
            aggregates.push(aggregate.clone());
        }
    }
    for order in &plan.order_by {
        if let OrderTarget::Aggregate(aggregate) = &order.target {
            if !aggregates.contains(aggregate) {
                aggregates.push(aggregate.clone());
            }
        }
    }

    struct Group {
        key: Vec<Json>,
        accs: Vec<Acc>,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for doc in docs {
        let key: Vec<Json> = plan.group_by.iter().map(|p| p.get(doc).cloned().unwrap_or(Json::Null)).collect();
        let id = serde_json::to_string(&key).map_err(AppError::internal)?;
        let at = *index.entry(id).or_insert_with(|| {
            groups.push(Group { key, accs: aggregates.iter().map(Acc::new).collect() });
            groups.len() - 1
        });
        for (acc, aggregate) in groups[at].accs.iter_mut().zip(&aggregates) {
            acc.add(aggregate, doc)?;
        }
    }
    // An aggregate over nothing is still one row (COUNT(*) = 0), unless it is grouped.
    if groups.is_empty() && plan.group_by.is_empty() {
        groups.push(Group { key: Vec::new(), accs: aggregates.iter().map(Acc::new).collect() });
    }

    // One row per group: group keys and finished aggregates by name, then the select list.
    struct Finished {
        keys: Vec<(String, Json)>,
        values: Vec<(String, Value, &'static str)>,
    }
    let finished: Vec<Finished> = groups
        .into_iter()
        .map(|group| Finished {
            keys: plan.group_by.iter().map(Path::name).zip(group.key).collect(),
            values: aggregates
                .iter()
                .map(Aggregate::name)
                .zip(group.accs)
                .map(|(name, acc)| {
                    let (value, kind) = acc.finish();
                    (name, value, kind)
                })
                .collect(),
        })
        .collect();

    let mut columns: Vec<(String, String)> = Vec::new();
    let mut rows: Vec<Vec<Value>> = vec![Vec::new(); finished.len()];
    for item in &plan.items {
        let (name, source) = match item {
            Item::Column { path, alias } => (alias.clone().unwrap_or_else(|| path.name()), path.name()),
            Item::Aggregate { aggregate, alias } => (alias.clone().unwrap_or_else(|| aggregate.name()), aggregate.name()),
            Item::Star => return Err(AppError::invalid_input("SELECT * can't be combined with aggregates or GROUP BY.")),
        };
        let mut kind = "null".to_string();
        for (row, group) in rows.iter_mut().zip(&finished) {
            let cell = match item {
                Item::Aggregate { .. } => group.values.iter().find(|(n, _, _)| *n == source).map(|(_, v, k)| (v.clone(), (*k).to_string())),
                _ => group.keys.iter().find(|(n, _)| *n == source).map(|(_, v)| (json_to_value(v), kind_name(v).to_string())),
            };
            let (value, value_kind) = cell.unwrap_or((Value::Null, "null".to_string()));
            if kind == "null" {
                kind = value_kind;
            }
            row.push(value);
        }
        columns.push((name, kind));
    }

    // ORDER BY over the finished rows: by select-list name or alias, or by a group
    // column / aggregate that the select list does not show.
    if !plan.order_by.is_empty() {
        let mut keyed: Vec<(Vec<Value>, Vec<Value>)> = Vec::new();
        for (row, group) in rows.into_iter().zip(&finished) {
            let mut sort_values = Vec::new();
            for order in &plan.order_by {
                let name = order.target.name();
                let from_list = plan
                    .items
                    .iter()
                    .zip(&columns)
                    .position(|(item, (column, _))| *column == name || item_source(item) == name)
                    .and_then(|i| row.get(i).cloned());
                let value = match from_list {
                    Some(v) => v,
                    None => group
                        .values
                        .iter()
                        .find(|(n, _, _)| *n == name)
                        .map(|(_, v, _)| v.clone())
                        .or_else(|| group.keys.iter().find(|(n, _)| *n == name).map(|(_, v)| json_to_value(v)))
                        .ok_or_else(|| AppError::invalid_input(format!("ORDER BY {name} is not a GROUP BY column or an aggregate of this query.")))?,
                };
                sort_values.push(value);
            }
            keyed.push((sort_values, row));
        }
        for (position, order) in plan.order_by.iter().enumerate().rev() {
            keyed.sort_by(|(a, _), (b, _)| compare_directed(Some(&cell_json(&a[position])), Some(&cell_json(&b[position])), order.desc));
        }
        rows = keyed.into_iter().map(|(_, row)| row).collect();
    }
    if let Some(limit) = plan.limit {
        rows.truncate(limit);
    }
    Ok(Grid { columns, rows })
}

fn item_source(item: &Item) -> String {
    match item {
        Item::Column { path, .. } => path.name(),
        Item::Aggregate { aggregate, .. } => aggregate.name(),
        Item::Star => String::new(),
    }
}

/// A finished cell as JSON, so it sorts with the same rules as a raw item value.
fn cell_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(i) => Json::from(*i),
        Value::Float(f) => serde_json::Number::from_f64(*f).map(Json::Number).unwrap_or(Json::Null),
        Value::Decimal(d) => d.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Json::Number).unwrap_or(Json::Null),
        Value::Text(t) | Value::DateTime(t) | Value::Bytes(t) | Value::Unsupported(t) => Json::String(t.clone()),
        Value::Json(j) => j.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plan(sql: &str) -> Plan {
        match parse(sql) {
            Ok(Some(plan)) => plan,
            other => panic!("expected a plan for {sql}: {other:?}"),
        }
    }

    fn orders() -> Vec<Json> {
        vec![
            json!({"id": "a", "customer": "ann", "total": 10, "tags": ["x"], "meta": {"city": "Oslo"}}),
            json!({"id": "b", "customer": "bob", "total": 25, "meta": {"city": "Rome"}}),
            json!({"id": "c", "customer": "ann", "total": 5.5}),
            json!({"id": "d", "customer": "cy"}),
            json!({"id": "e", "customer": "bob", "total": 40}),
        ]
    }

    fn cell(grid: &Grid, row: usize, col: usize) -> Value {
        grid.rows.get(row).and_then(|r| r.get(col)).cloned().unwrap_or(Value::Unsupported("missing".into()))
    }

    #[test]
    fn statements_dynamodb_can_run_are_left_alone() {
        for sql in [
            "SELECT * FROM orders",
            "SELECT id, total FROM \"orders\" WHERE id = 'a'",
            "UPDATE orders SET total = 1 WHERE id = 'a'",
            "SELECT SIZE(tags) FROM orders",
            "INSERT INTO orders VALUE {'id': 'z'}",
        ] {
            assert!(matches!(parse(sql), Ok(None)), "{sql}");
        }
    }

    #[test]
    fn the_rewrite_keeps_the_table_and_the_where_and_drops_the_rest() {
        let p = plan("SELECT customer, COUNT(*) AS n FROM \"orders\" WHERE total > 5 AND note = 'GROUP BY ORDER BY' GROUP BY customer ORDER BY n DESC LIMIT 3;");
        assert_eq!(p.base_statement, "SELECT * FROM \"orders\" WHERE total > 5 AND note = 'GROUP BY ORDER BY'");
        assert_eq!(p.limit, Some(3));
        assert!(p.needs_every_item());
        let p = plan("SELECT COUNT(*) FROM \"orders\".\"by_customer\"");
        assert_eq!(p.base_statement, "SELECT * FROM \"orders\".\"by_customer\"");
    }

    #[test]
    fn order_by_alone_can_be_tried_natively_first_and_limit_alone_can_stop_early() {
        assert!(plan("SELECT * FROM orders WHERE id = 'a' ORDER BY total DESC").is_order_by_only());
        assert!(!plan("SELECT * FROM orders ORDER BY total LIMIT 2").is_order_by_only());
        let limit_only = plan("SELECT * FROM orders LIMIT 2");
        assert!(!limit_only.needs_every_item(), "reading can stop once there are enough items");
    }

    #[test]
    fn count_sum_avg_min_max_over_the_whole_set() {
        let p = plan("SELECT COUNT(*), COUNT(total), SUM(total), AVG(total), MIN(total), MAX(total) FROM orders");
        let grid = evaluate(&p, &orders()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(grid.columns.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["COUNT(*)", "COUNT(total)", "SUM(total)", "AVG(total)", "MIN(total)", "MAX(total)"]);
        assert_eq!(cell(&grid, 0, 0), Value::Int(5), "every item");
        assert_eq!(cell(&grid, 0, 1), Value::Int(4), "only the items that have a total");
        assert_eq!(cell(&grid, 0, 2), Value::Float(80.5), "a float among ints makes a float");
        assert_eq!(cell(&grid, 0, 3), Value::Float(20.125));
        assert_eq!(cell(&grid, 0, 4), Value::Float(5.5));
        assert_eq!(cell(&grid, 0, 5), Value::Int(40));
    }

    #[test]
    fn integer_sums_stay_exact_integers() {
        let p = plan("SELECT SUM(n) FROM t");
        let docs = vec![json!({"n": 9_007_199_254_740_993_i64}), json!({"n": 1})];
        let grid = evaluate(&p, &docs).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cell(&grid, 0, 0), Value::Int(9_007_199_254_740_994), "no detour through a float");
    }

    #[test]
    fn aggregates_over_no_items_follow_sql() {
        let p = plan("SELECT COUNT(*), SUM(total), AVG(total), MIN(total) FROM orders");
        let grid = evaluate(&p, &[]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((cell(&grid, 0, 0), cell(&grid, 0, 1), cell(&grid, 0, 2), cell(&grid, 0, 3)), (Value::Int(0), Value::Null, Value::Null, Value::Null));
        let grouped = plan("SELECT customer, COUNT(*) FROM orders GROUP BY customer");
        assert_eq!(evaluate(&grouped, &[]).map(|g| g.rows.len()).ok(), Some(0), "no groups, no rows");
    }

    #[test]
    fn group_by_with_order_by_and_limit() {
        let p = plan("SELECT customer, COUNT(*) AS orders, SUM(total) AS spent FROM orders GROUP BY customer ORDER BY spent DESC LIMIT 2");
        let grid = evaluate(&p, &orders()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(grid.columns.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["customer", "orders", "spent"]);
        assert_eq!(grid.rows.len(), 2);
        assert_eq!(cell(&grid, 0, 0), Value::Text("bob".into()));
        assert_eq!(cell(&grid, 0, 2), Value::Int(65));
        assert_eq!(cell(&grid, 1, 0), Value::Text("ann".into()));
        assert_eq!(cell(&grid, 1, 2), Value::Float(15.5));
    }

    #[test]
    fn nulls_sort_last_whichever_way_the_sort_runs() {
        let asc = plan("SELECT customer, SUM(total) AS spent FROM orders GROUP BY customer ORDER BY spent");
        let grid = evaluate(&asc, &orders()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cell(&grid, 2, 0), Value::Text("cy".into()), "cy has no total, so its SUM is NULL and sorts last");
        let desc = plan("SELECT customer, SUM(total) AS spent FROM orders GROUP BY customer ORDER BY spent DESC");
        let grid = evaluate(&desc, &orders()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cell(&grid, 0, 0), Value::Text("bob".into()));
        assert_eq!(cell(&grid, 2, 0), Value::Text("cy".into()), "still last: a missing value is not the biggest one");
    }

    #[test]
    fn order_by_an_aggregate_the_select_list_does_not_show() {
        let p = plan("SELECT customer FROM orders GROUP BY customer ORDER BY COUNT(*) DESC, customer");
        let grid = evaluate(&p, &orders()).unwrap_or_else(|e| panic!("{e}"));
        let names: Vec<Value> = (0..grid.rows.len()).map(|i| cell(&grid, i, 0)).collect();
        assert_eq!(names, vec![Value::Text("ann".into()), Value::Text("bob".into()), Value::Text("cy".into())], "ties broken by the second key");
    }

    #[test]
    fn count_distinct_and_nested_paths() {
        let p = plan("SELECT COUNT(DISTINCT customer) AS people, COUNT(meta.city) AS cities, MIN(meta.city) FROM orders");
        let grid = evaluate(&p, &orders()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cell(&grid, 0, 0), Value::Int(3));
        assert_eq!(cell(&grid, 0, 1), Value::Int(2));
        assert_eq!(cell(&grid, 0, 2), Value::Text("Oslo".into()));
        let indexed = plan("SELECT COUNT(tags[0]) FROM orders");
        assert_eq!(cell(&evaluate(&indexed, &orders()).unwrap_or_else(|e| panic!("{e}")), 0, 0), Value::Int(1));
    }

    #[test]
    fn min_and_max_work_on_text() {
        let p = plan("SELECT MIN(customer), MAX(customer) FROM orders");
        let grid = evaluate(&p, &orders()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((cell(&grid, 0, 0), cell(&grid, 0, 1)), (Value::Text("ann".into()), Value::Text("cy".into())));
        assert_eq!(grid.columns.first().map(|(_, t)| t.as_str()), Some("string"));
    }

    #[test]
    fn summing_text_is_refused_by_name() {
        let p = plan("SELECT SUM(customer) FROM orders");
        let err = evaluate(&p, &orders()).err().map(|e| e.message().to_string()).unwrap_or_default();
        assert!(err.contains("SUM(customer) needs numbers"), "{err}");
    }

    #[test]
    fn order_by_without_aggregates_sorts_the_items() {
        let p = plan("SELECT id, total FROM orders ORDER BY total DESC LIMIT 3");
        let grid = evaluate(&p, &orders()).unwrap_or_else(|e| panic!("{e}"));
        let ids: Vec<Value> = (0..grid.rows.len()).map(|i| cell(&grid, i, 0)).collect();
        assert_eq!(ids, vec![Value::Text("e".into()), Value::Text("b".into()), Value::Text("a".into())], "the item with no total ranks below every item that has one");
        assert_eq!(grid.columns, vec![("id".to_string(), "string".to_string()), ("total".to_string(), "number".to_string())]);
        let star = plan("SELECT * FROM orders ORDER BY id DESC LIMIT 1");
        let grid = evaluate(&star, &orders()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(grid.rows.len(), 1);
        let id_column = grid.columns.iter().position(|(n, _)| n == "id").unwrap_or(usize::MAX);
        assert_eq!(cell(&grid, 0, id_column), Value::Text("e".into()));
        assert!(grid.columns.iter().any(|(n, _)| n == "customer"));
    }

    #[test]
    fn multi_key_order_by_is_stable() {
        let p = plan("SELECT id FROM orders ORDER BY customer, total DESC");
        let grid = evaluate(&p, &orders()).unwrap_or_else(|e| panic!("{e}"));
        let ids: Vec<Value> = (0..grid.rows.len()).map(|i| cell(&grid, i, 0)).collect();
        assert_eq!(
            ids,
            ["a", "c", "e", "b", "d"].iter().map(|s| Value::Text((*s).into())).collect::<Vec<_>>(),
            "ann (10 then 5.5), bob (40 then 25), cy"
        );
    }

    #[test]
    fn what_cannot_be_emulated_says_so() {
        let message = |sql: &str| parse(sql).err().map(|e| e.message().to_string()).unwrap_or_default();
        assert!(message("SELECT customer, COUNT(*) FROM orders").contains("must appear in GROUP BY"));
        assert!(message("SELECT *, COUNT(*) FROM orders").contains("SELECT *"));
        assert!(message("SELECT customer, COUNT(*) FROM orders GROUP BY customer HAVING COUNT(*) > 1").contains("HAVING"));
        assert!(message("SELECT COUNT(*) FROM orders LIMIT many").contains("LIMIT"));
        assert!(message("SELECT SIZE(tags), COUNT(*) FROM orders").contains("can't be used with aggregates"));
        assert!(message("SELECT COUNT(*) FROM orders WHERE name = 'oops").contains("unterminated"));
    }

    #[test]
    fn keywords_inside_quotes_and_brackets_do_not_split() {
        let p = plan("SELECT COUNT(*) FROM \"order by\" WHERE note = 'x LIMIT 1' AND tags[0] = 'GROUP BY'");
        assert_eq!(p.base_statement, "SELECT * FROM \"order by\" WHERE note = 'x LIMIT 1' AND tags[0] = 'GROUP BY'");
        assert_eq!(p.limit, None);
    }
}
