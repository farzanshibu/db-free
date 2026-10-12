// SOT: history-tab, query-history-grid, history-filters, history-paging, history-open-table
import { useCallback, useEffect, useMemo, useState } from "react";
import type { HistoryEntry, HistoryOrigin, TableRef, Value } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { DENSITIES, formatCount } from "@/lib/format";
import { useWorkspace } from "@/stores/workspace";
import { DataGrid, type GridColumn } from "@/features/grid/DataGrid";
import { AppSelect, Segmented } from "@/components/global/Field";
import { ConfirmDialog } from "@/components/global/ConfirmDialog";
import { IconButton } from "@/components/global/Button";
import { EmptyState } from "@/components/global/EmptyState";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";

type Filter = "all" | "user" | "system";

const PAGE_SIZES = [
  { value: "25", label: "25 rows" },
  { value: "50", label: "50 rows" },
  { value: "100", label: "100 rows" },
  { value: "200", label: "200 rows" },
] satisfies readonly { value: string; label: string }[];

const COLUMNS: readonly GridColumn[] = [
  { name: "time", typeName: "timestamp" },
  { name: "status", typeName: "text" },
  { name: "source", typeName: "text" },
  { name: "operation", typeName: "text" },
  { name: "type", typeName: "text" },
  { name: "table", typeName: "text", linkTo: "table" },
  { name: "query", typeName: "text" },
  { name: "rows", typeName: "int" },
  { name: "ms", typeName: "int" },
  { name: "user", typeName: "text" },
  { name: "error", typeName: "text" },
];
const TABLE_COLUMN = COLUMNS.findIndex((c) => c.name === "table");

const OPERATIONS = { read: "Read", write: "Write", destructive: "Destructive" } satisfies Record<NonNullable<HistoryEntry["operation"]>, string>;

const text = (value: string | null): Value => (value === null ? { t: "null" } : { t: "text", v: value });

function rowOf(e: HistoryEntry): Value[] {
  return [
    { t: "date_time", v: new Date(e.executedAt).toLocaleString() },
    { t: "text", v: e.status === "ok" ? "Success" : "Error" },
    { t: "text", v: e.fromEditor ? "Editor" : e.origin === "system" ? "App" : "Other" },
    text(e.operation === null ? null : OPERATIONS[e.operation]),
    text(e.statementType),
    text(e.tableName),
    { t: "text", v: e.sql.replace(/\s+/g, " ") },
    e.rowCount === null ? { t: "null" } : { t: "int", v: e.rowCount },
    { t: "int", v: e.elapsedMs },
    text(e.user),
    text(e.error),
  ];
}

// WHAT:  The page of the query log the grid shows, and the counts that go with it.
// WHY:   The log can hold a hundred thousand rows; the server pages it, so the
//        grid never holds more than one page.
interface Page {
  entries: HistoryEntry[];
  total: number;
}

// WHAT:  Full-tab query log: All / System / User, a page of 25–200 rows from the
//        server with the total, a click on a row to reopen its SQL, and the table
//        column opens the table.
export function HistoryTab({ connectionId }: { connectionId: string }) {
  const density = useWorkspace((s) => s.density);
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const openQuery = useWorkspace((s) => s.openQuery);
  const openTable = useWorkspace((s) => s.openTable);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [filter, setFilter] = useState<Filter>("all");
  const [pageSize, setPageSize] = useState("50");
  const [pageIndex, setPageIndex] = useState(0);
  const [page, setPage] = useState<Page>({ entries: [], total: 0 });
  const [loaded, setLoaded] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [clearOpen, setClearOpen] = useState(false);
  const [clearing, setClearing] = useState(false);

  const size = Number(pageSize);

  useEffect(() => {
    const token = { cancelled: false };
    const origin: HistoryOrigin | null = filter === "all" ? null : filter;
    void (async () => {
      try {
        const result = await ipc("list_history_page", { connectionId, origin, fromEditor: null, limit: size, offset: pageIndex * size });
        if (!token.cancelled) {
          setPage({ entries: result.entries, total: result.total });
          setLoaded(true);
        }
      } catch (raw) {
        if (!token.cancelled) showError(normalizeError(raw));
      }
    })();
    return () => {
      token.cancelled = true;
    };
  }, [connectionId, filter, size, pageIndex, refresh, showError]);

  const rows = useMemo<Value[][]>(() => page.entries.map(rowOf), [page.entries]);
  const getRow = useCallback((i: number) => rows[i], [rows]);

  const pages = Math.max(1, Math.ceil(page.total / size));
  const first = page.total === 0 ? 0 : pageIndex * size + 1;
  const last = pageIndex * size + page.entries.length;

  // The table a log row names, as the sidebar knows it: an unqualified name lives in whichever schema holds it.
  const tableOf = (name: string): TableRef => {
    const dot = name.lastIndexOf(".");
    const wanted = dot < 0 ? name : name.slice(dot + 1);
    const schema = dot < 0 ? null : name.slice(0, dot);
    const known = (catalog?.schemas ?? []).flatMap((s) => s.tables).find((t) => t.name === wanted && (schema === null || t.schema === schema));
    return known === undefined ? { schema, name: wanted } : { schema: known.schema, name: known.name };
  };

  const clear = async () => {
    setClearing(true);
    try {
      const n = await ipc("clear_history", { connectionId });
      showInfo(`Removed ${formatCount(n)} history entries.`);
      setClearOpen(false);
      setPageIndex(0);
      setRefresh((r) => r + 1);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setClearing(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex app-toolbar shrink-0 items-center gap-2 border-b border-border bg-surface ">
        <Segmented
          label="History filter"
          value={filter}
          onChange={(next) => {
            setFilter(next);
            setPageIndex(0);
          }}
          options={[{ value: "all", label: "All" }, { value: "system", label: "System" }, { value: "user", label: "User" }]}
        />
        <IconButton icon="trash" label="Clear history" onClick={() => { setClearOpen(true); }} />
        <IconButton icon="refresh" label="Refresh" onClick={() => { setRefresh((r) => r + 1); }} />
        <span className="ml-auto text-xs text-muted">{page.total === 0 ? "No rows" : `${formatCount(first)}–${formatCount(last)} of ${formatCount(page.total)}`}</span>
      </div>
      <div className="min-h-0 flex-1">
        {loaded && page.entries.length === 0 && page.total > 0 ? (
          // Rows were removed while this page was open (or the filter narrowed): the page no longer exists.
          <EmptyState icon="history" title="No rows on this page" body="The log has fewer rows than it did." action={<Button size="sm" variant="secondary" onClick={() => { setPageIndex(0); }}>Back to the first page</Button>} />
        ) : loaded && page.entries.length === 0 ? (
          <EmptyState icon="history" title="No history yet" body="Statements you run (and the queries the app runs for table pages) appear here." action={<Button size="sm" variant="secondary" onClick={() => { openQuery(connectionId); }}><Icon name="terminal" size={13} />New query</Button>} />
        ) : (
          <DataGrid
            columns={COLUMNS}
            rowCount={rows.length}
            getRow={getRow}
            rowHeight={DENSITIES[density].rowHeight}
            onCellSelect={(row, col) => {
              if (col === TABLE_COLUMN) return;
              const entry = page.entries[row];
              if (entry) openQuery(connectionId, entry.sql);
            }}
            onLinkOpen={(row) => {
              const name = page.entries[row]?.tableName;
              if (name !== null && name !== undefined) openTable(connectionId, tableOf(name));
            }}
          />
        )}
      </div>
      <div className="flex h-7 shrink-0 items-center gap-2 border-t border-border bg-surface px-2.5 text-xs text-muted">
        <AppSelect ariaLabel="Rows per page" value={pageSize} options={PAGE_SIZES} size="sm" className="w-28" onChange={(next) => { setPageSize(next); setPageIndex(0); }} />
        <span className="ml-auto flex items-center gap-1">
          <IconButton icon="chevron-left" label="Previous page" disabled={pageIndex === 0} onClick={() => { setPageIndex((p) => Math.max(0, p - 1)); }} size={13} />
          <span className="min-w-20 text-center tabular-nums">
            Page {formatCount(pageIndex + 1)} of {formatCount(pages)}
          </span>
          <IconButton icon="chevron-right" label="Next page" disabled={pageIndex + 1 >= pages} onClick={() => { setPageIndex((p) => p + 1); }} size={13} />
        </span>
      </div>
      <ConfirmDialog
        open={clearOpen}
        onOpenChange={setClearOpen}
        title="Clear query history?"
        description="Every logged statement for this connection is deleted. This cannot be undone."
        confirmLabel="Clear history"
        pending={clearing}
        onConfirm={clear}
      />
    </div>
  );
}
