// SOT: fk-store, fk-remap-cache, column-cache, user-defined-foreign-key-state
import { create } from "zustand";
import type { ColumnInfo, FkRemap, TableRef } from "@/lib/bindings";
import { ipc } from "@/lib/ipc";

// WHAT:  The links the user made between columns and tables, per connection, and a
//        cache of table columns used to check a guessed link.
// WHY:   Every open table tab needs the same remaps, and saving one in a tab should
//        light the link up in the others without a reload.
// HOW:   Loaded from the app store on first use of a connection; saved and removed
//        through the fk-remap commands, then merged into the cached list.
// WHERE: src-tauri/src/store/fk_remaps.rs, src/features/grid/TableTab.tsx
interface FkState {
  remaps: Record<string, readonly FkRemap[]>;
  load: (connectionId: string) => Promise<void>;
  save: (remap: Omit<FkRemap, "id" | "createdAt"> & { id?: string }) => Promise<FkRemap>;
  remove: (connectionId: string, id: string) => Promise<void>;
}

const EMPTY: readonly FkRemap[] = [];
const requested = new Set<string>();

export const useFkRemaps = create<FkState>((set, get) => ({
  remaps: {},
  load: async (connectionId) => {
    if (requested.has(connectionId)) return;
    requested.add(connectionId);
    try {
      const remaps = await ipc("list_fk_remaps", { id: connectionId });
      set((s) => ({ remaps: { ...s.remaps, [connectionId]: remaps } }));
    } catch {
      // Without the list, links fall back to constraints and guesses; a retry on next open.
      requested.delete(connectionId);
    }
  },
  save: async (remap) => {
    const saved = await ipc("save_fk_remap", {
      remap: { id: remap.id ?? "", connectionId: remap.connectionId, from: remap.from, fromColumn: remap.fromColumn, to: remap.to, toColumn: remap.toColumn, createdAt: "" },
    });
    set((s) => {
      const others = (s.remaps[remap.connectionId] ?? EMPTY).filter(
        (r) => !(r.id === saved.id || (r.from.name === saved.from.name && (r.from.schema ?? null) === (saved.from.schema ?? null) && r.fromColumn === saved.fromColumn)),
      );
      return { remaps: { ...s.remaps, [remap.connectionId]: [...others, saved] } };
    });
    return saved;
  },
  remove: async (connectionId, id) => {
    await ipc("delete_fk_remap", { id });
    set((s) => ({ remaps: { ...s.remaps, [connectionId]: (s.remaps[connectionId] ?? get().remaps[connectionId] ?? EMPTY).filter((r) => r.id !== id) } }));
  },
}));

export function remapsFor(remaps: Record<string, readonly FkRemap[]>, connectionId: string): readonly FkRemap[] {
  return remaps[connectionId] ?? EMPTY;
}

// ---- column cache -------------------------------------------------------------

const columnCache = new Map<string, Promise<readonly ColumnInfo[] | null>>();

// WHAT:  A table's columns, fetched once per connection and remembered.
// WHY:   Checking a guessed link asks for the columns of every candidate table; a
//        grid refresh must not repeat those round trips.
export function cachedColumns(connectionId: string, table: TableRef): Promise<readonly ColumnInfo[] | null> {
  const key = `${connectionId}:${table.schema ?? ""}.${table.name}`;
  const hit = columnCache.get(key);
  if (hit) return hit;
  const pending = ipc("load_columns", { connectionId, table }).then(
    (columns): readonly ColumnInfo[] | null => columns,
    () => null,
  );
  columnCache.set(key, pending);
  return pending;
}

/// Forgets the cached columns of a connection (after a schema change).
export function forgetColumns(connectionId: string): void {
  for (const key of [...columnCache.keys()]) {
    if (key.startsWith(`${connectionId}:`)) columnCache.delete(key);
  }
}
