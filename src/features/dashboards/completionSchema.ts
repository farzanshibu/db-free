// SOT: tile-sql-completion-schema, mentioned-tables, column-aware-completion
import type { SQLNamespace } from "@codemirror/lang-sql";
import type { ColumnInfo, SchemaCatalog, TableRef } from "@/lib/bindings";

// WHAT:  What a tile's SQL editor completes against: every table in the
//        catalogue, with real column names for the tables the statement already
//        mentions.
// WHY:   The inspector's editor knew table names only (empty column lists), so
//        `SELECT o.` offered nothing. Columns cost a round trip per table, so
//        they are fetched for the tables being written about, not all of them.
// HOW:   `mentionedTables` reads the identifiers after FROM / JOIN / UPDATE /
//        INTO, forgives quoting, and matches them to catalogue tables by bare
//        name or `schema.name`. The caller loads their columns into the
//        workspace cache; `buildCompletionSchema` then reads the cache. The
//        namespace has the shape the editor's own completion walks: `{table:
//        [columns]}`, nested by schema where the catalogue has schemas.
// WHERE: ./TileInspector.tsx, src/features/editor/SqlEditor.tsx (completion),
//        src/stores/workspace.ts (loadColumns, columnsCache)

const MAX_TABLES = 6;
/// One name part: "quoted", `quoted`, [quoted] or a bare identifier.
const PART = String.raw`(?:"[^"]+"|` + "`[^`]+`" + String.raw`|\[[^\]]+\]|[A-Za-z_][\w$]*)`;
const REFERENCE = new RegExp(String.raw`\b(?:from|join|update|into)\s+(${PART}(?:\s*\.\s*${PART})*)`, "gi");
const PARTS = new RegExp(PART, "g");

function keyOf(connectionId: string, table: TableRef): string {
  return `${connectionId}:${table.schema === null ? table.name : `${table.schema}.${table.name}`}`;
}

function unquoted(part: string): string {
  return part.replace(/^["`[]|["`\]]$/g, "");
}

export function mentionedTables(sql: string, catalog: SchemaCatalog | undefined): TableRef[] {
  if (catalog === undefined) return [];
  const all = catalog.schemas.flatMap((s) => s.tables);
  const found: TableRef[] = [];
  for (const hit of sql.matchAll(REFERENCE)) {
    const parts = (hit[1]?.match(PARTS) ?? []).map(unquoted);
    const name = (parts.at(-1) ?? "").toLowerCase();
    const schema = parts.length > 1 ? (parts.at(-2) ?? "").toLowerCase() : null;
    if (name.length === 0) continue;
    for (const table of all) {
      if (table.name.toLowerCase() !== name) continue;
      if (schema !== null && (table.schema ?? "").toLowerCase() !== schema) continue;
      if (!found.some((f) => f.schema === table.schema && f.name === table.name)) found.push({ schema: table.schema, name: table.name });
    }
    if (found.length >= MAX_TABLES) break;
  }
  return found.slice(0, MAX_TABLES);
}

export function buildCompletionSchema(connectionId: string, catalog: SchemaCatalog | undefined, columns: Readonly<Record<string, ColumnInfo[]>>): SQLNamespace {
  if (catalog === undefined) return {};
  const out: Record<string, Record<string, string[]> | string[]> = {};
  for (const schema of catalog.schemas) {
    const tables: Record<string, string[]> = {};
    for (const table of schema.tables) tables[table.name] = (columns[keyOf(connectionId, table)] ?? []).map((c) => c.name);
    if (schema.tables.some((t) => t.schema !== null)) {
      out[schema.name] = tables;
    } else {
      for (const [name, cols] of Object.entries(tables)) out[name] = cols;
    }
  }
  return out;
}
