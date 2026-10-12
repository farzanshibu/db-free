// SOT: workflow-run-store, workflow-run-events-ui, workflow-node-status, workflow-run-control
import { create } from "zustand";
import type { NodePreview, NodeUpdate, RunTrigger, WorkflowEvent, WorkflowRunReport } from "@/lib/bindings";
import { ipc, onWorkflowEvent } from "@/lib/ipc";

// WHAT:  What the canvas knows about workflow runs: per workflow, the live status of
//        every node, its last output, and how the last run ended.
// WHY:   A run lives in the core, so it can start without this window (a schedule)
//        and outlive a tab. One store, fed by the core's `workflow:event`, means a
//        tab opened mid-run shows it, and closing a tab never loses a result.
// HOW:   Events are applied as they arrive. Starting a run here marks it running
//        at once (so Stop is there immediately); the core's RunStarted then resets
//        the node statuses for the new run. Outputs are kept across runs until a
//        node produces new ones.
// WHERE: src-tauri/src/commands/workflows.rs (emits), src/features/workflows/WorkflowTab.tsx

export interface RunView {
  runId: string | null;
  running: boolean;
  trigger: RunTrigger | null;
  /// Latest status of each node in the current (or last) run.
  nodes: Readonly<Record<string, NodeUpdate>>;
  /// The last output a node produced, kept until it produces another.
  outputs: Readonly<Record<string, readonly NodePreview[]>>;
  report: WorkflowRunReport | null;
  /// Bumped on every node update; restarts the edges' travelling spark.
  tick: number;
}

export const EMPTY_VIEW: RunView = { runId: null, running: false, trigger: null, nodes: {}, outputs: {}, report: null, tick: 0 };

interface RunStore {
  views: Readonly<Record<string, RunView>>;
  apply: (event: WorkflowEvent) => void;
  begin: (workflowId: string, runId: string) => void;
  settle: (workflowId: string, runId: string, report: WorkflowRunReport | null) => void;
  forget: (workflowId: string) => void;
}

function viewOf(views: Readonly<Record<string, RunView>>, workflowId: string): RunView {
  return views[workflowId] ?? EMPTY_VIEW;
}

export const useRunStore = create<RunStore>((set) => ({
  views: {},
  apply: (event) => {
    set((state) => {
      switch (event.type) {
        case "run_started": {
          const view = viewOf(state.views, event.workflowId);
          return { views: { ...state.views, [event.workflowId]: { ...view, runId: event.runId, running: true, trigger: event.trigger, nodes: {}, report: null, tick: view.tick + 1 } } };
        }
        case "node": {
          const update = event.update;
          const view = viewOf(state.views, update.workflowId);
          if (view.runId !== null && view.runId !== update.runId) return state;
          const outputs = update.state === "success" && update.outputs.length > 0 ? { ...view.outputs, [update.nodeId]: update.outputs } : view.outputs;
          return {
            views: { ...state.views, [update.workflowId]: { ...view, runId: update.runId, running: view.running || update.state === "running", nodes: { ...view.nodes, [update.nodeId]: update }, outputs, tick: view.tick + 1 } },
          };
        }
        case "run_finished": {
          const view = viewOf(state.views, event.workflowId);
          if (view.runId !== null && view.runId !== event.runId) return state;
          return { views: { ...state.views, [event.workflowId]: { ...view, runId: event.runId, running: false, report: event.report } } };
        }
      }
    });
  },
  begin: (workflowId, runId) => {
    set((state) => {
      const view = viewOf(state.views, workflowId);
      return { views: { ...state.views, [workflowId]: { ...view, runId, running: true, trigger: "manual", nodes: {}, report: null, tick: view.tick + 1 } } };
    });
  },
  settle: (workflowId, runId, report) => {
    set((state) => {
      const view = viewOf(state.views, workflowId);
      if (view.runId !== runId) return state;
      return { views: { ...state.views, [workflowId]: { ...view, running: false, report: report ?? view.report } } };
    });
  },
  forget: (workflowId) => {
    set((state) => {
      const rest = Object.fromEntries(Object.entries(state.views).filter(([id]) => id !== workflowId));
      return { views: rest };
    });
  },
}));

let listening = false;

// WHAT:  Starts feeding the store from the core's events (once per app run).
// WHY:   The Tauri event API is absent in the browser preview; a missing listener
//        only means no live status there.
export function ensureRunListener(): void {
  if (listening) return;
  listening = true;
  void onWorkflowEvent((event) => {
    useRunStore.getState().apply(event);
  }).catch(() => {
    listening = false;
  });
}

function newRunId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `run-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

export interface StartOptions {
  /// Run just this node and what it needs; absent runs the whole workflow.
  nodeId?: string;
  /// Pause between levels so the data can be watched moving.
  stepDelayMs?: number;
}

// WHAT:  Runs a saved workflow and resolves with the report when it ends.
// WHY:   The workflow must be saved first: the core runs what is stored.
export async function startRun(workflowId: string, options: StartOptions = {}): Promise<WorkflowRunReport> {
  ensureRunListener();
  const runId = newRunId();
  const store = useRunStore.getState();
  store.begin(workflowId, runId);
  try {
    const report = await ipc("run_workflow", {
      id: workflowId,
      runId,
      ...(options.nodeId !== undefined ? { nodeId: options.nodeId } : {}),
      ...(options.stepDelayMs !== undefined ? { stepDelayMs: options.stepDelayMs } : {}),
    });
    store.settle(workflowId, runId, report);
    return report;
  } catch (raw) {
    store.settle(workflowId, runId, null);
    throw raw;
  }
}

// WHAT:  Stops the workflow's current run, if it has one.
export async function stopRun(workflowId: string): Promise<void> {
  const runId = useRunStore.getState().views[workflowId]?.runId;
  if (runId === null || runId === undefined) return;
  await ipc("cancel_workflow", { runId });
}
