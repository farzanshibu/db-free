// SOT: inspector-json-tab, row-json-editor, json-format-button
import { useMemo, useState } from "react";
import type { Value } from "@/lib/bindings";
import { parseJson } from "@/lib/json";
import { cn } from "@/lib/cn";
import { JsonViewer } from "@/components/global/JsonViewer";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { changesFromJson, plainRecord, type InspectorColumn, type InspectorEdit } from "./inspectorValues";

interface JsonTabProps {
  columns: readonly InspectorColumn[];
  rows: readonly (readonly Value[])[];
  edit: InspectorEdit | undefined;
  partial: boolean;
}

// WHAT:  The inspector's JSON tab: the row as an object. With editing on it is a text
//        area with Format, Reset and Update; several rows, or a read-only inspector,
//        show it as a tree.
// WHY:   Pasting a whole record, or fixing three fields in a JSON column's neighbours,
//        is quicker as text than as a form.
// HOW:   Update compares the edited object with the real row and stages only the
//        cells that differ, each parsed by its column's type (same rules as the
//        cell editor). A key that is not a column, a generated column and a NULL in a
//        NOT NULL column are refused with the reason.
// WHERE: src/features/grid/inspectorValues.ts (`changesFromJson`)
export function InspectorJson({ columns, rows, edit, partial }: JsonTabProps) {
  const row = rows[0];
  const original = useMemo(() => (row === undefined ? "{}" : JSON.stringify(plainRecord(columns, row), null, 2)), [columns, row]);
  const [text, setText] = useState<string | null>(null);
  const shown = text ?? original;
  const editable = edit !== undefined && edit.disabledReason === undefined && !partial && rows.length === 1;

  const result = useMemo(() => (editable && row !== undefined && text !== null ? changesFromJson(shown, columns, row, parseJson) : null), [editable, row, text, shown, columns]);
  const valid = parseJson(shown) !== undefined;

  if (!editable || row === undefined) {
    const records = rows.map((r) => plainRecord(columns, r));
    const only = records.length === 1 ? records[0] : undefined;
    return (
      <ScrollArea className="min-h-0 flex-1">
        <JsonViewer value={only ?? records} className="p-2.5" />
      </ScrollArea>
    );
  }
  const apply = () => {
    if (!result?.ok) return;
    const edits = [];
    for (const [name, cell] of result.changes) {
      const col = columns.findIndex((c) => c.name === name);
      if (col >= 0) edits.push({ row: 0, col, value: cell.value, raw: cell.raw });
    }
    if (edits.length > 0) edit.onApply(edits);
    setText(null);
  };
  const changes = result?.ok ? result.changes.size : 0;
  return (
    <>
      <div className="flex min-h-0 flex-1 flex-col gap-1.5 px-2.5 pb-1.5">
        <Textarea
          aria-label="Row as JSON"
          value={shown}
          onChange={(e) => setText(e.target.value)}
          spellCheck={false}
          className={cn("selectable min-h-0 flex-1 resize-none font-mono text-[11px] leading-relaxed", valid ? "" : "border-danger")}
        />
        <span className={cn("min-h-4 text-[11px]", result !== null && !result.ok ? "text-danger" : "text-muted")}>
          {result === null ? "Edit the object, then Update." : result.ok ? (changes === 0 ? "No changes." : `${changes} ${changes === 1 ? "column" : "columns"} will change.`) : result.error}
        </span>
      </div>
      <div className="flex shrink-0 items-center gap-1.5 border-t border-border/40 glass-header p-2">
        <Button
          size="sm"
          variant="tertiary"
          disabled={!valid}
          onClick={() => {
            const parsed = parseJson(shown);
            if (parsed !== undefined) setText(JSON.stringify(parsed, null, 2));
          }}
        >
          Format
        </Button>
        <span className="ml-auto" />
        <Button size="sm" variant="tertiary" disabled={text === null} onClick={() => setText(null)}>
          Reset
        </Button>
        <Button size="sm" disabled={result === null || !result.ok || changes === 0} onClick={apply}>
          Update{changes > 0 ? ` (${changes})` : ""}
        </Button>
      </div>
    </>
  );
}
