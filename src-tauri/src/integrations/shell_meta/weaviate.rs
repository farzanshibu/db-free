// SOT: shell-meta-weaviate, weaviate-console-help

use super::args::eq_ci;
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// WEAVIATE
//
// WHAT:  The structure commands of Weaviate's console: a `help` listing of the
//        shorthands, REST envelopes and GraphQL documents that show classes,
//        shards, nodes and counts.
// WHY:   Weaviate is reached over REST and GraphQL and has no shell with
//        meta-commands, so there is nothing to translate. The structure calls are
//        native to the console and the help listing is how a user finds them.
// HOW:   Only `?` and `\?` are intercepted, never the bare word `help`: a GraphQL
//        document spread over lines lists one property per line, and a class can
//        have a property called `help`; cutting that line out would break the query.
//        `parse_command` in integrations/weaviate.rs reads `SCHEMA` / `CLASSES`,
//        `OBJECTS <class> [n]`, a `{"path": ...}` passthrough and GraphQL.
// WHERE: src-tauri/src/integrations/weaviate.rs (`parse_command`, `run`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["?", "\\?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("?", "? | \\?", "List the calls that show the structure of this server"),
    ShellCommand::native("SCHEMA", "SCHEMA | CLASSES", "Every class with its properties and vectorizer"),
    ShellCommand::native("OBJECTS", "OBJECTS <class> [n]", "The first objects of a class, to see their properties"),
    ShellCommand::native("{\"path\": \"/v1/schema/<class>\"}", "{\"path\": \"/v1/schema/<class>\"}", "The definition of one class"),
    ShellCommand::native("{\"path\": \"/v1/schema/<class>/shards\"}", "{\"path\": \"/v1/schema/<class>/shards\"}", "Shards of a class and their status"),
    ShellCommand::native("{\"path\": \"/v1/nodes?output=verbose\"}", "{\"path\": \"/v1/nodes?output=verbose\"}", "Nodes with shard and object counts"),
    ShellCommand::native("{\"path\": \"/v1/meta\"}", "{\"path\": \"/v1/meta\"}", "Server version and enabled modules"),
    ShellCommand::native("{ Aggregate { <Class> { meta { count } } } }", "{ Aggregate { <Class> { meta { count } } } }", "GraphQL: number of objects in a class"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    DIALECT.help.iter().any(|h| eq_ci(h, text)).then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::super::{plan, CommandOrigin, Ctx, Step};
    use crate::model::Engine;

    #[test]
    fn the_help_marks_are_translated_and_list_every_command() {
        for word in ["?", "\\?", "?;"] {
            let found = plan(&Ctx::bare(Engine::Weaviate), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}: {found:?}");
        }
    }

    #[test]
    fn a_graphql_property_called_help_is_never_cut_out_of_its_query() {
        let query = "{\n  Get {\n    Faq {\n      title\n      help\n    }\n  }\n}";
        for line in query.lines() {
            assert!(plan(&Ctx::bare(Engine::Weaviate), line.trim()).is_none(), "{line}");
        }
        assert!(super::super::segments(Engine::Weaviate, query).iter().all(|s| matches!(s, super::super::Segment::Native(_))));
    }

    #[test]
    fn natives_and_documents_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Weaviate), native.name).is_none(), "{}", native.name);
        }
        for text in ["SCHEMA", "OBJECTS Article 5", "{ Get { Article { title } } }"] {
            assert!(plan(&Ctx::bare(Engine::Weaviate), text).is_none(), "{text}");
        }
    }
}
