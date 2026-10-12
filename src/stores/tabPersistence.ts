// SOT: tab-persistence, restored-tabs, closed-tab-stack-entry
import type { DocumentKind, ObjectKind, ObjectRef, TableRef, Tool } from "@/lib/bindings";
import type { JsonValue } from "@/lib/bindings/serde_json/JsonValue";
import { parseJson } from "@/lib/json";
import { OBJECT_KINDS, TOOLS } from "@/lib/objects";
import { initialPanes, type PaneLayout, type PaneNode } from "@/lib/panes";
import { windowContext } from "@/lib/shellEvents";
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

// WHAT:  Where this window keeps its tabs: the main window the original key, a
//        connection window one of its own, and a plain extra window none.
// WHY:   Every window shares one localStorage, so two windows writing one key
//        would erase each other's tabs. Connection windows have a stable label
//        and so come back with their tabs; `win-N` windows are throwaway.
function storageKey(): string | null {
  const { label } = windowContext();
  if (label === "main") return TABS_KEY;
  return label.startsWith("conn-") ? `${TABS_KEY}:${label}` : null;
}

export interface StoredTabs {
  tabs: Tab[];
  active: string | null;
  /// The pane layout the tabs were in; null for tabs stored before panes existed.
  layout: PaneLayout | null;
}

const NOTHING_STORED: StoredTabs = { tabs: [], active: null, layout: null };

export function readStoredTabs(): StoredTabs {
  const key = storageKey();
  if (key === null) return NOTHING_STORED;
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(key);
  } catch {
    return NOTHING_STORED;
  }
  const root = raw === null ? undefined : parseJson(raw);
  const obj = record(root);
  if (!obj) return NOTHING_STORED;
  const list = Array.isArray(obj.tabs) ? obj.tabs : [];
  return { tabs: list.flatMap((t) => decodeTab(t) ?? []), active: text(obj.active), layout: decodeLayout(obj.layout) };
}

export function writeStoredTabs(tabs: readonly Tab[], active: string | null, layout: PaneLayout): void {
  const key = storageKey();
  if (key === null) return;
  const stored = tabs.map(storable);
  try {
    localStorage.setItem(key, JSON.stringify({ tabs: stored, active, layout }));
  } catch {
    // storage unavailable: the workspace just will not come back next launch
  }
}

// WHAT:  Tab ids that other windows have claimed (their stored tab lists).
// WHY:   A query tab's text lives in a buffer keyed by tab id, and the main
//        window restores every buffer that has text. Without this it would also
//        restore the buffers of tabs a connection window owns, and the same
//        script would be open in two windows at once.
export function tabIdsOfOtherWindows(): Set<string> {
  const mine = storageKey();
  const ids = new Set<string>();
  try {
    for (let i = 0; i < localStorage.length; i += 1) {
      const key = localStorage.key(i);
      if (key === null || key === mine || !key.startsWith(`${TABS_KEY}:`)) continue;
      const raw = localStorage.getItem(key);
      const list = record(raw === null ? undefined : parseJson(raw))?.tabs;
      if (!Array.isArray(list)) continue;
      for (const entry of list) {
        const id = text(record(entry)?.id);
        if (id !== null) ids.add(id);
      }
    }
  } catch {
    // storage unavailable: nothing is claimed, which only risks a duplicate editor
  }
  return ids;
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

const MAX_STORED_DEPTH = 4;

function decodeNode(value: JsonValue | undefined, depth: number): PaneNode | null {
  const obj = record(value);
  const id = text(obj?.id);
  const kind = text(obj?.kind);
  if (!obj || id === null || kind === null || depth > MAX_STORED_DEPTH) return null;
  if (kind === "leaf") return { kind: "leaf", id };
  if (kind !== "split") return null;
  const direction = text(obj.direction);
  const first = decodeNode(obj.first, depth + 1);
  const second = decodeNode(obj.second, depth + 1);
  const ratio = obj.ratio;
  if ((direction !== "row" && direction !== "column") || first === null || second === null || typeof ratio !== "number") return null;
  return { kind: "split", id, direction, ratio, first, second };
}

// WHAT:  The pane layout saved with the tabs, or null when it is missing or
//        does not check out (the tabs then restore into one pane).
function decodeLayout(value: JsonValue | undefined): PaneLayout | null {
  const obj = record(value);
  const root = decodeNode(obj?.root, 0);
  const focused = text(obj?.focused);
  const counter = obj?.counter;
  if (!obj || root === null || focused === null || typeof counter !== "number") return null;
  const active: Record<string, string | null> = {};
  for (const [pane, tab] of Object.entries(record(obj.active) ?? {})) active[pane] = text(tab);
  const owner: Record<string, string> = {};
  for (const [tab, pane] of Object.entries(record(obj.owner) ?? {})) {
    const owned = text(pane);
    if (owned !== null) owner[tab] = owned;
  }
  return { root, focused, active, owner, counter };
}

/// The layout to start from: the stored one, or a single pane.
export function startingLayout(stored: StoredTabs): PaneLayout {
  return stored.layout ?? initialPanes();
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
    case "schema-compare":
      return { id, kind, connectionId, schema: text(obj.schema) };
    case "data-compare":
      return { id, kind, connectionId, table: tableRef(obj.table) };
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
