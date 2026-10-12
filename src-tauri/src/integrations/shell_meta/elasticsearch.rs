// SOT: shell-meta-elasticsearch, elasticsearch-console-help, opensearch-console-help

use super::args::eq_ci;
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// ELASTICSEARCH / OPENSEARCH
//
// WHAT:  The structure commands of the Elasticsearch family's console: a `help`
//        listing of the REST calls and SQL statements that answer "what indices,
//        aliases, shards, mappings and settings are there".
// WHY:   Elasticsearch and OpenSearch ship no interactive shell with meta-commands
//        (Kibana's Dev Tools and OpenSearch Dashboards send REST calls, and the SQL
//        CLI sends SQL), so there is nothing to translate. The structure calls are
//        already native to the console, and what a newcomer lacks is the list of them.
// HOW:   Nothing is intercepted but the help word. `GET /_cat/indices` and the
//        others are `ShellCommand::native` entries: `parse_command` in
//        integrations/elasticsearch.rs reads a REST verb line (and adds
//        `format=json` to `_cat` calls so the grid can show them), and
//        `SHOW TABLES` / `DESCRIBE` go to the SQL endpoint. The help word is `?`
//        (and `\?`), never the bare `help`: a SQL statement spread over lines can
//        hold a column called `help` on a line of its own, and a command line must
//        be text that cannot be a statement.
// WHERE: src-tauri/src/integrations/elasticsearch.rs (`parse_command`, `split_console`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["?", "\\?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("?", "? | \\?", "List the REST calls and SQL statements that show the structure"),
    ShellCommand::native("GET /_cat/indices", "GET /_cat/indices[?v&s=index]", "Indices with health, document count and size"),
    ShellCommand::native("GET /_cat/aliases", "GET /_cat/aliases", "Aliases and the indices they point to"),
    ShellCommand::native("GET /_cat/shards", "GET /_cat/shards[/<index>]", "Shards per index and the node holding each"),
    ShellCommand::native("GET /_cat/nodes", "GET /_cat/nodes", "Nodes of the cluster with their roles and load"),
    ShellCommand::native("GET /_cat/health", "GET /_cat/health", "Cluster health in one line"),
    ShellCommand::native("GET /_cat/templates", "GET /_cat/templates", "Index templates and the patterns they apply to"),
    ShellCommand::native("GET /_cat/count", "GET /_cat/count[/<index>]", "Document count of the cluster or of an index"),
    ShellCommand::native("GET /<index>/_mapping", "GET /<index>/_mapping", "Field mappings of an index"),
    ShellCommand::native("GET /<index>/_settings", "GET /<index>/_settings", "Settings of an index"),
    ShellCommand::native("GET /_cluster/health", "GET /_cluster/health", "Cluster status with shard counts"),
    ShellCommand::native("GET /_cluster/settings", "GET /_cluster/settings", "Persistent and transient cluster settings"),
    ShellCommand::native("GET /_aliases", "GET /_aliases", "Alias definitions grouped by index"),
    ShellCommand::native("GET /_index_template", "GET /_index_template[/<name>]", "Composable index templates"),
    ShellCommand::native("SHOW TABLES", "SHOW TABLES [LIKE '<pattern>']", "SQL: the indices (OpenSearch needs the LIKE clause)"),
    ShellCommand::native("DESCRIBE", "DESCRIBE <index> | DESCRIBE TABLES LIKE '<pattern>'", "SQL: the columns of an index (the second form is OpenSearch's)"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    DIALECT.help.iter().any(|h| eq_ci(h, text)).then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::super::{plan, Ctx, Step};
    use crate::model::Engine;

    #[test]
    fn only_the_help_word_is_translated_and_it_lists_every_command() {
        for engine in [Engine::Elasticsearch, Engine::Opensearch] {
            for word in ["?", "\\?", "?;"] {
                let found = plan(&Ctx::bare(engine), word);
                assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}: {found:?}");
            }
        }
    }

    #[test]
    fn a_bare_help_word_stays_text_because_it_can_be_a_column() {
        // `SELECT\n  help\nFROM docs` is valid SQL; the line `help` must never be cut out of it.
        assert!(plan(&Ctx::bare(Engine::Elasticsearch), "help").is_none());
        assert!(plan(&Ctx::bare(Engine::Elasticsearch), "HELP").is_none());
    }

    #[test]
    fn native_calls_and_statements_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == super::super::CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Elasticsearch), native.name).is_none(), "{}", native.name);
        }
        for text in ["GET /_cat/indices?v", "SELECT * FROM logs LIMIT 5", "{\"index\": \"logs\", \"query\": {\"match_all\": {}}}", "DESCRIBE logs", "POST /logs/_search"] {
            assert!(plan(&Ctx::bare(Engine::Opensearch), text).is_none(), "{text}");
        }
    }
}
