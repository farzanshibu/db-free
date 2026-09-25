// SOT: chart-widget-sql, chart-projection-sql, chart-pivot-sql, sql-identifier-quoting, widget-factory, widget-defaults
import type { Engine, Value, Widget } from "@/lib/bindings";
import { ENGINE_FACTS } from "@/lib/bindings/EngineFacts.gen";
import { chartGroups, isNumericColumn, OTHER_SERIES, type ChartMapping } from "./charts";

// WHAT:  The SQL a dashboard widget runs so it draws what the results chart drew.
// WHY:   A widget stores only SQL and infers its axes (`chartData`: first text
//        column = X, every numeric column = a series). Saving the user's query
//        verbatim would re-infer and lose the columns they picked, so the query
//        is wrapped in a projection, or a pivot when a series column is chosen.
// HOW:   Unchanged when the query already has the chosen shape. Otherwise
//        `SELECT x, y… FROM (<query>) chart_source`; a numeric X is cast to
//        text so it stays the label column. A series column becomes one
//        `SUM(CASE WHEN g = '…' THEN y END)` per kept group (the same groups,
//        and the same "Other" fold, as the on-screen chart), grouped by X.
//        Identifiers are quoted per family: backticks for MySQL-compatible
//        engines, double quotes everywhere else.
// WHERE: ./charts.tsx (chartData, mapChartData), src/features/editor/ResultChart.tsx

function isMysql(engine: Engine): boolean {
  return ENGINE_FACTS[engine].family === "mysql";
}

export function quoteIdent(engine: Engine, name: string): string {
  return isMysql(engine) ? `\`${name.replace(/`/g, "``")}\`` : `"${name.replace(/"/g, '""')}"`;
}

function textLiteral(text: string): string {
  return `'${text.replace(/'/g, "''")}'`;
}

export function widgetSql(engine: Engine, sql: string, columns: readonly string[], rows: readonly (readonly Value[])[], mapping: ChartMapping): string {
  const base = sql.trim().replace(/;+\s*$/, "").trim();
  const numeric = columns.map((_, i) => isNumericColumn(rows, i));
  const q = (i: number) => quoteIdent(engine, columns[i] ?? "");
  const ys = mapping.ys.slice(0, 7);
  const xNumeric = numeric[mapping.x] ?? false;

  if (mapping.group === null) {
    // Already the inferred shape: X is the first text column and the Ys are every numeric column, in order.
    const inferredYs = numeric.map((n, i) => (n ? i : -1)).filter((i) => i >= 0).slice(0, 7);
    if (!xNumeric && numeric.findIndex((n) => !n) === mapping.x && inferredYs.join(",") === ys.join(",")) return base;
  }

  const cast = isMysql(engine) ? "CHAR" : "VARCHAR(255)";
  const xSelect = xNumeric ? `CAST(${q(mapping.x)} AS ${cast}) AS ${quoteIdent(engine, `${columns[mapping.x] ?? "x"}_label`)}` : q(mapping.x);
  const source = `FROM (\n${base}\n) chart_source`;

  if (mapping.group === null) return `SELECT ${[xSelect, ...ys.map(q)].join(", ")}\n${source}`;

  const y = ys[0];
  if (y === undefined) return base;
  const g = q(mapping.group);
  const { kept, other } = chartGroups(rows, mapping.group, y);
  const match = (name: string) => (name === "NULL" ? `${g} IS NULL` : `${g} = ${textLiteral(name)}`);
  const series = kept.map((name) => `SUM(CASE WHEN ${match(name)} THEN ${q(y)} END) AS ${quoteIdent(engine, name)}`);
  if (other) series.push(`SUM(CASE WHEN ${kept.map(match).join(" OR ")} THEN NULL ELSE ${q(y)} END) AS ${quoteIdent(engine, OTHER_SERIES)}`);
  return `SELECT ${[xSelect, ...series].join(",\n  ")}\n${source}\nGROUP BY ${q(mapping.x)}\nORDER BY ${q(mapping.x)}`;
}

let widgetCounter = 0;

// WHAT:  A widget with every option at its default, plus the caller's fields.
// WHY:   The dashboard's "+ Widget" and the results chart's "Add to dashboard"
//        must create the same shape; the defaults live once.
export function newWidget(fields: Partial<Widget>): Widget {
  widgetCounter += 1;
  return {
    id: `w-${Date.now().toString(36)}-${widgetCounter}`,
    title: "",
    kind: "line",
    sql: "",
    x: 0,
    y: 0,
    w: 4,
    h: 3,
    tint: "series-1",
    showChange: false,
    maxValue: null,
    text: null,
    url: null,
    xLabel: null,
    yLabel: null,
    horizontal: false,
    showPercent: true,
    showValues: false,
    pulse: false,
    conditions: [],
    ...fields,
  };
}
