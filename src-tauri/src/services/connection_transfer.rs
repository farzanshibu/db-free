// SOT: connection-export, connection-import, connection-dedupe, connection-file-write

use crate::error::{AppError, AppResult};
use crate::model::{AuthSecrets, ConnectionExportReport, ConnectionImportReport, ConnectionInput, ConnectionSkipped, ConnectionSummary, FormKind};
use crate::services::connection::{self, Resolved};
use crate::services::connection_formats::{self, DbFreeConnection, DbFreeFile, DbFreeTag, ImportedConnection, FORMAT_NAME, FORMAT_VERSION};
use crate::state::AppState;
use std::collections::HashSet;
use std::path::Path;

// ============================================================================
// WHAT:  Export saved connections to a JSON file and import them back (or from
//        DB Pro / Navicat).
// WHY:   Moving to a new machine or sharing a team's list should not mean
//        retyping forty connections.
// HOW:   Export decrypts a password only when the caller opted in, and writes it
//        straight to the file chosen in the save dialog: the plaintext never
//        crosses to the UI. Without the opt-in the file carries no secret at all.
//        Import parses the whole file first (nothing is written if it is
//        invalid), skips connections already saved (same engine, host, port,
//        database and user; same file for file databases), seals any password
//        with this machine's key and reports what it did.
// WHERE: src-tauri/src/services/connection_formats.rs (the formats),
//        src/features/connections/ConnectionTransferDialogs.tsx
// ============================================================================

// ----------------------------------------------------------------------------
// Export
// ----------------------------------------------------------------------------

pub fn export(state: &AppState, ids: &[String], include_passwords: bool, path: &str) -> AppResult<ConnectionExportReport> {
    if ids.is_empty() {
        return Err(AppError::invalid_input("Choose at least one connection to export."));
    }
    let target = path.trim();
    if target.is_empty() {
        return Err(AppError::invalid_input("Choose where to save the export."));
    }
    let org = state.with_store(|store| store.list_connection_org())?;
    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        let Resolved { connection: resolved, ssh_secret, auth_secrets } = connection::resolve(state, id)?;
        let summary = resolved.summary;
        if summary.extra.transient {
            continue;
        }
        let folders = summary
            .extra
            .folder_ids
            .iter()
            .filter_map(|fid| org.folders.iter().find(|f| &f.id == fid).map(|f| f.name.clone()))
            .collect();
        let tags = summary
            .extra
            .tag_ids
            .iter()
            .filter_map(|tid| org.tags.iter().find(|t| &t.id == tid).map(|t| DbFreeTag { name: t.name.clone(), color: t.color }))
            .collect();
        entries.push(DbFreeConnection {
            name: summary.name,
            engine: summary.engine,
            environment: summary.environment,
            read_only: summary.read_only,
            host: summary.host,
            port: summary.port,
            database: summary.database,
            username: summary.username,
            password: if include_passwords { resolved.secret } else { None },
            file_path: summary.file_path,
            ssl_mode: summary.ssl_mode,
            ssh: summary.ssh,
            ssh_secret: if include_passwords { ssh_secret } else { None },
            color: summary.color,
            favorite: summary.favorite,
            uri_params: summary.extra.uri_params,
            srv: summary.extra.srv,
            tls: summary.extra.tls,
            auth: summary.auth,
            auth_secrets: if include_passwords { auth_secrets } else { AuthSecrets::default() },
            folders,
            tags,
        });
    }
    if entries.is_empty() {
        return Err(AppError::invalid_input("Files opened for a single session are not exported; save them as connections first."));
    }
    let count = u32::try_from(entries.len()).unwrap_or(u32::MAX);
    let file = DbFreeFile {
        format: FORMAT_NAME.to_string(),
        version: FORMAT_VERSION,
        exported_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        includes_passwords: include_passwords,
        connections: entries,
    };
    let json = serde_json::to_string_pretty(&file).map_err(AppError::internal)?;
    write_private_file(Path::new(target), json.as_bytes(), include_passwords)?;
    Ok(ConnectionExportReport { path: target.to_string(), count, includes_passwords: include_passwords })
}

// WHAT:  Writes the export; a file holding passwords is owner-only on Unix.
fn write_private_file(path: &Path, bytes: &[u8], private: bool) -> AppResult<()> {
    use std::io::Write;
    let fail = |e: std::io::Error| AppError::invalid_input(format!("Could not write {}: {e}", path.display()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    owner_only_on_create(&mut options, private);
    let mut file = options.open(path).map_err(fail)?;
    file.write_all(bytes).map_err(fail)?;
    // `mode` only applies to a file this call created; an existing one keeps its old bits.
    owner_only(path, private);
    Ok(())
}

#[cfg(unix)]
fn owner_only_on_create(options: &mut std::fs::OpenOptions, private: bool) {
    use std::os::unix::fs::OpenOptionsExt;
    if private {
        options.mode(0o600);
    }
}

#[cfg(not(unix))]
fn owner_only_on_create(_options: &mut std::fs::OpenOptions, _private: bool) {}

#[cfg(unix)]
fn owner_only(path: &Path, private: bool) {
    use std::os::unix::fs::PermissionsExt;
    if private {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
}

#[cfg(not(unix))]
fn owner_only(_path: &Path, _private: bool) {}

// ----------------------------------------------------------------------------
// Import
// ----------------------------------------------------------------------------

// WHAT:  What makes two connections "the same server": engine, host, port, database, user.
// HOW:   Case-insensitive; the engine's default port stands in for a blank one;
//        file databases are the same when they open the same file.
pub fn dedupe_key(input: &ConnectionInput) -> String {
    let lower = |v: &Option<String>| v.as_deref().map(str::trim).unwrap_or_default().to_ascii_lowercase();
    if input.engine.form() == FormKind::File {
        return format!("{}|{}", input.engine.as_str(), lower(&input.file_path));
    }
    let port = input.port.or_else(|| input.engine.default_port()).map(|p| p.to_string()).unwrap_or_default();
    format!("{}|{}|{}|{}|{}", input.engine.as_str(), lower(&input.host), port, lower(&input.database), lower(&input.username))
}

fn summary_key(summary: &ConnectionSummary) -> String {
    let lower = |v: &Option<String>| v.as_deref().map(str::trim).unwrap_or_default().to_ascii_lowercase();
    if summary.engine.form() == FormKind::File {
        return format!("{}|{}", summary.engine.as_str(), lower(&summary.file_path));
    }
    let port = summary.port.or_else(|| summary.engine.default_port()).map(|p| p.to_string()).unwrap_or_default();
    format!("{}|{}|{}|{}|{}", summary.engine.as_str(), lower(&summary.host), port, lower(&summary.database), lower(&summary.username))
}

pub fn import(state: &AppState, path: &str) -> AppResult<ConnectionImportReport> {
    let file = Path::new(path.trim());
    let metadata = std::fs::metadata(file).map_err(|_| AppError::not_found(format!("File not found: {}", file.display())))?;
    if metadata.len() > 5 * 1024 * 1024 {
        return Err(AppError::invalid_input("Connection files must be 5 MB or smaller."));
    }
    let content = std::fs::read_to_string(file).map_err(|e| AppError::invalid_input(format!("Could not read {}: {e}", file.display())))?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    import_text(state, &content, &name)
}

// WHAT:  The import itself, from text. Split from `import` so tests need no file.
pub fn import_text(state: &AppState, content: &str, file_name: &str) -> AppResult<ConnectionImportReport> {
    let parsed = connection_formats::parse_import(content, file_name)?;
    let existing = state.with_store(|store| store.list_connections())?;
    let mut seen: HashSet<String> = existing.iter().filter(|c| !c.extra.transient).map(summary_key).collect();
    let mut skipped: Vec<ConnectionSkipped> = parsed.skipped.clone();
    let mut imported_names = Vec::new();
    let mut warnings = parsed.warnings.clone();
    for ImportedConnection { mut input, folder_names, tags } in parsed.connections {
        let label = input.name.clone();
        if let Err(err) = input.validate() {
            skipped.push(ConnectionSkipped { label, reason: err.message().to_string() });
            continue;
        }
        if !seen.insert(dedupe_key(&input)) {
            skipped.push(ConnectionSkipped { label, reason: "Duplicate connection".to_string() });
            continue;
        }
        // Folders and tags are created by name; a name already in use is reused.
        let folder_ids = state.with_store(|store| folder_names.iter().map(|n| store.ensure_connection_folder(n)).collect::<AppResult<Vec<_>>>())?;
        let tag_ids = state.with_store(|store| {
            tags.iter().map(|(n, color)| store.ensure_connection_tag(n, *color)).collect::<AppResult<Vec<_>>>()
        })?;
        input.extra.folder_ids = folder_ids;
        input.extra.tag_ids = tag_ids;
        // Cheap, correct and bounded: a transient flag from a file is meaningless.
        input.extra.transient = false;
        // The same path the form saves through: it seals the password, the SSH
        // secret and the sign-in secrets, and records how the connection signs in.
        match connection::save(state, None, &input) {
            Ok(summary) => imported_names.push(summary.name),
            Err(err) => skipped.push(ConnectionSkipped { label, reason: err.message().to_string() }),
        }
    }
    if imported_names.is_empty() && skipped.iter().all(|s| s.reason == "Duplicate connection") && !skipped.is_empty() {
        warnings.push("Every connection in the file is already saved.".to_string());
    }
    Ok(ConnectionImportReport {
        source: parsed.source,
        imported: u32::try_from(imported_names.len()).unwrap_or(u32::MAX),
        imported_names,
        skipped,
        warnings,
        includes_passwords: parsed.includes_passwords,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::{ConnectionExtras, Engine, Environment, SshTunnel, SslMode};
    use crate::store::Store;

    fn state() -> AppState {
        AppState::new(Store::open_in_memory().unwrap_or_else(|e| panic!("{e}")), Box::new(MemoryKeyProvider::default()))
    }

    fn input(name: &str, host: &str) -> ConnectionInput {
        ConnectionInput {
            name: name.into(),
            engine: Engine::Postgres,
            environment: Environment::Production,
            read_only: true,
            host: Some(host.into()),
            port: Some(5432),
            database: Some("app".into()),
            username: Some("me".into()),
            password: Some("hunter2".into()),
            file_path: None,
            ssl_mode: SslMode::Require,
            ssh: SshTunnel::default(),
            ssh_secret: None,
            folder: None,
            color: None,
            favorite: false,
            auth: Default::default(),
            auth_secrets: Default::default(),
            extra: ConnectionExtras::default(),
        }
    }

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("db-free-transfer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir.join(name)
    }

    #[test]
    fn export_without_the_opt_in_carries_no_secret_and_import_round_trips_organisation() {
        let source = state();
        let folder = source.with_store(|s| s.save_connection_folder(None, "Clients")).unwrap_or_else(|e| panic!("{e}"));
        let mut organised = input("prod", "db.internal");
        organised.extra.folder_ids = vec![folder.id];
        organised.extra.tag_ids = vec!["preset-production".into()];
        let saved = connection::save(&source, None, &organised).unwrap_or_else(|e| panic!("{e}"));

        let path = temp_file("out.json");
        let report = export(&source, std::slice::from_ref(&saved.id), false, &path.to_string_lossy()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((report.count, report.includes_passwords), (1, false));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            !text.contains("hunter2") && !text.contains("\"password\":") && !text.contains("\"sshSecret\"") && !text.contains("\"authSecrets\""),
            "no secret without the opt-in"
        );

        let target = state();
        let imported = import(&target, &path.to_string_lossy()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(imported.imported, 1);
        let got = target.with_store(|s| s.list_connections()).unwrap_or_default();
        assert_eq!(got.len(), 1);
        assert!(!got[0].has_secret, "the password was not exported");
        assert_eq!(got[0].folder.as_deref(), Some("Clients"));
        assert_eq!(got[0].extra.tag_ids, ["preset-production"]);
        assert!(got[0].read_only);
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new("/nonexistent")));
    }

    #[test]
    fn export_with_passwords_writes_them_and_import_reseals_under_the_new_key() {
        let source = state();
        let saved = connection::save(&source, None, &input("prod", "db.internal")).unwrap_or_else(|e| panic!("{e}"));
        let path = temp_file("out.json");
        export(&source, &[saved.id], true, &path.to_string_lossy()).unwrap_or_else(|e| panic!("{e}"));
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(text.contains("hunter2"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).map(|m| m.permissions().mode() & 0o777).unwrap_or(0);
            assert_eq!(mode, 0o600, "a file holding passwords is owner-only");
        }
        let target = state();
        let report = import(&target, &path.to_string_lossy()).unwrap_or_else(|e| panic!("{e}"));
        assert!(report.includes_passwords);
        let id = target.with_store(|s| s.list_connections()).unwrap_or_default()[0].id.clone();
        let resolved = connection::resolve(&target, &id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resolved.connection.secret.as_deref(), Some("hunter2"), "re-sealed under the target's own key");
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new("/nonexistent")));
    }

    #[test]
    fn importing_twice_skips_duplicates_and_reports_them() {
        let source = state();
        let saved = connection::save(&source, None, &input("prod", "DB.internal")).unwrap_or_else(|e| panic!("{e}"));
        let path = temp_file("out.json");
        export(&source, &[saved.id], false, &path.to_string_lossy()).unwrap_or_else(|e| panic!("{e}"));
        let target = state();
        connection::save(&target, None, &input("already here", "db.INTERNAL")).unwrap_or_else(|e| panic!("{e}"));
        let report = import(&target, &path.to_string_lossy()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(report.imported, 0);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, "Duplicate connection");
        assert!(report.warnings.iter().any(|w| w.contains("already saved")));
        assert_eq!(target.with_store(|s| s.list_connections()).unwrap_or_default().len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(Path::new("/nonexistent")));
    }

    #[test]
    fn dedupe_uses_the_default_port_and_the_file_for_file_databases() {
        let mut a = input("a", "h");
        a.port = None;
        let mut b = input("b", "H");
        b.port = Some(5432);
        assert_eq!(dedupe_key(&a), dedupe_key(&b));
        b.database = Some("other".into());
        assert_ne!(dedupe_key(&a), dedupe_key(&b));
        let mut file = input("f", "");
        file.engine = Engine::Sqlite;
        file.file_path = Some("/Data/A.db".into());
        let mut same = file.clone();
        same.file_path = Some("/data/a.DB".into());
        same.name = "other name".into();
        assert_eq!(dedupe_key(&file), dedupe_key(&same));
    }

    #[test]
    fn invalid_connections_in_a_file_are_skipped_not_fatal() {
        let state = state();
        let text = r#"{ "version": "1.0", "includesPasswords": false, "connections": [
            { "label": "no host", "database": "postgresql", "connectionType": "standard" },
            { "label": "fine", "database": "postgresql", "connectionType": "standard", "host": "h", "port": "5432" } ] }"#;
        let report = import_text(&state, text, "x.dbpro").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(report.imported_names, ["fine"]);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].label, "no host");
        assert!(report.skipped[0].reason.contains("Host is required"));
    }

    #[test]
    fn export_refuses_an_empty_selection_and_a_missing_path() {
        let state = state();
        assert!(export(&state, &[], false, "/tmp/x.json").is_err());
        assert!(export(&state, &["nope".into()], false, " ").is_err());
        assert!(export(&state, &["nope".into()], false, "/tmp/x.json").is_err(), "unknown connection");
    }
}
