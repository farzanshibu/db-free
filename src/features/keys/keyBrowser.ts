// SOT: key-browser-store, redis-key-scan-state, redis-key-search, key-value-previews
import { create } from "zustand";
import type { RedisKeyEntry, RedisKeyType, SchemaCatalog } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { readStored, writeStored } from "@/lib/storage";

// WHAT:  The sidebar's view of a Redis keyspace: the keys scanned so far, where
//        the cursor stands, whether the scan finished, and the value-preview
//        switch, per connection.
// WHY:   A keyspace of millions cannot be a catalog. The tree fills in a page at a
//        time, in the background, and keeps filling until it has everything or
//        reaches the auto-scan limit — beyond that the user asks for more.
// HOW:   `load` starts a scan; `more` continues past the limit; `refresh` starts
//        over. Each scan carries a generation number: a refresh, a disconnect or a
//        reconnect that overtakes it makes the older loop stop quietly.
// WHERE: src-tauri/src/integrations/redis.rs (scan_keys), src/features/shell/KeyTree.tsx
export const PAGE_KEYS = 500;
/// Keys one `load` / `more` gathers before it waits for the user.
export const AUTO_SCAN_KEYS = 10_000;
const PREVIEWS_STORAGE = "db-free.redis.valuePreviews";

export interface KeyBrowser {
  keys: readonly RedisKeyEntry[];
  cursor: string;
  /// The scan visited the whole keyspace.
  done: boolean;
  /// A scan loop is running.
  scanning: boolean;
  /// `DBSIZE`, for "N of ~M".
  total: number | null;
  error: string | null;
  previews: boolean;
  generation: number;
  /// The catalog the scan was started for: a new one (refresh, reconnect) means
  /// the keyspace may have changed, so the scan starts over.
  catalog: SchemaCatalog | null;
}

interface KeyBrowserState {
  browsers: Readonly<Record<string, KeyBrowser>>;
  load: (connectionId: string, catalog: SchemaCatalog | null) => Promise<void>;
  more: (connectionId: string) => Promise<void>;
  refresh: (connectionId: string) => Promise<void>;
  setPreviews: (connectionId: string, on: boolean) => Promise<void>;
  /// A key was created or renamed: show it without a rescan.
  add: (connectionId: string, name: string, kind: RedisKeyType) => void;
  remove: (connectionId: string, name: string) => void;
  forget: (connectionId: string) => void;
}

export function previewsPreferred(): boolean {
  return readStored(PREVIEWS_STORAGE) === "1";
}

function fresh(previews: boolean, generation: number, catalog: SchemaCatalog | null): KeyBrowser {
  return { keys: [], cursor: "0", done: false, scanning: false, total: null, error: null, previews, generation, catalog };
}

export const useKeyBrowser = create<KeyBrowserState>((set, get) => {
  const patch = (connectionId: string, change: (current: KeyBrowser) => KeyBrowser) =>
    set((s) => {
      const current = s.browsers[connectionId];
      return current === undefined ? s : { browsers: { ...s.browsers, [connectionId]: change(current) } };
    });

  // WHAT:  Pages from the stored cursor until `budget` more keys are in, the
  //        scan ends, a newer scan replaces this one, or a page fails.
  const run = async (connectionId: string, generation: number, budget: number): Promise<void> => {
    let gathered = 0;
    patch(connectionId, (b) => (b.generation === generation ? { ...b, scanning: true, error: null } : b));
    try {
      while (gathered < budget) {
        const snapshot = get().browsers[connectionId];
        if (snapshot?.generation !== generation || snapshot.done) break;
        const page = await ipc("scan_keys", {
          connectionId,
          scan: { search: null, cursor: snapshot.cursor, count: PAGE_KEYS, limit: PAGE_KEYS, previews: snapshot.previews },
        });
        gathered += page.keys.length;
        const outcome = { replaced: false };
        patch(connectionId, (b) => {
          if (b.generation !== generation) {
            outcome.replaced = true;
            return b;
          }
          return { ...b, keys: appendKeys(b.keys, page.keys), cursor: page.cursor, done: page.done, total: page.total };
        });
        if (outcome.replaced || page.done) break;
      }
    } catch (raw) {
      patch(connectionId, (b) => (b.generation === generation ? { ...b, error: normalizeError(raw).message } : b));
    } finally {
      patch(connectionId, (b) => (b.generation === generation ? { ...b, scanning: false } : b));
    }
  };

  const start = (connectionId: string, previews: boolean, catalog: SchemaCatalog | null): Promise<void> => {
    const generation = (get().browsers[connectionId]?.generation ?? 0) + 1;
    set((s) => ({ browsers: { ...s.browsers, [connectionId]: fresh(previews, generation, catalog) } }));
    return run(connectionId, generation, AUTO_SCAN_KEYS);
  };

  return {
    browsers: {},
    load: async (connectionId, catalog) => {
      const current = get().browsers[connectionId];
      if (current?.catalog === catalog) return;
      await start(connectionId, current?.previews ?? previewsPreferred(), catalog);
    },
    more: async (connectionId) => {
      const current = get().browsers[connectionId];
      if (current === undefined || current.done || current.scanning) return;
      await run(connectionId, current.generation, AUTO_SCAN_KEYS);
    },
    refresh: async (connectionId) => {
      const current = get().browsers[connectionId];
      await start(connectionId, current?.previews ?? previewsPreferred(), current?.catalog ?? null);
    },
    setPreviews: async (connectionId, on) => {
      writeStored(PREVIEWS_STORAGE, on ? "1" : "0");
      // Previews ride along with the scan, so turning them on rescans.
      await start(connectionId, on, get().browsers[connectionId]?.catalog ?? null);
    },
    add: (connectionId, name, kind) =>
      patch(connectionId, (b) => (b.keys.some((k) => k.name === name) ? b : { ...b, keys: [...b.keys, { name, kind, preview: null }], total: b.total === null ? null : b.total + 1 })),
    remove: (connectionId, name) =>
      patch(connectionId, (b) => (b.keys.some((k) => k.name === name) ? { ...b, keys: b.keys.filter((k) => k.name !== name), total: b.total === null ? null : Math.max(0, b.total - 1) } : b)),
    forget: (connectionId) => set((s) => ({ browsers: Object.fromEntries(Object.entries(s.browsers).filter(([id]) => id !== connectionId)) })),
  };
});

// WHAT:  New keys onto the list, skipping any already there (SCAN may repeat one).
function appendKeys(have: readonly RedisKeyEntry[], page: readonly RedisKeyEntry[]): readonly RedisKeyEntry[] {
  if (page.length === 0) return have;
  const seen = new Set(have.map((k) => k.name));
  return [...have, ...page.filter((k) => !seen.has(k.name))];
}
