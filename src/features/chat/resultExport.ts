// SOT: chat-result-export, result-as-sql, result-copy, result-download
import type { Engine, QueryOutcome, Value } from "@/lib/bindings";
import { ENGINE_FACTS } from "@/lib/bindings/EngineFacts.gen";
import { toCsvText, toJsonText } from "@/lib/export";

// WHAT:  A result set from the chat as CSV, JSON or INSERT statements, to copy
//        or save.
// WHY:   An answer in the conversation is only useful if the rows can leave it.
//        CSV and JSON already have one canonical serializer (lib/export); SQL is
//        the third form people ask for, and the one that needs care: values must
//        be quoted for the engine they came from.
// HOW:   Pure functions over the grid's own `Value` shape. The table name for the
//        INSERTs is read from the statement that produced the rows.
// WHERE: src/features/chat/InlineResult.tsx

export type ResultFormat = "csv" | "json" | "sql";

export interface ResultRows {
  columns: readonly { name: string }[];
  rows: readonly (readonly Value[])[];
}

export const RESULT_FORMATS: readonly { value: ResultFormat; label: string; extension: string; mime: string }[] = [
  { value: "csv", label: "CSV", extension: "csv", mime: "text/csv" },
  { value: "json", label: "JSON", extension: "json", mime: "application/json" },
  { value: "sql", label: "SQL", extension: "sql", mime: "application/sql" },
];

/// The first statement that returned rows, or null when none did.
export function firstRows(outcome: QueryOutcome): ResultRows | null {
  for (const statement of outcome.statements) {
    if (statement.kind === "rows") return { columns: statement.result.columns, rows: statement.result.rows };
  }
  return null;
}

/// The table a SELECT reads from: the identifier after the first FROM.
export function guessTable(sql: string): string | null {
  const match = /\bfrom\s+((?:[`"[]?[\w$]+[`"\]]?\.)*[`"[]?[\w$]+[`"\]]?)/i.exec(sql);
  const name = match?.[1];
  return name === undefined ? null : name.replace(/[`"[\]]/g, "");
}

function quoteIdent(engine: Engine, name: string): string {
  if (ENGINE_FACTS[engine].family === "mysql") return `\`${name.replace(/`/g, "``")}\``;
  return `"${name.replace(/"/g, '""')}"`;
}

function quoteTable(engine: Engine, table: string): string {
  return table
    .split(".")
    .map((part) => quoteIdent(engine, part))
    .join(".");
}

function textLiteral(text: string): string {
  return `'${text.replace(/'/g, "''")}'`;
}

// WHAT:  One cell as a SQL literal.
// HOW:   NULL, booleans and finite numbers are bare; a decimal is bare only when it
//        is plainly numeric (never trust the text of a "decimal" into a statement);
//        everything else — text, dates, JSON, bytes — is a quoted string.
export function sqlLiteral(value: Value | undefined): string {
  if (value === undefined) return "NULL";
  switch (value.t) {
    case "null":
      return "NULL";
    case "bool":
      return value.v ? "TRUE" : "FALSE";
    case "int":
    case "float":
      return Number.isFinite(value.v) ? String(value.v) : "NULL";
    case "decimal":
      return /^-?\d+(\.\d+)?([eE][+-]?\d+)?$/.test(value.v) ? value.v : textLiteral(value.v);
    case "json":
      return textLiteral(JSON.stringify(value.v));
    case "text":
    case "bytes":
    case "date_time":
    case "unsupported":
      return textLiteral(value.v);
  }
}

/// One INSERT per row, with the column list spelled out.
export function toInsertSql(engine: Engine, table: string, data: ResultRows): string {
  const target = quoteTable(engine, table);
  const names = data.columns.map((column) => quoteIdent(engine, column.name)).join(", ");
  return data.rows
    .map((row) => `INSERT INTO ${target} (${names}) VALUES (${data.columns.map((_, i) => sqlLiteral(row[i])).join(", ")});`)
    .join("\n");
}

export function resultText(format: ResultFormat, data: ResultRows, target: { engine: Engine; table: string }): string {
  switch (format) {
    case "csv":
      return toCsvText(data.columns, data.rows);
    case "json":
      return toJsonText(data.columns, data.rows);
    case "sql":
      return toInsertSql(target.engine, target.table, data);
  }
}

// WHAT:  Hands text to the user as a file.
// WHY:   lib/export's downloader only knows CSV and JSON mime types; a .sql file
//        needs its own. Same Blob + anchor technique, so no backend round trip.
export function saveTextFile(filename: string, text: string, mime: string): void {
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
