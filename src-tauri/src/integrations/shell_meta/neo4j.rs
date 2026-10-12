// SOT: shell-meta-neo4j, cypher-shell-colon-commands, neo4j-schema-command, memgraph-schema-command

use super::args::{split_word, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};
use crate::model::Engine;

// ============================================================================
// NEO4J / MEMGRAPH
//
// WHAT:  The colon commands of cypher-shell and Neo4j Browser that describe a
//        database: `:schema` (indexes and constraints), `:sysinfo` (version and
//        databases) and `:help`; plus a listing of the Cypher statements that
//        answer the rest (`CALL db.labels()`, `SHOW INDEXES`…).
// WHY:   Cypher has no meta-commands of its own; `:schema` and friends belong to
//        the clients and the server would reject them as a syntax error. What a
//        user wants from them is a catalog read, and that is plain Cypher on
//        Neo4j and a differently spelled one on Memgraph.
// HOW:   Each command becomes one or two Cypher statements the adapter runs as
//        typed (`neo4j::split_statements`, `execute`). The engine picks the
//        spelling: Neo4j `SHOW INDEXES` / `SHOW CONSTRAINTS` (4.3 and later),
//        Memgraph `SHOW INDEX INFO` / `SHOW CONSTRAINT INFO`; `:sysinfo` is
//        `CALL dbms.components()` plus `SHOW DATABASES` on Neo4j and
//        `SHOW STORAGE INFO` on Memgraph. The commands take no arguments, so no
//        user text reaches a statement at all. The colon is the dialect's prefix
//        but not a strict one: a Cypher line can legitimately begin with `:`
//        (a label on the second line of a pattern), so an unrecognised `:foo`
//        is left to the server rather than refused.
// WHERE: src-tauri/src/integrations/neo4j.rs (`execute`, `catalog_rows`: the
//        same statements feed the object explorer), src-tauri/src/integrations/shell_meta/mod.rs
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: Some(":"), strict: false, help: &[":help"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell(":help", ":help", "List the commands this console understands"),
    ShellCommand::shell(":schema", ":schema", "Indexes and constraints (SHOW INDEXES and SHOW CONSTRAINTS; Memgraph: SHOW INDEX INFO and SHOW CONSTRAINT INFO)"),
    ShellCommand::shell(":sysinfo", ":sysinfo", "The server's components and databases (Memgraph: its storage info)"),
    ShellCommand::native("CALL db.labels()", "CALL db.labels()", "Node labels in use"),
    ShellCommand::native("CALL db.relationshipTypes()", "CALL db.relationshipTypes()", "Relationship types in use"),
    ShellCommand::native("CALL db.propertyKeys()", "CALL db.propertyKeys()", "Property keys in use"),
    ShellCommand::native("CALL db.schema.visualization()", "CALL db.schema.visualization()", "The graph's schema as nodes and relationships"),
    ShellCommand::native("CALL db.schema.nodeTypeProperties()", "CALL db.schema.nodeTypeProperties()", "Properties and types per node label"),
    ShellCommand::native("CALL db.schema.relTypeProperties()", "CALL db.schema.relTypeProperties()", "Properties and types per relationship type"),
    ShellCommand::native("SHOW INDEXES", "SHOW INDEXES [YIELD ...]", "Neo4j: indexes with their state and type"),
    ShellCommand::native("SHOW CONSTRAINTS", "SHOW CONSTRAINTS [YIELD ...]", "Neo4j: constraints"),
    ShellCommand::native("SHOW DATABASES", "SHOW DATABASES", "Databases of the server"),
    ShellCommand::native("SHOW PROCEDURES", "SHOW PROCEDURES", "Neo4j: callable procedures"),
    ShellCommand::native("SHOW FUNCTIONS", "SHOW FUNCTIONS", "Neo4j: available functions"),
    ShellCommand::native("SHOW USERS", "SHOW USERS", "Database users"),
    ShellCommand::native("SHOW ROLES", "SHOW ROLES", "Roles (Neo4j Enterprise, Memgraph)"),
    ShellCommand::native("SHOW INDEX INFO", "SHOW INDEX INFO", "Memgraph: indexes"),
    ShellCommand::native("SHOW CONSTRAINT INFO", "SHOW CONSTRAINT INFO", "Memgraph: constraints"),
    ShellCommand::native("SHOW STORAGE INFO", "SHOW STORAGE INFO", "Memgraph: storage figures"),
];

fn plan(ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let text = strip_terminator(text);
    let (head, rest) = split_word(text);
    let command = head.strip_prefix(':')?.to_ascii_lowercase();
    let memgraph = ctx.engine == Engine::Memgraph;
    let found = match command.as_str() {
        "help" => help_plan(&DIALECT),
        "schema" if memgraph => Plan::natives(["SHOW INDEX INFO", "SHOW CONSTRAINT INFO"]),
        "schema" => Plan::natives(["SHOW INDEXES", "SHOW CONSTRAINTS"]),
        "sysinfo" if memgraph => Plan::native("SHOW STORAGE INFO"),
        "sysinfo" => Plan::natives(["CALL dbms.components() YIELD name, versions, edition RETURN name, versions, edition", "SHOW DATABASES"]),
        _ => return None,
    };
    if !rest.is_empty() {
        return Some(Err(AppError::invalid_input(format!("`:{command}` takes no argument."))));
    }
    Some(Ok(found))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{segments, CommandOrigin, Segment, Step};

    fn run(engine: Engine, text: &str) -> Option<AppResult<Plan>> {
        plan(&Ctx::bare(engine), text)
    }

    fn statements(engine: Engine, text: &str) -> Vec<String> {
        match run(engine, text) {
            Some(Ok(plan)) => plan.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn schema_is_the_engines_own_index_and_constraint_listing() {
        assert_eq!(statements(Engine::Neo4j, ":schema"), ["SHOW INDEXES", "SHOW CONSTRAINTS"]);
        assert_eq!(statements(Engine::Neo4j, ":SCHEMA;"), ["SHOW INDEXES", "SHOW CONSTRAINTS"]);
        assert_eq!(statements(Engine::Memgraph, ":schema"), ["SHOW INDEX INFO", "SHOW CONSTRAINT INFO"]);
    }

    #[test]
    fn sysinfo_reports_version_and_databases() {
        let neo = statements(Engine::Neo4j, ":sysinfo");
        assert_eq!(neo.len(), 2);
        assert!(neo[0].starts_with("CALL dbms.components()"));
        assert_eq!(neo[1], "SHOW DATABASES");
        assert_eq!(statements(Engine::Memgraph, ":sysinfo"), ["SHOW STORAGE INFO"]);
    }

    #[test]
    fn help_lists_every_command() {
        for text in [":help", ":HELP", ":Help;"] {
            let Some(Ok(plan)) = run(Engine::Neo4j, text) else { panic!("{text}") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn arguments_are_refused_so_no_user_text_reaches_a_statement() {
        for text in [":schema Person", ":sysinfo MATCH (n) DETACH DELETE n", ":help schema", ":schema 'x'"] {
            assert!(matches!(run(Engine::Neo4j, text), Some(Err(AppError::InvalidInput { .. }))), "{text}");
        }
    }

    #[test]
    fn cypher_that_starts_with_a_colon_is_not_a_command() {
        for text in [":Person)-[:KNOWS]->(b)", ":use neo4j", ":param x => 1", ":", ":schemas", "MATCH (n) RETURN n", "SHOW INDEXES"] {
            assert!(run(Engine::Neo4j, text).is_none(), "{text}");
        }
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(run(Engine::Neo4j, command.name).is_none(), "{}", command.name);
        }
    }

    #[test]
    fn a_script_keeps_cypher_and_commands_in_order() {
        let script = "MATCH (n:Person)\nRETURN n\n:schema\nCALL db.labels()";
        assert_eq!(
            segments(Engine::Neo4j, script),
            vec![
                Segment::Native("MATCH (n:Person)\nRETURN n".to_string()),
                Segment::Meta(":schema".to_string()),
                Segment::Native("CALL db.labels()".to_string()),
            ]
        );
        // A pattern that continues on a line starting with a label stays whole.
        let pattern = "MATCH (a)-[r\n:KNOWS]->(b) RETURN a";
        assert_eq!(segments(Engine::Neo4j, pattern), vec![Segment::Native(pattern.to_string())]);
    }

    #[test]
    fn nothing_emitted_mutates() {
        // The adapter's own mutation check (`looks_mutating`) is private; these are the keywords it looks for.
        let mutating = ["create", "merge", "delete", "set", "remove", "drop", "detach", "load", "alter", "grant", "revoke", "deny"];
        for engine in [Engine::Neo4j, Engine::Memgraph] {
            for text in [":schema", ":sysinfo"] {
                for statement in statements(engine, text) {
                    let words: Vec<String> = statement.split(|c: char| !c.is_alphanumeric() && c != '_').map(str::to_ascii_lowercase).collect();
                    assert!(!words.iter().any(|w| mutating.contains(&w.as_str())), "{statement}");
                }
            }
        }
    }
}
