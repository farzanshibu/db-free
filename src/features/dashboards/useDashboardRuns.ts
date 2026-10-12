// SOT: dashboard-tile-runs, tile-query-execution, tile-rerun-on-change, dashboard-auto-refresh
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { DashboardVariable, Engine, QueryOutcome, Widget } from "@/lib/bindings";
import { ENGINE_FACTS } from "@/lib/bindings/EngineFacts.gen";
import { errorMessage, ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { useDashboardResults, type TileRun } from "./dashboardResults";
import { dialectOfFamily, substituteVariables, substitutionProblem, type SqlDialect } from "./variables";

// WHAT:  Runs a dashboard's tile queries and keeps their results: once when
//        they are first needed, again exactly when a tile's statement changes
//        (an edit, or a variable it reads), on Refresh, and on the auto-refresh
//        timer.
// WHY:   A change to one variable should re-run the tiles that use it and leave
//        the rest alone; opening a tab again should cost nothing if nothing
//        changed. Both fall out of one rule: a tile re-runs when the statement it
//        would send (variables already filled in) differs from the one its
//        cached result answered.
// HOW:   `planFor` fills the variables in and signs the result with the
//        connection. The effect compares each tile's signature with its cached
//        run; a mismatch runs it. A statement that cannot be built (an undefined
//        variable, one in a comment) is a tile error, not a query. Each tile has
//        a run token so a slow answer to an old statement never overwrites a newer
//        one, and the timer skips a tile still waiting on its last run.
// WHERE: ./dashboardResults.ts (cache), ./variables.ts (substitution), ./DashboardTab.tsx

const MAX_ROWS = 2000;
const NO_RUNS: Readonly<Record<string, TileRun | undefined>> = {};

interface Plan {
  /// The statement to send; null when it cannot be built.
  sql: string | null;
  signature: string;
  problem: string | null;
}

function planFor(widget: Widget, variables: readonly DashboardVariable[], dialect: SqlDialect, connectionId: string): Plan | null {
  if (widget.sql.trim().length === 0) return null;
  const filled = substituteVariables(widget.sql, variables, dialect);
  const problem = substitutionProblem(filled);
  return { sql: problem === null ? filled.sql : null, signature: JSON.stringify([connectionId, problem === null ? filled.sql : `!${problem}`]), problem };
}

export interface DashboardRuns {
  runs: Readonly<Record<string, TileRun | undefined>>;
  running: ReadonlySet<string>;
  /// Runs every tile again now.
  refresh: () => void;
  /// Runs one statement the way a tile would (variables filled in) without
  /// touching the tile's cached result: the inspector's preview.
  preview: (sql: string) => Promise<{ outcome: QueryOutcome | null; error: string | null }>;
}

interface Options {
  dashboardId: string;
  connectionId: string | null;
  engine: Engine;
  widgets: readonly Widget[];
  variables: readonly DashboardVariable[];
  refreshSeconds: number;
}

export function useDashboardRuns({ dashboardId, connectionId, engine, widgets, variables, refreshSeconds }: Options): DashboardRuns {
  const sessions = useWorkspace((s) => s.sessions);
  const connect = useWorkspace((s) => s.connect);
  const runs = useDashboardResults((s) => s.runs[dashboardId]) ?? NO_RUNS;
  const putRun = useDashboardResults((s) => s.putRun);
  const pruneRuns = useDashboardResults((s) => s.pruneRuns);
  const [running, setRunning] = useState<ReadonlySet<string>>(new Set());
  const tokens = useRef(new Map<string, number>());
  const inflight = useRef(new Map<string, string>());
  const dialect = useMemo(() => dialectOfFamily(ENGINE_FACTS[engine].family), [engine]);

  const plans = useMemo(() => {
    const out = new Map<string, Plan>();
    if (connectionId === null) return out;
    for (const w of widgets) {
      const plan = planFor(w, variables, dialect, connectionId);
      if (plan !== null) out.set(w.id, plan);
    }
    return out;
  }, [widgets, variables, dialect, connectionId]);

  // Every tile asks for the session at once; they share one connect.
  const connecting = useRef<Promise<boolean> | null>(null);
  const ensureSession = useCallback(
    (id: string): Promise<boolean> => {
      if (sessions.includes(id)) return Promise.resolve(true);
      connecting.current ??= connect(id).finally(() => { connecting.current = null; });
      return connecting.current;
    },
    [sessions, connect],
  );

  const execute = useCallback(
    async (widgetId: string, plan: Plan) => {
      if (connectionId === null) return;
      const token = (tokens.current.get(widgetId) ?? 0) + 1;
      tokens.current.set(widgetId, token);
      inflight.current.set(widgetId, plan.signature);
      const finish = (run: Omit<TileRun, "signature" | "at">) => {
        if (tokens.current.get(widgetId) !== token) return;
        putRun(dashboardId, widgetId, { signature: plan.signature, at: Date.now(), ...run });
      };
      if (plan.sql === null) {
        finish({ outcome: null, error: plan.problem });
        inflight.current.delete(widgetId);
        return;
      }
      setRunning((r) => new Set(r).add(widgetId));
      try {
        if (!(await ensureSession(connectionId))) {
          finish({ outcome: null, error: "Not connected." });
        } else {
          finish({ outcome: await ipc("execute_query", { connectionId, sql: plan.sql, confirmDestructive: false, maxRows: MAX_ROWS, schema: null }), error: null });
        }
      } catch (raw) {
        finish({ outcome: null, error: errorMessage(normalizeError(raw)) });
      } finally {
        if (tokens.current.get(widgetId) === token) {
          inflight.current.delete(widgetId);
          setRunning((r) => {
            const next = new Set(r);
            next.delete(widgetId);
            return next;
          });
        }
      }
    },
    [connectionId, dashboardId, ensureSession, putRun],
  );

  // Run what is new or changed.
  useEffect(() => {
    for (const [id, plan] of plans) {
      if (runs[id]?.signature === plan.signature || inflight.current.get(id) === plan.signature) continue;
      void execute(id, plan);
    }
  }, [plans, runs, execute]);

  // Forget tiles that were removed.
  useEffect(() => {
    pruneRuns(dashboardId, widgets.map((w) => w.id));
  }, [dashboardId, widgets, pruneRuns]);

  const refresh = useCallback(() => {
    for (const [id, plan] of plans) void execute(id, plan);
  }, [plans, execute]);

  // The timer reads the latest plans through a ref, so editing a tile does not
  // restart its countdown, and leaves tiles that are still waiting alone.
  const latest = useRef({ plans, execute });
  useEffect(() => {
    latest.current = { plans, execute };
  }, [plans, execute]);
  useEffect(() => {
    if (refreshSeconds <= 0 || connectionId === null) return;
    const id = window.setInterval(() => {
      const { plans: current, execute: run } = latest.current;
      for (const [widgetId, plan] of current) if (!inflight.current.has(widgetId)) void run(widgetId, plan);
    }, refreshSeconds * 1000);
    return () => { window.clearInterval(id); };
  }, [refreshSeconds, connectionId]);

  const preview = useCallback(
    async (sql: string) => {
      if (connectionId === null) return { outcome: null, error: "Pick a connection first." };
      const filled = substituteVariables(sql, variables, dialect);
      const problem = substitutionProblem(filled);
      if (problem !== null) return { outcome: null, error: problem };
      try {
        if (!(await ensureSession(connectionId))) return { outcome: null, error: "Not connected." };
        return { outcome: await ipc("execute_query", { connectionId, sql: filled.sql, confirmDestructive: false, maxRows: MAX_ROWS, schema: null }), error: null };
      } catch (raw) {
        return { outcome: null, error: errorMessage(normalizeError(raw)) };
      }
    },
    [connectionId, variables, dialect, ensureSession],
  );

  return { runs, running, refresh, preview };
}
