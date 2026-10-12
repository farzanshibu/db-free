// SOT: alter-table-sql, ddl-diff, ddl-column-changes, ddl-rename-table, ddl-sqlite-rebuild, ddl-constraint-changes

use super::create::{
    auto_fk_name, build_create, column_definition, default_value, duckdb_sequence, foreign_key_clause, index_statement, primary_key_clause,
    validate, ColumnUse,
};
use super::dialect::{derived_name, renamed, Cx, Dialect};
use super::mongo;
use super::profile::support_of;
use super::types::render_type;
use crate::error::{AppError, AppResult};
use crate::model::{AlterTableSpec, ColumnDefault, ColumnSpec, Engine, ForeignKeySpec, IndexSpec, TableRef};
use std::cell::Cell;
use std::collections::HashSet;

// WHAT:  ALTER TABLE from an (original, desired) pair: rename the table, add, drop
//        and rename columns, change type / nullability / default / uniqueness /
//        auto-increment, change the primary key, add and drop foreign keys and
//        indexes.
// WHY:   The Edit form shows the table as it is and lets the user change it
//        freely; what to run is a diff, and a diff is only trustworthy if it is
//        computed from the same description on both sides (so an untouched column
//        produces no statement) and ordered so every statement's dependencies
//        still exist when it runs.
// HOW:   Columns and constraints keep their identity through `original_name`.
//        Statements run in a fixed order: drop what is going away (indexes, keys,
//        columns), rename, modify, add, then create indexes, and rename the table
//        last so every earlier statement can use one name. SQLite can do little in
//        place; whatever it cannot is done by rebuilding the table (create the new
//        shape, copy the rows, drop, rename), the procedure SQLite documents.
// WHERE: src-tauri/src/integrations/ddl/mod.rs

/// Suffix of the scratch table a SQLite rebuild fills before it replaces the original.
pub const REBUILD_SUFFIX: &str = "__rebuild";

struct Pair<'a> {
    old: &'a ColumnSpec,
    new: &'a ColumnSpec,
}

fn pairs_of(spec: &AlterTableSpec) -> Vec<Pair<'_>> {
    spec.desired
        .columns
        .iter()
        .filter_map(|new| {
            let name = new.original_name.as_deref()?;
            spec.original.columns.iter().find(|c| c.name == name).map(|old| Pair { old, new })
        })
        .collect()
}

/// Names of the original columns the desired table still has.
pub fn matched_original(spec: &AlterTableSpec) -> HashSet<String> {
    spec.desired.columns.iter().filter_map(|c| c.original_name.clone()).collect()
}

fn type_fields_differ(a: &ColumnSpec, b: &ColumnSpec) -> bool {
    !a.data_type.eq_ignore_ascii_case(&b.data_type)
        || a.length != b.length
        || a.precision != b.precision
        || a.scale != b.scale
        || a.unsigned != b.unsigned
        || a.wrapper != b.wrapper
        || a.enum_values != b.enum_values
}

/// A column changes type (used only to word a caveat; the diff itself compares rendered types).
pub fn changes_type(spec: &AlterTableSpec) -> bool {
    pairs_of(spec).iter().any(|p| type_fields_differ(p.old, p.new))
}

fn effective_nullable(c: &ColumnSpec) -> bool {
    c.nullable && !c.primary_key
}

/// `DEFAULT NULL` and no default are the same thing on a nullable column.
fn normalised_default(c: &ColumnSpec) -> ColumnDefault {
    match c.default {
        ColumnDefault::Null => ColumnDefault::None,
        _ => c.default.clone(),
    }
}

fn has_default(c: &ColumnSpec) -> bool {
    normalised_default(c) != ColumnDefault::None
}

fn schema_of(table: &TableRef) -> Option<&str> {
    table.schema.as_deref().filter(|s| !s.is_empty())
}

struct Plan<'a> {
    cx: &'a Cx,
    spec: &'a AlterTableSpec,
    pairs: Vec<Pair<'a>>,
    added: Vec<&'a ColumnSpec>,
    dropped: Vec<&'a ColumnSpec>,
    /// SQL Server needs a fresh variable name for every dynamic-SQL snippet in a script.
    snippets: Cell<u32>,
}

impl<'a> Plan<'a> {
    fn build(cx: &'a Cx, spec: &'a AlterTableSpec) -> AppResult<Plan<'a>> {
        validate(cx, &spec.desired)?;
        if schema_of(&spec.original.table) != schema_of(&spec.desired.table) {
            return Err(AppError::invalid_input("A table cannot be moved to another schema from this form."));
        }
        let mut seen: HashSet<&str> = HashSet::new();
        let mut pairs = Vec::new();
        let mut added = Vec::new();
        for new in &spec.desired.columns {
            match new.original_name.as_deref() {
                Some(name) => {
                    let Some(old) = spec.original.columns.iter().find(|c| c.name == name) else {
                        return Err(AppError::invalid_input(format!("Column \"{name}\" is no longer in the table; reload the table and try again.")));
                    };
                    if !seen.insert(name) {
                        return Err(AppError::invalid_input(format!("Column \"{name}\" is listed twice.")));
                    }
                    pairs.push(Pair { old, new });
                }
                None => added.push(new),
            }
        }
        let dropped = spec.original.columns.iter().filter(|c| !seen.contains(c.name.as_str())).collect();
        Ok(Plan { cx, spec, pairs, added, dropped, snippets: Cell::new(0) })
    }

    fn table(&self) -> &TableRef {
        &self.spec.original.table
    }

    /// The statement prefix. Every statement before the rename uses the original name.
    fn at(&self) -> String {
        format!("ALTER TABLE {}", self.cx.table(self.table()))
    }

    fn rename_to(&self) -> Option<&str> {
        let to = self.spec.desired.table.name.as_str();
        (to != self.spec.original.table.name).then_some(to)
    }

    fn snippet(&self) -> u32 {
        let n = self.snippets.get() + 1;
        self.snippets.set(n);
        n
    }

    fn final_name(&self, original: &str) -> String {
        self.pairs.iter().find(|p| p.old.name == original).map_or_else(|| original.to_string(), |p| p.new.name.clone())
    }

    fn type_changed(&self, p: &Pair<'_>) -> AppResult<bool> {
        let new = render_type(self.cx.dialect, p.new)?;
        Ok(render_type(self.cx.dialect, p.old).map_or(true, |old| old != new))
    }

    fn null_changed(&self, p: &Pair<'_>) -> bool {
        // ClickHouse spells nullability inside the type, which `type_changed` covers.
        self.cx.dialect != Dialect::Clickhouse && effective_nullable(p.old) != effective_nullable(p.new)
    }

    fn default_changed(&self, p: &Pair<'_>) -> bool {
        normalised_default(p.old) != normalised_default(p.new)
    }

    fn auto_changed(&self, p: &Pair<'_>) -> bool {
        p.old.auto_increment != p.new.auto_increment
    }

    /// Whether the primary key changes, and if it does, the key as it must end up.
    fn pk_change(&self) -> Option<Vec<String>> {
        let old: Vec<Option<String>> =
            self.spec.original.columns.iter().filter(|c| c.primary_key).map(|c| self.pairs.iter().find(|p| p.old.name == c.name).map(|p| p.new.name.clone())).collect();
        let new: Vec<String> = self.spec.desired.columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect();
        let unchanged = old.iter().all(Option::is_some) && old.iter().flatten().cloned().collect::<Vec<_>>() == new;
        (!unchanged).then_some(new)
    }

    fn had_primary_key(&self) -> bool {
        self.spec.original.columns.iter().any(|c| c.primary_key)
    }

    fn same_reference(&self, a: &TableRef, b: &TableRef) -> bool {
        let home = schema_of(self.table());
        a.name == b.name && schema_of(a).or(home) == schema_of(b).or(home)
    }

    fn fk_unchanged(&self, old: &ForeignKeySpec, new: &ForeignKeySpec) -> bool {
        let mapped: Vec<String> = old.columns.iter().map(|c| self.final_name(c)).collect();
        let renamed_constraint = matches!((&old.name, &new.name), (Some(a), Some(b)) if a != b);
        mapped == new.columns
            && old.ref_columns == new.ref_columns
            && old.on_delete == new.on_delete
            && old.on_update == new.on_update
            && self.same_reference(&old.ref_table, &new.ref_table)
            && !renamed_constraint
    }

    /// Foreign keys to drop (live ones that are gone or changed) and to add (new or changed).
    fn fk_changes(&self) -> (Vec<&'a ForeignKeySpec>, Vec<&'a ForeignKeySpec>) {
        let spec = self.spec;
        let key = |fk: &ForeignKeySpec| fk.original_name.clone().or_else(|| fk.name.clone());
        let mut drops = Vec::new();
        let mut adds = Vec::new();
        let mut kept: HashSet<String> = HashSet::new();
        for old in &spec.original.foreign_keys {
            let identity = key(old);
            let counterpart = spec.desired.foreign_keys.iter().find(|n| n.original_name.is_some() && n.original_name == identity);
            match counterpart {
                Some(new) if self.fk_unchanged(old, new) => {
                    if let Some(i) = identity {
                        kept.insert(i);
                    }
                }
                _ => drops.push(old),
            }
        }
        for new in &spec.desired.foreign_keys {
            let is_kept = new.original_name.as_ref().is_some_and(|n| kept.contains(n));
            if !is_kept {
                adds.push(new);
            }
        }
        (drops, adds)
    }

    fn index_unchanged(&self, old: &IndexSpec, new: &IndexSpec) -> bool {
        let mapped: Vec<String> = old.columns.iter().map(|c| self.final_name(c)).collect();
        old.name == new.name && old.unique == new.unique && mapped == new.columns
    }

    fn index_changes(&self) -> (Vec<&'a IndexSpec>, Vec<&'a IndexSpec>) {
        let spec = self.spec;
        let drops = spec.original.indexes.iter().filter(|o| !spec.desired.indexes.iter().any(|n| self.index_unchanged(o, n))).collect();
        let adds = spec.desired.indexes.iter().filter(|n| !spec.original.indexes.iter().any(|o| self.index_unchanged(o, n))).collect();
        (drops, adds)
    }

    /// Refuses, by name, every change the dialect cannot make on a live table.
    fn require(&self, drop_fks: &[&ForeignKeySpec], add_fks: &[&ForeignKeySpec], pk_changes: bool) -> AppResult<()> {
        let cx = self.cx;
        let sup = support_of(cx.dialect);
        let refuse = |ok: bool, what: &str| if ok { Ok(()) } else { Err(cx.cannot(what)) };
        refuse(sup.alter_table, "alter tables")?;
        refuse(self.rename_to().is_none() || sup.rename_table, "rename a table")?;
        refuse(self.dropped.is_empty() || sup.drop_column, "drop a column")?;
        refuse(!self.pairs.iter().any(|p| p.old.name != p.new.name) || sup.rename_column, "rename a column (only primary-key columns can be renamed, and not from this form)")?;
        for p in &self.pairs {
            refuse(!self.type_changed(p)? || sup.change_type, "change a column's type")?;
            refuse(!self.null_changed(p) || sup.change_nullability, "change whether a column allows NULL")?;
            refuse(!self.default_changed(p) || sup.change_default, "change a column's default")?;
            refuse(p.old.unique == p.new.unique || sup.change_unique, "add or drop a UNIQUE constraint on an existing table (define it when creating the table)")?;
        }
        for c in &self.added {
            refuse(!c.unique || sup.change_unique || cx.dialect != Dialect::Duckdb, "add a UNIQUE column to an existing table (define it when creating the table)")?;
        }
        if cx.dialect == Dialect::Duckdb {
            // DuckDB refuses to rename, retype or drop a column that a key or an index depends on.
            let original = &self.spec.original;
            let depended_on = |name: &str, own_key: bool| {
                own_key || original.foreign_keys.iter().any(|f| f.columns.iter().any(|c| c == name)) || original.indexes.iter().any(|i| i.columns.iter().any(|c| c == name))
            };
            for p in &self.pairs {
                let verb = if p.old.name != p.new.name {
                    Some("rename")
                } else if self.type_changed(p)? {
                    Some("change the type of")
                } else {
                    None
                };
                if let Some(verb) = verb {
                    if depended_on(&p.old.name, p.old.primary_key || p.old.unique) {
                        return Err(cx.cannot(&format!("{verb} column \"{}\" while a key or index depends on it", p.old.name)));
                    }
                }
            }
            for c in &self.dropped {
                if depended_on(&c.name, c.primary_key || c.unique) {
                    return Err(cx.cannot(&format!("drop column \"{}\" while a key or index depends on it", c.name)));
                }
            }
        }
        refuse(!pk_changes || sup.change_primary_key, "change the primary key of an existing table (define it when creating the table)")?;
        refuse((drop_fks.is_empty() && add_fks.is_empty()) || sup.alter_foreign_keys, "add or drop a foreign key on an existing table (define it when creating the table)")?;
        if cx.dialect == Dialect::Clickhouse {
            let (a, b) = (&self.spec.original, &self.spec.desired);
            if let (Some(x), Some(y)) = (&a.engine, &b.engine) {
                if !x.eq_ignore_ascii_case(y) {
                    return Err(cx.cannot("change a table's ENGINE (create a new table and copy the rows)"));
                }
            }
            if !a.order_by.is_empty() && !b.order_by.is_empty() && a.order_by != b.order_by {
                return Err(cx.cannot("change a table's ORDER BY (create a new table and copy the rows)"));
            }
        }
        Ok(())
    }
}

pub fn alter_table(cx: &Cx, spec: &AlterTableSpec) -> AppResult<Vec<String>> {
    if cx.dialect == Dialect::Mongo {
        return if spec.desired.table.name == spec.original.table.name { Ok(Vec::new()) } else { mongo::rename_collection(&spec.original.table, &spec.desired.table.name) };
    }
    let plan = Plan::build(cx, spec)?;
    if cx.dialect == Dialect::Sqlite {
        return sqlite(&plan);
    }
    generic(&plan)
}

// ---- the dialects that ALTER in place --------------------------------------------------

fn generic(plan: &Plan<'_>) -> AppResult<Vec<String>> {
    let cx = plan.cx;
    let (drop_fks, add_fks) = plan.fk_changes();
    let (drop_indexes, add_indexes) = plan.index_changes();
    let new_pk = plan.pk_change();
    plan.require(&drop_fks, &add_fks, new_pk.is_some())?;

    let mut out: Vec<String> = Vec::new();
    for index in &drop_indexes {
        out.push(drop_index(plan, index)?);
    }
    for fk in &drop_fks {
        out.push(drop_foreign_key(plan, fk)?);
    }
    if new_pk.is_some() && plan.had_primary_key() {
        out.extend(drop_primary_key(plan)?);
    }
    for p in plan.pairs.iter().filter(|p| p.old.unique && !p.new.unique) {
        out.extend(drop_unique(plan, p.old)?);
    }
    for column in &plan.dropped {
        out.extend(drop_column(plan, column)?);
    }
    for p in plan.pairs.iter().filter(|p| p.old.name != p.new.name) {
        out.push(rename_column(plan, p));
    }
    for p in &plan.pairs {
        out.extend(modify_column(plan, p)?);
    }
    for column in &plan.added {
        out.extend(add_column(plan, column)?);
    }
    for p in plan.pairs.iter().filter(|p| !p.old.unique && p.new.unique) {
        out.push(add_unique(plan, p.new)?);
    }
    if let Some(columns) = &new_pk {
        if !columns.is_empty() {
            out.push(add_primary_key(plan, columns)?);
        }
    }
    for fk in &add_fks {
        out.push(add_foreign_key(plan, fk)?);
    }
    for index in &add_indexes {
        if let Some(sql) = index_statement(cx, plan.table(), index) {
            out.push(sql);
        }
    }
    if let Some(to) = plan.rename_to() {
        out.push(rename_table(plan, to)?);
    }
    Ok(out)
}

fn rename_table(plan: &Plan<'_>, to: &str) -> AppResult<String> {
    let cx = plan.cx;
    let from = cx.table(plan.table());
    Ok(match cx.dialect {
        Dialect::Mysql => format!("RENAME TABLE {from} TO {}", cx.table(&renamed(plan.table(), to))),
        Dialect::Clickhouse => format!("RENAME TABLE {from} TO {}", cx.table(&renamed(plan.table(), to))),
        Dialect::Mssql => format!("EXEC sp_rename N{}, N{}", cx.lit(&cx.table(plan.table())), cx.lit(to)),
        Dialect::Cassandra => return Err(cx.cannot("rename a table")),
        _ => format!("ALTER TABLE {from} RENAME TO {}", cx.q(to)),
    })
}

fn rename_column(plan: &Plan<'_>, p: &Pair<'_>) -> String {
    let cx = plan.cx;
    if cx.dialect == Dialect::Mssql {
        let object = format!("{}.{}", cx.table(plan.table()), cx.q(&p.old.name));
        return format!("EXEC sp_rename N{}, N{}, N'COLUMN'", cx.lit(&object), cx.lit(&p.new.name));
    }
    format!("{} RENAME COLUMN {} TO {}", plan.at(), cx.q(&p.old.name), cx.q(&p.new.name))
}

/// SQL Server names default constraints itself; this finds and drops the live one.
fn mssql_drop_default(plan: &Plan<'_>, column: &str) -> String {
    let cx = plan.cx;
    let n = plan.snippet();
    let object = cx.lit(&cx.table(plan.table()));
    format!(
        "DECLARE @df{n} sysname = (SELECT dc.name FROM sys.default_constraints dc JOIN sys.columns c ON c.default_object_id = dc.object_id \
         WHERE dc.parent_object_id = OBJECT_ID(N{object}) AND c.name = N{col}) \
         IF @df{n} IS NOT NULL EXEC(N'ALTER TABLE ' + N{object} + N' DROP CONSTRAINT ' + QUOTENAME(@df{n}))",
        col = cx.lit(column)
    )
}

fn drop_column(plan: &Plan<'_>, column: &ColumnSpec) -> AppResult<Vec<String>> {
    let cx = plan.cx;
    let at = plan.at();
    let name = cx.q(&column.name);
    Ok(match cx.dialect {
        Dialect::Mssql => {
            let mut out = vec![mssql_drop_default(plan, &column.name)];
            if column.unique {
                out.extend(drop_unique(plan, column)?);
            }
            out.push(format!("{at} DROP COLUMN {name}"));
            out
        }
        Dialect::Cassandra => vec![format!("{at} DROP {name}")],
        _ => vec![format!("{at} DROP COLUMN {name}")],
    })
}

fn add_column(plan: &Plan<'_>, column: &ColumnSpec) -> AppResult<Vec<String>> {
    let cx = plan.cx;
    let at = plan.at();
    let definition = column_definition(cx, column, ColumnUse { table: plan.table(), sole_primary_key: false, adding: true })?;
    let mut out = Vec::new();
    match cx.dialect {
        Dialect::Mssql => out.push(format!("{at} ADD {definition}")),
        Dialect::Oracle => out.push(format!("{at} ADD ({definition})")),
        Dialect::Cassandra => out.push(format!("{at} ADD {definition}")),
        Dialect::Duckdb => {
            if column.auto_increment {
                out.push(format!("CREATE SEQUENCE {}", cx.table(&duckdb_sequence(plan.table(), &column.name))));
            }
            out.push(format!("{at} ADD COLUMN {definition}"));
            if !effective_nullable(column) {
                out.push(format!("{at} ALTER COLUMN {} SET NOT NULL", cx.q(&column.name)));
            }
        }
        Dialect::Bigquery => {
            if !effective_nullable(column) {
                return Err(cx.cannot("add a NOT NULL column to an existing table (add it as nullable)"));
            }
            out.push(format!("{at} ADD COLUMN {definition}"));
        }
        _ => out.push(format!("{at} ADD COLUMN {definition}")),
    }
    Ok(out)
}

/// Statements that bring one existing column from its old definition to its new one.
fn modify_column(plan: &Plan<'_>, p: &Pair<'_>) -> AppResult<Vec<String>> {
    let cx = plan.cx;
    let at = plan.at();
    let name = cx.q(&p.new.name);
    let type_changed = plan.type_changed(p)?;
    let null_changed = plan.null_changed(p);
    let default_changed = plan.default_changed(p);
    let auto_changed = plan.auto_changed(p);
    if !(type_changed || null_changed || default_changed || auto_changed) {
        return Ok(Vec::new());
    }
    let ty = render_type(cx.dialect, p.new)?;
    let new_default = default_value(cx, p.new)?;
    let mut out = Vec::new();
    match cx.dialect {
        Dialect::Postgres => {
            let had_default = has_default(p.old);
            if type_changed && had_default {
                out.push(format!("{at} ALTER COLUMN {name} DROP DEFAULT"));
            }
            if type_changed {
                out.push(format!("{at} ALTER COLUMN {name} TYPE {ty} USING {name}::{ty}"));
            }
            if null_changed || (auto_changed && p.new.auto_increment && effective_nullable(p.old)) {
                out.push(format!("{at} ALTER COLUMN {name} {} NOT NULL", if effective_nullable(p.new) && !p.new.auto_increment { "DROP" } else { "SET" }));
            }
            if auto_changed {
                if p.new.auto_increment {
                    out.push(format!("{at} ALTER COLUMN {name} ADD GENERATED BY DEFAULT AS IDENTITY"));
                } else {
                    out.push(format!("{at} ALTER COLUMN {name} DROP IDENTITY IF EXISTS"));
                    out.push(format!("{at} ALTER COLUMN {name} DROP DEFAULT"));
                }
            }
            let dropped_already = type_changed && had_default;
            match new_default {
                Some(d) if default_changed || dropped_already => out.push(format!("{at} ALTER COLUMN {name} SET DEFAULT {d}")),
                None if default_changed && !dropped_already => out.push(format!("{at} ALTER COLUMN {name} DROP DEFAULT")),
                _ => {}
            }
        }
        Dialect::Mysql => {
            if type_changed || null_changed || auto_changed {
                // MODIFY restates the column, so the clause must not repeat its UNIQUE.
                let mut column = p.new.clone();
                column.unique = false;
                let definition = column_definition(cx, &column, ColumnUse { table: plan.table(), sole_primary_key: false, adding: false })?;
                out.push(format!("{at} MODIFY COLUMN {definition}"));
            } else if let Some(d) = new_default {
                out.push(format!("{at} ALTER COLUMN {name} SET DEFAULT {d}"));
            } else {
                out.push(format!("{at} ALTER COLUMN {name} DROP DEFAULT"));
            }
        }
        Dialect::Mssql => {
            if auto_changed {
                return Err(cx.cannot("add or remove IDENTITY on an existing column (add a new column and copy the values)"));
            }
            let had_default = has_default(p.old);
            let redefault = default_changed || (type_changed && had_default);
            if redefault && had_default {
                out.push(mssql_drop_default(plan, &p.old.name));
            }
            if type_changed || null_changed {
                out.push(format!("{at} ALTER COLUMN {name} {ty} {}", if effective_nullable(p.new) { "NULL" } else { "NOT NULL" }));
            }
            if redefault {
                if let Some(d) = new_default {
                    let constraint = cx.q(&derived_name(&["DF", &plan.spec.desired.table.name, &p.new.name]));
                    out.push(format!("{at} ADD CONSTRAINT {constraint} DEFAULT {d} FOR {name}"));
                }
            }
        }
        Dialect::Oracle => {
            let mut clauses = Vec::new();
            if type_changed {
                clauses.push(ty);
            }
            if default_changed {
                clauses.push(format!("DEFAULT {}", new_default.unwrap_or_else(|| "NULL".to_string())));
            }
            if auto_changed {
                clauses.push(if p.new.auto_increment { "GENERATED BY DEFAULT AS IDENTITY" } else { "DROP IDENTITY" }.to_string());
            }
            if null_changed {
                clauses.push(if effective_nullable(p.new) { "NULL" } else { "NOT NULL" }.to_string());
            }
            out.push(format!("{at} MODIFY ({name} {})", clauses.join(" ")));
        }
        Dialect::Duckdb | Dialect::Snowflake | Dialect::Bigquery => {
            if auto_changed {
                return Err(cx.cannot("add or remove auto-increment on an existing column (add a new column and copy the values)"));
            }
            if type_changed {
                out.push(format!("{at} ALTER COLUMN {name} SET DATA TYPE {ty}"));
            }
            if null_changed {
                if effective_nullable(p.new) {
                    out.push(format!("{at} ALTER COLUMN {name} DROP NOT NULL"));
                } else if cx.dialect == Dialect::Bigquery {
                    return Err(cx.cannot("make an existing column NOT NULL"));
                } else {
                    out.push(format!("{at} ALTER COLUMN {name} SET NOT NULL"));
                }
            }
            if default_changed {
                match new_default {
                    Some(d) => out.push(format!("{at} ALTER COLUMN {name} SET DEFAULT {d}")),
                    None => out.push(format!("{at} ALTER COLUMN {name} DROP DEFAULT")),
                }
            }
        }
        Dialect::Clickhouse => {
            if auto_changed {
                return Err(cx.cannot("use auto-increment columns"));
            }
            if type_changed || (new_default.is_some() && default_changed) {
                let default = new_default.as_ref().map(|d| format!(" DEFAULT {d}")).unwrap_or_default();
                out.push(format!("{at} MODIFY COLUMN {name} {ty}{default}"));
            }
            if new_default.is_none() && default_changed && has_default(p.old) {
                out.push(format!("{at} MODIFY COLUMN {name} REMOVE DEFAULT"));
            }
        }
        Dialect::Cassandra => return Err(cx.cannot("change a column once the table exists (columns can only be added or dropped)")),
        Dialect::Sqlite | Dialect::Mongo => return Err(cx.cannot("change a column in place")),
    }
    Ok(out)
}

// ---- constraints -----------------------------------------------------------------------

fn unique_name(plan: &Plan<'_>, column: &str) -> String {
    let table = &plan.spec.desired.table.name;
    match plan.cx.dialect {
        Dialect::Postgres => derived_name(&[table, column, "key"]),
        _ => derived_name(&["uq", table, column]),
    }
}

fn add_unique(plan: &Plan<'_>, column: &ColumnSpec) -> AppResult<String> {
    let cx = plan.cx;
    Ok(format!("{} ADD CONSTRAINT {} UNIQUE ({})", plan.at(), cx.q(&unique_name(plan, &column.name)), cx.q(&column.name)))
}

fn drop_unique(plan: &Plan<'_>, column: &ColumnSpec) -> AppResult<Vec<String>> {
    let cx = plan.cx;
    let at = plan.at();
    let table = &plan.table().name;
    Ok(match cx.dialect {
        Dialect::Postgres => {
            let name = column.unique_name.clone().unwrap_or_else(|| derived_name(&[table, &column.name, "key"]));
            // A UNIQUE constraint and a standalone unique index drop differently; whichever it is, the other is a no-op.
            vec![
                format!("{at} DROP CONSTRAINT IF EXISTS {}", cx.q(&name)),
                format!("DROP INDEX IF EXISTS {}", cx.table(&renamed(plan.table(), &name))),
            ]
        }
        Dialect::Mysql => {
            let name = column.unique_name.clone().unwrap_or_else(|| column.name.clone());
            vec![format!("{at} DROP INDEX {}", cx.q(&name))]
        }
        Dialect::Mssql => {
            let name = column.unique_name.clone().unwrap_or_else(|| unique_name(plan, &column.name));
            let object = cx.lit(&cx.table(plan.table()));
            vec![format!(
                "IF EXISTS (SELECT 1 FROM sys.key_constraints WHERE name = N{n} AND parent_object_id = OBJECT_ID(N{object})) {at} DROP CONSTRAINT {q} ELSE DROP INDEX {q} ON {t}",
                n = cx.lit(&name),
                q = cx.q(&name),
                t = cx.table(plan.table())
            )]
        }
        Dialect::Oracle | Dialect::Snowflake => match &column.unique_name {
            Some(name) if cx.dialect == Dialect::Oracle => vec![format!("{at} DROP CONSTRAINT {}", cx.q(name))],
            _ => vec![format!("{at} DROP UNIQUE ({})", cx.q(&column.name))],
        },
        _ => return Err(cx.cannot("drop a UNIQUE constraint")),
    })
}

fn drop_primary_key(plan: &Plan<'_>) -> AppResult<Vec<String>> {
    let cx = plan.cx;
    let at = plan.at();
    Ok(match cx.dialect {
        Dialect::Postgres => {
            let name = plan.spec.original.primary_key_name.clone().unwrap_or_else(|| derived_name(&[&plan.table().name, "pkey"]));
            vec![format!("{at} DROP CONSTRAINT {}", cx.q(&name))]
        }
        Dialect::Mysql | Dialect::Oracle | Dialect::Snowflake => vec![format!("{at} DROP PRIMARY KEY")],
        Dialect::Mssql => {
            let n = plan.snippet();
            let object = cx.lit(&cx.table(plan.table()));
            vec![format!(
                "DECLARE @pk{n} sysname = (SELECT name FROM sys.key_constraints WHERE type = 'PK' AND parent_object_id = OBJECT_ID(N{object})) \
                 IF @pk{n} IS NOT NULL EXEC(N'ALTER TABLE ' + N{object} + N' DROP CONSTRAINT ' + QUOTENAME(@pk{n}))"
            )]
        }
        _ => return Err(cx.cannot("drop a primary key")),
    })
}

fn add_primary_key(plan: &Plan<'_>, columns: &[String]) -> AppResult<String> {
    let cx = plan.cx;
    let at = plan.at();
    let refs: Vec<&str> = columns.iter().map(String::as_str).collect();
    let clause = primary_key_clause(cx, &refs);
    let table = &plan.spec.desired.table.name;
    Ok(match cx.dialect {
        Dialect::Postgres => format!("{at} ADD CONSTRAINT {} {clause}", cx.q(&derived_name(&[table, "pkey"]))),
        Dialect::Mssql => format!("{at} ADD CONSTRAINT {} {clause}", cx.q(&derived_name(&["PK", table]))),
        _ => format!("{at} ADD {clause}"),
    })
}

fn foreign_key_identity(fk: &ForeignKeySpec) -> Option<String> {
    fk.original_name.clone().or_else(|| fk.name.clone()).filter(|n| !n.trim().is_empty())
}

fn drop_foreign_key(plan: &Plan<'_>, fk: &ForeignKeySpec) -> AppResult<String> {
    let cx = plan.cx;
    let at = plan.at();
    if cx.dialect == Dialect::Snowflake {
        return Ok(format!("{at} DROP FOREIGN KEY ({})", cx.columns(&fk.columns)));
    }
    let Some(name) = foreign_key_identity(fk) else {
        return Err(AppError::invalid_input(format!("The foreign key on ({}) has no known name to drop; reload the table.", fk.columns.join(", "))));
    };
    Ok(match cx.dialect {
        Dialect::Mysql => format!("{at} DROP FOREIGN KEY {}", cx.q(&name)),
        _ => format!("{at} DROP CONSTRAINT {}", cx.q(&name)),
    })
}

fn add_foreign_key(plan: &Plan<'_>, fk: &ForeignKeySpec) -> AppResult<String> {
    let cx = plan.cx;
    // A self-reference written with the new table name must still resolve before the rename.
    let mut fk = fk.clone();
    if plan.rename_to().is_some() && fk.ref_table.name == plan.spec.desired.table.name && schema_of(&fk.ref_table).or(schema_of(plan.table())) == schema_of(plan.table()) {
        fk.ref_table.name = plan.table().name.clone();
    }
    if fk.name.as_deref().is_none_or(|n| n.trim().is_empty()) {
        fk.name = Some(auto_fk_name(&plan.spec.desired.table.name, &fk));
    }
    Ok(format!("{} ADD {}", plan.at(), foreign_key_clause(cx, &plan.spec.desired.table.name, &fk, true)?))
}

fn drop_index(plan: &Plan<'_>, index: &IndexSpec) -> AppResult<String> {
    let cx = plan.cx;
    Ok(match cx.dialect {
        Dialect::Mysql | Dialect::Mssql => format!("DROP INDEX {} ON {}", cx.q(&index.name), cx.table(plan.table())),
        Dialect::Postgres | Dialect::Oracle | Dialect::Duckdb => format!("DROP INDEX {}", cx.table(&renamed(plan.table(), &index.name))),
        _ => return Err(cx.cannot("drop an index")),
    })
}

// ---- SQLite ----------------------------------------------------------------------------

/// Whether ADD COLUMN can carry this column (SQLite refuses keys, non-constant
/// defaults and NOT NULL without a default).
fn sqlite_can_add(plan: &Plan<'_>, column: &ColumnSpec) -> AppResult<bool> {
    if column.primary_key || column.unique || column.auto_increment {
        return Ok(false);
    }
    let default = default_value(plan.cx, column)?;
    let constant = default.as_ref().is_none_or(|d| !d.starts_with('(') && !d.to_ascii_uppercase().starts_with("CURRENT_"));
    let satisfiable = effective_nullable(column) || default.as_deref().is_some_and(|d| !d.eq_ignore_ascii_case("NULL"));
    Ok(constant && satisfiable)
}

/// Whether DROP COLUMN will be accepted (not a key, not indexed, not in a foreign key).
fn sqlite_can_drop(plan: &Plan<'_>, column: &ColumnSpec) -> bool {
    let original = &plan.spec.original;
    !column.primary_key
        && !column.unique
        && !original.indexes.iter().any(|i| i.columns.contains(&column.name))
        && !original.foreign_keys.iter().any(|f| f.columns.contains(&column.name))
}

fn sqlite(plan: &Plan<'_>) -> AppResult<Vec<String>> {
    let cx = plan.cx;
    let (drop_fks, add_fks) = plan.fk_changes();
    let (drop_indexes, add_indexes) = plan.index_changes();
    let pk_changes = plan.pk_change().is_some();
    plan.require(&drop_fks, &add_fks, pk_changes)?;

    let mut rebuild = pk_changes || !drop_fks.is_empty() || !add_fks.is_empty();
    for p in &plan.pairs {
        rebuild |= plan.type_changed(p)? || plan.null_changed(p) || plan.default_changed(p) || plan.auto_changed(p) || p.old.unique != p.new.unique;
    }
    for column in &plan.added {
        rebuild |= !sqlite_can_add(plan, column)?;
    }
    rebuild |= plan.dropped.iter().any(|c| !sqlite_can_drop(plan, c));

    if rebuild {
        return sqlite_rebuild(plan);
    }
    let mut out = Vec::new();
    let at = plan.at();
    for index in &drop_indexes {
        out.push(format!("DROP INDEX {}", cx.table(&renamed(plan.table(), &index.name))));
    }
    for column in &plan.dropped {
        out.push(format!("{at} DROP COLUMN {}", cx.q(&column.name)));
    }
    for p in plan.pairs.iter().filter(|p| p.old.name != p.new.name) {
        out.push(rename_column(plan, p));
    }
    for column in &plan.added {
        let definition = column_definition(cx, column, ColumnUse { table: plan.table(), sole_primary_key: false, adding: true })?;
        out.push(format!("{at} ADD COLUMN {definition}"));
    }
    for index in &add_indexes {
        out.extend(index_statement(cx, plan.table(), index));
    }
    if let Some(to) = plan.rename_to() {
        out.push(rename_table(plan, to)?);
    }
    Ok(out)
}

// WHAT:  Replaces the table with a rebuilt copy: create the new shape, copy the
//        rows, drop the old table, rename the copy, recreate the indexes.
// WHY:   SQLite's ALTER TABLE cannot retype a column, change nullability,
//        defaults, keys or foreign keys. This is the procedure its documentation
//        prescribes, with foreign-key enforcement off so that dropping the old
//        table cannot cascade into tables that reference it.
// HOW:   A table rename is done first as its own ALTER (so SQLite rewrites the
//        foreign keys of other tables that point at it). The PRAGMAs stay outside
//        the transaction the script is wrapped in. A NOT NULL column gaining a
//        default copies COALESCE(old, default) so existing NULLs do not abort it.
fn sqlite_rebuild(plan: &Plan<'_>) -> AppResult<Vec<String>> {
    let cx = plan.cx;
    let spec = plan.spec;
    let d1 = cx.engine == Engine::CloudflareD1;
    let mut out = vec![if d1 { "PRAGMA defer_foreign_keys = ON" } else { "PRAGMA foreign_keys = OFF" }.to_string()];

    let final_name = spec.desired.table.name.clone();
    if let Some(to) = plan.rename_to() {
        out.push(rename_table(plan, to)?);
    }
    let live = renamed(plan.table(), &final_name);
    let scratch = renamed(&live, &format!("{final_name}{REBUILD_SUFFIX}"));
    out.push(format!("DROP TABLE IF EXISTS {}", cx.table(&scratch)));

    let mut shape = spec.desired.clone();
    shape.table = scratch.clone();
    shape.indexes.clear();
    shape.if_not_exists = false;
    for fk in &mut shape.foreign_keys {
        if fk.ref_table.name == spec.original.table.name {
            fk.ref_table.name.clone_from(&final_name);
        }
    }
    out.push(build_create(cx, &shape)?.table);

    if !plan.pairs.is_empty() {
        let targets = plan.pairs.iter().map(|p| cx.q(&p.new.name)).collect::<Vec<_>>().join(", ");
        let mut sources = Vec::new();
        for p in &plan.pairs {
            let column = cx.q(&p.old.name);
            let fill = default_value(cx, p.new)?.filter(|d| !d.eq_ignore_ascii_case("NULL"));
            sources.push(match fill {
                Some(d) if !effective_nullable(p.new) && effective_nullable(p.old) => format!("COALESCE({column}, {d})"),
                _ => column,
            });
        }
        out.push(format!("INSERT INTO {} ({targets}) SELECT {} FROM {}", cx.table(&scratch), sources.join(", "), cx.table(&live)));
    }
    out.push(format!("DROP TABLE {}", cx.table(&live)));
    out.push(format!("ALTER TABLE {} RENAME TO {}", cx.table(&scratch), cx.q(&final_name)));
    for index in &spec.desired.indexes {
        out.extend(index_statement(cx, &live, index));
    }
    if !d1 {
        out.push("PRAGMA foreign_keys = ON".to_string());
    }
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::integrations::ddl::create::tests::{fk, pk, spec, table};
    use crate::integrations::ddl::types::tests::col;
    use crate::model::{FkAction, TableSpec};

    /// A described table: every column and constraint is the live one.
    pub fn live(mut table: TableSpec) -> TableSpec {
        for c in &mut table.columns {
            c.original_name = Some(c.name.clone());
        }
        for f in &mut table.foreign_keys {
            if f.original_name.is_none() {
                f.original_name = f.name.clone();
            }
        }
        table
    }

    fn alter(engine: Engine, original: &TableSpec, edit: impl FnOnce(&mut TableSpec)) -> Vec<String> {
        let mut desired = original.clone();
        edit(&mut desired);
        super::super::alter_table(engine, &AlterTableSpec { original: original.clone(), desired }).unwrap_or_else(|e| vec![format!("ERR {e}")])
    }

    fn people(engine_types: (&str, &str)) -> TableSpec {
        let (int, text) = engine_types;
        let mut name = col("name", text);
        name.length = Some(80);
        name.nullable = false;
        let mut bio = col("bio", text);
        bio.default = ColumnDefault::Custom { expression: "'n/a'".into() };
        let mut email = col("email", text);
        email.length = Some(120);
        email.unique = true;
        email.unique_name = Some("people_email_key".into());
        let mut t = spec(table("people"), vec![pk("id", int), name, bio, email]);
        t.primary_key_name = Some("people_pkey".into());
        live(t)
    }

    #[test]
    fn nothing_changed_means_nothing_to_run() {
        for (engine, types) in [
            (Engine::Postgres, ("integer", "varchar")),
            (Engine::Mysql, ("int", "varchar")),
            (Engine::Mssql, ("int", "varchar")),
            (Engine::Sqlite, ("INTEGER", "TEXT")),
            (Engine::Duckdb, ("INTEGER", "VARCHAR")),
        ] {
            assert!(alter(engine, &people(types), |_| {}).is_empty(), "{engine:?}");
        }
        // Reordering the form's rows is not a change either.
        assert!(alter(Engine::Postgres, &people(("integer", "varchar")), |d| d.columns.reverse()).is_empty());
    }

    #[test]
    fn postgres_edit_orders_statements_so_dependencies_hold() {
        let original = people(("integer", "varchar"));
        let statements = alter(Engine::Postgres, &original, |d| {
            d.table.name = "persons".into();
            d.columns.retain(|c| c.name != "bio");
            d.columns[1].name = "full_name".into();
            d.columns[1].length = Some(200);
            d.columns[1].nullable = true;
            d.columns[2].unique = false;
            let mut age = col("age", "integer");
            age.nullable = false;
            age.default = ColumnDefault::Custom { expression: "0".into() };
            age.unique = true;
            d.columns.push(age);
        });
        assert_eq!(
            statements,
            vec![
                "ALTER TABLE \"people\" DROP CONSTRAINT IF EXISTS \"people_email_key\"",
                "DROP INDEX IF EXISTS \"people_email_key\"",
                "ALTER TABLE \"people\" DROP COLUMN \"bio\"",
                "ALTER TABLE \"people\" RENAME COLUMN \"name\" TO \"full_name\"",
                "ALTER TABLE \"people\" ALTER COLUMN \"full_name\" TYPE varchar(200) USING \"full_name\"::varchar(200)",
                "ALTER TABLE \"people\" ALTER COLUMN \"full_name\" DROP NOT NULL",
                "ALTER TABLE \"people\" ADD COLUMN \"age\" integer NOT NULL DEFAULT 0 UNIQUE",
                "ALTER TABLE \"people\" RENAME TO \"persons\"",
            ]
        );
    }

    #[test]
    fn postgres_default_and_type_changes_do_not_trip_over_each_other() {
        let original = people(("integer", "varchar"));
        // bio has a default; changing its type drops, converts, then restores it.
        let retyped = alter(Engine::Postgres, &original, |d| {
            d.columns[2].data_type = "text".into();
            d.columns[2].length = None;
        });
        assert_eq!(
            retyped,
            vec![
                "ALTER TABLE \"people\" ALTER COLUMN \"bio\" DROP DEFAULT",
                "ALTER TABLE \"people\" ALTER COLUMN \"bio\" TYPE text USING \"bio\"::text",
                "ALTER TABLE \"people\" ALTER COLUMN \"bio\" SET DEFAULT 'n/a'",
            ]
        );
        let cleared = alter(Engine::Postgres, &original, |d| d.columns[2].default = ColumnDefault::None);
        assert_eq!(cleared, vec!["ALTER TABLE \"people\" ALTER COLUMN \"bio\" DROP DEFAULT"]);
        let set = alter(Engine::Postgres, &original, |d| d.columns[1].default = ColumnDefault::Empty);
        assert_eq!(set, vec!["ALTER TABLE \"people\" ALTER COLUMN \"name\" SET DEFAULT ''"]);
        // DEFAULT NULL on a nullable column is the same as no default.
        let mut with_null = original.clone();
        with_null.columns[2].default = ColumnDefault::Null;
        assert!(alter(Engine::Postgres, &with_null, |d| d.columns[2].default = ColumnDefault::None).is_empty());
    }

    #[test]
    fn postgres_primary_key_and_foreign_key_changes() {
        let mut original = people(("integer", "varchar"));
        let mut owner = fk(&["id"], "owners", &["id"]);
        owner.name = Some("people_owner_fkey".into());
        owner.on_delete = FkAction::Cascade;
        original.foreign_keys = vec![owner];
        let original = live(original);
        let statements = alter(Engine::Postgres, &original, |d| {
            d.columns[0].primary_key = false;
            d.columns[1].primary_key = true;
            d.foreign_keys[0].on_delete = FkAction::SetNull;
            let mut audit = fk(&["email"], "audit", &["email"]);
            audit.on_update = FkAction::Cascade;
            d.foreign_keys.push(audit);
        });
        assert_eq!(
            statements,
            vec![
                "ALTER TABLE \"people\" DROP CONSTRAINT \"people_owner_fkey\"",
                "ALTER TABLE \"people\" DROP CONSTRAINT \"people_pkey\"",
                "ALTER TABLE \"people\" ADD CONSTRAINT \"people_pkey\" PRIMARY KEY (\"name\")",
                "ALTER TABLE \"people\" ADD CONSTRAINT \"people_owner_fkey\" FOREIGN KEY (\"id\") REFERENCES \"owners\" (\"id\") ON DELETE SET NULL",
                "ALTER TABLE \"people\" ADD CONSTRAINT \"people_email_fkey\" FOREIGN KEY (\"email\") REFERENCES \"audit\" (\"email\") ON UPDATE CASCADE",
            ]
        );
        // An untouched FK is left alone even when a column it uses was renamed.
        let renamed_fk = alter(Engine::Postgres, &original, |d| {
            d.columns[0].name = "person_id".into();
            d.foreign_keys[0].columns = vec!["person_id".into()];
        });
        assert_eq!(renamed_fk, vec!["ALTER TABLE \"people\" RENAME COLUMN \"id\" TO \"person_id\""]);
    }

    #[test]
    fn mysql_modifies_whole_columns_and_drops_indexes_by_name() {
        let original = people(("int", "varchar"));
        let statements = alter(Engine::Mariadb, &original, |d| {
            d.table.name = "persons".into();
            d.columns[1].length = Some(255);
            d.columns[2].default = ColumnDefault::Empty;
            d.columns[3].unique = false;
            d.columns.push({
                let mut c = col("age", "tinyint");
                c.unsigned = true;
                c
            });
        });
        assert_eq!(
            statements,
            vec![
                "ALTER TABLE `people` DROP INDEX `people_email_key`",
                "ALTER TABLE `people` MODIFY COLUMN `name` varchar(255) NOT NULL",
                "ALTER TABLE `people` ALTER COLUMN `bio` SET DEFAULT ''",
                "ALTER TABLE `people` ADD COLUMN `age` tinyint UNSIGNED NULL",
                "RENAME TABLE `people` TO `persons`",
            ]
        );
        let mut with_fk = original.clone();
        with_fk.foreign_keys = vec![{
            let mut f = fk(&["id"], "owners", &["id"]);
            f.name = Some("fk_owner".into());
            f.original_name = Some("fk_owner".into());
            f
        }];
        let dropped = alter(Engine::Mysql, &with_fk, |d| d.foreign_keys.clear());
        assert_eq!(dropped, vec!["ALTER TABLE `people` DROP FOREIGN KEY `fk_owner`"]);
        let pk_swap = alter(Engine::Mysql, &original, |d| {
            d.columns[0].primary_key = false;
            d.columns[3].primary_key = true;
        });
        assert!(pk_swap.contains(&"ALTER TABLE `people` DROP PRIMARY KEY".to_string()));
        assert!(pk_swap.contains(&"ALTER TABLE `people` ADD PRIMARY KEY (`email`)".to_string()));
    }

    #[test]
    fn sql_server_resolves_default_constraints_and_renames_with_sp_rename() {
        let original = people(("int", "varchar"));
        let statements = alter(Engine::Mssql, &original, |d| {
            d.table.name = "persons".into();
            d.columns.retain(|c| c.name != "bio");
            d.columns[1].name = "full_name".into();
            d.columns[1].length = Some(255);
            // `bio` is gone, so `email` is now the third column.
            d.columns[2].default = ColumnDefault::Now;
        });
        assert_eq!(statements.len(), 6, "{statements:#?}");
        assert!(statements[0].starts_with("DECLARE @df1 sysname = (SELECT dc.name FROM sys.default_constraints dc"), "{}", statements[0]);
        assert!(!statements[0].contains(';'), "one statement, no inner semicolons");
        assert_eq!(statements[1], "ALTER TABLE [people] DROP COLUMN [bio]");
        assert_eq!(statements[2], "EXEC sp_rename N'[people].[name]', N'full_name', N'COLUMN'");
        assert_eq!(statements[3], "ALTER TABLE [people] ALTER COLUMN [full_name] varchar(255) NOT NULL");
        assert_eq!(statements[4], "ALTER TABLE [people] ADD CONSTRAINT [DF_persons_email] DEFAULT GETDATE() FOR [email]");
        assert_eq!(statements[5], "EXEC sp_rename N'[people]', N'persons'");
        // Two snippets in one script must not redeclare a variable.
        let mut original = original;
        original.columns[3].default = ColumnDefault::Custom { expression: "'a'".into() };
        let both = alter(Engine::Mssql, &original, |d| {
            d.columns.retain(|c| c.name != "bio");
            d.columns[2].default = ColumnDefault::Now;
            d.columns[2].data_type = "nvarchar".into();
        });
        let declared: Vec<&String> = both.iter().filter(|s| s.starts_with("DECLARE")).collect();
        assert!(declared.len() >= 2 && declared[0] != declared[1]);
        assert!(alter(Engine::Mssql, &original, |d| d.columns[2].auto_increment = true).iter().any(|s| s.starts_with("ERR")));
    }

    #[test]
    fn sqlite_keeps_simple_edits_in_place() {
        let original = people(("INTEGER", "TEXT"));
        let statements = alter(Engine::Sqlite, &original, |d| {
            d.table.name = "persons".into();
            d.columns.retain(|c| c.name != "bio");
            d.columns[1].name = "full_name".into();
            let mut nick = col("nick", "TEXT");
            nick.default = ColumnDefault::Custom { expression: "'x'".into() };
            d.columns.push(nick);
        });
        assert_eq!(
            statements,
            vec![
                "ALTER TABLE \"people\" DROP COLUMN \"bio\"",
                "ALTER TABLE \"people\" RENAME COLUMN \"name\" TO \"full_name\"",
                "ALTER TABLE \"people\" ADD COLUMN \"nick\" TEXT DEFAULT 'x'",
                "ALTER TABLE \"people\" RENAME TO \"persons\"",
            ]
        );
    }

    #[test]
    fn sqlite_rebuilds_for_everything_it_cannot_alter() {
        let mut original = people(("INTEGER", "TEXT"));
        original.indexes = vec![IndexSpec { name: "people_name_idx".into(), columns: vec!["name".into()], unique: false }];
        let statements = alter(Engine::Sqlite, &original, |d| {
            d.columns[1].nullable = true;
            d.columns[1].name = "full_name".into();
            d.columns[1].data_type = "VARCHAR".into();
            d.indexes[0].columns = vec!["full_name".into()];
        });
        assert_eq!(statements.first().map(String::as_str), Some("PRAGMA foreign_keys = OFF"));
        assert_eq!(statements.last().map(String::as_str), Some("PRAGMA foreign_keys = ON"));
        assert_eq!(statements[1], "DROP TABLE IF EXISTS \"people__rebuild\"");
        assert!(statements[2].starts_with("CREATE TABLE \"people__rebuild\" (\n  \"id\" INTEGER NOT NULL,\n  \"full_name\" VARCHAR(80),"), "{}", statements[2]);
        assert_eq!(statements[3], "INSERT INTO \"people__rebuild\" (\"id\", \"full_name\", \"bio\", \"email\") SELECT \"id\", \"name\", \"bio\", \"email\" FROM \"people\"");
        assert_eq!(statements[4], "DROP TABLE \"people\"");
        assert_eq!(statements[5], "ALTER TABLE \"people__rebuild\" RENAME TO \"people\"");
        assert_eq!(statements[6], "CREATE INDEX \"people_name_idx\" ON \"people\" (\"full_name\")");
        // A table rename comes first, as its own ALTER, and the copy takes the new name.
        let both = alter(Engine::Sqlite, &original, |d| {
            d.table.name = "persons".into();
            d.columns[0].primary_key = false;
        });
        assert_eq!(both[1], "ALTER TABLE \"people\" RENAME TO \"persons\"");
        assert!(both[3].starts_with("CREATE TABLE \"persons__rebuild\""));
        assert!(both.contains(&"ALTER TABLE \"persons__rebuild\" RENAME TO \"persons\"".to_string()));
        // A NOT NULL column that gains a default must not abort on existing NULLs.
        let filled = alter(Engine::Sqlite, &original, |d| {
            d.columns[2].nullable = false;
            d.columns[2].default = ColumnDefault::Custom { expression: "'n/a'".into() };
        });
        assert!(filled[3].contains("COALESCE(\"bio\", 'n/a')"), "{}", filled[3]);
        // D1 cannot turn foreign keys off; it defers them.
        let d1 = alter(Engine::CloudflareD1, &original, |d| d.columns[1].nullable = true);
        assert_eq!(d1.first().map(String::as_str), Some("PRAGMA defer_foreign_keys = ON"));
        assert_ne!(d1.last().map(String::as_str), Some("PRAGMA foreign_keys = ON"));
    }

    #[test]
    fn engines_that_cannot_alter_say_so_by_name() {
        let duck = people(("INTEGER", "VARCHAR"));
        let refused = alter(Engine::Duckdb, &duck, |d| d.columns[3].unique = false);
        assert!(refused[0].starts_with("ERR DuckDB cannot add or drop a UNIQUE constraint"), "{}", refused[0]);
        let fk_refused = alter(Engine::Duckdb, &duck, |d| d.foreign_keys.push(fk(&["id"], "o", &["id"])));
        assert!(fk_refused[0].starts_with("ERR DuckDB cannot add or drop a foreign key"), "{}", fk_refused[0]);
        let ok = alter(Engine::Duckdb, &duck, |d| {
            d.columns[1].data_type = "TEXT".into();
            d.columns[1].length = None;
            d.columns[1].nullable = true;
            d.columns[2].default = ColumnDefault::None;
            let mut c = col("age", "INTEGER");
            c.nullable = false;
            d.columns.push(c);
        });
        assert_eq!(
            ok,
            vec![
                "ALTER TABLE \"people\" ALTER COLUMN \"name\" SET DATA TYPE TEXT",
                "ALTER TABLE \"people\" ALTER COLUMN \"name\" DROP NOT NULL",
                "ALTER TABLE \"people\" ALTER COLUMN \"bio\" DROP DEFAULT",
                "ALTER TABLE \"people\" ADD COLUMN \"age\" INTEGER",
                "ALTER TABLE \"people\" ALTER COLUMN \"age\" SET NOT NULL",
            ]
        );
        let cass = live(spec(table("t"), vec![pk("id", "uuid"), col("a", "text")]));
        assert!(alter(Engine::Cassandra, &cass, |d| d.columns[1].data_type = "int".into())[0].starts_with("ERR Cassandra cannot change a column"));
        assert_eq!(alter(Engine::Cassandra, &cass, |d| d.columns.push(col("b", "int"))), vec!["ALTER TABLE \"t\" ADD \"b\" int"]);
        assert_eq!(alter(Engine::Cassandra, &cass, |d| d.columns.retain(|c| c.name != "a")), vec!["ALTER TABLE \"t\" DROP \"a\""]);
        assert!(alter(Engine::Cassandra, &cass, |d| d.table.name = "u".into())[0].starts_with("ERR Cassandra cannot rename a table"));
    }

    #[test]
    fn clickhouse_modifies_columns_and_protects_the_sorting_key() {
        let mut ch = spec(table("events"), vec![pk("id", "UInt64"), col("name", "String"), col("at", "DateTime")]);
        ch.engine = Some("MergeTree".into());
        ch.order_by = vec!["id".into()];
        let original = live(ch);
        let statements = alter(Engine::Clickhouse, &original, |d| {
            d.columns[1].nullable = false;
            d.columns[1].default = ColumnDefault::Custom { expression: "'x'".into() };
            d.columns[2].name = "created".into();
            d.table.name = "events2".into();
        });
        assert_eq!(
            statements,
            vec![
                "ALTER TABLE \"events\" RENAME COLUMN \"at\" TO \"created\"",
                "ALTER TABLE \"events\" MODIFY COLUMN \"name\" String DEFAULT 'x'",
                "RENAME TABLE \"events\" TO \"events2\"",
            ]
        );
        assert!(alter(Engine::Clickhouse, &original, |d| d.order_by = vec!["at".into()])[0].starts_with("ERR ClickHouse cannot change a table's ORDER BY"));
        assert!(alter(Engine::Clickhouse, &original, |d| d.engine = Some("Memory".into()))[0].starts_with("ERR ClickHouse cannot change a table's ENGINE"));
        assert!(alter(Engine::Clickhouse, &original, |d| d.columns[1].unique = true)[0].starts_with("ERR"));
    }

    #[test]
    fn oracle_modifies_only_what_changed() {
        let original = live(spec(table("t"), vec![pk("id", "NUMBER"), {
            let mut c = col("name", "VARCHAR2");
            c.length = Some(40);
            c
        }]));
        let statements = alter(Engine::Oracle, &original, |d| {
            d.columns[1].length = Some(80);
            d.columns[1].nullable = false;
            d.columns[1].default = ColumnDefault::Empty;
        });
        assert_eq!(statements, vec!["ALTER TABLE \"t\" MODIFY (\"name\" VARCHAR2(80) DEFAULT '' NOT NULL)"]);
        let unique = alter(Engine::Oracle, &original, |d| d.columns[1].unique = true);
        assert_eq!(unique, vec!["ALTER TABLE \"t\" ADD CONSTRAINT \"uq_t_name\" UNIQUE (\"name\")"]);
        assert_eq!(alter(Engine::Oracle, &original, |d| d.columns.push(col("x", "DATE"))), vec!["ALTER TABLE \"t\" ADD (\"x\" DATE)"]);
    }

    #[test]
    fn mongo_only_renames_and_stale_columns_are_caught() {
        let coll = spec(TableRef { schema: Some("shop".into()), name: "orders".into() }, Vec::new());
        assert!(alter(Engine::Mongodb, &coll, |_| {}).is_empty());
        assert_eq!(
            alter(Engine::Mongodb, &coll, |d| d.table.name = "orders2".into()),
            vec![r#"{"renameCollection":"shop.orders","to":"shop.orders2","$db":"admin"}"#.to_string()]
        );
        let original = people(("integer", "varchar"));
        let stale = alter(Engine::Postgres, &original, |d| d.columns[1].original_name = Some("ghost".into()));
        assert!(stale[0].starts_with("ERR Column \"ghost\" is no longer in the table"), "{}", stale[0]);
        let moved = alter(Engine::Postgres, &original, |d| d.table.schema = Some("other".into()));
        assert!(moved[0].starts_with("ERR A table cannot be moved"), "{}", moved[0]);
        let dup = alter(Engine::Postgres, &original, |d| d.columns[2].original_name = Some("name".into()));
        assert!(dup[0].starts_with("ERR"), "{}", dup[0]);
    }
}
