// SOT: paste-rows, clipboard-rows-parse, json-rows-paste, tsv-rows-paste
import { parseJson } from "@/lib/json";
import { parseEdited } from "@/lib/fields";
import { parseTsv } from "./cellRange";
import { enumTraitsOf, jsonToCell, type DraftCell, type InspectorColumn } from "./inspectorValues";

// WHAT:  Clipboard text turned into new rows: a JSON array of objects (what "Copy rows
//        as JSON" produces), a single JSON object, or TSV whose first line names the columns.
// WHY:   Duplicating records across tables, or pasting a spreadsheet block as rows, is
//        the other half of copy; a value pasted into one cell must stay a value, so
//        only text that is clearly rows (keys or headers that are this table's columns)
//        is taken.
// HOW:   Columns the database fills in (generated, auto-increment, and a primary key
//        that has a default) are left out so a pasted copy does not collide with the
//        original; keys that are not columns are reported, not guessed at.
// WHERE: src/features/grid/TableTab.tsx (`pasteRows`)
export type PastedRows = { ok: true; rows: Map<string, DraftCell>[]; dropped: string[] } | { ok: false; error: string | null };

/// Whether the database writes this column itself, so a pasted value for it is dropped.
function filledByDatabase(column: InspectorColumn): boolean {
  return column.readOnly === true || column.autoIncrement === true || (column.primaryKey && column.hasDefault === true);
}

function findColumn(columns: readonly InspectorColumn[], name: string): InspectorColumn | undefined {
  return columns.find((c) => c.name === name) ?? columns.find((c) => c.name.toLowerCase() === name.toLowerCase());
}

export function parsePastedRows(text: string, columns: readonly InspectorColumn[]): PastedRows {
  const trimmed = text.trim();
  if (trimmed.length === 0) return { ok: false, error: null };
  if (trimmed.startsWith("[") || trimmed.startsWith("{")) return fromJson(trimmed, columns);
  return fromTsv(text, columns);
}

function fromJson(text: string, columns: readonly InspectorColumn[]): PastedRows {
  const parsed = parseJson(text);
  if (parsed === undefined || parsed === null || typeof parsed !== "object") return { ok: false, error: null };
  const objects = Array.isArray(parsed) ? parsed : [parsed];
  if (objects.length === 0 || !objects.every((o) => o !== null && typeof o === "object" && !Array.isArray(o))) return { ok: false, error: null };
  const rows: Map<string, DraftCell>[] = [];
  const dropped = new Set<string>();
  let recognised = 0;
  for (const object of objects) {
    if (object === null || typeof object !== "object" || Array.isArray(object)) continue;
    const cells = new Map<string, DraftCell>();
    for (const [key, json] of Object.entries(object)) {
      const column = findColumn(columns, key);
      if (column === undefined) {
        dropped.add(key);
        continue;
      }
      recognised += 1;
      if (filledByDatabase(column)) continue;
      const cell = jsonToCell(json, column, undefined);
      if (!cell.ok) return { ok: false, error: cell.error };
      cells.set(column.name, cell.cell);
    }
    rows.push(cells);
  }
  // Nothing in it is a column of this table: it is a value, not rows.
  if (recognised === 0) return { ok: false, error: null };
  return { ok: true, rows, dropped: [...dropped] };
}

function fromTsv(text: string, columns: readonly InspectorColumn[]): PastedRows {
  const grid = parseTsv(text);
  const header = grid[0];
  if (grid.length < 2 || header === undefined) return { ok: false, error: null };
  const mapped = header.map((name) => findColumn(columns, name.trim()));
  // Every header must be a column, or it is just a block of cells.
  if (mapped.some((c) => c === undefined)) return { ok: false, error: null };
  const rows: Map<string, DraftCell>[] = [];
  for (const line of grid.slice(1)) {
    if (line.length === 1 && line[0] === "") continue;
    const cells = new Map<string, DraftCell>();
    mapped.forEach((column, index) => {
      const raw = line[index];
      if (column === undefined || raw === undefined || filledByDatabase(column)) return;
      const value = raw === "" && column.nullable !== false ? ({ t: "null" } as const) : parseEdited(raw, column.dataType, undefined, enumTraitsOf(column));
      cells.set(column.name, { value });
    });
    rows.push(cells);
  }
  return rows.length === 0 ? { ok: false, error: null } : { ok: true, rows, dropped: [] };
}
