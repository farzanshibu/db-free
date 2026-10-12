// SOT: relation-edge-geometry, edge-path, bend-point-routing, arrowhead
import type { DiagramSide } from "@/lib/bindings";
import type { Point } from "./metrics";

// WHAT:  The shape of a relation line: a curve between two sides of two cards, or
//        a polyline with rounded corners through the user's bend points, plus the
//        arrowhead at the target and a point to hang the label on.
// WHY:   The canvas draws it with React Flow and the image export draws it into a
//        standalone SVG. Both ask this one function, so an exported diagram shows
//        the lines exactly as they were on screen.
// WHERE: src/features/diagrams/RelationEdge.tsx, exportSvg.ts

export interface EdgeRoute {
  /// SVG path data for the line.
  d: string;
  /// Where the label sits: the middle of the line.
  label: Point;
  /// The arrowhead's tip (the target anchor) and the direction it points.
  tip: Point;
  angle: number;
}

const CORNER_RADIUS = 10;
const ARROW_LENGTH = 9;
const ARROW_HALF_WIDTH = 4.5;

function outward(side: DiagramSide): Point {
  switch (side) {
    case "top":
      return { x: 0, y: -1 };
    case "bottom":
      return { x: 0, y: 1 };
    case "left":
      return { x: -1, y: 0 };
    case "right":
      return { x: 1, y: 0 };
  }
}

function distance(a: Point, b: Point): number {
  return Math.hypot(b.x - a.x, b.y - a.y);
}

function toward(from: Point, to: Point, length: number): Point {
  const d = distance(from, to);
  if (d === 0) return from;
  const k = Math.min(length, d / 2) / d;
  return { x: from.x + (to.x - from.x) * k, y: from.y + (to.y - from.y) * k };
}

const round = (n: number): number => Math.round(n * 100) / 100;
const fmt = (p: Point): string => `${round(p.x)},${round(p.y)}`;

// WHAT:  Routes a line from `from` (leaving `fromSide`) to `to` (entering `toSide`).
export function routeEdge(from: Point, fromSide: DiagramSide, to: Point, toSide: DiagramSide, bends: readonly Point[]): EdgeRoute {
  if (bends.length > 0) return polyline([from, ...bends, to]);
  const a = outward(fromSide);
  const b = outward(toSide);
  // Both ends on the same point (a table related to itself): loop out and back.
  const loop = distance(from, to) < 2;
  const reach = loop ? 70 : Math.max(40, distance(from, to) * 0.35);
  const spread = loop ? 40 : 0;
  const c1: Point = { x: from.x + a.x * reach - a.y * spread, y: from.y + a.y * reach + a.x * spread };
  const c2: Point = { x: to.x + b.x * reach + b.y * spread, y: to.y + b.y * reach - b.x * spread };
  // Cubic Bézier at t = 0.5.
  const label: Point = { x: (from.x + 3 * c1.x + 3 * c2.x + to.x) / 8, y: (from.y + 3 * c1.y + 3 * c2.y + to.y) / 8 };
  return {
    d: `M ${fmt(from)} C ${fmt(c1)} ${fmt(c2)} ${fmt(to)}`,
    label,
    tip: to,
    angle: Math.atan2(to.y - c2.y, to.x - c2.x),
  };
}

// WHAT:  A path through every point, corners rounded where the line turns.
function polyline(points: readonly Point[]): EdgeRoute {
  const first = points[0];
  const last = points[points.length - 1];
  const beforeLast = points[points.length - 2];
  if (!first || !last || !beforeLast) return { d: "", label: { x: 0, y: 0 }, tip: { x: 0, y: 0 }, angle: 0 };
  let d = `M ${fmt(first)}`;
  for (let i = 1; i < points.length - 1; i += 1) {
    const prev = points[i - 1];
    const corner = points[i];
    const next = points[i + 1];
    if (!prev || !corner || !next) continue;
    const enter = toward(corner, prev, CORNER_RADIUS);
    const leave = toward(corner, next, CORNER_RADIUS);
    d += ` L ${fmt(enter)} Q ${fmt(corner)} ${fmt(leave)}`;
  }
  d += ` L ${fmt(last)}`;

  // The label sits halfway along the line's length.
  const total = points.slice(1).reduce((sum, p, i) => sum + distance(points[i] ?? p, p), 0);
  let walked = 0;
  let label = first;
  for (let i = 1; i < points.length; i += 1) {
    const a = points[i - 1];
    const b = points[i];
    if (!a || !b) continue;
    const length = distance(a, b);
    if (walked + length >= total / 2) {
      const k = length === 0 ? 0 : (total / 2 - walked) / length;
      label = { x: a.x + (b.x - a.x) * k, y: a.y + (b.y - a.y) * k };
      break;
    }
    walked += length;
  }
  return { d, label, tip: last, angle: Math.atan2(last.y - beforeLast.y, last.x - beforeLast.x) };
}

// WHAT:  The three corners of the arrowhead for a route, as an SVG `points` value.
export function arrowPoints(route: Pick<EdgeRoute, "tip" | "angle">, scale = 1): string {
  const { tip, angle } = route;
  const back: Point = { x: tip.x - Math.cos(angle) * ARROW_LENGTH * scale, y: tip.y - Math.sin(angle) * ARROW_LENGTH * scale };
  const side: Point = { x: -Math.sin(angle) * ARROW_HALF_WIDTH * scale, y: Math.cos(angle) * ARROW_HALF_WIDTH * scale };
  return [fmt(tip), fmt({ x: back.x + side.x, y: back.y + side.y }), fmt({ x: back.x - side.x, y: back.y - side.y })].join(" ");
}

// WHAT:  A point on the line's bends, used to insert a new bend point where the
//        user double-clicked: the index it belongs at among the existing bends.
export function insertionIndex(from: Point, bends: readonly Point[], to: Point, at: Point): number {
  const chain = [from, ...bends, to];
  let best = 0;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (let i = 0; i < chain.length - 1; i += 1) {
    const a = chain[i];
    const b = chain[i + 1];
    if (!a || !b) continue;
    const d = distanceToSegment(at, a, b);
    if (d < bestDistance) {
      bestDistance = d;
      best = i;
    }
  }
  return best;
}

function distanceToSegment(p: Point, a: Point, b: Point): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared === 0) return distance(p, a);
  const t = Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / lengthSquared));
  return distance(p, { x: a.x + t * dx, y: a.y + t * dy });
}
