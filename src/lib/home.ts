// SOT: home-entities, home-item-meta, pin-entities, recent-entities
import type { ConnectionSummary, Document, HomeEntity, HomeItemKind, SavedQuery, TableRef } from "./bindings";
import type { IconName } from "./icons";

// WHAT:  What a Home pin or recent is made of: the label and icon of each kind, and
//        the constructors that turn a connection, table, saved query or document
//        into the `HomeEntity` the server stores.
// WHY:   Pin buttons, the recents recorder, the palette and the Home board all need
//        the same entity for the same thing; building it in one place is what makes
//        "is this pinned?" agree between them.
// WHERE: src/stores/workspace.ts (pins), src/stores/recents.ts, src/features/home

export const HOME_KINDS: Record<HomeItemKind, { label: string; plural: string; icon: IconName }> = {
  connection: { label: "Connection", plural: "Connections", icon: "database" },
  table: { label: "Table", plural: "Tables", icon: "table" },
  saved_query: { label: "Query", plural: "Queries", icon: "file" },
  dashboard: { label: "Dashboard", plural: "Dashboards", icon: "columns" },
  workflow: { label: "Workflow", plural: "Workflows", icon: "play" },
  diagram: { label: "Diagram", plural: "Diagrams", icon: "view" },
};

export function connectionEntity(connection: Pick<ConnectionSummary, "id" | "name">): HomeEntity {
  return { kind: "connection", itemId: connection.id, label: connection.name, connectionId: connection.id, schemaName: null, tableName: null };
}

// WHAT:  `<connection id>:<schema.table>` — unique across connections, so the same
//        table name on two servers pins separately.
export function tableItemId(connectionId: string, table: TableRef): string {
  return `${connectionId}:${table.schema === null ? table.name : `${table.schema}.${table.name}`}`;
}

export function tableEntity(connectionId: string, table: TableRef): HomeEntity {
  return {
    kind: "table",
    itemId: tableItemId(connectionId, table),
    label: table.schema === null ? table.name : `${table.schema}.${table.name}`,
    connectionId,
    schemaName: table.schema,
    tableName: table.name,
  };
}

// WHAT:  A saved query opens on its own connection, or — when it is shared
//        across connections — on the one the caller is working in. Null when
//        there is neither.
export function savedQueryEntity(query: Pick<SavedQuery, "id" | "name" | "connectionId">, fallbackConnectionId: string | null): HomeEntity | null {
  const connectionId = query.connectionId ?? fallbackConnectionId;
  if (connectionId === null) return null;
  return { kind: "saved_query", itemId: query.id, label: query.name, connectionId, schemaName: null, tableName: null };
}

export function documentEntity(doc: Pick<Document, "id" | "kind" | "name" | "connectionId">, fallbackConnectionId: string | null): HomeEntity | null {
  const connectionId = doc.connectionId ?? fallbackConnectionId;
  if (connectionId === null) return null;
  return { kind: doc.kind, itemId: doc.id, label: doc.name, connectionId, schemaName: null, tableName: null };
}

export function sameEntity(a: Pick<HomeEntity, "kind" | "itemId">, b: Pick<HomeEntity, "kind" | "itemId">): boolean {
  return a.kind === b.kind && a.itemId === b.itemId;
}

// WHAT:  Entity key for React lists and lookups.
export function entityKey(entity: Pick<HomeEntity, "kind" | "itemId">): string {
  return `${entity.kind}:${entity.itemId}`;
}
