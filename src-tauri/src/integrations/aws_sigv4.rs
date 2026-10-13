// SOT: aws-sigv4, aws-request-signing, aws-credentials

use crate::error::{AppError, AppResult};
use crate::model::ResolvedConnection;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

// ============================================================================
// AWS SIGNATURE V4
//
// WHAT:  Signs requests to AWS services (DynamoDB, QLDB, S3) with the
//        connection's access key / secret. No AWS SDK: the services this app
//        talks to are plain HTTPS, and the SDK would add ~150 crates.
// WHY:   Keeping the signing in one audited module means the secret key is
//        only ever read here and never logged.
// HOW:   https://docs.aws.amazon.com/IAM/latest/UserGuide/create-signed-request.html
// WHERE: src-tauri/src/integrations/{dynamodb,qldb,s3}.rs
// ============================================================================

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
}

impl AwsCredentials {
    /// host = region, username = access key id, secret = secret access key.
    /// A `secret` of the form `SECRET|SESSION_TOKEN` carries a temporary session token.
    pub fn from_connection(conn: &ResolvedConnection) -> AppResult<AwsCredentials> {
        let s = &conn.summary;
        let region = s
            .host
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .ok_or_else(|| AppError::invalid_input("AWS region is required (e.g. us-east-1)."))?
            .to_string();
        let access_key = s
            .username
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| AppError::invalid_input("AWS access key ID is required."))?
            .to_string();
        let raw_secret = conn
            .secret
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .ok_or_else(|| AppError::invalid_input("AWS secret access key is required."))?;
        let (secret_key, legacy_token) = match raw_secret.split_once('|') {
            Some((k, t)) if !t.trim().is_empty() => (k.trim().to_string(), Some(t.trim().to_string())),
            _ => (raw_secret.to_string(), None),
        };
        // The session token has its own field now (sealed with the other sign-in
        // secrets); a `SECRET|TOKEN` password saved by an older build still parses.
        let session_token = crate::integrations::cloud_auth::session_token().or(legacy_token);
        Ok(AwsCredentials { region, access_key, secret_key, session_token })
    }
}

fn hmac(key: &[u8], data: &[u8]) -> AppResult<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| AppError::crypto(e.to_string()))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

#[derive(Debug, Clone)]
pub struct SignedHeaders {
    pub headers: Vec<(String, String)>,
}

// WHAT:  Percent-encodes one path segment or query value the way SigV4 wants.
// WHY:   S3 keys carry spaces, `+` and unicode; the canonical request must use
//        the same encoding the server derives or every signature mismatches.
//        Unreserved characters are A-Z a-z 0-9 - _ . ~ and nothing else.
pub fn uri_encode(raw: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else if c == '/' && !encode_slash {
            out.push('/');
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

// WHAT:  One request to sign. `query` is the canonical query string — keys
//        sorted, both sides already `uri_encode`d — or "" when there is none.
//        `content_type` is None for bodyless verbs so it is not signed.
// HOW:   Returns every header the request must carry (host, x-amz-date,
//        x-amz-target, content-type, x-amz-security-token, authorization).
pub struct SignRequest<'a> {
    pub service: &'a str,
    pub method: &'a str,
    pub host: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub amz_target: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub body: &'a [u8],
    pub now: chrono::DateTime<chrono::Utc>,
}

/// Signs a POST with a JSON body — the shape DynamoDB and QLDB use.
pub fn sign_post(creds: &AwsCredentials, req: &SignRequest<'_>) -> AppResult<SignedHeaders> {
    sign(creds, req)
}

pub fn sign(creds: &AwsCredentials, req: &SignRequest<'_>) -> AppResult<SignedHeaders> {
    let SignRequest { service, method, host, path, query, amz_target, content_type, body, now } = *req;
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let payload_hash = sha256_hex(body);

    let mut headers: Vec<(String, String)> = vec![
        ("host".into(), host.into()),
        ("x-amz-content-sha256".into(), payload_hash.clone()),
        ("x-amz-date".into(), amz_date.clone()),
    ];
    if let Some(ct) = content_type {
        headers.push(("content-type".into(), ct.into()));
    }
    if let Some(t) = amz_target {
        headers.push(("x-amz-target".into(), t.into()));
    }
    if let Some(tok) = &creds.session_token {
        headers.push(("x-amz-security-token".into(), tok.clone()));
    }
    headers.sort_by(|a, b| a.0.cmp(&b.0));

    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
    let signed_headers: String = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical_request = format!("{method}\n{path}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}");

    let scope = format!("{date_stamp}/{}/{service}/aws4_request", creds.region);
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha256_hex(canonical_request.as_bytes()));

    let k_date = hmac(format!("AWS4{}", creds.secret_key).as_bytes(), date_stamp.as_bytes())?;
    let k_region = hmac(&k_date, creds.region.as_bytes())?;
    let k_service = hmac(&k_region, service.as_bytes())?;
    let k_signing = hmac(&k_service, b"aws4_request")?;
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes())?);

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        creds.access_key
    );
    headers.push(("authorization".into(), authorization));
    Ok(SignedHeaders { headers })
}

/// SHA-256 of an empty body: the payload hash of a presigned GET (RDS IAM tokens).
pub const EMPTY_PAYLOAD_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

// WHAT:  One GET to presign: the credentials travel in the query string, so the
//        URL itself is the proof of identity.
// HOW:   `params` are the request's own parameters (raw, not yet encoded);
//        `payload_hash` is `UNSIGNED-PAYLOAD` for S3 or `EMPTY_PAYLOAD_HASH`
//        for services that sign the empty body. Only `host` is signed.
// WHERE: src-tauri/src/integrations/cloud_auth.rs (RDS IAM authentication token)
pub struct PresignRequest<'a> {
    pub service: &'a str,
    /// `host` or `host:port`, exactly as the client will send it.
    pub host: &'a str,
    pub path: &'a str,
    pub params: &'a [(&'a str, &'a str)],
    pub expires_secs: u32,
    pub payload_hash: &'a str,
    pub now: chrono::DateTime<chrono::Utc>,
}

// WHAT:  Signature Version 4 query-string presign. Returns the full query string
//        (`Action=…&X-Amz-Algorithm=…&X-Amz-Signature=…`), parameters sorted.
// WHY:   The RDS IAM "password" is exactly this: a presigned `rds-db:connect`
//        request with the scheme stripped. The session token, when there is one,
//        is part of the signed query, as AWS requires.
// HOW:   https://docs.aws.amazon.com/AmazonS3/latest/API/sigv4-query-string-auth.html
pub fn presign_get(creds: &AwsCredentials, req: &PresignRequest<'_>) -> AppResult<String> {
    let PresignRequest { service, host, path, params, expires_secs, payload_hash, now } = *req;
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let scope = format!("{date_stamp}/{}/{service}/aws4_request", creds.region);

    let mut query: Vec<(String, String)> = params.iter().map(|(k, v)| (uri_encode(k, true), uri_encode(v, true))).collect();
    query.push(("X-Amz-Algorithm".into(), "AWS4-HMAC-SHA256".into()));
    query.push(("X-Amz-Credential".into(), uri_encode(&format!("{}/{scope}", creds.access_key), true)));
    query.push(("X-Amz-Date".into(), amz_date.clone()));
    query.push(("X-Amz-Expires".into(), expires_secs.to_string()));
    query.push(("X-Amz-SignedHeaders".into(), "host".into()));
    if let Some(token) = &creds.session_token {
        query.push(("X-Amz-Security-Token".into(), uri_encode(token, true)));
    }
    query.sort();
    let canonical_query = query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");

    let canonical_request = format!("GET\n{path}\n{canonical_query}\nhost:{host}\n\nhost\n{payload_hash}");
    let string_to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha256_hex(canonical_request.as_bytes()));

    let k_date = hmac(format!("AWS4{}", creds.secret_key).as_bytes(), date_stamp.as_bytes())?;
    let k_region = hmac(&k_date, creds.region.as_bytes())?;
    let k_service = hmac(&k_region, service.as_bytes())?;
    let k_signing = hmac(&k_service, b"aws4_request")?;
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes())?);
    Ok(format!("{canonical_query}&X-Amz-Signature={signature}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known-answer test derived from the AWS "signing examples" (GET-less POST variant):
    // the derived signing key for the documented example credentials must match.
    #[test]
    fn signing_key_matches_aws_example() {
        let creds = AwsCredentials {
            region: "us-east-1".into(),
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let k_date = hmac(b"AWS4wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", b"20150830").unwrap_or_default();
        let k_region = hmac(&k_date, b"us-east-1").unwrap_or_default();
        let k_service = hmac(&k_region, b"iam").unwrap_or_default();
        let k_signing = hmac(&k_service, b"aws4_request").unwrap_or_default();
        assert_eq!(hex::encode(k_signing), "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9");
        let now = chrono::DateTime::parse_from_rfc3339("2015-08-30T12:36:00Z").map(|d| d.with_timezone(&chrono::Utc));
        let signed = sign_post(
            &creds,
            &SignRequest {
                service: "dynamodb",
                method: "POST",
                host: "dynamodb.us-east-1.amazonaws.com",
                path: "/",
                query: "",
                amz_target: Some("DynamoDB_20120810.ListTables"),
                content_type: Some("application/x-amz-json-1.0"),
                body: b"{}",
                now: now.unwrap_or_default(),
            },
        );
        let signed = signed.unwrap_or(SignedHeaders { headers: vec![] });
        let auth = signed.headers.iter().find(|(k, _)| k == "authorization").map(|(_, v)| v.clone()).unwrap_or_default();
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/dynamodb/aws4_request, SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date;x-amz-target, Signature="));
        assert_eq!(auth.len(), auth.find("Signature=").unwrap_or(0) + "Signature=".len() + 64);
    }

    #[test]
    fn session_token_splits_from_secret() {
        use crate::model::{ConnectionSummary, Engine, Environment, SslMode};
        let conn = ResolvedConnection {
            summary: ConnectionSummary {
                id: "x".into(),
                name: "x".into(),
                engine: Engine::Dynamodb,
                environment: Environment::Local,
                read_only: false,
                host: Some("eu-west-1".into()),
                port: None,
                database: None,
                username: Some("AKIA".into()),
                file_path: None,
                ssl_mode: SslMode::Prefer,
                has_secret: true,
                ssh: crate::model::SshTunnel::default(),
                has_ssh_secret: false,
                folder: None,
                color: None,
                favorite: false,
                auth: Default::default(),
                has_auth_secrets: Default::default(),
                created_at: String::new(),
                updated_at: String::new(),
                extra: Default::default(),
            },
            secret: Some("SECRET|TOKEN".into()),
        };
        let c = AwsCredentials::from_connection(&conn).unwrap_or_else(|_| AwsCredentials { region: String::new(), access_key: String::new(), secret_key: String::new(), session_token: None });
        assert_eq!(c.secret_key, "SECRET");
        assert_eq!(c.session_token.as_deref(), Some("TOKEN"));
    }

    #[test]
    fn uri_encoding_follows_the_sigv4_rules() {
        // Unreserved characters pass through; everything else is %XX upper-case.
        assert_eq!(uri_encode("a-z_0.9~", true), "a-z_0.9~");
        assert_eq!(uri_encode("my key+1", true), "my%20key%2B1");
        // A path keeps its separators, an object key used as a value does not.
        assert_eq!(uri_encode("logs/2026/app.log", false), "logs/2026/app.log");
        assert_eq!(uri_encode("logs/2026/app.log", true), "logs%2F2026%2Fapp.log");
    }

    fn at(rfc3339: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(rfc3339).map(|d| d.with_timezone(&chrono::Utc)).unwrap_or_default()
    }

    // The worked example from "Authenticating Requests: Using Query Parameters
    // (AWS Signature Version 4)" in the S3 documentation, signature included.
    #[test]
    fn presign_matches_the_documented_s3_example() {
        let creds = AwsCredentials {
            region: "us-east-1".into(),
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let query = presign_get(
            &creds,
            &PresignRequest {
                service: "s3",
                host: "examplebucket.s3.amazonaws.com",
                path: "/test.txt",
                params: &[],
                expires_secs: 86400,
                payload_hash: "UNSIGNED-PAYLOAD",
                now: at("2013-05-24T00:00:00Z"),
            },
        )
        .unwrap_or_default();
        assert_eq!(
            query,
            "X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request\
             &X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host\
             &X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
    }

    // An RDS `rds-db:connect` token has the shape the RDS documentation shows
    // (`Action=connect&DBUser=…&X-Amz-…&X-Amz-Signature=…`); the signatures were
    // derived with an independent implementation of the same algorithm that
    // reproduces the S3 vector above.
    #[test]
    fn rds_iam_token_is_a_presigned_connect_request() {
        let request = |token: Option<&str>| {
            let creds = AwsCredentials {
                region: "us-east-1".into(),
                access_key: if token.is_some() { "ASIATEMP" } else { "AKIDEXAMPLE" }.into(),
                secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
                session_token: token.map(str::to_string),
            };
            presign_get(
                &creds,
                &PresignRequest {
                    service: "rds-db",
                    host: "mydb.abc.us-east-1.rds.amazonaws.com:5432",
                    path: "/",
                    params: &[("Action", "connect"), ("DBUser", "iam_user")],
                    expires_secs: 900,
                    payload_hash: EMPTY_PAYLOAD_HASH,
                    now: at("2024-01-02T03:04:05Z"),
                },
            )
            .unwrap_or_default()
        };
        assert_eq!(
            request(None),
            "Action=connect&DBUser=iam_user&X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIDEXAMPLE%2F20240102%2Fus-east-1%2Frds-db%2Faws4_request\
             &X-Amz-Date=20240102T030405Z&X-Amz-Expires=900&X-Amz-SignedHeaders=host\
             &X-Amz-Signature=812963d83f4c6c43ba0e6037a72bd548dced6bb7af2f7cb596c1c64201452a11"
        );
        // Temporary credentials add the (encoded) token to the signed query.
        assert_eq!(
            request(Some("IQoJb3JpZ2luX2VjE+/=")),
            "Action=connect&DBUser=iam_user&X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=ASIATEMP%2F20240102%2Fus-east-1%2Frds-db%2Faws4_request\
             &X-Amz-Date=20240102T030405Z&X-Amz-Expires=900&X-Amz-Security-Token=IQoJb3JpZ2luX2VjE%2B%2F%3D&X-Amz-SignedHeaders=host\
             &X-Amz-Signature=37d3182da768cd165acc77f8cc475574af3503d7a177c93842de7eef9e461ae2"
        );
    }

    #[test]
    fn empty_payload_hash_is_the_sha256_of_nothing() {
        assert_eq!(sha256_hex(b""), EMPTY_PAYLOAD_HASH);
    }
}
