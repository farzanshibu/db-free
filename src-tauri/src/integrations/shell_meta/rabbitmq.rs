// SOT: shell-meta-rabbitmq, rabbitmqctl-list-commands, rabbitmqadmin-list-commands

use super::args::{eq_ci, raw_args, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// RABBITMQ
//
// WHAT:  The structure commands of RabbitMQ's command lines: `rabbitmqctl
//        list_queues`, `rabbitmqctl list_exchanges`, `rabbitmqadmin list queues`
//        and `rabbitmqadmin list exchanges`.
// WHY:   RabbitMQ operators list queues and exchanges with these commands; they
//        are pasted from runbooks together with options of their own shell, so the
//        options that only choose a node, a vhost, a format or a timeout are
//        accepted and the connection answers.
// HOW:   `list_queues` / `list queues` become the console's `QUEUES` shorthand and
//        `list_exchanges` / `list exchanges` become `EXCHANGES`; `parse_command` in
//        integrations/rabbitmq.rs reads both and `queues_result` / `exchanges_result`
//        list every vhost of the broker with a vhost column (a superset of one
//        vhost's listing) and every column of the management API (a superset of the
//        columns a command names, which are accepted and ignored).
//        `rabbitmqctl` subcommands the management client has no counterpart for
//        (bindings, vhosts, users, connections, channels, consumers) are refused
//        with the list of what is translated.
// WHERE: src-tauri/src/integrations/rabbitmq.rs (`parse_command`, `queues_result`, `exchanges_result`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | ?", "List the commands this console understands"),
    ShellCommand::shell("rabbitmqctl list_queues", "rabbitmqctl list_queues [-p <vhost>] [<column> ...]", "Every queue with its message and consumer counts (all vhosts; -p and the columns are ignored)"),
    ShellCommand::shell("rabbitmqctl list_exchanges", "rabbitmqctl list_exchanges [-p <vhost>] [<column> ...]", "Every exchange with its type and durability (all vhosts; -p and the columns are ignored)"),
    ShellCommand::shell("rabbitmqadmin list queues", "rabbitmqadmin list queues [<column> ...]", "Every queue with its message and consumer counts (all vhosts; the columns are ignored)"),
    ShellCommand::shell("rabbitmqadmin list exchanges", "rabbitmqadmin list exchanges [<column> ...]", "Every exchange with its type and durability (all vhosts; the columns are ignored)"),
    ShellCommand::native("QUEUES", "QUEUES", "Every queue with its message and consumer counts"),
    ShellCommand::native("EXCHANGES", "EXCHANGES", "Every exchange with its type and durability"),
    ShellCommand::native("CONSUME", "CONSUME <queue> [n]", "Peeks at the messages of a queue without removing them"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    if DIALECT.help.iter().any(|h| eq_ci(h, text)) {
        return Some(Ok(help_plan(&DIALECT)));
    }
    let (head, rest) = split_word(text);
    // Runbooks spell the program with its directory: `/usr/sbin/rabbitmqctl`, `sbin/rabbitmqadmin`.
    let head = head.rsplit(['/', '\\']).next().unwrap_or(head).to_ascii_lowercase();
    match head.as_str() {
        "rabbitmqctl" => Some(ctl(rest)),
        "rabbitmqadmin" => Some(admin(rest)),
        _ => None,
    }
}

/// Options that take a value in `rabbitmqctl` and `rabbitmqadmin`, none of which changes what the console lists.
const VALUE_OPTIONS: &[&str] = &[
    "-p", "--vhost", "-V", "-n", "--node", "--formatter", "-t", "--timeout", "-f", "--format", "-H", "--host", "-P", "--port", "-u", "--username", "--password",
];
/// Options that stand alone.
const FLAG_OPTIONS: &[&str] = &["-q", "--quiet", "-s", "--silent", "--no-table-headers", "-l", "--longnames", "--offline", "--online", "--local"];

/// The words of a command line after its program name: `(subcommand, columns)`.
fn subcommand(rest: &str) -> AppResult<(String, Vec<String>)> {
    let tokens = raw_args(rest).map_err(AppError::invalid_input)?;
    let mut words: Vec<String> = Vec::new();
    let mut iter = tokens.iter();
    while let Some(token) = iter.next() {
        let name = token.split('=').next().unwrap_or(token);
        if token.starts_with('-') {
            if VALUE_OPTIONS.contains(&name) {
                if !token.contains('=') {
                    iter.next();
                }
            } else if !FLAG_OPTIONS.contains(&name) {
                return Err(AppError::invalid_input(format!("The option {name} is not translated; the console lists queues and exchanges with all their columns.")));
            }
        } else {
            words.push(token.clone());
        }
    }
    let mut words = words.into_iter();
    let verb = words.next().unwrap_or_default();
    Ok((verb, words.collect()))
}

fn ctl(rest: &str) -> AppResult<Plan> {
    let (verb, columns) = subcommand(rest)?;
    list_for(verb.to_ascii_lowercase().as_str(), &columns)
}

fn admin(rest: &str) -> AppResult<Plan> {
    let (verb, mut words) = subcommand(rest)?;
    if !eq_ci(&verb, "list") || words.is_empty() {
        return Err(AppError::invalid_input("Supported: rabbitmqadmin list queues, rabbitmqadmin list exchanges."));
    }
    let what = words.remove(0).to_ascii_lowercase();
    list_for(&format!("list_{what}"), &words)
}

fn list_for(verb: &str, columns: &[String]) -> AppResult<Plan> {
    let statement = match verb {
        "list_queues" => "QUEUES",
        "list_exchanges" => "EXCHANGES",
        _ => return Err(AppError::invalid_input("Supported: list_queues and list_exchanges (rabbitmqadmin: list queues, list exchanges). The Objects panel shows bindings, vhosts and users.")),
    };
    // Column names are words like `name` or `messages_ready`; anything else is not a column.
    if let Some(bad) = columns.iter().find(|c| !c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')) {
        return Err(AppError::invalid_input(format!("{bad} is not a column name.")));
    }
    Ok(Plan::native(statement))
}

#[cfg(test)]
mod tests {
    use super::super::{plan, CommandOrigin, Ctx, Step};
    use crate::model::Engine;

    fn statements(text: &str) -> Vec<String> {
        match plan(&Ctx::bare(Engine::Rabbitmq), text) {
            Some(Ok(p)) => p.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn refused(text: &str) -> String {
        match plan(&Ctx::bare(Engine::Rabbitmq), text) {
            Some(Err(e)) => e.message().to_string(),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn help_lists_every_command() {
        for word in ["help", "?", "Help;"] {
            let found = plan(&Ctx::bare(Engine::Rabbitmq), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}");
        }
    }

    #[test]
    fn rabbitmqctl_listings_run_the_consoles_shorthands() {
        assert_eq!(statements("rabbitmqctl list_queues"), ["QUEUES"]);
        assert_eq!(statements("RabbitMQCtl LIST_EXCHANGES;"), ["EXCHANGES"]);
        assert_eq!(statements("rabbitmqctl list_queues -p / name messages consumers"), ["QUEUES"]);
        assert_eq!(statements("rabbitmqctl -q list_queues --vhost=/prod --formatter json"), ["QUEUES"]);
        assert_eq!(statements("rabbitmqctl list_exchanges -n rabbit@host name type"), ["EXCHANGES"]);
        assert_eq!(statements("/usr/sbin/rabbitmqctl list_queues"), ["QUEUES"]);
        assert_eq!(statements("sbin/rabbitmqadmin list exchanges"), ["EXCHANGES"]);
    }

    #[test]
    fn rabbitmqadmin_listings_run_the_consoles_shorthands() {
        assert_eq!(statements("rabbitmqadmin list queues"), ["QUEUES"]);
        assert_eq!(statements("rabbitmqadmin list exchanges name type"), ["EXCHANGES"]);
        assert_eq!(statements("rabbitmqadmin list queues name messages_ready"), ["QUEUES"]);
    }

    #[test]
    fn what_the_management_client_cannot_list_is_refused_with_what_it_can() {
        for text in ["rabbitmqctl list_bindings", "rabbitmqctl list_vhosts", "rabbitmqctl list_users", "rabbitmqctl list_connections", "rabbitmqctl", "rabbitmqadmin list bindings"] {
            assert!(refused(text).contains("Supported"), "{text}");
        }
        assert!(refused("rabbitmqadmin show queue").contains("Supported"));
        assert!(refused("rabbitmqctl delete_queue q").contains("Supported"));
        assert!(refused("rabbitmqctl list_queues --evil 1").contains("--evil"));
        assert!(refused("rabbitmqctl list_queues 'name; DROP").contains("quote"));
        assert!(refused("rabbitmqctl list_queues name;x").contains("not a column"));
    }

    #[test]
    fn natives_and_json_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Rabbitmq), native.name).is_none(), "{}", native.name);
        }
        for text in ["QUEUES", "CONSUME orders 5", "{\"queue\": \"orders\"}", "rabbitmqctlx list_queues", "/usr/sbin/rabbitmqctlx list_queues"] {
            assert!(plan(&Ctx::bare(Engine::Rabbitmq), text).is_none(), "{text}");
        }
    }
}
