// SOT: connection-colours, connection-colour-tokens, connection-groups, connection-search, folder-grouping
import type { ConnectionColor, ConnectionInput, ConnectionSummary } from "./bindings";
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

// WHAT:  Search over name, host / file and engine label, case-insensitive.
export function matchesConnection(c: ConnectionSummary, needle: string): boolean {
  const q = needle.trim().toLowerCase();
  if (q.length === 0) return true;
  const hay = [c.name, c.host ?? "", c.filePath ?? "", c.engine, engineMeta(c.engine).label, c.folder ?? ""];
  return hay.some((h) => h.toLowerCase().includes(q));
}

export interface ConnectionGroup {
  /// Stable key for collapse state: "favorites", "folder:<name>" or "ungrouped".
  key: string;
  label: string;
  kind: "favorites" | "folder" | "ungrouped";
  items: ConnectionSummary[];
}

// WHAT:  Favourites first, then one group per folder (A–Z), then the rest.
// WHY:   Someone with forty connections finds the three they use daily at the
//        top and the rest where they filed them. A favourite is listed once,
//        under Favourites, not again in its folder.
// HOW:   Pure: the page and the sidebar switcher render the same grouping.
export function groupConnections(connections: readonly ConnectionSummary[]): ConnectionGroup[] {
  const byName = (a: ConnectionSummary, b: ConnectionSummary) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  const favorites = connections.filter((c) => c.favorite).sort(byName);
  const folders = new Map<string, ConnectionSummary[]>();
  const loose: ConnectionSummary[] = [];
  for (const c of connections) {
    if (c.favorite) continue;
    const folder = c.folder?.trim() ?? "";
    if (folder.length === 0) loose.push(c);
    else folders.set(folder, [...(folders.get(folder) ?? []), c]);
  }
  const groups: ConnectionGroup[] = [];
  if (favorites.length > 0) groups.push({ key: "favorites", label: "Favourites", kind: "favorites", items: favorites });
  for (const name of [...folders.keys()].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }))) {
    groups.push({ key: `folder:${name}`, label: name, kind: "folder", items: (folders.get(name) ?? []).sort(byName) });
  }
  if (loose.length > 0) groups.push({ key: "ungrouped", label: groups.length > 0 ? "Other connections" : "Connections", kind: "ungrouped", items: loose.sort(byName) });
  return groups;
}

// WHAT:  Every folder name in use, for the "Move to folder" suggestions.
export function folderNames(connections: readonly ConnectionSummary[]): string[] {
  const names = new Set<string>();
  for (const c of connections) {
    const folder = c.folder?.trim() ?? "";
    if (folder.length > 0) names.add(folder);
  }
  return [...names].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
}

// WHAT:  The editable input for a saved connection: secrets blank = keep.
// WHY:   Both the form and the one-click organisation actions (favourite, move
//        to folder) save through `save_connection`, which takes a full input.
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
    folder: summary.folder,
    color: summary.color,
    favorite: summary.favorite,
  };
}
