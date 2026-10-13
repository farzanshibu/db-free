// SOT: ddl-script-assembly, ddl-transaction-wrapper, ddl-confirm-name, ddl-operation-notes

use super::alter::{changes_type, matched_original, REBUILD_SUFFIX};
use super::dialect::{dialect_of, transactional_ddl, Dialect};
use super::Assembled;
use crate::model::{AlterTableSpec, DdlOperation, Engine, NamespaceKind, NamespaceOp, RemoveAction, TableKind};

// WHAT:  Joins a form's statements into the one script that is previewed and run,
//        and decides what the form must collect before running it.
// WHY:   The script is what `execute_query` is given, so the guard classifies and
//        runs exactly the text the user read. The wrapper, the confirmation name
//        and the caveats are properties of (engine, operation) and belong with
//        the builders, not in the webview.
// HOW:   A multi-statement script is wrapped in a transaction only where the
//        engine's DDL is transactional, never around CREATE / DROP DATABASE
//        (refused inside a transaction block), and a leading / trailing PRAGMA
//        stays outside it because SQLite ignores `foreign_keys` mid-transaction.
// WHERE: src-tauri/src/services/schema.rs (preview_ddl)

fn is_pragma(statement: &str) -> bool {
    statement.trim_start().get(..6).is_some_and(|head| head.eq_ignore_ascii_case("PRAGMA"))
}

fn terminated(statement: &str) -> String {
    let trimmed = statement.trim().trim_end_matches(';').trim_end();
    format!("{trimmed};")
}

fn creates_or_drops_database(operation: &DdlOperation) -> bool {
    matches!(operation, DdlOperation::Namespace { spec } if spec.kind == NamespaceKind::Database)
}

fn script(engine: Engine, operation: &DdlOperation, statements: &[String]) -> String {
    if dialect_of(engine) == Some(Dialect::Mongo) {
        return statements.iter().map(|s| s.trim()).collect::<Vec<_>>().join("\n\n");
    }
    let head = statements.iter().take_while(|s| is_pragma(s)).count();
    let (before, remainder) = statements.split_at(head);
    let tail = remainder.iter().rev().take_while(|s| is_pragma(s)).count();
    let (middle, after) = remainder.split_at(remainder.len() - tail);

    let wrap = transactional_ddl(engine) && middle.len() > 1 && !creates_or_drops_database(operation);
    let (begin, commit) = if engine == Engine::Mssql { ("BEGIN TRANSACTION;", "COMMIT TRANSACTION;") } else { ("BEGIN;", "COMMIT;") };
    let mut lines: Vec<String> = before.iter().map(|s| terminated(s)).collect();
    if wrap {
        lines.push(begin.to_string());
    }
    lines.extend(middle.iter().map(|s| terminated(s)));
    if wrap {
        lines.push(commit.to_string());
    }
    lines.extend(after.iter().map(|s| terminated(s)));
    lines.join("\n")
}

fn dropped_columns(spec: &AlterTableSpec) -> bool {
    let kept = matched_original(spec);
    spec.original.columns.iter().any(|c| !kept.contains(&c.name))
}

fn confirm_name(operation: &DdlOperation) -> Option<String> {
    match operation {
        DdlOperation::RemoveTable { spec } => Some(spec.table.name.clone()),
        DdlOperation::AlterTable { spec } if dropped_columns(spec) => Some(spec.original.table.name.clone()),
        DdlOperation::Namespace { spec } if matches!(spec.op, NamespaceOp::Drop { .. }) => Some(spec.name.clone()),
        _ => None,
    }
}

fn notes(engine: Engine, operation: &DdlOperation, statements: &[String]) -> Vec<String> {
    let Some(dialect) = dialect_of(engine) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match operation {
        DdlOperation::RemoveTable { spec } => match (spec.action, spec.kind) {
            (RemoveAction::Truncate, _) => out.push("Every row in the table is deleted.".to_string()),
            (RemoveAction::Drop, TableKind::View) => out.push("The view is removed; the tables it reads are untouched.".to_string()),
            (RemoveAction::Drop, TableKind::Table) => out.push("The table and all its data are deleted.".to_string()),
        },
        DdlOperation::Namespace { spec } => {
            if let NamespaceOp::Drop { cascade, .. } = &spec.op {
                let what = spec.kind.noun();
                out.push(if *cascade || spec.kind == NamespaceKind::Database {
                    format!("Everything inside the {what} is deleted with it.")
                } else {
                    format!("The {what} must be empty, or the drop is refused.")
                });
            }
        }
        DdlOperation::AlterTable { spec } => {
            if statements.iter().any(|s| s.contains(REBUILD_SUFFIX)) {
                out.push(
                    "SQLite cannot make this change in place, so the table is rebuilt: its rows are copied into a new table, the old one is dropped and the new one renamed. Triggers on the table are not recreated.".to_string(),
                );
            }
            if dropped_columns(spec) {
                out.push("Dropping a column deletes its data permanently.".to_string());
            }
            if spec.original.table.name != spec.desired.table.name {
                out.push("Views, routines and queries that use the old table name stop working until they are updated.".to_string());
            }
            if changes_type(spec) {
                out.push("Changing a column's type fails, or loses data, when existing values do not fit the new type.".to_string());
            }
        }
        DdlOperation::DuplicateTable { .. } => {
            out.push("The copy is built from the table's columns, keys and indexes; triggers, comments and grants are not copied.".to_string());
        }
        DdlOperation::CreateTable { .. } => {}
    }
    if dialect == Dialect::Mongo {
        out.push("MongoDB commands run immediately and cannot be undone.".to_string());
    } else if !transactional_ddl(engine) && statements.len() > 1 {
        out.push(format!("{} commits each DDL statement immediately, so a failure part-way leaves the earlier statements applied.", dialect.label()));
    }
    out
}

pub fn assemble(engine: Engine, operation: &DdlOperation, statements: &[String]) -> Assembled {
    Assembled { script: script(engine, operation, statements), confirm_name: confirm_name(operation), notes: notes(engine, operation, statements) }
}
