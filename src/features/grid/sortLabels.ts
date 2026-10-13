// SOT: sort-direction-labels, type-aware-sort-labels
import { fieldKind } from "@/lib/fields";

export interface SortLabels {
  asc: string;
  desc: string;
}

// WHAT:  What "ascending" and "descending" mean for a column of this type, in words.
// WHY:   "A → Z" and "Newest first" say what will happen; "ASC" makes the reader translate.
export function sortLabels(typeName: string | undefined): SortLabels {
  if (typeName === undefined || typeName.length === 0) return { asc: "Ascending", desc: "Descending" };
  switch (fieldKind(typeName)) {
    case "text":
    case "enum":
      return { asc: "A → Z", desc: "Z → A" };
    case "int":
    case "float":
    case "decimal":
      return { asc: "1 → 9", desc: "9 → 1" };
    case "date":
    case "time":
    case "datetime":
      return { asc: "Oldest first", desc: "Newest first" };
    case "bool":
      return { asc: "false → true", desc: "true → false" };
    case "json":
    case "bytes":
    case "array":
    case "vector":
      return { asc: "Ascending", desc: "Descending" };
  }
}
