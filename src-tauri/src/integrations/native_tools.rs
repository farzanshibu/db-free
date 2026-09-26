// SOT: native-tools, native-backup-restore, pg-dump-args, pg-restore-args, mysqldump-args, mongodump-args, tool-discovery, process-runner, secret-temp-file, sqlite-file-restore

use crate::error::{AppError, AppResult};
use crate::integrations::sqlite;
use crate::model::{BackupFormat, BackupMethod, BackupOptions, ConnectionSummary, Engine, NativeTool, NativeToolStatus, SslMode};
use std::collections::{BTreeMap, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::oneshot;

// ============================================================================
// NATIVE BACKUP / RESTORE
//
// WHAT:  Backs up and restores databases with the engine's own client tools
//        (pg_dump / pg_restore / psql, mysqldump / mysql, mongodump /
//        mongorestore), and file engines by a consistent copy.
// WHY:   These are the tools whose output every DBA already trusts and can
//        restore without this app. This file is the only place the app starts
//        an external process, so argument building, password hand-off and
//        cancellation are reviewed in one spot.
// HOW:   Arguments are built by pure functions (unit-tested below) and passed
//        as an argv array — never through a shell. Every value is attached with
//        `--flag=value`, so a table or database name can never be read as an
//        option. Passwords go through the environment (PGPASSWORD) or a temp
//        option file created owner-only and deleted on drop — never argv, which
//        other local users can read from the process list.
// WHERE: src-tauri/src/services/backup.rs (orchestration), src-tauri/src/guard/mod.rs
//        (step 11: restore is destructive), src/features/backup/BackupDialog.tsx
// ============================================================================

/// Lines of tool output kept for the error message when a run fails.
const TAIL_LINES: usize = 12;
/// How often the size of the file being written is reported.
const PROGRESS_TICK: Duration = Duration::from_millis(500);

// WHAT:  Where a running tool's output and progress go.
// WHY:   Services and integrations may not import tauri; the command layer
//        implements this to forward frames to the window as events.
pub trait ToolSink: Send + Sync {
    fn log(&self, line: &str);
    fn progress(&self, bytes: u64, total: Option<u64>);
}

/// How a run ended when it did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    Finished,
    Cancelled,
}

// ---------------------------------------------------------------------------
// Which engines, which tools
// ---------------------------------------------------------------------------

// WHAT:  The backup path an engine has in the app, if any.
// WHY:   Wire compatibility is not dump compatibility: CockroachDB, YugabyteDB
//        and QuestDB speak the Postgres protocol but pg_dump cannot read their
//        catalogs, so only real Postgres builds get pg_dump.
pub fn method_for(engine: Engine) -> Option<BackupMethod> {
    match engine {
        Engine::Postgres | Engine::Supabase | Engine::Neon | Engine::Timescaledb | Engine::Pgvector | Engine::Postgis => {
            Some(BackupMethod::PgDump)
        }
        Engine::Mysql | Engine::Mariadb | Engine::Tidb | Engine::Planetscale => Some(BackupMethod::Mysqldump),
        Engine::Mongodb => Some(BackupMethod::Mongodump),
        Engine::Sqlite | Engine::Spatialite => Some(BackupMethod::SqliteCopy),
        Engine::Duckdb => Some(BackupMethod::FileCopy),
        _ => None,
    }
}

pub fn tools_for(method: BackupMethod) -> &'static [NativeTool] {
    match method {
        BackupMethod::PgDump => &[NativeTool::PgDump, NativeTool::PgRestore, NativeTool::Psql],
        BackupMethod::Mysqldump => &[NativeTool::Mysqldump, NativeTool::Mysql],
        BackupMethod::Mongodump => &[NativeTool::Mongodump, NativeTool::Mongorestore],
        BackupMethod::SqliteCopy | BackupMethod::FileCopy => &[],
    }
}

/// Formats a backup can be written in; the first is the default.
pub fn formats_for(method: BackupMethod) -> Vec<BackupFormat> {
    match method {
        BackupMethod::PgDump => vec![BackupFormat::Custom, BackupFormat::Plain, BackupFormat::Directory, BackupFormat::Tar],
        BackupMethod::Mysqldump => vec![BackupFormat::Plain],
        BackupMethod::Mongodump => vec![BackupFormat::Archive],
        BackupMethod::SqliteCopy | BackupMethod::FileCopy => vec![BackupFormat::File],
    }
}

pub fn unsupported_note(engine: Engine) -> String {
    format!(
        "{} has no native backup in DB Free yet. Use the engine's own backup tooling, or Export / Import for table data.",
        engine.label()
    )
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

pub fn not_found(tool: NativeTool) -> AppError {
    AppError::not_found(format!(
        "{} not found — install {} or set its path in Settings → Advanced.",
        tool.program(),
        tool.package()
    ))
}

fn executable_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd"), format!("{name}.bat")]
    } else {
        vec![name.to_string()]
    }
}

fn find_in(dir: &Path, tool: NativeTool) -> Option<PathBuf> {
    tool.candidates()
        .iter()
        .flat_map(|name| executable_names(name))
        .map(|file| dir.join(file))
        .find(|path| path.is_file())
}

/// Subdirectories of `base` (optionally only those starting with `prefix`),
/// newest-looking first, each joined with `bin`.
fn versioned_bins(base: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs.reverse();
    dirs.into_iter().map(|d| d.join("bin")).collect()
}

// WHAT:  Places installers put client tools when they do not touch PATH.
// WHY:   A GUI app on macOS does not inherit the shell's PATH (Homebrew's bin
//        is invisible to it), and the Windows installers for PostgreSQL, MySQL
//        and MongoDB leave PATH alone by default.
fn well_known_dirs(tool: NativeTool) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        let program_files = PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into()));
        match tool {
            NativeTool::PgDump | NativeTool::PgRestore | NativeTool::Psql => {
                dirs.extend(versioned_bins(&program_files.join("PostgreSQL"), ""));
            }
            NativeTool::Mysqldump | NativeTool::Mysql => {
                dirs.extend(versioned_bins(&program_files.join("MySQL"), ""));
                dirs.extend(versioned_bins(&program_files, "MariaDB"));
            }
            NativeTool::Mongodump | NativeTool::Mongorestore => {
                dirs.extend(versioned_bins(&program_files.join("MongoDB").join("Tools"), ""));
            }
        }
    } else {
        for fixed in [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/opt/homebrew/opt/libpq/bin",
            "/usr/local/opt/libpq/bin",
            "/opt/homebrew/opt/mysql-client/bin",
            "/usr/local/opt/mysql-client/bin",
            "/Applications/Postgres.app/Contents/Versions/latest/bin",
        ] {
            dirs.push(PathBuf::from(fixed));
        }
        dirs.extend(versioned_bins(Path::new("/usr/lib/postgresql"), ""));
    }
    dirs
}

// WHAT:  Resolves a tool: the Settings override when there is one, else PATH,
//        else the installers' usual folders. The bool is "came from override".
// HOW:   An override may name the executable or the directory holding it. A
//        set-but-wrong override is reported as not found rather than silently
//        falling back to PATH — the user asked for that binary.
pub fn locate(tool: NativeTool, overrides: &BTreeMap<NativeTool, String>) -> (Option<PathBuf>, bool) {
    if let Some(raw) = overrides.get(&tool).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let path = PathBuf::from(raw);
        let found = if path.is_file() { Some(path) } else if path.is_dir() { find_in(&path, tool) } else { None };
        return (found, true);
    }
    let on_path = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    let found = on_path
        .iter()
        .chain(well_known_dirs(tool).iter())
        .find_map(|dir| find_in(dir, tool));
    (found, false)
}

pub fn require(tool: NativeTool, overrides: &BTreeMap<NativeTool, String>) -> AppResult<PathBuf> {
    locate(tool, overrides).0.ok_or_else(|| not_found(tool))
}

/// First line of `<tool> --version`, or None when it does not answer in time.
pub async fn version(path: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    no_window(&mut cmd);
    let output = tokio::time::timeout(Duration::from_secs(5), cmd.output()).await.ok()?.ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

pub async fn status(tool: NativeTool, overrides: &BTreeMap<NativeTool, String>) -> NativeToolStatus {
    let (path, overridden) = locate(tool, overrides);
    let version = match &path {
        Some(p) => version(p).await,
        None => None,
    };
    NativeToolStatus {
        tool,
        path: path.map(|p| p.to_string_lossy().into_owned()),
        version,
        overridden,
        package: tool.package().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Argument building (pure)
// ---------------------------------------------------------------------------

fn host(s: &ConnectionSummary) -> String {
    s.host.as_deref().map(str::trim).filter(|h| !h.is_empty()).unwrap_or("localhost").to_string()
}

fn port(s: &ConnectionSummary) -> u16 {
    s.port.or_else(|| s.engine.default_port()).unwrap_or(0)
}

fn nonblank(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|x| !x.is_empty())
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

// WHAT:  The value for libpq's `--dbname`.
// WHY:   libpq reads a dbname containing `=` (or a postgres:// URI) as a whole
//        connection string, so a database literally named `host=elsewhere`
//        would redirect the dump. Quoting it as `dbname='…'` keeps it a name;
//        host, port and user still come from their own flags.
pub fn pg_dbname(database: &str) -> String {
    if database.contains('=') || database.starts_with("postgres://") || database.starts_with("postgresql://") {
        let escaped = database.replace('\\', "\\\\").replace('\'', "\\'");
        format!("dbname='{escaped}'")
    } else {
        database.to_string()
    }
}

fn pg_connection_args(s: &ConnectionSummary) -> Vec<String> {
    let mut args = vec![format!("--host={}", host(s)), format!("--port={}", port(s))];
    if let Some(user) = nonblank(&s.username) {
        args.push(format!("--username={user}"));
    }
    args.push(format!("--dbname={}", pg_dbname(nonblank(&s.database).unwrap_or("postgres"))));
    // Never prompt: stdin is closed, and a prompt would hang the run.
    args.push("--no-password".into());
    args
}

fn pg_filter_args(opts: &BackupOptions, args: &mut Vec<String>) {
    if opts.schema_only {
        args.push("--schema-only".into());
    } else if opts.data_only {
        args.push("--data-only".into());
    }
    for schema in opts.schemas.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        args.push(format!("--schema={schema}"));
    }
    for table in opts.tables.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        args.push(format!("--table={table}"));
    }
}

// WHAT:  libpq settings that are not secrets but are not flags either.
pub fn pg_env(s: &ConnectionSummary, secret: Option<&str>) -> Vec<(String, String)> {
    let ssl = match s.ssl_mode {
        SslMode::Disable => "disable",
        SslMode::Prefer => "prefer",
        SslMode::Require => "require",
        SslMode::VerifyCa => "verify-ca",
        SslMode::VerifyFull => "verify-full",
    };
    let mut env = vec![
        ("PGSSLMODE".to_string(), ssl.to_string()),
        ("PGCONNECT_TIMEOUT".to_string(), "15".to_string()),
        ("PGAPPNAME".to_string(), "db-free".to_string()),
    ];
    if let Some(password) = secret.filter(|p| !p.is_empty()) {
        env.push(("PGPASSWORD".to_string(), password.to_string()));
    }
    env
}

pub fn pg_dump_args(s: &ConnectionSummary, opts: &BackupOptions, out: &Path) -> Vec<String> {
    let format = match opts.format {
        BackupFormat::Plain => "p",
        BackupFormat::Directory => "d",
        BackupFormat::Tar => "t",
        BackupFormat::Custom | BackupFormat::Archive | BackupFormat::File => "c",
    };
    let mut args = vec!["--verbose".to_string(), format!("--format={format}"), format!("--file={}", path_arg(out))];
    args.extend(pg_connection_args(s));
    pg_filter_args(opts, &mut args);
    // Archive formats decide --clean at restore time; only a script bakes it in.
    if opts.clean && opts.format == BackupFormat::Plain {
        args.push("--clean".into());
        args.push("--if-exists".into());
    }
    if opts.no_owner {
        args.push("--no-owner".into());
    }
    args
}

pub fn pg_restore_args(s: &ConnectionSummary, opts: &BackupOptions, input: &Path) -> Vec<String> {
    let mut args = vec!["--verbose".to_string()];
    args.extend(pg_connection_args(s));
    pg_filter_args(opts, &mut args);
    if opts.clean {
        args.push("--clean".into());
        args.push("--if-exists".into());
    }
    if opts.no_owner {
        args.push("--no-owner".into());
    }
    if opts.single_transaction {
        args.push("--single-transaction".into());
        args.push("--exit-on-error".into());
    }
    // `--` ends option parsing: the archive path is data, whatever it starts with.
    args.push("--".into());
    args.push(path_arg(input));
    args
}

pub fn psql_args(s: &ConnectionSummary, opts: &BackupOptions, input: &Path) -> Vec<String> {
    let mut args = vec!["--no-psqlrc".to_string()];
    args.extend(pg_connection_args(s));
    args.push("--set=ON_ERROR_STOP=1".into());
    if opts.single_transaction {
        args.push("--single-transaction".into());
    }
    args.push(format!("--file={}", path_arg(input)));
    args
}

// WHAT:  What kind of pg_dump output a restore source is.
// HOW:   Custom archives start with "PGDMP"; tar archives carry "ustar" at byte
//        257; a directory is a directory. Anything else is taken as SQL for psql.
pub fn detect_pg_format(path: &Path) -> AppResult<BackupFormat> {
    if path.is_dir() {
        return Ok(BackupFormat::Directory);
    }
    let mut head = [0u8; 262];
    let read = {
        let mut file = std::fs::File::open(path).map_err(|e| AppError::invalid_input(format!("Cannot read {}: {e}", path.display())))?;
        let mut total = 0;
        loop {
            match std::io::Read::read(&mut file, &mut head[total..]) {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if total == head.len() {
                        break;
                    }
                }
                Err(e) => return Err(AppError::invalid_input(format!("Cannot read {}: {e}", path.display()))),
            }
        }
        total
    };
    let head = &head[..read];
    if head.starts_with(b"PGDMP") {
        Ok(BackupFormat::Custom)
    } else if head.len() >= 262 && &head[257..262] == b"ustar" {
        Ok(BackupFormat::Tar)
    } else {
        Ok(BackupFormat::Plain)
    }
}

pub fn is_gzip(path: &Path) -> bool {
    let mut magic = [0u8; 2];
    std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic))
        .is_ok()
        && magic == [0x1f, 0x8b]
}

/// Names that go into argv positionally must not look like options.
fn positional(kind: &str, name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.starts_with('-') {
        return Err(AppError::invalid_input(format!("{kind} name \"{name}\" cannot start with '-'.")));
    }
    Ok(name.to_string())
}

// WHAT:  mysql / mysqldump connection flags.
// HOW:   `--defaults-extra-file` must be the very first argument or the client
//        refuses it — so it is placed first here, and tested.
fn mysql_connection_args(s: &ConnectionSummary, defaults_file: Option<&Path>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(file) = defaults_file {
        args.push(format!("--defaults-extra-file={}", path_arg(file)));
    }
    args.push(format!("--host={}", host(s)));
    args.push(format!("--port={}", port(s)));
    args.push("--protocol=TCP".into());
    if let Some(user) = nonblank(&s.username) {
        args.push(format!("--user={user}"));
    }
    args
}

pub fn mysqldump_args(s: &ConnectionSummary, opts: &BackupOptions, out: &Path, defaults_file: Option<&Path>) -> AppResult<Vec<String>> {
    let mut args = mysql_connection_args(s, defaults_file);
    args.push("--verbose".into());
    if opts.single_transaction {
        args.push("--single-transaction".into());
    }
    if opts.routines {
        args.push("--routines".into());
    }
    args.push(if opts.triggers { "--triggers".into() } else { "--skip-triggers".into() });
    if opts.schema_only {
        args.push("--no-data".into());
    } else if opts.data_only {
        args.push("--no-create-info".into());
    }
    args.push(format!("--result-file={}", path_arg(out)));
    match nonblank(&s.database) {
        Some(db) => {
            // Positional database (not --databases): the dump carries no
            // CREATE DATABASE / USE, so it restores into whichever database
            // the restoring connection names.
            args.push(positional("Database", db)?);
            for table in opts.tables.iter().filter(|t| !t.trim().is_empty()) {
                args.push(positional("Table", table)?);
            }
        }
        None => args.push("--all-databases".into()),
    }
    Ok(args)
}

pub fn mysql_restore_args(s: &ConnectionSummary, defaults_file: Option<&Path>) -> Vec<String> {
    let mut args = mysql_connection_args(s, defaults_file);
    if let Some(db) = nonblank(&s.database) {
        args.push(format!("--database={db}"));
    }
    args
}

// WHAT:  The `[client]` option file that carries the MySQL password.
// HOW:   The value is double-quoted; the client strips the quotes and then
//        unescapes backslash sequences, so backslashes and line breaks are
//        escaped here and survive the round trip.
pub fn mysql_option_file(password: &str) -> String {
    let mut escaped = String::with_capacity(password.len());
    for c in password.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    format!("[client]\npassword=\"{escaped}\"\n")
}

fn mongo_connection_args(s: &ConnectionSummary, config: Option<&Path>) -> Vec<String> {
    let mut args = vec![format!("--host={}", host(s)), format!("--port={}", port(s))];
    if let Some(user) = nonblank(&s.username) {
        args.push(format!("--username={user}"));
        // Same auth database the adapter uses (integrations/mongodb.rs).
        args.push("--authenticationDatabase=admin".into());
    }
    if let Some(file) = config {
        args.push(format!("--config={}", path_arg(file)));
    }
    if matches!(s.ssl_mode, SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull) {
        args.push("--ssl".into());
    }
    args
}

pub fn mongodump_args(s: &ConnectionSummary, opts: &BackupOptions, out: &Path, config: Option<&Path>) -> Vec<String> {
    let mut args = mongo_connection_args(s, config);
    if let Some(db) = nonblank(&s.database) {
        args.push(format!("--db={db}"));
        // mongodump takes a single --collection; several tables means the whole database.
        let tables: Vec<&str> = opts.tables.iter().map(|t| t.trim()).filter(|t| !t.is_empty()).collect();
        if let [only] = tables.as_slice() {
            args.push(format!("--collection={only}"));
        }
    }
    args.push(format!("--archive={}", path_arg(out)));
    if opts.gzip {
        args.push("--gzip".into());
    }
    args
}

pub fn mongorestore_args(s: &ConnectionSummary, opts: &BackupOptions, input: &Path, gzip: bool, config: Option<&Path>) -> Vec<String> {
    let mut args = mongo_connection_args(s, config);
    args.push(format!("--archive={}", path_arg(input)));
    if gzip {
        args.push("--gzip".into());
    }
    if opts.drop {
        args.push("--drop".into());
    }
    args
}

// WHAT:  The YAML `--config` file that carries the MongoDB password.
pub fn mongo_config_file(password: &str) -> String {
    let mut escaped = String::with_capacity(password.len());
    for c in password.chars() {
        match c {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    format!("password: \"{escaped}\"\n")
}

// ---------------------------------------------------------------------------
// Secret hand-off
// ---------------------------------------------------------------------------

// WHAT:  A temp file holding a password, removed when this value drops.
// WHY:   mysqldump and mongodump have no environment variable for the password
//        that current versions still honour, and argv is world-readable.
// HOW:   Created exclusively (a pre-planted file or symlink is refused) with
//        mode 0600 on Unix; on Windows the per-user temp directory is already
//        private to the account. Dropped on every path out of a run — success,
//        failure, cancel or panic unwinding.
pub struct SecretFile {
    path: PathBuf,
}

impl SecretFile {
    pub fn create(extension: &str, contents: &str) -> AppResult<SecretFile> {
        let path = std::env::temp_dir().join(format!("db-free-{}.{extension}", uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|e| AppError::internal(format!("could not create a temp option file: {e}")))?;
        let secret = SecretFile { path };
        file.write_all(contents.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|e| AppError::internal(format!("could not write a temp option file: {e}")))?;
        Ok(secret)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SecretFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Process runner
// ---------------------------------------------------------------------------

pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Streamed into the child's stdin, with progress (mysql restore).
    pub stdin: Option<PathBuf>,
    /// A file the tool writes, whose size is reported as progress.
    pub output: Option<PathBuf>,
}

#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    // CREATE_NO_WINDOW: a GUI app must not flash a console per tool run.
    cmd.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

fn program_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

// WHAT:  Forwards one output stream line by line, keeping the last few lines.
// HOW:   Bytes, not `lines()`: tools on Windows print in the console code page,
//        and a UTF-8 error that stopped reading would fill the pipe and hang
//        the child.
async fn pump<R: AsyncRead + Unpin>(reader: R, sink: Arc<dyn ToolSink>) -> VecDeque<String> {
    let mut reader = BufReader::new(reader);
    let mut tail = VecDeque::with_capacity(TAIL_LINES);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_string();
                if line.trim().is_empty() {
                    continue;
                }
                sink.log(&line);
                if tail.len() == TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        }
    }
    tail
}

async fn feed(path: PathBuf, mut stdin: ChildStdin, sink: Arc<dyn ToolSink>) -> std::io::Result<()> {
    let mut file = tokio::fs::File::open(&path).await?;
    let total = file.metadata().await?.len();
    let mut buf = vec![0u8; 256 * 1024];
    let mut sent: u64 = 0;
    let mut reported = Instant::now();
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        stdin.write_all(&buf[..n]).await?;
        sent += n as u64;
        if reported.elapsed() >= PROGRESS_TICK {
            sink.progress(sent, Some(total));
            reported = Instant::now();
        }
    }
    sink.progress(sent, Some(total));
    stdin.shutdown().await
}

// WHAT:  Runs one tool to completion or cancellation, streaming its output.
// HOW:   argv only (no shell). The child is killed on cancel and on drop, so
//        an abandoned run never leaves a dump writing in the background. A
//        non-zero exit becomes a Driver error carrying the output's last lines.
pub async fn run(inv: Invocation, sink: Arc<dyn ToolSink>, mut cancel: oneshot::Receiver<()>) -> AppResult<RunEnd> {
    let name = program_name(&inv.program);
    let mut cmd = Command::new(&inv.program);
    cmd.args(&inv.args)
        .envs(inv.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(if inv.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    no_window(&mut cmd);
    // argv never holds a secret (see the header), so it is safe to echo.
    sink.log(&format!("$ {name} {}", inv.args.join(" ")));
    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::driver(format!("Could not start {}: {e}", inv.program.display())))?;

    let out_task = child.stdout.take().map(|s| tokio::spawn(pump(s, Arc::clone(&sink))));
    let err_task = child.stderr.take().map(|s| tokio::spawn(pump(s, Arc::clone(&sink))));
    let feed_task = match (inv.stdin.clone(), child.stdin.take()) {
        (Some(path), Some(stdin)) => Some(tokio::spawn(feed(path, stdin, Arc::clone(&sink)))),
        _ => None,
    };

    let mut ticker = tokio::time::interval(PROGRESS_TICK);
    let mut cancel_armed = true;
    let status = loop {
        tokio::select! {
            status = child.wait() => break Some(status.map_err(|e| AppError::driver(format!("{name} failed: {e}")))?),
            signal = &mut cancel, if cancel_armed => {
                if signal.is_ok() {
                    let _ = child.kill().await;
                    break None;
                }
                // The sender went away without cancelling: keep running.
                cancel_armed = false;
            }
            _ = ticker.tick() => {
                if let Some(out) = &inv.output {
                    if let Ok(meta) = tokio::fs::metadata(out).await {
                        if meta.is_file() {
                            sink.progress(meta.len(), None);
                        }
                    }
                }
            }
        }
    };

    let feed_result = match feed_task {
        Some(task) if status.is_some() => task.await.ok(),
        Some(task) => {
            task.abort();
            None
        }
        None => None,
    };
    let mut tail = VecDeque::new();
    for task in [err_task, out_task].into_iter().flatten() {
        if let Ok(lines) = task.await {
            if tail.is_empty() {
                tail = lines;
            }
        }
    }

    let Some(status) = status else {
        sink.log(&format!("{name} cancelled."));
        return Ok(RunEnd::Cancelled);
    };
    if !status.success() {
        let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "a signal".into());
        let detail: Vec<String> = tail.into_iter().collect();
        return Err(AppError::driver(format!("{name} exited with {code}.\n{}", detail.join("\n"))));
    }
    if let Some(Err(e)) = feed_result {
        return Err(AppError::driver(format!("Could not stream the file into {name}: {e}")));
    }
    if let Some(out) = &inv.output {
        if let Ok(meta) = tokio::fs::metadata(out).await {
            if meta.is_file() {
                sink.progress(meta.len(), Some(meta.len()));
            }
        }
    }
    Ok(RunEnd::Finished)
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

// WHAT:  One backup or restore, fully described.
pub struct Job<'a> {
    pub summary: &'a ConnectionSummary,
    pub secret: Option<&'a str>,
    pub options: &'a BackupOptions,
    /// The backup file (or directory) written or read.
    pub path: &'a Path,
    pub overrides: &'a BTreeMap<NativeTool, String>,
}

fn secret_of<'a>(job: &Job<'a>) -> Option<&'a str> {
    job.secret.filter(|s| !s.is_empty())
}

fn file_target(summary: &ConnectionSummary) -> AppResult<PathBuf> {
    nonblank(&summary.file_path)
        .map(PathBuf::from)
        .ok_or_else(|| AppError::invalid_input("This connection has no database file."))
}

/// A sidecar next to a database file: `app.db` + `-wal` → `app.db-wal`.
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

// WHAT:  Writes a backup. Returns how it ended and the tool that did the work.
pub async fn backup(job: &Job<'_>, method: BackupMethod, sink: Arc<dyn ToolSink>, cancel: oneshot::Receiver<()>) -> AppResult<(RunEnd, String)> {
    match method {
        BackupMethod::PgDump => {
            let program = require(NativeTool::PgDump, job.overrides)?;
            let inv = Invocation {
                args: pg_dump_args(job.summary, job.options, job.path),
                env: pg_env(job.summary, secret_of(job)),
                stdin: None,
                output: Some(job.path.to_path_buf()),
                program,
            };
            Ok((run(inv, sink, cancel).await?, NativeTool::PgDump.program().into()))
        }
        BackupMethod::Mysqldump => {
            let program = require(NativeTool::Mysqldump, job.overrides)?;
            let secret = secret_of(job).map(|p| SecretFile::create("cnf", &mysql_option_file(p))).transpose()?;
            let inv = Invocation {
                args: mysqldump_args(job.summary, job.options, job.path, secret.as_ref().map(SecretFile::path))?,
                env: Vec::new(),
                stdin: None,
                output: Some(job.path.to_path_buf()),
                program,
            };
            let end = run(inv, sink, cancel).await;
            drop(secret);
            Ok((end?, NativeTool::Mysqldump.program().into()))
        }
        BackupMethod::Mongodump => {
            let program = require(NativeTool::Mongodump, job.overrides)?;
            let secret = secret_of(job).map(|p| SecretFile::create("yaml", &mongo_config_file(p))).transpose()?;
            let inv = Invocation {
                args: mongodump_args(job.summary, job.options, job.path, secret.as_ref().map(SecretFile::path)),
                env: Vec::new(),
                stdin: None,
                output: Some(job.path.to_path_buf()),
                program,
            };
            let end = run(inv, sink, cancel).await;
            drop(secret);
            Ok((end?, NativeTool::Mongodump.program().into()))
        }
        BackupMethod::SqliteCopy => {
            let source = file_target(job.summary)?;
            // VACUUM INTO refuses an existing file; the save dialog already
            // asked the user whether to replace it.
            if job.path.is_file() {
                std::fs::remove_file(job.path).map_err(|e| AppError::invalid_input(format!("Cannot replace {}: {e}", job.path.display())))?;
            }
            sink.log(&format!("VACUUM INTO '{}'", job.path.display()));
            sqlite::vacuum_into(&source.to_string_lossy(), job.path).await?;
            Ok((RunEnd::Finished, "VACUUM INTO".into()))
        }
        BackupMethod::FileCopy => {
            let source = file_target(job.summary)?;
            sink.log(&format!("copy {} → {}", source.display(), job.path.display()));
            let bytes = tokio::fs::copy(&source, job.path)
                .await
                .map_err(|e| AppError::driver(format!("Copy failed: {e}")))?;
            let wal = sidecar(&source, ".wal");
            if wal.is_file() {
                tokio::fs::copy(&wal, sidecar(job.path, ".wal"))
                    .await
                    .map_err(|e| AppError::driver(format!("Copying the WAL failed: {e}")))?;
            }
            sink.progress(bytes, Some(bytes));
            Ok((RunEnd::Finished, "file copy".into()))
        }
    }
}

// WHAT:  Replaces the target database with the contents of a backup.
// WHY:   Destructive by definition; the guard (step 11) has already refused
//        read-only connections and demanded confirmation before this runs.
pub async fn restore(job: &Job<'_>, method: BackupMethod, sink: Arc<dyn ToolSink>, cancel: oneshot::Receiver<()>) -> AppResult<(RunEnd, String)> {
    match method {
        BackupMethod::PgDump => {
            let format = detect_pg_format(job.path)?;
            let (tool, args) = if format == BackupFormat::Plain {
                (NativeTool::Psql, psql_args(job.summary, job.options, job.path))
            } else {
                (NativeTool::PgRestore, pg_restore_args(job.summary, job.options, job.path))
            };
            sink.log(&format!("Detected a {} backup; restoring with {}.", format_label(format), tool.program()));
            let inv = Invocation { program: require(tool, job.overrides)?, args, env: pg_env(job.summary, secret_of(job)), stdin: None, output: None };
            Ok((run(inv, sink, cancel).await?, tool.program().into()))
        }
        BackupMethod::Mysqldump => {
            let program = require(NativeTool::Mysql, job.overrides)?;
            let secret = secret_of(job).map(|p| SecretFile::create("cnf", &mysql_option_file(p))).transpose()?;
            let inv = Invocation {
                args: mysql_restore_args(job.summary, secret.as_ref().map(SecretFile::path)),
                env: Vec::new(),
                stdin: Some(job.path.to_path_buf()),
                output: None,
                program,
            };
            let end = run(inv, sink, cancel).await;
            drop(secret);
            Ok((end?, NativeTool::Mysql.program().into()))
        }
        BackupMethod::Mongodump => {
            let program = require(NativeTool::Mongorestore, job.overrides)?;
            let gzip = is_gzip(job.path);
            let secret = secret_of(job).map(|p| SecretFile::create("yaml", &mongo_config_file(p))).transpose()?;
            let inv = Invocation {
                args: mongorestore_args(job.summary, job.options, job.path, gzip, secret.as_ref().map(SecretFile::path)),
                env: Vec::new(),
                stdin: None,
                output: None,
                program,
            };
            let end = run(inv, sink, cancel).await;
            drop(secret);
            Ok((end?, NativeTool::Mongorestore.program().into()))
        }
        BackupMethod::SqliteCopy => {
            sqlite::quick_check(job.path).await?;
            sink.log("Integrity check passed.");
            replace_file(job, &["-wal", "-shm", "-journal"], &sink).await?;
            Ok((RunEnd::Finished, "file copy".into()))
        }
        BackupMethod::FileCopy => {
            replace_file(job, &[".wal"], &sink).await?;
            Ok((RunEnd::Finished, "file copy".into()))
        }
    }
}

// WHAT:  Puts a backup file in place of the connection's database file.
// HOW:   Copies to a temp name beside the target, then renames over it, so a
//        failed copy never leaves a half-written database. Stale journal / WAL
//        sidecars are removed first: replayed onto the restored file they
//        would corrupt it.
async fn replace_file(job: &Job<'_>, sidecars: &[&str], sink: &Arc<dyn ToolSink>) -> AppResult<()> {
    let target = file_target(job.summary)?;
    if same_file(job.path, &target) {
        return Err(AppError::invalid_input("The backup file is the database file itself."));
    }
    let staging = sidecar(&target, ".restoring");
    sink.log(&format!("copy {} → {}", job.path.display(), target.display()));
    let bytes = tokio::fs::copy(job.path, &staging)
        .await
        .map_err(|e| AppError::driver(format!("Copy failed: {e}")))?;
    for suffix in sidecars {
        let stale = sidecar(&target, suffix);
        if stale.exists() {
            tokio::fs::remove_file(&stale)
                .await
                .map_err(|e| AppError::driver(format!("Cannot remove {}: {e}", stale.display())))?;
        }
    }
    // DuckDB: a WAL taken with the backup belongs with it.
    for suffix in sidecars.iter().filter(|s| **s == ".wal") {
        let with_backup = sidecar(job.path, suffix);
        if with_backup.is_file() {
            tokio::fs::copy(&with_backup, sidecar(&target, suffix))
                .await
                .map_err(|e| AppError::driver(format!("Copying the WAL failed: {e}")))?;
        }
    }
    if let Err(e) = tokio::fs::rename(&staging, &target).await {
        let _ = tokio::fs::remove_file(&staging).await;
        return Err(AppError::driver(format!("Cannot replace {}: {e}", target.display())));
    }
    sink.progress(bytes, Some(bytes));
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn format_label(format: BackupFormat) -> &'static str {
    match format {
        BackupFormat::Custom => "custom-format",
        BackupFormat::Plain => "plain SQL",
        BackupFormat::Directory => "directory-format",
        BackupFormat::Tar => "tar-format",
        BackupFormat::Archive => "archive",
        BackupFormat::File => "file",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionInput, Environment};
    use std::sync::Mutex;

    fn summary(engine: Engine, database: Option<&str>, username: Option<&str>) -> ConnectionSummary {
        let input = ConnectionInput {
            name: "t".into(),
            engine,
            environment: Environment::Local,
            read_only: false,
            host: Some("db.internal".into()),
            port: None,
            database: database.map(String::from),
            username: username.map(String::from),
            password: None,
            file_path: None,
            ssl_mode: SslMode::Require,
        };
        ConnectionSummary::draft(&input, true)
    }

    fn options() -> BackupOptions {
        BackupOptions {
            format: BackupFormat::Custom,
            schema_only: false,
            data_only: false,
            schemas: vec![],
            tables: vec![],
            clean: false,
            no_owner: false,
            routines: true,
            triggers: true,
            single_transaction: true,
            gzip: true,
            drop: false,
        }
    }

    #[test]
    fn pg_dump_args_are_flag_value_pairs() {
        let s = summary(Engine::Postgres, Some("shop"), Some("alice"));
        let mut o = options();
        o.schemas = vec!["public".into(), " ".into()];
        o.tables = vec!["public.orders".into()];
        o.no_owner = true;
        o.clean = true; // ignored for custom format
        let args = pg_dump_args(&s, &o, Path::new("/tmp/shop.dump"));
        assert_eq!(
            args,
            vec![
                "--verbose",
                "--format=c",
                "--file=/tmp/shop.dump",
                "--host=db.internal",
                "--port=5432",
                "--username=alice",
                "--dbname=shop",
                "--no-password",
                "--schema=public",
                "--table=public.orders",
                "--no-owner",
            ]
        );
        o.format = BackupFormat::Plain;
        o.schema_only = true;
        let plain = pg_dump_args(&s, &o, Path::new("/tmp/shop.sql"));
        assert!(plain.contains(&"--format=p".to_string()));
        assert!(plain.contains(&"--schema-only".to_string()));
        assert!(plain.contains(&"--clean".to_string()) && plain.contains(&"--if-exists".to_string()));
    }

    #[test]
    fn pg_password_travels_in_the_environment_only() {
        let s = summary(Engine::Postgres, Some("shop"), Some("alice"));
        let args = pg_dump_args(&s, &options(), Path::new("/tmp/x"));
        assert!(!args.iter().any(|a| a.contains("s3cret")));
        let env = pg_env(&s, Some("s3cret"));
        assert!(env.contains(&("PGPASSWORD".into(), "s3cret".into())));
        assert!(env.contains(&("PGSSLMODE".into(), "require".into())));
        assert!(!pg_env(&s, None).iter().any(|(k, _)| k == "PGPASSWORD"));
    }

    #[test]
    fn pg_dbname_cannot_become_a_connection_string() {
        assert_eq!(pg_dbname("shop"), "shop");
        assert_eq!(pg_dbname("host=evil"), "dbname='host=evil'");
        assert_eq!(pg_dbname("o'brien=x"), "dbname='o\\'brien=x'");
        assert_eq!(pg_dbname("postgres://evil/db"), "dbname='postgres://evil/db'");
    }

    #[test]
    fn pg_restore_and_psql_args() {
        let s = summary(Engine::Supabase, None, None);
        let mut o = options();
        o.clean = true;
        let args = pg_restore_args(&s, &o, Path::new("-weird.dump"));
        assert!(args.contains(&"--dbname=postgres".to_string()), "no database falls back to postgres");
        assert!(args.contains(&"--clean".to_string()));
        assert!(args.contains(&"--single-transaction".to_string()));
        assert_eq!(&args[args.len() - 2..], &["--".to_string(), "-weird.dump".to_string()]);
        let psql = psql_args(&s, &o, Path::new("/b/x.sql"));
        assert!(psql.contains(&"--set=ON_ERROR_STOP=1".to_string()));
        assert_eq!(psql.last().map(String::as_str), Some("--file=/b/x.sql"));
    }

    #[test]
    fn mysqldump_args_put_the_option_file_first() {
        let s = summary(Engine::Mariadb, Some("shop"), Some("root"));
        let mut o = options();
        o.tables = vec!["orders".into(), "items".into()];
        o.triggers = false;
        o.data_only = true;
        let args = mysqldump_args(&s, &o, Path::new("/b/shop.sql"), Some(Path::new("/tmp/p.cnf"))).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(args.first().map(String::as_str), Some("--defaults-extra-file=/tmp/p.cnf"));
        assert!(args.contains(&"--port=3306".to_string()));
        assert!(args.contains(&"--user=root".to_string()));
        assert!(args.contains(&"--skip-triggers".to_string()));
        assert!(args.contains(&"--no-create-info".to_string()));
        assert!(args.contains(&"--result-file=/b/shop.sql".to_string()));
        assert_eq!(&args[args.len() - 3..], &["shop".to_string(), "orders".to_string(), "items".to_string()]);

        let all = mysqldump_args(&summary(Engine::Mysql, None, None), &options(), Path::new("/b/all.sql"), None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(all.last().map(String::as_str), Some("--all-databases"));
        assert!(!all.iter().any(|a| a.starts_with("--defaults-extra-file")));

        let mut evil = options();
        evil.tables = vec!["--where=1".into()];
        assert!(mysqldump_args(&s, &evil, Path::new("/b/x.sql"), None).is_err(), "a positional name cannot be an option");

        let restore = mysql_restore_args(&s, Some(Path::new("/tmp/p.cnf")));
        assert_eq!(restore.first().map(String::as_str), Some("--defaults-extra-file=/tmp/p.cnf"));
        assert_eq!(restore.last().map(String::as_str), Some("--database=shop"));
    }

    #[test]
    fn option_files_escape_the_password() {
        assert_eq!(mysql_option_file("a\\b\"c\nd"), "[client]\npassword=\"a\\\\b\"c\\nd\"\n");
        assert_eq!(mongo_config_file("a\\b\"c"), "password: \"a\\\\b\\\"c\"\n");
    }

    #[test]
    fn mongo_args() {
        let s = summary(Engine::Mongodb, Some("app"), Some("admin"));
        let mut o = options();
        o.tables = vec!["users".into()];
        let dump = mongodump_args(&s, &o, Path::new("/b/app.archive"), Some(Path::new("/tmp/m.yaml")));
        assert_eq!(
            dump,
            vec![
                "--host=db.internal",
                "--port=27017",
                "--username=admin",
                "--authenticationDatabase=admin",
                "--config=/tmp/m.yaml",
                "--ssl",
                "--db=app",
                "--collection=users",
                "--archive=/b/app.archive",
                "--gzip",
            ]
        );
        o.tables = vec!["a".into(), "b".into()];
        assert!(!mongodump_args(&s, &o, Path::new("/b/x"), None).iter().any(|a| a.starts_with("--collection")));
        o.drop = true;
        let restore = mongorestore_args(&s, &o, Path::new("/b/x"), false, None);
        assert!(restore.contains(&"--drop".to_string()));
        assert!(!restore.contains(&"--gzip".to_string()));
    }

    #[test]
    fn engines_map_to_methods() {
        assert_eq!(method_for(Engine::Neon), Some(BackupMethod::PgDump));
        assert_eq!(method_for(Engine::Cockroachdb), None, "wire-compatible is not dump-compatible");
        assert_eq!(method_for(Engine::Tidb), Some(BackupMethod::Mysqldump));
        assert_eq!(method_for(Engine::Spatialite), Some(BackupMethod::SqliteCopy));
        assert_eq!(method_for(Engine::Duckdb), Some(BackupMethod::FileCopy));
        assert_eq!(method_for(Engine::Redis), None);
        assert_eq!(formats_for(BackupMethod::PgDump).first(), Some(&BackupFormat::Custom));
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("db-free-native-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir
    }

    #[test]
    fn detects_pg_archive_formats() {
        let dir = scratch("fmt");
        let custom = dir.join("a.dump");
        std::fs::write(&custom, b"PGDMP\x01\x0e\x00rest").unwrap_or_else(|e| panic!("{e}"));
        let mut tar = vec![0u8; 600];
        tar[257..262].copy_from_slice(b"ustar");
        let tar_path = dir.join("a.tar");
        std::fs::write(&tar_path, &tar).unwrap_or_else(|e| panic!("{e}"));
        let sql = dir.join("a.sql");
        std::fs::write(&sql, b"CREATE TABLE t (a int);").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(detect_pg_format(&custom).ok(), Some(BackupFormat::Custom));
        assert_eq!(detect_pg_format(&tar_path).ok(), Some(BackupFormat::Tar));
        assert_eq!(detect_pg_format(&sql).ok(), Some(BackupFormat::Plain));
        assert_eq!(detect_pg_format(&dir).ok(), Some(BackupFormat::Directory));
        assert!(detect_pg_format(&dir.join("missing")).is_err());
        let gz = dir.join("a.gz");
        std::fs::write(&gz, [0x1f, 0x8b, 0x08]).unwrap_or_else(|e| panic!("{e}"));
        assert!(is_gzip(&gz));
        assert!(!is_gzip(&sql));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locate_honours_an_override_directory_and_reports_a_bad_one() {
        let dir = scratch("locate");
        let exe = dir.join(executable_names("pg_dump").remove(0));
        std::fs::write(&exe, b"").unwrap_or_else(|e| panic!("{e}"));
        let mut overrides = BTreeMap::new();
        overrides.insert(NativeTool::PgDump, dir.to_string_lossy().into_owned());
        assert_eq!(locate(NativeTool::PgDump, &overrides), (Some(exe.clone()), true));
        overrides.insert(NativeTool::PgDump, exe.to_string_lossy().into_owned());
        assert_eq!(locate(NativeTool::PgDump, &overrides), (Some(exe), true));
        overrides.insert(NativeTool::PgDump, dir.join("nope").to_string_lossy().into_owned());
        assert_eq!(locate(NativeTool::PgDump, &overrides), (None, true));
        let message = not_found(NativeTool::PgDump).to_string();
        assert!(message.starts_with("pg_dump not found — install PostgreSQL client tools"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn secret_file_is_removed_on_drop() {
        let file = SecretFile::create("cnf", "[client]\npassword=\"x\"\n").unwrap_or_else(|e| panic!("{e}"));
        let path = file.path().to_path_buf();
        assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), "[client]\npassword=\"x\"\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).map(|m| m.permissions().mode() & 0o777).unwrap_or_default();
            assert_eq!(mode, 0o600);
        }
        drop(file);
        assert!(!path.exists());
    }

    #[derive(Default)]
    struct Collect(Mutex<Vec<String>>);

    impl ToolSink for Collect {
        fn log(&self, line: &str) {
            if let Ok(mut lines) = self.0.lock() {
                lines.push(line.to_string());
            }
        }
        fn progress(&self, _bytes: u64, _total: Option<u64>) {}
    }

    fn shell(script: &str) -> (PathBuf, Vec<String>) {
        if cfg!(windows) {
            (PathBuf::from("cmd"), vec!["/C".into(), script.into()])
        } else {
            (PathBuf::from("sh"), vec!["-c".into(), script.into()])
        }
    }

    #[tokio::test]
    async fn runner_streams_output_and_reports_failure() {
        let sink = Arc::new(Collect::default());
        let (program, args) = shell("echo hello");
        let (_tx, rx) = oneshot::channel();
        let end = run(Invocation { program, args, env: vec![], stdin: None, output: None }, sink.clone(), rx).await;
        assert_eq!(end.ok(), Some(RunEnd::Finished));
        assert!(sink.0.lock().map(|l| l.iter().any(|x| x.trim() == "hello")).unwrap_or(false));

        let (program, args) = shell("echo broken 1>&2 && exit 3");
        let (_tx, rx) = oneshot::channel();
        let err = run(Invocation { program, args, env: vec![], stdin: None, output: None }, Arc::new(Collect::default()), rx).await.err();
        assert!(matches!(&err, Some(AppError::Driver { message }) if message.contains("exited with 3") && message.contains("broken")));
    }

    #[tokio::test]
    async fn runner_kills_the_child_on_cancel() {
        let (program, args) = if cfg!(windows) {
            (PathBuf::from("ping"), vec!["-n".into(), "30".into(), "127.0.0.1".into()])
        } else {
            (PathBuf::from("sleep"), vec!["30".into()])
        };
        let (tx, rx) = oneshot::channel();
        let started = Instant::now();
        let handle = tokio::spawn(run(Invocation { program, args, env: vec![], stdin: None, output: None }, Arc::new(Collect::default()), rx));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = tx.send(());
        let end = handle.await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(end.ok(), Some(RunEnd::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn sqlite_backup_and_restore_round_trip() {
        let dir = scratch("sqlite");
        let db = dir.join("live.db");
        let backup_path = dir.join("live.backup.db");
        {
            let conn = crate::integrations::sqlite::connect(&crate::model::ResolvedConnection {
                summary: file_summary(&db),
                secret: None,
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
            conn.execute("CREATE TABLE t (a INTEGER); INSERT INTO t VALUES (1);", 10).await.unwrap_or_else(|e| panic!("{e}"));
            conn.close().await;
        }
        let s = file_summary(&db);
        let overrides = BTreeMap::new();
        let mut o = options();
        o.format = BackupFormat::File;
        let job = Job { summary: &s, secret: None, options: &o, path: &backup_path, overrides: &overrides };
        let (_tx, rx) = oneshot::channel();
        let (end, tool) = backup(&job, BackupMethod::SqliteCopy, Arc::new(Collect::default()), rx).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((end, tool.as_str()), (RunEnd::Finished, "VACUUM INTO"));

        // Change the live file, then restore the backup over it.
        std::fs::write(&db, b"").unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(sidecar(&db, "-wal"), b"stale").unwrap_or_else(|e| panic!("{e}"));
        let (_tx, rx) = oneshot::channel();
        restore(&job, BackupMethod::SqliteCopy, Arc::new(Collect::default()), rx).await.unwrap_or_else(|e| panic!("{e}"));
        assert!(!sidecar(&db, "-wal").exists(), "a stale WAL must not survive a restore");
        assert!(!sidecar(&db, ".restoring").exists());
        crate::integrations::sqlite::quick_check(&db).await.unwrap_or_else(|e| panic!("{e}"));
        assert!(std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0) > 0);

        // Restoring a file onto itself is refused.
        let self_job = Job { summary: &s, secret: None, options: &o, path: &db, overrides: &overrides };
        let (_tx, rx) = oneshot::channel();
        assert!(restore(&self_job, BackupMethod::SqliteCopy, Arc::new(Collect::default()), rx).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn file_summary(path: &Path) -> ConnectionSummary {
        let input = ConnectionInput {
            name: "f".into(),
            engine: Engine::Sqlite,
            environment: Environment::Local,
            read_only: false,
            host: None,
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: Some(path.to_string_lossy().into_owned()),
            ssl_mode: SslMode::Disable,
        };
        ConnectionSummary::draft(&input, false)
    }
}
