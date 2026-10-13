// SOT: designer-model, diagram-body-operations, schema-designer-edits, diagram-relations-model
import type {
  ConnectionColor,
  DiagramBody,
  DiagramColumn,
  DiagramColumnPair,
  DiagramNote,
  DiagramRelation,
  DiagramSide,
  DiagramTable,
  Document,
} from "@/lib/bindings";
import { tableHeight, TABLE_WIDTH, type Point } from "./metrics";
import { autoLayout, type LayoutDirection } from "./layout";
import type { TypeCatalog } from "./columnTypes";
import type { ExportModel } from "./exportSvg";
import { DEFAULT_NOTE_COLOR, DEFAULT_NOTE_FONT_SIZE, lineWidthPx } from "./palette";

// WHAT:  Every edit the schema designer makes to a diagram, as a pure function from
//        one `DiagramBody` to the next.
// WHY:   The document body is the source of truth and React Flow's state is
//        derived from it. Keeping the edits here, free of React, is what lets one
//        rule (a renamed column follows into the relations that use it; a deleted
//        table takes its relations with it) hold for every way of making the edit:
//        the inspector, the keyboard, the context menu, a bulk action.
// WHERE: src/features/diagrams/DesignerTab.tsx, TableInspector.tsx, RelationInspector.tsx

export const GRID_SIZES: readonly number[] = [0, 8, 16, 24, 32];
export const DEFAULT_GRID = 16;
export const NOTE_WIDTH = 220;
export const NOTE_HEIGHT = 120;

export function emptyBody(): DiagramBody {
  return { tables: [], relations: [] };
}

export function bodyOf(doc: Document): DiagramBody {
  return doc.body.kind === "diagram" ? doc.body.data : emptyBody();
}

export function gridOf(body: DiagramBody): number {
  return body.gridSize ?? DEFAULT_GRID;
}

export function notesOf(body: DiagramBody): readonly DiagramNote[] {
  return body.notes ?? [];
}

export function notesVisibleOf(body: DiagramBody): boolean {
  return body.notesVisible ?? true;
}

export function newId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID().slice(0, 8)}`;
}

// ------------------------------------------------------------------ factories

export function newColumn(name: string, dataType: string, patch: Partial<DiagramColumn> = {}): DiagramColumn {
  return { name, dataType, primaryKey: false, nullable: true, unique: false, autoIncrement: false, defaultValue: null, comment: null, ...patch };
}

export function newTable(body: DiagramBody, catalog: TypeCatalog, at: Point): DiagramTable {
  return {
    id: newId("tbl"),
    name: uniqueName(body.tables.map((t) => t.name), "table"),
    x: at.x,
    y: at.y,
    schema: null,
    color: null,
    comment: null,
    columns: [newColumn("id", catalog.idType, { primaryKey: true, nullable: false, autoIncrement: catalog.autoIncrement })],
  };
}

export function newRelation(fromTable: string, fromColumn: string, toTable: string, toColumn: string, sides: { from: DiagramSide | null; to: DiagramSide | null }): DiagramRelation {
  return {
    id: newId("rel"),
    fromTable,
    fromColumn,
    toTable,
    toColumn,
    extraColumns: [],
    name: null,
    onDelete: null,
    onUpdate: null,
    color: null,
    width: null,
    fromSide: sides.from,
    toSide: sides.to,
    waypoints: [],
  };
}

export function newNote(at: Point): DiagramNote {
  return { id: newId("note"), x: at.x, y: at.y, width: NOTE_WIDTH, height: NOTE_HEIGHT, text: "", color: null, fontSize: null, hideOnExport: false };
}

/// `table`, `table_2`, `table_3`… — the first stem not already taken.
export function uniqueName(taken: readonly string[], stem: string): string {
  const lower = new Set(taken.map((n) => n.toLowerCase()));
  if (!lower.has(stem.toLowerCase())) return stem;
  for (let n = 2; ; n += 1) {
    const candidate = `${stem}_${n}`;
    if (!lower.has(candidate.toLowerCase())) return candidate;
  }
}

// ------------------------------------------------------------------ relations

export interface ColumnLink {
  from: string;
  to: string;
}

export function linksOf(rel: DiagramRelation): ColumnLink[] {
  return [{ from: rel.fromColumn, to: rel.toColumn }, ...rel.extraColumns.map((p) => ({ from: p.fromColumn, to: p.toColumn }))];
}

// WHAT:  Writes a list of column pairs back as the first pair plus the extras.
export function withLinks(rel: DiagramRelation, links: readonly ColumnLink[]): DiagramRelation {
  const [first, ...rest] = links;
  const extra: DiagramColumnPair[] = rest.map((l) => ({ fromColumn: l.from, toColumn: l.to }));
  return { ...rel, fromColumn: first?.from ?? "", toColumn: first?.to ?? "", extraColumns: extra };
}

export function setRelationLinks(body: DiagramBody, id: string, links: readonly ColumnLink[]): DiagramBody {
  return { ...body, relations: body.relations.map((r) => (r.id === id ? withLinks(r, links) : r)) };
}

// WHAT:  Reverses a relation: the referencing table becomes the referenced one.
export function swapRelation(body: DiagramBody, id: string): DiagramBody {
  return {
    ...body,
    relations: body.relations.map((r) =>
      r.id === id
        ? {
            ...withLinks(
              r,
              linksOf(r).map((l) => ({ from: l.to, to: l.from })),
            ),
            fromTable: r.toTable,
            toTable: r.fromTable,
            fromSide: r.toSide,
            toSide: r.fromSide,
            waypoints: [...r.waypoints].reverse(),
          }
        : r,
    ),
  };
}

export function isComplete(rel: DiagramRelation): boolean {
  return linksOf(rel).every((l) => l.from.trim() !== "" && l.to.trim() !== "");
}

/// "user_id → id", "a, b → x, y", or an em dash while no columns are chosen.
export function relationLabel(rel: DiagramRelation): string {
  const links = linksOf(rel).filter((l) => l.from !== "" || l.to !== "");
  if (links.length === 0) return "—";
  return `${links.map((l) => l.from || "?").join(", ")} → ${links.map((l) => l.to || "?").join(", ")}`;
}

// WHAT:  For each table, the columns that are the owning side of a relation, with
//        what they point at ("→ users.id"). The card marks them FK and the
//        inspector shows the target.
export function foreignKeyMarks(body: DiagramBody): Map<string, Map<string, string>> {
  const names = new Map(body.tables.map((t) => [t.id, t.name]));
  const marks = new Map<string, Map<string, string>>();
  for (const rel of body.relations) {
    for (const link of linksOf(rel)) {
      if (link.from === "") continue;
      const perTable = marks.get(rel.fromTable) ?? new Map<string, string>();
      perTable.set(link.from, `→ ${names.get(rel.toTable) ?? "?"}${link.to === "" ? "" : `.${link.to}`}`);
      marks.set(rel.fromTable, perTable);
    }
  }
  return marks;
}

// ----------------------------------------------------------------- table edits

export function updateTable(body: DiagramBody, id: string, patch: Partial<DiagramTable>): DiagramBody {
  return { ...body, tables: body.tables.map((t) => (t.id === id ? { ...t, ...patch } : t)) };
}

export function updateRelation(body: DiagramBody, id: string, patch: Partial<DiagramRelation>): DiagramBody {
  return { ...body, relations: body.relations.map((r) => (r.id === id ? { ...r, ...patch } : r)) };
}

export function updateNote(body: DiagramBody, id: string, patch: Partial<DiagramNote>): DiagramBody {
  return { ...body, notes: notesOf(body).map((n) => (n.id === id ? { ...n, ...patch } : n)) };
}

export function updateColumn(body: DiagramBody, tableId: string, index: number, patch: Partial<DiagramColumn>): DiagramBody {
  return updateTable(body, tableId, {
    columns: (body.tables.find((t) => t.id === tableId)?.columns ?? []).map((c, i) => (i === index ? { ...c, ...patch } : c)),
  });
}

// WHAT:  Renames a column and follows it into every relation that names it, on
//        either end, so a rename never silently detaches a foreign key.
export function renameColumn(body: DiagramBody, tableId: string, index: number, name: string): DiagramBody {
  const table = body.tables.find((t) => t.id === tableId);
  const old = table?.columns[index]?.name;
  const renamed = updateColumn(body, tableId, index, { name });
  if (old === undefined || old === name) return renamed;
  const follow = (rel: DiagramRelation): DiagramRelation => {
    const links = linksOf(rel).map((l) => ({
      from: rel.fromTable === tableId && l.from === old ? name : l.from,
      to: rel.toTable === tableId && l.to === old ? name : l.to,
    }));
    return withLinks(rel, links);
  };
  return { ...renamed, relations: renamed.relations.map((r) => (r.fromTable === tableId || r.toTable === tableId ? follow(r) : r)) };
}

export function addColumn(body: DiagramBody, tableId: string, column: DiagramColumn): DiagramBody {
  return updateTable(body, tableId, { columns: [...(body.tables.find((t) => t.id === tableId)?.columns ?? []), column] });
}

export function moveColumn(body: DiagramBody, tableId: string, index: number, delta: -1 | 1): DiagramBody {
  const columns = [...(body.tables.find((t) => t.id === tableId)?.columns ?? [])];
  const target = index + delta;
  const moving = columns[index];
  const other = columns[target];
  if (moving === undefined || other === undefined) return body;
  columns[index] = other;
  columns[target] = moving;
  return updateTable(body, tableId, { columns });
}

// WHAT:  Removes a column. A relation that used it loses that pair; one left with
//        no pairs stays on the canvas with its columns blank, to be re-chosen.
export function removeColumn(body: DiagramBody, tableId: string, index: number): DiagramBody {
  const table = body.tables.find((t) => t.id === tableId);
  const gone = table?.columns[index]?.name;
  const without = updateTable(body, tableId, { columns: (table?.columns ?? []).filter((_, i) => i !== index) });
  if (gone === undefined) return without;
  const relations = without.relations.map((rel) => {
    if (rel.fromTable !== tableId && rel.toTable !== tableId) return rel;
    const kept = linksOf(rel).filter((l) => !((rel.fromTable === tableId && l.from === gone) || (rel.toTable === tableId && l.to === gone)));
    return withLinks(rel, kept);
  });
  return { ...without, relations };
}

export function removeTables(body: DiagramBody, ids: ReadonlySet<string>): DiagramBody {
  return {
    ...body,
    tables: body.tables.filter((t) => !ids.has(t.id)),
    relations: body.relations.filter((r) => !ids.has(r.fromTable) && !ids.has(r.toTable)),
  };
}

export function removeRelations(body: DiagramBody, ids: ReadonlySet<string>): DiagramBody {
  return { ...body, relations: body.relations.filter((r) => !ids.has(r.id)) };
}

export function removeNotes(body: DiagramBody, ids: ReadonlySet<string>): DiagramBody {
  return { ...body, notes: notesOf(body).filter((n) => !ids.has(n.id)) };
}

export function setColor(body: DiagramBody, ids: ReadonlySet<string>, color: ConnectionColor | null): DiagramBody {
  return { ...body, tables: body.tables.map((t) => (ids.has(t.id) ? { ...t, color } : t)) };
}

// ----------------------------------------------------------------------- layout

export function tableBox(table: DiagramTable): { x: number; y: number; width: number; height: number } {
  return { x: table.x, y: table.y, width: TABLE_WIDTH, height: tableHeight(table.columns.length) };
}

// WHAT:  Moves every table to where the auto-layout puts it. Notes stay where they are.
// WHY:   Bend points are absolute coordinates; after the tables move they would float
//        in empty space, so the lines go back to their natural route.
export function applyLayout(body: DiagramBody, direction: LayoutDirection): DiagramBody {
  const placed = autoLayout(
    body.tables.map((t) => ({ id: t.id, width: TABLE_WIDTH, height: tableHeight(t.columns.length) })),
    body.relations.map((r) => ({ source: r.fromTable, target: r.toTable })),
    direction,
  );
  return {
    ...body,
    tables: body.tables.map((t) => ({ ...t, x: placed.get(t.id)?.x ?? t.x, y: placed.get(t.id)?.y ?? t.y })),
    relations: body.relations.map((r) => (r.waypoints.length === 0 ? r : { ...r, waypoints: [] })),
  };
}

// WHAT:  Copies several tables at once, and the relations that run between them,
//        so a copied group stays wired together. The copies are shifted along the
//        diagonal until the first one is clear of every existing table.
export function duplicateTables(body: DiagramBody, ids: ReadonlySet<string>): { body: DiagramBody; copies: string[] } {
  const sources = body.tables.filter((t) => ids.has(t.id));
  const first = sources[0];
  if (!first) return { body, copies: [] };
  const occupied = (x: number, y: number) => body.tables.some((t) => Math.abs(t.x - x) < 20 && Math.abs(t.y - y) < 20);
  let step = 1;
  while (occupied(first.x + 40 * step, first.y + 40 * step) && step < 100) step += 1;
  const shift = 40 * step;
  const taken = body.tables.map((t) => t.name);
  const mapped = new Map<string, string>();
  const copies: DiagramTable[] = sources.map((source) => {
    const id = newId("tbl");
    mapped.set(source.id, id);
    const name = uniqueName(taken, `${source.name}_copy`);
    taken.push(name);
    return { ...source, id, name, x: source.x + shift, y: source.y + shift, columns: source.columns.map((c) => ({ ...c })) };
  });
  const relations = body.relations
    .filter((r) => mapped.has(r.fromTable) && mapped.has(r.toTable))
    .map((r) => ({
      ...r,
      id: newId("rel"),
      fromTable: mapped.get(r.fromTable) ?? r.fromTable,
      toTable: mapped.get(r.toTable) ?? r.toTable,
      extraColumns: r.extraColumns.map((p) => ({ ...p })),
      waypoints: r.waypoints.map((w) => ({ x: w.x + shift, y: w.y + shift })),
    }));
  return { body: { ...body, tables: [...body.tables, ...copies], relations: [...body.relations, ...relations] }, copies: copies.map((c) => c.id) };
}

// WHAT:  The diagram as the image export draws it. Notes marked "hide on export",
//        and every note while notes are hidden, are left out.
export function toExportModel(body: DiagramBody): ExportModel {
  const marks = foreignKeyMarks(body);
  return {
    tables: body.tables.map((t) => ({
      id: t.id,
      x: t.x,
      y: t.y,
      title: t.name,
      schema: t.schema,
      color: t.color,
      columns: t.columns.map((c) => ({ name: c.name, type: c.dataType, pk: c.primaryKey, fk: marks.get(t.id)?.has(c.name) === true })),
    })),
    relations: body.relations.map((r) => ({
      id: r.id,
      from: r.fromTable,
      to: r.toTable,
      fromSide: r.fromSide,
      toSide: r.toSide,
      color: r.color,
      width: lineWidthPx(r.width),
      bends: r.waypoints.map((w) => ({ x: w.x, y: w.y })),
      label: linksOf(r).some((l) => l.from !== "" || l.to !== "") ? relationLabel(r) : "",
    })),
    notes: notesVisibleOf(body)
      ? notesOf(body)
          .filter((n) => !n.hideOnExport)
          .map((n) => ({
            id: n.id,
            x: n.x,
            y: n.y,
            width: n.width,
            height: n.height,
            text: n.text,
            color: n.color ?? DEFAULT_NOTE_COLOR,
            fontSize: n.fontSize ?? DEFAULT_NOTE_FONT_SIZE,
          }))
      : [],
  };
}

// -------------------------------------------------------------------------- stats

export function describe(body: DiagramBody): string {
  const count = (n: number, noun: string) => `${n} ${noun}${n === 1 ? "" : "s"}`;
  return [count(body.tables.length, "table"), count(body.relations.length, "relation"), count(notesOf(body).length, "note")].join(" · ");
}

export function isEmpty(body: DiagramBody): boolean {
  return body.tables.length === 0 && notesOf(body).length === 0;
}
