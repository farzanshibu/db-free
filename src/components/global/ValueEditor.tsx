// SOT: value-editor, typed-cell-editor, insert-form-field, json-editor-modal, enum-editor, array-editor, vector-editor, token-editor
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { ColumnInfo, ColumnTraits, SqlToken, Value } from "@/lib/bindings";
import type { JsonValue } from "@/lib/bindings/serde_json/JsonValue";
import { parseJson } from "@/lib/json";
import { IS_MAC } from "@/lib/keymap";
import { editText, fieldKind, isArrayType, parseEdited, parseVectorText, vectorDimension, type EnumTraits, type FieldKind } from "@/lib/fields";
import { blankPlaceholder, isRequired } from "@/lib/columnTraits";
import { formatPgArray, parsePgArray } from "@/lib/pgArray";
import { tokenFromTyped, tokenValue } from "@/lib/sqlTokens";
import { AppSelect, Check, DateTimeField, Field, NumberInput } from "./Field";
import { JsonViewer } from "./JsonViewer";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input, SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { Toggle as ToggleButton } from "@/components/ui/toggle";

/// One candidate row from the table a foreign key points at.
export interface LookupRow {
  /// The referenced column's value, as text — what the cell will hold.
  value: string;
  /// The rest of the row, so the choice is made on more than an opaque id.
  detail: string;
}

/// What a form control holds for a column: the value, and the token when it is a
/// `now()` / DEFAULT rather than a literal. Absent = the column is left out.
export interface FormCell {
  value: Value;
  raw?: SqlToken | undefined;
}

interface CellEditorProps {
  typeName: string;
  value: Value;
  onCommit: (next: Value, raw?: SqlToken) => void;
  onCancel: () => void;
  /// Present on a foreign-key column: searches the referenced table.
  lookup?: ((search: string) => Promise<LookupRow[]>) | undefined;
  /// Allowed values of an ENUM / SET column; picks the dropdown or chip editor.
  traits?: EnumTraits | undefined;
  /// Backspace on an empty text cell sets NULL only where NULL is allowed.
  nullable?: boolean | undefined;
}

// WHAT:  Inline editor for one grid cell, picked by field kind:
//          text / decimal        → Input (typing `NULL` stages a null; Backspace on
//                                   an empty one does too)
//          int / float           → NumberInput
//          date / time / datetime → DateTimeField, text first (`NOW()` and
//                                   `CURRENT_TIMESTAMP` become the database's own
//                                   current time); the calendar opens from its button
//          json                  → JsonEditorModal (tree + raw)
//          enum / SET            → EnumEditor (dropdown list, or checklist for SET)
//          array (Postgres)      → ArrayEditor (chips)
//          vector                → Input that validates `[1.0, 2.0]` and its length
//        Enter commits, Escape cancels. Text and number editors also cancel on
//        blur. The latest draft lives in a ref so an Enter that also changes the
//        value (calendar cell) commits the new value.
// WHERE: src/features/grid/DataGrid.tsx
export function CellEditor({ typeName, value, onCommit, onCancel, lookup, traits, nullable = true }: CellEditorProps) {
  const kind = fieldKind(typeName, value, traits);
  const initialText = editText(value);
  const initialNumber = value.t === "int" || value.t === "float" ? value.v : null;
  // State drives the controls; the refs mirror it so a keydown that both changes
  // the value and commits (Enter on a calendar cell) commits the fresh draft.
  // Refs are written in event handlers only, never read during render.
  const draft = useRef<string>(initialText);
  const number = useRef<number | null>(initialNumber);
  const [text, setText] = useState(initialText);
  const [num, setNum] = useState<number | null>(initialNumber);
  const dateBox = useRef<HTMLDivElement | null>(null);

  // A date cell opens text-first: the field is focused with its text selected.
  useEffect(() => {
    if (kind === "date" || kind === "time" || kind === "datetime") dateBox.current?.querySelector("input")?.select();
  }, [kind]);

  const onKey = (e: KeyboardEvent<HTMLElement>, commit: () => void) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
    }
  };
  const commitToken = (raw: string): boolean => {
    const token = tokenFromTyped(raw);
    if (token === null) return false;
    onCommit(tokenValue(token), token);
    return true;
  };
  const commitText = () => onCommit(parseEdited(draft.current, typeName, value, traits));
  const commitNumber = () => onCommit(number.current === null ? { t: "null" } : { t: kind === "int" ? "int" : "float", v: number.current });
  const commitDate = () => {
    if (draft.current.trim() === "") onCommit({ t: "null" });
    else if (!commitToken(draft.current)) onCommit({ t: "date_time", v: draft.current.trim() });
  };

  // A foreign key is chosen from the rows it can point at, not typed from memory.
  if (lookup) {
    return <LookupPicker initial={initialText} load={lookup} onCommit={(next) => onCommit(parseEdited(next, typeName, value, traits))} onNull={() => onCommit({ t: "null" })} onCancel={onCancel} />;
  }

  switch (kind) {
    case "bool":
    case "bytes":
      return null;
    case "int":
    case "float":
      return (
        <div className="h-[calc(100%-4px)] w-full" onKeyDown={(e) => onKey(e, commitNumber)} onBlur={onCancel}>
          <NumberInput
            compact
            autoFocus
            integer={kind === "int"}
            ariaLabel="Edit number"
            value={num}
            onChange={(n) => {
              number.current = n;
              setNum(n);
            }}
          />
        </div>
      );
    case "date":
    case "time":
    case "datetime":
      return (
        // No confirm buttons: picking a date is the decision. The edit is staged
        // like every other cell edit and reviewed once in Pending Changes, so a
        // second ✓ here would be asking the same question twice. Escape cancels.
        <div ref={dateBox} className="h-[calc(100%-4px)] w-full min-w-0" onKeyDown={(e) => onKey(e, commitDate)}>
          <DateTimeField
            compact
            autoFocus
            kind={kind}
            ariaLabel={`Edit ${kind}`}
            value={text}
            onChange={(next) => {
              draft.current = next;
              setText(next);
            }}
            onDone={commitDate}
          />
        </div>
      );
    case "json":
      return (
        <JsonEditorModal
          open
          title={`Edit ${typeName}`}
          initial={value.t === "json" ? value.v : null}
          onSave={(v) => onCommit({ t: "json", v })}
          onClose={onCancel}
          secondaryAction={{ label: "Set NULL", onClick: () => onCommit({ t: "null" }) }}
        />
      );
    case "enum":
      return (
        <EnumEditor
          values={traits?.enumValues ?? []}
          multi={traits?.multi ?? false}
          arrayLiteral={isArrayType(typeName)}
          current={value}
          nullable={nullable}
          onCommit={(next) => onCommit(next)}
          onCancel={onCancel}
        />
      );
    case "array":
      return <ArrayEditor title={`Edit ${typeName}`} current={value} nullable={nullable} onCommit={(next) => onCommit(next)} onCancel={onCancel} />;
    case "vector": {
      const dimension = vectorDimension(typeName);
      const check = parseVectorText(text, dimension);
      return (
        <Input
          autoFocus
          value={text}
          onChange={(e) => {
            draft.current = e.target.value;
            setText(e.target.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              if (draft.current.trim() === "" && nullable) onCommit({ t: "null" });
              else if (check.ok) onCommit({ t: "text", v: check.text });
            } else if (e.key === "Escape") {
              e.preventDefault();
              onCancel();
            }
          }}
          onBlur={onCancel}
          title={check.ok ? "Enter to save" : check.error}
          aria-invalid={!check.ok}
          placeholder={dimension === null ? "[1.0, 2.0, 3.0]" : `${dimension} numbers, e.g. [${Array.from({ length: Math.min(dimension, 3) }, (_, i) => `${i + 1}.0`).join(", ")}${dimension > 3 ? ", …" : ""}]`}
          className={cn("h-[calc(100%-4px)] w-full rounded-sm border bg-background px-1 font-mono text-[12px] text-foreground", check.ok ? "border-accent" : "border-danger")}
          aria-label="Edit vector"
        />
      );
    }
    case "decimal":
    case "text":
      // A row-height input shows the tail of a user agent string and nothing
      // else, so anything long or multi-line is edited in a panel that shows all
      // of it. The threshold is the point where a grid cell stops being a
      // sensible place to read a value.
      if (kind === "text" && (text.length > LONG_TEXT || text.includes("\n"))) {
        return (
          <TextAreaEditor
            title={`Edit ${typeName}`}
            initial={text}
            onCommit={(next) => onCommit(parseEdited(next, typeName, value, traits))}
            onNull={() => onCommit({ t: "null" })}
            onCancel={onCancel}
          />
        );
      }
      return (
        <Input
          autoFocus
          value={text}
          onChange={(e) => {
            draft.current = e.target.value;
            setText(e.target.value);
          }}
          onKeyDown={(e) => {
            // Backspace on an already-empty cell clears it to NULL, as a spreadsheet clears a cell.
            if (e.key === "Backspace" && draft.current === "" && nullable) {
              e.preventDefault();
              onCommit({ t: "null" });
              return;
            }
            onKey(e, commitText);
          }}
          onBlur={onCancel}
          inputMode={kind === "decimal" ? "decimal" : undefined}
          className="h-[calc(100%-4px)] w-full rounded-sm border border-accent bg-background px-1 font-mono text-[12px] text-foreground"
          aria-label="Edit cell"
        />
      );
  }
}

// WHAT:  Picks one ENUM value from a list, or several for a SET / enum array.
// WHY:   Typing a label from memory is how an enum edit fails with "invalid input
//        value"; the column already knows the answers it accepts.
// HOW:   Single: choosing commits at once. Multi: a checklist and Done. A SET is
//        written `a,b` (MySQL), an enum array `{a,b}` (Postgres).
function EnumEditor({ values, multi, arrayLiteral, current, nullable, onCommit, onCancel }: { values: readonly string[]; multi: boolean; arrayLiteral: boolean; current: Value; nullable: boolean; onCommit: (next: Value) => void; onCancel: () => void }) {
  const initial = useMemo(() => selectedOf(current, multi, arrayLiteral), [current, multi, arrayLiteral]);
  const [selected, setSelected] = useState<readonly string[]>(initial);
  const [search, setSearch] = useState("");
  const needle = search.trim().toLowerCase();
  const shown = values.filter((v) => needle.length === 0 || v.toLowerCase().includes(needle));
  const write = (items: readonly string[]): Value => {
    const ordered = values.filter((v) => items.includes(v));
    return { t: "text", v: arrayLiteral ? formatPgArray(ordered) : ordered.join(",") };
  };
  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle>{multi ? "Choose values" : "Choose a value"}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          {values.length > 8 ? <SearchInput value={search} onChange={setSearch} aria-label="Search values" placeholder="Search…" autoFocus className="glass-input mb-1.5 w-full rounded-md text-xs" /> : null}
          <ScrollArea hideScrollBar className="max-h-72">
            <ul className="flex flex-col gap-0.5">
              {shown.map((label) => (
                <li key={label}>
                  {multi ? (
                    <span className="flex items-center gap-2 rounded-md px-2 py-1 text-[12px] hover:bg-surface-secondary/60">
                      <Check
                        label={label}
                        checked={selected.includes(label)}
                        onChange={(on) => setSelected((s) => (on ? [...s, label] : s.filter((x) => x !== label)))}
                      />
                      <span className="truncate font-mono">{label}</span>
                    </span>
                  ) : (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => onCommit({ t: "text", v: label })}
                      className={cn("flex h-auto w-full min-w-0 justify-start rounded-md px-2 py-1 text-left font-mono text-[12px]", initial.includes(label) ? "glass-pill text-accent" : "text-foreground hover:bg-surface-secondary/60")}
                    >
                      <span className="truncate">{label}</span>
                    </Button>
                  )}
                </li>
              ))}
              {shown.length === 0 ? <li className="px-2 py-1.5 text-xs text-muted">No matching values.</li> : null}
            </ul>
          </ScrollArea>
        </DialogBody>
        <DialogFooter>
          {nullable ? (
            <Button size="sm" variant="tertiary" onClick={() => onCommit({ t: "null" })}>
              Set NULL
            </Button>
          ) : null}
          <Button size="sm" variant="tertiary" onClick={onCancel}>
            Cancel
          </Button>
          {multi ? (
            <Button size="sm" onClick={() => onCommit(write(selected))}>
              Done
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// The labels a SET / enum-array cell currently holds.
function selectedOf(value: Value, multi: boolean, arrayLiteral: boolean): readonly string[] {
  if (value.t === "null") return [];
  const text = editText(value);
  if (!multi) return [text];
  if (arrayLiteral) return (parsePgArray(text) ?? []).filter((x): x is string => x !== null);
  return text.split(",").map((x) => x.trim()).filter((x) => x.length > 0);
}

// WHAT:  Chip editor for a Postgres array: add items, remove them, keep NULL elements.
// WHY:   `{"a b",c,NULL}` is easy to get wrong by hand — quoting, commas, NULL — and
//        the cell is written back as exactly that text.
// HOW:   A multi-dimensional array does not fit chips; it opens as plain text.
function ArrayEditor({ title, current, nullable, onCommit, onCancel }: { title: string; current: Value; nullable: boolean; onCommit: (next: Value) => void; onCancel: () => void }) {
  const initial = useMemo(() => (current.t === "null" ? [] : parsePgArray(editText(current))), [current]);
  const [items, setItems] = useState<(string | null)[]>(initial ?? []);
  const [draft, setDraft] = useState("");
  const [raw, setRaw] = useState(current.t === "null" ? "" : editText(current));
  const add = () => {
    if (draft.length === 0) return;
    setItems((list) => [...list, draft]);
    setDraft("");
  };
  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-[520px]">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          {initial === null ? (
            <>
              <p className="mb-1 text-[11px] text-muted">This array has more than one dimension; edit its text.</p>
              <Textarea value={raw} onChange={(e) => setRaw(e.target.value)} className="selectable h-32 w-full font-mono text-[12px]" aria-label="Array text" />
            </>
          ) : (
            <>
              <ScrollArea hideScrollBar className="max-h-52">
                <ul className="flex flex-wrap gap-1.5">
                  {items.map((item, index) => (
                    <li key={`${index}-${item ?? ""}`}>
                      <Badge size="sm" variant="soft" className="gap-1 font-mono">
                        <span className={item === null ? "italic text-muted" : ""}>{item ?? "NULL"}</span>
                        <Button variant="ghost" size="icon-sm" aria-label={`Remove ${item ?? "NULL"}`} onClick={() => setItems((list) => list.filter((_, i) => i !== index))} className="size-4 min-w-4">
                          <Icon name="x" size={10} />
                        </Button>
                      </Badge>
                    </li>
                  ))}
                  {items.length === 0 ? <li className="text-xs text-muted">Empty array.</li> : null}
                </ul>
              </ScrollArea>
              <div className="mt-2 flex items-center gap-1.5">
                <Input
                  autoFocus
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      add();
                    }
                  }}
                  placeholder="Add an item, press Enter"
                  aria-label="New array item"
                  className="flex-1 font-mono text-xs"
                />
                <Button size="sm" variant="tertiary" onClick={add} disabled={draft.length === 0}>
                  Add
                </Button>
                <Button size="sm" variant="tertiary" onClick={() => setItems((list) => [...list, null])}>
                  Add NULL
                </Button>
              </div>
            </>
          )}
        </DialogBody>
        <DialogFooter>
          {nullable ? (
            <Button size="sm" variant="tertiary" onClick={() => onCommit({ t: "null" })}>
              Set NULL
            </Button>
          ) : null}
          <Button size="sm" variant="tertiary" onClick={onCancel}>
            Cancel
          </Button>
          <Button size="sm" onClick={() => onCommit({ t: "text", v: initial === null ? raw : formatPgArray(draft.length > 0 ? [...items, draft] : items) })}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// WHAT:  Picks a foreign-key value from the referenced table, with each row's
//        own columns shown next to the id.
// WHY:   Editing an FK meant opening the other table, copying an id and coming
//        back; the id alone says nothing about which row it is.
// HOW:   The search runs against the referenced table on every keystroke the
//        caller allows, so it works on any engine that can filter a page.
function LookupPicker({ initial, load, onCommit, onNull, onCancel }: { initial: string; load: (search: string) => Promise<LookupRow[]>; onCommit: (next: string) => void; onNull: () => void; onCancel: () => void }) {
  // The field is both the search and the value: pick a row, or type a value the
  // list does not offer (a row created elsewhere, an id not yet inserted) and
  // press Enter. A foreign key the database has not seen yet is the database's
  // to reject, not this dialog's.
  const [search, setSearch] = useState(initial);
  // One state for the whole request, so a keystroke cannot leave "loading" and
  // "failed" describing different searches.
  const [result, setResult] = useState<{ rows: LookupRow[]; failed: boolean } | null>(null);

  useEffect(() => {
    let cancelled = false;
    void load(search)
      .then((rows) => {
        if (!cancelled) setResult({ rows, failed: false });
      })
      .catch(() => {
        if (!cancelled) setResult({ rows: [], failed: true });
      });
    return () => {
      cancelled = true;
    };
  }, [search, load]);

  const rows = result?.rows ?? null;
  const failed = result?.failed ?? false;

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-[640px]">
        <DialogHeader>
          <DialogTitle>Choose a referenced row</DialogTitle>
        </DialogHeader>
        <DialogBody>
          <SearchInput value={search} onChange={setSearch} aria-label="Search or type a referenced value" placeholder="Search or type a value…" autoFocus className="glass-input w-full rounded-md font-mono text-xs" />
          <ScrollArea hideScrollBar className="mt-2 max-h-72">
            {rows === null ? (
              <p className="px-1 py-1.5 text-xs text-muted">Loading…</p>
            ) : rows.length === 0 ? (
              <p className="px-1 py-1.5 text-xs text-muted">
                {failed ? "Could not read the referenced table — the typed value is still usable." : "No matching rows. Press Enter to use what you typed."}
              </p>
            ) : (
              <ul className="flex flex-col gap-0.5">
                {rows.map((row) => (
                  <li key={row.value}>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => onCommit(row.value)}
                      className={cn(
                        "flex h-auto w-full min-w-0 flex-col items-start gap-0.5 rounded-md px-2 py-1 text-left",
                        row.value === initial ? "glass-pill text-accent" : "text-foreground hover:bg-surface-secondary/60",
                      )}
                    >
                      <span className="truncate font-mono text-[12px]">{row.value}</span>
                      {row.detail.length > 0 ? <span className="truncate text-[11px] text-muted">{row.detail}</span> : null}
                    </Button>
                  </li>
                ))}
              </ul>
            )}
          </ScrollArea>
        </DialogBody>
        <DialogFooter>
          <Button size="sm" variant="tertiary" onClick={onNull}>Set NULL</Button>
          <Button size="sm" variant="tertiary" onClick={onCancel}>Cancel</Button>
          <Button size="sm" disabled={search.trim().length === 0} onClick={() => onCommit(search.trim())}>
            Use typed value
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// Length past which a value is edited in a panel rather than in the cell.
const LONG_TEXT = 80;

// WHAT:  Full-value editor for long or multi-line text: the whole value, wrapped,
//        resizable, with the length shown.
// WHY:   Tokens, user agents, URLs and JSON strings are common cell values and
//        unreadable in a 28px row.
// WHERE: CellEditor (text branch), src/features/grid/DataGrid.tsx
function TextAreaEditor({ title, initial, onCommit, onNull, onCancel }: { title: string; initial: string; onCommit: (next: string) => void; onNull: () => void; onCancel: () => void }) {
  const [text, setText] = useState(initial);
  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-[720px]">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <DialogBody>
          <Textarea
            autoFocus
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              // Enter is a newline here; Cmd/Ctrl+Enter commits.
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                onCommit(text);
              } else if (e.key === "Escape") {
                e.preventDefault();
                onCancel();
              }
            }}
            className="selectable h-72 w-full resize-y rounded-lg border border-border/40 bg-background p-2 font-mono text-[12px] whitespace-pre-wrap text-foreground"
            aria-label={title}
          />
          <p className="mt-1 text-[11px] text-muted">{text.length.toLocaleString()} characters · {IS_MAC ? "⌘" : "Ctrl+"}↵ to save</p>
        </DialogBody>
        <DialogFooter>
          <Button size="sm" variant="tertiary" onClick={onNull}>Set NULL</Button>
          <Button size="sm" variant="tertiary" onClick={onCancel}>Cancel</Button>
          <Button size="sm" onClick={() => onCommit(text)}>Save</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface FormValueFieldProps {
  column: ColumnInfo;
  /// What the database knows about the column; omitted = nothing beyond the catalog.
  traits?: ColumnTraits | undefined;
  /// undefined = omitted (database default applies); { t: "null" } = explicit NULL.
  cell: FormCell | undefined;
  onChange: (next: FormCell | undefined) => void;
}

const NO_TRAITS: ColumnTraits = { name: "", enumValues: [], multi: false, hasDefault: false, defaultExpr: null, generated: false, autoIncrement: false };

// WHAT:  Insert-form control for one column, same kind → control mapping as
//        the grid, plus a NULL toggle for nullable columns and the database's
//        own answer for a blank one ("Auto", "DEFAULT now()", "REQUIRED").
// HOW:   A generated column is shown locked and never written. A blank column is
//        left out of the INSERT so the database fills it in.
// WHERE: src/features/grid/TableTab.tsx (InsertRowModal)
export function FormValueField({ column, traits = NO_TRAITS, cell, onChange }: FormValueFieldProps) {
  const kind = fieldKind(column.dataType, undefined, traits);
  const value = cell?.value;
  const isNull = value?.t === "null";
  const required = isRequired(column, traits);
  const placeholder = blankPlaceholder(column, traits);
  const set = (next: Value | undefined) => onChange(next === undefined ? undefined : { value: next });
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1.5 text-xs font-medium text-foreground">
        <span className="truncate">{column.name}</span>
        {column.primaryKey ? <Icon name="key" size={11} className="shrink-0 text-warning" /> : null}
        <span className="truncate font-mono text-[10px] font-normal text-muted">{column.dataType}</span>
        {required && value === undefined ? (
          <Badge size="sm" variant="soft" color="danger" className="ml-auto text-[9px]">
            REQUIRED
          </Badge>
        ) : null}
        {traits.generated ? <Icon name="lock" size={11} className="ml-auto shrink-0 text-muted" /> : null}
      </div>
      <div className="flex items-end gap-1.5">
        <div className="min-w-0 flex-1">
          {traits.generated ? (
            <Input disabled value="" placeholder="Generated by the database" aria-label={`${column.name} is generated`} className="font-mono text-xs" />
          ) : (
            <ValueControl kind={kind} column={column} traits={traits} placeholder={placeholder} cell={cell} isNull={isNull} onChange={onChange} set={set} />
          )}
        </div>
        {column.nullable && !traits.generated ? (
          <ToggleButton pressed={isNull} onPressedChange={(selected) => set(selected ? { t: "null" } : undefined)} aria-label={`Set ${column.name} to NULL`} className="mb-px h-6.5 min-w-0 shrink-0 px-2 font-mono text-[10px]">
            NULL
          </ToggleButton>
        ) : null}
      </div>
    </div>
  );
}

// WHAT:  The control for one value, by field kind — shared by the insert form and the
//        record inspector so a `timestamp` is edited the same way in both.
export function ValueControl({ kind, column, traits, placeholder, cell, isNull, onChange, set }: { kind: FieldKind; column: ColumnInfo; traits: ColumnTraits; placeholder: string; cell: FormCell | undefined; isNull: boolean; onChange: (next: FormCell | undefined) => void; set: (next: Value | undefined) => void }) {
  const [jsonOpen, setJsonOpen] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const value = cell?.value;
  const label = column.name;
  switch (kind) {
    case "int":
    case "float":
      return <NumberInput ariaLabel={label} integer={kind === "int"} disabled={isNull} value={value?.t === "int" || value?.t === "float" ? value.v : null} onChange={(n) => set(n === null ? undefined : { t: kind === "int" ? "int" : "float", v: n })} />;
    case "bool":
      return (
        <AppSelect
          ariaLabel={label}
          disabled={isNull}
          value={value?.t === "bool" ? (value.v ? "true" : "false") : "default"}
          options={[
            { value: "default", label: placeholder },
            { value: "true", label: "true" },
            { value: "false", label: "false" },
          ]}
          onChange={(v) => set(v === "default" ? undefined : { t: "bool", v: v === "true" })}
        />
      );
    case "date":
    case "time":
    case "datetime": {
      const token = cell?.raw;
      return (
        <div className="flex items-end gap-1">
          <div className="min-w-0 flex-1">
            <DateTimeField kind={kind} ariaLabel={label} disabled={isNull} value={token !== undefined ? "" : value?.t === "date_time" ? value.v : ""} onChange={(t) => set(t === "" ? undefined : { t: "date_time", v: t })} />
          </div>
          <Button
            variant={token !== undefined ? "soft" : "tertiary"}
            size="sm"
            disabled={isNull}
            onClick={() => (token !== undefined ? onChange(undefined) : onChange({ value: tokenValue(kind === "date" ? "current_date" : kind === "time" ? "current_time" : "now"), raw: kind === "date" ? "current_date" : kind === "time" ? "current_time" : "now" }))}
            aria-label={`Set ${label} to the current time`}
            className="h-6.5 shrink-0 px-2 font-mono text-[10px]"
          >
            {token !== undefined ? tokenText(token) : "now()"}
          </Button>
        </div>
      );
    }
    case "json": {
      const summary = value?.t === "json" ? JSON.stringify(value.v) : "";
      return (
        <div className="flex w-full flex-col gap-1">
          <Button variant="tertiary" disabled={isNull} onClick={() => setJsonOpen(true)} className="w-full justify-start truncate font-mono text-xs">
            <Icon name="braces" size={13} className="shrink-0 text-accent" />
            <span className={cn("truncate", summary ? "text-foreground" : "text-muted")}>{summary || (placeholder === column.dataType ? "Edit JSON…" : placeholder)}</span>
          </Button>
          <JsonEditorModal
            open={jsonOpen}
            title={column.name}
            initial={value?.t === "json" ? value.v : null}
            onSave={(v) => {
              set({ t: "json", v });
              setJsonOpen(false);
            }}
            onClose={() => setJsonOpen(false)}
            secondaryAction={{
              label: "Use default",
              onClick: () => {
                set(undefined);
                setJsonOpen(false);
              },
            }}
          />
        </div>
      );
    }
    case "enum": {
      const options = [{ value: "", label: placeholder }, ...traits.enumValues.map((v) => ({ value: v, label: v }))];
      if (traits.multi) {
        const text = value === undefined || isNull ? "" : editText(value);
        return (
          <>
            <Button variant="tertiary" disabled={isNull} onClick={() => setPickerOpen(true)} className="w-full justify-start truncate font-mono text-xs">
              <span className={cn("truncate", text ? "text-foreground" : "text-muted")}>{text || placeholder}</span>
            </Button>
            {pickerOpen ? (
              <EnumEditor
                values={traits.enumValues}
                multi
                arrayLiteral={isArrayType(column.dataType)}
                current={value ?? { t: "text", v: "" }}
                nullable={false}
                onCommit={(next) => {
                  set(next);
                  setPickerOpen(false);
                }}
                onCancel={() => setPickerOpen(false)}
              />
            ) : null}
          </>
        );
      }
      return <AppSelect ariaLabel={label} disabled={isNull} value={value?.t === "text" ? value.v : ""} options={options} onChange={(v) => set(v === "" ? undefined : { t: "text", v })} />;
    }
    case "array": {
      const text = value === undefined || isNull ? "" : editText(value);
      return (
        <>
          <div className="flex items-end gap-1">
            <div className="min-w-0 flex-1">
              <Field label={label} className="[&_label]:hidden" mono disabled={isNull} value={text} onChange={(t) => set(t.length === 0 ? undefined : { t: "text", v: t })} placeholder={placeholder === column.dataType ? "{a,b,c}" : placeholder} />
            </div>
            <Button variant="tertiary" disabled={isNull} onClick={() => setPickerOpen(true)} aria-label={`Edit ${label} items`} className="shrink-0 px-2">
              <Icon name="list" size={13} />
            </Button>
          </div>
          {pickerOpen ? (
            <ArrayEditor
              title={`Edit ${column.name}`}
              current={value ?? { t: "text", v: "{}" }}
              nullable={false}
              onCommit={(next) => {
                set(next);
                setPickerOpen(false);
              }}
              onCancel={() => setPickerOpen(false)}
            />
          ) : null}
        </>
      );
    }
    case "vector": {
      const text = value === undefined || isNull ? "" : editText(value);
      const check = parseVectorText(text, vectorDimension(column.dataType));
      return (
        <div className="flex flex-col gap-0.5">
          <Field label={label} className="[&_label]:hidden" mono disabled={isNull} value={text} onChange={(t) => set(t.length === 0 ? undefined : { t: "text", v: t.trim().startsWith("[") ? t : `[${t}]` })} placeholder={placeholder === column.dataType ? "[1.0, 2.0, 3.0]" : placeholder} />
          {text.length > 0 && !check.ok ? <span className="text-[10px] text-danger">{check.error}</span> : null}
        </div>
      );
    }
    case "bytes":
    case "decimal":
    case "text":
      return (
        <Field
          label={label}
          className="[&_label]:hidden"
          mono
          disabled={isNull}
          value={isNull || value === undefined ? "" : editText(value)}
          onChange={(t) => set(t.length === 0 ? undefined : parseEdited(t, column.dataType, undefined, traits))}
          placeholder={placeholder}
        />
      );
  }
}

function tokenText(token: SqlToken): string {
  const shown = tokenValue(token);
  return shown.t === "text" ? shown.v : "";
}

interface JsonEditorModalProps {
  open: boolean;
  title: string;
  initial: JsonValue | null;
  onSave: (value: JsonValue) => void;
  onClose: () => void;
  secondaryAction?: { label: string; onClick: () => void };
}

// WHAT:  JSON editor: raw text on the left (Format / Minify), live collapsible
//        tree on the right; Save is disabled until the text parses.
export function JsonEditorModal({ open, title, initial, onSave, onClose, secondaryAction }: JsonEditorModalProps) {
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-[940px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Icon name="braces" size={14} className="text-accent" />
            {title}
          </DialogTitle>
        </DialogHeader>
        {open ? <JsonEditorBody initial={initial} onSave={onSave} onClose={onClose} {...(secondaryAction ? { secondaryAction } : {})} /> : null}
      </DialogContent>
    </Dialog>
  );
}

function JsonEditorBody({ initial, onSave, onClose, secondaryAction }: Omit<JsonEditorModalProps, "open" | "title">) {
  const [text, setText] = useState(() => (initial === null ? "" : JSON.stringify(initial, null, 2)));
  const parsed = useMemo(() => (text.trim().length === 0 ? undefined : parseJson(text)), [text]);
  const valid = parsed !== undefined;
  return (
    <>
      <DialogBody className="max-h-[70vh] overflow-hidden">
        <div className="grid h-[52vh] min-h-[320px] grid-cols-1 gap-2 md:grid-cols-2">
          <div className="flex min-h-0 flex-col gap-1.5">
            <div className="flex h-6 items-center gap-1 text-xs text-muted">
              <span>Raw</span>
              <span className="ml-auto" />
              <Button size="sm" variant="ghost" disabled={!valid} className="h-6 min-w-0 rounded-md px-1.5 text-[11px]" onClick={() => parsed !== undefined && setText(JSON.stringify(parsed, null, 2))}>
                Format
              </Button>
              <Button size="sm" variant="ghost" disabled={!valid} className="h-6 min-w-0 rounded-md px-1.5 text-[11px]" onClick={() => parsed !== undefined && setText(JSON.stringify(parsed))}>
                Minify
              </Button>
            </div>
            <Textarea
              aria-label="JSON source"
              value={text}
              onChange={(e) => setText(e.target.value)}
              spellCheck={false}
              placeholder='{ "key": "value" }'
              className={cn("min-h-0 flex-1 resize-none font-mono text-[12px] leading-relaxed", valid ? "" : "border-danger")}
            />
            <span className={cn("h-4 text-[11px]", valid ? "text-muted" : "text-danger")}>{valid ? (parsed === null ? "null" : Array.isArray(parsed) ? `array · ${parsed.length} items` : typeof parsed === "object" ? `object · ${Object.keys(parsed).length} keys` : typeof parsed) : "Not valid JSON."}</span>
          </div>
          <div className="flex min-h-0 min-w-0 flex-col gap-1.5">
            <div className="flex h-6 items-center text-xs text-muted">Tree</div>
            <ScrollArea className="min-h-0 flex-1 overflow-x-auto rounded-lg border border-border/40 bg-background/60 p-2">
              {parsed !== undefined ? <JsonViewer value={parsed} defaultDepth={3} /> : <span className="text-xs text-muted">Fix the JSON on the left to preview it here.</span>}
            </ScrollArea>
          </div>
        </div>
      </DialogBody>
      <DialogFooter>
        {secondaryAction ? (
          <Button variant="ghost" className="mr-auto text-muted hover:text-foreground" onClick={secondaryAction.onClick}>
            {secondaryAction.label}
          </Button>
        ) : null}
        <Button variant="tertiary" onClick={onClose}>
          Cancel
        </Button>
        <Button disabled={!valid} onClick={() => parsed !== undefined && onSave(parsed)}>
          Save
        </Button>
      </DialogFooter>
    </>
  );
}
