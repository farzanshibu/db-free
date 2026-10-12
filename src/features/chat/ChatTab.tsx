// SOT: chat-tab, ai-database-conversation, agent-streaming-chat, agent-permission-ui, agent-artifacts, saved-chats-ui, chat-seed-consumer, chat-mentions-ui
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  AgentEvent,
  AiChatSummary,
  ChatMention,
  DocumentKind,
  ModelChoice,
  PermissionDecision,
  PermissionRequest,
} from "@/lib/bindings";
import { ipc, onAgentEvent } from "@/lib/ipc";
import { engineMeta } from "@/lib/engines";
import { readStored, writeStored } from "@/lib/storage";
import { useWorkspace } from "@/stores/workspace";
import { IconButton } from "@/components/global/Button";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Markdown, type CodeRunner } from "./Markdown";
import { ArtifactView } from "./ArtifactView";
import { PermissionPrompt, ToolTrace } from "./ToolTrace";
import { ChatHistory } from "./ChatHistory";
import { MentionInput } from "./MentionInput";
import { ModelPicker } from "./ModelPicker";
import { AiWelcomeDialog, shouldWelcome } from "./AiWelcomeDialog";
import { aiEnabled } from "./access";
import {
  fromSaved,
  pendingAssistant,
  stopNote,
  type AssistantMessage,
  type ChatMessageView,
} from "./chatMessages";
import { ipcMessage } from "./ipcMessage";
import {
  MENTION_KIND_LABEL,
  mentionsFromText,
  mentionsInText,
  splitMentions,
  tokenOf,
  type MentionCandidate,
} from "./mentions";
import { emptyDashboardPrompt, emptyDiagramPrompt, emptyWorkflowPrompt } from "./prompts";
import { GENERIC_STARTERS, ROTATING_HINTS, pickSuggestions } from "./suggestions";
import { defaultChoice, rememberChoice, storedChoice } from "./useAiModels";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Conversation, ConversationContent, ConversationEmptyState } from "@/components/ui/conversation";
import { Message, MessageContent } from "@/components/ui/message";
import { PromptInput, PromptInputBody, PromptInputSubmit, PromptInputToolbar } from "@/components/ui/prompt-input";
import { Spinner } from "@/components/ui/spinner";
import { Suggestion, Suggestions } from "@/components/ui/suggestion";

// WHAT:  Conversation with the connected database, driven by the agent loop.
// WHY:   The old chat sent the whole schema with every message, waited in
//        silence for one blocking reply, and printed it as plain text. This one
//        streams: prose renders as markdown while it is written, each tool call
//        appears as it runs, anything that would write stops and asks, charts the
//        assistant drew are rendered inline and stay interactive — and the
//        conversation is kept: switch to an old chat, rename it, delete it, pick
//        the model per message, point at tables and saved queries with `@`, and
//        run the code it writes without leaving the chat.
// HOW:   `agent_chat` returns the finished turn; everything before that arrives
//        on the `agent:event` stream and is folded into the pending message. The
//        server saves both sides of every turn, so this tab only mirrors it: the
//        history list and a chat's messages are read back with `ai_chat_*`.
// WHERE: src/features/chat/*, src-tauri/src/commands/{agent,ai}.rs

/// Survives a tab switch within the session; the saved chat is the real copy.
const threadCache = new Map<string, ChatMessageView[]>();
/// The chat each connection was showing, so coming back to the tab restores it.
const activeChats = new Map<string, string>();

const lastChatKey = (connectionId: string): string => `db-free.ai-chat.${connectionId}`;

/// Schemas that hold the engine's own tables; nobody means them by `@`.
const SYSTEM_SCHEMAS = new Set(["information_schema", "pg_catalog", "pg_toast", "performance_schema", "sys", "mysql"]);

const DOCUMENT_KINDS: readonly DocumentKind[] = ["dashboard", "workflow", "diagram"];

/// Seconds between looks at a chat whose last question has no answer yet.
const POLL_MS = 3000;
const POLL_LIMIT = 40;

function greeting(): string {
  const hour = new Date().getHours();
  if (hour >= 5 && hour < 12) return "Good morning";
  if (hour >= 12 && hour < 17) return "Good afternoon";
  if (hour >= 17 && hour < 22) return "Good evening";
  return "Hello";
}

/// The user's words with their `@` tokens picked out.
function UserText({ text, mentions }: { text: string; mentions: readonly ChatMention[] }) {
  return (
    <>
      {splitMentions(text, mentions).map((part, index) =>
        part.mention === null ? (
          <span key={index}>{part.text}</span>
        ) : (
          <span key={index} className="rounded bg-accent-foreground/20 px-1 font-mono text-[11px]">
            {part.text}
          </span>
        ),
      )}
    </>
  );
}

export function ChatTab({ connectionId }: { connectionId: string }) {
  const connections = useWorkspace((s) => s.connections);
  const catalogs = useWorkspace((s) => s.catalogs);
  const settings = useWorkspace((s) => s.settings);
  const savedQueries = useWorkspace((s) => s.savedQueries);
  const documents = useWorkspace((s) => s.documents);
  const loadDocuments = useWorkspace((s) => s.loadDocuments);
  const openQuery = useWorkspace((s) => s.openQuery);
  const goSettingsSection = useWorkspace((s) => s.goSettingsSection);
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

  const aiConfigured = aiEnabled(settings);
  const autonomy = settings?.ai.autonomy ?? "ask_on_write";
  const dataAccess = settings?.ai.dataAccess ?? "full";
  const profiles = settings?.ai.profiles;

  const [chatId, setChatId] = useState<string>(() => activeChats.get(connectionId) ?? crypto.randomUUID());
  const [messages, setMessages] = useState<ChatMessageView[]>(() => threadCache.get(activeChats.get(connectionId) ?? "") ?? []);
  const [chats, setChats] = useState<AiChatSummary[]>([]);
  const [restoring, setRestoring] = useState<boolean>(() => !activeChats.has(connectionId));
  const [input, setInput] = useState("");
  const [mentions, setMentions] = useState<ChatMention[]>([]);
  const [runId, setRunId] = useState<string | null>(null);
  const [permission, setPermission] = useState<PermissionRequest | null>(null);
  const [hint, setHint] = useState(0);
  const [seed, setSeed] = useState(() => Math.floor(Math.random() * 1_000_000));
  const [stored, setStored] = useState<ModelChoice | null>(() => storedChoice(settings));
  const [poll, setPoll] = useState<{ chatId: string; ticks: number }>({ chatId: "", ticks: 0 });
  const [welcome, setWelcome] = useState<boolean>(() => shouldWelcome(settings));
  const endRef = useRef<HTMLDivElement>(null);

  // The listener is registered once and reads the live run through this ref, so
  // it never needs re-subscribing mid-stream. Written only in `send`, never
  // during render.
  const runIdRef = useRef<string | null>(null);

  // --- the model this chat uses --------------------------------------------
  const choice: ModelChoice | null =
    stored !== null && profiles?.some((p) => p.id === stored.profileId) === true ? stored : defaultChoice(settings);

  // --- what `@` can point at ------------------------------------------------
  const candidates = useMemo<MentionCandidate[]>(() => {
    const out: MentionCandidate[] = [];
    for (const schema of catalog?.schemas ?? []) {
      if (SYSTEM_SCHEMAS.has(schema.name)) continue;
      for (const table of schema.tables) {
        const label = table.schema !== null && table.schema.length > 0 ? `${table.schema}.${table.name}` : table.name;
        out.push({
          kind: "table",
          id: label,
          label,
          detail: table.kind === "view" ? "view" : table.rowEstimate !== null && table.rowEstimate >= 0 ? `~${table.rowEstimate} rows` : null,
        });
      }
    }
    for (const query of savedQueries) {
      if (query.connectionId !== null && query.connectionId !== connectionId) continue;
      out.push({ kind: "saved_query", id: query.id, label: tokenOf(query.name), detail: query.sql.trim().split("\n")[0]?.slice(0, 60) ?? null });
    }
    for (const kind of DOCUMENT_KINDS) {
      for (const doc of documents[kind]) {
        if (doc.connectionId !== null && doc.connectionId !== connectionId) continue;
        out.push({ kind, id: doc.id, label: tokenOf(doc.name), detail: null });
      }
    }
    return out;
  }, [catalog, savedQueries, documents, connectionId]);

  const tables = useMemo(() => candidates.filter((c) => c.kind === "table"), [candidates]);
  const suggestions = useMemo(() => pickSuggestions(tables, seed), [tables, seed]);

  // Dashboards, workflows and diagrams are mentionable once their lists are loaded.
  useEffect(() => {
    void (async () => {
      for (const kind of DOCUMENT_KINDS) {
        try {
          await loadDocuments(kind);
        } catch {
          // Without the list those documents simply are not offered after `@`.
        }
      }
    })();
  }, [loadDocuments]);

  // --- remembering where we are --------------------------------------------
  useEffect(() => {
    threadCache.set(chatId, messages);
  }, [chatId, messages]);

  // Not while restoring: until the saved chat is picked, `chatId` is only a
  // placeholder, and recording it would make the restore think a chat was showing.
  useEffect(() => {
    if (restoring) return;
    activeChats.set(connectionId, chatId);
    writeStored(lastChatKey(connectionId), chatId);
  }, [connectionId, chatId, restoring]);

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, permission]);

  const refreshChats = useCallback(async () => {
    try {
      setChats(await ipc("ai_chat_list", { connectionId }));
    } catch (raw) {
      showError(ipcMessage(raw));
    }
  }, [connectionId, showError]);

  // --- restoring the last chat ---------------------------------------------
  useEffect(() => {
    // The cleanup flips this while the awaits below are pending. Read through a
    // function so the compiler does not narrow it to its first value.
    const run = { alive: true };
    const live = () => run.alive;
    void (async () => {
      try {
        const list = await ipc("ai_chat_list", { connectionId });
        if (!live()) return;
        setChats(list);

        const showing = activeChats.get(connectionId);
        if (showing !== undefined) {
          // Coming back to the tab. A message that was still streaming when the
          // tab was left has lost its event feed; the saved copy is the truth.
          const cached = threadCache.get(showing);
          if (cached?.some((m) => m.kind === "assistant" && m.streaming) === true && list.some((c) => c.id === showing)) {
            const detail = await ipc("ai_chat_load", { chatId: showing });
            if (live()) setMessages(fromSaved(detail.messages));
          }
          return;
        }
        const remembered = readStored(lastChatKey(connectionId));
        const pick = list.find((c) => c.id === remembered) ?? list[0];
        if (pick === undefined) return;
        const detail = await ipc("ai_chat_load", { chatId: pick.id });
        if (!live()) return;
        setChatId(pick.id);
        setMessages(fromSaved(detail.messages));
      } catch (raw) {
        if (live()) showError(ipcMessage(raw));
      } finally {
        if (live()) setRestoring(false);
      }
    })();
    return () => {
      run.alive = false;
    };
  }, [connectionId, showError]);

  // --- an answer that arrives while we are not watching ---------------------
  // The tab was left mid-answer, or the app was closed: the saved chat ends on
  // a question. Look again until the answer lands (or give up after a while).
  const lastIsQuestion = messages.at(-1)?.kind === "user";
  const ticks = poll.chatId === chatId ? poll.ticks : 0;
  const waiting = lastIsQuestion && runId === null && !restoring && ticks < POLL_LIMIT;
  useEffect(() => {
    if (!waiting) return;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const detail = await ipc("ai_chat_load", { chatId });
          if (detail.messages.at(-1)?.role === "assistant") setMessages(fromSaved(detail.messages));
          else setPoll({ chatId, ticks: ticks + 1 });
          await refreshChats();
        } catch {
          // The chat may have been deleted meanwhile; there is nothing to wait for.
          setPoll({ chatId, ticks: POLL_LIMIT });
        }
      })();
    }, POLL_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [waiting, chatId, ticks, refreshChats]);

  // --- a prompt handed over by another feature -------------------------------
  // `openChat(connectionId, prompt)` leaves the prompt in the store. It is taken
  // once: right after mount (the tab may have just been opened for it) and
  // whenever the store changes while the tab is open. Taking it inside a
  // callback — not the effect body — is the pattern for state that arrives from
  // outside React.
  const candidatesRef = useRef<readonly MentionCandidate[]>([]);
  useEffect(() => {
    candidatesRef.current = candidates;
  }, [candidates]);
  useEffect(() => {
    const consume = () => {
      const state = useWorkspace.getState();
      if (state.chatSeed?.connectionId !== connectionId) return;
      const prompt = state.takeChatSeed(connectionId);
      if (prompt === null) return;
      const seeded = mentionsFromText(prompt, candidatesRef.current);
      setInput(seeded.text);
      setMentions(seeded.mentions);
    };
    const first = window.setTimeout(consume, 0);
    const unsubscribe = useWorkspace.subscribe(consume);
    return () => {
      window.clearTimeout(first);
      unsubscribe();
    };
  }, [connectionId]);

  // --- the rotating hint in the empty box ------------------------------------
  useEffect(() => {
    if (input.length > 0 || messages.length > 0) return;
    const timer = window.setInterval(() => {
      setHint((h) => (h + 1) % ROTATING_HINTS.length);
    }, 4000);
    return () => {
      window.clearInterval(timer);
    };
  }, [input, messages.length]);

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
        case "started":
          patchAssistant(event.runId, (m) => ({ ...m, model: event.model }));
          break;
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
            steps: event.turn.usage.steps,
            model: event.turn.model,
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
        case "step":
        case "usage":
          break;
      }
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [patchAssistant]);

  // WHAT:  Runs one request against the agent and folds its stream into the
  //        pending assistant message `nextRun`; both a new question and Continue
  //        go through it.
  // HOW:   A question carries its text and `@` references. Continue carries
  //        neither: the server picks the chat's transcript up where the step
  //        limit stopped it, so nothing is asked and no message of yours is added.
  const execute = useCallback(
    async (nextRun: string, body: { prompt: string; mentions: readonly ChatMention[]; resume: boolean }) => {
      if (!connection) return;
      setRunId(nextRun);
      runIdRef.current = nextRun;

      try {
        await ipc("agent_chat", {
          connectionId: connection.id,
          chatId,
          runId: nextRun,
          prompt: body.prompt,
          context: null,
          useTools: true,
          ...(body.resume ? { resume: true } : {}),
          ...(body.mentions.length > 0 ? { mentions: [...body.mentions] } : {}),
          ...(choice !== null ? { model: choice } : {}),
        });
      } catch (raw) {
        const message = ipcMessage(raw);
        patchAssistant(nextRun, (m) => ({ ...m, streaming: false, error: message }));
        showError(message);
      } finally {
        setRunId(null);
        runIdRef.current = null;
        setPermission(null);
        void refreshChats();
      }
    },
    [chatId, choice, connection, patchAssistant, refreshChats, showError],
  );

  const send = useCallback(
    async (rawText: string, withMentions: readonly ChatMention[]) => {
      const text = rawText.trim();
      if (text.length === 0 || runId !== null || restoring || !connection) return;

      const nextRun = crypto.randomUUID();
      const now = Date.now();
      const referenced = mentionsInText(text, withMentions);
      setMessages((prev) => [
        ...prev,
        { kind: "user", id: `u-${nextRun}`, text, mentions: referenced, at: now },
        pendingAssistant(nextRun, now),
      ]);
      await execute(nextRun, { prompt: text, mentions: referenced, resume: false });
    },
    [connection, execute, restoring, runId],
  );

  // WHAT:  Lets a run that stopped on the step limit carry on for another round.
  const continueRun = useCallback(async () => {
    if (runId !== null || restoring || !connection) return;
    const nextRun = crypto.randomUUID();
    setMessages((prev) => [...prev, pendingAssistant(nextRun, Date.now())]);
    await execute(nextRun, { prompt: "", mentions: [], resume: true });
  }, [connection, execute, restoring, runId]);

  const submit = useCallback(() => {
    const text = input;
    const sent = mentions;
    setInput("");
    setMentions([]);
    void send(text, sent);
  }, [input, mentions, send]);

  const decide = useCallback(
    (decision: PermissionDecision) => {
      if (runId === null) return;
      setPermission(null);
      void (async () => {
        try {
          await ipc("agent_decide", { runId, decision });
        } catch (raw) {
          showError(ipcMessage(raw));
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
        showError(ipcMessage(raw));
      }
    })();
  }, [runId, showError]);

  // --- chat management ---------------------------------------------------------
  const newChat = useCallback(() => {
    if (runId !== null) return;
    setChatId(crypto.randomUUID());
    setMessages([]);
    setPermission(null);
    setInput("");
    setMentions([]);
  }, [runId]);

  const selectChat = useCallback(
    (target: string) => {
      if (runId !== null || target === chatId) return;
      void (async () => {
        try {
          const detail = await ipc("ai_chat_load", { chatId: target });
          setChatId(target);
          setMessages(fromSaved(detail.messages));
          setPermission(null);
        } catch (raw) {
          showError(ipcMessage(raw));
          void refreshChats();
        }
      })();
    },
    [chatId, refreshChats, runId, showError],
  );

  const renameChat = useCallback(
    async (target: string, title: string) => {
      try {
        await ipc("ai_chat_rename", { chatId: target, title });
        await refreshChats();
      } catch (raw) {
        showError(ipcMessage(raw));
      }
    },
    [refreshChats, showError],
  );

  const deleteChat = useCallback(
    async (target: string) => {
      try {
        await ipc("ai_chat_delete", { chatId: target });
        threadCache.delete(target);
        if (target === chatId) newChat();
        await refreshChats();
      } catch (raw) {
        showError(ipcMessage(raw));
      }
    },
    [chatId, newChat, refreshChats, showError],
  );

  const copy = useCallback(
    (code: string) => {
      void (async () => {
        try {
          await navigator.clipboard.writeText(code);
          showInfo("Copied to clipboard.");
        } catch (raw) {
          showError(ipcMessage(raw));
        }
      })();
    },
    [showError, showInfo],
  );

  const runInEditor = useCallback(
    (code: string) => {
      if (!connection) return;
      openQuery(connection.id, code, "From chat");
    },
    [connection, openQuery],
  );

  const runner = useMemo<CodeRunner | undefined>(
    () =>
      connection && meta
        ? { connectionId: connection.id, engine: connection.engine, allowJson: meta.commandLanguage !== "SQL" }
        : undefined,
    [connection, meta],
  );

  if (!connection || !meta) {
    return (
      <div className="flex h-full items-center justify-center p-4 text-muted">
        <span>Connection not found.</span>
      </div>
    );
  }

  const busy = runId !== null;
  const title = chats.find((chat) => chat.id === chatId)?.title ?? null;
  const placeholder = busy
    ? "Working…"
    : input.length === 0 && messages.length === 0
      ? (ROTATING_HINTS[hint] ?? ROTATING_HINTS[0] ?? "")
      : `Ask about your data, or describe a ${meta.commandLanguage} query…`;

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col bg-background select-none">
      {/* Toolbar */}
      <div className="flex app-toolbar shrink-0 items-center justify-between gap-2 border-b border-border/40 glass-header">
        <div className="flex min-w-0 items-center gap-2">
          <div className="flex size-6 shrink-0 items-center justify-center rounded-lg bg-accent/15 text-accent">
            <Icon name="sparkles" size={13} />
          </div>
          <span className="truncate text-xs font-semibold tracking-tight text-foreground">
            {title ?? `Chat with ${connection.name}`}
          </span>
          <Badge size="sm" variant="secondary" className="hidden h-5 shrink-0 px-2 text-[10px] glass-pill text-muted sm:flex">
            {meta.label}
          </Badge>
          <span className="hidden shrink-0 text-[11px] text-muted lg:inline">
            {tableCount > 0 ? `${tableCount} tables` : "catalog loading…"}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {dataAccess === "schema" ? (
            <Badge size="sm" variant="secondary" className="hidden h-5 px-2 text-[10px] glass-pill text-muted md:flex">
              <Icon name="eye-off" size={9} />
              Schema only
            </Badge>
          ) : null}
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
              className="rounded-md px-2 text-xs font-medium glass-pill text-danger liquid-hover"
              onClick={stop}
            >
              <Icon name="x" size={11} />
              Stop
            </Button>
          ) : null}
          <ChatHistory
            chats={chats}
            activeId={chatId}
            busy={busy}
            onSelect={selectChat}
            onNew={newChat}
            onRename={renameChat}
            onDelete={deleteChat}
          />
          <IconButton icon="plus" label="New chat" disabled={busy || messages.length === 0} onClick={newChat} size={14} />
        </div>
      </div>

      {!aiConfigured ? (
        <div className="p-2.5">
          <Alert variant="warning" className="rounded-lg border-warning/30 glass-modal">
            <AlertIndicator />
            <AlertContent>
              <AlertTitle className="text-xs font-semibold">
                {dataAccess === "off" ? "The assistant is turned off" : "No AI provider configured"}
              </AlertTitle>
              <AlertDescription className="text-xs text-muted">
                {dataAccess === "off"
                  ? "Turn it on in Settings → AI to chat with this database."
                  : "Add a provider and key in Settings to chat with this database."}
              </AlertDescription>
            </AlertContent>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => { goSettingsSection("ai"); }}
              className="ml-auto glass-pill text-xs"
            >
              Open Settings
            </Button>
          </Alert>
        </div>
      ) : null}

      {/* Thread */}
      <Conversation>
        {restoring ? (
          <div className="flex flex-1 items-center justify-center p-4">
            <Spinner size="sm" />
          </div>
        ) : messages.length === 0 ? (
          <ConversationEmptyState
            icon={
              <div className="flex size-9 items-center justify-center rounded-lg bg-accent/15 text-accent">
                <Icon name="sparkles" size={18} />
              </div>
            }
            title={`${greeting()} — ask about ${connection.name}`}
            description={
              dataAccess === "schema"
                ? `I can read the schema and write ${meta.commandLanguage} for you to run. I never see your rows, and I ask before saving anything.`
                : `I can read the schema, sample rows, run ${meta.commandLanguage} and draw charts. I look things up rather than guessing, and I ask before changing anything.`
            }
          >
            <Suggestions className="mt-5">
              {suggestions.length > 0
                ? suggestions.map((suggestion) => (
                    <Suggestion
                      key={suggestion.label}
                      suggestion={suggestion.label}
                      onSelect={() => { void send(suggestion.prompt, [suggestion.mention]); }}
                    />
                  ))
                : GENERIC_STARTERS.map((starter) => (
                    <Suggestion key={starter.label} suggestion={starter.label} onSelect={() => { void send(starter.prompt, []); }} />
                  ))}
            </Suggestions>
            <div className="mt-3 flex flex-wrap items-center justify-center gap-1.5">
              {(
                [
                  { label: "Build a dashboard", icon: "chart-bar", prompt: emptyDashboardPrompt() },
                  { label: "Create a workflow", icon: "flow", prompt: emptyWorkflowPrompt() },
                  { label: "Draw a diagram", icon: "chart-relationship", prompt: emptyDiagramPrompt() },
                ] as const
              ).map((starter) => (
                <Button
                  key={starter.label}
                  size="sm"
                  variant="tertiary"
                  className="gap-1.5 rounded-md px-2 text-[11px]"
                  disabled={!aiConfigured}
                  onClick={() => { void send(starter.prompt, []); }}
                >
                  <Icon name={starter.icon} size={12} />
                  {starter.label}
                </Button>
              ))}
              {tables.length > 3 ? (
                <Button
                  size="sm"
                  variant="ghost"
                  className="gap-1.5 rounded-md px-2 text-[11px] text-muted"
                  onClick={() => { setSeed((s) => s + 1); }}
                >
                  <Icon name="refresh" size={11} />
                  More ideas
                </Button>
              ) : null}
            </div>
          </ConversationEmptyState>
        ) : (
          <ConversationContent>
            {messages.map((message) =>
              message.kind === "user" ? (
                <Message key={message.id} from="user">
                  <MessageContent from="user">
                    <UserText text={message.text} mentions={message.mentions} />
                  </MessageContent>
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
                        {...(runner !== undefined ? { runner } : {})}
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
                        engine={connection.engine}
                        connectionId={connection.id}
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

                    {stopNote(message.stop, message.steps) !== null ? (
                      <p className="mt-1 text-[10.5px] text-muted">{stopNote(message.stop, message.steps)}</p>
                    ) : null}

                    {message.stop === "max_steps" && !message.streaming && !busy && message.id === messages.at(-1)?.id ? (
                      <div className="mt-1 flex items-center gap-1.5">
                        <Button size="sm" variant="soft" disabled={!aiConfigured || restoring} onClick={() => { void continueRun(); }}>
                          <Icon name="play" size={11} />
                          Continue
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          className="text-muted"
                          onClick={() => { goSettingsSection("ai"); }}
                        >
                          Change the limit ({settings?.ai.maxSteps ?? 12} steps)
                        </Button>
                      </div>
                    ) : null}

                    {!message.streaming && message.model !== null ? (
                      <p className="mt-1 font-mono text-[10px] text-muted/70">{message.model}</p>
                    ) : null}
                  </MessageContent>
                </Message>
              ),
            )}
            {waiting ? (
              <div className="flex items-center gap-2 px-1 text-[11px] text-muted">
                <Spinner size="sm" />
                Waiting for the answer…
              </div>
            ) : null}
            <div ref={endRef} />
          </ConversationContent>
        )}
      </Conversation>

      {/* Composer */}
      <div className="shrink-0 border-t border-border/40 px-2.5 py-1.5 glass-header sm:px-3">
        {mentions.length > 0 ? (
          <div className="mx-auto mb-1.5 flex w-full max-w-3xl flex-wrap items-center gap-1">
            <span className="text-[10px] text-muted">Referencing</span>
            {mentions.map((mention) => (
              <span
                key={`${mention.kind}:${mention.id}`}
                title={MENTION_KIND_LABEL[mention.kind]}
                className="flex items-center gap-1 rounded-md bg-accent/12 px-1.5 py-0.5 font-mono text-[10.5px] text-accent"
              >
                @{mention.label}
                <button
                  type="button"
                  aria-label={`Stop referencing ${mention.label}`}
                  className="rounded text-accent/70 hover:text-accent"
                  onClick={() => {
                    setMentions((current) => current.filter((m) => !(m.kind === mention.kind && m.id === mention.id)));
                    setInput((current) => current.split(`@${mention.label}`).join(mention.label));
                  }}
                >
                  <Icon name="x" size={9} />
                </button>
              </span>
            ))}
          </div>
        ) : null}
        <PromptInput
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
        >
          <PromptInputBody>
            <MentionInput
              value={input}
              onChange={setInput}
              mentions={mentions}
              onMentionsChange={setMentions}
              candidates={candidates}
              placeholder={placeholder}
              disabled={!aiConfigured || busy}
            />
            <PromptInputToolbar className={cn("justify-between", !aiConfigured && "opacity-60")}>
              {profiles !== undefined && profiles.length > 0 ? (
                <ModelPicker
                  profiles={profiles}
                  value={choice}
                  onChange={(next) => {
                    setStored(next);
                    rememberChoice(next);
                  }}
                  disabled={busy || !aiConfigured}
                />
              ) : (
                <span />
              )}
              <span className="hidden text-[10px] text-muted sm:inline">Type @ to point at a table, query or dashboard</span>
            </PromptInputToolbar>
          </PromptInputBody>
          <PromptInputSubmit
            status={busy ? "streaming" : "ready"}
            onStop={stop}
            disabled={!aiConfigured || restoring || input.trim().length === 0}
          />
        </PromptInput>
      </div>

      {welcome && settings !== null ? (
        <AiWelcomeDialog
          settings={settings}
          onClose={() => {
            setWelcome(false);
          }}
        />
      ) : null}
    </div>
  );
}
