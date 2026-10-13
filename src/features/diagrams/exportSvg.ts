// SOT: diagram-svg-export, diagram-image-model, erd-export-svg
import type { ConnectionColor, DiagramSide } from "@/lib/bindings";
import { arrowPoints, routeEdge } from "./edgePath";
import { EMPTY_HEIGHT, HEADER_HEIGHT, MORE_HEIGHT, ROW_HEIGHT, TABLE_WIDTH, boundsOf, resolveSides, sideAnchor, tableHeight, type Box, type Point } from "./metrics";

// WHAT:  Draws a diagram as a standalone SVG document from its model.
// WHY:   Screenshotting the canvas (html-to-image and friends) needs a library, a
//        stylesheet the image does not have, and whatever happens to be on screen.
//        Drawing from the model needs none of that, exports the whole diagram or
//        just a selection, and looks the same however the canvas is zoomed.
// HOW:   Pure: (model, resolved colours) in, SVG text out. Table cards use
//        metrics.ts for their size and relations use edgePath.ts for their shape,
//        so the file matches the screen. Text is clipped by character count rather
//        than measured, because an SVG rendered as an image cannot load the app's
//        fonts and so cannot be measured with them.
// WHERE: src/features/diagrams/exportDiagram.ts (resolves the colours, rasterises, saves)

export interface ExportColumn {
  name: string;
  type: string;
  pk: boolean;
  fk: boolean;
}

export interface ExportTable {
  id: string;
  x: number;
  y: number;
  title: string;
  schema: string | null;
  color: ConnectionColor | null;
  columns: ExportColumn[];
}

export interface ExportRelation {
  id: string;
  from: string;
  to: string;
  fromSide: DiagramSide | null;
  toSide: DiagramSide | null;
  color: ConnectionColor | null;
  /// Stroke width in px.
  width: number;
  bends: Point[];
  label: string;
}

export interface ExportNote {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
  text: string;
  color: ConnectionColor;
  fontSize: number;
}

export interface ExportModel {
  tables: ExportTable[];
  relations: ExportRelation[];
  notes: ExportNote[];
}

/// Every colour the drawing uses, already resolved to something an SVG image can paint.
export interface ExportTheme {
  background: string;
  surface: string;
  border: string;
  separator: string;
  text: string;
  muted: string;
  accent: string;
  accentInk: string;
  warning: string;
  inkDark: string;
  inkLight: string;
  palette: Record<ConnectionColor, string>;
}

export interface SvgOptions {
  /// Rows drawn per card before "+N more"; unset draws every column.
  maxRows?: number;
}

export interface SvgImage {
  svg: string;
  width: number;
  height: number;
}

const PADDING = 48;
const FONT = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace";
const LIGHT_FILLS: ReadonlySet<ConnectionColor> = new Set<ConnectionColor>(["teal", "green", "amber", "orange", "pink"]);

export function escapeXml(raw: string): string {
  return raw.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&apos;");
}

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, Math.max(0, max - 1))}…` : text;
}

function cardBox(table: ExportTable, options: SvgOptions): Box {
  return { x: table.x, y: table.y, width: TABLE_WIDTH, height: tableHeight(table.columns.length, options.maxRows) };
}

// WHAT:  The model restricted to some tables: their relations (both ends inside)
//        and the notes named. Used for "export selection".
export function restrict(model: ExportModel, tableIds: ReadonlySet<string>, noteIds: ReadonlySet<string>): ExportModel {
  return {
    tables: model.tables.filter((t) => tableIds.has(t.id)),
    relations: model.relations.filter((r) => tableIds.has(r.from) && tableIds.has(r.to)),
    notes: model.notes.filter((n) => noteIds.has(n.id)),
  };
}

function drawTable(table: ExportTable, theme: ExportTheme, options: SvgOptions): string {
  const box = cardBox(table, options);
  const fill = table.color === null ? theme.accent : theme.palette[table.color];
  const ink = table.color === null ? theme.accentInk : LIGHT_FILLS.has(table.color) ? theme.inkDark : theme.inkLight;
  const parts: string[] = [
    `<g data-table="${escapeXml(table.id)}">`,
    `<rect x="${box.x}" y="${box.y}" width="${box.width}" height="${box.height}" rx="6" fill="${theme.surface}" stroke="${theme.border}" />`,
    `<rect x="${box.x}" y="${box.y}" width="${box.width}" height="${HEADER_HEIGHT}" rx="6" fill="${fill}" />`,
    `<rect x="${box.x}" y="${box.y + HEADER_HEIGHT - 6}" width="${box.width}" height="6" fill="${fill}" />`,
    `<text x="${box.x + 10}" y="${box.y + 19}" fill="${ink}" font-size="12" font-weight="600">${escapeXml(clip(table.title, table.schema === null ? 32 : 22))}</text>`,
  ];
  if (table.schema !== null) {
    parts.push(
      `<text x="${box.x + box.width - 10}" y="${box.y + 19}" text-anchor="end" fill="${ink}" fill-opacity="0.75" font-size="10">${escapeXml(clip(table.schema, 14))}</text>`,
    );
  }
  const top = box.y + HEADER_HEIGHT;
  if (table.columns.length === 0) {
    parts.push(`<text x="${box.x + 10}" y="${top + EMPTY_HEIGHT / 2 + 4}" fill="${theme.muted}" font-size="10" font-style="italic">no columns</text>`);
  } else {
    const shown = options.maxRows === undefined ? table.columns : table.columns.slice(0, options.maxRows);
    shown.forEach((column, i) => {
      const y = top + i * ROW_HEIGHT;
      const marker = column.pk ? { text: "PK", fill: theme.warning } : column.fk ? { text: "FK", fill: theme.accent } : null;
      parts.push(`<line x1="${box.x}" y1="${y}" x2="${box.x + box.width}" y2="${y}" stroke="${theme.separator}" />`);
      if (marker) parts.push(`<text x="${box.x + 10}" y="${y + 15}" fill="${marker.fill}" font-size="8" font-weight="700">${marker.text}</text>`);
      parts.push(`<text x="${box.x + 30}" y="${y + 15}" fill="${theme.text}" font-size="11">${escapeXml(clip(column.name === "" ? "unnamed" : column.name, 22))}</text>`);
      parts.push(`<text x="${box.x + box.width - 10}" y="${y + 15}" text-anchor="end" fill="${theme.muted}" font-size="10">${escapeXml(clip(column.type, 16))}</text>`);
    });
    const hidden = table.columns.length - shown.length;
    if (hidden > 0) {
      const y = top + shown.length * ROW_HEIGHT;
      parts.push(`<line x1="${box.x}" y1="${y}" x2="${box.x + box.width}" y2="${y}" stroke="${theme.separator}" />`);
      parts.push(`<text x="${box.x + 10}" y="${y + MORE_HEIGHT / 2 + 3}" fill="${theme.muted}" font-size="10">+${hidden} more</text>`);
    }
  }
  parts.push(`<rect x="${box.x}" y="${box.y}" width="${box.width}" height="${box.height}" rx="6" fill="none" stroke="${theme.border}" />`);
  parts.push("</g>");
  return parts.join("");
}

interface DrawnRelation {
  markup: string;
  points: Point[];
}

function drawRelation(relation: ExportRelation, boxes: ReadonlyMap<string, Box>, theme: ExportTheme): DrawnRelation | null {
  const fromBox = boxes.get(relation.from);
  const toBox = boxes.get(relation.to);
  if (!fromBox || !toBox) return null;
  const sides = resolveSides(relation.fromSide, relation.toSide, fromBox, toBox, relation.from === relation.to);
  const route = routeEdge(sideAnchor(fromBox, sides.fromSide), sides.fromSide, sideAnchor(toBox, sides.toSide), sides.toSide, relation.bends);
  const colour = relation.color === null ? theme.accent : theme.palette[relation.color];
  const parts = [
    `<g data-relation="${escapeXml(relation.id)}">`,
    `<path d="${route.d}" fill="none" stroke="${colour}" stroke-width="${relation.width}" stroke-linecap="round" />`,
    `<polygon points="${arrowPoints(route, Math.max(1, relation.width / 1.6))}" fill="${colour}" />`,
  ];
  if (relation.label !== "") {
    const label = clip(relation.label, 40);
    const width = Math.max(28, label.length * 6.2 + 12);
    parts.push(
      `<rect x="${route.label.x - width / 2}" y="${route.label.y - 8}" width="${width}" height="16" rx="3" fill="${theme.surface}" stroke="${theme.border}" />`,
      `<text x="${route.label.x}" y="${route.label.y + 3.5}" text-anchor="middle" fill="${theme.text}" font-size="10">${escapeXml(label)}</text>`,
    );
  }
  parts.push("</g>");
  return { markup: parts.join(""), points: [route.label, route.tip, ...relation.bends] };
}

function wrap(text: string, width: number, fontSize: number): string[] {
  const perLine = Math.max(8, Math.floor((width - 16) / (fontSize * 0.6)));
  const lines: string[] = [];
  for (const paragraph of text.split(/\r?\n/)) {
    let line = "";
    for (const word of paragraph.split(/\s+/).filter((w) => w !== "")) {
      const next = line === "" ? word : `${line} ${word}`;
      if (next.length > perLine && line !== "") {
        lines.push(line);
        line = word;
      } else {
        line = next;
      }
    }
    lines.push(line);
  }
  return lines;
}

function drawNote(note: ExportNote, theme: ExportTheme): string {
  const ink = LIGHT_FILLS.has(note.color) ? theme.inkDark : theme.inkLight;
  const room = Math.max(1, Math.floor((note.height - 24) / (note.fontSize * 1.35)));
  const lines = wrap(note.text, note.width, note.fontSize).slice(0, room);
  const text = lines
    .map((line, i) => `<tspan x="${note.x + 8}" dy="${i === 0 ? 0 : note.fontSize * 1.35}">${escapeXml(line)}</tspan>`)
    .join("");
  return [
    `<g data-note="${escapeXml(note.id)}">`,
    `<rect x="${note.x}" y="${note.y}" width="${note.width}" height="${note.height}" rx="6" fill="${theme.palette[note.color]}" stroke="${theme.border}" />`,
    `<rect x="${note.x}" y="${note.y}" width="${note.width}" height="14" rx="6" fill="${ink}" fill-opacity="0.12" />`,
    `<text x="${note.x + 8}" y="${note.y + 14 + note.fontSize + 2}" fill="${ink}" font-size="${note.fontSize}">${text}</text>`,
    "</g>",
  ].join("");
}

// WHAT:  The whole drawing: relations under notes under tables, on the theme's ground.
export function buildSvg(model: ExportModel, theme: ExportTheme, options: SvgOptions = {}): SvgImage {
  const boxes = new Map(model.tables.map((t) => [t.id, cardBox(t, options)]));
  const drawn = model.relations.flatMap((r) => drawRelation(r, boxes, theme) ?? []);
  const extent = boundsOf([
    ...boxes.values(),
    ...model.notes.map((n) => ({ x: n.x, y: n.y, width: n.width, height: n.height })),
    ...drawn.flatMap((d) => d.points.map((p) => ({ x: p.x, y: p.y, width: 0, height: 0 }))),
  ]) ?? { x: 0, y: 0, width: 1, height: 1 };
  const width = Math.max(1, Math.ceil(extent.width + PADDING * 2));
  const height = Math.max(1, Math.ceil(extent.height + PADDING * 2));
  const dx = PADDING - extent.x;
  const dy = PADDING - extent.y;
  const body = [...drawn.map((d) => d.markup), ...model.notes.map((n) => drawNote(n, theme)), ...model.tables.map((t) => drawTable(t, theme, options))].join("");
  const svg = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" font-family="${FONT}">`,
    `<rect width="${width}" height="${height}" fill="${theme.background}" />`,
    `<g transform="translate(${dx} ${dy})">${body}</g>`,
    "</svg>",
  ].join("");
  return { svg, width, height };
}
