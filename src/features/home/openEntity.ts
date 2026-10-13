// SOT: open-home-entity, pin-open, recent-open
import type { HomeEntity } from "@/lib/bindings";
import { useRecents } from "@/stores/recents";
import { useWorkspace } from "@/stores/workspace";

// WHAT:  Opens whatever a pin, a recent, a palette entry or a saved-query row stands
//        for: connects to its connection if needed, switches to it, then opens the
//        item — a table tab, a query tab with the saved SQL, a document tab.
// WHY:   "Click a pin and land in the item" has the same steps from every place that
//        lists one, and the steps have failure cases (the connection is gone, will
//        not connect, the item was deleted) that should read the same everywhere.
// HOW:   Failures are shown as a toast and end the call; nothing is thrown.
//        Saved queries have no tab of their own to watch, so they are recorded as
//        recents here; tables, documents and connections are recorded by the
//        workspace subscription in src/stores/recents.ts.
// WHERE: src/features/home/HomeBoard.tsx, src/features/queries/QueriesPanel.tsx, src/features/palette/CommandPalette.tsx
export async function openHomeEntity(entity: HomeEntity): Promise<void> {
  const workspace = useWorkspace.getState();
  if (!workspace.connections.some((c) => c.id === entity.connectionId)) {
    workspace.showError("That connection no longer exists, so its items cannot be opened.");
    return;
  }
  if (workspace.sessions.includes(entity.connectionId)) {
    workspace.selectConnection(entity.connectionId);
  } else if (!(await workspace.connect(entity.connectionId))) {
    return; // connect() already showed why
  }
  // Re-read: connecting changed the store.
  const state = useWorkspace.getState();
  switch (entity.kind) {
    case "connection":
      return;
    case "table":
      if (entity.tableName === null) {
        state.showError("This pin is missing its table name.");
        return;
      }
      state.openTable(entity.connectionId, { schema: entity.schemaName, name: entity.tableName });
      return;
    case "saved_query": {
      const query = state.savedQueries.find((q) => q.id === entity.itemId);
      if (query === undefined) {
        state.showError(`“${entity.label}” no longer exists.`);
        return;
      }
      state.openQuery(entity.connectionId, query.sql, query.name);
      useRecents.getState().record({ ...entity, label: query.name });
      return;
    }
    case "dashboard":
    case "workflow":
    case "diagram":
      state.openDocument(entity.kind, entity.itemId, entity.connectionId);
      return;
  }
}
