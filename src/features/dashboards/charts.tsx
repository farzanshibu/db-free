// SOT: dashboard-charts, chart-data-extraction, line-chart, area-chart, bar-chart, pie-chart, sankey, map-chart, progress-meter, metric-tile, stat-tile, text-widget, image-widget, widget-conditions, chart-tooltip, chart-legend
import { useId, useMemo, useState } from "react";
import type { LegendMode, QueryOutcome, Value, Widget } from "@/lib/bindings";
import { formatCount } from "@/lib/format";
import { cn } from "@/lib/cn";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useResolvedTheme } from "@/stores/useTheme";
import type { Appearance } from "./appearance";
import { cellText, isNumericColumn, isRows, rowsOf, toNumber } from "./cells";
import { matchedCondition } from "./conditions";
import { formatChange, formatMetricValue, legendVisible, relativeChange } from "./format";
import { SlippyMap } from "./SlippyMap";
import { parseMapPoints } from "./mapTiles";
import { substitute } from "./template";
import { ON_GRADIENT } from "./tiles";

// WHAT:  Inline-SVG charts for dashboard widgets, following the dataviz method:
//        2px lines, <=24px bars with 4px rounded data-ends and 2px surface gaps,
//        10% area wash, legend for >=2 series, hover tooltips, text in text tokens,
//        categorical hues assigned in fixed order (never cycled).
// WHY:   No chart library: small bundle, works offline, theme via CSS tokens.
// HOW:   Line, bar and pie take the optional presentation of the widget
//        (`legendMode`, `formatValue`, `stacked`); left out they draw as they
//        always did, which is what the chat's chart artifacts rely on.
// WHERE: src/styles/globals.css (--color-series-*), ./DashboardTab.tsx, src/features/chat/ArtifactView.tsx
export const SERIES_COLORS = ["var(--color-series-1)", "var(--color-series-2)", "var(--color-series-3)", "var(--color-series-4)", "var(--color-series-5)", "var(--color-series-6)", "var(--color-series-7)"] as const;

// Kept here so every earlier importer keeps working.
export { isNumericColumn, isRows } from "./cells";
export { conditionMatches } from "./conditions";
export { substitute } from "./template";

export interface ChartData {
  labels: string[];
  series: { name: string; values: number[] }[];
}

/// How a chart writes a number on its axes and in its tooltips.
type Formatter = (value: number) => string;

// WHAT:  First non-numeric column = x labels; every numeric column = one series.
//        A single numeric column with no label column is indexed 1..n.
export function chartData(outcome: QueryOutcome | null): ChartData {
  const rows = outcome?.statements.find(isRows);
  if (!rows) return { labels: [], series: [] };
  const cols = rows.result.columns;
  const numeric = cols.map((_, i) => isNumericColumn(rows.result.rows, i));
  const labelIndex = numeric.findIndex((n) => !n);
  const seriesIndexes = numeric.map((n, i) => (n ? i : -1)).filter((i) => i >= 0).slice(0, SERIES_COLORS.length);
  const labels = rows.result.rows.map((r, i) => (labelIndex >= 0 ? cellText(r[labelIndex]) : String(i + 1)));
  const series = seriesIndexes.map((i) => ({ name: cols[i]?.name ?? `Series ${i + 1}`, values: rows.result.rows.map((r) => toNumber(r[i])) }));
  return { labels, series };
}

/// Which result columns a chart reads: X labels, Y measures, optional series split.
export interface ChartMapping {
  x: number;
  ys: readonly number[];
  /// Column whose values become the series; null = one series per Y column.
  group: number | null;
}

/// Label of the series that collects every group past the palette's length.
export const OTHER_SERIES = "Other";

// WHAT:  Rows -> chart data for an explicit column choice (the query results
//        chart), where `chartData` infers one for dashboard widgets.
// HOW:   Without a group: one point per row, one series per Y (at most the
//        palette's length). With a group: X values in first-seen order, one
//        series per group value of the first Y (summed when a pair repeats);
//        groups past the sixth fold into "Other" rather than cycle a hue.
// WHERE: src/features/editor/ResultChart.tsx, ./widgetSql.ts (same shape as widget SQL)
export function mapChartData(rows: readonly (readonly Value[])[], columnNames: readonly string[], mapping: ChartMapping): ChartData {
  const label = (r: readonly Value[]) => groupName(r[mapping.x]);
  const ys = mapping.ys.slice(0, SERIES_COLORS.length);
  if (mapping.group === null) {
    return {
      labels: rows.map((r) => label(r)),
      series: ys.map((y) => ({ name: columnNames[y] ?? `Series ${y + 1}`, values: rows.map((r) => toNumber(r[y])) })),
    };
  }
  const y = ys[0];
  if (y === undefined) return { labels: [], series: [] };
  const group = mapping.group;
  const labels: string[] = [];
  const labelIndex = new Map<string, number>();
  for (const r of rows) {
    const l = label(r);
    if (!labelIndex.has(l)) {
      labelIndex.set(l, labels.length);
      labels.push(l);
    }
  }
  const { kept, other } = chartGroups(rows, group, y);
  const names = other ? [...kept, OTHER_SERIES] : kept;
  const values = names.map(() => labels.map(() => NaN));
  for (const r of rows) {
    const g = groupName(r[group]);
    const s = kept.includes(g) ? kept.indexOf(g) : names.length - 1;
    const at = labelIndex.get(label(r)) ?? -1;
    const v = toNumber(r[y]);
    const series = values[s];
    if (!series || at < 0 || !Number.isFinite(v)) continue;
    const prev = series[at] ?? NaN;
    series[at] = Number.isFinite(prev) ? prev + v : v;
  }
  return { labels, series: names.map((name, i) => ({ name, values: values[i] ?? [] })) };
}

// WHAT:  The group values that get their own series, in first-seen order, and
//        whether the rest fold into "Other".
// WHY:   Past the palette's seven hues a series would need a generated colour;
//        the six largest (by |sum| of Y) keep theirs and the tail is summed.
export function chartGroups(rows: readonly (readonly Value[])[], group: number, y: number): { kept: string[]; other: boolean } {
  const totals = new Map<string, number>();
  for (const r of rows) {
    const g = groupName(r[group]);
    const v = toNumber(r[y]);
    totals.set(g, (totals.get(g) ?? 0) + (Number.isFinite(v) ? Math.abs(v) : 0));
  }
  const groups = [...totals.keys()];
  if (groups.length <= SERIES_COLORS.length) return { kept: groups, other: false };
  const largest = new Set([...groups].sort((a, b) => (totals.get(b) ?? 0) - (totals.get(a) ?? 0)).slice(0, SERIES_COLORS.length - 1));
  return { kept: groups.filter((g) => largest.has(g)), other: true };
}

/// A group value as a series name; NULL is its own group.
export function groupName(v: Value | undefined): string {
  return v === undefined || v.t === "null" ? "NULL" : cellText(v);
}

function fmt(n: number): string {
  if (!Number.isFinite(n)) return "-";
  return Number.isInteger(n) ? formatCount(n) : n.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

// ---------------------------------------------------------------- conditions
// WHAT:  The content of the rule that matches the result ("Last match wins"),
//        or `fallback`, with {{column}} and {{a / b}} placeholders filled in.
export function matchedContent(widget: Widget, outcome: QueryOutcome | null, fallback: string): string {
  const matched = matchedCondition(widget.conditions, outcome);
  return substitute(matched === null ? fallback : matched.content, outcome);
}

// ---------------------------------------------------------------- shared bits
function Tooltip({ x, y, label, rows }: { x: number; y: number; label: string; rows: { name: string; value: string; color: string }[] }) {
  return (
    <div className="pointer-events-none absolute z-10 rounded-md border border-border bg-overlay px-2 py-1 text-[11px] text-foreground shadow" style={{ left: `${x}%`, top: `${y}%`, transform: "translate(-50%, -110%)" }}>
      <div className="mb-0.5 text-muted">{label}</div>
      {rows.map((r) => (
        <div key={r.name} className="flex items-center gap-1.5">
          <span className="size-2 rounded-sm" style={{ background: r.color }} />
          <span className="text-muted">{r.name}</span>
          <span className="ml-auto pl-2.5 tabular-nums">{r.value}</span>
        </div>
      ))}
    </div>
  );
}

function Legend({ data, colors, mode }: { data: ChartData; colors: readonly string[]; mode: LegendMode }) {
  if (!legendVisible(mode, data.series.length >= 2) || data.series.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center justify-center gap-2 px-2 pb-1 text-[11px] text-muted">
      {data.series.map((s, i) => (
        <span key={s.name} className="flex items-center gap-1.5">
          <span className="size-2 rounded-sm" style={{ background: colors[i % colors.length] }} />
          {s.name}
        </span>
      ))}
    </div>
  );
}

function Empty({ hint }: { hint: string }) {
  return <div className="flex h-full items-center justify-center px-3 text-center text-xs text-muted">{hint}</div>;
}

function AxisLabels({ x, y }: { x: string | null; y: string | null }) {
  if (!x && !y) return null;
  return (
    <div className="pointer-events-none absolute inset-0 text-[10px] text-muted">
      {x ? <span className="absolute bottom-0 left-1/2 -translate-x-1/2">{x}</span> : null}
      {y ? <span className="absolute top-1/2 left-0 -translate-y-1/2 -rotate-90">{y}</span> : null}
    </div>
  );
}

/// `↗ +12.5%` / `↘ -3.0%` for a fraction; white-on-glass when the tile has a gradient.
function ChangeBadge({ change, onGradient = false, className }: { change: number; onGradient?: boolean; className?: string }) {
  const up = change >= 0;
  return (
    <span
      className={cn("rounded-md px-1.5 py-0.5 text-[11px] font-medium", onGradient ? "" : up ? "bg-success-soft text-success" : "bg-danger-soft text-danger", className)}
      style={onGradient ? { background: "color-mix(in oklch, var(--tile-on-gradient) 22%, transparent)", color: ON_GRADIENT } : undefined}
    >
      {up ? "↗" : "↘"} {formatChange(change)}
    </span>
  );
}

function changeOf(values: number[]): number | null {
  const last = values.at(-1);
  const prev = values.at(-2);
  return last !== undefined && prev !== undefined && Number.isFinite(last) && Number.isFinite(prev) ? relativeChange(last, prev) : null;
}

// ---------------------------------------------------------------- metric / sparkline
// WHAT:  The headline number of a metric, in the widget's format, with the
//        change against its comparison value and, for a series, the strip under it.
// WHERE: ./metric.ts (the numbers), ./appearance.ts (colours and background)
export function MetricTile({ widget, appearance }: { widget: Widget; appearance: Appearance }) {
  const stats = appearance.metric;
  const onGradient = appearance.onGradient;
  const ink = onGradient ? ON_GRADIENT : appearance.accent;
  const value = stats?.value ?? null;
  const headline = value === null ? "—" : formatMetricValue(value, widget.format, widget.currency);
  const change = widget.showChange ? (stats?.change ?? null) : null;
  return (
    <div className="relative flex h-full flex-col items-center justify-center gap-1 px-2.5 pb-1.5">
      {change !== null ? <ChangeBadge change={change} onGradient={onGradient} className="absolute top-1.5 right-2" /> : null}
      <span className="max-w-full truncate text-[34px] leading-tight font-semibold tabular-nums" style={{ color: ink }}>{headline}</span>
      {stats?.series ? (
        <div className="mt-1 h-10 w-full">
          <LineChart data={{ labels: stats.series.labels, series: [{ name: "", values: stats.series.values }] }} colors={[ink]} compact area />
        </div>
      ) : null}
    </div>
  );
}

export function SparklineWidget({ data, widget, tint }: { data: ChartData; widget: Widget; tint: string }) {
  const change = changeOf(data.series[0]?.values ?? []);
  return (
    <div className="relative h-full p-2">
      {widget.showChange && change !== null ? <ChangeBadge change={change} className="absolute top-1 right-2 z-10" /> : null}
      <LineChart data={{ labels: data.labels, series: data.series.slice(0, 1) }} colors={[tint]} compact area />
    </div>
  );
}

// ---------------------------------------------------------------- line / area
interface LineProps {
  data: ChartData;
  colors: readonly string[];
  compact?: boolean;
  area?: boolean;
  /// Area only: series stack instead of overlapping.
  stacked?: boolean;
  xLabel?: string | null;
  yLabel?: string | null;
  legendMode?: LegendMode;
  formatValue?: Formatter | null;
}

/// Upper (and, stacked, lower) edge of every series at every label.
function stackEdges(data: ChartData, stack: boolean): { top: number[][]; bottom: number[][] | null } {
  if (!stack) return { top: data.series.map((s) => s.values), bottom: null };
  let running = data.labels.map(() => 0);
  const top: number[][] = [];
  const bottom: number[][] = [];
  for (const s of data.series) {
    bottom.push(running);
    running = running.map((r, i) => {
      const v = s.values[i];
      return r + (v !== undefined && Number.isFinite(v) ? v : 0);
    });
    top.push(running);
  }
  return { top, bottom };
}

export function LineChart({ data, colors, compact = false, area = false, stacked = false, xLabel = null, yLabel = null, legendMode = "auto", formatValue = null }: LineProps) {
  const [hover, setHover] = useState<number | null>(null);
  const gradient = useId().replace(/:/g, "");
  const fv = formatValue ?? fmt;
  const stack = area && stacked && data.series.length > 1;
  const { top, bottom } = useMemo(() => stackEdges(data, stack), [data, stack]);
  const W = 400;
  const H = compact ? 60 : 200;
  const padX = compact ? 2 : 36;
  const padTop = compact ? 4 : 12;
  const padBottom = compact ? 4 : 26;
  const n = data.labels.length;
  const all = top.flat().filter(Number.isFinite);
  const min = Math.min(...all, 0);
  const max = Math.max(...all, 0);
  const span = max - min || 1;
  if (n === 0 || data.series.length === 0) return <Empty hint="Return a label column plus one or more numeric columns." />;
  if (n === 1) {
    return (
      <div className="flex h-full items-center justify-center gap-2">
        <span className={compact ? "text-xl font-semibold tabular-nums" : "text-[40px] font-semibold tabular-nums"} style={{ color: colors[0] }}>{fv(data.series[0]?.values[0] ?? NaN)}</span>
      </div>
    );
  }
  const step = (W - padX * 2) / (n - 1);
  const px = (i: number) => padX + i * step;
  const py = (v: number) => padTop + (1 - ((Number.isFinite(v) ? v : min) - min) / span) * (H - padTop - padBottom);
  const tickEvery = Math.max(1, Math.ceil(n / 8));
  const edge = (values: readonly number[]) => values.map((v, i) => `${i === 0 ? "M" : "L"}${px(i).toFixed(1)},${py(v).toFixed(1)}`).join(" ");
  const tooltipTop = hover === null ? 0 : Math.max(...top.map((t) => t[hover] ?? min));
  return (
    <div className="relative flex h-full w-full flex-col">
      <div className="relative min-h-0 flex-1">
        <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="h-full w-full" role="img" aria-label={stack ? "Stacked area chart" : area ? "Area chart" : "Line chart"}>
          {stack ? (
            <defs>
              {data.series.map((s, si) => (
                <linearGradient key={s.name} id={`${gradient}-${si}`} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={colors[si % colors.length] ?? "currentColor"} stopOpacity={0.55} />
                  <stop offset="100%" stopColor={colors[si % colors.length] ?? "currentColor"} stopOpacity={0.08} />
                </linearGradient>
              ))}
            </defs>
          ) : null}
          {!compact ? [0, 0.5, 1].map((t) => <line key={t} x1={padX} x2={W - padX} y1={padTop + (H - padTop - padBottom) * t} y2={padTop + (H - padTop - padBottom) * t} stroke="var(--separator)" strokeWidth={1} />) : null}
          {data.series.map((s, si) => {
            const color = colors[si % colors.length] ?? colors[0] ?? "currentColor";
            const upper = top[si] ?? s.values;
            const lower = bottom?.[si];
            const line = edge(upper);
            const wash =
              lower !== undefined
                ? `${line} ${[...lower].reverse().map((v, k) => `L${px(n - 1 - k).toFixed(1)},${py(v).toFixed(1)}`).join(" ")} Z`
                : `${line} L${px(n - 1).toFixed(1)},${(H - padBottom).toFixed(1)} L${px(0).toFixed(1)},${(H - padBottom).toFixed(1)} Z`;
            return (
              <g key={s.name}>
                {area ? <path d={wash} fill={stack ? `url(#${gradient}-${si})` : color} opacity={stack ? 1 : 0.12} /> : null}
                <path d={line} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" vectorEffect="non-scaling-stroke" />
                {hover !== null ? <circle cx={px(hover)} cy={py(upper[hover] ?? min)} r={4} fill={color} stroke="var(--surface)" strokeWidth={2} /> : null}
              </g>
            );
          })}
          {!compact ? (
            <>
              <text x={padX - 4} y={padTop + 4} textAnchor="end" fontSize={10} fill="var(--muted)">{fv(max)}</text>
              <text x={padX - 4} y={H - padBottom} textAnchor="end" fontSize={10} fill="var(--muted)">{fv(min)}</text>
              {data.labels.map((l, i) => (i % tickEvery === 0 ? <text key={i} x={px(i)} y={H - padBottom + 14} textAnchor="middle" fontSize={10} fill="var(--muted)">{l.slice(0, 8)}</text> : null))}
            </>
          ) : null}
          {data.labels.map((_, i) => (
            <rect key={i} x={px(i) - step / 2} y={0} width={step} height={H} fill="transparent" onMouseEnter={() => { setHover(i); }} onMouseLeave={() => { setHover(null); }} />
          ))}
        </svg>
        {hover !== null ? (
          <Tooltip x={(px(hover) / W) * 100} y={Math.max(18, (py(tooltipTop) / H) * 100)} label={data.labels[hover] ?? ""} rows={data.series.map((s, si) => ({ name: s.name, value: fv(s.values[hover] ?? NaN), color: colors[si % colors.length] ?? "" }))} />
        ) : null}
        <AxisLabels x={compact ? null : xLabel} y={compact ? null : yLabel} />
      </div>
      {!compact ? <Legend data={data} colors={colors} mode={legendMode} /> : null}
    </div>
  );
}

// ---------------------------------------------------------------- bar
export function BarChart({ data, colors, horizontal = false, xLabel = null, yLabel = null, legendMode = "auto", formatValue = null }: { data: ChartData; colors: readonly string[]; horizontal?: boolean; xLabel?: string | null; yLabel?: string | null; legendMode?: LegendMode; formatValue?: Formatter | null }) {
  const [hover, setHover] = useState<number | null>(null);
  const fv = formatValue ?? fmt;
  const W = 400;
  const H = 200;
  const padX = horizontal ? 70 : 36;
  const padTop = 12;
  const padBottom = horizontal ? 12 : 26;
  const n = data.labels.length;
  const k = data.series.length;
  if (n === 0 || k === 0) return <Empty hint="Return a label column plus one or more numeric columns." />;
  const all = data.series.flatMap((s) => s.values).filter(Number.isFinite);
  const max = Math.max(...all, 0);
  const min = Math.min(...all, 0);
  const span = max - min || 1;
  const slot = horizontal ? (H - padTop - padBottom) / n : (W - padX * 2) / n;
  const thick = Math.min(24, (slot - 4) / k);
  const baselineV = horizontal ? padX + (W - padX * 2) * (-min / span) : padTop + (H - padTop - padBottom) * (max / span);
  return (
    <div className="relative flex h-full w-full flex-col">
      <div className="relative min-h-0 flex-1">
        <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="h-full w-full" role="img" aria-label="Bar chart">
          {horizontal ? <line x1={baselineV} x2={baselineV} y1={padTop} y2={H - padBottom} stroke="var(--separator)" strokeWidth={1} /> : <line x1={padX} x2={W - padX} y1={baselineV} y2={baselineV} stroke="var(--separator)" strokeWidth={1} />}
          {data.labels.map((label, i) =>
            data.series.map((s, si) => {
              const raw = s.values[i] ?? NaN;
              const v = Number.isFinite(raw) ? raw : 0;
              const len = (Math.abs(v) / span) * (horizontal ? W - padX * 2 : H - padTop - padBottom);
              const color = colors[si % colors.length] ?? "";
              const r = Math.min(4, thick / 2, len);
              const off = i * slot + (slot - thick * k - 2 * (k - 1)) / 2 + si * (thick + 2);
              let d: string;
              if (horizontal) {
                const y = padTop + off;
                const x0 = v >= 0 ? baselineV : baselineV - len;
                const x1 = v >= 0 ? baselineV + len : baselineV;
                d = v >= 0
                  ? `M${x0},${y} H${x1 - r} Q${x1},${y} ${x1},${y + r} V${y + thick - r} Q${x1},${y + thick} ${x1 - r},${y + thick} H${x0} Z`
                  : `M${x1},${y} H${x0 + r} Q${x0},${y} ${x0},${y + r} V${y + thick - r} Q${x0},${y + thick} ${x0 + r},${y + thick} H${x1} Z`;
              } else {
                const x = padX + off;
                const y = v >= 0 ? baselineV - len : baselineV;
                d = v >= 0
                  ? `M${x},${y + len} V${y + r} Q${x},${y} ${x + r},${y} H${x + thick - r} Q${x + thick},${y} ${x + thick},${y + r} V${y + len} Z`
                  : `M${x},${y} V${y + len - r} Q${x},${y + len} ${x + r},${y + len} H${x + thick - r} Q${x + thick},${y + len} ${x + thick},${y + len - r} V${y} Z`;
              }
              return <path key={`${label}-${s.name}`} d={d} fill={color} opacity={hover === null || hover === i ? 1 : 0.5} onMouseEnter={() => { setHover(i); }} onMouseLeave={() => { setHover(null); }} />;
            }),
          )}
          {horizontal
            ? data.labels.map((l, i) => <text key={i} x={padX - 6} y={padTop + i * slot + slot / 2 + 3} textAnchor="end" fontSize={10} fill="var(--muted)">{l.slice(0, 10)}</text>)
            : n <= 14
              ? data.labels.map((l, i) => <text key={i} x={padX + i * slot + slot / 2} y={H - 8} textAnchor="middle" fontSize={10} fill="var(--muted)">{l.slice(0, 8)}</text>)
              : null}
          {!horizontal ? <text x={padX - 4} y={padTop + 4} textAnchor="end" fontSize={10} fill="var(--muted)">{fv(max)}</text> : null}
          {horizontal ? <text x={W - padX} y={H - 1} textAnchor="end" fontSize={10} fill="var(--muted)">{fv(max)}</text> : null}
        </svg>
        {hover !== null ? (
          <Tooltip x={horizontal ? 50 : ((padX + hover * slot + slot / 2) / W) * 100} y={horizontal ? ((padTop + hover * slot) / H) * 100 : 20} label={data.labels[hover] ?? ""} rows={data.series.map((s, si) => ({ name: s.name, value: fv(s.values[hover] ?? NaN), color: colors[si % colors.length] ?? "" }))} />
        ) : null}
        <AxisLabels x={xLabel} y={yLabel} />
      </div>
      <Legend data={data} colors={colors} mode={legendMode} />
    </div>
  );
}

// ---------------------------------------------------------------- pie (donut)
function sliceAngles(values: readonly number[], total: number): { a0: number; a1: number; v: number }[] {
  const out: { a0: number; a1: number; v: number }[] = [];
  let angle = -Math.PI / 2;
  for (const v of values) {
    const a1 = angle + (v / total) * Math.PI * 2;
    out.push({ a0: angle, a1, v });
    angle = a1;
  }
  return out;
}

/// A slice's fill strength: the first is solid, the next fade in steps, as the tint is the only hue.
function sliceOpacity(i: number): number {
  return 1 - (i % 6) * 0.13;
}

export function PieChart({ data, tint, legendMode = "auto", formatValue = null }: { data: ChartData; tint: string; legendMode?: LegendMode; formatValue?: Formatter | null }) {
  const [hover, setHover] = useState<number | null>(null);
  const fv = formatValue ?? fmt;
  const values = (data.series[0]?.values ?? []).map((v) => (Number.isFinite(v) && v > 0 ? v : 0));
  const total = values.reduce((a, b) => a + b, 0);
  if (total <= 0) return <Empty hint="Return a label column and a positive numeric column." />;
  const R = 80;
  const r = 44;
  const slices = sliceAngles(values, total).map((s, i) => ({ ...s, i, label: data.labels[i] ?? "" }));
  const arc = (a0: number, a1: number) => {
    const gap = 0.02;
    const s0 = a0 + gap;
    const s1 = Math.max(s0, a1 - gap);
    const large = s1 - s0 > Math.PI ? 1 : 0;
    const p = (ang: number, rad: number) => `${(100 + rad * Math.cos(ang)).toFixed(2)},${(100 + rad * Math.sin(ang)).toFixed(2)}`;
    return `M${p(s0, R)} A${R},${R} 0 ${large} 1 ${p(s1, R)} L${p(s1, r)} A${r},${r} 0 ${large} 0 ${p(s0, r)} Z`;
  };
  const current = hover === null ? undefined : slices[hover];
  const showLegend = legendVisible(legendMode, slices.filter((s) => s.v > 0).length >= 2);
  return (
    <div className="flex h-full w-full items-center gap-2">
      <div className="relative flex h-full min-w-0 flex-1 items-center justify-center">
        <svg viewBox="0 0 200 200" className="h-full max-h-full" role="img" aria-label="Pie chart">
          {slices.map((s) => (
            <path key={s.i} d={arc(s.a0, s.a1)} fill={tint} opacity={hover === null || hover === s.i ? sliceOpacity(s.i) : 0.35} onMouseEnter={() => { setHover(s.i); }} onMouseLeave={() => { setHover(null); }} />
          ))}
        </svg>
        {current ? (
          <div className="pointer-events-none absolute rounded-md border border-border bg-overlay px-2 py-1 text-[11px] text-foreground shadow">
            <span className="text-muted">{current.label}</span> {fv(current.v)} ({((current.v / total) * 100).toFixed(1)}%)
          </div>
        ) : null}
      </div>
      {showLegend ? (
        <ScrollArea className="max-h-full w-[42%] shrink-0 py-1 pr-2 text-[11px]">
          <ul className="flex flex-col gap-1">
            {slices.map((s) => (
              <li key={s.i} className="flex min-w-0 items-center gap-1.5" onMouseEnter={() => { setHover(s.i); }} onMouseLeave={() => { setHover(null); }}>
                <span className="size-2 shrink-0 rounded-sm" style={{ background: tint, opacity: sliceOpacity(s.i) }} />
                <span className="truncate text-muted">{s.label}</span>
                <span className="ml-auto shrink-0 pl-1 tabular-nums">{fv(s.v)}</span>
                <span className="w-9 shrink-0 text-right text-muted tabular-nums">{((s.v / total) * 100).toFixed(0)}%</span>
              </li>
            ))}
          </ul>
        </ScrollArea>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------- sankey (source, target, value)
interface SankeyModel {
  nodes: { name: string; column: number; y: number; height: number }[];
  links: { source: number; target: number; value: number; width: number; sy: number; ty: number }[];
  columns: number;
}

export function SankeyChart({ outcome, tint }: { outcome: QueryOutcome | null; tint: string }) {
  const [hover, setHover] = useState<number | null>(null);
  const model = useMemo(() => buildSankey(outcome), [outcome]);
  if (!model) return <Empty hint="Return three columns: source, target, value. Each row is a flow; duplicate pairs are summed." />;
  const W = 400;
  const H = 200;
  const colW = 10;
  const colX = (c: number) => 16 + (c * (W - 32 - colW)) / Math.max(1, model.columns - 1);
  const hovered = hover === null ? undefined : model.links[hover];
  return (
    <div className="relative h-full w-full">
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className="h-full w-full" role="img" aria-label="Sankey diagram">
        {model.links.map((l, i) => {
          const s = model.nodes[l.source];
          const t = model.nodes[l.target];
          if (!s || !t) return null;
          const x0 = colX(s.column) + colW;
          const x1 = colX(t.column);
          const y0 = s.y + l.sy;
          const y1 = t.y + l.ty;
          const w = l.width;
          const c = (x0 + x1) / 2;
          const d = `M${x0},${y0} C${c},${y0} ${c},${y1} ${x1},${y1} L${x1},${y1 + w} C${c},${y1 + w} ${c},${y0 + w} ${x0},${y0 + w} Z`;
          return <path key={i} d={d} fill={tint} opacity={hover === null || hover === i ? 0.35 : 0.12} onMouseEnter={() => { setHover(i); }} onMouseLeave={() => { setHover(null); }} />;
        })}
        {model.nodes.map((n) => (
          <g key={n.name}>
            <rect x={colX(n.column)} y={n.y} width={colW} height={Math.max(2, n.height)} rx={2} fill={tint} />
            <text x={n.column === model.columns - 1 ? colX(n.column) - 4 : colX(n.column) + colW + 4} y={n.y + n.height / 2 + 3} textAnchor={n.column === model.columns - 1 ? "end" : "start"} fontSize={10} fill="var(--muted)">{n.name}</text>
          </g>
        ))}
      </svg>
      {hovered ? (
        <div className="pointer-events-none absolute top-1 left-1 rounded-md border border-border bg-overlay px-2 py-1 text-[11px] text-foreground shadow">
          <span className="text-muted">{model.nodes[hovered.source]?.name} to {model.nodes[hovered.target]?.name}</span> {fmt(hovered.value)}
        </div>
      ) : null}
    </div>
  );
}

function buildSankey(outcome: QueryOutcome | null): SankeyModel | null {
  const rows = rowsOf(outcome);
  if (!rows || rows.columns.length < 3) return null;
  const flows = new Map<string, { s: string; t: string; v: number }>();
  for (const r of rows.rows) {
    const s = cellText(r[0]);
    const t = cellText(r[1]);
    const v = toNumber(r[2]);
    if (!s || !t || !Number.isFinite(v) || v <= 0 || s === t) continue;
    const key = `${s} ${t}`;
    const existing = flows.get(key);
    if (existing) existing.v += v;
    else flows.set(key, { s, t, v });
  }
  if (flows.size === 0) return null;
  // Column = longest path from a pure source (cycles stop growing after n passes).
  const names = [...new Set([...flows.values()].flatMap((f) => [f.s, f.t]))];
  const column = new Map<string, number>(names.map((n) => [n, 0]));
  let passes = 0;
  let changed = true;
  while (changed && passes < names.length) {
    passes += 1;
    changed = false;
    for (const f of flows.values()) {
      const next = (column.get(f.s) ?? 0) + 1;
      if (next > (column.get(f.t) ?? 0) && next < names.length) {
        column.set(f.t, next);
        changed = true;
      }
    }
  }
  const columns = Math.max(...column.values()) + 1;
  const H = 200;
  const total = (n: string) =>
    Math.max(
      [...flows.values()].filter((f) => f.s === n).reduce((a, f) => a + f.v, 0),
      [...flows.values()].filter((f) => f.t === n).reduce((a, f) => a + f.v, 0),
    );
  const colTotals = Array.from({ length: columns }, (_, c) => names.filter((n) => column.get(n) === c).reduce((a, n) => a + total(n), 0));
  const scale = (H - 24) / Math.max(...colTotals, 1);
  const nodes: SankeyModel["nodes"] = [];
  for (let c = 0; c < columns; c++) {
    const inCol = names.filter((n) => column.get(n) === c);
    const used = inCol.reduce((a, n) => a + total(n) * scale, 0);
    const gap = inCol.length > 1 ? Math.min(12, (H - 24 - used) / (inCol.length - 1)) : 0;
    let y = 12;
    for (const n of inCol) {
      const h = total(n) * scale;
      nodes.push({ name: n, column: c, y, height: h });
      y += h + Math.max(2, gap);
    }
  }
  const index = new Map(nodes.map((n, i) => [n.name, i]));
  const outOffset = new Map<string, number>();
  const inOffset = new Map<string, number>();
  const links: SankeyModel["links"] = [];
  for (const f of [...flows.values()].sort((a, b) => (column.get(a.s) ?? 0) - (column.get(b.s) ?? 0))) {
    const width = f.v * scale;
    const sy = outOffset.get(f.s) ?? 0;
    const ty = inOffset.get(f.t) ?? 0;
    outOffset.set(f.s, sy + width);
    inOffset.set(f.t, ty + width);
    links.push({ source: index.get(f.s) ?? 0, target: index.get(f.t) ?? 0, value: f.v, width, sy, ty });
  }
  return { nodes, links, columns };
}

// ---------------------------------------------------------------- map (lat, lon[, label])
// WHAT:  A basemap with a marker per row. The basemap follows the app theme.
// WHERE: ./SlippyMap.tsx (the viewer), ./mapTiles.ts (points and geometry)
export function MapChart({ outcome, tint, pulse }: { outcome: QueryOutcome | null; tint: string; pulse: boolean }) {
  const theme = useResolvedTheme();
  const points = useMemo(() => parseMapPoints(outcome), [outcome]);
  return <SlippyMap points={points} tint={tint} pulse={pulse} basemap={theme} hint="Return latitude and longitude columns (lat, lon) plus an optional label." />;
}

// ---------------------------------------------------------------- progress
export function ProgressMeter({ outcome, widget, tint }: { outcome: QueryOutcome | null; widget: Widget; tint: string }) {
  const first = rowsOf(outcome)?.rows[0];
  const value = toNumber(first?.[0]);
  const max = widget.maxValue ?? toNumber(first?.[1]);
  if (!Number.isFinite(value) || !Number.isFinite(max) || max <= 0) return <Empty hint="Return a value (and optionally a target) or set the maximum in options." />;
  const pct = Math.max(0, Math.min(100, (value / max) * 100));
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 px-3">
      <div className="h-2 w-full overflow-hidden rounded-full bg-surface-tertiary">
        <div className="h-full rounded-full" style={{ width: `${pct}%`, background: tint }} />
      </div>
      <div className="flex items-center gap-2 text-xs">
        {widget.showPercent ? <span className="font-medium" style={{ color: tint }}>{pct.toFixed(0)}%</span> : null}
        {widget.showValues ? <span className="text-muted">{fmt(value)} / {fmt(max)}</span> : null}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- text / image
export function TextWidget({ widget, outcome, onGradient = false }: { widget: Widget; outcome: QueryOutcome | null; onGradient?: boolean }) {
  const text = matchedContent(widget, outcome, widget.text ?? "");
  if (text.trim().length === 0) return <Empty hint="Add content in the options. Use {{column}} or {{a / b * 100}} to insert query values." />;
  return (
    <ScrollArea className="h-full p-2.5">
      <div className="selectable flex min-h-full items-center justify-center text-center text-[13px] font-semibold break-words whitespace-pre-wrap text-foreground" style={onGradient ? { color: ON_GRADIENT } : undefined}>{text}</div>
    </ScrollArea>
  );
}

export function ImageWidget({ widget, outcome, gif }: { widget: Widget; outcome: QueryOutcome | null; gif: boolean }) {
  const url = matchedContent(widget, outcome, widget.url ?? "").trim();
  const [failed, setFailed] = useState<string | null>(null);
  if (url.length === 0) return <Empty hint={gif ? "Paste a GIF URL in the options." : "Paste an image URL in the options."} />;
  if (failed === url) return <Empty hint="The image could not be loaded." />;
  return (
    <div className="flex h-full items-center justify-center overflow-hidden p-2">
      <img src={url} alt={widget.title} referrerPolicy="no-referrer" className="max-h-full max-w-full rounded-md object-contain" onError={() => { setFailed(url); }} />
    </div>
  );
}
