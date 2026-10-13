// SOT: workspace-store, pane-layout-state, preview-tab, dirty-tabs, sidebar-layout-state, inspector-flag, cross-window-sync, pages, tabs, restored-query-tabs, reopen-closed-tab, tab-cycling, split-tab-state, sidebar-mode, active-connection, catalog-cache, foreign-key-cache, objects-cache, session-info-cache, settings-cache, pending-changes, saved-queries-cache, documents-cache, ui-toasts
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
import { useConnectionFailure } from "./connectionFailure";
import { readStoredTabs, startingLayout, storable, tabIdsOfOtherWindows, writeStoredTabs } from "./tabPersistence";
import { confirmLeavingTransaction, useTransactions } from "./transactions";
import {
  activateInLayout,
  activeOf,
  closePane as closePaneInLayout,
  focusPane as focusPaneInLayout,
  hasPane,
  initialPanes,
  moveToPane,
  normalize,
  paneIds,
  paneOf,
  placeTab,
  resizeSplit as resizeSplitInLayout,
  sideTabOf,
  splitWithTab,
  tabsInPane,
  type DropZone,
  type PaneLayout,
} from "@/lib/panes";
import {
  INSPECTOR_MAX,
  INSPECTOR_MIN,
  SIDEBAR_MAX,
  SIDEBAR_MIN,
  clampWidth,
  layoutPreset,
  readInspectorWidth,
  readSidebarCollapsed,
  readSidebarWidth,
  writeInspectorWidth,
  writeSidebarCollapsed,
  writeSidebarWidth,
  type LayoutPresetId,
} from "@/lib/layouts";
import { broadcastSync, isMainWindow, windowContext } from "@/lib/shellEvents";
import type { HomeEntity, HomeItemKind, HomePin } from "@/lib/bindings";
import { useLibrary } from "./library";

export type SidebarMode = "tables" | "objects" | "queries" | "dashboards" | "workflows" | "diagrams";

export type Page =
  | { kind: "connections" }
  | { kind: "connection-picker" }
  | { kind: "connection-form"; editingId: string | null; preset?: EnginePreset; draft?: ConnectionInput }
  | { kind: "workspace" }
  | { kind: "settings"; section?: SettingsSection }
  | { kind: "capabilities" }
  | { kind: "whats-new" };

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
  | { id: string; kind: "tool"; connectionId: string; tool: Tool }
  | { id: string; kind: "schema-compare"; connectionId: string; schema: string | null }
  | { id: string; kind: "data-compare"; connectionId: string; table: TableRef | null };

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
  loadCatalog: (id: string, force?: boolean) => Promise<void>;
  loadColumns: (id: string, table: TableRef) => Promise<ColumnInfo[]>;
  loadObjects: (id: string, kind: ObjectKind, parent: string | null, force?: boolean) => Promise<ObjectSummary[]>;
  /// Drops every cached listing for the connection (after an action changed the server).
  invalidateObjects: (id: string) => void;
  openTable: (connectionId: string, table: TableRef, filters?: FilterRule[]) => void;
  openQuery: (connectionId: string, seedSql?: string, title?: string) => void;
  openHistory: (connectionId: string) => void;
  openTransfer: (connectionId: string) => void;
  openErd: (connectionId: string, schema: string | null) => void;
  openChat: (connectionId: string, prompt?: string) => void;
  /// A prompt handed to the chat by openChat, consumed once by the chat tab.
  chatSeed: { connectionId: string; prompt: string } | null;
  takeChatSeed: (connectionId: string) => string | null;
  openObject: (connectionId: string, reference: ObjectRef) => void;
  openAdmin: (connectionId: string) => void;
  openTool: (connectionId: string, tool: Tool) => void;
  /// Schema compare with this connection (and schema) preselected as the left side.
  openSchemaCompare: (connectionId: string, schema: string | null) => void;
  /// Data compare with this connection (and table) preselected as the left side.
  openDataCompare: (connectionId: string, table: TableRef | null) => void;
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
  /// Items pinned to the Home board, in board order. Loaded at startup.
  pins: HomePin[];
  loadPins: () => Promise<void>;
  /// Pins an item to Home; pinning something already pinned changes nothing.
  pinItem: (entity: HomeEntity) => Promise<void>;
  unpinItem: (kind: HomeItemKind, itemId: string) => Promise<void>;
  isPinned: (kind: HomeItemKind, itemId: string) => boolean;
  /// Stores the board order: pin ids, first to last.
  reorderPins: (ids: string[]) => Promise<void>;
  /// Opens the table in a tab of its own even when it is already open (Shift+click,
  /// a table dropped on the split pane). Answers the new tab's id.
  openTableInNewTab: (connectionId: string, table: TableRef) => string;
  /// Forgets the cached columns of one table (or of the whole connection), so the next read asks the server.
  forgetColumns: (connectionId: string, table?: TableRef) => void;
  /// After a schema form ran: reloads the catalog and the caches, and closes the tabs of `removed` tables.
  schemaChanged: (connectionId: string, removed: readonly TableRef[]) => Promise<void>;
  /// Where every open tab sits: up to four panes in a split tree (src/lib/panes.ts). `tabs`
  /// stays the one flat list and `activeTabId` the focused pane's tab, so a feature that
  /// appends to `tabs` or reads `activeTabId` keeps working; the layout catches up on its own.
  panes: PaneLayout;
  /// The one tab shown as an italic preview; the next table opened from the sidebar replaces it.
  previewTabId: string | null;
  /// Makes a preview tab permanent (clicking its tab, editing it).
  promoteTab: (id: string) => void;
  /// Tabs with unsaved work, by id. A feature marks its tab here; closing it from the UI asks first.
  dirtyTabs: Record<string, boolean>;
  setTabDirty: (id: string, dirty: boolean) => void;
  /// Tabs waiting for the answer to "close with unsaved changes?" (the dialog in App.tsx).
  closeRequest: { ids: string[]; dirty: string[] } | null;
  /// Closing tabs from the UI (close button, ⌘W, the tab menu): asks first when any has unsaved
  /// work. `closeTab` itself never asks, so features can close a tab whose subject is gone.
  requestCloseTabs: (ids: string[]) => void;
  resolveCloseRequest: (close: boolean) => void;
  focusPane: (paneId: string) => void;
  /// Moves a tab into a new pane beside or below its own (needs another tab to stay behind).
  splitPane: (paneId: string, direction: "right" | "down", tabId: string) => void;
  closePane: (paneId: string) => void;
  /// A tab dropped on a pane: the middle moves it in, an edge splits the pane (max four).
  dropTab: (tabId: string, paneId: string, zone: DropZone) => void;
  /// A tab dropped on a tab strip: into that pane, just before `beforeId` (null: at the end).
  moveTab: (tabId: string, paneId: string, beforeId: string | null) => void;
  /// Drags a divider between two panes (`ratio` is the first pane's share, 0–1).
  resizeSplit: (splitId: string, ratio: number) => void;
  sidebarCollapsed: boolean;
  sidebarWidth: number;
  /// Sets the width; with a connection id it is also remembered for that connection.
  setSidebarWidth: (width: number, rememberFor?: string | null) => void;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  /// The record inspector, one flag for every grid: the grid reads these, ⌘L flips it.
  inspectorOpen: boolean;
  inspectorWidth: number;
  setInspectorOpen: (open: boolean) => void;
  toggleInspector: () => void;
  setInspectorWidth: (width: number) => void;
  applyLayout: (preset: LayoutPresetId) => void;
  aboutOpen: boolean;
  setAboutOpen: (open: boolean) => void;
  /// Reloads what other windows may have changed: connections, live sessions, settings.
  syncShared: (what: "settings" | "connections" | "sessions" | "all") => Promise<void>;
}

const DENSITY_KEY = "db-free.density";

function readDensity(): Density {
  try {
    const raw = localStorage.getItem(DENSITY_KEY);
    if (raw === "compact" || raw === "cozy" || raw === "comfortable") return raw;
  } catch {
    // storage unavailable — fall through to default
  }
  return "compact";
}

// WHAT:  The density Settings → General → "Density" stores, as a Density.
// WHY:   The setting is a free string in the settings document; any other text
//        reads as the compact default rather than failing.
function densityOf(setting: string): Density {
  return setting === "cozy" || setting === "comfortable" ? setting : "compact";
}

function withoutKey<T>(record: Record<string, T>, key: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => k !== key));
}

function withoutPrefix<T>(record: Record<string, T>, prefix: string): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([k]) => !k.startsWith(prefix)));
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

// ---------------------------------------------------------------- panes
// The layout (src/lib/panes.ts) says which pane each tab is in; `tabs` stays the
// flat list. These helpers keep `panes`, `activeTabId` and `splitTabId` in step.

/// `panes`, `activeTabId` and `splitTabId` for a tab list and a layout, repaired and consistent.
function paneState(tabs: readonly Tab[], panes: PaneLayout, requestedActive: string | null): Pick<WorkspaceState, "panes" | "activeTabId" | "splitTabId"> {
  const fixed = normalize(panes, tabs.map((t) => t.id), requestedActive);
  return { panes: fixed, activeTabId: activeOf(fixed), splitTabId: sideTabOf(fixed) };
}

// WHAT:  The tab list with `tab` added: next to the last tab of its own connection
//        in the focused pane, else at the end. Nothing already open moves.
// WHY:   Tabs of several connections share one strip; keeping each connection's
//        tabs together is half of telling them apart.
function withTab(s: WorkspaceState, tab: Tab): Tab[] {
  let last = -1;
  if (tab.connectionId !== null) {
    s.tabs.forEach((t, index) => {
      if (t.connectionId === tab.connectionId && paneOf(s.panes, t.id) === s.panes.focused) last = index;
    });
  }
  const at = last < 0 ? s.tabs.length : last + 1;
  return [...s.tabs.slice(0, at), tab, ...s.tabs.slice(at)];
}

// WHAT:  A tab brought to the front: added when new (into the focused pane),
//        then made its pane's active tab with that pane focused.
function showTab(s: WorkspaceState, tab: Tab, replace = false): Partial<WorkspaceState> {
  const exists = s.tabs.some((t) => t.id === tab.id);
  const tabs = exists ? (replace ? s.tabs.map((t) => (t.id === tab.id ? tab : t)) : s.tabs) : withTab(s, tab);
  return { tabs, ...paneState(tabs, activateInLayout(s.panes, tab.id), tab.id), page: { kind: "workspace" } };
}

// WHAT:  The layout once `leaving` tabs go: every pane whose active tab is
//        among them hands over to its neighbour (left first, else right).
function handOffActive(layout: PaneLayout, tabs: readonly Tab[], leaving: ReadonlySet<string>): PaneLayout {
  const active: Record<string, string | null> = { ...layout.active };
  let changed = false;
  for (const pane of paneIds(layout.root)) {
    const mine = tabsInPane(layout, tabs, pane).map((t) => t.id);
    const current = active[pane] ?? null;
    if (current === null || !leaving.has(current)) continue;
    const at = mine.indexOf(current);
    active[pane] =
      mine
        .slice(0, at)
        .reverse()
        .find((id) => !leaving.has(id)) ??
      mine.slice(at + 1).find((id) => !leaving.has(id)) ??
      null;
    changed = true;
  }
  return changed ? { ...layout, active } : layout;
}

function afterRemoval(s: WorkspaceState, gone: ReadonlySet<string>): Partial<WorkspaceState> {
  const tabs = s.tabs.filter((t) => !gone.has(t.id));
  return {
    tabs,
    ...paneState(tabs, handOffActive(s.panes, s.tabs, gone), null),
    previewTabId: s.previewTabId !== null && gone.has(s.previewTabId) ? null : s.previewTabId,
    dirtyTabs: Object.fromEntries(Object.entries(s.dirtyTabs).filter(([id]) => !gone.has(id))),
  };
}

// WHAT:  Closes tabs for good, asking nothing about unsaved work (that is
//        `requestCloseTabs`'s job). The one place tabs are removed.
// WHY:   Close, close others, close to the right, close all and a schema change
//        that drops a table all end here, so the layout, the reopen stack and
//        the query buffers are handled the same way for every one of them.
function removeTabs(ids: readonly string[]): void {
  const state = useWorkspace.getState();
  const gone = new Set(ids);
  const closed = state.tabs.filter((t) => gone.has(t.id));
  if (closed.length === 0) return;
  // The last query tab on a connection is the last Commit button for its transaction.
  for (const connectionId of new Set(closed.flatMap((t) => (t.kind === "query" ? [t.connectionId] : [])))) {
    const stays = state.tabs.some((o) => !gone.has(o.id) && o.kind === "query" && o.connectionId === connectionId);
    if (!stays && !confirmLeavingTransaction(connectionId, "Closing its last query tab leaves it open until you disconnect.")) return;
  }
  forgetBuffers(closed, rememberClosed);
  useWorkspace.setState((s) => ({ ...afterRemoval(s, gone), closedTabs: pushClosed(s.closedTabs, closed) }));
}

// WHAT:  Moves a tab into `paneId`, just before `beforeId` (null: at the end).
//        Inside its own pane this is a reorder and the active tab is left alone.
function moveTabPatch(s: WorkspaceState, tabId: string, paneId: string, beforeId: string | null): Partial<WorkspaceState> {
  if (!s.tabs.some((t) => t.id === tabId) || !hasPane(s.panes, paneId)) return {};
  const tabs = placeTab(s.tabs, s.panes, tabId, paneId, beforeId);
  if (paneOf(s.panes, tabId) === paneId) return { tabs };
  const handed = handOffActive(s.panes, s.tabs, new Set([tabId]));
  return { tabs, ...paneState(tabs, moveToPane(handed, tabId, paneId), null) };
}

/// A second open of the same table within this long (ms) counts as a double-click.
const DOUBLE_CLICK_MS = 500;
let lastTableOpen: { id: string; at: number } = { id: "", at: 0 };

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
      const here = windowContext();
      // Every other kind comes back from the stored tab list, in the order it
      // was left in; query tabs only when their buffer still has text.
      const stored = readStoredTabs();
      // A buffer is restored by the window that owns its tab: the main window
      // takes every buffer no other window claimed, a connection window only
      // its own, and a plain extra window none (it starts on the home page).
      const claimed = isMainWindow() ? tabIdsOfOtherWindows() : new Set<string>();
      const owned = new Set(stored.tabs.map((t) => t.id));
      const fromBuffers: Tab[] = buffers.flatMap((b) => {
        const connectionId = b.connectionId;
        if (connectionId === null || !known.has(connectionId) || b.content.trim().length === 0) return [];
        if (isMainWindow() ? claimed.has(b.id) : !owned.has(b.id)) return [];
        return [{ id: b.id, kind: "query" as const, connectionId, title: b.title.length > 0 ? b.title : "Query" }];
      });
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
      set({ connections, sessions, settings, savedQueries, ready: true, tabs: restored, ...paneState(restored, startingLayout(stored), active) });
      // A connection window opens on its connection (connecting it if it is not live yet);
      // the main window on the first live session; an extra window on the home page.
      const target = here.connectionId !== null && known.has(here.connectionId) ? here.connectionId : null;
      const live = target ?? (isMainWindow() ? sessions[0] : undefined);
      if (live !== undefined) {
        if (target !== null && !sessions.includes(target)) {
          await get().connect(target);
        } else {
          set({ activeConnectionId: live, page: { kind: "workspace" } });
          await get().loadCatalog(live);
        }
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
  // Picking a sidebar from the rail also brings back a collapsed sidebar.
  setSidebar: (mode) => {
    if (get().sidebarCollapsed) get().setSidebarCollapsed(false);
    set({ sidebar: mode, page: { kind: "workspace" } });
  },
  setPaletteOpen: (open) => set({ paletteOpen: open }),

  loadSettings: async () => {
    const settings = await ipc("get_settings");
    set({ settings, density: densityOf(settings.gridDensity) });
  },
  saveSettings: async (settings, aiApiKey) => {
    const saved = await ipc("save_settings", { settings, aiApiKey });
    set({ settings: saved, density: densityOf(saved.gridDensity) });
    // Other windows hold their own copy; tell them to reload it.
    void broadcastSync("settings");
  },

  saveConnection: async (id, input) => {
    const saved = await ipc("save_connection", { id, input });
    await get().refreshConnections();
    void broadcastSync("connections");
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
      void broadcastSync("connections");
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
      ...afterRemoval(s, new Set(s.tabs.filter((t) => t.connectionId === id).map((t) => t.id))),
      activeConnectionId: s.activeConnectionId === id ? null : s.activeConnectionId,
      pendingChanges: withoutKey(s.pendingChanges, id),
    }));
    void broadcastSync("connections");
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
      void broadcastSync("sessions");
      return true;
    } catch (raw) {
      const error = normalizeError(raw);
      // On the connections page a failed connect gets the checklist dialog (it stays
      // until read, with Retry / Edit); elsewhere it is a toast, as before.
      if (get().page.kind === "connections") {
        useConnectionFailure.getState().report({ connectionId: id, name: get().connections.find((c) => c.id === id)?.name ?? "this connection", message: errorMessage(error) });
      } else {
        get().showError(error);
      }
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
      ...afterRemoval(s, new Set(s.tabs.filter((t) => t.connectionId === id).map((t) => t.id))),
    }));
    void broadcastSync("sessions");
  },

  // WHAT:  Opens a connection: in this window, or — when "open connections in a new
  //        window" is on — in the window that belongs to it (created or focused).
  // WHY:   One window per connection keeps tabs, panes and the sidebar of two
  //        databases from mixing. Choosing the connection this window is already
  //        showing stays local, or the setting would make it impossible to stay.
  selectConnection: (id) => {
    const s = get();
    const showingIt = s.page.kind === "workspace" && s.activeConnectionId === id;
    if (s.settings?.openConnectionsInNewWindow === true && windowContext().connectionId !== id && !showingIt) {
      void (async () => {
        try {
          await ipc("open_window", { connectionId: id });
        } catch (raw) {
          get().showError(normalizeError(raw));
        }
      })();
      return;
    }
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
        ...afterRemoval(s, new Set(s.tabs.filter((t) => t.connectionId === id && (t.kind === "table" || t.kind === "erd" || t.kind === "object")).map((t) => t.id))),
        columnsCache: {},
        objectsCache: withoutPrefix(s.objectsCache, `${id}:`),
        schemaFilter: { ...s.schemaFilter, [id]: null },
        pendingChanges: withoutKey(s.pendingChanges, id),
      }));
    }
  },

  setSchemaFilter: (id, schema) => set((s) => ({ schemaFilter: { ...s.schemaFilter, [id]: schema } })),

  // WHAT:  `force` skips the persisted schema cache and asks the database (the sidebar's Refresh).
  loadCatalog: async (id, force = false) => {
    try {
      const [catalog, info, fks] = await Promise.all([
        ipc("load_catalog", { connectionId: id, force }),
        ipc("describe_session", { connectionId: id }),
        ipc("load_foreign_keys", { connectionId: id, force }).catch((): ForeignKey[] => []),
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
  // HOW:   With "reuse table preview tab" on, a plain open shows the table in one
  //        italic preview tab that the next table replaces in place. Opening the
  //        same preview table again within a double-click's time keeps it, which
  //        is what a double-click in the sidebar does.
  openTable: (connectionId, table, filters) => {
    const id = `table:${connectionId}:${tableKey(table)}`;
    const reuse = filters === undefined && get().settings?.reuseTablePreviewTab === true;
    const now = Date.now();
    const doubleClick = lastTableOpen.id === id && now - lastTableOpen.at < DOUBLE_CLICK_MS;
    lastTableOpen = { id, at: now };
    set((s) => {
      const existing = s.tabs.find((t) => t.id === id);
      const tab: Tab = { id, kind: "table", connectionId, table, filterKey: filters ? now : (existing?.kind === "table" ? existing.filterKey : 0), ...(filters ? { initialFilters: filters } : {}) };
      if (existing) {
        const keep = s.previewTabId === id && reuse && !doubleClick;
        return { ...showTab(s, tab, true), previewTabId: keep ? id : s.previewTabId === id ? null : s.previewTabId };
      }
      const preview = reuse && s.previewTabId !== null ? s.tabs.find((t) => t.id === s.previewTabId) : undefined;
      if (preview === undefined) return { ...showTab(s, tab), previewTabId: reuse ? id : s.previewTabId };
      // The new preview takes the old one's place, in its pane and at its position.
      const tabs = s.tabs.map((t) => (t.id === preview.id ? tab : t));
      const pane = paneOf(s.panes, preview.id);
      const owner = { ...withoutKey(s.panes.owner, preview.id), [id]: pane };
      const active = { ...s.panes.active, [pane]: s.panes.active[pane] === preview.id ? id : (s.panes.active[pane] ?? null) };
      return { tabs, ...paneState(tabs, activateInLayout({ ...s.panes, owner, active }, id), id), page: { kind: "workspace" }, previewTabId: id };
    });
  },

  promoteTab: (id) => set((s) => (s.previewTabId === id ? { previewTabId: null } : {})),

  openQuery: (connectionId, seedSql, title) => {
    queryCounter += 1;
    const id = `query:${connectionId}:${Date.now()}-${queryCounter}`;
    const tab: Tab = { id, kind: "query", connectionId, title: title ?? `Query ${queryCounter}`, ...(seedSql !== undefined ? { seedSql } : {}) };
    set((s) => showTab(s, tab));
  },

  openHistory: (connectionId) => {
    const id = `history:${connectionId}`;
    set((s) => showTab(s, { id, kind: "history", connectionId }));
  },

  openTransfer: (connectionId) => {
    const id = `transfer:${connectionId}`;
    set((s) => showTab(s, { id, kind: "transfer", connectionId }));
  },

  openErd: (connectionId, schema) => {
    const id = `erd:${connectionId}:${schema ?? "*"}`;
    set((s) => showTab(s, { id, kind: "erd", connectionId, schema }));
  },

  openChat: (connectionId, prompt) => {
    const id = `chat:${connectionId}`;
    set((s) => ({
      ...showTab(s, { id, kind: "chat", connectionId }),
      ...(prompt !== undefined && prompt.trim() !== "" ? { chatSeed: { connectionId, prompt } } : {}),
    }));
  },
  chatSeed: null,
  takeChatSeed: (connectionId) => {
    const seed = get().chatSeed;
    if (seed?.connectionId !== connectionId) return null;
    set({ chatSeed: null });
    return seed.prompt;
  },

  invalidateObjects: (id) => set((s) => ({ objectsCache: withoutPrefix(s.objectsCache, `${id}:`) })),

  openObject: (connectionId, reference) => {
    const id = `object:${connectionId}:${objectKey(reference)}`;
    set((s) => showTab(s, { id, kind: "object", connectionId, reference }));
  },

  openAdmin: (connectionId) => {
    const id = `admin:${connectionId}`;
    set((s) => showTab(s, { id, kind: "admin", connectionId }));
  },

  openTool: (connectionId, tool) => {
    const id = `tool:${connectionId}:${tool}`;
    set((s) => showTab(s, { id, kind: "tool", connectionId, tool }));
  },

  openSchemaCompare: (connectionId, schema) => {
    const id = `schema-compare:${connectionId}:${schema ?? "*"}`;
    set((s) => showTab(s, { id, kind: "schema-compare", connectionId, schema }));
  },

  openDataCompare: (connectionId, table) => {
    const id = `data-compare:${connectionId}:${table ? tableKey(table) : "*"}`;
    set((s) => showTab(s, { id, kind: "data-compare", connectionId, table }));
  },

  openDocument: (kind, documentId, connectionId) => {
    const id = `doc:${kind}:${documentId}`;
    set((s) => showTab(s, { id, kind: "document", connectionId, documentKind: kind, documentId }));
  },

  closeTab: (id) => removeTabs([id]),

  closeOtherTabs: (id) =>
    removeTabs(
      get()
        .tabs.filter((t) => t.id !== id)
        .map((t) => t.id),
    ),

  closeTabsToRight: (id) => {
    const at = get().tabs.findIndex((t) => t.id === id);
    if (at < 0) return;
    removeTabs(
      get()
        .tabs.slice(at + 1)
        .map((t) => t.id),
    );
  },

  closeAllTabs: () => removeTabs(get().tabs.map((t) => t.id)),

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
    set((s) => showTab(s, tab));
    get().activateTab(tab.id);
  },

  // WHAT:  "Open to the side": the tab moves to the pane beside its own, or into a
  //        new pane on the right when it is the only one.
  openToSide: (id) => {
    const { tabs, panes } = get();
    if (!tabs.some((t) => t.id === id)) return;
    const from = paneOf(panes, id);
    const order = paneIds(panes.root);
    const at = order.indexOf(from);
    const neighbour = order[at + 1] ?? order[at - 1];
    if (neighbour !== undefined) get().moveTab(id, neighbour, null);
    else get().splitPane(from, "right", id);
  },

  // WHAT:  The old split view's two buttons, now on the pane tree: close the
  //        side pane (its tabs stay, in the pane next to it), swap the two.
  closeSplit: () =>
    set((s) => {
      const side = paneIds(s.panes.root).find((p) => p !== s.panes.focused);
      return side === undefined ? {} : paneState(s.tabs, closePaneInLayout(s.panes, side), null);
    }),

  swapSplit: () =>
    set((s) => {
      const focused = s.panes.focused;
      const side = paneIds(s.panes.root).find((p) => p !== focused);
      const mine = s.panes.active[focused] ?? null;
      const theirs = side === undefined ? null : (s.panes.active[side] ?? null);
      if (side === undefined || mine === null || theirs === null) return {};
      const panes: PaneLayout = { ...s.panes, owner: { ...s.panes.owner, [mine]: side, [theirs]: focused }, active: { ...s.panes.active, [focused]: theirs, [side]: mine } };
      return paneState(s.tabs, panes, null);
    }),

  focusPane: (paneId) => {
    set((s) => paneState(s.tabs, focusPaneInLayout(s.panes, paneId), null));
    const tab = get().tabs.find((t) => t.id === get().activeTabId);
    if (tab?.connectionId && tab.connectionId !== get().activeConnectionId && get().sessions.includes(tab.connectionId)) set({ activeConnectionId: tab.connectionId });
  },

  splitPane: (paneId, direction, tabId) =>
    set((s) => {
      const inPane = tabsInPane(s.panes, s.tabs, paneId);
      // The source pane must keep a tab, or the split would only move the pane.
      if (inPane.length < 2 || !inPane.some((t) => t.id === tabId)) return {};
      const handed = handOffActive(s.panes, s.tabs, new Set([tabId]));
      const result = splitWithTab(handed, paneId, direction === "right" ? "right" : "bottom", tabId);
      return result === null ? {} : paneState(s.tabs, result.layout, null);
    }),

  closePane: (paneId) => set((s) => paneState(s.tabs, closePaneInLayout(s.panes, paneId), null)),

  dropTab: (tabId, paneId, zone) =>
    set((s) => {
      if (!s.tabs.some((t) => t.id === tabId) || !hasPane(s.panes, paneId)) return {};
      if (zone === "center") return moveTabPatch(s, tabId, paneId, null);
      if (paneOf(s.panes, tabId) === paneId && tabsInPane(s.panes, s.tabs, paneId).length < 2) return {};
      const result = splitWithTab(handOffActive(s.panes, s.tabs, new Set([tabId])), paneId, zone, tabId);
      return result === null ? {} : paneState(s.tabs, result.layout, null);
    }),

  moveTab: (tabId, paneId, beforeId) => set((s) => moveTabPatch(s, tabId, paneId, beforeId)),

  resizeSplit: (splitId, ratio) => set((s) => ({ panes: resizeSplitInLayout(s.panes, splitId, ratio) })),

  dirtyTabs: {},
  setTabDirty: (id, dirty) =>
    set((s) => {
      if ((s.dirtyTabs[id] === true) === dirty) return {};
      // Editing a preview tab is the user committing to it.
      return { dirtyTabs: dirty ? { ...s.dirtyTabs, [id]: true } : withoutKey(s.dirtyTabs, id), ...(dirty && s.previewTabId === id ? { previewTabId: null } : {}) };
    }),

  closeRequest: null,
  requestCloseTabs: (ids) => {
    const dirty = ids.filter((id) => get().dirtyTabs[id] === true);
    if (dirty.length === 0) removeTabs(ids);
    else set({ closeRequest: { ids, dirty } });
  },
  resolveCloseRequest: (close) => {
    const request = get().closeRequest;
    set({ closeRequest: null });
    if (close && request) removeTabs(request.ids);
  },

  // Ctrl+Tab / Ctrl+Shift+Tab walk the tabs of the pane that has the keyboard.
  cycleTab: (step) => {
    const { tabs, panes } = get();
    const mine = tabsInPane(panes, tabs, panes.focused);
    if (mine.length === 0) return;
    const at = mine.findIndex((t) => t.id === panes.active[panes.focused]);
    const next = mine[(at + step + mine.length) % mine.length];
    if (next) get().activateTab(next.id);
  },

  // WHAT:  Activating a tab whose connection is not live connects it first.
  // WHY:   Tabs are restored at startup before any session exists; clicking
  //        one should open it, not show "Not connected".
  activateTab: (id) => {
    const tab = get().tabs.find((t) => t.id === id);
    set((s) => ({ ...paneState(s.tabs, activateInLayout(s.panes, id), id), page: { kind: "workspace" } }));
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
  // A new query is filed in the folder picked in the save dialog (SaveFolderField), sent
  // with the write so the server checks the name against that folder, not the top level.
  saveQuery: async (query) => {
    const folderId = query.id === "" ? useLibrary.getState().saveFolder : null;
    const saved = await ipc("save_saved_query", { query, ...(folderId !== null ? { folderId } : {}) });
    await get().loadSavedQueries();
    if (folderId !== null) await useLibrary.getState().loadLibrary();
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
  pins: [],
  // Pins are a convenience: a failed read leaves the board as it was instead of interrupting startup.
  loadPins: async () => {
    try {
      set({ pins: await ipc("list_pins") });
    } catch {
      // keep the pins already held
    }
  },
  pinItem: async (entity) => {
    try {
      await ipc("pin_item", { entity });
      await get().loadPins();
    } catch (raw) {
      get().showError(normalizeError(raw));
    }
  },
  unpinItem: async (kind, itemId) => {
    try {
      await ipc("unpin_item", { kind, itemId });
      set((s) => ({ pins: s.pins.filter((p) => !(p.entity.kind === kind && p.entity.itemId === itemId)) }));
    } catch (raw) {
      get().showError(normalizeError(raw));
    }
  },
  isPinned: (kind, itemId) => get().pins.some((p) => p.entity.kind === kind && p.entity.itemId === itemId),
  reorderPins: async (ids) => {
    // Show the new order at once; the server's answer replaces it if the write fails.
    set((s) => ({ pins: [...s.pins].sort((a, b) => ids.indexOf(a.id) - ids.indexOf(b.id)) }));
    try {
      await ipc("reorder_pins", { ids });
    } catch (raw) {
      get().showError(normalizeError(raw));
      await get().loadPins();
    }
  },
  openTableInNewTab: (connectionId, table) => {
    const id = `table:${connectionId}:${tableKey(table)}#${crypto.randomUUID().slice(0, 8)}`;
    const tab: Tab = { id, kind: "table", connectionId, table, filterKey: 0 };
    set((s) => ({ tabs: [...s.tabs, tab], activeTabId: id, page: { kind: "workspace" } }));
    return id;
  },

  forgetColumns: (connectionId, table) =>
    set((s) => ({ columnsCache: table ? withoutKey(s.columnsCache, `${connectionId}:${tableKey(table)}`) : withoutPrefix(s.columnsCache, `${connectionId}:`) })),

  // WHAT:  Brings every cache a schema change can have made stale back in line
  //        with the server, and closes what showed a table that is gone.
  // WHY:   A drop, rename or ALTER leaves open table tabs, cached columns, the
  //        object listings and the catalog describing a table that no longer
  //        matches. Doing it here once means no form has to remember the list.
  // HOW:   Tabs close through `closeTab` (so the active-tab fallback and the
  //        split pane stay consistent) and are then dropped from the reopen
  //        stack, since reopening a dropped table's tab would show an error.
  schemaChanged: async (connectionId, removed) => {
    const gone = new Set(removed.map(tableKey));
    const showsRemoved = (tab: Tab): boolean =>
      tab.connectionId === connectionId && ((tab.kind === "table" && gone.has(tableKey(tab.table))) || (tab.kind === "data-compare" && tab.table !== null && gone.has(tableKey(tab.table))));
    for (const tab of get().tabs.filter(showsRemoved)) get().closeTab(tab.id);
    set((s) => ({
      closedTabs: s.closedTabs.filter((t) => !showsRemoved(t)),
      columnsCache: withoutPrefix(s.columnsCache, `${connectionId}:`),
      objectsCache: withoutPrefix(s.objectsCache, `${connectionId}:`),
    }));
    // Past the persisted schema cache: the server is what changed.
    await get().loadCatalog(connectionId, true);
  },
  panes: initialPanes(),
  previewTabId: null,
  sidebarCollapsed: readSidebarCollapsed(),
  sidebarWidth: readSidebarWidth(null),
  inspectorOpen: false,
  inspectorWidth: readInspectorWidth(),
  aboutOpen: false,
  setSidebarWidth: (width, rememberFor) => {
    const clamped = clampWidth(width, SIDEBAR_MIN, SIDEBAR_MAX);
    if (rememberFor !== undefined) writeSidebarWidth(rememberFor, clamped);
    set({ sidebarWidth: clamped });
  },
  setSidebarCollapsed: (collapsed) => {
    writeSidebarCollapsed(collapsed);
    set({ sidebarCollapsed: collapsed });
  },
  toggleSidebar: () => get().setSidebarCollapsed(!get().sidebarCollapsed),
  setInspectorOpen: (open) => set({ inspectorOpen: open }),
  toggleInspector: () => set((s) => ({ inspectorOpen: !s.inspectorOpen })),
  setInspectorWidth: (width) => {
    const clamped = clampWidth(width, INSPECTOR_MIN, INSPECTOR_MAX);
    writeInspectorWidth(clamped);
    set({ inspectorWidth: clamped });
  },
  // WHAT:  Applies a layout preset: sidebar shown or hidden and how wide (kept for
  //        the active connection), inspector open or closed and how wide.
  applyLayout: (preset) => {
    const found = layoutPreset(preset);
    if (found === undefined) return;
    get().setSidebarCollapsed(found.layout.sidebarCollapsed);
    get().setSidebarWidth(found.layout.sidebarWidth, get().activeConnectionId);
    get().setInspectorWidth(found.layout.inspectorWidth);
    set({ inspectorOpen: found.layout.inspectorOpen });
    const connectionId = get().activeConnectionId;
    if (found.opensChat && connectionId !== null && get().sessions.includes(connectionId)) get().openChat(connectionId);
  },
  setAboutOpen: (open) => set({ aboutOpen: open }),
  // WHAT:  Catches this window up with what another window changed.
  // WHY:   Windows share the backend (connections, sessions, settings) but each
  //        keeps its own copy in memory; a broadcast or a window focus calls this.
  syncShared: async (what) => {
    try {
      if (what === "settings" || what === "all") await get().loadSettings();
      if (what === "connections" || what === "all") await get().refreshConnections();
      if (what === "sessions" || what === "all") {
        const sessions = await ipc("active_sessions");
        const live = new Set(sessions);
        set((s) => {
          const dropped = s.sessions.filter((id) => !live.has(id));
          return {
            sessions,
            catalogs: dropped.reduce((acc, id) => withoutKey(acc, id), s.catalogs),
            sessionInfos: dropped.reduce((acc, id) => withoutKey(acc, id), s.sessionInfos),
            objectsCache: dropped.reduce((acc, id) => withoutPrefix(acc, `${id}:`), s.objectsCache),
          };
        });
        const active = get().activeConnectionId;
        if (active !== null && live.has(active) && !(active in get().catalogs)) await get().loadCatalog(active);
      }
    } catch {
      // Another window is mid-change; the next focus or broadcast tries again.
    }
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

// WHAT:  Keeps the pane layout, the preview tab and the unsaved-work flags in
//        step with `tabs`, whoever changed it, then writes the tabs to storage.
// WHY:   Features append to `tabs`, filter it and set `activeTabId` directly —
//        that is the contract they were given. The layout (src/lib/panes.ts) is
//        repaired here, in one place, so none of them has to know panes exist.
// HOW:   `normalize` returns the same object when nothing needs repair, so this
//        settles after one extra write. An `activeTabId` somebody just set
//        focuses the pane that tab is in.
useWorkspace.subscribe((state, previous) => {
  const requested = state.activeTabId !== previous.activeTabId ? state.activeTabId : null;
  const fixed = paneState(state.tabs, state.panes, requested);
  const ids = new Set(state.tabs.map((t) => t.id));
  const previewGone = state.previewTabId !== null && !ids.has(state.previewTabId);
  const dirtyGone = Object.keys(state.dirtyTabs).some((id) => !ids.has(id));
  if (fixed.panes !== state.panes || fixed.activeTabId !== state.activeTabId || fixed.splitTabId !== state.splitTabId || previewGone || dirtyGone) {
    useWorkspace.setState({
      ...fixed,
      ...(previewGone ? { previewTabId: null } : {}),
      ...(dirtyGone ? { dirtyTabs: Object.fromEntries(Object.entries(state.dirtyTabs).filter(([id]) => ids.has(id))) } : {}),
    });
    return;
  }
  // Written only after bootstrap: the empty initial list would wipe the stored
  // one before it had been read (tabPersistence.ts).
  if (!state.ready) return;
  if (state.tabs === previous.tabs && state.activeTabId === previous.activeTabId && state.panes === previous.panes) return;
  writeStoredTabs(state.tabs, state.activeTabId, state.panes);
});
