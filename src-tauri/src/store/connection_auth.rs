// SOT: connection-auth-store, auth-config-column, auth-secret-ciphertext, auth-secret-flags

use crate::error::{AppError, AppResult};
use crate::model::{AuthSecretFlags, ConnectionAuth, ConnectionSummary};
use crate::store::connections::SecretChange;
use crate::store::Store;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

// ============================================================================
// SIGN-IN METHOD STORAGE
//
// WHAT:  Reads and writes how each saved connection signs in: the method and
//        its settings (JSON, plain), which secrets are set (JSON of booleans,
//        plain) and the sealed secrets (one AES-GCM blob).
// WHY:   Kept in its own file and read through a second query rather than the
//        main column list, so the connection table's own SQL does not change
//        shape every time a feature adds a column, and a connection that never
//        used a cloud identity costs nothing.
// HOW:   `set_connection_auth` runs after `insert_connection` / `update_connection`
//        (services::connection::save); `attach_auth` fills `auth` and
//        `has_auth_secrets` on summaries as they are read. NULL columns read as
//        the password method with no secrets.
// WHERE: src-tauri/src/store/migrations/auth.rs (the columns),
//        src-tauri/src/model/auth.rs (the shapes)
// ============================================================================

fn flags_from_json(raw: Option<&str>) -> AuthSecretFlags {
    raw.and_then(|raw| serde_json::from_str(raw).ok()).unwrap_or_default()
}

// WHAT:  Fills `auth` and `has_auth_secrets` on the summaries just read.
pub(crate) fn attach_auth(conn: &Connection, summaries: &mut [ConnectionSummary]) -> AppResult<()> {
    if summaries.is_empty() {
        return Ok(());
    }
    let mut stmt = conn
        .prepare("SELECT id, auth_config, auth_secret_flags FROM connections WHERE auth_config IS NOT NULL OR auth_secret_flags IS NOT NULL")
        .map_err(AppError::store)?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)))
        .map_err(AppError::store)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(AppError::store)?;
    let by_id: HashMap<String, (Option<String>, Option<String>)> = rows.into_iter().map(|(id, config, flags)| (id, (config, flags))).collect();
    for summary in summaries {
        if let Some((config, flags)) = by_id.get(&summary.id) {
            summary.auth = ConnectionAuth::from_json(config.as_deref());
            summary.has_auth_secrets = flags_from_json(flags.as_deref());
        }
    }
    Ok(())
}

impl Store {
    // WHAT:  Stores the sign-in method, which of its secrets are set, and the
    //        sealed secrets (`Keep` leaves the stored blob and flags alone).
    // HOW:   The password method with no secrets stores NULLs, so a connection
    //        that never left passwords carries no extra data.
    pub fn set_connection_auth(&self, id: &str, auth: &ConnectionAuth, flags: &AuthSecretFlags, secret: SecretChange) -> AppResult<()> {
        let config = if auth.is_password() { None } else { Some(auth.to_json()?) };
        let changed = self
            .conn()
            .execute("UPDATE connections SET auth_config = ?2 WHERE id = ?1", params![id, config])
            .map_err(AppError::store)?;
        if changed == 0 {
            return Err(AppError::not_found(format!("Connection {id} does not exist.")));
        }
        let (blob, store_flags) = match secret {
            SecretChange::Keep => return Ok(()),
            SecretChange::Set(bytes) => (Some(bytes), Some(serde_json::to_string(flags).map_err(|e| AppError::internal(e.to_string()))?)),
            SecretChange::Clear => (None, None),
        };
        self.conn()
            .execute(
                "UPDATE connections SET auth_secret_ciphertext = ?2, auth_secret_flags = ?3 WHERE id = ?1",
                params![id, blob, store_flags],
            )
            .map_err(AppError::store)?;
        Ok(())
    }

    // WHAT:  The sealed sign-in secrets of a connection, if it has any.
    pub fn auth_secret_ciphertext(&self, id: &str) -> AppResult<Option<Vec<u8>>> {
        let found: Option<Option<Vec<u8>>> = self
            .conn()
            .query_row("SELECT auth_secret_ciphertext FROM connections WHERE id = ?1", params![id], |row| row.get(0))
            .optional()
            .map_err(AppError::store)?;
        Ok(found.flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::auth::{AwsAccess, AwsCredentialSource, AzureAdMode};
    use crate::model::{ConnectionInput, Engine, Environment, SshTunnel, SslMode};

    fn input(name: &str) -> ConnectionInput {
        ConnectionInput {
            name: name.into(),
            engine: Engine::Postgres,
            environment: Environment::Local,
            read_only: false,
            host: Some("db.internal".into()),
            port: Some(5432),
            database: None,
            username: Some("iam_user".into()),
            password: None,
            file_path: None,
            ssl_mode: SslMode::Prefer,
            ssh: SshTunnel::default(),
            ssh_secret: None,
            folder: None,
            color: None,
            favorite: false,
            extra: Default::default(),
            auth: Default::default(),
            auth_secrets: Default::default(),
        }
    }

    fn rds() -> ConnectionAuth {
        ConnectionAuth::AwsRdsIam {
            region: Some("eu-west-1".into()),
            credentials: AwsAccess { source: AwsCredentialSource::AccessKeys { access_key_id: "AKIA".into() }, ..AwsAccess::default() },
            ca_bundle: None,
        }
    }

    #[test]
    fn a_new_connection_signs_in_with_a_password_and_has_no_auth_secrets() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let created = store.insert_connection(&input("plain"), None).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(created.auth, ConnectionAuth::Password);
        assert_eq!(created.has_auth_secrets, AuthSecretFlags::default());
        assert_eq!(store.auth_secret_ciphertext(&created.id).unwrap_or(Some(vec![1])), None);
    }

    #[test]
    fn method_flags_and_sealed_secrets_round_trip_through_reads() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let created = store.insert_connection(&input("aws"), None).unwrap_or_else(|e| panic!("{e}"));
        let flags = AuthSecretFlags { aws_secret_access_key: true, ..AuthSecretFlags::default() };
        store.set_connection_auth(&created.id, &rds(), &flags, SecretChange::Set(vec![7, 7, 7])).unwrap_or_else(|e| panic!("{e}"));

        let read = store.get_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(read.auth, rds());
        assert_eq!(read.has_auth_secrets, flags);
        assert_eq!(store.auth_secret_ciphertext(&created.id).unwrap_or_default(), Some(vec![7, 7, 7]));
        let listed = store.list_connections().unwrap_or_default();
        assert_eq!(listed.first().map(|c| c.auth.clone()), Some(rds()), "the list carries the method too");

        // Keep changes the method only; the sealed blob and the flags stay.
        let azure = ConnectionAuth::AzureAd { mode: AzureAdMode::Default, tenant_id: None, client_id: None, username: None, scope: None };
        store.set_connection_auth(&created.id, &azure, &AuthSecretFlags::default(), SecretChange::Keep).unwrap_or_else(|e| panic!("{e}"));
        let kept = store.get_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(kept.auth, azure);
        assert_eq!(kept.has_auth_secrets, flags);
        assert_eq!(store.auth_secret_ciphertext(&created.id).unwrap_or_default(), Some(vec![7, 7, 7]));

        // Back to a password with the secrets cleared: NULLs again.
        store.set_connection_auth(&created.id, &ConnectionAuth::Password, &AuthSecretFlags::default(), SecretChange::Clear).unwrap_or_else(|e| panic!("{e}"));
        let reset = store.get_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((reset.auth, reset.has_auth_secrets), (ConnectionAuth::Password, AuthSecretFlags::default()));
        assert_eq!(store.auth_secret_ciphertext(&created.id).unwrap_or(Some(vec![1])), None);
    }

    #[test]
    fn a_duplicate_keeps_the_sign_in_method_and_its_sealed_secrets() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let created = store.insert_connection(&input("aws"), None).unwrap_or_else(|e| panic!("{e}"));
        let flags = AuthSecretFlags { aws_secret_access_key: true, ..AuthSecretFlags::default() };
        store.set_connection_auth(&created.id, &rds(), &flags, SecretChange::Set(vec![9])).unwrap_or_else(|e| panic!("{e}"));
        let copy = store.duplicate_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((copy.auth, copy.has_auth_secrets), (rds(), flags));
        assert_eq!(store.auth_secret_ciphertext(&copy.id).unwrap_or_default(), Some(vec![9]));
    }

    // The "credentials lost" detection must count sign-in secrets too, or a missing
    // keychain entry would mint a new key and orphan them silently.
    #[test]
    fn sign_in_secrets_count_as_sealed_and_are_forgotten_with_the_rest() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let created = store.insert_connection(&input("aws"), None).unwrap_or_else(|e| panic!("{e}"));
        let flags = AuthSecretFlags { aws_secret_access_key: true, ..AuthSecretFlags::default() };
        store.set_connection_auth(&created.id, &rds(), &flags, SecretChange::Set(vec![1])).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.sealed_secret_count().unwrap_or(0), 1);
        store.clear_sealed_secrets().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.sealed_secret_count().unwrap_or(99), 0);
        let after = store.get_connection(&created.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(after.has_auth_secrets, AuthSecretFlags::default(), "no flag outlives its secret");
        assert_eq!(after.auth, rds(), "the method itself is not a secret and stays");
    }

    #[test]
    fn an_unknown_connection_is_reported_not_silently_ignored() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let err = store.set_connection_auth("missing", &ConnectionAuth::Password, &AuthSecretFlags::default(), SecretChange::Keep);
        assert!(matches!(err, Err(AppError::NotFound { .. })));
    }

    #[test]
    fn an_unreadable_method_falls_back_to_a_password() {
        let store = Store::open_in_memory().unwrap_or_else(|e| panic!("{e}"));
        let created = store.insert_connection(&input("future"), None).unwrap_or_else(|e| panic!("{e}"));
        store
            .conn()
            .execute("UPDATE connections SET auth_config = '{\"method\":\"from_the_future\"}' WHERE id = ?1", params![created.id])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.get_connection(&created.id).map(|c| c.auth).unwrap_or(rds()), ConnectionAuth::Password);
    }
}
