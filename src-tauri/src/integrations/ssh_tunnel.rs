// SOT: ssh-tunnel, ssh-port-forward, ssh-host-key-check, tunnelled-integration

use crate::error::{AppError, AppResult};
use crate::integrations::{Capabilities, Integration};
use crate::model::connection::expand_home;
use crate::model::{
    ColumnInfo, ConnectionSummary, Engine, Family, FilterRule, ForeignKey, ObjectDetail, ObjectKind, ObjectRef, ObjectSummary,
    PageQuery, RangeQueryRequest, RangeResult, ResolvedConnection, ResultSet, SchemaCatalog, SearchRequest,
    SearchResult, ServerStats, SshAuth, SslMode, StatementResult, TableRef, VectorSearchRequest,
};
use async_trait::async_trait;
use russh::client::{self, AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::MethodKind;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

// ============================================================================
// SSH TUNNEL
//
// WHAT:  Local port forwarding over SSH (`ssh -L 127.0.0.1:<free>:db:5432 bastion`)
//        done in-process with `russh`, and an `Integration` wrapper that owns
//        the tunnel for as long as the session lives.
// WHY:   Databases in a private network are reachable only from a bastion. A
//        tunnel below the adapters means all of them (Postgres, MySQL, Redis,
//        Mongo …) work through it without knowing it exists.
// HOW:   1. connect + authenticate to the SSH host (password or key file);
//           the server key is checked against the pinned fingerprint.
//        2. bind 127.0.0.1:0; every accepted socket gets its own `direct-tcpip`
//           channel to target host:port, as resolved *by the SSH server*.
//        3. `integrations::connect_with_ssh` hands the adapter the same
//           connection with host/port rewritten to the local end.
//        4. `Tunnelled::close()` closes the adapter, then the tunnel.
//        Authentication: password, private key, or — when the server offers
//        `keyboard-interactive` instead of `password` (PAM setups) — the same
//        password answering its prompt. A prompt that asks for anything else
//        (a verification code) is reported by name rather than guessed at.
//        TLS through the tunnel: the adapter dials 127.0.0.1, so a driver that
//        checks the certificate against the host it dialled would fail. The tunnel
//        therefore records the real host (`extra.tunnel_origin_host`). SQL Server
//        connects to the tunnel's port but verifies the certificate against that
//        real name. sqlx (Postgres, MySQL) cannot separate the dial address from
//        the verified name, so `verify_full` is applied as `verify_ca` (the chain
//        is still checked; the SSH hop already authenticated the path). The
//        MongoDB driver verifies against the dialled host: use `require` and a
//        certificate that names it, or no TLS inside the tunnel. Clustered
//        drivers that discover other nodes (Mongo replica sets, Cassandra, Kafka
//        brokers) are only reachable when they advertise the tunnelled address.
//        One host is forwarded, never a list.
// WHERE: src-tauri/src/integrations/mod.rs (connect_with_ssh),
//        src-tauri/src/services/connection.rs (TOFU pin), model::SshTunnel
// ============================================================================

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const KEEPALIVE: Duration = Duration::from_secs(30);

// WHAT:  The server-key decision, kept apart from russh so it is unit-testable.
// HOW:   No pin = accept (trust on first use; the caller pins what was seen).
//        A pin = the fingerprint must match exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyVerdict {
    FirstUse,
    Match,
    Mismatch { expected: String, actual: String },
}

pub fn verify_host_key(pinned: Option<&str>, actual: &str) -> HostKeyVerdict {
    match pinned.map(str::trim).filter(|p| !p.is_empty()) {
        None => HostKeyVerdict::FirstUse,
        Some(expected) if expected == actual => HostKeyVerdict::Match,
        Some(expected) => HostKeyVerdict::Mismatch { expected: expected.to_string(), actual: actual.to_string() },
    }
}

// WHAT:  russh client callbacks: only the server-key check matters here.
struct Client {
    pinned: Option<String>,
    /// What the server presented and what we decided, read back after connect.
    seen: Arc<Mutex<Option<(String, HostKeyVerdict)>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, server_public_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key.public_key().fingerprint(HashAlg::Sha256).to_string();
        let verdict = verify_host_key(self.pinned.as_deref(), &fingerprint);
        let accept = !matches!(verdict, HostKeyVerdict::Mismatch { .. });
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some((fingerprint, verdict));
        }
        Ok(accept)
    }
}

// WHAT:  Where the tunnel forwards to: the connection's host and port as the
//        SSH server resolves them.
// HOW:   Accepts `host`, `host:port`, `[v6]:port` and bare IPv6. An explicit
//        port in the host field wins over the port field, which wins over the
//        engine default — the same order the adapters use.
pub fn target_of(summary: &ConnectionSummary) -> AppResult<(String, u16)> {
    let raw = summary.host.as_deref().map(str::trim).filter(|h| !h.is_empty()).unwrap_or("127.0.0.1");
    if raw.contains(',') {
        return Err(AppError::invalid_input(
            "An SSH tunnel forwards to a single host. Put one host in the Host field.",
        ));
    }
    if raw.contains("://") {
        return Err(AppError::invalid_input("With an SSH tunnel the Host field takes a host name, not a URL."));
    }
    let fallback = summary.port.or(summary.engine.default_port());
    let (host, explicit) = if let Some(rest) = raw.strip_prefix('[') {
        match rest.split_once(']') {
            Some((v6, tail)) => (v6.to_string(), tail.strip_prefix(':').and_then(|p| p.parse::<u16>().ok())),
            None => return Err(AppError::invalid_input(format!("\"{raw}\" is not a valid host."))),
        }
    } else {
        match raw.split_once(':') {
            Some((name, port)) if !port.contains(':') => match port.parse::<u16>() {
                Ok(port) => (name.to_string(), Some(port)),
                Err(_) => return Err(AppError::invalid_input(format!("\"{raw}\" has an invalid port."))),
            },
            _ => (raw.to_string(), None),
        }
    };
    let port = explicit.or(fallback).ok_or_else(|| AppError::invalid_input("Set the database port for the SSH tunnel."))?;
    Ok((host, port))
}

// WHAT:  The same connection, pointed at the tunnel's local end.
// HOW:   Remembers the real host for drivers that can verify it, and relaxes
//        `verify_full` to `verify_ca` for the drivers that cannot (see the header).
pub fn rewrite_to_local(conn: &ResolvedConnection, local_port: u16) -> ResolvedConnection {
    let mut local = conn.clone();
    local.summary.extra.tunnel_origin_host = target_of(&conn.summary).ok().map(|(host, _)| host);
    local.summary.host = Some("127.0.0.1".to_string());
    local.summary.port = Some(local_port);
    if local.summary.ssl_mode == SslMode::VerifyFull && matches!(local.summary.engine.family(), Family::Postgres | Family::Mysql) {
        local.summary.ssl_mode = SslMode::VerifyCa;
    }
    local
}

// WHAT:  A live SSH session plus the local listener that forwards through it.
pub struct Tunnel {
    session: Arc<Handle<Client>>,
    forwarder: JoinHandle<()>,
    local_port: u16,
    host_key: Option<String>,
}

impl std::fmt::Debug for Tunnel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tunnel").field("local_port", &self.local_port).finish_non_exhaustive()
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // Dropping the forwarder's JoinSet aborts every forwarded socket too.
        self.forwarder.abort();
    }
}

impl Tunnel {
    // WHAT:  Opens the SSH session and starts forwarding. `secret` is the SSH
    //        password (Password auth) or the key passphrase (PrivateKey auth).
    pub async fn open(summary: &ConnectionSummary, secret: Option<&str>) -> AppResult<Tunnel> {
        let ssh = &summary.ssh;
        let (target_host, target_port) = target_of(summary)?;
        let ssh_host = ssh.host.as_deref().map(str::trim).filter(|h| !h.is_empty())
            .ok_or_else(|| AppError::invalid_input("SSH host is required when the tunnel is on."))?;
        let user = ssh.user.as_deref().map(str::trim).filter(|u| !u.is_empty())
            .ok_or_else(|| AppError::invalid_input("SSH user is required when the tunnel is on."))?;

        let config = Arc::new(client::Config {
            keepalive_interval: Some(KEEPALIVE),
            nodelay: true,
            ..client::Config::default()
        });
        let seen = Arc::new(Mutex::new(None));
        let handler = Client { pinned: ssh.host_key.clone(), seen: Arc::clone(&seen) };
        let connecting = client::connect(config, (ssh_host.to_string(), ssh.port), handler);
        let connected = tokio::time::timeout(CONNECT_TIMEOUT, connecting)
            .await
            .map_err(|_| AppError::timeout(format!("SSH host {ssh_host}:{} did not answer.", ssh.port)))?;
        let observed = seen.lock().ok().and_then(|s| s.clone());
        if let Some((_, HostKeyVerdict::Mismatch { expected, actual })) = &observed {
            return Err(AppError::driver(format!(
                "SSH host key for {ssh_host} changed: pinned {expected}, server presented {actual}. \
                 If the server was re-keyed on purpose, clear the host key fingerprint on the SSH tab."
            )));
        }
        let mut session = connected.map_err(|e| AppError::driver(format!("SSH connection to {ssh_host} failed: {e}")))?;

        let authenticated = match ssh.auth {
            SshAuth::Password => authenticate_with_password(&mut session, user, secret.unwrap_or_default()).await?,
            SshAuth::PrivateKey => {
                let path = ssh.key_path.as_deref().map(str::trim).filter(|p| !p.is_empty())
                    .ok_or_else(|| AppError::invalid_input("Choose the SSH private key file."))?;
                let key = keys::load_secret_key(expand_home(path), secret.filter(|s| !s.is_empty()))
                    .map_err(|e| AppError::invalid_input(format!("Could not read the SSH key {path}: {e}")))?;
                let rsa_hash = session.best_supported_rsa_hash().await.map_err(ssh_error)?.flatten();
                session
                    .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash))
                    .await
                    .map_err(ssh_error)?
                    .success()
            }
        };
        if !authenticated {
            return Err(AppError::driver(format!("SSH authentication as {user}@{ssh_host} was rejected.")));
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(AppError::driver)?;
        let local_port = listener.local_addr().map_err(AppError::driver)?.port();
        let session = Arc::new(session);
        let forwarder = tokio::spawn(forward(listener, Arc::clone(&session), target_host, target_port));
        Ok(Tunnel { session, forwarder, local_port, host_key: observed.map(|(fingerprint, _)| fingerprint) })
    }

    pub fn rewrite(&self, conn: &ResolvedConnection) -> ResolvedConnection {
        rewrite_to_local(conn, self.local_port)
    }

    pub fn is_closed(&self) -> bool {
        self.session.is_closed() || self.forwarder.is_finished()
    }

    pub async fn close(&self) {
        self.forwarder.abort();
        let _ = self.session.disconnect(russh::Disconnect::ByApplication, "session closed", "en").await;
    }
}

// WHAT:  Accept loop: one `direct-tcpip` channel per local socket, bytes copied
//        both ways until either side closes.
async fn forward(listener: TcpListener, session: Arc<Handle<Client>>, host: String, port: u16) {
    let mut sockets = JoinSet::new();
    loop {
        let (mut socket, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(err) => {
                log::warn!("ssh tunnel stopped accepting: {err}");
                return;
            }
        };
        // Reap finished sockets so a long session does not accumulate handles.
        while sockets.try_join_next().is_some() {}
        let session = Arc::clone(&session);
        let host = host.clone();
        sockets.spawn(async move {
            let channel = session
                .channel_open_direct_tcpip(host.as_str(), u32::from(port), peer.ip().to_string(), u32::from(peer.port()))
                .await;
            match channel {
                Ok(channel) => {
                    let mut stream = channel.into_stream();
                    if let Err(err) = tokio::io::copy_bidirectional(&mut socket, &mut stream).await {
                        log::debug!("ssh tunnel socket closed: {err}");
                    }
                }
                Err(err) => log::warn!("ssh tunnel could not reach {host}:{port}: {err}"),
            }
        });
    }
}

fn ssh_error(err: russh::Error) -> AppError {
    AppError::driver(format!("SSH: {err}"))
}

/// Rounds of keyboard-interactive questions answered before giving up.
const MAX_INTERACTIVE_ROUNDS: usize = 5;

// WHAT:  Password login with a keyboard-interactive fallback.
// WHY:   Servers using PAM often disable the `password` method and offer only
//        `keyboard-interactive`; OpenSSH's client falls back silently, so a
//        password that works in a terminal must work here too.
// HOW:   `password` first. Only when the server lists keyboard-interactive among
//        the methods it still accepts is that tried, with the same password.
async fn authenticate_with_password(session: &mut Handle<Client>, user: &str, password: &str) -> AppResult<bool> {
    let first = session.authenticate_password(user, password).await.map_err(ssh_error)?;
    let interactive_offered = match &first {
        AuthResult::Success => return Ok(true),
        AuthResult::Failure { remaining_methods, .. } => remaining_methods.contains(&MethodKind::KeyboardInteractive),
    };
    if !interactive_offered {
        return Ok(false);
    }
    let mut response = session.authenticate_keyboard_interactive_start(user, None::<String>).await.map_err(ssh_error)?;
    for _ in 0..MAX_INTERACTIVE_ROUNDS {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let mut answers = Vec::with_capacity(prompts.len());
                for prompt in &prompts {
                    match answer_for_prompt(&prompt.prompt, password) {
                        Some(answer) => answers.push(answer),
                        None => {
                            return Err(AppError::driver(format!(
                                "The SSH server asked \"{}\", which DB Free cannot answer. Use a key file or a host that accepts a password.",
                                prompt.prompt.trim()
                            )))
                        }
                    }
                }
                response = session.authenticate_keyboard_interactive_respond(answers).await.map_err(ssh_error)?;
            }
        }
    }
    Ok(false)
}

// WHAT:  What to type for one keyboard-interactive prompt: the password, if it asks for one.
// WHY:   Anything else (a one-time code, a push approval) cannot be answered with
//        the stored secret; guessing would burn the user's login attempts.
fn answer_for_prompt(prompt: &str, password: &str) -> Option<String> {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("password") && !lower.contains("new password") && !lower.contains("verification") {
        Some(password.to_string())
    } else {
        None
    }
}

// WHAT:  The adapter session plus the tunnel it runs through.
// HOW:   Delegates every `Integration` method (defaults included, so an
//        adapter's own overrides still apply). Add a delegation here when the
//        trait gains a method.
pub struct Tunnelled {
    inner: Arc<dyn Integration>,
    tunnel: Tunnel,
}

impl Tunnelled {
    pub fn new(inner: Arc<dyn Integration>, tunnel: Tunnel) -> Tunnelled {
        Tunnelled { inner, tunnel }
    }
}

#[async_trait]
impl Integration for Tunnelled {
    fn engine(&self) -> Engine {
        self.inner.engine()
    }
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    async fn ping(&self) -> AppResult<()> {
        if self.tunnel.is_closed() {
            return Err(AppError::not_connected("The SSH tunnel closed."));
        }
        self.inner.ping().await
    }
    async fn server_version(&self) -> AppResult<Option<String>> {
        self.inner.server_version().await
    }
    fn current_database(&self) -> Option<String> {
        self.inner.current_database()
    }
    async fn databases(&self) -> AppResult<Vec<String>> {
        self.inner.databases().await
    }
    async fn catalog(&self) -> AppResult<SchemaCatalog> {
        self.inner.catalog().await
    }
    async fn columns(&self, table: &TableRef) -> AppResult<Vec<ColumnInfo>> {
        self.inner.columns(table).await
    }
    async fn row_estimate(&self, table: &TableRef) -> AppResult<Option<i64>> {
        self.inner.row_estimate(table).await
    }
    async fn count(&self, table: &TableRef, filters: &[FilterRule]) -> AppResult<i64> {
        self.inner.count(table, filters).await
    }
    async fn fetch_page(&self, table: &TableRef, query: &PageQuery) -> AppResult<ResultSet> {
        self.inner.fetch_page(table, query).await
    }
    async fn execute(&self, sql: &str, max_rows: usize) -> AppResult<Vec<StatementResult>> {
        self.inner.execute(sql, max_rows).await
    }
    async fn close(&self) {
        self.inner.close().await;
        self.tunnel.close().await;
    }
    async fn foreign_keys(&self) -> AppResult<Vec<ForeignKey>> {
        self.inner.foreign_keys().await
    }
    async fn ddl(&self, table: &TableRef) -> AppResult<Option<String>> {
        self.inner.ddl(table).await
    }
    fn create_template(&self, table: &TableRef, columns: &[ColumnInfo]) -> Option<String> {
        self.inner.create_template(table, columns)
    }
    fn use_namespace(&self, namespace: &str) -> Option<(String, usize)> {
        self.inner.use_namespace(namespace)
    }
    async fn objects(&self, kind: ObjectKind, parent: Option<&str>) -> AppResult<Vec<ObjectSummary>> {
        self.inner.objects(kind, parent).await
    }
    async fn object_detail(&self, reference: &ObjectRef) -> AppResult<ObjectDetail> {
        self.inner.object_detail(reference).await
    }
    fn object_table(&self, reference: &ObjectRef) -> TableRef {
        self.inner.object_table(reference)
    }
    async fn server_stats(&self) -> AppResult<ServerStats> {
        self.inner.server_stats().await
    }
    async fn vector_search(&self, req: &VectorSearchRequest) -> AppResult<ResultSet> {
        self.inner.vector_search(req).await
    }
    async fn search(&self, req: &SearchRequest) -> AppResult<SearchResult> {
        self.inner.search(req).await
    }
    async fn query_range(&self, req: &RangeQueryRequest) -> AppResult<RangeResult> {
        self.inner.query_range(req).await
    }
    async fn history(&self, reference: &ObjectRef) -> AppResult<ResultSet> {
        self.inner.history(reference).await
    }
    async fn download_object(&self, bucket: &str, key: &str) -> AppResult<Vec<u8>> {
        self.inner.download_object(bucket, key).await
    }
    fn ssh_host_key(&self) -> Option<String> {
        self.tunnel.host_key.clone()
    }
    async fn column_traits(&self, table: &TableRef) -> AppResult<Vec<crate::model::ColumnTraits>> {
        self.inner.column_traits(table).await
    }
    fn ddl_profile(&self) -> crate::model::DdlProfile {
        self.inner.ddl_profile()
    }
    async fn describe_table(&self, table: &TableRef) -> AppResult<crate::model::TableSpec> {
        self.inner.describe_table(table).await
    }
    fn create_table_sql(&self, spec: &crate::model::TableSpec) -> AppResult<Vec<String>> {
        self.inner.create_table_sql(spec)
    }
    fn alter_table_sql(&self, spec: &crate::model::AlterTableSpec) -> AppResult<Vec<String>> {
        self.inner.alter_table_sql(spec)
    }
    fn duplicate_table_sql(&self, spec: &crate::model::DuplicateSpec, source: &crate::model::TableSpec) -> AppResult<Vec<String>> {
        self.inner.duplicate_table_sql(spec, source)
    }
    fn remove_table_sql(&self, spec: &crate::model::RemoveSpec) -> AppResult<Vec<String>> {
        self.inner.remove_table_sql(spec)
    }
    fn namespace_sql(&self, spec: &crate::model::NamespaceSpec) -> AppResult<Vec<String>> {
        self.inner.namespace_sql(spec)
    }
    async fn scan_keys(&self, scan: &crate::model::RedisKeyScan) -> AppResult<crate::model::RedisKeyPage> {
        self.inner.scan_keys(scan).await
    }
    async fn key_info(&self, key: &str) -> AppResult<crate::model::RedisKeyInfo> {
        self.inner.key_info(key).await
    }
    async fn close_session(&self, key: &str) {
        self.inner.close_session(key).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionInput, Environment, SshTunnel, SslMode};

    fn summary(engine: Engine, host: &str, port: Option<u16>) -> ConnectionSummary {
        let input = ConnectionInput {
            name: "t".into(),
            engine,
            environment: Environment::Local,
            read_only: false,
            host: Some(host.into()),
            port,
            database: None,
            username: None,
            password: None,
            file_path: None,
            ssl_mode: SslMode::Disable,
            ssh: SshTunnel { enabled: true, ..SshTunnel::default() },
            ssh_secret: None,
            folder: None,
            color: None,
            favorite: false,
            auth: Default::default(),
            auth_secrets: Default::default(),
            extra: Default::default(),
        };
        ConnectionSummary::draft(&input, false)
    }

    #[test]
    fn host_key_is_trusted_on_first_use_then_pinned() {
        assert_eq!(verify_host_key(None, "SHA256:abc"), HostKeyVerdict::FirstUse);
        assert_eq!(verify_host_key(Some("  "), "SHA256:abc"), HostKeyVerdict::FirstUse, "blank pin = none");
        assert_eq!(verify_host_key(Some("SHA256:abc"), "SHA256:abc"), HostKeyVerdict::Match);
        assert_eq!(
            verify_host_key(Some("SHA256:abc"), "SHA256:evil"),
            HostKeyVerdict::Mismatch { expected: "SHA256:abc".into(), actual: "SHA256:evil".into() }
        );
    }

    #[test]
    fn target_uses_host_port_then_field_then_engine_default() {
        let t = |host: &str, port: Option<u16>| target_of(&summary(Engine::Postgres, host, port));
        assert_eq!(t("10.0.0.5", Some(6432)).ok(), Some(("10.0.0.5".into(), 6432)));
        assert_eq!(t("db.internal:5433", Some(6432)).ok(), Some(("db.internal".into(), 5433)));
        assert_eq!(t("db.internal", None).ok(), Some(("db.internal".into(), 5432)));
        assert_eq!(t("[fd00::5]:5433", None).ok(), Some(("fd00::5".into(), 5433)));
        assert_eq!(t("fd00::5", Some(1)).ok(), Some(("fd00::5".into(), 1)));
        assert!(t("a,b", None).is_err(), "one host only");
        assert!(t("postgres://x", None).is_err(), "no URLs");
        assert!(t("db:notaport", None).is_err());
    }

    #[test]
    fn rewrite_points_the_adapter_at_the_local_end() {
        let conn = ResolvedConnection { summary: summary(Engine::Mysql, "10.1.2.3", Some(3306)), secret: Some("pw".into()) };
        let local = rewrite_to_local(&conn, 40123);
        assert_eq!(local.summary.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(local.summary.port, Some(40123));
        assert_eq!(local.secret.as_deref(), Some("pw"), "database credentials pass through untouched");
        assert_eq!(local.summary.database, conn.summary.database);
    }

    // WHAT:  C17 — the real host survives the rewrite so a driver that can verify it does.
    #[test]
    fn the_tunnel_remembers_the_real_host_for_tls() {
        let mut mssql = summary(Engine::Mssql, "sql.corp.example:1433", None);
        mssql.ssl_mode = SslMode::VerifyFull;
        let local = rewrite_to_local(&ResolvedConnection { summary: mssql, secret: None }, 40000);
        assert_eq!(local.summary.extra.tunnel_origin_host.as_deref(), Some("sql.corp.example"));
        assert_eq!(local.summary.ssl_mode, SslMode::VerifyFull, "SQL Server verifies the real name, so nothing is relaxed");
    }

    #[test]
    fn sqlx_families_cannot_verify_the_name_so_verify_full_becomes_verify_ca() {
        for engine in [Engine::Postgres, Engine::Mysql, Engine::Supabase] {
            let mut s = summary(engine, "db.internal", Some(5432));
            s.ssl_mode = SslMode::VerifyFull;
            let local = rewrite_to_local(&ResolvedConnection { summary: s, secret: None }, 41000);
            assert_eq!(local.summary.ssl_mode, SslMode::VerifyCa, "{engine:?}");
        }
        let mut other = summary(Engine::Postgres, "db.internal", None);
        other.ssl_mode = SslMode::Require;
        let local = rewrite_to_local(&ResolvedConnection { summary: other, secret: None }, 41000);
        assert_eq!(local.summary.ssl_mode, SslMode::Require, "only verify_full is relaxed");
    }

    // WHAT:  C16 — a keyboard-interactive password prompt is answered; anything else is not guessed.
    #[test]
    fn only_password_prompts_are_answered() {
        assert_eq!(answer_for_prompt("Password: ", "pw").as_deref(), Some("pw"));
        assert_eq!(answer_for_prompt("ops@bastion's password:", "pw").as_deref(), Some("pw"));
        assert_eq!(answer_for_prompt("Verification code: ", "pw"), None);
        assert_eq!(answer_for_prompt("Duo passcode or option (1-3): ", "pw"), None);
        assert_eq!(answer_for_prompt("New password: ", "pw"), None, "never answer a change-password prompt with the old one");
    }

    #[test]
    fn home_is_expanded_in_key_paths() {
        let has_home = std::env::var_os("HOME").is_some() || std::env::var_os("USERPROFILE").is_some();
        if has_home {
            let expanded = expand_home("~/.ssh/id_ed25519");
            assert!(!expanded.to_string_lossy().starts_with('~'));
            assert!(expanded.ends_with(std::path::Path::new(".ssh").join("id_ed25519")));
        }
        assert_eq!(expand_home("/abs/key"), std::path::PathBuf::from("/abs/key"));
    }
}
