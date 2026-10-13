// SOT: filter-operators, per-type-operator-list, filter-entry, filter-op-registry
import type { FilterOp, FilterRule } from "@/lib/bindings";
import { fieldKind, type FieldKind } from "@/lib/fields";

export interface FilterOpMeta {
  label: string;
  needsValue: boolean;
}

// WHAT:  Operator registry for the GUI filter builder and the typed filter bar,
//        keyed by the Rust enum.
// WHERE: src-tauri/src/model/query.rs (FilterOp), src-tauri/src/integrations/sql.rs
export const FILTER_OPS = {
  eq: { label: "equals", needsValue: true },
  ne: { label: "not equals", needsValue: true },
  gt: { label: ">", needsValue: true },
  gte: { label: "≥", needsValue: true },
  lt: { label: "<", needsValue: true },
  lte: { label: "≤", needsValue: true },
  contains: { label: "contains", needsValue: true },
  not_contains: { label: "not contains", needsValue: true },
  starts_with: { label: "starts with", needsValue: true },
  ends_with: { label: "ends with", needsValue: true },
  in: { label: "in list (a, b, c)", needsValue: true },
  is_null: { label: "is null", needsValue: false },
  is_not_null: { label: "is not null", needsValue: false },
} satisfies Record<FilterOp, FilterOpMeta>;

export const ALL_OPS: readonly FilterOp[] = ["eq", "ne", "gt", "gte", "lt", "lte", "contains", "not_contains", "starts_with", "ends_with", "in", "is_null", "is_not_null"];

const NULLS: readonly FilterOp[] = ["is_null", "is_not_null"];

const BY_KIND = {
  text: ["eq", "ne", "contains", "not_contains", "starts_with", "ends_with", "in", ...NULLS],
  enum: ["eq", "ne", "in", ...NULLS],
  int: ["eq", "ne", "gt", "gte", "lt", "lte", "in", ...NULLS],
  float: ["eq", "ne", "gt", "gte", "lt", "lte", "in", ...NULLS],
  decimal: ["eq", "ne", "gt", "gte", "lt", "lte", "in", ...NULLS],
  date: ["eq", "ne", "gt", "gte", "lt", "lte", ...NULLS],
  time: ["eq", "ne", "gt", "gte", "lt", "lte", ...NULLS],
  datetime: ["eq", "ne", "gt", "gte", "lt", "lte", ...NULLS],
  bool: ["eq", "ne", ...NULLS],
  json: ["contains", "not_contains", ...NULLS],
  array: ["contains", "not_contains", ...NULLS],
  vector: [...NULLS],
  bytes: [...NULLS],
} satisfies Record<FieldKind, readonly FilterOp[]>;

// WHAT:  The operators that make sense for a column of this type.
// WHY:   "starts with" on a boolean or ">" on a blob is noise in the picker; an
//        type the registry does not know keeps the whole list rather than hiding something useful.
export function opsForType(typeName: string | undefined): readonly FilterOp[] {
  if (typeName === undefined || typeName.length === 0) return ALL_OPS;
  return BY_KIND[fieldKind(typeName)];
}

/// One filter rule and whether it currently applies.
export interface FilterEntry {
  rule: FilterRule;
  enabled: boolean;
}

/// The rules that apply: enabled and complete (a value-taking operator has a value).
export function activeRules(entries: readonly FilterEntry[]): FilterRule[] {
  return entries.filter((e) => e.enabled && e.rule.column.length > 0 && (!FILTER_OPS[e.rule.op].needsValue || e.rule.value.trim().length > 0)).map((e) => e.rule);
}

export function isFilterOp(value: string): value is FilterOp {
  return value in FILTER_OPS;
}
