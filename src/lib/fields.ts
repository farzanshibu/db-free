// SOT: field-kind-registry, column-type-classification, edited-text-parsing, vector-field, array-field, enum-field
import type { ColumnTraits, Value } from "./bindings";
import { parseJson } from "./json";
import { temporalKind } from "./datetime";

export type FieldKind = "bool" | "int" | "float" | "decimal" | "json" | "date" | "time" | "datetime" | "bytes" | "text" | "enum" | "array" | "vector";

/// The part of a column's traits that decides which control it gets.
export type EnumTraits = Pick<ColumnTraits, "enumValues" | "multi">;

/// pgvector's `vector(3)` / `halfvec(8)` / `sparsevec(10)`: a bracketed list of numbers.
const VECTOR_RE = /^(vector|halfvec)(\((\d+)\))?$/;

/// The dimension a `vector(n)` column was declared with, when it was.
export function vectorDimension(typeName: string): number | null {
  const match = VECTOR_RE.exec(typeName.trim().toLowerCase());
  const digits = match?.[3];
  return digits === undefined ? null : Number(digits);
}

/// Postgres spells an array column `integer[]` (format_type) or `_int4` (udt name).
export function isArrayType(typeName: string): boolean {
  const t = typeName.trim().toLowerCase();
  return t.endsWith("[]") || (t.startsWith("_") && t.length > 1);
}

export type VectorParse = { ok: true; text: string } | { ok: false; error: string };

// WHAT:  Validates a vector typed as `[1.0, 2.0]` (brackets optional) and writes it back
//        canonically, checking the dimension the column was declared with.
// WHY:   The database rejects a wrong-sized vector with an error about a cast; the
//        editor can say "3 numbers expected, got 2" before anything is staged.
export function parseVectorText(text: string, dimension: number | null): VectorParse {
  let body = text.trim();
  if (body.startsWith("[") && body.endsWith("]")) body = body.slice(1, -1);
  if (body.trim().length === 0) return { ok: false, error: "Enter at least one number." };
  const parts = body.split(",").map((p) => p.trim());
  const numbers: number[] = [];
  for (const part of parts) {
    const n = Number(part);
    if (part.length === 0 || !Number.isFinite(n)) return { ok: false, error: `"${part}" is not a number.` };
    numbers.push(n);
  }
  if (dimension !== null && numbers.length !== dimension) {
    return { ok: false, error: `Expected ${dimension} number${dimension === 1 ? "" : "s"}, got ${numbers.length}.` };
  }
  return { ok: true, text: `[${numbers.join(",")}]` };
}

// WHAT:  Picks the editor control for a column: declared SQL type first, then
//        the shape of the value the adapter returned.
// WHY:   One classification feeds the grid editor, the insert form and text
//        parsing, so a `timestamp` column gets the same picker everywhere.
// WHERE: src/components/global/ValueEditor.tsx, src/features/grid/DataGrid.tsx
export function fieldKind(typeName: string, sample?: Value, traits?: EnumTraits): FieldKind {
  const t = typeName.toLowerCase();
  // A column that lists its allowed values is picked from them, whatever the type says.
  if (traits !== undefined && traits.enumValues.length > 0) return "enum";
  if (VECTOR_RE.test(t.trim())) return "vector";
  // Postgres hands arrays over as `{a,b}` text; an engine that decodes them to JSON lists edits them as JSON.
  if (isArrayType(t)) {
    if (sample?.t === "json") return "json";
    if (sample === undefined || sample.t === "text" || sample.t === "null") return "array";
    return "text";
  }
  if (t.includes("bool")) return "bool";
  if (t.includes("json")) return "json";
  if (/bytea|blob|binary/.test(t)) return "bytes";
  const temporal = temporalKind(t, sample?.t === "date_time" ? sample.v : undefined);
  if (temporal) return temporal;
  if (!t.includes("interval") && /int|serial/.test(t)) return "int";
  if (/numeric|decimal|money/.test(t)) return "decimal";
  if (/real|float|double/.test(t)) return "float";
  switch (sample?.t) {
    case "bool":
      return "bool";
    case "int":
      return "int";
    case "float":
      return "float";
    case "decimal":
      return "decimal";
    case "json":
      return "json";
    case "bytes":
      return "bytes";
    case "date_time":
      return "datetime";
    case "null":
    case "text":
    case "unsupported":
    case undefined:
      return "text";
  }
}

// WHAT:  Turns typed text back into a Value using the column kind as a hint.
//        `NULL` (exact) is the null sentinel; anything unparseable stays text
//        so the database reports the real error.
export function parseEdited(text: string, typeName: string, original: Value | undefined, traits?: EnumTraits): Value {
  if (text === "NULL") return { t: "null" };
  const trimmed = text.trim();
  switch (fieldKind(typeName, original, traits)) {
    case "bool": {
      const lower = trimmed.toLowerCase();
      if (lower === "true" || lower === "1" || lower === "t") return { t: "bool", v: true };
      if (lower === "false" || lower === "0" || lower === "f") return { t: "bool", v: false };
      break;
    }
    case "int": {
      const n = Number(trimmed);
      if (trimmed.length > 0 && Number.isInteger(n)) return { t: "int", v: n };
      break;
    }
    case "decimal":
      if (trimmed.length > 0 && !Number.isNaN(Number(trimmed))) return { t: "decimal", v: trimmed };
      break;
    case "float": {
      const n = Number(trimmed);
      if (trimmed.length > 0 && !Number.isNaN(n)) return { t: "float", v: n };
      break;
    }
    case "json": {
      const parsed = parseJson(text);
      if (parsed !== undefined) return { t: "json", v: parsed };
      break;
    }
    case "date":
    case "time":
    case "datetime":
      return { t: "date_time", v: trimmed };
    case "bytes":
    case "text":
    case "enum":
    case "array":
    case "vector":
      break;
  }
  return { t: "text", v: text };
}

// WHAT:  Display text for an editable Value (what an inline text editor opens with).
export function editText(value: Value): string {
  switch (value.t) {
    case "null":
      return "";
    case "json":
      return JSON.stringify(value.v);
    case "bool":
      return value.v ? "true" : "false";
    case "int":
    case "float":
      return String(value.v);
    case "decimal":
    case "text":
    case "bytes":
    case "date_time":
    case "unsupported":
      return value.v;
  }
}
