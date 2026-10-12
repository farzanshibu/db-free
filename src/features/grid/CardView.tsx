// SOT: card-view, row-cards, grid-card-layout
import type { Value } from "@/lib/bindings";
import { formatCell } from "@/lib/format";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { InspectorColumn } from "./inspectorValues";

interface CardViewProps {
  /// The visible columns, in page order.
  columns: readonly InspectorColumn[];
  rows: readonly (readonly Value[])[];
  /// Visible column -> index into a row.
  columnIndexes: readonly number[];
  selected: number | null;
  onSelect: (row: number) => void;
  deletedRows: ReadonlySet<number>;
  /// Rows from this index on are staged inserts.
  insertedFrom: number;
  nullDisplay: string;
}

/// Most cards drawn at once; a page is at most 1,000 rows and a card is a lot of DOM.
const CARD_CAP = 200;

// WHAT:  The page as cards: one per row, each column a label and a value.
// WHY:   A wide table is read one record at a time more often than one column at a
//        time; cards show the whole record without scrolling sideways.
// HOW:   Read-only — editing and selection details live in the inspector, which the
//        card opens on click. The first CARD_CAP rows are drawn.
// WHERE: src/features/grid/TableTab.tsx
export function CardView({ columns, rows, columnIndexes, selected, onSelect, deletedRows, insertedFrom, nullDisplay }: CardViewProps) {
  const shown = rows.slice(0, CARD_CAP);
  return (
    <ScrollArea className="h-full min-h-0 bg-background/60">
      <ul className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-2 p-2">
        {shown.map((row, index) => {
          const isDeleted = deletedRows.has(index);
          const isInserted = index >= insertedFrom;
          return (
            <li key={index}>
              <button
                type="button"
                onClick={() => onSelect(index)}
                aria-pressed={selected === index}
                className={cn(
                  "flex w-full flex-col gap-0.5 rounded-lg border p-2 text-left text-xs transition-colors",
                  selected === index ? "border-accent bg-accent/10" : "border-border/40 glass-card hover:bg-surface-secondary/40",
                  isDeleted ? "border-danger/50 bg-danger-soft/40 line-through" : "",
                  isInserted ? "border-success/50 bg-success-soft/40" : "",
                )}
              >
                {columns.map((column, c) => {
                  const value = row[columnIndexes[c] ?? c];
                  const isNull = value === undefined || value.t === "null";
                  return (
                    <div key={column.name} className="flex gap-2 border-b border-separator/50 py-0.5 last:border-0">
                      <span className="flex w-28 shrink-0 items-center gap-1 truncate text-muted">
                        {column.primaryKey ? <Icon name="key" size={10} className="shrink-0 text-warning" /> : null}
                        <span className="truncate">{column.name}</span>
                      </span>
                      <span className={cn("min-w-0 flex-1 truncate font-mono", isNull ? "italic text-muted" : "text-foreground")}>
                        {value === undefined || value.t === "null" ? nullDisplay : formatCell(value).text}
                      </span>
                    </div>
                  );
                })}
              </button>
            </li>
          );
        })}
      </ul>
      {rows.length > CARD_CAP ? <p className="px-3 pb-2.5 text-center text-xs text-muted">Showing the first {CARD_CAP} cards of this page; use the grid view or a smaller page to see the rest.</p> : null}
    </ScrollArea>
  );
}
