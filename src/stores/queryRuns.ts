// SOT: query-run-store, per-tab-results, batch-run-state, run-without-limit, results-collapse-state
import { create } from "zustand";
import type { AppError, BatchEvent, BatchStatementPlan, BatchStatementResult, BatchStatus, BatchSummary, QueryOutcome, StatementIntent } from "@/lib/bindings";
import { executeBatch, ipc, normalizeError } from "@/lib/ipc";
import { parseJson } from "@/lib/json";
import { sessionKeyFor } from "./sessions";
import { useWorkspace } from "./workspace";

// WHAT:  Every query tab's last run: its statements with their status, time and
//        rows, and whether one is running now.
// WHY:   A tab's pane unmounts when another tab is shown. Results held in the
//        pane vanished with it, and a run in flight lost its answer. Held here,
//        they outlive the pane: switching away and back shows the same grid, and
//        a run that finishes while you are elsewhere is waiting when you return.
// HOW:   A run is started through `start`: one `execute_batch` call whose
//        progress events (plan, started, finished) update the tab's entry as they
//        arrive. The entry is dropped when its tab closes.
// WHERE: src/features/editor/QueryPane.tsx, src/features/editor/ResultsPane.tsx,
//        src-tauri/src/services/query.rs (execute_batch)

export type StatementState = "pending" | "running" | BatchStatus;

export interface StatementRun {
  index: number;
  sql: string;
  /// UTF-16 offsets of the statement inside `TabRun.script`.
  start: number;
  end: number;
  intent: StatementIntent;
  state: StatementState;
  elapsedMs: number | null;
  outcome: QueryOutcome | null;
  error: AppError | null;
}

/// A stretch of the editor's text that went into the script, in script order.
export interface RunPiece {
  from: number;
  to: number;
}

/// What a run needs beyond the script, kept so a statement can be run again.
export interface RunParams {
  connectionId: string;
  script: string;
  pieces: readonly RunPiece[];
  schema: string | null;
  /// null = no cap.
  maxRows: number | null;
  stopOnError: boolean;
  /// Cut the script at its statement boundaries (SQL); false sends the text whole.
  split: boolean;
}

export interface TabRun {
  /// Counts the tab's runs; keys the result views so a new run starts them clean.
  seq: number;
  runId: string | null;
  running: boolean;
  stopping: boolean;
  /// A single statement being run again without the row cap, or null.
  rerunning: number | null;
  script: string;
  pieces: readonly RunPiece[];
  params: RunParams | null;
  statements: StatementRun[];
  summary: BatchSummary | null;
  /// The run itself was refused or broke (not connected, read-only…): no statement ran.
  fatal: AppError | null;
}

export type StartResult =
  | { kind: "done" }
  /// A destructive statement needs the user's yes; nothing ran.
  | { kind: "confirm"; statements: string[] }
  | { kind: "failed"; error: AppError };

interface QueryRunsState {
  runs: Record<string, TabRun | undefined>;
  /// Tabs whose results area is folded away.
  collapsed: Record<string, boolean | undefined>;
  /// The saved query each tab is tied to (opened from it, or saved as it): tab id -> saved query id.
  links: Record<string, string | undefined>;
  /// Ties a tab to a saved query, or cuts the tie with null.
  link: (tabId: string, savedQueryId: string | null) => void;
  start: (tabId: string, params: RunParams, confirmDestructive: boolean) => Promise<StartResult>;
  stop: (tabId: string) => Promise<void>;
  /// Runs one statement of the last run again, with a different row cap, and replaces its result.
  rerunStatement: (tabId: string, index: number, maxRows: number | null) => Promise<void>;
  setCollapsed: (tabId: string, collapsed: boolean) => void;
  forget: (tabId: string) => void;
}

const LINKS_KEY = "db-free:query-tab-links";

function readLinks(): Record<string, string | undefined> {
  try {
    const raw = localStorage.getItem(LINKS_KEY);
    const parsed = raw === null ? undefined : parseJson(raw);
    if (parsed === undefined || parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const out: Record<string, string | undefined> = {};
    for (const [tab, id] of Object.entries(parsed)) if (typeof id === "string") out[tab] = id;
    return out;
  } catch {
    return {};
  }
}

function writeLinks(links: Readonly<Record<string, string | undefined>>): void {
  try {
    const live = Object.fromEntries(Object.entries(links).filter(([, id]) => id !== undefined));
    localStorage.setItem(LINKS_KEY, JSON.stringify(live));
  } catch {
    // storage unavailable: the link just does not survive a restart
  }
}

const BLANK: TabRun = {
  seq: 0,
  runId: null,
  running: false,
  stopping: false,
  rerunning: null,
  script: "",
  pieces: [],
  params: null,
  statements: [],
  summary: null,
  fatal: null,
};

function planned(plan: BatchStatementPlan): StatementRun {
  return { index: plan.index, sql: plan.sql, start: plan.start, end: plan.end, intent: plan.intent, state: "pending", elapsedMs: null, outcome: null, error: null };
}

function finishedState(result: BatchStatementResult): Partial<StatementRun> {
  return { state: result.status, elapsedMs: result.elapsedMs, outcome: result.outcome, error: result.error };
}

export const useQueryRuns = create<QueryRunsState>((set, get) => {
  // Applies `change` to the tab's run, but only while `runId` is still the run on screen.
  const patch = (tabId: string, runId: string, change: (run: TabRun) => Partial<TabRun>) =>
    set((s) => {
      const run = s.runs[tabId];
      if (run?.runId !== runId) return s;
      return { runs: { ...s.runs, [tabId]: { ...run, ...change(run) } } };
    });

  const patchStatement = (tabId: string, runId: string, index: number, change: Partial<StatementRun>) =>
    patch(tabId, runId, (run) => ({ statements: run.statements.map((s) => (s.index === index ? { ...s, ...change } : s)) }));

  return {
    runs: {},
    collapsed: {},
    links: readLinks(),

    link: (tabId, savedQueryId) =>
      set((s) => {
        const links = { ...s.links, [tabId]: savedQueryId ?? undefined };
        writeLinks(links);
        return { links };
      }),

    start: async (tabId, params, confirmDestructive) => {
      const previous = get().runs[tabId];
      if (previous?.running === true) return { kind: "done" };
      const runId = crypto.randomUUID();
      // The previous results stay on screen until the plan arrives, so a refused
      // run (read-only, needs a confirmation) leaves them untouched.
      set((s) => ({ runs: { ...s.runs, [tabId]: { ...(previous ?? BLANK), runId, running: true, stopping: false, rerunning: null } } }));
      const apply = (event: BatchEvent) => {
        switch (event.type) {
          case "planned":
            patch(tabId, runId, (run) => ({
              seq: run.seq + 1,
              script: params.script,
              pieces: params.pieces,
              params,
              statements: event.statements.map(planned),
              summary: null,
              fatal: null,
            }));
            break;
          case "started":
            patchStatement(tabId, runId, event.index, { state: "running" });
            break;
          case "finished":
            patchStatement(tabId, runId, event.result.index, finishedState(event.result));
            break;
          case "done":
            patch(tabId, runId, () => ({ summary: event.summary }));
            break;
        }
      };
      try {
        const summary = await executeBatch(
          {
            connectionId: params.connectionId,
            sql: params.script,
            confirmDestructive,
            maxRows: params.maxRows,
            schema: params.schema,
            runId,
            stopOnError: params.stopOnError,
            split: params.split,
            sessionKey: sessionKeyFor(tabId),
          },
          apply,
        );
        patch(tabId, runId, () => ({ summary, running: false, stopping: false, runId: null }));
        return { kind: "done" };
      } catch (raw) {
        const error = normalizeError(raw);
        const planArrived = get().runs[tabId]?.runId === runId && (get().runs[tabId]?.seq ?? 0) > (previous?.seq ?? 0);
        if (error.kind === "destructive_confirmation_required" && !planArrived) {
          set((s) => ({ runs: { ...s.runs, [tabId]: previous === undefined ? undefined : { ...previous, running: false, stopping: false } } }));
          return { kind: "confirm", statements: error.statements };
        }
        // Whatever never reported back did not finish: say so rather than leave a spinner.
        patch(tabId, runId, (run) => ({
          running: false,
          stopping: false,
          runId: null,
          ...(planArrived
            ? { statements: run.statements.map((s): StatementRun => (s.state === "pending" || s.state === "running" ? { ...s, state: "skipped" } : s)), fatal: error }
            : { seq: run.seq + 1, script: params.script, pieces: params.pieces, params, statements: [], summary: null, fatal: error }),
        }));
        return { kind: "failed", error };
      }
    },

    stop: async (tabId) => {
      const run = get().runs[tabId];
      if (run?.running !== true || run.runId === null || run.stopping) return;
      set((s) => ({ runs: { ...s.runs, [tabId]: { ...run, stopping: true } } }));
      try {
        await ipc("cancel_query", { runId: run.runId });
      } catch (raw) {
        set((s) => ({ runs: { ...s.runs, [tabId]: { ...run, stopping: false } } }));
        useWorkspace.getState().showError(normalizeError(raw));
      }
    },

    rerunStatement: async (tabId, index, maxRows) => {
      const run = get().runs[tabId];
      const statement = run?.statements.find((s) => s.index === index);
      if (run === undefined || statement === undefined || run.params === null || run.running || run.rerunning !== null) return;
      const seq = run.seq;
      const params = run.params;
      const mark = (rerunning: number | null) =>
        set((s) => {
          const current = s.runs[tabId];
          return current?.seq === seq ? { runs: { ...s.runs, [tabId]: { ...current, rerunning } } } : s;
        });
      mark(index);
      try {
        await executeBatch(
          {
            connectionId: params.connectionId,
            sql: statement.sql,
            // Only a read has rows to un-cap, and a read needs no confirmation.
            confirmDestructive: false,
            maxRows,
            schema: params.schema,
            runId: crypto.randomUUID(),
            stopOnError: true,
            split: false,
            sessionKey: sessionKeyFor(tabId),
          },
          (event) => {
            if (event.type !== "finished") return;
            set((s) => {
              const current = s.runs[tabId];
              if (current?.seq !== seq) return s;
              const statements = current.statements.map((st) => (st.index === index ? { ...st, ...finishedState(event.result) } : st));
              return { runs: { ...s.runs, [tabId]: { ...current, statements } } };
            });
          },
        );
      } catch (raw) {
        useWorkspace.getState().showError(normalizeError(raw));
      } finally {
        mark(null);
      }
    },

    setCollapsed: (tabId, collapsed) => set((s) => ({ collapsed: { ...s.collapsed, [tabId]: collapsed } })),

    forget: (tabId) =>
      set((s) => {
        const links = { ...s.links, [tabId]: undefined };
        writeLinks(links);
        return { runs: { ...s.runs, [tabId]: undefined }, collapsed: { ...s.collapsed, [tabId]: undefined }, links };
      }),
  };
});

// WHAT:  Drops a tab's results when the tab closes.
// HOW:   Watches the workspace's tab list rather than hooking `closeTab`, so every
//        way a tab can go away is covered by the one subscription.
useWorkspace.subscribe((state, previous) => {
  if (state.tabs === previous.tabs) return;
  const open = new Set(state.tabs.map((tab) => tab.id));
  for (const tab of previous.tabs) {
    if (tab.kind === "query" && !open.has(tab.id)) useQueryRuns.getState().forget(tab.id);
  }
});
