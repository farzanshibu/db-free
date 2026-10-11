// SOT: store-migrations, migration-steps, feature-migrations

use crate::error::{AppError, AppResult};
use rusqlite::{params, Connection, OptionalExtension};

mod ai;
mod auth;
mod connections;
mod dashboards;
mod diagrams;
mod editor;
mod engine;
mod grid;
mod library;
mod schema;
mod shell;
mod transfer;
mod workflows;

// WHAT:  Feature-owned migration step lists, applied after the numbered
//        `user_version` chain in store/mod.rs.
// WHY:   Features add tables independently. One list per feature keeps each
//        feature's steps together and lets every list grow without renumbering
//        the others.
// HOW:   `store_migrations(module, applied)` counts how many steps of each list
//        have run; startup runs the rest in order, each step in its own
//        transaction. Lists that reference another list's tables come after it.
// WHERE: src-tauri/src/store/mod.rs (Store::migrate)
const MODULES: &[(&str, &[&str])] = &[
    ("connections", connections::STEPS),
    ("auth", auth::STEPS),
    ("schema", schema::STEPS),
    ("grid", grid::STEPS),
    ("editor", editor::STEPS),
    ("library", library::STEPS),
    ("dashboards", dashboards::STEPS),
    ("workflows", workflows::STEPS),
    ("diagrams", diagrams::STEPS),
    ("transfer", transfer::STEPS),
    ("shell", shell::STEPS),
    ("ai", ai::STEPS),
    ("engine", engine::STEPS),
];

pub(super) fn apply(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS store_migrations (
            module TEXT PRIMARY KEY,
            applied INTEGER NOT NULL
        );",
    )
    .map_err(AppError::store)?;
    for (module, steps) in MODULES {
        let applied: i64 = conn
            .query_row("SELECT applied FROM store_migrations WHERE module = ?1", params![module], |row| row.get(0))
            .optional()
            .map_err(AppError::store)?
            .unwrap_or(0);
        let start = usize::try_from(applied).unwrap_or(0);
        for (index, step) in steps.iter().enumerate().skip(start) {
            let tx = conn.unchecked_transaction().map_err(AppError::store)?;
            tx.execute_batch(step).map_err(AppError::store)?;
            let done = i64::try_from(index + 1).unwrap_or(i64::MAX);
            tx.execute(
                "INSERT INTO store_migrations (module, applied) VALUES (?1, ?2)
                 ON CONFLICT(module) DO UPDATE SET applied = excluded.applied",
                params![module, done],
            )
            .map_err(AppError::store)?;
            tx.commit().map_err(AppError::store)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn module_names_are_unique() {
        let mut seen = HashSet::new();
        for (module, _) in MODULES {
            assert!(seen.insert(*module), "duplicate migration module {module}");
        }
    }

    #[test]
    fn applying_twice_runs_each_step_once() {
        // Store::open_in_memory runs the whole chain once; this is the second run.
        let store = crate::store::Store::open_in_memory().unwrap();
        let conn = store.conn();
        apply(conn).unwrap();
        for (module, steps) in MODULES {
            let applied: i64 = conn
                .query_row("SELECT COALESCE((SELECT applied FROM store_migrations WHERE module = ?1), 0)", params![module], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(applied, i64::try_from(steps.len()).unwrap(), "{module}");
        }
    }
}
