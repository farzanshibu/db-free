// SOT: filter-builder, filter-popover, filter-operators, filter-rule-toggle, filter-match-mode
import { useState } from "react";
import type { FilterMode, FilterOp, FilterRule } from "@/lib/bindings";
import { AppSelect, Check, Field, Segmented } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverHeading, PopoverTrigger } from "@/components/ui/popover";
import { ALL_OPS, FILTER_OPS, opsForType, type FilterEntry } from "./filterOps";

// The operator registry moved to filterOps.ts; re-exported for callers that import it from here.
export { FILTER_OPS } from "./filterOps";

interface FilterColumn {
  name: string;
  /// The declared type; narrows the operator list. Omitted = every operator.
  typeName?: string | undefined;
}

interface FilterPopoverProps {
  /// Only the names (and types) are read, so a table page and a query result both fit.
  columns: readonly FilterColumn[];
  /// The rules in force.
  filters: readonly FilterRule[];
  onApply: (filters: FilterRule[]) => void;
  /// The table browser's richer form: every rule with its on/off switch, and how
  /// they combine. When `onApplyEntries` is given the popover edits these and
  /// `filters` / `onApply` are not used for the result.
  entries?: readonly FilterEntry[] | undefined;
  mode?: FilterMode | undefined;
  onApplyEntries?: ((entries: FilterEntry[], mode: FilterMode) => void) | undefined;
}

const OP_LABELS = (ops: readonly FilterOp[]) => ops.map((op) => ({ value: op, label: FILTER_OPS[op].label }));

// WHAT:  The filter builder: one row per rule — on/off, column, operator (the ones
//        the column's type allows), value — and, in the table browser, whether
//        the rules must all hold or any one of them.
// WHY:   Switching a rule off to compare with and without it is what the on/off box
//        is for; without it you delete the rule and retype it.
// HOW:   Edits a draft and applies on Apply, so the grid does not refetch per keystroke.
export function FilterPopover({ columns, filters, onApply, entries, mode, onApplyEntries }: FilterPopoverProps) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<FilterEntry[]>([]);
  const [draftMode, setDraftMode] = useState<FilterMode>("all");
  const firstColumn = columns[0]?.name ?? "";
  const current: readonly FilterEntry[] = entries ?? filters.map((rule) => ({ rule, enabled: true }));
  const typeOf = (name: string) => columns.find((c) => c.name === name)?.typeName;
  const blank = (): FilterEntry => ({ rule: { column: firstColumn, op: defaultOp(typeOf(firstColumn)), value: "" }, enabled: true });
  const activeCount = current.filter((e) => e.enabled).length;

  const openWith = (next: boolean) => {
    if (next) {
      setDraft(current.length > 0 ? current.map((e) => ({ enabled: e.enabled, rule: { ...e.rule } })) : [blank()]);
      setDraftMode(mode ?? "all");
    }
    setOpen(next);
  };

  const update = (index: number, partial: Partial<FilterRule>) =>
    setDraft((d) =>
      d.map((entry, i) => {
        if (i !== index) return entry;
        const rule = { ...entry.rule, ...partial };
        // A new column may not support the operator that was picked: fall back to its first one.
        if (partial.column !== undefined && !opsAllowed(typeOf(rule.column)).includes(rule.op)) rule.op = defaultOp(typeOf(rule.column));
        return { ...entry, rule };
      }),
    );
  const toggle = (index: number, enabled: boolean) => setDraft((d) => d.map((entry, i) => (i === index ? { ...entry, enabled } : entry)));
  const remove = (index: number) => setDraft((d) => d.filter((_, i) => i !== index));
  const complete = (entry: FilterEntry) => entry.rule.column.length > 0 && (!FILTER_OPS[entry.rule.op].needsValue || entry.rule.value.trim().length > 0);
  const apply = () => {
    const kept = draft.filter(complete);
    if (onApplyEntries) onApplyEntries(kept, draftMode);
    else onApply(kept.filter((e) => e.enabled).map((e) => e.rule));
    setOpen(false);
  };
  const clear = () => {
    if (onApplyEntries) onApplyEntries([], "all");
    else onApply([]);
    setOpen(false);
  };

  return (
    <Popover open={open} onOpenChange={openWith}>
      <PopoverTrigger asChild>
        <Button size="xs" variant={activeCount > 0 ? "soft" : "toolbar"}>
          <Icon name="filter" size={12} />
          {activeCount > 0 ? `Filter (${activeCount})` : "Filter"}
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[480px] max-w-[94vw] p-2.5">
        <div className="flex items-center gap-2">
          <PopoverHeading className="text-[11px] font-semibold uppercase tracking-wider text-muted">Filters</PopoverHeading>
          {onApplyEntries ? (
            <span className="ml-auto flex items-center gap-1.5 text-[11px] text-muted">
              Match
              <Segmented label="How rules combine" value={draftMode} onChange={setDraftMode} options={[{ value: "all", label: "All" }, { value: "any", label: "Any" }]} />
            </span>
          ) : null}
        </div>
        <div className="mt-2 flex flex-col gap-1.5">
          {draft.map((entry, index) => {
            const rule = entry.rule;
            const ops = opsAllowed(typeOf(rule.column));
            return (
              <div key={index} className="grid grid-cols-[18px_minmax(0,1fr)_128px_minmax(0,1fr)_22px] items-center gap-1.5">
                {onApplyEntries ? <Check label={`Rule ${index + 1} is on`} checked={entry.enabled} onChange={(on) => toggle(index, on)} /> : <span />}
                <AppSelect ariaLabel="Column" value={rule.column} options={columns.map((c) => ({ value: c.name, label: c.name }))} onChange={(column) => update(index, { column })} size="sm" disabled={!entry.enabled} />
                <AppSelect ariaLabel="Operator" value={rule.op} options={OP_LABELS(ops)} onChange={(op) => update(index, { op })} size="sm" disabled={!entry.enabled} />
                <Field label="" value={rule.value} onChange={(value) => update(index, { value })} disabled={!entry.enabled || !FILTER_OPS[rule.op].needsValue} placeholder="value" mono compact className="[&_label]:hidden" />
                <IconButton icon="x" label="Remove filter" size={12} className="size-5.5 min-w-5.5" onClick={() => remove(index)} />
              </div>
            );
          })}
          <Button size="sm" variant="ghost" className="self-start px-1.5 text-[11px] text-muted" onClick={() => setDraft((d) => [...d, blank()])}>
            <Icon name="plus" size={12} />
            Add filter
          </Button>
        </div>
        <div className="mt-2 flex justify-end gap-1.5">
          <Button size="sm" variant="tertiary" onClick={clear}>
            Clear
          </Button>
          <Button size="sm" onClick={apply}>
            Apply
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}

function opsAllowed(typeName: string | undefined): readonly FilterOp[] {
  const ops = opsForType(typeName);
  return ops.length > 0 ? ops : ALL_OPS;
}

function defaultOp(typeName: string | undefined): FilterOp {
  return opsAllowed(typeName)[0] ?? "eq";
}
