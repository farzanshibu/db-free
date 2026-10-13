// SOT: inline-ai-prompt, editor-ai-at-cursor, inline-ai-stream
import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import type { AgentEvent, ConnectionSummary } from "@/lib/bindings";
import { engineMeta } from "@/lib/engines";
import { ipc, normalizeError, onAgentEvent } from "@/lib/ipc";
import { commentPrefix, dialectOf } from "@/lib/sqlCatalog";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { Check } from "@/components/global/Field";
import { Shortcut } from "@/components/global/Kbd";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { commentLine, extractCode, inlinePrompt, placeAnswer } from "./inlineAi";
import type { SqlEditorHandle } from "./SqlEditor";
import { ScrollArea } from "@/components/ui/scroll-area";

interface InlineAiPromptProps {
  connection: ConnectionSummary;
  /// The editor, for writing the answer.
  editorRef: RefObject<SqlEditorHandle | null>;
  /// What was selected when the prompt opened; an empty range is the caret.
  selection: { from: number; to: number; text: string };
  /// The caret's place when it opened, relative to the editor's box.
  anchor: { left: number; top: number; bottom: number; width: number; height: number } | null;
  /// The editor's current text, to place the answer between lines.
  documentText: () => string;
  /// The statement or script the model should know about, and the last error, if any.
  context: string | null;
  onClose: () => void;
}

const CARD_WIDTH = 440;
const CARD_HEIGHT = 190;

// WHAT:  The ⌘I prompt: a small card at the caret that turns an instruction into
//        code written straight into the editor.
// WHY:   Opening a side panel to ask for a WHERE clause and then copying the
//        answer back breaks the thought. Here the answer lands where the caret
//        was, or replaces the statement that was highlighted, and ⌘Z takes it back.
// HOW:   Runs on the same agent as the AI popover with tools off: one turn, no
//        database access, the reply streaming in. The preview shows the code as
//        it is written; when the turn finishes the code is written into the
//        editor as one edit and the card closes. Esc stops a run in flight, then
//        closes the card.
// WHERE: src-tauri/src/commands/agent.rs (agent_chat), src/features/editor/inlineAi.ts
export function InlineAiPrompt({ connection, editorRef, selection, anchor, documentText, context, onClose }: InlineAiPromptProps) {
  const settings = useWorkspace((s) => s.settings);
  const saveSettings = useWorkspace((s) => s.saveSettings);
  const goSettingsSection = useWorkspace((s) => s.goSettingsSection);
  const openChat = useWorkspace((s) => s.openChat);
  const showError = useWorkspace((s) => s.showError);
  const meta = engineMeta(connection.engine);
  const refine = selection.text.trim().length > 0;
  const aiReady = settings !== null && settings.ai.provider !== "none";
  const canComment = dialectOf(connection.engine).flavor !== null || meta.commandLanguage === "Cypher";
  const asComment = canComment && settings?.inlineAiPromptAsComment === true;

  const [instruction, setInstruction] = useState("");
  const [busy, setBusy] = useState(false);
  const [reply, setReply] = useState("");
  const [failure, setFailure] = useState<string | null>(null);
  /// The turn this card is showing; frames from any other run are ignored.
  const runRef = useRef<string | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    const pending = onAgentEvent((event: AgentEvent) => {
      if (event.runId !== runRef.current) return;
      if (event.type === "text") setReply((previous) => previous + event.delta);
    });
    return () => {
      void pending.then((unlisten) => { unlisten(); });
    };
  }, []);

  const finish = useCallback(() => {
    editorRef.current?.focus();
    onClose();
  }, [editorRef, onClose]);

  // Esc or the close button: stop the run in flight, if any, then leave.
  const cancel = useCallback(() => {
    const runId = runRef.current;
    runRef.current = null;
    if (runId !== null) void ipc("agent_cancel", { runId }).catch(() => undefined);
    finish();
  }, [finish]);

  const submit = async () => {
    const prompt = instruction.trim();
    if (prompt.length === 0 || busy || !aiReady) return;
    const runId = crypto.randomUUID();
    runRef.current = runId;
    setBusy(true);
    setReply("");
    setFailure(null);
    try {
      const turn = await ipc("agent_chat", {
        connectionId: connection.id,
        chatId: `editor:${connection.id}`,
        runId,
        prompt: inlinePrompt({ instruction: prompt, selection: selection.text, language: meta.commandLanguage, engine: meta.label }),
        context,
        useTools: false,
      });
      if (runRef.current !== runId) return;
      const code = (turn.sql ?? extractCode(turn.text)).trim();
      if (code.length === 0) {
        setFailure("The model answered without any code. Try rephrasing the instruction.");
        return;
      }
      const placement = placeAnswer(documentText(), selection.from, selection.to, code, asComment ? commentLine(commentPrefix(connection.engine), prompt) : null);
      editorRef.current?.replace(placement.from, placement.to, placement.insert);
      runRef.current = null;
      onClose();
    } catch (raw) {
      if (runRef.current !== runId) return;
      const error = normalizeError(raw);
      if (error.kind === "cancelled") return;
      setFailure(error.message);
      showError(error);
    } finally {
      if (runRef.current === runId || runRef.current === null) setBusy(false);
    }
  };

  const setAsComment = (next: boolean) => {
    if (settings === null) return;
    void (async () => {
      try {
        await saveSettings({ ...settings, inlineAiPromptAsComment: next }, null);
      } catch (raw) {
        showError(normalizeError(raw));
      }
    })();
  };

  // What "Ask in chat" hands over: the instruction, with the selection when there is one.
  const chatPrompt = (): string | undefined => {
    const ask = instruction.trim();
    if (refine) return `${ask.length > 0 ? ask : "Explain this and suggest improvements:"}\n\n\`\`\`\n${selection.text}\n\`\`\``;
    return ask.length > 0 ? ask : undefined;
  };

  // Placed under the caret, or above it when the card would run off the editor.
  const width = Math.min(CARD_WIDTH, Math.max(280, (anchor?.width ?? CARD_WIDTH) - 16));
  const left = Math.max(8, Math.min(anchor?.left ?? 8, (anchor?.width ?? CARD_WIDTH) - width - 8));
  const below = (anchor?.bottom ?? 8) + 6;
  const top = anchor === null || below + CARD_HEIGHT <= anchor.height ? below : Math.max(8, anchor.top - CARD_HEIGHT - 6);
  const preview = extractCode(reply);

  return (
    <div
      role="dialog"
      aria-label="Ask AI at the cursor"
      className="glass-modal absolute z-40 flex flex-col gap-2 rounded-lg p-2 shadow-lg"
      style={{ left, top, width }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          cancel();
        }
      }}
    >
      <div className="flex items-center gap-1.5 text-[11px] font-semibold text-muted">
        <Icon name="sparkles" size={12} className="text-accent" />
        {refine ? "Rewrite the selection" : `Write ${meta.commandLanguage} here`}
        <span className="ml-auto flex items-center gap-1 font-normal">
          <Shortcut action="inline-ai" />
        </span>
      </div>
      {aiReady ? (
        <>
          <Textarea
            ref={inputRef}
            aria-label="What should it write?"
            rows={2}
            value={instruction}
            disabled={busy}
            placeholder={refine ? "Say how to change it — add a filter, join the orders table…" : "Describe what you need — top 10 customers by revenue last month…"}
            className="min-h-[3.25rem] resize-none text-xs"
            onChange={(event) => { setInstruction(event.target.value); }}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                void submit();
              }
            }}
          />
          {busy || preview.length > 0 ? (
            <div className="overflow-hidden rounded-lg border border-border/60 bg-surface-secondary/60">
              <ScrollArea orientation="both" className="max-h-32">
                <pre className={cn("selectable px-2 py-1.5 font-mono text-[11px] leading-5 text-foreground", preview.length === 0 && "text-muted")}>
                  {preview.length > 0 ? preview : "Thinking…"}
                </pre>
              </ScrollArea>
            </div>
          ) : null}
          {failure !== null ? <p className="selectable text-xs text-danger">{failure}</p> : null}
          <div className="flex items-center gap-2">
            {canComment ? (
              <label className="flex cursor-pointer items-center gap-1.5 text-[11px] text-muted">
                <Check checked={asComment} onChange={setAsComment} label="Keep the prompt as a comment" />
                Keep prompt as a comment
              </label>
            ) : null}
            <span className="ml-auto flex items-center gap-1.5">
              <Button
                size="xs"
                variant="ghost"
                title="Open the chat with this question"
                onClick={() => {
                  const prompt = chatPrompt();
                  cancel();
                  openChat(connection.id, prompt);
                }}
              >
                Ask in chat
              </Button>
              {busy ? (
                <Button size="xs" variant="danger-soft" onClick={cancel}>
                  <Spinner size="sm" />
                  Stop
                </Button>
              ) : (
                <Button size="xs" disabled={instruction.trim().length === 0} onClick={() => { void submit(); }}>
                  <Icon name="send" size={11} />
                  {refine ? "Rewrite" : "Insert"}
                  <span className="text-[10px] opacity-70">↵</span>
                </Button>
              )}
            </span>
          </div>
        </>
      ) : (
        <div className="flex flex-col gap-2 text-xs text-muted">
          <p>Turn on an AI provider in Settings to write queries from a sentence. Your key stays on this machine.</p>
          <span className="flex items-center gap-1.5">
            <Button size="xs" onClick={() => { cancel(); goSettingsSection("ai"); }}>
              Open AI settings
            </Button>
            <Button size="xs" variant="ghost" onClick={cancel}>
              Close
            </Button>
          </span>
        </div>
      )}
    </div>
  );
}
