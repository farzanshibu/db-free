// SOT: smtp-adapter, workflow-email-node, email-delivery, smtp-tls-modes

use crate::error::{AppError, AppResult};
use crate::model::workflow::SmtpSecurity;
use lettre::message::header::ContentType;
use lettre::message::{Mailbox, Message};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use std::time::Duration;

// ============================================================================
// SMTP DELIVERY FOR THE WORKFLOW EMAIL NODE
//
// WHAT:  Sends one email through the user's own SMTP server.
// WHY:   Email is the notification channel people actually read; the app talks to
//        the mail server directly rather than through a hosted service.
// HOW:   `lettre` over rustls. A body that contains an opening HTML tag is sent
//        as HTML, anything else as plain text. Connection and command timeouts
//        are 20 s, so a wrong port or a TLS mismatch fails fast instead of
//        hanging. `To` may list several addresses separated by commas or
//        semicolons.
// WHERE: src-tauri/src/services/workflows/nodes/notify.rs
// ============================================================================

const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct EmailRequest {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// How the connection is secured once `Auto` has looked at the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TlsMode {
    /// TLS from the first byte.
    Wrapper,
    /// STARTTLS when the server offers it, plain text otherwise.
    Opportunistic,
    /// STARTTLS or fail.
    Required,
    Off,
}

fn tls_mode(security: SmtpSecurity, port: u16) -> TlsMode {
    match security {
        SmtpSecurity::Auto if port == 465 => TlsMode::Wrapper,
        SmtpSecurity::Auto => TlsMode::Opportunistic,
        SmtpSecurity::Implicit => TlsMode::Wrapper,
        SmtpSecurity::StartTls => TlsMode::Required,
        SmtpSecurity::None => TlsMode::Off,
    }
}

/// True when the text holds an opening HTML tag such as `<p>` or `<table class="x">`.
pub fn looks_like_html(body: &str) -> bool {
    let bytes = body.as_bytes();
    bytes.iter().enumerate().any(|(i, b)| {
        if *b != b'<' {
            return false;
        }
        let rest = &bytes[i + 1..];
        let name_len = rest.iter().take_while(|c| c.is_ascii_alphanumeric()).count();
        name_len > 0 && rest[0].is_ascii_alphabetic() && matches!(rest.get(name_len), Some(b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/'))
    })
}

fn split_addresses(list: &str) -> Vec<&str> {
    list.split([',', ';']).map(str::trim).filter(|a| !a.is_empty()).collect()
}

fn mailbox(text: &str, what: &str) -> AppResult<Mailbox> {
    text.parse::<Mailbox>().map_err(|e| AppError::invalid_input(format!("\"{text}\" is not a valid {what} address: {e}")))
}

fn build_message(request: &EmailRequest) -> AppResult<Message> {
    let recipients = split_addresses(&request.to);
    if recipients.is_empty() {
        return Err(AppError::invalid_input("To address is empty."));
    }
    let mut builder = Message::builder().from(mailbox(request.from.trim(), "From")?).subject(request.subject.clone());
    for recipient in recipients {
        builder = builder.to(mailbox(recipient, "To")?);
    }
    let content_type = if looks_like_html(&request.body) { ContentType::TEXT_HTML } else { ContentType::TEXT_PLAIN };
    builder.header(content_type).body(request.body.clone()).map_err(|e| AppError::invalid_input(format!("Could not build the message: {e}")))
}

// WHAT:  Delivers the message. Dropping the returned future abandons the send.
pub async fn send(request: &EmailRequest) -> AppResult<()> {
    let host = request.host.trim();
    if host.is_empty() {
        return Err(AppError::invalid_input("SMTP host is empty."));
    }
    if request.from.trim().is_empty() {
        return Err(AppError::invalid_input("From address is empty."));
    }
    let message = build_message(request)?;
    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host).port(request.port).timeout(Some(TIMEOUT));
    let params = || TlsParameters::new(host.to_string()).map_err(|e| AppError::driver(format!("TLS setup failed: {e}")));
    builder = match tls_mode(request.security, request.port) {
        TlsMode::Wrapper => builder.tls(Tls::Wrapper(params()?)),
        TlsMode::Opportunistic => builder.tls(Tls::Opportunistic(params()?)),
        TlsMode::Required => builder.tls(Tls::Required(params()?)),
        TlsMode::Off => builder,
    };
    if let Some(user) = request.username.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        builder = builder.credentials(Credentials::new(user.to_string(), request.password.clone().unwrap_or_default()));
    }
    let transport = builder.build();
    transport.send(message).await.map(|_| ()).map_err(|e| AppError::driver(format!("Could not send the email: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(to: &str, body: &str) -> EmailRequest {
        EmailRequest {
            host: "smtp.example.com".into(),
            port: 587,
            security: SmtpSecurity::Auto,
            username: None,
            password: None,
            from: "Alerts <alerts@example.com>".into(),
            to: to.into(),
            subject: "Hi".into(),
            body: body.into(),
        }
    }

    #[test]
    fn security_follows_the_port_in_auto_mode() {
        assert_eq!(tls_mode(SmtpSecurity::Auto, 465), TlsMode::Wrapper);
        assert_eq!(tls_mode(SmtpSecurity::Auto, 587), TlsMode::Opportunistic);
        assert_eq!(tls_mode(SmtpSecurity::Implicit, 2525), TlsMode::Wrapper);
        assert_eq!(tls_mode(SmtpSecurity::StartTls, 465), TlsMode::Required);
        assert_eq!(tls_mode(SmtpSecurity::None, 25), TlsMode::Off);
    }

    #[test]
    fn html_is_detected_by_an_opening_tag() {
        assert!(looks_like_html("<p>hello</p>"));
        assert!(looks_like_html("Report:\n<table class=\"x\"><tr></tr></table>"));
        assert!(looks_like_html("line<br/>break"));
        assert!(!looks_like_html("a < b and c > d"));
        assert!(!looks_like_html("1 <2> 3"));
        assert!(!looks_like_html("plain text, no markup"));
        assert!(!looks_like_html(""));
    }

    #[test]
    fn recipients_split_on_commas_and_semicolons() {
        assert_eq!(split_addresses("a@x.io, b@x.io;c@x.io ,"), vec!["a@x.io", "b@x.io", "c@x.io"]);
        assert!(split_addresses(" ; ").is_empty());
    }

    #[test]
    fn messages_are_built_or_rejected_before_any_network_call() {
        assert!(build_message(&request("a@x.io, B <b@x.io>", "hi")).is_ok());
        assert!(build_message(&request("", "hi")).is_err());
        assert!(build_message(&request("not-an-address", "hi")).is_err());
        let mut bad_from = request("a@x.io", "hi");
        bad_from.from = "nope".into();
        assert!(build_message(&bad_from).is_err());
    }

    #[tokio::test]
    async fn empty_settings_fail_fast() {
        let mut empty = request("a@x.io", "hi");
        empty.host = "  ".into();
        assert!(matches!(send(&empty).await, Err(AppError::InvalidInput { .. })));
        let mut no_from = request("a@x.io", "hi");
        no_from.from = String::new();
        assert!(matches!(send(&no_from).await, Err(AppError::InvalidInput { .. })));
    }
}
