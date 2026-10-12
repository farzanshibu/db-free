// SOT: table-view-state, stored-table-view, hidden-columns-persistence, page-size-memory
import type { FilterMode, SortRule } from "@/lib/bindings";
import { readStored, writeStored } from "@/lib/storage";
import { isFilterOp, type FilterEntry } from "./filterOps";

// WHAT:  What a table tab remembers between openings: its sort rules (each can be
//        switched off without losing it), its filter rules and how they combine,
//        the columns hidden from view, and the page size per connection.
// WHY:   Reopening the table you were working in should not throw away the view you
//        built. Sort and filters follow the "remember table state" setting; hidden
//        columns and the page size are plain view preferences and always stick.
// HOW:   Stored as delimited, URI-encoded fields rather than JSON: JSON would have to
//        be narrowed from an untyped value, which only the IPC boundary may do
//        (scripts/guardrail.py). Every field arrives as a string and is validated.
//        Strings written before rules could be switched off (two groups) still load.
// WHERE: src/features/grid/TableTab.tsx

/// Separators for the stored table view state: group, entry, field.
const GROUP_SEP = "\u0003";
const ENTRY_SEP = "\u0002";
const FIELD_SEP = "\u0001";

export interface SortEntry {
  rule: SortRule;
  enabled: boolean;
}

export interface TableViewState {
  sort: SortEntry[];
  filters: FilterEntry[];
  mode: FilterMode;
}

export const EMPTY_VIEW: TableViewState = { sort: [], filters: [], mode: "all" };

const decode = (raw: string): string => {
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
};

export function readTableState(key: string): TableViewState | null {
  const raw = readStored(key);
  if (raw === null) return null;
  const [sortPart = "", filterPart = "", modePart = ""] = raw.split(GROUP_SEP);
  const sort: SortEntry[] = [];
  for (const entry of sortPart.split(ENTRY_SEP).filter((e) => e.length > 0)) {
    const [column = "", desc = "", enabled = "1"] = entry.split(FIELD_SEP);
    if (column.length > 0) sort.push({ rule: { column: decode(column), desc: desc === "1" }, enabled: enabled !== "0" });
  }
  const filters: FilterEntry[] = [];
  for (const entry of filterPart.split(ENTRY_SEP).filter((e) => e.length > 0)) {
    const [column = "", op = "", value = "", enabled = "1"] = entry.split(FIELD_SEP);
    if (column.length > 0 && isFilterOp(op)) filters.push({ rule: { column: decode(column), op, value: decode(value) }, enabled: enabled !== "0" });
  }
  return { sort, filters, mode: modePart === "any" ? "any" : "all" };
}

export function writeTableState(key: string, state: TableViewState): void {
  const sort = state.sort.map((s) => [encodeURIComponent(s.rule.column), s.rule.desc ? "1" : "0", s.enabled ? "1" : "0"].join(FIELD_SEP)).join(ENTRY_SEP);
  const filters = state.filters
    .map((f) => [encodeURIComponent(f.rule.column), f.rule.op, encodeURIComponent(f.rule.value), f.enabled ? "1" : "0"].join(FIELD_SEP))
    .join(ENTRY_SEP);
  writeStored(key, [sort, filters, state.mode].join(GROUP_SEP));
}

export function tableStateKey(connectionId: string, tableKey: string): string {
  return `db-free:table-state:${connectionId}:${tableKey}`;
}

// ---- hidden columns -----------------------------------------------------------

export function hiddenColumnsKey(connectionId: string, tableKey: string): string {
  return `db-free:table-hidden:${connectionId}:${tableKey}`;
}

export function readHiddenColumns(key: string): ReadonlySet<string> {
  const raw = readStored(key);
  if (raw === null || raw.length === 0) return new Set();
  return new Set(raw.split(ENTRY_SEP).map(decode).filter((name) => name.length > 0));
}

export function writeHiddenColumns(key: string, hidden: ReadonlySet<string>): void {
  writeStored(key, [...hidden].map(encodeURIComponent).join(ENTRY_SEP));
}

// ---- page size ----------------------------------------------------------------

/// The sizes the pager offers, as option values.
export const PAGE_SIZES = [25, 50, 100, 200, 500, 1000] as const;
export const DEFAULT_PAGE_SIZE = 50;

export function pageSizeKey(connectionId: string): string {
  return `db-free:page-size:${connectionId}`;
}

/// The size remembered for this connection, else the default setting, else 50.
export function readPageSize(connectionId: string, fallback: number | undefined): number {
  const stored = Number(readStored(pageSizeKey(connectionId)));
  if (PAGE_SIZES.some((size) => size === stored)) return stored;
  return PAGE_SIZES.find((size) => size === fallback) ?? DEFAULT_PAGE_SIZE;
}

export function writePageSize(connectionId: string, size: number): void {
  writeStored(pageSizeKey(connectionId), String(size));
}
