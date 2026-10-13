// SOT: pending-change-groups, same-row-edit-grouping
import type { CellValue, StagedChange, Value } from "@/lib/bindings";
import { tableKey } from "@/stores/workspace";

// WHAT:  The pending list as cards: edits to the same row are one card with a diff
//        line per column, instead of one card per cell.
// WHY:   Changing four fields of a record is one decision, and reviewing it as four
//        unrelated cards hides that they belong together.
// HOW:   Updates group on table + primary key, in the order of the first edit; every
//        other change is its own card. Each group still lists the real changes, so
//        undoing a card undoes all of them.
// WHERE: src/features/changes/PendingChangesPanel.tsx
export interface ChangeGroup {
  id: string;
  kind: StagedChange["kind"];
  changes: readonly StagedChange[];
}

const keyText = (key: readonly CellValue[]): string => JSON.stringify(key.map((k) => [k.column, k.value]));

export function groupChanges(changes: readonly StagedChange[]): ChangeGroup[] {
  const groups: { id: string; kind: StagedChange["kind"]; changes: StagedChange[] }[] = [];
  const byRow = new Map<string, number>();
  for (const change of changes) {
    if (change.kind === "update") {
      const rowId = `${tableKey(change.table)}\u0000${keyText(change.key)}`;
      const at = byRow.get(rowId);
      if (at !== undefined) {
        groups[at]?.changes.push(change);
        continue;
      }
      byRow.set(rowId, groups.length);
    }
    groups.push({ id: change.id, kind: change.kind, changes: [change] });
  }
  return groups;
}

/// `id=7` / `a=1, b=2` for the cards' titles.
export function describeKey(key: readonly CellValue[], show: (value: Value) => string): string {
  return key.map((k) => `${k.column}=${show(k.value)}`).join(", ") || "row";
}
