// SOT: query-pane, run-query-flow, run-at-cursor-flow, destructive-confirm-flow, buffer-autosave, save-query-flow, save-sql-file-flow, ai-assist-flow, explain-flow, format-sql
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { format as formatSql } from "sql-formatter";
import type { AgentEvent, AppError, ConnectionSummary, PlanReport, QueryOutcome, StatementSpan } from "@/lib/bindings";
import type { SQLNamespace } from "@codemirror/lang-sql";
import { ipc, normalizeError, onAgentEvent } from "@/lib/ipc";
import { engineMeta } from "@/lib/engines";
import { Markdown } from "@/features/chat/Markdown";
import { pickSqlSavePath } from "@/lib/native";
import { useWorkspace } from "@/stores/workspace";
import { AppSelect, Field } from "@/components/global/Field";
import { EnvDot } from "@/components/global/Badge";
import { IconButton } from "@/components/global/Button";
import { Resizer } from "@/components/global/Resizer";
import { RunShortcut } from "@/components/global/Kbd";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { SqlEditor, type RunTarget } from "./SqlEditor";
import { ResultsPane } from "./ResultsPane";
import { HistoryPanel } from "./HistoryPanel";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Popover, PopoverContent, PopoverHeading, PopoverTrigger } from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";

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

function defaultRowCap(max: number | undefined): (typeof ROW_CAPS)[number]["value"] {
  const wanted = String(max ?? 1000);
  return ROW_CAPS.find((c) => c.value === wanted)?.value ?? "1000";
}

interface QueryPaneProps {
  connection: ConnectionSummary;
  tabId: string;
  /// The tab's name. Stored with the buffer, so a restored tab keeps it.
  title: string;
  seedSql?: string | undefined;
}

// WHAT:  SQL workbench tab: editor, run, results, history, autosaved buffer,
//        save-as-query, AI assist and plan explanation.
// WHY:   PRD §4.3 / §4.5 — execution runs on the Rust side; the UI awaits the
//        outcome. Destructive statements bounce back as a typed error the user
//        confirms explicitly.
// WHERE: src-tauri/src/guard/mod.rs, src-tauri/src/services/ai.rs
export function QueryPane({ connection, tabId, title, seedSql }: QueryPaneProps) {
  const catalog = useWorkspace((s) => s.catalogs[connection.id]);
  const live = useWorkspace((s) => s.sessions.includes(connection.id));
  const info = useWorkspace((s) => s.sessionInfos[connection.id]);
  const schemaFilter = useWorkspace((s) => s.schemaFilter[connection.id] ?? null);
  const setSchemaFilter = useWorkspace((s) => s.setSchemaFilter);
  const switchDatabase = useWorkspace((s) => s.switchDatabase);
  const connecting = useWorkspace((s) => s.connecting);
  const columnsCache = useWorkspace((s) => s.columnsCache);
  const settings = useWorkspace((s) => s.settings);
  const saveQuery = useWorkspace((s) => s.saveQuery);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const bufferId = tabId;
  const isSql = engineMeta(connection.engine).commandLanguage === "SQL";

  const [sql, setSql] = useState(seedSql ?? "");
  const [loaded, setLoaded] = useState(false);
  const [running, setRunning] = useState(false);
  const [outcome, setOutcome] = useState<QueryOutcome | null>(null);
  const [rowCap, setRowCap] = useState<(typeof ROW_CAPS)[number]["value"]>(() => defaultRowCap(settings?.maxQueryRows));
  const [confirm, setConfirm] = useState<{ statements: string[]; script: string } | null>(null);
  const [spans, setSpans] = useState<readonly StatementSpan[]>([]);
  const [target, setTarget] = useState<RunTarget | null>(null);
  const [savingFile, setSavingFile] = useState(false);
  const [historyKey, setHistoryKey] = useState(0);
  const [showHistory, setShowHistory] = useState(false);
  const [saveOpen, setSaveOpen] = useState(false);
  const [saveName, setSaveName] = useState("");
  const [saveTags, setSaveTags] = useState("");
  const [aiOpen, setAiOpen] = useState(false);
  const [aiPrompt, setAiPrompt] = useState("");
  const [aiBusy, setAiBusy] = useState(false);
  const [aiText, setAiText] = useState<string | null>(null);
  const [aiGeneratedSql, setAiGeneratedSql] = useState<string | null>(null);
  /// The turn this pane is showing; frames from any other run are ignored.
  const aiRunRef = useRef<string | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);
  const [plan, setPlan] = useState<PlanReport | null>(null);
  const [planBusy, setPlanBusy] = useState(false);
  const saveTimer = useRef<number | null>(null);

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
      return saved ? Math.max(220, Math.min(600, Number(saved))) : 288;
    } catch {
      return 288;
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

  // WHAT:  Sends one script — the whole buffer, the highlighted range, or the
  //        single statement the caret is in.
  // WHY:   PRD §4.3 — a tab holds a script, and the caller decides how much of it
  //        to run. Whatever is sent is what history logs and what the destructive
  //        gate quotes back, so the confirmation re-runs that exact text.
  const run = useCallback(
    async (script: string, confirmDestructive: boolean) => {
      if (running || script.trim().length === 0) return;
      setRunning(true);
      setConfirm(null);
      try {
        const result = await ipc("execute_query", {
          connectionId: connection.id,
          sql: script,
          confirmDestructive,
          maxRows: rowCap === "none" ? null : Number(rowCap),
          schema: schemaFilter,
        });
        setOutcome(result);
        setLastError(null);
      } catch (raw) {
        const error: AppError = normalizeError(raw);
        setLastError(error.message);
        if (error.kind === "destructive_confirmation_required") setConfirm({ statements: error.statements, script });
        else showError(error);
      } finally {
        setRunning(false);
        setHistoryKey((k) => k + 1);
      }
    },
    [connection.id, rowCap, running, schemaFilter, showError],
  );

  const runCurrent = useCallback(() => {
    void run(target?.text ?? sql, false);
  }, [run, sql, target]);

  const runAll = useCallback(() => {
    void run(sql, false);
  }, [run, sql]);

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

  const doFormat = useCallback(() => {
    if (!isSql) return;
    try {
      const language = connection.engine === "postgres" ? "postgresql" : connection.engine === "mysql" || connection.engine === "mariadb" ? "mysql" : connection.engine === "sqlite" ? "sqlite" : "sql";
      onChange(formatSql(sql, { language, keywordCase: "upper", expressionWidth: settings?.condenseSqlWhenFormatting ? 120 : 50 }));
    } catch (raw) {
      showError(normalizeError(raw));
    }
  }, [connection.engine, isSql, onChange, settings?.condenseSqlWhenFormatting, showError, sql]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.shiftKey && e.altKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        doFormat();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [doFormat]);

  const doSave = async () => {
    try {
      await saveQuery({ id: "", connectionId: connection.id, name: saveName, sql, tags: saveTags.split(",").map((t) => t.trim()).filter((t) => t.length > 0), createdAt: "", updatedAt: "" });
      setSaveOpen(false);
      setSaveName("");
      setSaveTags("");
      showInfo(`Saved "${saveName}".`);
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  // WHAT:  Writes the buffer to a .sql file the user picks.
  // WHY:   PRD §4.3 — a query outlives the tab it was written in: handed over,
  //        committed, opened by another tool.
  // WHERE: src/lib/native.ts (the dialog), src-tauri/src/services/scripts.rs
  const saveToFile = async () => {
    if (sql.trim().length === 0) return;
    setSavingFile(true);
    try {
      const path = await pickSqlSavePath(title);
      if (path !== null) showInfo(`Saved to ${await ipc("save_sql_file", { path, sql })}.`);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setSavingFile(false);
    }
  };

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
      void pending.then((unlisten) => unlisten());
    };
  }, []);

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

  const explain = async () => {
    const script = target?.text ?? sql;
    if (script.trim().length === 0) return;
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

  // WHAT:  What the Run button will send, said out loud.
  // WHY:   Run means three different things depending on where the caret is; the
  //        button has to admit which one, or a script gets run by surprise.
  const statement = target?.selected === false ? target.index : 0;
  const runLabel = target?.selected === true ? "Run selection" : spans.length > 1 && statement > 0 ? `Run statement ${statement}` : "Run";
  const empty = sql.trim().length === 0;

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
        <Button
          size="sm"
          pending={running}
          onClick={runCurrent}
          disabled={!loaded || empty}
          className="gap-2 rounded-lg pr-1.5 font-semibold liquid-hover"
        >
          <Icon name="play" size={12} />
          {runLabel}
          {/* The chord lives inside the button: one control, not a button with a
              loose hint parked next to it. */}
          <RunShortcut className="bg-accent-foreground/15 text-accent-foreground/85" />
        </Button>
        {spans.length > 1 ? (
          <Button
            size="sm"
            variant="ghost"
            className="rounded-lg text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
            onClick={runAll}
            disabled={!loaded || running || empty}
          >
            Run all {spans.length}
          </Button>
        ) : null}
        {isSql ? (
          <Button size="sm" variant="ghost" className="rounded-lg text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover" onClick={doFormat} disabled={empty}>
            Format
          </Button>
        ) : null}
        {isSql ? (
          <Button
            size="sm"
            variant="ghost"
            className="rounded-lg text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
            pending={planBusy}
            onClick={() => void explain()}
            disabled={empty}
          >
            Explain
          </Button>
        ) : null}
        <Button size="sm" variant="ghost" className="rounded-lg text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover" onClick={() => setSaveOpen(true)} disabled={empty}>
          Save
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="rounded-lg text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
          pending={savingFile}
          onClick={() => void saveToFile()}
          disabled={empty}
        >
          <Icon name="download" size={12} />
          .sql
        </Button>
        <Popover open={aiOpen} onOpenChange={setAiOpen}>
          <PopoverTrigger asChild>
            <Button size="sm" variant={aiEnabled ? "secondary" : "ghost"} className={cn("rounded-lg liquid-hover", aiEnabled ? "glass-pill text-accent" : "text-muted hover:bg-surface-secondary/70 hover:text-foreground")}>
              <Icon name="braces" size={12} />
              AI
            </Button>
          </PopoverTrigger>
          <PopoverContent className="w-[500px] rounded-xl glass-modal">
              <div className="flex items-center justify-between">
                <PopoverHeading className="text-sm font-semibold text-foreground">AI Database Assistant</PopoverHeading>
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
                  <div className="mt-2.5 flex flex-wrap gap-1.5">
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
                    <div className="mt-3 rounded-lg border border-border/60 bg-surface-secondary/70 p-2.5">
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
          <div className="relative shrink-0 flex flex-col" style={{ height: editorHeight }}>
            {loaded ? (
              <SqlEditor
                value={sql}
                spans={spans}
                onChange={onChange}
                onRun={(picked) => void run(picked.text, false)}
                onRunAll={runAll}
                onTargetChange={setTarget}
                engine={connection.engine}
                schema={schema}
                defaultSchema={defaultSchema}
              />
            ) : null}
          </div>
          <Resizer direction="vertical" onResize={handleEditorResize} />
          <div className="min-h-0 flex-1">
            {plan ? (
              <div className="flex h-full min-h-0 flex-col">
                <div className="flex h-9 shrink-0 items-center gap-2 border-b border-border/40 glass-header px-3 text-xs">
                  <span className="font-semibold text-foreground tracking-tight">Execution plan</span>
                  <span className="ml-auto">
                    <Button variant="ghost" size="icon-sm" aria-label="Close plan" onClick={() => setPlan(null)}><Icon name="x" /></Button>
                  </span>
                </div>
                <div className="grid min-h-0 flex-1 grid-cols-2 gap-0">
                  <ScrollArea className="overflow-x-auto border-r border-border/40 p-3">
                    <pre className="selectable font-mono text-[11px] text-foreground">{plan.plan}</pre>
                  </ScrollArea>
                  <ScrollArea className="selectable p-3 text-xs whitespace-pre-wrap text-muted">{plan.explanation ?? "Enable an AI provider in Settings to get a plain-language explanation of this plan."}</ScrollArea>
                </div>
              </div>
            ) : (
              <ResultsPane outcome={outcome} />
            )}
          </div>
        </div>
        {showHistory ? (
          <div className="relative shrink-0 flex flex-col border-l border-border/40 glass-sidebar select-none" style={{ width: historyWidth }}>
            <Resizer direction="horizontal" onResize={handleHistoryResize} className="absolute -left-1 top-0 bottom-0" />
            <HistoryPanel connectionId={connection.id} refreshKey={historyKey} onPick={(picked) => onChange(picked)} />
          </div>
        ) : null}
      </div>

      <Dialog open={saveOpen} onOpenChange={setSaveOpen}>
        <DialogContent className="sm:max-w-[440px]">
          <DialogHeader>
            <DialogTitle>Save query</DialogTitle>
          </DialogHeader>
          <DialogBody className="flex flex-col gap-4">
            <Field label="Name" value={saveName} onChange={setSaveName} placeholder="Top customers" autoFocus />
            <Field label="Tags" optional value={saveTags} onChange={setSaveTags} placeholder="reports, finance" />
          </DialogBody>
          <DialogFooter>
            <Button variant="tertiary" onClick={() => setSaveOpen(false)}>Cancel</Button>
            <Button onClick={() => void doSave()} disabled={saveName.trim().length === 0}>Save</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={confirm !== null} onOpenChange={(open) => !open && setConfirm(null)}>
        <DialogContent className="sm:max-w-[520px]">
          <DialogHeader>
            <span className="bg-danger-soft text-danger">
              <Icon name="trash" size={18} />
            </span>
            <DialogTitle>Run destructive statements?</DialogTitle>
          </DialogHeader>
          <DialogBody className="space-y-3">
            <Alert variant="danger" className="rounded-xl">
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
                <li key={s} className="selectable rounded-md bg-danger-soft px-2.5 py-1.5 font-mono text-[11px] text-danger">
                  {s}
                </li>
              ))}
            </ul>
          </DialogBody>
          <DialogFooter>
            <Button variant="tertiary" onClick={() => setConfirm(null)}>Cancel</Button>
            <Button variant="danger" onClick={() => void run(confirm?.script ?? sql, true)}>Run anyway</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
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
