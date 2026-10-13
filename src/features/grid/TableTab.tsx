// SOT: table-tab, table-toolbar, page-based-browsing, sort-state, export-copy, full-table-export, file-download, row-inspector, inspector-collapse, insert-row-flow, delete-rows-flow, cell-edit-staging, foreign-key-traversal, staged-row-mapping, filter-bar-state, find-in-grid-state, commit-row-patching, fk-peek-state, paste-rows-flow, grid-shortcuts, copy-export-formats-menu, inspector-staged-overlay, inspector-row-identity
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CellValue, ColumnInfo, ColumnTraits, ExportFormat, FilterMode, FilterRule, ForeignKey, SqlToken, StagedChange, TablePage, TableRef, Value } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { downloadTextFile, EXPORT_FORMAT_FACTS, exportFilename } from "@/lib/export";
import { DENSITIES, formatCell, formatCount } from "@/lib/format";
import { engineMeta, isObjectStorageEngine } from "@/lib/engines";
import { ENGINE_FACTS } from "@/lib/bindings/EngineFacts.gen";
import { pickSaveFile } from "@/lib/native";
import { isReadOnlyColumn, traitsOf } from "@/lib/columnTraits";
import { tokenValue } from "@/lib/sqlTokens";
import { tableKey, useWorkspace } from "@/stores/workspace";
import { askAboutRowPrompt, canAskAboutRow } from "@/features/chat/prompts";
import { useShortcut } from "@/stores/useShortcut";
import { onCommit, RefreshTablesEvent } from "@/features/changes/commitBus";
import { commitPending } from "@/features/changes/commitPending";
import { ExportDialog, type ExportScope } from "@/features/transfer/ExportDialog";
import { tableScopes } from "@/features/transfer/exportScopes";
import { DataGrid, type CellEdit, type GridColumn, type RowCopyFormat, type StagedCell } from "./DataGrid";
import type { LookupRow } from "@/components/global/ValueEditor";
import { FilterPopover } from "./FilterPopover";
import { FilterBar } from "./FilterBar";
import { SortPopover } from "./SortPopover";
import { FindBar } from "./FindBar";
import { Pager } from "./Pager";
import { ColumnsPopover } from "./ColumnsPopover";
import { FkPeekCard, FkPinnedPanel, type PeekTarget } from "./FkPeek";
import { FkRemapDialog } from "./FkRemapDialog";
import { RecordInspector, cellText, useInspectorCollapsed } from "./RecordInspector";
import { InsertRowModal } from "./InsertRowModal";
import { CardView } from "./CardView";
import { activeRules, type FilterEntry } from "./filterOps";
import { cachedColumns, remapsFor, useFkRemaps } from "./fkStore";
import { constraintLinks, inferLinks, mergeLinks, remapLinks, type FkLink } from "./fkInference";
import { findInGrid, matchKey, stepMatch } from "./gridFind";
import type { InspectorCellEdit, InspectorColumn, InspectorEdit } from "./inspectorValues";
import { parsePastedRows } from "./pasteRows";
import { COPY_FORMATS, copyLabel, rowsAsText } from "./copyFormats";
import { hiddenColumnsKey, readHiddenColumns, readPageSize, readTableState, tableStateKey, writeHiddenColumns, writePageSize, writeTableState, type SortEntry } from "./tableState";
import { nextSort } from "./clientRows";
import { IconButton } from "@/components/global/Button";
import { EmptyState } from "@/components/global/EmptyState";
import { Segmented } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

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

// Stable empty values: a selector (or a dependency list) must see the same reference for unchanged state.
const EMPTY_CHANGES: StagedChange[] = [];
const EMPTY_FKS: ForeignKey[] = [];
const EMPTY_TRAITS: ColumnTraits[] = [];
const EMPTY_LINKS: ReadonlyMap<string, FkLink> = new Map();
const NO_FULL_ROWS: Readonly<Record<number, readonly Value[]>> = {};
const NULL_VALUE: Value = { t: "null" };
type InsertChange = Extract<StagedChange, { kind: "insert" }>;
type UpdateChange = Extract<StagedChange, { kind: "update" }>;

/// Rows offered when picking a foreign-key value, and how many of the referenced
/// row's own columns are shown beside the id.
const LOOKUP_LIMIT = 50;
const LOOKUP_DETAIL_COLUMNS = 3;

/// Rows fetched per request when exporting the whole filtered table, and the
/// most rows a browser-side export will hold before pointing at Transfer.
const EXPORT_PAGE = 1000;
const MAX_EXPORT_ROWS = 500_000;

/// The formats "Download all rows" offers. SQL INSERT is left to Export with options,
/// which writes the file on the Rust side instead of sending every row through the webview.
const DOWNLOAD_ALL_FORMATS: readonly ExportFormat[] = COPY_FORMATS.filter((format) => format !== "sql");

/// Cells longer than this come with the page as a prefix only (the real value is
/// fetched by key when the cell is opened or edited).
const TRUNCATE_OVER = 1000;

/// Engine families that cannot write `SET col = DEFAULT`.
const NO_DEFAULT_KEYWORD: ReadonlySet<string> = new Set(["sqlite", "libsql", "val_town", "cloudflare_d1", "clickhouse", "druid", "dynamodb"]);

/// Auto-refresh choices, in seconds; 0 is off.
const REFRESH_INTERVALS = [0, 5, 10, 30, 60, 300] as const;
const REFRESH_LABEL: Record<number, string> = { 0: "Off", 5: "5 seconds", 10: "10 seconds", 30: "30 seconds", 60: "1 minute", 300: "5 minutes" };

const valueText = (value: Value): string => (value.t === "json" ? JSON.stringify(value.v) : value.t === "null" ? "" : String(value.v));

// WHAT:  One open table: toolbar (insert, refresh, filter, sort, columns, export, delete),
//        a typed filter bar, pager, virtualized grid with inline editing, find bar,
//        record inspector, foreign-key peek and pinned records.
// WHY:   Page-based browsing with an exact/estimated total; edits are staged
//        in review mode (Pending Changes) or committed at once in direct mode.
// HOW:   A request key (table + sort + filters + mode + page + size + refresh) drives
//        one effect; `loading` is derived from key mismatch, never set in render.
//        The row total is fetched once per view and kept while paging.
//        After a commit that only updated cells, the changed rows are patched from
//        the statements' RETURNING rows (or one fetch by key) instead of reloading
//        the page.
// WHERE: src-tauri/src/services/data.rs, src-tauri/src/services/changes.rs, src/features/changes/commitPending.ts
export function TableTab({ connectionId, table, initialFilters }: { connectionId: string; table: TableRef; initialFilters?: FilterRule[] | undefined }) {
  const density = useWorkspace((s) => s.density);
  const foreignKeys = useWorkspace((s) => s.foreignKeys[connectionId] ?? EMPTY_FKS);
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const openTable = useWorkspace((s) => s.openTable);
  const settings = useWorkspace((s) => s.settings);
  const engine = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.engine ?? "postgres");
  const readOnly = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.readOnly ?? false);
  const pending = useWorkspace((s) => s.pendingChanges[connectionId] ?? EMPTY_CHANGES);
  const stageChange = useWorkspace((s) => s.stageChange);
  const unstageChange = useWorkspace((s) => s.unstageChange);
  const setChangesPanelOpen = useWorkspace((s) => s.setChangesPanelOpen);
  const showInfo = useWorkspace((s) => s.showInfo);
  const showError = useWorkspace((s) => s.showError);
  const remapList = useFkRemaps((s) => remapsFor(s.remaps, connectionId));
  const loadRemaps = useFkRemaps((s) => s.load);
  const root = useRef<HTMLDivElement | null>(null);
  const filterInput = useRef<HTMLInputElement | null>(null);
  const findInput = useRef<HTMLInputElement | null>(null);

  const [pageIndex, setPageIndex] = useState(0);
  const [pageSize, setPageSizeState] = useState<number>(() => readPageSize(connectionId, settings?.defaultPageSize));
  // WHAT:  Sort and filters restored per table when the setting is on.
  // WHY:   Reopening the table you were working in should not throw away the
  //        view you built; the key is the connection + table so two tables never
  //        share state.
  const stateKey = tableStateKey(connectionId, tableKey(table));
  const restored = useMemo(() => (settings?.rememberTableState === false ? null : readTableState(stateKey)), [stateKey, settings?.rememberTableState]);
  const [sortEntries, setSortEntries] = useState<SortEntry[]>(restored?.sort ?? []);
  const [filterEntries, setFilterEntries] = useState<FilterEntry[]>(initialFilters ? initialFilters.map((rule) => ({ rule, enabled: true })) : (restored?.filters ?? []));
  const [filterMode, setFilterMode] = useState<FilterMode>(initialFilters ? "all" : (restored?.mode ?? "all"));
  const hiddenKey = hiddenColumnsKey(connectionId, tableKey(table));
  const [hiddenColumns, setHiddenColumns] = useState<ReadonlySet<string>>(() => readHiddenColumns(hiddenKey));
  const [autoRefresh, setAutoRefresh] = useState(0);
  const [refresh, setRefresh] = useState(0);
  const [loaded, setLoaded] = useState<Loaded>({ key: "", page: null, error: null });
  const [totals, setTotals] = useState<{ key: string; total: number | null; exact: boolean } | null>(null);
  const [selectedRows, setSelectedRows] = useState<Set<number>>(new Set());
  const [cell, setCell] = useState<{ row: number; col: number } | null>(null);
  const [insertOpen, setInsertOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [inspectorTab, setInspectorTab] = useState<string>(settings?.inspectorTabs[0] ?? "fields");
  const [inspectorCollapsed, setInspectorCollapsed, toggleInspector] = useInspectorCollapsed();
  const [view, setView] = useState<"grid" | "cards">("grid");
  const [find, setFind] = useState({ open: false, query: "", matchCase: false, index: 0 });
  const [peek, setPeek] = useState<PeekTarget | null>(null);
  const [pinned, setPinned] = useState<PeekTarget[]>([]);
  const [linkEdit, setLinkEdit] = useState<string | null>(null);
  const [inferred, setInferred] = useState<ReadonlyMap<string, FkLink>>(EMPTY_LINKS);
  const [fullRows, setFullRows] = useState<{ key: string; rows: Readonly<Record<number, readonly Value[]>> }>({ key: "", rows: NO_FULL_ROWS });

  const sort = useMemo(() => sortEntries.filter((e) => e.enabled).map((e) => e.rule), [sortEntries]);
  const filters = useMemo(() => activeRules(filterEntries), [filterEntries]);
  const limit = pageSize;
  // Several filter rules combine as All or Any; with one rule the mode is moot.
  const effectiveMode: FilterMode = filters.length > 1 ? filterMode : "all";
  const requestKey = JSON.stringify({ table, sort, filters, filterMode: effectiveMode, pageIndex, limit, refresh });
  // The total is re-read when the table, its filters or an explicit refresh change, not when paging.
  const countKey = JSON.stringify({ table, filters, filterMode: effectiveMode, refresh });
  const totalsRef = useRef(totals);
  useEffect(() => {
    totalsRef.current = totals;
  });

  /// Right-click "Filter: col = value" builds a rule from the clicked cell. The
  /// rule travels with the page request, so it works on every engine — SQL
  /// adapters compile it into WHERE, the others filter the fetched rows.
  const addFilter = useCallback((rule: FilterRule) => {
    setPageIndex(0);
    setSelectedRows(new Set());
    setFilterEntries((prev) => [...prev.filter((f) => !(f.rule.column === rule.column && f.rule.op === rule.op)), { rule, enabled: true }]);
  }, []);

  useEffect(() => {
    const token = { cancelled: false };
    const known = totalsRef.current;
    const skipCount = pageIndex > 0 && known !== null && known.key === countKey && known.total !== null;
    void (async () => {
      try {
        const next = await ipc("fetch_table_page", {
          connectionId,
          table,
          query: { sort, filters, offset: pageIndex * limit, limit },
          ...(effectiveMode === "any" ? { filterMode: effectiveMode } : {}),
          ...(skipCount ? { skipCount: true } : {}),
          truncateOver: TRUNCATE_OVER,
        });
        if (token.cancelled) return;
        setLoaded({ key: requestKey, page: next, error: null });
        if (!skipCount) setTotals({ key: countKey, total: next.total, exact: next.totalExact });
      } catch (raw) {
        if (!token.cancelled) setLoaded({ key: requestKey, page: null, error: normalizeError(raw).message });
      }
    })();
    return () => {
      token.cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- requestKey and countKey encode every input
  }, [connectionId, requestKey]);

  useEffect(() => {
    const onRefresh = (event: Event) => {
      // A commit that already patched this table's rows in place needs no reload.
      if (event instanceof RefreshTablesEvent && event.skip.has(tableKey(table))) return;
      setRefresh((r) => r + 1);
    };
    window.addEventListener("db-free:refresh-tables", onRefresh);
    return () => window.removeEventListener("db-free:refresh-tables", onRefresh);
  }, [table]);

  useEffect(() => {
    if (settings?.rememberTableState === false) return;
    writeTableState(stateKey, { sort: sortEntries, filters: filterEntries, mode: filterMode });
  }, [settings?.rememberTableState, stateKey, sortEntries, filterEntries, filterMode]);

  useEffect(() => {
    writeHiddenColumns(hiddenKey, hiddenColumns);
  }, [hiddenKey, hiddenColumns]);

  // Auto-refresh: 0 is off, anything else is the interval in seconds.
  useEffect(() => {
    if (autoRefresh === 0) return;
    const id = window.setInterval(() => setRefresh((r) => r + 1), autoRefresh * 1000);
    return () => window.clearInterval(id);
  }, [autoRefresh]);

  useEffect(() => {
    void loadRemaps(connectionId);
  }, [connectionId, loadRemaps]);

  const loading = loaded.key !== requestKey;
  const page = loaded.page;
  const rows = useMemo(() => page?.rows ?? [], [page]);
  const columns = useMemo(() => page?.columns ?? [], [page]);
  const traitList = page?.columnTraits ?? EMPTY_TRAITS;
  const total = totals !== null && totals.key === countKey ? totals.total : (page?.total ?? null);
  const totalExact = totals !== null && totals.key === countKey ? totals.exact : (page?.totalExact ?? false);
  const pageCount = total !== null ? Math.max(1, Math.ceil(total / limit)) : null;
  const hasNext = pageCount !== null ? pageIndex + 1 < pageCount : rows.length === limit;
  const editableEngine = page?.editable ?? false;
  const editable = editableEngine && !readOnly;
  const canSetDefault = !NO_DEFAULT_KEYWORD.has(ENGINE_FACTS[engine].family);

  const inspectorColumns: InspectorColumn[] = useMemo(
    () =>
      columns.map((c) => {
        const t = traitsOf(traitList, c.name);
        return {
          name: c.name,
          dataType: c.dataType,
          primaryKey: c.primaryKey,
          nullable: c.nullable,
          readOnly: isReadOnlyColumn(t),
          autoIncrement: t.autoIncrement,
          hasDefault: t.hasDefault,
          defaultExpr: t.defaultExpr,
          enumValues: t.enumValues,
          multi: t.multi,
        };
      }),
    [columns, traitList],
  );

  // Foreign keys leaving this table, by source column: the user's links first, then the
  // schema's constraints, then guesses from the column name.
  const constraints = useMemo(() => constraintLinks(foreignKeys, table), [foreignKeys, table]);
  const remaps = useMemo(() => remapLinks(remapList, table), [remapList, table]);
  useEffect(() => {
    if (!catalog || columns.length === 0) return;
    let cancelled = false;
    const known = new Set([...constraints.keys(), ...remaps.keys()]);
    void inferLinks(table, columns, catalog, known, (t) => cachedColumns(connectionId, t)).then((found) => {
      if (!cancelled) setInferred(found);
    });
    return () => {
      cancelled = true;
    };
  }, [catalog, columns, constraints, remaps, table, connectionId]);
  const links = useMemo(() => mergeLinks(remaps, constraints, inferred), [remaps, constraints, inferred]);

  const gridColumns: GridColumn[] = useMemo(
    () =>
      inspectorColumns
        .filter((c) => !hiddenColumns.has(c.name))
        .map((c) => {
          const link = links.get(c.name);
          return {
            name: c.name,
            typeName: c.dataType,
            primaryKey: c.primaryKey,
            nullable: c.nullable,
            readOnly: c.readOnly,
            autoIncrement: c.autoIncrement,
            enumValues: c.enumValues,
            multi: c.multi,
            ...(link ? { linkTo: tableKey(link.table), linkSource: link.source } : {}),
          };
        }),
    [inspectorColumns, links, hiddenColumns],
  );
  /// Grid column index -> page column index, so hiding a column does not shift
  /// the values under the ones still shown.
  const columnIndexes = useMemo(() => columns.map((c, i) => ({ c, i })).filter(({ c }) => !hiddenColumns.has(c.name)).map(({ i }) => i), [columns, hiddenColumns]);

  // WHAT:  FK traversal: open the referenced table filtered to the clicked value.
  const openLink = useCallback(
    (link: FkLink, value: Value) => {
      if (value.t === "null") return;
      openTable(connectionId, link.table, [{ column: link.column, op: "eq", value: valueText(value) }]);
    },
    [connectionId, openTable],
  );
  const openLinked = (rowIndex: number, colIndex: number) => {
    const column = columns[colIndex];
    const value = rows[rowIndex]?.[colIndex];
    const link = column ? links.get(column.name) : undefined;
    if (link && value) openLink(link, value);
  };
  const peekLinked = (rowIndex: number, colIndex: number, anchor: HTMLElement) => {
    const column = columns[colIndex];
    const value = rows[rowIndex]?.[colIndex];
    const link = column ? links.get(column.name) : undefined;
    if (!column || !link || !value || value.t === "null") return;
    const rect = anchor.getBoundingClientRect();
    setPeek({ id: `${column.name}:${valueText(value)}:${Date.now()}`, from: `${table.name}.${column.name}`, column: column.name, link, value, x: rect.left, y: rect.bottom + 4 });
  };
  const pkColumns = useMemo(() => columns.filter((c) => c.primaryKey), [columns]);
  const pkNames = useMemo(() => pkColumns.map((c) => c.name), [pkColumns]);

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
      if (rowIndex >= 0 && colIndex >= 0) map.set(`${rowIndex}:${colIndex}`, { value: c.newRaw !== undefined ? tokenValue(c.newRaw) : c.new, old: c.old, raw: c.newRaw });
    }
    return map;
  }, [tableChanges, rowIndexOf, columns]);
  const deletedRows = useMemo(() => new Set(tableChanges.filter((c) => c.kind === "delete").map((c) => rowIndexOf(c.key)).filter((i) => i >= 0)), [tableChanges, rowIndexOf]);
  const inserts = useMemo(() => tableChanges.filter((c): c is InsertChange => c.kind === "insert"), [tableChanges]);
  const allRows = useMemo(
    () => [
      ...rows,
      ...inserts.map((c) =>
        columns.map((col) => {
          const v = c.values.find((x) => x.column === col.name);
          return v ? (v.raw !== undefined ? tokenValue(v.raw) : v.value) : NULL_VALUE;
        }),
      ),
    ],
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
    (row: readonly Value[]): CellValue[] => pkColumns.map((c) => ({ column: c.name, value: row[columns.findIndex((col) => col.name === c.name)] ?? NULL_VALUE })),
    [pkColumns, columns],
  );

  // ---- truncated values --------------------------------------------------------

  const truncatedCells = useMemo(() => new Map((page?.truncated ?? []).map((t) => [`${t.row}:${t.col}`, t.size])), [page]);
  const truncatedRows = useMemo(() => new Set((page?.truncated ?? []).map((t) => t.row)), [page]);
  /// The grid's own coordinates for the cut cells: the grid skips hidden columns.
  const gridTruncated = useMemo(() => {
    const map = new Map<string, number>();
    for (const t of page?.truncated ?? []) {
      const gridCol = columnIndexes.indexOf(t.col);
      if (gridCol >= 0) map.set(`${t.row}:${gridCol}`, t.size);
    }
    return map;
  }, [page, columnIndexes]);
  const full = fullRows.key === requestKey ? fullRows.rows : NO_FULL_ROWS;

  // WHAT:  Reads one row whole (every cell in full) by primary key.
  // WHY:   A cell was cut for the page; opening or editing it needs the real value.
  const readFullRow = useCallback(
    async (rowIndex: number): Promise<Value[] | null> => {
      const row = rows[rowIndex];
      if (!row || pkColumns.length === 0) return null;
      try {
        const set = await ipc("fetch_rows", { connectionId, table, keyColumns: pkNames, keys: [keyOf(row).map((k) => k.value)] });
        const found = set.rows[0];
        if (!found) return null;
        return columns.map((c) => found[set.columns.findIndex((m) => m.name === c.name)] ?? NULL_VALUE);
      } catch (raw) {
        showError(normalizeError(raw));
        return null;
      }
    },
    [rows, pkColumns.length, pkNames, keyOf, columns, connectionId, table, showError],
  );
  const storeFullRow = useCallback(
    (rowIndex: number, whole: Value[]) => setFullRows((prev) => ({ key: requestKey, rows: { ...(prev.key === requestKey ? prev.rows : NO_FULL_ROWS), [rowIndex]: whole } })),
    [requestKey],
  );
  const loadFullRow = useCallback(
    async (rowIndex: number): Promise<readonly Value[] | null> => {
      const cached = full[rowIndex];
      if (cached) return cached;
      const whole = await readFullRow(rowIndex);
      if (whole) storeFullRow(rowIndex, whole);
      return whole;
    },
    [full, readFullRow, storeFullRow],
  );
  const loadFullCell = useCallback(
    async (gridRow: number, gridCol: number): Promise<Value | null> => {
      const pageCol = columnIndexes[gridCol];
      if (pageCol === undefined) return null;
      if (pkColumns.length === 0) {
        showError("This value was cut for the page and the table has no primary key to load it by.");
        return null;
      }
      const whole = await loadFullRow(gridRow);
      return whole?.[pageCol] ?? null;
    },
    [columnIndexes, pkColumns.length, loadFullRow, showError],
  );

  // ---- committing ---------------------------------------------------------------

  const applyChanges = async (changes: StagedChange[]) => {
    if (settings?.executionMode === "direct") {
      const attempt = await commitPending(connectionId, changes, false);
      if (attempt.status === "confirm" && window.confirm(`These statements are destructive:\n\n${attempt.statements.join("\n")}\n\nRun them?`)) {
        await commitPending(connectionId, changes, true);
      }
    } else {
      for (const c of changes) stageChange(connectionId, c);
    }
  };

  // WHAT:  After a commit that only changed cells, brings those rows up to date in place.
  // WHY:   Reloading the page throws away the scroll position and re-reads fifty
  //        rows to learn about one. The statements' RETURNING rows (Postgres, SQLite,
  //        DuckDB) hold the new state; elsewhere one fetch by key does. Anything else
  //        a commit did (insert, delete, a statement) reloads the page as before.
  const live = useRef({ page, columns, pkNames });
  useEffect(() => {
    live.current = { page, columns, pkNames };
  });
  useEffect(
    () =>
      onCommit((event) => {
        if (event.connectionId !== connectionId) return;
        const mine = event.changes.filter((c) => tableKey(c.table) === tableKey(table));
        const updates = mine.filter((c): c is UpdateChange => c.kind === "update");
        if (mine.length === 0 || updates.length !== mine.length) return;
        const { page: current, columns: cols, pkNames: keys } = live.current;
        if (!current) return;
        event.handled.add(tableKey(table));
        void (async () => {
          try {
            const byRow = new Map<string, { key: CellValue[]; edits: UpdateChange[] }>();
            for (const u of updates) {
              const id = JSON.stringify(u.key);
              const group = byRow.get(id) ?? { key: u.key, edits: [] };
              group.edits.push(u);
              byRow.set(id, group);
            }
            const patches = new Map<number, Value[]>();
            const lookups: { rowIndex: number; key: Value[] }[] = [];
            for (const group of byRow.values()) {
              const rowIndex = current.rows.findIndex((r) => group.key.every((k) => JSON.stringify(r[cols.findIndex((c) => c.name === k.column)]) === JSON.stringify(k.value)));
              if (rowIndex < 0) continue;
              const last = group.edits[group.edits.length - 1];
              const returned = last ? event.report.results.find((r) => r.id === last.id)?.returned : undefined;
              const row = returned?.rows[0];
              if (returned && row) {
                patches.set(rowIndex, cols.map((c) => row[returned.columns.indexOf(c.name)] ?? NULL_VALUE));
              } else {
                // The key may itself have been edited: look the row up by its new key.
                lookups.push({ rowIndex, key: group.key.map((k) => group.edits.find((e) => e.column === k.column)?.new ?? k.value) });
              }
            }
            if (lookups.length > 0) {
              if (keys.length === 0) throw new Error("no primary key to look the rows up by");
              const set = await ipc("fetch_rows", { connectionId, table, keyColumns: keys, keys: lookups.map((l) => l.key) });
              for (const lookup of lookups) {
                const hit = set.rows.find((r) => keys.every((k, i) => JSON.stringify(r[set.columns.findIndex((m) => m.name === k)]) === JSON.stringify(lookup.key[i])));
                if (!hit) throw new Error("a changed row could not be read back");
                patches.set(lookup.rowIndex, cols.map((c) => hit[set.columns.findIndex((m) => m.name === c.name)] ?? NULL_VALUE));
              }
            }
            setLoaded((prev) =>
              prev.page
                ? { ...prev, page: { ...prev.page, rows: prev.page.rows.map((r, i) => patches.get(i) ?? r), truncated: prev.page.truncated.filter((t) => !patches.has(t.row)) } }
                : prev,
            );
            setFullRows({ key: "", rows: NO_FULL_ROWS });
          } catch {
            setRefresh((r) => r + 1);
          }
        })();
      }),
    [connectionId, table],
  );

  // WHAT:  Applies one or many cell edits (inline edit, paste, fill, inspector) as a batch.
  // WHY:   A 50-cell paste must be one commit in direct mode and one staging
  //        pass otherwise; and several edits to the same staged insert have to
  //        merge, not overwrite each other from a stale `inserts` snapshot.
  const onCellsEdit = (edits: readonly CellEdit[]) => {
    const updates: StagedChange[] = [];
    const insertEdits = new Map<number, Map<string, { value: Value; raw: SqlToken | undefined }>>();
    let blocked: string | null = null;
    for (const { row: rowIndex, col: colIndex, value: next, raw } of edits) {
      const column = columns[colIndex];
      if (!column) continue;
      if (rowIndex >= rows.length) {
        // Ghost row: the edit rewrites the staged insert itself (same id → replaced in place).
        const byColumn = insertEdits.get(rowIndex) ?? new Map<string, { value: Value; raw: SqlToken | undefined }>();
        byColumn.set(column.name, { value: next, raw });
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
      if (truncatedCells.has(`${rowIndex}:${colIndex}`) && full[rowIndex] === undefined && next.t !== "null" && raw === undefined) {
        // The page holds only the start of the value this would be compared with.
        blocked = "That value was cut for the page; open the cell to load it before pasting over it.";
        continue;
      }
      const old = (full[rowIndex] ?? row)[colIndex] ?? NULL_VALUE;
      const key = keyOf(row);
      if (raw === undefined && JSON.stringify(old) === JSON.stringify(next)) {
        // Back to the original value: drop any staged edit for this cell instead of adding one.
        const existing = pending.find((c) => c.kind === "update" && tableKey(c.table) === tableKey(table) && c.column === column.name && JSON.stringify(c.key) === JSON.stringify(key));
        if (existing) unstageChange(connectionId, existing.id);
        continue;
      }
      updates.push({ kind: "update", id: nextChangeId(), table, key, column: column.name, old, new: next, ...(raw !== undefined ? { newRaw: raw } : {}) });
    }
    for (const [rowIndex, byColumn] of insertEdits) {
      const insert = inserts[rowIndex - rows.length];
      if (!insert) continue;
      const values = columns.flatMap((col): CellValue[] => {
        const edited = byColumn.get(col.name);
        if (edited !== undefined) return [{ column: col.name, value: edited.value, ...(edited.raw !== undefined ? { raw: edited.raw } : {}) }];
        const existing = insert.values.find((v) => v.column === col.name);
        return existing ? [existing] : [];
      });
      stageChange(connectionId, { ...insert, values });
    }
    if (blocked !== null) showError(blocked);
    if (updates.length > 0) void applyChanges(updates);
  };

  // WHAT:  Deletes rows: a staged insert is simply dropped, a real row becomes a staged delete.
  const deleteRows = (indexes: readonly number[]) => {
    for (const i of indexes) {
      const insert = i >= rows.length ? inserts[i - rows.length] : undefined;
      if (insert) unstageChange(connectionId, insert.id);
    }
    const real = indexes.filter((i) => i < rows.length && !deletedRows.has(i));
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
  const deleteSelected = () => deleteRows([...selectedRows]);
  // Row actions from the cell menu are staged changes like every other edit, so
  // review mode and the Changes panel still see them.
  const deleteRow = (rowIndex: number) => deleteRows([rowIndex]);

  const duplicateRow = (rowIndex: number) => {
    const row = allRows[rowIndex];
    if (!row) return;
    // A duplicate is an insert of every column except those the database fills in
    // again (auto-increment keys, generated columns).
    const values: CellValue[] = columns
      .map((c, i) => ({ column: c.name, value: row[i] ?? NULL_VALUE, traits: traitsOf(traitList, c.name) }))
      .filter(({ traits }) => !traits.autoIncrement && !traits.generated)
      .map(({ column, value }) => ({ column, value }));
    void applyChanges([{ kind: "insert", id: nextChangeId(), table, values }]);
  };

  // WHAT:  Pasted JSON rows or header-led TSV become new rows.
  const pasteRows = (text: string): boolean => {
    if (!editable) return false;
    const parsed = parsePastedRows(text, inspectorColumns);
    if (!parsed.ok) {
      if (parsed.error !== null) showError(parsed.error);
      return parsed.error !== null;
    }
    const changes: StagedChange[] = parsed.rows.map((cells) => ({
      kind: "insert",
      id: nextChangeId(),
      table,
      values: columns.flatMap((c): CellValue[] => {
        const entry = cells.get(c.name);
        return entry ? [{ column: c.name, value: entry.value, ...(entry.raw !== undefined ? { raw: entry.raw } : {}) }] : [];
      }),
    }));
    if (changes.length === 0) return false;
    void applyChanges(changes);
    const ignored = parsed.dropped.length > 0 ? `; ignored ${parsed.dropped.slice(0, 3).join(", ")}${parsed.dropped.length > 3 ? "…" : ""} (not columns here)` : "";
    showInfo(`${settings?.executionMode === "direct" ? "Inserted" : "Staged"} ${changes.length} new row${changes.length === 1 ? "" : "s"}${ignored}.`);
    return true;
  };

  // WHAT:  Candidate rows for a foreign-key cell: the referenced table, filtered
  //        by what the user typed, with each row's other columns as the detail.
  // WHY:   An id says nothing about which row it is; picking one should not mean
  //        opening the other table and copying a value back.
  const lookupFk = useCallback(
    async (gridColumn: number, search: string): Promise<LookupRow[]> => {
      const column = columns[pageColumn(gridColumn)];
      const link = column ? links.get(column.name) : undefined;
      if (!link) return [];
      const lookupFilters: FilterRule[] = search.trim().length > 0 ? [{ column: link.column, op: "contains", value: search.trim() }] : [];
      const found = await ipc("fetch_table_page", { connectionId, table: link.table, query: { sort: [], filters: lookupFilters, offset: 0, limit: LOOKUP_LIMIT }, skipCount: true, truncateOver: 200 });
      const keyIndex = found.columns.findIndex((c) => c.name === link.column);
      if (keyIndex < 0) return [];
      return found.rows.map((row) => ({
        value: cellText(row[keyIndex]),
        detail: found.columns
          .map((c, i) => ({ c, i }))
          .filter(({ i }) => i !== keyIndex)
          .slice(0, LOOKUP_DETAIL_COLUMNS)
          .map(({ c, i }) => `${c.name}: ${cellText(row[i])}`)
          .join(" · "),
      }));
    },
    [columns, pageColumn, links, connectionId],
  );

  // ---- copy & export ------------------------------------------------------------

  // WHAT:  The rows to copy or export, whole: a page with cut cells is read again without the cut.
  // WHY:   A cell shortened for the grid must never leave the app as if it were the value.
  const wholePage = async (): Promise<{ columns: ColumnInfo[]; rows: Value[][] } | null> => {
    if (!page) return null;
    if (page.truncated.length === 0) return { columns: page.columns, rows: page.rows };
    const again = await ipc("fetch_table_page", {
      connectionId,
      table,
      query: { sort, filters, offset: pageIndex * limit, limit },
      ...(effectiveMode === "any" ? { filterMode: effectiveMode } : {}),
      skipCount: true,
    });
    return { columns: again.columns, rows: again.rows };
  };
  const chosenOf = (all: Value[][]): Value[][] => (selectedRows.size > 0 ? all.filter((_, i) => selectedRows.has(i)) : all);
  /// Where SQL INSERT statements for these rows are written: this connection's dialect.
  const sqlTarget = { connectionId, table };

  // WHAT:  "Ask AI about this row": the row's values go to the chat as a prompt.
  // WHY:   Offered only when the AI may read rows (Settings → AI → data access).
  const askAiAboutRow = (rowIndex: number) => {
    const row = allRows[rowIndex];
    if (row === undefined) return;
    const values = columns.map((_, i) => {
      const value = row[i];
      return value === undefined ? "" : formatCell(value).text;
    });
    useWorkspace.getState().openChat(connectionId, askAboutRowPrompt(tableKey(table), columns.map((c) => c.name), values));
  };

  const copyRowsFrom = async (rowIndex: number, format: RowCopyFormat) => {
    try {
      const whole = await wholePage();
      if (!whole) return;
      const indexes = selectedRows.size > 0 ? [...selectedRows].sort((a, b) => a - b) : [rowIndex];
      // A staged insert is not in the database yet: it is copied as the grid shows it.
      const chosen = indexes.map((i) => (i < whole.rows.length ? whole.rows[i] : allRows[i])).filter((r): r is Value[] => r !== undefined);
      if (chosen.length === 0) return;
      await navigator.clipboard.writeText(await rowsAsText(format, whole.columns, chosen, sqlTarget));
      showInfo(`Copied ${chosen.length} row(s) as ${copyLabel(format)}.`);
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  const toggleSort = (column: string) => {
    setPageIndex(0);
    setSelectedRows(new Set());
    setSortEntries((current) =>
      nextSort(
        current.filter((e) => e.enabled).map((e) => e.rule),
        column,
      ).map((rule) => ({ rule, enabled: true })),
    );
  };
  // Shift+click on a header: another rule; a second one flips it, a third removes it.
  const addSort = (column: string) => {
    setPageIndex(0);
    setSortEntries((current) => {
      const at = current.findIndex((e) => e.rule.column === column);
      if (at < 0) return [...current, { rule: { column, desc: false }, enabled: true }];
      const existing = current[at];
      if (existing && !existing.rule.desc && existing.enabled) return current.map((e, i) => (i === at ? { ...e, rule: { column, desc: true } } : e));
      return current.filter((_, i) => i !== at);
    });
  };

  const copyAs = async (format: ExportFormat) => {
    try {
      const whole = await wholePage();
      if (!whole) return;
      const chosen = chosenOf(whole.rows);
      await navigator.clipboard.writeText(await rowsAsText(format, whole.columns, chosen, sqlTarget));
      showInfo(`Copied ${chosen.length} row(s) as ${copyLabel(format)} to the clipboard.`);
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  const downloadPage = async (format: ExportFormat) => {
    try {
      const whole = await wholePage();
      if (!whole) return;
      const chosen = chosenOf(whole.rows);
      const text = await rowsAsText(format, whole.columns, chosen, sqlTarget);
      downloadTextFile(exportFilename(tableKey(table), format), text, EXPORT_FORMAT_FACTS[format].mime);
      showInfo(`Downloaded ${chosen.length} row(s) as ${copyLabel(format)}.`);
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  // WHAT:  Every row matching the current sort + filters, page by page.
  // WHY:   Copy/export-page silently drops everything off-screen; the Transfer
  //        tab already streams huge tables server-side, so this caps at
  //        MAX_EXPORT_ROWS and points beyond that at Transfer.
  // HOW:   Same fetch_table_page every engine honours, 1k rows at a time (whole
  //        values, nothing cut), with the filters combined the way the grid has
  //        them; stops on a short page or an exact total.
  const loadAllRows = async (): Promise<{ columns: ColumnInfo[]; rows: Value[][]; capped: boolean }> => {
    const all: Value[][] = [];
    let fetchedColumns: ColumnInfo[] = columns;
    let offset = 0;
    let capped = false;
    for (;;) {
      const chunk = await ipc("fetch_table_page", {
        connectionId,
        table,
        query: { sort, filters, offset, limit: EXPORT_PAGE },
        ...(effectiveMode === "any" ? { filterMode: effectiveMode } : {}),
        skipCount: offset > 0,
      });
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
    return { columns: fetchedColumns, rows: all, capped };
  };
  const [exporting, setExporting] = useState(false);
  const downloadAll = async (format: ExportFormat) => {
    if (exporting) return;
    setExporting(true);
    try {
      const everything = await loadAllRows();
      const text = await rowsAsText(format, everything.columns, everything.rows, sqlTarget);
      downloadTextFile(exportFilename(`${tableKey(table)}-all`, format), text, EXPORT_FORMAT_FACTS[format].mime);
      showInfo(
        everything.capped
          ? `Downloaded the first ${formatCount(everything.rows.length)} rows as ${copyLabel(format)}. Use the Transfer tab for larger exports.`
          : `Downloaded ${formatCount(everything.rows.length)} row(s) as ${copyLabel(format)}.`,
      );
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setExporting(false);
    }
  };

  // The export dialog's "which rows" choices; the page and the selection are read whole when a cell was cut.
  const exportScopes = (): ExportScope[] => {
    const chosen = rows.filter((_, i) => selectedRows.has(i));
    const scopes = tableScopes({ selected: chosen, page: rows, total, loadAll: async () => (await loadAllRows()).rows });
    if (!page || page.truncated.length === 0) return scopes;
    return scopes.map((scope) =>
      scope.id === "all"
        ? scope
        : {
            ...scope,
            load: async () => {
              const whole = await wholePage();
              return whole ? (scope.id === "selection" ? chosenOf(whole.rows) : whole.rows) : [];
            },
          },
    );
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

  // ---- find in grid -------------------------------------------------------------

  const found = useMemo(() => {
    if (!find.open || find.query.length === 0) return { matches: [], capped: false };
    return findInGrid(
      allRows.length,
      gridColumns.length,
      (r, c) => {
        const value = getGridRow(r)?.[c];
        return value === undefined || value.t === "null" ? null : formatCell(value).text;
      },
      find.query,
      find.matchCase,
    );
  }, [find.open, find.query, find.matchCase, allRows.length, gridColumns.length, getGridRow]);
  const findKeys = useMemo(() => new Set(found.matches.map(matchKey)), [found]);
  const findIndex = found.matches.length === 0 ? 0 : Math.min(find.index, found.matches.length - 1);
  const findCurrent = find.open && found.matches.length > 0 ? (found.matches[findIndex] ?? null) : null;

  // ---- shortcuts (this tab only) -------------------------------------------------

  /// Grid shortcuts act on the focused tab: the keyboard must be inside it (or nowhere in particular).
  const inThisTab = (): boolean => {
    const active = document.activeElement;
    return active === null || active === document.body || (root.current?.contains(active) ?? false);
  };
  useShortcut("refresh-table", () => {
    if (inThisTab()) setRefresh((r) => r + 1);
  });
  useShortcut("focus-filter", () => {
    if (inThisTab()) filterInput.current?.focus();
  });
  useShortcut("find-in-grid", () => {
    if (!inThisTab()) return;
    setFind((f) => ({ ...f, open: true }));
    findInput.current?.focus();
    findInput.current?.select();
  });
  useShortcut("apply-changes", () => {
    if (!inThisTab() || pending.length === 0) return;
    void (async () => {
      const attempt = await commitPending(connectionId, pending, false);
      // A destructive statement needs its confirmation dialog: open the panel where it lives.
      if (attempt.status === "confirm") setChangesPanelOpen(true);
    })();
  });

  // ---- inspector ----------------------------------------------------------------

  const inspectedIndexes = useMemo(() => {
    if (selectedRows.size > 1) return [...selectedRows].filter((i) => i < rows.length).sort((a, b) => a - b);
    return cell ? [cell.row] : [];
  }, [selectedRows, rows.length, cell]);
  // WHAT:  A row as the inspector shows it: the database's values with the staged edits laid over.
  // WHY:   The grid paints a staged cell with its new value; an inspector that went back to
  //        the old one after "Update" would look as if the edit had been lost.
  const shownRow = useCallback(
    (index: number): readonly Value[] | undefined => {
      const base = index < rows.length ? (full[index] ?? rows[index]) : allRows[index];
      if (base === undefined || index >= rows.length || !base.some((_, c) => staged.has(`${index}:${c}`))) return base;
      return base.map((value, c) => staged.get(`${index}:${c}`)?.value ?? value);
    },
    [rows, full, allRows, staged],
  );
  const inspectedRows = useMemo(() => inspectedIndexes.map(shownRow).filter((r): r is readonly Value[] => r !== undefined), [inspectedIndexes, shownRow]);
  // Names the rows shown, so unsaved edits in the inspector's form are dropped when another row is picked.
  const inspectorIdentity = useMemo(
    () =>
      inspectedIndexes
        .map((i) => {
          const row = rows[i];
          if (row === undefined) return `new:${inserts[i - rows.length]?.id ?? i}`;
          return pkNames.length > 0 ? JSON.stringify(keyOf(row).map((k) => k.value)) : `row:${pageIndex}:${i}`;
        })
        .join("|"),
    [inspectedIndexes, rows, inserts, pkNames, keyOf, pageIndex],
  );
  const inspectedCut = inspectedIndexes.some((i) => truncatedRows.has(i) && full[i] === undefined);
  const primaryIndex = inspectedIndexes[0];
  const inspectedKey = inspectedIndexes.join(",");
  useEffect(() => {
    // The inspector reads whole rows: fetch the ones that hold cut cells.
    let cancelled = false;
    for (const i of inspectedKey.length === 0 ? [] : inspectedKey.split(",").map(Number)) {
      if (i < rows.length && truncatedRows.has(i) && full[i] === undefined) {
        void readFullRow(i).then((whole) => {
          if (whole && !cancelled) storeFullRow(i, whole);
        });
      }
    }
    return () => {
      cancelled = true;
    };
  }, [inspectedKey, rows.length, truncatedRows, full, readFullRow, storeFullRow]);
  const selectedColumn = cell ? inspectorColumns[cell.col] : undefined;
  const selectedRowValues = cell ? shownRow(cell.row) : undefined;
  const selectedValue = cell ? selectedRowValues?.[cell.col] : undefined;

  const inspectorEdit: InspectorEdit | undefined = (() => {
    if (!page) return undefined;
    const ghost = inspectedIndexes.some((i) => i >= rows.length);
    const reason = readOnly
      ? "This connection is read-only."
      : !editableEngine
        ? "Rows of this engine cannot be edited from the grid."
        : pkColumns.length === 0
          ? "This table has no primary key, so its rows cannot be edited safely."
          : ghost
            ? "This row is a staged insert: edit it in the grid."
            : deletedRows.has(primaryIndex ?? -1)
              ? "This row is staged for deletion."
              : undefined;
    return {
      connectionId,
      keyColumns: pkNames,
      ...(reason !== undefined ? { disabledReason: reason } : {}),
      onApply: (edits: readonly InspectorCellEdit[]) => {
        const mapped: CellEdit[] = [];
        for (const e of edits) {
          const row = inspectedIndexes[e.row];
          if (row !== undefined) mapped.push({ row, col: e.col, value: e.value, raw: e.raw });
        }
        onCellsEdit(mapped);
      },
      onDelete: () => deleteRows(inspectedIndexes),
    };
  })();

  return (
    <div ref={root} className="relative flex h-full min-h-0 flex-col">
      <ScrollArea orientation="horizontal" hideScrollBar className="flex app-toolbar shrink-0 items-center gap-0.5 border-b border-border/40 glass-header">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button size="xs" variant="soft" disabled={!editable || columns.length === 0} onClick={() => setInsertOpen(true)}>
              <Icon name="plus" size={12} />
              Insert
            </Button>
          </TooltipTrigger>
          <TooltipContent>{readOnly ? "Connection is read-only." : editableEngine ? "Insert a row" : `Rows of ${engineMeta(engine).label} cannot be edited from the grid.`}</TooltipContent>
        </Tooltip>
        <Button size="xs" variant="toolbar" onClick={() => setRefresh((r) => r + 1)}>
          <Icon name="refresh" size={12} />
          Refresh
        </Button>
        <FilterPopover
          columns={columns.map((c) => ({ name: c.name, typeName: c.dataType }))}
          filters={filters}
          onApply={(next) => {
            setPageIndex(0);
            setFilterEntries(next.map((rule) => ({ rule, enabled: true })));
          }}
          entries={filterEntries}
          mode={filterMode}
          onApplyEntries={(entries, mode) => {
            setPageIndex(0);
            setSelectedRows(new Set());
            setFilterEntries(entries);
            setFilterMode(mode);
          }}
        />
        <SortPopover
          columns={columns.map((c) => ({ name: c.name, typeName: c.dataType }))}
          entries={sortEntries}
          onChange={(entries) => {
            setPageIndex(0);
            setSelectedRows(new Set());
            setSortEntries(entries);
          }}
        />
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
              {(
                [
                  { id: "copy", label: `Copy ${selectedRows.size > 0 ? "selection" : "page"} as`, formats: COPY_FORMATS, run: copyAs },
                  { id: "download-page", label: `Download ${selectedRows.size > 0 ? "selection" : "page"} as`, formats: COPY_FORMATS, run: downloadPage },
                  { id: "download-all", label: `Download all${total !== null ? ` ${formatCount(total)}` : ""} rows as`, formats: DOWNLOAD_ALL_FORMATS, run: downloadAll },
                ] as const
              ).map((group) => (
                <DropdownMenuSub key={group.id}>
                  <DropdownMenuSubTrigger>{group.label}</DropdownMenuSubTrigger>
                  <DropdownMenuPortal>
                    <DropdownMenuSubContent className="min-w-40">
                      {group.formats.map((format) => (
                        <DropdownMenuItem key={format} textValue={`${group.label} ${copyLabel(format)}`} onSelect={() => void group.run(format)}>
                          <span>{copyLabel(format)}</span>
                        </DropdownMenuItem>
                      ))}
                    </DropdownMenuSubContent>
                  </DropdownMenuPortal>
                </DropdownMenuSub>
              ))}
              <DropdownMenuSeparator />
              <DropdownMenuItem textValue="Export with options" onSelect={() => { setExportOpen(true); }}><span>Export with options…</span></DropdownMenuItem>
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <ExportDialog open={exportOpen} onOpenChange={setExportOpen} connectionId={connectionId} title={tableKey(table)} baseName={tableKey(table)} table={table} columns={columns} scopes={exportScopes()} />
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
          <Button size="xs" variant="danger-soft" className="liquid-hover" onClick={deleteSelected}>
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
          <Segmented
            label="View"
            value={view}
            onChange={setView}
            options={[
              { value: "grid", label: "Grid" },
              { value: "cards", label: "Cards" },
            ]}
          />
          <IconButton icon="columns" label={cell === null ? "Inspect selected cell" : inspectorCollapsed ? "Show inspector" : "Hide inspector"} active={cell !== null && !inspectorCollapsed} disabled={cell === null} onClick={toggleInspector} />
          <Separator orientation="vertical" className="mx-0.5 h-4 opacity-50" />
          <Pager
            pageIndex={pageIndex}
            pageCount={pageCount}
            hasNext={hasNext}
            onPage={(i) => {
              setSelectedRows(new Set());
              setPageIndex(Math.max(0, i));
            }}
            pageSize={pageSize}
            onPageSize={(size) => {
              setPageIndex(0);
              setPageSizeState(size);
              writePageSize(connectionId, size);
            }}
            total={total}
            totalExact={totalExact}
            loadedRows={rows.length}
          />
        </div>
      </ScrollArea>

      <FilterBar
        columns={columns.map((c) => ({ name: c.name, typeName: c.dataType }))}
        entries={filterEntries}
        mode={filterMode}
        inputRef={filterInput}
        onApply={(entries, mode) => {
          setPageIndex(0);
          setSelectedRows(new Set());
          setFilterEntries(entries);
          setFilterMode(mode);
        }}
      />

      <div className="relative flex min-h-0 flex-1">
        <div className="min-w-0 flex-1">
          {loaded.error !== null ? (
            <EmptyState icon="table" title="Could not load table" body={loaded.error} action={<Button size="sm" onClick={() => setRefresh((r) => r + 1)}>Retry</Button>} />
          ) : view === "cards" ? (
            <CardView
              columns={inspectorColumns.filter((c) => !hiddenColumns.has(c.name))}
              rows={allRows}
              columnIndexes={columnIndexes}
              selected={cell?.row ?? null}
              onSelect={(row) => {
                setCell({ row, col: cell?.col ?? 0 });
                setInspectorCollapsed(false);
              }}
              deletedRows={deletedRows}
              insertedFrom={rows.length}
              nullDisplay={settings?.nullDisplay ?? "NULL"}
            />
          ) : (
            <DataGrid
              columns={gridColumns}
              rowCount={allRows.length}
              getRow={getGridRow}
              rowHeight={DENSITIES[density].rowHeight}
              sort={sort}
              onSortToggle={toggleSort}
              onSortAdd={addSort}
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
              selected={cell ? { row: cell.row, col: Math.max(0, columnIndexes.indexOf(cell.col)) } : null}
              {...(editable
                ? {
                    onCellEdit: (row: number, col: number, next: Value, raw?: SqlToken) => onCellsEdit([{ row, col: pageColumn(col), value: next, raw }]),
                    onCellsEdit: (edits: readonly CellEdit[]) => onCellsEdit(edits.map((e) => ({ ...e, col: pageColumn(e.col) }))),
                    onPasteRows: pasteRows,
                    allowDefaultToken: canSetDefault,
                  }
                : {})}
              onPasteNotice={(message) => showInfo(message)}
              staged={stagedForGrid(staged, columnIndexes)}
              deletedRows={deletedRows}
              insertedFrom={rows.length}
              nullDisplay={settings?.nullDisplay ?? "NULL"}
              onLinkOpen={(row, col) => openLinked(row, pageColumn(col))}
              onLinkPeek={(row, col, anchor) => peekLinked(row, pageColumn(col), anchor)}
              onLinkEdit={setLinkEdit}
              onHideColumn={(name) => setHiddenColumns((h) => new Set([...h, name]))}
              onFilter={addFilter}
              onSortSet={(rule) => {
                setPageIndex(0);
                setSortEntries([{ rule, enabled: true }]);
              }}
              onClearSort={() => setSortEntries([])}
              onCopied={(what) => showInfo(`${what} copied to the clipboard.`)}
              alternatingRows={settings?.alternatingRows ?? true}
              onInspect={(row, col) => {
                setCell({ row, col: pageColumn(col) });
                setInspectorCollapsed(false);
              }}
              onCopyRows={(row, format) => void copyRowsFrom(row, format)}
              {...(canAskAboutRow(settings) ? { onAskAi: askAiAboutRow } : {})}
              onLookup={lookupFk}
              truncated={gridTruncated}
              onLoadFull={loadFullCell}
              findMatches={findKeys}
              findCurrent={findCurrent}
              {...(editable ? { onInsertRow: () => setInsertOpen(true), onDuplicateRow: duplicateRow, onDeleteRow: deleteRow } : {})}
            />
          )}
        </div>
        {find.open ? (
          <FindBar
            query={find.query}
            onQuery={(query) => setFind((f) => ({ ...f, query, index: 0 }))}
            matchCase={find.matchCase}
            onMatchCase={(matchCase) => setFind((f) => ({ ...f, matchCase, index: 0 }))}
            total={found.matches.length}
            index={findIndex}
            capped={found.capped}
            onNext={() => setFind((f) => ({ ...f, index: stepMatch(findIndex, found.matches.length, 1) }))}
            onPrev={() => setFind((f) => ({ ...f, index: stepMatch(findIndex, found.matches.length, -1) }))}
            onClose={() => setFind((f) => ({ ...f, open: false }))}
            inputRef={findInput}
          />
        ) : null}
        {selectedRowValues && selectedValue !== undefined && selectedColumn && inspectedRows.length > 0 ? (
          <RecordInspector
            columns={inspectorColumns}
            row={inspectedRows[0] ?? selectedRowValues}
            rows={inspectedRows}
            column={selectedColumn}
            value={selectedValue}
            table={table}
            tabs={settings?.inspectorTabs ?? ["fields", "json", "sql"]}
            activeTab={inspectorTab}
            onTab={setInspectorTab}
            collapsed={inspectorCollapsed}
            onToggle={toggleInspector}
            onClose={() => setCell(null)}
            edit={inspectorEdit}
            connectionId={connectionId}
            partial={inspectedCut}
            identity={inspectorIdentity}
            onCopied={(what) => showInfo(`${what} copied to the clipboard.`)}
            onError={(message) => showError(message)}
          />
        ) : null}
      </div>

      <InsertRowModal
        open={insertOpen}
        onClose={() => setInsertOpen(false)}
        columns={columns}
        traits={traitList}
        onSubmit={(values) => {
          setInsertOpen(false);
          void applyChanges([{ kind: "insert", id: nextChangeId(), table, values }]);
        }}
      />

      {peek ? (
        <FkPeekCard
          connectionId={connectionId}
          target={peek}
          onClose={() => setPeek(null)}
          onOpenTable={(link, value) => {
            setPeek(null);
            openLink(link, value);
          }}
          onPin={(target) => {
            setPinned((list) => [...list, { ...target, id: `pin-${Date.now()}-${list.length}` }]);
            setPeek(null);
          }}
          onRemap={(target) => {
            setPeek(null);
            setLinkEdit(target.column);
          }}
        />
      ) : null}
      {pinned.map((target, index) => (
        <FkPinnedPanel key={target.id} connectionId={connectionId} target={target} offset={index} onClose={(id) => setPinned((list) => list.filter((p) => p.id !== id))} onOpenTable={openLink} />
      ))}
      {linkEdit !== null && catalog ? (
        <FkRemapDialog connectionId={connectionId} owner={table} column={linkEdit} catalog={catalog} current={links.get(linkEdit)} onClose={() => setLinkEdit(null)} onError={(message) => showError(message)} />
      ) : null}
    </div>
  );
}

/// Staged edits are keyed by page column; the grid asks by its own (hidden columns removed).
function stagedForGrid(staged: ReadonlyMap<string, StagedCell>, columnIndexes: readonly number[]): ReadonlyMap<string, StagedCell> {
  if (columnIndexes.every((pageCol, gridCol) => pageCol === gridCol)) return staged;
  const byPage = new Map(columnIndexes.map((pageCol, gridCol) => [pageCol, gridCol]));
  const out = new Map<string, StagedCell>();
  for (const [key, value] of staged) {
    const [row = "", col = ""] = key.split(":");
    const gridCol = byPage.get(Number(col));
    if (gridCol !== undefined) out.set(`${row}:${gridCol}`, value);
  }
  return out;
}
