// SOT: query-pane, query-params-prompt, stop-query-flow, manual-transaction-toolbar, run-query-flow, run-at-cursor-flow, destructive-confirm-flow, buffer-autosave, save-query-flow, save-sql-file-flow, open-sql-file-flow, ai-assist-flow, inline-ai-flow, explain-flow, format-sql, batch-run-flow, saved-query-link
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { AgentEvent, AppError, ConnectionSummary, PlanReport, StatementSpan, TableRef } from "@/lib/bindings";
import type { SQLNamespace } from "@codemirror/lang-sql";
import { ipc, normalizeError, onAgentEvent } from "@/lib/ipc";
import { engineMeta } from "@/lib/engines";
import { Markdown } from "@/features/chat/Markdown";
import { pickSqlSavePath } from "@/lib/native";
import { catalogFor } from "@/lib/sqlCatalog";
import { useWorkspace } from "@/stores/workspace";
import { useShortcut } from "@/stores/useShortcut";
import { useQueryRuns, type RunPiece, type StatementRun } from "@/stores/queryRuns";
import { AppSelect, Field, Segmented } from "@/components/global/Field";
import { useTransactions } from "@/stores/transactions";
import { Badge } from "@/components/ui/badge";
import { EnvDot } from "@/components/global/Badge";
import { IconButton } from "@/components/global/Button";
import { Resizer } from "@/components/global/Resizer";
import { RunShortcut } from "@/components/global/Kbd";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { SqlEditor, type EditorDiagnostic, type RunTarget, type SqlEditorHandle } from "./SqlEditor";
import { bindParams, findParams, paramLiteral, type ParamMode, type ParamValue } from "@/lib/params";
import { ResultsPane } from "./ResultsPane";
import { PlanTree } from "./PlanTree";
import { HistoryPanel } from "./HistoryPanel";
import { InlineAiPrompt } from "./InlineAiPrompt";
import { SaveQueryDialog } from "./SaveQueryDialog";
import { QueryVersionsDialog } from "@/features/queries/QueryVersionsDialog";
import { SaveFolderField } from "@/features/queries/SaveFolderField";
import { formatQuery } from "./formatQuery";
import { openSqlFileInNewTab } from "./openSqlFile";
import { askAboutPrompt, fixErrorPrompt } from "./inlineAi";
import { lineColumnOf } from "./QueryErrorPanel";
import { joinScript, runLabel, scriptOffsetToDoc } from "./runTargets";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Popover, PopoverContent, PopoverHeading, PopoverTrigger } from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { Toggle } from "@/components/ui/toggle";

/// Per-tab row cap. The default comes from Settings -> Max query rows, so the
/// app-wide answer is set once and a single tab can still override it.
/// "none" is the caller asking for every row; it reaches Rust as a null maxRows.
const ROW_CAPS = [
  { value: "500", label: "500 rows" },
  { value: "1000", label: "1,000 rows" },
  { value: "5000", label: "5,000 rows" },
  { value: "10000", label: "10,000 rows" },
  { value: "20000", label: "20,000 rows" },
  { value: "50000", label: "50,000 rows" },
  { value: "100000", label: "100,000 rows" },
  { value: "none", label: "No limit" },
] satisfies readonly { value: string; label: string }[];

type TxMode = "auto" | "manual";
type PlanView = "tree" | "text";

/// 0 is how Settings stores "Unlimited".
function defaultRowCap(max: number | undefined): (typeof ROW_CAPS)[number]["value"] {
  if (max === 0) return "none";
  const wanted = String(max ?? 1000);
  return ROW_CAPS.find((c) => c.value === wanted)?.value ?? "1000";
}

const STOP_ON_ERROR_KEY = "db-free:query-stop-on-error";

function readStopOnError(): boolean {
  try {
    return localStorage.getItem(STOP_ON_ERROR_KEY) === "1";
  } catch {
    return false;
  }
}

interface QueryPaneProps {
  connection: ConnectionSummary;
  tabId: string;
  /// The tab's name. Stored with the buffer, so a restored tab keeps it.
  title: string;
  seedSql?: string | undefined;
}

/// A script waiting for something before it runs: parameter values or a confirmation.
interface PendingRun {
  script: string;
  pieces: readonly RunPiece[];
}

// WHAT:  SQL workbench tab: editor, run, results, history, autosaved buffer,
//        save-as-query, AI assist and plan explanation.
// WHY:   PRD §4.3 / §4.5 — execution runs on the Rust side; the UI awaits the
//        outcome. Destructive statements bounce back as a typed error the user
//        confirms explicitly.
// HOW:   A Run sends the target's script through the run store, one statement at
//        a time, so the results survive switching tabs and each statement has its
//        own status. The store (not this pane) holds the run.
// WHERE: src-tauri/src/guard/mod.rs, src-tauri/src/services/query.rs, src/stores/queryRuns.ts
export function QueryPane({ connection, tabId, title, seedSql }: QueryPaneProps) {
  const catalog = useWorkspace((s) => s.catalogs[connection.id]);
  const live = useWorkspace((s) => s.sessions.includes(connection.id));
  const info = useWorkspace((s) => s.sessionInfos[connection.id]);
  const schemaFilter = useWorkspace((s) => s.schemaFilter[connection.id] ?? null);
  const setSchemaFilter = useWorkspace((s) => s.setSchemaFilter);
  const switchDatabase = useWorkspace((s) => s.switchDatabase);
  const connecting = useWorkspace((s) => s.connecting);
  const columnsCache = useWorkspace((s) => s.columnsCache);
  const loadColumns = useWorkspace((s) => s.loadColumns);
  const settings = useWorkspace((s) => s.settings);
  const savedQueries = useWorkspace((s) => s.savedQueries);
  const openChat = useWorkspace((s) => s.openChat);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const openQuery = useWorkspace((s) => s.openQuery);
  const setTabDirty = useWorkspace((s) => s.setTabDirty);
  const bufferId = tabId;
  const isSql = engineMeta(connection.engine).commandLanguage === "SQL";
  const language = engineMeta(connection.engine).commandLanguage;
  /// Engines whose scripts are cut at `;` into statements (everything SQL-like, CQL and PartiQL included).
  const splits = catalogFor(connection.engine) !== null;

  const run = useQueryRuns((s) => s.runs[tabId]);
  const collapsed = useQueryRuns((s) => s.collapsed[tabId] === true);
  const linkedId = useQueryRuns((s) => s.links[tabId]);
  const startRun = useQueryRuns((s) => s.start);
  const stopRun = useQueryRuns((s) => s.stop);
  const rerunStatement = useQueryRuns((s) => s.rerunStatement);
  const setCollapsed = useQueryRuns((s) => s.setCollapsed);
  const linkTab = useQueryRuns((s) => s.link);
  const running = run?.running === true;
  const stopping = run?.stopping === true;
  const linked = linkedId === undefined ? null : (savedQueries.find((q) => q.id === linkedId) ?? null);

  const [sql, setSql] = useState(seedSql ?? "");
  const [loaded, setLoaded] = useState(false);
  const [rowCap, setRowCap] = useState<(typeof ROW_CAPS)[number]["value"]>(() => defaultRowCap(settings?.maxQueryRows));
  const [stopOnError, setStopOnError] = useState(readStopOnError);
  const [confirm, setConfirm] = useState<{ statements: string[]; pending: PendingRun } | null>(null);
  const [spans, setSpans] = useState<readonly StatementSpan[]>([]);
  const [target, setTarget] = useState<RunTarget | null>(null);
  const [savingFile, setSavingFile] = useState(false);
  const [openingFile, setOpeningFile] = useState(false);
  const [txKey, setTxKey] = useState(0);
  const [showHistory, setShowHistory] = useState(false);
  const [saveOpen, setSaveOpen] = useState(false);
  const [versionsOpen, setVersionsOpen] = useState(false);
  /// The text last written to a .sql file: work that is on disk is not unsaved.
  const [filedSql, setFiledSql] = useState<string | null>(null);
  const [aiOpen, setAiOpen] = useState(false);
  const [aiPrompt, setAiPrompt] = useState("");
  const [aiBusy, setAiBusy] = useState(false);
  const [aiText, setAiText] = useState<string | null>(null);
  const [aiGeneratedSql, setAiGeneratedSql] = useState<string | null>(null);
  /// The turn this pane is showing; frames from any other run are ignored.
  const aiRunRef = useRef<string | null>(null);
  const [inlineAi, setInlineAi] = useState<{ key: number; selection: { from: number; to: number; text: string }; anchor: { left: number; top: number; bottom: number; width: number; height: number } | null; context: string | null } | null>(null);
  const inlineCounter = useRef(0);
  const [plan, setPlan] = useState<PlanReport | null>(null);
  const [planBusy, setPlanBusy] = useState(false);
  const [planView, setPlanView] = useState<PlanView>("tree");
  const saveTimer = useRef<number | null>(null);
  const editorRef = useRef<SqlEditorHandle | null>(null);
  /// A Run that stopped to ask for parameter values, and the values last used
  /// (kept per tab so the next Run pre-fills them).
  const [paramPrompt, setParamPrompt] = useState<(PendingRun & { names: string[]; then: "run" | "explain" }) | null>(null);
  const [paramValues, setParamValues] = useState<Record<string, ParamValue>>({});
  /// Manual transaction mode: per tab, but the transaction itself is the connection's.
  const txCapable = info?.manualTransactions === true;
  const manual = useTransactions((s) => s.manual[tabId] === true) && txCapable;
  const txOpen = useTransactions((s) => s.open[connection.id] === true);
  const setTxOpen = useTransactions((s) => s.setOpen);
  const setManual = useTransactions((s) => s.setManual);
  const [txBusy, setTxBusy] = useState(false);

  const [editorHeight, setEditorHeight] = useState<number>(() => {
    try {
      const saved = localStorage.getItem("db-free:query-editor-height");
      return saved ? Math.max(100, Math.min(800, Number(saved))) : 260;
    } catch {
      return 260;
    }
  });

  const handleEditorResize = useCallback((delta: number) => {
    setEditorHeight((prev) => {
      const next = Math.max(100, Math.min(800, prev + delta));
      try {
        localStorage.setItem("db-free:query-editor-height", String(next));
      } catch {
        // ignore
      }
      return next;
    });
  }, []);

  const [historyWidth, setHistoryWidth] = useState<number>(() => {
    try {
      const saved = localStorage.getItem("db-free:query-history-width");
      return saved ? Math.max(220, Math.min(600, Number(saved))) : 256;
    } catch {
      return 256;
    }
  });

  const handleHistoryResize = useCallback((delta: number) => {
    setHistoryWidth((prev) => {
      const next = Math.max(220, Math.min(600, prev - delta));
      try {
        localStorage.setItem("db-free:query-history-width", String(next));
      } catch {
        // ignore
      }
      return next;
    });
  }, []);

  useEffect(() => {
    const token = { cancelled: false };
    void (async () => {
      try {
        const buffers = await ipc("list_buffers");
        if (token.cancelled) return;
        const mine = buffers.find((b) => b.id === bufferId);
        if (mine && seedSql === undefined) setSql(mine.content);
      } catch (raw) {
        if (!token.cancelled) showError(normalizeError(raw));
      } finally {
        if (!token.cancelled) setLoaded(true);
      }
    })();
    return () => {
      token.cancelled = true;
    };
  }, [bufferId, seedSql, showError]);

  // A tab opened from a saved query (same name, same text) is tied to it, so ⌘S updates that query.
  useEffect(() => {
    if (seedSql === undefined || linkedId !== undefined) return;
    const origin = savedQueries.find((q) => q.name === title && q.sql === seedSql && (q.connectionId === null || q.connectionId === connection.id));
    if (origin !== undefined) linkTab(tabId, origin.id);
  }, [seedSql, linkedId, savedQueries, title, connection.id, linkTab, tabId]);

  // A queued autosave has to die with the tab: closing one deletes its buffer, and
  // a timer that fires afterwards would write the row straight back.
  useEffect(
    () => () => {
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
    },
    [],
  );

  const onChange = useCallback(
    (next: string) => {
      setSql(next);
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => {
        void (async () => {
          try {
            await ipc("save_buffer", { buffer: { id: bufferId, connectionId: connection.id, title, content: next, updatedAt: "" } });
          } catch (raw) {
            showError(normalizeError(raw));
          }
        })();
      }, 500);
    },
    [bufferId, connection.id, showError, title],
  );

  // WHAT:  Sends one script — the whole buffer, the highlighted range, the statement
  //        the caret is in, or the statements under several carets.
  // WHY:   PRD §4.3 — a tab holds a script, and the caller decides how much of it
  //        to run. Whatever is sent is what history logs and what the destructive
  //        gate quotes back, so the confirmation re-runs that exact text.
  // HOW:   The run goes through the store, which streams each statement's result
  //        back; this only decides what to send and what to ask first.
  const launch = useCallback(
    async (pending: PendingRun, confirmDestructive: boolean) => {
      if (useQueryRuns.getState().runs[tabId]?.running === true || pending.script.trim().length === 0) return;
      setConfirm(null);
      try {
        // Manual mode opens the transaction on the first Run after a Commit or
        // Rollback, so every statement until the next one is inside it.
        if (manual && !txOpen) setTxOpen(connection.id, await ipc("begin_transaction", { connectionId: connection.id }));
      } catch (raw) {
        showError(normalizeError(raw));
        return;
      }
      // Settings → Show Results Pane: a run brings the results back if they were folded away.
      if (settings?.showResultsPane !== false) setCollapsed(tabId, false);
      setPlan(null);
      const outcome = await startRun(
        tabId,
        { connectionId: connection.id, script: pending.script, pieces: pending.pieces, schema: schemaFilter, maxRows: rowCap === "none" ? null : Number(rowCap), stopOnError, split: splits },
        confirmDestructive,
      );
      if (outcome.kind === "confirm") setConfirm({ statements: outcome.statements, pending });
      setTxKey((k) => k + 1);
    },
    [connection.id, manual, rowCap, schemaFilter, setCollapsed, setTxOpen, settings?.showResultsPane, showError, splits, startRun, stopOnError, tabId, txOpen],
  );

  // WHAT:  Run for a target: fills in `:name` / `{{name}}` placeholders first.
  const runTarget = useCallback(
    (picked: RunTarget) => {
      if (useQueryRuns.getState().runs[tabId]?.running === true) return;
      const script = joinScript(sql, picked.pieces);
      if (script.trim().length === 0) return;
      const names = findParams(script, isSql);
      if (names.length > 0) {
        setParamPrompt({ script, pieces: picked.pieces, names, then: "run" });
        return;
      }
      void launch({ script, pieces: picked.pieces }, false);
    },
    [isSql, launch, sql, tabId],
  );

  const runAll = useCallback(() => {
    runTarget({ kind: "all", pieces: [{ from: 0, to: sql.length }], count: Math.max(spans.length, 1), index: 0, total: spans.length });
  }, [runTarget, sql.length, spans.length]);

  // WHAT:  Commit / Rollback for Manual mode.
  // HOW:   Both answer whether a transaction is still open; a failed COMMIT
  //        has ended it anyway, so an error also reads as "closed".
  // WHERE: src-tauri/src/services/query.rs (`transaction`)
  const endTransaction = async (commit: boolean) => {
    setTxBusy(true);
    try {
      const req = { connectionId: connection.id };
      setTxOpen(connection.id, commit ? await ipc("commit_transaction", req) : await ipc("rollback_transaction", req));
      showInfo(commit ? "Transaction committed." : "Transaction rolled back.");
    } catch (raw) {
      setTxOpen(connection.id, false);
      showError(normalizeError(raw));
    } finally {
      setTxBusy(false);
      setTxKey((k) => k + 1);
    }
  };

  // WHAT:  Stops the run in flight: the statement running is cancelled on the
  //        server and the ones after it are skipped.
  // WHERE: src-tauri/src/services/query.rs (execute_batch), src/stores/queryRuns.ts
  const stop = useCallback(() => {
    void stopRun(tabId);
  }, [stopRun, tabId]);

  const runCurrent = useCallback(() => {
    if (target !== null) runTarget(target);
    else runAll();
  }, [runAll, runTarget, target]);

  // WHAT:  Where each statement in the buffer starts and ends.
  // WHY:   The gutter ▶ and Run at cursor have to cut the script exactly where the
  //        block will, comments, quotes and $$…$$ included — so the split comes
  //        from the block's own tokenizer instead of a second one written here.
  // HOW:   Debounced: it is pure local text work, but not worth a call per keypress.
  // WHERE: src-tauri/src/guard/destructive.rs (spans)
  useEffect(() => {
    const token = { cancelled: false };
    const timer = window.setTimeout(() => {
      if (sql.trim().length === 0) {
        setSpans([]);
        return;
      }
      void (async () => {
        try {
          const found = await ipc("split_script", { sql });
          if (!token.cancelled) setSpans(found);
        } catch {
          // A split that fails costs the gutter markers, not the query: Run then
          // sends the whole buffer, which is what it did before this existed.
          if (!token.cancelled) setSpans([]);
        }
      })();
    }, 150);
    return () => {
      token.cancelled = true;
      window.clearTimeout(timer);
    };
  }, [sql]);

  // WHAT:  Format: the highlighted text when there is some, else the whole buffer.
  // HOW:   SQL in the engine's own dialect, JSON commands pretty-printed; the edit
  //        goes through the editor so ⌘Z undoes it.
  const doFormat = useCallback(() => {
    const handle = editorRef.current;
    const selection = handle?.selection();
    const range = selection !== undefined && selection.to > selection.from ? selection : { from: 0, to: sql.length };
    const formatted = formatQuery(sql.slice(range.from, range.to), connection.engine, { condense: settings?.condenseSqlWhenFormatting === true });
    if (!formatted.ok) {
      showInfo(formatted.reason);
      return;
    }
    if (handle === null) onChange(sql.slice(0, range.from) + formatted.text + sql.slice(range.to));
    else handle.replace(range.from, range.to, formatted.text, false);
  }, [connection.engine, onChange, settings?.condenseSqlWhenFormatting, showInfo, sql]);

  useShortcut("format", doFormat);

  // WHAT:  Save: a new saved query, or — for a tab tied to one — an overwrite
  //        after a diff, with "Save as new" to keep the original.
  const startSave = useCallback(() => {
    if (sql.trim().length === 0) return;
    if (linked !== null && linked.sql === sql) {
      showInfo(`"${linked.name}" already has this text.`);
      return;
    }
    setSaveOpen(true);
  }, [linked, showInfo, sql]);

  // WHAT:  Tells the tab bar whether closing this tab would throw away SQL that was never saved.
  // WHY:   Closing a tab deletes its autosaved buffer, so text nobody filed is gone with it.
  // HOW:   Clean when the text is empty, equals the saved query the tab is tied to (or,
  //        tied to none, the text it was opened with: a file, a history entry), or was
  //        last written to a .sql file.
  // WHERE: src/stores/workspace.ts (setTabDirty), src/features/shell/CloseConfirmDialog.tsx
  const savedText = linked !== null ? linked.sql : seedSql;
  const dirty = loaded && sql.trim().length > 0 && sql !== savedText && sql !== filedSql;
  useEffect(() => {
    setTabDirty(tabId, dirty);
  }, [dirty, setTabDirty, tabId]);

  // WHAT:  Writes the buffer to a .sql file the user picks.
  // WHY:   PRD §4.3 — a query outlives the tab it was written in: handed over,
  //        committed, opened by another tool.
  // WHERE: src/lib/native.ts (the dialog), src-tauri/src/services/scripts.rs
  const saveToFile = async () => {
    if (sql.trim().length === 0) return;
    setSavingFile(true);
    try {
      const path = await pickSqlSavePath(title);
      if (path !== null) {
        showInfo(`Saved to ${await ipc("save_sql_file", { path, sql })}.`);
        setFiledSql(sql);
      }
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setSavingFile(false);
    }
  };

  // WHAT:  Opens a .sql file from disk into a new query tab.
  // WHY:   Scripts live in files; the editor should take one without a copy and paste.
  // WHERE: src-tauri/src/services/scripts.rs (read)
  const openFile = useCallback(async () => {
    setOpeningFile(true);
    try {
      await openSqlFileInNewTab(connection.id);
    } finally {
      setOpeningFile(false);
    }
  }, [connection.id]);

  // WHAT:  Generate a statement, streamed.
  // WHY:   The editor's assistant used to block for several seconds behind a
  //        spinner with nothing to read. It runs on the agent now with tools
  //        off — one turn, no database access — so the prose arrives as it is
  //        written while the statement still lands in the editor at the end.
  useEffect(() => {
    const pending = onAgentEvent((event: AgentEvent) => {
      if (event.runId !== aiRunRef.current) return;
      if (event.type === "text") setAiText((prev) => (prev ?? "") + event.delta);
      if (event.type === "failed") setAiText((prev) => prev ?? null);
    });
    return () => {
      void pending.then((unlisten) => { unlisten(); });
    };
  }, []);

  /// The message of the first statement that failed in the last run.
  const lastError = useMemo(() => run?.statements.find((s) => s.state === "error")?.error?.message ?? run?.fatal?.message ?? null, [run?.statements, run?.fatal]);

  const askAi = async (overridePrompt?: string) => {
    const promptText = (overridePrompt ?? aiPrompt).trim();
    if (promptText.length === 0) return;
    if (overridePrompt) setAiPrompt(overridePrompt);
    const nextRun = crypto.randomUUID();
    aiRunRef.current = nextRun;
    setAiBusy(true);
    setAiText("");
    setAiGeneratedSql(null);

    const context = [
      sql.trim().length > 0 ? `Current editor statement:\n\`\`\`\n${sql.trim()}\n\`\`\`` : "",
      lastError !== null ? `The last run failed with:\n${lastError}` : "",
    ]
      .filter((part) => part.length > 0)
      .join("\n\n");

    try {
      const turn = await ipc("agent_chat", {
        connectionId: connection.id,
        chatId: `editor:${connection.id}`,
        runId: nextRun,
        prompt: promptText,
        context: context.length > 0 ? context : null,
        useTools: false,
      });
      setAiText(turn.text);
      setAiGeneratedSql(turn.sql);
      if (turn.sql !== null && sql.trim().length === 0) {
        onChange(turn.sql);
        showInfo("Generated query placed in editor.");
      }
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      if (aiRunRef.current === nextRun) aiRunRef.current = null;
      setAiBusy(false);
    }
  };

  // WHAT:  ⌘I: the prompt card at the caret, for writing or rewriting code in place.
  const openInlineAi = useCallback(() => {
    const handle = editorRef.current;
    if (handle === null) return;
    const selection = handle.selection();
    const range = handle.statementRange();
    const statementText = selection.text.trim().length > 0 ? selection.text : range === null ? sql : sql.slice(range.from, range.to);
    const parts = [
      statementText.trim().length > 0 ? `Current ${language}:\n\`\`\`\n${statementText.trim().slice(0, 6000)}\n\`\`\`` : "",
      lastError !== null ? `The last run failed with:\n${lastError}` : "",
    ].filter((part) => part.length > 0);
    inlineCounter.current += 1;
    setInlineAi({ key: inlineCounter.current, selection, anchor: handle.caretBox(), context: parts.length > 0 ? parts.join("\n\n") : null });
  }, [language, lastError, sql]);

  // WHAT:  "Ask AI about this SQL": the highlighted text, else the statement the
  //        caret is in, handed to the chat with the last error.
  const askAboutSql = () => {
    const handle = editorRef.current;
    const selection = handle?.selection();
    const range = handle?.statementRange() ?? null;
    const text = selection !== undefined && selection.text.trim().length > 0 ? selection.text : range === null ? sql : sql.slice(range.from, range.to);
    if (text.trim().length === 0) return;
    setAiOpen(false);
    openChat(connection.id, askAboutPrompt(text, language, lastError));
  };

  const explain = async (bound?: string) => {
    const range = editorRef.current?.statementRange() ?? null;
    const script = bound ?? (range === null ? sql : sql.slice(range.from, range.to));
    if (script.trim().length === 0) return;
    if (bound === undefined) {
      const names = findParams(script, isSql);
      if (names.length > 0) {
        setParamPrompt({ script, pieces: [], names, then: "explain" });
        return;
      }
    }
    setPlanBusy(true);
    try {
      // The same statement Run would send: a plan for a whole script is not one.
      setPlan(await ipc("explain_query", { connectionId: connection.id, sql: script }));
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setPlanBusy(false);
    }
  };

  const schema = useMemo<SQLNamespace>(() => buildNamespace(connection.id, catalog, columnsCache), [connection.id, catalog, columnsCache]);
  // Unqualified names complete against the schema the tab is pointed at; the
  // Rust side resolves them there too (`Integration::use_namespace`).
  const defaultSchema = schemaFilter ?? (connection.engine === "postgres" ? "public" : undefined);
  const aiEnabled = settings !== null && settings.ai.provider !== "none";

  // WHAT:  Fetches the columns of the tables the statement being written names, so
  //        `alias.` completes real column names without loading the whole schema.
  const loadMentioned = useCallback(
    (names: string[]) => {
      const tables = (catalog?.schemas ?? []).flatMap((s) => s.tables);
      for (const name of names.slice(0, 6)) {
        const parts = name.split(".");
        const bare = (parts[parts.length - 1] ?? "").toLowerCase();
        const schemaName = parts.length > 1 ? (parts[parts.length - 2] ?? "").toLowerCase() : null;
        const matches = tables.filter((t) => t.name.toLowerCase() === bare && (schemaName === null || (t.schema ?? "").toLowerCase() === schemaName));
        const table = matches.find((t) => t.schema === defaultSchema) ?? matches[0];
        if (table === undefined) continue;
        const ref: TableRef = { schema: table.schema, name: table.name };
        void loadColumns(connection.id, ref).catch(() => undefined);
      }
    },
    [catalog, connection.id, defaultSchema, loadColumns],
  );

  // WHAT:  The squiggle for the last run's first positioned error, while the text
  //        it points into is unchanged.
  const diagnostic = useMemo<EditorDiagnostic | null>(() => {
    if (run === undefined || run.running || run.pieces.length === 0 || joinScript(sql, run.pieces) !== run.script) return null;
    const failed = run.statements.find((s) => s.state === "error" && s.error?.kind === "driver" && s.error.position !== null);
    if (failed?.error?.kind !== "driver" || failed.error.position === null) return null;
    return { at: scriptOffsetToDoc(run.pieces, failed.start + failed.error.position - 1), message: failed.error.message, hint: failed.error.hint, detail: failed.error.detail };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- recomputed when the run changes, not on every keystroke
  }, [run]);

  const goToError = (statement: StatementRun) => {
    if (run === undefined || statement.error?.kind !== "driver" || statement.error.position === null) return;
    if (joinScript(sql, run.pieces) !== run.script) {
      showInfo("The query has changed since this run, so the position no longer applies.");
      return;
    }
    editorRef.current?.reveal(scriptOffsetToDoc(run.pieces, statement.start + statement.error.position - 1));
  };

  const fixWithAi = (statement: StatementRun) => {
    const error: AppError | null = statement.error;
    if (error === null) return;
    const position = error.kind === "driver" && error.position !== null ? lineColumnOf(statement.sql, error.position) : null;
    openChat(connection.id, fixErrorPrompt(statement.sql, language, error.message, position));
  };

  // WHAT:  What the Run button will send, said out loud.
  // WHY:   Run means different things depending on where the caret is and the
  //        run-scope setting; the button has to admit which one, or a script
  //        gets run by surprise.
  const label = runLabel(target);
  const empty = sql.trim().length === 0;
  const multi = spans.length > 1;

  // WHAT:  The database / schema this tab runs against, next to Run rather than
  //        only in the sidebar — a query is written against a namespace.
  // WHY:   Both pickers are the connection's own state, so the tab, the sidebar
  //        and the object explorer can never disagree about where a name lives.
  const databases = info?.databases ?? [];
  const currentDb = info?.database ?? "";
  const dbOptions = (databases.length > 0 ? databases : currentDb.length > 0 ? [currentDb] : []).map((d) => ({ value: d, label: d }));
  const schemas = catalog?.schemas ?? [];
  const schemaOptions = [{ value: "*", label: "All schemas" }, ...schemas.map((x) => ({ value: x.name, label: x.name }))];
  const showSchemas = (info?.capabilities.namespaces ?? false) && (schemas.length > 1 || (schemas[0] !== undefined && schemas[0].name !== "main"));

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex app-toolbar shrink-0 items-center gap-2 border-b border-border/40 glass-header ">
        {running ? (
          // While a run is in flight, Run becomes Stop in the same spot, so the
          // hand that started it finds the way out without looking.
          <Button size="sm" variant="danger" pending={stopping} onClick={stop} className="gap-2 rounded-md font-semibold liquid-hover">
            <Icon name="stop" size={12} />
            Stop
          </Button>
        ) : (
          <Button
            size="sm"
            onClick={runCurrent}
            disabled={!loaded || empty}
            className="gap-2 rounded-md pr-1.5 font-semibold liquid-hover"
          >
            <Icon name="play" size={12} />
            {label}
            {/* The chord lives inside the button: one control, not a button with a
                loose hint parked next to it. */}
            <RunShortcut className="bg-accent-foreground/15 text-accent-foreground/85" />
          </Button>
        )}
        {multi && target?.kind !== "all" ? (
          <Button
            size="sm"
            variant="ghost"
            className="rounded-md text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
            onClick={runAll}
            disabled={!loaded || running || empty}
          >
            Run all {spans.length}
          </Button>
        ) : null}
        {multi ? (
          <Toggle
            pressed={stopOnError}
            onPressedChange={(next) => {
              setStopOnError(next);
              try {
                localStorage.setItem(STOP_ON_ERROR_KEY, next ? "1" : "0");
              } catch {
                // the choice just does not survive a restart
              }
            }}
            aria-label="Stop at the first error"
            title="Stop running a script at the first statement that fails"
            className="h-6 gap-1 rounded-md px-2 text-xs"
          >
            Stop on error
          </Toggle>
        ) : null}
        <Button size="sm" variant="ghost" className="rounded-md text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover" onClick={doFormat} disabled={empty}>
          Format
        </Button>
        {isSql ? (
          <Button
            size="sm"
            variant="ghost"
            className="rounded-md text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
            pending={planBusy}
            onClick={() => void explain()}
            disabled={empty}
          >
            Explain
          </Button>
        ) : null}
        <Button size="sm" variant="ghost" className="rounded-md text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover" onClick={startSave} disabled={empty}>
          {linked !== null ? "Update" : "Save"}
        </Button>
        {linked !== null ? (
          <IconButton icon="history" label={`Versions of "${linked.name}"`} onClick={() => { setVersionsOpen(true); }} />
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          className="rounded-md text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
          pending={openingFile}
          onClick={() => void openFile()}
          title="Open a .sql file in a new tab"
        >
          <Icon name="file" size={12} />
          Open
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="rounded-md text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
          pending={savingFile}
          onClick={() => void saveToFile()}
          disabled={empty}
        >
          <Icon name="download" size={12} />
          .sql
        </Button>
        <Popover open={aiOpen} onOpenChange={setAiOpen}>
          <PopoverTrigger asChild>
            <Button size="sm" variant={aiEnabled ? "secondary" : "ghost"} className={cn("rounded-md liquid-hover", aiEnabled ? "glass-pill text-accent" : "text-muted hover:bg-surface-secondary/70 hover:text-foreground")}>
              <Icon name="braces" size={12} />
              AI
            </Button>
          </PopoverTrigger>
          <PopoverContent className="w-[460px] rounded-lg glass-modal">
              <div className="flex items-center justify-between">
                <PopoverHeading className="text-[13px] font-semibold text-foreground">AI Database Assistant</PopoverHeading>
                <div className="flex items-center gap-1.5">
                  <span className="rounded px-1.5 py-0.5 text-[10px] font-medium bg-surface-secondary text-muted">
                    {engineMeta(connection.engine).label}
                  </span>
                  {sql.trim().length > 0 ? (
                    <span className="rounded px-1.5 py-0.5 text-[10px] font-medium bg-accent/15 text-accent">
                      Editor Query
                    </span>
                  ) : null}
                  {lastError !== null ? (
                    <span className="rounded px-1.5 py-0.5 text-[10px] font-medium bg-danger-soft text-danger">
                      Error Context
                    </span>
                  ) : null}
                </div>
              </div>
              {aiEnabled ? (
                <>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {lastError !== null ? (
                      <Button
                        size="sm"
                        variant="outline"
                        className="h-6 rounded-full border-danger/40 bg-danger-soft/40 px-2 text-[11px] text-danger hover:bg-danger-soft liquid-hover"
                        onClick={() => void askAi("Fix the error in my query")}
                      >
                        <Icon name="refresh" size={10} />
                        Fix Error
                      </Button>
                    ) : null}
                    {sql.trim().length > 0 ? (
                      <>
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 rounded-full border-border/60 bg-surface-secondary/50 px-2 text-[11px] text-muted hover:text-foreground hover:bg-surface-secondary liquid-hover"
                          onClick={() => void askAi("Optimize this query for performance and explain")}
                        >
                          Optimize
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 rounded-full border-border/60 bg-surface-secondary/50 px-2 text-[11px] text-muted hover:text-foreground hover:bg-surface-secondary liquid-hover"
                          onClick={() => void askAi("Add pagination using LIMIT and OFFSET")}
                        >
                          Paginate
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 rounded-full border-border/60 bg-surface-secondary/50 px-2 text-[11px] text-muted hover:text-foreground hover:bg-surface-secondary liquid-hover"
                          onClick={() => void askAi("Explain what this query does in plain language")}
                        >
                          Explain
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 rounded-full border-border/60 bg-surface-secondary/50 px-2 text-[11px] text-muted hover:text-foreground hover:bg-surface-secondary liquid-hover"
                          onClick={askAboutSql}
                        >
                          <Icon name="message" size={10} />
                          Ask in chat
                        </Button>
                      </>
                    ) : (
                      <>
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 rounded-full border-border/60 bg-surface-secondary/50 px-2 text-[11px] text-muted hover:text-foreground hover:bg-surface-secondary liquid-hover"
                          onClick={() => void askAi("List top 10 rows ordered by latest date")}
                        >
                          Top 10 Rows
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-6 rounded-full border-border/60 bg-surface-secondary/50 px-2 text-[11px] text-muted hover:text-foreground hover:bg-surface-secondary liquid-hover"
                          onClick={() => void askAi("Count total rows grouped by status")}
                        >
                          Count by Status
                        </Button>
                      </>
                    )}
                  </div>
                  <Textarea
                    value={aiPrompt}
                    onChange={(event) => { setAiPrompt(event.target.value); }}
                    aria-label="Prompt"
                    placeholder="Ask to generate, modify, explain, or fix a query…"
                    rows={3}
                    className="mt-2 w-full"
                  />
                  <div className="mt-2 flex items-center justify-between">
                    <Button size="sm" pending={aiBusy} onClick={() => void askAi()} disabled={aiPrompt.trim().length === 0} className="font-semibold liquid-hover">
                      Generate {engineMeta(connection.engine).commandLanguage}
                    </Button>
                    <span className="text-[11px] text-muted">Schema, context & prompt sent to {settings.ai.provider}.</span>
                  </div>
                  {aiGeneratedSql !== null ? (
                    <div className="mt-2 rounded-lg border border-border/60 bg-surface-secondary/70 p-2">
                      <div className="mb-1.5 flex items-center justify-between">
                        <span className="text-[11px] font-semibold text-foreground tracking-tight">Generated Statement</span>
                        <div className="flex items-center gap-1">
                          <Button
                            size="sm"
                            variant="ghost"
                            className="h-5 px-1.5 text-[10.5px] rounded text-muted hover:text-foreground liquid-hover"
                            onClick={() => {
                              void navigator.clipboard.writeText(aiGeneratedSql);
                              showInfo("Copied statement to clipboard.");
                            }}
                          >
                            Copy
                          </Button>
                          <Button
                            size="sm"
                            variant="ghost"
                            className="h-5 px-1.5 text-[10.5px] rounded text-muted hover:text-foreground liquid-hover"
                            onClick={() => {
                              onChange(sql.trim().length > 0 ? `${sql.trimEnd()}\n\n${aiGeneratedSql}` : aiGeneratedSql);
                              showInfo("Inserted statement below.");
                            }}
                          >
                            Insert Below
                          </Button>
                          <Button
                            size="sm"
                            variant="secondary"
                            className="h-5 px-2 text-[10.5px] rounded font-medium glass-pill text-accent liquid-hover"
                            onClick={() => {
                              onChange(aiGeneratedSql);
                              showInfo("Replaced editor query.");
                            }}
                          >
                            Replace Editor
                          </Button>
                        </div>
                      </div>
                      <ScrollArea className="max-h-32">
                        <pre className="selectable font-mono text-[11px] text-foreground whitespace-pre-wrap">{aiGeneratedSql}</pre>
                      </ScrollArea>
                    </div>
                  ) : null}
                  {aiText !== null && aiText.length > 0 ? (
                    <ScrollArea className="mt-2 max-h-36 rounded-lg border border-border/40 bg-surface/60 p-2">
                      <Markdown
                        text={aiText}
                        language={engineMeta(connection.engine).commandLanguage}
                        streaming={aiBusy}
                        onCopy={(code) => {
                          void navigator.clipboard.writeText(code);
                          showInfo("Copied to clipboard.");
                        }}
                      />
                    </ScrollArea>
                  ) : null}
                </>
              ) : (
                <p className="mt-2 text-xs text-muted">Turn on a provider in Settings → AI (bring your own key).</p>
              )}
          </PopoverContent>
        </Popover>
        {txCapable ? (
          // WHAT:  Auto-commit / Manual. In Manual the first Run opens a
          //        transaction that stays open until Commit or Rollback.
          // WHY:   Auto cannot be picked back while one is open: leaving it
          //        open behind an Auto label is how work gets lost.
          <div className="flex items-center gap-1.5">
            <Segmented<TxMode>
              label="Transaction mode"
              value={manual ? "manual" : "auto"}
              options={[
                { value: "auto", label: "Auto-commit", disabled: txOpen },
                { value: "manual", label: "Manual" },
              ]}
              onChange={(mode) => setManual(tabId, mode === "manual")}
            />
            {txOpen ? (
              <>
                <Badge variant="warning" className="gap-1">
                  <Icon name="lock" size={10} />
                  Transaction open
                </Badge>
                <Button size="sm" variant="soft" className="rounded-md liquid-hover" pending={txBusy} disabled={running} onClick={() => void endTransaction(true)}>
                  <Icon name="check" size={12} />
                  Commit
                </Button>
                <Button size="sm" variant="ghost" className="rounded-md text-danger hover:bg-danger-soft liquid-hover" disabled={running || txBusy} onClick={() => void endTransaction(false)}>
                  <Icon name="refresh" size={12} />
                  Rollback
                </Button>
              </>
            ) : null}
          </div>
        ) : null}
        <div className="ml-auto flex items-center gap-2">
          {/* WHAT:  Live-connection dot beside the database picker, so a connected
              tab shows green at full opacity and a stale one dims to 35%. */}
          <EnvDot environment={connection.environment} live={live} />
          {dbOptions.length > 0 || showSchemas ? (
            <div className="flex items-center gap-1 text-xs text-muted">
              {dbOptions.length > 0 ? (
                <AppSelect
                  ariaLabel="Database"
                  value={currentDb}
                  options={dbOptions}
                  plain
                  className="w-auto min-w-0"
                  icon="database"
                  disabled={connecting === connection.id}
                  onChange={(db) => void switchDatabase(connection.id, db)}
                />
              ) : null}
              {showSchemas ? (
                <>
                  {dbOptions.length > 0 ? <span className="px-0.5 text-muted/60">/</span> : null}
                  <AppSelect
                    ariaLabel="Schema"
                    value={schemaFilter ?? "*"}
                    options={schemaOptions}
                    plain
                    className="w-auto min-w-0"
                    icon="folder"
                    onChange={(v) => setSchemaFilter(connection.id, v === "*" ? null : v)}
                  />
                </>
              ) : null}
            </div>
          ) : null}
          <AppSelect ariaLabel="Row cap" value={rowCap} options={ROW_CAPS} onChange={setRowCap} size="sm" className="w-32" />
          <IconButton icon="history" label="Query history" active={showHistory} onClick={() => setShowHistory((v) => !v)} />
        </div>
      </div>
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <div className={cn("relative flex flex-col", collapsed && !plan ? "min-h-0 flex-1" : "shrink-0")} style={collapsed && !plan ? undefined : { height: editorHeight }}>
            {loaded ? (
              <SqlEditor
                value={sql}
                spans={spans}
                onChange={onChange}
                onRun={runTarget}
                onRunAll={runAll}
                onTargetChange={setTarget}
                engine={connection.engine}
                schema={schema}
                defaultSchema={defaultSchema}
                runScope={settings?.runScope ?? "all"}
                diagnostic={diagnostic}
                onMentionedTables={loadMentioned}
                onInlineAi={openInlineAi}
                onSave={startSave}
                onOpenFile={() => void openFile()}
                handleRef={editorRef}
              />
            ) : null}
            {inlineAi !== null ? (
              <InlineAiPrompt
                key={inlineAi.key}
                connection={connection}
                editorRef={editorRef}
                selection={inlineAi.selection}
                anchor={inlineAi.anchor}
                documentText={() => sql}
                context={inlineAi.context}
                onClose={() => { setInlineAi(null); }}
              />
            ) : null}
          </div>
          {collapsed && !plan ? null : <Resizer direction="vertical" onResize={handleEditorResize} />}
          <div className={cn("min-h-0", collapsed && !plan ? "h-7 shrink-0" : "flex-1")}>
            {plan ? (
              <div className="flex h-full min-h-0 flex-col">
                <div className="flex h-7 shrink-0 items-center gap-2 border-b border-border/40 glass-header px-2.5 text-xs">
                  <span className="font-semibold text-foreground tracking-tight">Execution plan</span>
                  {plan.planTree !== null ? (
                    <Segmented<PlanView>
                      label="Plan view"
                      value={planView}
                      options={[
                        { value: "tree", label: "Tree" },
                        { value: "text", label: "Text" },
                      ]}
                      onChange={setPlanView}
                    />
                  ) : null}
                  <span className="ml-auto">
                    <Button variant="ghost" size="icon-sm" aria-label="Close plan" onClick={() => setPlan(null)}><Icon name="x" /></Button>
                  </span>
                </div>
                <div className="grid min-h-0 flex-1 grid-cols-2 gap-0">
                  <ScrollArea className="overflow-x-auto border-r border-border/40 p-2.5">
                    {plan.planTree !== null && planView === "tree" ? (
                      <PlanTree root={plan.planTree} />
                    ) : (
                      <pre className="selectable font-mono text-[11px] text-foreground">{plan.plan}</pre>
                    )}
                  </ScrollArea>
                  <ScrollArea className="selectable p-2.5 text-xs whitespace-pre-wrap text-muted">{plan.explanation ?? "Enable an AI provider in Settings to get a plain-language explanation of this plan."}</ScrollArea>
                </div>
              </div>
            ) : (
              <ResultsPane
                run={run}
                connectionId={connection.id}
                collapsed={collapsed}
                onToggleCollapsed={() => { setCollapsed(tabId, !collapsed); }}
                onGoToError={goToError}
                onAskAi={aiEnabled ? fixWithAi : undefined}
                onRunWithoutLimit={(statement) => { void rerunStatement(tabId, statement.index, null); }}
              />
            )}
          </div>
        </div>
        {showHistory ? (
          <div className="relative shrink-0 flex flex-col border-l border-border/40 glass-sidebar select-none" style={{ width: historyWidth }}>
            <Resizer direction="horizontal" onResize={handleHistoryResize} className="absolute -left-1 top-0 bottom-0" />
            <HistoryPanel
              connectionId={connection.id}
              refreshKey={(run?.seq ?? 0) * 4 + (running ? 1 : 0) + txKey * 8}
              onPick={(picked) => {
                const handle = editorRef.current;
                if (handle === null) onChange(picked);
                else handle.replace(0, sql.length, picked, false);
              }}
            />
          </div>
        ) : null}
      </div>

      {saveOpen ? (
        <SaveQueryDialog
          open
          onOpenChange={setSaveOpen}
          connectionId={connection.id}
          sql={sql}
          linked={linked}
          extra={<SaveFolderField />}
          onSaved={(saved) => { linkTab(tabId, saved.id); }}
        />
      ) : null}

      {versionsOpen && linked !== null ? (
        <QueryVersionsDialog
          query={linked}
          onOpenVersion={(versionSql, label) => {
            openQuery(connection.id, versionSql, label);
            setVersionsOpen(false);
          }}
          onRestored={(restored) => {
            // The saved query now holds the restored text; the tab follows it.
            const handle = editorRef.current;
            if (handle === null) onChange(restored.sql);
            else handle.replace(0, sql.length, restored.sql, false);
          }}
          onClose={() => { setVersionsOpen(false); }}
        />
      ) : null}

      <Dialog open={confirm !== null} onOpenChange={(open) => !open && setConfirm(null)}>
        <DialogContent className="sm:max-w-[520px]">
          <DialogHeader>
            <span className="bg-danger-soft text-danger">
              <Icon name="trash" size={16} />
            </span>
            <DialogTitle>Run destructive statements?</DialogTitle>
          </DialogHeader>
          <DialogBody className="space-y-2">
            <Alert variant="danger" className="rounded-lg">
              <AlertIndicator />
              <AlertContent>
                <AlertTitle className="font-semibold text-xs">Destructive Operations</AlertTitle>
                <AlertDescription className="text-xs">
                  These statements change or remove data without a safety net. Review before continuing.
                </AlertDescription>
              </AlertContent>
            </Alert>
            <ul className="flex flex-col gap-1.5">
              {confirm?.statements.map((s) => (
                <li key={s} className="selectable rounded-md bg-danger-soft px-2 py-1 font-mono text-[11px] text-danger">
                  {s}
                </li>
              ))}
            </ul>
          </DialogBody>
          <DialogFooter>
            <Button variant="tertiary" onClick={() => setConfirm(null)}>Cancel</Button>
            <Button variant="danger" onClick={() => { if (confirm !== null) void launch(confirm.pending, true); }}>Run anyway</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {paramPrompt ? (
        <ParamsDialog
          names={paramPrompt.names}
          initial={paramValues}
          onCancel={() => setParamPrompt(null)}
          onRun={(values) => {
            const pending = paramPrompt;
            setParamValues((prev) => ({ ...prev, ...values }));
            setParamPrompt(null);
            const filled = bindParams(pending.script, isSql, values);
            if (pending.then === "explain") void explain(filled);
            // The values change the script's length, so its positions no longer line up with the editor's text.
            else void launch({ script: filled, pieces: [] }, false);
          }}
        />
      ) : null}
      {aiBusy ? <span className="sr-only"><Spinner size="sm" /></span> : null}
    </div>
  );
}

function buildNamespace(
  connectionId: string,
  catalog: ReturnType<typeof useWorkspace.getState>["catalogs"][string] | undefined,
  columnsCache: ReturnType<typeof useWorkspace.getState>["columnsCache"],
): SQLNamespace {
  if (!catalog) return {};
  const out: Record<string, Record<string, string[]> | string[]> = {};
  for (const schema of catalog.schemas) {
    const tables: Record<string, string[]> = {};
    for (const table of schema.tables) {
      const key = `${connectionId}:${table.schema === null ? table.name : `${table.schema}.${table.name}`}`;
      tables[table.name] = (columnsCache[key] ?? []).map((c) => c.name);
    }
    if (schema.tables.some((t) => t.schema !== null)) {
      out[schema.name] = tables;
    } else {
      for (const [name, cols] of Object.entries(tables)) out[name] = cols;
    }
  }
  return out;
}

const PARAM_MODES = [
  { value: "auto", label: "Auto" },
  { value: "text", label: "Text" },
  { value: "raw", label: "Raw SQL" },
] satisfies readonly { value: ParamMode; label: string }[];

// WHAT:  Asks for every placeholder in the script before it runs, pre-filled
//        with the values this tab used last, and previews each literal.
// WHERE: src/lib/params.ts
function ParamsDialog({ names, initial, onCancel, onRun }: { names: readonly string[]; initial: Readonly<Record<string, ParamValue>>; onCancel: () => void; onRun: (values: Record<string, ParamValue>) => void }) {
  const [values, setValues] = useState<Record<string, ParamValue>>(() => Object.fromEntries(names.map((n) => [n, initial[n] ?? { value: "", mode: "auto" }])));
  const set = (name: string, next: Partial<ParamValue>) => setValues((prev) => ({ ...prev, [name]: { value: prev[name]?.value ?? "", mode: prev[name]?.mode ?? "auto", ...next } }));
  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-[560px]">
        <DialogHeader>
          <DialogTitle>Query parameters</DialogTitle>
        </DialogHeader>
        <DialogBody className="flex max-h-[60vh] flex-col gap-2 overflow-y-auto">
          <p className="text-xs text-muted">Auto leaves numbers, TRUE/FALSE and NULL bare and quotes everything else. Raw SQL is inserted as typed.</p>
          <form
            id="query-params"
            className="flex flex-col gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              onRun(values);
            }}
          >
            {names.map((name, i) => {
              const param = values[name] ?? { value: "", mode: "auto" };
              return (
                <div key={name} className="flex items-end gap-2">
                  <Field label={name} value={param.value} onChange={(value) => set(name, { value })} autoFocus={i === 0} mono className="min-w-0 flex-1" />
                  <AppSelect<ParamMode> ariaLabel={`${name} type`} value={param.mode} options={PARAM_MODES} onChange={(mode) => set(name, { mode })} className="w-28" />
                  <code className="mb-1 w-28 truncate font-mono text-[11px] text-muted" title={paramLiteral(param)}>{paramLiteral(param)}</code>
                </div>
              );
            })}
          </form>
        </DialogBody>
        <DialogFooter>
          <Button variant="tertiary" onClick={onCancel}>Cancel</Button>
          <Button type="submit" form="query-params">Run</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
