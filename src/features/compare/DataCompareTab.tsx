// SOT: data-compare-tab, row-diff-grid, sync-script-open
import { useEffect, useMemo, useState } from "react";
import type { ColumnInfo, CompareDirection, DataCompare, RowDiff, RowStatus, TableRef, Value } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { DENSITIES, formatCount } from "@/lib/format";
import { useWorkspace } from "@/stores/workspace";
import { DataGrid, type GridColumn, type StagedCell } from "@/features/grid/DataGrid";
import { EmptyState } from "@/components/global/EmptyState";
import { Check, NumberInput, Segmented } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverHeading, PopoverTrigger } from "@/components/ui/popover";
import { ConnectionPicker, DirectionToggle, SchemaPicker, TablePicker, targetOf, useLiveConnections } from "./ComparePickers";

type Filter = "all" | RowStatus;

const DEFAULT_MAX_ROWS = 100_000;

interface Side {
  connectionId: string;
  schema: string | null;
  table: TableRef | null;
}

// WHAT:  How a row reads in the chosen direction: the script inserts rows only
//        in the source, deletes rows only in the target, updates the rest.
function rowLabel(status: RowStatus, direction: CompareDirection): string {
  if (status === "different") return "differs";
  const sourceOnly = (status === "only_left") === (direction === "left_to_right");
  return sourceOnly ? `${status === "only_left" ? "left" : "right"} only · insert` : `${status === "only_left" ? "left" : "right"} only · delete`;
}

// WHAT:  Data compare: two tables (same or different connections) matched by
//        key, the differing rows in a grid, and a sync script for the target.
// WHY:   Checking that two copies of a table agree — reference data across
//        environments, a migration's copy — row by row is otherwise manual.
// HOW:   `compare_table_data` resolves both sessions through the guard and
//        merge-joins the rows in Rust. The grid shows what the target will
//        hold after the sync: a changed cell carries the source value tinted
//        like a staged edit, with the target's current value as "was"; rows
//        the sync deletes are struck through, rows it inserts are green. The
//        script opens in a query tab on the target connection, so it runs
//        through the guarded execute path.
// WHERE: src-tauri/src/services/data_compare.rs, src-tauri/src/commands/compare.rs
export function DataCompareTab({ connectionId, table }: { connectionId: string; table: TableRef | null }) {
  const live = useLiveConnections();
  const density = useWorkspace((s) => s.density);
  const loadColumns = useWorkspace((s) => s.loadColumns);
  const openQuery = useWorkspace((s) => s.openQuery);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [left, setLeft] = useState<Side>({ connectionId, schema: table?.schema ?? null, table });
  const [right, setRight] = useState<Side>(() => ({ connectionId: live.find((c) => c.id !== connectionId)?.id ?? connectionId, schema: null, table }));
  const [direction, setDirection] = useState<CompareDirection>("left_to_right");
  const [keyColumns, setKeyColumns] = useState<string[]>([]);
  const [leftColumns, setLeftColumns] = useState<ColumnInfo[]>([]);
  const [maxRows, setMaxRows] = useState<number | null>(DEFAULT_MAX_ROWS);
  const [result, setResult] = useState<DataCompare | null>(null);
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState<Filter>("all");

  const leftTable = left.table;
  useEffect(() => {
    if (leftTable === null) return;
    const token = { cancelled: false };
    void loadColumns(left.connectionId, leftTable)
      .then((cols) => {
        if (!token.cancelled) setLeftColumns(cols);
      })
      .catch(() => {
        if (!token.cancelled) setLeftColumns([]);
      });
    return () => {
      token.cancelled = true;
    };
  }, [left.connectionId, leftTable, loadColumns]);

  const compare = async (includeScript: boolean): Promise<DataCompare | null> => {
    if (left.table === null || right.table === null) return null;
    setBusy(true);
    try {
      const out = await ipc("compare_table_data", {
        left: { connectionId: left.connectionId, table: left.table },
        right: { connectionId: right.connectionId, table: right.table },
        keyColumns,
        maxRows,
        direction,
        includeScript,
      });
      setResult(out);
      return out;
    } catch (raw) {
      showError(normalizeError(raw));
      return null;
    } finally {
      setBusy(false);
    }
  };

  const openScript = async () => {
    // The script is built on demand: most compares are only looked at.
    const cached = result?.script;
    const out = cached !== null && cached !== undefined ? result : await compare(true);
    if (out === null) return;
    if (out.script === null) {
      showError("The target engine takes no SQL, so there is no sync script.");
      return;
    }
    openQuery(targetOf(out.direction, left.connectionId, right.connectionId), out.script, "sync");
    showInfo("Review the sync script, then run it.");
  };

  const view = useMemo(() => (result ? buildGridView(result, filter, (status) => rowLabel(status, result.direction)) : null), [result, filter]);
  const pkNames = leftColumns.filter((c) => c.primaryKey).map((c) => c.name);
  const keyLabel = keyColumns.length > 0 ? keyColumns.join(", ") : pkNames.length > 0 ? `${pkNames.join(", ")} (primary key)` : "choose…";

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex app-toolbar shrink-0 flex-wrap items-center gap-2 border-b border-border bg-surface">
        <SidePicker label="Left" side={left} onChange={(next) => { setLeft(next); setKeyColumns([]); setResult(null); }} />
        <DirectionToggle value={direction} onChange={(next) => { setDirection(next); setResult(null); }} />
        <SidePicker label="Right" side={right} onChange={(next) => { setRight(next); setResult(null); }} />
      </div>
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border/40 px-3 py-1.5 text-xs text-muted">
        <span>Match rows by</span>
        <Popover>
          <PopoverTrigger asChild>
            <Button size="xs" variant="outline" disabled={leftColumns.length === 0}>
              <Icon name="key" size={12} />
              {keyLabel}
            </Button>
          </PopoverTrigger>
          <PopoverContent className="w-60">
            <PopoverHeading>Key columns</PopoverHeading>
            <p className="mb-2 text-[11px] text-muted">None checked = the primary key.</p>
            <div className="flex max-h-64 flex-col gap-1 overflow-auto">
              {leftColumns.map((c) => (
                <label key={c.name} className="flex items-center gap-2 text-[12px] text-foreground">
                  <Check
                    label={`Key column ${c.name}`}
                    checked={keyColumns.includes(c.name)}
                    onChange={(on) => { setKeyColumns((ks) => (on ? [...ks, c.name] : ks.filter((k) => k !== c.name))); setResult(null); }}
                  />
                  <span className="truncate">{c.name}</span>
                  {c.primaryKey ? <Icon name="key" size={11} className="ml-auto text-warning" /> : null}
                </label>
              ))}
            </div>
          </PopoverContent>
        </Popover>
        <span className="ml-2">Max rows per side</span>
        <NumberInput ariaLabel="Max rows per side" value={maxRows} onChange={setMaxRows} integer compact className="h-7 w-28" />
        <Button size="sm" pending={busy} disabled={left.table === null || right.table === null} onClick={() => void compare(false)} className="ml-auto">
          <Icon name="exchange" size={13} />
          Compare
        </Button>
        <Button size="sm" variant="soft" disabled={result === null || busy} onClick={() => void openScript()}>
          <Icon name="terminal" size={13} />
          Open sync script in query tab
        </Button>
      </div>

      {result === null || view === null ? (
        <EmptyState icon="exchange" title="Compare the rows of two tables" body="Pick a table on each side (the same or another connection), check the key the rows are matched by, then Compare. Only connected connections are listed." />
      ) : (
        <>
          <div className="flex shrink-0 flex-wrap items-center gap-2 px-3 py-2 text-xs">
            <Segmented
              label="Filter rows"
              value={filter}
              onChange={setFilter}
              options={[
                { value: "all", label: `All (${formatCount(result.onlyLeft + result.onlyRight + result.different)})` },
                { value: "different", label: `Different (${formatCount(result.different)})` },
                { value: "only_left", label: `Left only (${formatCount(result.onlyLeft)})` },
                { value: "only_right", label: `Right only (${formatCount(result.onlyRight)})` },
              ]}
            />
            <Badge size="sm" variant="outline" className="font-mono">
              {formatCount(result.identical)} identical
            </Badge>
            <span className="text-muted">
              read {formatCount(result.leftRows)}
              {result.leftCapped ? "+" : ""} left, {formatCount(result.rightRows)}
              {result.rightCapped ? "+" : ""} right · key {result.keyColumns.join(", ")}
            </span>
          </div>
          {result.notes.length > 0 || result.rowsTruncated ? (
            <ul className="shrink-0 space-y-0.5 border-y border-border/40 bg-warning-soft/40 px-3 py-1.5 text-[11.5px] text-warning">
              {result.rowsTruncated ? <li>Showing the first {formatCount(result.rows.length)} differing rows; the counts and the sync script cover all of them.</li> : null}
              {result.notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          ) : null}
          <div className="min-h-0 flex-1">
            {view.rows.length === 0 ? (
              <EmptyState icon="check" title={filter === "all" ? "The tables match" : "No rows in this filter"} body={filter === "all" ? `${formatCount(result.identical)} rows are identical on both sides.` : "Pick another filter to see the other differences."} />
            ) : (
              <DataGrid
                columns={view.columns}
                rowCount={view.rows.length}
                getRow={(i) => view.rows[i]?.cells}
                rowHeight={DENSITIES[density].rowHeight}
                staged={view.staged}
                deletedRows={view.deleted}
                insertedFrom={view.insertedFrom}
              />
            )}
          </div>
        </>
      )}
    </div>
  );
}

function SidePicker({ label, side, onChange }: { label: string; side: Side; onChange: (next: Side) => void }) {
  return (
    <>
      <span className="text-[11px] font-semibold uppercase tracking-wider text-muted">{label}</span>
      <ConnectionPicker label={`${label} connection`} value={side.connectionId} onChange={(id) => { onChange({ connectionId: id, schema: null, table: null }); }} />
      <SchemaPicker label={`${label} schema`} connectionId={side.connectionId} value={side.schema} onChange={(schema) => { onChange({ ...side, schema, table: null }); }} />
      <TablePicker label={`${label} table`} connectionId={side.connectionId} schema={side.schema} value={side.table} onChange={(t) => { onChange({ ...side, table: t }); }} />
    </>
  );
}

interface GridRow {
  cells: Value[];
}

// WHAT:  Grid rows in sync order — updates, then deletes, then inserts —
//        because DataGrid tints "inserted" rows as a tail from one index.
function buildGridView(out: DataCompare, which: Filter, label: (status: RowStatus) => string) {
  const leftIsSource = out.direction === "left_to_right";
  const picked = out.rows.filter((r) => which === "all" || r.status === which);
  const isInsert = (r: RowDiff) => r.status !== "different" && (r.status === "only_left") === leftIsSource;
  const updates = picked.filter((r) => r.status === "different");
  const deletes = picked.filter((r) => r.status !== "different" && !isInsert(r));
  const inserts = picked.filter(isInsert);
  const ordered = [...updates, ...deletes, ...inserts];
  const columns: GridColumn[] = [{ name: "·", typeName: "text" }, ...out.columns.map((c) => ({ name: c.name, typeName: c.dataType, primaryKey: out.keyColumns.includes(c.name) }))];
  const staged = new Map<string, StagedCell>();
  const deleted = new Set<number>();
  const rows: GridRow[] = ordered.map((r, index) => {
    const source = leftIsSource ? r.left : r.right;
    const target = leftIsSource ? r.right : r.left;
    const shown = source ?? target ?? [];
    if (r.status === "different" && target !== null) {
      out.columns.forEach((c, i) => {
        const now = shown[i];
        const was = target[i];
        if (r.changed.includes(c.name) && now !== undefined && was !== undefined) staged.set(`${index}:${i + 1}`, { value: now, old: was });
      });
    }
    if (index >= updates.length && index < updates.length + deletes.length) deleted.add(index);
    return { cells: [{ t: "text", v: label(r.status) }, ...shown] };
  });
  return { columns, rows, staged, deleted, insertedFrom: updates.length + deletes.length };
}
