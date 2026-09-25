// SOT: result-grid, editable-result, result-edit-target, result-edit-staging, result-view-state, result-quick-filter, result-inspector
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CellValue, ColumnInfo, FilterRule, ResultSet, SchemaCatalog, SortRule, StagedChange, TableRef, Value } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { analyzeSelect, type EditTarget } from "@/lib/editableSelect";
import { engineMeta } from "@/lib/engines";
import { DENSITIES, formatCount } from "@/lib/format";
import { Icon } from "@/lib/icons";
import { tableKey, useWorkspace } from "@/stores/workspace";
import { DataGrid, type CellEdit, type StagedCell } from "@/features/grid/DataGrid";
import { FilterPopover } from "@/features/grid/FilterPopover";
import { ColumnsPopover } from "@/features/grid/ColumnsPopover";
import { RecordInspector, useInspectorCollapsed, type InspectorColumn } from "@/features/grid/RecordInspector";
import { nextSort, viewIndexes } from "@/features/grid/clientRows";
import { IconButton } from "@/components/global/Button";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

// Stable empties: a selector must return the same reference for unchanged state.
const EMPTY_CHANGES: StagedChange[] = [];
const NULL_VALUE: Value = { t: "null" };

let changeCounter = 0;
function nextChangeId(): string {
  changeCounter += 1;
  return `res-${Date.now()}-${changeCounter}`;
}

/// Whether a result can be written back, and how its columns map onto the table.
type EditState =
  | { status: "checking" }
  | { status: "readonly"; reason: string }
  | {
      status: "editable";
      table: TableRef;
      /// Per result column: the table column it is, or null (computed, ambiguous, not in the table).
      columnOf: readonly (string | null)[];
      /// Primary-key columns with the result column that carries each.
      keys: readonly { column: string; index: number }[];
    };

function sameName(a: string, b: string, exact: boolean): boolean {
  return exact ? a === b : a.toLowerCase() === b.toLowerCase();
}

// WHAT:  The table a SELECT names, resolved against the loaded catalog.
// WHY:   Staged changes are grouped by `tableKey`, so a result edit and the
//        table tab on the same table must agree on the exact ref: unquoted
//        names fold case, and an unqualified name takes the schema the query
//        ran in (or the only schema that has it).
function resolveTable(target: EditTarget, catalog: SchemaCatalog | undefined, schemaFilter: string | null): TableRef {
  const name = target.parts.at(-1) ?? "";
  const nameQuoted = target.quoted.at(-1) ?? false;
  const schema = target.parts.length >= 2 ? (target.parts.at(-2) ?? null) : null;
  const schemaQuoted = target.parts.length >= 2 ? (target.quoted.at(-2) ?? false) : false;
  const tables = (catalog?.schemas ?? []).flatMap((s) => s.tables.map((t) => ({ schema: t.schema, name: t.name })));
  const candidates = tables.filter((t) => sameName(t.name, name, nameQuoted) && (schema === null || (t.schema !== null && sameName(t.schema, schema, schemaQuoted))));
  const exact = candidates.filter((t) => t.name === name);
  const pool = exact.length > 0 ? exact : candidates;
  const preferred = pool.find((t) => t.schema === schemaFilter) ?? pool.find((t) => t.schema === "public") ?? (pool.length === 1 ? pool[0] : undefined);
  return preferred ?? { schema, name };
}

// WHAT:  Result column -> table column, and the primary key's place in the row.
// HOW:   A column is writable only when it is a plain reference in the select
//        list (or comes from `*`), its name is unique in the result, and the
//        table really has it. Every key column must be present that way, or no
//        row could be addressed.
function mapColumns(target: EditTarget, table: TableRef, result: ResultSet, tableColumns: readonly ColumnInfo[]): EditState {
  const names = result.columns.map((c) => c.name);
  const lookup = <T,>(map: ReadonlyMap<string, T>, key: string): T | undefined => map.get(key) ?? [...map].find(([k]) => k.toLowerCase() === key.toLowerCase())?.[1];
  const computed = new Set([...target.computed].map((c) => c.toLowerCase()));
  const columnOf = names.map((name) => {
    if (names.filter((n) => n === name).length > 1 || computed.has(name.toLowerCase())) return null;
    const source = lookup(target.columns, name) ?? (target.star ? name : undefined);
    if (source === undefined) return null;
    return (tableColumns.find((c) => c.name === source) ?? tableColumns.find((c) => c.name.toLowerCase() === source.toLowerCase()))?.name ?? null;
  });
  const pk = tableColumns.filter((c) => c.primaryKey);
  if (pk.length === 0) return { status: "readonly", reason: `${table.name} has no primary key` };
  const keys = pk.map((c) => ({ column: c.name, index: columnOf.indexOf(c.name) }));
  const missing = keys.filter((k) => k.index < 0).map((k) => k.column);
  if (missing.length > 0) return { status: "readonly", reason: `the primary key (${missing.join(", ")}) is not in the result` };
  return { status: "editable", table, columnOf, keys };
}

interface ResultGridProps {
  connectionId: string;
  /// The statement that produced `result`; decides whether it can be edited.
  sql: string;
  result: ResultSet;
  /// Only a script with exactly one statement can be written back.
  single: boolean;
}

// WHAT:  A query result as a working grid: client-side sort, quick search,
//        per-column filter rules, column visibility, record inspector, and
//        inline edit / delete when the result is one table's rows.
// WHY:   A result is where users look at data most; having to reopen the same
//        rows in a table tab to fix one value, or re-run with ORDER BY to read
//        them another way, is the friction this removes.
// HOW:   Rows never move: a view (search + filters + sort) yields source row
//        indexes, and every grid coordinate is mapped back through it, so an
//        edit lands on the row that was clicked whatever the order. Edits go
//        the TableTab way: staged in review mode, committed at once in direct
//        mode. After a commit the SELECT is re-run (it is a plain single-table
//        SELECT, checked by analyzeSelect) so the grid shows what was stored.
// WHERE: src/lib/editableSelect.ts, src/features/grid/TableTab.tsx, src/features/grid/clientRows.ts
export function ResultGrid({ connectionId, sql, result: initial, single }: ResultGridProps) {
  const density = useWorkspace((s) => s.density);
  const settings = useWorkspace((s) => s.settings);
  const engine = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.engine ?? "postgres");
  const readOnly = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.readOnly ?? false);
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const schemaFilter = useWorkspace((s) => s.schemaFilter[connectionId] ?? null);
  const loadColumns = useWorkspace((s) => s.loadColumns);
  const pending = useWorkspace((s) => s.pendingChanges[connectionId] ?? EMPTY_CHANGES);
  const stageChange = useWorkspace((s) => s.stageChange);
  const unstageChange = useWorkspace((s) => s.unstageChange);
  const showInfo = useWorkspace((s) => s.showInfo);
  const showError = useWorkspace((s) => s.showError);

  // Re-run output replaces the original rows; the columns stay the same statement's.
  const [fresh, setFresh] = useState<ResultSet | null>(null);
  const result = fresh ?? initial;
  const rows = result.rows;
  const columns = result.columns;

  const [search, setSearch] = useState("");
  const [filters, setFilters] = useState<FilterRule[]>([]);
  const [sort, setSort] = useState<SortRule[]>([]);
  const [hidden, setHidden] = useState<ReadonlySet<string>>(new Set());
  // Selections are kept as source indexes so re-sorting never moves them.
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [cell, setCell] = useState<{ row: number; col: number } | null>(null);
  const [inspectorTab, setInspectorTab] = useState<string>(settings?.inspectorTabs[0] ?? "fields");
  const [inspectorCollapsed, setInspectorCollapsed, toggleInspector] = useInspectorCollapsed();

  // WHAT:  Can this result be written back? Checked once per statement.
  // HOW:   Cheap rules first (engine, read-only, statement shape), then the
  //        table's columns from the store cache for the primary key.
  const early = useMemo((): { reason: string } | { target: EditTarget; table: TableRef } => {
    if (engineMeta(engine).commandLanguage !== "SQL") return { reason: "editing is only available for SQL engines" };
    if (readOnly) return { reason: "the connection is read-only" };
    if (!single) return { reason: "the script has more than one statement" };
    const shape = analyzeSelect(sql);
    if (!shape.editable) return { reason: shape.reason };
    return { target: shape.target, table: resolveTable(shape.target, catalog, schemaFilter) };
  }, [engine, readOnly, single, sql, catalog, schemaFilter]);
  const checkKey = "table" in early ? tableKey(early.table) : "";
  const [checked, setChecked] = useState<{ key: string; state: EditState } | null>(null);
  useEffect(() => {
    if (!("table" in early)) return;
    const token = { cancelled: false };
    const { target, table } = early;
    const settle = (state: EditState) => {
      if (!token.cancelled) setChecked({ key: tableKey(table), state });
    };
    void (async () => {
      try {
        const tableColumns = await loadColumns(connectionId, table);
        settle(tableColumns.length === 0 ? { status: "readonly", reason: `${table.name} is not a table this connection can see` } : mapColumns(target, table, initial, tableColumns));
      } catch (raw) {
        settle({ status: "readonly", reason: `the table's columns could not be loaded (${normalizeError(raw).message})` });
      }
    })();
    return () => {
      token.cancelled = true;
    };
  }, [early, loadColumns, connectionId, initial]);
  const edit: EditState = "reason" in early ? { status: "readonly", reason: early.reason } : checked?.key === checkKey ? checked.state : { status: "checking" };
  const editable = edit.status === "editable" ? edit : null;

  // WHAT:  The view: source row order after search, filters and sort, and the
  //        visible column indexes. Inverse maps turn source indexes into grid ones.
  const order = useMemo(() => viewIndexes(columns, rows, { search, filters, sort }), [columns, rows, search, filters, sort]);
  const viewOf = useMemo(() => new Map(order.map((src, view) => [src, view])), [order]);
  const shownCols = useMemo(() => columns.map((c, i) => ({ c, i })).filter(({ c }) => !hidden.has(c.name)).map(({ i }) => i), [columns, hidden]);

  const keyOf = useCallback((row: readonly Value[]): CellValue[] => (editable ? editable.keys.map((k) => ({ column: k.column, value: row[k.index] ?? NULL_VALUE })) : []), [editable]);
  const tableChanges = useMemo(() => (editable ? pending.filter((c) => tableKey(c.table) === tableKey(editable.table)) : []), [pending, editable]);
  const sourceRowOf = useCallback(
    (key: readonly CellValue[]) => rows.findIndex((r) => key.every((k) => JSON.stringify(r[editable?.keys.find((e) => e.column === k.column)?.index ?? -1]) === JSON.stringify(k.value))),
    [rows, editable],
  );

  // WHAT:  Staged changes for this table drawn on the grid: edited cells (every
  //        result column showing that table column) and rows staged for delete.
  const staged = useMemo(() => {
    const map = new Map<string, StagedCell>();
    if (!editable) return map;
    for (const c of tableChanges) {
      if (c.kind !== "update") continue;
      const view = viewOf.get(sourceRowOf(c.key));
      if (view === undefined) continue;
      shownCols.forEach((src, gridCol) => {
        if (editable.columnOf[src] === c.column) map.set(`${view}:${gridCol}`, { value: c.new, old: c.old });
      });
    }
    return map;
  }, [editable, tableChanges, viewOf, sourceRowOf, shownCols]);
  const deletedSource = useMemo(() => new Set(tableChanges.filter((c) => c.kind === "delete").map((c) => sourceRowOf(c.key)).filter((i) => i >= 0)), [tableChanges, sourceRowOf]);
  const deletedRows = useMemo(() => new Set([...deletedSource].map((s) => viewOf.get(s)).filter((v): v is number => v !== undefined)), [deletedSource, viewOf]);

  const getGridRow = useCallback(
    (view: number): readonly Value[] | undefined => {
      const row = rows[order[view] ?? -1];
      return row === undefined ? undefined : shownCols.map((c) => row[c] ?? NULL_VALUE);
    },
    [rows, order, shownCols],
  );
  const sourceCell = useCallback((view: number, gridCol: number) => ({ row: order[view] ?? -1, col: shownCols[gridCol] ?? -1 }), [order, shownCols]);

  // WHAT:  Re-reads the result after a commit, from the same statement.
  // WHY:   Defaults, triggers and rounding decide what was stored; showing the
  //        typed value would be a guess. Only an editable result re-runs, and
  //        that is a plain single-table SELECT.
  const touched = useRef(false);
  const reload = useCallback(async () => {
    try {
      const next = await ipc("execute_query", { connectionId, sql, confirmDestructive: false, maxRows: initial.truncated ? initial.rows.length : null, schema: schemaFilter });
      const first = next.statements[0];
      if (first?.kind === "rows") setFresh(first.result);
    } catch (raw) {
      showError(normalizeError(raw));
    }
  }, [connectionId, sql, initial, schemaFilter, showError]);
  // Pending Changes announces a commit with this event; only a grid that staged something re-reads.
  useEffect(() => {
    const onRefresh = () => {
      if (!touched.current) return;
      touched.current = false;
      void reload();
    };
    window.addEventListener("db-free:refresh-tables", onRefresh);
    return () => window.removeEventListener("db-free:refresh-tables", onRefresh);
  }, [reload]);

  const applyChanges = async (changes: StagedChange[]) => {
    if (settings?.executionMode === "direct") {
      try {
        await ipc("commit_changes", { connectionId, changes });
        showInfo(`Applied ${changes.length} change(s).`);
        await reload();
      } catch (raw) {
        showError(normalizeError(raw));
      }
    } else {
      touched.current = true;
      for (const c of changes) stageChange(connectionId, c);
    }
  };

  // WHAT:  Inline edit / paste / fill, as one batch (see TableTab.onCellsEdit).
  const onCellsEdit = (edits: readonly CellEdit[]) => {
    if (!editable) return;
    const updates: StagedChange[] = [];
    let blocked: string | null = null;
    for (const e of edits) {
      const at = sourceCell(e.row, e.col);
      const row = rows[at.row];
      const column = editable.columnOf[at.col];
      if (!row) continue;
      if (column === null || column === undefined) {
        blocked = `${columns[at.col]?.name ?? "This column"} is computed by the query, so it cannot be written back.`;
        continue;
      }
      if (deletedSource.has(at.row)) {
        blocked = "This row is staged for deletion. Undo the delete in Pending Changes first.";
        continue;
      }
      const old = row[at.col] ?? NULL_VALUE;
      const key = keyOf(row);
      if (JSON.stringify(old) === JSON.stringify(e.value)) {
        const existing = pending.find((c) => c.kind === "update" && tableKey(c.table) === tableKey(editable.table) && c.column === column && JSON.stringify(c.key) === JSON.stringify(key));
        if (existing) unstageChange(connectionId, existing.id);
        continue;
      }
      updates.push({ kind: "update", id: nextChangeId(), table: editable.table, key, column, old, new: e.value });
    }
    if (blocked !== null) showError(blocked);
    if (updates.length > 0) void applyChanges(updates);
  };

  const deleteSources = (sources: readonly number[]) => {
    if (!editable) return;
    const changes: StagedChange[] = sources
      .filter((s) => !deletedSource.has(s))
      .map((s) => rows[s])
      .filter((r): r is Value[] => r !== undefined)
      .map((r) => ({ kind: "delete", id: nextChangeId(), table: editable.table, key: keyOf(r) }));
    if (changes.length === 0) {
      setSelected(new Set());
      return;
    }
    if (settings?.executionMode === "direct" && !window.confirm(changes.length === 1 ? "Delete this row now?" : `Delete ${changes.length} row(s) now?`)) return;
    setSelected(new Set());
    void applyChanges(changes);
  };

  const addFilter = useCallback((rule: FilterRule) => setFilters((prev) => [...prev.filter((f) => !(f.column === rule.column && f.op === rule.op)), rule]), []);
  const gridSelected = useMemo(() => new Set([...selected].map((s) => viewOf.get(s)).filter((v): v is number => v !== undefined)), [selected, viewOf]);
  const gridCell = cell ? { row: viewOf.get(cell.row) ?? -1, col: shownCols.indexOf(cell.col) } : null;
  const inspectorColumns: InspectorColumn[] = useMemo(
    () => columns.map((c, i) => ({ name: c.name, dataType: c.typeName, primaryKey: editable?.keys.some((k) => k.index === i) ?? false })),
    [columns, editable],
  );
  const inspectedRow = cell ? rows[cell.row] : undefined;
  const inspectedValue = cell ? inspectedRow?.[cell.col] : undefined;
  const inspectedColumn = cell ? inspectorColumns[cell.col] : undefined;
  const narrowed = order.length !== rows.length;
  const gridColumns = useMemo(
    () => shownCols.map((i) => ({ name: columns[i]?.name ?? "", typeName: columns[i]?.typeName ?? "", primaryKey: editable?.keys.some((k) => k.index === i) ?? false })),
    [shownCols, columns, editable],
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ScrollArea orientation="horizontal" hideScrollBar className="flex h-8 shrink-0 items-center gap-0.5 border-b border-border/40 px-1.5">
        <SearchInput value={search} onChange={setSearch} aria-label="Search the result" placeholder="Search result…" className="glass-input h-6 w-48 rounded-md text-xs" />
        <FilterPopover columns={columns} filters={filters} onApply={setFilters} />
        <ColumnsPopover columns={columns} hidden={hidden} onChange={setHidden} />
        <Button size="xs" variant={sort.length > 0 ? "soft" : "toolbar"} onClick={() => setSort([])} disabled={sort.length === 0}>
          <Icon name="sort" size={12} />
          {sort.length > 0 ? `Sorted by ${sort.map((s) => s.column).join(", ")}` : "Sort"}
        </Button>
        {selected.size > 0 && editable ? (
          <Button size="xs" variant="danger-soft" onClick={() => deleteSources([...selected])}>
            <Icon name="trash" size={12} />
            Delete {selected.size}
          </Button>
        ) : null}
        <div className="ml-auto flex shrink-0 items-center gap-2 pl-2 text-xs whitespace-nowrap text-muted">
          {narrowed ? <span className="tabular-nums font-mono text-[11px]">{formatCount(order.length)} of {formatCount(rows.length)} rows</span> : null}
          {edit.status === "editable" ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Badge size="sm" variant="soft" color="accent" className="gap-1 font-medium">
                  <Icon name="pencil" size={11} />
                  Editable · {tableKey(edit.table)}
                </Badge>
              </TooltipTrigger>
              <TooltipContent>Double-click a cell to edit. {settings?.executionMode === "direct" ? "Changes apply immediately." : "Changes are staged in Pending Changes."}</TooltipContent>
            </Tooltip>
          ) : edit.status === "readonly" ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Badge size="sm" variant="soft" className="gap-1">
                  <Icon name="lock" size={11} />
                  Read-only result
                </Badge>
              </TooltipTrigger>
              <TooltipContent>Not editable: {edit.reason}.</TooltipContent>
            </Tooltip>
          ) : null}
          <Separator orientation="vertical" className="mx-0.5 h-4 opacity-50" />
          <IconButton icon="columns" label={cell === null ? "Inspect selected cell" : inspectorCollapsed ? "Show inspector" : "Hide inspector"} active={cell !== null && !inspectorCollapsed} disabled={cell === null} onClick={toggleInspector} />
        </div>
      </ScrollArea>
      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1">
          <DataGrid
            columns={gridColumns}
            rowCount={order.length}
            getRow={getGridRow}
            rowHeight={DENSITIES[density].rowHeight}
            sort={sort}
            onSortToggle={(column) => setSort((current) => nextSort(current, column))}
            onSortSet={(rule) => setSort([rule])}
            onClearSort={() => setSort([])}
            onFilter={addFilter}
            {...(editable
              ? {
                  selectedRows: gridSelected,
                  onToggleRow: (view: number) => {
                    const src = order[view];
                    if (src === undefined) return;
                    setSelected((s) => {
                      const next = new Set(s);
                      if (next.has(src)) next.delete(src);
                      else next.add(src);
                      return next;
                    });
                  },
                  onToggleAll: () => setSelected((s) => (s.size === order.length ? new Set() : new Set(order))),
                  onCellEdit: (row: number, col: number, next: Value) => onCellsEdit([{ row, col, value: next }]),
                  onCellsEdit,
                  onDeleteRow: (view: number) => {
                    const src = order[view];
                    if (src !== undefined) deleteSources([src]);
                  },
                }
              : {})}
            onPasteNotice={(message) => showInfo(message)}
            staged={staged}
            deletedRows={deletedRows}
            onCellSelect={(row, col) => setCell(sourceCell(row, col))}
            selected={gridCell}
            onInspect={(row, col) => {
              setCell(sourceCell(row, col));
              setInspectorCollapsed(false);
            }}
            nullDisplay={settings?.nullDisplay ?? "NULL"}
            alternatingRows={settings?.alternatingRows ?? true}
            onCopied={(what) => showInfo(`${what} copied to the clipboard.`)}
          />
        </div>
        {inspectedRow && inspectedValue !== undefined && inspectedColumn ? (
          <RecordInspector
            columns={inspectorColumns}
            row={inspectedRow}
            column={inspectedColumn}
            value={inspectedValue}
            table={editable?.table ?? null}
            tabs={settings?.inspectorTabs ?? ["fields", "json", "sql"]}
            activeTab={inspectorTab}
            onTab={setInspectorTab}
            collapsed={inspectorCollapsed}
            onToggle={toggleInspector}
            onClose={() => setCell(null)}
          />
        ) : null}
      </div>
    </div>
  );
}
