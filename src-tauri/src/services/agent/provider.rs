// SOT: agent-provider, rig-adapter, llm-transcript, llm-streaming, tool-definition-wire

use crate::error::{AppError, AppResult};
use crate::model::{AiProvider, AiSettings};
use futures::StreamExt;
use rig_core::client::CompletionClient;
use rig_core::completion::message::{AssistantContent, Message, ToolCall, ToolResultContent, UserContent};
use rig_core::completion::{CompletionModel, ToolDefinition};
use rig_core::providers::{anthropic, ollama, openai, openrouter};
use rig_core::streaming::StreamedAssistantContent;

// WHAT:  The only file that speaks to an LLM vendor. Wraps rig-core so the loop
//        above it works in this codebase's own types.
// WHY:   Four providers with three different tool-calling wire shapes. rig gives
//        one `CompletionModel` over all of them and — the part worth the
//        dependency — reassembles tool-call arguments that arrive in fragments,
//        so the loop always sees a whole call.
// HOW:   `Transcript` owns the message history as rig types and never lends them
//        out; callers exchange `PendingCall` / `ToolOutput`, which carry the rig
//        handle privately so a result can be matched back to its call.
// WHERE: src-tauri/src/services/agent/mod.rs (the loop), scripts/guardrail.py
//        (rig_core is scoped to this file)

/// Ceiling for one assistant turn. Generous: the agent writes prose *and* SQL.
const MAX_TOKENS: u64 = 8192;

/// What the stream produced between tool calls, pushed to the UI as it arrives.
pub enum StreamDelta<'a> {
    /// Assistant prose. Arrives token by token and renders as markdown live.
    Text(&'a str),
    /// Summarized reasoning, when the provider returns any.
    Reasoning(&'a str),
}

/// A tool call the model asked for, waiting to be dispatched.
pub struct PendingCall {
    /// Stable handle the UI correlates its timeline rows by.
    pub ui_id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    /// The rig call this answers. Private: replaying a result needs the
    /// provider's own identifiers, which never leave this module.
    inner: ToolCall,
}

/// The result of running one `PendingCall`.
pub struct ToolOutput {
    pub call: PendingCall,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// One assistant turn: its prose, the calls it wants run, and what it cost.
pub struct ModelTurn {
    pub text: String,
    pub calls: Vec<PendingCall>,
    pub usage: TurnUsage,
}

impl ModelTurn {
    /// A turn with no tool calls is the end of the run.
    pub fn wants_tools(&self) -> bool {
        !self.calls.is_empty()
    }
}

// WHAT:  One model, whichever vendor backs it.
// WHY:   `CompletionModel` returns `impl Future`, so it is not object safe and
//        cannot be a `Box<dyn …>`. An enum keeps one concrete type.
pub enum Chat {
    Anthropic(anthropic::completion::CompletionModel),
    Openai(openai::completion::CompletionModel),
    Openrouter(openrouter::completion::CompletionModel),
    Ollama(ollama::CompletionModel),
}

impl Chat {
    // WHAT:  Build the model named in Settings → AI.
    // WHY:   The key is unsealed per request and never stored on the model.
    pub fn build(settings: &AiSettings, api_key: Option<&str>) -> AppResult<Chat> {
        let model = settings.model.trim();
        if model.is_empty() {
            return Err(AppError::invalid_input("Set an AI model in Settings → AI."));
        }
        let base = settings.base_url.as_deref().map(str::trim).filter(|u| !u.is_empty());

        match settings.provider {
            AiProvider::None => Err(AppError::invalid_input("Choose an AI provider in Settings → AI first.")),
            AiProvider::Anthropic => {
                let key = require_key(api_key, "Anthropic")?;
                let mut builder = anthropic::Client::builder().api_key(key);
                if let Some(url) = base {
                    builder = builder.base_url(url);
                }
                let client = builder.build().map_err(client_error)?;
                Ok(Chat::Anthropic(client.completion_model(model)))
            }
            AiProvider::Openai => {
                let key = require_key(api_key, "OpenAI")?;
                // `openai::Client` is the Responses API flavour; Chat Completions
                // is a separate client type. See the note below on why.
                let mut builder = openai::CompletionsClient::builder().api_key(key);
                if let Some(url) = base {
                    builder = builder.base_url(url);
                }
                let client = builder.build().map_err(client_error)?;
                // Chat Completions, not the Responses API: the Base URL setting
                // exists so people can point this at an OpenAI-*compatible*
                // proxy, and Chat Completions is the shape those implement.
                Ok(Chat::Openai(openai::completion::CompletionModel::new(client, model)))
            }
            AiProvider::Openrouter => {
                let key = require_key(api_key, "OpenRouter")?;
                let mut builder = openrouter::Client::builder().api_key(key);
                if let Some(url) = base {
                    builder = builder.base_url(url);
                }
                let client = builder.build().map_err(client_error)?;
                Ok(Chat::Openrouter(client.completion_model(model)))
            }
            AiProvider::Ollama => {
                // Local daemon: no credential, but the builder still wants the slot filled.
                let mut builder = ollama::Client::builder().api_key("");
                builder = builder.base_url(base.unwrap_or("http://127.0.0.1:11434"));
                let client = builder.build().map_err(client_error)?;
                Ok(Chat::Ollama(client.completion_model(model)))
            }
        }
    }
}

fn require_key<'a>(api_key: Option<&'a str>, provider: &str) -> AppResult<&'a str> {
    match api_key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(key) => Ok(key),
        None => Err(AppError::invalid_input(format!(
            "Add your {provider} API key in Settings → AI."
        ))),
    }
}

fn client_error(err: impl std::fmt::Display) -> AppError {
    AppError::driver(format!("Could not reach the AI provider: {err}"))
}

// WHAT:  The conversation, in the provider's own message shapes.
// WHY:   Held server-side for the life of a chat so the stable prefix (system
//        prompt, tool list, earlier turns) stays byte-identical between
//        requests, which is what makes provider-side prompt caching hit.
#[derive(Default)]
pub struct Transcript {
    messages: Vec<Message>,
    /// Monotonic, so every call this chat ever made has a distinct UI handle.
    call_seq: u64,
}

impl Transcript {
    pub fn new() -> Transcript {
        Transcript::default()
    }

    pub fn push_user(&mut self, text: &str) {
        self.messages.push(Message::user(text));
    }

    /// Turns so far. The loop uses it to decide when to compact.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Drop everything. "Clear conversation" in the UI.
    pub fn reset(&mut self) {
        self.messages.clear();
    }

    // WHAT:  Keep the transcript bounded by dropping the oldest exchanges.
    // WHY:   A long session would otherwise grow past the context window. The
    //        first message is kept because dropping it can orphan a tool result.
    pub fn trim_to(&mut self, keep: usize) {
        if self.messages.len() > keep {
            let drop = self.messages.len() - keep;
            self.messages.drain(..drop);
            // A transcript must not open on a tool result with no call before it.
            while matches!(self.messages.first(), Some(Message::User { content }) if content
                .iter()
                .any(|c| matches!(c, UserContent::ToolResult(_))))
            {
                self.messages.remove(0);
            }
        }
    }

    /// Record the answers and hand the turn back to the model.
    pub fn push_tool_outputs(&mut self, outputs: Vec<ToolOutput>) {
        if outputs.is_empty() {
            return;
        }
        let content: Vec<UserContent> = outputs
            .into_iter()
            .map(|out| {
                let text = if out.is_error {
                    format!("ERROR: {}", out.content)
                } else {
                    out.content
                };
                UserContent::tool_result_for(
                    out.call.inner.id.clone(),
                    out.call.inner.provider.clone(),
                    out.call.name.clone(),
                    vec![ToolResultContent::text(text)],
                )
            })
            .collect();
        self.messages.push(Message::User { content });
    }

    // WHAT:  One model round trip, streamed.
    // WHY:   The UI shows prose as it is written; waiting for the whole turn is
    //        what made the old assistant feel dead for ten seconds at a time.
    // HOW:   rig hands us whole `ToolCall`s even when the wire fragments their
    //        arguments, so nothing here parses partial JSON. The assistant
    //        message is rebuilt from what streamed and appended, because the
    //        next request must replay the calls it is answering.
    pub async fn run_turn(
        &mut self,
        chat: &Chat,
        system: &str,
        tools: &[ToolDefinition],
        // `+ Send`: this is held across every await below, and a tauri command's
        // future must be `Send`.
        on_delta: &mut (dyn FnMut(StreamDelta<'_>) + Send),
    ) -> AppResult<ModelTurn> {
        // The last message is the turn being answered; everything before it is
        // history. rig appends the prompt after the history, so handing it a
        // placeholder here would post an empty user turn — which providers reject.
        let Some((prompt, history)) = self.messages.split_last() else {
            return Err(AppError::internal("the agent tried to run with no messages"));
        };
        let prompt = prompt.clone();
        let history = history.to_vec();

        let mut text = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut usage = TurnUsage::default();

        macro_rules! drive {
            ($model:expr) => {{
                let request = $model
                    .completion_request(prompt.clone())
                    .preamble(system.to_string())
                    .messages(history.to_vec())
                    .tools(tools.to_vec())
                    .max_tokens(MAX_TOKENS)
                    .build();
                let mut stream = $model.stream(request).await.map_err(completion_error)?;
                while let Some(item) = stream.next().await {
                    match item.map_err(completion_error)? {
                        StreamedAssistantContent::Text(part) => {
                            if !part.text.is_empty() {
                                on_delta(StreamDelta::Text(&part.text));
                                text.push_str(&part.text);
                            }
                        }
                        StreamedAssistantContent::ToolCall { tool_call, .. } => calls.push(tool_call),
                        StreamedAssistantContent::Reasoning { reasoning, .. } => {
                            let summary = reasoning_text(&reasoning);
                            if !summary.is_empty() {
                                on_delta(StreamDelta::Reasoning(&summary));
                            }
                        }
                        StreamedAssistantContent::Final(final_part) => {
                            usage.input_tokens = final_part.usage.input_tokens;
                            usage.output_tokens = final_part.usage.output_tokens;
                        }
                        // Argument fragments are reassembled by rig into the
                        // `ToolCall` above; nothing to do with them here.
                        _ => {}
                    }
                }
            }};
        }

        match chat {
            Chat::Anthropic(model) => drive!(model),
            Chat::Openai(model) => drive!(model),
            Chat::Openrouter(model) => drive!(model),
            Chat::Ollama(model) => drive!(model),
        }

        // Replay fidelity: the assistant turn goes back exactly as it came.
        let mut blocks: Vec<AssistantContent> = Vec::new();
        if !text.trim().is_empty() {
            blocks.push(AssistantContent::text(text.clone()));
        }
        for call in &calls {
            blocks.push(AssistantContent::ToolCall(call.clone()));
        }
        if !blocks.is_empty() {
            self.messages.push(Message::Assistant { id: None, content: blocks });
        }

        let pending = calls
            .into_iter()
            .map(|call| {
                self.call_seq += 1;
                PendingCall {
                    ui_id: format!("call-{}", self.call_seq),
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                    inner: call,
                }
            })
            .collect();

        Ok(ModelTurn { text, calls: pending, usage })
    }
}

fn reasoning_text(reasoning: &rig_core::completion::message::Reasoning) -> String {
    reasoning
        .content
        .iter()
        .filter_map(|block| match block {
            rig_core::completion::message::ReasoningContent::Text { text, .. } => Some(text.as_str()),
            rig_core::completion::message::ReasoningContent::Summary(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

// WHAT:  Provider failures become the app's own error kinds.
// WHY:   The UI switches on `kind`; a raw vendor string would be untyped noise.
fn completion_error(err: rig_core::completion::CompletionError) -> AppError {
    let text = err.to_string();
    let lowered = text.to_lowercase();
    if lowered.contains("401") || lowered.contains("unauthorized") || lowered.contains("invalid api key") {
        AppError::invalid_input(format!("The AI provider rejected the key: {text}"))
    } else if lowered.contains("429") || lowered.contains("rate limit") {
        AppError::driver(format!("The AI provider is rate limiting: {text}"))
    } else if lowered.contains("timed out") || lowered.contains("timeout") {
        AppError::timeout(format!("The AI provider timed out: {text}"))
    } else {
        AppError::driver(format!("AI request failed: {text}"))
    }
}

/// Build a tool the model can call. `parameters` is a JSON Schema object.
pub fn tool(name: &str, description: &str, parameters: serde_json::Value) -> ToolDefinition {
    ToolDefinition { name: name.to_string(), description: description.to_string(), parameters }
}

/// Re-exported so the tool registry can name the type without importing rig.
pub type ToolSpec = ToolDefinition;
