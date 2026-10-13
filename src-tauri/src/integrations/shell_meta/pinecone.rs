// SOT: shell-meta-pinecone, pinecone-cli-structure-commands

use super::args::{eq_ci, raw_args, split_word, unquote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};
use serde_json::json;

// ============================================================================
// PINECONE
//
// WHAT:  The structure commands of the Pinecone CLI (`pc`): `pc index list` and
//        `pc index describe --index-name <index>`.
// WHY:   People who manage Pinecone from a terminal type these; the query tab
//        answers them through the connection instead of failing with "Unknown
//        command".
// HOW:   `pc index list` runs the console's `INDEXES` shorthand (the control
//        plane's `GET /indexes`: name, dimension, metric, host, status).
//        `pc index describe` runs the raw envelope `{"method": "GET", "path":
//        "/indexes/<name>"}` with the name percent-encoded into one path segment
//        and the whole envelope built by `serde_json`, so a name can never leave its
//        segment or its string. `parse_command` in integrations/pinecone.rs reads
//        both; a path under `/indexes` goes to the control plane and, being a GET,
//        is never a mutation. The name is accepted as `--index-name <n>` / `-i <n>` (the
//        CLI's flags), their `=` forms, the deprecated `--name` / `-n` alias, or as the
//        one positional argument. `--json` / `-j` and, for the listing, `--wide` / `-w`
//        only choose the CLI's output format and are accepted and ignored. A name made of
//        dots alone is refused: the URL parser reads even `%2E%2E` as a `..` step, so it
//        could not be kept inside its segment.
// WHERE: src-tauri/src/integrations/pinecone.rs (`parse_command`, `run`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | ?", "List the commands this console understands"),
    ShellCommand::shell("pc index list", "pc index list [--wide] [--json]", "Every index with its dimension, metric, host and status (--wide and --json are ignored)"),
    ShellCommand::shell("pc index describe", "pc index describe --index-name <index> [--json]", "Spec, host and status of one index (--json is ignored)"),
    ShellCommand::native("INDEXES", "INDEXES", "Every index with its dimension, metric, host and status"),
    ShellCommand::native("STATS", "STATS <index>", "Vector counts per namespace and the dimension"),
    ShellCommand::native("LIST", "LIST <index> [n] [namespace]", "The first vector ids of an index, to see their metadata"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    if DIALECT.help.iter().any(|h| eq_ci(h, text)) {
        return Some(Ok(help_plan(&DIALECT)));
    }
    let (head, rest) = split_word(text);
    if !eq_ci(head, "pc") {
        return None;
    }
    let (group, rest) = split_word(rest);
    if !eq_ci(group, "index") {
        return Some(Err(AppError::invalid_input("Supported: pc index list, pc index describe --index-name <index>.")));
    }
    let (verb, rest) = split_word(rest);
    Some(match verb.to_ascii_lowercase().as_str() {
        "list" => list(rest),
        "describe" => describe(rest),
        _ => Err(AppError::invalid_input("Supported: pc index list, pc index describe --index-name <index>.")),
    })
}

fn list(rest: &str) -> AppResult<Plan> {
    let mut tokens = raw_args(rest).map_err(AppError::invalid_input)?;
    // Output format only: the grid is the answer.
    tokens.retain(|t| !matches!(t.as_str(), "--json" | "-j" | "--wide" | "-w"));
    match tokens.first() {
        None => Ok(Plan::native("INDEXES")),
        Some(extra) => Err(AppError::invalid_input(format!("pc index list does not take {extra}."))),
    }
}

/// The flags that name the index: the CLI's own, then its deprecated alias.
const NAME_FLAGS: &[&str] = &["-i", "--index-name", "-n", "--name"];

/// The value of `--index-name=<v>` / `--name=<v>`.
fn inline_value(token: &str) -> Option<&str> {
    NAME_FLAGS.iter().filter(|f| f.starts_with("--")).find_map(|f| token.strip_prefix(*f)?.strip_prefix('='))
}

fn describe(rest: &str) -> AppResult<Plan> {
    let mut tokens = raw_args(rest).map_err(AppError::invalid_input)?;
    // Output format only: the grid is the answer.
    tokens.retain(|t| !matches!(t.as_str(), "--json" | "-j"));
    let name = match tokens.iter().position(|t| NAME_FLAGS.contains(&t.as_str()) || inline_value(t).is_some()) {
        Some(at) => {
            let token = tokens.remove(at);
            match inline_value(&token) {
                Some(inline) => unquote(inline),
                None if at < tokens.len() => unquote(&tokens.remove(at)),
                None => return Err(AppError::invalid_input(format!("{token} needs a value."))),
            }
        }
        None if tokens.len() == 1 && !tokens[0].starts_with('-') => unquote(&tokens.remove(0)),
        None => return Err(AppError::invalid_input("pc index describe needs the index: --index-name <index>.")),
    };
    if let Some(extra) = tokens.first() {
        return Err(AppError::invalid_input(format!("pc index describe does not take {extra}.")));
    }
    if name.is_empty() {
        return Err(AppError::invalid_input("--index-name needs a value."));
    }
    if name.bytes().all(|b| b == b'.') {
        return Err(AppError::invalid_input(format!("\"{name}\" is not an index name.")));
    }
    let path = format!("/indexes/{}", segment(&name));
    Ok(Plan::native(json!({ "method": "GET", "path": path }).to_string()))
}

/// One URL path segment: everything but unreserved characters is percent-encoded.
/// (A segment of nothing but dots never gets here: `describe` refuses it.)
fn segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'~' | b'.') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{plan, CommandOrigin, Ctx, Step};
    use crate::model::Engine;
    use serde_json::{json, Value};

    fn statements(text: &str) -> Vec<String> {
        match plan(&Ctx::bare(Engine::Pinecone), text) {
            Some(Ok(p)) => p.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn refused(text: &str) -> String {
        match plan(&Ctx::bare(Engine::Pinecone), text) {
            Some(Err(e)) => e.message().to_string(),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn help_lists_every_command() {
        for word in ["help", "?", "Help;"] {
            let found = plan(&Ctx::bare(Engine::Pinecone), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}");
        }
    }

    #[test]
    fn listing_indexes_runs_the_consoles_indexes_shorthand() {
        assert_eq!(statements("pc index list"), ["INDEXES"]);
        assert_eq!(statements("PC  Index   LIST;"), ["INDEXES"]);
        assert_eq!(statements("pc index list --wide --json"), ["INDEXES"], "output formats are ignored");
        assert_eq!(statements("pc index list -w -j"), ["INDEXES"]);
    }

    #[test]
    fn describing_an_index_reads_one_control_plane_path() {
        let expected = json!({ "method": "GET", "path": "/indexes/docs-v2" });
        for text in [
            "pc index describe --index-name docs-v2",
            "pc index describe --index-name=docs-v2",
            "pc index describe -i docs-v2",
            "pc index describe --name docs-v2",
            "pc index describe --name=docs-v2",
            "pc index describe -n docs-v2",
            "pc index describe docs-v2",
            "pc index describe --index-name \"docs-v2\" --json",
            "pc index describe -j -i docs-v2",
        ] {
            let found = statements(text);
            assert_eq!(found.len(), 1, "{text}");
            assert_eq!(serde_json::from_str::<Value>(&found[0]).unwrap_or(Value::Null), expected, "{text}");
        }
    }

    #[test]
    fn a_hostile_name_cannot_leave_its_path_segment() {
        let found = statements("pc index describe --index-name 'a/../keys?x=1#\"}'");
        let parsed: Value = serde_json::from_str(&found[0]).unwrap_or(Value::Null);
        assert_eq!(parsed["method"], json!("GET"));
        assert_eq!(parsed["path"], json!("/indexes/a%2F..%2Fkeys%3Fx%3D1%23%22%7D"));
        assert_eq!(parsed.as_object().map(|o| o.len()), Some(2));
        // The URL parser reads `%2E%2E` as a `..` step, so a name of dots alone cannot be kept in its segment.
        for dots in [".", "..", "..."] {
            assert!(refused(&format!("pc index describe --index-name {dots}")).contains("not an index name"), "{dots}");
        }
        assert_eq!(statements("pc index describe a..b").len(), 1, "dots inside a name are fine");
    }

    #[test]
    fn bad_arguments_say_how_to_fix_them() {
        assert!(refused("pc index describe").contains("--index-name"));
        assert!(refused("pc index describe --index-name").contains("needs a value"));
        assert!(refused("pc index describe -i ''").contains("needs a value"));
        assert!(refused("pc index describe a b").contains("--index-name"));
        assert!(refused("pc index describe --index-name a --verbose").contains("does not take"));
        assert!(refused("pc index list now").contains("does not take"));
        assert!(refused("pc collection list").contains("Supported"));
        assert!(refused("pc index create").contains("Supported"));
    }

    #[test]
    fn native_shorthands_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::Pinecone), native.name).is_none(), "{}", native.name);
        }
        for text in ["STATS docs", "LIST docs 5", "{\"index\": \"docs\", \"query\": {\"topK\": 3}}", "pcs index list"] {
            assert!(plan(&Ctx::bare(Engine::Pinecone), text).is_none(), "{text}");
        }
    }
}
