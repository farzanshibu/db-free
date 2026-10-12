// SOT: collection-editor, redis-hash-editor, redis-list-editor, redis-set-editor, redis-zset-editor, row-edit-add-remove
import { useCallback, useEffect, useState } from "react";
import type { FilterRule, Value } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { AppSelect } from "@/components/global/Field";
import { EmptyState } from "@/components/global/EmptyState";
import { Button } from "@/components/ui/button";
import { Input, SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { formatCount } from "@/lib/format";
import { exportCellText } from "@/lib/export";
import {
  addCommands,
  COLLECTION_COLUMNS,
  draftReady,
  editCommands,
  FILTER_COLUMN,
  removalMarker,
  removeCommands,
  type CollectionKind,
  type Draft,
  type EntryRow,
} from "./collectionCommands";
import { ConfirmPopover } from "./ConfirmPopover";
import { keyRef, runRedis } from "./keyCommands";

// WHAT:  The editor for a hash, list, set or sorted set: its rows in a table, a
//        search box, and add / edit / remove for each row.
// WHY:   These four types were read-only in the grid; changing one member meant
//        typing HSET / LSET / SADD / ZADD by hand in a query tab.
// HOW:   Rows are read a page at a time through `fetch_table_page` (the adapter
//        windows a list with LRANGE; the others load the key and slice). Every
//        change is a command line from `collectionCommands.ts` sent through
//        `execute_query`, after which the first page is read again so the table
//        shows what the server holds. Removing a row is confirmed on the button.
// WHERE: src-tauri/src/integrations/redis.rs (fetch_page), src/features/keys/KeyTab.tsx
const PAGE = 200;
const NO_ROWS: readonly Row[] = [];

interface Row extends EntryRow {
  /// A cell holds bytes that are not text: shown, not editable.
  binary: boolean;
}

interface CollectionEditorProps {
  connectionId: string;
  keyName: string;
  kind: CollectionKind;
  readOnly: boolean;
  /// A write changed the key (its size, perhaps its existence): refresh the header.
  onMutated: () => void;
}

function cellString(value: Value): string {
  return value.t === "bytes" ? `(binary, ${Math.floor((value.v.length * 3) / 4)} bytes)` : exportCellText(value);
}

// WHAT:  A row's identity is its first cell: a list's index, a hash's field, a set's
//        or sorted set's member.
function toRow(cells: readonly Value[]): Row {
  const texts = cells.map(cellString);
  return { id: texts[0] ?? "", cells: texts, binary: cells.some((c) => c.t === "bytes") };
}

const GRID: Record<CollectionKind, string> = {
  hash: "grid-cols-[minmax(8rem,1fr)_minmax(12rem,3fr)_2.5rem]",
  list: "grid-cols-[4rem_minmax(12rem,1fr)_2.5rem]",
  set: "grid-cols-[minmax(12rem,1fr)_2.5rem]",
  zset: "grid-cols-[minmax(12rem,1fr)_7rem_2.5rem]",
};

const EMPTY_DRAFT: Draft = { name: "", value: "", head: false };

export function CollectionEditor({ connectionId, keyName, kind, readOnly, onMutated }: CollectionEditorProps) {
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  // The rows read for one request (`key`), so "loading" is whether the request
  // now wanted has been answered rather than a flag set from inside an effect.
  const [view, setView] = useState<{ key: string; rows: readonly Row[]; total: number | null }>({ key: "", rows: NO_ROWS, total: null });
  const [version, setVersion] = useState(0);
  const [more, setMore] = useState(false);
  const [filter, setFilter] = useState("");
  const [applied, setApplied] = useState("");
  const [editing, setEditing] = useState<{ id: string; column: number; text: string } | null>(null);
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const [busy, setBusy] = useState(false);
  const columns = COLLECTION_COLUMNS[kind];

  // The search box applies after a short pause, not on every keystroke.
  useEffect(() => {
    const id = window.setTimeout(() => {
      setApplied(filter.trim());
    }, 300);
    return () => {
      window.clearTimeout(id);
    };
  }, [filter]);

  const fetchPage = useCallback(
    async (offset: number, needle: string) => {
      const filters: FilterRule[] = needle === "" ? [] : [{ column: FILTER_COLUMN[kind], op: "contains", value: needle }];
      // A sorted set reads best by score; the others in their own order.
      const sort = kind === "zset" ? [{ column: "score", desc: false }] : [];
      return ipc("fetch_table_page", { connectionId, table: keyRef(keyName), query: { sort, filters, offset, limit: PAGE } });
    },
    [connectionId, keyName, kind],
  );

  const requestKey = `${applied}\u0000${version}`;
  const loading = view.key !== requestKey;
  const { rows, total } = view;

  useEffect(() => {
    const token = { stale: false };
    void (async () => {
      try {
        const page = await fetchPage(0, applied);
        if (!token.stale) setView({ key: requestKey, rows: page.rows.map(toRow), total: page.total });
      } catch (raw) {
        if (token.stale) return;
        showError(normalizeError(raw));
        setView((v) => ({ ...v, key: requestKey }));
      }
    })();
    return () => {
      token.stale = true;
    };
  }, [fetchPage, applied, requestKey, showError]);

  // Re-reads the first page after a write.
  const reload = () => {
    setVersion((v) => v + 1);
  };

  const loadMore = async () => {
    const asked = requestKey;
    setMore(true);
    try {
      const page = await fetchPage(rows.length, applied);
      setView((v) => (v.key === asked ? { ...v, rows: [...v.rows, ...page.rows.map(toRow)] } : v));
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setMore(false);
    }
  };

  // WHAT:  Sends command lines, then re-reads the table and tells the header.
  const write = async (lines: readonly string[], message: string): Promise<boolean> => {
    setBusy(true);
    try {
      await runRedis(connectionId, lines);
      showInfo(message);
      reload();
      onMutated();
      return true;
    } catch (raw) {
      showError(normalizeError(raw));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const commitEdit = async () => {
    if (editing === null) return;
    const row = rows.find((r) => r.id === editing.id);
    if (row === undefined) return;
    const lines = editCommands(kind, keyName, row, editing.column, editing.text);
    if (lines === null) {
      showError(kind === "zset" && editing.column === 1 ? "A score must be a number (or +inf / -inf)." : "That cell cannot be edited.");
      return;
    }
    if ((row.cells[editing.column] ?? "") === editing.text) {
      setEditing(null);
      return;
    }
    if (await write(lines, "Saved.")) setEditing(null);
  };

  const remove = (row: Row) => {
    void write(removeCommands(kind, keyName, row, removalMarker()), `Removed ${row.cells[0] ?? ""}.`);
  };

  const add = async () => {
    if (!draftReady(kind, draft)) return;
    if (await write(addCommands(kind, keyName, draft), "Added.")) setDraft({ ...EMPTY_DRAFT, head: draft.head });
  };

  const rowKey = (row: Row, index: number) => `${index}:${row.id}`;
  const shown = rows.length;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border px-2.5 py-1.5">
        <SearchInput value={filter} onChange={setFilter} aria-label={`Filter by ${FILTER_COLUMN[kind]}`} placeholder={`Filter by ${FILTER_COLUMN[kind]}…`} className="w-52 text-xs" />
        <span className="text-xs text-muted">{total === null ? `${formatCount(shown)} loaded` : `${formatCount(shown)} of ${formatCount(total)}`}</span>
        {loading ? <Spinner size="sm" /> : null}
        {!readOnly ? (
          <div className="ml-auto flex min-w-0 flex-1 items-center justify-end gap-2">
            {kind === "hash" ? <DraftInput label="Field" value={draft.name} onChange={(name) => setDraft({ ...draft, name })} onEnter={() => void add()} /> : null}
            {kind === "set" || kind === "zset" ? <DraftInput label="Member" value={draft.name} onChange={(name) => setDraft({ ...draft, name })} onEnter={() => void add()} /> : null}
            {kind === "zset" ? <DraftInput label="Score" value={draft.value} onChange={(value) => setDraft({ ...draft, value })} onEnter={() => void add()} narrow /> : null}
            {kind === "hash" || kind === "list" ? <DraftInput label="Value" value={draft.value} onChange={(value) => setDraft({ ...draft, value })} onEnter={() => void add()} /> : null}
            {kind === "list" ? (
              <AppSelect
                ariaLabel="Add at"
                value={draft.head ? "head" : "tail"}
                options={[
                  { value: "tail", label: "Append (tail)" },
                  { value: "head", label: "Prepend (head)" },
                ]}
                onChange={(where) => setDraft({ ...draft, head: where === "head" })}
                size="sm"
                className="w-36"
              />
            ) : null}
            <Button size="sm" onClick={() => void add()} disabled={busy || !draftReady(kind, draft)}>
              <Icon name="plus" size={12} />
              Add
            </Button>
          </div>
        ) : null}
      </div>
      {rows.length === 0 && !loading ? (
        <EmptyState icon="table" title={applied === "" ? "This key is empty" : "No rows match"} body={applied === "" ? "Redis deletes a collection when its last element goes." : `Nothing contains "${applied}".`} />
      ) : (
        <ScrollArea className="min-h-0 flex-1">
          <div className={cn("sticky top-0 z-10 grid items-center gap-2 border-b border-border bg-surface px-2.5 py-1 text-[11px] font-medium uppercase tracking-wide text-muted", GRID[kind])}>
            {columns.map((c) => (
              <span key={c.name}>{c.name}</span>
            ))}
            <span />
          </div>
          {rows.map((row, index) => (
            <div key={rowKey(row, index)} className={cn("group grid items-start gap-2 border-b border-border/40 px-2.5 py-1 text-xs hover:bg-surface-secondary/50", GRID[kind])}>
              {columns.map((column, c) =>
                editing !== null && editing.id === row.id && editing.column === c ? (
                  <CellEditor
                    key={column.name}
                    text={editing.text}
                    busy={busy}
                    onChange={(text) => setEditing({ ...editing, text })}
                    onCommit={() => void commitEdit()}
                    onCancel={() => setEditing(null)}
                  />
                ) : (
                  <button
                    key={column.name}
                    type="button"
                    disabled={readOnly || !column.editable || row.binary}
                    title={row.cells[c]}
                    onClick={() => setEditing({ id: row.id, column: c, text: row.cells[c] ?? "" })}
                    className={cn(
                      "min-w-0 truncate rounded-sm px-1 py-0.5 text-left font-mono",
                      !readOnly && column.editable && !row.binary ? "cursor-text hover:bg-surface-tertiary" : "cursor-default",
                      c === 0 && kind !== "set" && kind !== "zset" ? "text-muted" : "text-foreground",
                    )}
                  >
                    {row.cells[c] === "" ? <span className="text-muted/60">(empty)</span> : row.cells[c]}
                  </button>
                ),
              )}
              {!readOnly ? (
                <ConfirmPopover
                  title={`Remove ${kind === "list" ? `element ${row.cells[0] ?? ""}` : `"${row.cells[0] ?? ""}"`}?`}
                  body={kind === "list" ? "The element at this position is taken out of the list." : "This changes the key on the server."}
                  confirmLabel="Remove"
                  onConfirm={() => {
                    remove(row);
                  }}
                  trigger={
                    <Button variant="ghost" size="icon-sm" aria-label="Remove row" disabled={busy} className="justify-self-end opacity-0 group-hover:opacity-100 focus-visible:opacity-100">
                      <Icon name="trash" size={13} />
                    </Button>
                  }
                />
              ) : (
                <span />
              )}
            </div>
          ))}
          {total !== null && shown < total ? (
            <div className="flex items-center justify-center p-2.5">
              <Button size="sm" variant="secondary" pending={more} onClick={() => void loadMore()}>
                Load more ({formatCount(total - shown)} left)
              </Button>
            </div>
          ) : null}
        </ScrollArea>
      )}
    </div>
  );
}

function DraftInput({ label, value, onChange, onEnter, narrow = false }: { label: string; value: string; onChange: (value: string) => void; onEnter: () => void; narrow?: boolean }) {
  return (
    <Input
      aria-label={label}
      placeholder={label}
      value={value}
      onChange={(event) => {
        onChange(event.target.value);
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter") onEnter();
      }}
      className={cn("min-w-0 font-mono text-xs", narrow ? "w-24" : "w-full max-w-64")}
    />
  );
}

// WHAT:  The in-place editor of one cell. Enter saves a single line; a value with
//        line breaks gets a text area, which saves with Ctrl / Cmd + Enter.
function CellEditor({ text, busy, onChange, onCommit, onCancel }: { text: string; busy: boolean; onChange: (text: string) => void; onCommit: () => void; onCancel: () => void }) {
  const multiline = text.includes("\n");
  return (
    <div className="flex min-w-0 items-start gap-1">
      {multiline ? (
        <Textarea
          autoFocus
          aria-label="Edit value"
          value={text}
          rows={4}
          spellCheck={false}
          onChange={(event) => {
            onChange(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") onCancel();
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) onCommit();
          }}
          className="min-w-0 flex-1 font-mono text-xs"
        />
      ) : (
        <Input
          autoFocus
          aria-label="Edit value"
          value={text}
          spellCheck={false}
          onChange={(event) => {
            onChange(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") onCancel();
            if (event.key === "Enter") onCommit();
          }}
          className="h-6 min-w-0 flex-1 font-mono text-xs"
        />
      )}
      <Button size="icon-sm" aria-label="Save" disabled={busy} onClick={onCommit}>
        <Icon name="check" size={13} />
      </Button>
      <Button size="icon-sm" variant="ghost" aria-label="Cancel" onClick={onCancel}>
        <Icon name="x" size={13} />
      </Button>
    </div>
  );
}
