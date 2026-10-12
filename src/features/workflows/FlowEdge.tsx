// SOT: workflow-flow-edge, animated-spark-edge, branch-edge-label
import { BaseEdge, EdgeLabelRenderer, getBezierPath, type Edge, type EdgeProps } from "@xyflow/react";

// eslint-disable-next-line @typescript-eslint/consistent-type-definitions
export type FlowEdgeData = {
  /// Rows are travelling along this wire (the source finished and the target is working or done).
  flowing: boolean;
  /// Restarts the spark each time the run reports progress.
  tick: number;
  /// A Branch side: "true" / "false" labels the wire.
  side: "true" | "false" | null;
  /// The wire carried nothing in the last run (a closed Branch side).
  quiet: boolean;
};

export type FlowEdgeType = Edge<FlowEdgeData, "flow">;

// WHAT:  A wire between two nodes: a curve with an arrowhead, and while data is
//        moving along it a small bright streak that travels from source to target.
// WHY:   Seeing which wires carried rows is most of what makes a run readable.
// HOW:   The streak is an SVG `animateMotion` on the same path; `key={tick}` restarts
//        it when the run reports progress. Colours are theme tokens, so both themes work.
export function FlowEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, selected, data, markerEnd }: EdgeProps<FlowEdgeType>) {
  const [path, labelX, labelY] = getBezierPath({ sourceX, sourceY, sourcePosition, targetX, targetY, targetPosition });
  const flowing = data?.flowing === true;
  const side = data?.side ?? null;
  const stroke = selected === true ? "var(--accent)" : flowing ? "var(--success)" : "var(--muted)";
  return (
    <>
      <BaseEdge id={id} path={path} {...(markerEnd !== undefined ? { markerEnd } : {})} style={{ stroke, strokeWidth: selected === true ? 2.5 : 2, opacity: data?.quiet === true ? 0.35 : 0.9 }} />
      {flowing ? (
        <ellipse key={data.tick} rx={14} ry={1.4} fill="var(--success)" opacity={0.95}>
          <animateMotion dur="0.9s" repeatCount="indefinite" path={path} rotate="auto" calcMode="linear" />
        </ellipse>
      ) : null}
      {side !== null ? (
        <EdgeLabelRenderer>
          <span
            className="nodrag nopan pointer-events-none absolute rounded bg-surface-elevated px-1 text-[10px] font-medium"
            style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`, color: side === "true" ? "var(--success)" : "var(--danger)" }}
          >
            {side}
          </span>
        </EdgeLabelRenderer>
      ) : null}
    </>
  );
}
