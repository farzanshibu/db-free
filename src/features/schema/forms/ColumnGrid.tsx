// SOT: column-grid, column-editor-rows, column-type-picker
import type { ReactNode } from "react";
import type { DdlProfile } from "@/lib/bindings";
import { Button } from "@/components/ui/button";
import { AppSelect, Check, Field, type Option } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { columnLocks } from "../ddlRules";
import {
  DEFAULT_KINDS,
  allowsAutoIncrement,
  allowsUnsigned,
  blankColumn,
  findType,
  paramsOf,
  pickType,
  toCustomType,
  type ColumnDraft,
  type DefaultKind,
} from "../ddlForm";

// WHAT:  The editable list of a table's columns: name, type (+ its length /
//        precision / values), default, and the PK / Null / Unique / Auto-increment /
//        Unsigned switches, one row each.
// WHY:   Create, Edit and the editor of a duplicate all need the same grid. Which
//        controls are live on an existing column is a property of the engine
//        (`columnLocks`), so the grid greys them with the reason instead of every
//        form re-deriving it.
// HOW:   Pure view over `ColumnDraft[]`: every edit builds a new array and hands it
//        up. A removed row (Edit) stays in the list, struck through, so Undo can
//        bring it back; new rows get a placeholder name so the form starts valid.
// WHERE: src/features/schema/ddlForm.ts (state, locks), forms/CreateTableDialog.tsx, forms/EditTableDialog.tsx
const OTHER = "\u0000other";
const NO_WRAPPER = "\u0000none";

const GRID =
  "grid grid-cols-[minmax(8rem,1.4fr)_minmax(9rem,1.3fr)_minmax(6.5rem,1fr)_minmax(10rem,1.3fr)_repeat(5,2.25rem)_5.5rem] items-center gap-x-2";

interface ColumnGridProps {
  profile: DdlProfile;
  columns: ColumnDraft[];
  onChange: (next: ColumnDraft[]) => void;
  /// Problems by column key (from `validateDraft`).
  issues: Record<string, string>;
  /// Edit mode: existing rows are removable (not deletable outright) and not reorderable.
  edit: boolean;
}

// WHAT:  A whole-number input that fits a grid row (length, precision, scale).
// WHY:   `NumberInput`'s compact mode is styled as a cell being edited (accent
//        border); in a form row it should look like its neighbours, so this wraps
//        the plain `Field` and parses the text itself.
function SizeInput({ label, value, onChange, disabled }: { label: string; value: number | null; onChange: (next: number | null) => void; disabled: boolean }) {
  return (
    <Field
      label={label}
      type="number"
      mono
      compact
      disabled={disabled}
      className="[&_label]:sr-only"
      value={value === null ? "" : String(value)}
      onChange={(raw) => {
        const n = Math.trunc(Number(raw));
        onChange(raw.trim() === "" || Number.isNaN(n) ? null : n);
      }}
    />
  );
}

function Locked({ reason, children }: { reason: string | null; children: ReactNode }) {
  return <span className="flex min-w-0 items-center" title={reason ?? undefined}>{children}</span>;
}

export function ColumnGrid({ profile, columns, onChange, issues, edit }: ColumnGridProps) {
  const live = columns.filter((c) => !c.removed).length;
  const patch = (key: string, next: Partial<ColumnDraft>) => {
    onChange(columns.map((c) => (c.key === key ? { ...c, ...next } : c)));
  };
  const replace = (key: string, next: ColumnDraft) => {
    onChange(columns.map((c) => (c.key === key ? next : c)));
  };
  const move = (index: number, by: -1 | 1) => {
    const target = index + by;
    const a = columns[index];
    const b = columns[target];
    if (a === undefined || b === undefined) return;
    onChange(columns.map((c, i) => (i === index ? b : i === target ? a : c)));
  };
  const addColumn = () => {
    onChange([...columns, blankColumn(profile, `column_${columns.length + 1}`)]);
  };

  const typeOptions: Option<string>[] = [...profile.types.map((t) => ({ value: t.name, label: t.name })), { value: OTHER, label: "Other…" }];
  const wrapperOptions: Option<string>[] = [{ value: NO_WRAPPER, label: "—" }, ...profile.wrappers.map((w) => ({ value: w, label: w }))];
  const defaultOptions: Option<DefaultKind>[] = DEFAULT_KINDS.map((d) => ({ value: d.value, label: d.label }));

  return (
    <div className="flex flex-col gap-1.5" role="group" aria-label="Columns">
      <div className={cn(GRID, "px-1 text-[10.5px] font-semibold tracking-wider text-muted uppercase")}>
        <span>Name</span>
        <span>Type</span>
        <span>Size</span>
        <span>Default</span>
        <span title="Primary key">PK</span>
        <span title="Allows NULL">Null</span>
        <span title="Unique">Uniq</span>
        <span title="Auto-increment">Auto</span>
        <span title="Unsigned">Uns</span>
        <span />
      </div>
      {columns.map((column, index) => {
        const locks = columnLocks(profile, column);
        const params = paramsOf(profile, column);
        const off = column.removed;
        const issue = issues[column.key];
        const typeValue = column.customType ? OTHER : (findType(profile, column.dataType)?.name ?? OTHER);
        const typeLocked = off || locks.type !== null;
        return (
          <div key={column.key} className={cn("flex flex-col gap-0.5 rounded-lg px-1 py-0.5", off ? "opacity-55" : "", issue !== undefined ? "bg-danger/5" : "")}>
            <div className={GRID}>
              <Locked reason={locks.name}>
                <Field label="Column name" value={column.name} onChange={(name) => { patch(column.key, { name }); }} mono compact disabled={off || locks.name !== null} className={cn("[&_label]:sr-only", off ? "line-through" : "")} />
              </Locked>
              <Locked reason={locks.type}>
                <span className="flex w-full min-w-0 items-center gap-1">
                  {profile.wrappers.length > 0 && !column.customType ? (
                    <AppSelect
                      ariaLabel="Type wrapper"
                      size="sm"
                      className="w-24 shrink-0"
                      value={column.wrapper ?? NO_WRAPPER}
                      options={wrapperOptions}
                      disabled={typeLocked}
                      onChange={(w) => { patch(column.key, { wrapper: w === NO_WRAPPER ? null : w }); }}
                    />
                  ) : null}
                  {column.customType ? (
                    <>
                      <Field label="Type" value={column.dataType} onChange={(dataType) => { patch(column.key, { dataType }); }} placeholder="type name" mono compact disabled={typeLocked} className="[&_label]:sr-only" />
                      <Button variant="ghost" size="icon-sm" aria-label="Pick the type from the list" disabled={typeLocked} onClick={() => { replace(column.key, pickType(profile, column, profile.defaultType)); }}>
                        <Icon name="list" size={13} />
                      </Button>
                    </>
                  ) : (
                    <AppSelect
                      ariaLabel="Type"
                      size="sm"
                      value={typeValue}
                      options={typeOptions}
                      disabled={typeLocked}
                      onChange={(v) => { replace(column.key, v === OTHER ? toCustomType(column) : pickType(profile, column, v)); }}
                    />
                  )}
                </span>
              </Locked>
              <Locked reason={locks.type}>
                <span className="flex w-full min-w-0 items-center gap-1">
                  {params === "length" ? (
                    <SizeInput label="Length" value={column.length} onChange={(length) => { patch(column.key, { length }); }} disabled={typeLocked} />
                  ) : null}
                  {params === "precision" ? (
                    <SizeInput label="Precision" value={column.precision} onChange={(precision) => { patch(column.key, { precision }); }} disabled={typeLocked} />
                  ) : null}
                  {params === "precision_scale" ? (
                    <>
                      <SizeInput label="Precision" value={column.precision} onChange={(precision) => { patch(column.key, { precision }); }} disabled={typeLocked} />
                      <SizeInput label="Scale" value={column.scale} onChange={(scale) => { patch(column.key, { scale }); }} disabled={typeLocked} />
                    </>
                  ) : null}
                  {params === "values" ? (
                    <Field label="Allowed values" value={column.enumText} onChange={(enumText) => { patch(column.key, { enumText }); }} placeholder="a, b, c" mono compact disabled={typeLocked} className="[&_label]:sr-only" />
                  ) : null}
                  {params === "none" ? <span className="px-1 text-xs text-muted/60">—</span> : null}
                </span>
              </Locked>
              <Locked reason={locks.defaultValue}>
                <span className="flex w-full min-w-0 items-center gap-1">
                  <AppSelect
                    ariaLabel="Default"
                    size="sm"
                    className={column.defaultKind === "custom" ? "w-24 shrink-0" : "w-full"}
                    value={column.defaultKind}
                    options={defaultOptions}
                    disabled={off || locks.defaultValue !== null}
                    onChange={(defaultKind) => { patch(column.key, { defaultKind }); }}
                  />
                  {column.defaultKind === "custom" ? (
                    <Field label="Default expression" value={column.defaultExpression} onChange={(defaultExpression) => { patch(column.key, { defaultExpression }); }} placeholder="expression" mono compact disabled={off || locks.defaultValue !== null} className="[&_label]:sr-only" />
                  ) : null}
                </span>
              </Locked>
              <Locked reason={locks.primaryKey}>
                <Check
                  label="Primary key"
                  checked={column.primaryKey}
                  onChange={off || locks.primaryKey !== null ? undefined : (primaryKey) => { patch(column.key, primaryKey ? { primaryKey, nullable: false } : { primaryKey, autoIncrement: false }); }}
                />
              </Locked>
              <Locked reason={locks.nullable}>
                <Check label="Allows NULL" checked={column.nullable} onChange={off || locks.nullable !== null ? undefined : (nullable) => { patch(column.key, { nullable }); }} />
              </Locked>
              <Locked reason={locks.unique}>
                <Check label="Unique" checked={column.unique} onChange={off || locks.unique !== null ? undefined : (unique) => { patch(column.key, { unique }); }} />
              </Locked>
              <Locked reason={locks.autoIncrement}>
                {allowsAutoIncrement(profile, column) ? (
                  <Check
                    label="Auto-increment"
                    checked={column.autoIncrement}
                    onChange={off || locks.autoIncrement !== null ? undefined : (autoIncrement) => { patch(column.key, autoIncrement ? { autoIncrement, nullable: false } : { autoIncrement }); }}
                  />
                ) : (
                  <span className="px-1 text-xs text-muted/60">—</span>
                )}
              </Locked>
              <Locked reason={locks.type}>
                {allowsUnsigned(profile, column) ? (
                  <Check label="Unsigned" checked={column.unsigned} onChange={typeLocked ? undefined : (unsigned) => { patch(column.key, { unsigned }); }} />
                ) : (
                  <span className="px-1 text-xs text-muted/60">—</span>
                )}
              </Locked>
              <span className="flex items-center justify-end gap-0.5">
                {edit ? (
                  off ? (
                    <Button size="xs" variant="soft" onClick={() => { patch(column.key, { removed: false }); }}>
                      Undo
                    </Button>
                  ) : column.originalName === null ? (
                    <IconButton icon="trash" label="Remove column" onClick={() => { onChange(columns.filter((c) => c.key !== column.key)); }} />
                  ) : (
                    <span title={locks.remove ?? undefined}>
                      <IconButton icon="trash" label="Drop column" disabled={locks.remove !== null || live <= 1} onClick={() => { patch(column.key, { removed: true }); }} />
                    </span>
                  )
                ) : (
                  <>
                    <IconButton icon="arrow-up" label="Move up" disabled={index === 0} onClick={() => { move(index, -1); }} />
                    <IconButton icon="arrow-down" label="Move down" disabled={index === columns.length - 1} onClick={() => { move(index, 1); }} />
                    <IconButton icon="trash" label="Remove column" disabled={columns.length <= 1} onClick={() => { onChange(columns.filter((c) => c.key !== column.key)); }} />
                  </>
                )}
              </span>
            </div>
            {issue !== undefined ? <p className="px-1 text-[11px] text-danger">{issue}</p> : null}
          </div>
        );
      })}
      <Button size="sm" variant="ghost" className="self-start text-muted" onClick={addColumn}>
        <Icon name="plus" size={12} />
        Add column
      </Button>
    </div>
  );
}
