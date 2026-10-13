// SOT: connection-org-store, connection-tags-cache, connection-folders-cache, folder-chip-filter, connection-membership-actions
import { create } from "zustand";
import type { ConnectionFolder, ConnectionSummary, ConnectionTag, ConnectionTagColor } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";

const FILTER_KEY = "db-free:connection-folder-filter";

function loadFilter(): string | null {
  try {
    const stored = localStorage.getItem(FILTER_KEY);
    return stored !== null && stored.length > 0 ? stored : null;
  } catch {
    return null;
  }
}

function saveFilter(id: string | null) {
  try {
    if (id === null) localStorage.removeItem(FILTER_KEY);
    else localStorage.setItem(FILTER_KEY, id);
  } catch {
    // Storage unavailable: the chip filter is a convenience only.
  }
}

interface ConnectionOrgState {
  tags: ConnectionTag[];
  folders: ConnectionFolder[];
  loaded: boolean;
  /// The folder chip that is on: a folder id, or null for "All".
  folderFilter: string | null;

  load: () => Promise<void>;
  setFolderFilter: (id: string | null) => void;
  /// Create (id null) or rename / recolour a tag. Throws the IpcError so a form can show it inline.
  saveTag: (id: string | null, name: string, color: ConnectionTagColor) => Promise<ConnectionTag>;
  deleteTag: (id: string) => Promise<void>;
  saveFolder: (id: string | null, name: string) => Promise<ConnectionFolder>;
  deleteFolder: (id: string) => Promise<void>;
  /// Replace one connection's tags and / or folders; a side left undefined is untouched.
  assign: (connectionId: string, patch: { tagIds?: string[]; folderIds?: string[] }) => Promise<ConnectionSummary | null>;
}

// WHAT:  The connections page's tags and folders, and which folder chip is selected.
// WHY:   Tags and folders are entities now (a connection may hold several of each),
//        so the list of them is state of its own, loaded once and refreshed after
//        every change. Connections carry only ids (`extra.tagIds` / `folderIds`).
// HOW:   Every mutation ends by reloading the entity list and the connection
//        list: deleting a tag changes every connection that wore it, and a
//        count in the manager changes when a connection is filed.
// WHERE: src-tauri/src/commands/connection_org.rs
export const useConnectionOrg = create<ConnectionOrgState>((set, get) => ({
  tags: [],
  folders: [],
  loaded: false,
  folderFilter: loadFilter(),

  load: async () => {
    try {
      const org = await ipc("list_connection_org");
      // A chip for a folder that no longer exists would filter everything out.
      const filter = get().folderFilter;
      const stillThere = filter === null || org.folders.some((f) => f.id === filter);
      if (!stillThere) saveFilter(null);
      set({ tags: org.tags, folders: org.folders, loaded: true, ...(stillThere ? {} : { folderFilter: null }) });
    } catch (raw) {
      set({ loaded: true });
      useWorkspace.getState().showError(normalizeError(raw));
    }
  },

  setFolderFilter: (id) => {
    saveFilter(id);
    set({ folderFilter: id });
  },

  saveTag: async (id, name, color) => {
    const tag = await ipc("save_connection_tag", { id, name, color });
    await get().load();
    return tag;
  },

  deleteTag: async (id) => {
    await ipc("delete_connection_tag", { id });
    await Promise.all([get().load(), useWorkspace.getState().refreshConnections()]);
  },

  saveFolder: async (id, name) => {
    const folder = await ipc("save_connection_folder", { id, name });
    await Promise.all([get().load(), useWorkspace.getState().refreshConnections()]);
    return folder;
  },

  deleteFolder: async (id) => {
    await ipc("delete_connection_folder", { id });
    await Promise.all([get().load(), useWorkspace.getState().refreshConnections()]);
  },

  assign: async (connectionId, patch) => {
    try {
      const saved = await ipc("set_connection_org", { connectionId, tagIds: patch.tagIds ?? null, folderIds: patch.folderIds ?? null });
      useWorkspace.setState((s) => ({ connections: s.connections.map((c) => (c.id === saved.id ? saved : c)) }));
      await get().load();
      return saved;
    } catch (raw) {
      useWorkspace.getState().showError(normalizeError(raw));
      return null;
    }
  },
}));
