// SOT: recents-store, recents-recorder, home-runtime, library-startup-load
import { create } from "zustand";
import type { DocumentKind, HomeEntity, HomeRecent } from "@/lib/bindings";
import { connectionEntity, documentEntity, entityKey, tableEntity } from "@/lib/home";
import { ipc } from "@/lib/ipc";
import { useLibrary } from "./library";
import { useWorkspace, type Tab } from "./workspace";

// WHAT:  The "recently opened" list on Home, and the recorder that fills it.
// WHY:   Opening things is spread over the whole app (sidebar, palette, tab bar,
//        AI chat); hooking each call site would miss some and break whenever a new
//        one appears. Watching the workspace store sees every one of them.
// HOW:   The recorder subscribes to the workspace store. A newly activated table or
//        document tab, or a connection becoming the active one, is a recent. The
//        first change after startup (tabs restored from last time) and activation
//        caused by closing the previous tab are not: neither is the user opening
//        anything. Saved queries have no tab of their own, so the places that open
//        one call `record` themselves.
// WHERE: src-tauri/src/store/home.rs (the list), src/features/home/HomeBoard.tsx

const DOCUMENT_KINDS: readonly DocumentKind[] = ["dashboard", "workflow", "diagram"];

/// Same cap as the server's list, so the optimistic copy never shows more than a reload would.
const RECENT_LIMIT = 50;

interface RecentsState {
  recents: HomeRecent[];
  loaded: boolean;
  load: () => Promise<void>;
  /// Puts an item at the front of the list now and tells the server.
  record: (entity: HomeEntity) => void;
  clear: () => Promise<void>;
}

export const useRecents = create<RecentsState>((set, get) => ({
  recents: [],
  loaded: false,

  load: async () => {
    const recents = await ipc("list_recents");
    set({ recents, loaded: true });
  },

  record: (entity) => {
    const key = entityKey(entity);
    const current = get().recents;
    // Already first with the same label: nothing to change, nothing to send.
    const first = current[0];
    if (first !== undefined && entityKey(first.entity) === key && first.entity.label === entity.label) return;
    const entry: HomeRecent = { id: key, entity, openedAt: new Date().toISOString() };
    set({ recents: [entry, ...current.filter((r) => entityKey(r.entity) !== key)].slice(0, RECENT_LIMIT) });
    void ipc("record_recent", { entity }).catch(() => {
      // Recents are a convenience; a failed write is not worth interrupting anyone.
    });
  },

  clear: async () => {
    await ipc("clear_recents");
    set({ recents: [] });
  },
}));

// WHAT:  The entity a freshly activated tab stands for, if it stands for one.
function tabEntity(tab: Tab): HomeEntity | null {
  if (tab.kind === "table") return tableEntity(tab.connectionId, tab.table);
  return null;
}

let started = false;

// WHAT:  Loads the library, pins and recents once the workspace is ready, and keeps
//        recording from then on. Called once, when this module loads.
function start() {
  if (started) return;
  started = true;
  useWorkspace.subscribe((state, prev) => {
    if (!state.ready) return;
    if (!prev.ready) {
      void Promise.all([state.loadPins(), useRecents.getState().load(), useLibrary.getState().loadLibrary()]).catch(() => {
        // Home and the sidebars load again when opened; nothing to tell anyone yet.
      });
      return;
    }

    // A connection became the one being worked in.
    if (state.activeConnectionId !== null && state.activeConnectionId !== prev.activeConnectionId) {
      const connection = state.connections.find((c) => c.id === state.activeConnectionId);
      if (connection !== undefined && state.sessions.includes(connection.id)) useRecents.getState().record(connectionEntity(connection));
    }

    // A tab became the active one — but not because the previous active tab closed.
    if (state.activeTabId !== null && state.activeTabId !== prev.activeTabId) {
      const closedPrevious = prev.activeTabId !== null && !state.tabs.some((t) => t.id === prev.activeTabId);
      const tab = state.tabs.find((t) => t.id === state.activeTabId);
      if (!closedPrevious && tab !== undefined) {
        const entity = tabEntity(tab);
        if (entity !== null) {
          useRecents.getState().record(entity);
        } else if (tab.kind === "document") {
          void recordDocument(tab.documentKind, tab.documentId, tab.connectionId);
        }
      }
    }

    // The palette is about to list documents; make sure every kind is loaded.
    if (state.paletteOpen && !prev.paletteOpen) {
      for (const kind of DOCUMENT_KINDS) {
        if (state.documents[kind].length === 0) void state.loadDocuments(kind).catch(() => undefined);
      }
    }
  });
}

// WHAT:  Records an opened document under its current name, loading the kind's list
//        first when the document was opened before the sidebar ever loaded it.
async function recordDocument(kind: DocumentKind, documentId: string, tabConnectionId: string | null) {
  let state = useWorkspace.getState();
  if (!state.documents[kind].some((d) => d.id === documentId)) {
    try {
      await state.loadDocuments(kind);
    } catch {
      return;
    }
    state = useWorkspace.getState();
  }
  const doc = state.documents[kind].find((d) => d.id === documentId);
  if (doc === undefined) return;
  const entity = documentEntity(doc, tabConnectionId ?? state.activeConnectionId);
  if (entity !== null) useRecents.getState().record(entity);
}

start();
