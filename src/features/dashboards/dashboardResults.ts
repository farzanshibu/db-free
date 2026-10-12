// SOT: dashboard-result-cache, tile-run-cache, inspector-drafts
import { create } from "zustand";
import type { DashboardBody, QueryOutcome, Widget } from "@/lib/bindings";

// WHAT:  What each open dashboard's tiles last returned, the inspector edits
//        that were never applied, and the unsaved copy of the dashboard itself,
//        held outside the tab's component.
// WHY:   A tab unmounts when another tab is shown. With results in component
//        state, coming back re-ran every query, and an unsaved layout was gone.
//        Here they outlive the tab, so switching away and back costs nothing
//        until a variable, a statement or the refresh timer says the answer is
//        stale. The same goes for a tile edit left half-done when the user clicks
//        another tile.
// HOW:   One record per dashboard id, keyed by widget id. A run carries the
//        `signature` it answered (connection plus the statement as it was sent,
//        variables already filled in): a tile re-runs exactly when that changes.
// WHERE: ./useDashboardRuns.ts (writes runs), ./TileInspector.tsx (reads and writes drafts)

export interface TileRun {
  /// What was asked: the connection and the statement as sent.
  signature: string;
  outcome: QueryOutcome | null;
  error: string | null;
  /// Milliseconds since the epoch.
  at: number;
}

/// The unsaved state of a dashboard tab. `base` is the `updatedAt` of the
/// saved document it was started from: a copy older than the saved document
/// (saved elsewhere since) is ignored.
export interface WorkingCopy {
  base: string;
  name: string;
  connectionId: string | null;
  body: DashboardBody;
}

interface DashboardResultsState {
  runs: Record<string, Record<string, TileRun>>;
  drafts: Record<string, Record<string, Widget>>;
  working: Record<string, WorkingCopy>;
  setWorking: (dashboardId: string, copy: WorkingCopy) => void;
  clearWorking: (dashboardId: string) => void;
  /// The dashboard the user last touched: with a split view two can be open,
  /// and a shortcut belongs to one of them.
  activeId: string | null;
  setActive: (dashboardId: string | null) => void;
  putRun: (dashboardId: string, widgetId: string, run: TileRun) => void;
  /// Forget results for tiles that no longer exist.
  pruneRuns: (dashboardId: string, keep: readonly string[]) => void;
  clearRuns: (dashboardId: string) => void;
  /// null removes the draft.
  setDraft: (dashboardId: string, widgetId: string, draft: Widget | null) => void;
  clearDrafts: (dashboardId: string) => void;
}

export const useDashboardResults = create<DashboardResultsState>((set) => ({
  runs: {},
  drafts: {},
  working: {},
  setWorking: (dashboardId, copy) => { set((s) => ({ working: { ...s.working, [dashboardId]: copy } })); },
  clearWorking: (dashboardId) => {
    set((s) => (s.working[dashboardId] === undefined ? s : { working: Object.fromEntries(Object.entries(s.working).filter(([id]) => id !== dashboardId)) }));
  },
  activeId: null,
  setActive: (dashboardId) => { set((s) => (s.activeId === dashboardId ? s : { activeId: dashboardId })); },
  putRun: (dashboardId, widgetId, run) =>
    set((s) => ({ runs: { ...s.runs, [dashboardId]: { ...s.runs[dashboardId], [widgetId]: run } } })),
  pruneRuns: (dashboardId, keep) =>
    set((s) => {
      const current = s.runs[dashboardId];
      if (current === undefined) return s;
      const kept = Object.fromEntries(Object.entries(current).filter(([id]) => keep.includes(id)));
      return Object.keys(kept).length === Object.keys(current).length ? s : { runs: { ...s.runs, [dashboardId]: kept } };
    }),
  clearRuns: (dashboardId) =>
    set((s) => {
      if (s.runs[dashboardId] === undefined) return s;
      return { runs: Object.fromEntries(Object.entries(s.runs).filter(([id]) => id !== dashboardId)) };
    }),
  setDraft: (dashboardId, widgetId, draft) =>
    set((s) => {
      if (draft === null && s.drafts[dashboardId]?.[widgetId] === undefined) return s;
      const others = Object.entries(s.drafts[dashboardId] ?? {}).filter(([id]) => id !== widgetId);
      const current = Object.fromEntries(draft === null ? others : [...others, [widgetId, draft]]);
      return { drafts: { ...s.drafts, [dashboardId]: current } };
    }),
  clearDrafts: (dashboardId) =>
    set((s) => {
      if (s.drafts[dashboardId] === undefined) return s;
      return { drafts: Object.fromEntries(Object.entries(s.drafts).filter(([id]) => id !== dashboardId)) };
    }),
}));
