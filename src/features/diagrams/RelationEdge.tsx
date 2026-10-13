// SOT: relation-edge, diagram-edge-renderer, bend-point-editor, edge-label
import { createContext, useContext, useState, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { EdgeLabelRenderer, Position, useReactFlow, type Edge, type EdgeProps } from "@xyflow/react";
import type { DiagramSide } from "@/lib/bindings";
import { arrowPoints, insertionIndex, routeEdge } from "./edgePath";
import type { Point } from "./metrics";

// eslint-disable-next-line @typescript-eslint/consistent-type-definitions
export type RelationEdgeData = {
  /// "user_id → id"; empty = no label.
  label: string;
  /// A CSS colour (token) for the line, the arrowhead and the bend handles.
  color: string;
  /// Stroke width in px.
  width: number;
  /// Drawn dashed: a relation whose columns are not chosen yet is not a foreign key.
  dashed: boolean;
  /// Another relation is selected, so this one fades back.
  dimmed: boolean;
  bends: Point[];
  /// Bend points can be added, moved and removed.
  editable: boolean;
};

// WHAT:  What a relation line can ask its canvas to do.
// WHY:   Passed through context rather than stored in each edge's data, so the data
//        stays plain values and the handlers are created once, where the state they
//        change lives.
export interface EdgeActions {
  /// Replaces the relation's bend points.
  setBends: (relationId: string, bends: Point[]) => void;
  /// Clicking the label selects the relation, like clicking the line.
  select: (relationId: string) => void;
}

export const EdgeActionsContext = createContext<EdgeActions>({
  setBends: () => undefined,
  select: () => undefined,
});

export type RelationFlowEdge = Edge<RelationEdgeData, "relation">;

export function sideOfPosition(position: Position): DiagramSide {
  switch (position) {
    case Position.Top:
      return "top";
    case Position.Right:
      return "right";
    case Position.Bottom:
      return "bottom";
    case Position.Left:
      return "left";
  }
}

// WHAT:  A relation line: a curve between two card sides, or a polyline through
//        the user's bend points, with an arrowhead and a label.
// WHY:   React Flow's built-in edges cannot bend through user points, take a width
//        or fade on cue. The geometry comes from edgePath.ts, which the image
//        export also uses, so the file matches the screen.
// HOW:   Double-click the line to add a bend where you clicked, drag a bend to move
//        it (the line follows live; the change is committed on release), double-click
//        a bend to remove it. Bends show only while the line is selected.
export function RelationEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, selected, data }: EdgeProps<RelationFlowEdge>) {
  const { screenToFlowPosition } = useReactFlow();
  const actions = useContext(EdgeActionsContext);
  const [drag, setDrag] = useState<{ index: number; at: Point } | null>(null);
  if (!data) return null;

  const from: Point = { x: sourceX, y: sourceY };
  const to: Point = { x: targetX, y: targetY };
  const bends = drag ? data.bends.map((b, i) => (i === drag.index ? drag.at : b)) : data.bends;
  const route = routeEdge(from, sideOfPosition(sourcePosition), to, sideOfPosition(targetPosition), bends);
  const width = data.width + (selected === true ? 1 : 0);

  const addBend = (event: ReactMouseEvent) => {
    if (!data.editable) return;
    event.stopPropagation();
    const at = screenToFlowPosition({ x: event.clientX, y: event.clientY });
    const point = { x: Math.round(at.x), y: Math.round(at.y) };
    const index = insertionIndex(from, data.bends, to, point);
    actions.setBends(id, [...data.bends.slice(0, index), point, ...data.bends.slice(index)]);
  };

  const startDrag = (index: number) => (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!data.editable) return;
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    let latest: Point | null = null;
    const move = (e: PointerEvent) => {
      const at = screenToFlowPosition({ x: e.clientX, y: e.clientY });
      latest = { x: Math.round(at.x), y: Math.round(at.y) };
      setDrag({ index, at: latest });
    };
    const finish = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", finish);
      setDrag(null);
      const dropped = latest;
      if (dropped) actions.setBends(id, data.bends.map((b, i) => (i === index ? dropped : b)));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", finish);
  };

  return (
    <>
      <g style={{ opacity: data.dimmed ? 0.18 : 1 }}>
        <path d={route.d} fill="none" stroke={data.color} strokeWidth={width} strokeLinecap="round" {...(data.dashed ? { strokeDasharray: "6 4" } : {})} />
        <polygon points={arrowPoints(route, Math.max(1, data.width / 1.6))} fill={data.color} />
        {/* A wide invisible stroke over the line: easy to hit, and where a bend is added.
            React Flow's own class keeps the marching-ants animation of a selected line off it,
            or the dashes would leave gaps in the hit area. */}
        <path
          className="react-flow__edge-interaction"
          d={route.d}
          fill="none"
          stroke="transparent"
          strokeWidth={16}
          onDoubleClick={addBend}
          style={{ cursor: data.editable ? "copy" : "pointer" }}
        />
      </g>
      <EdgeLabelRenderer>
        {data.label !== "" ? (
          <button
            type="button"
            className="nodrag nopan absolute rounded border border-border bg-surface px-1.5 py-px font-mono text-[10px] whitespace-nowrap text-foreground"
            style={{ transform: `translate(-50%, -50%) translate(${route.label.x}px, ${route.label.y}px)`, pointerEvents: "all", opacity: data.dimmed ? 0.3 : 1 }}
            onClick={() => {
              actions.select(id);
            }}
          >
            {data.label}
          </button>
        ) : null}
        {selected === true && data.editable
          ? bends.map((bend, index) => (
              <div
                key={index}
                role="button"
                tabIndex={-1}
                aria-label="Bend point. Drag to move, double-click to remove."
                className="nodrag nopan absolute size-2.5 cursor-grab rounded-full border-2 border-background active:cursor-grabbing"
                style={{
                  transform: `translate(-50%, -50%) translate(${bend.x}px, ${bend.y}px)`,
                  pointerEvents: "all",
                  background: data.color,
                }}
                onPointerDown={startDrag(index)}
                onDoubleClick={(event) => {
                  event.stopPropagation();
                  actions.setBends(id, data.bends.filter((_, i) => i !== index));
                }}
              />
            ))
          : null}
      </EdgeLabelRenderer>
    </>
  );
}
