// SOT: designer-nodes, designer-edges, diagram-to-react-flow
import type { DiagramBody } from "@/lib/bindings";
import { typeIcon } from "@/lib/icons";
import type { NoteFlowNode } from "./NoteNode";
import type { RelationFlowEdge } from "./RelationEdge";
import type { TableFlowNode } from "./TableNode";
import { isComplete, linksOf, notesOf, notesVisibleOf, relationLabel, tableBox } from "./designerModel";
import { resolveSides, type Box } from "./metrics";
import { DEFAULT_LINE_COLOR, colorVar, lineWidthPx } from "./palette";

// WHAT:  Turns the diagram body into the nodes and edges React Flow draws.
// WHY:   The body is the source of truth; React Flow's copy only adds what the
//        canvas needs (selection, measured sizes, a card mid-drag). Building both
//        from the body in one place keeps the canvas a pure view of it.
// HOW:   Nodes are rebuilt on every edit but keep what React Flow learned about
//        them — whether they are selected and how big they measured — otherwise a
//        rebuild would deselect everything and blank the cards until re-measured.
//        Edges are derived afresh each render from the nodes' live positions, so a
//        relation re-routes while a card is dragged.
// WHERE: src/features/diagrams/DesignerTab.tsx

export type DesignerNode = TableFlowNode | NoteFlowNode;

export interface NodeContext {
  /// Tables to keep at full strength; null = no focus mode.
  focus: ReadonlySet<string> | null;
  /// Per table, the columns that are foreign keys and what they point at.
  marks: ReadonlyMap<string, ReadonlyMap<string, string>>;
}

export function buildNodes(body: DiagramBody, ctx: NodeContext, previous: readonly DesignerNode[]): DesignerNode[] {
  const before = new Map(previous.map((n) => [n.id, n]));
  const keep = (id: string) => {
    const old = before.get(id);
    return {
      ...(old?.measured ? { measured: old.measured } : {}),
      ...(old?.selected !== undefined ? { selected: old.selected } : {}),
    };
  };
  const notes: NoteFlowNode[] = notesVisibleOf(body)
    ? notesOf(body).map((note) => ({
        id: note.id,
        type: "note",
        position: { x: note.x, y: note.y },
        width: note.width,
        height: note.height,
        zIndex: 0,
        data: { note, editable: true },
        ...keep(note.id),
      }))
    : [];
  const tables: TableFlowNode[] = body.tables.map((table) => ({
    id: table.id,
    type: "table",
    position: { x: table.x, y: table.y },
    zIndex: 1,
    data: {
      title: table.name === "" ? "untitled" : table.name,
      schema: table.schema,
      color: table.color,
      dimmed: ctx.focus !== null && !ctx.focus.has(table.id),
      mode: "designer",
      columns: table.columns.map((c) => ({
        name: c.name,
        type: c.dataType,
        pk: c.primaryKey,
        fk: ctx.marks.get(table.id)?.get(c.name) ?? null,
        unique: c.unique,
        icon: typeIcon(c.dataType, c.primaryKey),
      })),
    },
    ...keep(table.id),
  }));
  return [...notes, ...tables];
}

export function buildEdges(
  body: DiagramBody,
  nodes: readonly DesignerNode[],
  selectedEdge: string | null,
  focus: ReadonlySet<string> | null,
): RelationFlowEdge[] {
  const at = new Map(nodes.map((n) => [n.id, n.position]));
  const boxes = new Map<string, Box>();
  for (const table of body.tables) {
    const position = at.get(table.id);
    boxes.set(table.id, position ? { ...tableBox(table), x: position.x, y: position.y } : tableBox(table));
  }
  return body.relations.flatMap((rel) => {
    const from = boxes.get(rel.fromTable);
    const to = boxes.get(rel.toTable);
    if (!from || !to) return [];
    const sides = resolveSides(rel.fromSide, rel.toSide, from, to, rel.fromTable === rel.toTable);
    const active = selectedEdge === rel.id;
    const inFocus = focus === null || (focus.has(rel.fromTable) && focus.has(rel.toTable));
    const edge: RelationFlowEdge = {
      id: rel.id,
      type: "relation",
      source: rel.fromTable,
      target: rel.toTable,
      sourceHandle: sides.fromSide,
      targetHandle: sides.toSide,
      selected: active,
      animated: active,
      data: {
        label: linksOf(rel).some((l) => l.from !== "" || l.to !== "") ? relationLabel(rel) : "",
        color: rel.color === null ? DEFAULT_LINE_COLOR : colorVar(rel.color),
        width: lineWidthPx(rel.width),
        dashed: !isComplete(rel),
        dimmed: !inFocus || (selectedEdge !== null && !active),
        bends: rel.waypoints.map((w) => ({ x: w.x, y: w.y })),
        editable: true,
      },
    };
    return [edge];
  });
}
