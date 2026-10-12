// SOT: shell-meta-redis, redis-cli-help, redis-command-docs, redis-structure-commands

use super::args::{eq_ci, keyword, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// REDIS / VALKEY / DRAGONFLY
//
// WHAT:  redis-cli's structure commands for the query tab: `help` lists what is
//        there to ask, and `help <command>` shows a command's documentation.
// WHY:   redis-cli has no catalog meta-commands to translate. Everything it
//        offers for structure (`INFO keyspace`, `DBSIZE`, `SCAN`, `TYPE`,
//        `CONFIG GET`, `CLIENT LIST`…) is an ordinary Redis command that the
//        adapter already sends, so those are listed as native and left alone.
//        What redis-cli does answer locally is `help`, from a table compiled
//        into the client; the server has no `HELP` command, so a typed `help`
//        would come back as "unknown command".
// HOW:   `help` and `?` return the command list as a local table. `help SET` and
//        `help CONFIG GET` run `COMMAND DOCS`, the server's own documentation
//        for a command (Redis 7 / Valkey 7.2 and later), so nothing is invented
//        client-side. The names are validated tokens: the adapter tokenises each
//        line like redis-cli (the adapter's `parse_commands`), so a name carrying a
//        quote or a space could become several arguments; only letters, digits
//        and `. _ -` pass. Every statement is a read by the adapter's own list
//        (`is_read_only_command`: COMMAND DOCS), which the tests check.
//        `help @group` is refused with the server command that lists a group
//        (`COMMAND LIST FILTERBY ACLCAT`): redis-cli's groups are its own
//        table, not the ACL categories.
// WHERE: src-tauri/src/integrations/redis.rs (`parse_commands`, `execute`),
//        src-tauri/src/integrations/shell_meta/mod.rs
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help [command [subcommand]]", "List the commands here, or show a command's documentation (Redis 7+)"),
    ShellCommand::shell("?", "?", "Same as help"),
    ShellCommand::native("INFO keyspace", "INFO [section]", "Databases with their key and expiry counts"),
    ShellCommand::native("INFO", "INFO [server|clients|memory|persistence|stats|replication|cpu|cluster|keyspace]", "Server figures by section"),
    ShellCommand::native("DBSIZE", "DBSIZE", "Number of keys in the selected database"),
    ShellCommand::native("SCAN", "SCAN cursor [MATCH pattern] [COUNT n] [TYPE type]", "Walk the keys a page at a time"),
    ShellCommand::native("KEYS", "KEYS pattern", "Every key matching a pattern (walks the whole keyspace)"),
    ShellCommand::native("TYPE", "TYPE key", "The data type stored at a key"),
    ShellCommand::native("TTL", "TTL key", "Seconds until a key expires"),
    ShellCommand::native("EXISTS", "EXISTS key [key ...]", "How many of the keys exist"),
    ShellCommand::native("OBJECT ENCODING", "OBJECT ENCODING key", "The internal encoding of a key"),
    ShellCommand::native("MEMORY USAGE", "MEMORY USAGE key [SAMPLES n]", "Bytes a key and its value occupy"),
    ShellCommand::native("XINFO STREAM", "XINFO STREAM key", "Structure of a stream"),
    ShellCommand::native("PUBSUB CHANNELS", "PUBSUB CHANNELS [pattern]", "Active pub/sub channels"),
    ShellCommand::native("CLIENT LIST", "CLIENT LIST", "Connected clients"),
    ShellCommand::native("CONFIG GET", "CONFIG GET pattern", "Configuration parameters matching a pattern"),
    ShellCommand::native("COMMAND COUNT", "COMMAND COUNT", "Number of commands the server knows"),
    ShellCommand::native("COMMAND INFO", "COMMAND INFO name [name ...]", "Arity, flags and key positions of commands"),
    ShellCommand::native("MODULE LIST", "MODULE LIST", "Loaded modules"),
    ShellCommand::native("FUNCTION LIST", "FUNCTION LIST", "Registered function libraries"),
    ShellCommand::native("ACL LIST", "ACL LIST", "Access-control rules"),
    ShellCommand::native("SLOWLOG GET", "SLOWLOG GET [count]", "Recent slow commands"),
    ShellCommand::native("CLUSTER NODES", "CLUSTER NODES", "The cluster's nodes and slots"),
    ShellCommand::native("ROLE", "ROLE", "Whether this server is a primary or a replica"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let text = strip_terminator(text);
    if text == "?" || eq_ci(text, "help") {
        return Some(Ok(help_plan(&DIALECT)));
    }
    keyword(text, "help").map(docs)
}

// WHAT:  `help SET` / `help CONFIG GET` → `COMMAND DOCS set` / `COMMAND DOCS config|get`.
fn docs(rest: &str) -> AppResult<Plan> {
    let names: Vec<&str> = rest.split_whitespace().collect();
    if names.iter().any(|n| n.starts_with('@')) {
        return Err(AppError::invalid_input("Groups come from the server: COMMAND LIST FILTERBY ACLCAT <category>, for example `COMMAND LIST FILTERBY ACLCAT hash`."));
    }
    match names.as_slice() {
        [command] if is_name(command) => Ok(Plan::native(format!("COMMAND DOCS {}", command.to_ascii_lowercase()))),
        [command, sub] if is_name(command) && is_name(sub) => Ok(Plan::native(format!("COMMAND DOCS {}|{}", command.to_ascii_lowercase(), sub.to_ascii_lowercase()))),
        _ => Err(AppError::invalid_input("Usage: help [command [subcommand]], for example `help SET` or `help CONFIG GET`.")),
    }
}

/// A command or sub-command name: letters, digits and `. _ -` (module commands are `JSON.GET`, `FT.SEARCH`).
fn is_name(word: &str) -> bool {
    !word.is_empty() && word.len() <= 64 && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::redis::{is_read_only_command, parse_commands};
    use crate::integrations::shell_meta::{segments, CommandOrigin, Segment, Step};
    use crate::model::Engine;

    fn run(text: &str) -> Option<AppResult<Plan>> {
        plan(&Ctx::bare(Engine::Redis), text)
    }

    fn statement(text: &str) -> String {
        match run(text) {
            Some(Ok(plan)) => match plan.steps.as_slice() {
                [Step::Native(statement)] => statement.clone(),
                other => panic!("{text}: expected one statement, got {other:?}"),
            },
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn help_and_question_mark_list_the_commands_in_any_case() {
        for text in ["help", "HELP", "Help", "?", "help;", "  help  "] {
            let Some(Ok(plan)) = run(text) else { panic!("{text} is not understood") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn help_with_a_command_asks_the_server_for_its_documentation() {
        assert_eq!(statement("help SET"), "COMMAND DOCS set");
        assert_eq!(statement("HELP json.get"), "COMMAND DOCS json.get");
        assert_eq!(statement("help CONFIG GET"), "COMMAND DOCS config|get");
        assert_eq!(statement("help client-list"), "COMMAND DOCS client-list");
    }

    #[test]
    fn what_help_sends_is_a_read_the_adapter_parses_as_one_command() {
        for text in ["help set", "help config get", "help JSON.GET", "help x_y"] {
            let sent = statement(text);
            let commands = parse_commands(&sent).unwrap_or_default();
            assert_eq!(commands.len(), 1, "{sent}");
            assert_eq!(commands[0].len(), 3, "{sent}: COMMAND DOCS <name> is three tokens");
            assert!(is_read_only_command(&commands[0]), "{sent}");
        }
    }

    #[test]
    fn a_hostile_command_name_never_reaches_the_server() {
        for text in [
            "help set\" FLUSHALL",
            "help set;FLUSHALL",
            "help a b c",
            "help \"x y\"",
            "help 'a'",
            "help `x`",
            "help $(x)",
            "help set,flushall",
        ] {
            let found = run(text);
            assert!(matches!(found, Some(Err(AppError::InvalidInput { .. }))), "{text:?}: {found:?}");
        }
        let group = run("help @string");
        assert!(matches!(&group, Some(Err(AppError::InvalidInput { message })) if message.contains("ACLCAT")), "{group:?}");
    }

    #[test]
    fn native_commands_are_left_for_the_server() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(run(command.name).is_none(), "{}", command.name);
        }
        for text in ["GET helper", "SET help 1", "HGETALL help", "helper", "INFO keyspace", "CONFIG GET maxmemory", "SELECT 1"] {
            assert!(run(text).is_none(), "{text}");
        }
    }

    #[test]
    fn a_script_keeps_commands_and_help_in_order() {
        let found = segments(Engine::Valkey, "DBSIZE\nhelp\nhelp set\nTYPE a");
        assert_eq!(
            found,
            vec![
                Segment::Native("DBSIZE".to_string()),
                Segment::Meta("help".to_string()),
                Segment::Meta("help set".to_string()),
                Segment::Native("TYPE a".to_string()),
            ]
        );
    }

    #[test]
    fn the_native_commands_listed_are_all_reads() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            let tokens: Vec<String> = command.name.split_whitespace().map(str::to_string).collect();
            assert!(is_read_only_command(&tokens), "{}", command.name);
        }
    }
}
