// SOT: connections-table, connection-persistence, secret-change, ssh-host-key-pin

use crate::error::{AppError, AppResult};
use crate::model::{
    ConnectionInput, ConnectionRecord, ConnectionSummary, Engine, Environment, SshAuth, SshTunnel, SslMode,
};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, OptionalExtension, Row};

// WHAT:  How an update treats the stored secret.
// WHY:   The UI never sees the secret, so "field left blank" must mean Keep.
pub enum SecretChange {
    Keep,
    Set(Vec<u8>),
    Clear,
}

const COLUMNS: &str = "id, name, engine, environment, read_only, host, port, database, username, \
                       file_path, ssl_mode, secret_ciphertext IS NOT NULL, created_at, updated_at, \
                       ssh_enabled, ssh_host, ssh_port, ssh_user, ssh_auth, ssh_key_path, ssh_host_key, \
                       ssh_secret_ciphertext IS NOT NULL";

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<ConnectionSummary> {
    let engine_raw: String = row.get(2)?;
    let env_raw: String = row.get(3)?;
    let ssl_raw: String = row.get(10)?;
    let port: Option<i64> = row.get(6)?;
    let ssh_auth_raw: String = row.get(18)?;
    let ssh_port: i64 = row.get(16)?;
    let ssh = SshTunnel {
        enabled: row.get::<_, i64>(14)? != 0,
        host: row.get(15)?,
        port: u16::try_from(ssh_port).unwrap_or(crate::model::connection::DEFAULT_SSH_PORT),
        user: row.get(17)?,
        auth: SshAuth::parse(&ssh_auth_raw).unwrap_or(SshAuth::Password),
        key_path: row.get(19)?,
        host_key: row.get(20)?,
    };
    Ok(ConnectionSummary {
        id: row.get(0)?,
        name: row.get(1)?,
        engine: Engine::parse(&engine_raw).unwrap_or(Engine::Postgres),
        environment: Environment::parse(&env_raw).unwrap_or(Environment::Local),
        read_only: row.get::<_, i64>(4)? != 0,
        host: row.get(5)?,
        port: port.and_then(|p| u16::try_from(p).ok()),
        database: row.get(7)?,
        username: row.get(8)?,
        file_path: row.get(9)?,
        ssl_mode: SslMode::parse(&ssl_raw).unwrap_or(SslMode::Prefer),
        has_secret: row.get::<_, i64>(11)? != 0,
        ssh,
        has_ssh_secret: row.get::<_, i64>(21)? != 0,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

impl Store {
    pub fn list_connections(&self) -> AppResult<Vec<ConnectionSummary>> {
        let sql = format!("SELECT {COLUMNS} FROM connections ORDER BY lower(name), created_at");
        let mut stmt = self.conn().prepare(&sql).map_err(AppError::store)?;
        let rows = stmt
            .query_map([], summary_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        Ok(rows)
    }

    pub fn get_connection(&self, id: &str) -> AppResult<ConnectionSummary> {
        let sql = format!("SELECT {COLUMNS} FROM connections WHERE id = ?1");
        self.conn()
            .query_row(&sql, params![id], summary_from_row)
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::not_found(format!("Connection {id} does not exist.")))
    }

    pub fn get_connection_record(&self, id: &str) -> AppResult<ConnectionRecord> {
        let summary = self.get_connection(id)?;
        let (secret_ciphertext, ssh_secret_ciphertext): (Option<Vec<u8>>, Option<Vec<u8>>) = self
            .conn()
            .query_row(
                "SELECT secret_ciphertext, ssh_secret_ciphertext FROM connections WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(AppError::store)?;
        Ok(ConnectionRecord { summary, secret_ciphertext, ssh_secret_ciphertext })
    }

    pub fn insert_connection(
        &self,
        input: &ConnectionInput,
        secret: Option<Vec<u8>>,
    ) -> AppResult<ConnectionSummary> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_rfc3339();
        self.conn()
            .execute(
                "INSERT INTO connections (id, name, engine, environment, read_only, host, port, \
                 database, username, file_path, ssl_mode, secret_ciphertext, created_at, updated_at, \
                 ssh_enabled, ssh_host, ssh_port, ssh_user, ssh_auth, ssh_key_path, ssh_host_key) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, \
                 ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                params![
                    id,
                    input.name.trim(),
                    input.engine.as_str(),
                    input.environment.as_str(),
                    i64::from(input.read_only),
                    input.host,
                    input.port.map(i64::from),
                    input.database,
                    input.username,
                    input.file_path,
                    input.ssl_mode.as_str(),
                    secret,
                    now,
                    i64::from(input.ssh.enabled),
                    trimmed(&input.ssh.host),
                    i64::from(input.ssh.port),
                    trimmed(&input.ssh.user),
                    input.ssh.auth.as_str(),
                    trimmed(&input.ssh.key_path),
                    trimmed(&input.ssh.host_key),
                ],
            )
            .map_err(AppError::store)?;
        self.get_connection(&id)
    }

    pub fn update_connection(
        &self,
        id: &str,
        input: &ConnectionInput,
        secret: SecretChange,
    ) -> AppResult<ConnectionSummary> {
        let now = now_rfc3339();
        let changed = self
            .conn()
            .execute(
                "UPDATE connections SET name = ?2, engine = ?3, environment = ?4, read_only = ?5, \
                 host = ?6, port = ?7, database = ?8, username = ?9, file_path = ?10, \
                 ssl_mode = ?11, updated_at = ?12, ssh_enabled = ?13, ssh_host = ?14, ssh_port = ?15, \
                 ssh_user = ?16, ssh_auth = ?17, ssh_key_path = ?18, ssh_host_key = ?19 WHERE id = ?1",
                params![
                    id,
                    input.name.trim(),
                    input.engine.as_str(),
                    input.environment.as_str(),
                    i64::from(input.read_only),
                    input.host,
                    input.port.map(i64::from),
                    input.database,
                    input.username,
                    input.file_path,
                    input.ssl_mode.as_str(),
                    now,
                    i64::from(input.ssh.enabled),
                    trimmed(&input.ssh.host),
                    i64::from(input.ssh.port),
                    trimmed(&input.ssh.user),
                    input.ssh.auth.as_str(),
                    trimmed(&input.ssh.key_path),
                    trimmed(&input.ssh.host_key),
                ],
            )
            .map_err(AppError::store)?;
        if changed == 0 {
            return Err(AppError::not_found(format!("Connection {id} does not exist.")));
        }
        match secret {
            SecretChange::Keep => {}
            SecretChange::Set(bytes) => {
                self.conn()
                    .execute(
                        "UPDATE connections SET secret_ciphertext = ?2 WHERE id = ?1",
                        params![id, bytes],
                    )
                    .map_err(AppError::store)?;
            }
            SecretChange::Clear => {
                self.conn()
                    .execute(
                        "UPDATE connections SET secret_ciphertext = NULL WHERE id = ?1",
                        params![id],
                    )
                    .map_err(AppError::store)?;
            }
        }
        self.get_connection(id)
    }

    // WHAT:  Replaces or clears the sealed SSH password / key passphrase.
    // WHY:   Kept apart from `update_connection` so the database secret and the
    //        SSH secret change independently (blank field = Keep for each).
    // WHERE: src-tauri/src/services/connection.rs (seals before calling)
    pub fn set_ssh_secret(&self, id: &str, change: SecretChange) -> AppResult<()> {
        let bytes = match change {
            SecretChange::Keep => return Ok(()),
            SecretChange::Set(bytes) => Some(bytes),
            SecretChange::Clear => None,
        };
        self.conn()
            .execute(
                "UPDATE connections SET ssh_secret_ciphertext = ?2 WHERE id = ?1",
                params![id, bytes],
            )
            .map_err(AppError::store)?;
        Ok(())
    }

    // WHAT:  Pins the SSH server's key fingerprint (trust on first use).
    // HOW:   Only fills an empty pin: a pin the user set or one recorded earlier
    //        is never overwritten here, so a changed key keeps failing.
    // WHERE: src-tauri/src/integrations/ssh_tunnel.rs (verifies against it)
    pub fn pin_ssh_host_key(&self, id: &str, fingerprint: &str) -> AppResult<()> {
        self.conn()
            .execute(
                "UPDATE connections SET ssh_host_key = ?2 WHERE id = ?1 AND ssh_host_key IS NULL",
                params![id, fingerprint],
            )
            .map_err(AppError::store)?;
        Ok(())
    }

    pub fn delete_connection(&self, id: &str) -> AppResult<()> {
        let changed = self
            .conn()
            .execute("DELETE FROM connections WHERE id = ?1", params![id])
            .map_err(AppError::store)?;
        if changed == 0 {
            return Err(AppError::not_found(format!("Connection {id} does not exist.")));
        }
        self.conn()
            .execute("DELETE FROM query_history WHERE connection_id = ?1", params![id])
            .map_err(AppError::store)?;
        Ok(())
    }
}

/// Blank strings are stored as NULL so "not set" has one spelling.
fn trimmed(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str) -> ConnectionInput {
        ConnectionInput {
            name: name.into(),
            engine: Engine::Sqlite,
            environment: Environment::Production,
            read_only: true,
            host: None,
            port: None,
            database: None,
            username: None,
            password: None,
            file_path: Some("/tmp/a.db".into()),
            ssl_mode: SslMode::Disable,
            ssh: crate::model::SshTunnel::default(),
            ssh_secret: None,
        }
    }

    #[test]
    fn insert_update_delete_round_trip() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let created = store
            .insert_connection(&input("A"), Some(vec![1, 2, 3]))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(created.has_secret);
        assert_eq!(created.environment, Environment::Production);
        assert!(created.read_only);

        let record = store.get_connection_record(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(record.secret_ciphertext, Some(vec![1, 2, 3]));

        let updated = store
            .update_connection(&created.id, &input("B"), SecretChange::Keep)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(updated.name, "B");
        assert!(updated.has_secret);

        let cleared = store
            .update_connection(&created.id, &input("B"), SecretChange::Clear)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!cleared.has_secret);

        assert_eq!(store.list_connections().unwrap_or_default().len(), 1);
        store.delete_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(store.get_connection(&created.id), Err(AppError::NotFound { .. })));
    }

    #[test]
    fn ssh_settings_secret_and_pin_round_trip() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let mut with_ssh = input("tunnelled");
        with_ssh.engine = Engine::Postgres;
        with_ssh.ssh = SshTunnel {
            enabled: true,
            host: Some(" bastion.example.com ".into()),
            port: 2222,
            user: Some("ops".into()),
            auth: SshAuth::PrivateKey,
            key_path: Some("/home/ops/.ssh/id_ed25519".into()),
            host_key: None,
        };
        let created = store.insert_connection(&with_ssh, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(created.ssh.host.as_deref(), Some("bastion.example.com"), "trimmed on the way in");
        assert_eq!(created.ssh.port, 2222);
        assert_eq!(created.ssh.auth, SshAuth::PrivateKey);
        assert!(!created.has_ssh_secret);

        store.set_ssh_secret(&created.id, SecretChange::Set(vec![9, 9])).unwrap_or_else(|e| panic!("{e}"));
        let record = store.get_connection_record(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert!(record.summary.has_ssh_secret);
        assert_eq!(record.ssh_secret_ciphertext, Some(vec![9, 9]));
        assert_eq!(record.secret_ciphertext, None, "the two secrets are independent");

        store.pin_ssh_host_key(&created.id, "SHA256:first").unwrap_or_else(|e| panic!("{e}"));
        store.pin_ssh_host_key(&created.id, "SHA256:second").unwrap_or_else(|e| panic!("{e}"));
        let pinned = store.get_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(pinned.ssh.host_key.as_deref(), Some("SHA256:first"), "a pin is never overwritten");

        // Saving the form with the pin cleared forgets it; Keep leaves the SSH secret alone.
        let mut forget = with_ssh.clone();
        forget.ssh.host_key = None;
        store.update_connection(&created.id, &forget, SecretChange::Keep).unwrap_or_else(|e| panic!("{e}"));
        store.set_ssh_secret(&created.id, SecretChange::Keep).unwrap_or_else(|e| panic!("{e}"));
        let after = store.get_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(after.ssh.host_key, None);
        assert!(after.has_ssh_secret);

        store.set_ssh_secret(&created.id, SecretChange::Clear).unwrap_or_else(|e| panic!("{e}"));
        assert!(!store.get_connection(&created.id).map(|c| c.has_ssh_secret).unwrap_or(true));
    }

    // WHAT:  A store created by an older build (schema v2) upgrades in place.
    #[test]
    fn migrates_a_v2_connections_table() {
        let dir = std::env::temp_dir().join(format!("db-free-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let path = dir.join("app.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap_or_else(|e| panic!("{e}"));
            conn.execute_batch(
                "CREATE TABLE connections (id TEXT PRIMARY KEY, name TEXT NOT NULL, engine TEXT NOT NULL, \
                 environment TEXT NOT NULL, read_only INTEGER NOT NULL DEFAULT 0, host TEXT, port INTEGER, \
                 database TEXT, username TEXT, file_path TEXT, ssl_mode TEXT NOT NULL DEFAULT 'prefer', \
                 secret_ciphertext BLOB, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE query_history (id INTEGER PRIMARY KEY AUTOINCREMENT, connection_id TEXT NOT NULL, \
                 sql TEXT NOT NULL, status TEXT NOT NULL, error TEXT, elapsed_ms INTEGER NOT NULL, \
                 row_count INTEGER, executed_at TEXT NOT NULL, origin TEXT NOT NULL DEFAULT 'user');
                 CREATE TABLE editor_buffers (id TEXT PRIMARY KEY, connection_id TEXT, title TEXT NOT NULL, \
                 content TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE saved_queries (id TEXT PRIMARY KEY, connection_id TEXT, name TEXT NOT NULL, \
                 sql TEXT NOT NULL, tags TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE documents (id TEXT PRIMARY KEY, kind TEXT NOT NULL, connection_id TEXT, \
                 name TEXT NOT NULL, body TEXT NOT NULL, tags TEXT NOT NULL DEFAULT '', \
                 created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
                 INSERT INTO connections (id, name, engine, environment, host, port, created_at, updated_at) \
                 VALUES ('old', 'legacy', 'postgres', 'staging', 'db', 5432, 'x', 'x');
                 PRAGMA user_version = 2;",
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        let store = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let legacy = store.get_connection("old").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(legacy.name, "legacy");
        assert_eq!(legacy.ssh, SshTunnel::default(), "old rows read as tunnel off");
        assert!(!legacy.has_ssh_secret);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
