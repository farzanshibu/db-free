// SOT: shell-meta-influxdb, influx-cli-bucket-list, influxql-show-statements

use super::args::{self, eq_ci, split_word, unquote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// INFLUXDB
//
// WHAT:  `influx bucket list [--name NAME]`, the InfluxDB 2 command line's
//        listing of buckets, and `help`; the InfluxQL `SHOW …` statements that
//        the 1.x shell uses for structure are listed as native.
// WHY:   The `influx` v1 shell has no structure meta-commands (`use`, `format`,
//        `precision` set client state), and the `SHOW` statements it takes are
//        InfluxQL the server runs itself. On a 2.x server the question "which
//        buckets are there" is the CLI's `influx bucket list`, which Flux
//        answers with `buckets()`.
// HOW:   `influx bucket list` becomes the Flux `buckets()` call, which the
//        adapter recognises by its prefix and sends to the Flux endpoint
//        (src-tauri/src/integrations/influxdb.rs, `classify` / `run_command`).
//        A 1.x server has no Flux, so there the statement fails with the
//        server's own message and `SHOW DATABASES` is the way. The optional
//        bucket name goes into a Flux string, so it is refused if it holds a
//        quote, a backslash, a `$` (Flux interpolates `${…}`) or a control
//        character.
// WHERE: src-tauri/src/integrations/shell_meta/mod.rs (`Dialect`)
// ============================================================================

pub const DIALECT: Dialect = Dialect {
    prefix: None,
    strict: false,
    help: &["help"],
    cut: Cut::Statements { line_words: &["influx", "help"] },
    commands: COMMANDS,
    plan,
};

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("influx bucket list", "influx bucket list [--name NAME]", "List the buckets of the organization (InfluxDB 2, answered with Flux buckets())"),
    ShellCommand::shell("help", "help", "List the commands this console understands"),
    ShellCommand::native("SHOW DATABASES", "SHOW DATABASES", "List the databases (InfluxDB 1)"),
    ShellCommand::native("SHOW MEASUREMENTS", "SHOW MEASUREMENTS [ON db]", "List the measurements"),
    ShellCommand::native("SHOW SERIES", "SHOW SERIES [FROM measurement]", "List the series"),
    ShellCommand::native("SHOW TAG KEYS", "SHOW TAG KEYS [FROM measurement]", "List the tag keys"),
    ShellCommand::native("SHOW TAG VALUES", "SHOW TAG VALUES FROM measurement WITH KEY = \"key\"", "List the values of a tag"),
    ShellCommand::native("SHOW FIELD KEYS", "SHOW FIELD KEYS [FROM measurement]", "List the field keys and their types"),
    ShellCommand::native("SHOW RETENTION POLICIES", "SHOW RETENTION POLICIES [ON db]", "List the retention policies"),
    ShellCommand::native("buckets()", "buckets()", "List the buckets (Flux)"),
    ShellCommand::native("import \"influxdata/influxdb/schema\"", "import \"influxdata/influxdb/schema\"\nschema.measurements(bucket: \"bucket\")", "List the measurements of a bucket (Flux)"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    let (head, rest) = split_word(text);
    if eq_ci(head, "help") && rest.is_empty() {
        return Some(Ok(help_plan(&DIALECT)));
    }
    eq_ci(head, "influx").then(|| influx(rest))
}

fn influx(rest: &str) -> AppResult<Plan> {
    let words = args::raw_args(rest).map_err(AppError::invalid_input)?;
    let words: Vec<String> = words.iter().map(|w| unquote(w)).collect();
    let mut words = words.iter().map(String::as_str);
    match (words.next(), words.next()) {
        (Some(a), Some(b)) if a.eq_ignore_ascii_case("bucket") && (b.eq_ignore_ascii_case("list") || b.eq_ignore_ascii_case("ls")) => bucket_list(&words.collect::<Vec<_>>()),
        _ => Err(AppError::invalid_input("DB Free reads `influx bucket list [--name NAME]`; type InfluxQL (SHOW MEASUREMENTS…) or Flux for the rest.")),
    }
}

// WHAT:  `buckets()`, or `buckets(name: "NAME")` for `--name NAME` / `-n NAME`.
fn bucket_list(flags: &[&str]) -> AppResult<Plan> {
    let usage = || AppError::invalid_input("influx bucket list reads --name NAME only.");
    match flags {
        [] => Ok(Plan::native("buckets()")),
        [flag, value] if matches!(*flag, "--name" | "-n") => named(value),
        [flag] => match flag.strip_prefix("--name=") {
            Some(value) => named(&unquote(value)),
            None => Err(usage()),
        },
        _ => Err(usage()),
    }
}

fn named(name: &str) -> AppResult<Plan> {
    if name.is_empty() || name.chars().any(|c| matches!(c, '"' | '\\' | '$') || c.is_control()) {
        return Err(AppError::invalid_input("A bucket name cannot be empty or hold a quote, a backslash, a $ or a control character."));
    }
    Ok(Plan::native(format!("buckets(name: \"{name}\")")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::shell_meta::{plan as run_plan, segments, Segment, Step};
    use crate::model::Engine;

    fn planned(text: &str) -> Option<AppResult<Plan>> {
        run_plan(&Ctx::bare(Engine::Influxdb), text)
    }

    fn statements(text: &str) -> Vec<String> {
        match planned(text) {
            Some(Ok(plan)) => plan.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn error(text: &str) -> String {
        match planned(text) {
            Some(Err(err)) => err.message().to_string(),
            other => panic!("{text}: expected an error, got {other:?}"),
        }
    }

    #[test]
    fn bucket_list_becomes_the_flux_buckets_call() {
        assert_eq!(statements("influx bucket list"), ["buckets()"]);
        assert_eq!(statements("INFLUX Bucket LS;"), ["buckets()"]);
        assert_eq!(statements("influx bucket list --name telegraf"), ["buckets(name: \"telegraf\")"]);
        assert_eq!(statements("influx bucket list -n telegraf"), ["buckets(name: \"telegraf\")"]);
        assert_eq!(statements("influx bucket list --name=\"my bucket\""), ["buckets(name: \"my bucket\")"]);
    }

    #[test]
    fn the_flux_the_adapter_receives_is_recognised_as_flux() {
        // influxdb.rs `classify`: Flux starts with `buckets(`.
        for text in ["influx bucket list", "influx bucket list --name b"] {
            assert!(statements(text)[0].starts_with("buckets("), "{text}");
        }
    }

    #[test]
    fn other_influx_commands_are_refused_with_a_pointer() {
        assert!(error("influx").contains("influx bucket list"));
        assert!(error("influx org list").contains("influx bucket list"));
        assert!(error("influx bucket create -n x").contains("influx bucket list"));
        assert!(error("influx bucket list --org acme").contains("--name NAME only"));
        assert!(error("influx bucket list --name").contains("--name NAME only"));
        assert!(error("influx bucket list --name a b").contains("--name NAME only"));
    }

    #[test]
    fn a_bucket_name_cannot_break_out_of_the_flux_string() {
        for hostile in ["a\"b", "a\\b", "${x}", "a$b", "a\nb"] {
            let text = format!("influx bucket list --name '{hostile}'");
            assert!(matches!(planned(&text), Some(Err(_))), "{hostile}");
        }
        assert!(error("influx bucket list --name ''").contains("cannot be empty"));
    }

    #[test]
    fn influxql_and_flux_are_left_for_the_server() {
        for text in ["SHOW DATABASES", "SHOW MEASUREMENTS", "SELECT * FROM cpu LIMIT 5", "buckets()", "from(bucket: \"b\") |> range(start: -1h)", "help me", "influxdb"] {
            assert!(planned(text).is_none(), "{text}");
        }
    }

    #[test]
    fn help_lists_every_command() {
        for text in ["help", "HELP;"] {
            let Some(Ok(plan)) = planned(text) else { panic!("{text} is not understood") };
            assert!(matches!(plan.steps.as_slice(), [Step::Table(table)] if table.rows.len() == COMMANDS.len()), "{text}");
        }
    }

    #[test]
    fn influx_commands_are_cut_at_their_line() {
        let found = segments(Engine::Influxdb, "influx bucket list\nSHOW MEASUREMENTS;");
        assert_eq!(found, vec![Segment::Meta("influx bucket list".to_string()), Segment::Native("SHOW MEASUREMENTS".to_string())]);
    }
}
