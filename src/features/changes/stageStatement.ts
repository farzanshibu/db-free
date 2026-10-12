// SOT: stage-statement, ddl-change-builder, pending-statement-entry
import type { StagedChange, TableRef } from "@/lib/bindings";
import { useWorkspace } from "@/stores/workspace";
import { commitPending } from "./commitPending";

// WHAT:  Puts a finished statement (a column rename, an index drop, a Redis `DEL`) into
//        the connection's Pending Changes, beside the row edits.
// WHY:   Schema changes and key edits deserve the same review step as cell edits: the
//        exact statement is shown, can be commented, runs in order with the rest in one
//        commit, and a destructive one asks before it runs. Features that build their
//        own statements call this instead of running them behind the panel's back.
// HOW:   Review mode stages it and opens the panel. Direct mode runs it at once; when the
//        block asks for the user's yes to a destructive statement, the statement is
//        staged instead and the panel opens, where the confirmation dialog lives.
// WHERE: src/features/changes/PendingChangesPanel.tsx, src-tauri/src/services/changes.rs (`StagedChange::Ddl`)
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

export async function stageStatement(connectionId: string, input: StatementChange): Promise<void> {
  const workspace = useWorkspace.getState();
  const change = statementChange(input);
  if (workspace.settings?.executionMode === "direct") {
    const attempt = await commitPending(connectionId, [change], false);
    // Needs the user's yes: the panel asks, so the statement waits there.
    if (attempt.status !== "confirm") return;
  }
  workspace.stageChange(connectionId, change);
  workspace.setChangesPanelOpen(true);
}
