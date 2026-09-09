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
