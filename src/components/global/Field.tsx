// SOT: form-fields, text-field, select-field, toggle-field, segmented-control, checkbox-field, datetime-field, number-input
import { useId, useState, type ReactNode } from "react";
import { cn } from "@/lib/cn";
import { Icon, type IconName } from "@/lib/icons";
import { formatDbTemporal, parseDbTemporal, type DbTemporal, type TemporalKind } from "@/lib/datetime";
import { Button } from "@/components/ui/button";
import { Calendar } from "@/components/ui/calendar";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";

// WHAT:  Typed, labelled wrappers over the shadcn/ui form primitives.
// WHY:   shadcn ships unopinionated parts — a Label, an Input, a Select root and
//        four sub-components — and leaves assembly to the caller. Assembling
//        them at 60-odd call sites is where inconsistency creeps in, so the
//        composition happens once, here. The wrappers are also generic over the
//        caller's own string union, which is what keeps features free of the
//        casts a `string`-typed Select value would otherwise force.
// WHERE: https://ui.shadcn.com/docs/components/{input,select,switch,checkbox,tabs}

// WHAT:  Controls fill their container unless the caller sets an explicit width.
function widthClass(className: string | undefined): string {
  return className !== undefined && /\bw-/.test(className) ? "" : "w-full";
}

interface FieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string | undefined;
  type?: "text" | "password" | "number" | "email";
  description?: string | undefined;
  optional?: boolean;
  disabled?: boolean;
  autoFocus?: boolean;
  className?: string;
  suffix?: ReactNode;
  mono?: boolean;
  /// Row-height control for dense surfaces (filter builder, toolbars): 28px
  /// input, 12px text, no label gap.
  compact?: boolean;
}

export function Field({ label, value, onChange, placeholder, type = "text", description, optional = false, disabled = false, autoFocus = false, className, suffix, mono = false, compact = false }: FieldProps) {
  const id = useId();
  return (
    <div className={cn("group/field flex w-full flex-col", compact ? "gap-0" : "gap-1.5", className)} data-disabled={disabled}>
      <Label htmlFor={id}>
        {label}
        {optional ? <span className="text-muted/70">(optional)</span> : null}
      </Label>
      {/* The suffix sits inside the field's own box rather than beside it, so a
          reveal-password or unit affordance keeps the input's focus ring. */}
      <div className="relative flex w-full items-center">
        <Input
          id={id}
          type={type}
          value={value}
          onChange={(event) => { onChange(event.target.value); }}
          disabled={disabled}
          autoFocus={autoFocus}
          className={cn(mono ? "font-mono" : "", compact ? "h-7 px-2 text-xs" : "", suffix ? "pr-8" : "")}
          {...(placeholder !== undefined ? { placeholder } : {})}
        />
        {suffix ? <span className="absolute right-1 flex items-center">{suffix}</span> : null}
      </div>
      {description !== undefined && description !== "" ? <p className="text-xs text-muted">{description}</p> : null}
    </div>
  );
}

export interface Option<T extends string> {
  value: T;
  label: string;
  icon?: IconName;
  /// Arbitrary leading node (an engine logo); wins over `icon`.
  leading?: ReactNode;
}

interface AppSelectProps<T extends string> {
  value: T;
  options: readonly Option<T>[];
  onChange: (value: T) => void;
  label?: string | undefined;
  ariaLabel?: string | undefined;
  className?: string | undefined;
  disabled?: boolean;
  size?: "sm" | "md";
  icon?: IconName | undefined;
  /// Borderless breadcrumb-style trigger (sidebar database / schema switcher).
  plain?: boolean;
}

export function AppSelect<T extends string>({ value, options, onChange, label, ariaLabel, className, disabled = false, size = "md", icon, plain = false }: AppSelectProps<T>) {
  const current = options.find((o) => o.value === value);
  const leading = icon ?? current?.icon;
  return (
    <div className={cn("flex flex-col gap-1.5", plain ? "w-auto" : widthClass(className), "min-w-0", className)}>
      {label !== undefined && label !== "" ? <Label>{label}</Label> : null}
      <Select
        value={value}
        onValueChange={(next) => {
          // Radix hands back a plain string; only a value this component
          // published can match, which is what re-narrows it to T without a cast.
          const option = options.find((o) => o.value === next);
          if (option) onChange(option.value);
        }}
        disabled={disabled}
      >
        <SelectTrigger
          size={size}
          aria-label={label ?? ariaLabel ?? "Select"}
          className={cn(
            plain
              ? "h-6 w-auto gap-1 rounded-md border-0 bg-transparent px-1.5 text-xs text-foreground shadow-none hover:bg-surface-secondary"
              : "",
          )}
        >
          <span className="flex min-w-0 flex-1 items-center gap-1.5">
            {icon === undefined && current?.leading ? (
              <span className="flex shrink-0 items-center">{current.leading}</span>
            ) : leading !== undefined ? (
              <Icon name={leading} size={plain ? 12 : 14} className="shrink-0 text-accent" />
            ) : null}
            {/* The trigger shows the label alone: `SelectValue` would re-render
                the whole item, drawing a second copy of an option's logo. */}
            <SelectValue className="truncate">
              <span className={cn("truncate text-left", plain ? "max-w-[150px]" : "")}>{current?.label ?? ""}</span>
            </SelectValue>
          </span>
        </SelectTrigger>
        <SelectContent>
          {options.map((o) => (
            <SelectItem key={o.value} value={o.value}>
              {o.leading ? <span className="flex shrink-0 items-center">{o.leading}</span> : o.icon !== undefined ? <Icon name={o.icon} size={14} className="text-muted" /> : null}
              {o.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

interface ToggleProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
}

export function Toggle({ checked, onChange, label, description }: ToggleProps) {
  const id = useId();
  return (
    <div className="flex w-full flex-col gap-1">
      <div className="flex items-center gap-3">
        <Switch id={id} checked={checked} onCheckedChange={onChange} aria-label={label.length > 0 ? label : "toggle"} />
        {label.length > 0 ? (
          <Label htmlFor={id} className="text-[13px] font-normal text-foreground">
            {label}
          </Label>
        ) : null}
      </div>
      {description !== undefined && description !== "" ? <p className="text-xs text-muted">{description}</p> : null}
    </div>
  );
}

interface CheckProps {
  checked: boolean;
  onChange?: ((next: boolean) => void) | undefined;
  label: string;
  indeterminate?: boolean;
}

// WHAT:  A bare checkbox: the label is the accessible name, not a rendered one.
// WHY:   Its call sites are grid row selectors and column pickers, where the
//        text beside it belongs to the row, not to the control.
export function Check({ checked, onChange, label, indeterminate = false }: CheckProps) {
  return (
    <Checkbox
      checked={indeterminate ? "indeterminate" : checked}
      disabled={onChange === undefined}
      onCheckedChange={(next) => { onChange?.(next === true); }}
      aria-label={label}
    />
  );
}

interface SegmentedProps<T extends string> {
  value: T;
  options: readonly { value: T; label: string; disabled?: boolean }[];
  onChange: (value: T) => void;
  label: string;
  className?: string | undefined;
}

// WHAT:  Segmented control = shadcn Tabs used for its list alone, with no panels.
export function Segmented<T extends string>({ value, options, onChange, label, className }: SegmentedProps<T>) {
  return (
    <Tabs
      value={value}
      onValueChange={(next) => {
        const option = options.find((o) => o.value === next);
        if (option) onChange(option.value);
      }}
      className={className}
    >
      <TabsList aria-label={label}>
        {options.map((o) => (
          <TabsTrigger key={o.value} value={o.value} disabled={o.disabled ?? false}>
            {o.label}
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  );
}

interface DateTimeFieldProps {
  /// date → calendar; time → clock; datetime → both.
  kind: TemporalKind;
  /// DB text (`YYYY-MM-DD`, `HH:MM:SS`, `YYYY-MM-DD HH:MM:SS[.f][+00]`); "" = empty.
  value: string;
  onChange: (text: string) => void;
  label?: string | undefined;
  ariaLabel?: string | undefined;
  /// Fits a grid row: no label, cell-height control, tighter padding.
  compact?: boolean;
  autoFocus?: boolean;
  /// Opens the picker as soon as the field mounts: a grid cell edit goes
  /// straight to the calendar, no trigger click.
  autoOpen?: boolean;
  /// Fired when the picker closes: the caller treats that as "the user is done"
  /// rather than asking them to confirm a second time.
  onDone?: (() => void) | undefined;
  disabled?: boolean;
  className?: string | undefined;
}

// WHAT:  A temporal cell editor: the DB literal stays visible and typeable in a
//        monospace input, with a popover holding a calendar and a clock.
// WHY:   The column's own text is the contract — an engine may store a precision
//        or an offset no picker models — so the text is the control and the
//        pickers write into it, never the other way round.
// WHERE: https://ui.shadcn.com/docs/components/date-picker
export function DateTimeField({ kind, value, onChange, label, ariaLabel, compact = false, autoFocus = false, autoOpen = false, onDone, disabled = false, className }: DateTimeFieldProps) {
  const [open, setOpen] = useState(autoOpen);
  const id = useId();
  const parts = parseDbTemporal(value);
  const a11y = ariaLabel ?? label ?? kind;

  const write = (next: DbTemporal) => { onChange(formatDbTemporal(kind, next)); };

  return (
    <div className={cn("flex flex-col", compact ? "h-full gap-0" : "w-full gap-1.5", className)}>
      {label !== undefined && label !== "" ? <Label htmlFor={id}>{label}</Label> : null}
      <div className="relative flex h-full items-center">
        <Input
          id={id}
          value={value}
          onChange={(event) => { onChange(event.target.value); }}
          disabled={disabled}
          autoFocus={autoFocus}
          aria-label={a11y}
          placeholder={kind === "time" ? "HH:MM:SS" : kind === "date" ? "YYYY-MM-DD" : "YYYY-MM-DD HH:MM:SS"}
          className={cn("pr-7 font-mono", compact ? "h-full min-h-0 rounded-sm border-accent bg-background px-1 text-[12px] shadow-none" : "")}
        />
        <Popover
          open={open}
          onOpenChange={(next) => {
            setOpen(next);
            if (!next) onDone?.();
          }}
        >
          <PopoverTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              disabled={disabled}
              aria-label={`Pick ${a11y}`}
              className={cn("absolute right-0.5", compact ? "size-5 min-w-5" : "size-6 min-w-6")}
            >
              {kind === "time" ? <Icon name="clock" className={compact ? "size-3" : "size-3.5"} /> : <Icon name="calendar" className={compact ? "size-3" : "size-3.5"} />}
            </Button>
          </PopoverTrigger>
          <PopoverContent className="w-auto p-2" align="end">
            {kind === "time" ? null : (
              <Calendar
                mode="single"
                autoFocus
                {...(parts.date !== null ? { selected: parts.date, defaultMonth: parts.date } : {})}
                onSelect={(date) => {
                  if (date) write({ ...parts, date });
                }}
              />
            )}
            {kind === "date" ? null : (
              <div className={cn("flex items-center justify-between gap-3", kind === "datetime" ? "mt-2 border-t border-border/40 pt-2" : "")}>
                <span className="flex items-center gap-1.5 text-xs text-muted">
                  <Icon name="clock" className="size-3" />
                  Time
                </span>
                <Input
                  type="time"
                  step={1}
                  aria-label={`${a11y} time`}
                  value={parts.time === "" ? "" : parts.time}
                  onChange={(event) => {
                    const time = event.target.value === "" ? "" : `${event.target.value}:00`.slice(0, 8);
                    write({ ...parts, time, date: parts.date ?? new Date() });
                  }}
                  className="h-7 w-full min-w-[7.5rem] font-mono text-xs"
                />
              </div>
            )}
          </PopoverContent>
        </Popover>
      </div>
    </div>
  );
}

interface NumberInputProps {
  /// null = empty.
  value: number | null;
  onChange: (next: number | null) => void;
  /// Whole numbers only (int / serial columns).
  integer?: boolean;
  label?: string | undefined;
  ariaLabel?: string | undefined;
  /// Fits a grid row: no label, no stepper buttons, cell-height input.
  compact?: boolean;
  autoFocus?: boolean;
  disabled?: boolean;
  className?: string | undefined;
}

// WHAT:  A numeric field in DB style — no locale grouping, so `1234567.5` is
//        what is typed and what is sent.
// WHY:   shadcn has no number field, and the browser's own spinner is both
//        locale-aware and unstyleable; these steppers are plain buttons.
export function NumberInput({ value, onChange, integer = false, label, ariaLabel, compact = false, autoFocus = false, disabled = false, className }: NumberInputProps) {
  const id = useId();
  const step = (delta: number) => { onChange(Number(((value ?? 0) + delta).toFixed(integer ? 0 : 10))); };
  return (
    <div className={cn("flex flex-col", compact ? "h-full gap-0" : "w-full gap-1.5", className)}>
      {label !== undefined && label !== "" ? <Label htmlFor={id}>{label}</Label> : null}
      <div className={cn("flex h-full items-center", compact ? "" : "gap-1")}>
        {compact ? null : (
          <Button variant="secondary" size="icon-sm" aria-label="Decrement" disabled={disabled} onClick={() => { step(-1); }}>
            <Icon name="minus" />
          </Button>
        )}
        <Input
          id={id}
          type="number"
          inputMode={integer ? "numeric" : "decimal"}
          step={integer ? 1 : "any"}
          value={value ?? ""}
          onChange={(event) => { onChange(event.target.value === "" ? null : Number(event.target.value)); }}
          disabled={disabled}
          autoFocus={autoFocus}
          aria-label={ariaLabel ?? label ?? "number"}
          className={cn(
            "font-mono tabular-nums [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none",
            compact ? "h-full min-h-0 rounded-sm border-accent bg-background px-1 text-[12px] shadow-none" : "",
          )}
        />
        {compact ? null : (
          <Button variant="secondary" size="icon-sm" aria-label="Increment" disabled={disabled} onClick={() => { step(1); }}>
            <Icon name="plus" />
          </Button>
        )}
      </div>
    </div>
  );
}
