// SOT: backup-service, native-backup-orchestration, backup-tool-detection, restore-service

use crate::error::{AppError, AppResult};
use crate::integrations::native_tools::{self, Job, RunEnd};
use crate::model::{BackupMethod, BackupOptions, BackupReport, BackupSupport, ConnectionSummary, NativeTool};
use crate::services::{connection, settings};
use crate::state::AppState;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::oneshot;

pub use crate::integrations::native_tools::ToolSink;

// WHAT:  Backup / restore with the engine's own tools, for one connection.
// WHY:   The integration knows how to run pg_dump; this layer knows which
//        connection, which secret, which tool paths the user configured, and
//        the rules that are about the app rather than the tool (file engines
//        must not be replaced under a live session).
// HOW:   The guard (step 11) has already resolved the connection and, for a
//        restore, refused read-only targets and demanded confirmation.
// WHERE: src-tauri/src/integrations/native_tools.rs, src-tauri/src/commands/backup.rs

// WHAT:  Which tools exist, for a connection's engine (the dialog) or for every
//        tool (Settings → Advanced, `connection_id: None`).
pub async fn support(state: &AppState, connection_id: Option<&str>) -> AppResult<BackupSupport> {
    let overrides = settings::get(state)?.native_tool_paths;
    let Some(id) = connection_id else {
        let tools = futures::future::join_all(NativeTool::ALL.iter().map(|t| native_tools::status(*t, &overrides))).await;
        return Ok(BackupSupport {
            method: None,
            formats: Vec::new(),
            tools,
            restore_needs_disconnect: false,
            backup_needs_disconnect: false,
            note: None,
        });
    };
    let summary = state.with_store(|store| store.get_connection(id))?;
    let Some(method) = native_tools::method_for(summary.engine) else {
        return Ok(BackupSupport {
            method: None,
            formats: Vec::new(),
            tools: Vec::new(),
            restore_needs_disconnect: false,
            backup_needs_disconnect: false,
            note: Some(native_tools::unsupported_note(summary.engine)),
        });
    };
    let tools = futures::future::join_all(native_tools::tools_for(method).iter().map(|t| native_tools::status(*t, &overrides))).await;
    let file_engine = matches!(method, BackupMethod::SqliteCopy | BackupMethod::FileCopy);
    Ok(BackupSupport {
        method: Some(method),
        formats: native_tools::formats_for(method),
        tools,
        restore_needs_disconnect: file_engine,
        backup_needs_disconnect: method == BackupMethod::FileCopy,
        note: None,
    })
}

fn absolute(path: &str) -> AppResult<&Path> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_input("Choose a backup file first."));
    }
    let p = Path::new(trimmed);
    if !p.is_absolute() {
        return Err(AppError::invalid_input("The backup path must be absolute."));
    }
    Ok(p)
}

fn method_of(summary: &ConnectionSummary) -> AppResult<BackupMethod> {
    native_tools::method_for(summary.engine).ok_or_else(|| AppError::invalid_input(native_tools::unsupported_note(summary.engine)))
}

async fn refuse_live_session(state: &AppState, summary: &ConnectionSummary, doing: &str) -> AppResult<()> {
    if state.session(&summary.id).await.is_some() {
        return Err(AppError::invalid_input(format!(
            "Disconnect \"{}\" before {doing} — the app has its database file open.",
            summary.name
        )));
    }
    Ok(())
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn size_of(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().filter(|m| m.is_file()).map(|m| m.len())
}

// WHAT:  Writes a backup of the connection's database to `path`.
// HOW:   A cancelled or failed run leaves no partial backup behind — a file
//        that looks like a backup but is not one is worse than no file. A
//        directory is only removed when this run created it.
pub async fn backup(
    state: &AppState,
    summary: &ConnectionSummary,
    path: &str,
    options: &BackupOptions,
    sink: Arc<dyn ToolSink>,
    cancel: oneshot::Receiver<()>,
) -> AppResult<BackupReport> {
    let path = absolute(path)?;
    let method = method_of(summary)?;
    if !native_tools::formats_for(method).contains(&options.format) {
        return Err(AppError::invalid_input("That backup format is not available for this engine."));
    }
    if options.schema_only && options.data_only {
        return Err(AppError::invalid_input("Choose schema only or data only, not both."));
    }
    if method == BackupMethod::FileCopy {
        refuse_live_session(state, summary, "backing it up").await?;
    }
    let secret = connection::resolve(state, &summary.id)?.connection.secret;
    let overrides = settings::get(state)?.native_tool_paths;
    let dir_existed = path.is_dir();
    let started = Instant::now();
    let job = Job { summary, secret: secret.as_deref(), options, path, overrides: &overrides };
    let outcome = native_tools::backup(&job, method, sink, cancel).await;
    let cancelled = matches!(outcome, Ok((RunEnd::Cancelled, _)));
    if outcome.is_err() || cancelled {
        if path.is_file() {
            let _ = std::fs::remove_file(path);
        } else if path.is_dir() && !dir_existed {
            let _ = std::fs::remove_dir_all(path);
        }
    }
    let (_, tool) = outcome?;
    Ok(BackupReport {
        path: path.to_string_lossy().into_owned(),
        bytes: if cancelled { None } else { size_of(path) },
        elapsed_ms: elapsed_ms(started),
        tool,
        cancelled,
    })
}

// WHAT:  Restores the connection's database from the backup at `path`.
pub async fn restore(
    state: &AppState,
    summary: &ConnectionSummary,
    path: &str,
    options: &BackupOptions,
    sink: Arc<dyn ToolSink>,
    cancel: oneshot::Receiver<()>,
) -> AppResult<BackupReport> {
    let path = absolute(path)?;
    if !path.exists() {
        return Err(AppError::not_found(format!("{} does not exist.", path.display())));
    }
    let method = method_of(summary)?;
    if matches!(method, BackupMethod::SqliteCopy | BackupMethod::FileCopy) {
        refuse_live_session(state, summary, "restoring it").await?;
    }
    let secret = connection::resolve(state, &summary.id)?.connection.secret;
    let overrides = settings::get(state)?.native_tool_paths;
    let started = Instant::now();
    let job = Job { summary, secret: secret.as_deref(), options, path, overrides: &overrides };
    let (end, tool) = native_tools::restore(&job, method, sink, cancel).await?;
    Ok(BackupReport {
        path: path.to_string_lossy().into_owned(),
        bytes: size_of(path),
        elapsed_ms: elapsed_ms(started),
        tool,
        cancelled: end == RunEnd::Cancelled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::{BackupFormat, ConnectionInput, Engine, Environment, SslMode};
    use crate::store::Store;

    struct Quiet;
    impl ToolSink for Quiet {
        fn log(&self, _line: &str) {}
        fn progress(&self, _bytes: u64, _total: Option<u64>) {}
    }

    fn options(format: BackupFormat) -> BackupOptions {
        BackupOptions {
            format,
            schema_only: false,
            data_only: false,
            schemas: vec![],
            tables: vec![],
            clean: false,
            no_owner: false,
            routines: false,
            triggers: true,
            single_transaction: false,
            gzip: false,
            drop: false,
        }
    }

    fn state_with(engine: Engine, file: Option<&Path>) -> (AppState, ConnectionSummary) {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let input = ConnectionInput {
            name: "b".into(),
            engine,
            environment: Environment::Local,
            read_only: false,
            host: Some("localhost".into()),
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: file.map(|p| p.to_string_lossy().into_owned()),
            ssl_mode: SslMode::Disable,
            ssh: crate::model::SshTunnel::default(),
            ssh_secret: None,
            folder: None,
            color: None,
            favorite: false,
        };
        let summary = store.insert_connection(&input, None).unwrap_or_else(|e| panic!("{e}"));
        (AppState::new(store, Box::new(MemoryKeyProvider::default())), summary)
    }

    #[tokio::test]
    async fn support_describes_the_engine() {
        let (state, pg) = state_with(Engine::Postgres, None);
        let s = support(&state, Some(&pg.id)).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.method, Some(BackupMethod::PgDump));
        assert_eq!(s.tools.len(), 3);
        let all = support(&state, None).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(all.tools.len(), NativeTool::ALL.len());
    }

    #[tokio::test]
    async fn backup_rejects_bad_requests_and_restore_needs_a_disconnected_file() {
        let dir = std::env::temp_dir().join(format!("db-free-backup-svc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let db = dir.join("a.db");
        std::fs::File::create(&db).unwrap_or_else(|e| panic!("{e}"));
        let (state, summary) = state_with(Engine::Sqlite, Some(&db));

        let (_tx, rx) = oneshot::channel();
        let relative = backup(&state, &summary, "a.db", &options(BackupFormat::File), Arc::new(Quiet), rx).await;
        assert!(matches!(relative, Err(AppError::InvalidInput { .. })));
        let (_tx, rx) = oneshot::channel();
        let target = dir.join("out.db").to_string_lossy().into_owned();
        let wrong = backup(&state, &summary, &target, &options(BackupFormat::Custom), Arc::new(Quiet), rx).await;
        assert!(matches!(wrong, Err(AppError::InvalidInput { .. })), "custom is a pg_dump format");

        let (_tx, rx) = oneshot::channel();
        let report = backup(&state, &summary, &target, &options(BackupFormat::File), Arc::new(Quiet), rx).await.unwrap_or_else(|e| panic!("{e}"));
        assert!(!report.cancelled);
        assert!(report.bytes.is_some_and(|b| b > 0));

        let resolved = crate::model::ResolvedConnection { summary: summary.clone(), secret: None };
        let live = crate::integrations::connect(&resolved).await.unwrap_or_else(|e| panic!("{e}"));
        state.insert_session(summary.id.clone(), live).await;
        let (_tx, rx) = oneshot::channel();
        let blocked = restore(&state, &summary, &target, &options(BackupFormat::File), Arc::new(Quiet), rx).await;
        assert!(matches!(blocked, Err(AppError::InvalidInput { ref message }) if message.contains("Disconnect")));
        state.remove_session(&summary.id).await;
        let (_tx, rx) = oneshot::channel();
        restore(&state, &summary, &target, &options(BackupFormat::File), Arc::new(Quiet), rx).await.unwrap_or_else(|e| panic!("{e}"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
