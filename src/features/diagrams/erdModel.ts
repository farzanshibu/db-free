// SOT: erd-model, erd-graph, foreign-key-graph-build, edit-in-designer-conversion
import type { ColumnInfo, ConnectionColor, DiagramBody, DiagramRelation, DiagramTable, ForeignKey, TableRef } from "@/lib/bindings";
import { tableKey } from "@/stores/workspace";
import { autoLayout } from "./layout";
import { MAX_ERD_ROWS, TABLE_WIDTH, tableHeight, type Point } from "./metrics";
import { newColumn, newId } from "./designerModel";

// WHAT:  The ER diagram's graph — tables with their columns, and the foreign keys
//        between them — and the rules for placing it and converting it to a
//        designed diagram.
// WHY:   The canvas, the layout memory and "Edit in designer" all need the same
//        graph and the same stable keys for its parts; building it once, free of
//        React Flow, keeps them in step.
// WHERE: src/features/diagrams/ErdTab.tsx, designerModel.ts

export interface ErdTable {
  /// `schema.table`: the stable id of the card and the key its layout is stored under.
  key: string;
  ref: TableRef;
  columns: ColumnInfo[];
}

export interface ErdRelation {
  /// `from->to:constraint`: the id of the line and the key its bend points are stored under.
  key: string;
  source: string;
  target: string;
  fk: ForeignKey;
}

export interface ErdGraph {
  tables: ErdTable[];
  relations: ErdRelation[];
}

// WHAT:  Joins tables, their columns and the foreign keys between them. A key whose
//        other end is outside the diagram (another schema, a table past the cap) is dropped.
export function buildErdGraph(refs: readonly TableRef[], columns: readonly (readonly ColumnInfo[])[], fks: readonly ForeignKey[]): ErdGraph {
  const tables: ErdTable[] = refs.map((ref, i) => ({ key: tableKey(ref), ref, columns: [...(columns[i] ?? [])] }));
  const known = new Set(tables.map((t) => t.key));
  const relations: ErdRelation[] = [];
  const seen = new Set<string>();
  for (const fk of fks) {
    const source = tableKey({ schema: fk.fromSchema, name: fk.fromTable });
    const target = tableKey({ schema: fk.toSchema, name: fk.toTable });
    if (!known.has(source) || !known.has(target)) continue;
    const key = `${source}->${target}:${fk.name}`;
    if (seen.has(key)) continue;
    seen.add(key);
    relations.push({ key, source, target, fk });
  }
  return { tables, relations };
}

/// For each table, the columns that are foreign keys and what they point at.
export function erdForeignKeys(graph: ErdGraph): Map<string, Map<string, string>> {
  const marks = new Map<string, Map<string, string>>();
  for (const rel of graph.relations) {
    const perTable = marks.get(rel.source) ?? new Map<string, string>();
    rel.fk.fromColumns.forEach((column, i) => {
      perTable.set(column, `→ ${rel.fk.toTable}.${rel.fk.toColumns[i] ?? ""}`);
    });
    marks.set(rel.source, perTable);
  }
  return marks;
}

export function cardSize(table: ErdTable): { width: number; height: number } {
  return { width: TABLE_WIDTH, height: tableHeight(table.columns.length, MAX_ERD_ROWS) };
}

// WHAT:  Where each card starts: the remembered position when there is one, and for
//        a table seen for the first time, the auto-layout — placed below the
//        remembered ones so a new table never lands on top of an old one.
export function initialPositions(graph: ErdGraph, saved: ReadonlyMap<string, Point>): Map<string, Point> {
  const positions = new Map<string, Point>();
  const fresh = graph.tables.filter((t) => !saved.has(t.key));
  for (const t of graph.tables) {
    const at = saved.get(t.key);
    if (at) positions.set(t.key, at);
  }
  if (fresh.length === 0) return positions;
  const freshKeys = new Set(fresh.map((t) => t.key));
  const placed = autoLayout(
    fresh.map((t) => ({ id: t.key, ...cardSize(t) })),
    graph.relations.filter((r) => freshKeys.has(r.source) && freshKeys.has(r.target)).map((r) => ({ source: r.source, target: r.target })),
    "TB",
  );
  let origin: Point = { x: 0, y: 0 };
  const remembered = graph.tables.filter((t) => saved.has(t.key));
  if (remembered.length > 0) {
    const xs = remembered.map((t) => saved.get(t.key)?.x ?? 0);
    const bottoms = remembered.map((t) => (saved.get(t.key)?.y ?? 0) + cardSize(t).height);
    origin = { x: Math.min(...xs), y: Math.max(...bottoms) + 80 };
  }
  for (const t of fresh) {
    const at = placed.get(t.key) ?? { x: 0, y: 0 };
    positions.set(t.key, { x: origin.x + at.x, y: origin.y + at.y });
  }
  return positions;
}

// WHAT:  The ER diagram as an editable `DiagramBody`: the same tables where they
//        are on screen, in the colours they were given, with the same bend points.
// WHY:   "Edit in designer" saves a copy the user can change freely; the ER diagram
//        itself stays derived from the database.
export function erdToDiagram(
  graph: ErdGraph,
  positions: ReadonlyMap<string, Point>,
  colors: ReadonlyMap<string, ConnectionColor>,
  bends: ReadonlyMap<string, readonly Point[]>,
): DiagramBody {
  const ids = new Map(graph.tables.map((t) => [t.key, newId("tbl")]));
  const tables: DiagramTable[] = graph.tables.map((t) => ({
    id: ids.get(t.key) ?? newId("tbl"),
    name: t.ref.name,
    schema: t.ref.schema,
    x: Math.round(positions.get(t.key)?.x ?? 0),
    y: Math.round(positions.get(t.key)?.y ?? 0),
    color: colors.get(t.key) ?? null,
    comment: null,
    columns: t.columns.map((c) => newColumn(c.name, c.dataType, { primaryKey: c.primaryKey, nullable: c.nullable })),
  }));
  const relations: DiagramRelation[] = graph.relations.flatMap((rel) => {
    const from = ids.get(rel.source);
    const to = ids.get(rel.target);
    const pairs = rel.fk.fromColumns.slice(0, rel.fk.toColumns.length).map((column, i) => ({ fromColumn: column, toColumn: rel.fk.toColumns[i] ?? "" }));
    const [first, ...rest] = pairs;
    if (from === undefined || to === undefined) return [];
    return [
      {
        id: newId("rel"),
        fromTable: from,
        fromColumn: first?.fromColumn ?? "",
        toTable: to,
        toColumn: first?.toColumn ?? "",
        extraColumns: rest,
        name: rel.fk.name,
        onDelete: null,
        onUpdate: null,
        color: null,
        width: null,
        fromSide: null,
        toSide: null,
        waypoints: (bends.get(rel.key) ?? []).map((b) => ({ x: b.x, y: b.y })),
      },
    ];
  });
  return { tables, relations };
}
