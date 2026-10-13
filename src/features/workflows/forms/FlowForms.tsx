// SOT: workflow-schedule-form, workflow-filter-form, workflow-branch-form, workflow-gate-form, workflow-repeat-form, workflow-delay-form, workflow-note-form
import { useId, useMemo } from "react";
import type { GateOp, IntervalUnit, ScheduleMode, Weekday } from "@/lib/bindings";
import { AppSelect, Field, NumberInput, Segmented } from "@/components/global/Field";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { summarizeSchedule } from "../model";
import { TemplateHint, type FormProps } from "./common";
import { RuleEditor } from "./RuleEditor";

const MODES: readonly { value: ScheduleMode; label: string }[] = [
  { value: "interval", label: "Every…" },
  { value: "at_time", label: "At a time" },
];

const UNITS: readonly { value: IntervalUnit; label: string }[] = [
  { value: "seconds", label: "seconds" },
  { value: "minutes", label: "minutes" },
  { value: "hours", label: "hours" },
  { value: "days", label: "days" },
];

const DAYS: readonly { value: Weekday; label: string }[] = [
  { value: "mon", label: "Mon" },
  { value: "tue", label: "Tue" },
  { value: "wed", label: "Wed" },
  { value: "thu", label: "Thu" },
  { value: "fri", label: "Fri" },
  { value: "sat", label: "Sat" },
  { value: "sun", label: "Sun" },
];

/// IANA zone names the webview knows; empty where `Intl.supportedValuesOf` is missing.
function timeZones(): string[] {
  try {
    return Intl.supportedValuesOf("timeZone");
  } catch {
    return [];
  }
}

// WHAT:  Schedule trigger: every N seconds / minutes / hours / days, or at a clock
//        time on chosen weekdays in a chosen time zone.
// WHY:   Schedules run in the core, so they fire with this tab closed, but only
//        while the workflow's "Schedules active" switch is on.
export function ScheduleForm({ config, onChange }: FormProps<"schedule">) {
  const zoneList = useId();
  const zones = useMemo(() => timeZones(), []);
  const toggleDay = (day: Weekday) => {
    const days = config.days.includes(day) ? config.days.filter((d) => d !== day) : [...config.days, day];
    onChange({ ...config, days });
  };
  return (
    <div className="flex flex-col gap-2">
      <Segmented value={config.mode} options={MODES} onChange={(mode) => { onChange({ ...config, mode }); }} label="When" />
      {config.mode === "interval" ? (
        <div className="flex items-end gap-2">
          <div className="w-28">
            <NumberInput integer label="Every" value={config.intervalAmount} onChange={(next) => { onChange({ ...config, intervalAmount: Math.max(0, Math.round(next ?? 0)) }); }} />
          </div>
          <AppSelect ariaLabel="Unit" className="flex-1" value={config.intervalUnit} options={UNITS} onChange={(intervalUnit) => { onChange({ ...config, intervalUnit }); }} />
        </div>
      ) : (
        <>
          <Field label="Time (24-hour HH:MM)" mono value={config.time} placeholder="09:00" onChange={(time) => { onChange({ ...config, time }); }} />
          <div className="flex flex-col gap-1.5">
            <Label>Days</Label>
            <div className="flex flex-wrap gap-1">
              {DAYS.map((day) => (
                <Button
                  key={day.value}
                  size="xs"
                  variant={config.days.includes(day.value) ? "primary" : "secondary"}
                  aria-pressed={config.days.includes(day.value)}
                  onClick={() => { toggleDay(day.value); }}
                >
                  {day.label}
                </Button>
              ))}
            </div>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={zoneList}>Time zone</Label>
            <Input id={zoneList} list={`${zoneList}-zones`} value={config.timezone} placeholder="Europe/Paris (blank = this machine)" onChange={(event) => { onChange({ ...config, timezone: event.target.value }); }} />
            <datalist id={`${zoneList}-zones`}>
              {zones.map((zone) => (
                <option key={zone} value={zone} />
              ))}
            </datalist>
            <p className="text-[11px] text-muted">Daylight saving time is followed automatically.</p>
          </div>
        </>
      )}
      <p className={cn("rounded-md bg-surface-secondary/60 px-2 py-1.5 text-xs text-foreground")}>{summarizeSchedule(config)}</p>
      <p className="text-[11px] leading-relaxed text-muted">
        Fires only while <span className="text-foreground">Schedules active</span> is on (toolbar), even with this tab closed. An interval is never shorter than 10 seconds. Each fire hands on one row,{" "}
        <code className="font-mono text-foreground">{"{{firedAt}}"}</code>.
      </p>
    </div>
  );
}

// WHAT:  Filter and Branch share their rule editor; the node decides what the match means.
export function FilterForm({ config, onChange, context }: FormProps<"filter">) {
  return (
    <div className="flex flex-col gap-2">
      <RuleEditor group={config.group} columns={context.columns} onChange={(group) => { onChange({ ...config, group }); }} />
      <p className="text-[11px] text-muted">Keeps the rows that match. Numbers compare as numbers; everything else compares as text.</p>
    </div>
  );
}

export function BranchForm({ config, onChange, context }: FormProps<"branch">) {
  return (
    <div className="flex flex-col gap-2">
      <RuleEditor group={config.group} columns={context.columns} onChange={(group) => { onChange({ ...config, group }); }} />
      <p className="text-[11px] text-muted">Matching rows leave by the True handle, the rest by the False handle. A side with no rows is skipped, with everything after it.</p>
    </div>
  );
}

const GATE_OPS: readonly { value: GateOp; label: string }[] = [
  { value: "gt", label: "is greater than" },
  { value: "gte", label: "is at least" },
  { value: "lt", label: "is less than" },
  { value: "lte", label: "is at most" },
  { value: "eq", label: "equals" },
  { value: "neq", label: "does not equal" },
];

export function GateForm({ config, onChange }: FormProps<"row_count_gate">) {
  return (
    <div className="flex flex-col gap-2">
      <AppSelect label="Let rows through when the row count" value={config.op} options={GATE_OPS} onChange={(op) => { onChange({ ...config, op }); }} />
      <NumberInput label="Count" value={config.value} onChange={(next) => { onChange({ ...config, value: next ?? 0 }); }} />
      <p className="text-[11px] text-muted">When the rule fails the branch stops here: nothing after the gate runs. Use it to skip notifications when a query found nothing.</p>
    </div>
  );
}

export function RepeatForm({ config, onChange }: FormProps<"repeat">) {
  return (
    <div className="flex flex-col gap-2">
      <NumberInput integer label="Run everything after this node" value={config.count} onChange={(next) => { onChange({ ...config, count: Math.max(0, Math.round(next ?? 0)) }); }} />
      <TemplateHint>
        Each pass carries <code className="font-mono text-foreground">{"{{i}}"}</code> (from 0) and <code className="font-mono text-foreground">{"{{total}}"}</code>. Page through a table with{" "}
        <code className="font-mono text-foreground">{"LIMIT 100 OFFSET {{i * 100}}"}</code>. Two Repeat nodes feeding one node run every combination.
      </TemplateHint>
    </div>
  );
}

const DELAY_UNITS = [
  { value: "ms", label: "milliseconds" },
  { value: "s", label: "seconds" },
  { value: "m", label: "minutes" },
] as const;

export function DelayForm({ config, onChange }: FormProps<"delay">) {
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-end gap-2">
        <div className="w-32">
          <NumberInput label="Wait" value={config.amount} onChange={(next) => { onChange({ ...config, amount: Math.max(0, next ?? 0) }); }} />
        </div>
        <AppSelect ariaLabel="Unit" className="flex-1" value={config.unit} options={DELAY_UNITS} onChange={(unit) => { onChange({ ...config, unit }); }} />
      </div>
      <p className="text-[11px] text-muted">Pass the rows on after the pause. Stop ends the wait at once.</p>
    </div>
  );
}

export function NoteForm({ config, onChange }: FormProps<"note">) {
  return (
    <div className="flex flex-col gap-2">
      <Label>Note</Label>
      <Textarea value={config.text} rows={6} className="font-sans" placeholder="A reminder for whoever reads this workflow" onChange={(event) => { onChange({ ...config, text: event.target.value }); }} />
      <p className="text-[11px] text-muted">Notes are never run. You can also edit the text on the canvas.</p>
    </div>
  );
}
