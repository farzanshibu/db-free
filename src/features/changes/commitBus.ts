// SOT: commit-bus, committed-changes-event, refresh-tables-event
import type { CommitReport, StagedChange } from "@/lib/bindings";

// WHAT:  A committed batch, announced so each open table can bring itself up to date.
// WHY:   Commit All lives in a side panel; the tables it changed are tabs elsewhere.
//        A full reload of every one throws away scroll position and re-reads rows
//        nobody touched. A listener that can patch just its changed rows says so by
//        adding its table to `handled`, and the generic refresh that follows skips it.
// HOW:   `publishCommit` runs the listeners synchronously; `RefreshTablesEvent` is the
//        window event the rest of the app already listens for, carrying the tables
//        that need no reload.
// WHERE: src/features/changes/PendingChangesPanel.tsx, src/features/grid/TableTab.tsx
export interface CommitEvent {
  connectionId: string;
  /// The changes that landed (those that matched no row are not here).
  changes: readonly StagedChange[];
  report: CommitReport;
  /// Table keys (`schema.name`) a listener brought up to date by itself.
  handled: Set<string>;
}

type Listener = (event: CommitEvent) => void;

const listeners = new Set<Listener>();

export function onCommit(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function publishCommit(event: CommitEvent): void {
  for (const listener of [...listeners]) listener(event);
}

/// The window event other tabs reload on; `skip` lists the tables already patched in place.
export class RefreshTablesEvent extends Event {
  readonly skip: ReadonlySet<string>;

  constructor(skip: ReadonlySet<string> = new Set()) {
    super("db-free:refresh-tables");
    this.skip = skip;
  }
}
