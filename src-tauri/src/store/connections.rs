// SOT: connections-table, connection-persistence, secret-change, ssh-host-key-pin, connection-organisation, transient-connections, connection-duplicate, sealed-secret-count

use crate::error::{AppError, AppResult};
use crate::model::{
    ConnectionColor, ConnectionExtras, ConnectionInput, ConnectionRecord, ConnectionSummary, Engine, Environment, SshAuth, SshTunnel,
    SslMode, TlsFiles,
};
use crate::store::connection_auth::attach_auth;
use crate::store::connection_org::{attach_membership, ensure_folder, replace_membership};
use crate::store::{now_rfc3339, Store};
use rusqlite::{params, Connection, OptionalExtension, Row};

// WHAT:  How an update treats the stored secret.
// WHY:   The UI never sees the secret, so "field left blank" must mean Keep.
pub enum SecretChange {
    Keep,
    Set(Vec<u8>),
    Clear,
}

// WHAT:  The columns a `ConnectionSummary` is read from, in `summary_from_row` order.
// WHY:   The legacy free-text `folder` column is not read any more: folders are
//        entities now and `attach_membership` derives the summary's `folder`.
const COLUMNS: &str = "id, name, engine, environment, read_only, host, port, database, username, \
                       file_path, ssl_mode, secret_ciphertext IS NOT NULL, created_at, updated_at, \
                       ssh_enabled, ssh_host, ssh_port, ssh_user, ssh_auth, ssh_key_path, ssh_host_key, \
                       ssh_secret_ciphertext IS NOT NULL, color, favorite, uri_params, srv, \
                       tls_ca_path, tls_cert_path, tls_key_path, transient";

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<ConnectionSummary> {
    let engine_raw: String = row.get(2)?;
    let env_raw: String = row.get(3)?;
    let ssl_raw: String = row.get(10)?;
    let port: Option<i64> = row.get(6)?;
    let ssh_auth_raw: String = row.get(18)?;
    let color_raw: Option<String> = row.get(22)?;
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
    let extra = ConnectionExtras {
        uri_params: row.get(24)?,
        srv: row.get::<_, i64>(25)? != 0,
        tls: TlsFiles { ca_path: row.get(26)?, cert_path: row.get(27)?, key_path: row.get(28)? },
        transient: row.get::<_, i64>(29)? != 0,
        // Tags and folders are attached by `attach_membership`.
        ..ConnectionExtras::default()
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
        folder: None,
        color: color_raw.as_deref().and_then(ConnectionColor::parse),
        favorite: row.get::<_, i64>(23)? != 0,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        extra,
        // Filled by `attach_auth` (sign-in method and which secrets are set).
        auth: Default::default(),
        has_auth_secrets: Default::default(),
    })
}

impl Store {
    pub fn list_connections(&self) -> AppResult<Vec<ConnectionSummary>> {
        let sql = format!("SELECT {COLUMNS} FROM connections ORDER BY lower(name), created_at");
        let mut stmt = self.conn().prepare(&sql).map_err(AppError::store)?;
        let mut rows = stmt
            .query_map([], summary_from_row)
            .map_err(AppError::store)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::store)?;
        attach_membership(self.conn(), &mut rows)?;
        attach_auth(self.conn(), &mut rows)?;
        Ok(rows)
    }

    pub fn get_connection(&self, id: &str) -> AppResult<ConnectionSummary> {
        let sql = format!("SELECT {COLUMNS} FROM connections WHERE id = ?1");
        let found = self
            .conn()
            .query_row(&sql, params![id], summary_from_row)
            .optional()
            .map_err(AppError::store)?
            .ok_or_else(|| AppError::not_found(format!("Connection {id} does not exist.")))?;
        let mut one = [found];
        attach_membership(self.conn(), &mut one)?;
        attach_auth(self.conn(), &mut one)?;
        let [summary] = one;
        Ok(summary)
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

    pub fn insert_connection(&self, input: &ConnectionInput, secret: Option<Vec<u8>>) -> AppResult<ConnectionSummary> {
        self.insert_connection_full(input, secret, None, false)
    }

    // WHAT:  Inserts a connection with both sealed secrets and its tags / folders in one transaction.
    // WHY:   Import writes a connection and everything it carries at once; a
    //        half-written row (secret but no folders) must never be visible.
    // HOW:   `transient` marks a file opened from the OS or a drop: it is listed
    //        like any connection but removed on quit unless saved.
    pub fn insert_connection_full(
        &self,
        input: &ConnectionInput,
        secret: Option<Vec<u8>>,
        ssh_secret: Option<Vec<u8>>,
        transient: bool,
    ) -> AppResult<ConnectionSummary> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        tx.execute(
            "INSERT INTO connections (id, name, engine, environment, read_only, host, port, \
             database, username, file_path, ssl_mode, secret_ciphertext, created_at, updated_at, \
             ssh_enabled, ssh_host, ssh_port, ssh_user, ssh_auth, ssh_key_path, ssh_host_key, \
             ssh_secret_ciphertext, color, favorite, uri_params, srv, tls_ca_path, tls_cert_path, \
             tls_key_path, transient) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, \
             ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)",
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
                ssh_secret,
                input.color.map(ConnectionColor::as_str),
                i64::from(input.favorite),
                trimmed(&input.extra.uri_params),
                i64::from(input.extra.srv),
                trimmed(&input.extra.tls.ca_path),
                trimmed(&input.extra.tls.cert_path),
                trimmed(&input.extra.tls.key_path),
                i64::from(transient),
            ],
        )
        .map_err(AppError::store)?;
        let folder_ids = folder_ids_for(&tx, input)?;
        replace_membership(&tx, &id, Some(&input.extra.tag_ids), Some(&folder_ids))?;
        tx.commit().map_err(AppError::store)?;
        self.get_connection(&id)
    }

    pub fn update_connection(&self, id: &str, input: &ConnectionInput, secret: SecretChange) -> AppResult<ConnectionSummary> {
        let now = now_rfc3339();
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        let changed = tx
            .execute(
                "UPDATE connections SET name = ?2, engine = ?3, environment = ?4, read_only = ?5, \
                 host = ?6, port = ?7, database = ?8, username = ?9, file_path = ?10, \
                 ssl_mode = ?11, updated_at = ?12, ssh_enabled = ?13, ssh_host = ?14, ssh_port = ?15, \
                 ssh_user = ?16, ssh_auth = ?17, ssh_key_path = ?18, ssh_host_key = ?19, \
                 color = ?20, favorite = ?21, uri_params = ?22, srv = ?23, tls_ca_path = ?24, \
                 tls_cert_path = ?25, tls_key_path = ?26 WHERE id = ?1",
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
                    input.color.map(ConnectionColor::as_str),
                    i64::from(input.favorite),
                    trimmed(&input.extra.uri_params),
                    i64::from(input.extra.srv),
                    trimmed(&input.extra.tls.ca_path),
                    trimmed(&input.extra.tls.cert_path),
                    trimmed(&input.extra.tls.key_path),
                ],
            )
            .map_err(AppError::store)?;
        if changed == 0 {
            return Err(AppError::not_found(format!("Connection {id} does not exist.")));
        }
        match secret {
            SecretChange::Keep => {}
            SecretChange::Set(bytes) => {
                tx.execute("UPDATE connections SET secret_ciphertext = ?2 WHERE id = ?1", params![id, bytes]).map_err(AppError::store)?;
            }
            SecretChange::Clear => {
                tx.execute("UPDATE connections SET secret_ciphertext = NULL WHERE id = ?1", params![id]).map_err(AppError::store)?;
            }
        }
        let folder_ids = folder_ids_for(&tx, input)?;
        replace_membership(&tx, id, Some(&input.extra.tag_ids), Some(&folder_ids))?;
        tx.commit().map_err(AppError::store)?;
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
            .execute("UPDATE connections SET ssh_secret_ciphertext = ?2 WHERE id = ?1", params![id, bytes])
            .map_err(AppError::store)?;
        Ok(())
    }

    // WHAT:  Pins the SSH server's key fingerprint (trust on first use).
    // HOW:   Only fills an empty pin: a pin the user set or one recorded earlier
    //        is never overwritten here, so a changed key keeps failing.
    // WHERE: src-tauri/src/integrations/ssh_tunnel.rs (verifies against it)
    pub fn pin_ssh_host_key(&self, id: &str, fingerprint: &str) -> AppResult<()> {
        self.conn()
            .execute("UPDATE connections SET ssh_host_key = ?2 WHERE id = ?1 AND ssh_host_key IS NULL", params![id, fingerprint])
            .map_err(AppError::store)?;
        Ok(())
    }

    pub fn delete_connection(&self, id: &str) -> AppResult<()> {
        let changed = self.conn().execute("DELETE FROM connections WHERE id = ?1", params![id]).map_err(AppError::store)?;
        if changed == 0 {
            return Err(AppError::not_found(format!("Connection {id} does not exist.")));
        }
        self.conn().execute("DELETE FROM query_history WHERE connection_id = ?1", params![id]).map_err(AppError::store)?;
        Ok(())
    }

    // WHAT:  A copy of a saved connection under a free "(copy)" name.
    // WHY:   The sealed secrets are copied as ciphertext, so a duplicate never
    //        needs the master key (and works even when it is unavailable).
    // HOW:   The copy is `INSERT … SELECT` over *every* column of the table
    //        (read from the schema), so a column another feature adds later
    //        (sign-in method, sealed auth config) is carried along without this
    //        function knowing it. Only identity, name, timestamps and the
    //        transient flag are replaced; tags and folders are copied too.
    pub fn duplicate_connection(&self, id: &str) -> AppResult<ConnectionSummary> {
        let original = self.get_connection(id)?;
        let name = self.free_copy_name(&original.name)?;
        let new_id = uuid::Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let columns: Vec<String> = {
            let mut stmt = self.conn().prepare("SELECT name FROM pragma_table_info('connections')").map_err(AppError::store)?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0)).map_err(AppError::store)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(AppError::store)?
        };
        let select: Vec<&str> = columns
            .iter()
            .map(|c| match c.as_str() {
                "id" => "?2",
                "name" => "?3",
                "created_at" | "updated_at" => "?4",
                "transient" => "0",
                other => other,
            })
            .collect();
        let sql = format!("INSERT INTO connections ({}) SELECT {} FROM connections WHERE id = ?1", columns.join(", "), select.join(", "));
        let tx = self.conn().unchecked_transaction().map_err(AppError::store)?;
        tx.execute(&sql, params![id, new_id, name, now]).map_err(AppError::store)?;
        replace_membership(&tx, &new_id, Some(&original.extra.tag_ids), Some(&original.extra.folder_ids))?;
        tx.commit().map_err(AppError::store)?;
        self.get_connection(&new_id)
    }

    fn free_copy_name(&self, base: &str) -> AppResult<String> {
        let taken = |candidate: &str| -> AppResult<bool> {
            self.conn()
                .query_row("SELECT 1 FROM connections WHERE name = ?1 COLLATE NOCASE", params![candidate], |_| Ok(true))
                .optional()
                .map_err(AppError::store)
                .map(|found| found.unwrap_or(false))
        };
        let first = format!("{base} (copy)");
        if !taken(&first)? {
            return Ok(first);
        }
        for n in 2..1000 {
            let candidate = format!("{base} (copy {n})");
            if !taken(&candidate)? {
                return Ok(candidate);
            }
        }
        Ok(format!("{base} (copy {})", uuid::Uuid::new_v4()))
    }

    // ------------------------------------------------------------------
    // Transient connections (files opened from the OS or dropped on the window)
    // ------------------------------------------------------------------

    // WHAT:  The transient connection already open on this file, if any.
    // WHY:   Opening the same file twice must reuse the connection, not stack up copies.
    pub fn find_transient_for_file(&self, path: &str) -> AppResult<Option<ConnectionSummary>> {
        let id: Option<String> = self
            .conn()
            .query_row("SELECT id FROM connections WHERE transient = 1 AND file_path = ?1", params![path], |row| row.get(0))
            .optional()
            .map_err(AppError::store)?;
        id.map(|id| self.get_connection(&id)).transpose()
    }

    // WHAT:  "Save as connection": keeps a transient connection past the next quit.
    pub fn make_connection_permanent(&self, id: &str, name: Option<&str>) -> AppResult<ConnectionSummary> {
        let now = now_rfc3339();
        let changed = self
            .conn()
            .execute(
                "UPDATE connections SET transient = 0, updated_at = ?2, name = COALESCE(?3, name) WHERE id = ?1",
                params![id, now, name.map(str::trim).filter(|n| !n.is_empty())],
            )
            .map_err(AppError::store)?;
        if changed == 0 {
            return Err(AppError::not_found(format!("Connection {id} does not exist.")));
        }
        self.get_connection(id)
    }

    // WHAT:  Removes every transient connection (and its history). Returns how many.
    // WHY:   Run at startup and on quit: a file opened "just to look" leaves nothing behind.
    pub fn delete_transient_connections(&self) -> AppResult<usize> {
        self.conn()
            .execute("DELETE FROM query_history WHERE connection_id IN (SELECT id FROM connections WHERE transient = 1)", [])
            .map_err(AppError::store)?;
        self.conn().execute("DELETE FROM connections WHERE transient = 1", []).map_err(AppError::store)
    }

    // ------------------------------------------------------------------
    // Sealed secrets
    // ------------------------------------------------------------------

    // WHAT:  How many sealed secrets exist: connection passwords, SSH secrets, sign-in
    //        secrets (keys, client secrets, tokens) and the AI key.
    // WHY:   If the master key is missing while this is non-zero, a new key would
    //        make every one of them unreadable. The caller reports "credentials
    //        lost" instead of minting a key. Features that seal secrets elsewhere
    //        add their tables here.
    pub fn sealed_secret_count(&self) -> AppResult<u32> {
        let total: i64 = self
            .conn()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM connections WHERE secret_ciphertext IS NOT NULL) \
                      + (SELECT COUNT(*) FROM connections WHERE ssh_secret_ciphertext IS NOT NULL) \
                      + (SELECT COUNT(*) FROM connections WHERE auth_secret_ciphertext IS NOT NULL) \
                      + (SELECT COUNT(*) FROM settings WHERE key = 'ai_key')",
                [],
                |row| row.get(0),
            )
            .map_err(AppError::store)?;
        // Workflows seal their SMTP password and Slack webhook inside the document body.
        let total = total.saturating_add(i64::from(self.workflow_sealed_secret_count()?));
        Ok(u32::try_from(total).unwrap_or(u32::MAX))
    }

    // WHAT:  Forgets every sealed secret, after the master key was lost for good.
    // WHY:   They can never be opened again; clearing them lets the user enter
    //        passwords under a fresh key instead of hitting a decrypt error forever.
    pub fn clear_sealed_secrets(&self) -> AppResult<()> {
        self.conn()
            .execute(
                "UPDATE connections SET secret_ciphertext = NULL, ssh_secret_ciphertext = NULL, \
                 auth_secret_ciphertext = NULL, auth_secret_flags = NULL",
                [],
            )
            .map_err(AppError::store)?;
        self.conn().execute("DELETE FROM settings WHERE key = 'ai_key'", []).map_err(AppError::store)?;
        self.clear_workflow_sealed_secrets()?;
        Ok(())
    }
}

// WHAT:  The folder ids a save should leave the connection in: the ids the input
//        names, plus the legacy `folder` name (created when missing).
fn folder_ids_for(conn: &Connection, input: &ConnectionInput) -> AppResult<Vec<String>> {
    let mut ids = input.extra.folder_ids.clone();
    if let Some(name) = trimmed(&input.folder) {
        let id = ensure_folder(conn, name)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
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
            folder: None,
            color: None,
            favorite: false,
            extra: ConnectionExtras::default(),
            auth: Default::default(),
            auth_secrets: Default::default(),
        }
    }

    #[test]
    fn insert_update_delete_round_trip() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let created = store.insert_connection(&input("A"), Some(vec![1, 2, 3])).unwrap_or_else(|e| panic!("{e}"));
        assert!(created.has_secret);
        assert_eq!(created.environment, Environment::Production);
        assert!(created.read_only);

        let record = store.get_connection_record(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(record.secret_ciphertext, Some(vec![1, 2, 3]));

        let updated = store.update_connection(&created.id, &input("B"), SecretChange::Keep).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(updated.name, "B");
        assert!(updated.has_secret);

        let cleared = store.update_connection(&created.id, &input("B"), SecretChange::Clear).unwrap_or_else(|e| panic!("{e}"));
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

    #[test]
    fn colour_favourite_and_legacy_folder_name_round_trip() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let mut organised = input("filed");
        organised.folder = Some("  Clients / Acme ".into());
        organised.color = Some(ConnectionColor::Teal);
        organised.favorite = true;
        let created = store.insert_connection(&organised, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(created.folder.as_deref(), Some("Clients / Acme"), "the legacy name creates a folder entity");
        assert_eq!(created.extra.folder_ids.len(), 1);
        assert_eq!(created.color, Some(ConnectionColor::Teal));
        assert!(created.favorite);

        // The form echoes the ids back and clears the legacy name: the connection leaves the folder.
        let mut plain = organised.clone();
        plain.folder = None;
        plain.extra.folder_ids = Vec::new();
        plain.color = None;
        plain.favorite = false;
        let updated = store.update_connection(&created.id, &plain, SecretChange::Keep).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(updated.folder, None, "no folder named, no folder");
        assert_eq!(updated.color, None);
        assert!(!updated.favorite);
        assert_eq!(store.list_connection_org().unwrap_or_else(|e| panic!("{e}")).folders.len(), 1, "the now-empty folder remains");
    }

    #[test]
    fn extras_round_trip_and_blank_values_become_null() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let mut mongo = input("atlas");
        mongo.engine = Engine::Mongodb;
        mongo.host = Some("cluster0.abc.mongodb.net".into());
        mongo.extra.uri_params = Some(" authSource=admin&retryWrites=true ".into());
        mongo.extra.srv = true;
        mongo.extra.tls = TlsFiles { ca_path: Some("/ca.pem".into()), cert_path: Some("  ".into()), key_path: None };
        mongo.extra.tag_ids = vec!["preset-production".into()];
        let created = store.insert_connection(&mongo, None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(created.extra.uri_params.as_deref(), Some("authSource=admin&retryWrites=true"));
        assert!(created.extra.srv);
        assert_eq!(created.extra.tls.ca_path.as_deref(), Some("/ca.pem"));
        assert_eq!(created.extra.tls.cert_path, None, "blank is not set");
        assert_eq!(created.extra.tag_ids, ["preset-production"]);
        assert!(!created.extra.transient);
        let listed = store.list_connections().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(listed.first().map(|c| c.extra.clone()), Some(created.extra));
    }

    #[test]
    fn duplicate_copies_ciphertext_ssh_and_organisation_under_a_free_name() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let mut original = input("prod");
        original.engine = Engine::Postgres;
        original.host = Some("db".into());
        original.ssh = SshTunnel { enabled: true, host: Some("bastion".into()), user: Some("ops".into()), ..SshTunnel::default() };
        original.extra.tag_ids = vec!["preset-production".into()];
        let created = store.insert_connection_full(&original, Some(vec![1, 2]), Some(vec![3, 4]), false).unwrap_or_else(|e| panic!("{e}"));
        store.pin_ssh_host_key(&created.id, "SHA256:abc").unwrap_or_else(|e| panic!("{e}"));

        let copy = store.duplicate_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(copy.name, "prod (copy)");
        assert_ne!(copy.id, created.id);
        assert_eq!(copy.ssh.host_key.as_deref(), Some("SHA256:abc"));
        assert_eq!(copy.extra.tag_ids, ["preset-production"]);
        let record = store.get_connection_record(&copy.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(record.secret_ciphertext, Some(vec![1, 2]));
        assert_eq!(record.ssh_secret_ciphertext, Some(vec![3, 4]));

        assert_eq!(store.duplicate_connection(&created.id).map(|c| c.name).unwrap_or_default(), "prod (copy 2)");
        assert_eq!(store.duplicate_connection(&copy.id).map(|c| c.name).unwrap_or_default(), "prod (copy) (copy)");
        assert!(store.duplicate_connection("missing").is_err());
    }

    #[test]
    fn transient_connections_are_reused_saved_and_swept() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let kept = store.insert_connection(&input("kept"), None).unwrap_or_else(|e| panic!("{e}"));
        let mut file = input("scratch.db");
        file.file_path = Some("/data/scratch.db".into());
        let transient = store.insert_connection_full(&file, None, None, true).unwrap_or_else(|e| panic!("{e}"));
        assert!(transient.extra.transient);
        let again = store.find_transient_for_file("/data/scratch.db").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.map(|c| c.id), Some(transient.id.clone()));
        assert!(store.find_transient_for_file("/data/other.db").unwrap_or_else(|e| panic!("{e}")).is_none());
        let saved = store.make_connection_permanent(&transient.id, Some("  My scratch ")).unwrap_or_else(|e| panic!("{e}"));
        assert!(!saved.extra.transient);
        assert_eq!(saved.name, "My scratch");

        let second = store.insert_connection_full(&file, None, None, true).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.delete_transient_connections().unwrap_or_default(), 1);
        assert!(store.get_connection(&second.id).is_err());
        let names: Vec<String> = store.list_connections().unwrap_or_default().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["kept", "My scratch"]);
        let _ = kept;
    }

    #[test]
    fn sealed_secrets_are_counted_and_can_be_forgotten() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.sealed_secret_count().unwrap_or(99), 0);
        let created = store.insert_connection_full(&input("a"), Some(vec![1]), Some(vec![2]), false).unwrap_or_else(|e| panic!("{e}"));
        store.set_setting("ai_key", "sealed").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.sealed_secret_count().unwrap_or(0), 3);
        store.clear_sealed_secrets().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.sealed_secret_count().unwrap_or(99), 0);
        let record = store.get_connection_record(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert!(!record.summary.has_secret && !record.summary.has_ssh_secret);
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
        assert_eq!((legacy.folder, legacy.color, legacy.favorite), (None, None, false));
        assert_eq!(legacy.extra, ConnectionExtras::default());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // WHAT:  Free-text folders become folder entities; saved Mongo users keep their auth database.
    #[test]
    fn migration_moves_legacy_folders_and_keeps_the_mongo_auth_database() {
        let dir = std::env::temp_dir().join(format!("db-free-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let path = dir.join("app.db");
        {
            // The store as the previous release left it: the connections table has `folder`, the new tables do not exist.
            let store = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
            let conn = store.conn();
            conn.execute_batch(
                "DROP TABLE connection_folder_members; DROP TABLE connection_folders; DROP TABLE connection_tag_links; DROP TABLE connection_tags;
                 DELETE FROM store_migrations WHERE module = 'connections';
                 INSERT INTO connections (id, name, engine, environment, host, port, username, folder, created_at, updated_at) VALUES
                   ('a', 'a', 'postgres', 'local', 'h', 5432, 'u', 'Clients', 'x', 'x'),
                   ('b', 'b', 'postgres', 'local', 'h', 5432, 'u', ' clients ', 'x', 'x'),
                   ('c', 'c', 'postgres', 'local', 'h', 5432, 'u', 'Internal', 'x', 'x'),
                   ('d', 'd', 'postgres', 'local', 'h', 5432, 'u', NULL, 'x', 'x'),
                   ('m', 'm', 'mongodb', 'local', 'h', 27017, 'root', NULL, 'x', 'x'),
                   ('n', 'n', 'mongodb', 'local', 'h', 27017, NULL, NULL, 'x', 'x');
                 ALTER TABLE connections DROP COLUMN uri_params;
                 ALTER TABLE connections DROP COLUMN srv;
                 ALTER TABLE connections DROP COLUMN tls_ca_path;
                 ALTER TABLE connections DROP COLUMN tls_cert_path;
                 ALTER TABLE connections DROP COLUMN tls_key_path;
                 ALTER TABLE connections DROP COLUMN transient;",
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        let store = Store::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let org = store.list_connection_org().unwrap_or_else(|e| panic!("{e}"));
        let folders: Vec<(&str, u32)> = org.folders.iter().map(|f| (f.name.as_str(), f.connection_count)).collect();
        assert_eq!(folders, [("Clients", 2), ("Internal", 1)], "case-insensitive duplicates merge");
        assert_eq!(store.get_connection("b").unwrap_or_else(|e| panic!("{e}")).folder.as_deref(), Some("Clients"));
        assert_eq!(store.get_connection("d").unwrap_or_else(|e| panic!("{e}")).folder, None);
        assert_eq!(store.get_connection("m").unwrap_or_else(|e| panic!("{e}")).extra.uri_params.as_deref(), Some("authSource=admin"));
        assert_eq!(store.get_connection("n").unwrap_or_else(|e| panic!("{e}")).extra.uri_params, None, "no user, no auth database");
        assert_eq!(org.tags.len(), 5);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
