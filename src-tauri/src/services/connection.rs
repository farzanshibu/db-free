// SOT: connection-service, connection-lifecycle, secret-sealing, session-open-close, ssh-tofu-pin, session-ping

use crate::adapters::crypto;
use crate::guard::SessionCtx;
use crate::integrations::{self, SessionInfo};
use crate::error::{AppError, AppResult};
use crate::model::{ConnectionInput, ConnectionSummary, ResolvedConnection};
use crate::state::AppState;
use crate::store::connections::SecretChange;

pub fn list(state: &AppState) -> AppResult<Vec<ConnectionSummary>> {
    state.with_store(|store| store.list_connections())
}

// WHAT:  Creates or updates a connection, sealing the password (and the SSH
//        password / key passphrase) before either is stored.
// WHY:   Passwords never touch disk in the clear (PRD 4.1).
// HOW:   Blank secret on update = keep the existing one, for each secret.
// WHERE: src-tauri/src/adapters/crypto.rs
pub fn save(state: &AppState, id: Option<&str>, input: &ConnectionInput) -> AppResult<ConnectionSummary> {
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
            None => store.insert_connection(&stored, sealed)?,
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

// WHAT:  A saved connection with both secrets unsealed, in memory only.
pub(crate) struct Resolved {
    pub(crate) connection: ResolvedConnection,
    pub(crate) ssh_secret: Option<String>,
}

pub(crate) fn resolve(state: &AppState, id: &str) -> AppResult<Resolved> {
    let record = state.with_store(|store| store.get_connection_record(id))?;
    let secret = unseal(state, record.secret_ciphertext.as_deref())?;
    let ssh_secret = unseal(state, record.ssh_secret_ciphertext.as_deref())?;
    Ok(Resolved { connection: ResolvedConnection { summary: record.summary, secret }, ssh_secret })
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

// WHAT:  Opens an adapter session for a saved connection and registers it.
// HOW:   `database` overrides the saved default for this session only, which is
//        how the sidebar's database switcher works (reconnect, replace session).
pub async fn connect(state: &AppState, id: &str, database: Option<&str>) -> AppResult<ConnectionSummary> {
    let Resolved { connection: mut resolved, ssh_secret } = resolve(state, id)?;
    if let Some(db) = database.map(str::trim).filter(|d| !d.is_empty()) {
        resolved.summary.database = Some(db.to_string());
    }
    let integration = integrations::connect_with_ssh(&resolved, ssh_secret.as_deref()).await?;
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
    let typed = |value: Option<&str>| value.filter(|v| !v.is_empty()).map(str::to_string);
    let secret = typed(input.password.as_deref())
        .or_else(|| stored.as_ref().and_then(|r| r.connection.secret.clone()));
    let ssh_secret = typed(input.ssh_secret.as_deref()).or_else(|| stored.and_then(|r| r.ssh_secret));
    let resolved = ResolvedConnection {
        summary: ConnectionSummary::draft_with_ssh(input, secret.is_some(), ssh_secret.is_some()),
        secret,
    };
    let integration = integrations::connect_with_ssh(&resolved, ssh_secret.as_deref()).await?;
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
