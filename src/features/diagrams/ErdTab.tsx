// SOT: erd-tab, auto-er-diagram, foreign-key-graph, diagram-export, erd-layout-memory
import { useCallback, useEffect, useMemo, useState } from "react";
import { Background, ConnectionMode, Controls, MiniMap, Panel, ReactFlow, ReactFlowProvider, useNodesState, useReactFlow, type NodeMouseHandler, type OnMove } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { ColumnInfo, ConnectionColor, TableRef } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { useResolvedTheme } from "@/stores/useTheme";
import { IconButton } from "@/components/global/Button";
import { EmptyState } from "@/components/global/EmptyState";
import { Icon, typeIcon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { EdgeActionsContext, RelationEdge, type EdgeActions, type RelationFlowEdge } from "./RelationEdge";
import { TableNode, type TableFlowNode } from "./TableNode";
import { NodeMenu } from "./NodeMenu";
import { ToolbarMenu, type ToolbarMenuItem } from "./ToolbarMenu";
import { buildErdGraph, cardSize, erdForeignKeys, erdToDiagram, initialPositions, type ErdGraph } from "./erdModel";
import { exportDiagram, type ExportFormat } from "./exportDiagram";
import { restrict, type ExportModel } from "./exportSvg";
import { autoLayout, type LayoutDirection } from "./layout";
import { MAX_ERD_ROWS, resolveSides, type Box } from "./metrics";
import { DEFAULT_LINE_COLOR } from "./palette";
import { useDiagramCustomizations, type DiagramCustomizations } from "./useDiagramCustomizations";

const NODE_TYPES = { table: TableNode };
const EDGE_TYPES = { relation: RelationEdge };
const SNAP_GRID: [number, number] = [16, 16];
/// The ER diagram draws this many tables at most; more would be unreadable and slow.
const TABLE_LIMIT = 120;

const LAYOUT_ITEMS: readonly ToolbarMenuItem[] = [
  { id: "TB", label: "Top to bottom", icon: "rows" },
  { id: "LR", label: "Left to right", icon: "columns" },
  { id: "compact", label: "Compact", icon: "grid" },
];

function isLayoutDirection(id: string): id is LayoutDirection {
  return id === "TB" || id === "LR" || id === "compact";
}

// WHAT:  ER diagram derived from the database's foreign keys (PRD §4.4), with the
//        user's arrangement remembered: card positions, colours, bend points and the
//        pan / zoom are stored per (connection, schema) and restored on the next open.
// HOW:   This component reads the schema (tables, columns, foreign keys) and the
//        stored customizations, then mounts the canvas once both are in; reloading
//        from the database remounts it with the same stored arrangement.
// WHERE: src-tauri/src/integrations/*.rs (foreign_keys), src-tauri/src/store/diagram_customizations.rs,
//        src/features/diagrams/TableNode.tsx
export function ErdTab({ connectionId, schema }: { connectionId: string; schema: string | null }) {
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const showError = useWorkspace((s) => s.showError);
  const [loaded, setLoaded] = useState<{ graph: ErdGraph; version: number; truncated: boolean } | null>(null);
  const [loading, setLoading] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const custom = useDiagramCustomizations(connectionId, schema, (message) => {
    showError({ kind: "internal", message });
  });

  const tables = useMemo<TableRef[]>(
    () => (catalog?.schemas ?? []).filter((s) => schema === null || s.name === schema).flatMap((s) => s.tables.filter((t) => t.kind === "table").map((t) => ({ schema: t.schema, name: t.name }))),
    [catalog, schema],
  );

  useEffect(() => {
    const token = { cancelled: false };
    void (async () => {
      setLoading(true);
      try {
        const fks = await ipc("load_foreign_keys", { connectionId });
        const limited = tables.slice(0, TABLE_LIMIT);
        const columns = await Promise.all(limited.map((t) => ipc("load_columns", { connectionId, table: t }).catch((): ColumnInfo[] => [])));
        if (token.cancelled) return;
        setLoaded((previous) => ({ graph: buildErdGraph(limited, columns, fks), version: (previous?.version ?? 0) + 1, truncated: tables.length > TABLE_LIMIT }));
      } catch (raw) {
        if (!token.cancelled) showError(normalizeError(raw));
      } finally {
        if (!token.cancelled) setLoading(false);
      }
    })();
    return () => {
      token.cancelled = true;
    };
  }, [connectionId, tables, refresh, showError]);

  if (tables.length === 0) {
    return <EmptyState icon="view" title="No tables to diagram" body="Connect to a database with tables, or pick another schema in the sidebar." />;
  }

  return (
    <div className="relative h-full w-full">
      {loaded && custom.ready ? (
        <ReactFlowProvider>
          <ErdCanvas
            key={loaded.version}
            connectionId={connectionId}
            schema={schema}
            graph={loaded.graph}
            truncated={loaded.truncated}
            tableCount={tables.length}
            loading={loading}
            custom={custom}
            onReload={() => {
              setRefresh((r) => r + 1);
            }}
          />
        </ReactFlowProvider>
      ) : (
        <div className="flex h-full items-center justify-center">
          <Spinner size="sm" />
        </div>
      )}
    </div>
  );
}

interface CanvasProps {
  connectionId: string;
  schema: string | null;
  graph: ErdGraph;
  truncated: boolean;
  tableCount: number;
  loading: boolean;
  custom: DiagramCustomizations;
  onReload: () => void;
}

function ErdCanvas({ connectionId, schema, graph, truncated, tableCount, loading, custom, onReload }: CanvasProps) {
  const colorMode = useResolvedTheme();
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const openTable = useWorkspace((s) => s.openTable);
  const openDocument = useWorkspace((s) => s.openDocument);
  const saveDocument = useWorkspace((s) => s.saveDocument);
  const connectionName = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.name ?? null);
  const { fitView } = useReactFlow();
  const marks = useMemo(() => erdForeignKeys(graph), [graph]);

  // The cards start where they were left; the canvas owns their positions from here on.
  const [initial] = useState<TableFlowNode[]>(() => {
    const positions = initialPositions(graph, custom.positions);
    return graph.tables.map((t) => ({
      id: t.key,
      type: "table",
      position: positions.get(t.key) ?? { x: 0, y: 0 },
      data: {
        title: t.ref.name,
        schema: t.ref.schema,
        color: null,
        dimmed: false,
        mode: "erd",
        columns: t.columns.map((c) => ({
          name: c.name,
          type: c.dataType,
          pk: c.primaryKey,
          fk: marks.get(t.key)?.get(c.name) ?? null,
          unique: false,
          icon: typeIcon(c.dataType, c.primaryKey),
        })),
      },
    }));
  });
  const [nodes, setNodes, onNodesChange] = useNodesState<TableFlowNode>(initial);
  const [selectedEdge, setSelectedEdge] = useState<string | null>(null);
  const [snapOn, setSnapOn] = useState(false);
  const [modifierHeld, setModifierHeld] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number; keys: string[]; title: string } | null>(null);
  const [exporting, setExporting] = useState(false);
  const [viewport] = useState(() => custom.viewport);

  // Holding Ctrl / Cmd snaps while dragging, without touching the toggle.
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey) setModifierHeld(true);
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === "Control" || e.key === "Meta") setModifierHeld(false);
    };
    const blur = () => {
      setModifierHeld(false);
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, []);

  const displayNodes = useMemo(
    () =>
      nodes.map((n) => {
        const color = custom.colors.get(n.id) ?? null;
        return n.data.color === color ? n : { ...n, data: { ...n.data, color } };
      }),
    [nodes, custom.colors],
  );

  const edges = useMemo<RelationFlowEdge[]>(() => {
    const at = new Map(nodes.map((n) => [n.id, n.position]));
    const boxes = new Map<string, Box>();
    for (const table of graph.tables) {
      const position = at.get(table.key);
      if (position) boxes.set(table.key, { x: position.x, y: position.y, ...cardSize(table) });
    }
    return graph.relations.map((rel) => {
      const from = boxes.get(rel.source);
      const to = boxes.get(rel.target);
      const sides = from && to ? resolveSides(null, null, from, to, rel.source === rel.target) : { fromSide: "bottom" as const, toSide: "top" as const };
      const active = selectedEdge === rel.key;
      return {
        id: rel.key,
        type: "relation",
        source: rel.source,
        target: rel.target,
        sourceHandle: sides.fromSide,
        targetHandle: sides.toSide,
        selected: active,
        animated: active,
        data: {
          label: `${rel.fk.fromColumns.join(", ")} → ${rel.fk.toColumns.join(", ")}`,
          color: DEFAULT_LINE_COLOR,
          width: active ? 2.4 : 1.6,
          dashed: false,
          dimmed: selectedEdge !== null && !active,
          bends: [...(custom.bends.get(rel.key) ?? [])],
          editable: true,
        },
      };
    });
  }, [graph, nodes, selectedEdge, custom]);

  const edgeActions = useMemo<EdgeActions>(
    () => ({
      setBends: custom.setBends,
      select: setSelectedEdge,
    }),
    [custom.setBends],
  );

  const selectedKeys = useMemo(() => nodes.filter((n) => n.selected === true).map((n) => n.id), [nodes]);

  const layout = useCallback(
    (direction: LayoutDirection) => {
      const placed = autoLayout(
        graph.tables.map((t) => ({ id: t.key, ...cardSize(t) })),
        graph.relations.map((r) => ({ source: r.source, target: r.target })),
        direction,
      );
      setNodes((current) => current.map((n) => ({ ...n, position: placed.get(n.id) ?? n.position })));
      custom.setPositions([...placed]);
      // Bend points are absolute coordinates; after a re-layout they would float in empty space.
      custom.clearBends(graph.relations.map((r) => r.key));
      setSelectedEdge(null);
      window.setTimeout(() => {
        void fitView({ duration: 300, padding: 0.1 });
      }, 50);
    },
    [graph, custom, setNodes, fitView],
  );

  const toExportModel = useCallback((): ExportModel => {
    const at = new Map(nodes.map((n) => [n.id, n.position]));
    return {
      tables: graph.tables.map((t) => ({
        id: t.key,
        x: at.get(t.key)?.x ?? 0,
        y: at.get(t.key)?.y ?? 0,
        title: t.ref.name,
        schema: t.ref.schema,
        color: custom.colors.get(t.key) ?? null,
        columns: t.columns.map((c) => ({ name: c.name, type: c.dataType, pk: c.primaryKey, fk: marks.get(t.key)?.has(c.name) === true })),
      })),
      relations: graph.relations.map((r) => ({
        id: r.key,
        from: r.source,
        to: r.target,
        fromSide: null,
        toSide: null,
        color: null,
        width: 1.6,
        bends: [...(custom.bends.get(r.key) ?? [])],
        label: `${r.fk.fromColumns.join(", ")} → ${r.fk.toColumns.join(", ")}`,
      })),
      notes: [],
    };
  }, [nodes, graph, custom.colors, custom.bends, marks]);

  const runExport = async (format: ExportFormat, selectionOnly: boolean) => {
    setExporting(true);
    try {
      const whole = toExportModel();
      const model = selectionOnly ? restrict(whole, new Set(selectedKeys), new Set()) : whole;
      const written = await exportDiagram({
        model,
        format,
        name: `${schema ?? connectionName ?? "schema"}-diagram${selectionOnly ? "-selection" : ""}`,
        dark: colorMode === "dark",
        options: { maxRows: MAX_ERD_ROWS },
      });
      if (written !== null) showInfo(`Diagram exported to ${written}.`);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setExporting(false);
    }
  };

  const editInDesigner = async () => {
    const positions = new Map(nodes.map((n) => [n.id, n.position]));
    const body = erdToDiagram(graph, positions, custom.colors, custom.bends);
    try {
      const saved = await saveDocument({
        id: "",
        kind: "diagram",
        connectionId,
        name: `${schema ?? connectionName ?? "Database"} diagram`,
        body: { kind: "diagram", data: body },
        tags: [],
        createdAt: "",
        updatedAt: "",
      });
      openDocument("diagram", saved.id, connectionId);
      showInfo("Saved an editable copy in Schema Diagrams.");
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  const onNodeDoubleClick: NodeMouseHandler<TableFlowNode> = (_, node) => {
    const table = graph.tables.find((t) => t.key === node.id);
    if (table) openTable(connectionId, table.ref);
  };

  const onNodeContextMenu: NodeMouseHandler<TableFlowNode> = (event, node) => {
    event.preventDefault();
    const keys = selectedKeys.includes(node.id) ? selectedKeys : [node.id];
    setMenu({ x: event.clientX, y: event.clientY, keys, title: keys.length > 1 ? `${keys.length} tables` : node.data.title });
  };

  const onMoveEnd: OnMove = (event, vp) => {
    // `event` is null for a programmatic move (fit view); only the user's pan and zoom are remembered.
    if (event !== null) custom.setViewport(vp);
  };

  const exportItems: ToolbarMenuItem[] = [
    { id: "png", label: "PNG image", icon: "download", section: "Whole diagram" },
    { id: "svg", label: "SVG image", icon: "download" },
    { id: "png-selection", label: "PNG image", icon: "download", section: "Selected tables", disabled: selectedKeys.length === 0 },
    { id: "svg-selection", label: "SVG image", icon: "download", disabled: selectedKeys.length === 0 },
  ];

  const menuColor = menu ? (menu.keys.length === 1 ? (custom.colors.get(menu.keys[0] ?? "") ?? null) : null) : null;
  const menuTable = menu?.keys.length === 1 ? graph.tables.find((t) => t.key === menu.keys[0]) : undefined;

  return (
    <EdgeActionsContext value={edgeActions}>
      <ReactFlow
        nodes={displayNodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onNodeDragStop={(_, __, dragged) => {
          custom.setPositions(dragged.map((n) => [n.id, n.position] as const));
        }}
        onNodeClick={() => {
          setSelectedEdge(null);
        }}
        onNodeDoubleClick={onNodeDoubleClick}
        onNodeContextMenu={onNodeContextMenu}
        onEdgeClick={(_, edge) => {
          // Never a toggle: the first click of a double-click (which adds a bend) must not
          // be undone by the second.
          setSelectedEdge(edge.id);
        }}
        onPaneClick={() => {
          setSelectedEdge(null);
          setMenu(null);
        }}
        onMoveEnd={onMoveEnd}
        nodeTypes={NODE_TYPES}
        edgeTypes={EDGE_TYPES}
        connectionMode={ConnectionMode.Loose}
        nodesConnectable={false}
        deleteKeyCode={null}
        snapToGrid={snapOn || modifierHeld}
        snapGrid={SNAP_GRID}
        minZoom={0.05}
        maxZoom={2}
        colorMode={colorMode}
        proOptions={{ hideAttribution: true }}
        {...(viewport ? { defaultViewport: viewport } : { fitView: true })}
      >
        <Background gap={24} size={1} />
        <Controls showInteractive={false} />
        <MiniMap pannable zoomable className="!bg-surface" nodeColor="var(--accent)" maskColor="var(--backdrop)" />
        {selectedEdge !== null ? (
          <Panel position="bottom-center" className="rounded-md border border-border bg-surface/90 px-2 py-1 text-[11px] text-muted backdrop-blur">
            Double-click the line to add a bend. Drag a bend to move it; double-click it to remove it.
          </Panel>
        ) : null}
      </ReactFlow>
      <div className="absolute top-2 right-2 flex items-center gap-1 rounded-lg border border-border bg-surface/90 p-1 backdrop-blur">
        <IconButton icon="refresh" label="Reload from database" onClick={onReload} />
        <ToolbarMenu
          icon="sort"
          label="Auto layout"
          items={LAYOUT_ITEMS}
          onSelect={(id) => {
            if (isLayoutDirection(id)) layout(id);
          }}
        />
        <IconButton icon="grid" label={snapOn ? "Snap to grid: on (hold Ctrl / Cmd to snap while dragging)" : "Snap to grid: off (hold Ctrl / Cmd to snap while dragging)"} active={snapOn} onClick={() => { setSnapOn((v) => !v); }} />
        <ToolbarMenu
          icon="download"
          label="Export image"
          items={exportItems}
          disabled={exporting}
          onSelect={(id) => {
            const format: ExportFormat = id.startsWith("png") ? "png" : "svg";
            void runExport(format, id.endsWith("selection"));
          }}
        />
        <IconButton icon="pencil" label="Edit in designer (saves an editable copy)" onClick={() => void editInDesigner()} />
      </div>
      <div className="absolute bottom-3 left-14 flex items-center gap-2 rounded-md border border-border bg-surface/90 px-2 py-1 text-[11px] text-muted backdrop-blur">
        {loading ? <Spinner size="sm" /> : <Icon name="view" size={12} />}
        {graph.tables.length} tables · {graph.relations.length} relations{truncated ? ` · showing first ${TABLE_LIMIT} of ${tableCount}` : ""}
      </div>
      {nodes.length === 0 && !loading ? (
        <div className="absolute inset-0 flex items-center justify-center">
          <Button size="sm" variant="secondary" onClick={onReload}>
            Reload
          </Button>
        </div>
      ) : null}
      {menu ? (
        <NodeMenu
          at={menu}
          title={menu.title}
          onClose={() => {
            setMenu(null);
          }}
          color={{
            value: menuColor,
            resetLabel: "Default colour",
            onChange: (color: ConnectionColor | null) => {
              custom.setColor(menu.keys, color);
              setMenu(null);
            },
          }}
          entries={menuTable ? [{ id: "open", label: "Open table", icon: "table" }] : []}
          onAction={(id) => {
            if (id === "open" && menuTable) openTable(connectionId, menuTable.ref);
          }}
        />
      ) : null}
    </EdgeActionsContext>
  );
}
