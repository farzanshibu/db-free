// SOT: ddl-form-state, ddl-form-validation, ddl-spec-conversion, ddl-column-locks, ddl-identifier-rule
import type { ColumnDefault, ColumnSpec, DdlProfile, DdlType, FkAction, ForeignKeySpec, IndexSpec, TableRef, TableSpec, TypeParams } from "@/lib/bindings";

// ============================================================================
// SCHEMA FORM STATE (pure)
//
// WHAT:  The editable shape of a table while a schema form is open, its
//        conversion to and from the `TableSpec` the Rust builders take, the
//        client-side validation that keeps the live preview quiet until a form
//        is complete, and the rules for which controls an engine lets you touch.
// WHY:   The forms (create / edit / duplicate) share one column grid. Keeping the
//        state and the rules in a React-free module means they are small enough to
//        read in one sitting, and the components only render what this decides.
//        No SQL is built here: a draft becomes a `TableSpec` and Rust writes the
//        statements.
// HOW:   Each row carries a stable client `key` (React identity), separate from
//        `originalName` (the live column's name, which is how Rust tells a rename
//        from a drop + add). `toTableSpec(draft)` leaves removed rows out, so the
//        spec is exactly "the table as the form leaves it".
// WHERE: src/features/schema/forms/*, src-tauri/src/model/ddl.rs (the spec types)
// ============================================================================

/// A table name, column name or schema name the forms accept for new objects.
/// Quoting makes other names legal on the server, but the forms stay strict (a
/// name that needs quoting everywhere is a trap); existing names are never re-checked.
export const IDENT_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

export type DefaultKind = ColumnDefault["kind"];

export const DEFAULT_KINDS: readonly { value: DefaultKind; label: string }[] = [
  { value: "none", label: "No default" },
  { value: "null", label: "NULL" },
  { value: "now", label: "NOW()" },
  { value: "empty", label: "Empty string" },
  { value: "custom", label: "Custom…" },
];

export const FK_ACTIONS: readonly { value: FkAction; label: string }[] = [
  { value: "no_action", label: "NO ACTION" },
  { value: "restrict", label: "RESTRICT" },
  { value: "cascade", label: "CASCADE" },
  { value: "set_null", label: "SET NULL" },
  { value: "set_default", label: "SET DEFAULT" },
];

let keyCounter = 0;

/// A client-only identity for a row, unique for the life of the page.
export function nextKey(prefix: string): string {
  keyCounter += 1;
  return `${prefix}-${keyCounter}`;
}

export interface ColumnDraft {
  key: string;
  /// The live column's name; null for a column being added.
  originalName: string | null;
  name: string;
  /// Base type: a name from the profile's list, or free text when `customType`.
  dataType: string;
  /// The type is typed by hand ("Other…") rather than picked from the list.
  customType: boolean;
  length: number | null;
  precision: number | null;
  scale: number | null;
  wrapper: string | null;
  /// Comma separated; parsed into `enumValues` by `toColumnSpec`.
  enumText: string;
  nullable: boolean;
  primaryKey: boolean;
  unique: boolean;
  uniqueName: string | null;
  autoIncrement: boolean;
  unsigned: boolean;
  defaultKind: DefaultKind;
  defaultExpression: string;
  /// Edit: marked for removal (shown struck through, left out of the spec).
  removed: boolean;
}

export interface ForeignKeyDraft {
  key: string;
  /// The live constraint's name; null for one being added.
  originalName: string | null;
  /// Empty = let the engine pick.
  name: string;
  columns: string[];
  /// Points at the table being created / edited itself (a parent_id key).
  refSelf: boolean;
  refSchema: string | null;
  refTable: string;
  refColumns: string[];
  onDelete: FkAction;
  onUpdate: FkAction;
  removed: boolean;
}

export interface TableDraft {
  schema: string | null;
  name: string;
  columns: ColumnDraft[];
  foreignKeys: ForeignKeyDraft[];
  /// Read from the live table, kept so an Edit that rebuilds the table can recreate them.
  indexes: IndexSpec[];
  primaryKeyName: string | null;
  engine: string | null;
  orderBy: string[];
  ifNotExists: boolean;
}

// ---- profile lookups ------------------------------------------------------------

export function findType(profile: DdlProfile, name: string): DdlType | undefined {
  const lower = name.trim().toLowerCase();
  return profile.types.find((t) => t.name.toLowerCase() === lower);
}

/// The engine has columns to edit (a collection store has only a name).
export function hasColumns(profile: DdlProfile): boolean {
  return profile.types.length > 0;
}

/// Which extra inputs the column's type asks for.
export function paramsOf(profile: DdlProfile, column: ColumnDraft): TypeParams {
  if (column.customType) return "none";
  return findType(profile, column.dataType)?.params ?? "none";
}

export function allowsAutoIncrement(profile: DdlProfile, column: ColumnDraft): boolean {
  if (!profile.support.autoIncrement || column.customType) return false;
  return findType(profile, column.dataType)?.autoIncrement === true;
}

export function allowsUnsigned(profile: DdlProfile, column: ColumnDraft): boolean {
  if (column.customType) return false;
  return findType(profile, column.dataType)?.unsigned === true;
}

/// "table" / "collection": the engine's word, lower-cased for running text.
export function nounOf(profile: DdlProfile): string {
  return profile.noun.toLowerCase();
}

export function qualifiedLabel(ref: TableRef): string {
  return ref.schema === null || ref.schema === "" ? ref.name : `${ref.schema}.${ref.name}`;
}

// ---- building rows --------------------------------------------------------------

export function parseEnumText(text: string): string[] {
  return text
    .split(",")
    .map((v) => v.trim())
    .filter((v) => v !== "");
}

// WHAT:  Picks a type from the list and resets everything that belonged to the
//        previous one (length, precision, enum values, unsigned, auto-increment).
// WHY:   A leftover `255` on an `integer`, or an auto-increment flag on a `text`
//        column, would reach the builder as a mistake the user cannot see.
export function pickType(profile: DdlProfile, column: ColumnDraft, name: string): ColumnDraft {
  const type = findType(profile, name);
  const params = type?.params ?? "none";
  return {
    ...column,
    dataType: type?.name ?? name,
    customType: false,
    length: params === "length" ? (column.length ?? 255) : null,
    precision: params === "precision" ? column.precision : params === "precision_scale" ? (column.precision ?? 10) : null,
    scale: params === "precision_scale" ? (column.scale ?? 2) : null,
    enumText: params === "values" ? column.enumText : "",
    unsigned: type?.unsigned === true ? column.unsigned : false,
    autoIncrement: type?.autoIncrement === true && profile.support.autoIncrement ? column.autoIncrement : false,
  };
}

/// "Other…": the type is typed by hand, with no parameters of its own.
export function toCustomType(column: ColumnDraft): ColumnDraft {
  return { ...column, customType: true, length: null, precision: null, scale: null, enumText: "", unsigned: false, autoIncrement: false };
}

export function blankColumn(profile: DdlProfile, name = ""): ColumnDraft {
  const base: ColumnDraft = {
    key: nextKey("col"),
    originalName: null,
    name,
    dataType: "",
    customType: false,
    length: null,
    precision: null,
    scale: null,
    wrapper: null,
    enumText: "",
    nullable: true,
    primaryKey: false,
    unique: false,
    uniqueName: null,
    autoIncrement: false,
    unsigned: false,
    defaultKind: "none",
    defaultExpression: "",
    removed: false,
  };
  return pickType(profile, base, profile.defaultType);
}

/// The first column of a new table: `id`, the key type, primary key, auto-increment where the engine has it.
export function keyColumn(profile: DdlProfile): ColumnDraft {
  const typed = pickType(profile, blankColumn(profile, "id"), profile.keyType);
  return { ...typed, nullable: false, primaryKey: true, autoIncrement: allowsAutoIncrement(profile, typed) };
}

export function blankForeignKey(columns: readonly string[]): ForeignKeyDraft {
  return {
    key: nextKey("fk"),
    originalName: null,
    name: "",
    columns: columns.length > 0 && columns[0] !== undefined ? [columns[0]] : [],
    refSelf: false,
    refSchema: null,
    refTable: "",
    refColumns: [],
    onDelete: "no_action",
    onUpdate: "no_action",
    removed: false,
  };
}

export function blankDraft(profile: DdlProfile, schema: string | null): TableDraft {
  return {
    schema,
    name: "",
    columns: hasColumns(profile) ? [keyColumn(profile)] : [],
    foreignKeys: [],
    indexes: [],
    primaryKeyName: null,
    engine: profile.support.tableEngine ? (profile.engines[0] ?? null) : null,
    orderBy: [],
    ifNotExists: false,
  };
}

// ---- spec -> draft ---------------------------------------------------------------

function defaultParts(value: ColumnDefault): { kind: DefaultKind; expression: string } {
  return value.kind === "custom" ? { kind: "custom", expression: value.expression } : { kind: value.kind, expression: "" };
}

export function fromColumnSpec(profile: DdlProfile, spec: ColumnSpec): ColumnDraft {
  const known = findType(profile, spec.dataType) !== undefined;
  const parts = defaultParts(spec.default);
  return {
    key: nextKey("col"),
    originalName: spec.originalName,
    name: spec.name,
    dataType: spec.dataType,
    customType: !known,
    length: spec.length,
    precision: spec.precision,
    scale: spec.scale,
    wrapper: spec.wrapper,
    enumText: spec.enumValues.join(", "),
    nullable: spec.nullable,
    primaryKey: spec.primaryKey,
    unique: spec.unique,
    uniqueName: spec.uniqueName,
    autoIncrement: spec.autoIncrement,
    unsigned: spec.unsigned,
    defaultKind: parts.kind,
    defaultExpression: parts.expression,
    removed: false,
  };
}

export function fromForeignKeySpec(spec: ForeignKeySpec): ForeignKeyDraft {
  return {
    key: nextKey("fk"),
    originalName: spec.originalName,
    name: spec.name ?? "",
    columns: spec.columns,
    // A described key keeps pointing at the name it was read with: the server
    // follows a table rename on its own, and re-pointing it would force a rewrite.
    refSelf: false,
    refSchema: spec.refTable.schema,
    refTable: spec.refTable.name,
    refColumns: spec.refColumns,
    onDelete: spec.onDelete,
    onUpdate: spec.onUpdate,
    removed: false,
  };
}

export function fromTableSpec(profile: DdlProfile, spec: TableSpec): TableDraft {
  return {
    schema: spec.table.schema,
    name: spec.table.name,
    columns: spec.columns.map((c) => fromColumnSpec(profile, c)),
    foreignKeys: spec.foreignKeys.map(fromForeignKeySpec),
    indexes: spec.indexes,
    primaryKeyName: spec.primaryKeyName,
    engine: spec.engine,
    orderBy: spec.orderBy,
    ifNotExists: false,
  };
}

// ---- draft -> spec ---------------------------------------------------------------

function toDefault(column: ColumnDraft): ColumnDefault {
  switch (column.defaultKind) {
    case "none":
      return { kind: "none" };
    case "null":
      return { kind: "null" };
    case "now":
      return { kind: "now" };
    case "empty":
      return { kind: "empty" };
    case "custom":
      return { kind: "custom", expression: column.defaultExpression.trim() };
  }
}

export function toColumnSpec(profile: DdlProfile, column: ColumnDraft): ColumnSpec {
  const wrapper = column.wrapper !== null && profile.wrappers.includes(column.wrapper) ? column.wrapper : null;
  return {
    name: column.name,
    originalName: column.originalName,
    dataType: column.dataType.trim(),
    length: column.length,
    precision: column.precision,
    scale: column.scale,
    wrapper,
    enumValues: paramsOf(profile, column) === "values" ? parseEnumText(column.enumText) : [],
    nullable: column.nullable,
    primaryKey: column.primaryKey,
    unique: column.unique,
    uniqueName: column.uniqueName,
    autoIncrement: column.autoIncrement,
    unsigned: column.unsigned,
    default: toDefault(column),
  };
}

export function toForeignKeySpec(draft: TableDraft, fk: ForeignKeyDraft): ForeignKeySpec {
  const name = fk.name.trim();
  return {
    name: name === "" ? null : name,
    originalName: fk.originalName,
    columns: fk.columns,
    refTable: fk.refSelf ? { schema: draft.schema, name: draft.name } : { schema: fk.refSchema, name: fk.refTable },
    refColumns: fk.refColumns,
    onDelete: fk.onDelete,
    onUpdate: fk.onUpdate,
  };
}

/// The table as the form leaves it: removed rows are not in it.
export function toTableSpec(profile: DdlProfile, draft: TableDraft): TableSpec {
  return {
    table: { schema: draft.schema, name: draft.name },
    columns: draft.columns.filter((c) => !c.removed).map((c) => toColumnSpec(profile, c)),
    primaryKeyName: draft.primaryKeyName,
    foreignKeys: draft.foreignKeys.filter((f) => !f.removed).map((f) => toForeignKeySpec(draft, f)),
    indexes: draft.indexes,
    engine: draft.engine,
    orderBy: draft.orderBy,
    ifNotExists: draft.ifNotExists,
  };
}

/// True when the form's table differs from `baseline` (the draft it was opened with).
export function isDirty(profile: DdlProfile, baseline: TableDraft, current: TableDraft): boolean {
  return JSON.stringify(toTableSpec(profile, baseline)) !== JSON.stringify(toTableSpec(profile, current));
}

// WHAT:  Replaces the column list, carrying column renames into the references a
//        draft holds by name (new foreign keys, ClickHouse ORDER BY) and dropping
//        ORDER BY entries whose column is gone.
// WHY:   Foreign keys and ORDER BY name columns as text. Without this, renaming a
//        column in the grid would silently orphan the key that points at it.
// HOW:   Renames are found by row `key`, so a new name that happens to equal
//        another column's old name is not mistaken for a swap. Live foreign keys
//        (those with an `originalName`) are left alone: the server keeps them
//        pointing at the renamed column on its own.
export function withColumns(draft: TableDraft, next: ColumnDraft[]): TableDraft {
  const renames = new Map<string, string>();
  for (const column of next) {
    const before = draft.columns.find((c) => c.key === column.key);
    if (before !== undefined && before.name !== column.name) renames.set(before.name, column.name);
  }
  const follow = (names: string[]): string[] => names.map((n) => renames.get(n) ?? n);
  const alive = new Set(next.filter((c) => !c.removed).map((c) => c.name));
  return {
    ...draft,
    columns: next,
    foreignKeys: draft.foreignKeys.map((f) => (f.originalName === null ? { ...f, columns: follow(f.columns), refColumns: f.refSelf ? follow(f.refColumns) : f.refColumns } : f)),
    orderBy: follow(draft.orderBy).filter((n) => alive.has(n)),
  };
}
