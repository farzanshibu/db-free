// SOT: agent-skills, task-playbooks, progressive-instruction-disclosure

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::model::AgentSkill;
use crate::services::agent::tools::ToolRun;

// WHAT:  Task playbooks the model loads by name instead of carrying in every prompt.
// WHY:   The same reason the schema is not pasted in wholesale. A schema-audit
//        checklist is 40 lines the model needs on one turn in fifty; paying for
//        it on every turn is what made the system prompt enormous. Only the
//        one-line descriptions sit in context, and the body arrives on request.
// HOW:   Static text, compiled in. No files to ship, nothing to go stale against
//        the binary, and it works with no network.
// WHERE: src-tauri/src/services/agent/tools.rs (list_skills / load_skill)

struct Skill {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    body: &'static str,
}

const SKILLS: &[Skill] = &[
    Skill {
        id: "schema-audit",
        name: "Audit a schema",
        description: "Review a database's structure for missing keys, weak types and absent indexes.",
        body: "Auditing a schema\n\
               1. list_schemas, then list_tables — get the shape before any detail.\n\
               2. describe_table every table that matters. Do not describe all of them if there \
                  are more than about fifteen; ask the user which area to focus on.\n\
               3. Look for, and report only what you actually found:\n\
                  - tables with no primary key\n\
                  - foreign key columns with no index behind them\n\
                  - text columns holding dates, numbers or booleans\n\
                  - nullable columns that every row fills, and NOT NULL columns with no default\n\
                  - duplicated lookup tables, and columns repeated across tables that should be a join\n\
               4. Rank findings by what would actually hurt: correctness first, then performance, \
                  then tidiness. Give the exact DDL to fix each one.\n\
               5. Never recommend an index without saying which query it serves.",
    },
    Skill {
        id: "query-tuning",
        name: "Tune a slow query",
        description: "Diagnose why a statement is slow and propose a measured fix.",
        body: "Tuning a slow query\n\
               1. explain_query first. Read the plan before theorising.\n\
               2. Name the specific cost: a sequential scan on a large table, a filter on an \
                  unindexed column, a nested loop over many rows, a sort that spills.\n\
               3. describe_table the tables involved to see what indexes already exist, so you do \
                  not propose one that is there.\n\
               4. Propose the smallest change that removes the cost you named. Give exact DDL.\n\
               5. State the trade-off honestly — an index costs write throughput and disk.\n\
               6. If the plan looks fine, say so and look at the query shape instead: SELECT *, \
                  a function wrapped around an indexed column, an accidental cross join, OR that \
                  should be UNION ALL.",
    },
    Skill {
        id: "data-profiling",
        name: "Profile data quality",
        description: "Measure null rates, cardinality, ranges and outliers in a table's columns.",
        body: "Profiling data quality\n\
               1. describe_table for the column list, then sample_rows to see real values.\n\
               2. Run one aggregate query per group of columns rather than one per column — \
                  count(*), count(col), count(distinct col), min, max in a single pass.\n\
               3. Report as a table: column, null %, distinct count, min, max, and a note.\n\
               4. Call out what is actually wrong: columns that are entirely null, a 'status' \
                  column with 40 distinct spellings, dates in the future, negative amounts, \
                  emails without an @, distinct counts equal to the row count on a non-key column.\n\
               5. Prefer render_chart for the null-rate breakdown — it reads far faster than numbers.",
    },
    Skill {
        id: "relationship-map",
        name: "Explain how tables relate",
        description: "Trace the foreign keys and join paths between a set of tables.",
        body: "Mapping relationships\n\
               1. describe_table each table in question; the foreign keys come back with it.\n\
               2. Where no foreign key is declared, infer joins from column naming (user_id -> \
                  users.id) but say plainly that it is inferred, not enforced.\n\
               3. Give the join path as runnable SQL, not prose.\n\
               4. Warn about fan-out: a join through a one-to-many multiplies rows and will \
                  silently inflate any SUM or COUNT downstream.\n\
               5. Use render_graph when the engine supports it, otherwise describe the path \
                  as a short chain: orders -> customers -> regions.",
    },
    Skill {
        id: "dashboard-design",
        name: "Design a dashboard",
        description: "Choose the metrics, charts and query shapes for a dashboard before building it.",
        body: "Designing a dashboard\n\
               1. Decide what question the dashboard answers and for whom. If the user did not say, \
                  infer it from the tables they mentioned and say what you assumed.\n\
               2. list_tables / describe_table only for the tables that serve that question.\n\
               3. Lead with two to four headline metrics (kind metric): one row, one numeric column \
                  each — a count, a sum, a rate.\n\
               4. Then the charts that explain the metrics: a line or area over time (label column = \
                  a truncated date, then numeric columns), a bar for a ranked breakdown (limit it to \
                  ten rows and order it), a pie only for parts of a whole with a handful of slices.\n\
               5. End with a table of recent or top rows, limited and ordered.\n\
               6. Every widget is one read-only SELECT. Name columns (no SELECT *) and alias them \
                  so the chart labels read well. Use a variable ({{name}}) for a filter the user may \
                  want to change, and declare it with its type (a date range gives {{period.start}} \
                  and {{period.end}}). If you can run queries, run each widget's statement once \
                  (limited, with a variable's value written in) before including it, so no tile \
                  opens broken.\n\
               7. Call create_dashboard once with all the widgets. Do not narrate each one afterwards.",
    },
    Skill {
        id: "workflow-design",
        name: "Design a workflow",
        description: "Lay out the nodes of an automation (query, filter, branch, notify, save) before building it.",
        body: "Designing a workflow\n\
               1. Say in a sentence what the workflow does and when it should run. A workflow that \
                  runs on its own starts with a schedule node; otherwise the user runs it by hand.\n\
               2. describe_table only for the tables it touches, so every query uses real names.\n\
               3. Order the nodes the way the rows travel: a source (query, http, read_file), then \
                  what narrows or routes them (filter, row_count_gate, branch, repeat, delay, regex, \
                  javascript), then the outputs (notification, email, slack, save_file, insert).\n\
               4. Put a row_count_gate between a query and any alert, so nothing is sent when the \
                  query found nothing. Use a branch only when both sides do something.\n\
               5. Carry values forward with {{column}}; list rows with {{rows.table}} and count them \
                  with {{rows.count}}. Give every node a label that says what it does.\n\
               6. Keep writes few and obvious. A DELETE or TRUNCATE node will not run until the user \
                  ticks Allow destructive statements on it themselves — say which nodes need that.\n\
               7. Never invent an SMTP host, a password or a webhook: email and slack nodes reuse the \
                  user's last setup, and the user enters anything missing.\n\
               8. Call create_workflow once. Then say what runs, in what order, and that a schedule \
                  starts paused.",
    },
    Skill {
        id: "diagram-design",
        name: "Design a schema diagram",
        description: "Model tables, keys and relations for a new schema, or document an existing one, as a diagram.",
        body: "Designing a schema diagram\n\
               1. Existing schema: describe_table the tables in question and copy names and types \
                  exactly; render_graph already shows how they connect. New schema: one table per \
                  thing the user described, named in the plural or singular style they already use.\n\
               2. Give every table a primary key. A foreign key column sits on the table that holds \
                  it, and the relation points from it to the referenced table's key.\n\
               3. Many-to-many becomes a join table whose primary key is the two foreign keys together \
                  (a composite relation, with extraColumns).\n\
               4. Use the engine's own column types, NOT NULL where a value is always there, unique \
                  where it must be, and a default for timestamps. Choose ON DELETE deliberately: \
                  CASCADE for owned rows, RESTRICT or SET NULL otherwise.\n\
               5. Colour related tables alike and add a note for any design decision the user would \
                  otherwise have to ask about.\n\
               6. Call create_diagram once. Nothing changes in the database; the user opens the \
                  designer to refine it and generate the SQL.",
    },
    Skill {
        id: "safe-migration",
        name: "Write a safe migration",
        description: "Plan a schema change that can be applied and rolled back without data loss.",
        body: "Writing a safe migration\n\
               1. describe_table the target first. Never write DDL against a guessed shape.\n\
               2. Prefer additive steps: add a nullable column, backfill, add the constraint. \
                  A single ALTER that rewrites a large table locks it.\n\
               3. Give forward and rollback statements as a matched pair, every time.\n\
               4. Say explicitly which steps take a lock and roughly how long they hold it.\n\
               5. Never propose DROP without a backup step before it, and say so out loud.\n\
               6. Do not run the migration. Present it and let the user decide — they will be \
                  asked to approve each statement anyway.",
    },
];

/// The catalogue, as the settings UI and the `list_skills` tool both see it.
pub fn catalogue() -> Vec<AgentSkill> {
    SKILLS
        .iter()
        .map(|skill| AgentSkill {
            id: skill.id.to_string(),
            name: skill.name.to_string(),
            description: skill.description.to_string(),
        })
        .collect()
}

/// One line per skill, cheap enough to sit in the system prompt.
pub fn index_text() -> String {
    let mut out = String::from("Task guides you can load with load_skill:\n");
    for skill in SKILLS {
        out.push_str(&format!("- {} — {}\n", skill.id, skill.description));
    }
    out
}

pub fn list_run(_ctx: &SessionCtx) -> ToolRun {
    let mut out = String::new();
    for skill in SKILLS {
        out.push_str(&format!("{} — {}: {}\n", skill.id, skill.name, skill.description));
    }
    ToolRun::from_text(format!("{} guides", SKILLS.len()), out)
}

pub fn load_run(_ctx: &SessionCtx, id: &str) -> AppResult<ToolRun> {
    match SKILLS.iter().find(|skill| skill.id.eq_ignore_ascii_case(id)) {
        Some(skill) => Ok(ToolRun::from_text(skill.name.to_string(), skill.body.to_string())),
        None => Err(AppError::not_found(format!(
            "No guide called \"{id}\". Available: {}",
            SKILLS.iter().map(|s| s.id).collect::<Vec<_>>().join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_ids_are_unique_and_described() {
        let mut seen = std::collections::HashSet::new();
        for skill in SKILLS {
            assert!(seen.insert(skill.id), "duplicate skill {}", skill.id);
            assert!(!skill.description.is_empty() && !skill.body.is_empty());
        }
        for id in ["dashboard-design", "workflow-design", "diagram-design"] {
            assert!(SKILLS.iter().any(|s| s.id == id), "{id}");
        }
    }
}
