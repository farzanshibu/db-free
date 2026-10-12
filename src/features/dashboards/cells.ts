// SOT: result-cell-reading, statement-rows-guard, numeric-column-detection
import type { QueryOutcome, ResultSet, StatementResult, Value } from "@/lib/bindings";
import { formatCell } from "@/lib/format";

// WHAT:  Reading a query result the way a tile does: the rows statement, a cell
//        as text, a cell as a number, the first cell of the first row.
// WHY:   Every tile (metric, text, progress, conditions) starts from these few
//        reads; keeping them in a module with no React in it lets the logic
//        around them (change, conditions, templates) be exercised on its own.
// WHERE: ./charts.tsx, ./metric.ts, ./conditions.ts, ./template.ts

export function isRows(s: StatementResult): s is Extract<StatementResult, { kind: "rows" }> {
  return s.kind === "rows";
}

/// The first statement that returned rows.
export function rowsOf(outcome: QueryOutcome | null): ResultSet | null {
  const found = outcome?.statements.find(isRows);
  return found === undefined ? null : found.result;
}

export function cellText(v: Value | undefined): string {
  return v === undefined ? "" : v.t === "null" ? "" : formatCell(v).text;
}

/// NaN when the cell is missing or not a number.
export function toNumber(v: Value | undefined): number {
  if (!v) return NaN;
  if (v.t === "int" || v.t === "float") return v.v;
  if (v.t === "decimal" || v.t === "text") return v.v.trim().length === 0 ? NaN : Number(v.v);
  if (v.t === "bool") return v.v ? 1 : 0;
  return NaN;
}

export function isNumericColumn(rows: readonly (readonly Value[])[], i: number): boolean {
  const first = rows.find((r) => {
    const v = r[i];
    return v !== undefined && v.t !== "null";
  })?.[i];
  return first !== undefined && (first.t === "int" || first.t === "float" || first.t === "decimal");
}

/// The first column of the first row, which is what conditions read.
export function firstCell(outcome: QueryOutcome | null): Value | undefined {
  return rowsOf(outcome)?.rows[0]?.[0];
}

/// Whether the result has a first row at all (a NULL cell still counts).
export function hasFirstRow(outcome: QueryOutcome | null): boolean {
  return (rowsOf(outcome)?.rows.length ?? 0) > 0;
}
