// SOT: connection-extras, tls-files, connection-uri-params, connection-tag, connection-tag-colour, connection-folder, connection-org, connection-field-issue, connection-transfer-report, ssh-config-host, credential-status

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ============================================================================
// Per-connection settings beyond host / port / user
// ============================================================================

// WHAT:  Certificate files a connection presents or trusts for TLS.
// WHY:   Self-hosted servers sign with a private CA and managed ones (RDS, Cloud
//        SQL, Azure) publish a CA bundle; verifying against it needs a file the
//        OS trust store does not have. Only the *paths* are stored: the files are
//        read when a session opens, so a rotated certificate needs no re-save.
// HOW:   Postgres and MySQL take all three; MongoDB takes the CA and a combined
//        certificate + key PEM in `cert_path`; SQL Server takes the CA only. The
//        form shows the fields only for the families that read them.
// WHERE: src-tauri/src/integrations/{postgres,mysql,mssql,mongodb}.rs
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct TlsFiles {
    /// Certificate authority (PEM) the server certificate must chain to.
    pub ca_path: Option<String>,
    /// Client certificate (PEM) presented to servers that require mutual TLS.
    pub cert_path: Option<String>,
    /// Private key (PEM) for `cert_path`.
    pub key_path: Option<String>,
}

impl TlsFiles {
    pub fn is_empty(&self) -> bool {
        [&self.ca_path, &self.cert_path, &self.key_path].iter().all(|p| p.as_deref().map(str::trim).unwrap_or_default().is_empty())
    }
}

// WHAT:  URI query parameters that would put a secret in a plain-text column.
// WHY:   `uri_params` is stored and exported in the clear (it carries
//        `authSource`, `replicaSet`, `options`…); a password pasted in a URI
//        belongs in the sealed secret instead.
pub const SECRET_URI_PARAMS: [&str; 8] =
    ["password", "passwd", "pwd", "sslpassword", "tlscertificatekeyfilepassword", "secret", "token", "sas_token"];

pub fn is_secret_param(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_URI_PARAMS.contains(&lower.as_str())
}

// WHAT:  Everything a connection carries beyond the core connect fields.
// WHY:   Kept as one nested value so adding a per-connection setting is a change
//        here, not a new field on every `ConnectionInput` / `ConnectionSummary`
//        literal in the crate.
// HOW:   `uri_params` is the query string of the URI the connection came from
//        (minus ssl / password handling the form already consumed), handed to
//        the driver that understands it. `tag_ids` / `folder_ids` are the
//        connection's organisation: a save replaces both sets with exactly what
//        the input names. `transient` is an echo (a file opened from the OS or a
//        drop, removed on quit unless saved) and is ignored on save.
//        `tunnel_origin_host` is runtime-only: the SSH tunnel sets it on the
//        copy it hands the adapter, so a driver that can verify the real server
//        name does (SQL Server); it is never stored.
// WHERE: src-tauri/src/store/connections.rs, src-tauri/src/integrations/ssh_tunnel.rs
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct ConnectionExtras {
    pub uri_params: Option<String>,
    /// MongoDB `mongodb+srv://`: the host is a DNS SRV name and takes no port.
    pub srv: bool,
    pub tls: TlsFiles,
    pub tag_ids: Vec<String>,
    pub folder_ids: Vec<String>,
    pub transient: bool,
    pub tunnel_origin_host: Option<String>,
}

impl ConnectionExtras {
    /// The `uri_params` as decoded key/value pairs, in the order written.
    pub fn uri_pairs(&self) -> Vec<(String, String)> {
        match self.uri_params.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
            Some(raw) => url::form_urlencoded::parse(raw.as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned())).collect(),
            None => Vec::new(),
        }
    }

    /// One parameter by name, case-insensitive; the last occurrence wins.
    pub fn uri_param(&self, key: &str) -> Option<String> {
        self.uri_pairs().into_iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }
}

// WHAT:  The form field a validation message belongs to.
// WHY:   The form paints the message under its field instead of one banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ConnectionField {
    Name,
    Host,
    Port,
    Database,
    Username,
    Password,
    FilePath,
    Folder,
    SshHost,
    SshPort,
    SshUser,
    SshKeyPath,
    TlsCaPath,
    TlsCertPath,
    TlsKeyPath,
    UriParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionFieldIssue {
    pub field: ConnectionField,
    pub message: String,
}

impl ConnectionFieldIssue {
    pub fn new(field: ConnectionField, message: impl Into<String>) -> ConnectionFieldIssue {
        ConnectionFieldIssue { field, message: message.into() }
    }
}

// ============================================================================
// Tags and folders
// ============================================================================

pub const MAX_CONNECTION_TAG_NAME_LEN: usize = 40;

// WHAT:  The palette a connection tag can be drawn in: 19 named colours.
// WHY:   A key, not a hex value: the UI paints it through `--tag-<key>` design
//        tokens, so a tag follows the light / dark theme and cannot smuggle in a
//        colour that fails contrast.
// WHERE: src/features/connections/connectionTags.ts, src/styles/globals.css
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ConnectionTagColor {
    Red,
    Rose,
    Pink,
    Orange,
    Amber,
    Yellow,
    Lime,
    Green,
    Emerald,
    Teal,
    Cyan,
    Sky,
    Blue,
    Indigo,
    Violet,
    Purple,
    Fuchsia,
    Slate,
    Gray,
}

impl ConnectionTagColor {
    pub const ALL: [ConnectionTagColor; 19] = [
        ConnectionTagColor::Red,
        ConnectionTagColor::Rose,
        ConnectionTagColor::Pink,
        ConnectionTagColor::Orange,
        ConnectionTagColor::Amber,
        ConnectionTagColor::Yellow,
        ConnectionTagColor::Lime,
        ConnectionTagColor::Green,
        ConnectionTagColor::Emerald,
        ConnectionTagColor::Teal,
        ConnectionTagColor::Cyan,
        ConnectionTagColor::Sky,
        ConnectionTagColor::Blue,
        ConnectionTagColor::Indigo,
        ConnectionTagColor::Violet,
        ConnectionTagColor::Purple,
        ConnectionTagColor::Fuchsia,
        ConnectionTagColor::Slate,
        ConnectionTagColor::Gray,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ConnectionTagColor::Red => "red",
            ConnectionTagColor::Rose => "rose",
            ConnectionTagColor::Pink => "pink",
            ConnectionTagColor::Orange => "orange",
            ConnectionTagColor::Amber => "amber",
            ConnectionTagColor::Yellow => "yellow",
            ConnectionTagColor::Lime => "lime",
            ConnectionTagColor::Green => "green",
            ConnectionTagColor::Emerald => "emerald",
            ConnectionTagColor::Teal => "teal",
            ConnectionTagColor::Cyan => "cyan",
            ConnectionTagColor::Sky => "sky",
            ConnectionTagColor::Blue => "blue",
            ConnectionTagColor::Indigo => "indigo",
            ConnectionTagColor::Violet => "violet",
            ConnectionTagColor::Purple => "purple",
            ConnectionTagColor::Fuchsia => "fuchsia",
            ConnectionTagColor::Slate => "slate",
            ConnectionTagColor::Gray => "gray",
        }
    }

    pub fn parse(raw: &str) -> Option<ConnectionTagColor> {
        ConnectionTagColor::ALL.into_iter().find(|c| c.as_str() == raw)
    }
}

// WHAT:  A named, coloured label attachable to any number of connections.
// WHERE: src-tauri/src/store/connection_org.rs (connection_tags)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionTag {
    pub id: String,
    pub name: String,
    pub color: ConnectionTagColor,
    /// One of the five seeded labels (Production, Staging, Development, Testing, Local).
    pub is_preset: bool,
    pub sort_order: i32,
    /// How many connections carry it.
    pub connection_count: u32,
}

// WHAT:  A named group of connections. A connection may sit in several.
// WHY:   An entity rather than a free-text column so an empty folder can exist,
//        a rename is one write, and deleting it never touches a connection.
// WHERE: src-tauri/src/store/connection_org.rs (connection_folders)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionFolder {
    pub id: String,
    pub name: String,
    pub sort_order: i32,
    pub connection_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionOrg {
    pub tags: Vec<ConnectionTag>,
    pub folders: Vec<ConnectionFolder>,
}

// ============================================================================
// Export / import
// ============================================================================

// WHAT:  Which file format an import came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ConnectionImportSource {
    DbFree,
    DbPro,
    Navicat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionSkipped {
    pub label: String,
    pub reason: String,
}

// WHAT:  What an import did: how many were created, which were left out and why,
//        and the caveats worth showing (passwords not in the file, keys skipped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionImportReport {
    pub source: ConnectionImportSource,
    pub imported: u32,
    pub imported_names: Vec<String>,
    pub skipped: Vec<ConnectionSkipped>,
    pub warnings: Vec<String>,
    pub includes_passwords: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionExportReport {
    pub path: String,
    pub count: u32,
    pub includes_passwords: bool,
}

// ============================================================================
// ~/.ssh/config and the master key
// ============================================================================

// WHAT:  One concrete `Host` entry of an OpenSSH config, ready to fill the tunnel form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SshConfigHost {
    /// The `Host` alias.
    pub alias: String,
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
}

// WHAT:  Pushed to the UI when the OS hands the app a database file (double-click,
//        "Open with", a second launch): the transient connection that now exists.
// WHERE: src-tauri/src/open_files.rs (emits "connection:opened")
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionOpenedEvent {
    pub connection_id: String,
    pub name: String,
}

// WHAT:  Where the master key lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum KeyStorage {
    /// macOS Keychain / Windows Credential Manager / Secret Service.
    Keychain,
    /// A 0600 file in the app data directory: no OS keychain was available.
    File,
    /// Not persisted (tests).
    Memory,
}

// WHAT:  Whether saved passwords can be decrypted, and how the key is kept.
// WHY:   A missing keychain entry used to mint a new key silently, after which
//        every saved password failed to decrypt with no explanation. `lost` names
//        that state; `storage == File` is the visible warning for machines
//        without a keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CredentialStatus {
    pub storage: KeyStorage,
    /// Path of the key file when `storage` is `file`.
    pub key_path: Option<String>,
    /// The key is gone but sealed secrets exist: they can no longer be opened.
    pub lost: bool,
    /// How many sealed secrets that affects.
    pub lost_secrets: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_colours_round_trip_and_are_nineteen() {
        assert_eq!(ConnectionTagColor::ALL.len(), 19);
        for c in ConnectionTagColor::ALL {
            assert_eq!(ConnectionTagColor::parse(c.as_str()), Some(c));
            assert_eq!(serde_json::to_string(&c).unwrap_or_default(), format!("\"{}\"", c.as_str()));
        }
        assert_eq!(ConnectionTagColor::parse("magenta"), None);
    }

    #[test]
    fn uri_params_decode_and_look_up_case_insensitively() {
        let extras = ConnectionExtras { uri_params: Some("authSource=admin&replicaSet=rs%200&authsource=other".into()), ..ConnectionExtras::default() };
        assert_eq!(extras.uri_param("authSource").as_deref(), Some("other"), "last one wins");
        assert_eq!(extras.uri_param("replicaset").as_deref(), Some("rs 0"));
        assert_eq!(extras.uri_param("missing"), None);
        assert!(ConnectionExtras::default().uri_pairs().is_empty());
    }

    #[test]
    fn secret_param_names_are_recognised() {
        assert!(is_secret_param("Password"));
        assert!(is_secret_param("tlsCertificateKeyFilePassword"));
        assert!(!is_secret_param("authSource"));
    }

    #[test]
    fn empty_tls_files_are_empty() {
        assert!(TlsFiles::default().is_empty());
        assert!(TlsFiles { ca_path: Some("  ".into()), ..TlsFiles::default() }.is_empty());
        assert!(!TlsFiles { ca_path: Some("/ca.pem".into()), ..TlsFiles::default() }.is_empty());
    }
}
