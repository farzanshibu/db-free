// SOT: commit-pending, commit-flow, commit-result-handling, apply-changes
import type { AppError, CommitReport, StagedChange } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { useChangeNotes } from "./changeNotes";
import { publishCommit, RefreshTablesEvent } from "./commitBus";
import { runAfterCommit } from "./afterCommit";

export type CommitAttempt =
  | { status: "done"; report: CommitReport; applied: number; unmatched: number }
  /// The block wants the user's yes to these statements before running them.
  | { status: "confirm"; statements: string[] }
  | { status: "failed"; error: AppError };

// WHAT:  Commits a connection's pending changes and settles what the result says.
// WHY:   The panel's Commit All and the grid's ⌘↵ both end here, so neither can
//        forget to keep the unmatched edits, tell the open tables, or ask first
//        about a destructive statement; schema statements run their follow-up
//        (`afterCommit`) once they land.
// HOW:   * the commit is one transaction in Rust, rolled back on any failure;
//        * `confirmDestructive` is only ever set after the user said yes to the
//          statements a `confirm` attempt listed;
//        * changes that landed are un-staged; those that matched no row stay,
//          flagged, with a message;
//        * open tables hear about the landed changes (`publishCommit`) and patch
//          just those rows, then everything else reloads (`RefreshTablesEvent`).
// WHERE: src-tauri/src/commands/changes.rs (`commit_changes`), src/features/changes/commitBus.ts
export async function commitPending(connectionId: string, changes: readonly StagedChange[], confirmDestructive: boolean): Promise<CommitAttempt> {
  const workspace = useWorkspace.getState();
  try {
    const report = await ipc("commit_changes", { connectionId, changes: [...changes], ...(confirmDestructive ? { confirmDestructive: true } : {}) });
    const unmatchedIds = report.results.filter((r) => r.matchedNothing).map((r) => r.id);
    const landed = changes.filter((c) => !unmatchedIds.includes(c.id));
    for (const change of landed) workspace.unstageChange(connectionId, change.id);
    useChangeNotes.getState().forget(landed.map((c) => c.id));
    useChangeNotes.getState().markUnmatched(unmatchedIds);
    const handled = new Set<string>();
    publishCommit({ connectionId, changes: landed, report, handled });
    await runAfterCommit(landed.map((c) => c.id));
    window.dispatchEvent(new RefreshTablesEvent(handled));
    if (landed.length > 0) workspace.showInfo(`Committed ${landed.length} change${landed.length === 1 ? "" : "s"}.${report.atomic ? "" : " This engine has no transactions, so each change ran on its own."}`);
    if (unmatchedIds.length > 0) {
      // Direct mode never queued the change, so there is nothing to keep flagged.
      const queued = new Set((useWorkspace.getState().pendingChanges[connectionId] ?? []).map((c) => c.id));
      const kept = unmatchedIds.filter((id) => queued.has(id)).length;
      workspace.showError(
        `${unmatchedIds.length} change${unmatchedIds.length === 1 ? "" : "s"} matched no row — the row was probably changed or deleted elsewhere.${
          kept === 0 ? "" : ` ${kept === 1 ? "It stays" : "They stay"} in the list, flagged.`
        }`,
      );
    }
    return { status: "done", report, applied: landed.length, unmatched: unmatchedIds.length };
  } catch (raw) {
    const error = normalizeError(raw);
    if (error.kind === "destructive_confirmation_required") return { status: "confirm", statements: error.statements };
    workspace.showError(error);
    return { status: "failed", error };
  }
}
