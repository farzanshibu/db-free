// SOT: dashboard-grid-layout, tile-placement, tile-swap, tile-resize, draw-to-create, legacy-layout-migration
// WHAT:  The geometry of a dashboard grid, with no DOM in it: 12 columns of 60 px
//        rows, tiles that never overlap, moving one onto another swaps them,
//        resizing stops where the next tile starts, a drawn rectangle becomes a
//        tile, and an older flow layout is converted into positions.
// WHY:   Every rule the editor enforces (stay in bounds, find a free slot, swap
//        on drop) is arithmetic on rectangles. Kept here it is the same code for
//        a drag preview, a drop, a duplicate and a load, and it can be tested
//        without a browser.
// HOW:   A tile is `{id, x, y, w, h}` in grid cells. Functions take the whole
//        list and return a new one; a tile that did not change keeps its object.
//        Rows are unbounded downwards, columns are `cols` (12).
// WHERE: ./DashboardGrid.tsx (pointer handling), ./DashboardTab.tsx (load, duplicate)

export const GRID_COLS = 12;
export const ROW_HEIGHT = 60;
export const GRID_GAP = 8;
/// `DashboardBody.layoutVersion` of positions written in this grid.
export const LAYOUT_VERSION = 1;
export const MIN_W = 2;
export const MIN_H = 2;
/// Below this container width tiles stack in one column and the grid is read-only.
export const NARROW_WIDTH = 560;
/// A drag that moves less than this many pixels is a click.
export const DRAG_THRESHOLD = 6;

export interface Cell {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Placed extends Cell {
  id: string;
}

export function overlaps(a: Cell, b: Cell): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

function whole(value: number, fallback: number): number {
  return Number.isFinite(value) ? Math.floor(value) : fallback;
}

function clamp(value: number, low: number, high: number): number {
  return Math.max(low, Math.min(high, value));
}

/// A cell made whole and kept inside the columns.
export function clampCell(cell: Cell, cols: number = GRID_COLS): Cell {
  const w = clamp(whole(cell.w, 1), 1, cols);
  const h = Math.max(1, whole(cell.h, 1));
  return { x: clamp(whole(cell.x, 0), 0, cols - w), y: Math.max(0, whole(cell.y, 0)), w, h };
}

function bottomOf(cells: readonly Cell[]): number {
  return cells.reduce((max, c) => Math.max(max, c.y + c.h), 0);
}

/// Rows the grid needs to show every tile.
export function rowsUsed(cells: readonly Cell[]): number {
  return bottomOf(cells);
}

// WHAT:  The nearest free spot for a `w` by `h` tile.
// HOW:   The preferred spots first (usually "just below" and "just to the right"
//        of where the tile wanted to be), then a row-major scan from `fromY`,
//        then a new row at the bottom.
export function findFreeSlot(obstacles: readonly Cell[], w: number, h: number, preferred: readonly { x: number; y: number }[], fromY: number, cols: number = GRID_COLS): { x: number; y: number } {
  const fits = (x: number, y: number) => x >= 0 && y >= 0 && x + w <= cols && !obstacles.some((o) => overlaps({ x, y, w, h }, o));
  for (const spot of preferred) if (fits(spot.x, spot.y)) return spot;
  const bottom = bottomOf(obstacles);
  for (let y = Math.max(0, fromY); y < bottom; y += 1) {
    for (let x = 0; x + w <= cols; x += 1) if (fits(x, y)) return { x, y };
  }
  return { x: 0, y: bottom };
}

// WHAT:  The list made valid: every tile whole and inside the columns, and no
//        two overlapping. A tile that overlaps an earlier one moves to a free spot.
// WHY:   Saved positions can be anything (hand-edited, from an older build, from
//        the agent); the editor must never draw two tiles on top of each other.
export function normalizeLayout<T extends Cell>(items: readonly T[], cols: number = GRID_COLS): T[] {
  const placed: T[] = [];
  items.forEach((item, index) => {
    const fitted = clampCell(item, cols);
    let next: T = fitted.x === item.x && fitted.y === item.y && fitted.w === item.w && fitted.h === item.h ? item : { ...item, ...fitted };
    if (placed.some((p) => overlaps(p, next))) {
      const obstacles: Cell[] = [...placed, ...items.slice(index + 1).map((o) => clampCell(o, cols))];
      const slot = findFreeSlot(obstacles, next.w, next.h, [{ x: next.x, y: next.y + next.h }, { x: next.x + next.w, y: next.y }], next.y, cols);
      next = { ...next, x: slot.x, y: slot.y };
    }
    placed.push(next);
  });
  return placed;
}

// WHAT:  The first grid's flow layout turned into positions.
// WHY:   Dashboards saved before positions mattered stored a width (2-12 of 12
//        columns) and a height in 120 px rows, and the page laid them out left
//        to right, wrapping. Their x / y are all zero. This places them where
//        that flow put them, with heights doubled for the 60 px row.
// HOW:   Each tile takes the first free spot at or after the previous one
//        (row-major), as CSS grid auto-placement does.
export function flowLayout<T extends Cell>(items: readonly T[], cols: number = GRID_COLS): T[] {
  const placed: T[] = [];
  let row = 0;
  let col = 0;
  for (const item of items) {
    const w = clamp(whole(item.w, 4), 2, cols);
    const h = Math.max(2, whole(item.h, 3) * 2);
    let spotRow = row;
    let spotCol = col;
    for (;;) {
      if (spotCol + w > cols) {
        spotRow += 1;
        spotCol = 0;
        continue;
      }
      const probe = { x: spotCol, y: spotRow, w, h };
      const hit = placed.find((p) => overlaps(p, probe));
      if (hit === undefined) break;
      spotCol = hit.x + hit.w;
    }
    placed.push({ ...item, x: spotCol, y: spotRow, w, h });
    row = spotRow;
    col = spotCol + w;
  }
  return placed;
}

/// Positions for a dashboard saved under `layoutVersion` (absent = the first grid).
export function loadLayout<T extends Cell>(items: readonly T[], layoutVersion: number | undefined, cols: number = GRID_COLS): T[] {
  return (layoutVersion ?? 0) >= LAYOUT_VERSION ? normalizeLayout(items, cols) : normalizeLayout(flowLayout(items, cols), cols);
}

// ---------------------------------------------------------------- pixels
export interface Metrics {
  /// Width of one column.
  col: number;
  row: number;
  gap: number;
}

export function metricsFor(width: number, cols: number = GRID_COLS): Metrics {
  return { col: Math.max(1, (width - GRID_GAP * (cols + 1)) / cols), row: ROW_HEIGHT, gap: GRID_GAP };
}

export interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export function cellRect(cell: Cell, m: Metrics): Rect {
  return {
    left: m.gap + cell.x * (m.col + m.gap),
    top: m.gap + cell.y * (m.row + m.gap),
    width: cell.w * m.col + (cell.w - 1) * m.gap,
    height: cell.h * m.row + (cell.h - 1) * m.gap,
  };
}

/// The cell under a point (grid-relative pixels), kept inside the columns.
export function pointToCell(px: number, py: number, m: Metrics, cols: number = GRID_COLS): { gx: number; gy: number } {
  return {
    gx: clamp(Math.floor((px - m.gap) / (m.col + m.gap)), 0, cols - 1),
    gy: Math.max(0, Math.floor((py - m.gap) / (m.row + m.gap))),
  };
}

/// How many whole cells a pixel distance spans, rounded to the nearest.
export function pixelsToCells(distance: number, size: number, gap: number): number {
  return Math.round(distance / (size + gap));
}

/// Pixel height of a grid `rows` tall.
export function gridHeight(rows: number, m: Metrics): number {
  return rows * m.row + (rows + 1) * m.gap;
}

// ---------------------------------------------------------------- move
// WHAT:  The layout while a tile is dragged over `desired` (its top-left cell).
// HOW:   The dragged tile follows the pointer, bounded by the columns. Dropping
//        it onto another tile swaps the two (the other takes the dragged tile's
//        old place) when that leaves no overlap; otherwise the other tiles stay
//        and `settleDrop` finds the dragged one a free spot on release.
export function previewMove<T extends Placed>(base: readonly T[], id: string, desired: { x: number; y: number }, cols: number = GRID_COLS): T[] {
  const origin = base.find((t) => t.id === id);
  if (origin === undefined) return [...base];
  const dragged: T = { ...origin, x: clamp(Math.floor(desired.x), 0, cols - origin.w), y: Math.max(0, Math.floor(desired.y)) };
  const others = base.filter((t) => t.id !== id);
  const hit = others.find((t) => overlaps(dragged, t));
  const follow = base.map((t) => (t.id === id ? dragged : t));
  if (hit === undefined) return follow;
  const moved: T = { ...origin, x: hit.x, y: hit.y };
  const displaced: T = { ...hit, x: origin.x, y: origin.y };
  const swapped = base.map((t) => (t.id === id ? moved : t.id === hit.id ? displaced : t));
  const inBounds = moved.x + moved.w <= cols && displaced.x + displaced.w <= cols;
  const clear = swapped.every((a, i) => swapped.every((b, j) => i >= j || !overlaps(a, b)));
  return inBounds && clear ? swapped : follow;
}

/// After a drop: a tile still overlapping another moves to the nearest free spot.
export function settleDrop<T extends Placed>(layout: readonly T[], id: string, cols: number = GRID_COLS): T[] {
  const dropped = layout.find((t) => t.id === id);
  if (dropped === undefined) return [...layout];
  const others = layout.filter((t) => t.id !== id);
  if (!others.some((t) => overlaps(dropped, t))) return [...layout];
  const slot = findFreeSlot(others, dropped.w, dropped.h, [{ x: dropped.x, y: dropped.y + dropped.h }, { x: dropped.x + dropped.w, y: dropped.y }], dropped.y, cols);
  return layout.map((t) => (t.id === id ? { ...t, x: slot.x, y: slot.y } : t));
}

/// A tile nudged by whole cells (keyboard), bounded, swapping like a drag.
export function nudge<T extends Placed>(layout: readonly T[], id: string, dx: number, dy: number, cols: number = GRID_COLS): T[] {
  const tile = layout.find((t) => t.id === id);
  if (tile === undefined) return [...layout];
  return settleDrop(previewMove(layout, id, { x: tile.x + dx, y: tile.y + dy }, cols), id, cols);
}

// ---------------------------------------------------------------- resize
// WHAT:  The layout with tile `id` resized toward `want`.
// HOW:   The size is bounded by the columns and the minimum; if the wanted size
//        would run into another tile, the nearest size that does not is used, so
//        the handle stops at the neighbour instead of snapping back.
export function resizeTo<T extends Placed>(layout: readonly T[], id: string, want: { w: number; h: number }, cols: number = GRID_COLS): T[] {
  const tile = layout.find((t) => t.id === id);
  if (tile === undefined) return [...layout];
  const others = layout.filter((t) => t.id !== id);
  const maxW = cols - tile.x;
  const goalW = clamp(Math.round(want.w), Math.min(MIN_W, maxW), maxW);
  const goalH = Math.max(MIN_H, Math.round(want.h));
  let best: { w: number; h: number } | null = null;
  let bestCost = Infinity;
  for (let w = Math.min(MIN_W, maxW); w <= goalW; w += 1) {
    for (let h = MIN_H; h <= goalH; h += 1) {
      if (others.some((o) => overlaps({ x: tile.x, y: tile.y, w, h }, o))) continue;
      const cost = goalW - w + (goalH - h);
      if (cost < bestCost) {
        bestCost = cost;
        best = { w, h };
      }
    }
  }
  if (best === null || (best.w === tile.w && best.h === tile.h)) return [...layout];
  const size = best;
  return layout.map((t) => (t.id === id ? { ...t, w: size.w, h: size.h } : t));
}

// ---------------------------------------------------------------- create
/// The rectangle between two corner cells, either order, kept inside the columns.
export function drawnCell(a: { gx: number; gy: number }, b: { gx: number; gy: number }, cols: number = GRID_COLS): Cell {
  const x = Math.min(a.gx, b.gx);
  const y = Math.min(a.gy, b.gy);
  return { x, y, w: Math.min(Math.abs(a.gx - b.gx) + 1, cols - x), h: Math.abs(a.gy - b.gy) + 1 };
}

/// A drawn rectangle too thin to hold a tile grows to a usable one.
export function finalizeDrawn(cell: Cell, cols: number = GRID_COLS): Cell {
  const w = cell.w < MIN_W ? 3 : cell.w;
  return { x: Math.min(cell.x, cols - Math.min(w, cols)), y: cell.y, w: Math.min(w, cols), h: cell.h < MIN_H ? 3 : cell.h };
}

/// Where a drawn tile lands: where it was drawn if that is free, else the nearest free spot.
export function slotForDrawn(items: readonly Cell[], drawn: Cell, cols: number = GRID_COLS): { x: number; y: number } {
  if (!items.some((o) => overlaps(drawn, o))) return { x: drawn.x, y: drawn.y };
  return findFreeSlot(items, drawn.w, drawn.h, [{ x: drawn.x, y: drawn.y + drawn.h }, { x: drawn.x + drawn.w, y: drawn.y }], drawn.y, cols);
}

/// Where a copy of `source` lands: below it, else beside it, else the nearest free spot.
export function slotForDuplicate(items: readonly Cell[], source: Cell, cols: number = GRID_COLS): { x: number; y: number } {
  return findFreeSlot(items, source.w, source.h, [{ x: source.x, y: source.y + source.h }, { x: source.x + source.w, y: source.y }], source.y, cols);
}

/// Where a "+ Widget" tile of the default size lands: the first free spot from the top.
export function slotForNew(items: readonly Cell[], w: number, h: number, cols: number = GRID_COLS): { x: number; y: number } {
  return findFreeSlot(items, w, h, [], 0, cols);
}

// ---------------------------------------------------------------- narrow
/// The tiles as one full-width column, in reading order (top to bottom, then left to right).
export function stackLayout<T extends Placed>(items: readonly T[], cols: number = GRID_COLS): T[] {
  const ordered = [...items].sort((a, b) => a.y - b.y || a.x - b.x);
  let y = 0;
  return ordered.map((t) => {
    const stacked: T = { ...t, x: 0, y, w: cols };
    y += t.h;
    return stacked;
  });
}
