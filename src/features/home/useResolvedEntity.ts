// SOT: resolve-home-entity, live-pin-labels, stale-pin-filter
import { useCallback } from "react";
import type { ConnectionSummary, Document, DocumentKind, HomeEntity, SavedQuery } from "@/lib/bindings";
import { useWorkspace } from "@/stores/workspace";

export interface ResolvedEntity {
  /// The name the item has now (a rename since it was pinned shows up here).
  label: string;
  connection: ConnectionSummary;
}

// WHAT:  Looks a pin or recent up against what the app currently holds: the live name,
//        and the connection it belongs to. Null means it cannot be opened any more.
// WHY:   A pin stores the label it had when it was pinned. Showing that after a rename
//        — or showing a pin at all after its connection or saved query was deleted —
//        would make the board lie.
// HOW:   Connections and saved queries are always loaded. Documents load per kind when
//        a sidebar opens, so an unloaded kind keeps the stored label rather than being
//        treated as deleted. Tables live in the user's database and are not checked.
// WHERE: src/features/home/PinnedBoard.tsx, src/features/home/RecentsPanel.tsx, src/features/palette/libraryActions.ts
export interface EntityLookup {
  connections: readonly ConnectionSummary[];
  savedQueries: readonly SavedQuery[];
  documents: Readonly<Record<DocumentKind, readonly Document[]>>;
}

export function resolveEntity(entity: HomeEntity, lookup: EntityLookup): ResolvedEntity | null {
  const connection = lookup.connections.find((c) => c.id === entity.connectionId);
  if (connection === undefined) return null;
  switch (entity.kind) {
    case "connection":
      return { label: connection.name, connection };
    case "table":
      return { label: entity.label, connection };
    case "saved_query": {
      const query = lookup.savedQueries.find((q) => q.id === entity.itemId);
      return query === undefined ? null : { label: query.name, connection };
    }
    case "dashboard":
    case "workflow":
    case "diagram": {
      const loaded = lookup.documents[entity.kind];
      if (loaded.length === 0) return { label: entity.label, connection };
      const doc = loaded.find((d) => d.id === entity.itemId);
      return doc === undefined ? null : { label: doc.name, connection };
    }
  }
}

export function useResolveEntity(): (entity: HomeEntity) => ResolvedEntity | null {
  const connections = useWorkspace((s) => s.connections);
  const savedQueries = useWorkspace((s) => s.savedQueries);
  const documents = useWorkspace((s) => s.documents);
  return useCallback((entity) => resolveEntity(entity, { connections, savedQueries, documents }), [connections, savedQueries, documents]);
}
