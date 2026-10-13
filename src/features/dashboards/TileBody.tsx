// SOT: tile-body, widget-body, tile-renderer, tile-series-colours
import { memo, useMemo } from "react";
import type { QueryOutcome, Widget } from "@/lib/bindings";
import { DataGrid } from "@/features/grid/DataGrid";
import type { Appearance } from "./appearance";
import { isRows } from "./cells";
import { BarChart, ImageWidget, LineChart, MapChart, MetricTile, PieChart, ProgressMeter, SERIES_COLORS, SankeyChart, SparklineWidget, TextWidget, chartData } from "./charts";
import { chartFormatter } from "./format";
import { tintCss } from "./tiles";

// WHAT:  What a tile draws inside its frame for one widget kind, given the
//        result of its query and how it is dressed.
// WHY:   The dashboard grid, the inspector's preview and (later) a fullscreen
//        view all draw the same tile; this is the one switch over `WidgetKind`,
//        so a new kind fails `tsc` here and nowhere else.
// HOW:   Memoised on the widget and the outcome, so a drag that only moves the
//        frame never redraws a chart. A query error or an unrun query is said in
//        words rather than drawn as an empty chart.
// WHERE: ./DashboardGrid.tsx (frames), ./TileInspector.tsx (preview), ./charts.tsx

/// Series colours: the widget tint first, then the fixed categorical order.
function seriesColors(tint: string | null): readonly string[] {
  const first = tintCss(tint);
  return [first, ...SERIES_COLORS.filter((c) => c !== first)];
}

const NEEDS_DATA: Record<Widget["kind"], boolean> = {
  area: true,
  line: true,
  bar: true,
  pie: true,
  sankey: true,
  table: true,
  metric: true,
  sparkline: true,
  map: true,
  progress: true,
  text: false,
  image: false,
  gif: false,
};

interface TileBodyProps {
  widget: Widget;
  outcome: QueryOutcome | null;
  appearance: Appearance;
  rowHeight: number;
  error: string | null;
}

function Message({ children, tone = "muted" }: { children: string; tone?: "muted" | "danger" }) {
  return <div className={tone === "danger" ? "flex h-full items-center justify-center px-3 text-center text-xs break-words text-danger" : "flex h-full items-center justify-center px-3 text-center text-xs text-muted"}>{children}</div>;
}

export const TileBody = memo(function TileBody({ widget, outcome, appearance, rowHeight, error }: TileBodyProps) {
  const data = useMemo(() => chartData(outcome), [outcome]);
  const colors = useMemo(() => seriesColors(widget.tint), [widget.tint]);
  const formatValue = useMemo(() => chartFormatter(widget.valueFormat, widget.currency), [widget.valueFormat, widget.currency]);
  if (error !== null) return <Message tone="danger">{error}</Message>;
  if (NEEDS_DATA[widget.kind] && outcome === null) return <Message>{widget.sql.trim().length === 0 ? "Open the tile and add a query" : "Run the query to populate"}</Message>;
  const tint = appearance.accent;
  switch (widget.kind) {
    case "metric":
      return <MetricTile widget={widget} appearance={appearance} />;
    case "sparkline":
      return <SparklineWidget data={data} widget={widget} tint={tint} />;
    case "line":
      return <div className="h-full p-2"><LineChart data={data} colors={colors} xLabel={widget.xLabel} yLabel={widget.yLabel} legendMode={widget.legendMode} formatValue={formatValue} /></div>;
    case "area":
      return <div className="h-full p-2"><LineChart data={data} colors={colors} area stacked={widget.stacked} xLabel={widget.xLabel} yLabel={widget.yLabel} legendMode={widget.legendMode} formatValue={formatValue} /></div>;
    case "bar":
      return <div className="h-full p-2"><BarChart data={data} colors={colors} horizontal={widget.horizontal} xLabel={widget.xLabel} yLabel={widget.yLabel} legendMode={widget.legendMode} formatValue={formatValue} /></div>;
    case "pie":
      return <div className="h-full p-2"><PieChart data={data} tint={tint} legendMode={widget.legendMode} formatValue={formatValue} /></div>;
    case "sankey":
      return <div className="h-full p-2"><SankeyChart outcome={outcome} tint={tint} /></div>;
    case "map":
      return <MapChart outcome={outcome} tint={tint} pulse={widget.pulse} />;
    case "progress":
      return <ProgressMeter outcome={outcome} widget={widget} tint={tint} />;
    case "text":
      return <TextWidget widget={widget} outcome={outcome} onGradient={appearance.onGradient} />;
    case "image":
      return <ImageWidget widget={widget} outcome={outcome} gif={false} />;
    case "gif":
      return <ImageWidget widget={widget} outcome={outcome} gif />;
    case "table": {
      const rows = outcome?.statements.find(isRows);
      if (!rows) return <Message>No rows</Message>;
      return <DataGrid columns={rows.result.columns.map((c) => ({ name: c.name, typeName: c.typeName }))} rowCount={rows.result.rows.length} getRow={(i) => rows.result.rows[i]} rowHeight={rowHeight} />;
    }
  }
});
