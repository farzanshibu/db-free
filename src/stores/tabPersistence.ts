// SOT: tab-persistence, restored-tabs, closed-tab-stack-entry
import type { DocumentKind, ObjectKind, ObjectRef, TableRef, Tool } from "@/lib/bindings";
import type { JsonValue } from "@/lib/bindings/serde_json/JsonValue";
import { parseJson } from "@/lib/json";
import { OBJECT_KINDS, TOOLS } from "@/lib/objects";
import type { Tab } from "./workspace";

// WHAT:  The open tabs (every kind, in order) and the active one, written to
//        localStorage and read back at startup.
// WHY:   Query tabs already came back from their buffers; a table, a diagram
//        or a dashboard you had open did not, so a restart meant rebuilding the
//        workspace by hand.
// HOW:   Stored as JSON and decoded field by field through `parseJson` (the
//        one audited JSON boundary): a tab whose shape or connection no longer
//        checks out is dropped rather than trusted. Transient fields (a query
//        tab's seed text, a table tab's one-off filters) are not stored — the
//        buffer and the remembered table state hold those.
// WHERE: src/stores/workspace.ts (bootstrap, subscribe)
const TABS_KEY = "db-free:open-tabs";

export interface StoredTabs {
  tabs: Tab[];
  active: string | null;
}

export function readStoredTabs(): StoredTabs {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(TABS_KEY);
  } catch {
    return { tabs: [], active: null };
  }
  const root = raw === null ? undefined : parseJson(raw);
  const obj = record(root);
  if (!obj) return { tabs: [], active: null };
  const list = Array.isArray(obj.tabs) ? obj.tabs : [];
  return { tabs: list.flatMap((t) => decodeTab(t) ?? []), active: text(obj.active) };
}

export function writeStoredTabs(tabs: readonly Tab[], active: string | null): void {
  const stored = tabs.map(storable);
  try {
    localStorage.setItem(TABS_KEY, JSON.stringify({ tabs: stored, active }));
  } catch {
    // storage unavailable: the workspace just will not come back next launch
  }
}

/// Drops the fields that only mean something the moment a tab is opened.
export function storable(tab: Tab): Tab {
  if (tab.kind === "query") return { id: tab.id, kind: "query", connectionId: tab.connectionId, title: tab.title };
  if (tab.kind === "table") return { id: tab.id, kind: "table", connectionId: tab.connectionId, table: tab.table, filterKey: 0 };
  return tab;
}

type JsonRecord = Record<string, JsonValue>;

function record(value: JsonValue | undefined): JsonRecord | null {
  return value !== undefined && value !== null && typeof value === "object" && !Array.isArray(value) ? value : null;
}

function text(value: JsonValue | undefined): string | null {
  return typeof value === "string" ? value : null;
}

function isObjectKind(value: string): value is ObjectKind {
  return value in OBJECT_KINDS;
}

function isTool(value: string): value is Tool {
  return value in TOOLS;
}

function isDocumentKind(value: string): value is DocumentKind {
  return value === "dashboard" || value === "workflow" || value === "diagram";
}

function tableRef(value: JsonValue | undefined): TableRef | null {
  const obj = record(value);
  const name = text(obj?.name);
  if (!obj || name === null) return null;
  return { schema: text(obj.schema), name };
}

function objectRef(value: JsonValue | undefined): ObjectRef | null {
  const obj = record(value);
  const name = text(obj?.name);
  const kind = text(obj?.kind);
  if (!obj || name === null || kind === null || !isObjectKind(kind)) return null;
  return { kind, name, parent: text(obj.parent) };
}

function decodeTab(value: JsonValue): Tab | null {
  const obj = record(value);
  const id = text(obj?.id);
  const kind = text(obj?.kind);
  if (!obj || id === null || kind === null) return null;
  const connectionId = text(obj.connectionId);
  if (kind === "document") {
    const documentKind = text(obj.documentKind);
    const documentId = text(obj.documentId);
    if (documentKind === null || !isDocumentKind(documentKind) || documentId === null) return null;
    return { id, kind, connectionId, documentKind, documentId };
  }
  if (connectionId === null) return null;
  switch (kind) {
    case "query": {
      const title = text(obj.title);
      return { id, kind, connectionId, title: title ?? "Query" };
    }
    case "table": {
      const table = tableRef(obj.table);
      return table ? { id, kind, connectionId, table, filterKey: 0 } : null;
    }
    case "erd":
      return { id, kind, connectionId, schema: text(obj.schema) };
    case "object": {
      const reference = objectRef(obj.reference);
      return reference ? { id, kind, connectionId, reference } : null;
    }
    case "tool": {
      const tool = text(obj.tool);
      return tool !== null && isTool(tool) ? { id, kind, connectionId, tool } : null;
    }
    case "history":
    case "transfer":
    case "chat":
    case "admin":
      return { id, kind, connectionId };
    default:
      return null;
  }
}
