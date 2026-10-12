// SOT: agent-workflow-builder, ai-workflow-schema, workflow-draft-validation, workflow-graph-layout, workflow-node-rules

use super::{choice, clean_name, flag, list, plural, preview, properties, text, worse, worst_of, Draft};
use crate::error::{AppError, AppResult};
use crate::guard::destructive::StatementKind;
use crate::model::workflow::{Combinator, EmailConfig, NotifierKind, RuleGroup, SlackConfig, SourceHandle};
use crate::model::{DocumentBody, NodeConfig, NodeSecret, WorkflowBody, WorkflowEdge, WorkflowNode};
use crate::services::agent::provider::{tool, ToolSpec};
use crate::services::workflows::graph::Graph;
use crate::services::workflows::schedule;
use crate::state::AppState;
use serde::Serialize;
use serde_json::{json, Value as Json};
use std::collections::{HashMap, HashSet};

// WHAT:  `create_workflow`: what the model is told a workflow is (a graph of
//        nodes), and the check that turns its answer into a real `WorkflowBody`.
// WHY:   A workflow is automation that runs without anyone reading it, so the
//        model proposes the graph and this file decides what is allowed:
//        every node's config must be one the engine can run, the wiring must
//        follow the canvas's own connection rules, and what the model must never
//        decide — the destructive-statement opt-in, secrets, the connection a
//        node runs on, where a file is written — is set here, not taken from it.
// HOW:   Node configs are the engine's own `NodeConfig` (read with serde, then
//        sanitised); the connection rules mirror `canConnect` in
//        src/features/workflows/model.ts; cycles and Repeat limits come from the
//        engine's own `Graph`. A graph that arrives without positions is laid
//        out left to right by depth, as the canvas's "Tidy up" does.
// WHERE: ./mod.rs (the shared create/update flow), src-tauri/src/model/workflow.rs,
//        src-tauri/src/services/workflows (the engine), src/features/workflows (the canvas)

const MAX_NODES: usize = 30;
const MAX_EDGES: usize = 60;
const MAX_LABEL: usize = 80;
const MAX_REPEAT: u32 = 1_000;

/// Every node kind the assistant may place (the canvas's own list).
const NODE_KINDS: [&str; 17] = [
    "schedule", "query", "http", "read_file", "save_file", "insert", "filter", "row_count_gate", "branch", "repeat", "delay", "regex",
    "javascript", "notification", "email", "slack", "note",
];

// Canvas geometry (src/features/workflows/model.ts `autoLayout`).
const NODE_WIDTH: f64 = 264.0;
const NODE_HEIGHT: f64 = 92.0;
const COLUMN_GAP: f64 = 72.0;
const ROW_GAP: f64 = 56.0;
const ORIGIN: f64 = 80.0;

// ---------------------------------------------------------------------------
// What the model is told
// ---------------------------------------------------------------------------

pub fn workflow_tool(language: &str) -> ToolSpec {
    tool(
        "create_workflow",
        &format!(
            "Build a workflow — a graph of nodes that pass rows along, run through the same safety checks \
             as the editor — and save it for the user. The user reviews every node and approves before \
             anything is saved; nothing runs until they run the workflow. To change a workflow the user \
             @-mentioned, or whose document id they quoted, pass its documentId; the new nodes and edges \
             REPLACE the old ones (reuse a node's id to keep its saved email or Slack login).\n\
             Each node is {{ id, label?, continueOnError?, config: {{ kind, …fields }} }}. Kinds and fields:\n\
             - schedule: mode \"interval\" (intervalAmount, intervalUnit seconds|minutes|hours|days) or \"at_time\" (time \"HH:MM\", \
               days [mon..sun], timezone IANA name or blank). A trigger: nothing feeds it; it hands on one row, {{{{firedAt}}}}. The \
               user switches schedules on; they are paused when saved.\n\
             - query: sql — {language} statements; the last result set is passed on, a write passes on one row, rowsAffected. \
               Runs on the workflow's own connection.\n\
             - http: method get|post|put|patch|delete|head, url, headers (a JSON object as text), body, responsePath (dot path to the rows).\n\
             - read_file: path, format csv|tsv|json|ndjson. No input.\n\
             - filter: group {{ combinator and|or, rules [{{ field, op, value }}] }} keeps matching rows.\n\
             - branch: the same group; matches leave by the \"true\" handle, the rest by \"false\".\n\
             - row_count_gate: op gt|gte|lt|lte|eq|neq, value — stops the branch unless the row count satisfies it.\n\
             - repeat: count — runs everything after it that many times; {{{{i}}}} (from 0) and {{{{total}}}} are available. No input.\n\
             - delay: amount, unit ms|s|m.\n\
             - regex: sourceField, pattern (Rust syntax), flags (i m s g), mode extract|replace, replacement, outputField.\n\
             - javascript: code — the body of an async function with rows, fields; return the rows. Sandboxed: no network or files.\n\
             - save_file: format csv|tsv|json|ndjson|markdown|sql, tableName (sql). The user picks the file on the first run. A sink.\n\
             - insert: schema, table — inserts the incoming rows. A sink.\n\
             - notification: title, body. A sink.\n\
             - email: to, subject, body (the SMTP login of the user's last email node is reused). A sink.\n\
             - slack: text (the user's last Slack webhook is reused). A sink.\n\
             - note: text — a comment on the canvas, never run, never wired.\n\
             rule ops: eq neq gt lt gte lte contains starts_with ends_with is_null is_not_null in. Text fields (sql, url, body, \
             subject…) may use {{{{column}}}} to read a value from the rows that reach the node, {{{{rows.table}}}} for them all, \
             {{{{rows.count}}}} for the count, and {{{{date.now}}}}. Placeholders in SQL are written as safe literals.\n\
             Edges are {{ source, target, sourceHandle? }}: omit \"edges\" to chain the nodes in the order given (not for a branch); \
             only a branch has two outputs and its edges must say sourceHandle \"true\" or \"false\"; sinks and notes have no output; \
             a schedule, read_file, repeat and note take no input; no cycles. Prefer a straight line unless the task needs a branch."
        ),
        json!({
            "type": "object",
            "properties": properties(vec![
                ("name", json!({ "type": "string", "description": "Workflow name." })),
                ("documentId", json!({ "type": "string", "description": "Id of an existing workflow to replace, from an @mention or an id the user quoted. Omit to create a new one." })),
                ("nodes", json!({ "type": "array", "minItems": 1, "maxItems": MAX_NODES, "items": node_schema() })),
                ("edges", json!({ "type": "array", "maxItems": MAX_EDGES, "items": edge_schema() })),
            ]),
            "required": ["name", "nodes"],
            "additionalProperties": false
        }),
    )
}

fn node_schema() -> Json {
    json!({
        "type": "object",
        "properties": properties(vec![
            ("id", json!({ "type": "string", "description": "Short unique id (letters, digits, - and _). Edges refer to it. Generated when omitted." })),
            ("label", json!({ "type": "string", "description": "Title on the canvas; say what the node does." })),
            ("continueOnError", json!({ "type": "boolean", "description": "Keep the run going if this node fails. Defaults to false." })),
            ("config", json!({
                "type": "object",
                "description": "kind plus that kind's fields, as listed above.",
                "properties": { "kind": { "type": "string", "enum": NODE_KINDS } },
                "required": ["kind"]
            })),
        ]),
        "required": ["config"],
        "additionalProperties": false
    })
}

fn edge_schema() -> Json {
    json!({
        "type": "object",
        "properties": properties(vec![
            ("source", json!({ "type": "string", "description": "Id of the node the rows come from." })),
            ("target", json!({ "type": "string", "description": "Id of the node they go to." })),
            ("sourceHandle", json!({ "type": "string", "enum": ["true", "false"], "description": "Only for an edge leaving a branch." })),
        ]),
        "required": ["source", "target"],
        "additionalProperties": false
    })
}

// ---------------------------------------------------------------------------
// The user's earlier email / Slack setup
// ---------------------------------------------------------------------------

// WHAT:  The last saved Email and Slack setups, which a new node starts from.
// WHY:   The canvas prefills a new Email node with the user's last SMTP host,
//        login name and sender, and the server copies the sealed password when
//        it is saved. An assistant-made node should start the same way — the
//        model has no way to know any of it, and must never be handed the secret.
#[derive(Default)]
pub(super) struct Remembered {
    email: Option<EmailConfig>,
    slack: Option<SlackConfig>,
}

impl Remembered {
    pub(super) fn load(state: &AppState) -> Remembered {
        let fetch = |kind: NotifierKind| match crate::services::workflows::last_node_config(state, kind) {
            Ok(found) => found,
            Err(err) => {
                log::warn!("could not read the last {kind:?} setup: {err}");
                None
            }
        };
        Remembered {
            email: match fetch(NotifierKind::Email) {
                Some(NodeConfig::Email(email)) => Some(email),
                _ => None,
            },
            slack: match fetch(NotifierKind::Slack) {
                Some(NodeConfig::Slack(slack)) => Some(slack),
                _ => None,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// What the model's answer becomes
// ---------------------------------------------------------------------------

/// What a node does, for the approval prompt, and what it needs the user to do.
struct Facts {
    /// How invasive its statements are (a Query's SQL; an Insert is a write).
    worst: StatementKind,
    /// "reads: SELECT count(*) FROM orders".
    detail: String,
    /// Something the user must still do for the node to work.
    needs: Option<String>,
}

// WHAT:  A workflow proposal → a validated `WorkflowBody`.
// HOW:   Nodes first (each checked on its own), then the wiring, then the
//        whole graph against the engine's rules, then positions. On an update the
//        saved workflow's "schedules active" switch is kept: the assistant never
//        turns a schedule on.
pub(super) fn parse(args: &Json, previous: Option<&DocumentBody>, remembered: &Remembered) -> AppResult<Draft> {
    let what = "A workflow";
    let name = clean_name(args, what)?;
    let raw_nodes = list(args, "nodes", what, 1, MAX_NODES)?;
    let before = match previous {
        Some(DocumentBody::Workflow(body)) => Some(body),
        _ => None,
    };

    let mut nodes: Vec<WorkflowNode> = Vec::with_capacity(raw_nodes.len());
    let mut facts: Vec<Facts> = Vec::with_capacity(raw_nodes.len());
    let mut taken: HashSet<String> = HashSet::new();
    for (index, raw) in raw_nodes.iter().enumerate() {
        let id = node_id(raw, index, &taken)?;
        taken.insert(id.clone());
        let node = parse_node(raw, id, remembered, before)?;
        facts.push(check(&node)?);
        nodes.push(node);
    }
    if nodes.iter().all(|node| matches!(node.config, NodeConfig::Note(_))) {
        return Err(AppError::invalid_input("A workflow needs at least one node that does something, not only notes."));
    }

    let edges = parse_edges(args, &nodes)?;
    let levels = levels_of(&nodes, &edges)?;
    position(&mut nodes, &levels);

    let mut worst = StatementKind::Read;
    let mut previews = Vec::with_capacity(nodes.len() + 1);
    let mut caveats: Vec<String> = Vec::new();
    for (index, (node, fact)) in nodes.iter().zip(&facts).enumerate() {
        worst = worse(worst, fact.worst);
        previews.push(format!("{}. {} — {}", index + 1, title_of(node), fact.detail));
        if let Some(need) = &fact.needs {
            caveats.push(need.clone());
        }
    }
    if !edges.is_empty() {
        previews.push(format!("Connections: {}", describe_edges(&nodes, &edges)));
    }
    caveats.extend(follow_ups(&nodes, &facts, before));

    let mut summary = plural(nodes.iter().filter(|n| !matches!(n.config, NodeConfig::Note(_))).count(), "node", "nodes");
    if !edges.is_empty() {
        summary.push_str(&format!(" · {}", plural(edges.len(), "connection", "connections")));
    }
    let body = WorkflowBody { steps: Vec::new(), nodes, edges, schedules_active: before.is_some_and(|b| b.schedules_active) };
    Ok(Draft { name, body: DocumentBody::Workflow(body), summary, previews, worst, caveats })
}

/// "Count orders [query]", or just the kind when the node has no label.
fn title_of(node: &WorkflowNode) -> String {
    if node.label.trim().is_empty() {
        node.config.kind_name().to_string()
    } else {
        format!("{} [{}]", node.label.trim(), node.config.kind_name())
    }
}

/// An enum's wire spelling ("gte"), for the sentences in the approval prompt.
fn spelled<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

// ------------------------------------------------------------------- nodes

fn node_id(raw: &Json, index: usize, taken: &HashSet<String>) -> AppResult<String> {
    match text(raw, "id") {
        Some(id) => {
            let valid = id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !valid {
                return Err(AppError::invalid_input(format!(
                    "Node id \"{id}\" may only hold letters, digits, - and _ (40 characters at most)."
                )));
            }
            if taken.contains(id) {
                return Err(AppError::invalid_input(format!("Two nodes share the id \"{id}\".")));
            }
            Ok(id.to_string())
        }
        None => {
            let mut n = index + 1;
            loop {
                let id = format!("n{n}");
                if !taken.contains(&id) {
                    return Ok(id);
                }
                n += 1;
            }
        }
    }
}

fn parse_node(raw: &Json, id: String, remembered: &Remembered, before: Option<&WorkflowBody>) -> AppResult<WorkflowNode> {
    let what = format!("Node \"{id}\"");
    let config = raw
        .get("config")
        .filter(|c| c.is_object())
        .ok_or_else(|| AppError::invalid_input(format!("{what} needs a \"config\" object with a \"kind\".")))?;
    let kind = choice(config, "kind", &NODE_KINDS, &what)?
        .ok_or_else(|| AppError::invalid_input(format!("{what}: config needs a \"kind\". Use {}.", NODE_KINDS.join(", "))))?;

    let mut tagged = config.clone();
    tagged["kind"] = json!(kind);
    let mut config = serde_json::from_value::<NodeConfig>(tagged)
        .map_err(|err| AppError::invalid_input(format!("{what}: the {kind} config is not valid: {err}")))?;
    sanitize(&mut config, &id, remembered, before);

    let label: String = text(raw, "label").unwrap_or_default().chars().take(MAX_LABEL).collect();
    Ok(WorkflowNode { id, x: 0.0, y: 0.0, label, continue_on_error: flag(raw, "continueOnError").unwrap_or(false), config })
}

// WHAT:  Set the things the model does not get to decide.
// WHY:   The destructive-statement opt-in is the user's own tick in the inspector;
//        a model that sets it would be switching off its own safety check (a tick
//        the user already gave to the very same statement is the one exception —
//        it was their decision about that text). The same goes for secrets (never
//        invented, only inherited from the user's earlier setup), the connection a
//        node runs on (always the workflow's own) and where a file is written
//        (chosen by the user on the first run).
fn sanitize(config: &mut NodeConfig, id: &str, remembered: &Remembered, before: Option<&WorkflowBody>) {
    match config {
        NodeConfig::Query(query) => {
            query.connection_id = None;
            // A tick the user already gave to exactly this statement stays; any other statement starts unticked.
            query.allow_destructive = before.is_some_and(|body| {
                body.nodes.iter().any(|node| {
                    node.id == id
                        && matches!(&node.config, NodeConfig::Query(saved) if saved.allow_destructive && saved.sql.trim() == query.sql.trim())
                })
            });
        }
        NodeConfig::Insert(insert) => insert.connection_id = None,
        NodeConfig::SaveFile(save) => save.path = String::new(),
        NodeConfig::Email(email) => {
            email.password = NodeSecret::default();
            let mut inherits = false;
            if let Some(base) = &remembered.email {
                let same_server = blank(&email.host) || email.host.trim().eq_ignore_ascii_case(base.host.trim());
                if blank(&email.host) {
                    email.host = base.host.clone();
                    email.port = base.port;
                    email.security = base.security;
                }
                if same_server {
                    if blank(&email.username) {
                        email.username = base.username.clone();
                    }
                    inherits = base.password.has_secret;
                }
                if blank(&email.from) {
                    email.from = base.from.clone();
                }
            }
            email.password = NodeSecret {
                has_secret: inherits || keeps_secret(before, id, NotifierKind::Email),
                inherit_last: inherits,
                ..NodeSecret::default()
            };
        }
        NodeConfig::Slack(slack) => {
            let inherits = remembered.slack.as_ref().is_some_and(|base| base.webhook.has_secret);
            slack.webhook = NodeSecret {
                has_secret: inherits || keeps_secret(before, id, NotifierKind::Slack),
                inherit_last: inherits,
                ..NodeSecret::default()
            };
        }
        _ => {}
    }
}

/// Whether the saved workflow already holds a login for this node (same id, same kind).
fn keeps_secret(before: Option<&WorkflowBody>, id: &str, kind: NotifierKind) -> bool {
    before.is_some_and(|body| {
        body.nodes.iter().any(|node| {
            node.id == id
                && match (&node.config, kind) {
                    (NodeConfig::Email(email), NotifierKind::Email) => email.password.has_secret || email.password.sealed.is_some(),
                    (NodeConfig::Slack(slack), NotifierKind::Slack) => slack.webhook.has_secret || slack.webhook.sealed.is_some(),
                    _ => false,
                }
        })
    })
}

fn blank(value: &str) -> bool {
    value.trim().is_empty()
}

fn rules_text(group: &RuleGroup) -> String {
    if group.rules.is_empty() {
        return "every row".to_string();
    }
    let joiner = match group.combinator {
        Combinator::And => " and ",
        Combinator::Or => " or ",
    };
    group
        .rules
        .iter()
        .map(|rule| {
            let op = spelled(&rule.op);
            if rule.op.needs_value() { format!("{} {op} {}", rule.field, rule.value) } else { format!("{} {op}", rule.field) }
        })
        .collect::<Vec<_>>()
        .join(joiner)
}

fn check_rules(what: &str, group: &RuleGroup) -> AppResult<()> {
    for rule in &group.rules {
        if blank(&rule.field) {
            return Err(AppError::invalid_input(format!("{what}: every rule needs a \"field\" (a column name).")));
        }
        if rule.op.needs_value() && rule.value.is_empty() {
            return Err(AppError::invalid_input(format!("{what}: the rule on \"{}\" needs a \"value\".", rule.field)));
        }
    }
    Ok(())
}

/// The host part of a URL: what the user is told the workflow calls.
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority.rsplit('@').next().unwrap_or_default().to_string()
}

// WHAT:  Check one node's config against what the engine will do with it, and
//        describe it for the approval prompt.
fn check(node: &WorkflowNode) -> AppResult<Facts> {
    let what = format!("Node \"{}\"", node.id);
    let read = |detail: String| Facts { worst: StatementKind::Read, detail, needs: None };
    match &node.config {
        NodeConfig::Schedule(config) => {
            if let Some(problem) = schedule::problem(config) {
                return Err(AppError::invalid_input(format!("{what}: {problem}")));
            }
            Ok(read(format!("fires {}", schedule::summarize(config).to_lowercase())))
        }
        NodeConfig::Query(query) => {
            if blank(&query.sql) {
                return Err(AppError::invalid_input(format!("{what}: a query node needs \"sql\".")));
            }
            let kind = worst_of(&query.sql);
            let tag = match kind {
                StatementKind::Read => "reads",
                StatementKind::Write => "WRITES",
                StatementKind::Destructive => "DESTROYS",
            };
            let needs = (kind == StatementKind::Destructive).then(|| {
                format!(
                    "\"{}\" contains a destructive statement, so it will not run until you tick “Allow destructive statements” on that node.",
                    title_of(node)
                )
            });
            Ok(Facts { worst: kind, detail: format!("{tag}: {}", preview(&query.sql)), needs })
        }
        NodeConfig::Http(http) => {
            let url = http.url.trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(AppError::invalid_input(format!("{what}: the http url must start with http:// or https://.")));
            }
            if !blank(&http.headers) && !serde_json::from_str::<Json>(&http.headers).is_ok_and(|v| v.is_object()) {
                return Err(AppError::invalid_input(format!("{what}: headers must be a JSON object written as text.")));
            }
            Ok(read(format!("calls {} {}", http.method.as_str(), preview(url))))
        }
        NodeConfig::ReadFile(file) => {
            if blank(&file.path) {
                return Err(AppError::invalid_input(format!("{what}: a read_file node needs a \"path\".")));
            }
            Ok(read(format!("reads the file {}", preview(&file.path))))
        }
        NodeConfig::SaveFile(save) => Ok(read(format!("saves the rows as {}; you pick the file on the first run", spelled(&save.format)))),
        NodeConfig::Insert(insert) => {
            if blank(&insert.table) {
                return Err(AppError::invalid_input(format!("{what}: an insert node needs a \"table\".")));
            }
            let target = if blank(&insert.schema) {
                insert.table.trim().to_string()
            } else {
                format!("{}.{}", insert.schema.trim(), insert.table.trim())
            };
            Ok(Facts { worst: StatementKind::Write, detail: format!("WRITES: inserts the rows into {target}"), needs: None })
        }
        NodeConfig::Filter(filter) => {
            check_rules(&what, &filter.group)?;
            Ok(read(format!("keeps rows where {}", rules_text(&filter.group))))
        }
        NodeConfig::Branch(branch) => {
            check_rules(&what, &branch.group)?;
            Ok(read(format!("true when {}, false for the rest", rules_text(&branch.group))))
        }
        NodeConfig::RowCountGate(gate) => Ok(read(format!("continues only if the row count is {} {}", spelled(&gate.op), gate.value))),
        NodeConfig::Repeat(repeat) => {
            if repeat.count == 0 || repeat.count > MAX_REPEAT {
                return Err(AppError::invalid_input(format!("{what}: a repeat count must be between 1 and {MAX_REPEAT}.")));
            }
            Ok(read(format!("repeats everything after it {} times", repeat.count)))
        }
        NodeConfig::Delay(delay) => {
            if !delay.amount.is_finite() || delay.amount < 0.0 {
                return Err(AppError::invalid_input(format!("{what}: a delay needs a positive amount.")));
            }
            Ok(read(format!("waits {} {}", delay.amount, spelled(&delay.unit))))
        }
        NodeConfig::Regex(regex) => {
            if blank(&regex.source_field) || blank(&regex.pattern) {
                return Err(AppError::invalid_input(format!("{what}: a regex node needs a \"sourceField\" and a \"pattern\".")));
            }
            let outcome = crate::services::workflows::test_regex(&regex.pattern, &regex.flags, regex.mode, &regex.replacement, "");
            if let Some(error) = outcome.error {
                return Err(AppError::invalid_input(format!("{what}: the pattern is not valid: {error}")));
            }
            Ok(read(format!("applies /{}/ to {}", regex.pattern, regex.source_field.trim())))
        }
        NodeConfig::Javascript(script) => {
            if blank(&script.code) {
                return Err(AppError::invalid_input(format!("{what}: a javascript node needs \"code\".")));
            }
            let first = script.code.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
            Ok(read(format!("runs a script (sandboxed): {}", preview(first))))
        }
        NodeConfig::Notification(notice) => {
            if blank(&notice.title) && blank(&notice.body) {
                return Err(AppError::invalid_input(format!("{what}: a notification needs a \"title\" or a \"body\".")));
            }
            Ok(read(format!("shows a notification: {}", preview(&notice.title))))
        }
        NodeConfig::Email(email) => {
            if blank(&email.to) {
                return Err(AppError::invalid_input(format!("{what}: an email node needs a \"to\" address.")));
            }
            let needs = (!email.password.has_secret || blank(&email.host)).then(|| {
                format!("\"{}\" needs its SMTP host and password entered on that node before it can send.", title_of(node))
            });
            Ok(Facts { worst: StatementKind::Read, detail: format!("emails {}: {}", preview(&email.to), preview(&email.subject)), needs })
        }
        NodeConfig::Slack(slack) => {
            if blank(&slack.text) {
                return Err(AppError::invalid_input(format!("{what}: a slack node needs \"text\".")));
            }
            let needs = (!slack.webhook.has_secret)
                .then(|| format!("\"{}\" needs its Slack webhook URL entered on that node before it can post.", title_of(node)));
            Ok(Facts { worst: StatementKind::Read, detail: format!("posts to Slack: {}", preview(&slack.text)), needs })
        }
        NodeConfig::Note(note) => Ok(read(format!("note: {}", preview(&note.text)))),
    }
}

/// Things worth telling the user that are not about one node.
fn follow_ups(nodes: &[WorkflowNode], facts: &[Facts], before: Option<&WorkflowBody>) -> Vec<String> {
    let mut out = Vec::new();
    let scheduled = nodes.iter().any(|node| matches!(node.config, NodeConfig::Schedule(_)));
    if scheduled && !before.is_some_and(|body| body.schedules_active) {
        out.push("The schedule is saved paused: it fires only once “Schedules active” is switched on in the workflow's toolbar.".to_string());
    }
    let mut hosts: Vec<String> = Vec::new();
    for node in nodes {
        if let NodeConfig::Http(http) = &node.config {
            let host = host_of(http.url.trim());
            if !hosts.contains(&host) {
                hosts.push(host);
            }
        }
    }
    if !hosts.is_empty() {
        out.push(format!("It calls {} over the network; rows from earlier nodes can be sent there.", hosts.join(", ")));
    }
    if facts.iter().any(|fact| fact.worst == StatementKind::Write) {
        out.push("Some nodes write to the database when the workflow runs.".to_string());
    }
    out
}

// ------------------------------------------------------------------- edges

/// Where a node sits in the wiring: whether it takes input and which outputs it has.
struct Ports {
    input: bool,
    /// Zero for a sink, two for a branch, otherwise one.
    outputs: u8,
}

// The same table as `NODE_SPECS` in src/features/workflows/model.ts.
fn ports(config: &NodeConfig) -> Ports {
    match config {
        NodeConfig::Schedule(_) | NodeConfig::ReadFile(_) | NodeConfig::Repeat(_) => Ports { input: false, outputs: 1 },
        NodeConfig::Note(_) => Ports { input: false, outputs: 0 },
        NodeConfig::SaveFile(_) | NodeConfig::Insert(_) | NodeConfig::Notification(_) | NodeConfig::Email(_) | NodeConfig::Slack(_) => {
            Ports { input: true, outputs: 0 }
        }
        NodeConfig::Branch(_) => Ports { input: true, outputs: 2 },
        _ => Ports { input: true, outputs: 1 },
    }
}

fn handle_of(raw: &Json, what: &str) -> AppResult<Option<SourceHandle>> {
    let word = raw.get("sourceHandle").or_else(|| raw.get("handle")).and_then(Json::as_str).map(|w| w.trim().to_lowercase());
    match word.as_deref() {
        None | Some("" | "default" | "out") => Ok(None),
        Some("true") => Ok(Some(SourceHandle::True)),
        Some("false") => Ok(Some(SourceHandle::False)),
        Some(other) => Err(AppError::invalid_input(format!("{what}: sourceHandle \"{other}\" is not true or false."))),
    }
}

fn edge_id(source: &str, handle: Option<SourceHandle>, target: &str) -> String {
    let side = match handle {
        Some(SourceHandle::True) => "true",
        Some(SourceHandle::False) => "false",
        None => "out",
    };
    format!("e-{source}-{side}-{target}")
}

// WHAT:  Whether a wire between two nodes is allowed (the canvas's `canConnect`).
fn can_connect(
    nodes: &HashMap<&str, &WorkflowNode>,
    existing: &[WorkflowEdge],
    source: &str,
    target: &str,
    handle: Option<SourceHandle>,
) -> AppResult<()> {
    let (Some(from), Some(to)) = (nodes.get(source), nodes.get(target)) else {
        return Err(AppError::invalid_input(format!("An edge joins \"{source}\" and \"{target}\", and one of them is not a node.")));
    };
    if source == target {
        return Err(AppError::invalid_input(format!("Node \"{source}\" cannot feed itself.")));
    }
    let out = ports(&from.config);
    if out.outputs == 0 {
        return Err(AppError::invalid_input(format!(
            "\"{source}\" ({}) has no output, so nothing can follow it.",
            from.config.kind_name()
        )));
    }
    if !ports(&to.config).input {
        return Err(AppError::invalid_input(format!("\"{target}\" ({}) takes no input, so nothing can feed it.", to.config.kind_name())));
    }
    match (out.outputs, handle) {
        (2, None) => {
            return Err(AppError::invalid_input(format!(
                "The edge from the branch \"{source}\" must say sourceHandle \"true\" or \"false\"."
            )));
        }
        (1, Some(_)) => {
            return Err(AppError::invalid_input(format!(
                "\"{source}\" ({}) has a single output: leave sourceHandle out.",
                from.config.kind_name()
            )));
        }
        _ => {}
    }
    if existing.iter().any(|edge| edge.source == source && edge.target == target && edge.source_handle == handle) {
        return Err(AppError::invalid_input(format!("\"{source}\" is connected to \"{target}\" twice.")));
    }
    Ok(())
}

fn parse_edges(args: &Json, nodes: &[WorkflowNode]) -> AppResult<Vec<WorkflowEdge>> {
    let by_id: HashMap<&str, &WorkflowNode> = nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut edges: Vec<WorkflowEdge> = Vec::new();

    let Some(raw_edges) = args.get("edges").and_then(Json::as_array) else {
        // No wiring given: a straight line through everything that is not a note.
        let line: Vec<&WorkflowNode> = nodes.iter().filter(|node| !matches!(node.config, NodeConfig::Note(_))).collect();
        for pair in line.windows(2) {
            if let [from, to] = pair {
                can_connect(&by_id, &edges, &from.id, &to.id, None).map_err(|err| {
                    AppError::invalid_input(format!(
                        "{} Give \"edges\" explicitly for a graph that is not a straight line.",
                        err.message()
                    ))
                })?;
                edges.push(WorkflowEdge {
                    id: edge_id(&from.id, None, &to.id),
                    source: from.id.clone(),
                    target: to.id.clone(),
                    source_handle: None,
                });
            }
        }
        return Ok(edges);
    };
    if raw_edges.len() > MAX_EDGES {
        return Err(AppError::invalid_input(format!("A workflow may have at most {MAX_EDGES} edges.")));
    }
    for raw in raw_edges {
        let source = text(raw, "source")
            .or_else(|| text(raw, "from"))
            .ok_or_else(|| AppError::invalid_input("Every edge needs a \"source\" node id."))?;
        let target = text(raw, "target")
            .or_else(|| text(raw, "to"))
            .ok_or_else(|| AppError::invalid_input("Every edge needs a \"target\" node id."))?;
        let handle = handle_of(raw, &format!("Edge {source} → {target}"))?;
        can_connect(&by_id, &edges, source, target, handle)?;
        edges.push(WorkflowEdge { id: edge_id(source, handle, target), source: source.to_string(), target: target.to_string(), source_handle: handle });
    }
    Ok(edges)
}

fn describe_edges(nodes: &[WorkflowNode], edges: &[WorkflowEdge]) -> String {
    let names: HashMap<&str, String> = nodes
        .iter()
        .map(|node| (node.id.as_str(), if blank(&node.label) { node.id.clone() } else { node.label.trim().to_string() }))
        .collect();
    let name = |id: &str| names.get(id).cloned().unwrap_or_else(|| id.to_string());
    let mut parts: Vec<String> = edges
        .iter()
        .take(20)
        .map(|edge| {
            let side = match edge.source_handle {
                Some(SourceHandle::True) => " (true)",
                Some(SourceHandle::False) => " (false)",
                None => "",
            };
            format!("{} →{side} {}", name(&edge.source), name(&edge.target))
        })
        .collect();
    if edges.len() > 20 {
        parts.push(format!("… and {} more", edges.len() - 20));
    }
    parts.join("; ")
}

// WHAT:  The engine's own checks: no cycle, no node feeding itself, and Repeat
//        nodes that do not multiply into an absurd number of passes. Answers the
//        nodes by depth, which is also the layout.
// WHY:   Doing it here means the user is never asked to approve a workflow the
//        engine would refuse to run.
fn levels_of(nodes: &[WorkflowNode], edges: &[WorkflowEdge]) -> AppResult<Vec<Vec<String>>> {
    let graph = Graph::new(nodes, edges)?;
    let all: HashSet<String> = nodes.iter().map(|node| node.id.clone()).collect();
    let levels = graph.levels(&all)?;
    graph.plan(&graph.leaves())?;
    Ok(levels)
}

// WHAT:  Lay the graph out left to right by depth, as the canvas's "Tidy up" does.
// WHY:   A workflow written without positions would pile every node at 0, 0.
fn position(nodes: &mut [WorkflowNode], levels: &[Vec<String>]) {
    let mut spots: HashMap<&str, (f64, f64)> = HashMap::new();
    for (depth, level) in levels.iter().enumerate() {
        for (row, id) in level.iter().enumerate() {
            let x = ORIGIN + f64::from(u32::try_from(depth).unwrap_or(0)) * (NODE_WIDTH + COLUMN_GAP);
            let y = ORIGIN + f64::from(u32::try_from(row).unwrap_or(0)) * (NODE_HEIGHT + ROW_GAP);
            spots.insert(id.as_str(), (x, y));
        }
    }
    for node in nodes.iter_mut() {
        if let Some((x, y)) = spots.get(node.id.as_str()) {
            node.x = *x;
            node.y = *y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::workflow::{EmailConfig, ScheduleMode, SmtpSecurity};

    fn node(id: &str, config: Json) -> Json {
        json!({ "id": id, "config": config })
    }

    fn query(id: &str, sql: &str) -> Json {
        node(id, json!({ "kind": "query", "sql": sql }))
    }

    fn built(args: &Json) -> (Draft, WorkflowBody) {
        let draft = parse(args, None, &Remembered::default()).unwrap_or_else(|e| panic!("{e}"));
        let body = match &draft.body {
            DocumentBody::Workflow(body) => body.clone(),
            _ => panic!("not a workflow"),
        };
        (draft, body)
    }

    fn refused(args: &Json) -> String {
        parse(args, None, &Remembered::default()).err().map(|e| e.message().to_string()).unwrap_or_default()
    }

    fn email_base() -> EmailConfig {
        EmailConfig {
            host: "smtp.example.com".into(),
            port: 2525,
            security: SmtpSecurity::StartTls,
            username: "reports".into(),
            from: "reports@example.com".into(),
            password: NodeSecret { has_secret: true, inherit_last: true, ..NodeSecret::default() },
            ..EmailConfig::default()
        }
    }

    #[test]
    fn nodes_without_edges_are_chained_in_order_and_laid_out_left_to_right() {
        let (draft, body) = built(&json!({
            "name": "Nightly",
            "nodes": [
                query("count", "SELECT count(*) AS n FROM orders"),
                node("gate", json!({ "kind": "row_count_gate", "op": "gt", "value": 0 })),
                node("tell", json!({ "kind": "notification", "title": "Orders", "body": "{{n}} orders" })),
            ]
        }));
        assert_eq!(draft.summary, "3 nodes · 2 connections");
        assert_eq!(draft.worst, StatementKind::Read);
        assert_eq!(body.edges.iter().map(|e| (e.source.as_str(), e.target.as_str())).collect::<Vec<_>>(), vec![("count", "gate"), ("gate", "tell")]);
        assert!(body.steps.is_empty(), "the graph form, not the legacy list");
        let xs: Vec<f64> = body.nodes.iter().map(|n| n.x).collect();
        assert!(xs[0] < xs[1] && xs[1] < xs[2], "{xs:?}");
        assert_eq!((body.nodes[0].x, body.nodes[0].y), (80.0, 80.0));
        assert_eq!(draft.previews.last().map(String::as_str), Some("Connections: count → gate; gate → tell"));
    }

    #[test]
    fn a_branch_needs_a_side_on_each_edge_and_the_sides_sit_on_separate_rows() {
        let nodes = json!([
            query("q", "SELECT amount FROM orders"),
            node("big", json!({ "kind": "branch", "group": { "combinator": "and", "rules": [{ "field": "amount", "op": "gt", "value": "100" }] } })),
            node("yes", json!({ "kind": "notification", "title": "Big order" })),
            node("no", json!({ "kind": "notification", "title": "Small order" })),
        ]);
        let wired = json!({ "name": "w", "nodes": nodes, "edges": [
            { "source": "q", "target": "big" },
            { "source": "big", "target": "yes", "sourceHandle": "true" },
            { "source": "big", "target": "no", "sourceHandle": "false" },
        ]});
        let (_, body) = built(&wired);
        assert_eq!(body.edges[1].source_handle, Some(SourceHandle::True));
        assert_eq!(body.edges[2].id, "e-big-false-no");
        let yes = body.nodes.iter().find(|n| n.id == "yes").map(|n| (n.x, n.y));
        let no = body.nodes.iter().find(|n| n.id == "no").map(|n| (n.x, n.y));
        assert_eq!(yes.map(|p| p.0), no.map(|p| p.0), "same depth");
        assert_ne!(yes.map(|p| p.1), no.map(|p| p.1), "different rows");

        let no_side = json!({ "name": "w", "nodes": nodes, "edges": [{ "source": "q", "target": "big" }, { "source": "big", "target": "yes" }] });
        assert!(refused(&no_side).contains("true"), "{}", refused(&no_side));
        let chained = json!({ "name": "w", "nodes": nodes });
        assert!(refused(&chained).contains("edges"), "a branch cannot be chained blindly: {}", refused(&chained));
        let extra_side = json!({ "name": "w", "nodes": nodes, "edges": [{ "source": "q", "target": "big", "sourceHandle": "true" }] });
        assert!(refused(&extra_side).contains("single output"));
    }

    #[test]
    fn the_wiring_follows_the_canvas_rules() {
        let pair = |a: Json, b: Json| json!({ "name": "w", "nodes": [a, b], "edges": [{ "source": "a", "target": "b" }] });
        let save = node("a", json!({ "kind": "save_file", "format": "csv" }));
        let q = query("b", "SELECT 1");
        assert!(refused(&pair(save, q.clone())).contains("no output"));
        let note = node("a", json!({ "kind": "note", "text": "hi" }));
        assert!(refused(&pair(note, q.clone())).contains("no output"));
        let into_schedule = pair(query("a", "SELECT 1"), node("b", json!({ "kind": "schedule" })));
        assert!(refused(&into_schedule).contains("takes no input"));
        let into_read = pair(query("a", "SELECT 1"), node("b", json!({ "kind": "read_file", "path": "/tmp/x.csv" })));
        assert!(refused(&into_read).contains("takes no input"));
        let missing = json!({ "name": "w", "nodes": [query("a", "SELECT 1")], "edges": [{ "source": "a", "target": "ghost" }] });
        assert!(refused(&missing).contains("not a node"));
        let twice = json!({ "name": "w", "nodes": [query("a", "SELECT 1"), query("b", "SELECT 2")], "edges": [{ "source": "a", "target": "b" }, { "source": "a", "target": "b" }] });
        assert!(refused(&twice).contains("twice"));
        let itself = json!({ "name": "w", "nodes": [query("a", "SELECT 1")], "edges": [{ "source": "a", "target": "a" }] });
        assert!(refused(&itself).contains("feed itself"));
        // The model's own spelling of an edge is accepted.
        let spelled_out = json!({ "name": "w", "nodes": [query("a", "SELECT 1"), query("b", "SELECT 2")], "edges": [{ "from": "a", "to": "b" }] });
        assert_eq!(built(&spelled_out).1.edges.len(), 1);
    }

    #[test]
    fn a_cycle_is_refused_before_the_user_is_asked() {
        let ring = json!({ "name": "w",
            "nodes": [query("a", "SELECT 1"), node("b", json!({ "kind": "filter" })), node("c", json!({ "kind": "delay", "amount": 1 }))],
            "edges": [{ "source": "a", "target": "b" }, { "source": "b", "target": "c" }, { "source": "c", "target": "a" }] });
        assert!(refused(&ring).contains("cycle"), "{}", refused(&ring));
        let loops = json!({ "name": "w", "nodes": [node("r", json!({ "kind": "repeat", "count": 1000 })), node("r2", json!({ "kind": "repeat", "count": 1000 })),
            query("q", "SELECT 1")], "edges": [{ "source": "r", "target": "r2" }] });
        // repeat takes no input, so nesting one in another is a wiring error before it is a limit error.
        assert!(refused(&loops).contains("takes no input"));
    }

    #[test]
    fn what_the_model_may_not_decide_is_set_here() {
        let (draft, body) = built(&json!({ "name": "Purge", "nodes": [
            node("purge", json!({ "kind": "query", "sql": "DELETE FROM sessions", "allowDestructive": true, "connectionId": "other-db" })),
            node("add", json!({ "kind": "insert", "table": "audit", "connectionId": "other-db" })),
            node("out", json!({ "kind": "save_file", "format": "csv", "path": "/etc/passwd" })),
        ], "edges": [{ "source": "purge", "target": "add" }, { "source": "purge", "target": "out" }] }));
        let NodeConfig::Query(query) = &body.nodes[0].config else { panic!("expected a query") };
        assert!(!query.allow_destructive, "the opt-in is the user's own tick");
        assert!(query.connection_id.is_none(), "nodes run on the workflow's own connection");
        let NodeConfig::Insert(insert) = &body.nodes[1].config else { panic!("expected an insert") };
        assert!(insert.connection_id.is_none());
        let NodeConfig::SaveFile(save) = &body.nodes[2].config else { panic!("expected a save") };
        assert!(save.path.is_empty(), "the user picks where a file goes");
        assert_eq!(draft.worst, StatementKind::Destructive);
        assert!(draft.caveats.iter().any(|c| c.contains("Allow destructive statements")), "{:?}", draft.caveats);
        assert!(draft.previews[0].contains("DESTROYS"), "{:?}", draft.previews);
        assert!(draft.previews[1].contains("WRITES"));
    }

    #[test]
    fn a_tick_the_user_gave_survives_only_for_the_same_statement() {
        let saved = |sql: &str| {
            DocumentBody::Workflow(WorkflowBody {
                nodes: vec![WorkflowNode {
                    id: "purge".into(),
                    x: 0.0,
                    y: 0.0,
                    label: String::new(),
                    continue_on_error: false,
                    config: NodeConfig::Query(crate::model::workflow::QueryConfig { connection_id: None, sql: sql.into(), allow_destructive: true }),
                }],
                ..WorkflowBody::default()
            })
        };
        let args = |sql: &str| json!({ "name": "w", "nodes": [node("purge", json!({ "kind": "query", "sql": sql, "allowDestructive": true }))] });
        let ticked = |previous: &DocumentBody, sql: &str| {
            let draft = parse(&args(sql), Some(previous), &Remembered::default()).unwrap_or_else(|e| panic!("{e}"));
            let DocumentBody::Workflow(body) = &draft.body else { panic!("not a workflow") };
            let NodeConfig::Query(query) = &body.nodes[0].config else { panic!("expected a query") };
            query.allow_destructive
        };
        let previous = saved("DELETE FROM sessions");
        assert!(ticked(&previous, "  DELETE FROM sessions "), "the same statement keeps its tick");
        assert!(!ticked(&previous, "DELETE FROM users"), "a different statement does not inherit it");
        assert!(!parse(&args("DELETE FROM sessions"), None, &Remembered::default()).is_ok_and(|d| matches!(&d.body, DocumentBody::Workflow(b) if matches!(&b.nodes[0].config, NodeConfig::Query(q) if q.allow_destructive))), "a new workflow starts unticked");
    }

    #[test]
    fn an_insert_alone_is_a_write() {
        let (draft, _) = built(&json!({ "name": "w", "nodes": [query("q", "SELECT 1"), node("i", json!({ "kind": "insert", "schema": "app", "table": "t" }))] }));
        assert_eq!(draft.worst, StatementKind::Write);
        assert!(draft.previews[1].contains("app.t"));
    }

    #[test]
    fn email_and_slack_start_from_the_last_setup_and_never_take_a_secret_from_the_model() {
        let remembered = Remembered {
            email: Some(email_base()),
            slack: Some(SlackConfig { webhook: NodeSecret { has_secret: true, inherit_last: true, ..NodeSecret::default() }, text: String::new() }),
        };
        let args = json!({ "name": "w", "nodes": [
            query("q", "SELECT 1"),
            node("mail", json!({ "kind": "email", "to": "me@example.com", "subject": "Report", "body": "{{rows.table}}",
                "password": { "plain": "hunter2", "hasSecret": true } })),
            node("chat", json!({ "kind": "slack", "text": "done", "webhook": { "plain": "https://hooks.slack.com/x" } })),
        ], "edges": [{ "source": "q", "target": "mail" }, { "source": "q", "target": "chat" }] });
        let draft = parse(&args, None, &remembered).unwrap_or_else(|e| panic!("{e}"));
        let DocumentBody::Workflow(body) = &draft.body else { panic!("not a workflow") };
        let NodeConfig::Email(email) = &body.nodes[1].config else { panic!("expected an email") };
        assert_eq!((email.host.as_str(), email.port, email.username.as_str(), email.from.as_str()), ("smtp.example.com", 2525, "reports", "reports@example.com"));
        assert!(email.password.inherit_last && email.password.plain.is_none(), "the server copies the sealed login; the model's attempt is dropped");
        let NodeConfig::Slack(slack) = &body.nodes[2].config else { panic!("expected a slack node") };
        assert!(slack.webhook.inherit_last && slack.webhook.plain.is_none());
        assert!(draft.caveats.is_empty(), "nothing left for the user to enter: {:?}", draft.caveats);
        let json = serde_json::to_string(&draft.body).unwrap_or_default();
        assert!(!json.contains("hunter2") && !json.contains("hooks.slack.com"), "{json}");
    }

    #[test]
    fn without_an_earlier_setup_the_user_is_told_what_to_enter() {
        let args = json!({ "name": "w", "nodes": [
            query("q", "SELECT 1"),
            node("mail", json!({ "kind": "email", "to": "me@example.com", "subject": "Hi" })),
        ]});
        let (draft, body) = built(&args);
        let NodeConfig::Email(email) = &body.nodes[1].config else { panic!("expected an email") };
        assert!(!email.password.inherit_last && !email.password.has_secret);
        assert!(draft.caveats.iter().any(|c| c.contains("SMTP")), "{:?}", draft.caveats);
    }

    #[test]
    fn an_update_keeps_the_login_a_node_already_holds() {
        let mut saved_email = email_base();
        saved_email.password = NodeSecret { sealed: Some("c2VhbGVk".into()), has_secret: true, ..NodeSecret::default() };
        let saved = WorkflowBody {
            nodes: vec![WorkflowNode {
                id: "mail".into(),
                x: 0.0,
                y: 0.0,
                label: String::new(),
                continue_on_error: false,
                config: NodeConfig::Email(saved_email),
            }],
            schedules_active: true,
            ..WorkflowBody::default()
        };
        let previous = DocumentBody::Workflow(saved);
        let args = json!({ "name": "w", "nodes": [
            query("q", "SELECT 1"),
            node("mail", json!({ "kind": "email", "to": "me@example.com", "subject": "Hi", "host": "smtp.other.com" })),
            node("when", json!({ "kind": "schedule", "mode": "interval", "intervalAmount": 1, "intervalUnit": "hours" })),
        ], "edges": [{ "source": "q", "target": "mail" }, { "source": "when", "target": "q" }] });
        let draft = parse(&args, Some(&previous), &Remembered::default()).unwrap_or_else(|e| panic!("{e}"));
        let DocumentBody::Workflow(body) = &draft.body else { panic!("not a workflow") };
        let NodeConfig::Email(email) = &body.nodes[1].config else { panic!("expected an email") };
        assert!(email.password.has_secret, "the same node id keeps its saved login (the server copies it on save)");
        assert!(!draft.caveats.iter().any(|c| c.contains("password")), "{:?}", draft.caveats);
        assert!(body.schedules_active, "the assistant never turns schedules off or on");
        assert!(!draft.caveats.iter().any(|c| c.contains("paused")), "{:?}", draft.caveats);
    }

    #[test]
    fn a_schedule_is_saved_paused_and_is_validated() {
        let args = json!({ "name": "w", "nodes": [
            node("when", json!({ "kind": "schedule", "mode": "at_time", "time": "07:30", "days": ["mon", "fri"], "timezone": "Europe/Paris" })),
            query("q", "SELECT 1"),
        ]});
        let (draft, body) = built(&args);
        assert!(!body.schedules_active);
        assert!(draft.caveats.iter().any(|c| c.contains("paused")), "{:?}", draft.caveats);
        let NodeConfig::Schedule(schedule) = &body.nodes[0].config else { panic!("expected a schedule") };
        assert_eq!(schedule.mode, ScheduleMode::AtTime);

        let bad_time = json!({ "name": "w", "nodes": [node("when", json!({ "kind": "schedule", "mode": "at_time", "time": "25:99" })), query("q", "SELECT 1")] });
        assert!(refused(&bad_time).contains("HH:MM"), "{}", refused(&bad_time));
        let bad_zone = json!({ "name": "w", "nodes": [node("when", json!({ "kind": "schedule", "mode": "at_time", "time": "07:30", "timezone": "Mars/Base" })), query("q", "SELECT 1")] });
        assert!(refused(&bad_zone).contains("time zone"));
        let zero = json!({ "name": "w", "nodes": [node("when", json!({ "kind": "schedule", "intervalAmount": 0 })), query("q", "SELECT 1")] });
        assert!(refused(&zero).contains("at least 1"));
    }

    #[test]
    fn node_configs_are_checked_against_what_the_engine_can_run() {
        let one = |config: Json| json!({ "name": "w", "nodes": [node("x", config)] });
        assert!(refused(&one(json!({ "kind": "teleport" }))).contains("not a valid kind"));
        assert!(refused(&one(json!({}))).contains("kind"));
        assert!(refused(&one(json!({ "kind": "query" }))).contains("sql"));
        assert!(refused(&one(json!({ "kind": "http", "url": "ftp://x" }))).contains("http"));
        assert!(refused(&one(json!({ "kind": "http", "url": "https://x.io", "headers": "[1]" }))).contains("headers"));
        assert!(refused(&one(json!({ "kind": "read_file" }))).contains("path"));
        assert!(refused(&one(json!({ "kind": "insert" }))).contains("table"));
        assert!(refused(&one(json!({ "kind": "repeat", "count": 0 }))).contains("between 1 and"));
        assert!(refused(&one(json!({ "kind": "regex", "sourceField": "a", "pattern": "(" }))).contains("not valid"));
        assert!(refused(&one(json!({ "kind": "regex", "sourceField": "a" }))).contains("pattern"));
        assert!(refused(&one(json!({ "kind": "javascript", "code": "  " }))).contains("code"));
        assert!(refused(&one(json!({ "kind": "filter", "group": { "rules": [{ "field": "", "op": "eq", "value": "1" }] } }))).contains("field"));
        assert!(refused(&one(json!({ "kind": "filter", "group": { "rules": [{ "field": "a", "op": "eq" }] } }))).contains("value"));
        assert!(refused(&one(json!({ "kind": "filter", "group": { "rules": [{ "field": "a", "op": "like", "value": "1" }] } }))).contains("not valid"));
        assert!(refused(&one(json!({ "kind": "notification" }))).contains("title"));
        assert!(refused(&one(json!({ "kind": "slack" }))).contains("text"));
        assert!(refused(&one(json!({ "kind": "email", "subject": "x" }))).contains("to"));
        assert!(refused(&one(json!({ "kind": "note", "text": "only a note" }))).contains("not only notes"));
        assert!(refused(&json!({ "name": "w", "nodes": [{ "id": "x" }] })).contains("config"));

        let ok = |config: Json| parse(&one(config), None, &Remembered::default()).is_ok();
        assert!(ok(json!({ "kind": "filter", "group": { "combinator": "or", "rules": [{ "field": "a", "op": "is_null" }] } })));
        assert!(ok(json!({ "kind": "http", "method": "post", "url": "https://api.example.com/x?y=1", "headers": "{\"Authorization\": \"Bearer {{token}}\"}" })));
        assert!(ok(json!({ "kind": "regex", "sourceField": "email", "pattern": "@(.+)$", "flags": "i" })));
        assert!(ok(json!({ "kind": "javascript", "code": "return rows;" })));
    }

    #[test]
    fn ids_are_unique_and_generated_when_missing() {
        let dup = json!({ "name": "w", "nodes": [query("a", "SELECT 1"), query("a", "SELECT 2")] });
        assert!(refused(&dup).contains("share the id"));
        let odd = json!({ "name": "w", "nodes": [query("has space", "SELECT 1")] });
        assert!(refused(&odd).contains("letters, digits"));
        let generated = json!({ "name": "w", "nodes": [query("n2", "SELECT 2"), { "config": { "kind": "query", "sql": "SELECT 1" } }, { "config": { "kind": "query", "sql": "SELECT 3" } }] });
        let (_, body) = built(&generated);
        let ids: Vec<&str> = body.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["n2", "n3", "n4"], "a generated id skips the ones taken");
    }

    #[test]
    fn notes_are_kept_but_never_wired() {
        let args = json!({ "name": "w", "nodes": [
            node("why", json!({ "kind": "note", "text": "Runs after the nightly load" })),
            query("q", "SELECT 1"),
            node("tell", json!({ "kind": "notification", "title": "Done" })),
        ]});
        let (draft, body) = built(&args);
        assert_eq!(draft.summary, "2 nodes · 1 connection", "a note is not counted as a node");
        assert_eq!(body.nodes.len(), 3);
        assert_eq!(body.edges.len(), 1);
    }

    #[test]
    fn the_http_hosts_are_called_out_for_the_user() {
        let args = json!({ "name": "w", "nodes": [
            node("get", json!({ "kind": "http", "url": "https://user:pw@api.example.com:8443/v1/items?x=1" })),
            node("again", json!({ "kind": "http", "url": "https://api.example.com/v2" })),
        ]});
        let (draft, _) = built(&args);
        assert!(draft.caveats.iter().any(|c| c.contains("api.example.com:8443") && c.contains("api.example.com")), "{:?}", draft.caveats);
        assert_eq!(host_of("https://user:pw@api.example.com:8443/v1"), "api.example.com:8443");
        assert_eq!(host_of("http://localhost"), "localhost");
    }
}
