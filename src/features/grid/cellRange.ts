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

export interface RangeStats {
  cells: number;
  /// Cells holding something other than NULL.
  filled: number;
  /// Cells that are numbers (int, float, decimal); sum/avg/min/max cover only these.
  numeric: number;
  sum: number;
  min: number | null;
  max: number | null;
  /// True when the range was larger than STATS_CELL_LIMIT and only its start was read.
  partial: boolean;
}

/// Cells read per stats pass. A Ctrl+A over a million rows would otherwise
/// walk every cell on each selection change.
export const STATS_CELL_LIMIT = 250_000;

// WHAT:  Count / sum / average / min / max over a selected range.
// WHY:   "What is the total of these?" is the first question after selecting
//        a column of amounts; copying to a spreadsheet to find out is a detour.
// HOW:   Only number-typed values are aggregated; text that looks numeric is
//        not, because the grid cannot tell a price from a zip code. Rows not
//        loaded yet count as cells but not as values.
// WHERE: src/features/grid/DataGrid.tsx (status strip under the grid)
export function rangeStats(b: RangeBounds, valueAt: (row: number, col: number) => Value | undefined): RangeStats {
  const out: RangeStats = { cells: cellCount(b), filled: 0, numeric: 0, sum: 0, min: null, max: null, partial: false };
  let read = 0;
  for (let r = b.top; r <= b.bottom; r += 1) {
    for (let c = b.left; c <= b.right; c += 1) {
      read += 1;
      if (read > STATS_CELL_LIMIT) {
        out.partial = true;
        return out;
      }
      const value = valueAt(r, c);
      if (value === undefined || value.t === "null") continue;
      out.filled += 1;
      const n = numberOf(value);
      if (n === null) continue;
      out.numeric += 1;
      out.sum += n;
      out.min = out.min === null ? n : Math.min(out.min, n);
      out.max = out.max === null ? n : Math.max(out.max, n);
    }
  }
  return out;
}

function numberOf(value: Value): number | null {
  switch (value.t) {
    case "int":
    case "float":
      return Number.isFinite(value.v) ? value.v : null;
    case "decimal": {
      const n = Number(value.v);
      return Number.isFinite(n) ? n : null;
    }
    case "null":
    case "bool":
    case "text":
    case "bytes":
    case "json":
    case "date_time":
    case "unsupported":
      return null;
  }
}

/// A stat for display: integers stay exact, fractions get at most 6 places.
export function formatStat(n: number): string {
  if (Number.isInteger(n)) return n.toLocaleString("en-US");
  return n.toLocaleString("en-US", { maximumFractionDigits: 6 });
}
