// SOT: diagram-auto-layout, layered-layout, compact-layout, diagram-layout
import type { Point } from "./metrics";

// WHAT:  Places table cards: top to bottom, left to right, or packed compactly.
// WHY:   The ER diagram and the designer need the same answer to "tidy this up",
//        so the algorithm lives once, free of React Flow, and works from sizes
//        alone.
// HOW:   Layered drawing. A card's layer is the longest path of relations that
//        reaches it (cycles are broken first by dropping the edge that closes
//        them), so a referenced table sits one layer beyond the tables pointing
//        at it. Within a layer, a few barycentre sweeps reorder cards to cut
//        crossings. A layer wider than a sensible line wraps onto further lines,
//        so a schema of unrelated tables becomes a block, not a mile-long row.
//        Compact ignores layers' geometry and shelf-packs the same order.
// WHERE: src/features/diagrams/ErdTab.tsx, DesignerTab.tsx

export type LayoutDirection = "TB" | "LR" | "compact";

export interface LayoutNode {
  id: string;
  width: number;
  height: number;
}

export interface LayoutEdge {
  source: string;
  target: string;
}

const GAPS = {
  TB: { within: 60, between: 90 },
  LR: { within: 40, between: 130 },
  compact: { within: 32, between: 32 },
} satisfies Record<LayoutDirection, { within: number; between: number }>;

/// Barycentre passes over the layers; the first few remove nearly every avoidable crossing.
const SWEEPS = 4;

// WHAT:  Returns each card's top-left corner. Cards named in no edge still get a place.
export function autoLayout(nodes: readonly LayoutNode[], edges: readonly LayoutEdge[], direction: LayoutDirection): Map<string, Point> {
  const placed = new Map<string, Point>();
  if (nodes.length === 0) return placed;
  const layered = orderedLayers(nodes, edges);
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const gap = GAPS[direction];
  const perLine = Math.max(3, Math.ceil(Math.sqrt(nodes.length) * 1.6));

  if (direction === "compact") {
    const sequence = layered.flat().flatMap((id) => byId.get(id) ?? []);
    const area = sequence.reduce((sum, n) => sum + (n.width + gap.within) * (n.height + gap.between), 0);
    const widest = Math.max(...sequence.map((n) => n.width));
    const lineWidth = Math.max(widest, Math.sqrt(area * 1.7));
    let x = 0;
    let y = 0;
    let lineHeight = 0;
    for (const n of sequence) {
      if (x > 0 && x + n.width > lineWidth) {
        x = 0;
        y += lineHeight + gap.between;
        lineHeight = 0;
      }
      placed.set(n.id, { x: Math.round(x), y: Math.round(y) });
      x += n.width + gap.within;
      lineHeight = Math.max(lineHeight, n.height);
    }
    return placed;
  }

  // Each layer becomes one or more lines of at most `perLine` cards.
  const lines: LayoutNode[][] = [];
  for (const layer of layered) {
    const cards = layer.flatMap((id) => byId.get(id) ?? []);
    for (let i = 0; i < cards.length; i += perLine) lines.push(cards.slice(i, i + perLine));
  }

  if (direction === "TB") {
    const widthOf = (line: LayoutNode[]) => line.reduce((sum, n) => sum + n.width, 0) + gap.within * (line.length - 1);
    const widest = Math.max(...lines.map(widthOf));
    let y = 0;
    for (const line of lines) {
      let x = (widest - widthOf(line)) / 2;
      for (const n of line) {
        placed.set(n.id, { x: Math.round(x), y: Math.round(y) });
        x += n.width + gap.within;
      }
      y += Math.max(...line.map((n) => n.height)) + gap.between;
    }
    return placed;
  }

  const heightOf = (line: LayoutNode[]) => line.reduce((sum, n) => sum + n.height, 0) + gap.within * (line.length - 1);
  const tallest = Math.max(...lines.map(heightOf));
  let x = 0;
  for (const line of lines) {
    let y = (tallest - heightOf(line)) / 2;
    for (const n of line) {
      placed.set(n.id, { x: Math.round(x), y: Math.round(y) });
      y += n.height + gap.within;
    }
    x += Math.max(...line.map((n) => n.width)) + gap.between;
  }
  return placed;
}

// WHAT:  Ids grouped by layer, each layer ordered to keep related cards beside each other.
export function orderedLayers(nodes: readonly LayoutNode[], edges: readonly LayoutEdge[]): string[][] {
  const ids = nodes.map((n) => n.id);
  const known = new Set(ids);
  const outgoing = new Map<string, Set<string>>(ids.map((id) => [id, new Set<string>()]));
  for (const e of edges) {
    if (e.source !== e.target && known.has(e.source) && known.has(e.target)) outgoing.get(e.source)?.add(e.target);
  }
  const acyclic = withoutBackEdges(ids, outgoing);

  // Longest path from any root, by Kahn's algorithm.
  const indegree = new Map<string, number>(ids.map((id) => [id, 0]));
  for (const targets of acyclic.values()) for (const t of targets) indegree.set(t, (indegree.get(t) ?? 0) + 1);
  const rank = new Map<string, number>(ids.map((id) => [id, 0]));
  const queue = ids.filter((id) => indegree.get(id) === 0);
  while (queue.length > 0) {
    const id = queue.shift();
    if (id === undefined) break;
    for (const next of acyclic.get(id) ?? []) {
      rank.set(next, Math.max(rank.get(next) ?? 0, (rank.get(id) ?? 0) + 1));
      const left = (indegree.get(next) ?? 1) - 1;
      indegree.set(next, left);
      if (left === 0) queue.push(next);
    }
  }

  const layerCount = Math.max(...ids.map((id) => rank.get(id) ?? 0)) + 1;
  const layers: string[][] = Array.from({ length: layerCount }, () => []);
  for (const id of ids) layers[rank.get(id) ?? 0]?.push(id);

  const incoming = new Map<string, string[]>(ids.map((id) => [id, []]));
  for (const [from, targets] of acyclic) for (const t of targets) incoming.get(t)?.push(from);

  const position = new Map<string, number>();
  const reindex = (layer: string[]) => layer.forEach((id, i) => position.set(id, i));
  layers.forEach(reindex);
  const barycentre = (id: string, neighbours: readonly string[]) => {
    if (neighbours.length === 0) return position.get(id) ?? 0;
    return neighbours.reduce((sum, n) => sum + (position.get(n) ?? 0), 0) / neighbours.length;
  };
  for (let sweep = 0; sweep < SWEEPS; sweep += 1) {
    for (let r = 1; r < layers.length; r += 1) {
      const layer = layers[r];
      if (!layer) continue;
      layer.sort((a, b) => barycentre(a, incoming.get(a) ?? []) - barycentre(b, incoming.get(b) ?? []));
      reindex(layer);
    }
    for (let r = layers.length - 2; r >= 0; r -= 1) {
      const layer = layers[r];
      if (!layer) continue;
      layer.sort((a, b) => barycentre(a, [...(acyclic.get(a) ?? [])]) - barycentre(b, [...(acyclic.get(b) ?? [])]));
      reindex(layer);
    }
  }
  return layers;
}

// WHAT:  A copy of the graph with the edges that close a cycle removed, found by
//        depth-first search (an edge into a card still on the stack is a back edge).
function withoutBackEdges(ids: readonly string[], outgoing: ReadonlyMap<string, ReadonlySet<string>>): Map<string, Set<string>> {
  const kept = new Map<string, Set<string>>(ids.map((id) => [id, new Set<string>()]));
  const colour = new Map<string, "grey" | "black">();
  for (const root of ids) {
    if (colour.has(root)) continue;
    colour.set(root, "grey");
    const stack: { id: string; targets: string[]; next: number }[] = [{ id: root, targets: [...(outgoing.get(root) ?? [])], next: 0 }];
    for (;;) {
      const frame = stack[stack.length - 1];
      if (!frame) break;
      const target = frame.targets[frame.next];
      if (target === undefined) {
        colour.set(frame.id, "black");
        stack.pop();
        continue;
      }
      frame.next += 1;
      const seen = colour.get(target);
      if (seen === "grey") continue;
      kept.get(frame.id)?.add(target);
      if (seen === undefined) {
        colour.set(target, "grey");
        stack.push({ id: target, targets: [...(outgoing.get(target) ?? [])], next: 0 });
      }
    }
  }
  return kept;
}
