// SOT: diagram-metrics, table-card-size, diagram-geometry
import type { DiagramSide } from "@/lib/bindings";

// WHAT:  The fixed dimensions of a table card, shared by the card component, the
//        auto-layout and the image export.
// WHY:   The export draws the diagram from the model, not from the DOM, so it has
//        to know how tall a card is without measuring it; layout needs the same
//        answer before React Flow has rendered anything. Pinning the card's rows
//        to these heights makes the formula exact rather than an estimate.
// WHERE: src/features/diagrams/TableNode.tsx (the heights are applied there),
//        layout.ts, exportSvg.ts

export const TABLE_WIDTH = 260;
export const HEADER_HEIGHT = 30;
export const ROW_HEIGHT = 22;
/// The single "no columns" row of an empty card.
export const EMPTY_HEIGHT = 24;
/// The "+N more" row under a capped card.
export const MORE_HEIGHT = 20;
/// 1px border top and bottom.
export const BORDER = 2;
/// The ER diagram caps a card at this many rows; the designer shows every column.
export const MAX_ERD_ROWS = 14;

export function tableHeight(columnCount: number, maxRows?: number): number {
  if (columnCount === 0) return HEADER_HEIGHT + EMPTY_HEIGHT + BORDER;
  const shown = maxRows === undefined ? columnCount : Math.min(columnCount, maxRows);
  const more = shown < columnCount ? MORE_HEIGHT : 0;
  return HEADER_HEIGHT + shown * ROW_HEIGHT + more + BORDER;
}

export interface Point {
  x: number;
  y: number;
}

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

export function sideAnchor(box: Box, side: DiagramSide): Point {
  switch (side) {
    case "top":
      return { x: box.x + box.width / 2, y: box.y };
    case "bottom":
      return { x: box.x + box.width / 2, y: box.y + box.height };
    case "left":
      return { x: box.x, y: box.y + box.height / 2 };
    case "right":
      return { x: box.x + box.width, y: box.y + box.height / 2 };
  }
}

// WHAT:  The two sides a relation should leave and enter by when the user has not
//        picked: the pair that faces each other across the larger gap.
export function facingSides(from: Box, to: Box): { fromSide: DiagramSide; toSide: DiagramSide } {
  const gapX = Math.max(to.x - (from.x + from.width), from.x - (to.x + to.width));
  const gapY = Math.max(to.y - (from.y + from.height), from.y - (to.y + to.height));
  if (gapX >= gapY) {
    return from.x + from.width / 2 <= to.x + to.width / 2 ? { fromSide: "right", toSide: "left" } : { fromSide: "left", toSide: "right" };
  }
  return from.y + from.height / 2 <= to.y + to.height / 2 ? { fromSide: "bottom", toSide: "top" } : { fromSide: "top", toSide: "bottom" };
}

// WHAT:  The sides a relation uses: the ones it names, else the pair that faces each
//        other. A relation from a table to itself leaves and returns by the right
//        side, which draws a loop outside the card instead of across it.
export function resolveSides(
  fromSide: DiagramSide | null,
  toSide: DiagramSide | null,
  from: Box,
  to: Box,
  sameTable: boolean,
): { fromSide: DiagramSide; toSide: DiagramSide } {
  if (fromSide !== null && toSide !== null) return { fromSide, toSide };
  const facing: { fromSide: DiagramSide; toSide: DiagramSide } = sameTable ? { fromSide: "right", toSide: "right" } : facingSides(from, to);
  return { fromSide: fromSide ?? facing.fromSide, toSide: toSide ?? facing.toSide };
}

export function snap(value: number, grid: number): number {
  return grid > 0 ? Math.round(value / grid) * grid : value;
}

export function boundsOf(boxes: readonly Box[]): Box | null {
  if (boxes.length === 0) return null;
  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  for (const b of boxes) {
    minX = Math.min(minX, b.x);
    minY = Math.min(minY, b.y);
    maxX = Math.max(maxX, b.x + b.width);
    maxY = Math.max(maxY, b.y + b.height);
  }
  return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}
