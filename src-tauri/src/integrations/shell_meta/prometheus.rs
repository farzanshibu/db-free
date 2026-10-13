// SOT: shell-meta-prometheus, prometheus-console-help, victoriametrics-console-help

use super::args::eq_ci;
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::AppResult;

// ============================================================================
// PROMETHEUS / VICTORIAMETRICS
//
// WHAT:  The structure commands of the Prometheus console: a `help` listing of the
//        shorthands and PromQL expressions that show label names, metric names and
//        series.
// WHY:   Prometheus and VictoriaMetrics have no interactive shell. `promtool query`
//        takes a server address and runs one query per invocation, which is what
//        this console already is, so there is nothing to translate. The structure
//        questions are answered by native shorthands and plain PromQL, and the help
//        listing is how a user finds them.
// HOW:   Only `?` and `\?` are intercepted, never the bare word `help`: in PromQL a
//        line holding only `help` is a valid selector of a metric of that name, and
//        a multi-line expression may hold such a line. `parse_command` in
//        integrations/prometheus.rs reads `LABELS`, `SERIES <selector>`,
//        `RANGE <query> <start> <end> <step>`, JSON and anything else as PromQL.
// WHERE: src-tauri/src/integrations/prometheus.rs (`parse_command`, `run_command`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["?", "\\?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("?", "? | \\?", "List the shorthands and queries that show the structure of this server"),
    ShellCommand::native("LABELS", "LABELS", "Every label name known to the server"),
    ShellCommand::native("SERIES", "SERIES <selector>", "The series a selector matches, e.g. SERIES up{job=\"api\"}"),
    ShellCommand::native("RANGE", "RANGE <query> <start> <end> <step>", "A range query, e.g. RANGE up -1h now 60s"),
    ShellCommand::native("count by (__name__) ({__name__=~\".+\"})", "count by (__name__) ({__name__=~\".+\"})", "PromQL: every metric name with its number of series"),
    ShellCommand::native("count by (job) (up)", "count by (job) (up)", "PromQL: targets per job"),
    ShellCommand::native("up", "up", "PromQL: 1 for each target that is up and 0 for each that is down"),
    ShellCommand::native("{__name__=~\"<prefix>.*\"}", "{__name__=~\"<prefix>.*\"}", "PromQL: the series of the metrics whose name starts with a prefix"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    DIALECT.help.iter().any(|h| eq_ci(h, text)).then(|| Ok(help_plan(&DIALECT)))
}

#[cfg(test)]
mod tests {
    use super::super::{plan, CommandOrigin, Ctx, Segment, Step};
    use crate::model::Engine;

    #[test]
    fn the_help_marks_are_translated_for_both_engines() {
        for engine in [Engine::Prometheus, Engine::Victoriametrics] {
            for word in ["?", "\\?", "?;"] {
                let found = plan(&Ctx::bare(engine), word);
                assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}: {found:?}");
            }
        }
    }

    #[test]
    fn a_metric_called_help_is_a_query_not_a_command() {
        assert!(plan(&Ctx::bare(Engine::Prometheus), "help").is_none());
        let expression = "sum(\n  help\n) by (job)";
        assert_eq!(super::super::segments(Engine::Prometheus, expression), vec![Segment::Native(expression.to_string())]);
    }

    #[test]
    fn shorthands_and_promql_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Prometheus), native.name).is_none(), "{}", native.name);
        }
        for text in ["LABELS", "SERIES up", "rate(http_requests_total[5m])", "{\"query\": \"up\"}"] {
            assert!(plan(&Ctx::bare(Engine::Victoriametrics), text).is_none(), "{text}");
        }
    }
}
