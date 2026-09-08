// SOT: result-export, csv-serialization, json-serialization, file-download
import type { Value } from "./bindings";
import type { JsonValue } from "./bindings/serde_json/JsonValue";
import { formatCell } from "./format";

export interface ExportColumn {
  name: string;
}

export type ExportFormat = "csv" | "json";

// WHAT:  One canonical CSV/JSON serializer + file download for grid results.
// WHY:   TableTab and ResultsPane both export Value rows; two copies already
//        drifted (clipboard-only, page-only). A shared helper keeps escaping,
//        NULL handling and JSON shapes identical everywhere.
// HOW:   Callers pass column names + Value rows, get text, then download or
//        copy it. `downloadTextFile` uses a Blob + anchor so it works in the
//        browser and the Tauri webview with no extra plugin.
// WHERE: src/features/grid/TableTab.tsx, src/features/editor/ResultsPane.tsx
export function exportCellText(value: Value | undefined): string {
  if (value === undefined) return "";
  if (value.t === "json") return JSON.stringify(value.v);
  if (value.t === "null") return "";
  return formatCell(value).text;
}

export function toCsvText(columns: readonly ExportColumn[], rows: readonly (readonly Value[])[]): string {
  const escape = (s: string) => (/[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);
  const header = columns.map((c) => escape(c.name)).join(",");
  const body = rows.map((r) => r.map((v) => escape(exportCellText(v))).join(","));
  return [header, ...body].join("\n");
}

export function toJsonText(columns: readonly ExportColumn[], rows: readonly (readonly Value[])[]): string {
  const objects = rows.map((r) => Object.fromEntries(columns.map((c, i) => [c.name, plainValue(r[i])])));
  return JSON.stringify(objects, null, 2);
}

export function plainValue(value: Value | undefined): JsonValue {
  if (value === undefined) return null;
  switch (value.t) {
    case "null":
      return null;
    case "bool":
    case "int":
    case "float":
    case "json":
      return value.v;
    case "decimal":
    case "text":
    case "bytes":
    case "date_time":
    case "unsupported":
      return value.v;
  }
}

// WHAT:  Triggers a real file download for exported text.
// WHY:   Export must leave the app as a .csv/.json file, not just sit on the
//        clipboard; Blob + anchor needs no backend round-trip or FS plugin.
export function downloadTextFile(filename: string, text: string, mime: "text/csv" | "application/json"): void {
  const blob = new Blob([text], { type: `${mime};charset=utf-8` });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}

// WHAT:  Safe file stem for exports ("public.users" stays readable, "a/b" does not become a path).
export function safeExportStem(base: string): string {
  const safe = base.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/^_+|_+$/g, "");
  return safe.length > 0 ? safe : "export";
}

export function exportFilename(base: string, format: ExportFormat): string {
  return `${safeExportStem(base)}.${format}`;
}
