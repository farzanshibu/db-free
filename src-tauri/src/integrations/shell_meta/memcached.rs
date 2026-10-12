// SOT: shell-meta-memcached, memcached-text-protocol-help, memcached-structure-commands

use super::args::{eq_ci, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// MEMCACHED
//
// WHAT:  A `help` listing of the text-protocol commands that describe a memcached
//        server: `stats` and its sections, `version`, the key dump.
// WHY:   Memcached has no shell. People reach it with `telnet host 11211` or
//        `nc` and type the protocol itself, and the adapter's tab is that same
//        console: one raw protocol command per line (`memcached::parse_script`,
//        `execute`). So every structure command is already native and is left
//        alone; what is missing is a place that says which ones exist, and the
//        protocol has no `help` (the server answers `ERROR`).
// HOW:   `help` returns the table built from `COMMANDS`. No translation, no
//        statements: nothing here reaches the server. The listed commands are
//        the read-only ones; the adapter refuses the writers (`set`, `delete`,
//        `flush_all`, `slabs`, `lru`…) itself on a read-only connection.
// WHERE: src-tauri/src/integrations/memcached.rs (`parse_script`, `execute`,
//        `is_write_command`), src-tauri/src/integrations/shell_meta/mod.rs
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::native("stats", "stats", "General-purpose statistics: uptime, connections, hits, memory"),
    ShellCommand::native("stats items", "stats items", "Per slab class item counts, ages and evictions"),
    ShellCommand::native("stats slabs", "stats slabs", "Per slab class chunk sizes and memory use"),
    ShellCommand::native("stats sizes", "stats sizes", "Histogram of item sizes (briefly blocks the server on large caches)"),
    ShellCommand::native("stats settings", "stats settings", "The settings the server runs with"),
    ShellCommand::native("stats conns", "stats conns", "Open client connections"),
    ShellCommand::native("stats cachedump", "stats cachedump slab_id limit", "Keys held in one slab class (removed in some builds)"),
    ShellCommand::native("lru_crawler metadump", "lru_crawler metadump all", "Every key with its expiry, last access and size"),
    ShellCommand::native("version", "version", "The server version"),
    ShellCommand::native("get", "get key [key ...]", "Read one or more values"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    eq_ci(strip_terminator(text), "help").then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::memcached::parse_script;
    use crate::integrations::shell_meta::{CommandOrigin, Step};
    use crate::model::Engine;

    fn run(text: &str) -> Option<AppResult<Plan>> {
        plan(&Ctx::bare(Engine::Memcached), text)
    }

    #[test]
    fn help_lists_every_command() {
        for text in ["help", "HELP", " Help ", "help;"] {
            let Some(Ok(plan)) = run(text) else { panic!("{text}") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn protocol_commands_are_not_intercepted() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(run(command.name).is_none(), "{}", command.name);
        }
        for text in ["stats", "stats items", "get help", "version", "helpful", "help me"] {
            assert!(run(text).is_none(), "{text}");
        }
    }

    #[test]
    fn every_listed_command_is_a_read_line_the_adapter_accepts() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            let line = command.syntax.split(" [").next().unwrap_or(command.syntax);
            let parsed = parse_script(line).unwrap_or_default();
            assert_eq!(parsed.len(), 1, "{line}");
            // The adapter refuses these on a read-only connection; none is listed.
            let verb = line.split_whitespace().next().unwrap_or_default();
            let writers = ["set", "add", "replace", "append", "prepend", "cas", "delete", "incr", "decr", "touch", "flush_all", "ms", "md", "ma", "verbosity", "slabs", "lru"];
            assert!(!writers.contains(&verb), "{verb} writes");
        }
    }
}
