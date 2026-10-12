// SOT: change-notes, pending-statement-comments, unmatched-changes
import { create } from "zustand";

// WHAT:  What a user wrote against a pending statement, and which pending changes
//        the last commit left behind because they matched no row.
// WHY:   "-- why I changed this" above the SQL is what makes a copied script
//        reviewable; and an edit that matched nothing (the row was deleted or
//        changed elsewhere) must stay visible, flagged, not vanish with the rest.
// HOW:   Both are keyed by staged-change id and live as long as the change is pending.
// WHERE: src/features/changes/PendingChangesPanel.tsx, src/features/changes/commitPending.ts
interface NotesState {
  notes: Record<string, string>;
  /// Changes the last commit could not land (they matched no row).
  unmatched: Readonly<Record<string, true>>;
  set: (id: string, note: string) => void;
  forget: (ids: readonly string[]) => void;
  markUnmatched: (ids: readonly string[]) => void;
}

const without = <T>(record: Readonly<Record<string, T>>, ids: readonly string[]): Record<string, T> =>
  Object.fromEntries(Object.entries(record).filter(([key]) => !ids.includes(key)));

export const useChangeNotes = create<NotesState>((set) => ({
  notes: {},
  unmatched: {},
  set: (id, note) =>
    set((s) => {
      if (note.trim().length === 0) return { notes: without(s.notes, [id]) };
      return { notes: { ...s.notes, [id]: note } };
    }),
  forget: (ids) => set((s) => ({ notes: without(s.notes, ids), unmatched: without(s.unmatched, ids) })),
  markUnmatched: (ids) => set((s) => ({ unmatched: { ...s.unmatched, ...Object.fromEntries(ids.map((id) => [id, true as const])) } })),
}));

/// A note as SQL comment lines.
export function commentLines(note: string): string {
  return note
    .split("\n")
    .map((line) => `-- ${line}`)
    .join("\n");
}
