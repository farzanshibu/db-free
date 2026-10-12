// SOT: connection-colours, connection-colour-tokens, connection-groups, connection-search, folder-grouping, connection-input-from-summary
import type { ConnectionColor, ConnectionFolder, ConnectionInput, ConnectionSummary, ConnectionTag } from "./bindings";
import { emptyAuthSecrets } from "./cloudAuth";
import { engineMeta } from "./engines";
import { keysOf } from "./records";

// WHAT:  Label and token classes per connection colour.
// WHY:   The colour is a fixed palette key chosen in Rust (`ConnectionColor`);
//        this registry fails `satisfies` until every key has tokens.
// WHERE: src/styles/globals.css (--color-conn-*), src-tauri/src/model/connection.rs
export interface ConnectionColorMeta {
  label: string;
  /// Solid fill: the card stripe and the swatch.
  fill: string;
}

export const CONNECTION_COLORS = {
  blue: { label: "Blue", fill: "bg-conn-blue" },
  teal: { label: "Teal", fill: "bg-conn-teal" },
  green: { label: "Green", fill: "bg-conn-green" },
  amber: { label: "Amber", fill: "bg-conn-amber" },
  orange: { label: "Orange", fill: "bg-conn-orange" },
  red: { label: "Red", fill: "bg-conn-red" },
  pink: { label: "Pink", fill: "bg-conn-pink" },
  violet: { label: "Violet", fill: "bg-conn-violet" },
  gray: { label: "Gray", fill: "bg-conn-gray" },
} satisfies Record<ConnectionColor, ConnectionColorMeta>;

export const CONNECTION_COLOR_ORDER: ConnectionColor[] = keysOf(CONNECTION_COLORS);

export function connectionColorMeta(color: ConnectionColor): ConnectionColorMeta {
  return CONNECTION_COLORS[color];
}

// WHAT:  What a connection points at, as one line (host:port/db or the file).
export function connectionTarget(c: ConnectionSummary): string {
  if (engineMeta(c.engine).form === "file") return c.filePath ?? "";
  return `${c.host ?? ""}${c.port !== null ? `:${c.port}` : ""}${c.database ? `/${c.database}` : ""}`;
}

// WHAT:  Search over name, host / file, engine label, folders and tags, case-insensitive.
// HOW:   `tags` is the tag list the connection's `extra.tagIds` index into; without
//        it the tag names simply are not searched.
export function matchesConnection(c: ConnectionSummary, needle: string, tags: readonly ConnectionTag[] = [], folders: readonly ConnectionFolder[] = []): boolean {
  const q = needle.trim().toLowerCase();
  if (q.length === 0) return true;
  const tagNames = tags.filter((t) => c.extra.tagIds.includes(t.id)).map((t) => t.name);
  const folderNames = folders.filter((f) => c.extra.folderIds.includes(f.id)).map((f) => f.name);
  const hay = [c.name, c.host ?? "", c.filePath ?? "", c.database ?? "", c.engine, engineMeta(c.engine).label, c.folder ?? "", ...folderNames, ...tagNames];
  return hay.some((h) => h.toLowerCase().includes(q));
}

export interface ConnectionGroup {
  /// Stable key for collapse state: "favorites", "folder:<name>", "transient" or "ungrouped".
  key: string;
  label: string;
  kind: "favorites" | "folder" | "ungrouped" | "transient";
  items: ConnectionSummary[];
  /// The folder entity behind a folder group, when the caller supplied them.
  folderId?: string;
}

const byName = (a: ConnectionSummary, b: ConnectionSummary) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
const byLabel = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base" });

// WHAT:  Files opened for one session first, then favourites, then one group per
//        folder (A–Z), then the rest.
// WHY:   Someone with forty connections finds the three they use daily at the
//        top and the rest where they filed them. A favourite is listed once,
//        under Favourites, not again in its folders; a connection in two
//        folders appears under both.
// HOW:   Pure: the page and the sidebar switcher render the same grouping. With
//        `folders` (the folder entities) the membership is `extra.folderIds` and
//        empty folders appear as empty groups; without them the legacy `folder`
//        name (the first folder) is used, which is all the switcher needs.
export function groupConnections(connections: readonly ConnectionSummary[], folders?: readonly ConnectionFolder[]): ConnectionGroup[] {
  const transient = connections.filter((c) => c.extra.transient).sort(byName);
  const saved = connections.filter((c) => !c.extra.transient);
  const favorites = saved.filter((c) => c.favorite).sort(byName);
  const loose: ConnectionSummary[] = [];
  const groups: ConnectionGroup[] = [];
  if (transient.length > 0) groups.push({ key: "transient", label: "Opened files", kind: "transient", items: transient });
  if (favorites.length > 0) groups.push({ key: "favorites", label: "Favourites", kind: "favorites", items: favorites });

  const folderGroups = new Map<string, ConnectionGroup>();
  const groupFor = (name: string, folderId?: string): ConnectionGroup => {
    const existing = folderGroups.get(name);
    if (existing) return existing;
    const created: ConnectionGroup = { key: `folder:${name}`, label: name, kind: "folder", items: [], ...(folderId !== undefined ? { folderId } : {}) };
    folderGroups.set(name, created);
    return created;
  };
  if (folders) for (const folder of folders) groupFor(folder.name, folder.id);

  for (const c of saved) {
    const names = folders ? folders.filter((f) => c.extra.folderIds.includes(f.id)).map((f) => f.name) : [c.folder?.trim() ?? ""].filter((n) => n.length > 0);
    if (!c.favorite) {
      if (names.length === 0) loose.push(c);
      for (const name of names) groupFor(name).items.push(c);
    }
  }
  for (const name of [...folderGroups.keys()].sort(byLabel)) {
    const group = folderGroups.get(name);
    if (group) groups.push({ ...group, items: [...group.items].sort(byName) });
  }
  if (loose.length > 0) groups.push({ key: "ungrouped", label: groups.length > 0 ? "Other connections" : "Connections", kind: "ungrouped", items: loose.sort(byName) });
  return groups;
}

// WHAT:  Every folder name in use, for suggestions.
export function folderNames(connections: readonly ConnectionSummary[]): string[] {
  const names = new Set<string>();
  for (const c of connections) {
    const folder = c.folder?.trim() ?? "";
    if (folder.length > 0) names.add(folder);
  }
  return [...names].sort(byLabel);
}

// WHAT:  The editable input for a saved connection: secrets blank = keep.
// WHY:   Both the form and the one-click organisation actions (favourite, colour)
//        save through `save_connection`, which takes a full input. Tags and
//        folders travel as ids in `extra`, which a save replaces wholesale, so
//        they must be echoed back; the legacy `folder` name is cleared so it
//        cannot re-add a folder the user just removed.
export function inputFromSummary(summary: ConnectionSummary): ConnectionInput {
  return {
    name: summary.name,
    engine: summary.engine,
    environment: summary.environment,
    readOnly: summary.readOnly,
    host: summary.host,
    port: summary.port,
    database: summary.database,
    username: summary.username,
    password: null,
    filePath: summary.filePath,
    sslMode: summary.sslMode,
    ssh: summary.ssh,
    sshSecret: null,
    folder: null,
    color: summary.color,
    favorite: summary.favorite,
    extra: { ...summary.extra, tunnelOriginHost: null, transient: false },
    auth: summary.auth,
    authSecrets: emptyAuthSecrets(),
  };
}
