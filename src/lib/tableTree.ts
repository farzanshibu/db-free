// SOT: table-tree-rows, sidebar-row-model, sidebar-sort-modes, tree-keyboard-intent, tag-grouping, tag-reorder-order
import type { ColumnInfo, SchemaInfo, TableInfo, TableTag, TagState } from "./bindings";
import { tableKey } from "@/stores/workspace";

// ============================================================================
// SIDEBAR TABLE TREE (pure)
//
// WHAT:  Turns the catalog (+ search, sort mode, tags, expanded column previews)
//        into the flat list of rows the sidebar draws, and decides what a key
//        press on that list means.
// WHY:   The list is virtualized, so it needs a flat array with a fixed height
//        per row kind — not nested components that each own their state. Keeping
//        the flattening and the keyboard rules here, with no React in sight,
//        means the component only draws rows and applies intents, and the same
//        rules can be read (and changed) in one place.
// HOW:   `buildRows` walks schemas -> (tag groups) -> tables -> column preview and
//        gives every row a stable `key` and its `parent`'s key; `keyIntent` walks
//        that array. Tags are scoped per schema ("" for engines without one):
//        see `schemaScope`.
// WHERE: src/features/shell/TablesPanel.tsx, src/features/schema/sidebar/
// ============================================================================

export type SortMode = "az" | "za" | "tags";

export const SORT_MODES: readonly { value: SortMode; label: string }[] = [
  { value: "az", label: "A-Z" },
  { value: "za", label: "Z-A" },
  { value: "tags", label: "Tags" },
];

interface RowBase {
  /// Unique within the list; a table shown under two tags has two keys.
  key: string;
  /// Key of the row this one sits under (a schema header, a tag group, a table).
  parent: string | null;
}

export type TreeRow =
  | (RowBase & { kind: "schema"; name: string; scope: string; count: number })
  /// `tag` null is the fixed "Untagged" group.
  | (RowBase & { kind: "group"; scope: string; tag: TableTag | null; count: number; collapsed: boolean })
  | (RowBase & { kind: "table"; table: TableInfo; scope: string; tags: readonly TableTag[]; expanded: boolean; expandable: boolean; ref: string })
  | (RowBase & { kind: "columns-header"; table: TableInfo; count: number })
  | (RowBase & { kind: "column"; table: TableInfo; column: ColumnInfo })
  | (RowBase & { kind: "columns-loading" });

export type RowKind = TreeRow["kind"];

/// Pixel height per row kind. Fixed, so the virtualizer never has to measure.
export const ROW_HEIGHT = {
  schema: 24,
  group: 24,
  table: 24,
  "columns-header": 20,
  column: 22,
  "columns-loading": 22,
} as const satisfies Record<RowKind, number>;

export function rowHeight(row: TreeRow): number {
  return ROW_HEIGHT[row.kind];
}

const COLLATOR = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

function byName(a: TableInfo, b: TableInfo): number {
  return COLLATOR.compare(a.name, b.name);
}

// ---- tags --------------------------------------------------------------------

/// The scope tags are filed under for a schema: its tables' own `schema` field
/// ("" when the engine reports none), so a tag and the table it labels always
/// agree on where they live.
export function schemaScope(schema: SchemaInfo, namespaced: boolean): string {
  const first = schema.tables[0];
  if (first !== undefined) return first.schema ?? "";
  return namespaced ? schema.name : "";
}

function byPosition(a: TableTag, b: TableTag): number {
  return a.position - b.position || COLLATOR.compare(a.name, b.name);
}

/// The scope's tags in display order.
export function tagsForScope(state: TagState | null | undefined, scope: string): TableTag[] {
  if (!state) return [];
  return state.tags.filter((t) => t.schema === scope).sort(byPosition);
}

interface TagIndex {
  byScope: Map<string, TableTag[]>;
  /// tag id -> names of the tables carrying it
  members: Map<string, Set<string>>;
  /// `${scope}\u0000${table}` -> tags (display order)
  byTable: Map<string, TableTag[]>;
  untaggedCollapsed: ReadonlySet<string>;
}

const NONE: ReadonlySet<string> = new Set();

function indexTags(state: TagState | null | undefined): TagIndex {
  const index: TagIndex = { byScope: new Map(), members: new Map(), byTable: new Map(), untaggedCollapsed: NONE };
  if (!state) return index;
  const known = new Map<string, TableTag>();
  for (const tag of [...state.tags].sort(byPosition)) {
    known.set(tag.id, tag);
    const list = index.byScope.get(tag.schema);
    if (list) list.push(tag);
    else index.byScope.set(tag.schema, [tag]);
  }
  for (const link of state.links) {
    const tag = known.get(link.tagId);
    if (!tag) continue;
    const names = index.members.get(tag.id);
    if (names) names.add(link.table);
    else index.members.set(tag.id, new Set([link.table]));
  }
  // Table -> tags in display order: walk the ordered tags, not the links.
  for (const tag of known.values()) {
    for (const name of index.members.get(tag.id) ?? NONE) {
      const key = `${tag.schema}\u0000${name}`;
      const list = index.byTable.get(key);
      if (list) list.push(tag);
      else index.byTable.set(key, [tag]);
    }
  }
  return { ...index, untaggedCollapsed: new Set(state.untaggedCollapsed) };
}

/// Tags carried by one table of a scope, in display order.
export function tagsOfTable(state: TagState | null | undefined, scope: string, table: string): TableTag[] {
  if (!state) return [];
  const ids = new Set(state.links.filter((l) => l.table === table).map((l) => l.tagId));
  return tagsForScope(state, scope).filter((t) => ids.has(t.id));
}

/// How many tables carry a tag.
export function tagMemberCount(state: TagState | null | undefined, tagId: string): number {
  return state ? state.links.filter((l) => l.tagId === tagId).length : 0;
}

// WHAT:  The tag ids in their new order after dragging `dragged` next to `target`.
// WHY:   The drop only knows which header the pointer is over and whether it is in
//        that header's upper or lower half; the server wants the whole order.
export function reorderIds(ids: readonly string[], dragged: string, target: string, placement: "before" | "after"): string[] {
  if (dragged === target || !ids.includes(dragged) || !ids.includes(target)) return [...ids];
  const rest = ids.filter((id) => id !== dragged);
  const at = rest.indexOf(target);
  rest.splice(placement === "before" ? at : at + 1, 0, dragged);
  return rest;
}

// ---- rows --------------------------------------------------------------------

export interface TreeInput {
  /// Already narrowed to the schema picker's choice.
  schemas: readonly SchemaInfo[];
  /// Draw a header per schema (several schemas on screen).
  headers: boolean;
  /// The engine groups tables under named schemas (`Capabilities.namespaces`).
  namespaced: boolean;
  mode: SortMode;
  needle: string;
  tags: TagState | null | undefined;
  /// `tableKey`s whose column preview is open.
  expanded: ReadonlySet<string>;
  /// False when the column preview is switched off in Settings.
  expandColumns: boolean;
  /// Cached columns, or undefined while they load.
  columnsFor: (table: TableInfo) => ColumnInfo[] | undefined;
}

export function buildRows(input: TreeInput): TreeRow[] {
  const needle = input.needle.trim().toLowerCase();
  const index = indexTags(input.tags);
  const rows: TreeRow[] = [];

  for (const schema of input.schemas) {
    const matching = needle === "" ? schema.tables : schema.tables.filter((t) => t.name.toLowerCase().includes(needle));
    if (matching.length === 0) continue;
    const scope = schemaScope(schema, input.namespaced);
    let parent: string | null = null;
    if (input.headers) {
      parent = `schema:${schema.name}`;
      rows.push({ kind: "schema", key: parent, parent: null, name: schema.name, scope, count: matching.length });
    }

    if (input.mode !== "tags") {
      const sorted = [...matching].sort(byName);
      if (input.mode === "za") sorted.reverse();
      for (const table of sorted) pushTable(rows, input, table, scope, parent, "", index.byTable.get(`${scope}\u0000${table.name}`) ?? []);
      continue;
    }

    const sorted = [...matching].sort(byName);
    const tagged = new Set<string>();
    for (const tag of index.byScope.get(scope) ?? []) {
      const names = index.members.get(tag.id) ?? NONE;
      const inTag = sorted.filter((t) => names.has(t.name));
      for (const t of inTag) tagged.add(t.name);
      // A search shows only the groups that hold a match; with none, empty groups stay so they can be managed.
      if (needle !== "" && inTag.length === 0) continue;
      const collapsed = needle === "" && tag.collapsed;
      const key = `group:${scope}:${tag.id}`;
      rows.push({ kind: "group", key, parent, scope, tag, count: inTag.length, collapsed });
      if (collapsed) continue;
      for (const table of inTag) pushTable(rows, input, table, scope, key, key, []);
    }
    const untagged = sorted.filter((t) => !tagged.has(t.name));
    if (untagged.length > 0) {
      const collapsed = needle === "" && index.untaggedCollapsed.has(scope);
      const key = `group:${scope}:untagged`;
      rows.push({ kind: "group", key, parent, scope, tag: null, count: untagged.length, collapsed });
      if (!collapsed) for (const table of untagged) pushTable(rows, input, table, scope, key, key, []);
    }
  }
  return rows;
}

function pushTable(rows: TreeRow[], input: TreeInput, table: TableInfo, scope: string, parent: string | null, groupKey: string, tags: readonly TableTag[]): void {
  const ref = tableKey(table);
  const key = `table:${groupKey}:${ref}`;
  const expanded = input.expandColumns && input.expanded.has(ref);
  rows.push({ kind: "table", key, parent, table, scope, tags, expanded, expandable: input.expandColumns, ref });
  if (!expanded) return;
  const columns = input.columnsFor(table);
  if (columns === undefined) {
    rows.push({ kind: "columns-loading", key: `${key}|loading`, parent: key });
    return;
  }
  rows.push({ kind: "columns-header", key: `${key}|cols`, parent: key, table, count: columns.length });
  for (const column of columns) rows.push({ kind: "column", key: `${key}|col:${column.name}`, parent: key, table, column });
}

// ---- keyboard ----------------------------------------------------------------

/// Rows the keyboard can land on (headers of the column list and loaders cannot).
export function isFocusable(row: TreeRow): boolean {
  return row.kind === "schema" || row.kind === "group" || row.kind === "table" || row.kind === "column";
}

export type KeyIntent =
  | { type: "none" }
  | { type: "focus"; key: string }
  /// Enter on a table; `newTab` is Shift+Enter.
  | { type: "open"; key: string; newTab: boolean }
  /// Open a table's column preview or a tag group.
  | { type: "expand"; key: string }
  | { type: "collapse"; key: string };

export interface KeyPress {
  key: string;
  shiftKey: boolean;
}

// WHAT:  What a key press means on the list, given the row that has the focus.
// WHY:   ArrowRight/Left mean "go in / come out" on a tree: open a closed node, step
//        into an open one; collapse an open node, step out to the parent from a
//        closed one. Spelled once here, they cannot drift between row kinds.
export function keyIntent(rows: readonly TreeRow[], focusKey: string | null, press: KeyPress): KeyIntent {
  const focusable = rows.filter(isFocusable);
  if (focusable.length === 0) return { type: "none" };
  const at = focusKey === null ? -1 : focusable.findIndex((r) => r.key === focusKey);
  const current = at >= 0 ? focusable[at] : undefined;
  const go = (index: number): KeyIntent => {
    const row = focusable[Math.max(0, Math.min(focusable.length - 1, index))];
    return row ? { type: "focus", key: row.key } : { type: "none" };
  };
  switch (press.key) {
    case "ArrowDown":
      return go(at + 1);
    case "ArrowUp":
      return go(at < 0 ? focusable.length - 1 : at - 1);
    case "Home":
      return go(0);
    case "End":
      return go(focusable.length - 1);
    default:
      break;
  }
  if (current === undefined) return { type: "none" };
  const parent = current.parent !== null && focusable.some((r) => r.key === current.parent) ? current.parent : null;
  const next = focusable[at + 1];
  // Open nodes collapse, closed ones expand: Space and Enter on a group, Space on a table.
  const toggle = (row: TreeRow): KeyIntent => {
    if (row.kind === "table") return row.expandable ? { type: row.expanded ? "collapse" : "expand", key: row.key } : { type: "none" };
    if (row.kind === "group") return { type: row.collapsed ? "expand" : "collapse", key: row.key };
    return { type: "none" };
  };
  switch (press.key) {
    case "Enter":
      return current.kind === "table" ? { type: "open", key: current.key, newTab: press.shiftKey } : toggle(current);
    case " ":
      return toggle(current);
    case "ArrowRight": {
      if (current.kind !== "table" && current.kind !== "group") return { type: "none" };
      const open = current.kind === "table" ? current.expanded : !current.collapsed;
      if (!open) return toggle(current);
      return next?.parent === current.key ? { type: "focus", key: next.key } : { type: "none" };
    }
    case "ArrowLeft":
      if ((current.kind === "table" && current.expanded) || (current.kind === "group" && !current.collapsed)) return toggle(current);
      return parent !== null ? { type: "focus", key: parent } : { type: "none" };
    default:
      return { type: "none" };
  }
}
