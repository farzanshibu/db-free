// SOT: schema-service, catalog-loading, column-loading, foreign-key-loading, ddl-loading, ddl-preview, describe-table

use crate::error::{AppError, AppResult};
use crate::guard::destructive::{classify, StatementKind};
use crate::guard::SessionCtx;
use crate::integrations::ddl;
use crate::model::{ColumnInfo, DdlOperation, DdlPreview, DdlProfile, ForeignKey, SchemaCatalog, TableRef, TableSpec};
use crate::state::AppState;

pub async fn catalog(ctx: &SessionCtx) -> AppResult<SchemaCatalog> {
    ctx.integration.catalog().await
}

pub async fn columns(ctx: &SessionCtx, table: &TableRef) -> AppResult<Vec<ColumnInfo>> {
    ctx.integration.columns(table).await
}

pub async fn foreign_keys(ctx: &SessionCtx) -> AppResult<Vec<ForeignKey>> {
    ctx.integration.foreign_keys().await
}

// WHAT:  The same three reads, answered from the persisted schema cache first
//        after a launch and live afterwards; `force` always goes to the adapter.
// WHERE: src-tauri/src/services/schema_cache.rs
pub async fn catalog_cached(state: &AppState, ctx: &SessionCtx, force: bool) -> AppResult<SchemaCatalog> {
    state.schema_cache().catalog(&state.store_handle(), ctx, force).await
}

pub async fn columns_cached(state: &AppState, ctx: &SessionCtx, table: &TableRef, force: bool) -> AppResult<Vec<ColumnInfo>> {
    state.schema_cache().columns(&state.store_handle(), ctx, table, force).await
}

pub async fn foreign_keys_cached(state: &AppState, ctx: &SessionCtx, force: bool) -> AppResult<Vec<ForeignKey>> {
    state.schema_cache().foreign_keys(&state.store_handle(), ctx, force).await
}

pub async fn ddl(ctx: &SessionCtx, table: &TableRef) -> AppResult<Option<String>> {
    ctx.integration.ddl(table).await
}

// WHAT:  What the schema forms offer for this engine: type picker, switches, caveats.
pub fn ddl_profile(ctx: &SessionCtx) -> DdlProfile {
    ctx.integration.ddl_profile()
}

// WHAT:  A live table as the Edit form loads it (columns, keys, defaults, indexes).
pub async fn describe_table(ctx: &SessionCtx, table: &TableRef) -> AppResult<TableSpec> {
    ctx.integration.describe_table(table).await
}

// WHAT:  The statements a schema form would run, assembled into one script, with
//        what the form must collect before running it.
// WHY:   The form never runs DDL itself: the script goes through `execute_query`, so
//        the read-only lock and the destructive confirmation apply as for any other
//        script. `destructive` is the block's own verdict on that script (the same
//        classifier), so the form asks exactly when the block would.
// HOW:   Duplicate reads the source's structure here, once, so the form sends a
//        name and two switches rather than a table description it would have to
//        keep in step with the server.
// WHERE: src-tauri/src/integrations/ddl/, src-tauri/src/guard/destructive.rs
pub async fn preview_ddl(ctx: &SessionCtx, operation: &DdlOperation) -> AppResult<DdlPreview> {
    let integration = &ctx.integration;
    let statements = match operation {
        DdlOperation::CreateTable { spec } => integration.create_table_sql(spec)?,
        DdlOperation::AlterTable { spec } => integration.alter_table_sql(spec)?,
        DdlOperation::DuplicateTable { spec } => {
            let source = integration.describe_table(&spec.source).await?;
            integration.duplicate_table_sql(spec, &source)?
        }
        DdlOperation::RemoveTable { spec } => integration.remove_table_sql(spec)?,
        DdlOperation::Namespace { spec } => integration.namespace_sql(spec)?,
    };
    if statements.is_empty() {
        return Err(AppError::invalid_input("There is nothing to change."));
    }
    let assembled = ddl::assemble(integration.engine(), operation, &statements);
    let flagged = classify(&assembled.script).iter().any(|s| s.kind == StatementKind::Destructive);
    Ok(DdlPreview {
        destructive: flagged || assembled.confirm_name.is_some(),
        statements,
        script: assembled.script,
        confirm_name: assembled.confirm_name,
        notes: assembled.notes,
    })
}
