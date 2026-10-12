// SOT: workflow-rules, filter-rule-evaluation, branch-split, row-count-gate

use super::data::{cell_number, cell_text, Dataset};
use crate::model::workflow::{Combinator, GateOp, Rule, RuleGroup, RuleOp};
use crate::model::Value;

// WHAT:  Does a row satisfy a rule group?
// WHY:   Filter keeps matching rows, Branch sends them out its True side; both
//        need the same answer.
// HOW:   `eq`/`neq`/`contains`/`starts_with`/`ends_with`/`in` compare the cell's
//        text with the rule's value; `gt`/`lt`/`gte`/`lte` compare numbers (a
//        non-numeric side never matches); `is_null` means SQL NULL (an empty
//        string is not null). An empty group matches every row.
// WHERE: src-tauri/src/services/workflows/nodes/transform.rs
pub fn row_matches(group: &RuleGroup, data: &Dataset, row: &[Value]) -> bool {
    if group.rules.is_empty() {
        return true;
    }
    let test = |rule: &Rule| rule_matches(rule, data, row);
    match group.combinator {
        Combinator::And => group.rules.iter().all(test),
        Combinator::Or => group.rules.iter().any(test),
    }
}

fn rule_matches(rule: &Rule, data: &Dataset, row: &[Value]) -> bool {
    let cell = data.index_of(&rule.field).and_then(|i| row.get(i)).unwrap_or(&Value::Null);
    let text = cell_text(cell);
    let wanted = rule.value.as_str();
    match rule.op {
        RuleOp::Eq => text == wanted,
        RuleOp::Neq => text != wanted,
        RuleOp::Gt => compare(cell, wanted).is_some_and(|(a, b)| a > b),
        RuleOp::Lt => compare(cell, wanted).is_some_and(|(a, b)| a < b),
        RuleOp::Gte => compare(cell, wanted).is_some_and(|(a, b)| a >= b),
        RuleOp::Lte => compare(cell, wanted).is_some_and(|(a, b)| a <= b),
        RuleOp::Contains => text.contains(wanted),
        RuleOp::StartsWith => text.starts_with(wanted),
        RuleOp::EndsWith => text.ends_with(wanted),
        RuleOp::IsNull => matches!(cell, Value::Null),
        RuleOp::IsNotNull => !matches!(cell, Value::Null),
        RuleOp::In => wanted.split(',').map(str::trim).filter(|item| !item.is_empty()).any(|item| item == text),
    }
}

fn compare(cell: &Value, wanted: &str) -> Option<(f64, f64)> {
    let left = cell_number(cell)?;
    let right = wanted.trim().parse::<f64>().ok().filter(|n| n.is_finite())?;
    Some((left, right))
}

/// The rows that match, with the input's columns.
pub fn filter(group: &RuleGroup, data: &Dataset) -> Dataset {
    let rows = data.rows.iter().filter(|row| row_matches(group, data, row)).cloned().collect();
    Dataset { columns: data.columns.clone(), rows }
}

/// (matching rows, the rest), both with the input's columns.
pub fn split(group: &RuleGroup, data: &Dataset) -> (Dataset, Dataset) {
    let mut matched = Vec::new();
    let mut rest = Vec::new();
    for row in &data.rows {
        if row_matches(group, data, row) {
            matched.push(row.clone());
        } else {
            rest.push(row.clone());
        }
    }
    (Dataset { columns: data.columns.clone(), rows: matched }, Dataset { columns: data.columns.clone(), rows: rest })
}

/// Does the row count satisfy `count <op> threshold`?
pub fn gate_open(op: GateOp, threshold: f64, count: usize) -> bool {
    let count = count as f64;
    match op {
        GateOp::Gt => count > threshold,
        GateOp::Gte => count >= threshold,
        GateOp::Lt => count < threshold,
        GateOp::Lte => count <= threshold,
        GateOp::Eq => (count - threshold).abs() < f64::EPSILON,
        GateOp::Neq => (count - threshold).abs() >= f64::EPSILON,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ColumnMeta;

    fn people() -> Dataset {
        let col = |n: &str| ColumnMeta { name: n.into(), type_name: "any".into() };
        Dataset {
            columns: vec![col("name"), col("age"), col("email")],
            rows: vec![
                vec![Value::Text("Ada".into()), Value::Int(36), Value::Text("ada@x.io".into())],
                vec![Value::Text("Bob".into()), Value::Text("17".into()), Value::Null],
                vec![Value::Text("Cy".into()), Value::Null, Value::Text("".into())],
            ],
        }
    }

    fn rule(field: &str, op: RuleOp, value: &str) -> Rule {
        Rule { field: field.into(), op, value: value.into() }
    }

    fn group(combinator: Combinator, rules: Vec<Rule>) -> RuleGroup {
        RuleGroup { combinator, rules }
    }

    fn names(d: &Dataset) -> Vec<String> {
        d.rows.iter().map(|r| cell_text(&r[0])).collect()
    }

    #[test]
    fn empty_group_passes_everything() {
        assert_eq!(filter(&RuleGroup::default(), &people()).rows.len(), 3);
    }

    #[test]
    fn numeric_operators_ignore_non_numbers() {
        let d = people();
        assert_eq!(names(&filter(&group(Combinator::And, vec![rule("age", RuleOp::Gt, "18")]), &d)), vec!["Ada"]);
        assert_eq!(names(&filter(&group(Combinator::And, vec![rule("age", RuleOp::Lte, "17")]), &d)), vec!["Bob"]);
        assert!(filter(&group(Combinator::And, vec![rule("age", RuleOp::Gte, "abc")]), &d).rows.is_empty());
    }

    #[test]
    fn text_operators_and_null_checks() {
        let d = people();
        assert_eq!(names(&filter(&group(Combinator::And, vec![rule("name", RuleOp::StartsWith, "A")]), &d)), vec!["Ada"]);
        assert_eq!(names(&filter(&group(Combinator::And, vec![rule("email", RuleOp::Contains, "@")]), &d)), vec!["Ada"]);
        assert_eq!(names(&filter(&group(Combinator::And, vec![rule("email", RuleOp::IsNull, "")]), &d)), vec!["Bob"]);
        assert_eq!(names(&filter(&group(Combinator::And, vec![rule("email", RuleOp::IsNotNull, "")]), &d)), vec!["Ada", "Cy"]);
        assert_eq!(names(&filter(&group(Combinator::And, vec![rule("name", RuleOp::In, "Bob, Cy ,")]), &d)), vec!["Bob", "Cy"]);
        assert_eq!(names(&filter(&group(Combinator::And, vec![rule("name", RuleOp::Neq, "Bob")]), &d)), vec!["Ada", "Cy"]);
        assert_eq!(names(&filter(&group(Combinator::And, vec![rule("ghost", RuleOp::IsNull, "")]), &d)), vec!["Ada", "Bob", "Cy"]);
    }

    #[test]
    fn or_and_and_combine() {
        let d = people();
        let rules = vec![rule("name", RuleOp::Eq, "Ada"), rule("name", RuleOp::Eq, "Cy")];
        assert_eq!(names(&filter(&group(Combinator::Or, rules.clone()), &d)), vec!["Ada", "Cy"]);
        assert!(filter(&group(Combinator::And, rules), &d).rows.is_empty());
    }

    #[test]
    fn split_keeps_the_schema_on_both_sides() {
        let (yes, no) = split(&group(Combinator::And, vec![rule("age", RuleOp::Gt, "18")]), &people());
        assert_eq!(names(&yes), vec!["Ada"]);
        assert_eq!(names(&no), vec!["Bob", "Cy"]);
        assert_eq!(yes.column_names(), no.column_names());
    }

    #[test]
    fn gates_compare_the_row_count() {
        assert!(gate_open(GateOp::Gt, 0.0, 1));
        assert!(!gate_open(GateOp::Gt, 0.0, 0));
        assert!(gate_open(GateOp::Eq, 3.0, 3));
        assert!(gate_open(GateOp::Neq, 3.0, 2));
        assert!(gate_open(GateOp::Lte, 5.0, 5));
        assert!(!gate_open(GateOp::Lt, 5.0, 5));
        assert!(gate_open(GateOp::Gte, 5.0, 5));
    }
}
