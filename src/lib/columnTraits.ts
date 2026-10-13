// SOT: column-traits-helpers, required-column, read-only-column, insert-placeholder
import type { ColumnInfo, ColumnTraits } from "./bindings";

/// A column with nothing known about it beyond the catalog.
const NONE: ColumnTraits = { name: "", enumValues: [], multi: false, hasDefault: false, defaultExpr: null, generated: false, autoIncrement: false };

export function traitsOf(traits: readonly ColumnTraits[], name: string): ColumnTraits {
  return traits.find((t) => t.name === name) ?? { ...NONE, name };
}

/// The database computes this column, so the grid never writes it.
export function isReadOnlyColumn(traits: ColumnTraits): boolean {
  return traits.generated;
}

/// An insert must name this column: NOT NULL with nothing to fill it in.
export function isRequired(column: Pick<ColumnInfo, "nullable">, traits: ColumnTraits): boolean {
  return !column.nullable && !traits.hasDefault && !traits.autoIncrement && !traits.generated;
}

/// What the insert form says for a column left blank: the database's own answer.
export function blankPlaceholder(column: Pick<ColumnInfo, "nullable" | "dataType">, traits: ColumnTraits): string {
  if (traits.autoIncrement) return "Auto";
  if (traits.generated) return "Generated";
  if (traits.hasDefault) return traits.defaultExpr === null || traits.defaultExpr.length > 24 ? "DEFAULT" : `DEFAULT ${traits.defaultExpr}`;
  return column.nullable ? "NULL" : column.dataType;
}
