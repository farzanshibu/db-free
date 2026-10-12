// SOT: shell-meta-sparql, sparql-console-help, sparql-structure-queries

use super::args::eq_ci;
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// SPARQL (Jena Fuseki, GraphDB, Stardog, Blazegraph, Virtuoso)
//
// WHAT:  The structure commands of the SPARQL console: a `help` listing of the
//        queries that show named graphs, classes, predicates and counts.
// WHY:   A triple store has no schema to list. Its structure is asked for with
//        SPARQL itself (`SELECT DISTINCT ?class WHERE { ?s a ?class }`), and the
//        vendors' command lines (`stardog-admin db list`, GraphDB's workbench)
//        talk to admin APIs that the SPARQL protocol does not carry, so there is
//        no shell command this console could run verbatim.
// HOW:   Only `help` and `?` are intercepted: a bare word is not a SPARQL token
//        (a variable starts with `?` or `$`, a name needs a prefix or `<...>`), so
//        the line can never belong to a query. The queries are `ShellCommand::native`
//        entries; `query_kind` in integrations/sparql.rs sends SELECT and ASK to the
//        query endpoint as typed.
// WHERE: src-tauri/src/integrations/sparql.rs (`execute`, `query_kind`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | ?", "List the queries that show the structure of this store"),
    ShellCommand::native(
        "SELECT DISTINCT ?graph WHERE { GRAPH ?graph { ?s ?p ?o } }",
        "SELECT DISTINCT ?graph WHERE { GRAPH ?graph { ?s ?p ?o } }",
        "The named graphs that hold triples",
    ),
    ShellCommand::native(
        "SELECT ?class (COUNT(?s) AS ?instances) WHERE { ?s a ?class } GROUP BY ?class ORDER BY DESC(?instances)",
        "SELECT ?class (COUNT(?s) AS ?instances) WHERE { ?s a ?class } GROUP BY ?class ORDER BY DESC(?instances)",
        "The rdf:type classes with their number of instances",
    ),
    ShellCommand::native(
        "SELECT ?predicate (COUNT(*) AS ?uses) WHERE { ?s ?predicate ?o } GROUP BY ?predicate ORDER BY DESC(?uses)",
        "SELECT ?predicate (COUNT(*) AS ?uses) WHERE { ?s ?predicate ?o } GROUP BY ?predicate ORDER BY DESC(?uses)",
        "The predicates with how often each is used",
    ),
    ShellCommand::native(
        "SELECT DISTINCT ?predicate WHERE { ?s a <class-iri> ; ?predicate ?o }",
        "SELECT DISTINCT ?predicate WHERE { ?s a <class-iri> ; ?predicate ?o }",
        "The predicates used by the instances of one class",
    ),
    ShellCommand::native("SELECT (COUNT(*) AS ?triples) WHERE { ?s ?p ?o }", "SELECT (COUNT(*) AS ?triples) WHERE { ?s ?p ?o }", "Number of triples in the default graph"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    DIALECT.help.iter().any(|h| eq_ci(h, text)).then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::super::{plan, CommandOrigin, Ctx, Step};
    use crate::model::Engine;

    const ENGINES: [Engine; 5] = [Engine::ApacheJena, Engine::Graphdb, Engine::Stardog, Engine::Blazegraph, Engine::Virtuoso];

    #[test]
    fn help_is_translated_for_every_store_and_lists_every_query() {
        for engine in ENGINES {
            for word in ["help", "HELP", "?", "help;"] {
                let found = plan(&Ctx::bare(engine), word);
                assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}: {found:?}");
            }
        }
    }

    #[test]
    fn queries_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::ApacheJena), native.name).is_none(), "{}", native.name);
        }
        for text in ["ASK { ?s ?p ?o }", "PREFIX ex: <http://example.org/>", "  ?s ?p ?o .", "DESCRIBE <http://example.org/a>"] {
            assert!(plan(&Ctx::bare(Engine::Stardog), text).is_none(), "{text}");
        }
    }
}
