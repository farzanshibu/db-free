// SOT: schema-diff-service, schema-compare, migration-script, alter-table-dialect, schema-snapshot

use crate::error::{AppError, AppResult};
use crate::guard::SessionCtx;
use crate::integrations::{qualified_name_for, quote_ident_for};
use crate::model::{
    ColumnChange, ColumnDiff, ColumnInfo, CompareDirection, DiffStatus, Engine, Family, ForeignKey, ForeignKeyDiff,
    SchemaDiff, TableDiff, TableKind, TableRef,
};
use std::collections::{BTreeMap, HashMap, HashSet};

// ============================================================================
// SCHEMA COMPARE
//
// WHAT:  Compares one schema on each of two sessions (the same connection or
//        two different ones) and writes the migration that turns the target
//        side into the source side.
// WHY:   "What changed between staging and production" is otherwise a manual
//        read of two catalogs; the script is where mistakes happen, so it is
//        generated in the target engine's own ALTER spelling.
// HOW:   Two phases. `snapshot` reads the catalog, every table's columns and
//        the foreign keys through the adapters (async, per side). `diff` and
//        `render_script` are pure over those snapshots, so the matching rules
//        and every dialect's statements are unit-tested without a server.
//        Nothing is executed here: the script opens in a query tab, and the
//        guard's read-only lock and destructive confirmation apply when the
//        user runs it. DROP TABLE is written commented out on purpose.
// WHERE: src-tauri/src/commands/compare.rs (caller), src-tauri/src/model/compare.rs,
//        src/features/compare/SchemaCompareTab.tsx
// ============================================================================

#[derive(Debug, Clone)]
pub struct TableSnapshot {
    pub table: TableRef,
    pub columns: Vec<ColumnInfo>,
    /// Foreign keys declared on this table.
    pub foreign_keys: Vec<ForeignKey>,
}

#[derive(Debug, Clone)]
pub struct SchemaSnapshot {
    pub engine: Engine,
    /// What a new table's `TableRef::schema` is on this side (None for engines
    /// without namespaces, e.g. SQLite's pseudo-schema "main").
    pub namespace: Option<String>,
    pub tables: Vec<TableSnapshot>,
}

// WHAT:  Everything `diff` needs from one side, read through the adapter.
// HOW:   `schema` is a name from the catalog; None takes the first schema.
//        Views are skipped: their shape follows from their query, which is
//        what would need migrating.
pub async fn snapshot(ctx: &SessionCtx, schema: Option<&str>) -> AppResult<SchemaSnapshot> {
    let engine = ctx.connection.engine;
    let caps = ctx.integration.capabilities();
    if !caps.sql || !caps.fixed_columns {
        return Err(AppError::invalid_input(format!(
            "Schema compare needs a SQL engine with fixed columns; \"{}\" is {}.",
            ctx.connection.name,
            engine.label()
        )));
    }
    let catalog = ctx.integration.catalog().await?;
    let info = match schema {
        Some(name) => catalog
            .schemas
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| AppError::not_found(format!("Schema \"{name}\" not found on \"{}\".", ctx.connection.name)))?,
        None => catalog
            .schemas
            .first()
            .ok_or_else(|| AppError::not_found(format!("\"{}\" has no schemas.", ctx.connection.name)))?,
    };
    let namespace = match info.tables.first() {
        Some(t) => t.schema.clone(),
        None if caps.namespaces => Some(info.name.clone()),
        None => None,
    };
    // A failing foreign-key probe degrades to "no keys" rather than failing the compare.
    let all_keys = ctx.integration.foreign_keys().await.unwrap_or_default();
    let mut tables = Vec::new();
    for t in info.tables.iter().filter(|t| t.kind == TableKind::Table) {
        let table = TableRef { schema: t.schema.clone(), name: t.name.clone() };
        let columns = ctx.integration.columns(&table).await?;
        let foreign_keys = all_keys
            .iter()
            .filter(|fk| fk.from_table == table.name && (fk.from_schema.is_none() || table.schema.is_none() || fk.from_schema == table.schema))
            .cloned()
            .collect();
        tables.push(TableSnapshot { table, columns, foreign_keys });
    }
    Ok(SchemaSnapshot { engine, namespace, tables })
}

// WHAT:  The full compare: both snapshots, the diff, the CREATE statements for
//        tables the target lacks, and the script.
// HOW:   A new table is created from the source's own DDL when both sides are
//        the same family and namespace (it then carries everything the adapter
//        knows: defaults, inline keys); otherwise from the target adapter's
//        `create_template`, so the statement is in the target's language and
//        lands in the target's schema.
pub async fn compare(
    left: &SessionCtx,
    right: &SessionCtx,
    left_schema: Option<&str>,
    right_schema: Option<&str>,
    direction: CompareDirection,
) -> AppResult<SchemaDiff> {
    let left_snap = snapshot(left, left_schema).await?;
    let right_snap = snapshot(right, right_schema).await?;
    let tables = diff(&left_snap, &right_snap, direction);
    let (source_ctx, target_ctx, source, target) =
        if direction.left_is_source() { (left, right, &left_snap, &right_snap) } else { (right, left, &right_snap, &left_snap) };

    let mut creates: HashMap<String, String> = HashMap::new();
    for t in tables.iter().filter(|t| t.status == DiffStatus::Added) {
        let Some(snap) = source.tables.iter().find(|s| s.table.name.to_lowercase() == t.name.to_lowercase()) else {
            continue;
        };
        let same_home = source.engine.family() == target.engine.family() && source.namespace == target.namespace;
        let from_ddl = if same_home { source_ctx.integration.ddl(&snap.table).await.unwrap_or(None) } else { None };
        let statement = match from_ddl {
            Some(ddl) => ddl,
            None => {
                let target_ref = TableRef { schema: target.namespace.clone(), name: snap.table.name.clone() };
                target_ctx
                    .integration
                    .create_template(&target_ref, &snap.columns)
                    .unwrap_or_else(|| format!("-- {} has no CREATE statement for {}; create it by hand.", target.engine.label(), snap.table.name))
            }
        };
        creates.insert(t.name.to_lowercase(), statement);
    }

    let mut notes = Vec::new();
    if source.engine.family() != target.engine.family() {
        notes.push(format!(
            "The sides are different engines ({} and {}): column types are copied verbatim and may need translating.",
            left_snap.engine.label(),
            right_snap.engine.label()
        ));
    }
    if dialect(target.engine) == Dialect::Sqlite && tables.iter().any(|t| t.status == DiffStatus::Changed) {
        notes.push("SQLite cannot alter a column's type, nullability or keys in place; those changes are written as comments describing the table rebuild.".to_string());
    }
    if tables.iter().any(|t| t.status == DiffStatus::Removed) {
        notes.push("DROP TABLE statements are commented out; uncomment the ones you mean.".to_string());
    }
    let script = render_script(&tables, source, target, &creates, direction);
    Ok(SchemaDiff { direction, left_engine: left_snap.engine, right_engine: right_snap.engine, tables, script, notes })
}

// ---- diff (pure) --------------------------------------------------------------

fn fold(name: &str) -> String {
    name.to_lowercase()
}

// WHAT:  Type spelling for comparison only: case and whitespace are not a change.
fn normalize_type(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

// WHAT:  Status of something found on one side only, read in the direction.
fn one_sided(in_left: bool, direction: CompareDirection) -> DiffStatus {
    if in_left == direction.left_is_source() {
        DiffStatus::Added
    } else {
        DiffStatus::Removed
    }
}

fn fk_signature(fk: &ForeignKey) -> String {
    let cols = |c: &[String]| c.iter().map(|x| fold(x)).collect::<Vec<_>>().join(",");
    format!("{}({})->{}({})", fold(&fk.from_table), cols(&fk.from_columns), fold(&fk.to_table), cols(&fk.to_columns))
}

// WHAT:  Matches tables and columns by name (case-insensitively, so an Oracle
//        upper-case catalog lines up with a lower-case one) and reports every
//        table, sorted by name.
pub fn diff(left: &SchemaSnapshot, right: &SchemaSnapshot, direction: CompareDirection) -> Vec<TableDiff> {
    let mut names: BTreeMap<String, (Option<&TableSnapshot>, Option<&TableSnapshot>)> = BTreeMap::new();
    for t in &left.tables {
        names.entry(fold(&t.table.name)).or_default().0 = Some(t);
    }
    for t in &right.tables {
        names.entry(fold(&t.table.name)).or_default().1 = Some(t);
    }
    names
        .into_values()
        .map(|pair| match pair {
            (Some(l), Some(r)) => table_diff(l, r, direction),
            (Some(l), None) => one_side_table(l, true, direction),
            (None, Some(r)) => one_side_table(r, false, direction),
            (None, None) => TableDiff { name: String::new(), status: DiffStatus::Identical, left: None, right: None, columns: Vec::new(), foreign_keys: Vec::new() },
        })
        .collect()
}

fn one_side_table(t: &TableSnapshot, in_left: bool, direction: CompareDirection) -> TableDiff {
    let status = one_sided(in_left, direction);
    let side = |c: &ColumnInfo| if in_left { (Some(c.clone()), None) } else { (None, Some(c.clone())) };
    TableDiff {
        name: t.table.name.clone(),
        status,
        left: if in_left { Some(t.table.clone()) } else { None },
        right: if in_left { None } else { Some(t.table.clone()) },
        columns: t
            .columns
            .iter()
            .map(|c| {
                let (left, right) = side(c);
                ColumnDiff { name: c.name.clone(), status, left, right, changes: Vec::new() }
            })
            .collect(),
        foreign_keys: t
            .foreign_keys
            .iter()
            .map(|fk| ForeignKeyDiff {
                status,
                left: if in_left { Some(fk.clone()) } else { None },
                right: if in_left { None } else { Some(fk.clone()) },
            })
            .collect(),
    }
}

fn table_diff(l: &TableSnapshot, r: &TableSnapshot, direction: CompareDirection) -> TableDiff {
    let mut columns = Vec::new();
    let mut ordered: Vec<&ColumnInfo> = l.columns.iter().collect();
    ordered.sort_by_key(|c| c.ordinal);
    for lc in ordered {
        match r.columns.iter().find(|rc| fold(&rc.name) == fold(&lc.name)) {
            Some(rc) => {
                let mut changes = Vec::new();
                if normalize_type(&lc.data_type) != normalize_type(&rc.data_type) {
                    changes.push(ColumnChange::Type);
                }
                if lc.nullable != rc.nullable {
                    changes.push(ColumnChange::Nullable);
                }
                if lc.primary_key != rc.primary_key {
                    changes.push(ColumnChange::PrimaryKey);
                }
                let status = if changes.is_empty() { DiffStatus::Identical } else { DiffStatus::Changed };
                columns.push(ColumnDiff { name: lc.name.clone(), status, left: Some(lc.clone()), right: Some(rc.clone()), changes });
            }
            None => columns.push(ColumnDiff { name: lc.name.clone(), status: one_sided(true, direction), left: Some(lc.clone()), right: None, changes: Vec::new() }),
        }
    }
    let mut right_only: Vec<&ColumnInfo> = r.columns.iter().filter(|rc| !l.columns.iter().any(|lc| fold(&lc.name) == fold(&rc.name))).collect();
    right_only.sort_by_key(|c| c.ordinal);
    for rc in right_only {
        columns.push(ColumnDiff { name: rc.name.clone(), status: one_sided(false, direction), left: None, right: Some(rc.clone()), changes: Vec::new() });
    }

    let right_sigs: HashSet<String> = r.foreign_keys.iter().map(fk_signature).collect();
    let left_sigs: HashSet<String> = l.foreign_keys.iter().map(fk_signature).collect();
    let mut foreign_keys: Vec<ForeignKeyDiff> = l
        .foreign_keys
        .iter()
        .filter(|fk| !right_sigs.contains(&fk_signature(fk)))
        .map(|fk| ForeignKeyDiff { status: one_sided(true, direction), left: Some(fk.clone()), right: None })
        .collect();
    foreign_keys.extend(
        r.foreign_keys
            .iter()
            .filter(|fk| !left_sigs.contains(&fk_signature(fk)))
            .map(|fk| ForeignKeyDiff { status: one_sided(false, direction), left: None, right: Some(fk.clone()) }),
    );

    let changed = columns.iter().any(|c| c.status != DiffStatus::Identical) || !foreign_keys.is_empty();
    TableDiff {
        name: l.table.name.clone(),
        status: if changed { DiffStatus::Changed } else { DiffStatus::Identical },
        left: Some(l.table.clone()),
        right: Some(r.table.clone()),
        columns,
        foreign_keys,
    }
}

// ---- script (pure) -----------------------------------------------------------

// WHAT:  The ALTER spelling a target engine takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Postgres,
    Mysql,
    Mssql,
    Sqlite,
    Oracle,
    /// ANSI-style ALTER (DuckDB, Snowflake, ClickHouse-via-ANSI…): Postgres spelling.
    Generic,
}

fn dialect(engine: Engine) -> Dialect {
    match engine.family() {
        Family::Postgres => Dialect::Postgres,
        Family::Mysql => Dialect::Mysql,
        Family::Mssql => Dialect::Mssql,
        Family::Oracle => Dialect::Oracle,
        Family::Sqlite | Family::Libsql | Family::ValTown | Family::CloudflareD1 => Dialect::Sqlite,
        _ => Dialect::Generic,
    }
}

struct Writer<'a> {
    engine: Engine,
    dialect: Dialect,
    namespace: Option<&'a str>,
    out: Vec<String>,
}

impl Writer<'_> {
    fn ident(&self, raw: &str) -> String {
        quote_ident_for(self.engine, raw)
    }

    fn table(&self, name: &str) -> String {
        qualified_name_for(self.engine, &TableRef { schema: self.namespace.map(str::to_string), name: name.to_string() })
    }

    fn line(&mut self, text: String) {
        self.out.push(text);
    }

    fn columns(&self, names: &[String]) -> String {
        names.iter().map(|n| self.ident(n)).collect::<Vec<_>>().join(", ")
    }

    fn add_column(&mut self, table: &str, col: &ColumnInfo) {
        let t = self.table(table);
        let c = self.ident(&col.name);
        let not_null = if col.nullable { "" } else { " NOT NULL" };
        if !col.nullable {
            self.line(format!("-- {}.{} is NOT NULL: adding it fails on a table with rows unless you add a DEFAULT.", table, col.name));
        }
        let stmt = match self.dialect {
            Dialect::Mssql => format!("ALTER TABLE {t} ADD {c} {}{};", col.data_type, if col.nullable { " NULL" } else { " NOT NULL" }),
            Dialect::Oracle => format!("ALTER TABLE {t} ADD ({c} {}{not_null});", col.data_type),
            Dialect::Postgres | Dialect::Mysql | Dialect::Sqlite | Dialect::Generic => {
                format!("ALTER TABLE {t} ADD COLUMN {c} {}{not_null};", col.data_type)
            }
        };
        self.line(stmt);
    }

    fn drop_column(&mut self, table: &str, name: &str) {
        let stmt = format!("ALTER TABLE {} DROP COLUMN {};", self.table(table), self.ident(name));
        self.line(stmt);
    }

    // WHAT:  Type and/or nullability change of one column to the source's.
    fn alter_column(&mut self, table: &str, source: &ColumnInfo, target: &ColumnInfo, changes: &[ColumnChange]) {
        let type_changed = changes.contains(&ColumnChange::Type);
        let null_changed = changes.contains(&ColumnChange::Nullable);
        if !type_changed && !null_changed {
            return;
        }
        let t = self.table(table);
        let c = self.ident(&source.name);
        let ty = &source.data_type;
        let null_word = if source.nullable { "NULL" } else { "NOT NULL" };
        match self.dialect {
            Dialect::Postgres | Dialect::Generic => {
                if type_changed {
                    self.line(format!("ALTER TABLE {t} ALTER COLUMN {c} TYPE {ty};"));
                }
                if null_changed {
                    let verb = if source.nullable { "DROP NOT NULL" } else { "SET NOT NULL" };
                    self.line(format!("ALTER TABLE {t} ALTER COLUMN {c} {verb};"));
                }
            }
            // MODIFY restates the whole column, so type and nullability go together.
            Dialect::Mysql => self.line(format!("ALTER TABLE {t} MODIFY COLUMN {c} {ty} {null_word};")),
            Dialect::Mssql => self.line(format!("ALTER TABLE {t} ALTER COLUMN {c} {ty} {null_word};")),
            Dialect::Oracle => {
                let body = match (type_changed, null_changed) {
                    (true, true) => format!("{c} {ty} {null_word}"),
                    (true, false) => format!("{c} {ty}"),
                    _ => format!("{c} {null_word}"),
                };
                self.line(format!("ALTER TABLE {t} MODIFY ({body});"));
            }
            Dialect::Sqlite => self.line(format!(
                "-- SQLite cannot alter {table}.{}: {} {} -> {} {}. Rebuild the table (create the new shape, copy rows, drop, rename).",
                source.name,
                target.data_type,
                if target.nullable { "NULL" } else { "NOT NULL" },
                ty,
                null_word
            )),
        }
    }

    fn primary_key(&mut self, table: &str, source: &[String], target: &[String]) {
        let t = self.table(table);
        let list = |cols: &[String]| if cols.is_empty() { "none".to_string() } else { cols.join(", ") };
        self.line(format!("-- Primary key differs on {table}: target ({}) -> source ({}).", list(target), list(source)));
        match self.dialect {
            Dialect::Sqlite => self.line("-- SQLite cannot change a primary key in place; rebuild the table.".to_string()),
            Dialect::Mysql => {
                let stmt = match (target.is_empty(), source.is_empty()) {
                    (true, false) => format!("ALTER TABLE {t} ADD PRIMARY KEY ({});", self.columns(source)),
                    (false, true) => format!("ALTER TABLE {t} DROP PRIMARY KEY;"),
                    (false, false) => format!("ALTER TABLE {t} DROP PRIMARY KEY, ADD PRIMARY KEY ({});", self.columns(source)),
                    (true, true) => return,
                };
                self.line(stmt);
            }
            Dialect::Postgres | Dialect::Mssql | Dialect::Oracle | Dialect::Generic => {
                if !target.is_empty() {
                    // The constraint's name is engine-generated and not in the catalog read.
                    self.line(format!("-- ALTER TABLE {t} DROP CONSTRAINT <primary key constraint name>;"));
                }
                if !source.is_empty() {
                    let stmt = format!("ALTER TABLE {t} ADD PRIMARY KEY ({});", self.columns(source));
                    self.line(stmt);
                }
            }
        }
    }

    fn add_foreign_key(&mut self, fk: &ForeignKey) {
        let t = self.table(&fk.from_table);
        let refs = self.table(&fk.to_table);
        let body = format!("FOREIGN KEY ({}) REFERENCES {refs} ({})", self.columns(&fk.from_columns), self.columns(&fk.to_columns));
        let stmt = match self.dialect {
            Dialect::Sqlite => format!("-- SQLite cannot add a foreign key to {} in place; rebuild it with: {body}", fk.from_table),
            _ => format!("ALTER TABLE {t} ADD CONSTRAINT {} {body};", self.ident(&fk.name)),
        };
        self.line(stmt);
    }

    fn drop_foreign_key(&mut self, fk: &ForeignKey) {
        let t = self.table(&fk.from_table);
        let name = self.ident(&fk.name);
        let stmt = match self.dialect {
            Dialect::Sqlite => format!("-- SQLite cannot drop foreign key {} in place; rebuild {} without it.", fk.name, fk.from_table),
            Dialect::Mysql => format!("ALTER TABLE {t} DROP FOREIGN KEY {name};"),
            _ => format!("ALTER TABLE {t} DROP CONSTRAINT {name};"),
        };
        self.line(stmt);
    }
}

fn source_side<'a, T>(left: &'a Option<T>, right: &'a Option<T>, direction: CompareDirection) -> Option<&'a T> {
    if direction.left_is_source() { left.as_ref() } else { right.as_ref() }
}

fn target_side<'a, T>(left: &'a Option<T>, right: &'a Option<T>, direction: CompareDirection) -> Option<&'a T> {
    if direction.left_is_source() { right.as_ref() } else { left.as_ref() }
}

// WHAT:  Orders new tables so a referenced table is created before the table
//        that references it (inline REFERENCES in copied DDL need that).
fn creation_order<'a>(added: &[&'a TableDiff], source: &SchemaSnapshot) -> Vec<&'a TableDiff> {
    let names: HashSet<String> = added.iter().map(|t| fold(&t.name)).collect();
    let deps = |t: &TableDiff| -> Vec<String> {
        source
            .tables
            .iter()
            .find(|s| fold(&s.table.name) == fold(&t.name))
            .map(|s| s.foreign_keys.iter().map(|fk| fold(&fk.to_table)).filter(|d| names.contains(d) && *d != fold(&t.name)).collect())
            .unwrap_or_default()
    };
    let mut done: HashSet<String> = HashSet::new();
    let mut out: Vec<&TableDiff> = Vec::new();
    let mut pending: Vec<&TableDiff> = added.to_vec();
    while !pending.is_empty() {
        let (ready, rest): (Vec<&TableDiff>, Vec<&TableDiff>) = pending.into_iter().partition(|t| deps(t).iter().all(|d| done.contains(d)));
        if ready.is_empty() {
            // A reference cycle: emit the rest as they are; the ALTER-added keys still apply.
            out.extend(rest);
            break;
        }
        for t in &ready {
            done.insert(fold(&t.name));
        }
        out.extend(ready);
        pending = rest;
    }
    out
}

fn terminated(statement: &str) -> String {
    let trimmed = statement.trim_end();
    if trimmed.ends_with(';') || trimmed.starts_with("--") {
        trimmed.to_string()
    } else {
        format!("{trimmed};")
    }
}

// WHAT:  The migration script, in dependency-safe order:
//          1. drop foreign keys the source no longer has (before columns go)
//          2. create new tables (referenced ones first)
//          3. per changed table: add, alter, primary key, drop columns
//          4. add foreign keys the target lacks
//          5. removed tables, as commented DROP TABLE
// HOW:   `creates` maps a lower-cased table name to its CREATE statement. A
//        new table's keys are added by ALTER only when that statement did not
//        already declare them inline.
pub fn render_script(
    tables: &[TableDiff],
    source: &SchemaSnapshot,
    target: &SchemaSnapshot,
    creates: &HashMap<String, String>,
    direction: CompareDirection,
) -> String {
    let mut w = Writer { engine: target.engine, dialect: dialect(target.engine), namespace: target.namespace.as_deref(), out: Vec::new() };
    w.line(format!("-- Migration generated by DB Free schema compare ({} target).", target.engine.label()));
    w.line("-- Review every statement before running it.".to_string());
    let before = w.out.len();

    let changed: Vec<&TableDiff> = tables.iter().filter(|t| t.status == DiffStatus::Changed).collect();
    let removed_keys: Vec<&ForeignKey> = changed
        .iter()
        .flat_map(|t| t.foreign_keys.iter())
        .filter(|fk| fk.status == DiffStatus::Removed)
        .filter_map(|fk| target_side(&fk.left, &fk.right, direction))
        .collect();
    if !removed_keys.is_empty() {
        w.line(String::new());
        w.line("-- Foreign keys the source does not have".to_string());
        for fk in removed_keys {
            w.drop_foreign_key(fk);
        }
    }

    let added: Vec<&TableDiff> = tables.iter().filter(|t| t.status == DiffStatus::Added).collect();
    let mut keys_to_add: Vec<ForeignKey> = Vec::new();
    if !added.is_empty() {
        w.line(String::new());
        w.line("-- New tables".to_string());
        for t in creation_order(&added, source) {
            let statement = creates.get(&fold(&t.name)).cloned().unwrap_or_else(|| format!("-- No CREATE statement for {}.", t.name));
            let inline_keys = statement.to_uppercase().contains("REFERENCES");
            w.line(terminated(&statement));
            if !inline_keys {
                keys_to_add.extend(t.foreign_keys.iter().filter_map(|fk| source_side(&fk.left, &fk.right, direction)).cloned());
            }
        }
    }

    for t in &changed {
        let Some(target_ref) = target_side(&t.left, &t.right, direction) else { continue };
        let name = target_ref.name.clone();
        let columns_changed = t.columns.iter().any(|c| c.status != DiffStatus::Identical);
        if !columns_changed {
            continue;
        }
        w.line(String::new());
        w.line(format!("-- {name}"));
        for c in t.columns.iter().filter(|c| c.status == DiffStatus::Added) {
            if let Some(col) = source_side(&c.left, &c.right, direction) {
                w.add_column(&name, col);
            }
        }
        for c in t.columns.iter().filter(|c| c.status == DiffStatus::Changed) {
            if let (Some(s), Some(tg)) = (source_side(&c.left, &c.right, direction), target_side(&c.left, &c.right, direction)) {
                w.alter_column(&name, s, tg, &c.changes);
            }
        }
        let source_pk: Vec<String> =
            t.columns.iter().filter_map(|c| source_side(&c.left, &c.right, direction)).filter(|c| c.primary_key).map(|c| c.name.clone()).collect();
        let target_pk: Vec<String> =
            t.columns.iter().filter_map(|c| target_side(&c.left, &c.right, direction)).filter(|c| c.primary_key).map(|c| c.name.clone()).collect();
        let folded = |cols: &[String]| cols.iter().map(|c| fold(c)).collect::<Vec<_>>();
        if folded(&source_pk) != folded(&target_pk) {
            w.primary_key(&name, &source_pk, &target_pk);
        }
        for c in t.columns.iter().filter(|c| c.status == DiffStatus::Removed) {
            w.drop_column(&name, &c.name);
        }
    }

    keys_to_add.extend(
        changed
            .iter()
            .flat_map(|t| t.foreign_keys.iter())
            .filter(|fk| fk.status == DiffStatus::Added)
            .filter_map(|fk| source_side(&fk.left, &fk.right, direction))
            .cloned(),
    );
    if !keys_to_add.is_empty() {
        w.line(String::new());
        w.line("-- Foreign keys the target does not have".to_string());
        for fk in &keys_to_add {
            w.add_foreign_key(fk);
        }
    }

    let removed: Vec<&TableDiff> = tables.iter().filter(|t| t.status == DiffStatus::Removed).collect();
    if !removed.is_empty() {
        w.line(String::new());
        w.line("-- Tables only in the target. Commented out: dropping a table deletes its rows.".to_string());
        for t in removed {
            let stmt = format!("-- DROP TABLE {};", w.table(&t.name));
            w.line(stmt);
        }
    }

    if w.out.len() == before {
        w.line(String::new());
        w.line("-- The schemas match: nothing to migrate.".to_string());
    }
    let mut script = w.out.join("\n");
    script.push('\n');
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str, nullable: bool, pk: bool, ordinal: u32) -> ColumnInfo {
        ColumnInfo { name: name.into(), data_type: ty.into(), nullable, primary_key: pk, ordinal }
    }

    fn table(schema: Option<&str>, name: &str, columns: Vec<ColumnInfo>, fks: Vec<ForeignKey>) -> TableSnapshot {
        TableSnapshot { table: TableRef { schema: schema.map(str::to_string), name: name.into() }, columns, foreign_keys: fks }
    }

    fn fk(name: &str, from: &str, cols: &[&str], to: &str, to_cols: &[&str]) -> ForeignKey {
        ForeignKey {
            name: name.into(),
            from_schema: None,
            from_table: from.into(),
            from_columns: cols.iter().map(|c| c.to_string()).collect(),
            to_schema: None,
            to_table: to.into(),
            to_columns: to_cols.iter().map(|c| c.to_string()).collect(),
        }
    }

    fn snap(engine: Engine, ns: Option<&str>, tables: Vec<TableSnapshot>) -> SchemaSnapshot {
        SchemaSnapshot { engine, namespace: ns.map(str::to_string), tables }
    }

    // left = the desired shape; right = what the target has today.
    fn pair(engine: Engine) -> (SchemaSnapshot, SchemaSnapshot) {
        let ns = if engine.family() == Family::Sqlite { None } else { Some("app") };
        let left = snap(
            engine,
            ns,
            vec![
                table(ns, "users", vec![col("id", "integer", false, true, 1), col("email", "varchar(320)", false, false, 2), col("nick", "text", true, false, 3)], vec![]),
                table(ns, "orders", vec![col("id", "integer", false, true, 1), col("user_id", "integer", false, false, 2)], vec![fk("orders_user_fk", "orders", &["user_id"], "users", &["id"])]),
                table(ns, "same", vec![col("id", "integer", false, true, 1)], vec![]),
            ],
        );
        let right = snap(
            engine,
            ns,
            vec![
                table(ns, "USERS", vec![col("id", "integer", false, true, 1), col("email", "varchar(100)", true, false, 2), col("legacy", "text", true, false, 3)], vec![]),
                table(ns, "same", vec![col("id", "INTEGER", false, true, 1)], vec![]),
                table(ns, "old_audit", vec![col("id", "integer", false, true, 1)], vec![]),
            ],
        );
        (left, right)
    }

    fn find<'a>(tables: &'a [TableDiff], name: &str) -> &'a TableDiff {
        tables.iter().find(|t| fold(&t.name) == name).unwrap_or_else(|| panic!("{name} missing"))
    }

    #[test]
    fn classifies_tables_and_columns() {
        let (l, r) = pair(Engine::Postgres);
        let tables = diff(&l, &r, CompareDirection::LeftToRight);
        assert_eq!(tables.len(), 4);
        assert_eq!(find(&tables, "orders").status, DiffStatus::Added);
        assert_eq!(find(&tables, "old_audit").status, DiffStatus::Removed);
        assert_eq!(find(&tables, "same").status, DiffStatus::Identical, "type case is not a change");
        let users = find(&tables, "users");
        assert_eq!(users.status, DiffStatus::Changed);
        let email = users.columns.iter().find(|c| c.name == "email").unwrap_or_else(|| panic!("email"));
        assert_eq!(email.changes, vec![ColumnChange::Type, ColumnChange::Nullable]);
        assert_eq!(users.columns.iter().find(|c| c.name == "nick").map(|c| c.status), Some(DiffStatus::Added));
        assert_eq!(users.columns.iter().find(|c| c.name == "legacy").map(|c| c.status), Some(DiffStatus::Removed));
    }

    #[test]
    fn direction_flips_added_and_removed() {
        let (l, r) = pair(Engine::Postgres);
        let tables = diff(&l, &r, CompareDirection::RightToLeft);
        assert_eq!(find(&tables, "orders").status, DiffStatus::Removed);
        assert_eq!(find(&tables, "old_audit").status, DiffStatus::Added);
    }

    #[test]
    fn primary_key_change_is_reported() {
        let l = snap(Engine::Postgres, None, vec![table(None, "t", vec![col("a", "int", false, true, 1), col("b", "int", false, true, 2)], vec![])]);
        let r = snap(Engine::Postgres, None, vec![table(None, "t", vec![col("a", "int", false, true, 1), col("b", "int", false, false, 2)], vec![])]);
        let tables = diff(&l, &r, CompareDirection::LeftToRight);
        let b = tables[0].columns.iter().find(|c| c.name == "b").unwrap_or_else(|| panic!("b"));
        assert_eq!(b.changes, vec![ColumnChange::PrimaryKey]);
        let script = render_script(&tables, &l, &r, &HashMap::new(), CompareDirection::LeftToRight);
        assert!(script.contains("ALTER TABLE \"t\" ADD PRIMARY KEY (\"a\", \"b\");"), "{script}");
        assert!(script.contains("-- ALTER TABLE \"t\" DROP CONSTRAINT"), "{script}");
    }

    fn script_for(engine: Engine) -> String {
        let (l, r) = pair(engine);
        let tables = diff(&l, &r, CompareDirection::LeftToRight);
        let mut creates = HashMap::new();
        creates.insert("orders".to_string(), "CREATE TABLE orders (id integer PRIMARY KEY, user_id integer NOT NULL)".to_string());
        render_script(&tables, &l, &r, &creates, CompareDirection::LeftToRight)
    }

    #[test]
    fn postgres_script() {
        let s = script_for(Engine::Postgres);
        assert!(s.contains("CREATE TABLE orders (id integer PRIMARY KEY, user_id integer NOT NULL);"), "{s}");
        assert!(s.contains("ALTER TABLE \"app\".\"USERS\" ADD COLUMN \"nick\" text;"), "{s}");
        assert!(s.contains("ALTER TABLE \"app\".\"USERS\" ALTER COLUMN \"email\" TYPE varchar(320);"), "{s}");
        assert!(s.contains("ALTER TABLE \"app\".\"USERS\" ALTER COLUMN \"email\" SET NOT NULL;"), "{s}");
        assert!(s.contains("ALTER TABLE \"app\".\"USERS\" DROP COLUMN \"legacy\";"), "{s}");
        assert!(s.contains("ALTER TABLE \"app\".\"orders\" ADD CONSTRAINT \"orders_user_fk\" FOREIGN KEY (\"user_id\") REFERENCES \"app\".\"users\" (\"id\");"), "{s}");
        assert!(s.contains("-- DROP TABLE \"app\".\"old_audit\";"), "{s}");
        assert!(!s.lines().any(|l| l.starts_with("DROP TABLE")), "drops stay commented");
        // Order: new table before the key that references it, drop last.
        let create = s.find("CREATE TABLE orders").unwrap_or(0);
        let key = s.find("ADD CONSTRAINT").unwrap_or(0);
        let drop = s.find("-- DROP TABLE").unwrap_or(0);
        assert!(create < key && key < drop, "{s}");
    }

    #[test]
    fn mysql_uses_modify_column() {
        let s = script_for(Engine::Mysql);
        assert!(s.contains("ALTER TABLE `app`.`USERS` MODIFY COLUMN `email` varchar(320) NOT NULL;"), "{s}");
        assert!(s.contains("ALTER TABLE `app`.`USERS` ADD COLUMN `nick` text;"), "{s}");
    }

    #[test]
    fn mssql_uses_alter_column_with_nullability() {
        let s = script_for(Engine::Mssql);
        assert!(s.contains("ALTER TABLE [app].[USERS] ALTER COLUMN [email] varchar(320) NOT NULL;"), "{s}");
        assert!(s.contains("ALTER TABLE [app].[USERS] ADD [nick] text NULL;"), "{s}");
    }

    #[test]
    fn oracle_uses_modify_parens() {
        let s = script_for(Engine::Oracle);
        assert!(s.contains("ALTER TABLE \"app\".\"USERS\" MODIFY (\"email\" varchar(320) NOT NULL);"), "{s}");
        assert!(s.contains("ALTER TABLE \"app\".\"USERS\" ADD (\"nick\" text);"), "{s}");
    }

    #[test]
    fn sqlite_writes_rebuild_comments() {
        let s = script_for(Engine::Sqlite);
        assert!(s.contains("ALTER TABLE \"USERS\" ADD COLUMN \"nick\" text;"), "{s}");
        assert!(s.contains("-- SQLite cannot alter USERS.email"), "{s}");
        assert!(s.contains("-- SQLite cannot add a foreign key to orders"), "{s}");
        assert!(!s.contains("MODIFY"), "{s}");
    }

    #[test]
    fn inline_references_are_not_added_twice() {
        let (l, r) = pair(Engine::Postgres);
        let tables = diff(&l, &r, CompareDirection::LeftToRight);
        let mut creates = HashMap::new();
        creates.insert("orders".to_string(), "CREATE TABLE orders (id integer, user_id integer REFERENCES users(id))".to_string());
        let s = render_script(&tables, &l, &r, &creates, CompareDirection::LeftToRight);
        assert!(!s.contains("ADD CONSTRAINT"), "{s}");
    }

    #[test]
    fn referenced_tables_are_created_first() {
        let l = snap(
            Engine::Postgres,
            None,
            vec![
                table(None, "a_child", vec![col("id", "int", false, true, 1)], vec![fk("c_fk", "a_child", &["id"], "z_parent", &["id"])]),
                table(None, "z_parent", vec![col("id", "int", false, true, 1)], vec![]),
            ],
        );
        let r = snap(Engine::Postgres, None, vec![]);
        let tables = diff(&l, &r, CompareDirection::LeftToRight);
        let mut creates = HashMap::new();
        creates.insert("a_child".to_string(), "CREATE TABLE a_child (id int)".to_string());
        creates.insert("z_parent".to_string(), "CREATE TABLE z_parent (id int)".to_string());
        let s = render_script(&tables, &l, &r, &creates, CompareDirection::LeftToRight);
        assert!(s.find("z_parent (id").unwrap_or(usize::MAX) < s.find("a_child (id").unwrap_or(0), "{s}");
    }

    #[test]
    fn identical_schemas_say_so() {
        let (l, _) = pair(Engine::Postgres);
        let tables = diff(&l, &l, CompareDirection::LeftToRight);
        assert!(tables.iter().all(|t| t.status == DiffStatus::Identical));
        let s = render_script(&tables, &l, &l, &HashMap::new(), CompareDirection::LeftToRight);
        assert!(s.contains("nothing to migrate"), "{s}");
    }

    #[test]
    fn foreign_key_removed_is_dropped_first() {
        let l = snap(Engine::Mysql, None, vec![table(None, "t", vec![col("id", "int", false, true, 1), col("p", "int", true, false, 2)], vec![])]);
        let r = snap(Engine::Mysql, None, vec![table(None, "t", vec![col("id", "int", false, true, 1), col("p", "int", true, false, 2)], vec![fk("t_p_fk", "t", &["p"], "p", &["id"])])]);
        let tables = diff(&l, &r, CompareDirection::LeftToRight);
        assert_eq!(tables[0].status, DiffStatus::Changed);
        let s = render_script(&tables, &l, &r, &HashMap::new(), CompareDirection::LeftToRight);
        assert!(s.contains("ALTER TABLE `t` DROP FOREIGN KEY `t_p_fk`;"), "{s}");
    }

    // ---- end to end over two SQLite files ------------------------------------

    async fn sqlite_ctx(setup: &str) -> SessionCtx {
        use crate::model::{ConnectionInput, ConnectionSummary, Environment, ResolvedConnection, SslMode};
        let dir = std::env::temp_dir().join(format!("db-free-schema-diff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let path = dir.join("s.db").to_string_lossy().into_owned();
        let input = ConnectionInput {
            name: "side".into(),
            engine: Engine::Sqlite,
            environment: Environment::Local,
            read_only: false,
            host: None,
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: Some(path),
            ssl_mode: SslMode::Disable,
            ssh: crate::model::SshTunnel::default(),
            ssh_secret: None,
            folder: None,
            color: None,
            favorite: false,
        };
        let summary = ConnectionSummary::draft(&input, false);
        let resolved = ResolvedConnection { summary: summary.clone(), secret: None };
        let integration = crate::integrations::connect(&resolved).await.unwrap_or_else(|e| panic!("{e}"));
        integration.execute(setup, 10).await.unwrap_or_else(|e| panic!("{e}"));
        SessionCtx { connection: summary, integration, started: std::time::Instant::now() }
    }

    #[tokio::test]
    async fn compares_two_sqlite_databases() {
        let left = sqlite_ctx(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL, nick TEXT); \
             CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER REFERENCES users(id));",
        )
        .await;
        let right = sqlite_ctx("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT); CREATE TABLE gone (id INTEGER);").await;
        let result = compare(&left, &right, None, None, CompareDirection::LeftToRight).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(find(&result.tables, "orders").status, DiffStatus::Added);
        assert_eq!(find(&result.tables, "gone").status, DiffStatus::Removed);
        assert_eq!(find(&result.tables, "users").status, DiffStatus::Changed);
        // Same family and namespace: the source's own DDL is copied, inline key included.
        assert!(result.script.contains("CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER REFERENCES users(id));"), "{}", result.script);
        assert!(result.script.to_lowercase().contains("alter table \"users\" add column \"nick\" text;"), "{}", result.script);
        assert!(result.script.contains("-- DROP TABLE \"gone\";"), "{}", result.script);

        // The script runs against the target and leaves only the commented-out drop.
        right.integration.execute(&result.script, 10).await.unwrap_or_else(|e| panic!("{e}\n{}", result.script));
        let again = compare(&left, &right, None, None, CompareDirection::LeftToRight).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(find(&again.tables, "orders").status, DiffStatus::Identical);
        assert_eq!(find(&again.tables, "gone").status, DiffStatus::Removed);
    }
}
