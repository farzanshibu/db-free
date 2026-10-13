// SOT: ddl-builders, create-table-sql, alter-table-sql, duplicate-table-sql, truncate-drop-sql, namespace-sql, ddl-profile, describe-table, ddl-script

mod alter;
mod assemble;
mod create;
mod describe;
mod dialect;
mod duplicate;
#[cfg(test)]
mod e2e;
mod mongo;
mod namespace;
mod profile;
mod remove;
mod types;

use crate::error::AppResult;
use crate::integrations::Integration;
use crate::model::{AlterTableSpec, DdlOperation, DdlProfile, DuplicateSpec, Engine, NamespaceSpec, RemoveSpec, TableRef, TableSpec};
use dialect::Cx;

// ============================================================================
// DDL BUILDERS (pure) AND TABLE INTROSPECTION
//
// WHAT:  Turns the typed specs of `model::ddl` into the statements an engine runs
//        (CREATE / ALTER / duplicate / TRUNCATE / DROP, and the same for a
//        schema or database), plus the per-engine catalogue the forms are built
//        from (`profile`) and the reader that fills a `TableSpec` from a live
//        table (`describe`).
// WHY:   Identifier quoting, type spelling, auto-increment, defaults and ALTER
//        syntax differ per dialect. One builder per dialect, with unit tests,
//        keeps the webview free of SQL and the adapters free of string assembly.
// HOW:   Everything except `describe` is a pure function of (engine, spec): no
//        session, no IO. The `Integration` trait's schema-form methods default
//        to these, so an adapter that needs another spelling overrides one
//        method instead of touching a shared builder. Engines map onto eleven
//        dialects (`dialect::dialect_of`); an engine outside them is refused by
//        name, never guessed at.
// WHERE: src-tauri/src/integrations/mod.rs (trait methods), src-tauri/src/services/schema.rs
// ============================================================================

/// What a form submission becomes once its statements are built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    /// The statements as one runnable script (wrapped in a transaction where the
    /// engine's DDL is transactional).
    pub script: String,
    /// The name the user must type to confirm a destructive operation.
    pub confirm_name: Option<String>,
    /// Caveats worth reading before running.
    pub notes: Vec<String>,
}

/// The forms' catalogue for this engine: types, switches and caveats.
pub fn profile(engine: Engine) -> DdlProfile {
    profile::profile(engine)
}

pub fn create_table(engine: Engine, spec: &TableSpec) -> AppResult<Vec<String>> {
    create::create_table(&Cx::new(engine)?, spec)
}

pub fn alter_table(engine: Engine, spec: &AlterTableSpec) -> AppResult<Vec<String>> {
    alter::alter_table(&Cx::new(engine)?, spec)
}

/// `source` is the live structure of `spec.source`, as `describe` read it.
pub fn duplicate_table(engine: Engine, spec: &DuplicateSpec, source: &TableSpec) -> AppResult<Vec<String>> {
    duplicate::duplicate_table(&Cx::new(engine)?, spec, source)
}

pub fn remove_table(engine: Engine, spec: &RemoveSpec) -> AppResult<Vec<String>> {
    remove::remove_table(&Cx::new(engine)?, spec)
}

pub fn namespace(engine: Engine, spec: &NamespaceSpec) -> AppResult<Vec<String>> {
    namespace::namespace(&Cx::new(engine)?, spec)
}

/// Joins `statements` into the script that is previewed and run.
pub fn assemble(engine: Engine, operation: &DdlOperation, statements: &[String]) -> Assembled {
    assemble::assemble(engine, operation, statements)
}

/// Reads `table` from the live engine into the spec the Edit form loads.
pub async fn describe<I: Integration + ?Sized>(integration: &I, table: &TableRef) -> AppResult<TableSpec> {
    describe::describe(integration, table).await
}
