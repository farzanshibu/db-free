// SOT: workspace-store, pages, tabs, restored-query-tabs, reopen-closed-tab, tab-cycling, split-tab-state, sidebar-mode, active-connection, catalog-cache, foreign-key-cache, objects-cache, session-info-cache, settings-cache, pending-changes, saved-queries-cache, documents-cache, ui-toasts
import { create } from "zustand";
import type {
  AppError,
  AppSettings,
  ColumnInfo,
  ConnectionInput,
  ConnectionSummary,
  Document,
  DocumentKind,
  FilterRule,
  ForeignKey,
  ObjectKind,
  ObjectRef,
  ObjectSummary,
  SavedQuery,
  SchemaCatalog,
  SessionInfo,
  StagedChange,
  TableRef,
  Tool,
} from "@/lib/bindings";
import { errorMessage, ipc, normalizeError } from "@/lib/ipc";
import type { Density } from "@/lib/format";
import type { EnginePreset } from "@/lib/engines";
import type { SettingsSection } from "@/lib/settingsSections";
import { inputFromSummary } from "@/lib/connectionGroups";
import { toast } from "@/components/ui/sonner";
import { readStoredTabs, storable, writeStoredTabs } from "./tabPersistence";
import { confirmLeavingTransaction, useTransactions } from "./transactions";

export type SidebarMode = "tables" | "objects" | "queries" | "dashboards" | "workflows" | "diagrams";

export type Page =
  | { kind: "connections" }
  | { kind: "connection-picker" }
  | { kind: "connection-form"; editingId: string | null; preset?: EnginePreset; draft?: ConnectionInput }
  | { kind: "workspace" }
  | { kind: "settings"; section?: SettingsSection }
  | { kind: "capabilities" };

export type Tab =
  | { id: string; kind: "table"; connectionId: string; table: TableRef; initialFilters?: FilterRule[]; filterKey: number }
  | { id: string; kind: "query"; connectionId: string; title: string; seedSql?: string }
  | { id: string; kind: "history"; connectionId: string }
  | { id: string; kind: "transfer"; connectionId: string }
  | { id: string; kind: "erd"; connectionId: string; schema: string | null }
  | { id: string; kind: "document"; connectionId: string | null; documentKind: DocumentKind; documentId: string }
  | { id: string; kind: "chat"; connectionId: string }
  | { id: string; kind: "object"; connectionId: string; reference: ObjectRef }
  | { id: string; kind: "admin"; connectionId: string }
  | { id: string; kind: "tool"; connectionId: string; tool: Tool };

export function tableKey(table: TableRef): string {
  return table.schema === null ? table.name : `${table.schema}.${table.name}`;
}

export function objectKey(reference: ObjectRef): string {
  return `${reference.kind}:${reference.parent ?? ""}:${reference.name}`;
}

// WHAT:  Cache key for one object listing (connection + kind + namespace).
export function objectsKey(connectionId: string, kind: ObjectKind, parent: string | null): string {
  return `${connectionId}:${kind}:${parent ?? ""}`;
}

let queryCounter = 0;

interface WorkspaceState {
  ready: boolean;
  page: Page;
  sidebar: SidebarMode;
  paletteOpen: boolean;
  settings: AppSettings | null;
  connections: ConnectionSummary[];
  sessions: string[];
  sessionInfos: Record<string, SessionInfo>;
  catalogs: Record<string, SchemaCatalog>;
  foreignKeys: Record<string, ForeignKey[]>;
  columnsCache: Record<string, ColumnInfo[]>;
  /// Object explorer listings, keyed by objectsKey(); cleared on disconnect / database switch.
  objectsCache: Record<string, ObjectSummary[]>;
  schemaFilter: Record<string, string | null>;
  activeConnectionId: string | null;
  connecting: string | null;
  tabs: Tab[];
  activeTabId: string | null;
  /// Most recently closed last; Reopen closed tab pops from the end.
  closedTabs: Tab[];
  /// The tab shown in the side pane of a split view; never the active tab.
  splitTabId: string | null;
  density: Density;
  savedQueries: SavedQuery[];
  documents: Record<DocumentKind, Document[]>;
  /// Review-mode edits waiting for Commit, keyed by connection id.
  pendingChanges: Record<string, StagedChange[]>;
  changesPanelOpen: boolean;

  bootstrap: () => Promise<void>;
  refreshConnections: () => Promise<void>;
  goConnections: () => void;
  goPicker: () => void;
  goWorkspace: () => void;
  goSettings: () => void;
  /// Settings open at one section (the page nav, ⌘K "Keyboard shortcuts").
  goSettingsSection: (section: SettingsSection) => void;
  goCapabilities: () => void;
  openForm: (editingId?: string, preset?: EnginePreset, draft?: ConnectionInput) => void;
  setSidebar: (mode: SidebarMode) => void;
  setPaletteOpen: (open: boolean) => void;
  loadSettings: () => Promise<void>;
  saveSettings: (settings: AppSettings, aiApiKey: string | null) => Promise<void>;
  saveConnection: (id: string | null, input: ConnectionInput) => Promise<ConnectionSummary>;
  deleteConnection: (id: string) => Promise<void>;
  /// Favourite / folder / colour change from the connections list; keeps any live session.
  organizeConnection: (id: string, patch: Partial<Pick<ConnectionInput, "folder" | "color" | "favorite">>) => Promise<void>;
  connect: (id: string, database?: string) => Promise<boolean>;
  disconnect: (id: string) => Promise<void>;
  selectConnection: (id: string) => void;
  switchDatabase: (id: string, database: string) => Promise<void>;
  setSchemaFilter: (id: string, schema: string | null) => void;
  loadCatalog: (id: string) => Promise<void>;
  loadColumns: (id: string, table: TableRef) => Promise<ColumnInfo[]>;
  loadObjects: (id: string, kind: ObjectKind, parent: string | null, force?: boolean) => Promise<ObjectSummary[]>;
  /// Drops every cached listing for the connection (after an action changed the server).
  invalidateObjects: (id: string) => void;
  openTable: (connectionId: string, table: TableRef, filters?: FilterRule[]) => void;
  openQuery: (connectionId: string, seedSql?: string, title?: string) => void;
  openHistory: (connectionId: string) => void;
  openTransfer: (connectionId: string) => void;
  openErd: (connectionId: string, schema: string | null) => void;
  openChat: (connectionId: string) => void;
  openObject: (connectionId: string, reference: ObjectRef) => void;
  openAdmin: (connectionId: string) => void;
  openTool: (connectionId: string, tool: Tool) => void;
  openDocument: (kind: DocumentKind, documentId: string, connectionId: string | null) => void;
  closeTab: (id: string) => void;
  /// Tab-bar context menu: keep one tab, drop the rest.
  closeOtherTabs: (id: string) => void;
  /// Tab-bar context menu: drop every tab after this one.
  closeTabsToRight: (id: string) => void;
  closeAllTabs: () => void;
  /// Brings back the last closed tab (a query tab with the text it had).
  reopenClosedTab: () => void;
  /// Ctrl+Tab / Ctrl+Shift+Tab: the next or previous tab, wrapping.
  cycleTab: (step: 1 | -1) => void;
  /// Shows a tab in the side pane; the main pane moves to a neighbour if it was that tab.
  openToSide: (id: string) => void;
  closeSplit: () => void;
  /// Exchanges the main and side tabs.
  swapSplit: () => void;
  activateTab: (id: string) => void;
  setDensity: (density: Density) => void;
  loadSavedQueries: () => Promise<void>;
  saveQuery: (query: SavedQuery) => Promise<SavedQuery>;
  deleteSavedQuery: (id: string) => Promise<void>;
  loadDocuments: (kind: DocumentKind) => Promise<void>;
  saveDocument: (doc: Document) => Promise<Document>;
  deleteDocument: (kind: DocumentKind, id: string) => Promise<void>;
  stageChange: (connectionId: string, change: StagedChange) => void;
  unstageChange: (connectionId: string, changeId: string) => void;
  clearChanges: (connectionId: string) => void;
  setChangesPanelOpen: (open: boolean) => void;
  showError: (error: AppError | string) => void;
  showInfo: (text: string) => void;
}

const DENSITY_KEY = "db-free.density";

function readDensity(): Density {
  try {
    const raw = localStorage.getItem(DENSITY_KEY);
    if (raw === "compact" || raw === "cozy" || raw === "comfortable") return raw;
  } catch {
    // storage unavailable — fall through to default
  }
  return "cozy";
}

function withoutKey<T>(record: Record<string, T>, key: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => k !== key));
}

function withoutPrefix<T>(record: Record<string, T>, prefix: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => !k.startsWith(prefix)));
}

function addTab(tabs: Tab[], tab: Tab): Tab[] {
  return tabs.some((t) => t.id === tab.id) ? tabs : [...tabs, tab];
}

/// How many closed tabs Reopen can walk back through.
const CLOSED_LIMIT = 25;

// WHAT:  Drops the stored buffer behind every query tab in `closed`, after
//        copying its text into the closed-tab stack.
// WHY:   A query tab is restored from its buffer at startup, so a tab the user
//        closed has to take its buffer with it or it comes back tomorrow — but
//        Reopen closed tab has to be able to bring the text back.
// WHERE: src-tauri/src/store/buffers.rs
function forgetBuffers(closed: readonly Tab[], keep: (tab: Tab) => void): void {
  const queries = closed.filter((t) => t.kind === "query");
  if (queries.length === 0) return;
  void (async () => {
    try {
      const buffers = await ipc("list_buffers");
      for (const tab of queries) {
        const content = buffers.find((b) => b.id === tab.id)?.content;
        if (content !== undefined) keep({ ...tab, seedSql: content });
      }
    } catch {
      // Without the text a reopened query tab starts empty; nothing else is lost.
    }
    for (const tab of queries) {
      void ipc("delete_buffer", { id: tab.id }).catch(() => {
        // The tab is gone from the UI either way; a failed cleanup is not worth a toast.
      });
    }
  })();
}

function pushClosed(stack: readonly Tab[], closed: readonly Tab[]): Tab[] {
  return [...stack, ...closed.map(storable)].slice(-CLOSED_LIMIT);
}

export const useWorkspace = create<WorkspaceState>()((set, get) => ({
  ready: false,
  page: { kind: "connections" },
  sidebar: "tables",
  paletteOpen: false,
  settings: null,
  connections: [],
  sessions: [],
  sessionInfos: {},
  catalogs: {},
  foreignKeys: {},
  columnsCache: {},
  objectsCache: {},
  schemaFilter: {},
  activeConnectionId: null,
  connecting: null,
  tabs: [],
  activeTabId: null,
  closedTabs: [],
  splitTabId: null,
  density: readDensity(),
  savedQueries: [],
  documents: { dashboard: [], workflow: [], diagram: [] },
  pendingChanges: {},
  changesPanelOpen: false,

  // WHAT:  Loads the app's own state, and puts back the query tabs that were open.
  // WHY:   PRD §4.3 — an editor that loses the script you were writing when the app
  //        restarts is an editor you cannot trust. The buffers were already saved
  //        per tab; without this they were saved and never read again, because a
  //        fresh tab id never matched one.
  // WHERE: src-tauri/src/store/buffers.rs, src/features/editor/QueryPane.tsx
  bootstrap: async () => {
    try {
      const [connections, sessions, settings, savedQueries, buffers] = await Promise.all([
        ipc("list_connections"),
        ipc("active_sessions"),
        ipc("get_settings"),
        ipc("list_saved_queries"),
        ipc("list_buffers"),
      ]);
      const known = new Set(connections.map((c) => c.id));
      const fromBuffers: Tab[] = buffers.flatMap((b) => {
        const connectionId = b.connectionId;
        if (connectionId === null || !known.has(connectionId) || b.content.trim().length === 0) return [];
        return [{ id: b.id, kind: "query" as const, connectionId, title: b.title.length > 0 ? b.title : "Query" }];
      });
      // Every other kind comes back from the stored tab list, in the order it
      // was left in; query tabs only when their buffer still has text.
      const stored = readStoredTabs();
      const buffered = new Map(fromBuffers.map((t) => [t.id, t]));
      const kept = stored.tabs.flatMap((t): Tab[] => {
        if (t.kind === "query") {
          const tab = buffered.get(t.id);
          return tab ? [tab] : [];
        }
        return t.connectionId === null || known.has(t.connectionId) ? [t] : [];
      });
      const restored = [...kept, ...fromBuffers.filter((t) => !kept.some((k) => k.id === t.id))];
      queryCounter = restored.filter((t) => t.kind === "query").length;
      const active = restored.some((t) => t.id === stored.active) ? stored.active : (restored[restored.length - 1]?.id ?? null);
      set({ connections, sessions, settings, savedQueries, ready: true, tabs: restored, activeTabId: active });
      const live = sessions[0];
      if (live !== undefined) {
        set({ activeConnectionId: live, page: { kind: "workspace" } });
        await get().loadCatalog(live);
      }
    } catch (raw) {
      set({ ready: true });
      get().showError(normalizeError(raw));
    }
  },

  refreshConnections: async () => {
    const connections = await ipc("list_connections");
    set({ connections });
  },

  goConnections: () => set({ page: { kind: "connections" } }),
  goPicker: () => set({ page: { kind: "connection-picker" } }),
  goWorkspace: () => set({ page: { kind: "workspace" } }),
  goSettings: () => set({ page: { kind: "settings" } }),
  goSettingsSection: (section) => set({ page: { kind: "settings", section } }),
  goCapabilities: () => set({ page: { kind: "capabilities" } }),
  openForm: (editingId, preset, draft) =>
    set({ page: { kind: "connection-form", editingId: editingId ?? null, ...(preset ? { preset } : {}), ...(draft ? { draft } : {}) } }),
  setSidebar: (mode) => set({ sidebar: mode, page: { kind: "workspace" } }),
  setPaletteOpen: (open) => set({ paletteOpen: open }),

  loadSettings: async () => {
    const settings = await ipc("get_settings");
    set({ settings });
  },
  saveSettings: async (settings, aiApiKey) => {
    const saved = await ipc("save_settings", { settings, aiApiKey });
    set({ settings: saved });
  },

  saveConnection: async (id, input) => {
    const saved = await ipc("save_connection", { id, input });
    await get().refreshConnections();
    if (id !== null && get().sessions.includes(id)) {
      await ipc("disconnect", { id });
      useTransactions.getState().forget(id);
      set((s) => ({
        sessions: s.sessions.filter((x) => x !== id),
        catalogs: withoutKey(s.catalogs, id),
        sessionInfos: withoutKey(s.sessionInfos, id),
      }));
    }
    return saved;
  },

  // WHAT:  One-click organisation (favourite, move to folder) without the form.
  // WHY:   Saving through the form reconnects a live session because host or
  //        credentials may have changed; filing a connection changes neither,
  //        so the session stays open here.
  // HOW:   The full input is rebuilt from the summary: secrets blank = keep.
  organizeConnection: async (id, patch) => {
    const current = get().connections.find((c) => c.id === id);
    if (!current) return;
    try {
      const saved = await ipc("save_connection", { id, input: { ...inputFromSummary(current), ...patch } });
      set((s) => ({ connections: s.connections.map((c) => (c.id === id ? saved : c)) }));
    } catch (raw) {
      get().showError(normalizeError(raw));
    }
  },

  deleteConnection: async (id) => {
    await ipc("delete_connection", { id });
    set((s) => ({
      connections: s.connections.filter((c) => c.id !== id),
      sessions: s.sessions.filter((x) => x !== id),
      catalogs: withoutKey(s.catalogs, id),
      foreignKeys: withoutKey(s.foreignKeys, id),
      sessionInfos: withoutKey(s.sessionInfos, id),
      objectsCache: withoutPrefix(s.objectsCache, `${id}:`),
      tabs: s.tabs.filter((t) => t.connectionId !== id),
      activeTabId: s.tabs.find((t) => t.id === s.activeTabId)?.connectionId === id ? null : s.activeTabId,
      splitTabId: s.tabs.find((t) => t.id === s.splitTabId)?.connectionId === id ? null : s.splitTabId,
      activeConnectionId: s.activeConnectionId === id ? null : s.activeConnectionId,
      pendingChanges: withoutKey(s.pendingChanges, id),
    }));
  },

  connect: async (id, database) => {
    set({ connecting: id });
    try {
      const opened = await ipc("connect", { id, database: database ?? null });
      set((s) => ({
        // The first tunnelled connect pins the SSH host key; keep the form's copy current.
        connections: s.connections.map((c) => (c.id === id ? { ...c, ssh: opened.ssh } : c)),
        sessions: s.sessions.includes(id) ? s.sessions : [...s.sessions, id],
        activeConnectionId: id,
        page: { kind: "workspace" },
      }));
      await get().loadCatalog(id);
      return true;
    } catch (raw) {
      get().showError(normalizeError(raw));
      return false;
    } finally {
      set({ connecting: null });
    }
  },

  disconnect: async (id) => {
    if (!confirmLeavingTransaction(id, "Disconnecting rolls it back.")) return;
    await ipc("disconnect", { id });
    useTransactions.getState().forget(id);
    set((s) => ({
      sessions: s.sessions.filter((x) => x !== id),
      catalogs: withoutKey(s.catalogs, id),
      sessionInfos: withoutKey(s.sessionInfos, id),
      objectsCache: withoutPrefix(s.objectsCache, `${id}:`),
      tabs: s.tabs.filter((t) => t.connectionId !== id),
      splitTabId: s.tabs.find((t) => t.id === s.splitTabId)?.connectionId === id ? null : s.splitTabId,
    }));
  },

  selectConnection: (id) => {
    if (get().sessions.includes(id)) {
      set({ activeConnectionId: id, page: { kind: "workspace" } });
      if (!(id in get().catalogs)) void get().loadCatalog(id);
    } else {
      void get().connect(id);
    }
  },

  switchDatabase: async (id, database) => {
    if (!confirmLeavingTransaction(id, "Switching database reconnects and rolls it back.")) return;
    const ok = await get().connect(id, database);
    if (ok) useTransactions.getState().forget(id);
    if (ok) {
      set((s) => ({
        tabs: s.tabs.filter((t) => !(t.connectionId === id && (t.kind === "table" || t.kind === "erd" || t.kind === "object"))),
        columnsCache: {},
        objectsCache: withoutPrefix(s.objectsCache, `${id}:`),
        schemaFilter: { ...s.schemaFilter, [id]: null },
        pendingChanges: withoutKey(s.pendingChanges, id),
      }));
    }
  },

  setSchemaFilter: (id, schema) => set((s) => ({ schemaFilter: { ...s.schemaFilter, [id]: schema } })),

  loadCatalog: async (id) => {
    try {
      const [catalog, info, fks] = await Promise.all([
        ipc("load_catalog", { connectionId: id }),
        ipc("describe_session", { connectionId: id }),
        ipc("load_foreign_keys", { connectionId: id }).catch((): ForeignKey[] => []),
      ]);
      set((s) => {
        const current = s.schemaFilter[id];
        const names = catalog.schemas.map((x) => x.name);
        const preferred = current !== undefined && current !== null && names.includes(current) ? current : names.includes("public") ? "public" : null;
        return {
          catalogs: { ...s.catalogs, [id]: catalog },
          foreignKeys: { ...s.foreignKeys, [id]: fks },
          sessionInfos: { ...s.sessionInfos, [id]: info },
          schemaFilter: { ...s.schemaFilter, [id]: names.length > 1 ? preferred : null },
        };
      });
    } catch (raw) {
      get().showError(normalizeError(raw));
    }
  },

  loadColumns: async (id, table) => {
    const key = `${id}:${tableKey(table)}`;
    const cached = get().columnsCache[key];
    if (cached) return cached;
    const columns = await ipc("load_columns", { connectionId: id, table });
    set((s) => ({ columnsCache: { ...s.columnsCache, [key]: columns } }));
    return columns;
  },

  // WHAT:  One object listing per (connection, kind, namespace), cached until a
  //        refresh asks with `force` or the session changes.
  loadObjects: async (id, kind, parent, force = false) => {
    const key = objectsKey(id, kind, parent);
    const cached = get().objectsCache[key];
    if (cached && !force) return cached;
    const objects = await ipc("list_objects", { connectionId: id, kind, parent });
    set((s) => ({ objectsCache: { ...s.objectsCache, [key]: objects } }));
    return objects;
  },

  // WHAT:  Opens (or focuses) a table tab; with `filters` (foreign-key traversal)
  //        the tab remounts with those filters applied.
  openTable: (connectionId, table, filters) => {
    const id = `table:${connectionId}:${tableKey(table)}`;
    set((s) => {
      const existing = s.tabs.find((t) => t.id === id);
      const tab: Tab = { id, kind: "table", connectionId, table, filterKey: filters ? Date.now() : (existing?.kind === "table" ? existing.filterKey : 0), ...(filters ? { initialFilters: filters } : {}) };
      const tabs = existing ? s.tabs.map((t) => (t.id === id ? tab : t)) : [...s.tabs, tab];
      return { tabs, activeTabId: id, page: { kind: "workspace" } };
    });
  },

  openQuery: (connectionId, seedSql, title) => {
    queryCounter += 1;
    const id = `query:${connectionId}:${Date.now()}-${queryCounter}`;
    const tab: Tab = { id, kind: "query", connectionId, title: title ?? `Query ${queryCounter}`, ...(seedSql !== undefined ? { seedSql } : {}) };
    set((s) => ({ tabs: [...s.tabs, tab], activeTabId: id, page: { kind: "workspace" } }));
  },

  openHistory: (connectionId) => {
    const id = `history:${connectionId}`;
    set((s) => ({ tabs: addTab(s.tabs, { id, kind: "history", connectionId }), activeTabId: id, page: { kind: "workspace" } }));
  },

  openTransfer: (connectionId) => {
    const id = `transfer:${connectionId}`;
    set((s) => ({ tabs: addTab(s.tabs, { id, kind: "transfer", connectionId }), activeTabId: id, page: { kind: "workspace" } }));
  },

  openErd: (connectionId, schema) => {
    const id = `erd:${connectionId}:${schema ?? "*"}`;
    set((s) => ({ tabs: addTab(s.tabs, { id, kind: "erd", connectionId, schema }), activeTabId: id, page: { kind: "workspace" } }));
  },

  openChat: (connectionId) => {
    const id = `chat:${connectionId}`;
    set((s) => ({ tabs: addTab(s.tabs, { id, kind: "chat", connectionId }), activeTabId: id, page: { kind: "workspace" } }));
  },

  invalidateObjects: (id) => set((s) => ({ objectsCache: withoutPrefix(s.objectsCache, `${id}:`) })),

  openObject: (connectionId, reference) => {
    const id = `object:${connectionId}:${objectKey(reference)}`;
    set((s) => ({ tabs: addTab(s.tabs, { id, kind: "object", connectionId, reference }), activeTabId: id, page: { kind: "workspace" } }));
  },

  openAdmin: (connectionId) => {
    const id = `admin:${connectionId}`;
    set((s) => ({ tabs: addTab(s.tabs, { id, kind: "admin", connectionId }), activeTabId: id, page: { kind: "workspace" } }));
  },

  openTool: (connectionId, tool) => {
    const id = `tool:${connectionId}:${tool}`;
    set((s) => ({ tabs: addTab(s.tabs, { id, kind: "tool", connectionId, tool }), activeTabId: id, page: { kind: "workspace" } }));
  },

  openDocument: (kind, documentId, connectionId) => {
    const id = `doc:${kind}:${documentId}`;
    set((s) => ({ tabs: addTab(s.tabs, { id, kind: "document", connectionId, documentKind: kind, documentId }), activeTabId: id, page: { kind: "workspace" } }));
  },

  closeTab: (id) => {
    const closed = get().tabs.filter((t) => t.id === id);
    // The last query tab on a connection is the last Commit button for its transaction.
    const last = closed.find((t) => t.kind === "query" && !get().tabs.some((o) => o.id !== t.id && o.kind === "query" && o.connectionId === t.connectionId));
    if (last?.connectionId && !confirmLeavingTransaction(last.connectionId,"Closing its last query tab leaves it open until you disconnect.")) return;
    forgetBuffers(closed, rememberClosed);
    set((s) => {
      const index = s.tabs.findIndex((t) => t.id === id);
      const tabs = s.tabs.filter((t) => t.id !== id);
      const fallback = tabs[Math.max(0, index - 1)] ?? tabs[0];
      const splitTabId = s.splitTabId === id ? null : s.splitTabId;
      const activeTabId = s.activeTabId === id ? (tabs.filter((t) => t.id !== splitTabId)[Math.max(0, index - 1)]?.id ?? fallback?.id ?? null) : s.activeTabId;
      return { tabs, activeTabId, splitTabId: activeTabId === splitTabId ? null : splitTabId, closedTabs: pushClosed(s.closedTabs, closed) };
    });
  },

  closeOtherTabs: (id) => {
    const closed = get().tabs.filter((t) => t.id !== id);
    forgetBuffers(closed, rememberClosed);
    set((s) => ({ tabs: s.tabs.filter((t) => t.id === id), activeTabId: s.tabs.some((t) => t.id === id) ? id : null, splitTabId: null, closedTabs: pushClosed(s.closedTabs, closed) }));
  },

  closeTabsToRight: (id) => {
    const at = get().tabs.findIndex((t) => t.id === id);
    if (at < 0) return;
    const closed = get().tabs.slice(at + 1);
    forgetBuffers(closed, rememberClosed);
    set((s) => {
      const index = s.tabs.findIndex((t) => t.id === id);
      if (index < 0) return {};
      const tabs = s.tabs.slice(0, index + 1);
      const activeTabId = tabs.some((t) => t.id === s.activeTabId) ? s.activeTabId : id;
      const splitTabId = tabs.some((t) => t.id === s.splitTabId) && s.splitTabId !== activeTabId ? s.splitTabId : null;
      return { tabs, activeTabId, splitTabId, closedTabs: pushClosed(s.closedTabs, closed) };
    });
  },

  closeAllTabs: () => {
    const closed = get().tabs;
    forgetBuffers(closed, rememberClosed);
    set((s) => ({ tabs: [], activeTabId: null, splitTabId: null, closedTabs: pushClosed(s.closedTabs, closed) }));
  },

  // WHAT:  Ctrl+Shift+T. A query tab comes back as a new tab seeded with the
  //        text it had (its buffer was deleted on close); every other kind
  //        reopens as it was, unless its connection has since been deleted.
  reopenClosedTab: () => {
    const { closedTabs, connections } = get();
    const tab = closedTabs[closedTabs.length - 1];
    if (!tab) return;
    set({ closedTabs: closedTabs.slice(0, -1) });
    if (tab.connectionId !== null && !connections.some((c) => c.id === tab.connectionId)) return;
    if (tab.kind === "query") {
      get().openQuery(tab.connectionId, tab.seedSql, tab.title);
      return;
    }
    set((s) => ({ tabs: addTab(s.tabs, tab), page: { kind: "workspace" } }));
    get().activateTab(tab.id);
  },

  openToSide: (id) => {
    const { tabs, activeTabId } = get();
    if (!tabs.some((t) => t.id === id)) return;
    if (activeTabId === id) {
      const index = tabs.findIndex((t) => t.id === id);
      const neighbour = tabs[index - 1] ?? tabs[index + 1];
      if (!neighbour) return;
      set({ splitTabId: id, activeTabId: neighbour.id });
      return;
    }
    set({ splitTabId: id });
  },

  closeSplit: () => set({ splitTabId: null }),

  swapSplit: () => set((s) => (s.splitTabId === null || s.activeTabId === null ? {} : { splitTabId: s.activeTabId, activeTabId: s.splitTabId })),

  cycleTab: (step) => {
    const { tabs, activeTabId } = get();
    if (tabs.length === 0) return;
    const at = tabs.findIndex((t) => t.id === activeTabId);
    const next = tabs[(at + step + tabs.length) % tabs.length];
    if (next) get().activateTab(next.id);
  },

  // WHAT:  Activating a tab whose connection is not live connects it first.
  // WHY:   Tabs are restored at startup before any session exists; clicking
  //        one should open it, not show "Not connected".
  activateTab: (id) => {
    const tab = get().tabs.find((t) => t.id === id);
    // Picking the tab in the side pane brings it to the main one.
    if (id === get().splitTabId) set((s) => ({ splitTabId: s.activeTabId !== id ? s.activeTabId : null }));
    set({ activeTabId: id, page: { kind: "workspace" } });
    if (tab && tab.connectionId !== null) {
      if (!get().sessions.includes(tab.connectionId)) void get().connect(tab.connectionId);
      else if (get().activeConnectionId !== tab.connectionId) set({ activeConnectionId: tab.connectionId });
    }
  },

  setDensity: (density) => {
    try {
      localStorage.setItem(DENSITY_KEY, density);
    } catch {
      // storage unavailable — keep in memory only
    }
    set({ density });
  },

  loadSavedQueries: async () => {
    const savedQueries = await ipc("list_saved_queries");
    set({ savedQueries });
  },
  saveQuery: async (query) => {
    const saved = await ipc("save_saved_query", { query });
    await get().loadSavedQueries();
    return saved;
  },
  deleteSavedQuery: async (id) => {
    await ipc("delete_saved_query", { id });
    set((s) => ({ savedQueries: s.savedQueries.filter((q) => q.id !== id) }));
  },

  loadDocuments: async (kind) => {
    const docs = await ipc("list_documents", { kind });
    set((s) => ({ documents: { ...s.documents, [kind]: docs } }));
  },
  saveDocument: async (doc) => {
    const saved = await ipc("save_document", { document: doc });
    await get().loadDocuments(doc.kind);
    return saved;
  },
  deleteDocument: async (kind, id) => {
    await ipc("delete_document", { id });
    set((s) => ({
      documents: { ...s.documents, [kind]: s.documents[kind].filter((d) => d.id !== id) },
      tabs: s.tabs.filter((t) => !(t.kind === "document" && t.documentId === id)),
    }));
  },

  // WHAT:  Review mode: an edit to a cell already staged (or a re-staged change
  //        with the same id) replaces the earlier one in place, keeping its position.
  stageChange: (connectionId, change) =>
    set((s) => {
      const current = s.pendingChanges[connectionId] ?? [];
      const replaces = (c: StagedChange) =>
        c.id === change.id ||
        (c.kind === "update" && change.kind === "update" && tableKey(c.table) === tableKey(change.table) && c.column === change.column && JSON.stringify(c.key) === JSON.stringify(change.key));
      const index = current.findIndex(replaces);
      const next = index >= 0 ? current.map((c, i) => (i === index ? change : c)) : [...current, change];
      return { pendingChanges: { ...s.pendingChanges, [connectionId]: next }, changesPanelOpen: true };
    }),
  unstageChange: (connectionId, changeId) =>
    set((s) => ({ pendingChanges: { ...s.pendingChanges, [connectionId]: (s.pendingChanges[connectionId] ?? []).filter((c) => c.id !== changeId) } })),
  clearChanges: (connectionId) => set((s) => ({ pendingChanges: withoutKey(s.pendingChanges, connectionId) })),
  setChangesPanelOpen: (open) => set({ changesPanelOpen: open }),

  // WHAT:  Notifications are Sonner toasts (stacked, bottom-right); errors stay
  //        longer since they often carry a SQL message worth reading.
  showError: (error) => {
    toast.error(typeof error === "string" ? error : errorMessage(error), { duration: 8000 });
  },
  showInfo: (text) => {
    toast.success(text);
  },
}));

export function useActiveConnection(): ConnectionSummary | null {
  return useWorkspace((s) => s.connections.find((c) => c.id === s.activeConnectionId) ?? null);
}

export function useActiveTab(): Tab | null {
  return useWorkspace((s) => s.tabs.find((t) => t.id === s.activeTabId) ?? null);
}

// WHAT:  The connection a tab belongs to, which is not always the active one.
// WHY:   The tab bar holds every open tab, and restoring query tabs at startup
//        puts back tabs from several connections at once. A tab has to run against
//        the database it was written for, not against whichever one is selected.
// WHERE: src/App.tsx (tab routing)
export function useTabConnection(tab: Tab | null): ConnectionSummary | null {
  const connectionId = tab === null ? null : tab.connectionId;
  return useWorkspace((s) => s.connections.find((c) => c.id === connectionId) ?? null);
}

export function usePendingCount(connectionId: string | null): number {
  return useWorkspace((s) => (connectionId ? (s.pendingChanges[connectionId]?.length ?? 0) : 0));
}

// WHAT:  Fills in the text of a closed query tab once its buffer has been read.
// HOW:   Matched by id, so a stack entry pushed synchronously on close gets
//        its seed a moment later.
function rememberClosed(tab: Tab): void {
  useWorkspace.setState((s) => ({ closedTabs: s.closedTabs.map((t) => (t.id === tab.id ? tab : t)) }));
}

// WHAT:  Writes the open tabs whenever the list or the active tab changes.
// WHY:   A restart puts the workspace back the way it was (tabPersistence.ts).
// HOW:   Only after bootstrap: writing the empty initial list would wipe the
//        stored one before it had been read.
useWorkspace.subscribe((state, previous) => {
  if (!state.ready) return;
  if (state.tabs === previous.tabs && state.activeTabId === previous.activeTabId) return;
  writeStoredTabs(state.tabs, state.activeTabId);
});
