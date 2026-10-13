// SOT: diagram-ddl, diagram-sql-generation, designer-create-table-script, designer-foreign-key-alter

use crate::integrations::{quote_ident, quote_ident_for};
use crate::model::{DiagramBody, DiagramColumn, DiagramDdl, DiagramRelation, DiagramTable, Engine, Family, ReferentialAction};
use std::collections::{HashMap, HashSet};

// WHAT:  Writes the SQL for a designed diagram in the dialect of the connection
//        it belongs to: identifier quoting, schema-qualified names, one CREATE
//        TABLE per table, foreign keys as ALTER TABLE … ADD CONSTRAINT with their
//        ON DELETE / ON UPDATE actions, and a warning for everything the dialect
//        cannot express.
// WHY:   The designer used to double-quote everything and inline the foreign keys,
//        which is wrong for MySQL (backticks), SQL Server (brackets), SQLite
//        (no ALTER ADD CONSTRAINT) and ClickHouse (no foreign keys at all). A
//        statement the database rejects teaches the user nothing; a script plus
//        a list of what was left out does.
// HOW:   Pure: (engine, diagram, optional table) in, (sql, warnings) out. The
//        caller resolves the engine from the diagram's connection. With `only`
//        set the result is that one table: its CREATE TABLE and the foreign keys
//        it owns, which is what the inspector's per-table preview shows.
// WHERE: src-tauri/src/services/diagrams.rs (caller), src-tauri/src/commands/diagrams.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Postgres,
    Mysql,
    Mssql,
    Oracle,
    Sqlite,
    Clickhouse,
    Duckdb,
    Snowflake,
    Bigquery,
    /// No connection, or an engine without SQL DDL of its own: ANSI spelling.
    Generic,
}

fn dialect_of(engine: Engine) -> Dialect {
    match engine.family() {
        Family::Postgres => Dialect::Postgres,
        Family::Mysql => Dialect::Mysql,
        Family::Mssql => Dialect::Mssql,
        Family::Oracle => Dialect::Oracle,
        Family::Sqlite | Family::Libsql | Family::ValTown | Family::CloudflareD1 => Dialect::Sqlite,
        Family::Clickhouse => Dialect::Clickhouse,
        Family::Duckdb => Dialect::Duckdb,
        Family::Snowflake => Dialect::Snowflake,
        Family::Bigquery => Dialect::Bigquery,
        _ => Dialect::Generic,
    }
}

/// Where a foreign key can be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FkMode {
    /// `ALTER TABLE … ADD CONSTRAINT … FOREIGN KEY`.
    Alter,
    /// Inside the CREATE TABLE of the owning table (SQLite, DuckDB).
    Inline,
    /// The engine has none.
    Unsupported,
}

impl Dialect {
    fn label(self) -> &'static str {
        match self {
            Dialect::Postgres => "PostgreSQL",
            Dialect::Mysql => "MySQL",
            Dialect::Mssql => "SQL Server",
            Dialect::Oracle => "Oracle",
            Dialect::Sqlite => "SQLite",
            Dialect::Clickhouse => "ClickHouse",
            Dialect::Duckdb => "DuckDB",
            Dialect::Snowflake => "Snowflake",
            Dialect::Bigquery => "BigQuery",
            Dialect::Generic => "this dialect",
        }
    }

    fn fk_mode(self) -> FkMode {
        match self {
            Dialect::Sqlite | Dialect::Duckdb => FkMode::Inline,
            Dialect::Clickhouse => FkMode::Unsupported,
            _ => FkMode::Alter,
        }
    }

    /// Longest identifier the engine accepts for a constraint name.
    fn ident_max(self) -> usize {
        match self {
            Dialect::Postgres => 63,
            Dialect::Mysql => 64,
            Dialect::Mssql | Dialect::Oracle => 128,
            _ => 255,
        }
    }

    /// Type to fall back on for a column left without one.
    fn fallback_type(self) -> &'static str {
        match self {
            Dialect::Sqlite => "TEXT",
            Dialect::Clickhouse => "String",
            Dialect::Bigquery => "STRING",
            Dialect::Mssql => "nvarchar(255)",
            Dialect::Oracle => "VARCHAR2(255)",
            _ => "text",
        }
    }
}

/// What a dialect does with one referential action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionSupport {
    Emit,
    /// Written as this action instead (SQL Server has no RESTRICT).
    Replace(ReferentialAction),
    /// Left out; `warn` is false when the omitted action is the default anyway.
    Omit { warn: bool },
}

fn action_support(dialect: Dialect, on_delete: bool, action: ReferentialAction) -> ActionSupport {
    use ReferentialAction::{Cascade, NoAction, Restrict, SetDefault, SetNull};
    match dialect {
        Dialect::Postgres | Dialect::Sqlite | Dialect::Generic => ActionSupport::Emit,
        Dialect::Mysql => match action {
            SetDefault => ActionSupport::Omit { warn: true },
            _ => ActionSupport::Emit,
        },
        Dialect::Mssql => match action {
            Restrict => ActionSupport::Replace(NoAction),
            _ => ActionSupport::Emit,
        },
        Dialect::Oracle => match (on_delete, action) {
            (true, Cascade | SetNull) => ActionSupport::Emit,
            (_, NoAction) => ActionSupport::Omit { warn: false },
            _ => ActionSupport::Omit { warn: true },
        },
        Dialect::Duckdb => match action {
            NoAction | Restrict => ActionSupport::Emit,
            _ => ActionSupport::Omit { warn: true },
        },
        Dialect::Snowflake | Dialect::Bigquery | Dialect::Clickhouse => match action {
            NoAction => ActionSupport::Omit { warn: false },
            _ => ActionSupport::Omit { warn: true },
        },
    }
}

// ---------------------------------------------------------------- generator

/// A table that can be written: named, with at least one named column.
struct Usable<'a> {
    table: &'a DiagramTable,
    columns: Vec<&'a DiagramColumn>,
}

/// A foreign key that survived validation.
struct Fk<'a> {
    child: usize,
    parent: usize,
    relation: &'a DiagramRelation,
    name: String,
    from: Vec<String>,
    to: Vec<String>,
}

struct Generator<'a> {
    engine: Option<Engine>,
    dialect: Dialect,
    body: &'a DiagramBody,
    only: Option<&'a str>,
    warnings: Vec<String>,
}

pub fn generate(engine: Option<Engine>, body: &DiagramBody, only: Option<&str>) -> DiagramDdl {
    let dialect = engine.map_or(Dialect::Generic, dialect_of);
    let mut generator = Generator { engine, dialect, body, only, warnings: Vec::new() };
    let sql = generator.run();
    let mut seen = HashSet::new();
    generator.warnings.retain(|w| seen.insert(w.clone()));
    DiagramDdl { sql, warnings: generator.warnings }
}

impl<'a> Generator<'a> {
    fn warn(&mut self, message: String) {
        self.warnings.push(message);
    }

    fn quote(&self, raw: &str) -> String {
        match self.engine {
            Some(engine) => quote_ident_for(engine, raw),
            None => quote_ident(raw),
        }
    }

    fn quote_list(&self, names: &[String]) -> String {
        names.iter().map(|n| self.quote(n)).collect::<Vec<_>>().join(", ")
    }

    /// `schema.table`, or the bare name where the dialect has no schemas.
    fn qualified(&self, table: &DiagramTable) -> String {
        let name = self.quote(table.name.trim());
        match table.schema.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(schema) if self.dialect != Dialect::Sqlite => format!("{}.{name}", self.quote(schema)),
            _ => name,
        }
    }

    // WHAT:  A string literal. MySQL, ClickHouse and BigQuery also read a
    //        backslash as an escape, so it is doubled there.
    fn literal(&self, raw: &str) -> String {
        let escaped = raw.replace('\'', "''");
        match self.dialect {
            Dialect::Mysql | Dialect::Clickhouse | Dialect::Bigquery => format!("'{}'", escaped.replace('\\', "\\\\")),
            _ => format!("'{escaped}'"),
        }
    }

    fn in_scope(&self, table_id: &str) -> bool {
        self.only.is_none_or(|o| o == table_id)
    }

    fn run(&mut self) -> String {
        let body = self.body;
        let mut usable: Vec<Usable<'a>> = Vec::new();
        let mut qualified_seen: HashSet<String> = HashSet::new();
        for table in &body.tables {
            let scoped = self.in_scope(&table.id);
            if table.name.trim().is_empty() {
                if scoped {
                    self.warn("Skipped a table with no name.".to_string());
                }
                continue;
            }
            let columns: Vec<&DiagramColumn> = table.columns.iter().filter(|c| !c.name.trim().is_empty()).collect();
            if columns.is_empty() {
                if scoped {
                    self.warn(format!("Table \"{}\" has no columns; skipped.", table.name.trim()));
                }
                continue;
            }
            if scoped && !qualified_seen.insert(self.qualified(table).to_lowercase()) {
                self.warn(format!("Two tables are both called \"{}\"; the script would fail on the second.", table.name.trim()));
            }
            usable.push(Usable { table, columns });
        }
        if let Some(only) = self.only {
            if !body.tables.iter().any(|t| t.id == only) {
                self.warn("That table is no longer in the diagram.".to_string());
            }
        }
        if self.dialect == Dialect::Sqlite && usable.iter().any(|u| u.table.schema.as_deref().is_some_and(|s| !s.trim().is_empty())) {
            self.warn("SQLite has no schemas; the schema names were left out.".to_string());
        }

        let fks = self.plan_foreign_keys(&usable);
        let order = self.creation_order(&usable, &fks);

        let mut table_blocks: Vec<String> = Vec::new();
        for index in order {
            let Some(item) = usable.get(index) else { continue };
            if !self.in_scope(&item.table.id) {
                continue;
            }
            table_blocks.push(self.table_block(&usable, item, index, &fks));
        }

        let mut sections = table_blocks.join("\n\n");
        if self.dialect.fk_mode() == FkMode::Alter {
            let owned: Vec<&Fk<'a>> = fks.iter().filter(|fk| usable.get(fk.child).is_some_and(|u| self.in_scope(&u.table.id))).collect();
            let statements: Vec<String> = owned.into_iter().map(|fk| self.alter_statement(&usable, fk)).collect();
            if !statements.is_empty() {
                if !sections.is_empty() {
                    sections.push_str("\n\n");
                }
                sections.push_str("-- Foreign keys\n");
                sections.push_str(&statements.join("\n"));
            }
        }
        sections
    }

    // ---------------------------------------------------------- foreign keys

    fn plan_foreign_keys(&mut self, usable: &[Usable<'a>]) -> Vec<Fk<'a>> {
        let body = self.body;
        let by_id: HashMap<&str, usize> = usable.iter().enumerate().map(|(i, u)| (u.table.id.as_str(), i)).collect();
        let mut plans: Vec<Fk<'a>> = Vec::new();
        let mut used_names: HashSet<String> = HashSet::new();
        let mut dropped_for_dialect = 0usize;
        for relation in &body.relations {
            if !self.in_scope(&relation.from_table) {
                continue;
            }
            let label = self.relation_label(relation);
            let (Some(&child), Some(&parent)) = (by_id.get(relation.from_table.as_str()), by_id.get(relation.to_table.as_str())) else {
                self.warn(format!("Relation {label} points at a table that is missing or cannot be written; skipped."));
                continue;
            };
            if self.dialect.fk_mode() == FkMode::Unsupported {
                dropped_for_dialect += 1;
                continue;
            }
            let pairs = relation.pairs();
            if pairs.iter().any(|(from, to)| from.trim().is_empty() || to.trim().is_empty()) {
                self.warn(format!("Relation {label} has no columns chosen; skipped."));
                continue;
            }
            let (Some(child_t), Some(parent_t)) = (usable.get(child), usable.get(parent)) else { continue };
            let missing = pairs.iter().find_map(|(from, to)| {
                if !child_t.columns.iter().any(|c| c.name == *from) {
                    Some(format!("\"{}.{}\"", child_t.table.name, from))
                } else if !parent_t.columns.iter().any(|c| c.name == *to) {
                    Some(format!("\"{}.{}\"", parent_t.table.name, to))
                } else {
                    None
                }
            });
            if let Some(column) = missing {
                self.warn(format!("Relation {label} uses the column {column}, which does not exist; skipped."));
                continue;
            }
            let from: Vec<String> = pairs.iter().map(|(f, _)| (*f).to_string()).collect();
            let to: Vec<String> = pairs.iter().map(|(_, t)| (*t).to_string()).collect();
            self.check_pairs(&label, child_t, parent_t, &from, &to);
            let name = self.constraint_name(relation, child_t.table, &from, &mut used_names);
            plans.push(Fk { child, parent, relation, name, from, to });
        }
        if dropped_for_dialect > 0 {
            self.warn(format!(
                "{} has no foreign keys; {dropped_for_dialect} relation{} left out.",
                self.dialect.label(),
                if dropped_for_dialect == 1 { " was" } else { "s were" }
            ));
        }
        plans
    }

    fn relation_label(&self, relation: &DiagramRelation) -> String {
        let name_of = |id: &str| {
            self.body
                .tables
                .iter()
                .find(|t| t.id == id)
                .map_or_else(|| "?".to_string(), |t| t.name.trim().to_string())
        };
        format!("\"{} → {}\"", name_of(&relation.from_table), name_of(&relation.to_table))
    }

    // WHAT:  Warns about a pair a database may reject: a referenced key that is
    //        not unique, or columns of different types.
    fn check_pairs(&mut self, label: &str, child: &Usable<'_>, parent: &Usable<'_>, from: &[String], to: &[String]) {
        let parent_pk: Vec<&str> = parent.columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
        let covers_pk = !parent_pk.is_empty() && parent_pk.len() == to.len() && parent_pk.iter().all(|p| to.iter().any(|t| t.as_str() == *p));
        let single_unique = to.len() == 1 && parent.columns.iter().any(|c| c.name == to[0] && (c.unique || c.primary_key));
        if !covers_pk && !single_unique {
            self.warn(format!(
                "Relation {label} references columns that are not a primary key or unique; the database may reject the constraint."
            ));
        }
        for (f, t) in from.iter().zip(to) {
            let (Some(fc), Some(tc)) = (child.columns.iter().find(|c| &c.name == f), parent.columns.iter().find(|c| &c.name == t)) else {
                continue;
            };
            if type_key(&fc.data_type) != type_key(&tc.data_type) {
                self.warn(format!(
                    "Relation {label}: \"{f}\" is {} but \"{t}\" is {}; the types differ.",
                    fc.data_type.trim(),
                    tc.data_type.trim()
                ));
            }
        }
    }

    fn constraint_name(&mut self, relation: &DiagramRelation, child: &DiagramTable, from: &[String], used: &mut HashSet<String>) -> String {
        let base = relation
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map_or_else(|| format!("fk_{}_{}", child.name.trim(), from.join("_")), str::to_string);
        let max = self.dialect.ident_max();
        let mut candidate: String = base.chars().take(max).collect();
        if base.chars().count() > max {
            self.warn(format!("The constraint name \"{base}\" is longer than {} allows; it was shortened.", self.dialect.label()));
        }
        let mut n = 2;
        while !used.insert(candidate.to_lowercase()) {
            let suffix = format!("_{n}");
            let keep = max.saturating_sub(suffix.chars().count());
            candidate = format!("{}{suffix}", base.chars().take(keep).collect::<String>());
            n += 1;
        }
        candidate
    }

    // WHAT:  ` ON DELETE x ON UPDATE y`, with what the dialect cannot do left
    //        out and reported.
    fn actions(&mut self, label: &str, relation: &DiagramRelation) -> String {
        let mut out = String::new();
        for (clause, is_delete, action) in [("ON DELETE", true, relation.on_delete), ("ON UPDATE", false, relation.on_update)] {
            let Some(action) = action else { continue };
            match action_support(self.dialect, is_delete, action) {
                ActionSupport::Emit => out.push_str(&format!(" {clause} {}", action.sql())),
                ActionSupport::Replace(instead) => {
                    self.warn(format!("Relation {label}: {} has no {clause} {}; {} was written instead.", self.dialect.label(), action.sql(), instead.sql()));
                    out.push_str(&format!(" {clause} {}", instead.sql()));
                }
                ActionSupport::Omit { warn } => {
                    if warn {
                        self.warn(format!("Relation {label}: {clause} {} is not supported by {}; left out.", action.sql(), self.dialect.label()));
                    }
                }
            }
        }
        out
    }

    fn fk_clause(&mut self, usable: &[Usable<'a>], fk: &Fk<'a>) -> String {
        let label = self.relation_label(fk.relation);
        let parent = usable.get(fk.parent).map_or_else(String::new, |p| self.qualified(p.table));
        let mut text = format!(
            "FOREIGN KEY ({}) REFERENCES {parent} ({})",
            self.quote_list(&fk.from),
            self.quote_list(&fk.to)
        );
        text.push_str(&self.actions(&label, fk.relation));
        if self.dialect == Dialect::Bigquery {
            text.push_str(" NOT ENFORCED");
        }
        text
    }

    fn alter_statement(&mut self, usable: &[Usable<'a>], fk: &Fk<'a>) -> String {
        let child = usable.get(fk.child).map_or_else(String::new, |c| self.qualified(c.table));
        let clause = self.fk_clause(usable, fk);
        format!("ALTER TABLE {child} ADD CONSTRAINT {} {clause};", self.quote(&fk.name))
    }

    // WHAT:  Parents before children, so an inline foreign key never points at a
    //        table that does not exist yet. A cycle keeps its diagram order.
    fn creation_order(&mut self, usable: &[Usable<'a>], fks: &[Fk<'a>]) -> Vec<usize> {
        let count = usable.len();
        if self.dialect.fk_mode() != FkMode::Inline {
            return (0..count).collect();
        }
        let mut indegree = vec![0usize; count];
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
        for fk in fks.iter().filter(|fk| fk.child != fk.parent) {
            if let (Some(deg), Some(list)) = (indegree.get_mut(fk.child), children.get_mut(fk.parent)) {
                *deg += 1;
                list.push(fk.child);
            }
        }
        let mut ready: Vec<usize> = (0..count).filter(|i| indegree.get(*i) == Some(&0)).collect();
        let mut order = Vec::with_capacity(count);
        while !ready.is_empty() {
            let next = ready.remove(0);
            order.push(next);
            for &child in children.get(next).map(Vec::as_slice).unwrap_or_default() {
                if let Some(deg) = indegree.get_mut(child) {
                    *deg -= 1;
                    if *deg == 0 {
                        ready.push(child);
                    }
                }
            }
            ready.sort_unstable();
        }
        if order.len() < count {
            self.warn("These tables reference each other in a circle; their order may need adjusting by hand.".to_string());
            let left_out: Vec<usize> = (0..count).filter(|i| !order.contains(i)).collect();
            order.extend(left_out);
        }
        order
    }

    // ---------------------------------------------------------------- tables

    fn table_block(&mut self, usable: &[Usable<'a>], item: &Usable<'a>, index: usize, fks: &[Fk<'a>]) -> String {
        let table = item.table;
        let qualified = self.qualified(table);
        let table_name = table.name.trim();
        let primary: Vec<&DiagramColumn> = item.columns.iter().copied().filter(|c| c.primary_key).collect();
        let inline_key = self.dialect == Dialect::Sqlite
            && primary.len() == 1
            && primary.first().is_some_and(|c| c.auto_increment && is_integer_type(&c.data_type));

        let mut lines: Vec<String> = item.columns.iter().map(|c| self.column_def(table_name, c, inline_key)).collect();
        if !primary.is_empty() && !inline_key && self.dialect != Dialect::Clickhouse {
            let names: Vec<String> = primary.iter().map(|c| c.name.clone()).collect();
            let not_enforced = if self.dialect == Dialect::Bigquery { " NOT ENFORCED" } else { "" };
            lines.push(format!("PRIMARY KEY ({}){not_enforced}", self.quote_list(&names)));
        }
        if self.dialect.fk_mode() == FkMode::Inline {
            for fk in fks.iter().filter(|fk| fk.child == index) {
                let clause = self.fk_clause(usable, fk);
                lines.push(format!("CONSTRAINT {} {clause}", self.quote(&fk.name)));
            }
        }

        let mut statement = format!("CREATE TABLE {qualified} (\n  {}\n)", lines.join(",\n  "));
        let table_comment = table.comment.as_deref().map(str::trim).filter(|c| !c.is_empty());
        match self.dialect {
            Dialect::Clickhouse => {
                let keys: Vec<String> = primary.iter().map(|c| self.quote(&c.name)).collect();
                let order = if keys.is_empty() { "tuple()".to_string() } else { format!("({})", keys.join(", ")) };
                statement.push_str(&format!(" ENGINE = MergeTree ORDER BY {order}"));
                if let Some(comment) = table_comment {
                    statement.push_str(&format!(" COMMENT {}", self.literal(comment)));
                }
            }
            Dialect::Mysql => {
                if let Some(comment) = table_comment {
                    statement.push_str(&format!(" COMMENT={}", self.literal(comment)));
                }
            }
            Dialect::Bigquery => {
                if let Some(comment) = table_comment {
                    statement.push_str(&format!(" OPTIONS(description={})", self.literal(comment)));
                }
            }
            _ => {}
        }
        statement.push(';');

        let mut block = vec![statement];
        match self.dialect {
            Dialect::Postgres | Dialect::Oracle | Dialect::Duckdb | Dialect::Snowflake | Dialect::Generic => {
                if let Some(comment) = table_comment {
                    block.push(format!("COMMENT ON TABLE {qualified} IS {};", self.literal(comment)));
                }
                for column in &item.columns {
                    if let Some(comment) = column.comment.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                        block.push(format!("COMMENT ON COLUMN {qualified}.{} IS {};", self.quote(&column.name), self.literal(comment)));
                    }
                }
            }
            Dialect::Mssql | Dialect::Sqlite => {
                let has_comment = table_comment.is_some() || item.columns.iter().any(|c| c.comment.as_deref().is_some_and(|t| !t.trim().is_empty()));
                if has_comment {
                    self.warn(format!("{} has no inline comments; the comments on \"{table_name}\" were left out.", self.dialect.label()));
                }
            }
            Dialect::Mysql | Dialect::Clickhouse | Dialect::Bigquery => {}
        }
        block.join("\n")
    }

    fn column_def(&mut self, table_name: &str, column: &DiagramColumn, inline_key: bool) -> String {
        let name = self.quote(column.name.trim());
        let given = column.data_type.trim();
        let data_type = if given.is_empty() {
            let fallback = self.dialect.fallback_type();
            self.warn(format!("Column \"{table_name}.{}\" has no type; {fallback} was used.", column.name.trim()));
            fallback
        } else {
            given
        };
        if inline_key {
            return format!("{name} INTEGER PRIMARY KEY AUTOINCREMENT");
        }
        let shown_type = if self.dialect == Dialect::Clickhouse && column.nullable && !column.primary_key && !is_wrapped_clickhouse(data_type) {
            format!("Nullable({data_type})")
        } else {
            data_type.to_string()
        };
        let mut out = format!("{name} {shown_type}");

        let default = column.default_value.as_deref().map(str::trim).filter(|d| !d.is_empty());
        let identity = self.identity(table_name, column, data_type);
        if let Identity::AfterType(text) = identity {
            out.push(' ');
            out.push_str(text);
        }
        if let Some(default) = default {
            if identity == Identity::None {
                out.push_str(&format!(" DEFAULT {default}"));
            } else {
                self.warn(format!("Column \"{table_name}.{}\" is auto-increment; its default was left out.", column.name.trim()));
            }
        }
        match self.dialect {
            Dialect::Clickhouse => {}
            Dialect::Mssql if column.nullable && !column.primary_key => out.push_str(" NULL"),
            _ => {
                if !column.nullable || column.primary_key {
                    out.push_str(" NOT NULL");
                }
            }
        }
        if let Identity::AfterNull(text) = identity {
            out.push(' ');
            out.push_str(text);
        }
        if column.unique && !column.primary_key {
            if matches!(self.dialect, Dialect::Clickhouse | Dialect::Bigquery) {
                self.warn(format!("{} has no UNIQUE constraint; \"{table_name}.{}\" is not unique there.", self.dialect.label(), column.name.trim()));
            } else {
                out.push_str(" UNIQUE");
            }
        }
        if let Some(comment) = column.comment.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            match self.dialect {
                Dialect::Mysql | Dialect::Clickhouse => out.push_str(&format!(" COMMENT {}", self.literal(comment))),
                Dialect::Bigquery => out.push_str(&format!(" OPTIONS(description={})", self.literal(comment))),
                _ => {}
            }
        }
        out
    }

    fn identity(&mut self, table_name: &str, column: &DiagramColumn, data_type: &str) -> Identity {
        if !column.auto_increment {
            return Identity::None;
        }
        let which = format!("\"{table_name}.{}\"", column.name.trim());
        if !takes_identity(self.dialect, data_type) {
            self.warn(format!("Auto-increment on {which} needs an integer type; it was left out."));
            return Identity::None;
        }
        match self.dialect {
            Dialect::Postgres | Dialect::Oracle | Dialect::Generic => {
                if data_type.to_lowercase().contains("serial") {
                    Identity::None
                } else {
                    Identity::AfterType("GENERATED BY DEFAULT AS IDENTITY")
                }
            }
            Dialect::Mysql => Identity::AfterNull("AUTO_INCREMENT"),
            Dialect::Mssql => Identity::AfterType("IDENTITY(1,1)"),
            Dialect::Snowflake => Identity::AfterType("AUTOINCREMENT"),
            Dialect::Sqlite => {
                self.warn(format!("SQLite only auto-increments a single INTEGER primary key; {which} was left plain."));
                Identity::None
            }
            Dialect::Duckdb | Dialect::Clickhouse | Dialect::Bigquery => {
                self.warn(format!("{} has no auto-increment column; {which} was left plain.", self.dialect.label()));
                Identity::None
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Identity {
    None,
    AfterType(&'static str),
    AfterNull(&'static str),
}

// ------------------------------------------------------------------ helpers

fn base_type(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    let head = lower.split('(').next().unwrap_or_default();
    head.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_integer_type(raw: &str) -> bool {
    let base = base_type(raw);
    base.starts_with("int") || base.ends_with("int") || base.contains("serial")
}

// WHAT:  Whether a column of this type can be an identity column in this dialect.
//        Oracle and Snowflake spell their integers NUMBER.
fn takes_identity(dialect: Dialect, raw: &str) -> bool {
    is_integer_type(raw) || (matches!(dialect, Dialect::Oracle | Dialect::Snowflake) && base_type(raw).starts_with("number"))
}

/// ClickHouse types that already say "may be null" or cannot be wrapped.
fn is_wrapped_clickhouse(raw: &str) -> bool {
    let lower = raw.trim().to_lowercase();
    ["nullable(", "array(", "map(", "tuple(", "lowcardinality("].iter().any(|p| lower.starts_with(p))
}

// WHAT:  A coarse family for comparing the two ends of a foreign key.
// WHY:   `int` / `integer` / `serial` are one type to a database, so comparing
//        spellings would warn on every ordinary key. Unknown types compare as written.
fn type_key(raw: &str) -> String {
    let base = base_type(raw);
    let family = match base.as_str() {
        "int" | "integer" | "int4" | "serial" | "mediumint" | "serial4" => "int",
        "bigint" | "int8" | "bigserial" | "serial8" => "bigint",
        "smallint" | "int2" | "smallserial" | "tinyint" => "smallint",
        "text" | "varchar" | "character varying" | "char" | "character" | "nvarchar" | "nchar" | "string" | "varchar2" | "mediumtext" | "longtext" => "text",
        "uuid" | "uniqueidentifier" => "uuid",
        "bool" | "boolean" | "bit" => "bool",
        "timestamp" | "timestamptz" | "datetime" | "datetime2" | "timestamp with time zone" | "timestamp without time zone" => "timestamp",
        _ => return base,
    };
    family.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DiagramColumnPair;

    fn col(name: &str, ty: &str, pk: bool) -> DiagramColumn {
        DiagramColumn { name: name.into(), data_type: ty.into(), primary_key: pk, nullable: !pk, ..DiagramColumn::default() }
    }

    fn table(id: &str, name: &str, columns: Vec<DiagramColumn>) -> DiagramTable {
        DiagramTable { id: id.into(), name: name.into(), columns, ..DiagramTable::default() }
    }

    fn relation(id: &str, from: (&str, &str), to: (&str, &str)) -> DiagramRelation {
        DiagramRelation {
            id: id.into(),
            from_table: from.0.into(),
            from_column: from.1.into(),
            to_table: to.0.into(),
            to_column: to.1.into(),
            ..DiagramRelation::default()
        }
    }

    fn shop() -> DiagramBody {
        DiagramBody {
            tables: vec![
                table("u", "users", vec![col("id", "integer", true), col("email", "text", false)]),
                table("o", "orders", vec![col("id", "integer", true), col("user_id", "integer", false)]),
            ],
            relations: vec![relation("r", ("o", "user_id"), ("u", "id"))],
            ..DiagramBody::default()
        }
    }

    #[test]
    fn postgres_writes_tables_then_alter_constraints() {
        let mut body = shop();
        if let Some(rel) = body.relations.first_mut() {
            rel.on_delete = Some(ReferentialAction::Cascade);
            rel.on_update = Some(ReferentialAction::NoAction);
        }
        for t in &mut body.tables {
            t.schema = Some("shop".into());
        }
        let ddl = generate(Some(Engine::Postgres), &body, None);
        assert!(ddl.sql.contains("CREATE TABLE \"shop\".\"users\" (\n  \"id\" integer NOT NULL,\n  \"email\" text,\n  PRIMARY KEY (\"id\")\n);"), "{}", ddl.sql);
        assert!(
            ddl.sql.contains(
                "ALTER TABLE \"shop\".\"orders\" ADD CONSTRAINT \"fk_orders_user_id\" FOREIGN KEY (\"user_id\") REFERENCES \"shop\".\"users\" (\"id\") ON DELETE CASCADE ON UPDATE NO ACTION;"
            ),
            "{}",
            ddl.sql
        );
        assert!(ddl.warnings.is_empty(), "{:?}", ddl.warnings);
        assert!(ddl.sql.find("CREATE TABLE \"shop\".\"orders\"").unwrap_or(0) < ddl.sql.find("ALTER TABLE").unwrap_or(0));
    }

    #[test]
    fn mysql_uses_backticks_auto_increment_and_inline_comments() {
        let mut body = shop();
        if let Some(users) = body.tables.first_mut() {
            users.comment = Some("People who\\buy".into());
            if let Some(id) = users.columns.first_mut() {
                id.auto_increment = true;
                id.comment = Some("it's the key".into());
            }
        }
        let ddl = generate(Some(Engine::Mysql), &body, Some("u"));
        assert!(ddl.sql.contains("`id` integer NOT NULL AUTO_INCREMENT COMMENT 'it''s the key'"), "{}", ddl.sql);
        assert!(ddl.sql.contains(") COMMENT='People who\\\\buy';"), "{}", ddl.sql);
        assert!(!ddl.sql.contains("orders"));
    }

    #[test]
    fn sqlite_inlines_foreign_keys_and_orders_parents_first() {
        let mut body = shop();
        body.tables.reverse();
        if let Some(users) = body.tables.iter_mut().find(|t| t.id == "u") {
            users.schema = Some("main".into());
            if let Some(id) = users.columns.first_mut() {
                id.auto_increment = true;
            }
        }
        let ddl = generate(Some(Engine::Sqlite), &body, None);
        assert!(ddl.sql.find("CREATE TABLE \"users\"").unwrap_or(usize::MAX) < ddl.sql.find("CREATE TABLE \"orders\"").unwrap_or(0), "{}", ddl.sql);
        assert!(ddl.sql.contains("\"id\" INTEGER PRIMARY KEY AUTOINCREMENT"), "{}", ddl.sql);
        assert!(ddl.sql.contains("CONSTRAINT \"fk_orders_user_id\" FOREIGN KEY (\"user_id\") REFERENCES \"users\" (\"id\")"), "{}", ddl.sql);
        assert!(!ddl.sql.contains("ALTER TABLE"));
        assert!(ddl.warnings.iter().any(|w| w.contains("no schemas")), "{:?}", ddl.warnings);
    }

    #[test]
    fn sql_server_brackets_identity_and_restrict_substitution() {
        let mut body = shop();
        if let Some(rel) = body.relations.first_mut() {
            rel.on_delete = Some(ReferentialAction::Restrict);
        }
        if let Some(orders) = body.tables.get_mut(1) {
            if let Some(id) = orders.columns.first_mut() {
                id.auto_increment = true;
            }
        }
        let ddl = generate(Some(Engine::Mssql), &body, None);
        assert!(ddl.sql.contains("[id] integer IDENTITY(1,1) NOT NULL"), "{}", ddl.sql);
        assert!(ddl.sql.contains("[email] text NULL"), "{}", ddl.sql);
        assert!(ddl.sql.contains("ON DELETE NO ACTION;"), "{}", ddl.sql);
        assert!(ddl.warnings.iter().any(|w| w.contains("RESTRICT")), "{:?}", ddl.warnings);
    }

    #[test]
    fn oracle_drops_on_update_and_unsupported_delete_actions() {
        let mut body = shop();
        if let Some(rel) = body.relations.first_mut() {
            rel.on_delete = Some(ReferentialAction::Restrict);
            rel.on_update = Some(ReferentialAction::Cascade);
        }
        let ddl = generate(Some(Engine::Oracle), &body, None);
        assert!(!ddl.sql.contains("ON DELETE") && !ddl.sql.contains("ON UPDATE"), "{}", ddl.sql);
        assert_eq!(ddl.warnings.len(), 2, "{:?}", ddl.warnings);
    }

    #[test]
    fn clickhouse_wraps_nullable_adds_an_engine_and_drops_relations() {
        let ddl = generate(Some(Engine::Clickhouse), &shop(), None);
        assert!(ddl.sql.contains("\"email\" Nullable(text)"), "{}", ddl.sql);
        assert!(ddl.sql.contains("\"id\" integer,") || ddl.sql.contains("\"id\" integer\n"), "{}", ddl.sql);
        assert!(ddl.sql.contains(") ENGINE = MergeTree ORDER BY (\"id\");"), "{}", ddl.sql);
        assert!(!ddl.sql.contains("FOREIGN KEY") && !ddl.sql.contains("PRIMARY KEY"));
        assert!(ddl.warnings.iter().any(|w| w.contains("no foreign keys")), "{:?}", ddl.warnings);
    }

    #[test]
    fn duckdb_keeps_foreign_keys_inside_the_table_and_drops_the_actions_it_lacks() {
        let mut body = shop();
        if let Some(rel) = body.relations.first_mut() {
            rel.on_delete = Some(ReferentialAction::Cascade);
            rel.on_update = Some(ReferentialAction::Restrict);
        }
        if let Some(id) = body.tables.get_mut(1).and_then(|t| t.columns.first_mut()) {
            id.auto_increment = true;
        }
        // The child is drawn first; the script still creates the parent before it.
        body.tables.reverse();
        let ddl = generate(Some(Engine::Duckdb), &body, None);
        assert!(!ddl.sql.contains("ALTER TABLE"), "{}", ddl.sql);
        assert!(ddl.sql.contains("CONSTRAINT \"fk_orders_user_id\" FOREIGN KEY (\"user_id\") REFERENCES \"users\" (\"id\") ON UPDATE RESTRICT"), "{}", ddl.sql);
        assert!(!ddl.sql.contains("ON DELETE"), "{}", ddl.sql);
        assert!(ddl.warnings.iter().any(|w| w.contains("ON DELETE CASCADE is not supported")), "{:?}", ddl.warnings);
        assert!(ddl.warnings.iter().any(|w| w.contains("no auto-increment")), "{:?}", ddl.warnings);
        assert!(ddl.sql.find("CREATE TABLE \"users\"").unwrap_or(usize::MAX) < ddl.sql.find("CREATE TABLE \"orders\"").unwrap_or(0), "parents come first: {}", ddl.sql);
    }

    #[test]
    fn snowflake_uses_autoincrement_and_bigquery_marks_its_keys_not_enforced() {
        let mut body = shop();
        if let Some(id) = body.tables.first_mut().and_then(|t| t.columns.first_mut()) {
            id.data_type = "NUMBER(38,0)".into();
            id.auto_increment = true;
        }
        if let Some(users) = body.tables.first_mut() {
            users.comment = Some("People".into());
        }
        let snowflake = generate(Some(Engine::Snowflake), &body, None);
        assert!(snowflake.sql.contains("\"id\" NUMBER(38,0) AUTOINCREMENT NOT NULL"), "{}", snowflake.sql);
        assert!(snowflake.sql.contains("COMMENT ON TABLE \"users\" IS 'People';"), "{}", snowflake.sql);

        let bigquery = generate(Some(Engine::Bigquery), &shop(), None);
        assert!(bigquery.sql.contains("PRIMARY KEY (`id`) NOT ENFORCED"), "{}", bigquery.sql);
        assert!(bigquery.sql.contains("FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) NOT ENFORCED;"), "{}", bigquery.sql);
    }

    #[test]
    fn composite_keys_list_every_pair() {
        let mut body = DiagramBody {
            tables: vec![
                table("a", "parent", vec![col("x", "int", true), col("y", "int", true)]),
                table("b", "child", vec![col("id", "int", true), col("px", "integer", false), col("py", "int4", false)]),
            ],
            relations: vec![relation("r", ("b", "px"), ("a", "x"))],
            ..DiagramBody::default()
        };
        if let Some(rel) = body.relations.first_mut() {
            rel.extra_columns = vec![DiagramColumnPair { from_column: "py".into(), to_column: "y".into() }];
            rel.name = Some("child_parent".into());
        }
        let ddl = generate(Some(Engine::Postgres), &body, None);
        assert!(ddl.sql.contains("ADD CONSTRAINT \"child_parent\" FOREIGN KEY (\"px\", \"py\") REFERENCES \"parent\" (\"x\", \"y\");"), "{}", ddl.sql);
        assert!(ddl.warnings.is_empty(), "int aliases must not warn: {:?}", ddl.warnings);
    }

    #[test]
    fn broken_relations_are_skipped_with_a_reason() {
        let mut body = shop();
        body.relations.push(relation("blank", ("o", ""), ("u", "id")));
        body.relations.push(relation("ghost", ("o", "nope"), ("u", "id")));
        body.relations.push(relation("gone", ("o", "user_id"), ("missing", "id")));
        let ddl = generate(Some(Engine::Postgres), &body, None);
        assert_eq!(ddl.sql.matches("ADD CONSTRAINT").count(), 1, "{}", ddl.sql);
        assert!(ddl.warnings.iter().any(|w| w.contains("no columns chosen")));
        assert!(ddl.warnings.iter().any(|w| w.contains("does not exist")));
        assert!(ddl.warnings.iter().any(|w| w.contains("missing or cannot be written")));
    }

    #[test]
    fn a_key_that_is_not_unique_or_has_another_type_warns_but_is_written() {
        let mut body = shop();
        if let Some(rel) = body.relations.first_mut() {
            rel.to_column = "email".into();
        }
        let ddl = generate(Some(Engine::Postgres), &body, None);
        assert!(ddl.sql.contains("REFERENCES \"users\" (\"email\")"));
        assert!(ddl.warnings.iter().any(|w| w.contains("not a primary key or unique")), "{:?}", ddl.warnings);
        assert!(ddl.warnings.iter().any(|w| w.contains("types differ")), "{:?}", ddl.warnings);
    }

    #[test]
    fn unnamed_and_empty_tables_are_skipped_and_untyped_columns_get_a_fallback() {
        let body = DiagramBody {
            tables: vec![
                table("a", "  ", vec![col("id", "int", true)]),
                table("b", "empty", vec![]),
                table("c", "typed", vec![col("note", "", false)]),
            ],
            ..DiagramBody::default()
        };
        let ddl = generate(None, &body, None);
        assert_eq!(ddl.sql, "CREATE TABLE \"typed\" (\n  \"note\" text\n);");
        assert_eq!(ddl.warnings.len(), 3, "{:?}", ddl.warnings);
    }

    #[test]
    fn the_per_table_preview_holds_only_that_table_and_the_keys_it_owns() {
        let body = shop();
        let child = generate(Some(Engine::Postgres), &body, Some("o"));
        assert!(child.sql.contains("CREATE TABLE \"orders\""));
        assert!(child.sql.contains("ADD CONSTRAINT"));
        assert!(!child.sql.contains("CREATE TABLE \"users\""));
        let parent = generate(Some(Engine::Postgres), &body, Some("u"));
        assert!(parent.sql.contains("CREATE TABLE \"users\"") && !parent.sql.contains("ADD CONSTRAINT"));
        assert!(!generate(Some(Engine::Postgres), &body, Some("zzz")).warnings.is_empty());
    }

    #[test]
    fn constraint_names_stay_unique_and_within_the_limit() {
        let mut body = shop();
        body.relations.push(relation("r2", ("o", "user_id"), ("u", "id")));
        let ddl = generate(Some(Engine::Postgres), &body, None);
        assert!(ddl.sql.contains("\"fk_orders_user_id\"") && ddl.sql.contains("\"fk_orders_user_id_2\""), "{}", ddl.sql);
        let mut long = shop();
        if let Some(rel) = long.relations.first_mut() {
            rel.name = Some("x".repeat(80));
        }
        let ddl = generate(Some(Engine::Postgres), &long, None);
        assert!(ddl.sql.contains(&format!("\"{}\"", "x".repeat(63))));
        assert!(ddl.warnings.iter().any(|w| w.contains("shortened")));
    }

    #[test]
    fn comments_become_comment_on_statements_where_the_dialect_has_them() {
        let mut body = shop();
        if let Some(users) = body.tables.first_mut() {
            users.comment = Some("who".into());
            if let Some(email) = users.columns.get_mut(1) {
                email.comment = Some("login".into());
                email.unique = true;
                email.default_value = Some("''".into());
            }
        }
        let ddl = generate(Some(Engine::Postgres), &body, Some("u"));
        assert!(ddl.sql.contains("\"email\" text DEFAULT '' UNIQUE"), "{}", ddl.sql);
        assert!(ddl.sql.contains("COMMENT ON TABLE \"users\" IS 'who';"));
        assert!(ddl.sql.contains("COMMENT ON COLUMN \"users\".\"email\" IS 'login';"));
        let ms = generate(Some(Engine::Mssql), &body, Some("u"));
        assert!(!ms.sql.contains("COMMENT"));
        assert!(ms.warnings.iter().any(|w| w.contains("comments")));
    }

    #[test]
    fn an_empty_diagram_is_an_empty_script() {
        let ddl = generate(Some(Engine::Postgres), &DiagramBody::default(), None);
        assert!(ddl.sql.is_empty() && ddl.warnings.is_empty());
    }
}
