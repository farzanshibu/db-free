// SOT: workflow-network-node-tests, fake-http-endpoint, fake-smtp-server

use super::runner::{execute, RunRequest};
use super::{secrets, WorkflowHost};
use crate::adapters::keyring::MemoryKeyProvider;
use crate::error::AppResult;
use crate::model::documents::WorkflowRunReport;
use crate::model::workflow::{
    EmailConfig, HttpConfig, HttpMethod, JavascriptConfig, NodeConfig, NodeRunState, NodeSecret, RunStatus, RunTrigger, SlackConfig, SmtpSecurity,
    WorkflowEdge, WorkflowEvent, WorkflowNode,
};
use crate::model::WorkflowBody;
use crate::state::AppState;
use crate::store::Store;
use base64::Engine as _;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

// ============================================================================
// NETWORK NODES AGAINST LOCAL SERVERS
//
// WHAT:  Runs the HTTP, Slack and Email nodes through the real engine against a
//        tiny HTTP server and a tiny SMTP server bound to localhost.
// WHY:   These nodes are all request building: which placeholders were filled,
//        where the sealed secret ended up, what went out on the wire. A mock of
//        the transport would only test the mock.
// HOW:   Each server answers one conversation and hands back what it received.
// ============================================================================

#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<WorkflowEvent>>,
}

impl WorkflowHost for Recorder {
    fn emit(&self, event: WorkflowEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }

    fn notify(&self, _title: &str, _body: &str) -> AppResult<()> {
        Ok(())
    }
}

impl Recorder {
    fn state_of(&self, node: &str) -> Option<(NodeRunState, Option<String>, Option<u64>)> {
        let events = self.events.lock().ok()?;
        events.iter().rev().find_map(|event| match event {
            WorkflowEvent::Node { update } if update.node_id == node && update.state != NodeRunState::Running => {
                Some((update.state, update.error.clone(), update.outputs.first().map(|o| o.total_rows)))
            }
            _ => None,
        })
    }
}

fn state() -> AppState {
    AppState::new(Store::open_in_memory().unwrap(), Box::new(MemoryKeyProvider::default()))
}

fn node(id: &str, config: NodeConfig) -> WorkflowNode {
    WorkflowNode { id: id.into(), x: 0.0, y: 0.0, label: String::new(), continue_on_error: false, config }
}

fn edge(a: &str, b: &str) -> WorkflowEdge {
    WorkflowEdge { id: format!("{a}-{b}"), source: a.into(), target: b.into(), source_handle: None }
}

/// A source node: two rows, `{ id, name, email }`, from the sandbox (no database needed).
fn people() -> WorkflowNode {
    let code = "return [{ id: 1, name: \"ada\", email: \"ada@x.io\" }, { id: 2, name: \"o'neil\", email: \"on@x.io\" }];";
    node("people", NodeConfig::Javascript(JavascriptConfig { code: code.into() }))
}

async fn run(state: &AppState, host: &Recorder, nodes: Vec<WorkflowNode>, edges: Vec<WorkflowEdge>) -> WorkflowRunReport {
    let body = WorkflowBody { nodes, edges, ..WorkflowBody::default() };
    let leaves: Vec<String> = body.nodes.iter().filter(|n| !body.edges.iter().any(|e| e.source == n.id)).map(|n| n.id.clone()).collect();
    let request = RunRequest { run_id: "net", workflow_id: "wf", workflow_connection: None, body: &body, targets: leaves, trigger: RunTrigger::Manual, step_delay_ms: 0 };
    execute(state, host, &CancellationToken::new(), request).await.unwrap()
}

// ---------------------------------------------------------------- fake HTTP
fn request_complete(raw: &[u8]) -> bool {
    let text = String::from_utf8_lossy(raw);
    let Some((head, body)) = text.split_once("\r\n\r\n") else { return false };
    let length = head
        .lines()
        .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
        .unwrap_or(0);
    body.len() >= length
}

/// Answers one request with `status` and `reply`; resolves to the raw request it received.
async fn http_server(status: u16, reply: &'static str) -> (u16, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = stream.read(&mut chunk).await.unwrap();
            if read == 0 {
                break;
            }
            received.extend_from_slice(&chunk[..read]);
            if request_complete(&received) {
                break;
            }
        }
        let response = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len());
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.ok();
        String::from_utf8_lossy(&received).into_owned()
    });
    (port, handle)
}

#[tokio::test]
async fn the_http_node_fills_its_templates_and_reads_the_response_path() {
    let (port, request) = http_server(200, r#"{"data":{"results":[{"id":1},{"id":2},{"id":3}]}}"#).await;
    let state = state();
    let host = Recorder::default();
    let http = node(
        "call",
        NodeConfig::Http(HttpConfig {
            method: HttpMethod::Post,
            url: format!("http://127.0.0.1:{port}/items/{{{{id}}}}"),
            allow_private_network: true,
            headers: r#"{"X-Name": "{{name}}", "Content-Type": "application/json"}"#.into(),
            body: r#"{"who":"{{name}}","n":{{id + 1}}}"#.into(),
            response_path: "data.results".into(),
        }),
    );
    let report = run(&state, &host, vec![people(), http], vec![edge("people", "call")]).await;
    assert_eq!(report.status, RunStatus::Completed, "{:?}", report.nodes);
    assert_eq!(host.state_of("call").and_then(|s| s.2), Some(3), "one row per element of the picked array");
    let raw = request.await.unwrap();
    let lower = raw.to_ascii_lowercase();
    assert!(raw.starts_with("POST /items/1 HTTP/1.1"), "the URL is filled from the first input row: {raw}");
    assert!(lower.contains("x-name: ada"), "{raw}");
    assert!(raw.ends_with(r#"{"who":"ada","n":2}"#), "{raw}");
}

#[tokio::test]
async fn the_http_node_refuses_localhost_unless_the_workflow_opts_in() {
    let (port, _request) = http_server(200, "[]").await;
    let host = Recorder::default();
    let http = node("call", NodeConfig::Http(HttpConfig { url: format!("http://127.0.0.1:{port}/x"), ..HttpConfig::default() }));
    run(&state(), &host, vec![http], vec![]).await;
    let (_, error, _) = host.state_of("call").expect("the node reported a state");
    assert!(error.unwrap_or_default().contains("Blocked: 127.0.0.1 is a loopback address"));
}

#[tokio::test]
async fn a_bad_status_fails_the_http_node_with_the_status_line() {
    let (port, _request) = http_server(404, r#"{"error":"nope"}"#).await;
    let state = state();
    let host = Recorder::default();
    let http = node("call", NodeConfig::Http(HttpConfig { url: format!("http://127.0.0.1:{port}/missing"), allow_private_network: true, ..HttpConfig::default() }));
    let report = run(&state, &host, vec![http], vec![]).await;
    assert_eq!(report.status, RunStatus::Failed);
    let (outcome, error, _) = host.state_of("call").unwrap();
    assert_eq!(outcome, NodeRunState::Error);
    assert!(error.unwrap_or_default().starts_with("HTTP 404"));
}

#[tokio::test]
async fn the_slack_node_posts_the_message_to_its_sealed_webhook() {
    let (port, request) = http_server(200, "ok").await;
    let state = state();
    let webhook = secrets::seal(&state, &format!("http://127.0.0.1:{port}/services/T/B/secret")).unwrap();
    let host = Recorder::default();
    let slack = node(
        "say",
        NodeConfig::Slack(SlackConfig {
            webhook: NodeSecret { sealed: Some(webhook), has_secret: true, ..NodeSecret::default() },
            text: "{{rows.count}} people, first: {{name}}".into(),
        }),
    );
    let report = run(&state, &host, vec![people(), slack], vec![edge("people", "say")]).await;
    assert_eq!(report.status, RunStatus::Completed, "{:?}", report.nodes);
    let raw = request.await.unwrap();
    assert!(raw.starts_with("POST /services/T/B/secret HTTP/1.1"), "{raw}");
    assert!(raw.ends_with(r#"{"text":"2 people, first: ada"}"#), "{raw}");
}

#[tokio::test]
async fn a_slack_node_without_a_webhook_says_so() {
    let state = state();
    let host = Recorder::default();
    let slack = node("say", NodeConfig::Slack(SlackConfig { webhook: NodeSecret::default(), text: "hi".into() }));
    let report = run(&state, &host, vec![people(), slack], vec![edge("people", "say")]).await;
    assert_eq!(report.status, RunStatus::Failed);
    assert_eq!(host.state_of("say").unwrap().1.as_deref(), Some("Webhook URL is empty."));
}

// ---------------------------------------------------------------- fake SMTP
/// A server that accepts one message with `AUTH PLAIN`; resolves to every line the client sent.
async fn smtp_server() -> (u16, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        let mut seen = Vec::new();
        write.write_all(b"220 test ESMTP\r\n").await.unwrap();
        let mut in_data = false;
        while let Some(line) = lines.next_line().await.unwrap() {
            seen.push(line.clone());
            if in_data {
                if line == "." {
                    in_data = false;
                    write.write_all(b"250 queued\r\n").await.unwrap();
                }
                continue;
            }
            let upper = line.to_ascii_uppercase();
            let reply: &[u8] = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                b"250-test\r\n250-AUTH PLAIN\r\n250 8BITMIME\r\n"
            } else if upper.starts_with("AUTH") {
                b"235 welcome\r\n"
            } else if upper.starts_with("DATA") {
                in_data = true;
                b"354 go ahead\r\n"
            } else if upper.starts_with("QUIT") {
                write.write_all(b"221 bye\r\n").await.unwrap();
                break;
            } else {
                b"250 ok\r\n"
            };
            write.write_all(reply).await.unwrap();
        }
        seen
    });
    (port, handle)
}

#[tokio::test]
async fn the_email_node_logs_in_with_the_unsealed_password_and_addresses_every_recipient() {
    let (port, session) = smtp_server().await;
    let state = state();
    let password = secrets::seal(&state, "hunter2").unwrap();
    let host = Recorder::default();
    let mail = node(
        "mail",
        NodeConfig::Email(EmailConfig {
            host: "127.0.0.1".into(),
            port,
            security: SmtpSecurity::None,
            username: "alerts@example.com".into(),
            password: NodeSecret { sealed: Some(password), has_secret: true, ..NodeSecret::default() },
            from: "Alerts <alerts@example.com>".into(),
            to: "ops@example.com; {{email}}".into(),
            subject: "{{rows.count}} people".into(),
            body: "<p>Hello {{name}}</p>{{rows.html}}".into(),
        }),
    );
    let report = run(&state, &host, vec![people(), mail], vec![edge("people", "mail")]).await;
    assert_eq!(report.status, RunStatus::Completed, "{:?} {:?}", report.nodes, host.state_of("mail"));
    let lines = session.await.unwrap();
    let joined = lines.join("\n");
    let auth = lines.iter().find(|l| l.to_ascii_uppercase().starts_with("AUTH PLAIN")).unwrap_or_else(|| panic!("no AUTH line in {joined}"));
    let token = auth.split_whitespace().nth(2).unwrap_or_default();
    let decoded = base64::engine::general_purpose::STANDARD.decode(token).unwrap();
    assert_eq!(decoded, b"\0alerts@example.com\0hunter2", "the sealed password was opened for the login");
    assert!(lines.iter().any(|l| l.to_ascii_uppercase().starts_with("RCPT TO:<OPS@EXAMPLE.COM>")), "{joined}");
    assert!(lines.iter().any(|l| l.to_ascii_uppercase().starts_with("RCPT TO:<ADA@X.IO>")), "the placeholder in To was filled: {joined}");
    assert!(joined.contains("Subject: 2 people"), "{joined}");
    assert!(joined.to_ascii_lowercase().contains("content-type: text/html"), "a body with a tag is sent as HTML: {joined}");
}

#[tokio::test]
async fn an_email_node_with_a_missing_setting_names_it() {
    let state = state();
    let host = Recorder::default();
    let mail = node("mail", NodeConfig::Email(EmailConfig { host: "127.0.0.1".into(), from: "a@x.io".into(), ..EmailConfig::default() }));
    let report = run(&state, &host, vec![people(), mail], vec![edge("people", "mail")]).await;
    assert_eq!(report.status, RunStatus::Failed);
    assert_eq!(host.state_of("mail").unwrap().1.as_deref(), Some("To address is empty."));
}
