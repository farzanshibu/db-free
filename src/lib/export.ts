// SOT: result-export, csv-serialization, tsv-serialization, json-serialization, ndjson-serialization, markdown-serialization, export-options, export-formats, file-download
import type { ExportFormat, ExportOptions, Value } from "./bindings";
import type { JsonValue } from "./bindings/serde_json/JsonValue";

export type { ExportFormat, ExportOptions };

export interface ExportColumn {
  name: string;
}

// WHAT:  The six formats rows can be written in, with the file facts each needs.
// WHY:   One table so the dialog's picker, the save-file name and the download
//        MIME type cannot disagree; `satisfies` makes a new Rust format fail here.
// WHERE: src-tauri/src/model/transfer.rs (ExportFormat), features/transfer/ExportDialog.tsx
export type ExportMime = "text/csv" | "text/tab-separated-values" | "application/json" | "application/x-ndjson" | "text/markdown" | "application/sql";

interface FormatFacts {
  label: string;
  extension: string;
  mime: ExportMime;
}

export const EXPORT_FORMAT_FACTS = {
  csv: { label: "CSV", extension: "csv", mime: "text/csv" },
  tsv: { label: "TSV", extension: "tsv", mime: "text/tab-separated-values" },
  json: { label: "JSON", extension: "json", mime: "application/json" },
  ndjson: { label: "NDJSON", extension: "ndjson", mime: "application/x-ndjson" },
  markdown: { label: "Markdown", extension: "md", mime: "text/markdown" },
  sql: { label: "SQL INSERT", extension: "sql", mime: "application/sql" },
} as const satisfies Record<ExportFormat, FormatFacts>;

export const EXPORT_FORMATS: readonly ExportFormat[] = ["csv", "tsv", "json", "ndjson", "markdown", "sql"];

// WHAT:  The options an export starts from. Mirrors `ExportOptions::default()` in Rust.
export const DEFAULT_EXPORT_OPTIONS: ExportOptions = {
  format: "csv",
  header: true,
  delimiter: ",",
  nullText: "",
  bom: false,
  jsonShape: "array",
  pretty: true,
  quoteFormulas: true,
  includeSchema: false,
  includeData: true,
  batchRows: 100,
};

// WHAT:  One cell as plain text, for the text formats (CSV, TSV, Markdown).
// WHY:   Bytes are written as base64, exactly as the Rust exporter writes them;
//        the grid's "0x… (12 B)" label is a display, not data.
export function exportCellText(value: Value | undefined): string {
  if (value === undefined) return "";
  switch (value.t) {
    case "null":
      return "";
    case "bool":
    case "int":
    case "float":
      return String(value.v);
    case "json":
      return JSON.stringify(value.v);
    case "decimal":
    case "text":
    case "bytes":
    case "date_time":
    case "unsupported":
      return value.v;
  }
}

const FORMULA_START = /^[=+\-@\t\r]/;

function delimitedField(text: string, delimiter: string, forceQuote: boolean): string {
  if (forceQuote || text.includes(delimiter) || /["\n\r]/.test(text)) return `"${text.replace(/"/g, '""')}"`;
  return text;
}

// WHAT:  One CSV / TSV cell.
// WHY:   An empty string and a NULL are different cells: with no NULL text the
//        empty string is written `""` so a reader can tell them apart. Only text
//        is checked for formula injection — a number such as -5 is a value, not
//        a formula, and quoting it would corrupt the column.
function delimitedCell(value: Value | undefined, delimiter: string, options: ExportOptions): string {
  if (value === undefined || value.t === "null") return delimitedField(options.nullText, delimiter, false);
  if (value.t === "text") {
    if (value.v === "" && options.nullText === "") return '""';
    if (options.quoteFormulas && FORMULA_START.test(value.v)) return delimitedField(`'${value.v}`, delimiter, true);
  }
  return delimitedField(exportCellText(value), delimiter, false);
}

function markdownCell(text: string): string {
  return text.replace(/\|/g, "\\|").replace(/\r?\n|\r/g, "<br>");
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

function rowObject(columns: readonly ExportColumn[], row: readonly Value[]): Record<string, JsonValue> {
  return Object.fromEntries(columns.map((c, i) => [c.name, plainValue(row[i])]));
}

// WHAT:  Rows as text in one of the five formats that need no database (SQL
//        INSERT needs the connection's dialect; see `rows_as_sql`).
// WHY:   TableTab, ResultsPane and the export dialog all write rows already on
//        screen; this is the one writer, so escaping, NULL handling and JSON
//        shapes cannot differ between a menu item and the dialog's preview.
// HOW:   Same rules as the Rust exporter (src-tauri/src/services/export.rs). The
//        output has no trailing newline, like the quick-copy items always had.
//        `table` names the JSON envelope.
export function serializeRows(columns: readonly ExportColumn[], rows: readonly (readonly Value[])[], options: ExportOptions, table = "export"): string {
  const text = serializeBody(columns, rows, options, table);
  return options.bom ? `\uFEFF${text}` : text;
}

function serializeBody(columns: readonly ExportColumn[], rows: readonly (readonly Value[])[], options: ExportOptions, table: string): string {
  switch (options.format) {
    case "csv":
    case "tsv": {
      const delimiter = options.format === "tsv" ? "\t" : options.delimiter;
      const lines = rows.map((row) => columns.map((_, i) => delimitedCell(row[i], delimiter, options)).join(delimiter));
      if (options.header) lines.unshift(columns.map((c) => delimitedField(c.name, delimiter, false)).join(delimiter));
      return lines.join("\n");
    }
    case "json": {
      const objects = rows.map((row) => rowObject(columns, row));
      const payload = options.jsonShape === "envelope" ? { table, rows: objects } : objects;
      return JSON.stringify(payload, null, options.pretty ? 2 : undefined);
    }
    case "ndjson":
      return rows.map((row) => JSON.stringify(rowObject(columns, row))).join("\n");
    case "markdown": {
      const header = `| ${columns.map((c) => markdownCell(c.name)).join(" | ")} |`;
      const rule = `| ${columns.map(() => "---").join(" | ")} |`;
      const cellText = (cell: Value | undefined) => markdownCell(cell === undefined || cell.t === "null" ? options.nullText : exportCellText(cell));
      const body = rows.map((row) => `| ${columns.map((_, i) => cellText(row[i])).join(" | ")} |`);
      return [header, rule, ...body].join("\n");
    }
    case "sql":
      throw new Error("SQL INSERT statements are written by the connection's dialect, not here.");
  }
}

// WHAT:  The extension of the single file a multi-table export writes: a ZIP of
//        per-table files, one combined JSON or one combined SQL script.
// WHY:   The save dialog needs a default name before the export runs. Mirrors
//        `bundle_extension` in src-tauri/src/services/export.rs, which decides
//        the real shape.
export function bundleExtension(options: ExportOptions, tableCount: number): string {
  if (options.format === "sql") return "sql";
  if (options.format === "json" && (tableCount > 1 || options.includeSchema)) return "json";
  return tableCount > 1 ? "zip" : EXPORT_FORMAT_FACTS[options.format].extension;
}

// WHAT:  The columns and rows narrowed to the ones the user kept, in table order.
export function selectColumns<C extends ExportColumn>(columns: readonly C[], rows: readonly (readonly Value[])[], keep: ReadonlySet<string>): { columns: C[]; rows: Value[][] } {
  const indexes = columns.map((c, i) => ({ c, i })).filter(({ c }) => keep.has(c.name));
  return {
    columns: indexes.map(({ c }) => c),
    rows: rows.map((row) => indexes.map(({ i }) => row[i] ?? { t: "null" })),
  };
}

export function toCsvText(columns: readonly ExportColumn[], rows: readonly (readonly Value[])[]): string {
  return serializeRows(columns, rows, { ...DEFAULT_EXPORT_OPTIONS, format: "csv" });
}

export function toJsonText(columns: readonly ExportColumn[], rows: readonly (readonly Value[])[]): string {
  return serializeRows(columns, rows, { ...DEFAULT_EXPORT_OPTIONS, format: "json" });
}

// WHAT:  Triggers a real file download for exported text.
// WHY:   Export must leave the app as a file, not just sit on the clipboard; Blob
//        + anchor needs no backend round-trip or FS plugin.
export function downloadTextFile(filename: string, text: string, mime: ExportMime): void {
  const blob = new Blob([text], { type: `${mime};charset=utf-8` });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => {
    URL.revokeObjectURL(url);
  }, 1000);
}

// WHAT:  Safe file stem for exports ("public.users" stays readable, "a/b" does not become a path).
export function safeExportStem(base: string): string {
  const safe = base.replace(/[^A-Za-z0-9._-]+/g, "_").replace(/^_+|_+$/g, "");
  return safe.length > 0 ? safe : "export";
}

export function exportFilename(base: string, format: ExportFormat): string {
  return `${safeExportStem(base)}.${EXPORT_FORMAT_FACTS[format].extension}`;
}
