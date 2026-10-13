// SOT: variable-control, variable-date-range-control, variable-multi-select-control, committed-input
import { useState, type ReactNode } from "react";
import type { DateRange } from "react-day-picker";
import type { DashboardVariable } from "@/lib/bindings";
import { parseDbTemporal } from "@/lib/datetime";
import { Icon } from "@/lib/icons";
import { AppSelect, Check, Toggle } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Calendar } from "@/components/ui/calendar";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { parseInstant } from "./variables";
import { ScrollArea } from "@/components/ui/scroll-area";

// WHAT:  The control that edits one dashboard variable's value, by type: a
//        two-month range calendar for a date range, an input for text and
//        number, a dropdown for select, a checklist for multi-select, a switch
//        for boolean.
// WHY:   The toolbar's Variables popover and the settings panel edit the same
//        values; one component keeps them identical.
// HOW:   Text and number keep a local draft and commit on Enter or blur, so
//        typing a word does not re-run every tile once per letter. The other
//        controls commit on each choice. A control only ever returns a variable
//        of the type it was given.
// WHERE: ./VariablesToolbar.tsx, ./VariablesEditor.tsx

/// Radix cannot hold an empty item value; this stands for "nothing chosen".
const NONE = "\u0001none";

function dayText(date: Date): string {
  const two = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${two(date.getMonth() + 1)}-${two(date.getDate())}`;
}

function dayOf(text: string): Date | undefined {
  return parseDbTemporal(text).date ?? undefined;
}

/// Shown on the range button: "Jan 1, 2026 to Jan 31, 2026", or the raw text
/// where it is not a plain day.
function rangeSummary(start: string, end: string): string {
  if (start.length === 0 && end.length === 0) return "Any date";
  const show = (text: string) => {
    const day = parseInstant(text)?.time === null ? dayOf(text) : undefined;
    return day === undefined ? text || "…" : day.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
  };
  return `${show(start)} to ${show(end)}`;
}

// ---------------------------------------------------------------- committed input
function CommittedInput({ value, onCommit, type = "text", label, mono = false }: { value: string; onCommit: (next: string) => void; type?: "text" | "number"; label: string; mono?: boolean }) {
  const [draft, setDraft] = useState<{ source: string; text: string }>({ source: value, text: value });
  // A value changed from elsewhere replaces what is being typed.
  const text = draft.source === value ? draft.text : value;
  const commit = () => {
    if (text !== value) onCommit(text);
  };
  return (
    <Input
      type={type}
      aria-label={label}
      value={text}
      className={mono ? "h-6 text-xs font-mono" : "h-6 text-xs"}
      onChange={(event) => { setDraft({ source: value, text: event.target.value }); }}
      onBlur={commit}
      onKeyDown={(event) => {
        if (event.key === "Enter") commit();
        if (event.key === "Escape") setDraft({ source: value, text: value });
      }}
    />
  );
}

// ---------------------------------------------------------------- date range
const RANGE_CLASSES = {
  months: "relative flex flex-row gap-3",
  range_start: "rounded-l-md bg-accent/15 [&>button]:bg-accent! [&>button]:text-accent-foreground!",
  range_end: "rounded-r-md bg-accent/15 [&>button]:bg-accent! [&>button]:text-accent-foreground!",
  range_middle: "bg-accent/15 [&>button]:bg-transparent! [&>button]:rounded-none",
} as const;

function DateRangeControl({ start, end, onChange }: { start: string; end: string; onChange: (start: string, end: string) => void }) {
  const from = dayOf(start);
  const to = dayOf(end);
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="secondary" size="sm" className="w-full justify-between font-normal" aria-label="Date range">
          <span className="truncate">{rangeSummary(start, end)}</span>
          <Icon name="calendar" size={13} className="text-muted" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-auto p-2" align="end">
        <div className="mb-2 flex items-center gap-2">
          <Input aria-label="Range start" value={start} placeholder="YYYY-MM-DD" className="h-6 font-mono text-xs" onChange={(event) => { onChange(event.target.value, end); }} />
          <span className="text-xs text-muted">to</span>
          <Input aria-label="Range end" value={end} placeholder="YYYY-MM-DD" className="h-6 font-mono text-xs" onChange={(event) => { onChange(start, event.target.value); }} />
        </div>
        <Calendar
          mode="range"
          numberOfMonths={2}
          {...(from !== undefined ? { selected: { from, to } satisfies DateRange, defaultMonth: from } : {})}
          classNames={RANGE_CLASSES}
          onSelect={(range) => {
            if (range?.from === undefined) onChange("", "");
            else onChange(dayText(range.from), range.to === undefined ? "" : dayText(range.to));
          }}
        />
        <div className="mt-2 flex items-center justify-between border-t border-border/40 pt-1.5">
          <span className="text-[11px] text-muted">A bare day covers the whole day; add a time (YYYY-MM-DD HH:MM) to be exact.</span>
          <Button size="xs" variant="ghost" onClick={() => { onChange("", ""); }} disabled={start.length === 0 && end.length === 0}>
            Clear
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}

// ---------------------------------------------------------------- multi-select
function MultiSelectControl({ options, values, onChange }: { options: readonly string[]; values: readonly string[]; onChange: (next: string[]) => void }) {
  const all = options.length > 0 && values.length === options.length;
  const label = values.length === 0 ? "None" : all ? "All" : `${values.length} selected`;
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="secondary" size="sm" className="w-full justify-between font-normal" aria-label="Choose values">
          <span className="truncate">{label}</span>
          <Icon name="chevron-down" size={13} className="text-muted" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-56 p-1" align="end">
        {options.length === 0 ? (
          <p className="px-2 py-1 text-xs text-muted">No options yet. Add them in the variable's settings.</p>
        ) : (
          <>
            <div className="flex items-center justify-between px-2 py-1 text-[11px] text-muted">
              <span>{values.length} of {options.length}</span>
              <Button size="xs" variant="ghost" onClick={() => { onChange(all ? [] : [...options]); }}>
                {all ? "Clear" : "Select all"}
              </Button>
            </div>
            <ScrollArea className="max-h-60">
              {options.map((option) => (
                <label key={option} className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1 text-xs hover:bg-surface-secondary">
                  <Check checked={values.includes(option)} label={option} onChange={(next) => { onChange(next ? [...values, option] : values.filter((v) => v !== option)); }} />
                  <span className="truncate">{option}</span>
                </label>
              ))}
            </ScrollArea>
          </>
        )}
      </PopoverContent>
    </Popover>
  );
}

// ---------------------------------------------------------------- the control
export function VariableControl({ variable, onChange }: { variable: DashboardVariable; onChange: (next: DashboardVariable) => void }): ReactNode {
  switch (variable.type) {
    case "date_range":
      return <DateRangeControl start={variable.start} end={variable.end} onChange={(start, end) => { onChange({ ...variable, start, end }); }} />;
    case "text":
      return <CommittedInput label={variable.name} value={variable.value} onCommit={(value) => { onChange({ ...variable, value }); }} />;
    case "number":
      return (
        <CommittedInput
          type="number"
          label={variable.name}
          value={variable.value === null ? "" : String(variable.value)}
          onCommit={(text) => {
            const parsed = text.trim().length === 0 ? null : Number(text);
            if (parsed === null || Number.isFinite(parsed)) onChange({ ...variable, value: parsed });
          }}
        />
      );
    case "select":
      return variable.options.length === 0 ? (
        <p className="text-[11px] text-muted">Add options in the variable's settings.</p>
      ) : (
        <AppSelect
          ariaLabel={variable.name}
          size="sm"
          value={variable.value.length === 0 ? NONE : variable.value}
          options={[{ value: NONE, label: "—" }, ...variable.options.map((o) => ({ value: o, label: o }))]}
          onChange={(value) => { onChange({ ...variable, value: value === NONE ? "" : value }); }}
        />
      );
    case "multi_select":
      return <MultiSelectControl options={variable.options} values={variable.values} onChange={(values) => { onChange({ ...variable, values }); }} />;
    case "boolean":
      return <Toggle checked={variable.value} label={variable.value ? "true" : "false"} onChange={(value) => { onChange({ ...variable, value }); }} />;
  }
}
