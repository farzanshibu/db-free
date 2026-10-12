// SOT: redis-collection-commands, hash-list-set-zset-writes, zset-score-parsing, list-remove-by-index
import { redisLine } from "@/lib/redisCommands";

// WHAT:  The command lines each collection edit sends.
// WHY:   Hashes, lists, sets and sorted sets are edited row by row, and each
//        edit is a Redis command: HSET / HDEL, LSET / RPUSH / LREM, SADD / SREM,
//        ZADD / ZREM. Keeping the builders pure keeps the quoting and the odd
//        cases (removing a list element by index, renaming a set member) in one
//        place, away from the component.
// WHERE: src/features/keys/CollectionEditor.tsx, src/lib/redisCommands.ts (quoting)
export type CollectionKind = "hash" | "list" | "set" | "zset";

/// One row of a collection as the editor holds it, as text.
export interface EntryRow {
  /// hash: field · list: index · set: member · zset: member
  id: string;
  /// hash: [field, value] · list: [index, value] · set: [member] · zset: [member, score]
  cells: readonly string[];
}

export interface CollectionColumn {
  name: string;
  /// A click on the cell opens an editor for it.
  editable: boolean;
}

export const COLLECTION_COLUMNS: Record<CollectionKind, readonly CollectionColumn[]> = {
  hash: [
    { name: "field", editable: false },
    { name: "value", editable: true },
  ],
  list: [
    { name: "index", editable: false },
    { name: "value", editable: true },
  ],
  set: [{ name: "member", editable: true }],
  zset: [
    { name: "member", editable: true },
    { name: "score", editable: true },
  ],
};

// WHAT:  The column the search box filters on.
export const FILTER_COLUMN: Record<CollectionKind, string> = { hash: "field", list: "value", set: "member", zset: "member" };

// WHAT:  A score as Redis reads it: a finite number, or +inf / -inf. Returns the
//        text to send, or null when it is not a score.
export function parseScore(text: string): string | null {
  const trimmed = text.trim();
  if (/^[+-]?inf$/i.test(trimmed)) return trimmed.toLowerCase().startsWith("-") ? "-inf" : "+inf";
  if (trimmed === "") return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? trimmed : null;
}

/// What the user typed into the "add" row.
export interface Draft {
  /// hash: field · set / zset: member · list: unused
  name: string;
  /// hash / list: value · zset: score
  value: string;
  /// list: push at the head instead of the tail
  head: boolean;
}

export function addCommands(kind: CollectionKind, key: string, draft: Draft): string[] {
  switch (kind) {
    case "hash":
      return [redisLine("HSET", key, draft.name, draft.value)];
    case "list":
      return [redisLine(draft.head ? "LPUSH" : "RPUSH", key, draft.value)];
    case "set":
      return [redisLine("SADD", key, draft.name)];
    case "zset":
      return [redisLine("ZADD", key, parseScore(draft.value) ?? "0", draft.name)];
  }
}

/// A value no real list element is: LSET writes it where an element is about to
/// be removed, and LREM then removes exactly that one.
export function removalMarker(): string {
  return `__db_free_remove_${crypto.randomUUID()}`;
}

// WHAT:  Commands that remove one row.
// HOW:   Redis can remove a list element by value (LREM) but not by index, and
//        the value may occur elsewhere, so the element at the index is first
//        overwritten with a marker no element can equal, then that marker is removed.
export function removeCommands(kind: CollectionKind, key: string, row: EntryRow, marker: string): string[] {
  const first = row.cells[0] ?? "";
  switch (kind) {
    case "hash":
      return [redisLine("HDEL", key, first)];
    case "list":
      return [redisLine("LSET", key, row.id, marker), redisLine("LREM", key, "1", marker)];
    case "set":
      return [redisLine("SREM", key, first)];
    case "zset":
      return [redisLine("ZREM", key, first)];
  }
}

// WHAT:  Commands that replace one cell of a row.
// HOW:   Renaming a set member or a sorted-set member is a remove plus an add;
//        the other edits are a single write. Returns null for a cell that cannot
//        be edited or a score that is not a number.
export function editCommands(kind: CollectionKind, key: string, row: EntryRow, column: number, text: string): string[] | null {
  switch (kind) {
    case "hash":
      return column === 1 ? [redisLine("HSET", key, row.cells[0] ?? "", text)] : null;
    case "list":
      return column === 1 ? [redisLine("LSET", key, row.id, text)] : null;
    case "set":
      return column === 0 ? [redisLine("SREM", key, row.cells[0] ?? ""), redisLine("SADD", key, text)] : null;
    case "zset": {
      const member = row.cells[0] ?? "";
      const score = row.cells[1] ?? "0";
      if (column === 0) return [redisLine("ZREM", key, member), redisLine("ZADD", key, score, text)];
      const parsed = parseScore(text);
      return parsed === null ? null : [redisLine("ZADD", key, parsed, member)];
    }
  }
}

// WHAT:  True when a draft has what the type needs to be added.
export function draftReady(kind: CollectionKind, draft: Draft): boolean {
  switch (kind) {
    case "hash":
      return draft.name.length > 0;
    case "list":
      return draft.value.length > 0;
    case "set":
      return draft.name.length > 0;
    case "zset":
      return draft.name.length > 0 && parseScore(draft.value) !== null;
  }
}
