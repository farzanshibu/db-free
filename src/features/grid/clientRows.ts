// SOT: client-side-filter, client-side-sort, quick-search, row-view-indexes
import type { FilterRule, SortRule, Value } from "@/lib/bindings";
import { formatCell } from "@/lib/format";

// WHAT:  Filter, quick-search and sort for rows the webview already holds (a
//        query result), producing the order in which to show them.
// WHY:   A result set is not a table the server can page again: re-running the
//        statement to sort it would re-run side effects and cost a round trip.
// HOW:   Same semantics as the server-side fallback every non-SQL adapter uses
//        (text compare, numeric when both sides parse, case-insensitive
//        contains / starts / ends, NULL sorts first), so a rule reads the same
//        in a table tab and a result. Returns source row indexes, never copies,
//        so edits can still be mapped back to the row they came from.
// WHERE: src-tauri/src/integrations/http.rs (local::page)

export interface RowView {
  /// Case-insensitive substring over every column's text; empty = off.
  search: string;
  filters: readonly FilterRule[];
  sort: readonly SortRule[];
}

/// Text a filter compares against: what the grid shows, with NULL as empty.
function valueText(value: Value | undefined): string {
  return value === undefined || value.t === "null" ? "" : formatCell(value).text;
}

function numberOf(text: string): number | null {
  if (text.trim().length === 0) return null;
  const n = Number(text);
  return Number.isFinite(n) ? n : null;
}

function compareText(a: string, b: string): number {
  const x = numberOf(a);
  const y = numberOf(b);
  if (x !== null && y !== null) return x - y;
  return a < b ? -1 : a > b ? 1 : 0;
}

export function matchesRule(rule: FilterRule, value: Value | undefined): boolean {
  const text = valueText(value);
  const needle = rule.value.trim();
  switch (rule.op) {
    case "eq":
      return text === needle;
    case "ne":
      return text !== needle;
    case "gt":
      return compareText(text, needle) > 0;
    case "gte":
      return compareText(text, needle) >= 0;
    case "lt":
      return compareText(text, needle) < 0;
    case "lte":
      return compareText(text, needle) <= 0;
    case "contains":
      return text.toLowerCase().includes(needle.toLowerCase());
    case "starts_with":
      return text.toLowerCase().startsWith(needle.toLowerCase());
    case "ends_with":
      return text.toLowerCase().endsWith(needle.toLowerCase());
    case "in":
      return needle.split(",").some((part) => part.trim() === text);
    case "is_null":
      return value === undefined || value.t === "null";
    case "is_not_null":
      return value !== undefined && value.t !== "null";
  }
}

/// Orders two cells: NULL first, numbers and booleans by value, the rest by text.
export function compareValues(a: Value | undefined, b: Value | undefined): number {
  const aNull = a === undefined || a.t === "null";
  const bNull = b === undefined || b.t === "null";
  if (aNull || bNull) return aNull === bNull ? 0 : aNull ? -1 : 1;
  if ((a.t === "int" || a.t === "float") && (b.t === "int" || b.t === "float")) return a.v - b.v;
  if (a.t === "bool" && b.t === "bool") return Number(a.v) - Number(b.v);
  if (a.t === "decimal" && b.t === "decimal") return compareText(a.v, b.v);
  return compareText(valueText(a), valueText(b));
}

// WHAT:  Indexes of the rows to show, in display order.
// HOW:   Column rules resolve by name (first match, as the server does); a rule
//        naming a column the result does not have matches nothing. The sort is
//        stable, so equal keys keep the order the database returned.
export function viewIndexes(columns: readonly { name: string }[], rows: readonly (readonly Value[])[], view: RowView): number[] {
  const col = (name: string) => columns.findIndex((c) => c.name === name);
  const filters = view.filters.map((rule) => ({ rule, index: col(rule.column) }));
  const needle = view.search.trim().toLowerCase();
  const out: number[] = [];
  rows.forEach((row, i) => {
    if (!filters.every(({ rule, index }) => index >= 0 && matchesRule(rule, row[index]))) return;
    if (needle.length > 0 && !row.some((v) => valueText(v).toLowerCase().includes(needle))) return;
    out.push(i);
  });
  const keys = view.sort.map((s) => ({ index: col(s.column), desc: s.desc })).filter((k) => k.index >= 0);
  if (keys.length === 0) return out;
  return out
    .map((i, order) => ({ i, order }))
    .sort((x, y) => {
      for (const k of keys) {
        const c = compareValues(rows[x.i]?.[k.index], rows[y.i]?.[k.index]);
        if (c !== 0) return k.desc ? -c : c;
      }
      return x.order - y.order;
    })
    .map(({ i }) => i);
}

/// Header-click cycle shared by every sortable grid: ascending, descending, off.
export function nextSort(current: readonly SortRule[], column: string): SortRule[] {
  const existing = current.find((s) => s.column === column);
  if (!existing) return [{ column, desc: false }];
  if (!existing.desc) return [{ column, desc: true }];
  return [];
}
