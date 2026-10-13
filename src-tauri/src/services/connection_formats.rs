// SOT: connection-file-formats, db-free-connections-json, db-pro-import, navicat-ncx-import, connection-uri-parser

use crate::error::{AppError, AppResult};
use crate::model::connection_org::{is_secret_param, ConnectionExtras, TlsFiles};
use crate::model::{
    AuthSecrets, ConnectionAuth, ConnectionColor, ConnectionImportSource, ConnectionInput, ConnectionSkipped, ConnectionTagColor, Engine,
    Environment, FormKind, SshAuth, SshTunnel, SslMode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

// ============================================================================
// WHAT:  The three connection file formats DB Free reads, and the one it writes.
//        - DB Free JSON  `db-free-connections` v1 (what Export writes)
//        - DB Pro        `.dbpro` / `.json`, version "1.0"
//        - Navicat       `.ncx` XML
// WHY:   Moving a team's connection list should not mean retyping it. The parsers
//        are pure (text in, connections out) so they are testable without a
//        store, and nothing is saved until the whole file has been understood.
// HOW:   Every format lands in `ImportedConnection`: a `ConnectionInput` (with
//        the password / SSH secret when the file carried them) plus the folder
//        and tag names it asked for. Connections the app cannot represent are
//        listed as skipped with a reason rather than aborting the file. A file
//        that is not valid at all fails as a whole, before anything is written.
//        XML is parsed with entity expansion off and a DOCTYPE is refused, so a
//        hostile `.ncx` cannot pull in files or blow up memory.
// WHERE: src-tauri/src/services/connection_transfer.rs (reads, writes, saves)
// ============================================================================

pub const FORMAT_NAME: &str = "db-free-connections";
pub const FORMAT_VERSION: u32 = 1;
const MAX_IMPORT_BYTES: usize = 5 * 1024 * 1024;
const MAX_CONNECTIONS: usize = 1000;
const MAX_FIELD_LEN: usize = 4096;

// ----------------------------------------------------------------------------
// DB Free's own format
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbFreeFile {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub exported_at: String,
    #[serde(default)]
    pub includes_passwords: bool,
    pub connections: Vec<DbFreeConnection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbFreeTag {
    pub name: String,
    pub color: ConnectionTagColor,
}

// WHAT:  One connection in the DB Free file. Secrets appear only when the export
//        asked for them (`includes_passwords`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbFreeConnection {
    pub name: String,
    pub engine: Engine,
    #[serde(default = "no_environment")]
    pub environment: Environment,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default = "prefer_ssl")]
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub ssh: SshTunnel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_secret: Option<String>,
    #[serde(default)]
    pub color: Option<ConnectionColor>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub uri_params: Option<String>,
    #[serde(default)]
    pub srv: bool,
    #[serde(default)]
    pub tls: TlsFiles,
    /// How the connection signs in (password, cloud identity…); the settings, never the secrets.
    #[serde(default)]
    pub auth: ConnectionAuth,
    /// Keys, client secrets and tokens of the sign-in method: written only when the export opted in.
    #[serde(default, skip_serializing_if = "AuthSecrets::is_empty")]
    pub auth_secrets: AuthSecrets,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub tags: Vec<DbFreeTag>,
}

fn no_environment() -> Environment {
    Environment::None
}

fn prefer_ssl() -> SslMode {
    SslMode::Prefer
}

// ----------------------------------------------------------------------------
// Result of parsing
// ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ImportedConnection {
    pub input: ConnectionInput,
    pub folder_names: Vec<String>,
    pub tags: Vec<(String, ConnectionTagColor)>,
}

#[derive(Debug, Clone)]
pub struct ParsedImport {
    pub source: ConnectionImportSource,
    pub includes_passwords: bool,
    pub connections: Vec<ImportedConnection>,
    pub skipped: Vec<ConnectionSkipped>,
    pub warnings: Vec<String>,
}

// WHAT:  Parses a connection file by its name and content.
// HOW:   `.ncx` is Navicat; `.dbpro` / `.json` are JSON, told apart by the
//        `format` marker DB Free writes (anything else is read as DB Pro).
pub fn parse_import(content: &str, file_name: &str) -> AppResult<ParsedImport> {
    if content.len() > MAX_IMPORT_BYTES {
        return Err(AppError::invalid_input("Connection files must be 5 MB or smaller."));
    }
    if content.trim().is_empty() {
        return Err(AppError::invalid_input("The selected file is empty."));
    }
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".ncx") {
        return parse_navicat(content);
    }
    if lower.ends_with(".dbpro") || lower.ends_with(".json") {
        let json: Json = serde_json::from_str(content)
            .map_err(|_| AppError::invalid_input("Invalid JSON: choose a DB Free or DB Pro export."))?;
        return if json.get("format").and_then(Json::as_str) == Some(FORMAT_NAME) { parse_db_free(json) } else { parse_db_pro(&json) };
    }
    Err(AppError::invalid_input("Unsupported file type. Choose a .json, .dbpro or Navicat .ncx file."))
}

fn too_many(count: usize) -> AppResult<()> {
    if count == 0 {
        return Err(AppError::invalid_input("The file contains no connections."));
    }
    if count > MAX_CONNECTIONS {
        return Err(AppError::invalid_input(format!("Import files can contain at most {MAX_CONNECTIONS} connections.")));
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// DB Free JSON
// ----------------------------------------------------------------------------

fn parse_db_free(json: Json) -> AppResult<ParsedImport> {
    let file: DbFreeFile = serde_json::from_value(json)
        .map_err(|e| AppError::invalid_input(format!("This is not a valid DB Free connections file: {e}")))?;
    if file.version > FORMAT_VERSION {
        return Err(AppError::invalid_input(format!(
            "This file was written by a newer DB Free (format version {}). Update the app to read it.",
            file.version
        )));
    }
    too_many(file.connections.len())?;
    let mut connections = Vec::new();
    for entry in file.connections {
        let folder_names = entry.folders.clone();
        let tags = entry.tags.iter().map(|t| (t.name.clone(), t.color)).collect();
        let input = ConnectionInput {
            name: entry.name,
            engine: entry.engine,
            environment: entry.environment,
            read_only: entry.read_only,
            host: entry.host,
            port: entry.port,
            database: entry.database,
            username: entry.username,
            password: if file.includes_passwords { entry.password } else { None },
            file_path: entry.file_path,
            ssl_mode: entry.ssl_mode,
            ssh: entry.ssh,
            ssh_secret: if file.includes_passwords { entry.ssh_secret } else { None },
            folder: None,
            color: entry.color,
            favorite: entry.favorite,
            extra: ConnectionExtras { uri_params: entry.uri_params, srv: entry.srv, tls: entry.tls, ..ConnectionExtras::default() },
            auth: entry.auth,
            auth_secrets: if file.includes_passwords { entry.auth_secrets } else { AuthSecrets::default() },
        };
        connections.push(ImportedConnection { input, folder_names, tags });
    }
    let warnings = if file.includes_passwords {
        vec!["This file includes passwords. They are sealed with this machine's key on import.".to_string()]
    } else {
        vec!["This export does not include passwords. Add them after import.".to_string()]
    };
    Ok(ParsedImport { source: ConnectionImportSource::DbFree, includes_passwords: file.includes_passwords, connections, skipped: Vec::new(), warnings })
}

// ----------------------------------------------------------------------------
// DB Pro (.dbpro / .json, version "1.0")
// ----------------------------------------------------------------------------

fn object_string(object: &Json, field: &str, required: bool) -> AppResult<Option<String>> {
    match object.get(field) {
        None | Some(Json::Null) => {
            if required {
                Err(AppError::invalid_input(format!("{field} is required")))
            } else {
                Ok(None)
            }
        }
        Some(Json::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return if required { Err(AppError::invalid_input(format!("{field} is required"))) } else { Ok(None) };
            }
            if trimmed.chars().count() > MAX_FIELD_LEN {
                return Err(AppError::invalid_input(format!("{field} is too long")));
            }
            Ok(Some(trimmed.to_string()))
        }
        // DB Pro writes ports as strings but a hand-written file may use a number.
        Some(Json::Number(n)) => Ok(Some(n.to_string())),
        Some(_) => Err(AppError::invalid_input(format!("{field} must be a string"))),
    }
}

// WHAT:  A DB Pro `database` value → the DB Free engine, or None when DB Free has no such engine.
fn db_pro_engine(database: &str) -> Option<Engine> {
    Some(match database.to_ascii_lowercase().as_str() {
        "postgresql" | "postgres" => Engine::Postgres,
        "mysql" => Engine::Mysql,
        "mariadb" | "maria" => Engine::Mariadb,
        "mssql" | "sqlserver" => Engine::Mssql,
        "mongodb" => Engine::Mongodb,
        "sqlite" => Engine::Sqlite,
        "libsql" | "turso" => Engine::Libsql,
        "clickhouse" => Engine::Clickhouse,
        "redis" => Engine::Redis,
        "dynamodb" => Engine::Dynamodb,
        "supabase" => Engine::Supabase,
        "planetscale" | "planetscale-vitess" => Engine::Planetscale,
        "neon" => Engine::Neon,
        "cloudflare-d1" => Engine::CloudflareD1,
        "valtown" => Engine::ValTown,
        _ => return None,
    })
}

fn db_pro_ssl(raw: Option<&str>, engine: Engine) -> SslMode {
    match raw.map(str::to_ascii_lowercase).as_deref() {
        Some("disabled" | "disable" | "false") => SslMode::Disable,
        Some("enabled" | "require" | "required" | "true") => SslMode::Require,
        Some("verify-ca" | "verify_ca") => SslMode::VerifyCa,
        Some("verify-full" | "verify_full") => SslMode::VerifyFull,
        _ => default_ssl(engine),
    }
}

// WHAT:  TLS posture of an engine when nothing says otherwise: hosted services require it.
pub fn default_ssl(engine: Engine) -> SslMode {
    match engine {
        Engine::Supabase | Engine::Neon => SslMode::Require,
        Engine::Planetscale | Engine::PlanetscalePostgres => SslMode::VerifyFull,
        _ => SslMode::Prefer,
    }
}

fn parse_db_pro(json: &Json) -> AppResult<ParsedImport> {
    let list = json
        .get("connections")
        .and_then(Json::as_array)
        .ok_or_else(|| AppError::invalid_input("Invalid format: expected an object with a \"connections\" array."))?;
    too_many(list.len())?;
    let includes_passwords = json.get("includesPasswords").and_then(Json::as_bool).unwrap_or(false);
    let mut connections = Vec::new();
    let mut skipped = Vec::new();
    let mut pasted_keys = 0usize;
    let mut cloud_auth = 0usize;
    for (index, raw) in list.iter().enumerate() {
        let position = format!("Connection {}", index + 1);
        if !raw.is_object() {
            return Err(AppError::invalid_input(format!("{position} must be an object")));
        }
        let wrap = |e: AppError| AppError::invalid_input(format!("{position}: {}", e.message()));
        let label = object_string(raw, "label", true).map_err(wrap)?.unwrap_or_default();
        let database = object_string(raw, "database", true).map_err(wrap)?.unwrap_or_default();
        let Some(mut engine) = db_pro_engine(&database) else {
            skipped.push(ConnectionSkipped { label, reason: format!("Database type \"{database}\" is not supported") });
            continue;
        };
        let connection_type = object_string(raw, "connectionType", false).map_err(wrap)?;
        let uri = object_string(raw, "connectionUri", false).map_err(wrap)?;
        let mut input = blank_input(engine, &label);
        let mut password = if includes_passwords { object_string(raw, "password", false).map_err(wrap)? } else { None };
        let uses_uri = connection_type.as_deref() == Some("uri") || (connection_type.is_none() && uri.is_some());
        if let (true, Some(uri)) = (uses_uri, uri.as_deref()) {
            if engine.form() == FormKind::File {
                // A file database's "URI" is the path of the file.
                input.file_path = Some(uri.strip_prefix("sqlite://").or_else(|| uri.strip_prefix("file://")).unwrap_or(uri).to_string());
            } else {
                let Some(parts) = parse_uri(uri) else {
                    return Err(AppError::invalid_input(format!("{position}: connectionUri is not a valid URI")));
                };
                // A `postgresql://` string on *.psdb.cloud is PlanetScale's Postgres product.
                if engine == Engine::Planetscale && parts.scheme.starts_with("postgres") {
                    engine = Engine::PlanetscalePostgres;
                    input = blank_input(engine, &label);
                }
                apply_uri(&mut input, &parts);
                if includes_passwords && password.is_none() {
                    password = parts.password;
                }
            }
        } else {
            input.host = object_string(raw, "host", false).map_err(wrap)?;
            input.port = object_string(raw, "port", false).map_err(wrap)?.and_then(|p| p.parse().ok());
            input.database = object_string(raw, "databaseName", false).map_err(wrap)?;
            input.username = object_string(raw, "username", false).map_err(wrap)?;
            if engine.form() == FormKind::File {
                input.file_path = uri.clone().or(input.host.take());
            }
        }
        if input.username.is_none() {
            input.username = object_string(raw, "username", false).map_err(wrap)?;
        }
        input.password = password;
        if let Some(ssl) = object_string(raw, "ssl", false).map_err(wrap)? {
            input.ssl_mode = db_pro_ssl(Some(&ssl), engine);
        } else if !uses_uri {
            input.ssl_mode = default_ssl(engine);
        }
        if object_string(raw, "sshEnabled", false).map_err(wrap)?.as_deref() == Some("true") && engine.form() == FormKind::Server {
            input.ssh.enabled = true;
            input.ssh.host = object_string(raw, "sshHost", false).map_err(wrap)?;
            input.ssh.port = object_string(raw, "sshPort", false).map_err(wrap)?.and_then(|p| p.parse().ok()).unwrap_or(22);
            input.ssh.user = object_string(raw, "sshUsername", false).map_err(wrap)?;
            let key_auth = object_string(raw, "sshAuthMethod", false).map_err(wrap)?.as_deref() == Some("key");
            if key_auth {
                input.ssh.auth = SshAuth::PrivateKey;
                if raw.get("sshPrivateKey").is_some_and(|k| k.as_str().is_some_and(|s| !s.trim().is_empty())) {
                    pasted_keys += 1;
                }
                if includes_passwords {
                    input.ssh_secret = object_string(raw, "sshPassphrase", false).map_err(wrap)?;
                }
            } else if includes_passwords {
                input.ssh_secret = object_string(raw, "sshPassword", false).map_err(wrap)?;
            }
        }
        if object_string(raw, "authType", false).map_err(wrap)?.is_some_and(|t| t != "password") {
            cloud_auth += 1;
        }
        connections.push(ImportedConnection { input, folder_names: Vec::new(), tags: Vec::new() });
    }
    if connections.is_empty() {
        let reasons: Vec<&str> = skipped.iter().map(|s| s.reason.as_str()).collect();
        return Err(AppError::invalid_input(format!("No supported connections found. {}", reasons.join("; "))));
    }
    let mut warnings = vec![if includes_passwords {
        "This DB Pro export includes credentials. They are sealed with this machine's key on import.".to_string()
    } else {
        "This export does not include passwords. Add credentials after import.".to_string()
    }];
    if pasted_keys > 0 {
        warnings.push(format!(
            "{pasted_keys} connection{} used a pasted SSH private key. DB Free reads key files: choose the key file in each tunnel.",
            if pasted_keys == 1 { "" } else { "s" }
        ));
    }
    if cloud_auth > 0 {
        warnings.push(format!(
            "{cloud_auth} connection{} used a cloud sign-in method (IAM / Azure AD). Set it up again after import.",
            if cloud_auth == 1 { "" } else { "s" }
        ));
    }
    if !skipped.is_empty() {
        warnings.push(format!("Skipped {} connection{} DB Free has no engine for.", skipped.len(), if skipped.len() == 1 { "" } else { "s" }));
    }
    Ok(ParsedImport { source: ConnectionImportSource::DbPro, includes_passwords, connections, skipped, warnings })
}

// ----------------------------------------------------------------------------
// Navicat .ncx
// ----------------------------------------------------------------------------

fn navicat_engine(conn_type: &str) -> Option<Engine> {
    Some(match conn_type.to_ascii_uppercase().as_str() {
        "MYSQL" => Engine::Mysql,
        "MARIADB" => Engine::Mariadb,
        "POSTGRESQL" | "POSTGRES" => Engine::Postgres,
        "MSSQL" | "SQLSERVER" => Engine::Mssql,
        "MONGODB" => Engine::Mongodb,
        "SQLITE" => Engine::Sqlite,
        "REDIS" => Engine::Redis,
        _ => return None,
    })
}

// WHAT:  Every `<Connection>` of an `.ncx` as attribute / child-element maps.
// HOW:   A DOCTYPE is refused outright and any entity other than the five
//        predefined ones fails the attribute unescape, so no entity is ever
//        expanded. Navicat writes its settings as attributes; child elements with
//        text are read too for exports that use them.
fn ncx_connections(content: &str) -> AppResult<Vec<std::collections::HashMap<String, String>>> {
    use quick_xml::events::{BytesStart, Event};
    use quick_xml::{Reader, XmlVersion};
    let upper = content.to_ascii_uppercase();
    if upper.contains("<!DOCTYPE") || upper.contains("<!ENTITY") {
        return Err(AppError::invalid_input("Navicat files cannot contain DOCTYPE declarations or XML entities."));
    }
    let invalid = |detail: &str| AppError::invalid_input(format!("Invalid Navicat .ncx file: {detail}"));
    let mut reader = Reader::from_str(content);
    let mut out: Vec<std::collections::HashMap<String, String>> = Vec::new();
    let mut open: Option<std::collections::HashMap<String, String>> = None;
    let mut child: Option<String> = None;
    let mut saw_root = false;
    let attributes = |start: &BytesStart<'_>| -> AppResult<std::collections::HashMap<String, String>> {
        let mut map = std::collections::HashMap::new();
        for attribute in start.attributes() {
            let attribute = attribute.map_err(|_| invalid("malformed attribute"))?;
            let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
            let value = attribute.normalized_value(XmlVersion::Implicit1_0).map_err(|_| invalid("unsupported entity"))?;
            map.insert(key, value.trim().to_string());
        }
        Ok(map)
    };
    loop {
        match reader.read_event().map_err(|e| invalid(&e.to_string()))? {
            Event::DocType(_) => return Err(AppError::invalid_input("Navicat files cannot contain DOCTYPE declarations or XML entities.")),
            Event::GeneralRef(_) => return Err(AppError::invalid_input("Navicat files cannot contain XML entities.")),
            Event::Start(start) => {
                let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                if name == "Connections" {
                    saw_root = true;
                } else if name == "Connection" && open.is_none() {
                    open = Some(attributes(&start)?);
                } else if open.is_some() {
                    child = Some(name);
                }
            }
            Event::Empty(start) => {
                if start.name().as_ref() == b"Connection" && open.is_none() {
                    out.push(attributes(&start)?);
                }
            }
            Event::Text(text) => {
                if let (Some(map), Some(key)) = (open.as_mut(), child.as_ref()) {
                    let value = text.xml10_content().map_err(|_| invalid("unreadable text"))?;
                    map.insert(key.clone(), value.trim().to_string());
                }
            }
            Event::End(end) => {
                let name = String::from_utf8_lossy(end.name().as_ref()).into_owned();
                if name == "Connection" {
                    if let Some(map) = open.take() {
                        out.push(map);
                    }
                } else {
                    child = None;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !saw_root {
        return Err(AppError::invalid_input("Invalid Navicat .ncx file: expected a Connections root element."));
    }
    Ok(out)
}

fn navicat_flag(map: &std::collections::HashMap<String, String>, key: &str) -> bool {
    map.get(key).is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
}

fn navicat_value<'a>(map: &'a std::collections::HashMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| map.get(*k).map(String::as_str).filter(|v| !v.is_empty()))
}

fn parse_navicat(content: &str) -> AppResult<ParsedImport> {
    let list = ncx_connections(content)?;
    too_many(list.len())?;
    let mut connections = Vec::new();
    let mut skipped = Vec::new();
    for (index, map) in list.iter().enumerate() {
        let label = navicat_value(map, &["ConnectionName"]).map_or_else(|| format!("Navicat connection {}", index + 1), str::to_string);
        let Some(conn_type) = navicat_value(map, &["ConnType"]) else {
            return Err(AppError::invalid_input(format!("Connection {} ({label}) has no ConnType.", index + 1)));
        };
        let Some(engine) = navicat_engine(conn_type) else {
            skipped.push(ConnectionSkipped { label, reason: format!("Unsupported Navicat database type: {}", conn_type.to_ascii_uppercase()) });
            continue;
        };
        let mut input = blank_input(engine, &label);
        if engine.form() == FormKind::File {
            let Some(path) = navicat_value(map, &["DatabaseFileName", "Database"]) else {
                return Err(AppError::invalid_input(format!("Connection {} ({label}) has no database file.", index + 1)));
            };
            input.file_path = Some(path.to_string());
        } else {
            let Some(host) = navicat_value(map, &["Host"]) else {
                return Err(AppError::invalid_input(format!("Connection {} ({label}) has no host.", index + 1)));
            };
            input.host = Some(host.to_string());
            input.port = navicat_value(map, &["Port"]).and_then(|p| p.parse().ok());
            input.database = navicat_value(map, &["Database", "InitialDatabase"]).map(str::to_string);
            input.username = navicat_value(map, &["UserName", "Username"]).map(str::to_string);
            input.ssl_mode = if navicat_flag(map, "SSL") { SslMode::Require } else { SslMode::Disable };
            if navicat_flag(map, "SSH") {
                input.ssh.enabled = true;
                input.ssh.host = navicat_value(map, &["SSH_Host"]).map(str::to_string);
                input.ssh.port = navicat_value(map, &["SSH_Port"]).and_then(|p| p.parse().ok()).unwrap_or(22);
                input.ssh.user = navicat_value(map, &["SSH_UserName"]).map(str::to_string);
                if navicat_value(map, &["SSH_AuthenMethod"]).is_some_and(|m| m.eq_ignore_ascii_case("publickey")) {
                    input.ssh.auth = SshAuth::PrivateKey;
                }
            }
        }
        connections.push(ImportedConnection { input, folder_names: Vec::new(), tags: Vec::new() });
    }
    if connections.is_empty() {
        let reasons: Vec<&str> = skipped.iter().map(|s| s.reason.as_str()).collect();
        return Err(AppError::invalid_input(format!("No supported connections found. {}", reasons.join("; "))));
    }
    let mut warnings = vec!["Navicat passwords, SSH passwords and private keys are not imported. Add credentials after import.".to_string()];
    if !skipped.is_empty() {
        warnings.push(format!("Skipped {} unsupported Navicat connection{}.", skipped.len(), if skipped.len() == 1 { "" } else { "s" }));
    }
    Ok(ParsedImport { source: ConnectionImportSource::Navicat, includes_passwords: false, connections, skipped, warnings })
}

// ----------------------------------------------------------------------------
// URI parsing (DB Pro "uri" connections)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UriParts {
    pub scheme: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    /// The query string minus TLS switches and anything that looks like a secret.
    pub params: Option<String>,
    pub ssl: Option<SslMode>,
    pub srv: bool,
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        if byte == b'%' {
            if let (Some(high), Some(low)) = (bytes.get(i + 1).and_then(|b| hex_value(*b)), bytes.get(i + 2).and_then(|b| hex_value(*b))) {
                out.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        out.push(byte);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// WHAT:  `scheme://user:pass@host:port/db?params` → its parts, without `url::Url`.
// WHY:   MongoDB URIs carry a host list (`h1:27017,h2:27017`) that a strict URL
//        parser rejects; credentials are percent-decoded; nothing here connects.
pub fn parse_uri(raw: &str) -> Option<UriParts> {
    let text = raw.trim();
    let (scheme, rest) = text.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }
    let (before_query, query) = match rest.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (rest, None),
    };
    let (authority, path) = before_query.split_once('/').unwrap_or((before_query, ""));
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let mut parts = UriParts { scheme: scheme.to_ascii_lowercase(), srv: scheme.eq_ignore_ascii_case("mongodb+srv"), ..UriParts::default() };
    if let Some(info) = userinfo {
        let (user, password) = match info.split_once(':') {
            Some((u, p)) => (u, Some(p)),
            None => (info, None),
        };
        parts.username = Some(percent_decode(user)).filter(|u| !u.is_empty());
        parts.password = password.map(percent_decode).filter(|p| !p.is_empty());
    }
    if hostport.contains(',') {
        parts.host = Some(hostport.to_string());
    } else if let Some(rest) = hostport.strip_prefix('[') {
        let (v6, tail) = rest.split_once(']')?;
        parts.host = Some(format!("[{v6}]"));
        parts.port = tail.strip_prefix(':').and_then(|p| p.parse().ok());
    } else {
        match hostport.rsplit_once(':') {
            Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => {
                parts.host = Some(host.to_string());
                parts.port = port.parse().ok();
            }
            _ => parts.host = Some(hostport.to_string()),
        }
    }
    parts.host = parts.host.filter(|h| !h.is_empty());
    let database = percent_decode(path.split('/').next().unwrap_or_default());
    parts.database = Some(database).filter(|d| !d.is_empty());
    if let Some(query) = query {
        let mut kept: Vec<&str> = Vec::new();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = value.to_ascii_lowercase();
            match key.to_ascii_lowercase().as_str() {
                "sslmode" | "ssl" | "tls" | "secure" => {
                    parts.ssl = match value.as_str() {
                        "require" | "true" | "1" => Some(SslMode::Require),
                        "verify-ca" | "verify_ca" => Some(SslMode::VerifyCa),
                        "verify-full" | "verify_full" => Some(SslMode::VerifyFull),
                        "disable" | "false" | "0" => Some(SslMode::Disable),
                        "prefer" | "allow" => Some(SslMode::Prefer),
                        _ => parts.ssl,
                    };
                }
                key if is_secret_param(key) => {}
                _ => kept.push(pair),
            }
        }
        if !kept.is_empty() {
            parts.params = Some(kept.join("&"));
        }
    }
    if parts.srv && parts.ssl.is_none() {
        parts.ssl = Some(SslMode::Require);
    }
    Some(parts)
}

// WHAT:  Copies parsed URI parts onto an input, leaving unset parts at the engine default.
fn apply_uri(input: &mut ConnectionInput, parts: &UriParts) {
    input.host = parts.host.clone().or(input.host.take());
    input.port = parts.port.or(input.port);
    input.database = parts.database.clone().or(input.database.take());
    input.username = parts.username.clone().or(input.username.take());
    if let Some(ssl) = parts.ssl {
        input.ssl_mode = ssl;
    }
    input.extra.uri_params = parts.params.clone();
    input.extra.srv = parts.srv;
    if parts.srv {
        input.port = None;
    }
}

// WHAT:  A connection input with only the engine's defaults filled in.
pub fn blank_input(engine: Engine, name: &str) -> ConnectionInput {
    ConnectionInput {
        name: name.to_string(),
        engine,
        environment: Environment::None,
        read_only: false,
        host: None,
        port: engine.default_port(),
        database: None,
        username: None,
        password: None,
        file_path: None,
        ssl_mode: default_ssl(engine),
        ssh: SshTunnel::default(),
        ssh_secret: None,
        folder: None,
        color: None,
        favorite: false,
        auth: Default::default(),
        auth_secrets: Default::default(),
        extra: ConnectionExtras::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DB_PRO_EXAMPLE: &str = r#"{
      "version": "1.0",
      "includesPasswords": false,
      "connections": [
        { "label": "Local Postgres", "database": "postgresql", "connectionType": "standard",
          "host": "localhost", "port": "5432", "databaseName": "app", "username": "postgres" },
        { "label": "Atlas", "database": "mongodb", "connectionType": "uri",
          "connectionUri": "mongodb+srv://me:secret@cluster0.abc.mongodb.net/shop?retryWrites=true&w=majority" },
        { "label": "Weird", "database": "oracle9", "connectionType": "standard", "host": "x" }
      ]
    }"#;

    #[test]
    fn db_pro_standard_and_uri_connections_import_and_unknown_engines_are_skipped() {
        let parsed = parse_import(DB_PRO_EXAMPLE, "export.dbpro").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.source, ConnectionImportSource::DbPro);
        assert!(!parsed.includes_passwords);
        assert_eq!(parsed.connections.len(), 2);
        assert_eq!(parsed.skipped.len(), 1);
        assert!(parsed.skipped.first().is_some_and(|s| s.label == "Weird" && s.reason.contains("oracle9")));
        let pg = &parsed.connections[0].input;
        assert_eq!((pg.engine, pg.host.as_deref(), pg.port, pg.database.as_deref(), pg.username.as_deref()), (Engine::Postgres, Some("localhost"), Some(5432), Some("app"), Some("postgres")));
        let mongo = &parsed.connections[1].input;
        assert_eq!(mongo.engine, Engine::Mongodb);
        assert!(mongo.extra.srv);
        assert_eq!(mongo.host.as_deref(), Some("cluster0.abc.mongodb.net"));
        assert_eq!(mongo.database.as_deref(), Some("shop"));
        assert_eq!(mongo.extra.uri_params.as_deref(), Some("retryWrites=true&w=majority"));
        assert_eq!(mongo.password, None, "a file without passwords never yields one, even from a URI");
        assert!(parsed.warnings.iter().any(|w| w.contains("does not include passwords")));
    }

    #[test]
    fn db_pro_sqlite_uri_is_a_file_path() {
        let text = r#"{ "version": "1.0", "connections": [
            { "label": "notes", "database": "sqlite", "connectionType": "uri", "connectionUri": "/Users/me/notes.sqlite" } ] }"#;
        let parsed = parse_import(text, "x.dbpro").unwrap_or_else(|e| panic!("{e}"));
        let input = &parsed.connections[0].input;
        assert_eq!((input.engine, input.file_path.as_deref(), input.host.as_deref()), (Engine::Sqlite, Some("/Users/me/notes.sqlite"), None));
    }

    #[test]
    fn db_pro_credentials_are_read_only_when_the_file_says_it_has_them() {
        let with = r#"{ "version": "1.0", "includesPasswords": true, "connections": [
            { "label": "p", "database": "postgres", "connectionType": "standard", "host": "h", "password": "pw",
              "sshEnabled": "true", "sshHost": "bastion", "sshUsername": "ops", "sshAuthMethod": "key", "sshPrivateKey": "-----BEGIN-----", "sshPassphrase": "pp" } ] }"#;
        let parsed = parse_import(with, "x.json").unwrap_or_else(|e| panic!("{e}"));
        let input = &parsed.connections[0].input;
        assert_eq!(input.password.as_deref(), Some("pw"));
        assert!(input.ssh.enabled && input.ssh.auth == SshAuth::PrivateKey);
        assert_eq!(input.ssh_secret.as_deref(), Some("pp"));
        assert!(parsed.warnings.iter().any(|w| w.contains("pasted SSH private key")));
        let without = with.replace("\"includesPasswords\": true", "\"includesPasswords\": false");
        let parsed = parse_import(&without, "x.json").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.connections[0].input.password, None);
        assert_eq!(parsed.connections[0].input.ssh_secret, None);
    }

    #[test]
    fn invalid_files_fail_whole_with_a_reason() {
        assert!(parse_import("", "x.json").is_err());
        assert!(parse_import("{", "x.json").is_err());
        assert!(parse_import("{}", "x.json").is_err_and(|e| e.to_string().contains("connections")));
        assert!(parse_import("{\"connections\": []}", "x.json").is_err());
        assert!(parse_import("{\"connections\": [1]}", "x.json").is_err_and(|e| e.to_string().contains("Connection 1")));
        assert!(parse_import("[]", "x.csv").is_err_and(|e| e.to_string().contains("Unsupported file type")));
        let missing_host = r#"{"connections":[{"label":"a","database":"postgres","connectionType":"standard"}]}"#;
        assert!(parse_import(missing_host, "x.json").is_ok(), "host problems surface in the form validation, not the parser");
    }

    #[test]
    fn db_free_json_round_trips_and_withholds_secrets_unless_asked() {
        let mut input = blank_input(Engine::Postgres, "prod");
        input.host = Some("db.internal".into());
        let entry = DbFreeConnection {
            name: input.name.clone(),
            engine: input.engine,
            environment: Environment::Production,
            read_only: true,
            host: input.host.clone(),
            port: input.port,
            database: Some("app".into()),
            username: Some("me".into()),
            password: Some("pw".into()),
            file_path: None,
            ssl_mode: SslMode::VerifyFull,
            ssh: SshTunnel::default(),
            ssh_secret: Some("ssh-pw".into()),
            color: Some(ConnectionColor::Red),
            favorite: true,
            uri_params: Some("application_name=db-free".into()),
            srv: false,
            tls: TlsFiles { ca_path: Some("/ca.pem".into()), ..TlsFiles::default() },
            auth: ConnectionAuth::default(),
            auth_secrets: AuthSecrets::default(),
            folders: vec!["Clients".into()],
            tags: vec![DbFreeTag { name: "Production".into(), color: ConnectionTagColor::Red }],
        };
        let file = |includes: bool| DbFreeFile {
            format: FORMAT_NAME.into(),
            version: FORMAT_VERSION,
            exported_at: "2026-10-12T00:00:00Z".into(),
            includes_passwords: includes,
            connections: vec![entry.clone()],
        };
        let with = serde_json::to_string(&file(true)).unwrap_or_default();
        let parsed = parse_import(&with, "db-free-connections-2026-10-12.json").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.source, ConnectionImportSource::DbFree);
        let got = &parsed.connections[0];
        assert_eq!(got.input.password.as_deref(), Some("pw"));
        assert_eq!(got.input.ssh_secret.as_deref(), Some("ssh-pw"));
        assert_eq!(got.input.extra.tls.ca_path.as_deref(), Some("/ca.pem"));
        assert_eq!(got.input.extra.uri_params.as_deref(), Some("application_name=db-free"));
        assert_eq!(got.folder_names, ["Clients"]);
        assert_eq!(got.tags, [("Production".to_string(), ConnectionTagColor::Red)]);
        assert!(got.input.read_only && got.input.favorite);
        // The same file declared as having no passwords drops them even if they are present.
        let without = with.replace("\"includesPasswords\":true", "\"includesPasswords\":false");
        let parsed = parse_import(&without, "x.json").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.connections[0].input.password, None);
        let newer = with.replace("\"version\":1", "\"version\":99");
        assert!(parse_import(&newer, "x.json").is_err_and(|e| e.to_string().contains("newer")));
    }

    const NCX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
    <Connections Ver="1.5">
      <Connection ConnectionName="Shop MySQL" ConnType="MYSQL" Host="10.0.0.5" Port="3306" UserName="root" Password="ENCRYPTED" SSH="true" SSH_Host="bastion" SSH_Port="2222" SSH_UserName="ops" SSH_AuthenMethod="PUBLICKEY" />
      <Connection ConnectionName="Cache" ConnType="REDIS" Host="redis.local" Port="6379" />
      <Connection ConnectionName="Local file" ConnType="SQLITE" DatabaseFileName="/data/app.sqlite3" />
      <Connection ConnectionName="Legacy" ConnType="ORACLE" Host="x" />
    </Connections>"#;

    #[test]
    fn navicat_connections_import_without_passwords_and_unsupported_types_are_reported() {
        let parsed = parse_import(NCX, "export.ncx").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.source, ConnectionImportSource::Navicat);
        assert_eq!(parsed.connections.len(), 3);
        assert_eq!(parsed.skipped.len(), 1);
        let mysql = &parsed.connections[0].input;
        assert_eq!((mysql.engine, mysql.host.as_deref(), mysql.port, mysql.username.as_deref()), (Engine::Mysql, Some("10.0.0.5"), Some(3306), Some("root")));
        assert_eq!(mysql.password, None, "Navicat's encrypted password is never imported");
        assert!(mysql.ssh.enabled && mysql.ssh.port == 2222 && mysql.ssh.auth == SshAuth::PrivateKey);
        assert_eq!(parsed.connections[2].input.file_path.as_deref(), Some("/data/app.sqlite3"));
        assert!(parsed.warnings.iter().any(|w| w.contains("not imported")));
    }

    #[test]
    fn navicat_files_with_a_doctype_or_entities_are_refused() {
        let bomb = r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY a "aaaa">]><Connections><Connection ConnectionName="&a;" ConnType="MYSQL" Host="h"/></Connections>"#;
        assert!(parse_import(bomb, "x.ncx").is_err_and(|e| e.to_string().contains("DOCTYPE")));
        let entity = r#"<Connections><Connection ConnectionName="&custom;" ConnType="MYSQL" Host="h"/></Connections>"#;
        assert!(parse_import(entity, "x.ncx").is_err());
        assert!(parse_import("<Other/>", "x.ncx").is_err_and(|e| e.to_string().contains("Connections")));
        assert!(parse_import("<Connections><Connection ConnType=\"MYSQL\"", "x.ncx").is_err());
        let predefined = r#"<Connections><Connection ConnectionName="a &amp; b" ConnType="MYSQL" Host="h"/></Connections>"#;
        let parsed = parse_import(predefined, "x.ncx").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(parsed.connections[0].input.name, "a & b");
    }

    #[test]
    fn uris_split_into_parts_with_credentials_decoded_and_secrets_kept_out_of_params() {
        let p = parse_uri("postgres://us%40er:p%3Aw@db.example.com:6543/my%20db?sslmode=verify-full&application_name=x&password=leak").unwrap_or_default();
        assert_eq!(p.username.as_deref(), Some("us@er"));
        assert_eq!(p.password.as_deref(), Some("p:w"));
        assert_eq!((p.host.as_deref(), p.port, p.database.as_deref()), (Some("db.example.com"), Some(6543), Some("my db")));
        assert_eq!(p.ssl, Some(SslMode::VerifyFull));
        assert_eq!(p.params.as_deref(), Some("application_name=x"), "ssl handled, the password parameter dropped");
        let multi = parse_uri("mongodb://a:1,b:2/db?replicaSet=rs0").unwrap_or_default();
        assert_eq!((multi.host.as_deref(), multi.port), (Some("a:1,b:2"), None));
        let srv = parse_uri("mongodb+srv://c.mongodb.net/").unwrap_or_default();
        assert!(srv.srv);
        assert_eq!(srv.ssl, Some(SslMode::Require));
        let v6 = parse_uri("redis://[::1]:6380/2").unwrap_or_default();
        assert_eq!((v6.host.as_deref(), v6.port, v6.database.as_deref()), (Some("[::1]"), Some(6380), Some("2")));
        assert!(parse_uri("not a uri").is_none());
        assert!(parse_uri("://x").is_none());
    }

    #[test]
    fn hosted_engines_default_to_tls() {
        assert_eq!(default_ssl(Engine::Supabase), SslMode::Require);
        assert_eq!(default_ssl(Engine::Planetscale), SslMode::VerifyFull);
        assert_eq!(default_ssl(Engine::PlanetscalePostgres), SslMode::VerifyFull);
        assert_eq!(default_ssl(Engine::Postgres), SslMode::Prefer);
    }
}
