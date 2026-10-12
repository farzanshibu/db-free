// SOT: designer-tab, schema-diagram-designer, diagram-autosave, diagram-bulk-actions, diagram-canvas
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Background,
  BackgroundVariant,
  ConnectionMode,
  Controls,
  MiniMap,
  Panel,
  ReactFlow,
  ReactFlowProvider,
  useNodesState,
  useReactFlow,
  type Connection,
  type EdgeMouseHandler,
  type NodeMouseHandler,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { ConnectionColor, DiagramBody, Document } from "@/lib/bindings";
import { normalizeError } from "@/lib/ipc";
import { useActiveConnection, useWorkspace } from "@/stores/workspace";
import { useResolvedTheme } from "@/stores/useTheme";
import { IconButton } from "@/components/global/Button";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { typeCatalog } from "./columnTypes";
import { ColorSwatches } from "./ColorSwatches";
import {
  applyLayout,
  bodyOf,
  describe,
  duplicateTables,
  foreignKeyMarks,
  GRID_SIZES,
  gridOf,
  isEmpty,
  newNote,
  newRelation,
  newTable,
  notesOf,
  notesVisibleOf,
  removeNotes,
  removeRelations,
  removeTables,
  setColor,
  toExportModel,
  updateNote,
  updateRelation,
} from "./designerModel";
import { buildEdges, buildNodes, type DesignerNode, type NodeContext } from "./designerNodes";
import { exportDiagram, type ExportFormat } from "./exportDiagram";
import { restrict } from "./exportSvg";
import { NodeMenu } from "./NodeMenu";
import { NoteInspector } from "./NoteInspector";
import { NoteActionsContext, NoteNode, type NoteActions } from "./NoteNode";
import { EdgeActionsContext, RelationEdge, type EdgeActions, type RelationFlowEdge } from "./RelationEdge";
import { RelationInspector } from "./RelationInspector";
import { SqlDialog } from "./SqlPreview";
import { TableInspector, type Edit } from "./TableInspector";
import { TableNode, columnOfHandle, sideOfHandle } from "./TableNode";
import { TABLE_WIDTH, snap, type Point } from "./metrics";
import { ToolbarMenu, type ToolbarMenuItem } from "./ToolbarMenu";
import { VersionsMenu } from "./VersionsMenu";
import type { LayoutDirection } from "./layout";
import { useAutosave } from "./useAutosave";

const NODE_TYPES = { table: TableNode, note: NoteNode };
const EDGE_TYPES = { relation: RelationEdge };
const DELETE_KEYS = ["Backspace", "Delete"];

const LAYOUT_ITEMS: readonly ToolbarMenuItem[] = [
  { id: "TB", label: "Top to bottom", icon: "rows" },
  { id: "LR", label: "Left to right", icon: "columns" },
  { id: "compact", label: "Compact", icon: "grid" },
];

function isLayoutDirection(id: string): id is LayoutDirection {
  return id === "TB" || id === "LR" || id === "compact";
}

interface MenuTarget {
  kind: "table" | "note" | "relation";
  ids: string[];
  title: string;
}

// WHAT:  Whether keyboard focus is inside something floating (a dropdown's list, a
//        menu, a dialog) or the inspector.
// WHY:   React Flow listens for Backspace / Delete on the whole document. A key
//        pressed to dismiss or edit something there must not delete the selection.
function focusIsInAnOverlay(): boolean {
  const active = document.activeElement;
  return active !== null && active.closest('[role="listbox"], [role="menu"], [role="dialog"], [data-radix-popper-content-wrapper], .nokey') !== null;
}

// React Flow asks before deleting what the keyboard selected; the answer is no while an overlay has focus.
const confirmKeyboardDelete = (): Promise<boolean> => Promise.resolve(!focusIsInAnOverlay());

// WHAT:  The schema designer: a canvas of table cards and sticky notes joined by
//        foreign-key relations, an inspector for whatever is selected, SQL for the
//        diagram in its connection's dialect, image export, and a version history.
// HOW:   The document body is the source of truth and every edit goes through one
//        function (`edit`), which updates the body, rebuilds the canvas's nodes,
//        and tells the autosave there is something to write. Autosave saves a
//        moment after the last edit and files an automatic version after a pause;
//        closing the tab flushes both. React Flow never owns data: deleting a card
//        on the canvas calls back here and removes it from the body, which is why a
//        deletion survives a save.
// WHERE: src-tauri/src/model/documents.rs (DiagramBody), src-tauri/src/services/diagram_ddl.rs,
//        src-tauri/src/services/document_versions.rs
export function DesignerTab({ document: doc }: { document: Document }) {
  return (
    <ReactFlowProvider>
      <Designer doc={doc} />
    </ReactFlowProvider>
  );
}

function Designer({ doc }: { doc: Document }) {
  const colorMode = useResolvedTheme();
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const openChat = useWorkspace((s) => s.openChat);
  const activeConnection = useActiveConnection();
  // A diagram that belongs to no connection borrows the active one: its dialect for the
  // type list and the SQL, and where "Open in query tab" and "Ask AI" go.
  const connectionId = doc.connectionId ?? activeConnection?.id ?? null;
  const engine = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.engine ?? null);
  const { screenToFlowPosition, fitView } = useReactFlow();
  const wrapper = useRef<HTMLDivElement>(null);

  const catalog = useMemo(() => typeCatalog(engine), [engine]);

  // ------------------------------------------------------------- the document
  const [initialBody] = useState(() => bodyOf(doc));
  const [body, setBody] = useState<DiagramBody>(initialBody);
  const bodyRef = useRef(initialBody);
  const docRef = useRef(doc);
  const stamp = useRef(doc.updatedAt);
  const [nameDraft, setNameDraft] = useState<string | null>(null);
  const nameRef = useRef<string | null>(null);
  useEffect(() => {
    docRef.current = doc;
  }, [doc]);

  // ----------------------------------------------------------------- the canvas
  const [focus, setFocus] = useState<ReadonlySet<string> | null>(null);
  const focusRef = useRef<ReadonlySet<string> | null>(null);
  const nodeContext = useCallback(
    (next: DiagramBody = bodyRef.current, focusSet: ReadonlySet<string> | null = focusRef.current): NodeContext => ({
      focus: focusSet,
      marks: foreignKeyMarks(next),
    }),
    [],
  );
  const [initialNodes] = useState<DesignerNode[]>(() => buildNodes(initialBody, { focus: null, marks: foreignKeyMarks(initialBody) }, []));
  const [nodes, setNodes, onNodesChange] = useNodesState<DesignerNode>(initialNodes);
  const [selectedEdge, setSelectedEdge] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; target: MenuTarget } | null>(null);
  const [exporting, setExporting] = useState(false);
  const [prompt, setPrompt] = useState("");

  // ------------------------------------------------------------------- autosave
  const build = useCallback((): Document => {
    const typed = nameRef.current?.trim();
    return { ...docRef.current, name: typed !== undefined && typed !== "" ? typed : docRef.current.name, body: { kind: "diagram", data: bodyRef.current } };
  }, []);

  const publish = useCallback((saved: Document) => {
    stamp.current = saved.updatedAt;
    docRef.current = saved;
    useWorkspace.setState((s) => ({ documents: { ...s.documents, diagram: s.documents.diagram.map((d) => (d.id === saved.id ? saved : d)) } }));
  }, []);

  const autosave = useAutosave({
    build,
    onSaved: (saved) => {
      publish(saved);
      // The draft goes once the store holds exactly what is in the field. A draft with a
      // trailing space ("my ") stays, or the space would vanish under the user's cursor.
      if (nameRef.current !== null && saved.name === nameRef.current) {
        nameRef.current = null;
        setNameDraft(null);
      }
    },
    onError: showError,
  });
  const autosaveRef = useRef(autosave);
  useEffect(() => {
    autosaveRef.current = autosave;
  });

  // --------------------------------------------------------------------- editing
  const replaceBody = useCallback(
    (next: DiagramBody) => {
      bodyRef.current = next;
      setBody(next);
      setNodes((previous) => buildNodes(next, nodeContext(next), previous));
    },
    [nodeContext, setNodes],
  );

  const edit = useCallback<Edit>(
    (mutator) => {
      const next = mutator(bodyRef.current);
      if (next === bodyRef.current) return;
      replaceBody(next);
      autosaveRef.current.touch();
    },
    [replaceBody],
  );

  // Someone else saved this diagram (a restored version, the AI assistant): show it,
  // unless there are edits here still waiting to be saved, which win.
  useEffect(
    () =>
      useWorkspace.subscribe((state) => {
        const latest = state.documents.diagram.find((d) => d.id === doc.id);
        if (!latest || latest.updatedAt === stamp.current) return;
        stamp.current = latest.updatedAt;
        docRef.current = latest;
        if (autosaveRef.current.state === "dirty" || autosaveRef.current.state === "saving") return;
        const incoming = bodyOf(latest);
        if (JSON.stringify(incoming) !== JSON.stringify(bodyRef.current)) replaceBody(incoming);
      }),
    [doc.id, replaceBody],
  );

  const selectOnly = useCallback(
    (ids: ReadonlySet<string>) => {
      setNodes((previous) => previous.map((n) => ({ ...n, selected: ids.has(n.id) })));
    },
    [setNodes],
  );

  const selectEdge = useCallback(
    (id: string | null) => {
      setSelectedEdge(id);
      if (id !== null) selectOnly(new Set());
    },
    [selectOnly],
  );

  const clearSelection = useCallback(() => {
    setSelectedEdge(null);
    selectOnly(new Set());
  }, [selectOnly]);

  const applyFocus = useCallback(
    (next: ReadonlySet<string> | null) => {
      setFocus(next);
      focusRef.current = next;
      setNodes((previous) => buildNodes(bodyRef.current, nodeContext(bodyRef.current, next), previous));
    },
    [nodeContext, setNodes],
  );

  const grid = gridOf(body);
  const notesShown = notesVisibleOf(body);

  const centre = useCallback((): Point => {
    const rect = wrapper.current?.getBoundingClientRect();
    return rect ? screenToFlowPosition({ x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 }) : { x: 120, y: 120 };
  }, [screenToFlowPosition]);

  const addTable = () => {
    const jitter = (bodyRef.current.tables.length % 6) * 24;
    const middle = centre();
    const table = newTable(bodyRef.current, catalog, {
      x: snap(middle.x - TABLE_WIDTH / 2 + jitter, grid),
      y: snap(middle.y - 60 + jitter, grid),
    });
    edit((b) => ({ ...b, tables: [...b.tables, table] }));
    setSelectedEdge(null);
    selectOnly(new Set([table.id]));
  };

  const addNote = () => {
    const middle = centre();
    const note = newNote({ x: snap(middle.x - 110 + notesOf(bodyRef.current).length * 20, grid), y: snap(middle.y - 60 + notesOf(bodyRef.current).length * 20, grid) });
    edit((b) => ({ ...b, notes: [...notesOf(b), note], notesVisible: true }));
    setSelectedEdge(null);
    selectOnly(new Set([note.id]));
  };

  const layout = (direction: LayoutDirection) => {
    edit((b) => applyLayout(b, direction));
    window.setTimeout(() => {
      void fitView({ duration: 300, padding: 0.12 });
    }, 60);
  };

  const onConnect = (connection: Connection) => {
    const current = bodyRef.current;
    const from = current.tables.find((t) => t.id === connection.source);
    const to = current.tables.find((t) => t.id === connection.target);
    if (!from || !to) return;
    const fromIndex = columnOfHandle(connection.sourceHandle);
    const toIndex = columnOfHandle(connection.targetHandle);
    const fromColumn = fromIndex === null ? "" : (from.columns[fromIndex]?.name ?? "");
    let toColumn = toIndex === null ? "" : (to.columns[toIndex]?.name ?? "");
    // A key usually points at the other table's primary key: fill it in when there is just one.
    if (toColumn === "") {
      const keys = to.columns.filter((c) => c.primaryKey);
      const only = keys[0];
      if (keys.length === 1 && only) toColumn = only.name;
    }
    const relation = newRelation(from.id, fromColumn, to.id, toColumn, { from: sideOfHandle(connection.sourceHandle), to: sideOfHandle(connection.targetHandle) });
    edit((b) => ({ ...b, relations: [...b.relations, relation] }));
    selectEdge(relation.id);
  };

  const deleteSelection = useCallback(
    (tableIds: ReadonlySet<string>, noteIds: ReadonlySet<string>) => {
      edit((b) => removeNotes(removeTables(b, tableIds), noteIds));
    },
    [edit],
  );

  const duplicate = (ids: ReadonlySet<string>) => {
    const result = duplicateTables(bodyRef.current, ids);
    if (result.copies.length === 0) return;
    edit(() => result.body);
    selectOnly(new Set(result.copies));
  };

  // --------------------------------------------------------------- what is selected
  const selectedTables = useMemo(() => nodes.filter((n) => n.selected === true && n.type === "table").map((n) => n.id), [nodes]);
  const selectedNotes = useMemo(() => nodes.filter((n) => n.selected === true && n.type === "note").map((n) => n.id), [nodes]);
  const table = selectedEdge === null && selectedTables.length === 1 && selectedNotes.length === 0 ? body.tables.find((t) => t.id === selectedTables[0]) : undefined;
  const note = selectedEdge === null && selectedNotes.length === 1 && selectedTables.length === 0 ? notesOf(body).find((n) => n.id === selectedNotes[0]) : undefined;
  const relation = selectedEdge === null ? undefined : body.relations.find((r) => r.id === selectedEdge);

  const edges = useMemo(() => buildEdges(body, nodes, selectedEdge, focus), [body, nodes, selectedEdge, focus]);

  // What a relation line and a note can ask the canvas to do (see RelationEdge, NoteNode).
  const edgeActions = useMemo<EdgeActions>(
    () => ({
      setBends: (id, bends) => {
        edit((b) => updateRelation(b, id, { waypoints: bends }));
      },
      select: selectEdge,
    }),
    [edit, selectEdge],
  );
  const noteActions = useMemo<NoteActions>(
    () => ({
      change: (id, patch) => {
        edit((b) => updateNote(b, id, patch));
      },
    }),
    [edit],
  );

  // ---------------------------------------------------------------------- export
  const runExport = async (format: ExportFormat, selectionOnly: boolean) => {
    setExporting(true);
    try {
      const whole = toExportModel(bodyRef.current);
      const model = selectionOnly ? restrict(whole, new Set(selectedTables), new Set(selectedNotes)) : whole;
      const typed = nameRef.current?.trim();
      const name = typed !== undefined && typed !== "" ? typed : docRef.current.name;
      const written = await exportDiagram({ model, format, name: selectionOnly ? `${name}-selection` : name, dark: colorMode === "dark" });
      if (written !== null) showInfo(`Diagram exported to ${written}.`);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setExporting(false);
    }
  };

  const hasSelection = selectedTables.length + selectedNotes.length > 0;
  const exportItems: ToolbarMenuItem[] = [
    { id: "png", label: "PNG image", icon: "download", section: "Whole diagram", disabled: isEmpty(body) },
    { id: "svg", label: "SVG image", icon: "download", disabled: isEmpty(body) },
    { id: "png-selection", label: "PNG image", icon: "download", section: "Selection", disabled: !hasSelection },
    { id: "svg-selection", label: "SVG image", icon: "download", disabled: !hasSelection },
  ];

  const gridItems: ToolbarMenuItem[] = GRID_SIZES.map((size, i) => ({
    id: String(size),
    label: size === 0 ? "Off" : `${size}px`,
    selected: size === grid,
    ...(i === 0 ? { section: "Snap to grid" } : {}),
  }));

  const onNodeContextMenu: NodeMouseHandler<DesignerNode> = (event, node) => {
    event.preventDefault();
    const sameKind = (nodes.filter((n) => n.selected === true && n.type === node.type).map((n) => n.id));
    const ids = sameKind.includes(node.id) ? sameKind : [node.id];
    const title = node.type === "note" ? "Note" : ids.length > 1 ? `${ids.length} tables` : node.data.title;
    setMenu({ x: event.clientX, y: event.clientY, target: { kind: node.type === "note" ? "note" : "table", ids, title } });
  };

  const onEdgeContextMenu: EdgeMouseHandler<RelationFlowEdge> = (event, edge) => {
    event.preventDefault();
    const rel = bodyRef.current.relations.find((r) => r.id === edge.id);
    const names = new Map(bodyRef.current.tables.map((t) => [t.id, t.name === "" ? "untitled" : t.name]));
    const title = rel ? `${names.get(rel.fromTable) ?? "?"} → ${names.get(rel.toTable) ?? "?"}` : "Relation";
    setMenu({ x: event.clientX, y: event.clientY, target: { kind: "relation", ids: [edge.id], title } });
  };

  const menuColor = (target: MenuTarget): ConnectionColor | null => {
    const first = target.ids[0];
    if (target.kind !== "table" || target.ids.length !== 1 || first === undefined) return null;
    return body.tables.find((t) => t.id === first)?.color ?? null;
  };

  const askAi = () => {
    const text = prompt.trim();
    if (text === "" || connectionId === null) return;
    // The document id tells the assistant to fill this open diagram instead of creating a second one.
    openChat(connectionId, `Create a schema diagram: ${text} (document id ${doc.id})`);
    setPrompt("");
  };
  const canAskAi = connectionId !== null;

  const typedName = nameDraft ?? doc.name;
  const status =
    autosave.state === "saving"
      ? "Saving…"
      : autosave.state === "dirty"
        ? "Unsaved changes"
        : autosave.state === "error"
          ? "Not saved"
          : autosave.savedAt
            ? `Saved ${autosave.savedAt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
            : "Saved";

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      <div className="flex app-toolbar shrink-0 items-center gap-1 border-b border-border bg-surface">
        <Input
          value={typedName}
          onChange={(event) => {
            setNameDraft(event.target.value);
            nameRef.current = event.target.value;
            autosave.touch();
          }}
          className="h-6 w-48 rounded-md bg-transparent text-xs text-foreground"
          aria-label="Diagram name"
        />
        <IconButton icon="plus" label="Add table" onClick={addTable} />
        <IconButton icon="note" label="Add sticky note" onClick={addNote} />
        <span className="mx-1 h-5 w-px bg-separator" />
        <ToolbarMenu
          icon="sort"
          label="Auto layout"
          items={LAYOUT_ITEMS}
          onSelect={(id) => {
            if (isLayoutDirection(id)) layout(id);
          }}
        />
        <ToolbarMenu
          icon="grid"
          label="Snap to grid"
          items={gridItems}
          active={grid > 0}
          onSelect={(id) => {
            edit((b) => ({ ...b, gridSize: Number(id) }));
          }}
        />
        <IconButton
          icon={notesShown ? "eye" : "eye-off"}
          label={notesShown ? "Hide notes" : "Show notes"}
          active={!notesShown}
          onClick={() => {
            edit((b) => ({ ...b, notesVisible: !notesVisibleOf(b) }));
          }}
        />
        {focus ? (
          <Button
            size="sm"
            variant="soft"
            onClick={() => {
              applyFocus(null);
            }}
          >
            <Icon name="focus" size={13} />
            Clear focus
          </Button>
        ) : null}
        <span className="mx-1 h-5 w-px bg-separator" />
        <span className="text-xs text-muted">{describe(body)}</span>
        <div className="ml-auto flex items-center gap-1">
          <span className={cn("mr-1 text-[11px]", autosave.state === "error" ? "text-danger" : "text-muted")} aria-live="polite">
            {status}
          </span>
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
          <VersionsMenu
            documentId={doc.id}
            settle={() => autosave.flush({ version: true })}
            current={build}
            onRestored={(restored) => {
              publish(restored);
              nameRef.current = null;
              setNameDraft(null);
              replaceBody(bodyOf(restored));
              clearSelection();
            }}
          />
          <SqlDialog connectionId={connectionId} body={body} name={doc.name}>
            <Button size="sm" variant="ghost" className="text-muted">
              <Icon name="terminal" size={13} />
              SQL
            </Button>
          </SqlDialog>
        </div>
      </div>

      <div className="flex min-h-0 flex-1">
        <div ref={wrapper} className="relative min-w-0 flex-1">
          <EdgeActionsContext value={edgeActions}>
          <NoteActionsContext value={noteActions}>
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onConnect={onConnect}
            onNodeDragStop={(_, __, dragged) => {
              const moved = new Map(dragged.map((n) => [n.id, { x: Math.round(n.position.x), y: Math.round(n.position.y) }]));
              edit((b) => ({
                ...b,
                tables: b.tables.map((t) => {
                  const at = moved.get(t.id);
                  return at ? { ...t, x: at.x, y: at.y } : t;
                }),
                notes: notesOf(b).map((n) => {
                  const at = moved.get(n.id);
                  return at ? { ...n, x: at.x, y: at.y } : n;
                }),
              }));
            }}
            onNodesDelete={(deleted) => {
              deleteSelection(new Set(deleted.filter((n) => n.type === "table").map((n) => n.id)), new Set(deleted.filter((n) => n.type === "note").map((n) => n.id)));
            }}
            onEdgesDelete={(deleted) => {
              edit((b) => removeRelations(b, new Set(deleted.map((e) => e.id))));
              setSelectedEdge(null);
            }}
            onNodeClick={() => {
              setSelectedEdge(null);
            }}
            onEdgeClick={(_, edge) => {
              selectEdge(edge.id);
            }}
            onPaneClick={() => {
              clearSelection();
              setMenu(null);
            }}
            onNodeContextMenu={onNodeContextMenu}
            onEdgeContextMenu={onEdgeContextMenu}
            onBeforeDelete={confirmKeyboardDelete}
            nodeTypes={NODE_TYPES}
            edgeTypes={EDGE_TYPES}
            connectionMode={ConnectionMode.Loose}
            deleteKeyCode={DELETE_KEYS}
            snapToGrid={grid > 0}
            snapGrid={[Math.max(grid, 1), Math.max(grid, 1)]}
            minZoom={0.1}
            maxZoom={2}
            fitView
            colorMode={colorMode}
            proOptions={{ hideAttribution: true }}
          >
            <Background variant={BackgroundVariant.Dots} gap={grid > 0 ? grid : 24} size={1} />
            <Controls showInteractive={false} />
            <MiniMap pannable zoomable className="!bg-surface" nodeColor="var(--accent)" maskColor="var(--backdrop)" />
            {selectedTables.length >= 2 ? (
              <Panel position="top-center" className="flex items-center gap-2 rounded-lg border border-border bg-surface/95 px-2 py-1 shadow-md backdrop-blur">
                <span className="text-xs font-medium text-muted">{selectedTables.length} tables</span>
                <span className="h-4 w-px bg-separator" />
                <ColorSwatches
                  value={null}
                  resetLabel="Accent"
                  onChange={(color) => {
                    edit((b) => setColor(b, new Set(selectedTables), color));
                  }}
                />
                <span className="h-4 w-px bg-separator" />
                <IconButton
                  icon="focus"
                  label={focus ? "Clear focus" : "Focus selected (dim the rest)"}
                  active={focus !== null}
                  onClick={() => {
                    applyFocus(focus ? null : new Set(selectedTables));
                  }}
                />
                <IconButton
                  icon="copy"
                  label="Duplicate selected"
                  onClick={() => {
                    duplicate(new Set(selectedTables));
                  }}
                />
                <IconButton
                  icon="trash"
                  label="Delete selected"
                  onClick={() => {
                    deleteSelection(new Set(selectedTables), new Set(selectedNotes));
                  }}
                />
              </Panel>
            ) : null}
            {selectedEdge !== null ? (
              <Panel position="bottom-center" className="rounded-md border border-border bg-surface/90 px-2 py-1 text-[11px] text-muted backdrop-blur">
                Double-click the line to add a bend. Drag a bend to move it; double-click it to remove it.
              </Panel>
            ) : null}
          </ReactFlow>
          </NoteActionsContext>
          </EdgeActionsContext>
          {isEmpty(body) ? (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-3">
              <div className="pointer-events-auto flex w-[360px] max-w-full flex-col gap-2 rounded-lg border border-border bg-surface/95 p-3 shadow-md backdrop-blur">
                <p className="text-[13px] font-medium text-foreground">Start your schema</p>
                <p className="text-xs leading-relaxed text-muted">Add a table, then drag from the edge of one table to another to relate them.</p>
                <Button size="sm" variant="secondary" className="self-start" onClick={addTable}>
                  <Icon name="plus" size={13} />
                  Add a table
                </Button>
                {canAskAi ? (
                  <div className="flex flex-col gap-2 border-t border-border pt-2">
                    <p className="text-xs text-muted">Or describe it and let the assistant design it.</p>
                    <Textarea
                      value={prompt}
                      rows={2}
                      placeholder="e.g. a shop with customers, orders and products"
                      aria-label="Describe the schema you want"
                      className="font-sans text-xs"
                      onChange={(event) => {
                        setPrompt(event.target.value);
                      }}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" && !event.shiftKey) {
                          event.preventDefault();
                          askAi();
                        }
                      }}
                    />
                    <Button size="sm" className="self-start" disabled={prompt.trim() === ""} onClick={askAi}>
                      <Icon name="sparkles" size={13} />
                      Ask AI to design this
                    </Button>
                  </div>
                ) : null}
              </div>
            </div>
          ) : null}
        </div>

        {relation ? (
          <RelationInspector relation={relation} body={body} edit={edit} onClose={clearSelection} />
        ) : table ? (
          <TableInspector
            key={table.id}
            table={table}
            body={body}
            catalog={catalog}
            connectionId={connectionId}
            diagramName={typedName}
            edit={edit}
            onSelectRelation={selectEdge}
            onDuplicate={() => {
              duplicate(new Set([table.id]));
            }}
            onDelete={() => {
              deleteSelection(new Set([table.id]), new Set());
            }}
            onClose={clearSelection}
          />
        ) : note ? (
          <NoteInspector key={note.id} note={note} edit={edit} onClose={clearSelection} />
        ) : null}
      </div>

      {menu ? (
        <NodeMenu
          at={menu}
          title={menu.target.title}
          onClose={() => {
            setMenu(null);
          }}
          {...(menu.target.kind === "table"
            ? {
                color: {
                  value: menuColor(menu.target),
                  resetLabel: "Accent",
                  onChange: (color: ConnectionColor | null) => {
                    edit((b) => setColor(b, new Set(menu.target.ids), color));
                    setMenu(null);
                  },
                },
              }
            : {})}
          entries={
            menu.target.kind === "table"
              ? [
                  { id: "select", label: menu.target.ids.length > 1 ? "Select these tables" : "Edit table", icon: "pencil" },
                  { id: "duplicate", label: "Duplicate", icon: "copy" },
                  { id: "delete", label: "Delete", icon: "trash", danger: true },
                ]
              : menu.target.kind === "relation"
                ? [
                    { id: "select", label: "Edit relation", icon: "pencil" },
                    { id: "delete", label: "Delete relation", icon: "trash", danger: true },
                  ]
                : [{ id: "delete", label: "Delete note", icon: "trash", danger: true }]
          }
          onAction={(id) => {
            const ids = new Set(menu.target.ids);
            const kind = menu.target.kind;
            if (kind === "relation") {
              if (id === "select") selectEdge(menu.target.ids[0] ?? null);
              else if (id === "delete") {
                edit((b) => removeRelations(b, ids));
                setSelectedEdge(null);
              }
            } else if (id === "select") {
              setSelectedEdge(null);
              selectOnly(ids);
            } else if (id === "duplicate") {
              duplicate(ids);
            } else if (id === "delete") {
              deleteSelection(kind === "table" ? ids : new Set(), kind === "note" ? ids : new Set());
            }
          }}
        />
      ) : null}
    </div>
  );
}
