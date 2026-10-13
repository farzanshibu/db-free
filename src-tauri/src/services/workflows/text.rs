// SOT: workflow-regex, regex-extract-replace, js-style-replacement, regex-live-test

use super::data::{cell_text, Dataset};
use crate::error::{AppError, AppResult};
use crate::model::workflow::{RegexConfig, RegexMode, RegexTestResult};
use crate::model::{ColumnMeta, Value};
use regex::{Regex, RegexBuilder};

// ============================================================================
// REGEX NODE
//
// WHAT:  Per-row regex work on one column: pull out the first capture group
//        (or the whole match), or substitute matches in place.
// WHY:   The cheapest way to clean a column without opening the JavaScript node.
// HOW:   Flags work like a JavaScript RegExp: `i` ignore case, `m` multi-line,
//        `s` dot matches newline, `g` replace every match (without it Replace
//        changes the first match only). The pattern uses the Rust regex syntax:
//        there is no look-around and no back-reference. A replacement uses the
//        JavaScript spelling (`$1`, `$&`, `$<name>`, `$$`), translated to the
//        crate's own before use.
// WHERE: src-tauri/src/services/workflows/nodes/transform.rs
// ============================================================================

struct Compiled {
    regex: Regex,
    mode: RegexMode,
    global: bool,
    replacement: String,
}

fn compile(pattern: &str, flags: &str, mode: RegexMode, replacement: &str) -> AppResult<Compiled> {
    if pattern.is_empty() {
        return Err(AppError::invalid_input("The pattern is empty."));
    }
    let mut builder = RegexBuilder::new(pattern);
    let mut global = false;
    for flag in flags.chars().filter(|c| !c.is_whitespace()) {
        match flag {
            'i' => {
                builder.case_insensitive(true);
            }
            'm' => {
                builder.multi_line(true);
            }
            's' => {
                builder.dot_matches_new_line(true);
            }
            'g' => global = true,
            'u' => {}
            other => return Err(AppError::invalid_input(format!("Unsupported flag \"{other}\". Use i, m, s or g."))),
        }
    }
    let regex = builder.build().map_err(|e| AppError::invalid_input(format!("Invalid regex: {}", first_line(&e.to_string()))))?;
    let groups = regex.captures_len().saturating_sub(1);
    let names: Vec<String> = regex.capture_names().flatten().map(str::to_string).collect();
    Ok(Compiled { replacement: translate_replacement(replacement, groups, &names), regex, mode, global })
}

fn first_line(text: &str) -> &str {
    text.lines().last().map(str::trim).filter(|l| !l.is_empty()).unwrap_or(text)
}

// WHAT:  A JavaScript replacement string in the regex crate's syntax.
// HOW:   `$1`/`$12` -> `${1}` (when that group exists, else the text stays literal),
//        `$&` -> `${0}`, `$<name>` -> `${name}`, `$$` -> `$$`; every other `$` is literal.
fn translate_replacement(replacement: &str, groups: usize, names: &[String]) -> String {
    let chars: Vec<char> = replacement.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '$' {
            out.push(c);
            i += 1;
            continue;
        }
        match chars.get(i + 1) {
            Some('$') => {
                out.push_str("$$");
                i += 2;
            }
            Some('&') => {
                out.push_str("${0}");
                i += 2;
            }
            Some('<') => {
                let close = chars[i + 2..].iter().position(|ch| *ch == '>');
                match close {
                    Some(rel) => {
                        let name: String = chars[i + 2..i + 2 + rel].iter().collect();
                        if names.contains(&name) {
                            out.push_str(&format!("${{{name}}}"));
                        }
                        i += rel + 3;
                    }
                    None => {
                        out.push_str("$$");
                        i += 1;
                    }
                }
            }
            Some(d) if d.is_ascii_digit() => {
                let one = d.to_digit(10).map_or(0, |n| n as usize);
                let two = chars.get(i + 2).and_then(|n| n.to_digit(10)).map(|n| one * 10 + n as usize);
                match two {
                    Some(n) if (1..=groups).contains(&n) => {
                        out.push_str(&format!("${{{n}}}"));
                        i += 3;
                    }
                    _ if (1..=groups).contains(&one) => {
                        out.push_str(&format!("${{{one}}}"));
                        i += 2;
                    }
                    _ => {
                        out.push_str("$$");
                        i += 1;
                    }
                }
            }
            _ => {
                out.push_str("$$");
                i += 1;
            }
        }
    }
    out
}

impl Compiled {
    fn apply(&self, input: &str) -> Value {
        match self.mode {
            RegexMode::Extract => match self.regex.captures(input) {
                Some(caps) => {
                    let found = caps.get(1).or_else(|| caps.get(0));
                    found.map_or(Value::Null, |m| Value::Text(m.as_str().to_string()))
                }
                None => Value::Null,
            },
            RegexMode::Replace => {
                let replaced = if self.global {
                    self.regex.replace_all(input, self.replacement.as_str())
                } else {
                    self.regex.replace(input, self.replacement.as_str())
                };
                Value::Text(replaced.into_owned())
            }
        }
    }
}

// WHAT:  Runs the node over a dataset: the result column is the source column
//        replaced in place, or `output_field` (added when it is new).
pub fn apply(config: &RegexConfig, data: &Dataset) -> AppResult<Dataset> {
    let source = config.source_field.trim();
    if source.is_empty() {
        return Err(AppError::invalid_input("Source field is empty."));
    }
    let compiled = compile(config.pattern.trim(), &config.flags, config.mode, &config.replacement)?;
    if data.rows.is_empty() {
        return Ok(data.clone());
    }
    let source_index = data.index_of(source).ok_or_else(|| AppError::invalid_input(format!("The input has no column \"{source}\".")))?;
    let output = if config.output_field.trim().is_empty() { source } else { config.output_field.trim() };
    let mut out = data.clone();
    let target_index = match out.index_of(output) {
        Some(index) => index,
        None => {
            out.columns.push(ColumnMeta { name: output.to_string(), type_name: "string".to_string() });
            for row in &mut out.rows {
                row.push(Value::Null);
            }
            out.columns.len() - 1
        }
    };
    for row in &mut out.rows {
        let text = row.get(source_index).map(cell_text).unwrap_or_default();
        let value = compiled.apply(&text);
        if let Some(cell) = row.get_mut(target_index) {
            *cell = value;
        }
    }
    Ok(out)
}

// WHAT:  The inspector's live test: run the pattern over one sample string.
pub fn test(pattern: &str, flags: &str, mode: RegexMode, replacement: &str, sample: &str) -> RegexTestResult {
    match compile(pattern.trim(), flags, mode, replacement) {
        Err(err) => RegexTestResult { matched: false, output: None, error: Some(err.message().to_string()) },
        Ok(compiled) => {
            let matched = compiled.regex.is_match(sample);
            let output = match compiled.apply(sample) {
                Value::Text(text) => Some(text),
                _ => None,
            };
            RegexTestResult { matched, output, error: None }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_column(values: &[&str]) -> Dataset {
        Dataset {
            columns: vec![ColumnMeta { name: "s".into(), type_name: "string".into() }],
            rows: values.iter().map(|v| vec![Value::Text((*v).into())]).collect(),
        }
    }

    fn cfg(pattern: &str, flags: &str, mode: RegexMode, replacement: &str, output: &str) -> RegexConfig {
        RegexConfig {
            source_field: "s".into(),
            pattern: pattern.into(),
            flags: flags.into(),
            mode,
            replacement: replacement.into(),
            output_field: output.into(),
        }
    }

    #[test]
    fn extract_takes_the_first_group_or_the_whole_match() {
        let data = one_column(&["order #123 shipped", "no digits", "ORDER #9"]);
        let out = apply(&cfg(r"#(\d+)", "", RegexMode::Extract, "", "order"), &data).unwrap();
        assert_eq!(out.column_names(), vec!["s", "order"]);
        assert_eq!(out.rows[0][1], Value::Text("123".into()));
        assert_eq!(out.rows[1][1], Value::Null);
        assert_eq!(out.rows[2][1], Value::Text("9".into()));
        let whole = apply(&cfg(r"\d+", "", RegexMode::Extract, "", ""), &data).unwrap();
        assert_eq!(whole.rows[0][0], Value::Text("123".into()), "no output field writes back into the source");
        assert_eq!(whole.rows[1][0], Value::Null);
    }

    #[test]
    fn replace_is_first_match_unless_global() {
        let data = one_column(&["a-b-c"]);
        let first = apply(&cfg("-", "", RegexMode::Replace, "+", ""), &data).unwrap();
        assert_eq!(first.rows[0][0], Value::Text("a+b-c".into()));
        let all = apply(&cfg("-", "g", RegexMode::Replace, "+", ""), &data).unwrap();
        assert_eq!(all.rows[0][0], Value::Text("a+b+c".into()));
    }

    #[test]
    fn replacement_uses_javascript_references() {
        let data = one_column(&["John Smith"]);
        let swap = apply(&cfg(r"(\w+) (\w+)", "", RegexMode::Replace, "$2, $1", ""), &data).unwrap();
        assert_eq!(swap.rows[0][0], Value::Text("Smith, John".into()));
        let glued = apply(&cfg(r"(\w+) (\w+)", "", RegexMode::Replace, "$1abc", ""), &data).unwrap();
        assert_eq!(glued.rows[0][0], Value::Text("Johnabc".into()), "$1 followed by letters is still group 1");
        let literal = apply(&cfg(r"(\w+) (\w+)", "", RegexMode::Replace, "$$5 $3 $&", ""), &data).unwrap();
        assert_eq!(literal.rows[0][0], Value::Text("$5 $3 John Smith".into()));
        let named = apply(&cfg(r"(?P<first>\w+) \w+", "", RegexMode::Replace, "<$<first>>", ""), &data).unwrap();
        assert_eq!(named.rows[0][0], Value::Text("<John>".into()));
    }

    #[test]
    fn flags_change_matching() {
        let data = one_column(&["Hello\nWorld"]);
        assert_eq!(apply(&cfg("hello", "i", RegexMode::Extract, "", "x"), &data).unwrap().rows[0][1], Value::Text("Hello".into()));
        assert_eq!(apply(&cfg("hello", "", RegexMode::Extract, "", "x"), &data).unwrap().rows[0][1], Value::Null);
        assert_eq!(apply(&cfg("^World", "m", RegexMode::Extract, "", "x"), &data).unwrap().rows[0][1], Value::Text("World".into()));
        assert_eq!(apply(&cfg("Hello.World", "s", RegexMode::Extract, "", "x"), &data).unwrap().rows[0][1], Value::Text("Hello\nWorld".into()));
    }

    #[test]
    fn bad_input_is_reported() {
        let data = one_column(&["x"]);
        assert!(apply(&cfg("(", "", RegexMode::Extract, "", ""), &data).is_err());
        assert!(apply(&cfg("x", "q", RegexMode::Extract, "", ""), &data).is_err());
        assert!(apply(&cfg("", "", RegexMode::Extract, "", ""), &data).is_err());
        let mut missing = cfg("x", "", RegexMode::Extract, "", "");
        missing.source_field = "nope".into();
        assert!(apply(&missing, &data).is_err());
        missing.source_field = "  ".into();
        assert!(apply(&missing, &data).is_err());
        // An empty input passes through whatever it holds.
        assert!(apply(&missing_column_cfg(), &Dataset::empty()).unwrap().rows.is_empty());
    }

    fn missing_column_cfg() -> RegexConfig {
        let mut c = cfg("x", "", RegexMode::Extract, "", "");
        c.source_field = "nope".into();
        c
    }

    #[test]
    fn live_test_reports_matches_and_errors() {
        let hit = test(r"(\d+)", "", RegexMode::Extract, "", "id 42");
        assert!(hit.matched && hit.output.as_deref() == Some("42") && hit.error.is_none());
        let miss = test(r"(\d+)", "", RegexMode::Extract, "", "none");
        assert!(!miss.matched && miss.output.is_none());
        let sub = test("a", "g", RegexMode::Replace, "b", "banana");
        assert_eq!(sub.output.as_deref(), Some("bbnbnb"));
        assert!(test("(", "", RegexMode::Extract, "", "x").error.is_some());
    }
}
