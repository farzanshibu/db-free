// SOT: key-tree-model, key-separator-detection, key-folder-grouping, children-paging
import type { RedisKeyType } from "@/lib/bindings";

// WHAT:  Redis keys grouped into folders by their separator: `bull:queue:1` is
//        bull › queue › 1.
// WHY:   A flat list of tens of thousands of keys cannot be read; the namespaces
//        people give their keys (`:` is the convention, `/` and `.` are common)
//        are the structure the tree shows. The separator is found from the keys
//        themselves, so a `/`-style keyspace is grouped on `/`.
// HOW:   Pure functions over a key list: `detectSeparator` counts the candidates,
//        `buildTree` makes the trie, `sortedChildren` orders a folder (folders
//        first, then names), and the component pages a folder's children in
//        steps of `CHILDREN_STEP`.
// WHERE: src/features/shell/KeyTree.tsx

export const SEPARATORS = [":", "/", "."] as const;
export type Separator = (typeof SEPARATORS)[number];
export const CHILDREN_STEP = 100;

export interface KeyItem {
  name: string;
  /// Null for engines that cannot say (Memcached, RocksDB).
  kind: RedisKeyType | null;
  preview: string | null;
}

export interface KeyNode {
  /// This level's text between separators.
  name: string;
  /// The whole path up to and including this level (a folder's id).
  path: string;
  children: Map<string, KeyNode>;
  /// Set when a key ends exactly here (a folder can also be a key).
  item: KeyItem | null;
  /// Keys at or below this node.
  count: number;
}

// WHAT:  The separator that occurs most across the keys; ":" when none occurs.
export function detectSeparator(items: readonly KeyItem[]): Separator {
  let best: Separator = ":";
  let bestCount = 0;
  for (const separator of SEPARATORS) {
    let count = 0;
    for (const item of items) count += item.name.split(separator).length - 1;
    if (count > bestCount) {
      best = separator;
      bestCount = count;
    }
  }
  return best;
}

function node(name: string, path: string): KeyNode {
  return { name, path, children: new Map(), item: null, count: 0 };
}

export function buildTree(items: readonly KeyItem[], separator: Separator): KeyNode {
  const root = node("", "");
  for (const item of items) {
    const parts = item.name.split(separator);
    let current = root;
    let path = "";
    parts.forEach((part, index) => {
      path = index === 0 ? part : `${path}${separator}${part}`;
      let child = current.children.get(part);
      if (child === undefined) {
        child = node(part, path);
        current.children.set(part, child);
      }
      child.count += 1;
      if (index === parts.length - 1) child.item = item;
      current = child;
    });
  }
  return root;
}

// WHAT:  A folder's children in display order: those with children first, then by name.
export function sortedChildren(parent: KeyNode): KeyNode[] {
  return [...parent.children.values()].sort((a, b) => {
    const aFolder = a.children.size > 0;
    const bFolder = b.children.size > 0;
    if (aFolder !== bFolder) return aFolder ? -1 : 1;
    return a.name.localeCompare(b.name, undefined, { numeric: true });
  });
}
