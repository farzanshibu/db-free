// SOT: shell-meta-s3, aws-s3-ls-command, aws-s3api-list-commands, minio-mc-ls-command

use super::args::{eq_ci, raw_args, split_word, unquote};
use super::{help_plan, Ctx, Cut, Dialect, Plan, ShellCommand};
use crate::error::{AppError, AppResult};

// ============================================================================
// S3 (Amazon S3, MinIO, Cloudflare R2)
//
// WHAT:  The listing commands of the S3 tools: `aws s3 ls [s3://bucket[/prefix]]`,
//        `aws s3api list-buckets`, `aws s3api list-objects-v2 --bucket <b>
//        [--prefix <p>]` (and `list-objects`), and MinIO's `mc ls <alias>[/bucket
//        [/prefix]]`.
// WHY:   The people who use these stores list them with these commands; the query
//        tab answers them through the connection instead of failing with "Enter
//        BUCKETS, LIST ...".
// HOW:   Without a bucket they become the console's `BUCKETS`; with one they become
//        `LIST <bucket> [prefix]`. `execute` in integrations/s3.rs splits its command
//        on whitespace of any kind and joins the words with single spaces, so the
//        bucket must be one word (letters, digits, dots, hyphens, underscores) and a
//        prefix may hold single plain spaces only (no tabs, no line breaks, no other
//        Unicode blanks such as U+00A0, no runs of spaces); anything else is refused,
//        never reshaped. `LIST` lists keys flat, the way
//        `aws s3 ls --recursive` does, so both spellings run it; `--recursive`,
//        `--human-readable` and `--summarize` are accepted. Options that only say
//        where to connect (`--endpoint-url`, `--region`, `--profile`) are accepted
//        and ignored because the connection decides. In `mc ls` the first path
//        segment is mc's alias for the server and is ignored for the same reason.
//        Nothing here writes: `rm`, `cp`, `mb` and the rest are not translated.
// WHERE: src-tauri/src/integrations/s3.rs (`execute`)
// ============================================================================

pub const DIALECT: Dialect = Dialect { prefix: None, strict: false, help: &["help", "?"], cut: Cut::Lines, commands: COMMANDS, plan };

const COMMANDS: &[ShellCommand] = &[
    ShellCommand::shell("help", "help | ?", "List the commands this console understands"),
    ShellCommand::shell("aws s3 ls", "aws s3 ls [s3://<bucket>[/<prefix>]] [--recursive]", "The buckets, or the keys of a bucket under a prefix (listed flat)"),
    ShellCommand::shell("aws s3api list-buckets", "aws s3api list-buckets", "Every bucket with its creation date"),
    ShellCommand::shell("aws s3api list-objects-v2", "aws s3api list-objects-v2 --bucket <bucket> [--prefix <prefix>]", "The keys of a bucket under a prefix"),
    ShellCommand::shell("mc ls", "mc ls <alias>[/<bucket>[/<prefix>]]", "MinIO client: the buckets, or the keys of a bucket (the alias is ignored)"),
    ShellCommand::native("BUCKETS", "BUCKETS", "Every bucket with its creation date"),
    ShellCommand::native("LIST", "LIST <bucket> [prefix]", "The keys of a bucket under a prefix"),
    ShellCommand::native("HEAD", "HEAD <bucket> <key>", "Checks that an object exists (an error if not; the headers are not shown)"),
];

fn plan(_ctx: &Ctx, text: &str) -> Option<AppResult<Plan>> {
    if DIALECT.help.iter().any(|h| eq_ci(h, text)) {
        return Some(Ok(help_plan(&DIALECT)));
    }
    let (head, rest) = split_word(text);
    // Runbooks spell the program with its directory: `/usr/local/bin/aws`, `./mc`.
    let program = head.rsplit(['/', '\\']).next().unwrap_or(head);
    if eq_ci(program, "aws") {
        return Some(aws(rest));
    }
    if eq_ci(program, "mc") {
        let (verb, tail) = split_word(rest);
        return eq_ci(verb, "ls").then(|| mc_ls(tail));
    }
    None
}

// WHAT:  `rest` without the options `aws` takes before the service name
//        (`aws --profile dev --region eu-west-1 s3 ls`), all of which only say where to connect.
fn without_global_options(mut rest: &str) -> &str {
    loop {
        let (word, after) = split_word(rest);
        let name = word.split('=').next().unwrap_or(word);
        if CONNECTION_OPTIONS.contains(&name) {
            rest = if word.contains('=') { after } else { split_word(after).1 };
        } else if PLAIN_FLAGS.contains(&word) {
            rest = after;
        } else {
            return rest;
        }
    }
}

fn aws(rest: &str) -> AppResult<Plan> {
    let (service, rest) = split_word(without_global_options(rest));
    let (verb, rest) = split_word(rest);
    match (service.to_ascii_lowercase().as_str(), verb.to_ascii_lowercase().as_str()) {
        ("s3", "ls") => aws_s3_ls(rest),
        ("s3api", "list-buckets") => {
            let args = Args::read(rest, &[])?;
            args.nothing_else("aws s3api list-buckets")?;
            Ok(Plan::native("BUCKETS"))
        }
        ("s3api", "list-objects-v2" | "list-objects") => {
            let args = Args::read(rest, &["--bucket", "--prefix"])?;
            let bucket = args.value("--bucket").ok_or_else(|| AppError::invalid_input("aws s3api list-objects-v2 needs --bucket <bucket>."))?;
            args.nothing_else("aws s3api list-objects-v2")?;
            list(bucket, args.value("--prefix").unwrap_or(""))
        }
        _ => Err(AppError::invalid_input("Supported: aws s3 ls, aws s3api list-buckets, aws s3api list-objects-v2 --bucket <bucket>.")),
    }
}

fn aws_s3_ls(rest: &str) -> AppResult<Plan> {
    let args = Args::read(rest, &[])?;
    args.nothing_else("aws s3 ls")?;
    match args.positional.as_slice() {
        [] => Ok(Plan::native("BUCKETS")),
        [target] => {
            let path = target
                .strip_prefix("s3://")
                .ok_or_else(|| AppError::invalid_input("Name a bucket as s3://<bucket>[/<prefix>]."))?;
            let (bucket, prefix) = path.split_once('/').unwrap_or((path, ""));
            if bucket.is_empty() {
                return Ok(Plan::native("BUCKETS"));
            }
            list(bucket, prefix)
        }
        _ => Err(AppError::invalid_input("aws s3 ls takes one s3://<bucket>[/<prefix>] target.")),
    }
}

fn mc_ls(rest: &str) -> AppResult<Plan> {
    let mut args = Args::read(rest, &[])?;
    // mc's own flags that do not change what is listed.
    args.flags.retain(|f| !matches!(f.as_str(), "-r" | "--recursive" | "--json"));
    args.nothing_else("mc ls")?;
    match args.positional.as_slice() {
        [target] => {
            let mut parts = target.trim_start_matches('/').splitn(3, '/');
            let _alias = parts.next();
            match (parts.next(), parts.next()) {
                (None | Some(""), _) => Ok(Plan::native("BUCKETS")),
                (Some(bucket), prefix) => list(bucket, prefix.unwrap_or("")),
            }
        }
        _ => Err(AppError::invalid_input("mc ls takes one <alias>[/<bucket>[/<prefix>]] target.")),
    }
}

/// `LIST <bucket> [prefix]`, refusing what the console's whitespace-split command would reshape.
fn list(bucket: &str, prefix: &str) -> AppResult<Plan> {
    if bucket.is_empty() || !bucket.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) {
        return Err(AppError::invalid_input(format!("\"{bucket}\" is not a bucket name (letters, digits, dots, hyphens and underscores).")));
    }
    // `execute` splits on every Unicode blank and rejoins with a plain space, so only plain single spaces survive.
    if prefix.chars().any(|c| c.is_control() || (c.is_whitespace() && c != ' ')) || prefix.contains("  ") || prefix.starts_with(' ') || prefix.ends_with(' ') {
        return Err(AppError::invalid_input("A prefix with tabs, line breaks or repeated or edge spaces cannot be typed here; pick the folder in the Objects panel."));
    }
    Ok(Plan::native(if prefix.is_empty() { format!("LIST {bucket}") } else { format!("LIST {bucket} {prefix}") }))
}

/// The options accepted by every listing command; the connection decides where it runs.
const CONNECTION_OPTIONS: &[&str] = &["--endpoint-url", "--region", "--profile"];
const PLAIN_FLAGS: &[&str] = &["--recursive", "--human-readable", "--summarize", "--no-sign-request", "--no-verify-ssl", "--debug"];

struct Args {
    positional: Vec<String>,
    values: Vec<(String, String)>,
    /// Flags that were not recognised as a plain flag or a value option, kept so the caller can decide.
    flags: Vec<String>,
}

impl Args {
    // WHAT:  Splits arguments into positionals, `--name value` pairs for `wanted` plus the
    //        connection options, and flags. `--name=value` is read the same way.
    fn read(rest: &str, wanted: &[&str]) -> AppResult<Args> {
        let tokens = raw_args(rest).map_err(AppError::invalid_input)?;
        let mut out = Args { positional: Vec::new(), values: Vec::new(), flags: Vec::new() };
        let mut iter = tokens.iter();
        while let Some(token) = iter.next() {
            if !token.starts_with('-') {
                out.positional.push(unquote(token));
                continue;
            }
            let (name, inline) = match token.split_once('=') {
                Some((name, value)) if token.starts_with("--") => (name, Some(unquote(value))),
                _ => (token.as_str(), None),
            };
            if wanted.contains(&name) || CONNECTION_OPTIONS.contains(&name) {
                let value = match inline {
                    Some(value) => value,
                    None => iter.next().map(|v| unquote(v)).ok_or_else(|| AppError::invalid_input(format!("{name} needs a value.")))?,
                };
                out.values.push((name.to_string(), value));
            } else if PLAIN_FLAGS.contains(&name) {
                // Accepted, changes nothing: listing is flat and human-readable sizes are the grid's job.
            } else {
                out.flags.push(token.clone());
            }
        }
        Ok(out)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    fn nothing_else(&self, command: &str) -> AppResult<()> {
        match self.flags.first() {
            None => Ok(()),
            Some(flag) => Err(AppError::invalid_input(format!("{command} does not take {flag} here."))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{plan, CommandOrigin, Ctx, Step};
    use crate::model::Engine;

    fn statements(text: &str) -> Vec<String> {
        match plan(&Ctx::bare(Engine::S3), text) {
            Some(Ok(p)) => p.statements().map(str::to_string).collect(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn refused(text: &str) -> String {
        match plan(&Ctx::bare(Engine::Minio), text) {
            Some(Err(e)) => e.message().to_string(),
            other => panic!("{text}: {other:?}"),
        }
    }

    #[test]
    fn help_lists_every_command() {
        for word in ["help", "?", "Help;"] {
            let found = plan(&Ctx::bare(Engine::S3), word);
            assert!(matches!(found, Some(Ok(ref p)) if matches!(p.steps.as_slice(), [Step::Table(t)] if t.rows.len() == super::COMMANDS.len())), "{word}");
        }
    }

    #[test]
    fn aws_s3_ls_lists_buckets_or_keys() {
        assert_eq!(statements("aws s3 ls"), ["BUCKETS"]);
        assert_eq!(statements("AWS S3 LS s3://"), ["BUCKETS"]);
        assert_eq!(statements("aws s3 ls s3://logs"), ["LIST logs"]);
        assert_eq!(statements("aws s3 ls s3://logs/"), ["LIST logs"]);
        assert_eq!(statements("aws s3 ls s3://logs/2026/01/ --recursive --human-readable --summarize"), ["LIST logs 2026/01/"]);
        assert_eq!(statements("aws s3 ls s3://logs/a --endpoint-url http://localhost:9000 --region auto --profile dev"), ["LIST logs a"]);
        assert_eq!(statements("aws s3 ls \"s3://logs/my folder/\""), ["LIST logs my folder/"]);
        assert_eq!(statements("aws s3 ls --endpoint-url=http://localhost:9000;"), ["BUCKETS"]);
    }

    #[test]
    fn the_program_may_carry_its_directory_and_aws_its_global_options() {
        assert_eq!(statements("/usr/local/bin/aws s3 ls s3://logs"), ["LIST logs"]);
        assert_eq!(statements("aws --profile dev --region eu-west-1 s3 ls s3://logs/a"), ["LIST logs a"]);
        assert_eq!(statements("aws --endpoint-url=http://localhost:9000 --no-sign-request s3api list-buckets"), ["BUCKETS"]);
        assert_eq!(statements("./mc ls myminio/logs"), ["LIST logs"]);
        assert!(refused("aws --profile s3 ls").contains("Supported"), "a profile named s3 is a value, not the service");
        for text in ["/usr/bin/awsx s3 ls", "./mcx ls a", "awsome s3 ls"] {
            assert!(plan(&Ctx::bare(Engine::S3), text).is_none(), "{text}");
        }
    }

    #[test]
    fn aws_s3api_listings() {
        assert_eq!(statements("aws s3api list-buckets"), ["BUCKETS"]);
        assert_eq!(statements("aws s3api list-objects-v2 --bucket logs"), ["LIST logs"]);
        assert_eq!(statements("aws s3api list-objects-v2 --bucket=logs --prefix 2026/"), ["LIST logs 2026/"]);
        assert_eq!(statements("aws s3api list-objects --prefix 'a b/' --bucket logs"), ["LIST logs a b/"]);
    }

    #[test]
    fn mc_ls_ignores_the_alias() {
        assert_eq!(statements("mc ls myminio"), ["BUCKETS"]);
        assert_eq!(statements("mc ls myminio/"), ["BUCKETS"]);
        assert_eq!(statements("mc ls myminio/logs"), ["LIST logs"]);
        assert_eq!(statements("mc ls --recursive myminio/logs/2026/01/"), ["LIST logs 2026/01/"]);
        assert_eq!(statements("mc ls myminio/logs/"), ["LIST logs"]);
        assert_eq!(statements("MC LS -r myminio/logs"), ["LIST logs"]);
    }

    #[test]
    fn only_a_bucket_name_and_a_plain_prefix_reach_the_command() {
        assert!(refused("aws s3 ls s3://logs;DELETE").contains("not a bucket name"));
        assert!(refused("aws s3 ls \"s3://lo gs\"").contains("not a bucket name"));
        assert!(refused("aws s3api list-objects-v2 --bucket 'a b'").contains("not a bucket name"));
        assert!(refused("aws s3 ls \"s3://logs/a  b\"").contains("cannot be typed"));
        assert!(refused("aws s3 ls \"s3://logs/ a\"").contains("cannot be typed"));
        assert!(refused("mc ls m/logs/\"x\ty\"").contains("cannot be typed"));
        // The console's split_whitespace would turn U+00A0 or U+3000 into a plain space: a different prefix.
        assert!(refused("aws s3 ls \"s3://logs/a\u{a0}b\"").contains("cannot be typed"));
        assert!(refused("aws s3api list-objects-v2 --bucket logs --prefix \"a\u{3000}b\"").contains("cannot be typed"));
        assert!(refused("aws s3api list-objects-v2 --bucket logs --prefix \"a\u{2028}b\"").contains("cannot be typed"));
        assert_eq!(statements("aws s3 ls s3://logs/caf\u{e9}/"), ["LIST logs caf\u{e9}/"], "other characters are left alone");
    }

    #[test]
    fn unsupported_forms_say_what_is_supported() {
        assert!(refused("aws s3 ls logs").contains("s3://"));
        assert!(refused("aws s3 ls s3://a s3://b").contains("one"));
        assert!(refused("aws s3 ls s3://a --delete").contains("does not take --delete"));
        assert!(refused("aws s3 rm s3://a/k").contains("Supported"));
        assert!(refused("aws s3api delete-object --bucket a --key k").contains("Supported"));
        assert!(refused("aws s3api list-objects-v2").contains("--bucket"));
        assert!(refused("aws s3api list-objects-v2 --bucket").contains("needs a value"));
        assert!(refused("aws s3api list-objects-v2 --bucket a --max-items 3").contains("does not take"));
        assert!(refused("mc ls").contains("one"));
        assert!(refused("mc ls a b").contains("one"));
        assert!(refused("aws s3 ls 's3://a").contains("quote"));
    }

    #[test]
    fn natives_and_other_commands_are_sent_as_typed() {
        for native in super::COMMANDS.iter().filter(|c| c.origin == CommandOrigin::Native) {
            assert!(plan(&Ctx::bare(Engine::S3), native.name).is_none(), "{}", native.name);
        }
        for text in ["LIST logs 2026/", "GET logs a.txt", "DELETE logs a.txt", "mc cp a b", "mcx ls a"] {
            assert!(plan(&Ctx::bare(Engine::CloudflareR2), text).is_none(), "{text}");
        }
    }
}
