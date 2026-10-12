// SOT: metric-tile-stats, metric-comparison, metric-change
import type { QueryOutcome } from "@/lib/bindings";
import { cellText, rowsOf, toNumber } from "./cells";
import { relativeChange } from "./format";

// WHAT:  What a metric tile shows from its result: the headline value, the
//        value it is compared with, and how far it moved.
// WHY:   Two shapes of query feed a metric. `SELECT revenue, last_month FROM …`
//        is one row whose first number is the headline and whose second is the
//        base to compare with. `SELECT total FROM daily ORDER BY day` is a
//        series: the latest value is the headline, the one before is the base,
//        and the series draws as a strip under the number.
// HOW:   Numeric columns are those whose first-row cell is a number (text that
//        parses counts). Two or more: first row, first two. Exactly one: the
//        column down all rows. None: no value, the tile shows a dash.
// WHERE: ./appearance.ts, ./charts.tsx (MetricTile)

export interface MetricStats {
  /// The headline; null when the result holds no number.
  value: number | null;
  /// The base the change is measured against.
  compare: number | null;
  /// `relativeChange(value, compare)`, null without a base.
  change: number | null;
  /// The series behind a one-column result with several rows.
  series: { labels: string[]; values: number[] } | null;
}

const EMPTY: MetricStats = { value: null, compare: null, change: null, series: null };

export function metricStats(outcome: QueryOutcome | null): MetricStats {
  const set = rowsOf(outcome);
  const first = set?.rows[0];
  if (set === null || first === undefined) return EMPTY;
  const numeric = set.columns.map((_, i) => i).filter((i) => Number.isFinite(toNumber(first[i])));
  const [a, b] = numeric;
  if (a !== undefined && b !== undefined) {
    const value = toNumber(first[a]);
    const compare = toNumber(first[b]);
    return { value, compare, change: relativeChange(value, compare), series: null };
  }
  if (a === undefined) return EMPTY;
  const labelAt = set.columns.findIndex((_, i) => i !== a && !Number.isFinite(toNumber(first[i])));
  const values = set.rows.map((r) => toNumber(r[a]));
  const labels = set.rows.map((r, i) => (labelAt >= 0 ? cellText(r[labelAt]) : String(i + 1)));
  const finite = values.map((v, i) => ({ v, i })).filter((p) => Number.isFinite(p.v));
  const last = finite.at(-1);
  const before = finite.at(-2);
  if (last === undefined) return EMPTY;
  const value = last.v;
  const compare = before?.v ?? null;
  return {
    value,
    compare,
    change: compare === null ? null : relativeChange(value, compare),
    series: finite.length > 1 ? { labels: finite.map((p) => labels[p.i] ?? ""), values: finite.map((p) => p.v) } : null,
  };
}
