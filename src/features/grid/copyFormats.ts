// SOT: row-copy-formats, rows-as-text, copy-menu-formats
import type { ExportFormat, ExportOptions, TableRef, Value } from "@/lib/bindings";
import { DEFAULT_EXPORT_OPTIONS, EXPORT_FORMAT_FACTS, serializeRows, type ExportColumn } from "@/lib/export";
import { ipc } from "@/lib/ipc";

// WHAT:  The formats a row, a selection or a page can be copied (or saved) in, and the
//        one function that writes them.
// WHY:   The cell menu, the toolbar's Export menu and the inspector's Copy menu all
//        offer the same list; one writer means a Markdown table copied from the
//        grid is the Markdown table the inspector copies.
// HOW:   Everything but SQL is `serializeRows` (the exporter's own rules). SQL INSERT
//        statements are written by Rust in the connection's dialect (`rows_as_sql`),
//        so they need the table; a result that is not one table's rows may bring its
//        own generic statement as `fallbackSql`.
// WHERE: src/features/grid/DataGrid.tsx, src/features/grid/TableTab.tsx, src/features/grid/RecordInspector.tsx

/// The order the menus list the formats in.
export const COPY_FORMATS: readonly ExportFormat[] = ["json", "csv", "tsv", "markdown", "ndjson", "sql"];

/// What the menus call a format ("Markdown", "SQL INSERT").
export const copyLabel = (format: ExportFormat): string => EXPORT_FORMAT_FACTS[format].label;

/// The connection and table whose dialect writes SQL statements for the rows.
export interface SqlTarget {
  connectionId: string;
  table: TableRef;
}

// TSV is what the grid pastes back, so NULL stays spelled out and nothing gets a
// formula-guard prefix; every other format is the exporter's default.
function copyOptions(format: ExportFormat): ExportOptions {
  if (format === "tsv") return { ...DEFAULT_EXPORT_OPTIONS, format, nullText: "NULL", quoteFormulas: false };
  return { ...DEFAULT_EXPORT_OPTIONS, format };
}

export async function rowsAsText(
  format: ExportFormat,
  columns: readonly ExportColumn[],
  rows: readonly (readonly Value[])[],
  target: SqlTarget | null,
  fallbackSql?: string,
): Promise<string> {
  if (format !== "sql") return serializeRows(columns, rows, copyOptions(format));
  if (target !== null) {
    return ipc("rows_as_sql", {
      connectionId: target.connectionId,
      table: target.table,
      kind: "insert",
      columns: columns.map((c) => c.name),
      keyColumns: [],
      rows: rows.map((row) => [...row]),
    });
  }
  if (fallbackSql !== undefined) return fallbackSql;
  throw new Error("SQL statements need the table these rows came from.");
}

/// "3 rows as Markdown" for the toast after a copy.
export function copiedLabel(count: number, format: ExportFormat): string {
  return `${count === 1 ? "Row" : `${count} rows`} as ${copyLabel(format)}`;
}
