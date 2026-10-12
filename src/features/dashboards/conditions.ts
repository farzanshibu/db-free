// SOT: widget-conditions, condition-operators, condition-matching
import type { ConditionOp, QueryOutcome, Value, WidgetCondition } from "@/lib/bindings";
import { cellText, firstCell, hasFirstRow, toNumber } from "./cells";

// WHAT:  The rules a metric, text, image or GIF tile checks against its result:
//        "equals", "greater than", "is null"... applied to the first column of
//        the first row, the last rule that matches wins.
// WHY:   One definition for the tile that draws the matched rule and the
//        inspector that highlights it live, so what the editor marks is what the
//        tile shows.
// WHERE: ./appearance.ts, ./ConditionsEditor.tsx, ./charts.tsx

export const CONDITION_OPS: readonly { value: ConditionOp; label: string }[] = [
  { value: "equals", label: "equals" },
  { value: "not_equals", label: "not equals" },
  { value: "gt", label: "greater than" },
  { value: "gte", label: "greater or equal" },
  { value: "lt", label: "less than" },
  { value: "lte", label: "less or equal" },
  { value: "contains", label: "contains" },
  { value: "is_null", label: "is null" },
  { value: "is_not_null", label: "is not null" },
];

/// `is null` / `is not null` compare nothing, so they take no value.
export function opNeedsValue(op: ConditionOp): boolean {
  return op !== "is_null" && op !== "is_not_null";
}

// WHAT:  Whether one rule matches the first cell.
// HOW:   Equality and ordering compare as numbers when both sides are numbers,
//        as text otherwise. A result with no rows matches nothing, not even
//        `is null`: there is no cell to be null.
export function conditionMatches(cond: WidgetCondition, cell: Value | undefined, hasRow = true): boolean {
  if (!hasRow) return false;
  const isNull = cell === undefined || cell.t === "null";
  if (cond.op === "is_null") return isNull;
  if (cond.op === "is_not_null") return !isNull;
  const text = cellText(cell);
  const num = toNumber(cell);
  const target = Number(cond.value);
  const bothNumeric = Number.isFinite(num) && Number.isFinite(target) && cond.value.trim().length > 0;
  switch (cond.op) {
    case "equals":
      return bothNumeric ? num === target : text === cond.value;
    case "not_equals":
      return bothNumeric ? num !== target : text !== cond.value;
    case "gt":
      return bothNumeric && num > target;
    case "gte":
      return bothNumeric && num >= target;
    case "lt":
      return bothNumeric && num < target;
    case "lte":
      return bothNumeric && num <= target;
    case "contains":
      return text.toLowerCase().includes(cond.value.toLowerCase());
  }
}

/// Index of the last rule that matches the result, or -1.
export function matchedIndex(conditions: readonly WidgetCondition[], outcome: QueryOutcome | null): number {
  const cell = firstCell(outcome);
  const hasRow = hasFirstRow(outcome);
  let found = -1;
  conditions.forEach((c, i) => {
    if (conditionMatches(c, cell, hasRow)) found = i;
  });
  return found;
}

export function matchedCondition(conditions: readonly WidgetCondition[], outcome: QueryOutcome | null): WidgetCondition | null {
  return conditions[matchedIndex(conditions, outcome)] ?? null;
}

/// A fresh rule for the editor's "Add Condition".
export function newCondition(content: string): WidgetCondition {
  return { op: "equals", value: "", content, bg: null };
}
