// SOT: shell-meta-kafka, kafka-topics-command, redpanda-rpk-topic-list

use super::args::{eq_ci, raw_args, split_word};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// KAFKA / REDPANDA
//
// WHAT:  The structure commands of the Kafka tools: `kafka-topics --list` (also
//        spelled `kafka-topics.sh` and `kafka-topics.bat`) and Redpanda's
//        `rpk topic list` / `rpk topic ls`.
// WHY:   Kafka people list topics with these commands. They are pasted from
//        runbooks together with the connection flags of another shell, so the
//        flags that only say where to connect are accepted and the connected
//        cluster answers.
// HOW:   Both become the console's `TOPICS` shorthand: `parse_command` in
//        integrations/kafka.rs reads it and `topics_result` returns each topic with
//        its partition count and record total (a superset of what `--list` prints).
//        `--bootstrap-server`, `--command-config` and `--exclude-internal` are accepted and
//        ignored (the shorthand lists internal topics such as `__consumer_offsets` too);
//        `--topic` (a filter the shorthand does not have) is refused.
//        `--describe` needs partition leaders and replicas, which the client
//        library behind this connection (rskafka) does not expose, so it is refused
//        with a pointer to what does answer; the other kafka-topics actions create,
//        change or delete topics and are not translated. Consumer-group commands
//        (`kafka-consumer-groups`, `kafka-configs`) have no counterpart in the
//        adapter and are sent on as typed, so the console says what it understands.
// WHERE: src-tauri/src/integrations/kafka.rs (`parse_command`, `topics_result`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | ?", "List the commands this console understands"),
    ShellCommand::shell("kafka-topics --list", "kafka-topics[.sh] [--bootstrap-server <servers>] --list [--exclude-internal]", "Every topic with its partitions and record count (--exclude-internal is ignored: internal topics are listed)"),
    ShellCommand::shell("rpk topic list", "rpk topic list | rpk topic ls", "Redpanda: every topic with its partitions and record count"),
    ShellCommand::native("TOPICS", "TOPICS", "Every topic with its partitions and record count"),
    ShellCommand::native("CONSUME", "CONSUME <topic> [n]", "The latest records of a topic, to see their keys and values"),
    ShellCommand::native("{\"topic\": \"<topic>\", \"offset\": \"earliest\", \"limit\": 10}", "{\"topic\": \"<topic>\", \"offset\": \"earliest\", \"limit\": 10}", "Records of a topic from the start"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    if DIALECT.help.iter().any(|h| eq_ci(h, text)) {
        return Some(Ok(help_plan(&DIALECT)));
    }
    let (head, rest) = split_word(text);
    // Runbooks spell the program with its directory: `bin/kafka-topics.sh`, `bin\windows\kafka-topics.bat`.
    let program = head.rsplit(['/', '\\']).next().unwrap_or(head);
    if ["kafka-topics", "kafka-topics.sh", "kafka-topics.bat"].iter().any(|n| eq_ci(n, program)) {
        return Some(kafka_topics(rest));
    }
    if eq_ci(head, "rpk") {
        let (group, tail) = split_word(rest);
        let (verb, extra) = split_word(tail);
        if eq_ci(group, "topic") && (eq_ci(verb, "list") || eq_ci(verb, "ls")) {
            return Some(if extra.is_empty() {
                Ok(Plan::native("TOPICS"))
            } else {
                Err(AppError::invalid_input(format!("rpk topic list does not take {extra}; the grid filters the topics.")))
            });
        }
    }
    None
}

fn kafka_topics(rest: &str) -> AppResult<Plan> {
    let tokens = raw_args(rest).map_err(AppError::invalid_input)?;
    let mut list = false;
    let mut describe = false;
    let mut topic = false;
    let mut iter = tokens.iter();
    while let Some(token) = iter.next() {
        let name = token.split('=').next().unwrap_or(token);
        match name {
            "--list" => list = true,
            "--describe" => describe = true,
            // Where to connect and how to authenticate: the connection already says.
            "--bootstrap-server" | "--bootstrap-servers" | "--command-config" => {
                if !token.contains('=') {
                    iter.next();
                }
            }
            "--topic" => {
                topic = true;
                if !token.contains('=') {
                    iter.next();
                }
            }
            "--exclude-internal" => {}
            other => {
                return Err(AppError::invalid_input(format!(
                    "{other} is not translated: this console runs kafka-topics --list. Create, change and delete topics from the Objects panel."
                )))
            }
        }
    }
    if describe {
        return Err(AppError::invalid_input(
            "kafka-topics --describe needs the partition leaders and replicas, which this connection does not expose. kafka-topics --list shows each topic's partitions and record count, and the Objects panel lists a topic's partitions.",
        ));
    }
    if !list {
        return Err(AppError::invalid_input("Add --list: kafka-topics --list shows the topics."));
    }
    if topic {
        return Err(AppError::invalid_input("kafka-topics --list --topic <pattern> is not translated: TOPICS has no filter. Filter the topic column of the result instead."));
    }
    Ok(Plan::native("TOPICS"))
}

#[cfg(test)]
mod tests {
    use super::super::{plan, CommandOrigin, Ctx, Step};
    use crate::model::Engine;

    fn statements(text: &str) -> Vec<String> {
        match plan(&Ctx::bare(Engine::Kafka), text) {
            Some(Ok(p)) => p.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn refused(text: &str) -> String {
        match plan(&Ctx::bare(Engine::Redpanda), text) {
            Some(Err(e)) => e.message().to_string(),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn help_lists_every_command() {
        for word in ["help", "?", "HELP;"] {
            let found = plan(&Ctx::bare(Engine::Kafka), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}");
        }
    }

    #[test]
    fn listing_topics_runs_the_topics_shorthand_however_it_is_spelled() {
        for text in [
            "kafka-topics --list",
            "kafka-topics.sh --list",
            "KAFKA-TOPICS.BAT --list",
            "kafka-topics.sh --bootstrap-server localhost:9092 --list",
            "kafka-topics.sh --list --bootstrap-server=broker-1:9092,broker-2:9092",
            "kafka-topics --command-config client.properties --bootstrap-server b:9093 --list --exclude-internal",
            "kafka-topics.sh --list --bootstrap-servers b:9092",
            "bin/kafka-topics.sh --bootstrap-server b:9092 --list",
            "/opt/kafka/bin/kafka-topics.sh --list",
            "bin\\windows\\kafka-topics.bat --list",
            "rpk topic list",
            "RPK Topic ls;",
        ] {
            assert_eq!(statements(text), ["TOPICS"], "{text}");
        }
    }

    #[test]
    fn describe_is_refused_with_what_to_use_instead() {
        let message = refused("kafka-topics.sh --bootstrap-server b:9092 --describe --topic orders");
        assert!(message.contains("--list") && message.contains("replicas"), "{message}");
    }

    #[test]
    fn other_actions_and_missing_actions_are_refused() {
        assert!(refused("kafka-topics --create --topic t").contains("--create is not translated"));
        assert!(refused("kafka-topics --delete --topic t").contains("--delete is not translated"));
        assert!(refused("kafka-topics --bootstrap-server b:9092").contains("--list"));
        assert!(refused("kafka-topics --list --topic 'x").contains("quote"));
        assert!(refused("kafka-topics --list --topic orders.*").contains("no filter"));
        assert!(refused("rpk topic list orders.*").contains("does not take"));
    }

    #[test]
    fn natives_and_other_tools_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Kafka), native.name).is_none(), "{}", native.name);
        }
        for text in ["TOPICS", "CONSUME orders 5", "kafka-consumer-groups --list", "rpk group list", "rpk topic describe orders", "kafka-topicsx --list", "bin/kafka-topicsx.sh --list"] {
            assert!(plan(&Ctx::bare(Engine::Kafka), text).is_none(), "{text}");
        }
    }
}
