// SOT: columns-popover, column-visibility, hidden-columns
import { useState } from "react";
import { Check } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";
import { SearchInput } from "@/components/ui/input";
import { TriggerTip, WIDE_LABEL } from "./toolLabel";

// WHAT:  Column visibility: a searchable checklist over the grid's columns.
// WHY:   Wide tables are read one slice at a time; hiding the rest beats
//        scrolling past it. The owner decides how long the choice lives: a table
//        tab saves it per connection and table (tableState.ts), a query result
//        keeps it for the tab.
// WHERE: src/features/grid/TableTab.tsx, src/features/editor/ResultsPane.tsx
export function ColumnsPopover({
  columns,
  hidden,
  onChange,
  compact = false,
}: {
  columns: readonly { name: string }[];
  hidden: ReadonlySet<string>;
  onChange: (next: ReadonlySet<string>) => void;
  /// The trigger's text folds away on a narrow toolbar (the row carries `@container`).
  compact?: boolean | undefined;
}) {
  const [search, setSearch] = useState("");
  const needle = search.trim().toLowerCase();
  const shown = columns.filter((c) => needle.length === 0 || c.name.toLowerCase().includes(needle));
  const toggle = (name: string) => {
    const next = new Set(hidden);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    onChange(next);
  };
  return (
    <Popover>
      <TriggerTip tip={hidden.size > 0 ? `${columns.length - hidden.size} of ${columns.length} columns shown` : "Columns"} when={compact}>
        <PopoverTrigger asChild>
          <Button size="xs" variant={hidden.size > 0 ? "soft" : "toolbar"} aria-label={hidden.size > 0 ? `Columns: ${columns.length - hidden.size} of ${columns.length} shown` : "Columns"}>
            <Icon name="columns" size={12} />
            {hidden.size > 0 ? (
              <span className="tabular-nums">{`${columns.length - hidden.size}/${columns.length}`}</span>
            ) : (
              <span className={compact ? WIDE_LABEL : undefined}>Columns</span>
            )}
          </Button>
        </PopoverTrigger>
      </TriggerTip>
      <PopoverContent className="w-[min(280px,90vw)] p-2">
          <SearchInput value={search} onChange={setSearch} aria-label="Search columns" placeholder="Search…" autoFocus className="glass-input w-full text-xs" />
          <ScrollArea hideScrollBar className="mt-2 max-h-64">
            <ul className="flex flex-col gap-0.5">
              {shown.map((c, i) => (
                <li key={`${c.name}-${i}`}>
                  <span className="flex w-full items-center gap-2 rounded-md px-1 py-0.5 text-xs hover:bg-surface-secondary/60">
                    <Check label={c.name} checked={!hidden.has(c.name)} onChange={() => toggle(c.name)} />
                  </span>
                </li>
              ))}
            </ul>
          </ScrollArea>
          <div className="mt-2 flex justify-end gap-1.5 border-t border-border/40 pt-1.5">
            <Button size="sm" variant="tertiary" className="text-[11px]" onClick={() => onChange(new Set(columns.map((c) => c.name)))}>
              Hide all
            </Button>
            <Button size="sm" variant="tertiary" className="text-[11px]" onClick={() => onChange(new Set())}>
              Show all
            </Button>
          </div>
      </PopoverContent>
    </Popover>
  );
}
