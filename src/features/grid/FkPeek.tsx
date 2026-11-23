// SOT: fk-peek, fk-peek-card, pinned-fk-panel, referenced-row-lookup
import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import type { ColumnInfo, TableRef, Value } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { IconButton } from "@/components/global/Button";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cellText } from "./RecordInspector";
import type { FkLink } from "./fkInference";

/// One foreign-key value being looked at: where it points, what it holds, where to draw it.
export interface PeekTarget {
  id: string;
  /// "orders.user_id", for the title.
  from: string;
  /// The source column ("user_id"), for changing its link.
  column: string;
  link: FkLink;
  value: Value;
  /// Where the card opens (viewport px).
  x: number;
  y: number;
}

type Lookup = { state: "loading" } | { state: "missing" } | { state: "error"; message: string } | { state: "ready"; columns: readonly ColumnInfo[]; row: readonly Value[] };

/// Rows are shown shortened: a card is for recognising a record, not reading a blob.
const PEEK_TRUNCATE = 200;
const MAX_FIELDS = 24;

// WHAT:  Reads the row a foreign-key value points at.
// WHY:   "user_id = 42" says nothing; the user's name and email say who it is.
// HOW:   A one-row page of the referenced table filtered to the key. `refresh` re-reads it.
function useReferencedRow(connectionId: string, link: FkLink, value: Value, refresh: number): Lookup {
  const [lookup, setLookup] = useState<{ key: string; result: Lookup } | null>(null);
  const key = `${link.table.schema ?? ""}.${link.table.name}.${link.column}:${cellText(value)}:${refresh}`;
  useEffect(() => {
    const token = { cancelled: false };
    void (async () => {
      try {
        const page = await ipc("fetch_table_page", {
          connectionId,
          table: link.table,
          query: { sort: [], filters: [{ column: link.column, op: "eq", value: cellText(value) }], offset: 0, limit: 1 },
          skipCount: true,
          truncateOver: PEEK_TRUNCATE,
        });
        if (token.cancelled) return;
        const row = page.rows[0];
        setLookup({ key, result: row === undefined ? { state: "missing" } : { state: "ready", columns: page.columns, row } });
      } catch (raw) {
        if (!token.cancelled) setLookup({ key, result: { state: "error", message: normalizeError(raw).message } });
      }
    })();
    return () => {
      token.cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `key` encodes every input
  }, [connectionId, key]);
  return lookup !== null && lookup.key === key ? lookup.result : { state: "loading" };
}

function tableLabel(table: TableRef): string {
  return table.schema === null ? table.name : `${table.schema}.${table.name}`;
}

function RecordBody({ lookup, link, value }: { lookup: Lookup; link: FkLink; value: Value }) {
  if (lookup.state === "loading") {
    return (
      <div className="flex flex-col gap-1.5 p-2.5" aria-busy="true">
        <span className="h-2 w-32 animate-pulse rounded-sm bg-surface-tertiary" />
        <span className="h-2 w-48 animate-pulse rounded-sm bg-surface-tertiary" />
        <span className="h-2 w-24 animate-pulse rounded-sm bg-surface-tertiary" />
      </div>
    );
  }
  if (lookup.state === "missing") {
    return (
      <div className="p-2.5 text-xs">
        <p className="font-medium text-warning">Record not found</p>
        <p className="mt-1 text-muted">
          {tableLabel(link.table)} has no row with {link.column} = {cellText(value)}.
        </p>
      </div>
    );
  }
  if (lookup.state === "error") return <p className="p-2.5 text-xs text-danger">{lookup.message}</p>;
  const shown = lookup.columns.slice(0, MAX_FIELDS);
  return (
    <ScrollArea className="max-h-64">
      <dl className="px-2.5 py-1.5 text-xs">
        {shown.map((column, index) => (
          <div key={`${column.name}-${index}`} className="flex gap-2 border-b border-separator/60 py-1 last:border-0">
            <dt className="flex w-24 shrink-0 items-center gap-1 truncate font-medium text-muted">
              {column.primaryKey ? <Icon name="key" size={10} className="shrink-0 text-warning" /> : null}
              <span className="truncate">{column.name}</span>
            </dt>
            <dd className="selectable min-w-0 flex-1 truncate font-mono text-foreground">{cellText(lookup.row[index])}</dd>
          </div>
        ))}
        {lookup.columns.length > MAX_FIELDS ? <p className="pt-1 text-[11px] text-muted">{lookup.columns.length - MAX_FIELDS} more columns — open the table to see them.</p> : null}
      </dl>
    </ScrollArea>
  );
}

interface CardProps {
  connectionId: string;
  target: PeekTarget;
  onClose: () => void;
  onOpenTable: (link: FkLink, value: Value) => void;
  onRemap?: ((target: PeekTarget) => void) | undefined;
}

const CARD_WIDTH = 320;

// WHAT:  The peek card next to a foreign-key cell: the referenced record's fields,
//        "Open in table", "Pin" and "Change link…".
// HOW:   Opens at the cell, clamped into the window; closes on Escape or a click
//        outside it. A guessed link says so, so a wrong guess is easy to correct.
export function FkPeekCard({ connectionId, target, onClose, onOpenTable, onRemap, onPin }: CardProps & { onPin: (target: PeekTarget) => void }) {
  const lookup = useReferencedRow(connectionId, target.link, target.value, 0);
  const box = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const onDown = (event: MouseEvent) => {
      if (box.current && event.target instanceof Node && !box.current.contains(event.target)) onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);
  const left = Math.max(8, Math.min(target.x, window.innerWidth - CARD_WIDTH - 8));
  const top = Math.max(8, Math.min(target.y, window.innerHeight - 340));
  return (
    <div ref={box} role="dialog" aria-label={`${tableLabel(target.link.table)} record`} className="fixed z-40 rounded-xl glass-modal" style={{ left, top, width: CARD_WIDTH }}>
      <div className="flex items-center gap-2 border-b border-border/40 px-2.5 py-1.5 text-xs">
        <Icon name="link" size={12} className="shrink-0 text-accent" />
        <span className="truncate font-semibold text-foreground">{tableLabel(target.link.table)}</span>
        <span className="truncate font-mono text-muted">
          {target.link.column} = {cellText(target.value)}
        </span>
        <span className="ml-auto flex items-center gap-0.5">
          <IconButton icon="expand" label="Pin this record" onClick={() => onPin(target)} size={12} className="size-6 min-w-6" />
          <IconButton icon="x" label="Close" onClick={onClose} size={12} className="size-6 min-w-6" />
        </span>
      </div>
      <RecordBody lookup={lookup} link={target.link} value={target.value} />
      <div className="flex items-center gap-1.5 border-t border-border/40 px-2.5 py-1.5">
        <Button size="xs" variant="soft" onClick={() => onOpenTable(target.link, target.value)}>
          Open in table
        </Button>
        {onRemap ? (
          <Button size="xs" variant="toolbar" onClick={() => onRemap(target)}>
            Change link…
          </Button>
        ) : null}
        {target.link.source === "inferred" ? <span className="ml-auto text-[10px] text-muted">guessed from the column name</span> : null}
        {target.link.source === "remap" ? <span className="ml-auto text-[10px] text-muted">custom link</span> : null}
      </div>
    </div>
  );
}

// WHAT:  A pinned record: the same card, left on screen and movable by its title bar.
// WHY:   Comparing two customers, or keeping one open while you scroll the orders,
//        needs the record to stay put when the peek card would close.
export function FkPinnedPanel({ connectionId, target, onClose, onOpenTable, offset }: { connectionId: string; target: PeekTarget; onClose: (id: string) => void; onOpenTable: (link: FkLink, value: Value) => void; offset: number }) {
  const [refresh, setRefresh] = useState(0);
  const lookup = useReferencedRow(connectionId, target.link, target.value, refresh);
  const [pos, setPos] = useState({ x: Math.max(8, target.x), y: Math.max(8, target.y) + offset * 28 });
  const drag = useRef<{ dx: number; dy: number } | null>(null);
  const onDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.target instanceof HTMLElement && event.target.closest("button")) return;
    drag.current = { dx: event.clientX - pos.x, dy: event.clientY - pos.y };
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (d === null) return;
    setPos({
      x: Math.max(0, Math.min(window.innerWidth - 120, event.clientX - d.dx)),
      y: Math.max(0, Math.min(window.innerHeight - 40, event.clientY - d.dy)),
    });
  };
  const onUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    drag.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  };
  return (
    <div role="dialog" aria-label={`Pinned ${tableLabel(target.link.table)} record`} className={cn("fixed z-30 rounded-xl glass-modal")} style={{ left: pos.x, top: pos.y, width: CARD_WIDTH }}>
      <div onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} className="flex cursor-grab items-center gap-2 border-b border-border/40 px-2.5 py-1.5 text-xs active:cursor-grabbing">
        <Icon name="link" size={12} className="shrink-0 text-accent" />
        <span className="truncate font-semibold text-foreground">{tableLabel(target.link.table)}</span>
        <span className="truncate font-mono text-muted">
          {target.link.column} = {cellText(target.value)}
        </span>
        <span className="ml-auto flex items-center gap-0.5">
          <IconButton icon="refresh" label="Reload this record" onClick={() => setRefresh((n) => n + 1)} size={12} className="size-6 min-w-6" />
          <IconButton icon="x" label="Unpin" onClick={() => onClose(target.id)} size={12} className="size-6 min-w-6" />
        </span>
      </div>
      <RecordBody lookup={lookup} link={target.link} value={target.value} />
      <div className="flex items-center gap-1.5 border-t border-border/40 px-2.5 py-1.5">
        <Button size="xs" variant="soft" onClick={() => onOpenTable(target.link, target.value)}>
          Open in table
        </Button>
        <span className="ml-auto truncate text-[10px] text-muted">from {target.from}</span>
      </div>
    </div>
  );
}
