// SOT: workflow-notification-node, workflow-email-node-runner, workflow-slack-node, templated-messages

use super::{or_stopped, NodeCtx, NodeFailure, NodeOutput, NodeResult};
use crate::integrations::http::{self, OutboundRequest};
use crate::integrations::smtp::{self, EmailRequest};
use crate::model::workflow::{EmailConfig, NotificationConfig, SlackConfig};
use crate::services::workflows::{secrets, template};

// WHAT:  The Desktop notification node: a native notification with a templated title and body.
// WHY:   With no input rows the placeholders render empty, so a manual test fire still works.
pub fn notification(ctx: &NodeCtx<'_>, config: &NotificationConfig) -> NodeResult {
    let title = template::render_text(config.title.trim(), ctx.input);
    if config.title.trim().is_empty() {
        return Err(NodeFailure::new("Title is empty."));
    }
    let body = template::render_text(config.body.trim(), ctx.input);
    ctx.host.notify(title.trim(), body.trim())?;
    Ok(NodeOutput::sink(ctx.input.row_count()))
}

// WHAT:  The Email node: one message per run through the node's SMTP server.
// HOW:   To, subject and body are templated (`{{rows.html}}` for a table). The
//        password is decrypted here, for this call only, and never leaves it.
pub async fn email(ctx: &NodeCtx<'_>, config: &EmailConfig) -> NodeResult {
    if config.host.trim().is_empty() {
        return Err(NodeFailure::new("SMTP host is empty."));
    }
    if config.from.trim().is_empty() {
        return Err(NodeFailure::new("From address is empty."));
    }
    if config.to.trim().is_empty() {
        return Err(NodeFailure::new("To address is empty."));
    }
    if config.port == 0 {
        return Err(NodeFailure::new("The SMTP port must be between 1 and 65535."));
    }
    let password = secrets::reveal(ctx.state, &config.password)?;
    let request = EmailRequest {
        host: config.host.trim().to_string(),
        port: config.port,
        security: config.security,
        username: Some(config.username.trim().to_string()).filter(|u| !u.is_empty()),
        password,
        from: config.from.trim().to_string(),
        to: template::render_text(config.to.trim(), ctx.input),
        subject: template::render_text(config.subject.trim(), ctx.input),
        body: template::render_text(config.body.trim(), ctx.input),
    };
    or_stopped(ctx.cancel, smtp::send(&request)).await??;
    Ok(NodeOutput::sink(ctx.input.row_count()))
}

// WHAT:  The Slack node: posts `{ "text": … }` to the incoming-webhook URL.
pub async fn slack(ctx: &NodeCtx<'_>, config: &SlackConfig) -> NodeResult {
    let Some(webhook) = secrets::reveal(ctx.state, &config.webhook)?.map(|w| w.trim().to_string()).filter(|w| !w.is_empty()) else {
        return Err(NodeFailure::new("Webhook URL is empty."));
    };
    if config.text.trim().is_empty() {
        return Err(NodeFailure::new("Message is empty."));
    }
    let text = template::render_text(config.text.trim(), ctx.input);
    let payload = serde_json::json!({ "text": text }).to_string();
    let request = OutboundRequest {
        method: "POST".to_string(),
        url: webhook,
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: Some(payload),
    };
    let reply = or_stopped(ctx.cancel, http::send_outbound(&request)).await??;
    if !(200..300).contains(&reply.status) {
        let detail = reply.body.trim();
        let tail = if detail.is_empty() { String::new() } else { format!(": {detail}") };
        return Err(NodeFailure::new(format!("Slack responded {} {}{tail}", reply.status, reply.status_text)));
    }
    Ok(NodeOutput::sink(ctx.input.row_count()))
}
