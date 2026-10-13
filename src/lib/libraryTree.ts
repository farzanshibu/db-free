// SOT: library-tree, folder-tree-builder, folder-paths, folder-pickers, sidebar-name-clash
import type { LibraryFolder, LibraryPlacement, LibraryScope } from "./bindings";

// WHAT:  Pure folder-tree logic shared by the saved-query, dashboard, workflow and
//        diagram sidebars: build the tree from the flat folder / placement lists,
//        flatten it for pickers, and answer path / clash questions.
// WHY:   The server stores folders flat (`parentId`) and placements separately from
//        the items, so every sidebar has to join them. Doing it once, here, keeps the
//        four lists identical and the rules testable apart from React.
// WHERE: src/stores/library.ts (the data), src/features/documents/LibraryTree.tsx (the view)

export interface FolderNode<T> {
  folder: LibraryFolder;
  /// Subfolders that are shown.
  folders: FolderNode<T>[];
  /// Items directly in this folder that are shown.
  items: T[];
  /// Shown items anywhere underneath, subfolders included.
  count: number;
}

export interface LibraryTreeData<T> {
  folders: FolderNode<T>[];
  /// Shown items that are in no folder.
  items: T[];
  /// Shown items in total.
  count: number;
}

export type SortDirection = "az" | "za";

export interface BuildOptions<T> {
  /// Items that may be shown at all (the "this connection" filter, a search).
  visible: (item: T) => boolean;
  /// Hide folders with nothing visible in them, empty ones included (a search).
  /// Off: a folder with nothing in it at all stays; one whose items are all
  /// filtered out goes.
  hideEmptyFolders: boolean;
  direction: SortDirection;
}

function compareNames(a: string, b: string, direction: SortDirection): number {
  const order = a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });
  return direction === "az" ? order : -order;
}

// WHAT:  Builds the tree for one scope. Items name their folder through `placements`;
//        an item whose folder no longer exists sits at the top level.
export function buildTree<T extends { id: string; name: string }>(
  scope: LibraryScope,
  folders: readonly LibraryFolder[],
  placements: readonly LibraryPlacement[],
  items: readonly T[],
  options: BuildOptions<T>,
): LibraryTreeData<T> {
  const mine = folders.filter((f) => f.scope === scope);
  const known = new Set(mine.map((f) => f.id));
  const folderOf = new Map<string, string>();
  for (const p of placements) {
    if (p.scope === scope && known.has(p.folderId)) folderOf.set(p.itemId, p.folderId);
  }
  const childrenOf = new Map<string | null, LibraryFolder[]>();
  for (const f of mine) {
    // A folder whose parent is gone surfaces at the top level instead of vanishing.
    const parent = f.parentId !== null && known.has(f.parentId) ? f.parentId : null;
    childrenOf.set(parent, [...(childrenOf.get(parent) ?? []), f]);
  }
  const itemsIn = new Map<string | null, T[]>();
  for (const item of items) {
    const key = folderOf.get(item.id) ?? null;
    itemsIn.set(key, [...(itemsIn.get(key) ?? []), item]);
  }
  const sortItems = (list: T[]) => [...list].sort((a, b) => compareNames(a.name, b.name, options.direction));

  // Returns the node for `folder`, or null when it should not be shown. `seen`
  // stops a corrupt parent cycle from recursing forever.
  const build = (folder: LibraryFolder, seen: ReadonlySet<string>): FolderNode<T> | null => {
    if (seen.has(folder.id)) return null;
    const trail = new Set(seen).add(folder.id);
    const subfolders = (childrenOf.get(folder.id) ?? [])
      .map((child) => build(child, trail))
      .filter((node): node is FolderNode<T> => node !== null)
      .sort((a, b) => compareNames(a.folder.name, b.folder.name, options.direction));
    const all = itemsIn.get(folder.id) ?? [];
    const shown = sortItems(all.filter(options.visible));
    const count = shown.length + subfolders.reduce((sum, node) => sum + node.count, 0);
    const holdsAnything = all.length > 0 || (childrenOf.get(folder.id) ?? []).length > 0;
    // Visible content keeps a folder; so does being a genuinely empty branch (or
    // holding one) unless the caller is searching.
    const keep = count > 0 || (!options.hideEmptyFolders && (subfolders.length > 0 || !holdsAnything));
    return keep ? { folder, folders: subfolders, items: shown, count } : null;
  };

  const roots = (childrenOf.get(null) ?? [])
    .map((f) => build(f, new Set()))
    .filter((node): node is FolderNode<T> => node !== null)
    .sort((a, b) => compareNames(a.folder.name, b.folder.name, options.direction));
  const top = sortItems((itemsIn.get(null) ?? []).filter(options.visible));
  return { folders: roots, items: top, count: top.length + roots.reduce((sum, node) => sum + node.count, 0) };
}

// WHAT:  Every item shown under a node, subfolders included, in tree order.
export function itemsUnder<T>(node: FolderNode<T>): T[] {
  return [...node.items, ...node.folders.flatMap((child) => itemsUnder(child))];
}

export interface FlatFolder {
  folder: LibraryFolder;
  depth: number;
  /// "Reports / Weekly"
  path: string;
}

// WHAT:  The folders of one scope in tree order with their depth, for folder pickers.
// HOW:   `exclude` drops a folder and everything under it — the targets a folder
//        being moved must not offer.
export function flattenFolders(scope: LibraryScope, folders: readonly LibraryFolder[], exclude?: string): FlatFolder[] {
  const mine = folders.filter((f) => f.scope === scope);
  const known = new Set(mine.map((f) => f.id));
  const childrenOf = new Map<string | null, LibraryFolder[]>();
  for (const f of mine) {
    const parent = f.parentId !== null && known.has(f.parentId) ? f.parentId : null;
    childrenOf.set(parent, [...(childrenOf.get(parent) ?? []), f]);
  }
  const out: FlatFolder[] = [];
  const walk = (parent: string | null, depth: number, prefix: string, seen: ReadonlySet<string>) => {
    const children = [...(childrenOf.get(parent) ?? [])].sort((a, b) => compareNames(a.name, b.name, "az"));
    for (const folder of children) {
      if (folder.id === exclude || seen.has(folder.id)) continue;
      const path = prefix.length > 0 ? `${prefix} / ${folder.name}` : folder.name;
      out.push({ folder, depth, path });
      walk(folder.id, depth + 1, path, new Set(seen).add(folder.id));
    }
  };
  walk(null, 0, "", new Set());
  return out;
}

// WHAT:  "Reports / Weekly" for a folder id; "" for the top level or an id no folder has.
export function folderPath(id: string | null, folders: readonly LibraryFolder[]): string {
  if (id === null) return "";
  const byId = new Map(folders.map((f) => [f.id, f]));
  const parts: string[] = [];
  const seen = new Set<string>();
  let current = byId.get(id);
  while (current !== undefined && !seen.has(current.id)) {
    seen.add(current.id);
    parts.unshift(current.name);
    current = current.parentId === null ? undefined : byId.get(current.parentId);
  }
  return parts.join(" / ");
}

// WHAT:  The folder an item is in, or null at the top level.
export function folderIdOf(scope: LibraryScope, itemId: string, placements: readonly LibraryPlacement[], folders: readonly LibraryFolder[]): string | null {
  const placement = placements.find((p) => p.scope === scope && p.itemId === itemId);
  if (placement === undefined) return null;
  return folders.some((f) => f.id === placement.folderId) ? placement.folderId : null;
}

// WHAT:  Whether `name` is already used (case-insensitive, surrounding space ignored).
// WHY:   The same rule the server enforces, so a dialog can warn before it saves.
export function nameTaken(existing: readonly string[], name: string): boolean {
  const wanted = name.trim().toLowerCase();
  return wanted.length > 0 && existing.some((n) => n.trim().toLowerCase() === wanted);
}

// WHAT:  The names of the folders directly under `parentId` (a rename / create clash check).
export function siblingFolderNames(scope: LibraryScope, parentId: string | null, folders: readonly LibraryFolder[], except?: string): string[] {
  return folders.filter((f) => f.scope === scope && f.parentId === parentId && f.id !== except).map((f) => f.name);
}

// WHAT:  The node for a folder id, searching the whole tree.
export function findNode<T>(nodes: readonly FolderNode<T>[], id: string): FolderNode<T> | undefined {
  for (const node of nodes) {
    if (node.folder.id === id) return node;
    const inner = findNode(node.folders, id);
    if (inner !== undefined) return inner;
  }
  return undefined;
}

// WHAT:  The id of a node's folder and of every folder beneath it.
export function descendantIds<T>(node: FolderNode<T>): Set<string> {
  const ids = new Set<string>([node.folder.id]);
  for (const child of node.folders) for (const id of descendantIds(child)) ids.add(id);
  return ids;
}

// WHAT:  Every item filed in a folder or in any folder beneath it — whether or not a
//        filter currently hides it. What "export this folder" must include.
export function itemsInFolder<T extends { id: string }>(
  scope: LibraryScope,
  folderId: string,
  folders: readonly LibraryFolder[],
  placements: readonly LibraryPlacement[],
  items: readonly T[],
): T[] {
  const inside = new Set<string>([folderId]);
  // Repeat until no new folder joins: parents may be listed after their children.
  let grew = true;
  while (grew) {
    grew = false;
    for (const f of folders) {
      if (f.scope === scope && f.parentId !== null && inside.has(f.parentId) && !inside.has(f.id)) {
        inside.add(f.id);
        grew = true;
      }
    }
  }
  const filed = new Set(placements.filter((p) => p.scope === scope && inside.has(p.folderId)).map((p) => p.itemId));
  return items.filter((item) => filed.has(item.id));
}

