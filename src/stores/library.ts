// SOT: library-store, folder-state, expanded-folders, this-connection-filter, saved-query-versions-store
import { create } from "zustand";
import type { LibraryFolder, LibraryPlacement, LibraryScope } from "@/lib/bindings";
import { ipc } from "@/lib/ipc";
import { parseJson } from "@/lib/json";
import { readStored, writeStored } from "@/lib/storage";

// WHAT:  The folder layout of every library list (saved queries, dashboards,
//        workflows, diagrams) plus two remembered view preferences per list:
//        which folders are open and whether to show this connection's items only.
// WHY:   The four sidebars share one folder implementation; this is its state.
//        Items themselves stay in the workspace store (`savedQueries`,
//        `documents`); a placement says which folder each sits in.
// HOW:   Mutations call the server, then re-read the layout — a folder move can
//        change several rows (deleting a folder lifts its contents), and the
//        server is the one that knows how. View preferences live in localStorage.
// WHERE: src-tauri/src/commands/library.rs, src/lib/libraryTree.ts

const openKey = (scope: LibraryScope) => `db-free:library-open:${scope}`;
const filterKey = (scope: LibraryScope) => `db-free:library-this-connection:${scope}`;

function readOpen(scope: LibraryScope): string[] {
  const parsed = parseJson(readStored(openKey(scope)) ?? "[]");
  return Array.isArray(parsed) ? parsed.filter((v): v is string => typeof v === "string") : [];
}

// This connection only is the default: a list of every connection's items is
// rarely what a sidebar scoped to one connection should show.
function readThisConnectionOnly(scope: LibraryScope): boolean {
  return readStored(filterKey(scope)) !== "all";
}

function perScope<T>(read: (scope: LibraryScope) => T): Record<LibraryScope, T> {
  return { saved_query: read("saved_query"), dashboard: read("dashboard"), workflow: read("workflow"), diagram: read("diagram") };
}

interface LibraryState {
  folders: LibraryFolder[];
  placements: LibraryPlacement[];
  loaded: boolean;
  /// Ids of the folders that are open, per list.
  open: Record<LibraryScope, string[]>;
  /// Show only the active connection's items (and shared ones), per list.
  thisConnectionOnly: Record<LibraryScope, boolean>;
  /// The folder picked in the save-query dialog's folder field; the next new saved
  /// query is filed there (workspace.saveQuery reads it). Null = the top level.
  saveFolder: string | null;

  loadLibrary: () => Promise<void>;
  createFolder: (scope: LibraryScope, parentId: string | null, name: string) => Promise<LibraryFolder>;
  renameFolder: (id: string, name: string) => Promise<void>;
  moveFolder: (id: string, parentId: string | null) => Promise<void>;
  deleteFolder: (id: string) => Promise<void>;
  moveItem: (scope: LibraryScope, itemId: string, folderId: string | null) => Promise<void>;
  setFolderOpen: (scope: LibraryScope, folderId: string, open: boolean) => void;
  setThisConnectionOnly: (scope: LibraryScope, value: boolean) => void;
  setSaveFolder: (folderId: string | null) => void;
}

export const useLibrary = create<LibraryState>((set, get) => ({
  folders: [],
  placements: [],
  loaded: false,
  open: perScope(readOpen),
  thisConnectionOnly: perScope(readThisConnectionOnly),
  saveFolder: null,

  loadLibrary: async () => {
    const layout = await ipc("list_library");
    set({ folders: layout.folders, placements: layout.placements, loaded: true });
  },

  createFolder: async (scope, parentId, name) => {
    const folder = await ipc("create_library_folder", { scope, parentId, name });
    await get().loadLibrary();
    // Open the parent so the new folder is visible where it was made.
    if (parentId !== null) get().setFolderOpen(scope, parentId, true);
    return folder;
  },
  renameFolder: async (id, name) => {
    await ipc("rename_library_folder", { id, name });
    await get().loadLibrary();
  },
  moveFolder: async (id, parentId) => {
    const moved = await ipc("move_library_folder", { id, parentId });
    await get().loadLibrary();
    if (parentId !== null) get().setFolderOpen(moved.scope, parentId, true);
  },
  deleteFolder: async (id) => {
    await ipc("delete_library_folder", { id });
    await get().loadLibrary();
  },
  moveItem: async (scope, itemId, folderId) => {
    await ipc("move_library_item", { scope, itemId, folderId });
    await get().loadLibrary();
    if (folderId !== null) get().setFolderOpen(scope, folderId, true);
  },

  setFolderOpen: (scope, folderId, open) => {
    const current = get().open[scope];
    if (current.includes(folderId) === open) return;
    const next = open ? [...current, folderId] : current.filter((id) => id !== folderId);
    writeStored(openKey(scope), JSON.stringify(next));
    set((s) => ({ open: { ...s.open, [scope]: next } }));
  },
  setThisConnectionOnly: (scope, value) => {
    writeStored(filterKey(scope), value ? "this" : "all");
    set((s) => ({ thisConnectionOnly: { ...s.thisConnectionOnly, [scope]: value } }));
  },
  setSaveFolder: (folderId) => set({ saveFolder: folderId }),
}));
