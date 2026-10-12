// SOT: inspector-fields-tab, inspector-field-editor, inspector-null-toggle, inspector-update-reset-delete
import { useEffect, useMemo, useRef, useState } from "react";
import type { Value } from "@/lib/bindings";
import { blankPlaceholder } from "@/lib/columnTraits";
import { fieldKind } from "@/lib/fields";
import { formatCell } from "@/lib/format";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { ValueControl, type FormCell } from "@/components/global/ValueEditor";
import { JsonViewer } from "@/components/global/JsonViewer";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Toggle } from "@/components/ui/toggle";
import {
  emptyFor,
  enumTraitsOf,
  toColumnInfo,
  toTraits,
  validateCell,
  valuesEqual,
  type Draft,
  type DraftCell,
  type InspectorColumn,
  type InspectorEdit,
} from "./inspectorValues";

interface FieldsProps {
  columns: readonly InspectorColumn[];
  /// The rows the inspector speaks for: one, or several selected at once.
  rows: readonly (readonly Value[])[];
  /// The column the grid cell that opened the inspector sits in.
  focus: string;
  edit: InspectorEdit | undefined;
  /// The rows still hold shortened cells (the full ones are being loaded).
  partial: boolean;
}

/// The text of a cell for the read-only list: JSON compact, NULL spelled out.
function shown(value: Value | undefined): string {
  if (value === undefined) return "";
  if (value.t === "null") return "NULL";
  return value.t === "json" ? JSON.stringify(value.v) : formatCell(value).text;
}

// WHAT:  The inspector's Fields tab: every column of the row (or of several rows) as a
//        typed control, with Update / Reset / Delete.
// WHY:   A wide row is easier to edit as a form than cell by cell, and several
//        edits to one row belong in one Update.
// HOW:   Edits live in a draft until Update, which hands them to the grid to stage
//        (review mode) or apply (direct mode). With several rows selected a field
//        that differs between them reads "Multiple values", and setting it writes
//        the same value to all of them. Without `edit` it is a plain read-out.
// WHERE: src/features/grid/RecordInspector.tsx, src/components/global/ValueEditor.tsx (ValueControl)
export function InspectorFields({ columns, rows, focus, edit, partial }: FieldsProps) {
  const [draft, setDraft] = useState<Draft>({});
  const focusRef = useRef<HTMLDivElement | null>(null);
  const editable = edit !== undefined && edit.disabledReason === undefined && !partial;

  // The field of the clicked cell is brought into view when the inspector opens on it.
  useEffect(() => {
    focusRef.current?.scrollIntoView({ block: "nearest" });
  }, [focus]);

  const merged = (index: number): Value | "mixed" => {
    const first = rows[0]?.[index];
    if (first === undefined) return { t: "null" };
    return rows.every((r) => r[index] !== undefined && valuesEqual(r[index], first)) ? first : "mixed";
  };

  const changed = useMemo(
    () =>
      Object.entries(draft).filter(([name, cell]) => {
        const index = columns.findIndex((c) => c.name === name);
        return index >= 0 && rows.some((r) => r[index] === undefined || !valuesEqual(r[index], cell.value) || cell.raw !== undefined);
      }),
    [draft, columns, rows],
  );
  const problems = useMemo(() => {
    const out: Record<string, string> = {};
    for (const [name, cell] of changed) {
      const column = columns.find((c) => c.name === name);
      const problem = column === undefined ? null : validateCell(column, cell);
      if (problem !== null) out[name] = problem;
    }
    return out;
  }, [changed, columns]);
  const blocked = Object.keys(problems).length > 0;

  const put = (name: string, cell: DraftCell | undefined) =>
    setDraft((d) => {
      if (cell === undefined) return Object.fromEntries(Object.entries(d).filter(([key]) => key !== name));
      return { ...d, [name]: cell };
    });

  const apply = () => {
    if (!edit || blocked) return;
    const edits = [];
    for (const [name, cell] of changed) {
      const col = columns.findIndex((c) => c.name === name);
      for (let row = 0; row < rows.length; row += 1) {
        const current = rows[row]?.[col];
        if (current !== undefined && valuesEqual(current, cell.value) && cell.raw === undefined) continue;
        edits.push({ row, col, value: cell.value, raw: cell.raw });
      }
    }
    if (edits.length > 0) edit.onApply(edits);
    setDraft({});
  };

  if (edit === undefined) {
    const row = rows[0] ?? [];
    return (
      <ScrollArea className="min-h-0 flex-1">
        <dl className="px-2.5 pb-2 text-xs">
          {columns.map((c, i) => (
            <div key={`${c.name}-${i}`} className={`flex flex-col gap-0.5 border-b border-separator py-1 ${c.name === focus ? "text-foreground" : "text-muted"}`}>
              <dt className="flex items-center gap-1.5 font-medium">
                <Icon name={c.primaryKey ? "key" : "text"} size={11} />
                {c.name}
                <span className="ml-auto font-mono text-[10px]">{c.dataType}</span>
              </dt>
              {row[i]?.t === "json" ? <JsonViewer bare value={row[i].v} defaultDepth={1} className="pl-2.5" /> : <dd className="selectable truncate font-mono">{shown(row[i])}</dd>}
            </div>
          ))}
        </dl>
      </ScrollArea>
    );
  }

  return (
    <>
      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-2 px-2.5 pb-2">
          {edit.disabledReason !== undefined ? <p className="rounded-md bg-surface-secondary/60 px-2 py-1 text-[11px] text-muted">{edit.disabledReason}</p> : null}
          {partial ? <p className="rounded-md bg-surface-secondary/60 px-2 py-1 text-[11px] text-muted">Loading the full values of this row…</p> : null}
          {columns.map((c, index) => {
            const base = merged(index);
            const cell = draft[c.name];
            const effective: Value = cell?.value ?? (base === "mixed" ? { t: "null" } : base);
            const isNull = effective.t === "null";
            const kind = fieldKind(c.dataType, effective, enumTraitsOf(c));
            const info = toColumnInfo(c, index);
            const traits = toTraits(c);
            const locked = !editable || c.readOnly === true || kind === "bytes";
            const dirty = cell !== undefined && changed.some(([name]) => name === c.name);
            const isFocus = c.name === focus;
            const formCell: FormCell = { value: effective, raw: cell?.raw };
            return (
              <div key={`${c.name}-${index}`} ref={isFocus ? focusRef : undefined} className={cn("flex flex-col gap-1 rounded-md", isFocus ? "bg-accent/10 p-1.5 -m-1.5" : "")}>
                <div className="flex items-center gap-1.5 text-xs font-medium text-foreground">
                  {dirty ? <span aria-label="Changed" className="size-1.5 shrink-0 rounded-full bg-warning" /> : null}
                  <Icon name={c.primaryKey ? "key" : "text"} size={11} className={c.primaryKey ? "text-warning" : "text-muted"} />
                  <span className="truncate">{c.name}</span>
                  <span className="truncate font-mono text-[10px] font-normal text-muted">{c.dataType}</span>
                  <span className="ml-auto flex shrink-0 items-center gap-1">
                    {c.nullable === false && !c.hasDefault && !c.autoIncrement && !c.readOnly ? (
                      <Badge size="sm" variant="soft" className="text-[10px]">
                        NOT NULL
                      </Badge>
                    ) : null}
                    {c.autoIncrement === true ? (
                      <Badge size="sm" variant="soft" className="text-[10px]">
                        Auto
                      </Badge>
                    ) : null}
                    {c.readOnly === true ? <Icon name="lock" size={11} className="text-muted" aria-label="Generated by the database" /> : null}
                  </span>
                </div>
                {locked ? (
                  base !== "mixed" || cell !== undefined ? (
                    effective.t === "json" ? (
                      <JsonViewer bare value={effective.v} defaultDepth={1} className="pl-2.5" />
                    ) : (
                      <p className={cn("selectable truncate rounded-md border border-border/40 bg-field px-2 py-1 font-mono text-[12px]", isNull ? "italic text-muted" : "text-foreground")}>{shown(effective)}</p>
                    )
                  ) : (
                    <p className="rounded-md border border-border/40 bg-field px-2 py-1 text-[12px] italic text-muted">Multiple values</p>
                  )
                ) : (
                  <div className="flex items-end gap-1.5">
                    <div className="min-w-0 flex-1">
                      {base === "mixed" && cell === undefined ? (
                        <Button variant="tertiary" className="w-full justify-start italic text-muted" onClick={() => put(c.name, emptyFor(c))} title={distinctSummary(rows, index)}>
                          Multiple values · set for all {rows.length}
                        </Button>
                      ) : (
                        <ValueControl
                          kind={kind}
                          column={info}
                          traits={traits}
                          placeholder={blankPlaceholder(info, traits)}
                          cell={formCell}
                          isNull={isNull}
                          onChange={(next) => put(c.name, next === undefined ? undefined : { value: next.value, raw: next.raw })}
                          set={(next) => {
                            if (next === undefined) put(c.name, c.nullable === false ? undefined : { value: { t: "null" } });
                            else put(c.name, { value: next });
                          }}
                        />
                      )}
                    </div>
                    {c.nullable !== false ? (
                      <Toggle
                        pressed={isNull}
                        onPressedChange={(on) => {
                          if (on) put(c.name, { value: { t: "null" } });
                          else if (base !== "mixed" && base.t !== "null") put(c.name, undefined);
                          else put(c.name, emptyFor(c));
                        }}
                        aria-label={isNull ? `Set a value for ${c.name}` : `Set ${c.name} to NULL`}
                        className="h-6.5 min-w-0 shrink-0 px-2 font-mono text-[10px]"
                      >
                        {isNull ? "Set value" : "NULL"}
                      </Toggle>
                    ) : null}
                  </div>
                )}
                {problems[c.name] ? <span className="text-[11px] text-danger">{problems[c.name]}</span> : null}
              </div>
            );
          })}
        </div>
      </ScrollArea>
      <div className="flex shrink-0 items-center gap-1.5 border-t border-border/40 glass-header p-2">
        <Button size="sm" variant="danger-soft" disabled={!editable} onClick={edit.onDelete}>
          <Icon name="trash" size={12} />
          Delete{rows.length > 1 ? ` ${rows.length}` : ""}
        </Button>
        <span className="ml-auto" />
        <Button size="sm" variant="tertiary" disabled={changed.length === 0 && Object.keys(draft).length === 0} onClick={() => setDraft({})}>
          Reset
        </Button>
        <Button size="sm" disabled={!editable || changed.length === 0 || blocked} onClick={apply}>
          Update{changed.length > 0 ? ` (${changed.length})` : ""}
        </Button>
      </div>
    </>
  );
}

function distinctSummary(rows: readonly (readonly Value[])[], index: number): string {
  const seen: string[] = [];
  for (const row of rows) {
    const text = shown(row[index]);
    if (!seen.includes(text)) seen.push(text);
    if (seen.length >= 8) break;
  }
  return seen.join("\n");
}
