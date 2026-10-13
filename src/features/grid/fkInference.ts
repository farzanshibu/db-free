// SOT: fk-inference, foreign-key-resolution, fk-link, name-based-foreign-keys
import type { ColumnInfo, FkRemap, ForeignKey, SchemaCatalog, TableRef } from "@/lib/bindings";

// WHAT:  Where a column's foreign key leads, from the best source available:
//        a link the user made, the database's own constraint, or a guess from the name.
// WHY:   Plenty of schemas have no FOREIGN KEY constraints (or never declared the
//        one on `orders.user_id`), yet `user_id` obviously means `users.id`.
//        Peeking at the referenced row should not depend on how the schema was written.
// HOW:   remap > constraint > inferred. Inference only proposes a table that exists in
//        the catalog and whose column exists (checked by the caller via `hasColumn`).
// WHERE: src/features/grid/TableTab.tsx, src/features/grid/fkStore.ts

export type FkSource = "remap" | "constraint" | "inferred";

export interface FkLink {
  table: TableRef;
  column: string;
  source: FkSource;
  /// The saved remap, when `source` is "remap" — so it can be changed or removed.
  remapId?: string | undefined;
}

const sameTable = (a: TableRef, b: TableRef): boolean => a.name === b.name && (a.schema ?? null) === (b.schema ?? null);

/// Single-column foreign keys leaving `table`, by source column.
export function constraintLinks(foreignKeys: readonly ForeignKey[], table: TableRef): Map<string, FkLink> {
  const links = new Map<string, FkLink>();
  for (const fk of foreignKeys) {
    if (fk.fromTable !== table.name || (fk.fromSchema ?? null) !== (table.schema ?? null) || fk.fromColumns.length !== 1) continue;
    const from = fk.fromColumns[0];
    const to = fk.toColumns[0];
    if (from !== undefined && to !== undefined) links.set(from, { table: { schema: fk.toSchema, name: fk.toTable }, column: to, source: "constraint" });
  }
  return links;
}

/// User-made links for columns of `table`.
export function remapLinks(remaps: readonly FkRemap[], table: TableRef): Map<string, FkLink> {
  const links = new Map<string, FkLink>();
  for (const remap of remaps) {
    if (sameTable(remap.from, table)) links.set(remap.fromColumn, { table: remap.to, column: remap.toColumn, source: "remap", remapId: remap.id });
  }
  return links;
}

/// `user_id` / `userId` / `UserID` -> "user"; anything else -> null.
export function stemOf(column: string): string | null {
  const snake = /^(.+?)_id$/i.exec(column);
  if (snake?.[1]) return snake[1];
  const camel = /^([a-z][a-z0-9]*(?:[A-Z][a-z0-9]*)*)Id$/.exec(column);
  if (camel?.[1]) return camel[1];
  return null;
}

/// `user` -> user, users; `category` -> category, categories; `box` -> box, boxes.
export function nameForms(stem: string): string[] {
  const lower = stem.toLowerCase();
  const plural = /[^aeiou]y$/.test(lower) ? `${lower.slice(0, -1)}ies` : /(s|x|z|ch|sh)$/.test(lower) ? `${lower}es` : `${lower}s`;
  return [lower, plural];
}

// WHAT:  The catalog tables a column like `user_id` might point at, best guess first.
// HOW:   Same schema first, then the rest; for each, the singular then the plural
//        name (snake-cased stems like `order_item` also match `orderitems`). A
//        Supabase-style `auth.users` is the answer for `user_id` when the project
//        has no `users` table of its own.
export function candidateTables(column: string, owner: TableRef, catalog: SchemaCatalog): TableRef[] {
  const all: TableRef[] = catalog.schemas.flatMap((s) => s.tables.map((t) => ({ schema: t.schema, name: t.name })));
  const out: TableRef[] = [];
  const stem = stemOf(column);
  if (stem !== null) {
    const forms = nameForms(stem.replace(/([a-z0-9])([A-Z])/g, "$1_$2"));
    const squashed = forms.map((f) => f.replace(/_/g, ""));
    const ordered = [...all.filter((t) => (t.schema ?? null) === (owner.schema ?? null)), ...all.filter((t) => (t.schema ?? null) !== (owner.schema ?? null))];
    for (const names of [forms, squashed]) {
      for (const table of ordered) {
        if (names.includes(table.name.toLowerCase()) && !sameTable(table, owner) && !out.some((o) => sameTable(o, table))) out.push(table);
      }
    }
  }
  // An auth schema with a users table is Supabase's account table.
  if (/^user_?id$/i.test(column) || /^(created|updated|owner)_by$/i.test(column)) {
    const auth = all.find((t) => t.schema === "auth" && t.name === "users");
    if (auth && !out.some((o) => sameTable(o, auth))) out.push(auth);
  }
  return out;
}

/// The column a guessed link should land on: `id` when the table has it, else its only primary key.
export function targetColumn(columns: readonly ColumnInfo[]): string | null {
  const id = columns.find((c) => c.name.toLowerCase() === "id");
  if (id) return id.name;
  const keys = columns.filter((c) => c.primaryKey);
  const only = keys.length === 1 ? keys[0] : undefined;
  return only ? only.name : null;
}

// WHAT:  Guessed links for the columns that have neither a constraint nor a remap.
// HOW:   `columnsOf` loads (and caches) a candidate table's columns; the first
//        candidate with a usable target column wins. A column that is itself the
//        primary key is not a reference to anything.
export async function inferLinks(
  owner: TableRef,
  columns: readonly ColumnInfo[],
  catalog: SchemaCatalog,
  known: ReadonlySet<string>,
  columnsOf: (table: TableRef) => Promise<readonly ColumnInfo[] | null>,
): Promise<Map<string, FkLink>> {
  const links = new Map<string, FkLink>();
  for (const column of columns) {
    if (known.has(column.name) || column.primaryKey) continue;
    for (const candidate of candidateTables(column.name, owner, catalog)) {
      const targetColumns = await columnsOf(candidate);
      const target = targetColumns === null ? null : targetColumn(targetColumns);
      if (target !== null) {
        links.set(column.name, { table: candidate, column: target, source: "inferred" });
        break;
      }
    }
  }
  return links;
}

/// remap > constraint > inferred, merged by source column.
export function mergeLinks(...layers: ReadonlyMap<string, FkLink>[]): Map<string, FkLink> {
  const merged = new Map<string, FkLink>();
  for (const layer of [...layers].reverse()) {
    for (const [column, link] of layer) merged.set(column, link);
  }
  return merged;
}
