// SOT: described-table-loader, edit-table-load
import { useEffect, useState } from "react";
import type { TableRef, TableSpec } from "@/lib/bindings";
import { errorMessage, ipc, normalizeError } from "@/lib/ipc";

interface Loaded {
  attempt: number;
  spec: TableSpec | null;
  error: string | null;
}

export interface DescribedTable {
  spec: TableSpec | null;
  error: string | null;
  retry: () => void;
}

// WHAT:  Reads a live table as a `TableSpec` (columns, keys, defaults, indexes)
//        for the Edit form, with a Retry when the read fails.
// WHY:   The server, not the cached catalog, is the truth an ALTER is diffed
//        against: a stale column list would make the diff wrong.
// HOW:   Results are tagged with their attempt so an answer to an earlier try
//        cannot overwrite a later one.
// WHERE: src-tauri/src/integrations/ddl/ (describe), src/features/schema/forms/EditTableDialog.tsx
export function useDescribedTable(connectionId: string, table: TableRef): DescribedTable {
  const [attempt, setAttempt] = useState(0);
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const { schema, name } = table;

  useEffect(() => {
    const guard = { stale: false };
    void (async () => {
      try {
        const spec = await ipc("describe_table", { connectionId, table: { schema, name } });
        if (!guard.stale) setLoaded({ attempt, spec, error: null });
      } catch (raw) {
        if (!guard.stale) setLoaded({ attempt, spec: null, error: errorMessage(normalizeError(raw)) });
      }
    })();
    return () => {
      guard.stale = true;
    };
  }, [connectionId, schema, name, attempt]);

  const current = loaded !== null && loaded.attempt === attempt ? loaded : null;
  return {
    spec: current?.spec ?? null,
    error: current?.error ?? null,
    retry: () => {
      setAttempt((a) => a + 1);
    },
  };
}
