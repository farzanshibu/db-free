// SOT: inspector-values, inspector-draft, json-row-edit, cell-validation, empty-value-for-kind
import type { ColumnInfo, ColumnTraits, SqlToken, Value } from "@/lib/bindings";
import type { JsonValue } from "@/lib/bindings/serde_json/JsonValue";
import { editText, fieldKind, isArrayType, parseEdited, parseVectorText, vectorDimension, type EnumTraits } from "@/lib/fields";
import { formatPgArray } from "@/lib/pgArray";
import { tokenValue } from "@/lib/sqlTokens";
import { plainValue } from "@/lib/export";

// WHAT:  The pure parts of the record inspector's editing: what a field holds while
//        it is being edited, whether that is acceptable, and how a JSON edit of the
//        whole row maps back onto cells.
// WHY:   Both the Fields tab and the JSON tab end in the same staged edits; one set
//        of rules for "what is a valid value for this column" keeps them agreeing.
// WHERE: src/features/grid/RecordInspector.tsx

/// What the inspector needs to know about a column. A table page has all of it from
/// the catalog and the column traits; a query result only has names and types.
export interface InspectorColumn {
  name: string;
  dataType: string;
  primaryKey: boolean;
  nullable?: boolean | undefined;
  /// The database computes it: shown, never written.
  readOnly?: boolean | undefined;
  autoIncrement?: boolean | undefined;
  hasDefault?: boolean | undefined;
  defaultExpr?: string | null | undefined;
  enumValues?: readonly string[] | undefined;
  multi?: boolean | undefined;
}

/// A field being edited: the value, and the SQL token when it is DEFAULT / now().
export interface DraftCell {
  value: Value;
  raw?: SqlToken | undefined;
}

export type Draft = Readonly<Record<string, DraftCell>>;

/// One cell write the inspector asks its owner to stage: `row` indexes the rows the
/// inspector shows, `col` its columns.
export interface InspectorCellEdit {
  row: number;
  col: number;
  value: Value;
  raw?: SqlToken | undefined;
}

/// What makes the inspector editable. Absent = it only reads.
export interface InspectorEdit {
  connectionId: string;
  /// Stages (or, in direct mode, applies) the edits.
  onApply: (edits: readonly InspectorCellEdit[]) => void;
  /// Deletes the rows shown.
  onDelete: () => void;
  /// Primary-key column names: what SELECT / UPDATE / DELETE statements address a row by.
  keyColumns: readonly string[];
  /// Why editing is off right now (read-only connection, no key); the form still reads.
  disabledReason?: string | undefined;
}

export const valuesEqual = (a: Value, b: Value): boolean => JSON.stringify(a) === JSON.stringify(b);

export function enumTraitsOf(column: InspectorColumn): EnumTraits | undefined {
  return column.enumValues !== undefined && column.enumValues.length > 0 ? { enumValues: [...column.enumValues], multi: column.multi ?? false } : undefined;
}

export function toColumnInfo(column: InspectorColumn, ordinal: number): ColumnInfo {
  return { name: column.name, dataType: column.dataType, nullable: column.nullable ?? true, primaryKey: column.primaryKey, ordinal };
}

export function toTraits(column: InspectorColumn): ColumnTraits {
  return {
    name: column.name,
    enumValues: [...(column.enumValues ?? [])],
    multi: column.multi ?? false,
    hasDefault: column.hasDefault ?? false,
    defaultExpr: column.defaultExpr ?? null,
    generated: column.readOnly ?? false,
    autoIncrement: column.autoIncrement ?? false,
  };
}

// WHAT:  A starting value for a NULL field the user chose to give a value ("Set value").
// WHY:   An empty control cannot be told apart from "still NULL"; a sensible first
//        value (today's date, 0, an empty string) can be edited from there.
export function emptyFor(column: InspectorColumn): DraftCell {
  const traits = enumTraitsOf(column);
  switch (fieldKind(column.dataType, undefined, traits)) {
    case "bool":
      return { value: { t: "bool", v: false } };
    case "int":
      return { value: { t: "int", v: 0 } };
    case "float":
      return { value: { t: "float", v: 0 } };
    case "decimal":
      return { value: { t: "decimal", v: "0" } };
    case "json":
      return { value: { t: "json", v: {} } };
    case "date":
      return { value: tokenValue("current_date"), raw: "current_date" };
    case "time":
      return { value: tokenValue("current_time"), raw: "current_time" };
    case "datetime":
      return { value: tokenValue("now"), raw: "now" };
    case "enum":
      return { value: traits?.multi ? { t: "text", v: isArrayType(column.dataType) ? "{}" : "" } : { t: "text", v: traits?.enumValues[0] ?? "" } };
    case "array":
      return { value: { t: "text", v: "{}" } };
    case "vector": {
      const dimension = vectorDimension(column.dataType) ?? 1;
      return { value: { t: "text", v: `[${Array.from({ length: dimension }, () => "0").join(",")}]` } };
    }
    case "bytes":
    case "text":
      return { value: { t: "text", v: "" } };
  }
}

// WHAT:  Why a field's value cannot be saved, or null when it can.
export function validateCell(column: InspectorColumn, cell: DraftCell): string | null {
  const { value } = cell;
  if (value.t === "null") return column.nullable === false ? `${column.name} cannot be NULL.` : null;
  if (cell.raw !== undefined) return null;
  const kind = fieldKind(column.dataType, value, enumTraitsOf(column));
  switch (kind) {
    case "int":
      return value.t === "int" ? null : `${column.name} must be a whole number.`;
    case "float":
      return value.t === "float" || value.t === "int" ? null : `${column.name} must be a number.`;
    case "decimal":
      return value.t === "decimal" || value.t === "int" || value.t === "float" || (value.t === "text" && value.v.trim().length > 0 && !Number.isNaN(Number(value.v))) ? null : `${column.name} must be a number.`;
    case "vector": {
      const result = parseVectorText(editText(value), vectorDimension(column.dataType));
      return result.ok ? null : result.error;
    }
    case "enum": {
      const allowed = column.enumValues ?? [];
      if (column.multi === true) return null;
      return value.t === "text" && allowed.includes(value.v) ? null : `${column.name} must be one of: ${allowed.slice(0, 6).join(", ")}${allowed.length > 6 ? ", …" : ""}.`;
    }
    case "json":
      return value.t === "json" || value.t === "text" ? null : `${column.name} must be JSON.`;
    case "bool":
    case "date":
    case "time":
    case "datetime":
    case "bytes":
    case "text":
    case "array":
      return null;
  }
}

/// The row as the JSON tab shows it: column name -> plain value.
export function plainRecord(columns: readonly InspectorColumn[], row: readonly Value[]): Record<string, JsonValue> {
  return Object.fromEntries(columns.map((c, i) => [c.name, plainValue(row[i])]));
}

export type JsonCell = { ok: true; cell: DraftCell } | { ok: false; error: string };

// WHAT:  One value from an edited JSON row, as the cell it stands for.
// HOW:   By the column's kind: a number into an integer column must be whole, a string
//        into a date column is the date text, an array into a Postgres array column
//        becomes its `{a,b}` text. Anything else goes through the same parser the
//        cell editor uses.
export function jsonToCell(json: JsonValue, column: InspectorColumn, current: Value | undefined): JsonCell {
  if (json === null) return { ok: true, cell: { value: { t: "null" } } };
  const kind = fieldKind(column.dataType, current, enumTraitsOf(column));
  if (typeof json === "boolean") return { ok: true, cell: { value: { t: "bool", v: json } } };
  if (typeof json === "number") {
    if (kind === "int") return Number.isInteger(json) ? { ok: true, cell: { value: { t: "int", v: json } } } : { ok: false, error: `${column.name} must be a whole number.` };
    if (kind === "decimal") return { ok: true, cell: { value: { t: "decimal", v: String(json) } } };
    if (kind === "text" || kind === "enum") return { ok: true, cell: { value: { t: "text", v: String(json) } } };
    if (kind === "json") return { ok: true, cell: { value: { t: "json", v: json } } };
    return { ok: true, cell: { value: Number.isInteger(json) && kind !== "float" ? { t: "int", v: json } : { t: "float", v: json } } };
  }
  if (typeof json === "string") {
    if (kind === "text" || kind === "enum") return { ok: true, cell: { value: { t: "text", v: json } } };
    return { ok: true, cell: { value: parseEdited(json, column.dataType, current, enumTraitsOf(column)) } };
  }
  if (kind === "array" && Array.isArray(json) && json.every((item) => item === null || typeof item !== "object")) {
    return { ok: true, cell: { value: { t: "text", v: formatPgArray(json.map((item) => (item === null ? null : String(item)))) } } };
  }
  return { ok: true, cell: { value: { t: "json", v: json } } };
}

export type RowJsonParse = { ok: true; changes: Map<string, DraftCell> } | { ok: false; error: string };

// WHAT:  An edited JSON row compared with the real one: the cells that changed.
// WHY:   The JSON tab edits the whole row as text; only what differs becomes an update.
export function changesFromJson(text: string, columns: readonly InspectorColumn[], row: readonly Value[], parse: (text: string) => JsonValue | undefined): RowJsonParse {
  const parsed = parse(text);
  if (parsed === undefined) return { ok: false, error: "That is not valid JSON." };
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) return { ok: false, error: "Edit the row as a JSON object: { \"column\": value, … }." };
  const changes = new Map<string, DraftCell>();
  for (const [key, json] of Object.entries(parsed)) {
    const index = columns.findIndex((c) => c.name === key);
    const column = columns[index];
    if (column === undefined) return { ok: false, error: `There is no column "${key}".` };
    const current = row[index];
    if (JSON.stringify(plainValue(current)) === JSON.stringify(json)) continue;
    if (column.readOnly === true) return { ok: false, error: `${key} is generated by the database and cannot be edited.` };
    const cell = jsonToCell(json, column, current);
    if (!cell.ok) return cell;
    const problem = validateCell(column, cell.cell);
    if (problem !== null) return { ok: false, error: problem };
    changes.set(key, cell.cell);
  }
  return { ok: true, changes };
}
