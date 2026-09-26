// SOT: backup-model, native-tool, native-tool-status, backup-options, backup-format, backup-method, backup-event, backup-report, backup-support

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// WHAT:  The external client programs the app knows how to drive.
// WHY:   A backup made with the engine's own dumper is the one its restorer (and
//        every DBA) trusts; re-implementing pg_dump's catalog walk would be both
//        worse and never finished. The enum is the registry: discovery, the
//        Settings → Advanced path overrides and the argument builders all key on it.
// HOW:   Serialised snake_case, which is also the key in
//        `AppSettings::native_tool_paths`.
// WHERE: src-tauri/src/integrations/native_tools.rs (discovery + args)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NativeTool {
    PgDump,
    PgRestore,
    Psql,
    Mysqldump,
    Mysql,
    Mongodump,
    Mongorestore,
}

impl NativeTool {
    pub const ALL: [NativeTool; 7] = [
        NativeTool::PgDump,
        NativeTool::PgRestore,
        NativeTool::Psql,
        NativeTool::Mysqldump,
        NativeTool::Mysql,
        NativeTool::Mongodump,
        NativeTool::Mongorestore,
    ];

    /// The executable name, without a platform extension.
    pub fn program(self) -> &'static str {
        match self {
            NativeTool::PgDump => "pg_dump",
            NativeTool::PgRestore => "pg_restore",
            NativeTool::Psql => "psql",
            NativeTool::Mysqldump => "mysqldump",
            NativeTool::Mysql => "mysql",
            NativeTool::Mongodump => "mongodump",
            NativeTool::Mongorestore => "mongorestore",
        }
    }

    /// Names to look for on PATH, in order. MariaDB 11 ships `mariadb-dump` /
    /// `mariadb` and may drop the mysql-named symlinks.
    pub fn candidates(self) -> &'static [&'static str] {
        match self {
            NativeTool::Mysqldump => &["mysqldump", "mariadb-dump"],
            NativeTool::Mysql => &["mysql", "mariadb"],
            NativeTool::PgDump => &["pg_dump"],
            NativeTool::PgRestore => &["pg_restore"],
            NativeTool::Psql => &["psql"],
            NativeTool::Mongodump => &["mongodump"],
            NativeTool::Mongorestore => &["mongorestore"],
        }
    }

    /// What to install when the tool is missing.
    pub fn package(self) -> &'static str {
        match self {
            NativeTool::PgDump | NativeTool::PgRestore | NativeTool::Psql => "PostgreSQL client tools",
            NativeTool::Mysqldump | NativeTool::Mysql => "MySQL or MariaDB client tools",
            NativeTool::Mongodump | NativeTool::Mongorestore => "MongoDB Database Tools",
        }
    }
}

// WHAT:  How one engine is backed up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BackupMethod {
    /// pg_dump; restored with pg_restore (archives) or psql (plain SQL).
    PgDump,
    /// mysqldump; restored by piping the file into mysql.
    Mysqldump,
    /// mongodump --archive; restored with mongorestore --archive.
    Mongodump,
    /// SQLite `VACUUM INTO` — a consistent copy even while the file is open.
    SqliteCopy,
    /// Plain file copy (DuckDB), only while the app holds no session on it.
    FileCopy,
}

// WHAT:  What the backup file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BackupFormat {
    /// pg_dump -Fc: compressed, selectively restorable with pg_restore.
    Custom,
    /// A SQL script (pg_dump -Fp, mysqldump).
    Plain,
    /// pg_dump -Fd: one file per table in a directory.
    Directory,
    /// pg_dump -Ft.
    Tar,
    /// mongodump --archive.
    Archive,
    /// A copy of the database file itself (SQLite / DuckDB).
    File,
}

// WHAT:  Whether one tool was found, and where.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NativeToolStatus {
    pub tool: NativeTool,
    /// Resolved executable, when found.
    pub path: Option<String>,
    /// First line of `--version`, when it answered.
    pub version: Option<String>,
    /// True when the path came from Settings → Advanced rather than PATH.
    pub overridden: bool,
    /// What to install, for the "not found" message.
    pub package: String,
}

// WHAT:  Everything the Backup / Restore dialog needs to draw itself for one
//        connection (or, with no connection, every tool for Settings).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupSupport {
    /// None: this engine has no backup path in the app.
    pub method: Option<BackupMethod>,
    /// Formats a backup can be written in; the first is the default.
    pub formats: Vec<BackupFormat>,
    /// The tools this method uses, found or not.
    pub tools: Vec<NativeToolStatus>,
    /// True when the connection must be disconnected before a restore
    /// (file engines: the file is replaced underneath the session).
    pub restore_needs_disconnect: bool,
    /// True when the connection must be disconnected before a backup (DuckDB
    /// holds an exclusive lock on its file while open).
    pub backup_needs_disconnect: bool,
    /// Why there is no method, or a caveat about the one there is.
    pub note: Option<String>,
}

// WHAT:  The knobs a backup or restore takes. Engines ignore the ones that do
//        not apply to them (the dialog only shows the relevant ones).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupOptions {
    pub format: BackupFormat,
    pub schema_only: bool,
    pub data_only: bool,
    /// Postgres: limit to these schemas.
    pub schemas: Vec<String>,
    /// Postgres / MySQL: limit to these tables.
    pub tables: Vec<String>,
    /// Postgres: drop objects before recreating them (plain dump / pg_restore).
    pub clean: bool,
    /// Postgres: skip ALTER OWNER, so a restore works as a different role.
    pub no_owner: bool,
    /// MySQL: include stored procedures and functions.
    pub routines: bool,
    /// MySQL: include triggers.
    pub triggers: bool,
    /// MySQL dump: one consistent snapshot (InnoDB). Postgres / MySQL restore:
    /// all-or-nothing.
    pub single_transaction: bool,
    /// MongoDB: gzip the archive.
    pub gzip: bool,
    /// MongoDB restore: drop each collection before restoring it.
    pub drop: bool,
}

// WHAT:  What a finished (or cancelled) run reports.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupReport {
    pub path: String,
    /// Size of the backup written or read, when it is a single file.
    pub bytes: Option<u64>,
    pub elapsed_ms: u64,
    /// The program that did the work ("pg_dump", "VACUUM INTO", …).
    pub tool: String,
    /// True when the user stopped the run; a partial backup file is removed.
    pub cancelled: bool,
}

// WHAT:  Streamed while a backup or restore runs, on the "backup:progress" event.
// WHY:   A dump of a real database takes minutes; the tool's own verbose output
//        is the most honest progress report there is, and bytes written is the
//        only number every tool agrees on.
// WHERE: src-tauri/src/commands/backup.rs (emits), src/lib/ipc.ts (onBackupProgress)
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum BackupEvent {
    #[serde(rename_all = "camelCase")]
    Log { run_id: String, line: String },
    #[serde(rename_all = "camelCase")]
    Progress { run_id: String, bytes: u64, total: Option<u64> },
}
