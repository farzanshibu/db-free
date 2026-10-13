// SOT: stage-statement, ddl-change-builder, pending-statement-entry
import type { StagedChange, TableRef } from "@/lib/bindings";
import { errorMessage } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { forgetAfterCommit, registerAfterCommit } from "./afterCommit";
import { commitPending } from "./commitPending";

// WHAT:  Puts finished statements (a column rename, a table created by a form, a Redis `DEL`)
//        into the connection's Pending Changes, beside the row edits.
// WHY:   Schema changes and key edits deserve the same review step as cell edits: the
//        exact statement is shown, can be commented, runs in order with the rest in one
//        commit, and a destructive one asks before it runs. Features that build their
//        own statements call this instead of running them behind the panel's back.
// HOW:   Review mode stages them and opens the panel. Direct mode commits them at once
//        through `commitPending`, the same path a grid edit takes; when the block asks
//        for the user's yes to a destructive statement, they are staged instead and the
//        panel opens, where the confirmation dialog lives. `afterCommit` runs once the
//        statements have landed, wherever that happens.
// WHERE: src/features/changes/PendingChangesPanel.tsx, src/features/changes/afterCommit.ts, src-tauri/src/services/changes.rs (`StagedChange::Ddl`)
export interface StatementChange {
  /// The statement exactly as it will run; a trailing `;` is dropped.
  statement: string;
  /// One line saying what it does, shown on the card ("Rename column email to mail").
  description: string;
  /// Drops, truncates and flushes: the commit asks first.
  destructive: boolean;
  /// What it is about; omit for a statement that belongs to the schema or the server.
  table?: TableRef | undefined;
}

export type StageOutcome =
  /// Queued in Pending Changes (the panel is open).
  | { status: "staged" }
  /// Committed now (direct mode); `afterCommit` has run.
  | { status: "done" }
  /// Direct mode could not commit them; the error is already shown as a toast.
  | { status: "failed"; message: string };

let counter = 0;

export function statementChange(input: StatementChange): StagedChange {
  counter += 1;
  return {
    kind: "ddl",
    id: `stmt-${Date.now()}-${counter}`,
    table: input.table ?? { schema: null, name: "" },
    statement: input.statement,
    description: input.description,
    destructive: input.destructive,
  };
}

// WHAT:  Stages (or, in direct mode, commits) a list of statements in order.
// WHY:   A form's preview is several statements that must run together, in order,
//        in one commit: one `commitPending` call, not one per statement.
// HOW:   `afterCommit` is registered under every change id first, so it runs whichever
//        path lands them. A direct commit that is refused is forgotten again.
export async function stageStatements(connectionId: string, inputs: readonly StatementChange[], afterCommit?: () => Promise<void>): Promise<StageOutcome> {
  const workspace = useWorkspace.getState();
  const changes = inputs.map(statementChange);
  const ids = changes.map((c) => c.id);
  if (afterCommit !== undefined) registerAfterCommit(ids, afterCommit);
  if (workspace.settings?.executionMode === "direct") {
    const attempt = await commitPending(connectionId, changes, false);
    if (attempt.status === "done") return { status: "done" };
    forgetAfterCommit(ids);
    if (attempt.status === "failed") return { status: "failed", message: errorMessage(attempt.error) };
    // Needs the user's yes: the panel asks, so the statements wait there.
  }
  for (const change of changes) workspace.stageChange(connectionId, change);
  workspace.setChangesPanelOpen(true);
  return { status: "staged" };
}
