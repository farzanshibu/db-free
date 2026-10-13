// SOT: sql-clause-builder, where-clause, order-clause, quote-literal, filter-sql, filter-mode, like-escaping, filter-family-honours-mode

use crate::error::{AppError, AppResult};
use crate::integrations::quote_ident_for;
use crate::model::{ColumnInfo, Engine, Family, FilterMode, FilterOp, FilterRule, SortRule};
use std::future::Future;

tokio::task_local! {
    /// How the filter rules of the page being fetched combine (see `with_filter_mode`).
    static FILTER_MODE: FilterMode;
}

// WHAT:  Runs `fut` with `mode` visible to `where_clause` (and `local::page`) inside it.
// WHY:   Every SQL adapter builds its WHERE through `where_clause(engine, &filters)`,
//        and `PageQuery` is built by every adapter's tests and sub-queries, so a new
//        field would touch them all. A task-local carries the mode to the one
//        builder that needs it, the way the run id reaches the adapters that cancel.
// HOW:   The data service sets it around `fetch_page` / `count`; absent = All.
// WHERE: src-tauri/src/services/data.rs
pub async fn with_filter_mode<F: Future>(mode: FilterMode, fut: F) -> F::Output {
    FILTER_MODE.scope(mode, fut).await
}

/// `with_filter_mode` for code that is not async (describing the SQL for the history log).
pub fn with_filter_mode_sync<R>(mode: FilterMode, f: impl FnOnce() -> R) -> R {
    FILTER_MODE.sync_scope(mode, f)
}

/// The mode the data service set for this task; `All` outside one.
pub fn current_filter_mode() -> FilterMode {
    FILTER_MODE.try_with(|mode| *mode).unwrap_or_default()
}

// WHAT:  Whether this family's adapter joins the rules with OR in "any" mode: it
//        builds its WHERE with `where_clause`, or its own translation reads
//        `current_filter_mode` (MongoDB's `$or`, Elasticsearch's `should`).
// WHY:   The other adapters translate a rule at a time into their own language
//        (Cypher, CQL, JPQL, Flux…) and only know AND; for those the data service
//        filters a window of rows itself.
// HOW:   Listed by family because capability flags do not say it: Cassandra, OrientDB
//        and QLDB speak SQL-like text but build it themselves.
pub fn honours_filter_mode(family: Family) -> bool {
    matches!(
        family,
        Family::Mongodb
            | Family::Elasticsearch
            | Family::Postgres
            | Family::Mysql
            | Family::Mssql
            | Family::Oracle
            | Family::Sqlite
            | Family::Duckdb
            | Family::Clickhouse
            | Family::Druid
            | Family::Snowflake
            | Family::Bigquery
            | Family::Libsql
            | Family::ValTown
            | Family::CloudflareD1
            | Family::Immudb
    )
}

// WHAT:  Builds WHERE / ORDER BY fragments for the table browser.
// WHY:   Both SQL engines share the syntax; one builder means one set of tests.
// HOW:   Identifiers are double-quoted and checked against the table's columns;
//        values are single-quoted with '' escaping so user text can never
//        terminate the literal. Comparisons cast to TEXT only for the LIKE family.
// WHERE: src-tauri/src/services/data.rs (caller), src-tauri/src/integrations/{postgres,sqlite}.rs
pub fn quote_literal(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "''"))
}

/// Engines whose string literals treat a backslash as an escape character.
fn backslash_escapes(engine: Engine) -> bool {
    matches!(engine.family(), Family::Mysql | Family::Clickhouse | Family::Bigquery | Family::Snowflake)
}

// WHAT:  A string literal that cannot be broken out of on this engine.
// WHY:   `quote_literal` only doubles the quote. MySQL, ClickHouse, BigQuery and
//        Snowflake read `\'` as an escaped quote, so `x\' OR 1=1 -- ` would end
//        the literal after the doubled quote. They get backslashes and quotes
//        backslash-escaped instead, which is also what keeps `C:\path` intact.
pub fn quote_literal_for(engine: Engine, raw: &str) -> String {
    if backslash_escapes(engine) {
        format!("'{}'", raw.replace('\\', "\\\\").replace('\'', "\\'"))
    } else {
        quote_literal(raw)
    }
}

pub fn validate_columns(columns: &[ColumnInfo], sort: &[SortRule], filters: &[FilterRule]) -> AppResult<()> {
    let known = |name: &str| columns.iter().any(|c| c.name == name);
    for rule in sort {
        if !known(&rule.column) {
            return Err(AppError::invalid_input(format!("Unknown sort column \"{}\".", rule.column)));
        }
    }
    for rule in filters {
        if !known(&rule.column) {
            return Err(AppError::invalid_input(format!("Unknown filter column \"{}\".", rule.column)));
        }
        if rule.op.needs_value() && rule.value.trim().is_empty() {
            return Err(AppError::invalid_input(format!("Filter on \"{}\" needs a value.", rule.column)));
        }
    }
    Ok(())
}

pub fn where_clause(engine: Engine, filters: &[FilterRule]) -> String {
    if filters.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = filters.iter().map(|f| predicate(engine, f)).collect();
    match current_filter_mode() {
        FilterMode::Any if parts.len() > 1 => format!(" WHERE ({})", parts.join(" OR ")),
        _ => format!(" WHERE {}", parts.join(" AND ")),
    }
}

pub fn order_clause(engine: Engine, sort: &[SortRule]) -> String {
    if sort.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = sort
        .iter()
        .map(|s| format!("{}{}", quote_ident_for(engine, &s.column), if s.desc { " DESC" } else { " ASC" }))
        .collect();
    format!(" ORDER BY {}", parts.join(", "))
}

/// The character `LIKE` patterns escape with where the engine has an `ESCAPE` clause.
const LIKE_ESCAPE: char = '!';

// WHAT:  The right-hand side of a LIKE: `prefix` + the value with its wildcards
//        neutralised + `suffix`, and the ESCAPE clause that goes with it.
// WHY:   "contains 100%" must find the text `100%`, not everything starting 100.
//        Engines without an ESCAPE clause (ClickHouse, BigQuery) read a backslash
//        as the escape inside the pattern itself.
// HOW:   Without a wildcard, backslash or escape character in the value the
//        pattern is exactly what it was before this existed.
fn like_operand(engine: Engine, value: &str, prefix: &str, suffix: &str) -> String {
    if matches!(engine.family(), Family::Clickhouse | Family::Bigquery) {
        let escaped = value.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        return quote_literal_for(engine, &format!("{prefix}{escaped}{suffix}"));
    }
    if value.contains(['%', '_', '\\', LIKE_ESCAPE]) {
        let mut escaped = String::with_capacity(value.len() + 4);
        for c in value.chars() {
            if matches!(c, '%' | '_' | LIKE_ESCAPE) {
                escaped.push(LIKE_ESCAPE);
            }
            escaped.push(c);
        }
        return format!("{} ESCAPE '{LIKE_ESCAPE}'", quote_literal_for(engine, &format!("{prefix}{escaped}{suffix}")));
    }
    quote_literal_for(engine, &format!("{prefix}{value}{suffix}"))
}

fn predicate(engine: Engine, rule: &FilterRule) -> String {
    let col = quote_ident_for(engine, &rule.column);
    // MySQL has no TEXT cast target; CHAR is the portable spelling there.
    let text_type = match engine.family() {
        Family::Mysql => "CHAR",
        Family::Mssql => "NVARCHAR(MAX)",
        Family::Oracle => "VARCHAR2(4000)",
        // ClickHouse type names are case-sensitive: STRING is not a known data
        // type family there, only String.
        Family::Clickhouse => "String",
        Family::Bigquery | Family::Snowflake | Family::Duckdb | Family::Druid => "STRING",
        _ => "TEXT",
    };
    let text_col = format!("CAST({col} AS {text_type})");
    // Postgres and ClickHouse have a case-sensitive LIKE and a separate ILIKE;
    // the other engines here already match case-insensitively.
    let like = match engine.family() {
        Family::Postgres | Family::Duckdb | Family::Snowflake | Family::Cassandra | Family::Clickhouse => "ILIKE",
        _ => "LIKE",
    };
    let value = rule.value.trim();
    let lit = |raw: &str| quote_literal_for(engine, raw);
    match rule.op {
        FilterOp::Eq => format!("{col} = {}", lit(value)),
        FilterOp::Ne => format!("{col} <> {}", lit(value)),
        FilterOp::Gt => format!("{col} > {}", lit(value)),
        FilterOp::Gte => format!("{col} >= {}", lit(value)),
        FilterOp::Lt => format!("{col} < {}", lit(value)),
        FilterOp::Lte => format!("{col} <= {}", lit(value)),
        FilterOp::Contains => format!("{text_col} {like} {}", like_operand(engine, value, "%", "%")),
        // A NULL is not a value that contains the text, so it stays in the result.
        FilterOp::NotContains => format!("({col} IS NULL OR {text_col} NOT {like} {})", like_operand(engine, value, "%", "%")),
        FilterOp::StartsWith => format!("{text_col} {like} {}", like_operand(engine, value, "", "%")),
        FilterOp::EndsWith => format!("{text_col} {like} {}", like_operand(engine, value, "%", "")),
        FilterOp::In => {
            let items: Vec<String> = value
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(lit)
                .collect();
            if items.is_empty() {
                "FALSE".to_string()
            } else {
                format!("{col} IN ({})", items.join(", "))
            }
        }
        FilterOp::IsNull => format!("{col} IS NULL"),
        FilterOp::IsNotNull => format!("{col} IS NOT NULL"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(column: &str, op: FilterOp, value: &str) -> FilterRule {
        FilterRule { column: column.into(), op, value: value.into() }
    }

    #[test]
    fn literals_cannot_escape() {
        assert_eq!(quote_literal("it's"), "'it''s'");
        let w = where_clause(Engine::Sqlite, &[rule("name", FilterOp::Eq, "x'; DROP TABLE t; --")]);
        assert_eq!(w, " WHERE \"name\" = 'x''; DROP TABLE t; --'");
    }

    #[test]
    fn backslash_engines_escape_the_backslash_too() {
        // On MySQL `\'` is an escaped quote, so doubling the quote alone would let
        // this value close the literal early.
        assert_eq!(quote_literal_for(Engine::Mysql, "x\\' OR 1=1 -- "), "'x\\\\\\' OR 1=1 -- '");
        assert_eq!(quote_literal_for(Engine::Clickhouse, "C:\\tmp"), "'C:\\\\tmp'");
        assert_eq!(quote_literal_for(Engine::Postgres, "C:\\tmp"), "'C:\\tmp'");
        assert_eq!(quote_literal_for(Engine::Sqlite, "it's"), "'it''s'");
        let w = where_clause(Engine::Mysql, &[rule("name", FilterOp::Eq, "a\\")]);
        assert_eq!(w, " WHERE `name` = 'a\\\\'");
    }

    #[test]
    fn operators_render() {
        let w = where_clause(
            Engine::Postgres,
            &[
                rule("id", FilterOp::Gte, "5"),
                rule("email", FilterOp::Contains, "@x"),
                rule("tier", FilterOp::In, "gold, basic"),
                rule("note", FilterOp::IsNull, ""),
            ],
        );
        assert_eq!(
            w,
            " WHERE \"id\" >= '5' AND CAST(\"email\" AS TEXT) ILIKE '%@x%' AND \"tier\" IN ('gold', 'basic') AND \"note\" IS NULL"
        );
        assert_eq!(order_clause(Engine::Postgres, &[SortRule { column: "a".into(), desc: true }, SortRule { column: "b".into(), desc: false }]), " ORDER BY \"a\" DESC, \"b\" ASC");
        assert_eq!(order_clause(Engine::Mysql, &[SortRule { column: "a".into(), desc: false }]), " ORDER BY `a` ASC");
        assert_eq!(where_clause(Engine::Mysql, &[rule("name", FilterOp::Contains, "x")]), " WHERE CAST(`name` AS CHAR) LIKE '%x%'");
        assert_eq!(where_clause(Engine::Sqlite, &[]), "");
    }

    #[test]
    fn not_contains_keeps_nulls() {
        assert_eq!(
            where_clause(Engine::Postgres, &[rule("note", FilterOp::NotContains, "spam")]),
            " WHERE (\"note\" IS NULL OR CAST(\"note\" AS TEXT) NOT ILIKE '%spam%')"
        );
        assert_eq!(
            where_clause(Engine::Sqlite, &[rule("note", FilterOp::NotContains, "spam")]),
            " WHERE (\"note\" IS NULL OR CAST(\"note\" AS TEXT) NOT LIKE '%spam%')"
        );
    }

    #[test]
    fn like_wildcards_in_the_value_are_literal() {
        assert_eq!(
            where_clause(Engine::Postgres, &[rule("p", FilterOp::Contains, "100%")]),
            " WHERE CAST(\"p\" AS TEXT) ILIKE '%100!%%' ESCAPE '!'"
        );
        assert_eq!(
            where_clause(Engine::Sqlite, &[rule("p", FilterOp::StartsWith, "a_b")]),
            " WHERE CAST(\"p\" AS TEXT) LIKE 'a!_b%' ESCAPE '!'"
        );
        // No ESCAPE clause on ClickHouse: the backslash is the escape inside the pattern.
        assert_eq!(
            where_clause(Engine::Clickhouse, &[rule("p", FilterOp::EndsWith, "x_")]),
            " WHERE CAST(\"p\" AS String) ILIKE '%x\\\\_'"
        );
        // MySQL's default escape is a backslash, so a backslash in the value needs the clause too.
        assert_eq!(
            where_clause(Engine::Mysql, &[rule("p", FilterOp::Contains, "a\\b")]),
            " WHERE CAST(`p` AS CHAR) LIKE '%a\\\\b%' ESCAPE '!'"
        );
    }

    #[tokio::test]
    async fn any_mode_joins_with_or_inside_parentheses() {
        let rules = [rule("a", FilterOp::Eq, "1"), rule("b", FilterOp::IsNull, "")];
        let any = with_filter_mode(FilterMode::Any, async { where_clause(Engine::Sqlite, &rules) }).await;
        assert_eq!(any, " WHERE (\"a\" = '1' OR \"b\" IS NULL)");
        let all = with_filter_mode(FilterMode::All, async { where_clause(Engine::Sqlite, &rules) }).await;
        assert_eq!(all, " WHERE \"a\" = '1' AND \"b\" IS NULL");
        // A single rule needs no group, and no mode set means All.
        let one = with_filter_mode(FilterMode::Any, async { where_clause(Engine::Sqlite, &rules[..1]) }).await;
        assert_eq!(one, " WHERE \"a\" = '1'");
        assert_eq!(where_clause(Engine::Sqlite, &rules), all);
    }

    #[test]
    fn only_families_that_build_with_where_clause_honour_any() {
        assert!(honours_filter_mode(Family::Postgres) && honours_filter_mode(Family::Sqlite) && honours_filter_mode(Family::Clickhouse));
        assert!(honours_filter_mode(Family::Mongodb) && honours_filter_mode(Family::Elasticsearch));
        // Their own translators know AND only.
        assert!(!honours_filter_mode(Family::Cassandra) && !honours_filter_mode(Family::Orientdb) && !honours_filter_mode(Family::Qldb));
        assert!(!honours_filter_mode(Family::Neo4j) && !honours_filter_mode(Family::Redis));
    }

    #[test]
    fn unknown_columns_rejected() {
        let cols = vec![ColumnInfo { name: "id".into(), data_type: "int".into(), nullable: false, primary_key: true, ordinal: 1 }];
        assert!(validate_columns(&cols, &[SortRule { column: "nope".into(), desc: false }], &[]).is_err());
        assert!(validate_columns(&cols, &[], &[rule("id", FilterOp::Eq, "  ")]).is_err());
        assert!(validate_columns(&cols, &[], &[rule("id", FilterOp::IsNull, "")]).is_ok());
    }
}
