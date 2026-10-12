// SOT: shell-meta-rocksdb, rocksdb-ldb-list-column-families, rocksdb-structure-commands

use super::args::{eq_ci, strip_terminator};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// ROCKSDB
//
// WHAT:  `list_column_families`, the `ldb` tool's command for the one piece of
//        structure a RocksDB directory has, and a `help` listing of what the tab
//        itself runs.
// WHY:   RocksDB is a key-value library with no schema and no query language.
//        Its command-line tool, `ldb`, takes the directory as `--db=…` and a
//        sub-command: `list_column_families`, `get`, `scan`, `dump`,
//        `manifest_dump`… The adapter's tab already speaks a small language of
//        its own (`CF`, `KEYS`, `SCAN`, `GET`, with `--cf <name>`:
//        the adapter's `parse_command`), and those are left alone. Only the one `ldb`
//        sub-command that names the structure is worth translating; the others
//        either read data (`get`, `scan`) or need files the library does not
//        expose here.
// HOW:   `list_column_families` becomes `CF`, the adapter's own listing (column
//        family names with their estimated key counts). `help` returns the table.
//        Both statements are reads.
// WHERE: src-tauri/src/integrations/rocksdb.rs (`parse_command`, `run_command`),
//        src-tauri/src/integrations/shell_meta/mod.rs
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::shell("list_column_families", "list_column_families", "The ldb command: column families with their estimated key counts"),
    ShellCommand::native("CF", "CF", "Column families with their estimated key counts"),
    ShellCommand::native("KEYS", "KEYS [prefix] [limit]", "Keys of the default column family, optionally under a prefix"),
    ShellCommand::native("SCAN", "SCAN [prefix] [limit]", "Keys with their values and sizes"),
    ShellCommand::native("GET", "GET key", "One key's value"),
    ShellCommand::native("--cf", "--cf name KEYS|SCAN|GET ...", "Run KEYS, SCAN or GET in another column family"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let text = strip_terminator(text);
    if eq_ci(text, "help") {
        return Some(Ok(help_plan(&DIALECT)));
    }
    eq_ci(text, "list_column_families").then(|| Ok(Plan::native("CF")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::rocksdb::{parse_command, parse_script, Command};
    use crate::integrations::shell_meta::{CommandOrigin, Step};
    use crate::model::Engine;

    fn run(text: &str) -> Option<AppResult<Plan>> {
        plan(&Ctx::bare(Engine::Rocksdb), text)
    }

    #[test]
    fn help_lists_every_command() {
        for text in ["help", "HELP", "help;"] {
            let Some(Ok(plan)) = run(text) else { panic!("{text}") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn the_ldb_listing_becomes_the_adapters_own_listing() {
        for text in ["list_column_families", "LIST_COLUMN_FAMILIES", "List_Column_Families;"] {
            let Some(Ok(plan)) = run(text) else { panic!("{text}") };
            assert_eq!(plan.steps, vec![Step::Native("CF".to_string())], "{text}");
        }
        // What it sends is a command the adapter parses, and a read.
        assert_eq!(parse_command("CF").ok(), Some(Command::ListCfs));
    }

    #[test]
    fn anything_else_is_left_for_the_adapter() {
        for command in COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(run(command.name).is_none(), "{}", command.name);
        }
        for text in ["GET help", "list_column_families --db=/x", "KEYS", "PUT k v", "--cf meta SCAN", "list_column_families x"] {
            assert!(run(text).is_none(), "{text}");
        }
    }

    #[test]
    fn the_native_commands_listed_parse_and_none_of_them_writes() {
        for syntax in ["CF", "KEYS", "SCAN", "GET k", "--cf meta SCAN", "--cf meta KEYS", "--cf meta GET k"] {
            let commands = parse_script(syntax).unwrap_or_default();
            assert_eq!(commands.len(), 1, "{syntax}");
            assert!(!matches!(commands[0], Command::Put { .. } | Command::Delete { .. }), "{syntax}");
        }
    }
}
