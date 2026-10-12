// SOT: tile-text-template, column-placeholders, text-tile-arithmetic-substitution
import type { QueryOutcome } from "@/lib/bindings";
import { cellText, rowsOf, toNumber } from "./cells";
import { evaluateExpression, formatExpressionResult } from "./expression";

// WHAT:  `{{column}}` and `{{a / b * 100}}` in a tile's text replaced with the
//        first row's values.
// WHY:   Text tiles, image URLs and condition content all take placeholders. A
//        placeholder that is just a column name shows that cell; anything else
//        is read as arithmetic on the row, so a rate is `{{round(won / total * 100, 1)}}`
//        and needs no second query.
// HOW:   Placeholders are matched first as a whole column name (so a column
//        called `a-b` still works), then as an expression over the row's numeric
//        columns. A bare word naming no column is blank, as it always was; a
//        malformed expression is left in the text as typed so the mistake shows.
// WHERE: ./charts.tsx (TextWidget, ImageWidget), ./expression.ts

const PLACEHOLDER = /\{\{([^{}]+)\}\}/g;
const BARE_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

export function substitute(text: string, outcome: QueryOutcome | null): string {
  const set = rowsOf(outcome);
  if (set === null) return text;
  const first = set.rows[0] ?? [];
  const indexOf = new Map(set.columns.map((c, i) => [c.name, i]));
  return text.replace(PLACEHOLDER, (whole: string, raw: string) => {
    const expression = raw.trim();
    const column = indexOf.get(expression);
    if (column !== undefined) return cellText(first[column]);
    if (BARE_NAME.test(expression)) return "";
    const result = evaluateExpression(expression, (name) => {
      const at = indexOf.get(name);
      return at === undefined ? undefined : toNumber(first[at]);
    });
    return result === null ? whole : formatExpressionResult(result);
  });
}
