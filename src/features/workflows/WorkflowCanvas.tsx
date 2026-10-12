// SOT: workflow-canvas, react-flow-workflow-graph, workflow-connection-validation, workflow-context-menu, workflow-minimap
import { useCallback, useEffect, useMemo, useState } from "react";
import { Background, BackgroundVariant, Controls, MarkerType, MiniMap, ReactFlow, useNodesState, type Connection, type Edge } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { NodeUpdate, WorkflowEdge, WorkflowNode } from "@/lib/bindings";
import { useContextMenu, type MenuEntry } from "@/components/global/ContextMenu";
import { useResolvedTheme } from "@/stores/useTheme";
import { FlowEdge, type FlowEdgeType } from "./FlowEdge";
import { NodeCard, type StepFlowNode } from "./NodeCard";
import { NODE_SPECS, canConnect, problemOf } from "./model";

const NODE_TYPES = { step: NodeCard };
const EDGE_TYPES = { flow: FlowEdge };
const DELETE_KEYS = ["Backspace", "Delete"];

interface WorkflowCanvasProps {
  nodes: readonly WorkflowNode[];
  edges: readonly WorkflowEdge[];
  /// Status of each node in the current or last run.
  statuses: Readonly<Record<string, NodeUpdate>>;
  running: boolean;
  /// Changes whenever the run reports progress; restarts the edge animation.
  tick: number;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onMove: (moves: readonly { id: string; x: number; y: number }[]) => void;
  onConnect: (wire: { source: string; target: string; sourceHandle: "true" | "false" | null }) => void;
  onDelete: (nodeIds: readonly string[], edgeIds: readonly string[]) => void;
  onNoteText: (id: string, text: string) => void;
  onRunNode: (id: string) => void;
}

function handleOf(value: string | null | undefined): "true" | "false" | null {
  return value === "true" || value === "false" ? value : null;
}

// WHAT:  The document's nodes as React Flow nodes, merged onto the ones it already has.
// HOW:   A node keeps its measured size and, unless the document moved it since the
//        last merge (compared with the position stored in its own data), the position
//        the user dragged it to. Pure, so a re-run of the state updater gives the same answer.
function buildFlowNodes(
  previous: readonly StepFlowNode[],
  nodes: readonly WorkflowNode[],
  statuses: Readonly<Record<string, NodeUpdate>>,
  selectedId: string | null,
  onNoteText: (id: string, text: string) => void,
): StepFlowNode[] {
  return nodes.map((node): StepFlowNode => {
    const old = previous.find((p) => p.id === node.id);
    const documentMoved = old?.data.node.x !== node.x || old.data.node.y !== node.y;
    return {
      ...old,
      id: node.id,
      type: "step",
      position: old === undefined || documentMoved ? { x: node.x, y: node.y } : old.position,
      selected: node.id === selectedId,
      data: { node, update: statuses[node.id] ?? null, problem: problemOf(node), onNoteText },
    };
  });
}

// WHAT:  The workflow graph as an editable canvas: drag nodes, wire them by dragging
//        from an output to an input, delete with the key or the right-click menu.
// WHY:   Wiring rules are enforced while the wire is dragged (`isValidConnection`),
//        so an impossible connection never even highlights; the same rule set
//        (`canConnect`) guards the document.
// HOW:   React Flow owns positions and selection while the user is interacting; the
//        document's nodes are merged in whenever they change, keeping each node's
//        measured size and (unless the document moved it) its position.
export function WorkflowCanvas({ nodes, edges, statuses, running, tick, selectedId, onSelect, onMove, onConnect, onDelete, onNoteText, onRunNode }: WorkflowCanvasProps) {
  const colorMode = useResolvedTheme();
  const menu = useContextMenu();
  const [initialNodes] = useState(() => buildFlowNodes([], nodes, statuses, selectedId, onNoteText));
  const [flowNodes, setFlowNodes, onNodesChange] = useNodesState<StepFlowNode>(initialNodes);

  useEffect(() => {
    setFlowNodes((previous) => buildFlowNodes(previous, nodes, statuses, selectedId, onNoteText));
  }, [nodes, statuses, selectedId, onNoteText, setFlowNodes]);

  const flowEdges = useMemo<FlowEdgeType[]>(
    () =>
      edges.map((edge): FlowEdgeType => {
        const from = statuses[edge.source];
        const to = statuses[edge.target];
        return {
          id: edge.id,
          source: edge.source,
          target: edge.target,
          sourceHandle: edge.sourceHandle,
          type: "flow",
          markerEnd: { type: MarkerType.ArrowClosed, width: 14, height: 14 },
          data: {
            flowing: from?.state === "success" && to !== undefined && (to.state === "running" || (running && to.state === "success")),
            tick,
            side: edge.sourceHandle,
            quiet: to?.state === "skipped",
          },
        };
      }),
    [edges, statuses, running, tick],
  );

  const isValidConnection = useCallback(
    (candidate: Connection | Edge) => canConnect(nodes, edges, { source: candidate.source, target: candidate.target, sourceHandle: handleOf(candidate.sourceHandle) }).ok,
    [nodes, edges],
  );

  const nodeMenu = (id: string): readonly MenuEntry[] => {
    const node = nodes.find((n) => n.id === id);
    if (node === undefined) return [];
    const entries: MenuEntry[] = [];
    if (node.config.kind !== "note") entries.push({ id: "run", label: "Run up to this node", icon: "play", group: "run" });
    entries.push({ id: "delete", label: "Delete node", icon: "trash", danger: true, group: "edit" });
    return entries;
  };

  return (
    <div className="relative h-full w-full">
      <ReactFlow
        nodes={flowNodes}
        edges={flowEdges}
        nodeTypes={NODE_TYPES}
        edgeTypes={EDGE_TYPES}
        onNodesChange={onNodesChange}
        isValidConnection={isValidConnection}
        onConnect={(connection) => {
          onConnect({ source: connection.source, target: connection.target, sourceHandle: handleOf(connection.sourceHandle) });
        }}
        onNodeDragStop={(_, __, dragged) => {
          onMove(dragged.map((n) => ({ id: n.id, x: Math.round(n.position.x), y: Math.round(n.position.y) })));
        }}
        onSelectionChange={({ nodes: picked }) => {
          onSelect(picked[0]?.id ?? null);
        }}
        onPaneClick={() => { onSelect(null); }}
        onNodesDelete={(deleted) => { onDelete(deleted.map((n) => n.id), []); }}
        onEdgesDelete={(deleted) => { onDelete([], deleted.map((e) => e.id)); }}
        onNodeContextMenu={(event, node) => {
          menu.open(event, nodeMenu(node.id), (action) => {
            if (action === "run") onRunNode(node.id);
            if (action === "delete") onDelete([node.id], []);
          });
        }}
        onEdgeContextMenu={(event, edge) => {
          menu.open(event, [{ id: "delete", label: "Delete connection", icon: "trash", danger: true }], () => { onDelete([], [edge.id]); });
        }}
        deleteKeyCode={DELETE_KEYS}
        colorMode={colorMode}
        fitView
        fitViewOptions={{ padding: 0.25, maxZoom: 1 }}
        minZoom={0.2}
        maxZoom={1.75}
        proOptions={{ hideAttribution: true }}
        connectionRadius={28}
      >
        <Background variant={BackgroundVariant.Dots} gap={22} size={1.2} />
        <Controls showInteractive={false} position="bottom-left" />
        <MiniMap
          pannable
          zoomable
          position="bottom-right"
          className="!bg-surface-elevated"
          nodeColor={(flow) => {
            const domain = nodes.find((n) => n.id === flow.id);
            return `var(--color-conn-${domain === undefined ? "gray" : NODE_SPECS[domain.config.kind].color})`;
          }}
        />
      </ReactFlow>
      {menu.node}
    </div>
  );
}
