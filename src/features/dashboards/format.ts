// SOT: dashboard-value-format, metric-format, chart-value-format, currency-list, change-percentage
import type { LegendMode, MetricFormat, ValueFormat } from "@/lib/bindings";

// WHAT:  How a number reads on a dashboard: a metric's headline (Number /
//        Decimal / Currency / Percent), chart axes and tooltips (Number /
//        Compact / Currency / Percent), and the change between two values.
// WHY:   Tiles, axes and tooltips used to share one formatter that knew only
//        grouping and two decimals. Revenue wants a currency sign, a rate a
//        percent sign, a big count 1.2M; one module keeps every surface in step.
// HOW:   `Intl.NumberFormat` in the user's locale. A percent value is the
//        number of percent (12.5 prints 12.5%), not a fraction. A currency code
//        the runtime does not know falls back to the code in front of a plain
//        number rather than throwing.
// WHERE: ./charts.tsx, ./DashboardTab.tsx, ./TileInspector.tsx

/// The currencies a tile can show, in the order the picker lists them.
export const CURRENCY_CODES: readonly string[] = [
  "USD", "EUR", "GBP", "JPY", "CAD", "AUD", "CHF", "CNY", "INR", "KRW", "BRL", "MXN",
  "SEK", "NOK", "DKK", "NZD", "SGD", "HKD", "ZAR", "PLN", "CZK", "THB", "TWD", "TRY",
];

export const METRIC_FORMATS: readonly { value: MetricFormat; label: string }[] = [
  { value: "number", label: "Number" },
  { value: "decimal", label: "Decimal" },
  { value: "currency", label: "Currency" },
  { value: "percent", label: "Percent" },
];

export const VALUE_FORMATS: readonly { value: ValueFormat; label: string }[] = [
  { value: "number", label: "Number" },
  { value: "compact", label: "Compact (1.2K)" },
  { value: "currency", label: "Currency" },
  { value: "percent", label: "Percent" },
];

export const LEGEND_MODES: readonly { value: LegendMode; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "show", label: "Always show" },
  { value: "hide", label: "Hide" },
];

function intl(options: Intl.NumberFormatOptions): Intl.NumberFormat {
  return new Intl.NumberFormat(undefined, options);
}

function currencyFormat(value: number, currency: string, fractionDigits: { min: number; max: number }): string {
  try {
    return intl({ style: "currency", currency, minimumFractionDigits: fractionDigits.min, maximumFractionDigits: fractionDigits.max }).format(value);
  } catch {
    return `${currency} ${intl({ minimumFractionDigits: fractionDigits.min, maximumFractionDigits: fractionDigits.max }).format(value)}`;
  }
}

/// A metric tile's headline.
export function formatMetricValue(value: number, format: MetricFormat, currency: string): string {
  if (!Number.isFinite(value)) return "—";
  switch (format) {
    case "decimal":
      return intl({ minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(value);
    case "currency":
      return currencyFormat(value, currency, { min: 0, max: 0 });
    case "percent":
      return intl({ style: "percent", minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(value / 100);
    case "number":
      return intl({ maximumFractionDigits: 0 }).format(value);
  }
}

/// A value on a chart axis or in a tooltip.
export function formatChartValue(value: number, format: ValueFormat, currency: string): string {
  if (!Number.isFinite(value)) return "-";
  switch (format) {
    case "compact":
      return intl({ notation: "compact", maximumFractionDigits: 1 }).format(value);
    case "currency":
      return currencyFormat(value, currency, { min: 0, max: 2 });
    case "percent":
      return intl({ style: "percent", maximumFractionDigits: 1 }).format(value / 100);
    case "number":
      return intl({ maximumFractionDigits: 2 }).format(value);
  }
}

/// What 1234 looks like in a currency, for the picker's preview.
export function currencyExample(currency: string): string {
  return currencyFormat(1234, currency, { min: 0, max: 0 });
}

/// The symbol a currency is written with (`$`, `€`), or the code when the runtime has none.
export function currencySymbol(currency: string): string {
  try {
    return intl({ style: "currency", currency }).formatToParts(0).find((p) => p.type === "currency")?.value ?? currency;
  } catch {
    return currency;
  }
}

/// How far `value` moved from `compare`, as a fraction (0.125 = +12.5 %). A
/// zero base has no ratio: unchanged is 0, otherwise it is a full step up or down.
export function relativeChange(value: number, compare: number): number {
  if (compare === 0) return value === 0 ? 0 : value > 0 ? 1 : -1;
  return (value - compare) / Math.abs(compare);
}

/// `+12.5%` / `-3.0%` for a fraction.
export function formatChange(fraction: number): string {
  const percent = fraction * 100;
  return `${percent >= 0 ? "+" : ""}${percent.toFixed(1)}%`;
}

/// Whether a legend is drawn: Show and Hide are absolute, Auto follows the chart.
export function legendVisible(mode: LegendMode, autoVisible: boolean): boolean {
  if (mode === "show") return true;
  if (mode === "hide") return false;
  return autoVisible;
}

/// A formatter for a chart, or null for the chart's own default.
export type ValueFormatter = (value: number) => string;

export function chartFormatter(format: ValueFormat, currency: string): ValueFormatter | null {
  return format === "number" ? null : (value) => formatChartValue(value, format, currency);
}
