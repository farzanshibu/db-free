// SOT: chat-message-model, chat-thread-restore, chat-stop-notes, chat-relative-time
import type { AgentArtifact, AgentStop, AiChatMessage, ChatMention, ToolCallRecord } from "@/lib/bindings";

// WHAT:  The shapes the chat thread renders, and the conversion from a saved chat.
// WHY:   A live message is built up from stream events; a saved one arrives whole.
//        Both end as the same `ChatMessageView`, so restoring a chat and watching
//        one stream in render through exactly the same components.
// WHERE: src/features/chat/ChatTab.tsx, src-tauri/src/model/agent.rs (AiChatMessage)

export interface UserMessage {
  kind: "user";
  id: string;
  text: string;
  mentions: ChatMention[];
  at: number;
}

export interface AssistantMessage {
  kind: "assistant";
  id: string;
  runId: string;
  text: string;
  thinking: string;
  calls: ToolCallRecord[];
  artifacts: AgentArtifact[];
  sql: string | null;
  stop: AgentStop | null;
  /// Model round trips the run took; known once it has finished.
  steps: number | null;
  streaming: boolean;
  error: string | null;
  model: string | null;
  at: number;
}

export type ChatMessageView = UserMessage | AssistantMessage;

function timeOf(iso: string): number {
  const parsed = Date.parse(iso);
  return Number.isNaN(parsed) ? Date.now() : parsed;
}

/// A message that is about to stream: empty, waiting for its first event.
export function pendingAssistant(runId: string, at: number): AssistantMessage {
  return {
    kind: "assistant",
    id: `a-${runId}`,
    runId,
    text: "",
    thinking: "",
    calls: [],
    artifacts: [],
    sql: null,
    stop: null,
    steps: null,
    streaming: true,
    error: null,
    model: null,
    at,
  };
}

export function fromSaved(saved: readonly AiChatMessage[]): ChatMessageView[] {
  return saved.map((message): ChatMessageView => {
    const at = timeOf(message.createdAt);
    if (message.role === "user") {
      return { kind: "user", id: message.id, text: message.text, mentions: message.mentions, at };
    }
    const turn = message.turn;
    return {
      kind: "assistant",
      id: message.id,
      runId: message.id.startsWith("a-") ? message.id.slice(2) : message.id,
      text: turn?.text ?? message.text,
      thinking: "",
      calls: turn?.toolCalls ?? [],
      artifacts: turn?.artifacts ?? [],
      sql: turn?.sql ?? null,
      stop: turn?.stop ?? null,
      steps: turn?.usage.steps ?? null,
      streaming: false,
      error: message.error,
      model: turn?.model ?? null,
      at,
    };
  });
}

export function stopNote(stop: AgentStop | null, steps: number | null = null): string | null {
  switch (stop) {
    case "max_steps":
      return steps === null ? "Stopped at the step limit." : `Stopped after ${steps} steps, the step limit.`;
    case "cancelled":
      return "Stopped.";
    case "refusal":
      return "The model declined this request.";
    case "end_turn":
    case null:
      return null;
  }
}

/// "just now", "5m ago", "3h ago", "2d ago", "4mo ago".
export function relativeTime(iso: string, now = Date.now()): string {
  const minutes = Math.floor((now - timeOf(iso)) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return `${Math.floor(days / 30)}mo ago`;
}
