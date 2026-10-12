// SOT: transient-file-connection, open-database-file, ssh-config-import, demo-connection, startup-sweep

use crate::error::{AppError, AppResult};
use crate::model::{ConnectionExtras, ConnectionInput, ConnectionSummary, Engine, Environment, SshConfigHost, SshTunnel, SslMode};
use crate::state::AppState;
use crate::store::demo::{build_demo_database, DEMO_FILE_NAME};
use std::path::{Path, PathBuf};

// ============================================================================
// WHAT:  Connections that come from files rather than the form: a SQLite file
//        the OS or a drag-drop hands the app (transient until saved), the
//        sample database, and the hosts of ~/.ssh/config.
// WHY:   Opening a `.sqlite` file should be one gesture, not "New connection →
//        SQLite → Browse"; a first-time user should have something to look at;
//        and a bastion already described in ~/.ssh/config should not be retyped.
// HOW:   A transient connection is an ordinary row flagged `transient`: it lists,
//        connects and works like any other, and is removed on quit and at the
//        next startup unless "Save as connection" cleared the flag. Opening a
//        file that already has a saved connection reuses that one.
// WHERE: src-tauri/src/open_files.rs (OS events), src/features/connections/ (UI)
// ============================================================================

/// Extensions the OS may hand us and a drop may carry.
pub const DATABASE_EXTENSIONS: [&str; 4] = ["db", "sqlite", "sqlite3", "db3"];

pub fn is_database_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| DATABASE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

// WHAT:  The connection for a database file: the saved one if there is one, else a transient one.
pub fn open_database_file(state: &AppState, path: &Path) -> AppResult<ConnectionSummary> {
    if !is_database_file(path) {
        return Err(AppError::invalid_input("Only .db, .sqlite, .sqlite3 and .db3 files can be opened this way."));
    }
    let resolved = std::fs::canonicalize(path).map_err(|_| AppError::not_found(format!("File not found: {}", path.display())))?;
    if !resolved.is_file() {
        return Err(AppError::invalid_input(format!("{} is not a file.", resolved.display())));
    }
    let file_path = resolved.to_string_lossy().into_owned();
    state.with_store(|store| {
        let saved = store.list_connections()?;
        if let Some(existing) = saved
            .into_iter()
            .find(|c| c.engine == Engine::Sqlite && c.file_path.as_deref().is_some_and(|p| same_file(p, &file_path)))
        {
            return Ok(existing);
        }
        if let Some(transient) = store.find_transient_for_file(&file_path)? {
            return Ok(transient);
        }
        let name = resolved.file_name().map_or_else(|| "SQLite database".to_string(), |n| n.to_string_lossy().into_owned());
        let input = file_input(&name, Engine::Sqlite, &file_path);
        store.insert_connection_full(&input, None, None, true)
    })
}

fn same_file(a: &str, b: &str) -> bool {
    a == b || std::fs::canonicalize(a).is_ok_and(|a| a == Path::new(b))
}

fn file_input(name: &str, engine: Engine, file_path: &str) -> ConnectionInput {
    ConnectionInput {
        name: name.to_string(),
        engine,
        environment: Environment::None,
        read_only: false,
        host: None,
        port: None,
        database: None,
        username: None,
        password: None,
        file_path: Some(file_path.to_string()),
        ssl_mode: SslMode::Disable,
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

// WHAT:  "Save as connection": the transient connection becomes a normal one.
pub fn save_transient(state: &AppState, id: &str, name: Option<&str>) -> AppResult<ConnectionSummary> {
    state.with_store(|store| {
        let current = store.get_connection(id)?;
        if !current.extra.transient {
            return Ok(current);
        }
        store.make_connection_permanent(id, name)
    })
}

// WHAT:  Removes every transient connection. Run at startup and when the app exits.
pub fn sweep_transient(state: &AppState) -> AppResult<usize> {
    state.with_store(|store| store.delete_transient_connections())
}

// ----------------------------------------------------------------------------
// Demo database
// ----------------------------------------------------------------------------

// WHAT:  The sample e-commerce SQLite connection, created on first use.
// HOW:   The database file lives under `<data_dir>/demo/`. Asking again returns
//        the saved connection instead of adding a second one; if the user deleted
//        the connection but the file is still there, the file is reused.
pub fn create_demo(state: &AppState, data_dir: &Path) -> AppResult<ConnectionSummary> {
    let path: PathBuf = data_dir.join("demo").join(DEMO_FILE_NAME);
    if !path.is_file() {
        build_demo_database(&path)?;
    }
    let file_path = path.to_string_lossy().into_owned();
    state.with_store(|store| {
        if let Some(existing) = store
            .list_connections()?
            .into_iter()
            .find(|c| c.engine == Engine::Sqlite && !c.extra.transient && c.file_path.as_deref() == Some(file_path.as_str()))
        {
            return Ok(existing);
        }
        let mut input = file_input("Demo · E-commerce", Engine::Sqlite, &file_path);
        input.environment = Environment::Local;
        store.insert_connection(&input, None)
    })
}

// ----------------------------------------------------------------------------
// ~/.ssh/config
// ----------------------------------------------------------------------------

// WHAT:  The concrete `Host` entries of the user's OpenSSH config.
// WHY:   Fills the tunnel form from a bastion the user already described.
// HOW:   A missing file is an empty list, not an error: most machines have none.
pub fn ssh_config_hosts() -> AppResult<Vec<SshConfigHost>> {
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from) else {
        return Ok(Vec::new());
    };
    let path = home.join(".ssh").join("config");
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(parse_ssh_config(&text, &home)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(AppError::invalid_input(format!("Could not read {}: {err}", path.display()))),
    }
}

// WHAT:  `Keyword value` or `Keyword=value`, with the value unquoted.
fn split_directive(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let boundary = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let (keyword, rest) = line.split_at(boundary);
    let value = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim();
    let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
    if keyword.is_empty() || value.is_empty() {
        return None;
    }
    Some((keyword.to_ascii_lowercase(), value.to_string()))
}

#[derive(Default)]
struct HostBlock {
    aliases: Vec<String>,
    host_name: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identity_file: Option<String>,
}

fn flush(block: HostBlock, out: &mut Vec<SshConfigHost>) {
    for alias in block.aliases {
        out.push(SshConfigHost {
            alias,
            host_name: block.host_name.clone(),
            user: block.user.clone(),
            port: block.port,
            identity_file: block.identity_file.clone(),
        });
    }
}

// WHAT:  OpenSSH config → concrete hosts. Pure, so it is testable without a file.
// HOW:   Aliases with `*`, `?` or a leading `!` are patterns, not hosts, and are
//        skipped (their settings are not merged into others). As in OpenSSH, the
//        first value given for a keyword wins. `Match` ends the current block.
//        `IdentityFile` expands `~`.
pub fn parse_ssh_config(text: &str, home: &Path) -> Vec<SshConfigHost> {
    let mut out = Vec::new();
    let mut current: Option<HostBlock> = None;
    for line in text.lines() {
        let Some((keyword, value)) = split_directive(line) else {
            continue;
        };
        match keyword.as_str() {
            "host" => {
                if let Some(block) = current.take() {
                    flush(block, &mut out);
                }
                let aliases: Vec<String> = value
                    .split_whitespace()
                    .map(|a| a.trim_matches('"').to_string())
                    .filter(|a| !a.is_empty() && !a.contains(['*', '?']) && !a.starts_with('!'))
                    .collect();
                current = Some(HostBlock { aliases, ..HostBlock::default() });
            }
            "match" => {
                if let Some(block) = current.take() {
                    flush(block, &mut out);
                }
            }
            "hostname" => {
                if let Some(block) = current.as_mut() {
                    block.host_name.get_or_insert(value);
                }
            }
            "user" => {
                if let Some(block) = current.as_mut() {
                    block.user.get_or_insert(value);
                }
            }
            "port" => {
                if let (Some(block), Ok(port)) = (current.as_mut(), value.parse::<u16>()) {
                    block.port.get_or_insert(port);
                }
            }
            "identityfile" => {
                if let Some(block) = current.as_mut() {
                    let expanded = match value.strip_prefix("~/") {
                        Some(rest) => home.join(rest).to_string_lossy().into_owned(),
                        None => value,
                    };
                    block.identity_file.get_or_insert(expanded);
                }
            }
            _ => {}
        }
    }
    if let Some(block) = current.take() {
        flush(block, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::store::Store;

    fn state() -> AppState {
        AppState::new(Store::open_in_memory().unwrap_or_else(|e| panic!("{e}")), Box::new(MemoryKeyProvider::default()))
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("db-free-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir
    }

    #[test]
    fn only_database_extensions_open() {
        assert!(is_database_file(Path::new("/a/b.SQLITE3")));
        assert!(is_database_file(Path::new("c.db")));
        assert!(!is_database_file(Path::new("c.csv")));
        assert!(!is_database_file(Path::new("noext")));
        assert!(open_database_file(&state(), Path::new("/definitely/missing.db")).is_err());
    }

    #[test]
    fn opening_a_file_makes_one_transient_connection_and_reuses_it() {
        let dir = temp_dir();
        let file = dir.join("scratch.sqlite");
        std::fs::write(&file, b"").unwrap_or_else(|e| panic!("{e}"));
        let state = state();
        let first = open_database_file(&state, &file).unwrap_or_else(|e| panic!("{e}"));
        assert!(first.extra.transient);
        assert_eq!(first.name, "scratch.sqlite");
        assert_eq!(first.engine, Engine::Sqlite);
        let again = open_database_file(&state, &file).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.id, first.id, "the same file is the same connection");
        // Saving keeps it; a later open then finds the saved connection.
        let saved = save_transient(&state, &first.id, Some("Scratch")).unwrap_or_else(|e| panic!("{e}"));
        assert!(!saved.extra.transient);
        assert_eq!(open_database_file(&state, &file).map(|c| c.id).unwrap_or_default(), first.id);
        assert_eq!(sweep_transient(&state).unwrap_or(99), 0, "a saved connection survives the sweep");
        let other = dir.join("other.db");
        std::fs::write(&other, b"").unwrap_or_else(|e| panic!("{e}"));
        let transient = open_database_file(&state, &other).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(sweep_transient(&state).unwrap_or(0), 1);
        assert!(state.with_store(|s| s.get_connection(&transient.id)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_demo_connection_is_created_once_and_points_at_a_real_database() {
        let dir = temp_dir();
        let state = state();
        let first = create_demo(&state, &dir).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(first.engine, Engine::Sqlite);
        assert!(!first.extra.transient);
        assert!(first.file_path.as_deref().is_some_and(|p| Path::new(p).is_file()));
        let second = create_demo(&state, &dir).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(second.id, first.id);
        assert_eq!(state.with_store(|s| s.list_connections()).unwrap_or_default().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    const CONFIG: &str = r#"
# work machines
Host bastion
    HostName bastion.example.com
    User ops
    Port 2222
    IdentityFile ~/.ssh/work_ed25519
    IdentityFile ~/.ssh/ignored

Host db-prod db-staging
    HostName=10.0.0.5
    User "deploy"

Host *
    ServerAliveInterval 30
    User everyone

Host *.internal !bad web?
    User nobody

Match host legacy
    User legacy

Host bare
"#;

    #[test]
    fn ssh_config_yields_concrete_hosts_only() {
        let hosts = parse_ssh_config(CONFIG, Path::new("/home/me"));
        let aliases: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(aliases, ["bastion", "db-prod", "db-staging", "bare"], "wildcards and negations are skipped");
        let bastion = &hosts[0];
        assert_eq!(bastion.host_name.as_deref(), Some("bastion.example.com"));
        assert_eq!((bastion.user.as_deref(), bastion.port), (Some("ops"), Some(2222)));
        assert_eq!(bastion.identity_file.as_deref(), Some("/home/me/.ssh/work_ed25519"), "first IdentityFile wins, ~ expanded");
        assert_eq!(hosts[1].host_name.as_deref(), Some("10.0.0.5"), "key=value syntax");
        assert_eq!(hosts[2].user.as_deref(), Some("deploy"), "quotes are stripped; every alias of a block shares it");
        assert_eq!(hosts[3].host_name, None, "an alias with no HostName is its own host name");
    }

    #[test]
    fn directives_parse_with_equals_quotes_and_comments() {
        assert_eq!(split_directive("  Port=22 "), Some(("port".into(), "22".into())));
        assert_eq!(split_directive("HostName   \"a b\""), Some(("hostname".into(), "a b".into())));
        assert_eq!(split_directive("# comment"), None);
        assert_eq!(split_directive(""), None);
        assert_eq!(split_directive("Host"), None);
    }
}
