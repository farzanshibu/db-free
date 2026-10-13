// SOT: workflow-secrets, node-secret-sealing, secret-redaction, last-node-config, workflow-save-merge

use crate::adapters::crypto;
use crate::error::{AppError, AppResult};
use crate::model::documents::WorkflowBody;
use crate::model::workflow::{NodeConfig, NodeSecret, NotifierKind};
use crate::model::{Document, DocumentBody, DocumentKind};
use crate::state::AppState;
use base64::Engine as _;

// ============================================================================
// WORKFLOW SECRETS
//
// WHAT:  Seals the SMTP password and the Slack webhook URL when a workflow is
//        saved, hides them again whenever a workflow is sent to the UI, and
//        decrypts them for the engine at run time.
// WHY:   They are credentials. They get the same treatment as a connection's
//        password: AES-GCM under the master key in the OS keychain, and the UI
//        only ever learns "a secret is set" (`NodeSecret::has_secret`).
// HOW:   On save the UI's `NodeSecret` says what it wants: a new value (`plain`),
//        to forget it (`clear`), to take the last saved Email / Slack login
//        (`inherit_last`), or nothing, which keeps the stored secret. A sealed
//        value sent by the client is never trusted: it is always rebuilt from the
//        stored document. A copy of a workflow (same node id, new document) finds
//        its secret again by node id in the document it was copied from.
// WHERE: src-tauri/src/services/documents.rs (hooks), src-tauri/src/services/workflows/nodes/notify.rs
// ============================================================================

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

pub fn seal(state: &AppState, plain: &str) -> AppResult<String> {
    let blob = crypto::seal(state.master_key()?, plain.as_bytes())?;
    Ok(b64().encode(blob))
}

pub fn open(state: &AppState, sealed: &str) -> AppResult<String> {
    let blob = b64().decode(sealed).map_err(|_| AppError::crypto("a stored workflow secret is not valid base64"))?;
    let bytes = crypto::open(state.master_key()?, &blob)?;
    String::from_utf8(bytes).map_err(|_| AppError::crypto("a stored workflow secret is not UTF-8"))
}

/// The secret's plaintext for the engine, `None` when the node has none.
pub fn reveal(state: &AppState, secret: &NodeSecret) -> AppResult<Option<String>> {
    match secret.sealed.as_deref().filter(|s| !s.is_empty()) {
        Some(sealed) => Ok(Some(open(state, sealed)?)),
        None => Ok(None),
    }
}

fn secret_mut(config: &mut NodeConfig) -> Option<&mut NodeSecret> {
    match config {
        NodeConfig::Email(email) => Some(&mut email.password),
        NodeConfig::Slack(slack) => Some(&mut slack.webhook),
        _ => None,
    }
}

fn secret_ref(config: &NodeConfig) -> Option<&NodeSecret> {
    match config {
        NodeConfig::Email(email) => Some(&email.password),
        NodeConfig::Slack(slack) => Some(&slack.webhook),
        _ => None,
    }
}

fn notifier_of(config: &NodeConfig) -> Option<NotifierKind> {
    match config {
        NodeConfig::Email(_) => Some(NotifierKind::Email),
        NodeConfig::Slack(_) => Some(NotifierKind::Slack),
        _ => None,
    }
}

// WHAT:  A workflow as the UI may see it: the graph form, every secret hidden.
pub fn redact(body: WorkflowBody) -> WorkflowBody {
    let mut body = body.upgraded();
    for node in &mut body.nodes {
        if let Some(secret) = secret_mut(&mut node.config) {
            secret.has_secret = secret.sealed.as_deref().is_some_and(|s| !s.is_empty());
            secret.sealed = None;
            secret.plain = None;
            secret.clear = false;
            secret.inherit_last = false;
        }
    }
    body
}

/// A document as the UI may see it.
pub fn present(mut doc: Document) -> Document {
    if let DocumentBody::Workflow(body) = doc.body {
        doc.body = DocumentBody::Workflow(redact(body));
    }
    doc
}

fn sealed_of(config: &NodeConfig) -> Option<String> {
    secret_ref(config).and_then(|s| s.sealed.clone()).filter(|s| !s.is_empty())
}

fn all_workflows(state: &AppState) -> AppResult<Vec<Document>> {
    let mut docs = state.with_store(|store| store.list_documents(DocumentKind::Workflow))?;
    docs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(docs)
}

fn graph_of(doc: &Document) -> Option<WorkflowBody> {
    match &doc.body {
        DocumentBody::Workflow(body) => Some(body.clone().upgraded()),
        _ => None,
    }
}

// WHAT:  Turns what the editor sent into what is stored.
// HOW:   For each node that holds a secret the order is: a typed value, then
//        "clear", then the secret this node already had, then (for a copy) the
//        secret of the same node id in another workflow, then (for "use the last
//        login") the most recently saved Email / Slack node's secret.
pub fn prepare_save(state: &AppState, workflow_id: &str, body: WorkflowBody) -> AppResult<WorkflowBody> {
    let mut body = body.upgraded();
    let needs_lookup = body.nodes.iter().any(|n| secret_ref(&n.config).is_some());
    if !needs_lookup {
        return Ok(body);
    }
    let stored = all_workflows(state)?;
    let previous: Option<WorkflowBody> = stored.iter().find(|d| d.id == workflow_id && !workflow_id.is_empty()).and_then(graph_of);
    for node in &mut body.nodes {
        let kind = notifier_of(&node.config);
        let id = node.id.clone();
        let own = previous.as_ref().and_then(|p| p.nodes.iter().find(|n| n.id == id && notifier_of(&n.config) == kind)).and_then(|n| sealed_of(&n.config));
        let Some(secret) = secret_mut(&mut node.config) else { continue };
        let wants_copy = secret.has_secret || secret.inherit_last;
        let typed = secret.plain.take().filter(|p| !p.is_empty());
        let sealed = if let Some(plain) = typed {
            Some(seal(state, &plain)?)
        } else if secret.clear {
            None
        } else if let Some(own) = own {
            Some(own)
        } else if wants_copy {
            let elsewhere = stored
                .iter()
                .filter(|d| d.id != workflow_id)
                .filter_map(graph_of)
                .flat_map(|g| g.nodes)
                .find(|n| n.id == id && notifier_of(&n.config) == kind && sealed_of(&n.config).is_some())
                .and_then(|n| sealed_of(&n.config));
            match elsewhere {
                Some(found) => Some(found),
                None if secret.inherit_last => most_recent(&stored, kind),
                None => None,
            }
        } else {
            None
        };
        secret.has_secret = sealed.is_some();
        secret.sealed = sealed;
        secret.clear = false;
        secret.inherit_last = false;
    }
    Ok(body)
}

// WHAT:  The sealed secret of the most recently saved node of this kind, in any
//        workflow (this one's stored copy included: a second Email node added to a
//        workflow starts from the first one's login).
fn most_recent(stored: &[Document], kind: Option<NotifierKind>) -> Option<String> {
    stored
        .iter()
        .filter_map(graph_of)
        .flat_map(|g| g.nodes)
        .find(|n| notifier_of(&n.config) == kind && sealed_of(&n.config).is_some())
        .and_then(|n| sealed_of(&n.config))
}

// WHAT:  The most recently saved Email / Slack setup, for a new node to start from.
// WHY:   Typing the same SMTP host, login and sender into every new node is the
//        chore this removes. Host, port, security, user name and sender come
//        along; recipients, subject and body do not. The secret is not sent: the
//        returned node is marked `inherit_last`, so saving it copies the sealed
//        secret on the server.
pub fn last_node_config(state: &AppState, kind: NotifierKind) -> AppResult<Option<NodeConfig>> {
    for doc in all_workflows(state)? {
        let Some(body) = graph_of(&doc) else { continue };
        for node in body.nodes {
            match (&node.config, kind) {
                (NodeConfig::Email(email), NotifierKind::Email) if !email.host.trim().is_empty() || sealed_of(&node.config).is_some() => {
                    let mut remembered = email.clone();
                    remembered.to = String::new();
                    remembered.subject = String::new();
                    remembered.body = String::new();
                    remembered.password = NodeSecret { has_secret: email.password.sealed.is_some(), inherit_last: email.password.sealed.is_some(), ..NodeSecret::default() };
                    return Ok(Some(NodeConfig::Email(remembered)));
                }
                (NodeConfig::Slack(slack), NotifierKind::Slack) if sealed_of(&node.config).is_some() => {
                    let mut remembered = slack.clone();
                    remembered.text = String::new();
                    remembered.webhook = NodeSecret { has_secret: true, inherit_last: true, ..NodeSecret::default() };
                    return Ok(Some(NodeConfig::Slack(remembered)));
                }
                _ => {}
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::keyring::MemoryKeyProvider;
    use crate::model::workflow::{EmailConfig, SlackConfig, WorkflowNode};
    use crate::store::Store;

    fn state() -> AppState {
        AppState::new(Store::open_in_memory().unwrap(), Box::new(MemoryKeyProvider::default()))
    }

    fn email_node(id: &str, secret: NodeSecret) -> WorkflowNode {
        WorkflowNode {
            id: id.into(),
            x: 0.0,
            y: 0.0,
            label: String::new(),
            continue_on_error: false,
            config: NodeConfig::Email(EmailConfig { host: "smtp.example.com".into(), from: "a@x.io".into(), to: "b@x.io".into(), password: secret, ..EmailConfig::default() }),
        }
    }

    fn slack_node(id: &str, secret: NodeSecret) -> WorkflowNode {
        WorkflowNode {
            id: id.into(),
            x: 0.0,
            y: 0.0,
            label: String::new(),
            continue_on_error: false,
            config: NodeConfig::Slack(SlackConfig { webhook: secret, text: "hi".into() }),
        }
    }

    fn typed(plain: &str) -> NodeSecret {
        NodeSecret { plain: Some(plain.into()), ..NodeSecret::default() }
    }

    fn doc(id: &str, name: &str, nodes: Vec<WorkflowNode>) -> Document {
        let body = WorkflowBody { nodes, ..WorkflowBody::default() };
        serde_json::from_value(serde_json::json!({
            "id": id, "kind": "workflow", "connectionId": null, "name": name, "tags": [],
            "createdAt": "", "updatedAt": "",
            "body": { "kind": "workflow", "data": crate::model::workflow::persisting(|| serde_json::to_value(&body).unwrap()) }
        }))
        .unwrap()
    }

    fn password_of(body: &WorkflowBody, node: usize) -> NodeSecret {
        match &body.nodes[node].config {
            NodeConfig::Email(e) => e.password.clone(),
            NodeConfig::Slack(s) => s.webhook.clone(),
            _ => panic!("no secret"),
        }
    }

    fn save(state: &AppState, id: &str, name: &str, body: WorkflowBody) -> WorkflowBody {
        let prepared = prepare_save(state, id, body).unwrap();
        let stored = doc(id, name, prepared.nodes.clone());
        state.with_store(|s| s.upsert_document(&stored)).unwrap();
        prepared
    }

    #[test]
    fn a_typed_secret_is_sealed_and_never_returned() {
        let state = state();
        let saved = save(&state, "w1", "one", WorkflowBody { nodes: vec![email_node("n1", typed("hunter2"))], ..WorkflowBody::default() });
        let secret = password_of(&saved, 0);
        assert!(secret.has_secret && secret.plain.is_none());
        let sealed = secret.sealed.clone().unwrap();
        assert!(!sealed.contains("hunter2"));
        assert_eq!(reveal(&state, &secret).unwrap().as_deref(), Some("hunter2"));
        let shown = redact(saved);
        let hidden = password_of(&shown, 0);
        assert!(hidden.has_secret && hidden.sealed.is_none() && hidden.plain.is_none());
        let json = serde_json::to_string(&shown).unwrap();
        assert!(!json.contains("hunter2") && !json.contains(&sealed), "{json}");
    }

    #[test]
    fn saving_without_a_value_keeps_the_stored_secret() {
        let state = state();
        save(&state, "w1", "one", WorkflowBody { nodes: vec![email_node("n1", typed("hunter2"))], ..WorkflowBody::default() });
        // The UI echoes the redacted node back: has_secret true, nothing else.
        let echoed = NodeSecret { has_secret: true, ..NodeSecret::default() };
        let again = save(&state, "w1", "one", WorkflowBody { nodes: vec![email_node("n1", echoed)], ..WorkflowBody::default() });
        assert_eq!(reveal(&state, &password_of(&again, 0)).unwrap().as_deref(), Some("hunter2"));
        // A client-supplied sealed blob is ignored.
        let forged = NodeSecret { sealed: Some("Zm9yZ2Vk".into()), has_secret: true, ..NodeSecret::default() };
        let forged = save(&state, "w2", "two", WorkflowBody { nodes: vec![email_node("n9", forged)], ..WorkflowBody::default() });
        assert!(!password_of(&forged, 0).has_secret, "nothing real to keep, and the blob is not trusted");
    }

    #[test]
    fn clear_and_replace() {
        let state = state();
        save(&state, "w1", "one", WorkflowBody { nodes: vec![email_node("n1", typed("old"))], ..WorkflowBody::default() });
        let replaced = save(&state, "w1", "one", WorkflowBody { nodes: vec![email_node("n1", typed("new"))], ..WorkflowBody::default() });
        assert_eq!(reveal(&state, &password_of(&replaced, 0)).unwrap().as_deref(), Some("new"));
        let cleared = save(&state, "w1", "one", WorkflowBody { nodes: vec![email_node("n1", NodeSecret { clear: true, has_secret: true, ..NodeSecret::default() })], ..WorkflowBody::default() });
        assert!(!password_of(&cleared, 0).has_secret);
        assert_eq!(reveal(&state, &password_of(&cleared, 0)).unwrap(), None);
    }

    #[test]
    fn a_copied_workflow_finds_its_secret_by_node_id() {
        let state = state();
        save(&state, "orig", "original", WorkflowBody { nodes: vec![slack_node("s1", typed("https://hooks.slack.com/services/T/B/x"))], ..WorkflowBody::default() });
        // The copy goes through list + save: same node id, new document, redacted secret.
        let echoed = NodeSecret { has_secret: true, ..NodeSecret::default() };
        let copy = save(&state, "copy", "original (copy)", WorkflowBody { nodes: vec![slack_node("s1", echoed)], ..WorkflowBody::default() });
        assert_eq!(reveal(&state, &password_of(&copy, 0)).unwrap().as_deref(), Some("https://hooks.slack.com/services/T/B/x"));
    }

    #[test]
    fn a_new_node_can_start_from_the_last_saved_login() {
        let state = state();
        assert!(last_node_config(&state, NotifierKind::Email).unwrap().is_none());
        save(&state, "w1", "one", WorkflowBody { nodes: vec![email_node("n1", typed("hunter2"))], ..WorkflowBody::default() });
        let Some(NodeConfig::Email(remembered)) = last_node_config(&state, NotifierKind::Email).unwrap() else { panic!("expected a remembered email setup") };
        assert_eq!(remembered.host, "smtp.example.com");
        assert_eq!(remembered.from, "a@x.io");
        assert!(remembered.to.is_empty() && remembered.subject.is_empty() && remembered.body.is_empty());
        assert!(remembered.password.has_secret && remembered.password.inherit_last && remembered.password.sealed.is_none());
        // Saving a node built from it copies the sealed secret on the server.
        let fresh = WorkflowNode { id: "n2".into(), x: 0.0, y: 0.0, label: String::new(), continue_on_error: false, config: NodeConfig::Email(remembered) };
        let saved = save(&state, "w2", "two", WorkflowBody { nodes: vec![fresh], ..WorkflowBody::default() });
        assert_eq!(reveal(&state, &password_of(&saved, 0)).unwrap().as_deref(), Some("hunter2"));
        assert!(last_node_config(&state, NotifierKind::Slack).unwrap().is_none());
    }

    #[test]
    fn a_second_node_in_the_same_workflow_starts_from_the_first_ones_login() {
        let state = state();
        save(&state, "w1", "one", WorkflowBody { nodes: vec![email_node("n1", typed("hunter2"))], ..WorkflowBody::default() });
        // The editor adds node n2 from `last_node_config` and saves the whole workflow again.
        let Some(NodeConfig::Email(remembered)) = last_node_config(&state, NotifierKind::Email).unwrap() else { panic!("expected a remembered email setup") };
        let echoed = NodeSecret { has_secret: true, ..NodeSecret::default() };
        let second = WorkflowNode { id: "n2".into(), x: 0.0, y: 0.0, label: String::new(), continue_on_error: false, config: NodeConfig::Email(remembered) };
        let saved = save(&state, "w1", "one", WorkflowBody { nodes: vec![email_node("n1", echoed), second], ..WorkflowBody::default() });
        assert_eq!(reveal(&state, &password_of(&saved, 0)).unwrap().as_deref(), Some("hunter2"));
        assert_eq!(reveal(&state, &password_of(&saved, 1)).unwrap().as_deref(), Some("hunter2"), "the new node inherited the sealed login");
    }

    #[test]
    fn workflows_without_secrets_pass_straight_through() {
        let state = state();
        let body = WorkflowBody::default();
        assert!(prepare_save(&state, "", body).unwrap().nodes.is_empty());
    }
}
