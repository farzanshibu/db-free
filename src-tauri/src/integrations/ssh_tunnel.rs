// SOT: ssh-tunnel, ssh-port-forward, ssh-host-key-check, tunnelled-integration

use crate::error::{AppError, AppResult};
use crate::integrations::{Capabilities, Integration};
use crate::model::{
    ColumnInfo, ConnectionSummary, Engine, FilterRule, ForeignKey, ObjectDetail, ObjectKind, ObjectRef, ObjectSummary,
    PageQuery, RangeQueryRequest, RangeResult, ResolvedConnection, ResultSet, SchemaCatalog, SearchRequest,
    SearchResult, ServerStats, SshAuth, StatementResult, TableRef, VectorSearchRequest,
};
use async_trait::async_trait;
use russh::client::{self, Handle};
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
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
//        Limits: the adapter now talks to 127.0.0.1, so TLS `verify_full`
//        (hostname check) fails unless the certificate names 127.0.0.1; and
//        clustered drivers that discover other nodes (Mongo replica sets,
//        Cassandra, Kafka brokers) are only reachable when they advertise the
//        tunnelled address. One host is forwarded, never a list.
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
pub fn rewrite_to_local(conn: &ResolvedConnection, local_port: u16) -> ResolvedConnection {
    let mut local = conn.clone();
    local.summary.host = Some("127.0.0.1".to_string());
    local.summary.port = Some(local_port);
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
            SshAuth::Password => session
                .authenticate_password(user, secret.unwrap_or_default())
                .await
                .map_err(ssh_error)?,
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
            }
        };
        if !authenticated.success() {
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

// WHAT:  `~/.ssh/id_ed25519` → the user's home, as a shell would expand it.
fn expand_home(path: &str) -> std::path::PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match (path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")), home) {
        (Some(rest), Some(home)) => std::path::Path::new(&home).join(rest),
        _ => std::path::PathBuf::from(path),
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
