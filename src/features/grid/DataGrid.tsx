// SOT: data-grid, virtualized-grid, grid-cell-rendering, column-sort-header, column-resize, row-selection, inline-cell-edit, foreign-key-link, change-highlighting, drag-select, grid-copy-paste, selection-stats
import { useEffect, useMemo, useReducer, useRef, useState, type ClipboardEvent as ReactClipboardEvent, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { FilterRule, SortRule, Value } from "@/lib/bindings";
import { cellClass, formatCell } from "@/lib/format";
import { fieldKind, parseEdited } from "@/lib/fields";
import { Icon, typeIcon } from "@/lib/icons";
import { Check } from "@/components/global/Field";
import { CellEditor, type LookupRow } from "@/components/global/ValueEditor";
import { Resizer } from "@/components/global/Resizer";
import { useContextMenu, type MenuEntry } from "@/components/global/ContextMenu";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { bounds, cellCount, clipboardText, formatStat, inBounds, parseTsv, rangeStats, STATS_CELL_LIMIT, toTsv, type CellPos, type CellRange, type RangeBounds } from "./cellRange";

export interface GridColumn {
  name: string;
  typeName: string;
  primaryKey?: boolean;
  /// Set when the column is a foreign key; cells get a link button.
  linkTo?: string;
}

/// A cell with a staged (uncommitted) edit: what it shows now and what the database still holds.
export interface StagedCell {
  value: Value;
  old: Value;
}

/// One cell write from a paste: grid coordinates and the parsed value.
export interface CellEdit {
  row: number;
  col: number;
  value: Value;
}

interface DataGridProps {
  columns: readonly GridColumn[];
  rowCount: number;
  getRow: (index: number) => readonly Value[] | undefined;
  rowHeight: number;
  onRangeChange?: (start: number, end: number) => void;
  onCellSelect?: (rowIndex: number, colIndex: number) => void;
  selected?: { row: number; col: number } | null;
  sort?: readonly SortRule[];
  onSortToggle?: (column: string) => void;
  selectedRows?: ReadonlySet<number>;
  onToggleRow?: (rowIndex: number) => void;
  onToggleAll?: () => void;
  /// Present = cells are editable (double-click). Receives the edited Value.
  onCellEdit?: (rowIndex: number, colIndex: number, next: Value) => void;
  /// Batch form of onCellEdit for a paste: one call, so the owner stages or
  /// commits every pasted cell together. Falls back to onCellEdit per cell.
  onCellsEdit?: (edits: readonly CellEdit[]) => void;
  /// Feedback for a paste (how many cells, or why nothing was pasted).
  onPasteNotice?: (message: string) => void;
  /// Cells with a staged edit render highlighted with the staged value ("row:col" keys).
  staged?: ReadonlyMap<string, StagedCell>;
  /// Rows staged for deletion: red tint, struck through, not editable.
  deletedRows?: ReadonlySet<number>;
  /// Rows at or after this index are staged inserts: green tint, edits update the insert.
  insertedFrom?: number;
  nullDisplay?: string;
  /// Foreign-key traversal: called from the link button in a linked column's cell.
  onLinkOpen?: (rowIndex: number, colIndex: number) => void;
  /// Right-click menus build filter rules from the clicked value. The owner
  /// sends them with the page request, so every engine honours them: SQL
  /// adapters push them into WHERE, the rest filter the fetched page.
  onFilter?: (rule: FilterRule) => void;
  /// Right-click menu on a header sets an explicit direction (onSortToggle cycles).
  onSortSet?: (rule: SortRule) => void;
  onClearSort?: () => void;
  /// Feedback for the copy actions.
  onCopied?: (what: string) => void;
  /// Zebra striping; off gives every row the same ground.
  alternatingRows?: boolean;
  /// Row actions offered by the cell menu when the owner supports them.
  onInsertRow?: () => void;
  onDuplicateRow?: (rowIndex: number) => void;
  onDeleteRow?: (rowIndex: number) => void;
  /// Opens the owner's record inspector on the clicked cell.
  onInspect?: (rowIndex: number, colIndex: number) => void;
  /// Copies whole rows in a format the owner knows how to build (JSON, CSV, SQL).
  onCopyRows?: (rowIndex: number, format: "json" | "csv" | "sql") => void;
  /// Searches the table a foreign-key column points at, so the value is picked
  /// from real rows instead of copied from another tab.
  onLookup?: (colIndex: number, search: string) => Promise<LookupRow[]>;
}

const HEADER_HEIGHT = 32;
const CHECK_WIDTH = 40;
const MIN_COL_WIDTH = 56;
/// A dragged column may still go wide; only the automatic sizing is capped.
const MAX_COL_WIDTH = 1600;
/// Ceiling for a column the app sizes itself: past this, long values are for
/// the inspector, not the grid.
const MAX_FIT_WIDTH = 450;
/// Room a fitted column leaves around its text: cell padding, the sort icon and
/// enough gap that neighbouring values never read as one string.
const FIT_GAP = 50;
/// Rough advance width of the 12px monospace grid face, in px per character.
const CHAR_WIDTH = 7.2;
/// Grab area straddling a column's right edge. Wide enough to hit without
/// aiming: the visible line inside it stays 1px.
const HANDLE_WIDTH = 12;

// WHAT:  Two-axis virtualized grid: only visible rows AND columns are mounted.
// WHY:   PRD §4.2 — 10^6 rows at 60 FPS. Row data is pulled through `getRow`
//        so the owner keeps the data and fetches on demand.
// HOW:   TanStack Virtual for both axes; absolute positioning inside a single
//        scroll container; the header and the checkbox gutter are sticky.
//        Editing: double-click opens the typed CellEditor for the column kind
//        (text, number, date/time picker, JSON modal); Enter commits, Esc cancels.
//        Change highlighting: staged cell = amber with a left bar and a
//        "was: …" tooltip, staged delete = red strike-through row, staged
//        insert = green row appended after the fetched page.
//        Column resize: a Resizer straddles each header's right edge; widths
//        are keyed by column name so they survive a refetch, double-click
//        restores the type-based estimate.
//        Range selection: press and drag across cells (Shift+click or
//        Shift+arrows extend, Ctrl+A selects all). Ctrl+C copies the range as
//        TSV; Ctrl+V pastes TSV from the top-left cell of the range (a single
//        value fills the whole range) when the owner accepts edits.
// WHERE: src/features/grid/TableTab.tsx, src/features/editor/ResultsPane.tsx
export function DataGrid({
  columns,
  rowCount,
  getRow,
  rowHeight,
  onRangeChange,
  onCellSelect,
  selected = null,
  sort = [],
  onSortToggle,
  selectedRows,
  onToggleRow,
  onToggleAll,
  onCellEdit,
  onCellsEdit,
  onPasteNotice,
  staged,
  deletedRows,
  insertedFrom,
  nullDisplay = "NULL",
  onLinkOpen,
  onFilter,
  onSortSet,
  onClearSort,
  onCopied,
  alternatingRows = true,
  onInsertRow,
  onDuplicateRow,
  onDeleteRow,
  onInspect,
  onCopyRows,
  onLookup,
}: DataGridProps) {
  const parentRef = useRef<HTMLDivElement | null>(null);
  const rangeRef = useRef(onRangeChange);
  useEffect(() => {
    rangeRef.current = onRangeChange;
  });
  const [editing, setEditing] = useState<{ row: number; col: number } | null>(null);
  // Dragged widths by column name, in a ref rather than state: a drag must not
  // wait for a state round-trip (setState -> memo -> measure()) to be visible,
  // and at one event per pixel that round-trip is what made the handle look
  // dead. The virtualizer owns the rendered width via resizeItem(); this map is
  // only what a re-measure (new page, reset) reads back.
  const widthsRef = useRef<Record<string, number>>({});
  // resizeItem() notifies the virtualizer, which re-renders through its own
  // subscription; this tick is the belt to that braces, so a width change can
  // never be swallowed by a batched update mid-drag.
  const [, bumpWidths] = useReducer((n: number) => n + 1, 0);
  // Unclamped width of the column being dragged, so dragging past the minimum
  // and back does not leave the handle lagging behind the pointer.
  const dragWidth = useRef<number | null>(null);

  const rowVirtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 12,
    onChange: (instance) => {
      const range = instance.range;
      if (range && rangeRef.current) rangeRef.current(range.startIndex, range.endIndex);
    },
  });

  const colVirtualizer = useVirtualizer({
    horizontal: true,
    count: columns.length,
    getScrollElement: () => parentRef.current,
    estimateSize: (index) => {
      const column = columns[index];
      return column ? (widthsRef.current[column.name] ?? estimateWidth(column)) : 160;
    },
    overscan: 4,
  });

  useEffect(() => {
    rowVirtualizer.measure();
  }, [rowHeight, rowVirtualizer]);
  // A new page or a different table re-reads the widths map, so a column keeps
  // the width it was dragged to and a new column starts from its estimate.
  useEffect(() => {
    colVirtualizer.measure();
  }, [columns, colVirtualizer]);

  // WHAT:  Sizes every column the user has not dragged to the widest value on
  //        the loaded page, once per page.
  // WHY:   "Fit the content" is what a grid is expected to do on open; the cap
  //        keeps one 4 KB cell from owning the viewport.
  const fitted = useRef<readonly GridColumn[] | null>(null);
  useEffect(() => {
    if (rowCount === 0 || fitted.current === columns) return;
    fitted.current = columns;
    const sample = Math.min(rowCount, 60);
    columns.forEach((column, index) => {
      if (column.name in widthsRef.current) return;
      const texts: string[] = [];
      for (let row = 0; row < sample; row += 1) {
        const value = getRow(row)?.[index];
        if (value !== undefined) texts.push(formatCell(value).text);
      }
      if (texts.length === 0) return;
      const width = fitWidth(column, texts);
      widthsRef.current[column.name] = width;
      colVirtualizer.resizeItem(index, width);
    });
  }, [columns, rowCount, getRow, colVirtualizer]);

  const fitColumn = (column: GridColumn, index: number) => {
    const texts: string[] = [];
    for (const item of rowVirtualizer.getVirtualItems()) {
      const value = getRow(item.index)?.[index];
      if (value !== undefined) texts.push(formatCell(value).text);
    }
    const width = fitWidth(column, texts);
    widthsRef.current[column.name] = width;
    colVirtualizer.resizeItem(index, width);
  };

  const resizeColumn = (column: GridColumn, index: number, current: number, delta: number) => {
    const raw = (dragWidth.current ?? current) + delta;
    dragWidth.current = raw;
    const next = Math.min(MAX_COL_WIDTH, Math.max(MIN_COL_WIDTH, raw));
    if (widthsRef.current[column.name] === next) return;
    widthsRef.current[column.name] = next;
    // Writes the measurement and notifies: the header and every cell of that
    // column reposition on this frame.
    colVirtualizer.resizeItem(index, next);
    bumpWidths();
  };
  const resetColumn = (column: GridColumn, index: number) => {
    if (!(column.name in widthsRef.current)) return;
    widthsRef.current = Object.fromEntries(Object.entries(widthsRef.current).filter(([name]) => name !== column.name));
    colVirtualizer.resizeItem(index, estimateWidth(column));
    bumpWidths();
  };

  const menu = useContextMenu();

  const copy = (text: string, what: string) => {
    void navigator.clipboard.writeText(text);
    onCopied?.(what);
  };

  // WHAT:  Rectangular cell range in grid coordinates, driven by press-drag.
  // HOW:   mousedown sets the anchor, mouseenter while the button is held
  //        moves the focus, a window mouseup ends the drag (the pointer may be
  //        released outside the grid). A different column set drops the range.
  const [range, setRange] = useState<CellRange | null>(null);
  const dragging = useRef(false);
  const columnKey = columns.map((c) => c.name).join("\u0000");
  useEffect(() => {
    setRange(null);
  }, [columnKey]);
  useEffect(() => {
    const up = () => {
      dragging.current = false;
    };
    window.addEventListener("mouseup", up);
    return () => window.removeEventListener("mouseup", up);
  }, []);

  const canPaste = onCellsEdit !== undefined || onCellEdit !== undefined;
  const rangeBounds: RangeBounds | null = range ? bounds(range) : null;
  // Staged values win, as they do on screen: the stats describe what is shown.
  const stats = useMemo(
    () => (range && cellCount(bounds(range)) > 1 ? rangeStats(bounds(range), (r, c) => staged?.get(`${r}:${c}`)?.value ?? getRow(r)?.[c]) : null),
    [range, staged, getRow],
  );

  const startRange = (event: ReactMouseEvent, pos: CellPos) => {
    if (event.button !== 0 || editing !== null) return;
    parentRef.current?.focus({ preventScroll: true });
    setRange((r) => (event.shiftKey && r ? { anchor: r.anchor, focus: pos } : { anchor: pos, focus: pos }));
    dragging.current = true;
  };
  const extendRange = (pos: CellPos) => {
    if (!dragging.current) return;
    setRange((r) => (r && (r.focus.row !== pos.row || r.focus.col !== pos.col) ? { anchor: r.anchor, focus: pos } : r));
  };

  /// Scrolls the grid while a drag holds the pointer near an edge, so a range
  /// can grow past what is on screen.
  const autoScroll = (event: ReactMouseEvent) => {
    const node = parentRef.current;
    if (!dragging.current || !node) return;
    const box = node.getBoundingClientRect();
    const edge = 28;
    const dy = event.clientY > box.bottom - edge ? 18 : event.clientY < box.top + HEADER_HEIGHT + edge ? -18 : 0;
    const dx = event.clientX > box.right - edge ? 24 : event.clientX < box.left + gutter + edge ? -24 : 0;
    if (dx !== 0 || dy !== 0) node.scrollBy(dx, dy);
  };

  const rangeText = (b: RangeBounds, withHeaders: boolean): string => {
    const lines: string[][] = withHeaders ? [columns.slice(b.left, b.right + 1).map((c) => c.name)] : [];
    for (let r = b.top; r <= b.bottom; r += 1) {
      const row = getRow(r);
      const line: string[] = [];
      for (let c = b.left; c <= b.right; c += 1) line.push(clipboardText(staged?.get(`${r}:${c}`)?.value ?? row?.[c]));
      lines.push(line);
    }
    return toTsv(lines);
  };
  const rangeLabel = (b: RangeBounds) => (cellCount(b) === 1 ? "Value" : `${b.bottom - b.top + 1} × ${b.right - b.left + 1} cells`);

  // WHAT:  Writes clipboard TSV into the grid from the range's top-left cell.
  // HOW:   A single value fills the whole range. Cells that cannot take a
  //        value (past the last row/column, staged deletes, byte columns, rows
  //        not loaded yet) are skipped; text is parsed per column type exactly
  //        as the inline editor does.
  const pasteText = (text: string, target: RangeBounds | null = rangeBounds) => {
    if (!target || !canPaste) return;
    const grid = parseTsv(text);
    const first = grid[0];
    if (first === undefined) return;
    const single = grid.length === 1 && first.length === 1;
    const height = single ? target.bottom - target.top + 1 : grid.length;
    const width = single ? target.right - target.left + 1 : Math.max(...grid.map((r) => r.length));
    const edits: CellEdit[] = [];
    let skipped = 0;
    for (let r = 0; r < height; r += 1) {
      for (let c = 0; c < width; c += 1) {
        const row = target.top + r;
        const col = target.left + c;
        const cellText = single ? first[0] : grid[r]?.[c];
        const column = columns[col];
        if (cellText === undefined || column === undefined || row >= rowCount) continue;
        const current = staged?.get(`${row}:${col}`)?.value ?? getRow(row)?.[col];
        if (current === undefined || deletedRows?.has(row) || fieldKind(column.typeName, current) === "bytes") {
          skipped += 1;
          continue;
        }
        edits.push({ row, col, value: parseEdited(cellText, column.typeName, current) });
      }
    }
    if (edits.length === 0) {
      onPasteNotice?.("Nothing to paste into here.");
      return;
    }
    if (onCellsEdit) onCellsEdit(edits);
    else for (const e of edits) onCellEdit?.(e.row, e.col, e.value);
    setRange({
      anchor: { row: target.top, col: target.left },
      focus: { row: Math.min(rowCount - 1, target.top + height - 1), col: Math.min(columns.length - 1, target.left + width - 1) },
    });
    onPasteNotice?.(`Pasted ${edits.length} cell(s)${skipped > 0 ? `, skipped ${skipped}` : ""}.`);
  };

  /// Keyboard and clipboard events only belong to the grid when they come from
  /// the grid itself: not from the inline editor, and not from a portalled
  /// menu or dialog whose React events still bubble up through here.
  const ownsEvent = (event: { target: EventTarget; currentTarget: HTMLElement }) => {
    const t = event.target;
    if (!(t instanceof HTMLElement) || !event.currentTarget.contains(t)) return false;
    return editing === null && !t.isContentEditable && t.tagName !== "INPUT" && t.tagName !== "TEXTAREA";
  };

  const onGridCopy = (event: ReactClipboardEvent<HTMLDivElement>) => {
    if (!rangeBounds || !ownsEvent(event)) return;
    event.preventDefault();
    event.clipboardData.setData("text/plain", rangeText(rangeBounds, false));
    onCopied?.(rangeLabel(rangeBounds));
  };
  const onGridPaste = (event: ReactClipboardEvent<HTMLDivElement>) => {
    if (!rangeBounds || !canPaste || !ownsEvent(event)) return;
    const text = event.clipboardData.getData("text/plain");
    if (text.length === 0) return;
    event.preventDefault();
    pasteText(text);
  };

  const onGridKey = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!ownsEvent(event) || rowCount === 0 || columns.length === 0) return;
    const mod = event.ctrlKey || event.metaKey;
    if (mod && event.key.toLowerCase() === "a") {
      event.preventDefault();
      setRange({ anchor: { row: 0, col: 0 }, focus: { row: rowCount - 1, col: columns.length - 1 } });
      return;
    }
    if (event.key === "Escape" && range) {
      setRange(null);
      return;
    }
    const step: Partial<Record<string, CellPos>> = { ArrowUp: { row: -1, col: 0 }, ArrowDown: { row: 1, col: 0 }, ArrowLeft: { row: 0, col: -1 }, ArrowRight: { row: 0, col: 1 } };
    const move = step[event.key];
    if (!move || !range) return;
    event.preventDefault();
    const focus = {
      row: Math.min(rowCount - 1, Math.max(0, range.focus.row + move.row)),
      col: Math.min(columns.length - 1, Math.max(0, range.focus.col + move.col)),
    };
    setRange(event.shiftKey ? { anchor: range.anchor, focus } : { anchor: focus, focus });
    if (!event.shiftKey) onCellSelect?.(focus.row, focus.col);
    rowVirtualizer.scrollToIndex(focus.row, { align: "auto" });
    colVirtualizer.scrollToIndex(focus.col, { align: "auto" });
  };

  /// Excel-style outline: each range cell draws only the edges on the range border.
  const rangeEdge = (b: RangeBounds, row: number, col: number): CSSProperties => {
    const line = "var(--color-accent)";
    const edges = [
      row === b.top ? `inset 0 1px 0 ${line}` : null,
      row === b.bottom ? `inset 0 -1px 0 ${line}` : null,
      col === b.left ? `inset 1px 0 0 ${line}` : null,
      col === b.right ? `inset -1px 0 0 ${line}` : null,
    ].filter((e): e is string => e !== null);
    return edges.length > 0 ? { boxShadow: edges.join(", ") } : {};
  };

  /// Filter/sort entries are the same for a header and a cell; the cell adds the
  /// value-bound ones because it knows which value was clicked.
  const headerEntries = (column: GridColumn): MenuEntry[] => [
    ...(onSortToggle || onSortSet
      ? ([
          { id: "sort-asc", label: "Sort ascending", icon: "arrow-up" },
          { id: "sort-desc", label: "Sort descending", icon: "arrow-down" },
          { id: "sort-clear", label: "Clear sort", icon: "x", disabled: sort.length === 0 },
        ] satisfies MenuEntry[])
      : []),
    ...(onFilter
      ? ([
          { id: "filter-null", label: `${column.name} is NULL`, icon: "filter", group: "filter" },
          { id: "filter-not-null", label: `${column.name} is not NULL`, icon: "filter", group: "filter" },
        ] satisfies MenuEntry[])
      : []),
    { id: "select-column", label: "Select column", icon: "columns", group: "copy", disabled: rowCount === 0 },
    { id: "copy-column", label: "Copy column name", icon: "copy", group: "copy" },
    { id: "fit-width", label: "Fit column to content", icon: "columns", group: "copy" },
    { id: "reset-width", label: "Reset column width", icon: "columns", group: "copy" },
  ];

  const onHeaderMenu = (event: ReactMouseEvent, column: GridColumn, index: number) =>
    menu.open(event, headerEntries(column), (id) => {
      if (id === "sort-asc") (onSortSet ?? (() => onSortToggle?.(column.name)))({ column: column.name, desc: false });
      else if (id === "sort-desc") (onSortSet ?? (() => onSortToggle?.(column.name)))({ column: column.name, desc: true });
      else if (id === "sort-clear") onClearSort?.();
      else if (id === "filter-null") onFilter?.({ column: column.name, op: "is_null", value: "" });
      else if (id === "filter-not-null") onFilter?.({ column: column.name, op: "is_not_null", value: "" });
      else if (id === "select-column") {
        setRange({ anchor: { row: 0, col: index }, focus: { row: rowCount - 1, col: index } });
        parentRef.current?.focus({ preventScroll: true });
      } else if (id === "copy-column") copy(column.name, "Column name");
      else if (id === "fit-width") fitColumn(column, index);
      else if (id === "reset-width") resetColumn(column, index);
    });

  const onCellMenu = (event: ReactMouseEvent, column: GridColumn | undefined, rowIndex: number, colIndex: number, value: Value | undefined) => {
    if (!column) return;
    const text = value === undefined ? "" : formatCell(value).text;
    const isNull = value?.t === "null";
    const short = text.length > 24 ? `${text.slice(0, 24)}…` : text;
    const filterable = onFilter !== undefined && !isNull;
    // Right-clicking inside a multi-cell range keeps it (the menu acts on it);
    // anywhere else the clicked cell becomes the range.
    const keep = rangeBounds !== null && cellCount(rangeBounds) > 1 && inBounds(rangeBounds, rowIndex, colIndex);
    const target: RangeBounds = keep ? rangeBounds : { top: rowIndex, bottom: rowIndex, left: colIndex, right: colIndex };
    if (!keep) setRange({ anchor: { row: rowIndex, col: colIndex }, focus: { row: rowIndex, col: colIndex } });
    const entries: MenuEntry[] = [
      ...(keep
        ? ([
            { id: "copy-range", label: `Copy ${rangeLabel(target)}  (Ctrl+C)`, icon: "copy" },
            { id: "copy-range-headers", label: "Copy selection with headers", icon: "copy" },
          ] satisfies MenuEntry[])
        : []),
      ...(canPaste ? ([{ id: "paste", label: "Paste  (Ctrl+V)", icon: "clipboard" }] satisfies MenuEntry[]) : []),
      { id: "copy-value", label: "Copy value", icon: "copy" },
      { id: "copy-row-json", label: "Copy row as JSON", icon: "braces" },
      { id: "copy-row-csv", label: "Copy row as CSV", icon: "rows" },
      ...(filterable
        ? ([
            { id: "filter-eq", label: `Filter: ${column.name} = ${short}`, icon: "filter", group: "filter" },
            { id: "filter-ne", label: `Filter: ${column.name} ≠ ${short}`, icon: "filter", group: "filter" },
            { id: "filter-contains", label: `Filter: ${column.name} contains ${short}`, icon: "search", group: "filter" },
          ] satisfies MenuEntry[])
        : []),
      ...(onFilter && isNull ? ([{ id: "filter-null", label: `Filter: ${column.name} is NULL`, icon: "filter", group: "filter" }] satisfies MenuEntry[]) : []),
      ...(onSortSet || onSortToggle
        ? ([
            { id: "sort-asc", label: `Sort by ${column.name} ascending`, icon: "arrow-up", group: "sort" },
            { id: "sort-desc", label: `Sort by ${column.name} descending`, icon: "arrow-down", group: "sort" },
          ] satisfies MenuEntry[])
        : []),
      ...(onCellEdit !== undefined && value !== undefined
        ? ([
            { id: "edit", label: "Edit cell", icon: "pencil", group: "edit" },
            { id: "set-null", label: "Set NULL", icon: "minus", group: "edit", disabled: isNull },
          ] satisfies MenuEntry[])
        : []),
      ...(column.linkTo !== undefined && onLinkOpen !== undefined
        ? ([{ id: "open-link", label: `Open ${column.linkTo} rows`, icon: "link", group: "link" }] satisfies MenuEntry[])
        : []),
      ...(onInspect ? ([{ id: "inspect", label: "Open inspector", icon: "expand", group: "row" }] satisfies MenuEntry[]) : []),
      ...(onCopyRows
        ? ([
            { id: "rows-json", label: "Copy rows as JSON", icon: "braces", group: "rows" },
            { id: "rows-csv", label: "Copy rows as CSV", icon: "rows", group: "rows" },
            { id: "rows-sql", label: "Copy rows as SQL INSERT", icon: "terminal", group: "rows" },
          ] satisfies MenuEntry[])
        : []),
      ...(onInsertRow ? ([{ id: "insert-row", label: "Insert row", icon: "plus", group: "write" }] satisfies MenuEntry[]) : []),
      ...(onDuplicateRow ? ([{ id: "duplicate-row", label: "Duplicate row", icon: "copy", group: "write" }] satisfies MenuEntry[]) : []),
      ...(onDeleteRow ? ([{ id: "delete-row", label: "Delete row", icon: "trash", danger: true, group: "write" }] satisfies MenuEntry[]) : []),
    ];

    onCellSelect?.(rowIndex, colIndex);
    menu.open(event, entries, (id) => {
      const row = getRow(rowIndex);
      if (id === "copy-range") copy(rangeText(target, false), rangeLabel(target));
      else if (id === "copy-range-headers") copy(rangeText(target, true), rangeLabel(target));
      else if (id === "paste") {
        // The menu path has no paste event to read from, so it asks the
        // Clipboard API; the webview may refuse, and Ctrl+V always works.
        navigator.clipboard.readText().then((t) => { pasteText(t, target); }, () => onPasteNotice?.("Clipboard could not be read here. Use Ctrl+V."));
      } else if (id === "copy-value") copy(text, "Value");
      else if (id === "copy-row-json") copy(JSON.stringify(Object.fromEntries(columns.map((c, i) => [c.name, row?.[i] === undefined ? null : formatCell(row[i]).text])), null, 2), "Row");
      else if (id === "copy-row-csv") copy(columns.map((_, i) => csvCell(row?.[i])).join(","), "Row");
      else if (id === "filter-eq") onFilter?.({ column: column.name, op: "eq", value: text });
      else if (id === "filter-ne") onFilter?.({ column: column.name, op: "ne", value: text });
      else if (id === "filter-contains") onFilter?.({ column: column.name, op: "contains", value: text });
      else if (id === "filter-null") onFilter?.({ column: column.name, op: "is_null", value: "" });
      else if (id === "sort-asc") (onSortSet ?? (() => onSortToggle?.(column.name)))({ column: column.name, desc: false });
      else if (id === "sort-desc") (onSortSet ?? (() => onSortToggle?.(column.name)))({ column: column.name, desc: true });
      else if (id === "edit") setEditing({ row: rowIndex, col: colIndex });
      else if (id === "set-null") onCellEdit?.(rowIndex, colIndex, { t: "null" });
      else if (id === "open-link") onLinkOpen?.(rowIndex, colIndex);
      else if (id === "inspect") onInspect?.(rowIndex, colIndex);
      else if (id === "rows-json") onCopyRows?.(rowIndex, "json");
      else if (id === "rows-csv") onCopyRows?.(rowIndex, "csv");
      else if (id === "rows-sql") onCopyRows?.(rowIndex, "sql");
      else if (id === "insert-row") onInsertRow?.();
      else if (id === "duplicate-row") onDuplicateRow?.(rowIndex);
      else if (id === "delete-row") onDeleteRow?.(rowIndex);
    });
  };

  const totalWidth = colVirtualizer.getTotalSize();
  const totalHeight = rowVirtualizer.getTotalSize();
  const virtualRows = rowVirtualizer.getVirtualItems();
  const virtualCols = colVirtualizer.getVirtualItems();
  const selectable = selectedRows !== undefined && onToggleRow !== undefined;
  const gutter = selectable ? CHECK_WIDTH : 0;
  const allSelected = selectable && rowCount > 0 && selectedRows.size === rowCount;
  const someSelected = selectable && selectedRows.size > 0 && !allSelected;

  return (
    <div className="flex h-full w-full min-h-0 flex-col">
    <ScrollArea
      ref={parentRef}
      orientation="horizontal"
      tabIndex={0}
      aria-multiselectable="true"
      onKeyDown={onGridKey}
      onCopy={onGridCopy}
      onPaste={onGridPaste}
      onMouseMove={autoScroll}
      className="min-h-0 w-full flex-1 overflow-y-auto bg-background/60 font-mono text-[12px] select-none outline-none"
    >
      <div style={{ width: totalWidth + gutter, height: totalHeight + HEADER_HEIGHT, position: "relative" }}>
        <div className="sticky top-0 z-20 flex border-b border-border/50 glass-header" style={{ height: HEADER_HEIGHT, width: totalWidth + gutter }}>
          {selectable ? (
            <div className="sticky left-0 z-30 flex shrink-0 items-center justify-center border-r border-border/50 glass-header" style={{ width: CHECK_WIDTH }}>
              <Check label="Select all rows" checked={allSelected} indeterminate={someSelected} onChange={onToggleAll} />
            </div>
          ) : null}
          <div className="relative" style={{ width: totalWidth, height: HEADER_HEIGHT }}>
            {virtualCols.map((vc) => {
              const column = columns[vc.index];
              if (!column) return null;
              const rule = sort.find((s) => s.column === column.name);
              return (
                <Button
                  variant="ghost"
                  key={vc.key}
                  disabled={onSortToggle === undefined}
                  onClick={() => onSortToggle?.(column.name)}
                  onContextMenu={(e) => onHeaderMenu(e, column, vc.index)}
                  className={cn("absolute top-0 flex h-full items-center justify-start gap-1.5 truncate border-r border-border/40 px-2.5 text-left font-sans liquid-hover rounded-none", onSortToggle ? "hover:bg-surface-secondary/70" : "cursor-default")}
                  style={{ left: vc.start, width: vc.size }}
                >
                  <Icon name={column.linkTo !== undefined && !column.primaryKey ? "link" : typeIcon(column.typeName, column.primaryKey)} size={12} className={cn("shrink-0", column.primaryKey ? "text-warning" : column.linkTo !== undefined ? "text-accent" : "text-muted")} />
                  <span className="truncate text-[12px] font-medium text-foreground">{column.name}</span>
                  {column.linkTo !== undefined ? <span className="truncate text-[10px] text-muted">→ {column.linkTo}</span> : null}
                  {rule ? <Icon name={rule.desc ? "arrow-down" : "arrow-up"} size={11} className="ml-auto shrink-0 text-accent" /> : null}
                </Button>
              );
            })}
            {virtualCols.map((vc) => {
              const column = columns[vc.index];
              if (!column) return null;
              return (
                <div
                  key={`${vc.key}-resize`}
                  className="absolute top-0 z-30 h-full cursor-col-resize"
                  style={{ left: vc.end - HANDLE_WIDTH / 2, width: HANDLE_WIDTH }}
                  title="Drag to resize · double-click to fit the content"
                  onDoubleClick={() => fitColumn(column, vc.index)}
                >
                  <Resizer
                    direction="horizontal"
                    onResize={(delta) => resizeColumn(column, vc.index, vc.size, delta)}
                    onDragEnd={() => {
                      dragWidth.current = null;
                    }}
                    className="absolute inset-0 h-full w-full rounded-none"
                  />
                </div>
              );
            })}
          </div>
        </div>

        {virtualRows.map((vr) => {
          const row = getRow(vr.index);
          const isSelectedRow = selected?.row === vr.index;
          const isChecked = selectedRows?.has(vr.index) ?? false;
          const isDeleted = deletedRows?.has(vr.index) ?? false;
          const isInserted = insertedFrom !== undefined && vr.index >= insertedFrom;
          return (
            <div
              key={vr.key}
              className={cn(
                "absolute left-0 flex border-b border-separator/40 transition-colors duration-100",
                isChecked
                  ? "bg-accent/15"
                  : isDeleted
                    ? "bg-danger-soft/50 line-through decoration-danger/60"
                    : isInserted
                      ? "bg-success-soft/40"
                      : isSelectedRow
                        ? "bg-surface-secondary/80"
                        : alternatingRows && vr.index % 2 === 1
                          ? "row-stripe hover:bg-surface-secondary/40"
                          : "hover:bg-surface-secondary/40",
              )}
              style={{ top: vr.start + HEADER_HEIGHT, height: vr.size, width: totalWidth + gutter }}
              title={isDeleted ? "Staged for deletion" : isInserted ? "Staged insert" : undefined}
            >
              {selectable ? (
                <div className={cn("sticky left-0 z-10 flex shrink-0 items-center justify-center border-r border-separator/40 backdrop-blur-sm", isDeleted ? "bg-danger-soft/70" : isInserted ? "bg-success-soft/70" : "bg-surface/70")} style={{ width: CHECK_WIDTH }}>
                  <Check label={`Select row ${vr.index + 1}`} checked={isChecked} onChange={() => onToggleRow(vr.index)} />
                </div>
              ) : null}
              <div className="relative" style={{ width: totalWidth }}>
                {row === undefined ? (
                  <div className="absolute inset-y-0 left-2 flex items-center gap-2">
                    <span className="h-2 w-24 animate-pulse rounded-sm bg-surface-tertiary" />
                    <span className="h-2 w-40 animate-pulse rounded-sm bg-surface-tertiary" />
                  </div>
                ) : (
                  virtualCols.map((vc) => {
                    const column = columns[vc.index];
                    const stagedCell = staged?.get(`${vr.index}:${vc.index}`);
                    const value = stagedCell?.value ?? row[vc.index];
                    const formatted = value === undefined ? null : formatCell(value);
                    const isSelected = isSelectedRow && selected.col === vc.index;
                    const isEditing = editing !== null && editing.row === vr.index && editing.col === vc.index;
                    const kind = fieldKind(column?.typeName ?? "", value);
                    const editable = onCellEdit !== undefined && !isDeleted && value !== undefined && kind !== "bytes" && kind !== "bool";
                    const linked = column?.linkTo !== undefined && onLinkOpen !== undefined && value !== undefined && value.t !== "null";
                    // One colour per cell: a change state wins over the value-kind syntax colour.
                    const tone = stagedCell !== undefined ? "font-medium text-warning" : isDeleted ? "text-danger" : isInserted ? "text-success" : formatted ? cellClass(formatted.kind) : "";
                    const inRange = rangeBounds !== null && inBounds(rangeBounds, vr.index, vc.index);
                    const pos = { row: vr.index, col: vc.index };
                    return (
                      <div
                        key={vc.key}
                        onMouseDown={(e) => startRange(e, pos)}
                        onMouseEnter={() => extendRange(pos)}
                        onClick={() => onCellSelect?.(vr.index, vc.index)}
                        onDoubleClick={() => {
                          if (editable) setEditing({ row: vr.index, col: vc.index });
                        }}
                        onContextMenu={(e) => onCellMenu(e, column, vr.index, vc.index, value)}
                        className={cn(
                          "absolute top-0 flex h-full cursor-default items-center truncate border-r border-separator/40 px-2",
                          tone,
                          stagedCell !== undefined ? "border-l-2 border-l-warning bg-warning-soft" : "",
                          isSelected ? "ring-1 ring-accent ring-inset" : "",
                          isSelected && stagedCell === undefined ? "bg-accent/20" : "",
                          inRange && !isSelected && stagedCell === undefined ? "bg-accent/15" : "",
                          isEditing ? "select-text" : "",
                        )}
                        style={{ left: vc.start, width: vc.size, ...(inRange ? rangeEdge(rangeBounds, vr.index, vc.index) : {}) }}
                        title={stagedCell !== undefined ? `${formatted?.text ?? ""}\nwas: ${formatCell(stagedCell.old).text}` : formatted?.text}
                      >
                        {isEditing && value !== undefined && onCellEdit ? (
                          <CellEditor
                            {...(onLookup && column?.linkTo !== undefined ? { lookup: (search: string) => onLookup(vc.index, search) } : {})}
                            typeName={column?.typeName ?? ""}
                            value={value}
                            onCommit={(next) => {
                              setEditing(null);
                              onCellEdit(vr.index, vc.index, next);
                            }}
                            onCancel={() => setEditing(null)}
                          />
                        ) : value?.t === "bool" ? (
                          // Interactive when the grid is editable: toggling stages / applies the edit.
                          <Check
                            label={value.v ? "true" : "false"}
                            checked={value.v}
                            {...(onCellEdit && !isDeleted ? { onChange: (next: boolean) => onCellEdit(vr.index, vc.index, { t: "bool", v: next }) } : {})}
                          />
                        ) : value?.t === "null" ? (
                          <span className={cn("truncate", stagedCell === undefined && !isDeleted && !isInserted ? "" : "italic")}>{nullDisplay}</span>
                        ) : (
                          <span className="truncate">{formatted?.text ?? ""}</span>
                        )}
                        {stagedCell !== undefined && !isEditing ? <span aria-hidden="true" className="ml-auto size-1.5 shrink-0 rounded-full bg-warning" /> : null}
                        {linked && !isEditing ? (
                          <Button
                            variant="ghost"
                            size="sm"
                            aria-label={`Open ${column.linkTo ?? "related"} rows`}
                            onClick={() => {
                              onLinkOpen(vr.index, vc.index);
                            }}
                            className="ml-auto flex size-4.5 min-w-4.5 p-0 shrink-0 rounded-sm text-accent opacity-60 hover:bg-accent-soft hover:opacity-100"
                          >
                            <Icon name="link" size={11} />
                          </Button>
                        ) : null}
                      </div>
                    );
                  })
                )}
              </div>
            </div>
          );
        })}
      </div>
      {menu.node}
    </ScrollArea>
    {stats ? (
      // WHAT:  Spreadsheet-style status strip for a multi-cell selection;
      //        clicking an aggregate copies the raw number.
      <div className="flex h-6 shrink-0 items-center gap-3 border-t border-border/40 bg-surface px-2.5 font-mono text-[11px] text-muted tabular-nums" aria-live="polite">
        <span>Cells <span className="text-foreground">{formatStat(stats.cells)}</span></span>
        <span>Count <span className="text-foreground">{formatStat(stats.filled)}</span></span>
        {stats.numeric > 0 ? (
          <>
            <StatButton label="Sum" value={stats.sum} onCopy={copy} />
            <StatButton label="Avg" value={stats.sum / stats.numeric} onCopy={copy} />
            {stats.min !== null ? <StatButton label="Min" value={stats.min} onCopy={copy} /> : null}
            {stats.max !== null ? <StatButton label="Max" value={stats.max} onCopy={copy} /> : null}
          </>
        ) : null}
        {stats.partial ? <span className="text-warning">first {formatStat(STATS_CELL_LIMIT)} cells</span> : null}
      </div>
    ) : null}
    </div>
  );
}

function StatButton({ label, value, onCopy }: { label: string; value: number; onCopy: (text: string, what: string) => void }) {
  return (
    <Button variant="ghost" size="xs" className="h-5 gap-1 rounded px-1 font-mono text-[11px] font-normal text-muted hover:text-foreground" title={`Copy ${label.toLowerCase()}`} onClick={() => onCopy(String(value), label)}>
      {label} <span className="text-foreground">{formatStat(value)}</span>
    </Button>
  );
}

// WHAT:  Width for a column nobody has dragged: the type's usual shape, never
//        past MAX_FIT_WIDTH.
// WHY:   A `text` column holding a 4 KB token would otherwise push every other
//        column off-screen; 450px shows enough to recognise the value, and the
//        inspector shows the rest.
function estimateWidth(column: GridColumn): number {
  const t = column.typeName.toLowerCase();
  const header = column.name.length * CHAR_WIDTH + FIT_GAP;
  if (t.includes("bool")) return clampFit(Math.max(120, header));
  if (/int|serial|numeric|decimal|real|float|double/.test(t)) return clampFit(Math.max(110, header));
  if (/timestamp|date|time/.test(t)) return clampFit(Math.max(220, header));
  if (t.includes("uuid")) return clampFit(Math.max(300, header));
  if (/json|text|blob|bytea/.test(t)) return clampFit(Math.max(260, header));
  return clampFit(Math.max(140, header));
}

// WHAT:  Width that fits the widest value currently loaded, plus the gap.
// HOW:   Measured from the rows the grid can see rather than the whole table:
//        the page is what the user is looking at, and it costs nothing.
function fitWidth(column: GridColumn, values: readonly string[]): number {
  const longest = values.reduce((max, text) => Math.max(max, text.length), column.name.length);
  return clampFit(longest * CHAR_WIDTH + FIT_GAP);
}

function clampFit(width: number): number {
  return Math.min(MAX_FIT_WIDTH, Math.max(MIN_COL_WIDTH, Math.round(width)));
}

/// One CSV field: quoted when it holds a comma, quote or newline.
function csvCell(value: Value | undefined): string {
  const text = value === undefined ? "" : formatCell(value).text;
  return /[",\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}
