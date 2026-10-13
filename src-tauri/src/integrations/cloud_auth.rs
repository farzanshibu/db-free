// SOT: cloud-auth, cloud-token-resolver, rds-iam-token, azure-ad-token, gcp-iam-token, aws-credential-chain, token-cache, token-refresh, mongo-atlas-auth

use crate::error::{AppError, AppResult};
use crate::integrations::aws_sigv4::{presign_get, uri_encode, AwsCredentials, PresignRequest, EMPTY_PAYLOAD_HASH};
use crate::integrations::gcp_auth;
use crate::integrations::http::{Auth, HttpClient};
use crate::model::auth::{AuthSecrets, AwsAccess, AwsCredentialSource, AzureAdMode, ConnectionAuth, GcpAuthMode};
use crate::model::{ConnectionSummary, Family, ResolvedConnection, SslMode};
use base64::Engine as _;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

// ============================================================================
// CLOUD SIGN-IN
//
// WHAT:  Turns a connection's sign-in method (`model::auth::ConnectionAuth`) and
//        its secrets into what a driver can use: a database password that is a
//        short-lived token (RDS IAM, Azure AD, Cloud SQL IAM), an Entra token for
//        SQL Server, MongoDB Atlas mechanism settings, or AWS credentials for
//        DynamoDB.
// WHY:   The identity lives outside the app (a profile, `az login`, a key file)
//        and the token expires in minutes, so it has to be minted at connect
//        time and again whenever a pooled connection reconnects. No cloud SDK:
//        every provider here is a handful of plain HTTPS calls, and the SDKs
//        would add hundreds of crates.
// HOW:   * `CloudSession` is built by services::connection from the saved
//          connection plus its decrypted secrets, *before* any SSH tunnel
//          rewrites the host (an RDS token is signed for the real endpoint),
//          and rides to the adapter in a task-local (`with_session`) like the
//          run id does: the 45 `connect(conn)` signatures stay unchanged.
//        * Adapters ask for what they need: `db_plan` (PostgreSQL / MySQL /
//          SQL Server), `mongo_auth`, or `AwsSession` (DynamoDB).
//        * Every token is cached with its expiry and minted again 60 s before
//          it runs out (`is_fresh`). Pooled drivers reconnect on their own with
//          the options they were built with, so `keep_fresh` swaps new
//          credentials in before the old ones lapse.
//        * Everything that decides something (request builders, parsers, the
//          credential-chain planner) is a pure function over a clock and an
//          environment lookup, tested below with fixed values.
// WHERE: src-tauri/src/model/auth.rs (the settings), src-tauri/src/services/connection.rs
//        (builds the session), src-tauri/src/integrations/{postgres,mysql,mssql,mongodb,dynamodb}.rs
// ============================================================================

/// A token is minted again this long before it expires.
const REFRESH_SKEW_SECS: i64 = 60;
/// RDS authentication tokens are valid for 15 minutes; treat them as 14 so a
/// token is never handed out in its last minute.
const RDS_TOKEN_TTL_SECS: i64 = 14 * 60;
const RDS_PRESIGN_EXPIRES: u32 = 900;
const STS_PRESIGN_EXPIRES: u32 = 300;
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);

pub const AZURE_OSSRDBMS_SCOPE: &str = "https://ossrdbms-aad.database.windows.net/.default";
pub const AZURE_SQL_SCOPE: &str = "https://database.windows.net/.default";
/// Cloud SQL accepts either; a service-account JWT has to name them explicitly.
const GCP_SQL_SCOPES: &str = "https://www.googleapis.com/auth/sqlservice.admin https://www.googleapis.com/auth/sqlservice.login";
const GCP_CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// Amazon's RDS / Aurora certificate authorities (every region), fetched from
/// https://truststore.pki.rds.amazonaws.com/global/global-bundle.pem and kept in
/// the repo: nothing is downloaded at build or run time.
const AWS_RDS_CA_BUNDLE: &str = include_str!("../../resources/aws-rds-global-bundle.pem");

// ---------------------------------------------------------------------------
// Ambient session
// ---------------------------------------------------------------------------

tokio::task_local! {
    static SESSION: Option<Arc<CloudSession>>;
}

// WHAT:  Runs `fut` (a connect) with `session` visible to `current()` inside it.
// WHY:   Same reason as `integrations::with_run_id`: the adapters' `connect`
//        signature is shared by every engine, and only four of them care.
pub async fn with_session<F: Future>(session: Option<Arc<CloudSession>>, fut: F) -> F::Output {
    SESSION.scope(session, fut).await
}

/// The session of the connect in progress, if the connection signs in with one.
pub fn current() -> Option<Arc<CloudSession>> {
    SESSION.try_with(Clone::clone).ok().flatten()
}

/// The AWS session token typed for the connection in progress (DynamoDB keys).
pub fn session_token() -> Option<String> {
    current().and_then(|s| s.secrets.aws_session_token.clone()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

// ---------------------------------------------------------------------------
// Results handed to the adapters
// ---------------------------------------------------------------------------

/// A minted token and the moment it stops working.
#[derive(Clone)]
pub struct Minted {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for Minted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Minted").field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

/// What a SQL driver signs in with.
#[derive(Clone)]
pub struct DbCredentials {
    /// The database user, when the method decides it (otherwise the connection's own).
    pub user: Option<String>,
    /// The token.
    pub password: String,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for DbCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbCredentials").field("user", &self.user).field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

// WHAT:  Everything a PostgreSQL / MySQL / SQL Server connect needs to decide, in
//        one place: who to log in as, with what, how encrypted, and whether the
//        credentials must be refreshed while the session lives.
#[derive(Clone)]
pub struct ConnectPlan {
    pub user: Option<String>,
    pub password: Option<String>,
    pub ssl_mode: SslMode,
    /// PEM text to trust instead of the system roots (RDS).
    pub ca_pem: Option<String>,
    /// MySQL must send the token as a clear-text password (over TLS).
    pub cleartext_password: bool,
    /// Set when the credentials expire: pass to `keep_fresh`.
    pub refresh: Option<Arc<CloudSession>>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for ConnectPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectPlan").field("user", &self.user).field("ssl_mode", &self.ssl_mode).finish_non_exhaustive()
    }
}

/// MongoDB Atlas mechanism settings, free of any driver type.
#[derive(Clone, PartialEq, Eq)]
pub enum MongoAuth {
    /// `MONGODB-AWS` with explicit credentials.
    Aws { access_key_id: String, secret_access_key: String, session_token: Option<String> },
    /// `MONGODB-OIDC` with a built-in workload-identity environment.
    Oidc { environment: &'static str, token_resource: Option<String> },
}

impl std::fmt::Debug for MongoAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MongoAuth::Aws { access_key_id, .. } => f.debug_struct("MongoAuth::Aws").field("access_key_id", access_key_id).finish_non_exhaustive(),
            MongoAuth::Oidc { environment, token_resource } => {
                f.debug_struct("MongoAuth::Oidc").field("environment", environment).field("token_resource", token_resource).finish()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Token cache
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Cached {
    Token(String),
    Aws(ResolvedAws),
}

struct Entry {
    value: Cached,
    expires_at: DateTime<Utc>,
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

// WHAT:  A token is usable while more than 60 s of its life remain.
pub fn is_fresh(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now + ChronoDuration::seconds(REFRESH_SKEW_SECS) < expires_at
}

fn lookup(key: &str, now: DateTime<Utc>) -> Option<Entry> {
    let mut guard = cache().lock().ok()?;
    guard.retain(|_, entry| entry.expires_at > now);
    guard.get(key).filter(|e| is_fresh(e.expires_at, now)).map(|e| Entry { value: e.value.clone(), expires_at: e.expires_at })
}

fn remember(key: &str, value: Cached, expires_at: DateTime<Utc>) {
    if let Ok(mut guard) = cache().lock() {
        guard.insert(key.to_string(), Entry { value, expires_at });
    }
}

// WHAT:  The token for `key`, from the cache while it is fresh, else from `mint`.
// HOW:   `mint` is only polled on a miss; no lock is held across an await.
async fn cached_token(key: &str, mint: impl Future<Output = AppResult<Minted>>) -> AppResult<Minted> {
    if let Some(Entry { value: Cached::Token(token), expires_at }) = lookup(key, Utc::now()) {
        return Ok(Minted { token, expires_at });
    }
    let minted = mint.await?;
    remember(key, Cached::Token(minted.token.clone()), minted.expires_at);
    Ok(minted)
}

// ---------------------------------------------------------------------------
// The session
// ---------------------------------------------------------------------------

// WHAT:  One connection's sign-in: the method, its decrypted secrets and the
//        endpoint the credentials are for.
pub struct CloudSession {
    auth: ConnectionAuth,
    secrets: AuthSecrets,
    family: Family,
    host: String,
    port: u16,
    db_user: Option<String>,
    key: String,
}

impl std::fmt::Debug for CloudSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudSession").field("method", &self.auth.method()).field("host", &self.host).field("port", &self.port).finish_non_exhaustive()
    }
}

fn trimmed(value: &Option<String>) -> Option<String> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

impl CloudSession {
    // WHAT:  The session for a saved connection, or None for a plain password
    //        connection that carries no sign-in secret (the common case).
    // HOW:   `summary` is the connection as saved, before any SSH rewrite.
    pub fn for_connection(summary: &ConnectionSummary, secrets: AuthSecrets) -> Option<Arc<CloudSession>> {
        if summary.auth.is_password() && secrets.is_empty() {
            return None;
        }
        let family = summary.engine.family();
        let host = summary.host.as_deref().map(str::trim).unwrap_or_default().to_string();
        let port = summary.port.or(summary.engine.default_port()).unwrap_or(0);
        let db_user = trimmed(&summary.username);
        let key = format!(
            "{}|{host}:{port}|{}|{}|{}",
            summary.engine.as_str(),
            db_user.as_deref().unwrap_or_default(),
            summary.auth.to_json().unwrap_or_default(),
            secrets.to_json().unwrap_or_default()
        );
        Some(Arc::new(CloudSession { auth: summary.auth.clone(), secrets, family, host, port, db_user, key }))
    }

    pub fn auth(&self) -> &ConnectionAuth {
        &self.auth
    }

    pub fn secrets(&self) -> &AuthSecrets {
        &self.secrets
    }

    /// Whether `db_credentials` can produce a different answer later. A pasted
    /// access token cannot: when it lapses the user pastes another.
    pub fn refreshable(&self) -> bool {
        !matches!(self.auth, ConnectionAuth::AzureAd { mode: AzureAdMode::AccessToken, .. })
    }

    fn require_user(&self) -> AppResult<String> {
        self.db_user.clone().ok_or_else(|| AppError::invalid_input("Enter the database user: the token is issued for it."))
    }

    // WHAT:  The database user and token for the signed-in identity, minted or
    //        taken from the cache.
    pub async fn db_credentials(&self) -> AppResult<DbCredentials> {
        match &self.auth {
            ConnectionAuth::AwsRdsIam { region, credentials, .. } => {
                let user = self.require_user()?;
                let region = trimmed(region)
                    .or_else(|| region_from_rds_host(&self.host))
                    .ok_or_else(|| AppError::invalid_input("Set the AWS region: it could not be read from the host name."))?;
                let key = format!("rds|{}", self.key);
                let minted = cached_token(&key, async {
                    let aws = resolve_aws_cached(credentials, &self.secrets, &region).await?;
                    let now = Utc::now();
                    let token = rds_auth_token(&aws.credentials(&region), &self.host, self.port, &user, now)?;
                    Ok(Minted { token, expires_at: now + ChronoDuration::seconds(RDS_TOKEN_TTL_SECS) })
                })
                .await?;
                Ok(DbCredentials { user: Some(user), password: minted.token, expires_at: minted.expires_at })
            }
            ConnectionAuth::AzureAd { mode, tenant_id, client_id, username, scope } => {
                let scope = azure_scope(scope.as_deref(), self.family);
                let key = format!("azure|{scope}|{}", self.key);
                let account = trimmed(username).or_else(|| self.db_user.clone());
                let minted = cached_token(&key, azure_token(*mode, trimmed(tenant_id), trimmed(client_id), account, &scope, &self.secrets)).await?;
                let user = if self.family == Family::Mssql { None } else { Some(self.require_user()?) };
                Ok(DbCredentials { user, password: minted.token, expires_at: minted.expires_at })
            }
            ConnectionAuth::GcpCloudSqlIam { mode, impersonate_service_account } => {
                let user = self.require_user()?;
                let key = format!("gcp|{}", self.key);
                let minted = cached_token(&key, gcp_token(*mode, trimmed(impersonate_service_account), &self.secrets)).await?;
                Ok(DbCredentials { user: Some(user), password: minted.token, expires_at: minted.expires_at })
            }
            ConnectionAuth::Password
            | ConnectionAuth::MongoAtlasAws { .. }
            | ConnectionAuth::MongoAtlasOidc { .. }
            | ConnectionAuth::AwsCredentials { .. } => Err(AppError::invalid_input("This sign-in method does not issue a database password.")),
        }
    }

    // WHAT:  The MongoDB mechanism for an Atlas connection.
    // HOW:   AWS identities are resolved here (profile, chain, assume-role) and
    //        handed over as explicit keys, so the driver never walks its own chain.
    pub async fn mongo_auth(&self) -> AppResult<MongoAuth> {
        match &self.auth {
            ConnectionAuth::MongoAtlasAws { credentials } => {
                let region = aws_region_hint();
                let aws = resolve_aws_cached(credentials, &self.secrets, &region).await?;
                Ok(MongoAuth::Aws { access_key_id: aws.access_key, secret_access_key: aws.secret_key, session_token: aws.session_token })
            }
            ConnectionAuth::MongoAtlasOidc { provider, token_resource } => {
                Ok(MongoAuth::Oidc { environment: provider.environment(), token_resource: trimmed(token_resource) })
            }
            _ => Err(AppError::invalid_input("This sign-in method is not a MongoDB Atlas mechanism.")),
        }
    }
}

// WHAT:  The adapter-facing plan for a PostgreSQL / MySQL / SQL Server connect.
// HOW:   A password connection gets exactly what it always had. A cloud identity
//        gets the minted token as the password, the database user, TLS of at
//        least `require` (the token must not travel in the clear), the CA to
//        verify against, and the session to refresh from.
pub async fn db_plan(conn: &ResolvedConnection) -> AppResult<ConnectPlan> {
    let s = &conn.summary;
    let mut plan = ConnectPlan {
        user: s.username.clone(),
        password: conn.secret.clone(),
        ssl_mode: s.ssl_mode,
        ca_pem: None,
        cleartext_password: false,
        refresh: None,
        expires_at: None,
    };
    let ca_bundle = match &s.auth {
        ConnectionAuth::Password => return Ok(plan),
        ConnectionAuth::AwsRdsIam { ca_bundle, .. } => Some(
            trimmed(ca_bundle).unwrap_or_else(|| AWS_RDS_CA_BUNDLE.to_string()),
        ),
        ConnectionAuth::AzureAd { .. } | ConnectionAuth::GcpCloudSqlIam { .. } => None,
        ConnectionAuth::MongoAtlasAws { .. } | ConnectionAuth::MongoAtlasOidc { .. } | ConnectionAuth::AwsCredentials { .. } => {
            return Err(AppError::invalid_input("This sign-in method is not available for this engine."));
        }
    };
    let session = current().ok_or_else(|| {
        AppError::invalid_input("This connection signs in with a cloud identity: open it from its saved connection so its secrets are available.")
    })?;
    let creds = session.db_credentials().await?;
    plan.user = creds.user.or(plan.user);
    plan.password = Some(creds.password);
    plan.expires_at = Some(creds.expires_at);
    plan.cleartext_password = true;
    plan.ssl_mode = cloud_ssl_mode(s.ssl_mode, ca_bundle.is_some());
    plan.ca_pem = ca_bundle;
    if session.refreshable() {
        plan.refresh = Some(session);
    }
    Ok(plan)
}

// WHAT:  TLS floor for a token connection: never below `require`, and when a CA
//        is on hand (RDS) a default or lax mode becomes `verify_ca` so the chain
//        is actually checked. A stricter choice by the user stands.
pub fn cloud_ssl_mode(chosen: SslMode, have_ca: bool) -> SslMode {
    match chosen {
        SslMode::Disable | SslMode::Prefer => {
            if have_ca {
                SslMode::VerifyCa
            } else {
                SslMode::Require
            }
        }
        other => other,
    }
}

/// The Microsoft Entra access token for SQL Server, when the connection uses one.
pub async fn aad_token(conn: &ResolvedConnection) -> AppResult<Option<String>> {
    if !matches!(conn.summary.auth, ConnectionAuth::AzureAd { .. }) {
        return Ok(None);
    }
    Ok(Some(db_plan(conn).await?.password.unwrap_or_default()))
}

/// The MongoDB Atlas mechanism for the connection, if it uses one.
pub async fn mongo_auth(conn: &ResolvedConnection) -> AppResult<Option<MongoAuth>> {
    if !matches!(conn.summary.auth, ConnectionAuth::MongoAtlasAws { .. } | ConnectionAuth::MongoAtlasOidc { .. }) {
        return Ok(None);
    }
    let session = current().ok_or_else(|| AppError::invalid_input("This connection signs in with a cloud identity: open it from its saved connection."))?;
    session.mongo_auth().await.map(Some)
}

// ---------------------------------------------------------------------------
// Keeping pooled credentials fresh
// ---------------------------------------------------------------------------

// WHAT:  How long to wait before minting again: until 60 s before expiry, but
//        never less than a second (a clock set wrong must not spin) nor more than a day.
pub fn refresh_delay(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    let wait = (expires_at - now).num_seconds() - REFRESH_SKEW_SECS;
    Duration::from_secs(u64::try_from(wait.clamp(1, 86_400)).unwrap_or(1))
}

// WHAT:  Background task that mints fresh credentials ahead of expiry and hands
//        them to `apply` (an adapter swaps them into its pool's connect options).
// WHY:   A pool opens new connections lazily, with the options it was built
//        with; a 15-minute token would make every reconnect after the first
//        quarter hour fail. Ends when `closed` resolves (the pool was closed).
// HOW:   Errors are logged and retried every 30 s: a laptop that was offline
//        recovers on its own when the network is back.
pub fn keep_fresh<C, F>(session: Arc<CloudSession>, expires_at: Option<DateTime<Utc>>, closed: C, apply: F)
where
    C: Future<Output = ()> + Send + 'static,
    F: Fn(&DbCredentials) + Send + 'static,
{
    let Some(first) = expires_at else { return };
    tokio::spawn(async move {
        tokio::pin!(closed);
        let mut wait = refresh_delay(first, Utc::now());
        loop {
            tokio::select! {
                () = &mut closed => return,
                () = tokio::time::sleep(wait) => {}
            }
            wait = match session.db_credentials().await {
                Ok(creds) => {
                    apply(&creds);
                    refresh_delay(creds.expires_at, Utc::now())
                }
                Err(err) => {
                    log::warn!("could not refresh the database token: {err}");
                    Duration::from_secs(30)
                }
            };
        }
    });
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn home_dir() -> Option<std::path::PathBuf> {
    env_var("HOME").or_else(|| env_var("USERPROFILE")).map(std::path::PathBuf::from)
}

// WHAT:  `application/x-www-form-urlencoded` body from key/value pairs.
pub fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", uri_encode(k, true), uri_encode(v, true))).collect::<Vec<_>>().join("&")
}

fn http() -> AppResult<HttpClient> {
    HttpClient::new("https://localhost", Auth::None, false)
}

fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    let normal = raw.strip_suffix("UTC").map(|r| format!("{r}Z")).unwrap_or_else(|| raw.to_string());
    DateTime::parse_from_rfc3339(&normal).ok().map(|d| d.with_timezone(&Utc))
}

fn from_epoch_secs(secs: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(secs, 0)
}

// WHAT:  The `exp` claim of a JWT, without verifying it (only used to tell the
//        user a pasted token has lapsed).
pub fn jwt_expiry(token: &str) -> Option<DateTime<Utc>> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    from_epoch_secs(claims.get("exp")?.as_i64()?)
}

// WHAT:  A JSON field that providers send as either a number or a string.
fn json_i64(value: &serde_json::Value, key: &str) -> Option<i64> {
    let field = value.get(key)?;
    field.as_i64().or_else(|| field.as_str().and_then(|s| s.trim().parse().ok()))
}

fn json_str(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(serde_json::Value::as_str).map(str::to_string).filter(|s| !s.is_empty())
}

// WHAT:  The first line of a provider's explanation of a refusal.
fn oauth_failure(status: u16, body: &str) -> AppError {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    let message = json_str(&parsed, "error_description")
        .or_else(|| parsed.get("error").and_then(|e| e.get("message")).and_then(serde_json::Value::as_str).map(str::to_string))
        .or_else(|| json_str(&parsed, "error"))
        .or_else(|| json_str(&parsed, "message"))
        .unwrap_or_else(|| body.chars().take(300).collect());
    let first_line = message.lines().next().unwrap_or_default().trim().to_string();
    AppError::not_connected(format!("Sign-in was refused ({status}): {first_line}"))
}

fn ensure_ok(status: u16, body: &str) -> AppResult<()> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(oauth_failure(status, body))
    }
}

// WHAT:  Spawns `program args…` and returns stdout, killing it after 30 s.
async fn run_process(program: &str, args: &[String]) -> AppResult<String> {
    let mut command = tokio::process::Command::new(program);
    command.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(PROCESS_TIMEOUT, command.output())
        .await
        .map_err(|_| AppError::timeout(format!("`{program}` did not answer in {} seconds.", PROCESS_TIMEOUT.as_secs())))?
        .map_err(|e| AppError::not_connected(format!("Could not run `{program}`: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it exited with an error").trim();
        return Err(AppError::not_connected(format!("`{program}` failed: {reason}")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ---------------------------------------------------------------------------
// AWS: credentials
// ---------------------------------------------------------------------------

// WHAT:  Resolved AWS credentials and, for temporary ones, when they lapse.
#[derive(Clone)]
pub struct ResolvedAws {
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for ResolvedAws {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedAws").field("access_key", &self.access_key).field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

impl ResolvedAws {
    /// The signing credentials for requests to `region`.
    pub fn credentials(&self, region: &str) -> AwsCredentials {
        AwsCredentials {
            region: region.to_string(),
            access_key: self.access_key.clone(),
            secret_key: self.secret_key.clone(),
            session_token: self.session_token.clone(),
        }
    }
}

/// The region for calls that do not name one (STS, MongoDB Atlas).
fn aws_region_hint() -> String {
    env_var("AWS_REGION").or_else(|| env_var("AWS_DEFAULT_REGION")).unwrap_or_else(|| "us-east-1".to_string())
}

// WHAT:  `mydb.abc123.eu-west-1.rds.amazonaws.com[.cn]` → `eu-west-1`.
pub fn region_from_rds_host(host: &str) -> Option<String> {
    let parts: Vec<&str> = host.trim().trim_end_matches('.').split('.').collect();
    let at = parts.iter().position(|p| *p == "rds")?;
    if at == 0 || parts.get(at + 1) != Some(&"amazonaws") {
        return None;
    }
    let region = parts[at - 1];
    let looks_like_region = region.contains('-') && region.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    looks_like_region.then(|| region.to_string())
}

// WHAT:  The RDS IAM authentication token: a presigned `rds-db:connect` GET with
//        the scheme removed (`host:port/?Action=connect&DBUser=…&X-Amz-…`).
pub fn rds_auth_token(creds: &AwsCredentials, host: &str, port: u16, db_user: &str, now: DateTime<Utc>) -> AppResult<String> {
    let endpoint = format!("{host}:{port}");
    let query = presign_get(
        creds,
        &PresignRequest {
            service: "rds-db",
            host: &endpoint,
            path: "/",
            params: &[("Action", "connect"), ("DBUser", db_user)],
            expires_secs: RDS_PRESIGN_EXPIRES,
            payload_hash: EMPTY_PAYLOAD_HASH,
            now,
        },
    )?;
    Ok(format!("{endpoint}/?{query}"))
}

// ----- shared config / credentials files -----

/// `section -> key -> value`, as the AWS tools read `~/.aws/{config,credentials}`.
pub type Profiles = BTreeMap<String, BTreeMap<String, String>>;

// WHAT:  Parses one INI file. Comments (`#`, `;`) and the indented sub-settings
//        of a nested key (`s3 =\n  max_concurrent_requests = 10`) are skipped.
pub fn parse_ini(text: &str) -> Profiles {
    let mut out: Profiles = BTreeMap::new();
    let mut section: Option<String> = None;
    let mut nested = false;
    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() || trimmed_line.starts_with('#') || trimmed_line.starts_with(';') {
            continue;
        }
        if let Some(name) = trimmed_line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = Some(name.trim().to_string());
            nested = false;
            out.entry(name.trim().to_string()).or_default();
            continue;
        }
        if line.starts_with(char::is_whitespace) && nested {
            continue;
        }
        let Some(current) = &section else { continue };
        if let Some((key, value)) = trimmed_line.split_once('=') {
            let value = value.trim();
            nested = value.is_empty();
            if let Some(entries) = out.get_mut(current) {
                entries.insert(key.trim().to_string(), value.to_string());
            }
        }
    }
    out
}

// WHAT:  The two files merged the way the SDKs do: the config file names its
//        profiles `[profile x]` (and `[default]`), the credentials file `[x]`,
//        and a key in the credentials file wins.
pub fn merge_profiles(credentials: &str, config: &str) -> Profiles {
    let mut merged: Profiles = BTreeMap::new();
    for (section, entries) in parse_ini(config) {
        let name = match section.strip_prefix("profile ") {
            Some(profile) => profile.trim().to_string(),
            None => section,
        };
        merged.entry(name).or_default().extend(entries);
    }
    for (section, entries) in parse_ini(credentials) {
        merged.entry(section).or_default().extend(entries);
    }
    merged
}

async fn load_profiles() -> Profiles {
    let home = home_dir();
    let path = |env: &str, default: &str| env_var(env).map(std::path::PathBuf::from).or_else(|| home.as_ref().map(|h| h.join(".aws").join(default)));
    let read = |path: Option<std::path::PathBuf>| async move {
        match path {
            Some(p) => tokio::fs::read_to_string(p).await.unwrap_or_default(),
            None => String::new(),
        }
    };
    let credentials = read(path("AWS_SHARED_CREDENTIALS_FILE", "credentials")).await;
    let config = read(path("AWS_CONFIG_FILE", "config")).await;
    merge_profiles(&credentials, &config)
}

// ----- planning: which source, which roles -----

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseSource {
    Static { access_key: String, secret_key: String, session_token: Option<String> },
    /// `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN`.
    Environment,
    /// `credential_process`: a command printing credentials as JSON.
    Process(String),
    /// IAM Identity Center (SSO) role, with the cached `aws sso login` token.
    Sso { start_url: String, region: String, account_id: String, role_name: String },
    WebIdentity { role_arn: String, token_file: String, session_name: Option<String> },
    /// ECS / EKS container credentials endpoint.
    Container { url: String, authorization: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleStep {
    pub role_arn: String,
    pub session_name: Option<String>,
    pub external_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsPlan {
    pub base: BaseSource,
    /// Roles to assume, in order, on top of the base credentials.
    pub roles: Vec<RoleStep>,
}

type EnvFn<'a> = &'a dyn Fn(&str) -> Option<String>;

fn container_source(env: EnvFn<'_>) -> Option<BaseSource> {
    if let Some(relative) = env("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI") {
        return Some(BaseSource::Container { url: format!("http://169.254.170.2{relative}"), authorization: env("AWS_CONTAINER_AUTHORIZATION_TOKEN") });
    }
    env("AWS_CONTAINER_CREDENTIALS_FULL_URI").map(|url| BaseSource::Container { url, authorization: env("AWS_CONTAINER_AUTHORIZATION_TOKEN") })
}

fn profile_base(name: &str, profile: &BTreeMap<String, String>, profiles: &Profiles, env: EnvFn<'_>) -> AppResult<BaseSource> {
    let get = |key: &str| profile.get(key).map(String::as_str).map(str::trim).filter(|v| !v.is_empty());
    if let Some(command) = get("credential_process") {
        return Ok(BaseSource::Process(command.to_string()));
    }
    if get("sso_start_url").is_some() || get("sso_session").is_some() {
        let session = get("sso_session").and_then(|s| profiles.get(&format!("sso-session {s}")));
        let from = |key: &str| get(key).map(str::to_string).or_else(|| session.and_then(|s| s.get(key)).map(|v| v.trim().to_string())).filter(|v| !v.is_empty());
        let (Some(start_url), Some(region), Some(account_id), Some(role_name)) =
            (from("sso_start_url"), from("sso_region"), get("sso_account_id"), get("sso_role_name"))
        else {
            return Err(AppError::invalid_input(format!(
                "AWS profile \"{name}\" is an SSO profile but is missing sso_start_url, sso_region, sso_account_id or sso_role_name."
            )));
        };
        return Ok(BaseSource::Sso { start_url, region, account_id: account_id.to_string(), role_name: role_name.to_string() });
    }
    if let (Some(token_file), Some(role_arn)) = (get("web_identity_token_file"), get("role_arn")) {
        return Ok(BaseSource::WebIdentity { role_arn: role_arn.to_string(), token_file: token_file.to_string(), session_name: get("role_session_name").map(str::to_string) });
    }
    if let Some(source) = get("credential_source") {
        return match source {
            "Environment" => Ok(BaseSource::Environment),
            "EcsContainer" => container_source(env)
                .ok_or_else(|| AppError::invalid_input(format!("AWS profile \"{name}\" uses EcsContainer credentials but no container credentials endpoint is set."))),
            other => Err(AppError::invalid_input(format!(
                "AWS profile \"{name}\" uses credential_source = {other}, which is not supported here. Use environment variables, an SSO profile or credential_process."
            ))),
        };
    }
    if let (Some(access_key), Some(secret_key)) = (get("aws_access_key_id"), get("aws_secret_access_key")) {
        return Ok(BaseSource::Static {
            access_key: access_key.to_string(),
            secret_key: secret_key.to_string(),
            session_token: get("aws_session_token").map(str::to_string),
        });
    }
    Err(AppError::invalid_input(format!("AWS profile \"{name}\" has no credentials (no keys, SSO settings, role or credential_process).")))
}

// WHAT:  Works out how to get credentials for profile `name`: follows
//        `source_profile` links down to the profile that holds real credentials
//        and lists the roles to assume on the way back up.
pub fn plan_profile(profiles: &Profiles, name: &str, env: EnvFn<'_>) -> AppResult<AwsPlan> {
    let mut chain: Vec<(&str, &BTreeMap<String, String>)> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut current = name;
    loop {
        if !seen.insert(current) {
            return Err(AppError::invalid_input(format!("AWS profile \"{name}\" refers back to itself through source_profile.")));
        }
        let profile = profiles.get(current).ok_or_else(|| AppError::not_found(format!("AWS profile \"{current}\" was not found in ~/.aws/credentials or ~/.aws/config.")))?;
        chain.push((current, profile));
        let next = profile.get("role_arn").filter(|r| !r.trim().is_empty()).and_then(|_| profile.get("source_profile")).map(|s| s.trim()).filter(|s| !s.is_empty());
        match next {
            Some(source) => current = source,
            None => break,
        }
    }
    let Some(&(leaf_name, leaf)) = chain.last() else {
        return Err(AppError::invalid_input("AWS profile chain is empty."));
    };
    if let Some((mfa_name, _)) = chain.iter().find(|(_, profile)| profile.contains_key("mfa_serial")) {
        return Err(AppError::invalid_input(format!(
            "AWS profile \"{mfa_name}\" requires MFA, which cannot be prompted for here. Sign in with `aws sso login` or use a credential_process."
        )));
    }
    let base = profile_base(leaf_name, leaf, profiles, env)?;
    let mut roles = Vec::new();
    for (index, (_, profile)) in chain.iter().enumerate().rev() {
        let Some(role_arn) = profile.get("role_arn").map(|r| r.trim()).filter(|r| !r.is_empty()) else { continue };
        let is_leaf = index == chain.len() - 1;
        // A web-identity leaf already assumed its role while producing the base.
        if is_leaf && matches!(base, BaseSource::WebIdentity { .. }) {
            continue;
        }
        roles.push(RoleStep {
            role_arn: role_arn.to_string(),
            session_name: profile.get("role_session_name").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()),
            external_id: profile.get("external_id").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()),
        });
    }
    Ok(AwsPlan { base, roles })
}

// WHAT:  The SDK default chain: environment keys, the shared profile
//        (`AWS_PROFILE`, else `default` when it exists), web identity, then
//        container credentials. EC2 instance metadata is deliberately left out:
//        a desktop tool would wait seconds on a link-local address that is not
//        there, and the instance-role case is covered by container/web identity.
pub fn plan_default_chain(profiles: &Profiles, env: EnvFn<'_>) -> AppResult<AwsPlan> {
    if env("AWS_ACCESS_KEY_ID").is_some() && env("AWS_SECRET_ACCESS_KEY").is_some() {
        return Ok(AwsPlan { base: BaseSource::Environment, roles: Vec::new() });
    }
    let named = env("AWS_PROFILE").or_else(|| env("AWS_DEFAULT_PROFILE"));
    match &named {
        Some(profile) => return plan_profile(profiles, profile, env),
        None if profiles.contains_key("default") => return plan_profile(profiles, "default", env),
        None => {}
    }
    if let (Some(token_file), Some(role_arn)) = (env("AWS_WEB_IDENTITY_TOKEN_FILE"), env("AWS_ROLE_ARN")) {
        return Ok(AwsPlan { base: BaseSource::WebIdentity { role_arn, token_file, session_name: env("AWS_ROLE_SESSION_NAME") }, roles: Vec::new() });
    }
    if let Some(container) = container_source(env) {
        return Ok(AwsPlan { base: container, roles: Vec::new() });
    }
    Err(AppError::not_connected(
        "No AWS credentials were found. Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, configure a profile (`aws configure`, `aws sso login`), or choose a profile or access keys explicitly.",
    ))
}

// ----- STS -----

fn sts_host(region: &str) -> String {
    if region.starts_with("cn-") {
        format!("sts.{region}.amazonaws.com.cn")
    } else {
        format!("sts.{region}.amazonaws.com")
    }
}

fn default_session_name(now: DateTime<Utc>) -> String {
    format!("db-free-{}", now.timestamp())
}

// WHAT:  The presigned GET that performs `sts:AssumeRole`. The Query API accepts
//        its signature in the query string, which keeps every AWS call here a
//        plain GET with no custom headers.
pub fn assume_role_url(creds: &AwsCredentials, step: &RoleStep, now: DateTime<Utc>) -> AppResult<String> {
    let host = sts_host(&creds.region);
    let session = step.session_name.clone().unwrap_or_else(|| default_session_name(now));
    let mut params: Vec<(&str, &str)> = vec![
        ("Action", "AssumeRole"),
        ("DurationSeconds", "3600"),
        ("RoleArn", step.role_arn.as_str()),
        ("RoleSessionName", session.as_str()),
        ("Version", "2011-06-15"),
    ];
    if let Some(external) = step.external_id.as_deref() {
        params.push(("ExternalId", external));
    }
    let query = presign_get(
        creds,
        &PresignRequest { service: "sts", host: &host, path: "/", params: &params, expires_secs: STS_PRESIGN_EXPIRES, payload_hash: EMPTY_PAYLOAD_HASH, now },
    )?;
    Ok(format!("https://{host}/?{query}"))
}

// WHAT:  `sts:AssumeRoleWithWebIdentity` is anonymous: the token is the proof.
pub fn web_identity_url(region: &str, role_arn: &str, session_name: &str, token: &str) -> String {
    let query = form_encode(&[
        ("Action", "AssumeRoleWithWebIdentity"),
        ("DurationSeconds", "3600"),
        ("RoleArn", role_arn),
        ("RoleSessionName", session_name),
        ("Version", "2011-06-15"),
        ("WebIdentityToken", token),
    ]);
    format!("https://{}/?{query}", sts_host(region))
}

fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

// WHAT:  The text of the first `<tag>…</tag>` in an XML body.
// WHY:   STS answers in a flat, namespaced XML whose few fields are unique
//        strings; a parser dependency would be heavier than this.
pub fn xml_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(xml_unescape(xml[start..end].trim()))
}

// WHAT:  The credentials in an `AssumeRole*Response`, or the error STS explained.
pub fn parse_sts_response(status: u16, body: &str) -> AppResult<ResolvedAws> {
    if !(200..300).contains(&status) {
        let code = xml_text(body, "Code").unwrap_or_else(|| status.to_string());
        let message = xml_text(body, "Message").unwrap_or_else(|| body.chars().take(200).collect());
        return Err(AppError::not_connected(format!("AWS STS refused the request ({code}): {message}")));
    }
    let field = |tag: &str| xml_text(body, tag).ok_or_else(|| AppError::driver(format!("AWS STS answered without {tag}.")));
    Ok(ResolvedAws {
        access_key: field("AccessKeyId")?,
        secret_key: field("SecretAccessKey")?,
        session_token: xml_text(body, "SessionToken"),
        expires_at: xml_text(body, "Expiration").as_deref().and_then(parse_time),
    })
}

async fn get_text(url: &str, headers: &[(&str, &str)]) -> AppResult<(u16, String)> {
    http()?.exchange("GET", url, headers, None).await
}

async fn assume_role(base: &ResolvedAws, step: &RoleStep, region: &str) -> AppResult<ResolvedAws> {
    let url = assume_role_url(&base.credentials(region), step, Utc::now())?;
    let (status, body) = get_text(&url, &[]).await?;
    parse_sts_response(status, &body)
}

// ----- the other credential sources -----

// WHAT:  `{AccessKeyId, SecretAccessKey, SessionToken|Token, Expiration}` as
//        printed by `credential_process` and served by the container endpoint.
pub fn parse_credential_json(body: &str) -> AppResult<ResolvedAws> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| AppError::driver(format!("Credentials were not JSON: {e}")))?;
    let access_key = json_str(&v, "AccessKeyId").ok_or_else(|| AppError::driver("The credentials have no AccessKeyId."))?;
    let secret_key = json_str(&v, "SecretAccessKey").ok_or_else(|| AppError::driver("The credentials have no SecretAccessKey."))?;
    Ok(ResolvedAws {
        access_key,
        secret_key,
        session_token: json_str(&v, "SessionToken").or_else(|| json_str(&v, "Token")),
        expires_at: json_str(&v, "Expiration").as_deref().and_then(parse_time),
    })
}

// WHAT:  Splits a `credential_process` command line into arguments: whitespace
//        separates, double or single quotes group, a backslash escapes.
pub fn split_command(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', Some('"')) | ('\\', None) if chars.peek().is_some_and(|n| matches!(n, '"' | '\'' | '\\' | ' ')) => {
                if let Some(next) = chars.next() {
                    current.push(next);
                    started = true;
                }
            }
            ('"' | '\'', None) => {
                quote = Some(c);
                started = true;
            }
            (q, Some(open)) if q == open => quote = None,
            (c, None) if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (c, _) => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

async fn run_credential_process(command: &str) -> AppResult<ResolvedAws> {
    let mut parts = split_command(command);
    if parts.is_empty() {
        return Err(AppError::invalid_input("credential_process is empty."));
    }
    let program = parts.remove(0);
    let out = run_process(&program, &parts).await?;
    parse_credential_json(&out)
}

// WHAT:  Picks the unexpired `aws sso login` token for `start_url` out of the
//        cache files' contents.
pub fn pick_sso_token(files: &[String], start_url: &str, now: DateTime<Utc>) -> Option<String> {
    let normal = |url: &str| url.trim().trim_end_matches('/').trim_end_matches('#').to_ascii_lowercase();
    files
        .iter()
        .filter_map(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .filter(|v| json_str(v, "startUrl").is_some_and(|u| normal(&u) == normal(start_url)))
        .filter_map(|v| {
            let token = json_str(&v, "accessToken")?;
            let expires = json_str(&v, "expiresAt").as_deref().and_then(parse_time)?;
            (expires > now).then_some((expires, token))
        })
        .max_by_key(|(expires, _)| *expires)
        .map(|(_, token)| token)
}

// WHAT:  `GetRoleCredentials` answer: `{roleCredentials:{accessKeyId, secretAccessKey, sessionToken, expiration(ms)}}`.
pub fn parse_sso_role_credentials(body: &str) -> AppResult<ResolvedAws> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| AppError::driver(format!("SSO answered with something that is not JSON: {e}")))?;
    let c = v.get("roleCredentials").ok_or_else(|| AppError::driver("SSO answered without role credentials."))?;
    Ok(ResolvedAws {
        access_key: json_str(c, "accessKeyId").ok_or_else(|| AppError::driver("SSO credentials have no accessKeyId."))?,
        secret_key: json_str(c, "secretAccessKey").ok_or_else(|| AppError::driver("SSO credentials have no secretAccessKey."))?,
        session_token: json_str(c, "sessionToken"),
        expires_at: c.get("expiration").and_then(serde_json::Value::as_i64).and_then(DateTime::from_timestamp_millis),
    })
}

async fn sso_credentials(start_url: &str, region: &str, account_id: &str, role_name: &str) -> AppResult<ResolvedAws> {
    let cache = home_dir().map(|h| h.join(".aws").join("sso").join("cache")).ok_or_else(|| AppError::not_found("The home directory could not be determined."))?;
    let mut files = Vec::new();
    if let Ok(mut dir) = tokio::fs::read_dir(&cache).await {
        while let Ok(Some(entry)) = dir.next_entry().await {
            if entry.path().extension().is_some_and(|e| e == "json") {
                if let Ok(text) = tokio::fs::read_to_string(entry.path()).await {
                    files.push(text);
                }
            }
        }
    }
    let token = pick_sso_token(&files, start_url, Utc::now())
        .ok_or_else(|| AppError::not_connected(format!("The AWS SSO session for {start_url} has expired or was never started. Run `aws sso login` and try again.")))?;
    let url = format!(
        "https://portal.sso.{region}.amazonaws.com/federation/credentials?{}",
        form_encode(&[("role_name", role_name), ("account_id", account_id)])
    );
    let (status, body) = get_text(&url, &[("x-amz-sso_bearer_token", token.as_str()), ("Accept", "application/json")]).await?;
    ensure_ok(status, &body)?;
    parse_sso_role_credentials(&body)
}

async fn container_credentials(url: &str, authorization: Option<&str>) -> AppResult<ResolvedAws> {
    let headers: Vec<(&str, &str)> = authorization.map(|a| vec![("Authorization", a)]).unwrap_or_default();
    let (status, body) = tokio::time::timeout(PROBE_TIMEOUT * 4, get_text(url, &headers))
        .await
        .map_err(|_| AppError::timeout("The container credentials endpoint did not answer."))??;
    ensure_ok(status, &body)?;
    parse_credential_json(&body)
}

fn environment_credentials() -> AppResult<ResolvedAws> {
    match (env_var("AWS_ACCESS_KEY_ID"), env_var("AWS_SECRET_ACCESS_KEY")) {
        (Some(access_key), Some(secret_key)) => Ok(ResolvedAws { access_key, secret_key, session_token: env_var("AWS_SESSION_TOKEN"), expires_at: None }),
        _ => Err(AppError::not_connected("AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY are not set.")),
    }
}

async fn run_base(base: &BaseSource, region: &str) -> AppResult<ResolvedAws> {
    match base {
        BaseSource::Static { access_key, secret_key, session_token } => {
            Ok(ResolvedAws { access_key: access_key.clone(), secret_key: secret_key.clone(), session_token: session_token.clone(), expires_at: None })
        }
        BaseSource::Environment => environment_credentials(),
        BaseSource::Process(command) => run_credential_process(command).await,
        BaseSource::Sso { start_url, region: sso_region, account_id, role_name } => sso_credentials(start_url, sso_region, account_id, role_name).await,
        BaseSource::WebIdentity { role_arn, token_file, session_name } => {
            let token = tokio::fs::read_to_string(token_file)
                .await
                .map_err(|e| AppError::not_connected(format!("Could not read the web identity token file {token_file}: {e}")))?;
            let session = session_name.clone().unwrap_or_else(|| default_session_name(Utc::now()));
            let (status, body) = get_text(&web_identity_url(region, role_arn, &session, token.trim()), &[]).await?;
            parse_sts_response(status, &body)
        }
        BaseSource::Container { url, authorization } => container_credentials(url, authorization.as_deref()).await,
    }
}

// WHAT:  Executes a plan: base credentials, then each role in turn.
async fn run_plan(plan: &AwsPlan, region: &str) -> AppResult<ResolvedAws> {
    let mut creds = run_base(&plan.base, region).await?;
    for step in &plan.roles {
        creds = assume_role(&creds, step, region).await?;
    }
    Ok(creds)
}

// WHAT:  The AWS identity described by `access`: where its base credentials come
//        from, then the role to assume, if any.
// HOW:   Anything that expires (STS, SSO, a credential process) is cached until
//        60 s before it does; static keys are read fresh every time.
pub async fn resolve_aws_cached(access: &AwsAccess, secrets: &AuthSecrets, region: &str) -> AppResult<ResolvedAws> {
    let key = format!("aws|{region}|{}|{}", serde_json::to_string(access).unwrap_or_default(), secrets.to_json().unwrap_or_default());
    if let Some(Entry { value: Cached::Aws(aws), .. }) = lookup(&key, Utc::now()) {
        return Ok(aws);
    }
    let aws = resolve_aws(access, secrets, region).await?;
    if let Some(expires_at) = aws.expires_at {
        remember(&key, Cached::Aws(aws.clone()), expires_at);
    }
    Ok(aws)
}

async fn resolve_aws(access: &AwsAccess, secrets: &AuthSecrets, region: &str) -> AppResult<ResolvedAws> {
    let (base, mut roles) = match &access.source {
        AwsCredentialSource::AccessKeys { access_key_id } => {
            let secret_key = secrets
                .aws_secret_access_key
                .as_deref()
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .ok_or_else(|| AppError::invalid_input("AWS secret access key is required."))?;
            let base = BaseSource::Static {
                access_key: access_key_id.trim().to_string(),
                secret_key: secret_key.to_string(),
                session_token: secrets.aws_session_token.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string),
            };
            (base, Vec::new())
        }
        AwsCredentialSource::Profile { name } => {
            let profiles = load_profiles().await;
            let plan = plan_profile(&profiles, name.trim(), &env_var)?;
            (plan.base, plan.roles)
        }
        AwsCredentialSource::DefaultChain => {
            let profiles = load_profiles().await;
            let plan = plan_default_chain(&profiles, &env_var)?;
            (plan.base, plan.roles)
        }
    };
    if let Some(role_arn) = trimmed(&access.role_arn) {
        roles.push(RoleStep { role_arn, session_name: trimmed(&access.role_session_name), external_id: trimmed(&access.external_id) });
    }
    run_plan(&AwsPlan { base, roles }, region).await
}

// ---------------------------------------------------------------------------
// AWS: DynamoDB session
// ---------------------------------------------------------------------------

enum AwsMode {
    /// Typed access keys (the password method), possibly with a session token.
    Fixed(AwsCredentials),
    /// A profile or the default chain, resolved on demand and refreshed.
    Resolved { access: AwsAccess, secrets: AuthSecrets },
}

// WHAT:  The signing identity of a DynamoDB session: the region plus a way to
//        get current credentials for each request.
pub struct AwsSession {
    region: String,
    mode: AwsMode,
}

impl std::fmt::Debug for AwsSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsSession").field("region", &self.region).finish_non_exhaustive()
    }
}

impl AwsSession {
    // WHAT:  Reads the connection: region in `host`, keys in `username` / the
    //        password (+ the session token field) for the password method, or a
    //        profile / default chain for `AwsCredentials`.
    pub async fn for_connection(conn: &ResolvedConnection) -> AppResult<AwsSession> {
        match &conn.summary.auth {
            ConnectionAuth::AwsCredentials { credentials } => {
                let region = match conn.summary.host.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
                    Some(region) => region.to_string(),
                    None => aws_region_hint(),
                };
                let secrets = current().map(|s| s.secrets.clone()).unwrap_or_default();
                let session = AwsSession { region, mode: AwsMode::Resolved { access: credentials.clone(), secrets } };
                // Fail at connect, with the real reason, instead of on the first request.
                session.credentials().await?;
                Ok(session)
            }
            _ => {
                let creds = AwsCredentials::from_connection(conn)?;
                Ok(AwsSession { region: creds.region.clone(), mode: AwsMode::Fixed(creds) })
            }
        }
    }

    pub fn region(&self) -> &str {
        &self.region
    }

    // WHAT:  Credentials valid right now (temporary ones are renewed ahead of expiry).
    pub async fn credentials(&self) -> AppResult<AwsCredentials> {
        match &self.mode {
            AwsMode::Fixed(creds) => Ok(creds.clone()),
            AwsMode::Resolved { access, secrets } => Ok(resolve_aws_cached(access, secrets, &self.region).await?.credentials(&self.region)),
        }
    }
}

// ---------------------------------------------------------------------------
// Azure AD
// ---------------------------------------------------------------------------

// WHAT:  The token scope for a connection: the user's override, else the one the
//        engine's service expects.
pub fn azure_scope(configured: Option<&str>, family: Family) -> String {
    match configured.map(str::trim).filter(|s| !s.is_empty()) {
        Some(scope) => scope.to_string(),
        None if family == Family::Mssql => AZURE_SQL_SCOPE.to_string(),
        None => AZURE_OSSRDBMS_SCOPE.to_string(),
    }
}

/// The v1 "resource" a scope stands for (`…/.default` stripped), for managed identity and the CLI.
pub fn azure_resource(scope: &str) -> String {
    scope.trim().trim_end_matches("/.default").to_string()
}

pub fn azure_token_url(tenant: &str) -> String {
    format!("https://login.microsoftonline.com/{}/oauth2/v2.0/token", uri_encode(tenant, true))
}

pub fn azure_client_credentials_form(client_id: &str, client_secret: &str, scope: &str) -> String {
    form_encode(&[("grant_type", "client_credentials"), ("client_id", client_id), ("client_secret", client_secret), ("scope", scope)])
}

pub fn azure_password_form(client_id: &str, username: &str, password: &str, scope: &str) -> String {
    form_encode(&[("grant_type", "password"), ("client_id", client_id), ("username", username), ("password", password), ("scope", scope)])
}

// WHAT:  `access_token` + lifetime from an OAuth token response. Entra sends
//        `expires_in` (seconds), instance metadata sends `expires_on` (epoch) as strings.
pub fn parse_oauth_token(body: &str, now: DateTime<Utc>) -> AppResult<Minted> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| AppError::driver(format!("The token response was not JSON: {e}")))?;
    let token = json_str(&v, "access_token").ok_or_else(|| AppError::driver("The token response has no access_token."))?;
    let expires_at = json_i64(&v, "expires_in")
        .map(|secs| now + ChronoDuration::seconds(secs))
        .or_else(|| json_i64(&v, "expires_on").and_then(from_epoch_secs))
        .unwrap_or_else(|| now + ChronoDuration::hours(1));
    Ok(Minted { token, expires_at })
}

async fn post_form(url: &str, form: String) -> AppResult<Minted> {
    let (status, body) = http()?.exchange("POST", url, &[("Accept", "application/json")], Some(("application/x-www-form-urlencoded", form))).await?;
    ensure_ok(status, &body)?;
    parse_oauth_token(&body, Utc::now())
}

/// App Service / Functions expose managed identity through `IDENTITY_ENDPOINT`; everything else uses IMDS.
pub fn managed_identity_request(resource: &str, client_id: Option<&str>, env: EnvFn<'_>) -> (String, Vec<(String, String)>) {
    let mut query = vec![("resource", resource.to_string())];
    if let (Some(endpoint), Some(header)) = (env("IDENTITY_ENDPOINT"), env("IDENTITY_HEADER")) {
        query.push(("api-version", "2019-08-01".to_string()));
        if let Some(id) = client_id {
            query.push(("client_id", id.to_string()));
        }
        let pairs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        return (format!("{endpoint}?{}", form_encode(&pairs)), vec![("X-IDENTITY-HEADER".to_string(), header)]);
    }
    query.push(("api-version", "2018-02-01".to_string()));
    if let Some(id) = client_id {
        query.push(("client_id", id.to_string()));
    }
    let pairs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
    (format!("http://169.254.169.254/metadata/identity/oauth2/token?{}", form_encode(&pairs)), vec![("Metadata".to_string(), "true".to_string())])
}

async fn managed_identity_token(resource: &str, client_id: Option<&str>, wait: Duration) -> AppResult<Minted> {
    let (url, headers) = managed_identity_request(resource, client_id, &env_var);
    let pairs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (status, body) = tokio::time::timeout(wait, get_text(&url, &pairs))
        .await
        .map_err(|_| AppError::not_connected("No managed identity answered: this machine is not an Azure resource with one assigned."))??;
    ensure_ok(status, &body)?;
    parse_oauth_token(&body, Utc::now())
}

async fn azure_cli_token(resource: &str) -> AppResult<Minted> {
    let program = if cfg!(windows) { "az.cmd" } else { "az" };
    let out = run_process(program, &["account".into(), "get-access-token".into(), "--resource".into(), resource.into(), "--output".into(), "json".into()]).await?;
    let v: serde_json::Value = serde_json::from_str(&out).map_err(|e| AppError::driver(format!("`az` printed something that is not JSON: {e}")))?;
    let token = json_str(&v, "accessToken").ok_or_else(|| AppError::driver("`az account get-access-token` returned no token."))?;
    let expires_at = json_i64(&v, "expires_on").and_then(from_epoch_secs).unwrap_or_else(|| Utc::now() + ChronoDuration::minutes(45));
    Ok(Minted { token, expires_at })
}

// WHAT:  An Entra access token by the chosen mode.
// HOW:   `Default` tries a service principal from the environment, then a
//        managed identity, then the Azure CLI, and reports every miss.
async fn azure_token(
    mode: AzureAdMode,
    tenant: Option<String>,
    client_id: Option<String>,
    account: Option<String>,
    scope: &str,
    secrets: &AuthSecrets,
) -> AppResult<Minted> {
    let secret_of = |value: &Option<String>, what: &str| {
        value.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).ok_or_else(|| AppError::invalid_input(format!("{what} is required.")))
    };
    let need = |value: &Option<String>, what: &str| value.clone().ok_or_else(|| AppError::invalid_input(format!("{what} is required.")));
    match mode {
        AzureAdMode::AccessToken => {
            let token = secret_of(&secrets.azure_access_token, "The Azure AD access token")?;
            let expires_at = jwt_expiry(&token).unwrap_or_else(|| Utc::now() + ChronoDuration::hours(1));
            if expires_at <= Utc::now() {
                return Err(AppError::not_connected("The Azure AD access token has expired. Paste a new one."));
            }
            Ok(Minted { token, expires_at })
        }
        AzureAdMode::ClientSecret => {
            let tenant = need(&tenant, "The tenant ID")?;
            let client = need(&client_id, "The client ID")?;
            let secret = secret_of(&secrets.azure_client_secret, "The client secret")?;
            post_form(&azure_token_url(&tenant), azure_client_credentials_form(&client, &secret, scope)).await
        }
        AzureAdMode::UsernamePassword => {
            let tenant = need(&tenant, "The tenant ID")?;
            let client = need(&client_id, "The client ID")?;
            let user = need(&account, "The Entra account")?;
            let password = secret_of(&secrets.azure_password, "The account password")?;
            post_form(&azure_token_url(&tenant), azure_password_form(&client, &user, &password, scope)).await
        }
        AzureAdMode::ManagedIdentity => managed_identity_token(&azure_resource(scope), client_id.as_deref(), PROBE_TIMEOUT * 4).await,
        AzureAdMode::Default => {
            let resource = azure_resource(scope);
            let mut misses: Vec<String> = Vec::new();
            if let (Some(tenant), Some(client), Some(secret)) = (env_var("AZURE_TENANT_ID"), env_var("AZURE_CLIENT_ID"), env_var("AZURE_CLIENT_SECRET")) {
                match post_form(&azure_token_url(&tenant), azure_client_credentials_form(&client, &secret, scope)).await {
                    Ok(minted) => return Ok(minted),
                    Err(err) => misses.push(format!("service principal from the environment: {err}")),
                }
            }
            // An App Service / Functions identity, or a client id naming a user-assigned one,
            // is a deliberate setup: try it before the CLI.
            let hinted = env_var("IDENTITY_ENDPOINT").is_some() || env_var("MSI_ENDPOINT").is_some() || client_id.is_some();
            if hinted {
                match managed_identity_token(&resource, client_id.as_deref(), PROBE_TIMEOUT * 4).await {
                    Ok(minted) => return Ok(minted),
                    Err(err) => misses.push(format!("managed identity: {err}")),
                }
            }
            match azure_cli_token(&resource).await {
                Ok(minted) => return Ok(minted),
                Err(err) => misses.push(format!("Azure CLI: {err}")),
            }
            // Last, an unannounced system-assigned identity (an Azure VM): a short probe,
            // because off Azure the metadata address simply never answers.
            if !hinted {
                match managed_identity_token(&resource, None, PROBE_TIMEOUT).await {
                    Ok(minted) => return Ok(minted),
                    Err(err) => misses.push(format!("managed identity: {err}")),
                }
            }
            Err(AppError::not_connected(format!(
                "No Azure credentials were found. Set AZURE_TENANT_ID, AZURE_CLIENT_ID and AZURE_CLIENT_SECRET, run `az login`, or choose another mode. Tried: {}",
                misses.join("; ")
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// Google Cloud SQL IAM
// ---------------------------------------------------------------------------

/// `application_default_credentials.json` of `gcloud auth application-default login`.
/// Not `Debug`: it holds a refresh token.
#[derive(serde::Deserialize)]
struct AuthorizedUser {
    client_id: String,
    client_secret: String,
    refresh_token: String,
}

pub fn google_refresh_form(client_id: &str, client_secret: &str, refresh_token: &str) -> String {
    form_encode(&[("grant_type", "refresh_token"), ("client_id", client_id), ("client_secret", client_secret), ("refresh_token", refresh_token)])
}

pub fn impersonation_url(target: &str) -> String {
    format!("https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{}:generateAccessToken", uri_encode(target, true))
}

pub fn impersonation_body(scopes: &str) -> serde_json::Value {
    serde_json::json!({ "scope": scopes.split_whitespace().collect::<Vec<_>>(), "lifetime": "3600s", "delegates": Vec::<String>::new() })
}

/// `{accessToken, expireTime}` from `generateAccessToken`.
pub fn parse_impersonated_token(body: &str, now: DateTime<Utc>) -> AppResult<Minted> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| AppError::driver(format!("The impersonation response was not JSON: {e}")))?;
    let token = json_str(&v, "accessToken").ok_or_else(|| AppError::driver("The impersonation response has no accessToken."))?;
    let expires_at = json_str(&v, "expireTime").as_deref().and_then(parse_time).unwrap_or_else(|| now + ChronoDuration::hours(1));
    Ok(Minted { token, expires_at })
}

fn well_known_adc_path() -> Option<std::path::PathBuf> {
    if cfg!(windows) {
        env_var("APPDATA").map(|a| std::path::PathBuf::from(a).join("gcloud").join("application_default_credentials.json"))
    } else {
        home_dir().map(|h| h.join(".config").join("gcloud").join("application_default_credentials.json"))
    }
}

async fn metadata_server_token(scopes: &str) -> AppResult<Minted> {
    let url = format!(
        "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token?scopes={}",
        uri_encode(&scopes.split_whitespace().collect::<Vec<_>>().join(","), true)
    );
    let (status, body) = tokio::time::timeout(PROBE_TIMEOUT, get_text(&url, &[("Metadata-Flavor", "Google")]))
        .await
        .map_err(|_| AppError::not_connected("The Google metadata server did not answer."))??;
    ensure_ok(status, &body)?;
    parse_oauth_token(&body, Utc::now())
}

// WHAT:  Application Default Credentials: `GOOGLE_APPLICATION_CREDENTIALS`, then
//        the gcloud well-known file, then the metadata server (GCE, Cloud Run).
async fn adc_token(scopes: &str) -> AppResult<Minted> {
    let path = env_var("GOOGLE_APPLICATION_CREDENTIALS").map(std::path::PathBuf::from).or_else(|| well_known_adc_path().filter(|p| p.exists()));
    let Some(path) = path else {
        return metadata_server_token(scopes).await.map_err(|_| {
            AppError::not_connected(
                "No Google credentials were found. Run `gcloud auth application-default login`, set GOOGLE_APPLICATION_CREDENTIALS, or use a service-account key.",
            )
        });
    };
    let text = tokio::fs::read_to_string(&path).await.map_err(|e| AppError::not_connected(format!("Could not read {}: {e}", path.display())))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| AppError::invalid_input(format!("{} is not valid JSON: {e}", path.display())))?;
    match json_str(&v, "type").as_deref() {
        Some("service_account") => gcp_auth::service_account_token(&text, scopes).await.map(|(token, ttl)| Minted {
            token,
            expires_at: Utc::now() + ChronoDuration::seconds(i64::try_from(ttl).unwrap_or(3600)),
        }),
        Some("authorized_user") => {
            let user: AuthorizedUser = serde_json::from_value(v).map_err(|e| AppError::invalid_input(format!("{} is not a gcloud user credential file: {e}", path.display())))?;
            post_form(GOOGLE_TOKEN_URL, google_refresh_form(&user.client_id, &user.client_secret, &user.refresh_token)).await
        }
        other => Err(AppError::invalid_input(format!(
            "{} holds a \"{}\" credential, which is not supported here. Use `gcloud auth application-default login` or a service-account key.",
            path.display(),
            other.unwrap_or("unknown")
        ))),
    }
}

async fn gcp_base_token(mode: GcpAuthMode, secrets: &AuthSecrets, scopes: &str) -> AppResult<Minted> {
    match mode {
        GcpAuthMode::ServiceAccountKey => {
            let json = secrets
                .gcp_service_account_json
                .as_deref()
                .map(str::trim)
                .filter(|j| !j.is_empty())
                .ok_or_else(|| AppError::invalid_input("The service-account JSON key is required."))?;
            let (token, ttl) = gcp_auth::service_account_token(json, scopes).await?;
            Ok(Minted { token, expires_at: Utc::now() + ChronoDuration::seconds(i64::try_from(ttl).unwrap_or(3600)) })
        }
        GcpAuthMode::Adc => adc_token(scopes).await,
    }
}

// WHAT:  A Cloud SQL access token: from the chosen credentials, or minted as
//        another service account when impersonation is on.
async fn gcp_token(mode: GcpAuthMode, impersonate: Option<String>, secrets: &AuthSecrets) -> AppResult<Minted> {
    let Some(target) = impersonate else {
        return gcp_base_token(mode, secrets, GCP_SQL_SCOPES).await;
    };
    let source = gcp_base_token(mode, secrets, GCP_CLOUD_PLATFORM_SCOPE).await?;
    let client = HttpClient::new("https://iamcredentials.googleapis.com", Auth::None, false)?;
    let bearer = format!("Bearer {}", source.token);
    let (status, body) = client
        .exchange(
            "POST",
            &impersonation_url(&target),
            &[("Authorization", bearer.as_str()), ("Accept", "application/json")],
            Some(("application/json", impersonation_body(GCP_SQL_SCOPES).to_string())),
        )
        .await?;
    ensure_ok(status, &body)?;
    parse_impersonated_token(&body, Utc::now())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::auth::OidcProvider;
    use crate::model::{Engine, Environment};

    fn at(rfc3339: &str) -> DateTime<Utc> {
        parse_time(rfc3339).unwrap_or_default()
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| (*v).to_string())
    }

    fn profiles(credentials: &str, config: &str) -> Profiles {
        merge_profiles(credentials, config)
    }

    #[test]
    fn a_token_is_fresh_until_its_last_minute() {
        let expiry = at("2024-01-01T00:15:00Z");
        assert!(is_fresh(expiry, at("2024-01-01T00:00:00Z")));
        assert!(is_fresh(expiry, at("2024-01-01T00:13:59Z")));
        assert!(!is_fresh(expiry, at("2024-01-01T00:14:00Z")), "refreshed 60 s before expiry");
        assert!(!is_fresh(expiry, at("2024-01-01T00:20:00Z")));
    }

    #[tokio::test]
    async fn a_cached_token_inside_the_skew_window_is_minted_again() {
        let key = "test|skew-window";
        remember(key, Cached::Token("expiring".into()), Utc::now() + ChronoDuration::seconds(30));
        let minted = cached_token(key, async {
            Ok(Minted { token: "renewed".into(), expires_at: Utc::now() + ChronoDuration::hours(1) })
        })
        .await
        .map(|m| m.token)
        .unwrap_or_default();
        assert_eq!(minted, "renewed");
    }

    #[test]
    fn refresh_waits_until_a_minute_before_expiry_within_sane_bounds() {
        let now = at("2024-01-01T00:00:00Z");
        assert_eq!(refresh_delay(at("2024-01-01T00:14:00Z"), now), Duration::from_secs(13 * 60));
        assert_eq!(refresh_delay(at("2024-01-01T00:00:30Z"), now), Duration::from_secs(1), "already inside the skew: now, but not in a spin");
        assert_eq!(refresh_delay(at("2023-12-31T00:00:00Z"), now), Duration::from_secs(1));
        assert_eq!(refresh_delay(at("2030-01-01T00:00:00Z"), now), Duration::from_secs(86_400));
    }

    #[test]
    fn the_cache_hands_out_a_token_until_it_is_stale() {
        let key = "test|cache|hands-out";
        remember(key, Cached::Token("tok".into()), Utc::now() + ChronoDuration::minutes(10));
        assert!(matches!(lookup(key, Utc::now()), Some(Entry { value: Cached::Token(t), .. }) if t == "tok"));
        let stale = "test|cache|stale";
        remember(stale, Cached::Token("old".into()), Utc::now() + ChronoDuration::seconds(30));
        assert!(lookup(stale, Utc::now()).is_none(), "inside the 60 s skew counts as expired");
    }

    #[test]
    fn a_cached_token_is_minted_once() {
        let key = "test|cache|once";
        let minted = Minted { token: "first".into(), expires_at: Utc::now() + ChronoDuration::minutes(10) };
        remember(key, Cached::Token(minted.token.clone()), minted.expires_at);
        let again = futures::executor::block_on(cached_token(key, async { Err(AppError::internal("must not mint a second time")) }));
        assert_eq!(again.map(|m| m.token).unwrap_or_default(), "first");
    }

    #[test]
    fn rds_regions_come_from_the_host_name() {
        assert_eq!(region_from_rds_host("mydb.abc123xyz.eu-west-1.rds.amazonaws.com").as_deref(), Some("eu-west-1"));
        assert_eq!(region_from_rds_host("cluster-x.cluster-ro-abc.us-gov-west-1.rds.amazonaws.com.").as_deref(), Some("us-gov-west-1"));
        assert_eq!(region_from_rds_host("db.abc.cn-north-1.rds.amazonaws.com.cn").as_deref(), Some("cn-north-1"));
        assert_eq!(region_from_rds_host("localhost"), None);
        assert_eq!(region_from_rds_host("db.example.com"), None);
        assert_eq!(region_from_rds_host("rds.amazonaws.com"), None);
    }

    #[test]
    fn the_rds_token_is_the_presigned_url_without_a_scheme() {
        let creds = AwsCredentials {
            region: "us-east-1".into(),
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let token = rds_auth_token(&creds, "mydb.abc.us-east-1.rds.amazonaws.com", 5432, "iam_user", at("2024-01-02T03:04:05Z")).unwrap_or_default();
        assert!(token.starts_with("mydb.abc.us-east-1.rds.amazonaws.com:5432/?Action=connect&DBUser=iam_user&X-Amz-Algorithm=AWS4-HMAC-SHA256&"));
        assert!(token.ends_with("&X-Amz-Signature=812963d83f4c6c43ba0e6037a72bd548dced6bb7af2f7cb596c1c64201452a11"), "{token}");
        assert!(!token.contains("https://"));
    }

    #[test]
    fn rds_tls_is_never_below_require_and_verifies_when_it_has_a_ca() {
        assert_eq!(cloud_ssl_mode(SslMode::Prefer, true), SslMode::VerifyCa);
        assert_eq!(cloud_ssl_mode(SslMode::Disable, true), SslMode::VerifyCa);
        assert_eq!(cloud_ssl_mode(SslMode::Prefer, false), SslMode::Require);
        assert_eq!(cloud_ssl_mode(SslMode::Require, true), SslMode::Require, "an explicit choice stands");
        assert_eq!(cloud_ssl_mode(SslMode::VerifyFull, false), SslMode::VerifyFull);
    }

    #[test]
    fn the_bundled_rds_ca_is_a_pem_bundle() {
        let certs = AWS_RDS_CA_BUNDLE.matches("-----BEGIN CERTIFICATE-----").count();
        assert!(certs > 50, "{certs} certificates");
        assert_eq!(certs, AWS_RDS_CA_BUNDLE.matches("-----END CERTIFICATE-----").count());
    }

    const CREDENTIALS: &str = "\
[default]
aws_access_key_id = AKIADEFAULT
aws_secret_access_key = secretdefault

[ci]
aws_access_key_id=AKIACI
aws_secret_access_key=secretci
aws_session_token=tokenci
# a comment
";
    const CONFIG: &str = "\
[default]
region = eu-west-1

[profile admin]
role_arn = arn:aws:iam::111111111111:role/Admin
source_profile = ci
role_session_name = db-free-admin
external_id = ext-1

[profile chained]
role_arn = arn:aws:iam::222222222222:role/Chained
source_profile = admin

[profile sso-dev]
sso_session = corp
sso_account_id = 333333333333
sso_role_name = Developer
s3 =
  max_concurrent_requests = 10

[sso-session corp]
sso_start_url = https://corp.awsapps.com/start
sso_region = us-east-2

[profile proc]
credential_process = /usr/local/bin/get-creds --profile \"my prod\"

[profile loop-a]
role_arn = arn:aws:iam::1:role/A
source_profile = loop-b

[profile loop-b]
role_arn = arn:aws:iam::1:role/B
source_profile = loop-a

[profile mfa]
role_arn = arn:aws:iam::1:role/M
mfa_serial = arn:aws:iam::1:mfa/me
aws_access_key_id = AKIAMFA
aws_secret_access_key = s

[profile empty]
region = us-east-1
";

    #[test]
    fn the_shared_files_merge_the_way_the_sdks_read_them() {
        let p = profiles(CREDENTIALS, CONFIG);
        assert_eq!(p["default"]["aws_access_key_id"], "AKIADEFAULT", "keys from the credentials file");
        assert_eq!(p["default"]["region"], "eu-west-1", "region from the config file");
        assert_eq!(p["ci"]["aws_session_token"], "tokenci");
        assert_eq!(p["admin"]["role_arn"], "arn:aws:iam::111111111111:role/Admin", "[profile x] becomes x");
        assert!(!p["sso-dev"].contains_key("max_concurrent_requests"), "nested sub-settings are skipped");
        assert_eq!(p["sso-session corp"]["sso_region"], "us-east-2");
    }

    #[test]
    fn a_static_profile_plans_no_roles() {
        let plan = plan_profile(&profiles(CREDENTIALS, CONFIG), "ci", &no_env).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(plan.base, BaseSource::Static { access_key: "AKIACI".into(), secret_key: "secretci".into(), session_token: Some("tokenci".into()) });
        assert!(plan.roles.is_empty());
    }

    #[test]
    fn source_profile_chains_assume_roles_from_the_bottom_up() {
        let plan = plan_profile(&profiles(CREDENTIALS, CONFIG), "chained", &no_env).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(&plan.base, BaseSource::Static { access_key, .. } if access_key == "AKIACI"));
        assert_eq!(
            plan.roles,
            vec![
                RoleStep {
                    role_arn: "arn:aws:iam::111111111111:role/Admin".into(),
                    session_name: Some("db-free-admin".into()),
                    external_id: Some("ext-1".into())
                },
                RoleStep { role_arn: "arn:aws:iam::222222222222:role/Chained".into(), session_name: None, external_id: None },
            ]
        );
    }

    #[test]
    fn an_sso_profile_reads_its_session_section() {
        let plan = plan_profile(&profiles(CREDENTIALS, CONFIG), "sso-dev", &no_env).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            plan.base,
            BaseSource::Sso {
                start_url: "https://corp.awsapps.com/start".into(),
                region: "us-east-2".into(),
                account_id: "333333333333".into(),
                role_name: "Developer".into()
            }
        );
    }

    #[test]
    fn credential_process_profiles_keep_their_command_line() {
        let plan = plan_profile(&profiles(CREDENTIALS, CONFIG), "proc", &no_env).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(plan.base, BaseSource::Process("/usr/local/bin/get-creds --profile \"my prod\"".into()));
        assert_eq!(split_command("/usr/local/bin/get-creds --profile \"my prod\""), vec!["/usr/local/bin/get-creds", "--profile", "my prod"]);
        assert_eq!(split_command("  a   'b c' d\\ e  "), vec!["a", "b c", "d e"]);
        assert_eq!(split_command(""), Vec::<String>::new());
    }

    #[test]
    fn profile_problems_are_named() {
        let p = profiles(CREDENTIALS, CONFIG);
        assert!(plan_profile(&p, "nope", &no_env).is_err());
        assert!(plan_profile(&p, "loop-a", &no_env).is_err(), "source_profile cycle");
        assert!(plan_profile(&p, "mfa", &no_env).is_err(), "MFA cannot be prompted for");
        assert!(plan_profile(&p, "empty", &no_env).is_err(), "no credentials");
    }

    #[test]
    fn the_default_chain_prefers_environment_keys_then_the_profile_then_web_identity() {
        let p = profiles(CREDENTIALS, CONFIG);
        let env = env_of(&[("AWS_ACCESS_KEY_ID", "AKIAENV"), ("AWS_SECRET_ACCESS_KEY", "s")]);
        assert_eq!(plan_default_chain(&p, &env).map(|p| p.base).ok(), Some(BaseSource::Environment));
        let named = env_of(&[("AWS_PROFILE", "ci")]);
        assert!(matches!(plan_default_chain(&p, &named).map(|p| p.base), Ok(BaseSource::Static { access_key, .. }) if access_key == "AKIACI"));
        let missing = env_of(&[("AWS_PROFILE", "ghost")]);
        assert!(plan_default_chain(&p, &missing).is_err(), "a named profile that does not exist is an error, not a fallthrough");
        assert!(matches!(plan_default_chain(&p, &no_env).map(|p| p.base), Ok(BaseSource::Static { access_key, .. }) if access_key == "AKIADEFAULT"));
        let web = env_of(&[("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/token"), ("AWS_ROLE_ARN", "arn:aws:iam::1:role/R")]);
        assert!(matches!(plan_default_chain(&Profiles::new(), &web).map(|p| p.base), Ok(BaseSource::WebIdentity { .. })));
        let ecs = env_of(&[("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "/v2/credentials/abc")]);
        assert_eq!(
            plan_default_chain(&Profiles::new(), &ecs).map(|p| p.base).ok(),
            Some(BaseSource::Container { url: "http://169.254.170.2/v2/credentials/abc".into(), authorization: None })
        );
        assert!(plan_default_chain(&Profiles::new(), &no_env).is_err());
    }

    #[test]
    fn assume_role_is_a_presigned_sts_get() {
        let creds = AwsCredentials {
            region: "us-east-1".into(),
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: Some("tok".into()),
        };
        let step = RoleStep { role_arn: "arn:aws:iam::123456789012:role/Admin".into(), session_name: Some("me".into()), external_id: Some("x y".into()) };
        let url = assume_role_url(&creds, &step, at("2024-01-02T03:04:05Z")).unwrap_or_default();
        assert!(url.starts_with("https://sts.us-east-1.amazonaws.com/?Action=AssumeRole&DurationSeconds=3600&ExternalId=x%20y&RoleArn=arn%3Aaws%3Aiam%3A%3A123456789012%3Arole%2FAdmin&RoleSessionName=me&Version=2011-06-15&X-Amz-Algorithm=AWS4-HMAC-SHA256&"), "{url}");
        assert!(url.contains("X-Amz-Security-Token=tok&"), "temporary credentials sign their token");
        assert!(url.contains("X-Amz-Expires=300"));
        assert!(url.contains("&X-Amz-Signature="));
        let china = AwsCredentials { region: "cn-north-1".into(), ..creds };
        assert!(assume_role_url(&china, &step, at("2024-01-02T03:04:05Z")).unwrap_or_default().starts_with("https://sts.cn-north-1.amazonaws.com.cn/?"));
    }

    #[test]
    fn web_identity_is_an_anonymous_sts_get() {
        let url = web_identity_url("eu-west-1", "arn:aws:iam::1:role/R", "sess", "a.b+c");
        assert_eq!(
            url,
            "https://sts.eu-west-1.amazonaws.com/?Action=AssumeRoleWithWebIdentity&DurationSeconds=3600&RoleArn=arn%3Aaws%3Aiam%3A%3A1%3Arole%2FR&RoleSessionName=sess&Version=2011-06-15&WebIdentityToken=a.b%2Bc"
        );
    }

    #[test]
    fn sts_answers_parse_and_errors_explain_themselves() {
        let ok = "<AssumeRoleResponse xmlns=\"https://sts.amazonaws.com/doc/2011-06-15/\"><AssumeRoleResult><Credentials>\
                  <AccessKeyId>ASIAXYZ</AccessKeyId><SecretAccessKey>s3cr&amp;t</SecretAccessKey><SessionToken>tok+/=</SessionToken>\
                  <Expiration>2024-01-02T04:04:05Z</Expiration></Credentials></AssumeRoleResult></AssumeRoleResponse>";
        let creds = parse_sts_response(200, ok).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((creds.access_key.as_str(), creds.secret_key.as_str()), ("ASIAXYZ", "s3cr&t"));
        assert_eq!(creds.session_token.as_deref(), Some("tok+/="));
        assert_eq!(creds.expires_at, Some(at("2024-01-02T04:04:05Z")));
        let denied = "<ErrorResponse><Error><Type>Sender</Type><Code>AccessDenied</Code><Message>User is not authorized to perform: sts:AssumeRole</Message></Error></ErrorResponse>";
        let err = parse_sts_response(403, denied).map(|_| ()).unwrap_err().to_string();
        assert!(err.contains("AccessDenied") && err.contains("not authorized"), "{err}");
        assert!(parse_sts_response(200, "<AssumeRoleResponse/>").is_err());
    }

    #[test]
    fn credential_json_covers_processes_and_containers() {
        let process = parse_credential_json(r#"{"Version":1,"AccessKeyId":"A","SecretAccessKey":"S","SessionToken":"T","Expiration":"2024-01-02T04:04:05Z"}"#).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(process.session_token.as_deref(), Some("T"));
        assert_eq!(process.expires_at, Some(at("2024-01-02T04:04:05Z")));
        let container = parse_credential_json(r#"{"AccessKeyId":"A","SecretAccessKey":"S","Token":"CT"}"#).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(container.session_token.as_deref(), Some("CT"));
        assert!(container.expires_at.is_none());
        assert!(parse_credential_json(r#"{"AccessKeyId":"A"}"#).is_err());
        assert!(parse_credential_json("nope").is_err());
    }

    #[test]
    fn the_sso_token_is_the_latest_unexpired_one_for_the_start_url() {
        let now = at("2024-01-01T00:00:00Z");
        let files = vec![
            r#"{"startUrl":"https://corp.awsapps.com/start","accessToken":"old","expiresAt":"2024-01-01T01:00:00Z"}"#.to_string(),
            r#"{"startUrl":"https://corp.awsapps.com/start/","accessToken":"new","expiresAt":"2024-01-01T08:00:00UTC"}"#.to_string(),
            r#"{"startUrl":"https://corp.awsapps.com/start","accessToken":"expired","expiresAt":"2023-12-31T00:00:00Z"}"#.to_string(),
            r#"{"startUrl":"https://other.awsapps.com/start","accessToken":"other","expiresAt":"2024-01-02T00:00:00Z"}"#.to_string(),
            "not json".to_string(),
        ];
        assert_eq!(pick_sso_token(&files, "https://corp.awsapps.com/start", now).as_deref(), Some("new"));
        assert_eq!(pick_sso_token(&files, "https://nobody.awsapps.com/start", now), None);
        let creds = parse_sso_role_credentials(r#"{"roleCredentials":{"accessKeyId":"A","secretAccessKey":"S","sessionToken":"T","expiration":1704071045000}}"#).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(creds.expires_at, Some(at("2024-01-01T01:04:05Z")));
    }

    #[test]
    fn azure_scopes_follow_the_engine_unless_overridden() {
        assert_eq!(azure_scope(None, Family::Postgres), AZURE_OSSRDBMS_SCOPE);
        assert_eq!(azure_scope(Some(" "), Family::Mysql), AZURE_OSSRDBMS_SCOPE);
        assert_eq!(azure_scope(None, Family::Mssql), AZURE_SQL_SCOPE);
        assert_eq!(azure_scope(Some("https://x/.default"), Family::Mssql), "https://x/.default");
        assert_eq!(azure_resource(AZURE_OSSRDBMS_SCOPE), "https://ossrdbms-aad.database.windows.net");
    }

    #[test]
    fn azure_requests_are_form_encoded_for_the_tenant_endpoint() {
        assert_eq!(azure_token_url("contoso.onmicrosoft.com"), "https://login.microsoftonline.com/contoso.onmicrosoft.com/oauth2/v2.0/token");
        assert_eq!(
            azure_client_credentials_form("cid", "se&cret", AZURE_OSSRDBMS_SCOPE),
            "grant_type=client_credentials&client_id=cid&client_secret=se%26cret&scope=https%3A%2F%2Fossrdbms-aad.database.windows.net%2F.default"
        );
        assert_eq!(
            azure_password_form("cid", "me@corp.com", "p w", AZURE_SQL_SCOPE),
            "grant_type=password&client_id=cid&username=me%40corp.com&password=p%20w&scope=https%3A%2F%2Fdatabase.windows.net%2F.default"
        );
    }

    #[test]
    fn oauth_responses_give_a_token_and_a_lifetime() {
        let now = at("2024-01-01T00:00:00Z");
        let entra = parse_oauth_token(r#"{"token_type":"Bearer","expires_in":3599,"access_token":"AT"}"#, now).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((entra.token.as_str(), entra.expires_at), ("AT", at("2024-01-01T00:59:59Z")));
        let imds = parse_oauth_token(r#"{"access_token":"AT","expires_in":"3600","expires_on":"1704070800"}"#, now).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(imds.expires_at, at("2024-01-01T01:00:00Z"));
        let only_on = parse_oauth_token(r#"{"access_token":"AT","expires_on":"1704070800"}"#, now).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(only_on.expires_at, at("2024-01-01T01:00:00Z"));
        assert!(parse_oauth_token(r#"{"error":"x"}"#, now).is_err());
    }

    #[test]
    fn a_refusal_reads_as_the_providers_own_first_line() {
        let entra = r#"{"error":"invalid_client","error_description":"AADSTS7000215: Invalid client secret provided.\r\nTrace ID: 1\r\nCorrelation ID: 2"}"#;
        let message = oauth_failure(401, entra).to_string();
        assert!(message.contains("AADSTS7000215: Invalid client secret provided."), "{message}");
        assert!(!message.contains("Trace ID"), "only the first line: {message}");
        let google = r#"{"error":{"code":403,"message":"Permission 'iam.serviceAccounts.getAccessToken' denied"}}"#;
        assert!(oauth_failure(403, google).to_string().contains("denied"));
        assert!(oauth_failure(500, "plain text body").to_string().contains("plain text body"));
    }

    #[test]
    fn managed_identity_uses_app_service_when_it_is_there_and_imds_otherwise() {
        let (url, headers) = managed_identity_request("https://database.windows.net", None, &no_env);
        assert_eq!(url, "http://169.254.169.254/metadata/identity/oauth2/token?resource=https%3A%2F%2Fdatabase.windows.net&api-version=2018-02-01");
        assert_eq!(headers, vec![("Metadata".to_string(), "true".to_string())]);
        let env = env_of(&[("IDENTITY_ENDPOINT", "http://127.0.0.1:41234/msi/token"), ("IDENTITY_HEADER", "hdr")]);
        let (url, headers) = managed_identity_request("https://x", Some("client-1"), &env);
        assert_eq!(url, "http://127.0.0.1:41234/msi/token?resource=https%3A%2F%2Fx&api-version=2019-08-01&client_id=client-1");
        assert_eq!(headers, vec![("X-IDENTITY-HEADER".to_string(), "hdr".to_string())]);
        let (user_assigned, _) = managed_identity_request("https://x", Some("c"), &no_env);
        assert!(user_assigned.ends_with("&api-version=2018-02-01&client_id=c"));
    }

    #[test]
    fn a_pasted_azure_token_reports_its_own_expiry() {
        // header.payload.signature with payload {"exp":1704070800}
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"exp":1704070800}"#);
        assert_eq!(jwt_expiry(&format!("e30.{payload}.sig")), Some(at("2024-01-01T01:00:00Z")));
        assert_eq!(jwt_expiry("not-a-jwt"), None);
    }

    #[test]
    fn google_requests_and_answers() {
        assert_eq!(
            google_refresh_form("cid", "sec", "1//rt"),
            "grant_type=refresh_token&client_id=cid&client_secret=sec&refresh_token=1%2F%2Frt"
        );
        assert_eq!(
            impersonation_url("sa@proj.iam.gserviceaccount.com"),
            "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/sa%40proj.iam.gserviceaccount.com:generateAccessToken"
        );
        let body = impersonation_body(GCP_SQL_SCOPES);
        assert_eq!(body["lifetime"], "3600s");
        assert_eq!(body["scope"][0], "https://www.googleapis.com/auth/sqlservice.admin");
        assert_eq!(body["scope"][1], "https://www.googleapis.com/auth/sqlservice.login");
        let now = at("2024-01-01T00:00:00Z");
        let minted = parse_impersonated_token(r#"{"accessToken":"ya29.x","expireTime":"2024-01-01T01:00:00Z"}"#, now).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((minted.token.as_str(), minted.expires_at), ("ya29.x", at("2024-01-01T01:00:00Z")));
        assert!(parse_impersonated_token("{}", now).is_err());
    }

    fn summary(engine: Engine, auth: ConnectionAuth) -> ConnectionSummary {
        let input = crate::model::ConnectionInput {
            name: "t".into(),
            engine,
            environment: Environment::Local,
            read_only: false,
            host: Some("mydb.abc.eu-west-1.rds.amazonaws.com".into()),
            port: Some(5432),
            database: None,
            username: Some("iam_user".into()),
            password: None,
            file_path: None,
            ssl_mode: SslMode::Prefer,
            ssh: crate::model::SshTunnel::default(),
            ssh_secret: None,
            folder: None,
            color: None,
            favorite: false,
            extra: Default::default(),
            auth,
            auth_secrets: AuthSecrets::default(),
        };
        ConnectionSummary::draft(&input, false)
    }

    #[test]
    fn a_plain_password_connection_gets_no_session_and_an_unchanged_plan() {
        let s = summary(Engine::Postgres, ConnectionAuth::Password);
        assert!(CloudSession::for_connection(&s, AuthSecrets::default()).is_none());
        let conn = ResolvedConnection { summary: s, secret: Some("pw".into()) };
        let plan = futures::executor::block_on(db_plan(&conn)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((plan.user.as_deref(), plan.password.as_deref(), plan.ssl_mode), (Some("iam_user"), Some("pw"), SslMode::Prefer));
        assert!(plan.ca_pem.is_none() && plan.refresh.is_none() && !plan.cleartext_password);
    }

    #[test]
    fn dynamodb_session_tokens_ride_on_a_password_connection() {
        let secrets = AuthSecrets { aws_session_token: Some(" tok ".into()), ..AuthSecrets::default() };
        let session = CloudSession::for_connection(&summary(Engine::Dynamodb, ConnectionAuth::Password), secrets).unwrap_or_else(|| panic!("a session"));
        let seen = futures::executor::block_on(with_session(Some(session), async { session_token() }));
        assert_eq!(seen.as_deref(), Some("tok"));
        assert_eq!(session_token(), None, "outside a connect there is no ambient session");
    }

    #[test]
    fn a_cloud_method_without_a_session_asks_for_the_saved_connection() {
        let auth = ConnectionAuth::AzureAd { mode: AzureAdMode::Default, tenant_id: None, client_id: None, username: None, scope: None };
        let conn = ResolvedConnection { summary: summary(Engine::Postgres, auth), secret: None };
        let err = futures::executor::block_on(db_plan(&conn)).map(|_| ()).unwrap_err().to_string();
        assert!(err.contains("saved connection"), "{err}");
    }

    #[test]
    fn a_pasted_azure_token_is_used_as_the_password_and_never_refreshed() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{}}}"#, Utc::now().timestamp() + 3600));
        let token = format!("e30.{payload}.sig");
        let auth = ConnectionAuth::AzureAd { mode: AzureAdMode::AccessToken, tenant_id: None, client_id: None, username: None, scope: None };
        let secrets = AuthSecrets { azure_access_token: Some(token.clone()), ..AuthSecrets::default() };
        let s = summary(Engine::Postgres, auth);
        let session = CloudSession::for_connection(&s, secrets).unwrap_or_else(|| panic!("a session"));
        assert!(!session.refreshable());
        let conn = ResolvedConnection { summary: s, secret: None };
        let plan = futures::executor::block_on(with_session(Some(session), db_plan(&conn))).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(plan.password.as_deref(), Some(token.as_str()));
        assert_eq!(plan.user.as_deref(), Some("iam_user"));
        assert_eq!(plan.ssl_mode, SslMode::Require);
        assert!(plan.cleartext_password && plan.refresh.is_none());
    }

    #[test]
    fn an_expired_pasted_token_is_refused_with_a_reason() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"exp":1000}"#);
        let secrets = AuthSecrets { azure_access_token: Some(format!("e30.{payload}.sig")), ..AuthSecrets::default() };
        let outcome = futures::executor::block_on(azure_token(AzureAdMode::AccessToken, None, None, None, AZURE_SQL_SCOPE, &secrets));
        assert!(outcome.map(|_| ()).unwrap_err().to_string().contains("expired"));
    }

    #[test]
    fn mssql_uses_the_token_alone_and_postgres_needs_a_user_with_it() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{}}}"#, Utc::now().timestamp() + 3600));
        let token = format!("e30.{payload}.sig");
        let auth = ConnectionAuth::AzureAd { mode: AzureAdMode::AccessToken, tenant_id: None, client_id: None, username: None, scope: None };
        let secrets = AuthSecrets { azure_access_token: Some(token.clone()), ..AuthSecrets::default() };
        let mut sql_server = summary(Engine::Mssql, auth.clone());
        sql_server.username = None;
        let session = CloudSession::for_connection(&sql_server, secrets.clone()).unwrap_or_else(|| panic!("a session"));
        let conn = ResolvedConnection { summary: sql_server, secret: None };
        assert_eq!(futures::executor::block_on(with_session(Some(session), aad_token(&conn))).ok().flatten().as_deref(), Some(token.as_str()));
        let mut postgres = summary(Engine::Postgres, auth);
        postgres.username = None;
        let session = CloudSession::for_connection(&postgres, secrets).unwrap_or_else(|| panic!("a session"));
        let conn = ResolvedConnection { summary: postgres, secret: None };
        assert!(futures::executor::block_on(with_session(Some(session), db_plan(&conn))).is_err());
    }

    #[test]
    fn atlas_oidc_names_the_environment_and_resource() {
        let auth = ConnectionAuth::MongoAtlasOidc { provider: OidcProvider::Azure, token_resource: Some(" api://atlas ".into()) };
        let s = summary(Engine::Mongodb, auth);
        let session = CloudSession::for_connection(&s, AuthSecrets::default()).unwrap_or_else(|| panic!("a session"));
        let conn = ResolvedConnection { summary: s, secret: None };
        let mechanism = futures::executor::block_on(with_session(Some(session), mongo_auth(&conn))).unwrap_or(None);
        assert_eq!(mechanism, Some(MongoAuth::Oidc { environment: "azure", token_resource: Some("api://atlas".into()) }));
        let plain = ResolvedConnection { summary: summary(Engine::Mongodb, ConnectionAuth::Password), secret: None };
        assert_eq!(futures::executor::block_on(mongo_auth(&plain)).ok().flatten(), None);
    }

    #[test]
    fn atlas_aws_with_typed_keys_hands_the_driver_explicit_credentials() {
        let auth = ConnectionAuth::MongoAtlasAws {
            credentials: AwsAccess { source: AwsCredentialSource::AccessKeys { access_key_id: "AKIATYPED".into() }, ..AwsAccess::default() },
        };
        let secrets = AuthSecrets { aws_secret_access_key: Some("sekret".into()), aws_session_token: Some("tkn".into()), ..AuthSecrets::default() };
        let s = summary(Engine::Mongodb, auth);
        let session = CloudSession::for_connection(&s, secrets).unwrap_or_else(|| panic!("a session"));
        let conn = ResolvedConnection { summary: s, secret: None };
        let mechanism = futures::executor::block_on(with_session(Some(session), mongo_auth(&conn))).unwrap_or(None);
        assert_eq!(
            mechanism,
            Some(MongoAuth::Aws { access_key_id: "AKIATYPED".into(), secret_access_key: "sekret".into(), session_token: Some("tkn".into()) })
        );
    }

    #[test]
    fn typed_aws_keys_without_a_secret_say_so() {
        let access = AwsAccess { source: AwsCredentialSource::AccessKeys { access_key_id: "AKIA".into() }, ..AwsAccess::default() };
        let err = futures::executor::block_on(resolve_aws(&access, &AuthSecrets::default(), "us-east-1")).map(|_| ()).unwrap_err().to_string();
        assert!(err.contains("secret access key"), "{err}");
    }

    #[test]
    fn the_rds_token_is_minted_from_typed_keys_and_cached() {
        let auth = ConnectionAuth::AwsRdsIam {
            region: None,
            credentials: AwsAccess { source: AwsCredentialSource::AccessKeys { access_key_id: "AKIDEXAMPLE".into() }, ..AwsAccess::default() },
            ca_bundle: None,
        };
        let secrets = AuthSecrets { aws_secret_access_key: Some("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into()), ..AuthSecrets::default() };
        let s = summary(Engine::Postgres, auth);
        let session = CloudSession::for_connection(&s, secrets).unwrap_or_else(|| panic!("a session"));
        let first = futures::executor::block_on(session.db_credentials()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(first.user.as_deref(), Some("iam_user"));
        assert!(first.password.starts_with("mydb.abc.eu-west-1.rds.amazonaws.com:5432/?Action=connect&DBUser=iam_user&"), "{}", first.password);
        assert!(first.password.contains("%2Feu-west-1%2Frds-db%2Faws4_request"), "signed for the region read from the host");
        let second = futures::executor::block_on(session.db_credentials()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(second.password, first.password, "the same token comes from the cache within its lifetime");
        assert!(is_fresh(first.expires_at, Utc::now()));
        // The plan wraps it for the driver with a CA to verify against.
        let conn = ResolvedConnection { summary: s, secret: None };
        let plan = futures::executor::block_on(with_session(Some(session), db_plan(&conn))).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(plan.ssl_mode, SslMode::VerifyCa);
        assert!(plan.ca_pem.as_deref().is_some_and(|pem| pem.contains("BEGIN CERTIFICATE")));
        assert!(plan.refresh.is_some() && plan.cleartext_password);
    }

    #[test]
    fn a_dynamodb_password_connection_signs_with_its_typed_keys_and_session_token() {
        let mut s = summary(Engine::Dynamodb, ConnectionAuth::Password);
        s.host = Some("eu-west-1".into());
        s.username = Some("AKIATYPED".into());
        let secrets = AuthSecrets { aws_session_token: Some("tok".into()), ..AuthSecrets::default() };
        let session = CloudSession::for_connection(&s, secrets);
        let conn = ResolvedConnection { summary: s, secret: Some("sekret".into()) };
        let aws = futures::executor::block_on(with_session(session, AwsSession::for_connection(&conn))).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(aws.region(), "eu-west-1");
        let creds = futures::executor::block_on(aws.credentials()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((creds.access_key.as_str(), creds.secret_key.as_str(), creds.session_token.as_deref()), ("AKIATYPED", "sekret", Some("tok")));
    }

    #[test]
    fn a_dynamodb_secret_saved_with_the_old_pipe_still_signs() {
        let mut s = summary(Engine::Dynamodb, ConnectionAuth::Password);
        s.host = Some("us-east-1".into());
        s.username = Some("AKIAOLD".into());
        let conn = ResolvedConnection { summary: s, secret: Some("sekret|oldtoken".into()) };
        let aws = futures::executor::block_on(AwsSession::for_connection(&conn)).unwrap_or_else(|e| panic!("{e}"));
        let creds = futures::executor::block_on(aws.credentials()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((creds.secret_key.as_str(), creds.session_token.as_deref()), ("sekret", Some("oldtoken")));
    }

    #[test]
    fn a_dynamodb_keys_source_without_a_password_field_reads_its_sealed_secrets() {
        let auth = ConnectionAuth::AwsCredentials {
            credentials: AwsAccess { source: AwsCredentialSource::AccessKeys { access_key_id: "AKIAK".into() }, ..AwsAccess::default() },
        };
        let secrets = AuthSecrets { aws_secret_access_key: Some("s".into()), aws_session_token: Some("t".into()), ..AuthSecrets::default() };
        let mut s = summary(Engine::Dynamodb, auth);
        s.host = Some("ap-south-1".into());
        let session = CloudSession::for_connection(&s, secrets);
        let conn = ResolvedConnection { summary: s, secret: None };
        let aws = futures::executor::block_on(with_session(session, AwsSession::for_connection(&conn))).unwrap_or_else(|e| panic!("{e}"));
        let creds = futures::executor::block_on(aws.credentials()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((creds.region.as_str(), creds.access_key.as_str(), creds.secret_key.as_str(), creds.session_token.as_deref()), ("ap-south-1", "AKIAK", "s", Some("t")));
    }

    #[test]
    fn debug_output_never_contains_a_secret() {
        let secrets = AuthSecrets { aws_secret_access_key: Some("hunter2".into()), ..AuthSecrets::default() };
        let session = CloudSession::for_connection(&summary(Engine::Postgres, ConnectionAuth::Password), secrets).unwrap_or_else(|| panic!("a session"));
        assert!(!format!("{session:?}").contains("hunter2"));
        let minted = Minted { token: "hunter2".into(), expires_at: Utc::now() };
        assert!(!format!("{minted:?}").contains("hunter2"));
        let aws = ResolvedAws { access_key: "A".into(), secret_key: "hunter2".into(), session_token: Some("hunter2".into()), expires_at: None };
        assert!(!format!("{aws:?}").contains("hunter2"));
        let mongo = MongoAuth::Aws { access_key_id: "A".into(), secret_access_key: "hunter2".into(), session_token: None };
        assert!(!format!("{mongo:?}").contains("hunter2"));
    }

    #[test]
    fn the_aws_session_resolves_profile_keys_from_a_credentials_file() {
        // No AWS access here: only the planning half, which is pure.
        let plan = plan_profile(&profiles(CREDENTIALS, CONFIG), "default", &no_env).unwrap_or_else(|e| panic!("{e}"));
        let base = futures::executor::block_on(run_base(&plan.base, "eu-west-1")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((base.access_key.as_str(), base.secret_key.as_str()), ("AKIADEFAULT", "secretdefault"));
        assert!(base.expires_at.is_none());
    }

    #[test]
    fn keep_fresh_without_an_expiry_spawns_nothing() {
        let session = CloudSession::for_connection(&summary(Engine::Postgres, ConnectionAuth::Password), AuthSecrets { aws_session_token: Some("t".into()), ..AuthSecrets::default() })
            .unwrap_or_else(|| panic!("a session"));
        // Would panic outside a runtime if it tried to spawn.
        keep_fresh(session, None, async {}, |_| {});
    }
}
