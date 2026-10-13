// SOT: connection-auth, auth-method, auth-secrets, cloud-sign-in, aws-credential-source, azure-ad-mode

use crate::error::{AppError, AppResult};
use crate::model::Family;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ============================================================================
// ENTERPRISE SIGN-IN METHODS
//
// WHAT:  How a connection proves who it is: a plain password (the default and
//        what every existing connection uses), or a short-lived token minted
//        from a cloud identity (AWS RDS IAM, Azure AD, GCP Cloud SQL IAM), or a
//        MongoDB Atlas workload identity (MONGODB-AWS, MONGODB-OIDC).
// WHY:   Managed databases increasingly refuse long-lived passwords. The token
//        is derived from credentials the user already has (a profile, a
//        service-account key, an `az login`), so the connection stores *which*
//        identity to use, never a token.
// HOW:   Two halves, kept apart on purpose.
//          `ConnectionAuth`  the non-secret choice and its settings. Stored as
//                            JSON in `connections.auth_config` and echoed back
//                            to the UI whole.
//          `AuthSecrets`     keys, client secrets, tokens, service-account
//                            JSON. Write-only: sealed with AES-256-GCM, never
//                            returned, and reported to the UI as
//                            `AuthSecretFlags` (which ones are set).
//        The database *user* is the connection's own `username` for every
//        method, so there is exactly one place to type it.
// WHERE: src-tauri/src/integrations/cloud_auth.rs (turns this into credentials),
//        src-tauri/src/services/connection.rs (seals / merges the secrets),
//        src/features/connections/AuthSection.tsx
// ============================================================================

// WHAT:  Where AWS credentials come from.
// HOW:   `DefaultChain` is the AWS SDK order: environment variables, then the
//        shared config files (`AWS_PROFILE` or `default`), then web identity /
//        container credentials. `AccessKeys` keeps the key id here and the
//        secret (and optional session token) in `AuthSecrets`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "source", rename_all = "snake_case")]
#[ts(export)]
pub enum AwsCredentialSource {
    #[default]
    DefaultChain,
    Profile { name: String },
    #[serde(rename_all = "camelCase")]
    AccessKeys { access_key_id: String },
}

// WHAT:  An AWS identity: where the base credentials come from and, optionally,
//        a role to assume with them (STS AssumeRole).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AwsAccess {
    pub source: AwsCredentialSource,
    pub role_arn: Option<String>,
    pub role_session_name: Option<String>,
    pub external_id: Option<String>,
}

// WHAT:  How an Azure AD access token is obtained.
// HOW:   `Default` mirrors DefaultAzureCredential: service-principal
//        environment variables, then a managed identity, then the Azure CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AzureAdMode {
    #[default]
    Default,
    ClientSecret,
    ManagedIdentity,
    UsernamePassword,
    AccessToken,
}

// WHAT:  How a Google access token is obtained: Application Default
//        Credentials, or a service-account key stored with the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GcpAuthMode {
    #[default]
    Adc,
    ServiceAccountKey,
}

// WHAT:  Which workload identity MongoDB's OIDC mechanism asks for a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OidcProvider {
    #[default]
    Azure,
    Gcp,
    K8s,
}

impl OidcProvider {
    /// The driver's `ENVIRONMENT` mechanism property.
    pub fn environment(self) -> &'static str {
        match self {
            OidcProvider::Azure => "azure",
            OidcProvider::Gcp => "gcp",
            OidcProvider::K8s => "k8s",
        }
    }
}

// WHAT:  The sign-in method of a connection and its non-secret settings.
// WHY:   A tagged enum makes "a method with the wrong fields" unrepresentable,
//        and the TS side narrows on `method`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(tag = "method", rename_all = "snake_case")]
#[ts(export)]
pub enum ConnectionAuth {
    /// The connection's own username + password (every pre-existing connection).
    #[default]
    Password,
    /// RDS / Aurora / Redshift-style IAM database authentication: a 15-minute
    /// token presigned for `rds-db:connect` is the password. `region` blank =
    /// read it from the host name. `ca_bundle` is PEM text; blank = the bundled
    /// Amazon RDS global CA.
    #[serde(rename_all = "camelCase")]
    AwsRdsIam { region: Option<String>, credentials: AwsAccess, ca_bundle: Option<String> },
    /// Microsoft Entra ID (Azure AD) token. `scope` blank = the engine's default
    /// (`ossrdbms-aad` for PostgreSQL / MySQL, `database.windows.net` for SQL Server).
    #[serde(rename_all = "camelCase")]
    AzureAd {
        mode: AzureAdMode,
        tenant_id: Option<String>,
        client_id: Option<String>,
        /// The Entra account for `UsernamePassword` (defaults to the connection's user).
        username: Option<String>,
        scope: Option<String>,
    },
    /// Cloud SQL IAM database authentication: the Google OAuth token is the
    /// password. `impersonate_service_account` = mint the token as that account.
    #[serde(rename_all = "camelCase")]
    GcpCloudSqlIam { mode: GcpAuthMode, impersonate_service_account: Option<String> },
    /// MongoDB Atlas AWS IAM (`MONGODB-AWS`).
    #[serde(rename_all = "camelCase")]
    MongoAtlasAws { credentials: AwsAccess },
    /// MongoDB Atlas workload identity (`MONGODB-OIDC`).
    #[serde(rename_all = "camelCase")]
    MongoAtlasOidc { provider: OidcProvider, token_resource: Option<String> },
    /// AWS credentials resolved from a profile / the default chain instead of
    /// typed keys (DynamoDB).
    #[serde(rename_all = "camelCase")]
    AwsCredentials { credentials: AwsAccess },
}

fn blank(value: &Option<String>) -> bool {
    value.as_deref().map(str::trim).unwrap_or_default().is_empty()
}

fn validate_access(access: &AwsAccess) -> AppResult<()> {
    match &access.source {
        AwsCredentialSource::DefaultChain => {}
        AwsCredentialSource::Profile { name } => {
            if name.trim().is_empty() {
                return Err(AppError::invalid_input("Name the AWS profile to use."));
            }
        }
        AwsCredentialSource::AccessKeys { access_key_id } => {
            if access_key_id.trim().is_empty() {
                return Err(AppError::invalid_input("AWS access key ID is required."));
            }
        }
    }
    if let Some(arn) = access.role_arn.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        if !arn.starts_with("arn:") || arn.split(':').count() < 6 {
            return Err(AppError::invalid_input("The role to assume must be a full ARN (arn:aws:iam::<account>:role/<name>)."));
        }
    }
    Ok(())
}

impl ConnectionAuth {
    /// The `method` tag as stored and sent to the UI.
    pub fn method(&self) -> &'static str {
        match self {
            ConnectionAuth::Password => "password",
            ConnectionAuth::AwsRdsIam { .. } => "aws_rds_iam",
            ConnectionAuth::AzureAd { .. } => "azure_ad",
            ConnectionAuth::GcpCloudSqlIam { .. } => "gcp_cloud_sql_iam",
            ConnectionAuth::MongoAtlasAws { .. } => "mongo_atlas_aws",
            ConnectionAuth::MongoAtlasOidc { .. } => "mongo_atlas_oidc",
            ConnectionAuth::AwsCredentials { .. } => "aws_credentials",
        }
    }

    pub fn is_password(&self) -> bool {
        matches!(self, ConnectionAuth::Password)
    }

    // WHAT:  Whether this method exists for an adapter family.
    // WHY:   The token is only useful to a driver that knows what to do with it:
    //        PostgreSQL and MySQL take it as the password, SQL Server takes an
    //        Entra token in the login packet, MongoDB has its own mechanisms.
    //        NTLM is not offered: tiberius speaks Windows authentication only on
    //        Windows.
    // WHERE: src/lib/cloudAuth.ts restates this matrix for the method picker.
    pub fn allowed_for(&self, family: Family) -> bool {
        match self {
            ConnectionAuth::Password => true,
            ConnectionAuth::AwsRdsIam { .. } | ConnectionAuth::GcpCloudSqlIam { .. } => {
                matches!(family, Family::Postgres | Family::Mysql)
            }
            ConnectionAuth::AzureAd { .. } => matches!(family, Family::Postgres | Family::Mysql | Family::Mssql),
            ConnectionAuth::MongoAtlasAws { .. } | ConnectionAuth::MongoAtlasOidc { .. } => family == Family::Mongodb,
            ConnectionAuth::AwsCredentials { .. } => family == Family::Dynamodb,
        }
    }

    /// Whether the connection form must ask for the database user.
    pub fn needs_db_user(&self, family: Family) -> bool {
        match self {
            ConnectionAuth::Password => false,
            ConnectionAuth::AwsRdsIam { .. } | ConnectionAuth::GcpCloudSqlIam { .. } => true,
            ConnectionAuth::AzureAd { .. } => family != Family::Mssql,
            ConnectionAuth::MongoAtlasAws { .. } | ConnectionAuth::MongoAtlasOidc { .. } | ConnectionAuth::AwsCredentials { .. } => false,
        }
    }

    // WHAT:  Checks the settings that can be checked without any secret.
    // WHY:   Secrets are write-only and "blank = keep", so a missing one is
    //        reported when the token is minted, with the method's own wording.
    pub fn validate(&self, family: Family, username: Option<&str>) -> AppResult<()> {
        if !self.allowed_for(family) {
            return Err(AppError::invalid_input("This sign-in method is not available for this engine."));
        }
        if self.needs_db_user(family) && username.map(str::trim).unwrap_or_default().is_empty() {
            return Err(AppError::invalid_input("The database user is required for this sign-in method."));
        }
        match self {
            ConnectionAuth::Password => {}
            ConnectionAuth::AwsRdsIam { credentials, .. }
            | ConnectionAuth::MongoAtlasAws { credentials }
            | ConnectionAuth::AwsCredentials { credentials } => validate_access(credentials)?,
            ConnectionAuth::AzureAd { mode, tenant_id, client_id, username: aad_user, .. } => match mode {
                AzureAdMode::ClientSecret => {
                    if blank(tenant_id) || blank(client_id) {
                        return Err(AppError::invalid_input("A service principal needs the tenant ID and the client ID."));
                    }
                }
                AzureAdMode::UsernamePassword => {
                    if blank(tenant_id) || blank(client_id) {
                        return Err(AppError::invalid_input("Username and password sign-in needs the tenant ID and the client ID."));
                    }
                    if blank(aad_user) && username.map(str::trim).unwrap_or_default().is_empty() {
                        return Err(AppError::invalid_input("Enter the Entra account to sign in with."));
                    }
                }
                AzureAdMode::Default | AzureAdMode::ManagedIdentity | AzureAdMode::AccessToken => {}
            },
            ConnectionAuth::GcpCloudSqlIam { impersonate_service_account, .. } => {
                if let Some(target) = impersonate_service_account.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                    if !target.contains('@') {
                        return Err(AppError::invalid_input("The service account to impersonate is an email address."));
                    }
                }
            }
            ConnectionAuth::MongoAtlasOidc { provider, token_resource } => {
                if matches!(provider, OidcProvider::Azure | OidcProvider::Gcp) && blank(token_resource) {
                    return Err(AppError::invalid_input("Azure and GCP workload identity need the token resource (audience)."));
                }
            }
        }
        Ok(())
    }

    // WHAT:  The stored JSON (`connections.auth_config`).
    pub fn to_json(&self) -> AppResult<String> {
        serde_json::to_string(self).map_err(|e| AppError::internal(format!("could not store the sign-in method: {e}")))
    }

    // WHAT:  Reads the stored JSON back. A missing value is the default password
    //        method; one this build cannot read (written by a newer build) also
    //        falls back to it rather than making the connection unlistable.
    pub fn from_json(raw: Option<&str>) -> ConnectionAuth {
        match raw.map(str::trim).filter(|r| !r.is_empty()) {
            None => ConnectionAuth::Password,
            Some(raw) => serde_json::from_str(raw).unwrap_or_else(|err| {
                log::warn!("unreadable connection sign-in settings, falling back to password: {err}");
                ConnectionAuth::Password
            }),
        }
    }
}

// WHAT:  Every secret any sign-in method can hold. Write-only from the UI,
//        sealed at rest as one JSON document, decrypted only in memory.
// WHY:   One document, not a column per secret: the set of secrets follows the
//        method, and unused ones are dropped (`retain_for`) so switching method
//        does not leave a stale key behind.
// HOW:   A blank field on save means "keep what is stored" (`overlay`).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AuthSecrets {
    /// AWS secret access key (`AccessKeys` source).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_secret_access_key: Option<String>,
    /// Temporary-credential session token (`AccessKeys` source, and DynamoDB keys).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aws_session_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azure_client_secret: Option<String>,
    /// The Entra account's password (`UsernamePassword`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azure_password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub azure_access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gcp_service_account_json: Option<String>,
}

// WHAT:  Debug shows which secrets are set, never their values.
impl std::fmt::Debug for AuthSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthSecrets").field("set", &self.flags()).finish()
    }
}

// WHAT:  Which secrets are stored, for the UI ("•••• (unchanged)").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AuthSecretFlags {
    pub aws_secret_access_key: bool,
    pub aws_session_token: bool,
    pub azure_client_secret: bool,
    pub azure_password: bool,
    pub azure_access_token: bool,
    pub gcp_service_account_json: bool,
}

fn present(value: &Option<String>) -> bool {
    value.as_deref().is_some_and(|v| !v.trim().is_empty())
}

// WHAT:  How one typed field changes the stored one.
//        null = not touched, keep it · "" = remove it · text = replace it.
//        Whitespace alone counts as not touched.
fn overlay_field(slot: &mut Option<String>, newer: &Option<String>) {
    match newer.as_deref() {
        None => {}
        Some("") => *slot = None,
        Some(text) if text.trim().is_empty() => {}
        Some(_) => slot.clone_from(newer),
    }
}

impl AuthSecrets {
    pub fn is_empty(&self) -> bool {
        self.flags() == AuthSecretFlags::default()
    }

    pub fn flags(&self) -> AuthSecretFlags {
        AuthSecretFlags {
            aws_secret_access_key: present(&self.aws_secret_access_key),
            aws_session_token: present(&self.aws_session_token),
            azure_client_secret: present(&self.azure_client_secret),
            azure_password: present(&self.azure_password),
            azure_access_token: present(&self.azure_access_token),
            gcp_service_account_json: present(&self.gcp_service_account_json),
        }
    }

    /// Whether the form typed or cleared anything (a save that did neither needs no key).
    pub fn has_changes(&self) -> bool {
        [
            &self.aws_secret_access_key,
            &self.aws_session_token,
            &self.azure_client_secret,
            &self.azure_password,
            &self.azure_access_token,
            &self.gcp_service_account_json,
        ]
        .iter()
        .any(|field| field.as_deref().is_some_and(|text| !text.trim().is_empty() || text.is_empty()))
    }

    // WHAT:  `self` (stored) updated by `newer` (typed in the form): a field the
    //        form left null keeps the stored value, "" removes it.
    pub fn overlay(mut self, newer: &AuthSecrets) -> AuthSecrets {
        overlay_field(&mut self.aws_secret_access_key, &newer.aws_secret_access_key);
        overlay_field(&mut self.aws_session_token, &newer.aws_session_token);
        overlay_field(&mut self.azure_client_secret, &newer.azure_client_secret);
        overlay_field(&mut self.azure_password, &newer.azure_password);
        overlay_field(&mut self.azure_access_token, &newer.azure_access_token);
        overlay_field(&mut self.gcp_service_account_json, &newer.gcp_service_account_json);
        self
    }

    // WHAT:  Drops the secrets the chosen method does not use, and blanks.
    // WHY:   Changing method (or credential source) must not leave a service-
    //        account key or an AWS secret sealed on disk with nothing using it.
    pub fn retain_for(mut self, auth: &ConnectionAuth, family: Family) -> AuthSecrets {
        let uses_aws_keys = |access: &AwsAccess| matches!(access.source, AwsCredentialSource::AccessKeys { .. });
        let (aws_keys, session_token, client_secret, password, access_token, sa_json) = match auth {
            // DynamoDB / QLDB / S3 sign with typed keys under the password method;
            // the session token is the one extra secret they carry.
            ConnectionAuth::Password => (false, matches!(family, Family::Dynamodb | Family::Qldb | Family::S3), false, false, false, false),
            ConnectionAuth::AwsRdsIam { credentials, .. }
            | ConnectionAuth::MongoAtlasAws { credentials }
            | ConnectionAuth::AwsCredentials { credentials } => (uses_aws_keys(credentials), uses_aws_keys(credentials), false, false, false, false),
            ConnectionAuth::AzureAd { mode, .. } => (
                false,
                false,
                *mode == AzureAdMode::ClientSecret,
                *mode == AzureAdMode::UsernamePassword,
                *mode == AzureAdMode::AccessToken,
                false,
            ),
            ConnectionAuth::GcpCloudSqlIam { mode, .. } => (false, false, false, false, false, *mode == GcpAuthMode::ServiceAccountKey),
            ConnectionAuth::MongoAtlasOidc { .. } => (false, false, false, false, false, false),
        };
        let blank_out = |keep: bool, slot: &mut Option<String>| {
            if !keep || !present(slot) {
                *slot = None;
            }
        };
        blank_out(aws_keys, &mut self.aws_secret_access_key);
        blank_out(session_token, &mut self.aws_session_token);
        blank_out(client_secret, &mut self.azure_client_secret);
        blank_out(password, &mut self.azure_password);
        blank_out(access_token, &mut self.azure_access_token);
        blank_out(sa_json, &mut self.gcp_service_account_json);
        self
    }

    // WHAT:  The document that is sealed (`connections.auth_secret_ciphertext`).
    pub fn to_json(&self) -> AppResult<String> {
        serde_json::to_string(self).map_err(|e| AppError::internal(format!("could not store the sign-in secrets: {e}")))
    }

    pub fn from_json(raw: &str) -> AppResult<AuthSecrets> {
        serde_json::from_str(raw).map_err(|_| AppError::crypto("the stored sign-in secrets are unreadable"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rds() -> ConnectionAuth {
        ConnectionAuth::AwsRdsIam {
            region: Some("eu-west-1".into()),
            credentials: AwsAccess { source: AwsCredentialSource::AccessKeys { access_key_id: "AKIA".into() }, ..AwsAccess::default() },
            ca_bundle: None,
        }
    }

    #[test]
    fn password_is_the_default_and_round_trips_as_a_tag() {
        assert_eq!(ConnectionAuth::default(), ConnectionAuth::Password);
        assert_eq!(serde_json::to_string(&ConnectionAuth::Password).unwrap_or_default(), r#"{"method":"password"}"#);
        assert_eq!(ConnectionAuth::from_json(None), ConnectionAuth::Password);
        assert_eq!(ConnectionAuth::from_json(Some("  ")), ConnectionAuth::Password);
        assert_eq!(ConnectionAuth::from_json(Some("{not json")), ConnectionAuth::Password);
        assert_eq!(ConnectionAuth::from_json(Some(r#"{"method":"from_the_future"}"#)), ConnectionAuth::Password);
    }

    #[test]
    fn methods_use_camel_case_fields_and_snake_case_tags() {
        let json = serde_json::to_value(rds()).unwrap_or_default();
        assert_eq!(json["method"], "aws_rds_iam");
        assert_eq!(json["credentials"]["source"]["source"], "access_keys");
        assert_eq!(json["credentials"]["source"]["accessKeyId"], "AKIA");
        assert_eq!(json["caBundle"], serde_json::Value::Null);
        let back = ConnectionAuth::from_json(Some(&json.to_string()));
        assert_eq!(back, rds());
        assert_eq!(back.method(), "aws_rds_iam");
        let azure: ConnectionAuth = serde_json::from_str(r#"{"method":"azure_ad","mode":"client_secret","tenantId":"t","clientId":"c","username":null,"scope":null}"#)
            .unwrap_or_default();
        assert!(matches!(azure, ConnectionAuth::AzureAd { mode: AzureAdMode::ClientSecret, .. }));
    }

    #[test]
    fn each_method_is_offered_only_where_a_driver_can_use_it() {
        let azure = ConnectionAuth::AzureAd { mode: AzureAdMode::Default, tenant_id: None, client_id: None, username: None, scope: None };
        let gcp = ConnectionAuth::GcpCloudSqlIam { mode: GcpAuthMode::Adc, impersonate_service_account: None };
        let atlas = ConnectionAuth::MongoAtlasOidc { provider: OidcProvider::K8s, token_resource: None };
        assert!(rds().allowed_for(Family::Postgres) && rds().allowed_for(Family::Mysql));
        assert!(!rds().allowed_for(Family::Mssql) && !rds().allowed_for(Family::Mongodb));
        assert!(azure.allowed_for(Family::Mssql) && azure.allowed_for(Family::Postgres) && !azure.allowed_for(Family::Mongodb));
        assert!(gcp.allowed_for(Family::Mysql) && !gcp.allowed_for(Family::Mssql));
        assert!(atlas.allowed_for(Family::Mongodb) && !atlas.allowed_for(Family::Postgres));
        assert!(ConnectionAuth::Password.allowed_for(Family::Redis));
        let dynamo = ConnectionAuth::AwsCredentials { credentials: AwsAccess::default() };
        assert!(dynamo.allowed_for(Family::Dynamodb) && !dynamo.allowed_for(Family::S3));
    }

    #[test]
    fn validation_names_what_is_missing() {
        assert!(rds().validate(Family::Postgres, Some("iam_user")).is_ok());
        assert!(rds().validate(Family::Postgres, Some("  ")).is_err(), "IAM needs a database user");
        assert!(rds().validate(Family::Mssql, Some("u")).is_err(), "not offered for SQL Server");
        let bad_role = ConnectionAuth::AwsRdsIam {
            region: None,
            credentials: AwsAccess { role_arn: Some("role/x".into()), ..AwsAccess::default() },
            ca_bundle: None,
        };
        assert!(bad_role.validate(Family::Postgres, Some("u")).is_err());
        let profile = ConnectionAuth::AwsCredentials { credentials: AwsAccess { source: AwsCredentialSource::Profile { name: " ".into() }, ..AwsAccess::default() } };
        assert!(profile.validate(Family::Dynamodb, None).is_err());
        let sp = ConnectionAuth::AzureAd { mode: AzureAdMode::ClientSecret, tenant_id: Some("t".into()), client_id: None, username: None, scope: None };
        assert!(sp.validate(Family::Mssql, None).is_err(), "service principal needs the client id");
        let ropc = ConnectionAuth::AzureAd { mode: AzureAdMode::UsernamePassword, tenant_id: Some("t".into()), client_id: Some("c".into()), username: None, scope: None };
        assert!(ropc.validate(Family::Postgres, Some("me@corp.com")).is_ok(), "the db user doubles as the Entra account");
        assert!(ropc.validate(Family::Mssql, None).is_err());
        let managed = ConnectionAuth::AzureAd { mode: AzureAdMode::ManagedIdentity, tenant_id: None, client_id: None, username: None, scope: None };
        assert!(managed.validate(Family::Mssql, None).is_ok(), "SQL Server needs no database user with a token");
        assert!(managed.validate(Family::Postgres, None).is_err(), "PostgreSQL does");
        let oidc = ConnectionAuth::MongoAtlasOidc { provider: OidcProvider::Azure, token_resource: None };
        assert!(oidc.validate(Family::Mongodb, None).is_err());
        let k8s = ConnectionAuth::MongoAtlasOidc { provider: OidcProvider::K8s, token_resource: None };
        assert!(k8s.validate(Family::Mongodb, None).is_ok());
        let imp = ConnectionAuth::GcpCloudSqlIam { mode: GcpAuthMode::Adc, impersonate_service_account: Some("not-an-email".into()) };
        assert!(imp.validate(Family::Postgres, Some("u@p.iam")).is_err());
    }

    #[test]
    fn secrets_overlay_keeps_what_the_form_left_blank() {
        let stored = AuthSecrets {
            aws_secret_access_key: Some("old".into()),
            aws_session_token: Some("tok".into()),
            azure_password: Some("pw".into()),
            ..AuthSecrets::default()
        };
        let typed = AuthSecrets {
            aws_secret_access_key: Some("  ".into()),
            aws_session_token: Some("new".into()),
            azure_password: Some(String::new()),
            ..AuthSecrets::default()
        };
        assert!(typed.has_changes());
        let merged = stored.overlay(&typed);
        assert_eq!(merged.aws_secret_access_key.as_deref(), Some("old"), "whitespace keeps the stored secret");
        assert_eq!(merged.aws_session_token.as_deref(), Some("new"));
        assert_eq!(merged.azure_password, None, "an empty string removes it");
        let untouched = AuthSecrets { aws_secret_access_key: Some("  ".into()), ..AuthSecrets::default() };
        assert!(!untouched.has_changes() && !AuthSecrets::default().has_changes());
        assert_eq!(AuthSecrets { aws_secret_access_key: Some("old".into()), ..AuthSecrets::default() }.overlay(&untouched).aws_secret_access_key.as_deref(), Some("old"));
    }

    #[test]
    fn secrets_follow_the_method() {
        let all = AuthSecrets {
            aws_secret_access_key: Some("k".into()),
            aws_session_token: Some("t".into()),
            azure_client_secret: Some("c".into()),
            azure_password: Some("p".into()),
            azure_access_token: Some("a".into()),
            gcp_service_account_json: Some("{}".into()),
        };
        // Password under a normal engine keeps nothing; under DynamoDB it keeps the session token.
        assert!(all.clone().retain_for(&ConnectionAuth::Password, Family::Postgres).is_empty());
        let dynamo = all.clone().retain_for(&ConnectionAuth::Password, Family::Dynamodb);
        assert_eq!(dynamo.flags(), AuthSecretFlags { aws_session_token: true, ..AuthSecretFlags::default() });
        // AWS keys are kept only for the AccessKeys source.
        assert_eq!(all.clone().retain_for(&rds(), Family::Postgres).flags(), AuthSecretFlags { aws_secret_access_key: true, aws_session_token: true, ..AuthSecretFlags::default() });
        let profile = ConnectionAuth::AwsRdsIam { region: None, credentials: AwsAccess { source: AwsCredentialSource::Profile { name: "dev".into() }, ..AwsAccess::default() }, ca_bundle: None };
        assert!(all.clone().retain_for(&profile, Family::Postgres).is_empty());
        let ropc = ConnectionAuth::AzureAd { mode: AzureAdMode::UsernamePassword, tenant_id: None, client_id: None, username: None, scope: None };
        assert_eq!(all.clone().retain_for(&ropc, Family::Postgres).flags(), AuthSecretFlags { azure_password: true, ..AuthSecretFlags::default() });
        let sa = ConnectionAuth::GcpCloudSqlIam { mode: GcpAuthMode::ServiceAccountKey, impersonate_service_account: None };
        assert_eq!(all.retain_for(&sa, Family::Mysql).flags(), AuthSecretFlags { gcp_service_account_json: true, ..AuthSecretFlags::default() });
    }

    #[test]
    fn secrets_never_print_and_the_flags_carry_no_values() {
        let secrets = AuthSecrets { azure_client_secret: Some("hunter2".into()), ..AuthSecrets::default() };
        assert!(!format!("{secrets:?}").contains("hunter2"));
        let flags = serde_json::to_string(&secrets.flags()).unwrap_or_default();
        assert!(!flags.contains("hunter2") && flags.contains("\"azureClientSecret\":true"));
        // Unset fields are not written into the sealed document.
        let json = secrets.to_json().unwrap_or_default();
        assert_eq!(json, r#"{"azureClientSecret":"hunter2"}"#);
        assert_eq!(AuthSecrets::from_json(&json).unwrap_or_default(), secrets);
        assert!(AuthSecrets::from_json("nope").is_err());
    }

    #[test]
    fn an_old_client_that_sends_no_auth_secrets_deserialises_to_empty() {
        let none: AuthSecrets = serde_json::from_str("{}").unwrap_or_default();
        assert!(none.is_empty());
    }
}
