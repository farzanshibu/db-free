// SOT: use-diagram-customizations, er-diagram-layout-memory, table-colour-store, bend-point-store
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ConnectionColor, DiagramCustom, DiagramCustomKind, DiagramCustomization } from "@/lib/bindings";
import { ipc } from "@/lib/ipc";
import type { Point } from "./metrics";

// WHAT:  What the user changed on an ER diagram that the database cannot say for
//        them — where each card sits, its colour, the bend points of each line, the
//        pan and zoom — loaded for one (connection, schema) and saved as it changes.
// WHY:   The ER diagram is derived from the schema, so none of this lives in the
//        diagram itself; without it the diagram re-lays itself out on every open.
// HOW:   Changes apply to local state immediately and are written to the store in
//        one debounced batch (and again when the view closes), so a drag does not
//        write once per frame. A failed save is reported once through `onError`.
// WHERE: src-tauri/src/store/diagram_customizations.rs, src/features/diagrams/ErdTab.tsx

export interface Viewport {
  x: number;
  y: number;
  zoom: number;
}

export interface DiagramCustomizations {
  /// False until the stored customizations have been read (or failed to be).
  ready: boolean;
  positions: ReadonlyMap<string, Point>;
  colors: ReadonlyMap<string, ConnectionColor>;
  bends: ReadonlyMap<string, readonly Point[]>;
  viewport: Viewport | null;
  setPositions: (entries: Iterable<readonly [string, Point]>) => void;
  setColor: (keys: readonly string[], color: ConnectionColor | null) => void;
  setBends: (edgeKey: string, bends: readonly Point[]) => void;
  clearBends: (edgeKeys: Iterable<string>) => void;
  setViewport: (viewport: Viewport) => void;
}

const SAVE_DEBOUNCE_MS = 400;
const VIEWPORT_KEY = "main";

function emptyCustom(): DiagramCustom {
  return { x: null, y: null, zoom: null, color: null, waypoints: [] };
}

function rowKey(kind: DiagramCustomKind, entityKey: string): string {
  return `${kind}\u0000${entityKey}`;
}

export function useDiagramCustomizations(connectionId: string, schema: string | null, onError: (message: string) => void): DiagramCustomizations {
  const scopeKey = `${connectionId}\u0000${schema ?? ""}`;
  // What has been read, and for which scope: a different scope reads as "not ready"
  // until its own rows arrive, with no state reset needed in an effect.
  const [loaded, setLoaded] = useState<{ key: string; rows: ReadonlyMap<string, DiagramCustomization> } | null>(null);
  const ready = loaded?.key === scopeKey;
  const rows = useMemo(() => (loaded?.key === scopeKey ? loaded.rows : new Map<string, DiagramCustomization>()), [loaded, scopeKey]);
  const pending = useRef(new Map<string, DiagramCustomization>());
  const timer = useRef<number | null>(null);
  const scope = useRef({ connectionId, schema, onError });
  useEffect(() => {
    scope.current = { connectionId, schema, onError };
  });

  useEffect(() => {
    const token = { cancelled: false };
    void (async () => {
      let stored: DiagramCustomization[] = [];
      try {
        stored = await ipc("list_diagram_customizations", { connectionId, schema });
      } catch {
        // A diagram without its remembered layout is still a diagram.
      }
      if (!token.cancelled) setLoaded({ key: scopeKey, rows: new Map(stored.map((c) => [rowKey(c.kind, c.entityKey), c])) });
    })();
    return () => {
      token.cancelled = true;
    };
  }, [connectionId, schema, scopeKey]);

  const write = useCallback(async () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    const upserts = [...pending.current.values()];
    pending.current.clear();
    if (upserts.length === 0) return;
    const { connectionId: id, schema: s, onError: report } = scope.current;
    try {
      await ipc("save_diagram_customizations", { connectionId: id, schema: s, upserts, removals: [] });
    } catch (raw) {
      report(raw instanceof Error ? raw.message : "The diagram layout could not be saved.");
    }
  }, []);

  // The view closing flushes what is waiting.
  useEffect(
    () => () => {
      void write();
    },
    [write],
  );

  const patch = useCallback(
    (changes: readonly { kind: DiagramCustomKind; entityKey: string; apply: (current: DiagramCustom) => DiagramCustom }[]) => {
      setLoaded((current) => {
        if (!current) return current;
        const next = new Map(current.rows);
        for (const change of changes) {
          const key = rowKey(change.kind, change.entityKey);
          const updated: DiagramCustomization = {
            kind: change.kind,
            entityKey: change.entityKey,
            custom: change.apply(next.get(key)?.custom ?? emptyCustom()),
          };
          next.set(key, updated);
          pending.current.set(key, updated);
        }
        return { key: current.key, rows: next };
      });
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => {
        void write();
      }, SAVE_DEBOUNCE_MS);
    },
    [write],
  );

  const setPositions = useCallback(
    (entries: Iterable<readonly [string, Point]>) => {
      patch([...entries].map(([entityKey, p]) => ({ kind: "node", entityKey, apply: (c: DiagramCustom) => ({ ...c, x: Math.round(p.x), y: Math.round(p.y) }) })));
    },
    [patch],
  );

  const setColor = useCallback(
    (keys: readonly string[], color: ConnectionColor | null) => {
      patch(keys.map((entityKey) => ({ kind: "node", entityKey, apply: (c: DiagramCustom) => ({ ...c, color }) })));
    },
    [patch],
  );

  const setBends = useCallback(
    (edgeKey: string, bends: readonly Point[]) => {
      patch([{ kind: "edge", entityKey: edgeKey, apply: (c: DiagramCustom) => ({ ...c, waypoints: bends.map((b) => ({ x: b.x, y: b.y })) }) }]);
    },
    [patch],
  );

  const clearBends = useCallback(
    (edgeKeys: Iterable<string>) => {
      patch([...edgeKeys].map((entityKey) => ({ kind: "edge", entityKey, apply: (c: DiagramCustom) => ({ ...c, waypoints: [] }) })));
    },
    [patch],
  );

  const setViewport = useCallback(
    (viewport: Viewport) => {
      patch([{ kind: "viewport", entityKey: VIEWPORT_KEY, apply: (c: DiagramCustom) => ({ ...c, x: viewport.x, y: viewport.y, zoom: viewport.zoom }) }]);
    },
    [patch],
  );

  const derived = useMemo(() => {
    const positions = new Map<string, Point>();
    const colors = new Map<string, ConnectionColor>();
    const bends = new Map<string, readonly Point[]>();
    let viewport: Viewport | null = null;
    for (const row of rows.values()) {
      const { x, y, zoom, color, waypoints } = row.custom;
      if (row.kind === "node") {
        if (x !== null && y !== null) positions.set(row.entityKey, { x, y });
        if (color !== null) colors.set(row.entityKey, color);
      } else if (row.kind === "edge") {
        if (waypoints.length > 0) bends.set(row.entityKey, waypoints);
      } else if (x !== null && y !== null && zoom !== null) {
        viewport = { x, y, zoom };
      }
    }
    return { positions, colors, bends, viewport };
  }, [rows]);

  return { ready, ...derived, setPositions, setColor, setBends, clearBends, setViewport };
}
