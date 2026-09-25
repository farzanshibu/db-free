// SOT: table-tab, table-toolbar, page-based-browsing, sort-state, export-copy, full-table-export, file-download, row-inspector, inspector-collapse, insert-row-flow, delete-rows-flow, cell-edit-staging, foreign-key-traversal, staged-row-mapping
import { useCallback, useEffect, useMemo, useState } from "react";
import type { CellValue, ColumnInfo, FilterOp, FilterRule, ForeignKey, SortRule, StagedChange, TablePage, TableRef, Value } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { downloadTextFile, exportFilename, toCsvText, toJsonText, type ExportFormat } from "@/lib/export";
import { DENSITIES, formatCount } from "@/lib/format";
import { readStored, writeStored } from "@/lib/storage";
import { engineMeta, isObjectStorageEngine } from "@/lib/engines";
import { pickSaveFile } from "@/lib/native";
import { tableKey, useWorkspace } from "@/stores/workspace";
import { DataGrid, type CellEdit, type GridColumn, type StagedCell } from "./DataGrid";
import type { LookupRow } from "@/components/global/ValueEditor";
import { FILTER_OPS, FilterPopover } from "./FilterPopover";
import { ColumnsPopover } from "./ColumnsPopover";
import { nextSort } from "./clientRows";
import { RecordInspector, cellText, sqlLiteral, useInspectorCollapsed } from "./RecordInspector";
import { AppSelect } from "@/components/global/Field";
import { FormValueField } from "@/components/global/ValueEditor";
import { IconButton } from "@/components/global/Button";
import { EmptyState } from "@/components/global/EmptyState";
import { Icon } from "@/lib/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

const PAGE_SIZES = [
  { value: "50", label: "50 rows" },
  { value: "100", label: "100 rows" },
  { value: "200", label: "200 rows" },
  { value: "500", label: "500 rows" },
  { value: "1000", label: "1,000 rows" },
] satisfies readonly { value: string; label: string }[];
type PageSize = (typeof PAGE_SIZES)[number]["value"];

interface Loaded {
  key: string;
  page: TablePage | null;
  error: string | null;
}

let changeCounter = 0;
function nextChangeId(): string {
  changeCounter += 1;
  return `chg-${Date.now()}-${changeCounter}`;
}

// Stable empty list: a selector must return the same reference for unchanged state.
const EMPTY_CHANGES: StagedChange[] = [];
const EMPTY_FKS: ForeignKey[] = [];
const NULL_VALUE: Value = { t: "null" };
type InsertChange = Extract<StagedChange, { kind: "insert" }>;

/// Rows offered when picking a foreign-key value, and how many of the referenced
/// row's own columns are shown beside the id.
const LOOKUP_LIMIT = 50;
const LOOKUP_DETAIL_COLUMNS = 3;

/// Rows fetched per request when exporting the whole filtered table, and the
/// most rows a browser-side export will hold before pointing at Transfer.
const EXPORT_PAGE = 1000;
const MAX_EXPORT_ROWS = 500_000;

/// Separators for the stored table view state: group, entry, field.
const GROUP_SEP = "\u0003";
const ENTRY_SEP = "\u0002";
const FIELD_SEP = "\u0001";

interface TableViewState {
  sort: SortRule[];
  filters: FilterRule[];
}

/// Restores the sort and filters a table was last closed with.
///
/// Stored as delimited, URI-encoded fields rather than JSON: JSON would have to
/// be narrowed from an untyped value, which only the IPC boundary may do
/// (scripts/guardrail.py). Every field arrives as a string and is validated.
function readTableState(key: string): TableViewState | null {
  const raw = readStored(key);
  if (raw === null) return null;
  const [sortPart = "", filterPart = ""] = raw.split(GROUP_SEP);
  const sort: SortRule[] = [];
  for (const entry of sortPart.split(ENTRY_SEP).filter((e) => e.length > 0)) {
    const [column = "", desc = ""] = entry.split(FIELD_SEP);
    if (column.length > 0) sort.push({ column: decodeURIComponent(column), desc: desc === "1" });
  }
  const filters: FilterRule[] = [];
  for (const entry of filterPart.split(ENTRY_SEP).filter((e) => e.length > 0)) {
    const [column = "", op = "", value = ""] = entry.split(FIELD_SEP);
    if (column.length > 0 && isFilterOp(op)) filters.push({ column: decodeURIComponent(column), op, value: decodeURIComponent(value) });
  }
  return { sort, filters };
}

function writeTableState(key: string, state: TableViewState): void {
  const sort = state.sort.map((s) => [encodeURIComponent(s.column), s.desc ? "1" : "0"].join(FIELD_SEP)).join(ENTRY_SEP);
  const filters = state.filters.map((f) => [encodeURIComponent(f.column), f.op, encodeURIComponent(f.value)].join(FIELD_SEP)).join(ENTRY_SEP);
  writeStored(key, [sort, filters].join(GROUP_SEP));
}

function isFilterOp(value: string): value is FilterOp {
  return value in FILTER_OPS;
}

// WHAT:  One open table: toolbar (insert, refresh, filter, sort, export, delete),
//        pager, virtualized grid with inline editing, record inspector.
// WHY:   Page-based browsing with an exact/estimated total; edits are staged in
//        review mode (Pending Changes) or committed at once in direct mode.
// HOW:   A request key (table + sort + filters + page + size + refresh) drives
//        one effect; `loading` is derived from key mismatch, never set in render.
// WHERE: src-tauri/src/services/data.rs, src-tauri/src/services/changes.rs
export function TableTab({ connectionId, table, initialFilters }: { connectionId: string; table: TableRef; initialFilters?: FilterRule[] | undefined }) {
  const density = useWorkspace((s) => s.density);
  const foreignKeys = useWorkspace((s) => s.foreignKeys[connectionId] ?? EMPTY_FKS);
  const openTable = useWorkspace((s) => s.openTable);
  const settings = useWorkspace((s) => s.settings);
  const engine = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.engine ?? "postgres");
  const readOnly = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.readOnly ?? false);
  const pending = useWorkspace((s) => s.pendingChanges[connectionId] ?? EMPTY_CHANGES);
  const stageChange = useWorkspace((s) => s.stageChange);
  const unstageChange = useWorkspace((s) => s.unstageChange);
  const showInfo = useWorkspace((s) => s.showInfo);
  const showError = useWorkspace((s) => s.showError);
  const [pageIndex, setPageIndex] = useState(0);
  const [pageSize, setPageSize] = useState<PageSize>("50");
  // WHAT:  Sort and filters restored per table when the setting is on.
  // WHY:   Reopening the table you were working in should not throw away the
  //        view you built; the key is the connection + table so two tables never
  //        share state.
  const stateKey = `db-free:table-state:${connectionId}:${tableKey(table)}`;
  const restored = useMemo(() => readTableState(stateKey), [stateKey]);
  const [sort, setSort] = useState<SortRule[]>(restored?.sort ?? []);
  const [filters, setFilters] = useState<FilterRule[]>(initialFilters ?? restored?.filters ?? []);
  const [hiddenColumns, setHiddenColumns] = useState<ReadonlySet<string>>(new Set());
  const [autoRefresh, setAutoRefresh] = useState(0);
  /// Right-click "Filter: col = value" builds a rule from the clicked cell. The
  /// rule travels with the page request, so it works on every engine — SQL
  /// adapters compile it into WHERE, the others filter the fetched rows.
  const addFilter = useCallback((rule: FilterRule) => {
    setPageIndex(0);
    setFilters((prev) => [...prev.filter((f) => !(f.column === rule.column && f.op === rule.op)), rule]);
  }, []);
  const [refresh, setRefresh] = useState(0);
  const [loaded, setLoaded] = useState<Loaded>({ key: "", page: null, error: null });
  const [selectedRows, setSelectedRows] = useState<Set<number>>(new Set());
  const [cell, setCell] = useState<{ row: number; col: number } | null>(null);
  const [insertOpen, setInsertOpen] = useState(false);
  const [inspectorTab, setInspectorTab] = useState<string>(settings?.inspectorTabs[0] ?? "fields");
  const [inspectorCollapsed, setInspectorCollapsed, toggleInspector] = useInspectorCollapsed();

  const editable = engineMeta(engine).commandLanguage === "SQL" && !readOnly;
  const limit = Number(pageSize);
  const requestKey = JSON.stringify({ table, sort, filters, pageIndex, limit, refresh });

  useEffect(() => {
    const token = { cancelled: false };
    void (async () => {
      try {
        const page = await ipc("fetch_table_page", { connectionId, table, query: { sort, filters, offset: pageIndex * limit, limit } });
        if (!token.cancelled) setLoaded({ key: requestKey, page, error: null });
      } catch (raw) {
        if (!token.cancelled) setLoaded({ key: requestKey, page: null, error: normalizeError(raw).message });
      }
    })();
    return () => {
      token.cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- requestKey encodes every input
  }, [connectionId, requestKey]);

  useEffect(() => {
    const onRefresh = () => setRefresh((r) => r + 1);
    window.addEventListener("db-free:refresh-tables", onRefresh);
    return () => window.removeEventListener("db-free:refresh-tables", onRefresh);
  }, []);

  useEffect(() => {
    if (settings?.rememberTableState === false) return;
    writeTableState(stateKey, { sort, filters });
  }, [settings?.rememberTableState, stateKey, sort, filters]);

  // Auto-refresh: 0 is off, anything else is the interval in seconds.
  useEffect(() => {
    if (autoRefresh === 0) return;
    const id = window.setInterval(() => setRefresh((r) => r + 1), autoRefresh * 1000);
    return () => window.clearInterval(id);
  }, [autoRefresh]);

  const loading = loaded.key !== requestKey;
  const page = loaded.page;
  const rows = useMemo(() => page?.rows ?? [], [page]);
  const columns = useMemo(() => page?.columns ?? [], [page]);
  const total = page?.total ?? null;
  const pageCount = total !== null ? Math.max(1, Math.ceil(total / limit)) : null;
  const hasNext = pageCount !== null ? pageIndex + 1 < pageCount : rows.length === limit;

  // Foreign keys leaving this table, by source column (single-column keys only).
  const fkByColumn = useMemo(() => {
    const map = new Map<string, { table: TableRef; column: string }>();
    for (const fk of foreignKeys) {
      if (fk.fromTable !== table.name || (fk.fromSchema ?? null) !== (table.schema ?? null) || fk.fromColumns.length !== 1) continue;
      const from = fk.fromColumns[0];
      const to = fk.toColumns[0];
      if (from !== undefined && to !== undefined) map.set(from, { table: { schema: fk.toSchema, name: fk.toTable }, column: to });
    }
    return map;
  }, [foreignKeys, table]);

  const gridColumns: GridColumn[] = useMemo(
    () =>
      columns
        .filter((c) => !hiddenColumns.has(c.name))
        .map((c) => {
          const link = fkByColumn.get(c.name);
          return { name: c.name, typeName: c.dataType, primaryKey: c.primaryKey, ...(link ? { linkTo: tableKey(link.table) } : {}) };
        }),
    [columns, fkByColumn, hiddenColumns],
  );
  /// Grid column index -> page column index, so hiding a column does not shift
  /// the values under the ones still shown.
  const columnIndexes = useMemo(() => columns.map((c, i) => ({ c, i })).filter(({ c }) => !hiddenColumns.has(c.name)).map(({ i }) => i), [columns, hiddenColumns]);

  // WHAT:  FK traversal: open the referenced table filtered to the clicked value.
  const openLinked = (rowIndex: number, colIndex: number) => {
    const column = columns[colIndex];
    const value = rows[rowIndex]?.[colIndex];
    const link = column ? fkByColumn.get(column.name) : undefined;
    if (!link || !value || value.t === "null") return;
    const text = value.t === "json" ? JSON.stringify(value.v) : String(value.v);
    openTable(connectionId, link.table, [{ column: link.column, op: "eq", value: text }]);
  };
  const pkColumns = useMemo(() => columns.filter((c) => c.primaryKey), [columns]);

  // WHAT:  Staged changes for this table mapped onto grid rows: updated cells by
  //        "row:col" (with the old value for the tooltip), deleted rows by index,
  //        and staged inserts appended as editable ghost rows after the page.
  const tableChanges = useMemo(() => pending.filter((c) => tableKey(c.table) === tableKey(table)), [pending, table]);
  const rowIndexOf = useCallback(
    (key: readonly CellValue[]) => rows.findIndex((r) => key.every((k) => JSON.stringify(r[columns.findIndex((col) => col.name === k.column)]) === JSON.stringify(k.value))),
    [rows, columns],
  );
  const staged = useMemo(() => {
    const map = new Map<string, StagedCell>();
    for (const c of tableChanges) {
      if (c.kind !== "update") continue;
      const rowIndex = rowIndexOf(c.key);
      const colIndex = columns.findIndex((col) => col.name === c.column);
      if (rowIndex >= 0 && colIndex >= 0) map.set(`${rowIndex}:${colIndex}`, { value: c.new, old: c.old });
    }
    return map;
  }, [tableChanges, rowIndexOf, columns]);
  const deletedRows = useMemo(() => new Set(tableChanges.filter((c) => c.kind === "delete").map((c) => rowIndexOf(c.key)).filter((i) => i >= 0)), [tableChanges, rowIndexOf]);
  const inserts = useMemo(() => tableChanges.filter((c): c is InsertChange => c.kind === "insert"), [tableChanges]);
  const allRows = useMemo(
    () => [...rows, ...inserts.map((c) => columns.map((col) => c.values.find((v) => v.column === col.name)?.value ?? NULL_VALUE))],
    [rows, inserts, columns],
  );
  /// The grid only knows the visible columns, so rows are projected onto them
  /// and every column index coming back is mapped to the page's own.
  const getGridRow = useCallback(
    (i: number): readonly Value[] | undefined => {
      const row = allRows[i];
      return row === undefined ? undefined : columnIndexes.map((c) => row[c] ?? NULL_VALUE);
    },
    [allRows, columnIndexes],
  );
  const pageColumn = useCallback((gridColumn: number) => columnIndexes[gridColumn] ?? gridColumn, [columnIndexes]);

  const keyOf = useCallback(
    (row: readonly Value[]): CellValue[] => pkColumns.map((c) => ({ column: c.name, value: row[columns.findIndex((col) => col.name === c.name)] ?? { t: "null" } })),
    [pkColumns, columns],
  );

  const applyChanges = async (changes: StagedChange[]) => {
    if (settings?.executionMode === "direct") {
      try {
        await ipc("commit_changes", { connectionId, changes });
        showInfo(`Applied ${changes.length} change(s).`);
        setRefresh((r) => r + 1);
      } catch (raw) {
        showError(normalizeError(raw));
      }
    } else {
      for (const c of changes) stageChange(connectionId, c);
    }
  };

  // WHAT:  Applies one or many cell edits (inline edit, paste, fill) as a batch.
  // WHY:   A 50-cell paste must be one commit in direct mode and one staging
  //        pass otherwise; and several edits to the same staged insert have to
  //        merge, not overwrite each other from a stale `inserts` snapshot.
  const onCellsEdit = (edits: readonly CellEdit[]) => {
    const updates: StagedChange[] = [];
    const insertEdits = new Map<number, Map<string, Value>>();
    let blocked: string | null = null;
    for (const { row: rowIndex, col: colIndex, value: next } of edits) {
      const column = columns[colIndex];
      if (!column) continue;
      if (rowIndex >= rows.length) {
        // Ghost row: the edit rewrites the staged insert itself (same id → replaced in place).
        const byColumn = insertEdits.get(rowIndex) ?? new Map<string, Value>();
        byColumn.set(column.name, next);
        insertEdits.set(rowIndex, byColumn);
        continue;
      }
      if (deletedRows.has(rowIndex)) {
        blocked = "This row is staged for deletion. Undo the delete in Pending Changes first.";
        continue;
      }
      const row = rows[rowIndex];
      if (!row) continue;
      if (pkColumns.length === 0) {
        blocked = "This table has no primary key, so rows cannot be edited safely.";
        break;
      }
      const old = row[colIndex] ?? { t: "null" };
      const key = keyOf(row);
      if (JSON.stringify(old) === JSON.stringify(next)) {
        // Back to the original value: drop any staged edit for this cell instead of adding one.
        const existing = pending.find((c) => c.kind === "update" && tableKey(c.table) === tableKey(table) && c.column === column.name && JSON.stringify(c.key) === JSON.stringify(key));
        if (existing) unstageChange(connectionId, existing.id);
        continue;
      }
      updates.push({ kind: "update", id: nextChangeId(), table, key, column: column.name, old, new: next });
    }
    for (const [rowIndex, byColumn] of insertEdits) {
      const insert = inserts[rowIndex - rows.length];
      if (!insert) continue;
      const values = columns.flatMap((col) => {
        const edited = byColumn.get(col.name);
        if (edited !== undefined) return [{ column: col.name, value: edited }];
        const existing = insert.values.find((v) => v.column === col.name);
        return existing ? [existing] : [];
      });
      stageChange(connectionId, { ...insert, values });
    }
    if (blocked !== null) showError(blocked);
    if (updates.length > 0) void applyChanges(updates);
  };

  const deleteSelected = () => {
    // Selected ghost rows: drop the staged insert instead of staging a delete.
    for (const i of selectedRows) {
      const insert = i >= rows.length ? inserts[i - rows.length] : undefined;
      if (insert) unstageChange(connectionId, insert.id);
    }
    const real = [...selectedRows].filter((i) => i < rows.length && !deletedRows.has(i));
    if (real.length > 0 && pkColumns.length === 0) {
      showError("This table has no primary key, so rows cannot be deleted safely.");
      return;
    }
    const changes: StagedChange[] = real
      .map((i) => rows[i])
      .filter((r): r is Value[] => r !== undefined)
      .map((r) => ({ kind: "delete", id: nextChangeId(), table, key: keyOf(r) }));
    if (changes.length === 0) {
      setSelectedRows(new Set());
      return;
    }
    if (settings?.executionMode === "direct" && !window.confirm(`Delete ${changes.length} row(s) now?`)) return;
    setSelectedRows(new Set());
    void applyChanges(changes);
  };

  // WHAT:  Row actions from the cell menu, expressed as staged changes like every
  //        other edit so review mode and the Changes panel still see them.
  const deleteRow = (rowIndex: number) => {
    const insert = rowIndex >= rows.length ? inserts[rowIndex - rows.length] : undefined;
    if (insert) {
      unstageChange(connectionId, insert.id);
      return;
    }
    const row = rows[rowIndex];
    if (!row) return;
    if (pkColumns.length === 0) {
      showError("This table has no primary key, so rows cannot be deleted safely.");
      return;
    }
    if (settings?.executionMode === "direct" && !window.confirm("Delete this row now?")) return;
    void applyChanges([{ kind: "delete", id: nextChangeId(), table, key: keyOf(row) }]);
  };

  const duplicateRow = (rowIndex: number) => {
    const row = allRows[rowIndex];
    if (!row) return;
    // A duplicate is an insert of every column except generated keys, which the
    // database fills in again.
    const values: CellValue[] = columns
      .map((c, i) => ({ column: c.name, value: row[i] ?? NULL_VALUE }))
      .filter(({ column }) => !pkColumns.some((pk) => pk.name === column && /serial|identity|auto/i.test(pk.dataType)));
    void applyChanges([{ kind: "insert", id: nextChangeId(), table, values }]);
  };

  // WHAT:  Candidate rows for a foreign-key cell: the referenced table, filtered
  //        by what the user typed, with each row's other columns as the detail.
  // WHY:   An id says nothing about which row it is; picking one should not mean
  //        opening the other table and copying a value back.
  const lookupFk = useCallback(
    async (gridColumn: number, search: string): Promise<LookupRow[]> => {
      const column = columns[pageColumn(gridColumn)];
      const link = column ? fkByColumn.get(column.name) : undefined;
      if (!link) return [];
      const filters: FilterRule[] = search.trim().length > 0 ? [{ column: link.column, op: "contains", value: search.trim() }] : [];
      const page = await ipc("fetch_table_page", { connectionId, table: link.table, query: { sort: [], filters, offset: 0, limit: LOOKUP_LIMIT } });
      const keyIndex = page.columns.findIndex((c) => c.name === link.column);
      if (keyIndex < 0) return [];
      return page.rows.map((row) => ({
        value: cellText(row[keyIndex]),
        detail: page.columns
          .map((c, i) => ({ c, i }))
          .filter(({ i }) => i !== keyIndex)
          .slice(0, LOOKUP_DETAIL_COLUMNS)
          .map(({ c, i }) => `${c.name}: ${cellText(row[i])}`)
          .join(" · "),
      }));
    },
    [columns, pageColumn, fkByColumn, connectionId],
  );

  const copyRowsFrom = async (rowIndex: number, format: "json" | "csv" | "sql") => {
    if (!page) return;
    const chosen = selectedRows.size > 0 ? rows.filter((_, i) => selectedRows.has(i)) : [rows[rowIndex]].filter((r): r is Value[] => r !== undefined);
    const text = format === "json" ? toJsonText(page.columns, chosen) : format === "csv" ? toCsvText(page.columns, chosen) : toInserts(table, page, chosen);
    await navigator.clipboard.writeText(text);
    showInfo(`Copied ${chosen.length} row(s) as ${format.toUpperCase()}.`);
  };

  const toggleSort = (column: string) => {
    setPageIndex(0);
    setSort((current) => nextSort(current, column));
  };

  const copyAs = async (format: ExportFormat) => {
    if (!page) return;
    const chosen = selectedRows.size > 0 ? rows.filter((_, i) => selectedRows.has(i)) : rows;
    const text = format === "json" ? toJsonText(page.columns, chosen) : toCsvText(page.columns, chosen);
    await navigator.clipboard.writeText(text);
    showInfo(`Copied ${chosen.length} row(s) as ${format.toUpperCase()} to the clipboard.`);
  };

  const downloadPage = (format: ExportFormat) => {
    if (!page) return;
    const chosen = selectedRows.size > 0 ? rows.filter((_, i) => selectedRows.has(i)) : rows;
    const text = format === "json" ? toJsonText(page.columns, chosen) : toCsvText(page.columns, chosen);
    downloadTextFile(exportFilename(tableKey(table), format), text, format === "json" ? "application/json" : "text/csv");
    showInfo(`Downloaded ${chosen.length} row(s) as ${format.toUpperCase()}.`);
  };

  // WHAT:  Full-table export: replays the current sort + filters page by page so
  //        the file holds every matching row, not just the visible page.
  // WHY:   Copy/export-page silently drops everything off-screen; the Transfer
  //        tab already streams huge tables server-side, so this caps at
  //        MAX_EXPORT_ROWS and points beyond that at Transfer.
  // HOW:   Same fetch_table_page every engine honours, 1k rows at a time, stops
  //        on a short page or an exact total. Serialized with the shared helper.
  const [exporting, setExporting] = useState(false);
  const downloadAll = async (format: ExportFormat) => {
    if (exporting) return;
    setExporting(true);
    try {
      const all: Value[][] = [];
      let fetchedColumns = columns;
      let offset = 0;
      let capped = false;
      for (;;) {
        const chunk = await ipc("fetch_table_page", { connectionId, table, query: { sort, filters, offset, limit: EXPORT_PAGE } });
        if (offset === 0) fetchedColumns = chunk.columns;
        for (const r of chunk.rows) {
          if (all.length >= MAX_EXPORT_ROWS) {
            capped = true;
            break;
          }
          all.push(r);
        }
        offset += chunk.rows.length;
        if (capped || chunk.rows.length < EXPORT_PAGE) break;
        if (chunk.total !== null && chunk.totalExact && offset >= chunk.total) break;
      }
      const text = format === "json" ? toJsonText(fetchedColumns, all) : toCsvText(fetchedColumns, all);
      downloadTextFile(exportFilename(`${tableKey(table)}-all`, format), text, format === "json" ? "application/json" : "text/csv");
      showInfo(capped ? `Downloaded the first ${formatCount(all.length)} rows as ${format.toUpperCase()}. Use the Transfer tab for larger exports.` : `Downloaded ${formatCount(all.length)} row(s) as ${format.toUpperCase()}.`);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setExporting(false);
    }
  };

  const onExportAction = (key: string) => {
    if (key === "copy-csv") void copyAs("csv");
    else if (key === "copy-json") void copyAs("json");
    else if (key === "download-page-csv") downloadPage("csv");
    else if (key === "download-page-json") downloadPage("json");
    else if (key === "download-all-csv") void downloadAll("csv");
    else if (key === "download-all-json") void downloadAll("json");
  };

  // WHAT:  Byte-exact download for object storage (S3 / MinIO / R2).
  // WHY:   The grid row only shows key/size metadata; GET shows text, but binary
  //        files need a real local copy without JSON overhead.
  const isObjectStore = isObjectStorageEngine(engine);
  const [downloading, setDownloading] = useState(false);
  const downloadKey = useCallback((): string | null => {
    if (!isObjectStore || !page) return null;
    const keyIndex = columns.findIndex((c) => c.name === "key");
    if (keyIndex < 0) return null;
    const rowIndex = cell?.row ?? [...selectedRows][0];
    if (rowIndex === undefined) return null;
    const row = allRows[rowIndex];
    const value = row?.[keyIndex];
    if (!value || value.t === "null") return null;
    const text = value.t === "json" ? JSON.stringify(value.v) : String(value.v);
    return text.length > 0 ? text : null;
  }, [isObjectStore, page, columns, cell, selectedRows, allRows]);
  const downloadSelected = useCallback(async () => {
    const key = downloadKey();
    if (!key) {
      showError("Select a row to download.");
      return;
    }
    const suggested = key.split("/").pop() ?? key;
    const path = await pickSaveFile(suggested.length > 0 ? suggested : key);
    if (!path) return;
    setDownloading(true);
    try {
      const report = await ipc("download_object", { connectionId, bucket: table.name, key, path });
      showInfo(`Downloaded ${key} (${formatCount(report.bytes)} bytes) to ${report.path}.`);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setDownloading(false);
    }
  }, [connectionId, downloadKey, showError, showInfo, table.name]);

  const selectedRow = cell ? allRows[cell.row] : undefined;
  const selectedValue = cell ? selectedRow?.[cell.col] : undefined;
  const selectedColumn = cell ? columns[cell.col] : undefined;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ScrollArea orientation="horizontal" hideScrollBar className="flex app-toolbar shrink-0 items-center gap-0.5 border-b border-border/40 glass-header">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button size="xs" variant="soft" disabled={!editable || columns.length === 0} onClick={() => setInsertOpen(true)}>
              <Icon name="plus" size={12} />
              Insert
            </Button>
          </TooltipTrigger>
          <TooltipContent>{readOnly ? "Connection is read-only." : editable ? "Insert a row" : "Editing is only available for SQL engines."}</TooltipContent>
        </Tooltip>
        <Button size="xs" variant="toolbar" onClick={() => setRefresh((r) => r + 1)}>
          <Icon name="refresh" size={12} />
          Refresh
        </Button>
        <FilterPopover columns={columns} filters={filters} onApply={(next) => { setPageIndex(0); setFilters(next); }} />
        <ColumnsPopover columns={columns} hidden={hiddenColumns} onChange={setHiddenColumns} />
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="xs" variant={autoRefresh > 0 ? "soft" : "toolbar"}>
              <Icon name="clock" size={12} />
              {autoRefresh > 0 ? REFRESH_LABEL[autoRefresh] : "Auto"}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent className="min-w-44 glass-modal rounded-xl">
            <DropdownMenuGroup>
                {REFRESH_INTERVALS.map((seconds) => (
                  <DropdownMenuItem key={seconds} textValue={REFRESH_LABEL[seconds] ?? ""} onSelect={() => { setAutoRefresh(seconds); }}>
                    <span className="flex-1">{REFRESH_LABEL[seconds]}</span>
                    {autoRefresh === seconds ? <Icon name="check" size={13} className="ml-2 text-accent" /> : null}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <Button size="xs" variant={sort.length > 0 ? "soft" : "toolbar"} onClick={() => setSort([])} disabled={sort.length === 0}>
          <Icon name="sort" size={12} />
          {sort.length > 0 ? `Sorted by ${sort.length} rule` : "Sort"}
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="xs" variant="toolbar" disabled={rows.length === 0 || exporting}>
              <Icon name="download" size={12} />
              {exporting ? "Exporting…" : "Export"}
              <Icon name="chevron-down" size={10} />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent className="glass-modal rounded-xl">
            <DropdownMenuGroup>
              <DropdownMenuItem textValue="Copy as CSV" onSelect={() => { onExportAction("copy-csv"); }}><span>Copy {selectedRows.size > 0 ? "selection" : "page"} as CSV</span></DropdownMenuItem>
              <DropdownMenuItem textValue="Copy as JSON" onSelect={() => { onExportAction("copy-json"); }}><span>Copy {selectedRows.size > 0 ? "selection" : "page"} as JSON</span></DropdownMenuItem>
              <DropdownMenuItem textValue="Download page as CSV" onSelect={() => { onExportAction("download-page-csv"); }}><span>Download {selectedRows.size > 0 ? "selection" : "page"} as CSV</span></DropdownMenuItem>
              <DropdownMenuItem textValue="Download page as JSON" onSelect={() => { onExportAction("download-page-json"); }}><span>Download {selectedRows.size > 0 ? "selection" : "page"} as JSON</span></DropdownMenuItem>
              <DropdownMenuItem textValue="Download all rows as CSV" onSelect={() => { onExportAction("download-all-csv"); }}><span>Download all{total !== null ? ` ${formatCount(total)}` : ""} rows as CSV</span></DropdownMenuItem>
              <DropdownMenuItem textValue="Download all rows as JSON" onSelect={() => { onExportAction("download-all-json"); }}><span>Download all{total !== null ? ` ${formatCount(total)}` : ""} rows as JSON</span></DropdownMenuItem>
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        {isObjectStore ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button size="xs" variant="toolbar" disabled={rows.length === 0 || downloading || downloadKey() === null} onClick={() => void downloadSelected()}>
                <Icon name="download" size={12} />
                {downloading ? "Downloading…" : "Download"}
              </Button>
            </TooltipTrigger>
            <TooltipContent>Download the selected file to disk (byte-exact).</TooltipContent>
          </Tooltip>
        ) : null}
        {selectedRows.size > 0 && editable ? (
          <Button size="sm" variant="danger-soft" className="rounded-lg liquid-hover" onClick={deleteSelected}>
            <Icon name="trash" size={13} />
            Delete {selectedRows.size}
          </Button>
        ) : null}

        <div className="ml-auto flex shrink-0 items-center gap-2 text-xs whitespace-nowrap text-muted">
          {loading ? <span className="text-accent font-medium">loading…</span> : null}
          {selectedRows.size > 0 ? (
            <Badge size="sm" variant="soft" color="accent" className="font-medium">
              {selectedRows.size} selected
            </Badge>
          ) : null}
          <IconButton icon="columns" label={cell === null ? "Inspect selected cell" : inspectorCollapsed ? "Show inspector" : "Hide inspector"} active={cell !== null && !inspectorCollapsed} disabled={cell === null} onClick={toggleInspector} />
          <Separator orientation="vertical" className="mx-0.5 h-4 opacity-50" />
          <div className="flex items-center gap-1 rounded-lg glass-pill px-1.5 py-0.5">
            <IconButton icon="chevron-left" label="Previous page" disabled={pageIndex === 0} onClick={() => setPageIndex((p) => Math.max(0, p - 1))} size={13} className="size-5 min-w-5" />
            <span className="px-1 tabular-nums font-mono text-[11px] text-foreground">
              {pageIndex + 1}
              <span className="text-muted"> / {pageCount ?? "…"}</span>
            </span>
            <IconButton icon="chevron-right" label="Next page" disabled={!hasNext} onClick={() => setPageIndex((p) => p + 1)} size={13} className="size-5 min-w-5" />
          </div>
          <AppSelect ariaLabel="Rows per page" value={pageSize} options={PAGE_SIZES} size="sm" className="w-24 shrink-0" onChange={(v) => { setPageIndex(0); setPageSize(v); }} />
          <span className="min-w-16 text-right tabular-nums font-mono text-[11px] text-muted">{total !== null ? `${page?.totalExact ? "" : "≈ "}${formatCount(total)} rows` : `${formatCount(rows.length)} rows`}</span>
        </div>
      </ScrollArea>

      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1">
          {loaded.error !== null ? (
            <EmptyState icon="table" title="Could not load table" body={loaded.error} action={<Button size="sm" onClick={() => setRefresh((r) => r + 1)}>Retry</Button>} />
          ) : (
            <DataGrid
              columns={gridColumns}
              rowCount={allRows.length}
              getRow={getGridRow}
              rowHeight={DENSITIES[density].rowHeight}
              sort={sort}
              onSortToggle={toggleSort}
              selectedRows={selectedRows}
              onToggleRow={(i) =>
                setSelectedRows((s) => {
                  const next = new Set(s);
                  if (next.has(i)) next.delete(i);
                  else next.add(i);
                  return next;
                })
              }
              onToggleAll={() => setSelectedRows((s) => (s.size === allRows.length ? new Set() : new Set(allRows.map((_, i) => i))))}
              onCellSelect={(row, col) => setCell({ row, col: pageColumn(col) })}
              selected={cell}
              {...(editable
                ? {
                    onCellEdit: (row: number, col: number, next: Value) => onCellsEdit([{ row, col: pageColumn(col), value: next }]),
                    onCellsEdit: (edits: readonly CellEdit[]) => onCellsEdit(edits.map((e) => ({ ...e, col: pageColumn(e.col) }))),
                  }
                : {})}
              onPasteNotice={(message) => showInfo(message)}
              staged={staged}
              deletedRows={deletedRows}
              insertedFrom={rows.length}
              nullDisplay={settings?.nullDisplay ?? "NULL"}
              onLinkOpen={(row, col) => openLinked(row, pageColumn(col))}
              onFilter={addFilter}
              onSortSet={(rule) => { setPageIndex(0); setSort([rule]); }}
              onClearSort={() => setSort([])}
              onCopied={(what) => showInfo(`${what} copied to the clipboard.`)}
              alternatingRows={settings?.alternatingRows ?? true}
              onInspect={(row, col) => { setCell({ row, col: pageColumn(col) }); setInspectorCollapsed(false); }}
              onCopyRows={(row, format) => void copyRowsFrom(row, format)}
              onLookup={lookupFk}
              {...(editable ? { onInsertRow: () => setInsertOpen(true), onDuplicateRow: duplicateRow, onDeleteRow: deleteRow } : {})}
            />
          )}
        </div>
        {selectedRow && selectedValue !== undefined && selectedColumn ? (
          <RecordInspector
            columns={columns}
            row={selectedRow}
            column={selectedColumn}
            value={selectedValue}
            table={table}
            tabs={settings?.inspectorTabs ?? ["fields", "json", "sql"]}
            activeTab={inspectorTab}
            onTab={setInspectorTab}
            collapsed={inspectorCollapsed}
            onToggle={toggleInspector}
            onClose={() => setCell(null)}
          />
        ) : null}
      </div>

      <InsertRowModal open={insertOpen} onClose={() => setInsertOpen(false)} columns={columns} onSubmit={(values) => { setInsertOpen(false); void applyChanges([{ kind: "insert", id: nextChangeId(), table, values }]); }} />
    </div>
  );
}

// WHAT:  Insert form: one typed control per column (number field, date / time
//        picker, JSON editor, true/false select, text). Empty = omitted so the
//        database default applies; the NULL toggle sends an explicit NULL.
function InsertRowModal({ open, onClose, columns, onSubmit }: { open: boolean; onClose: () => void; columns: readonly ColumnInfo[]; onSubmit: (values: CellValue[]) => void }) {
  const [values, setValues] = useState<Record<string, Value | undefined>>({});
  const submit = () => {
    const out: CellValue[] = [];
    for (const c of columns) {
      const value = values[c.name];
      if (value !== undefined) out.push({ column: c.name, value });
    }
    setValues({});
    onSubmit(out);
  };
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-[560px]">
        <DialogHeader>
          <DialogTitle>Insert row</DialogTitle>
        </DialogHeader>
        <DialogBody className="max-h-[60vh] p-0">
          <ScrollArea className="max-h-[60vh] px-4 py-3">
            <div className="grid grid-cols-2 gap-3">
              {columns.map((c) => (
                <FormValueField key={c.name} column={c} value={values[c.name]} onChange={(v) => setValues((s) => ({ ...s, [c.name]: v }))} />
              ))}
            </div>
          </ScrollArea>
        </DialogBody>
        <DialogFooter>
          <Button variant="tertiary" onClick={onClose}>Cancel</Button>
          <Button onClick={submit}>Add row</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// Auto-refresh choices, in seconds; 0 is off.
const REFRESH_INTERVALS = [0, 5, 10, 30, 60, 300] as const;
const REFRESH_LABEL: Record<number, string> = { 0: "Off", 5: "5 seconds", 10: "10 seconds", 30: "30 seconds", 60: "1 minute", 300: "5 minutes" };

// WHAT:  Rows as INSERT statements, ready to paste into a query tab.
// WHY:   Copying a row to another environment is the usual reason to copy one;
//        JSON and CSV both lose the types the target has to be told about.
function toInserts(table: TableRef, page: TablePage, rows: readonly (readonly Value[])[]): string {
  const target = table.schema === null ? table.name : `${table.schema}.${table.name}`;
  const columns = page.columns.map((c) => c.name).join(", ");
  return rows.map((row) => `INSERT INTO ${target} (${columns}) VALUES (${row.map(sqlLiteral).join(", ")});`).join("\n");
}
