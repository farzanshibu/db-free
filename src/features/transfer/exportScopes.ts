// SOT: export-scopes, export-all-rows-loader, selection-page-all-scopes
import type { FilterRule, SortRule, TableRef, Value } from "@/lib/bindings";
import { ipc } from "@/lib/ipc";
import type { ExportScope } from "./ExportDialog";

// WHAT:  The "which rows" choices a table browser offers the export dialog, and
//        the loader that reads every row of a filtered table.
// WHY:   The selection, the visible page and the whole filtered table are the
//        three things people mean by "export this"; the browser knows the first
//        two, and the third is a replay of its sort and filters, page by page.
// HOW:   `loadAll` stops at `MAX_ROWS`; past that the Transfer tab, which streams
//        to disk on the Rust side, is the right tool.
// WHERE: src/features/grid/TableTab.tsx
type Rows = readonly (readonly Value[])[];

const PAGE = 1_000;
export const MAX_EXPORT_ROWS = 500_000;

export async function fetchAllRows(connectionId: string, table: TableRef, sort: readonly SortRule[], filters: readonly FilterRule[]): Promise<Value[][]> {
  const all: Value[][] = [];
  let offset = 0;
  for (;;) {
    const chunk = await ipc("fetch_table_page", { connectionId, table, query: { sort: [...sort], filters: [...filters], offset, limit: PAGE } });
    for (const row of chunk.rows) {
      if (all.length >= MAX_EXPORT_ROWS) return all;
      all.push(row);
    }
    offset += chunk.rows.length;
    if (chunk.rows.length < PAGE) return all;
    if (chunk.total !== null && chunk.totalExact && offset >= chunk.total) return all;
  }
}

interface TableScopeInput {
  /// The rows the user ticked, in grid order; empty when none is selected.
  selected: Rows;
  page: Rows;
  /// How many rows match the filters, when the engine says.
  total: number | null;
  loadAll: () => Promise<Rows>;
}

export function tableScopes({ selected, page, total, loadAll }: TableScopeInput): ExportScope[] {
  const scopes: ExportScope[] = [];
  if (selected.length > 0) scopes.push({ id: "selection", label: "Selected rows", count: selected.length, preview: selected, load: () => Promise.resolve(selected) });
  scopes.push({ id: "page", label: "This page", count: page.length, preview: page, load: () => Promise.resolve(page) });
  scopes.push({ id: "all", label: "Every matching row", count: total, preview: page, load: loadAll });
  return scopes;
}

export function resultScopes(rows: Rows, selected: Rows = []): ExportScope[] {
  const scopes: ExportScope[] = [];
  if (selected.length > 0) scopes.push({ id: "selection", label: "Selected rows", count: selected.length, preview: selected, load: () => Promise.resolve(selected) });
  scopes.push({ id: "all", label: "Every row of this result", count: rows.length, preview: rows, load: () => Promise.resolve(rows) });
  return scopes;
}
