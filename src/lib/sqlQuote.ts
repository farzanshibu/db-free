// SOT: sql-qualified-name, sql-identifier-quote-by-family, create-table-sketch
import type { ColumnInfo, Engine, TableRef } from "./bindings";
import { ENGINE_FACTS } from "./bindings/EngineFacts.gen";

// WHAT:  Identifier quoting and a CREATE TABLE sketch for the SQL the sidebar
//        writes into the clipboard or a query tab (SELECT *, COUNT(*), copy schema).
// WHY:   `schema.name` unquoted breaks the moment a name has a capital, a space or
//        a reserved word. The engine's own spelling of a quote is a property of its
//        adapter family, so it is read from the generated engine facts rather than
//        guessed per call site.
// HOW:   Backticks for the MySQL family and BigQuery, brackets for SQL Server, ANSI
//        double quotes for everything else. The doubled-quote escape keeps a name
//        that contains the quote character intact. Rust owns the statements that
//        *run* (`integrations::ddl`); this is only for text the user reads or edits.
// WHERE: src/features/shell/TablesPanel.tsx, src-tauri/src/integrations/mod.rs (`quote_ident_for`)

export function quoteIdentifier(engine: Engine, name: string): string {
  switch (ENGINE_FACTS[engine].family) {
    case "mysql":
      return `\`${name.replace(/`/g, "``")}\``;
    case "bigquery":
      return `\`${name.replace(/`/g, "\\`")}\``;
    case "mssql":
      return `[${name.replace(/\]/g, "]]")}]`;
    default:
      return `"${name.replace(/"/g, '""')}"`;
  }
}

/// `schema.name`, each part quoted; a table without a schema is just its name.
export function qualifiedIdentifier(engine: Engine, table: TableRef): string {
  const name = quoteIdentifier(engine, table.name);
  return table.schema === null ? name : `${quoteIdentifier(engine, table.schema)}.${name}`;
}

// WHAT:  The columns as a CREATE TABLE statement, for engines that cannot give
//        their own DDL. Types are copied as the catalog spelled them.
export function createTableSketch(engine: Engine, table: TableRef, columns: readonly ColumnInfo[]): string {
  const keys = columns.filter((c) => c.primaryKey);
  // One key column reads best inline; a composite key can only be a table constraint.
  const inline = keys.length === 1;
  const lines = columns.map((c) => `  ${quoteIdentifier(engine, c.name)} ${c.dataType}${c.nullable ? "" : " NOT NULL"}${inline && c.primaryKey ? " PRIMARY KEY" : ""}`);
  if (keys.length > 1) lines.push(`  PRIMARY KEY (${keys.map((c) => quoteIdentifier(engine, c.name)).join(", ")})`);
  return `CREATE TABLE ${qualifiedIdentifier(engine, table)} (\n${lines.join(",\n")}\n);`;
}
