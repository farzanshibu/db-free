// SOT: plan-tree-parsers, explain-json, postgres-explain-json, mysql-explain-json, sqlite-query-plan

use crate::model::PlanNode;
use serde_json::{Map, Value as Json};

// ============================================================================
// EXECUTION PLAN PARSERS
//
// WHAT:  Turns each engine's structured EXPLAIN into one `PlanNode` tree.
// WHY:   The visual plan (a bar per operator, the costliest highlighted) needs
//        the same shape from every engine; the adapters only know how to ask.
// HOW:   Pure functions over serde_json / plain rows, so they are unit-tested
//        without a server. Adapters run the EXPLAIN and hand the output here.
// WHERE: src-tauri/src/integrations/{postgres,mysql,sqlite}.rs (`explain_tree`)
// ============================================================================

// ---- Postgres: EXPLAIN (FORMAT JSON) --------------------------------------------

/// `[{"Plan": {...}}]` (or the bare object) -> tree. None when it is not a plan.
pub fn from_postgres_json(doc: &Json) -> Option<PlanNode> {
    let top = match doc {
        Json::Array(items) => items.first()?,
        other => other,
    };
    let plan = top.get("Plan").unwrap_or(top).as_object()?;
    plan.contains_key("Node Type").then(|| postgres_node(plan))
}

/// Keys that say *how* a node works, shown under its label in this order.
const PG_DETAIL_KEYS: &[&str] = &[
    "Join Type",
    "Strategy",
    "Index Name",
    "Index Cond",
    "Hash Cond",
    "Merge Cond",
    "Join Filter",
    "Filter",
    "Recheck Cond",
    "Sort Key",
    "Group Key",
    "Subplan Name",
    "CTE Name",
];

fn postgres_node(plan: &Map<String, Json>) -> PlanNode {
    let node_type = plan.get("Node Type").and_then(Json::as_str).unwrap_or("Plan");
    let target = plan
        .get("Relation Name")
        .and_then(Json::as_str)
        .map(|relation| match plan.get("Alias").and_then(Json::as_str) {
            Some(alias) if alias != relation => format!("{relation} {alias}"),
            _ => relation.to_string(),
        })
        .or_else(|| plan.get("Function Name").and_then(Json::as_str).map(str::to_string));
    let label = match target {
        Some(target) => format!("{node_type} on {target}"),
        None => node_type.to_string(),
    };
    let detail: Vec<String> = PG_DETAIL_KEYS
        .iter()
        .filter_map(|key| plan.get(*key).map(|value| format!("{key}: {}", plain(value))))
        .collect();
    // Actual time is per loop; the subtree's real cost is time x loops.
    let actual_ms = plan
        .get("Actual Total Time")
        .and_then(number)
        .map(|ms| ms * plan.get("Actual Loops").and_then(number).unwrap_or(1.0));
    PlanNode {
        label,
        detail: (!detail.is_empty()).then(|| detail.join("\n")),
        cost: plan.get("Total Cost").and_then(number),
        rows: plan.get("Actual Rows").or_else(|| plan.get("Plan Rows")).and_then(number),
        actual_ms,
        children: plan
            .get("Plans")
            .and_then(Json::as_array)
            .map(|children| children.iter().filter_map(Json::as_object).map(postgres_node).collect())
            .unwrap_or_default(),
    }
}

// ---- MySQL / MariaDB: EXPLAIN FORMAT=JSON ---------------------------------------

/// `{"query_block": {...}}` -> tree. Best effort: MySQL 5.7/8 and MariaDB name
/// their operators differently, so unknown keys are skipped, not fatal.
pub fn from_mysql_json(doc: &Json) -> Option<PlanNode> {
    let block = doc.get("query_block")?.as_object()?;
    Some(mysql_block(block))
}

fn mysql_block(block: &Map<String, Json>) -> PlanNode {
    let id = block.get("select_id").and_then(number).map(|n| format!(" #{n}")).unwrap_or_default();
    PlanNode {
        label: format!("Query block{id}"),
        detail: block.get("message").map(plain),
        cost: block.get("cost_info").and_then(|c| c.get("query_cost")).and_then(number),
        rows: None,
        actual_ms: None,
        children: mysql_children(block),
    }
}

/// Operators nested in `object`, in document order.
fn mysql_children(object: &Map<String, Json>) -> Vec<PlanNode> {
    object.iter().flat_map(|(key, value)| mysql_operator(key, value)).collect()
}

fn mysql_operator(key: &str, value: &Json) -> Vec<PlanNode> {
    match (key, value) {
        ("table", Json::Object(table)) => vec![mysql_table(table)],
        ("query_block", Json::Object(block)) => vec![mysql_block(block)],
        // Lists of steps: each element is an object holding one operator.
        ("nested_loop", Json::Array(steps)) => vec![wrapper("Nested loop", None, steps_children(steps))],
        (
            "query_specifications" | "attached_subqueries" | "optimized_away_subqueries" | "having_subqueries"
            | "select_list_subqueries" | "order_by_subqueries" | "group_by_subqueries" | "subqueries",
            Json::Array(steps),
        ) => steps_children(steps),
        ("ordering_operation" | "filesort" | "read_sorted_file", Json::Object(inner)) => {
            let sorts = flag(inner, "using_filesort").then_some("Using filesort");
            vec![with_cost(wrapper("Sort", sorts.map(str::to_string), mysql_children(inner)), inner)]
        }
        ("grouping_operation", Json::Object(inner)) => {
            let temp = flag(inner, "using_temporary_table").then_some("Using temporary table");
            vec![with_cost(wrapper("Group", temp.map(str::to_string), mysql_children(inner)), inner)]
        }
        ("duplicates_removal", Json::Object(inner)) => vec![wrapper("Distinct", None, mysql_children(inner))],
        ("windowing", Json::Object(inner)) => vec![wrapper("Window", None, mysql_children(inner))],
        ("temporary_table", Json::Object(inner)) => vec![wrapper("Temporary table", None, mysql_children(inner))],
        ("buffer_result", Json::Object(inner)) => vec![wrapper("Buffer result", None, mysql_children(inner))],
        ("union_result", Json::Object(inner)) => {
            let name = inner.get("table_name").map(plain);
            vec![wrapper("Union", name, mysql_children(inner))]
        }
        ("materialized_from_subquery", Json::Object(inner)) => vec![wrapper("Materialize", None, mysql_children(inner))],
        _ => Vec::new(),
    }
}

fn steps_children(steps: &[Json]) -> Vec<PlanNode> {
    steps.iter().filter_map(Json::as_object).flat_map(mysql_children).collect()
}

fn mysql_table(table: &Map<String, Json>) -> PlanNode {
    let name = table.get("table_name").map(plain).unwrap_or_else(|| "?".to_string());
    let access = table.get("access_type").and_then(Json::as_str).unwrap_or("");
    let how = match access {
        "ALL" => "Full scan".to_string(),
        "index" => "Index scan".to_string(),
        "range" => "Range scan".to_string(),
        "ref" | "eq_ref" | "ref_or_null" => "Index lookup".to_string(),
        "const" | "system" => "Constant row".to_string(),
        "" => "Table".to_string(),
        other => other.to_string(),
    };
    let detail: Vec<String> = [("key", "Index"), ("possible_keys", "Possible keys"), ("attached_condition", "Filter"), ("index_condition", "Index cond")]
        .iter()
        .filter_map(|(key, name)| table.get(*key).map(|value| format!("{name}: {}", plain(value))))
        .collect();
    let cost = table.get("cost_info").and_then(|c| {
        c.get("prefix_cost").and_then(number).or_else(|| {
            let read = c.get("read_cost").and_then(number)?;
            Some(read + c.get("eval_cost").and_then(number).unwrap_or(0.0))
        })
    });
    PlanNode {
        label: format!("{how} on {name}"),
        detail: (!detail.is_empty()).then(|| detail.join("\n")),
        cost,
        rows: table.get("rows_examined_per_scan").or_else(|| table.get("rows")).and_then(number),
        actual_ms: table.get("r_total_time_ms").and_then(number),
        children: mysql_children(table),
    }
}

fn wrapper(label: &str, detail: Option<String>, children: Vec<PlanNode>) -> PlanNode {
    PlanNode { label: label.to_string(), detail, cost: None, rows: None, actual_ms: None, children }
}

/// A sort / group step's own cost, when MySQL reports one for it.
fn with_cost(mut node: PlanNode, object: &Map<String, Json>) -> PlanNode {
    node.cost = object.get("cost_info").and_then(|c| c.get("sort_cost").or_else(|| c.get("query_cost"))).and_then(number);
    node
}

fn flag(object: &Map<String, Json>, key: &str) -> bool {
    object.get(key).and_then(Json::as_bool).unwrap_or(false)
}

// ---- SQLite: EXPLAIN QUERY PLAN -------------------------------------------------

/// `(id, parent, detail)` rows -> tree. Several top-level steps (a compound
/// SELECT, a subquery beside the main scan) get one synthetic root.
pub fn from_sqlite_rows(rows: &[(i64, i64, String)]) -> Option<PlanNode> {
    fn build(parent: i64, rows: &[(i64, i64, String)], depth: usize) -> Vec<PlanNode> {
        // Ids are unique and parents precede children; the depth cap only
        // guards against a malformed cycle.
        if depth > 64 {
            return Vec::new();
        }
        rows.iter()
            .filter(|(id, p, _)| *p == parent && *id != parent)
            .map(|(id, _, detail)| PlanNode {
                label: detail.clone(),
                detail: None,
                cost: None,
                rows: None,
                actual_ms: None,
                children: build(*id, rows, depth + 1),
            })
            .collect()
    }
    let mut roots = build(0, rows, 0);
    match roots.len() {
        0 => None,
        1 => roots.pop(),
        _ => Some(wrapper("Query plan", None, roots)),
    }
}

// ---- shared -----------------------------------------------------------------------

/// The statement an adapter wraps in EXPLAIN: one statement, no terminator.
/// Adapters prepare it (extended protocol / `prepare`), which refuses a second
/// statement rather than running it un-EXPLAINed.
pub fn explain_target(sql: &str) -> &str {
    sql.trim().trim_end_matches(';').trim_end()
}

/// A JSON number, or a number written as a string (MySQL writes costs as "1.25").
fn number(value: &Json) -> Option<f64> {
    match value {
        Json::Number(n) => n.as_f64(),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// A value as display text: strings unquoted, arrays comma-joined.
fn plain(value: &Json) -> String {
    match value {
        Json::String(s) => s.clone(),
        Json::Array(items) => items.iter().map(plain).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn postgres_json_becomes_a_tree_with_costs_and_details() {
        let doc = json!([{
            "Plan": {
                "Node Type": "Hash Join", "Join Type": "Inner", "Total Cost": 120.5, "Plan Rows": 40,
                "Hash Cond": "(o.user_id = u.id)",
                "Plans": [
                    { "Node Type": "Seq Scan", "Relation Name": "orders", "Alias": "o", "Total Cost": 80.0, "Plan Rows": 1000, "Filter": "(total > 10)" },
                    { "Node Type": "Hash", "Total Cost": 30.0, "Plan Rows": 50,
                      "Plans": [{ "Node Type": "Index Scan", "Relation Name": "users", "Alias": "users", "Index Name": "users_pkey", "Total Cost": 25.0, "Plan Rows": 50 }] }
                ]
            }
        }]);
        let tree = from_postgres_json(&doc).unwrap_or_else(|| panic!("no tree"));
        assert_eq!(tree.label, "Hash Join");
        assert_eq!(tree.cost, Some(120.5));
        assert_eq!(tree.rows, Some(40.0));
        assert_eq!(tree.detail.as_deref(), Some("Join Type: Inner\nHash Cond: (o.user_id = u.id)"));
        assert_eq!(tree.children.len(), 2);
        assert_eq!(tree.children[0].label, "Seq Scan on orders o");
        assert_eq!(tree.children[0].detail.as_deref(), Some("Filter: (total > 10)"));
        assert_eq!(tree.children[1].children[0].label, "Index Scan on users", "an alias equal to the table is not repeated");
    }

    #[test]
    fn postgres_analyze_time_counts_every_loop() {
        let doc = json!({ "Plan": { "Node Type": "Index Scan", "Actual Total Time": 0.5, "Actual Loops": 10, "Actual Rows": 3 } });
        let tree = from_postgres_json(&doc).unwrap_or_else(|| panic!("no tree"));
        assert_eq!(tree.actual_ms, Some(5.0));
        assert_eq!(tree.rows, Some(3.0), "actual rows win over the estimate");
    }

    #[test]
    fn postgres_rejects_what_is_not_a_plan() {
        assert!(from_postgres_json(&json!([])).is_none());
        assert!(from_postgres_json(&json!({ "hello": 1 })).is_none());
    }

    #[test]
    fn mysql_json_nests_loops_sorts_and_tables() {
        let doc = json!({
            "query_block": {
                "select_id": 1,
                "cost_info": { "query_cost": "12.40" },
                "ordering_operation": {
                    "using_filesort": true,
                    "nested_loop": [
                        { "table": { "table_name": "o", "access_type": "ALL", "rows_examined_per_scan": 100,
                                     "cost_info": { "read_cost": "9.00", "eval_cost": "1.00", "prefix_cost": "10.00" },
                                     "attached_condition": "(`o`.`total` > 10)" } },
                        { "table": { "table_name": "u", "access_type": "eq_ref", "key": "PRIMARY", "rows_examined_per_scan": 1,
                                     "cost_info": { "prefix_cost": "12.40" } } }
                    ]
                }
            }
        });
        let tree = from_mysql_json(&doc).unwrap_or_else(|| panic!("no tree"));
        assert_eq!(tree.label, "Query block #1");
        assert_eq!(tree.cost, Some(12.4), "string costs are numbers");
        let sort = &tree.children[0];
        assert_eq!(sort.label, "Sort");
        assert_eq!(sort.detail.as_deref(), Some("Using filesort"));
        let join = &sort.children[0];
        assert_eq!(join.label, "Nested loop");
        assert_eq!(join.children.len(), 2);
        assert_eq!(join.children[0].label, "Full scan on o");
        assert_eq!(join.children[0].cost, Some(10.0));
        assert_eq!(join.children[0].rows, Some(100.0));
        assert_eq!(join.children[0].detail.as_deref(), Some("Filter: (`o`.`total` > 10)"));
        assert_eq!(join.children[1].label, "Index lookup on u");
        assert_eq!(join.children[1].detail.as_deref(), Some("Index: PRIMARY"));
    }

    #[test]
    fn mysql_without_a_query_block_is_not_a_plan() {
        assert!(from_mysql_json(&json!({ "plan": 1 })).is_none());
    }

    #[test]
    fn explain_target_drops_the_terminator() {
        assert_eq!(explain_target("  SELECT 1 ;\n"), "SELECT 1");
        assert_eq!(explain_target("SELECT 1; SELECT 2"), "SELECT 1; SELECT 2", "a script is left for the prepare to refuse");
    }

    #[test]
    fn sqlite_rows_become_a_tree() {
        let rows = vec![
            (2, 0, "SCAN o".to_string()),
            (5, 0, "SEARCH u USING INTEGER PRIMARY KEY (rowid=?)".to_string()),
            (7, 0, "CORRELATED SCALAR SUBQUERY 1".to_string()),
            (9, 7, "SCAN i".to_string()),
        ];
        let tree = from_sqlite_rows(&rows).unwrap_or_else(|| panic!("no tree"));
        assert_eq!(tree.label, "Query plan", "several top-level steps share a synthetic root");
        assert_eq!(tree.children.len(), 3);
        assert_eq!(tree.children[2].children[0].label, "SCAN i");

        let single = from_sqlite_rows(&[(2, 0, "SCAN t".to_string())]).unwrap_or_else(|| panic!("no tree"));
        assert_eq!(single.label, "SCAN t");
        assert!(from_sqlite_rows(&[]).is_none());
    }
}
