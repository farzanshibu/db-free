// SOT: connection-service, connection-lifecycle, secret-sealing, session-open-close, ssh-tofu-pin, session-ping

use crate::adapters::crypto;
use crate::guard::SessionCtx;
use crate::integrations::cloud_auth::{self, CloudSession};
use crate::integrations::{self, SessionInfo};
use crate::error::{AppError, AppResult};
use crate::model::{
    AuthSecretFlags, AuthSecrets, ConnectionAuth, CredentialStatus, ConnectionField, ConnectionFieldIssue, ConnectionInput,
    ConnectionSummary, FormKind, ResolvedConnection,
};
use crate::model::connection::expand_home;
use crate::state::AppState;
use crate::store::connections::SecretChange;

pub fn list(state: &AppState) -> AppResult<Vec<ConnectionSummary>> {
    state.with_store(|store| store.list_connections())
}

// WHAT:  Creates or updates a connection: the row (`save_row`), then how it
//        signs in (`model::auth`) with its sealed secrets.
// WHY:   Keys, client secrets and tokens are as sensitive as the password and
//        follow the same rule: sealed on arrival, never returned.
// HOW:   The sign-in secrets are merged with the stored ones first (a blank field
//        keeps its stored value) so a failure there stops the save before the row
//        changes.
// WHERE: src-tauri/src/store/connection_auth.rs
pub fn save(state: &AppState, id: Option<&str>, input: &ConnectionInput) -> AppResult<ConnectionSummary> {
    let auth = prepare_auth(state, id, input)?;
    let summary = save_row(state, id, input)?;
    state.with_store(|store| {
        store.set_connection_auth(&summary.id, &input.auth, &auth.flags, auth.secret)?;
        store.get_connection(&summary.id)
    })
}

// WHAT:  How a save changes the stored sign-in secrets.
struct AuthChange {
    flags: AuthSecretFlags,
    secret: SecretChange,
}

// WHAT:  The sealed sign-in secrets after this save: stored ones overlaid with the
//        typed ones, minus whatever the chosen method does not use.
// HOW:   A save that types nothing and keeps the method needs no key at all.
fn prepare_auth(state: &AppState, id: Option<&str>, input: &ConnectionInput) -> AppResult<AuthChange> {
    let family = input.engine.family();
    let stored_auth = match id {
        Some(id) => state.with_store(|store| store.get_connection(id))?.auth,
        None => ConnectionAuth::Password,
    };
    if id.is_some() && !input.auth_secrets.has_changes() && stored_auth == input.auth {
        return Ok(AuthChange { flags: AuthSecretFlags::default(), secret: SecretChange::Keep });
    }
    let stored = match id {
        Some(id) => stored_auth_secrets(state, id)?,
        None => AuthSecrets::default(),
    };
    let merged = stored.overlay(&input.auth_secrets).retain_for(&input.auth, family);
    if merged.is_empty() {
        return Ok(AuthChange { flags: AuthSecretFlags::default(), secret: SecretChange::Clear });
    }
    let sealed = crypto::seal(state.master_key()?, merged.to_json()?.as_bytes())?;
    Ok(AuthChange { flags: merged.flags(), secret: SecretChange::Set(sealed) })
}

fn stored_auth_secrets(state: &AppState, id: &str) -> AppResult<AuthSecrets> {
    let blob = state.with_store(|store| store.auth_secret_ciphertext(id))?;
    match unseal(state, blob.as_deref())? {
        Some(json) => AuthSecrets::from_json(&json),
        None => Ok(AuthSecrets::default()),
    }
}

// WHAT:  Creates or updates a connection row, sealing the password (and the SSH
//        password / key passphrase) before either is stored.
// WHY:   Passwords never touch disk in the clear (PRD 4.1).
// HOW:   Blank secret on update = keep the existing one, for each secret.
// WHERE: src-tauri/src/adapters/crypto.rs
fn save_row(state: &AppState, id: Option<&str>, input: &ConnectionInput) -> AppResult<ConnectionSummary> {
    let sealed = seal_nonblank(state, input.password.as_deref())?;
    let sealed_ssh = seal_nonblank(state, input.ssh_secret.as_deref())?;
    let stored = input.without_password();
    state.with_store(|store| {
        let summary = match id {
            Some(id) => {
                let change = match sealed {
                    Some(bytes) => SecretChange::Set(bytes),
                    None => SecretChange::Keep,
                };
                store.update_connection(id, &stored, change)?
            }
            None => return store.insert_connection_full(&stored, sealed, sealed_ssh, false),
        };
        match sealed_ssh {
            Some(bytes) => {
                store.set_ssh_secret(&summary.id, SecretChange::Set(bytes))?;
                store.get_connection(&summary.id)
            }
            None => Ok(summary),
        }
    })
}

// WHAT:  A copy of a saved connection, "(copy)" appended to the name.
// WHY:   The sealed password and SSH secret are copied as ciphertext inside the
//        store, so a duplicate keeps working credentials without the master key
//        and without a decrypted secret ever existing in memory.
pub fn duplicate(state: &AppState, id: &str) -> AppResult<ConnectionSummary> {
    state.with_store(|store| store.duplicate_connection(id))
}

// WHAT:  Every problem with an input the form can show: the pure rules in
//        `ConnectionInput::check`, plus the files the connection points at.
// WHY:   A certificate or key path that does not exist only fails at connect
//        time with a driver error naming no field. Checked here (not in `save`)
//        so a path on a drive that is offline right now never blocks saving.
// HOW:   `~` is expanded the way the adapters expand it.
pub fn validate(input: &ConnectionInput) -> Vec<ConnectionFieldIssue> {
    let mut issues = input.check();
    let mut check_file = |field: ConnectionField, path: &Option<String>, what: &str| {
        let Some(path) = path.as_deref().map(str::trim).filter(|p| !p.is_empty()) else {
            return;
        };
        if !expand_home(path).is_file() {
            issues.push(ConnectionFieldIssue::new(field, format!("{what} not found: {path}")));
        }
    };
    if input.engine.form() == FormKind::Server {
        check_file(ConnectionField::TlsCaPath, &input.extra.tls.ca_path, "CA file");
        check_file(ConnectionField::TlsCertPath, &input.extra.tls.cert_path, "Client certificate");
        check_file(ConnectionField::TlsKeyPath, &input.extra.tls.key_path, "Client key");
        if input.ssh.enabled && input.ssh.auth == crate::model::SshAuth::PrivateKey {
            check_file(ConnectionField::SshKeyPath, &input.ssh.key_path, "SSH key file");
        }
    }
    issues
}

pub fn credential_status(state: &AppState) -> AppResult<CredentialStatus> {
    state.credential_status()
}

pub fn reset_credentials(state: &AppState) -> AppResult<CredentialStatus> {
    state.reset_credentials()
}

fn seal_nonblank(state: &AppState, secret: Option<&str>) -> AppResult<Option<Vec<u8>>> {
    match secret.filter(|s| !s.is_empty()) {
        Some(secret) => Ok(Some(crypto::seal(state.master_key()?, secret.as_bytes())?)),
        None => Ok(None),
    }
}

pub async fn delete(state: &AppState, id: &str) -> AppResult<()> {
    state.remove_session(id).await;
    state.with_store(|store| store.delete_connection(id))
}

// WHAT:  A saved connection with its secrets unsealed, in memory only: the
//        password, the SSH secret and the cloud sign-in secrets.
pub(crate) struct Resolved {
    pub(crate) connection: ResolvedConnection,
    pub(crate) ssh_secret: Option<String>,
    pub(crate) auth_secrets: AuthSecrets,
}

pub(crate) fn resolve(state: &AppState, id: &str) -> AppResult<Resolved> {
    let record = state.with_store(|store| store.get_connection_record(id))?;
    let secret = unseal(state, record.secret_ciphertext.as_deref())?;
    let ssh_secret = unseal(state, record.ssh_secret_ciphertext.as_deref())?;
    let auth_secrets = stored_auth_secrets(state, id)?;
    Ok(Resolved { connection: ResolvedConnection { summary: record.summary, secret }, ssh_secret, auth_secrets })
}

fn unseal(state: &AppState, blob: Option<&[u8]>) -> AppResult<Option<String>> {
    match blob {
        Some(blob) => {
            let bytes = crypto::open(state.master_key()?, blob)?;
            Ok(Some(String::from_utf8(bytes).map_err(|_| AppError::crypto("stored secret is not UTF-8"))?))
        }
        None => Ok(None),
    }
}

// WHAT:  The password a native tool (pg_dump, mysqldump…) logs in with: the saved
//        one, or, for a connection that signs in with a cloud identity, a token
//        minted now (valid for the minutes the dump needs).
// WHY:   Those connections store no password, so handing the tool `secret` would
//        make every backup of an IAM-authenticated database fail to log in.
// WHERE: src-tauri/src/services/backup.rs
pub(crate) async fn native_tool_secret(state: &AppState, id: &str) -> AppResult<Option<String>> {
    let Resolved { connection, auth_secrets, .. } = resolve(state, id)?;
    let token_login = matches!(
        connection.summary.auth,
        ConnectionAuth::AwsRdsIam { .. } | ConnectionAuth::AzureAd { .. } | ConnectionAuth::GcpCloudSqlIam { .. }
    );
    if !token_login {
        return Ok(connection.secret);
    }
    let session = CloudSession::for_connection(&connection.summary, auth_secrets);
    Ok(cloud_auth::with_session(session, cloud_auth::db_plan(&connection)).await?.password)
}

// WHAT:  Trust on first use for the SSH server key: the first fingerprint a
//        tunnel sees for a connection is pinned; later connects must match it.
// WHY:   Without a pin anyone able to intercept the SSH hop could impersonate
//        the bastion and read the database traffic it forwards.
// HOW:   The tunnel verifies against `summary.ssh.host_key` itself; this only
//        records the fingerprint when nothing was pinned yet.
// WHERE: src-tauri/src/integrations/ssh_tunnel.rs, src-tauri/src/store/connections.rs
fn pin_host_key(state: &AppState, id: &str, resolved: &ResolvedConnection, integration: &dyn integrations::Integration) {
    if resolved.summary.ssh.host_key.is_some() {
        return;
    }
    if let Some(fingerprint) = integration.ssh_host_key() {
        if let Err(err) = state.with_store(|store| store.pin_ssh_host_key(id, &fingerprint)) {
            log::warn!("could not pin the SSH host key for {id}: {err}");
        }
    }
}

// WHAT:  Opens the adapter session, with the cloud sign-in (if the connection has
//        one) available to the adapter while it connects.
// WHY:   The session is built from the connection *as saved*, before an SSH tunnel
//        rewrites its host: an RDS token is signed for the real endpoint.
// WHERE: src-tauri/src/integrations/cloud_auth.rs
async fn open_session(
    resolved: &ResolvedConnection,
    ssh_secret: Option<&str>,
    auth_secrets: AuthSecrets,
) -> AppResult<std::sync::Arc<dyn integrations::Integration>> {
    let session = CloudSession::for_connection(&resolved.summary, auth_secrets);
    cloud_auth::with_session(session, integrations::connect_with_ssh(resolved, ssh_secret)).await
}

// WHAT:  Opens an adapter session for a saved connection and registers it.
// HOW:   `database` overrides the saved default for this session only, which is
//        how the sidebar's database switcher works (reconnect, replace session).
pub async fn connect(state: &AppState, id: &str, database: Option<&str>) -> AppResult<ConnectionSummary> {
    let Resolved { connection: mut resolved, ssh_secret, auth_secrets } = resolve(state, id)?;
    if let Some(db) = database.map(str::trim).filter(|d| !d.is_empty()) {
        resolved.summary.database = Some(db.to_string());
    }
    let integration = open_session(&resolved, ssh_secret.as_deref(), auth_secrets).await?;
    if let Err(err) = integration.ping().await {
        integration.close().await;
        return Err(err);
    }
    pin_host_key(state, id, &resolved, integration.as_ref());
    state.insert_session(id.to_string(), integration).await;
    // Re-read so a key pinned just now reaches the UI with the session.
    let mut summary = state.with_store(|store| store.get_connection(id))?;
    summary.database = resolved.summary.database;
    Ok(summary)
}

pub async fn disconnect(state: &AppState, id: &str) -> AppResult<()> {
    state.remove_session(id).await;
    Ok(())
}

// WHAT:  "Test connection" for unsaved input. Reuses the stored secrets when the
//        form left a password blank on an existing connection.
// HOW:   Goes through the SSH tunnel exactly like `connect`, verifying against
//        the pin the form carries. A test never pins: the key is recorded on the
//        first real connect of the saved connection.
pub async fn test(state: &AppState, existing_id: Option<&str>, input: &ConnectionInput) -> AppResult<()> {
    let stored = match existing_id {
        Some(id) => Some(resolve(state, id)?),
        None => None,
    };
    let stored_auth = stored.as_ref().map(|r| r.auth_secrets.clone()).unwrap_or_default();
    let auth_secrets = stored_auth.overlay(&input.auth_secrets).retain_for(&input.auth, input.engine.family());
    let typed = |value: Option<&str>| value.filter(|v| !v.is_empty()).map(str::to_string);
    let secret = typed(input.password.as_deref())
        .or_else(|| stored.as_ref().and_then(|r| r.connection.secret.clone()));
    let ssh_secret = typed(input.ssh_secret.as_deref()).or_else(|| stored.and_then(|r| r.ssh_secret));
    let resolved = ResolvedConnection {
        summary: ConnectionSummary::draft_with_ssh(input, secret.is_some(), ssh_secret.is_some()),
        secret,
    };
    let integration = open_session(&resolved, ssh_secret.as_deref(), auth_secrets).await?;
    let ping = integration.ping().await;
    integration.close().await;
    ping
}

pub async fn active_sessions(state: &AppState) -> Vec<String> {
    state.session_ids().await
}

// WHAT:  Times one `Integration::ping` on the session (the health indicator).
// HOW:   Its own short deadline: a hung socket should read as "lost" within
//        seconds, not after the block's five-minute request timeout.
pub async fn ping(ctx: &SessionCtx) -> AppResult<u64> {
    let started = std::time::Instant::now();
    tokio::time::timeout(PING_TIMEOUT, ctx.integration.ping())
        .await
        .map_err(|_| AppError::timeout("The server did not answer the ping."))??;
    Ok(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
}

const PING_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub async fn describe(ctx: &SessionCtx) -> AppResult<SessionInfo> {
    integrations::describe(ctx.integration.as_ref()).await
}

// WHAT:  Engine of a saved connection without opening a session (SQL generation
//        needs the dialect before the guard resolves anything).
pub fn engine_of(state: &AppState, id: &str) -> AppResult<crate::model::Engine> {
    Ok(state.with_store(|store| store.get_connection(id))?.engine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::auth::{AwsAccess, AwsCredentialSource, AzureAdMode};
    use crate::model::{Engine, Environment, SshTunnel, SslMode};
    use crate::store::Store;

    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";

    fn state() -> AppState {
        AppState::new(Store::open_in_memory().unwrap_or_else(|e| panic!("{e}")), Box::new(MemoryKeyProvider::default()))
    }

    fn input(auth: ConnectionAuth, auth_secrets: AuthSecrets) -> ConnectionInput {
        ConnectionInput {
            name: "orders".into(),
            engine: Engine::Postgres,
            environment: Environment::Local,
            read_only: false,
            host: Some("orders.abc123.eu-west-1.rds.amazonaws.com".into()),
            port: Some(5432),
            database: Some("orders".into()),
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
            auth,
            auth_secrets,
        }
    }

    fn rds_keys() -> ConnectionAuth {
        ConnectionAuth::AwsRdsIam {
            region: None,
            credentials: AwsAccess { source: AwsCredentialSource::AccessKeys { access_key_id: "AKIDEXAMPLE".into() }, ..AwsAccess::default() },
            ca_bundle: None,
        }
    }

    fn secrets(key: Option<&str>, token: Option<&str>) -> AuthSecrets {
        AuthSecrets { aws_secret_access_key: key.map(str::to_string), aws_session_token: token.map(str::to_string), ..AuthSecrets::default() }
    }

    fn stored(state: &AppState, id: &str) -> AuthSecrets {
        resolve(state, id).map(|r| r.auth_secrets).unwrap_or_else(|e| panic!("{e}"))
    }

    // WHAT:  The secrets go in sealed, come back only as flags, and are readable again in memory.
    #[test]
    fn a_cloud_connection_seals_its_secrets_and_the_summary_carries_flags_only() {
        let state = state();
        let saved = save(&state, None, &input(rds_keys(), secrets(Some(SECRET), None))).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(saved.auth, rds_keys());
        assert!(saved.has_auth_secrets.aws_secret_access_key && !saved.has_auth_secrets.aws_session_token);
        let json = serde_json::to_string(&saved).unwrap_or_default();
        assert!(!json.contains(SECRET), "the secret never reaches the UI shape: {json}");
        let blob = state.with_store(|s| s.auth_secret_ciphertext(&saved.id)).unwrap_or_default().unwrap_or_default();
        assert!(!blob.is_empty() && !String::from_utf8_lossy(&blob).contains(SECRET), "sealed on disk");
        assert_eq!(stored(&state, &saved.id).aws_secret_access_key.as_deref(), Some(SECRET));
        // The listing is what the UI loads: still no value, same flags.
        let listed = list(&state).unwrap_or_default();
        assert_eq!(listed.first().map(|c| c.has_auth_secrets.aws_secret_access_key), Some(true));
    }

    #[test]
    fn saving_again_without_typing_secrets_keeps_them() {
        let state = state();
        let saved = save(&state, None, &input(rds_keys(), secrets(Some(SECRET), Some("tok")))).unwrap_or_else(|e| panic!("{e}"));
        let mut edit = input(rds_keys(), AuthSecrets::default());
        edit.name = "orders (renamed)".into();
        let again = save(&state, Some(&saved.id), &edit).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.name, "orders (renamed)");
        assert!(again.has_auth_secrets.aws_secret_access_key && again.has_auth_secrets.aws_session_token);
        let kept = stored(&state, &saved.id);
        assert_eq!((kept.aws_secret_access_key.as_deref(), kept.aws_session_token.as_deref()), (Some(SECRET), Some("tok")));
    }

    #[test]
    fn typing_one_secret_replaces_it_and_an_empty_string_removes_it() {
        let state = state();
        let saved = save(&state, None, &input(rds_keys(), secrets(Some(SECRET), Some("tok")))).unwrap_or_else(|e| panic!("{e}"));
        save(&state, Some(&saved.id), &input(rds_keys(), secrets(None, Some("new-token")))).unwrap_or_else(|e| panic!("{e}"));
        let replaced = stored(&state, &saved.id);
        assert_eq!((replaced.aws_secret_access_key.as_deref(), replaced.aws_session_token.as_deref()), (Some(SECRET), Some("new-token")));
        let removed = save(&state, Some(&saved.id), &input(rds_keys(), secrets(None, Some("")))).unwrap_or_else(|e| panic!("{e}"));
        assert!(removed.has_auth_secrets.aws_secret_access_key && !removed.has_auth_secrets.aws_session_token);
        assert_eq!(stored(&state, &saved.id).aws_session_token, None);
    }

    // WHAT:  Switching method must not leave a key sealed on disk with nothing using it.
    #[test]
    fn changing_the_method_drops_the_secrets_it_no_longer_uses() {
        let state = state();
        let saved = save(&state, None, &input(rds_keys(), secrets(Some(SECRET), Some("tok")))).unwrap_or_else(|e| panic!("{e}"));
        let profile = ConnectionAuth::AwsRdsIam {
            region: None,
            credentials: AwsAccess { source: AwsCredentialSource::Profile { name: "dev".into() }, ..AwsAccess::default() },
            ca_bundle: None,
        };
        let switched = save(&state, Some(&saved.id), &input(profile.clone(), AuthSecrets::default())).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(switched.auth, profile);
        assert_eq!(switched.has_auth_secrets, AuthSecretFlags::default());
        assert!(state.with_store(|s| s.auth_secret_ciphertext(&saved.id)).unwrap_or_default().is_none());
        // Typing a secret the new method has no use for is ignored the same way.
        let azure = ConnectionAuth::AzureAd { mode: AzureAdMode::Default, tenant_id: None, client_id: None, username: None, scope: None };
        let ignored = save(&state, Some(&saved.id), &input(azure, secrets(Some(SECRET), None))).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ignored.has_auth_secrets, AuthSecretFlags::default());
    }

    #[test]
    fn a_password_connection_stores_no_sign_in_secrets() {
        let state = state();
        let mut plain = input(ConnectionAuth::Password, AuthSecrets::default());
        plain.password = Some("hunter2".into());
        let saved = save(&state, None, &plain).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((saved.auth.clone(), saved.has_auth_secrets), (ConnectionAuth::Password, AuthSecretFlags::default()));
        assert!(saved.has_secret);
        assert!(state.with_store(|s| s.auth_secret_ciphertext(&saved.id)).unwrap_or_default().is_none());
        let resolved = resolve(&state, &saved.id).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resolved.connection.secret.as_deref(), Some("hunter2"));
        assert!(resolved.auth_secrets.is_empty());
    }

    // WHAT:  A backup tool gets the saved password, or a token minted now for an IAM connection.
    #[test]
    fn native_tools_log_in_with_the_password_or_a_fresh_token() {
        let state = state();
        let mut plain = input(ConnectionAuth::Password, AuthSecrets::default());
        plain.password = Some("hunter2".into());
        let password_id = save(&state, None, &plain).unwrap_or_else(|e| panic!("{e}")).id;
        let secret = futures::executor::block_on(native_tool_secret(&state, &password_id)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(secret.as_deref(), Some("hunter2"));

        let mut iam = input(rds_keys(), secrets(Some(SECRET), None));
        iam.name = "orders-iam".into();
        let iam_id = save(&state, None, &iam).unwrap_or_else(|e| panic!("{e}")).id;
        let token = futures::executor::block_on(native_tool_secret(&state, &iam_id)).unwrap_or_else(|e| panic!("{e}")).unwrap_or_default();
        assert!(token.starts_with("orders.abc123.eu-west-1.rds.amazonaws.com:5432/?Action=connect&DBUser=iam_user&"), "{token}");
    }

    // WHAT:  `save_connection` validates before the service runs; the rule it applies is the
    //        engine/method matrix, so a method the engine does not offer never reaches the store.
    #[test]
    fn a_method_the_engine_does_not_offer_is_refused_before_anything_is_stored() {
        let wrong = ConnectionAuth::MongoAtlasOidc { provider: crate::model::auth::OidcProvider::K8s, token_resource: None };
        assert!(input(wrong, AuthSecrets::default()).validate().is_err(), "Atlas workload identity is not a PostgreSQL method");
        assert!(input(rds_keys(), AuthSecrets::default()).validate().is_ok());
        let mut sql_server = input(rds_keys(), AuthSecrets::default());
        sql_server.engine = Engine::Mssql;
        assert!(sql_server.validate().is_err(), "RDS IAM is not offered for SQL Server");
    }
}
