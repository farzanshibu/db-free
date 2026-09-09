// SOT: chat-tab, ai-database-conversation, agent-streaming-chat, agent-permission-ui, agent-artifacts
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  AgentArtifact,
  AgentEvent,
  AgentStop,
  PermissionDecision,
  PermissionRequest,
  ToolCallRecord,
} from "@/lib/bindings";
import { ipc, normalizeError, onAgentEvent } from "@/lib/ipc";
import { engineMeta } from "@/lib/engines";
import { useWorkspace } from "@/stores/workspace";
import { IconButton } from "@/components/global/Button";
import { Icon } from "@/lib/icons";
import { Markdown } from "./Markdown";
import { ArtifactView } from "./ArtifactView";
import { PermissionPrompt, ToolTrace } from "./ToolTrace";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Conversation, ConversationContent, ConversationEmptyState } from "@/components/ui/conversation";
import { Message, MessageContent } from "@/components/ui/message";
import { PromptInput, PromptInputBody, PromptInputSubmit, PromptInputTextarea } from "@/components/ui/prompt-input";
import { Suggestion, Suggestions } from "@/components/ui/suggestion";

// WHAT:  Conversation with the connected database, driven by the agent loop.
// WHY:   The old chat sent the whole schema with every message, waited in
//        silence for one blocking reply, and printed it as plain text. This one
//        streams: prose renders as markdown while it is written, each tool call
//        appears as it runs, anything that would write stops and asks, and
//        charts the assistant drew are rendered inline and stay interactive.
// HOW:   `agent_chat` returns the finished turn; everything before that arrives
//        on the `agent:event` stream and is folded into the pending message.
// WHERE: src-tauri/src/commands/agent.rs, src/features/chat/{Markdown,ToolTrace,ArtifactView}.tsx

interface UserMessage {
  kind: "user";
  id: string;
  text: string;
  at: number;
}

interface AssistantMessage {
  kind: "assistant";
  id: string;
  runId: string;
  text: string;
  thinking: string;
  calls: ToolCallRecord[];
  artifacts: AgentArtifact[];
  sql: string | null;
  stop: AgentStop | null;
  streaming: boolean;
  error: string | null;
  at: number;
}

type Message = UserMessage | AssistantMessage;

/// Survives a tab switch within the session; the transcript itself lives in Rust.
const threadCache = new Map<string, Message[]>();

const STARTERS = [
  "What tables are in this database and how do they relate?",
  "Show me the 10 biggest tables by row count",
  "Chart new records per month for the main table",
  "Find any table missing a primary key",
] as const;

function stopNote(stop: AgentStop | null): string | null {
  switch (stop) {
    case "max_steps":
      return "Stopped after the step limit — ask a narrower question to go further.";
    case "cancelled":
      return "Stopped.";
    case "refusal":
      return "The model declined this request.";
    default:
      return null;
  }
}

export function ChatTab({ connectionId }: { connectionId: string }) {
  const connections = useWorkspace((s) => s.connections);
  const catalogs = useWorkspace((s) => s.catalogs);
  const settings = useWorkspace((s) => s.settings);
  const openQuery = useWorkspace((s) => s.openQuery);
  const goSettings = useWorkspace((s) => s.goSettings);
  const showInfo = useWorkspace((s) => s.showInfo);
  const showError = useWorkspace((s) => s.showError);

  const connection = useMemo(
    () => connections.find((c) => c.id === connectionId),
    [connections, connectionId],
  );
  const meta = useMemo(() => (connection ? engineMeta(connection.engine) : null), [connection]);
  const catalog = catalogs[connectionId];
  const tableCount = useMemo(
    () => catalog?.schemas.reduce((total, schema) => total + schema.tables.length, 0) ?? 0,
    [catalog],
  );

  const chatId = `chat:${connectionId}`;
  const aiConfigured = Boolean(settings?.ai && settings.ai.provider !== "none");
  const autonomy = settings?.ai.autonomy ?? "ask_on_write";

  const [messages, setMessages] = useState<Message[]>(() => threadCache.get(chatId) ?? []);
  const [input, setInput] = useState("");
  const [runId, setRunId] = useState<string | null>(null);
  const [permission, setPermission] = useState<PermissionRequest | null>(null);
  const endRef = useRef<HTMLDivElement>(null);

  // The listener is registered once and reads the live run through this ref, so
  // it never needs re-subscribing mid-stream. Written only in `send`, never
  // during render.
  const runIdRef = useRef<string | null>(null);

  useEffect(() => {
    threadCache.set(chatId, messages);
  }, [chatId, messages]);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, permission]);

  const patchAssistant = useCallback(
    (targetRun: string, apply: (message: AssistantMessage) => AssistantMessage) => {
      setMessages((prev) =>
        prev.map((message) =>
          message.kind === "assistant" && message.runId === targetRun ? apply(message) : message,
        ),
      );
    },
    [],
  );

  useEffect(() => {
    const pending = onAgentEvent((event: AgentEvent) => {
      // Frames from a run this tab no longer shows are dropped, not rendered.
      if (event.runId !== runIdRef.current) return;
      switch (event.type) {
        case "text":
          patchAssistant(event.runId, (m) => ({ ...m, text: m.text + event.delta }));
          break;
        case "thinking":
          patchAssistant(event.runId, (m) => ({ ...m, thinking: m.thinking + event.delta }));
          break;
        case "tool_started":
          patchAssistant(event.runId, (m) => ({ ...m, calls: [...m.calls, event.call] }));
          break;
        case "tool_finished":
          patchAssistant(event.runId, (m) => ({
            ...m,
            calls: m.calls.map((call) => (call.id === event.call.id ? event.call : call)),
          }));
          break;
        case "artifact":
          patchAssistant(event.runId, (m) => ({ ...m, artifacts: [...m.artifacts, event.artifact] }));
          break;
        case "permission":
          setPermission(event.request);
          break;
        case "finished":
          setPermission(null);
          patchAssistant(event.runId, (m) => ({
            ...m,
            text: event.turn.text.length > 0 ? event.turn.text : m.text,
            sql: event.turn.sql,
            stop: event.turn.stop,
            calls: event.turn.toolCalls.length > 0 ? event.turn.toolCalls : m.calls,
            // The finished turn is authoritative: if a frame was dropped while
            // the window was busy, this is where the message catches up.
            artifacts: m.artifacts.length > 0 ? m.artifacts : event.turn.artifacts,
            streaming: false,
          }));
          break;
        case "failed":
          setPermission(null);
          patchAssistant(event.runId, (m) => ({ ...m, streaming: false, error: event.message }));
          break;
        case "started":
        case "step":
        case "usage":
          break;
      }
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [patchAssistant]);

  const send = useCallback(
    async (override?: string) => {
      const text = (override ?? input).trim();
      if (text.length === 0 || runId !== null || !connection) return;

      const nextRun = crypto.randomUUID();
      const now = Date.now();
      setMessages((prev) => [
        ...prev,
        { kind: "user", id: `u-${now}`, text, at: now },
        {
          kind: "assistant",
          id: `a-${now}`,
          runId: nextRun,
          text: "",
          thinking: "",
          calls: [],
          artifacts: [],
          sql: null,
          stop: null,
          streaming: true,
          error: null,
          at: now,
        },
      ]);
      if (override === undefined) setInput("");
      setRunId(nextRun);
      runIdRef.current = nextRun;

      try {
        await ipc("agent_chat", {
          connectionId: connection.id,
          chatId,
          runId: nextRun,
          prompt: text,
          context: null,
          useTools: true,
        });
      } catch (raw) {
        const error = normalizeError(raw);
        patchAssistant(nextRun, (m) => ({ ...m, streaming: false, error: error.message }));
        showError(error);
      } finally {
        setRunId(null);
        runIdRef.current = null;
        setPermission(null);
      }
    },
    [chatId, connection, input, patchAssistant, runId, showError],
  );

  const decide = useCallback(
    (decision: PermissionDecision) => {
      if (runId === null) return;
      setPermission(null);
      void (async () => {
        try {
          await ipc("agent_decide", { runId, decision });
        } catch (raw) {
          showError(normalizeError(raw));
        }
      })();
    },
    [runId, showError],
  );

  const stop = useCallback(() => {
    if (runId === null) return;
    void (async () => {
      try {
        await ipc("agent_cancel", { runId });
      } catch (raw) {
        showError(normalizeError(raw));
      }
    })();
  }, [runId, showError]);

  const clear = useCallback(() => {
    setMessages([]);
    threadCache.delete(chatId);
    void (async () => {
      try {
        await ipc("agent_reset", { chatId });
      } catch (raw) {
        showError(normalizeError(raw));
      }
    })();
    showInfo("Conversation cleared.");
  }, [chatId, showError, showInfo]);

  const copy = useCallback(
    (code: string) => {
      void navigator.clipboard.writeText(code);
      showInfo("Copied to clipboard.");
    },
    [showInfo],
  );

  const runInEditor = useCallback(
    (code: string) => {
      if (!connection) return;
      openQuery(connection.id, code, "From chat");
    },
    [connection, openQuery],
  );

  if (!connection || !meta) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-muted">
        <span>Connection not found.</span>
      </div>
    );
  }

  const busy = runId !== null;

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col bg-background select-none">
      {/* Toolbar */}
      <div className="flex app-toolbar shrink-0 items-center justify-between gap-2 border-b border-border/40 glass-header">
        <div className="flex min-w-0 items-center gap-2">
          <div className="flex size-6 shrink-0 items-center justify-center rounded-lg bg-accent/15 text-accent">
            <Icon name="sparkles" size={13} />
          </div>
          <span className="truncate text-xs font-semibold tracking-tight text-foreground">
            Chat with {connection.name}
          </span>
          <Badge size="sm" variant="secondary" className="hidden h-5 shrink-0 px-2 text-[10px] glass-pill text-muted sm:flex">
            {meta.label}
          </Badge>
          <span className="hidden shrink-0 text-[11px] text-muted lg:inline">
            {tableCount > 0 ? `${tableCount} tables` : "catalog loading…"}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {connection.readOnly ? (
            <Badge size="sm" variant="secondary" className="h-5 px-2 text-[10px] glass-pill text-muted">
              <Icon name="lock" size={9} />
              Read-only
            </Badge>
          ) : (
            <Badge
              size="sm"
              variant="secondary"
              className="hidden h-5 px-2 text-[10px] glass-pill text-muted md:flex"
            >
              <Icon name="shield" size={9} />
              {autonomy === "read_only" ? "Reads only" : autonomy === "full" ? "Writes allowed" : "Asks before writing"}
            </Badge>
          )}
          {busy ? (
            <Button
              size="sm"
              variant="secondary"
              className="h-7 rounded-lg px-2.5 text-xs font-medium glass-pill text-danger liquid-hover"
              onClick={stop}
            >
              <Icon name="x" size={11} />
              Stop
            </Button>
          ) : null}
          <IconButton
            icon="trash"
            label="Clear conversation"
            disabled={messages.length === 0 || busy}
            onClick={clear}
            size={14}
          />
        </div>
      </div>

      {!aiConfigured ? (
        <div className="p-3">
          <Alert variant="warning" className="rounded-xl border-warning/30 glass-modal">
            <AlertIndicator />
            <AlertContent>
              <AlertTitle className="text-xs font-semibold">No AI provider configured</AlertTitle>
              <AlertDescription className="text-xs text-muted">
                Add a provider and key in Settings to chat with this database.
              </AlertDescription>
            </AlertContent>
            <Button size="sm" variant="secondary" onClick={goSettings} className="ml-auto glass-pill text-xs">
              Open Settings
            </Button>
          </Alert>
        </div>
      ) : null}

      {/* Thread */}
      <Conversation>
        {messages.length === 0 ? (
          <ConversationEmptyState
            icon={
              <div className="flex size-12 items-center justify-center rounded-2xl bg-accent/15 text-accent shadow-lg shadow-accent/10">
                <Icon name="sparkles" size={22} />
              </div>
            }
            title={`Ask about ${connection.name}`}
            description={`I can read the schema, sample rows, run ${meta.commandLanguage} and draw charts. I look things up rather than guessing, and I ask before changing anything.`}
          >
            <Suggestions className="mt-5">
              {STARTERS.map((prompt) => (
                <Suggestion key={prompt} suggestion={prompt} onSelect={(text) => void send(text)} />
              ))}
            </Suggestions>
          </ConversationEmptyState>
        ) : (
          <ConversationContent>
            {messages.map((message) =>
              message.kind === "user" ? (
                <Message key={message.id} from="user">
                  <MessageContent from="user">{message.text}</MessageContent>
                </Message>
              ) : (
                <Message key={message.id} from="assistant">
                  <MessageContent from="assistant" variant="flat">
                    <ToolTrace calls={message.calls} />

                    {message.thinking.length > 0 && message.text.length === 0 ? (
                      <p className="mb-1.5 text-[11px] text-muted italic">{message.thinking.slice(-300)}</p>
                    ) : null}

                    {message.text.length > 0 ? (
                      <Markdown
                        text={message.text}
                        language={meta.commandLanguage}
                        streaming={message.streaming}
                        onCopy={copy}
                        onOpen={runInEditor}
                      />
                    ) : message.streaming && message.calls.length === 0 ? (
                      <div className="flex items-center gap-2 text-muted">
                        <div className="flex size-5 animate-pulse items-center justify-center rounded bg-accent/15 text-accent">
                          <Icon name="sparkles" size={11} />
                        </div>
                        <span className="animate-pulse text-xs">Thinking…</span>
                      </div>
                    ) : null}

                    {message.artifacts.map((artifact) => (
                      <ArtifactView
                      key={artifact.id}
                      artifact={artifact}
                      language={meta.commandLanguage}
                      onOpen={runInEditor}
                    />
                    ))}

                    {permission !== null && message.runId === runId ? (
                      <PermissionPrompt request={permission} onDecide={decide} />
                    ) : null}

                    {message.error !== null ? (
                      <div className="mt-1.5 rounded-lg border border-danger/30 bg-danger/10 p-2 text-[11px] text-danger">
                        {message.error}
                      </div>
                    ) : null}

                    {stopNote(message.stop) !== null ? (
                      <p className="mt-1 text-[10.5px] text-muted">{stopNote(message.stop)}</p>
                    ) : null}
                  </MessageContent>
                </Message>
              ),
            )}
            <div ref={endRef} />
          </ConversationContent>
        )}
      </Conversation>

      {/* Composer */}
      <div className="shrink-0 border-t border-border/40 px-3 py-2.5 glass-header sm:px-4">
        <PromptInput
          onSubmit={(event) => {
            event.preventDefault();
            void send();
          }}
        >
          <PromptInputBody>
            <PromptInputTextarea
              value={input}
              onChange={(event) => { setInput(event.target.value); }}
              disabled={!aiConfigured || busy}
              aria-label="Message"
              placeholder={busy ? "Working…" : `Ask about your data, or describe a ${meta.commandLanguage} query…`}
            />
          </PromptInputBody>
          <PromptInputSubmit
            status={busy ? "streaming" : "ready"}
            onStop={stop}
            disabled={!aiConfigured || input.trim().length === 0}
          />
        </PromptInput>
      </div>
    </div>
  );
}
