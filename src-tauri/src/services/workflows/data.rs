// SOT: workflow-dataset, workflow-row-merge, workflow-json-rows, workflow-cell-text

use crate::integrations::http::{json_to_value, json_type_name};
use crate::model::{ColumnMeta, ResultSet, Value};
use crate::services::transfer::json_cell;

// WHAT:  The rows that flow along an edge: named columns and typed cells.
// WHY:   Every node reads and writes the same shape, so a Query can feed a Filter
//        that feeds a JavaScript node that feeds a file without any of them
//        knowing what the others are.
// HOW:   Cells are the app-wide `Value`, so a database result needs no conversion
//        and the grid can draw a preview as it draws a table.
// WHERE: src-tauri/src/services/workflows/{runner,nodes}
#[derive(Debug, Clone, Default)]
pub struct Dataset {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<Value>>,
}

impl Dataset {
    pub fn empty() -> Dataset {
        Dataset::default()
    }

    /// A dataset of one row from `(column, value)` pairs.
    pub fn single_row(pairs: Vec<(&str, Value)>) -> Dataset {
        let columns = pairs.iter().map(|(name, value)| ColumnMeta { name: (*name).to_string(), type_name: type_of(value).to_string() }).collect();
        Dataset { columns, rows: vec![pairs.into_iter().map(|(_, value)| value).collect()] }
    }

    pub fn row_count(&self) -> u64 {
        self.rows.len() as u64
    }

    pub fn column_names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    /// The same columns and no rows (a closed gate still passes the shape on).
    pub fn schema_only(&self) -> Dataset {
        Dataset { columns: self.columns.clone(), rows: Vec::new() }
    }

    pub fn from_result_set(set: ResultSet) -> Dataset {
        Dataset { columns: set.columns, rows: set.rows }
    }

    /// The first `max_rows` rows as a result set, for a preview grid.
    pub fn preview(&self, max_rows: usize) -> ResultSet {
        ResultSet { columns: self.columns.clone(), rows: self.rows.iter().take(max_rows).cloned().collect(), truncated: self.rows.len() > max_rows }
    }

    /// A cell of the first row, by column name.
    pub fn first_cell(&self, name: &str) -> Option<&Value> {
        let index = self.index_of(name)?;
        self.rows.first()?.get(index)
    }

    /// Columns are the union of keys in first-seen order; a scalar element becomes `{ "value": x }`.
    pub fn from_json_rows(items: &[serde_json::Value]) -> Dataset {
        Dataset::from_json_rows_ordered(items, &[])
    }

    /// Like `from_json_rows`, with the columns in `order` first (those that exist), then any others.
    pub fn from_json_rows_ordered(items: &[serde_json::Value], order: &[String]) -> Dataset {
        let mut names: Vec<String> = Vec::new();
        let wrapped: Vec<serde_json::Map<String, serde_json::Value>> = items
            .iter()
            .map(|item| match item {
                serde_json::Value::Object(map) => map.clone(),
                other => {
                    let mut map = serde_json::Map::new();
                    map.insert("value".to_string(), other.clone());
                    map
                }
            })
            .collect();
        for map in &wrapped {
            for key in map.keys() {
                if !names.iter().any(|n| n == key) {
                    names.push(key.clone());
                }
            }
        }
        if !order.is_empty() {
            let mut ordered: Vec<String> = Vec::with_capacity(names.len());
            for wanted in order {
                if names.contains(wanted) && !ordered.contains(wanted) {
                    ordered.push(wanted.clone());
                }
            }
            ordered.extend(names.iter().filter(|n| !ordered.contains(n)).cloned().collect::<Vec<_>>());
            names = ordered;
        }
        let rows = wrapped.iter().map(|map| names.iter().map(|n| map.get(n).map(json_to_value).unwrap_or(Value::Null)).collect()).collect();
        let columns = names
            .into_iter()
            .map(|name| {
                let type_name = wrapped
                    .iter()
                    .find_map(|m| m.get(&name).filter(|v| !v.is_null()).map(json_type_name))
                    .unwrap_or("any")
                    .to_string();
                ColumnMeta { name, type_name }
            })
            .collect();
        Dataset { columns, rows }
    }

    /// Rows as JSON objects (what the JavaScript node and the JSON file formats speak).
    pub fn to_json_rows(&self) -> Vec<serde_json::Value> {
        self.rows
            .iter()
            .map(|row| {
                let map: serde_json::Map<String, serde_json::Value> =
                    self.columns.iter().zip(row.iter()).map(|(c, v)| (c.name.clone(), json_cell(v))).collect();
                serde_json::Value::Object(map)
            })
            .collect()
    }

    /// Column metadata as JSON, `[{ "name": …, "dataType": … }]`.
    pub fn to_json_fields(&self) -> serde_json::Value {
        serde_json::Value::Array(self.columns.iter().map(|c| serde_json::json!({ "name": c.name, "dataType": c.type_name })).collect())
    }

    // WHAT:  Several inputs as one dataset: columns are the union (first-seen
    //        order), rows are concatenated, a column an input lacks reads NULL.
    pub fn union(inputs: &[&Dataset]) -> Dataset {
        match inputs {
            [] => Dataset::empty(),
            [only] => (*only).clone(),
            many => {
                let mut columns: Vec<ColumnMeta> = Vec::new();
                for input in many {
                    for column in &input.columns {
                        if !columns.iter().any(|c| c.name == column.name) {
                            columns.push(column.clone());
                        }
                    }
                }
                let mut rows = Vec::new();
                for input in many {
                    let map: Vec<Option<usize>> = columns.iter().map(|c| input.index_of(&c.name)).collect();
                    for row in &input.rows {
                        rows.push(map.iter().map(|slot| slot.and_then(|i| row.get(i)).cloned().unwrap_or(Value::Null)).collect());
                    }
                }
                Dataset { columns, rows }
            }
        }
    }
}

fn type_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Int(_) => "integer",
        Value::Float(_) | Value::Decimal(_) => "number",
        Value::Text(_) | Value::Unsupported(_) => "string",
        Value::Bytes(_) => "binary",
        Value::Json(_) => "json",
        Value::DateTime(_) => "timestamp",
    }
}

// WHAT:  A cell as plain text, the way a template or a text file shows it.
// HOW:   NULL is empty, JSON is compact, binary is its base64.
pub fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => format_number(*f),
        Value::Decimal(s) | Value::Text(s) | Value::DateTime(s) | Value::Unsupported(s) => s.clone(),
        Value::Bytes(b64) => b64.clone(),
        Value::Json(j) => match j {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        },
    }
}

/// Whole numbers print without a fraction, the way JavaScript prints them.
pub fn format_number(f: f64) -> String {
    if !f.is_finite() {
        return String::new();
    }
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{f:.0}")
    } else {
        f.to_string()
    }
}

/// A cell as a number, for arithmetic and numeric comparison.
pub fn cell_number(value: &Value) -> Option<f64> {
    let n = match value {
        Value::Int(i) => *i as f64,
        Value::Float(f) => *f,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Decimal(s) | Value::Text(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str) -> ColumnMeta {
        ColumnMeta { name: name.into(), type_name: "any".into() }
    }

    #[test]
    fn union_merges_columns_and_pads_missing_cells() {
        let a = Dataset { columns: vec![col("id"), col("name")], rows: vec![vec![Value::Int(1), Value::Text("a".into())]] };
        let b = Dataset { columns: vec![col("name"), col("age")], rows: vec![vec![Value::Text("b".into()), Value::Int(30)]] };
        let merged = Dataset::union(&[&a, &b]);
        assert_eq!(merged.column_names(), vec!["id", "name", "age"]);
        assert_eq!(merged.rows[0], vec![Value::Int(1), Value::Text("a".into()), Value::Null]);
        assert_eq!(merged.rows[1], vec![Value::Null, Value::Text("b".into()), Value::Int(30)]);
        assert!(Dataset::union(&[]).rows.is_empty());
    }

    #[test]
    fn json_rows_round_trip_with_scalars_wrapped() {
        let items = vec![serde_json::json!({"a": 1, "b": "x"}), serde_json::json!({"b": null, "c": [1, 2]}), serde_json::json!(7)];
        let ds = Dataset::from_json_rows(&items);
        assert_eq!(ds.column_names(), vec!["a", "b", "c", "value"]);
        assert_eq!(ds.rows[0][0], Value::Int(1));
        assert_eq!(ds.rows[1][0], Value::Null);
        assert_eq!(ds.rows[2][3], Value::Int(7));
        assert_eq!(ds.columns[0].type_name, "integer");
        let back = ds.to_json_rows();
        assert_eq!(back[0]["b"], serde_json::json!("x"));
        assert_eq!(back[1]["c"], serde_json::json!([1, 2]));
    }

    #[test]
    fn json_object_keys_keep_their_order() {
        // serde_json's `preserve_order` feature is on through another dependency; column order depends on it.
        let ds = Dataset::from_json_rows(&[serde_json::json!({"zebra": 1, "apple": 2, "mango": 3})]);
        assert_eq!(ds.column_names(), vec!["zebra", "apple", "mango"]);
        let reordered = Dataset::from_json_rows_ordered(&[serde_json::json!({"zebra": 1, "apple": 2, "mango": 3})], &["mango".to_string(), "ghost".to_string()]);
        assert_eq!(reordered.column_names(), vec!["mango", "zebra", "apple"]);
    }

    #[test]
    fn cells_read_as_text_and_numbers() {
        assert_eq!(cell_text(&Value::Null), "");
        assert_eq!(cell_text(&Value::Float(3.0)), "3");
        assert_eq!(cell_text(&Value::Float(2.5)), "2.5");
        assert_eq!(cell_text(&Value::Json(serde_json::json!({"a": 1}))), "{\"a\":1}");
        assert_eq!(cell_text(&Value::Json(serde_json::json!("plain"))), "plain");
        assert_eq!(cell_number(&Value::Text(" 12.5 ".into())), Some(12.5));
        assert_eq!(cell_number(&Value::Text("abc".into())), None);
        assert_eq!(cell_number(&Value::Null), None);
        assert_eq!(cell_number(&Value::Bool(true)), Some(1.0));
    }

    #[test]
    fn schema_only_keeps_the_shape() {
        let ds = Dataset::single_row(vec![("i", Value::Int(0)), ("total", Value::Int(3))]);
        let empty = ds.schema_only();
        assert_eq!(empty.column_names(), vec!["i", "total"]);
        assert!(empty.rows.is_empty());
        assert_eq!(ds.first_cell("total"), Some(&Value::Int(3)));
        assert_eq!(ds.first_cell("nope"), None);
    }
}
