// SOT: cell-range-selection, grid-clipboard, tsv-serialization, tsv-parsing
import type { Value } from "@/lib/bindings";
import { editText } from "@/lib/fields";

export interface CellPos {
  row: number;
  col: number;
}

/// A rectangular selection: where the drag started and where it is now.
export interface CellRange {
  anchor: CellPos;
  focus: CellPos;
}

export interface RangeBounds {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

// WHAT:  Rectangular cell selection + spreadsheet-style clipboard for DataGrid.
// WHY:   Drag-select and Ctrl+C / Ctrl+V are how people move data between a
//        grid, a spreadsheet and another table; TSV is what Excel, Sheets and
//        every other grid put on (and read off) the clipboard.
// HOW:   Pure helpers: the grid owns the range state and the events, this owns
//        the geometry and the text format. NULL copies as `NULL` so a copy
//        pasted back into the app round-trips through `parseEdited`.
// WHERE: src/features/grid/DataGrid.tsx
export function bounds(range: CellRange): RangeBounds {
  return {
    top: Math.min(range.anchor.row, range.focus.row),
    bottom: Math.max(range.anchor.row, range.focus.row),
    left: Math.min(range.anchor.col, range.focus.col),
    right: Math.max(range.anchor.col, range.focus.col),
  };
}

export function inBounds(b: RangeBounds, row: number, col: number): boolean {
  return row >= b.top && row <= b.bottom && col >= b.left && col <= b.right;
}

export function cellCount(b: RangeBounds): number {
  return (b.bottom - b.top + 1) * (b.right - b.left + 1);
}

/// Clipboard text for one value.
export function clipboardText(value: Value | undefined): string {
  if (value === undefined) return "";
  if (value.t === "null") return "NULL";
  return editText(value);
}

/// One TSV field, quoted only when it holds a tab, newline or quote.
function tsvField(text: string): string {
  return /[\t\n\r"]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

export function toTsv(rows: readonly (readonly string[])[]): string {
  return rows.map((r) => r.map(tsvField).join("\t")).join("\n");
}

// WHAT:  Parses clipboard TSV into rows of fields.
// HOW:   Excel quotes a field that contains a tab, newline or quote and
//        doubles inner quotes; a trailing newline (every spreadsheet adds one)
//        does not make an extra empty row.
export function parseTsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;
  let atFieldStart = true;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text.charAt(i);
    if (quoted) {
      if (ch === '"' && text.charAt(i + 1) === '"') {
        field += '"';
        i += 1;
      } else if (ch === '"') {
        quoted = false;
      } else {
        field += ch;
      }
      continue;
    }
    if (ch === '"' && atFieldStart) {
      quoted = true;
      atFieldStart = false;
    } else if (ch === "\t") {
      row.push(field);
      field = "";
      atFieldStart = true;
    } else if (ch === "\n" || ch === "\r") {
      if (ch === "\r" && text.charAt(i + 1) === "\n") i += 1;
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
      atFieldStart = true;
    } else {
      field += ch;
      atFieldStart = false;
    }
  }
  if (field.length > 0 || row.length > 0) {
    row.push(field);
    rows.push(row);
  }
  return rows;
}
