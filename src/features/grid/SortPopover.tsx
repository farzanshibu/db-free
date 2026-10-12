// SOT: sort-dialog, multi-column-sort, sort-rule-toggle
import { useState } from "react";
import { AppSelect, Check } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverHeading, PopoverTrigger } from "@/components/ui/popover";
import { sortLabels } from "./sortLabels";
import type { SortEntry } from "./tableState";

interface SortPopoverProps {
  columns: readonly { name: string; typeName?: string | undefined }[];
  entries: readonly SortEntry[];
  onChange: (entries: SortEntry[]) => void;
}

// WHAT:  The sort builder: an ordered list of rules, each with an on/off switch, a
//        column and a direction written for its type ("A → Z", "Newest first").
// WHY:   Sorting by country, then by revenue, then by name needs a place to say
//        so; the header click only ever sorts one column. Shift+click on a header
//        adds a rule here too.
// HOW:   Rules apply top to bottom; the arrows reorder. Edits are a draft until Apply.
// WHERE: src/features/grid/TableTab.tsx
export function SortPopover({ columns, entries, onChange }: SortPopoverProps) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<SortEntry[]>([]);
  const typeOf = (name: string) => columns.find((c) => c.name === name)?.typeName;
  const active = entries.filter((e) => e.enabled).length;
  const blank = (): SortEntry => ({ rule: { column: columns.find((c) => !draft.some((d) => d.rule.column === c.name))?.name ?? columns[0]?.name ?? "", desc: false }, enabled: true });

  const openWith = (next: boolean) => {
    if (next) setDraft(entries.length > 0 ? entries.map((e) => ({ enabled: e.enabled, rule: { ...e.rule } })) : [blank()]);
    setOpen(next);
  };
  const patch = (index: number, change: Partial<SortEntry["rule"]>) => setDraft((d) => d.map((e, i) => (i === index ? { ...e, rule: { ...e.rule, ...change } } : e)));
  const move = (index: number, delta: -1 | 1) =>
    setDraft((d) => {
      const target = index + delta;
      if (target < 0 || target >= d.length) return d;
      const next = [...d];
      const [item] = next.splice(index, 1);
      if (item) next.splice(target, 0, item);
      return next;
    });

  return (
    <Popover open={open} onOpenChange={openWith}>
      <PopoverTrigger asChild>
        <Button size="xs" variant={active > 0 ? "soft" : "toolbar"}>
          <Icon name="sort" size={12} />
          {active > 0 ? `Sorted by ${active} ${active === 1 ? "rule" : "rules"}` : "Sort"}
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[420px] max-w-[94vw] p-2.5">
        <PopoverHeading className="text-[11px] font-semibold uppercase tracking-wider text-muted">Sort</PopoverHeading>
        <div className="mt-2 flex flex-col gap-1.5">
          {draft.map((entry, index) => {
            const labels = sortLabels(typeOf(entry.rule.column));
            return (
              <div key={index} className="grid grid-cols-[18px_minmax(0,1fr)_128px_22px_22px_22px] items-center gap-1.5">
                <Check label={`Sort rule ${index + 1} is on`} checked={entry.enabled} onChange={(on) => setDraft((d) => d.map((e, i) => (i === index ? { ...e, enabled: on } : e)))} />
                <AppSelect ariaLabel="Sort column" value={entry.rule.column} options={columns.map((c) => ({ value: c.name, label: c.name }))} onChange={(column) => patch(index, { column })} size="sm" disabled={!entry.enabled} />
                <AppSelect
                  ariaLabel="Direction"
                  value={entry.rule.desc ? "desc" : "asc"}
                  options={[
                    { value: "asc", label: labels.asc },
                    { value: "desc", label: labels.desc },
                  ]}
                  onChange={(v) => patch(index, { desc: v === "desc" })}
                  size="sm"
                  disabled={!entry.enabled}
                />
                <IconButton icon="arrow-up" label="Move up" disabled={index === 0} onClick={() => move(index, -1)} size={11} className="size-5.5 min-w-5.5" />
                <IconButton icon="arrow-down" label="Move down" disabled={index === draft.length - 1} onClick={() => move(index, 1)} size={11} className="size-5.5 min-w-5.5" />
                <IconButton icon="x" label="Remove sort rule" onClick={() => setDraft((d) => d.filter((_, i) => i !== index))} size={12} className="size-5.5 min-w-5.5" />
              </div>
            );
          })}
          <Button size="sm" variant="ghost" className="self-start px-1.5 text-[11px] text-muted" onClick={() => setDraft((d) => [...d, blank()])} disabled={columns.length === 0}>
            <Icon name="plus" size={12} />
            Add sort
          </Button>
        </div>
        <div className="mt-2 flex justify-end gap-1.5">
          <Button
            size="sm"
            variant="tertiary"
            onClick={() => {
              onChange([]);
              setOpen(false);
            }}
          >
            Clear
          </Button>
          <Button
            size="sm"
            onClick={() => {
              onChange(draft.filter((e) => e.rule.column.length > 0));
              setOpen(false);
            }}
          >
            Apply
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
