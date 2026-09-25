// SOT: result-chart, chart-query-result, chart-column-picker, add-to-dashboard
import { useMemo, useState } from "react";
import type { Document, ResultSet, WidgetKind } from "@/lib/bindings";
import { normalizeError } from "@/lib/ipc";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { BarChart, LineChart, PieChart, SERIES_COLORS, isNumericColumn, mapChartData, type ChartMapping } from "@/features/dashboards/charts";
import { newWidget, widgetSql } from "@/features/dashboards/widgetSql";
import { AppSelect, Check, Field, Toggle } from "@/components/global/Field";
import { EmptyState } from "@/components/global/EmptyState";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { ScrollArea } from "@/components/ui/scroll-area";

/// The widget kinds the results chart draws: the ones that plot X against Y.
type ChartKind = Extract<WidgetKind, "bar" | "line" | "area" | "pie">;
const KINDS = [
  { value: "bar", label: "Bar", icon: "chart-bar" },
  { value: "line", label: "Line", icon: "chart" },
  { value: "area", label: "Area", icon: "chart" },
  { value: "pie", label: "Pie", icon: "chart" },
] satisfies readonly { value: ChartKind; label: string; icon: "chart" | "chart-bar" }[];

const NONE = "";
const NEW_DASHBOARD = "new";
/// Past these a bar chart drops its axis labels and a pie turns to slivers.
const BAR_LABEL_LIMIT = 60;
const PIE_SLICE_LIMIT = 8;

// WHAT:  The results pane's chart view: pick a chart type, the X column, one
//        or more numeric Y columns and an optional series column, then keep
//        it as a dashboard widget.
// WHY:   "What does this look like over time?" is the next question after
//        most aggregate queries; answering it should not mean building a
//        dashboard first.
// HOW:   Draws with the dashboard's own chart components (fixed categorical
//        order, legend from two series, hover tooltips). Defaults: X = the
//        first text column, Y = every numeric column. "Add to dashboard"
//        stores the statement wrapped by widgetSql so the widget, which infers
//        its axes, draws the same picture.
// WHERE: src/features/dashboards/charts.tsx, src/features/dashboards/widgetSql.ts
export function ResultChart({ connectionId, sql, result }: { connectionId: string; sql: string; result: ResultSet }) {
  const names = useMemo(() => result.columns.map((c) => c.name), [result]);
  const numeric = useMemo(() => result.columns.map((_, i) => isNumericColumn(result.rows, i)), [result]);
  const [kind, setKind] = useState<ChartKind>("bar");
  const [horizontal, setHorizontal] = useState(false);
  const [mapping, setMapping] = useState<ChartMapping>(() => {
    const firstText = numeric.findIndex((n) => !n);
    const x = firstText >= 0 ? firstText : 0;
    return { x, ys: numeric.map((n, i) => (n && i !== x ? i : -1)).filter((i) => i >= 0).slice(0, SERIES_COLORS.length), group: null };
  });

  const yOptions = names.map((name, i) => ({ name, i })).filter(({ i }) => numeric[i] && i !== mapping.x);
  const groupOptions = [{ value: NONE, label: "None" }, ...names.map((name, i) => ({ value: String(i), label: name })).filter((_, i) => !numeric[i] && i !== mapping.x)];
  const data = useMemo(() => mapChartData(result.rows, names, mapping), [result, names, mapping]);
  const yName = mapping.ys.length === 1 || mapping.group !== null ? (names[mapping.ys[0] ?? -1] ?? null) : null;
  const xName = names[mapping.x] ?? null;

  const setX = (value: string) => {
    const x = Number(value);
    setMapping((m) => ({ x, ys: m.ys.filter((y) => y !== x), group: m.group === x ? null : m.group }));
  };
  const toggleY = (y: number) =>
    setMapping((m) => ({ ...m, ys: m.ys.includes(y) ? m.ys.filter((i) => i !== y) : [...m.ys, y].sort((a, b) => a - b).slice(0, SERIES_COLORS.length) }));

  if (!numeric.some(Boolean)) {
    return <EmptyState icon="chart" title="Nothing to chart" body="The result has no numeric columns. Return a label column and at least one number." />;
  }

  const hint =
    mapping.ys.length === 0
      ? null
      : kind === "pie" && data.labels.length > PIE_SLICE_LIMIT
        ? `${data.labels.length} slices is hard to read as a pie; a bar chart compares them better.`
        : kind === "pie" && data.series.length > 1
          ? `A pie shows one measure: ${data.series[0]?.name ?? ""}.`
          : kind === "bar" && !horizontal && data.labels.length > BAR_LABEL_LIMIT
            ? `${data.labels.length} bars: labels are hidden; hover a bar to read it.`
            : mapping.group !== null && mapping.ys.length > 1
              ? `Split by a series column, the chart shows one measure: ${yName ?? ""}.`
              : null;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ScrollArea orientation="horizontal" hideScrollBar className="flex h-9 shrink-0 items-center gap-1.5 border-b border-border/40 px-2 text-xs">
        <AppSelect ariaLabel="Chart type" value={kind} options={KINDS} onChange={setKind} size="sm" className="w-24 shrink-0" />
        <span className="text-muted">X</span>
        <AppSelect ariaLabel="X column" value={String(mapping.x)} options={names.map((name, i) => ({ value: String(i), label: name }))} onChange={setX} size="sm" className="w-36 shrink-0" />
        <span className="text-muted">Y</span>
        <Popover>
          <PopoverTrigger asChild>
            <Button size="xs" variant="toolbar" className="max-w-56 shrink-0">
              <span className="truncate">{mapping.ys.length === 0 ? "Pick columns" : mapping.ys.map((y) => names[y]).join(", ")}</span>
              <Icon name="chevron-down" size={10} />
            </Button>
          </PopoverTrigger>
          <PopoverContent className="w-60 p-2">
            <ScrollArea hideScrollBar className="max-h-64">
              <ul className="flex flex-col gap-0.5">
                {yOptions.map(({ name, i }) => (
                  <li key={i} className="rounded-md px-1 py-0.5 text-[12px] hover:bg-surface-secondary/60">
                    <Check label={name} checked={mapping.ys.includes(i)} onChange={() => toggleY(i)} />
                  </li>
                ))}
              </ul>
            </ScrollArea>
            <p className="mt-2 border-t border-border/40 pt-2 text-[11px] text-muted">Numeric columns only, up to {SERIES_COLORS.length}.</p>
          </PopoverContent>
        </Popover>
        {kind !== "pie" && groupOptions.length > 1 ? (
          <>
            <span className="text-muted">Series</span>
            <AppSelect ariaLabel="Series column" value={mapping.group === null ? NONE : String(mapping.group)} options={groupOptions} onChange={(v) => setMapping((m) => ({ ...m, group: v === NONE ? null : Number(v) }))} size="sm" className="w-32 shrink-0" />
          </>
        ) : null}
        {kind === "bar" ? <Toggle checked={horizontal} onChange={setHorizontal} label="Horizontal" /> : null}
        <span className="ml-auto shrink-0 pl-2">
          <AddToDashboard connectionId={connectionId} sql={sql} result={result} mapping={kind === "pie" ? { ...mapping, group: null } : mapping} kind={kind} horizontal={horizontal} xLabel={xName} yLabel={yName} disabled={mapping.ys.length === 0} />
        </span>
      </ScrollArea>
      {hint !== null ? <p className="shrink-0 px-3 pt-1.5 text-[11px] text-muted">{hint}</p> : null}
      <div className="min-h-0 flex-1 p-3">
        {mapping.ys.length === 0 ? (
          <EmptyState icon="chart" title="Pick a Y column" body="Choose one or more numeric columns to plot." />
        ) : kind === "pie" ? (
          <PieChart data={{ labels: data.labels, series: data.series.slice(0, 1) }} tint={SERIES_COLORS[0]} />
        ) : kind === "bar" ? (
          <BarChart data={data} colors={SERIES_COLORS} horizontal={horizontal} xLabel={xName} yLabel={yName} />
        ) : (
          <LineChart data={data} colors={SERIES_COLORS} area={kind === "area"} xLabel={xName} yLabel={yName} />
        )}
      </div>
    </div>
  );
}

interface AddToDashboardProps {
  connectionId: string;
  sql: string;
  result: ResultSet;
  mapping: ChartMapping;
  kind: ChartKind;
  horizontal: boolean;
  xLabel: string | null;
  yLabel: string | null;
  disabled: boolean;
}

// WHAT:  Saves the chart as a widget in an existing dashboard or a new one.
// HOW:   Read-modify-write of the dashboard document through the store's
//        saveDocument; a dashboard with no connection adopts this one. Opens
//        the dashboard when it is not already open (an open tab keeps its own
//        unsaved state, so it is told to reopen instead of being overwritten).
// WHERE: src-tauri/src/model/documents.rs (DashboardBody), src/features/dashboards/DashboardTab.tsx
function AddToDashboard({ connectionId, sql, result, mapping, kind, horizontal, xLabel, yLabel, disabled }: AddToDashboardProps) {
  const dashboards = useWorkspace((s) => s.documents.dashboard);
  const loadDocuments = useWorkspace((s) => s.loadDocuments);
  const saveDocument = useWorkspace((s) => s.saveDocument);
  const openDocument = useWorkspace((s) => s.openDocument);
  const connections = useWorkspace((s) => s.connections);
  const engine = connections.find((c) => c.id === connectionId)?.engine ?? "postgres";
  const showInfo = useWorkspace((s) => s.showInfo);
  const showError = useWorkspace((s) => s.showError);
  const [open, setOpen] = useState(false);
  const [target, setTarget] = useState<string>(NEW_DASHBOARD);
  const [dashboardName, setDashboardName] = useState("");
  const [title, setTitle] = useState("");
  const [saving, setSaving] = useState(false);

  const onOpenChange = (next: boolean) => {
    setOpen(next);
    if (!next) return;
    setTitle(yLabel !== null && xLabel !== null ? `${yLabel} by ${xLabel}` : "");
    setDashboardName(`Dashboard ${dashboards.length + 1}`);
    void (async () => {
      try {
        await loadDocuments("dashboard");
      } catch (raw) {
        showError(normalizeError(raw));
      }
    })();
  };

  const chosen = dashboards.find((d) => d.id === target);
  const otherConnection = chosen?.connectionId !== null && chosen?.connectionId !== undefined && chosen.connectionId !== connectionId;
  const connectionName = (id: string | null) => connections.find((c) => c.id === id)?.name ?? "another connection";

  const save = async () => {
    const widget = newWidget({
      title: title.trim(),
      kind,
      sql: widgetSql(engine, sql, result.columns.map((c) => c.name), result.rows, mapping),
      w: 6,
      h: 3,
      horizontal: kind === "bar" && horizontal,
      xLabel,
      yLabel,
    });
    const doc: Document = chosen
      ? {
          ...chosen,
          connectionId: chosen.connectionId ?? connectionId,
          body: { kind: "dashboard", data: chosen.body.kind === "dashboard" ? { ...chosen.body.data, widgets: [...chosen.body.data.widgets, widget] } : { widgets: [widget], variables: [], refreshSeconds: 0 } },
        }
      : { id: "", kind: "dashboard", connectionId, name: dashboardName.trim().length > 0 ? dashboardName.trim() : "Dashboard", body: { kind: "dashboard", data: { widgets: [widget], variables: [], refreshSeconds: 0 } }, tags: [], createdAt: "", updatedAt: "" };
    setSaving(true);
    try {
      const saved = await saveDocument(doc);
      setOpen(false);
      const alreadyOpen = useWorkspace.getState().tabs.some((t) => t.kind === "document" && t.documentId === saved.id);
      if (alreadyOpen) showInfo(`Widget added to ${saved.name}. Close and reopen its tab to see it.`);
      else {
        showInfo(`Widget added to ${saved.name}.`);
        openDocument("dashboard", saved.id, saved.connectionId);
      }
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogTrigger asChild>
        <Button size="xs" variant="soft" disabled={disabled}>
          <Icon name="plus" size={12} />
          Add to dashboard
        </Button>
      </DialogTrigger>
      <DialogContent className="sm:max-w-110">
        <DialogHeader>
          <DialogTitle>Add chart to a dashboard</DialogTitle>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-3">
          <AppSelect label="Dashboard" value={target} options={[{ value: NEW_DASHBOARD, label: "New dashboard" }, ...dashboards.map((d) => ({ value: d.id, label: d.name }))]} onChange={setTarget} />
          {chosen ? null : <Field label="Dashboard name" value={dashboardName} onChange={setDashboardName} />}
          <Field label="Widget title" value={title} onChange={setTitle} placeholder="e.g. Revenue by month" optional />
          {otherConnection ? <p className="text-xs text-warning">This dashboard queries {connectionName(chosen.connectionId)}; the widget will run there, not on this connection.</p> : null}
          <p className="text-xs text-muted">The widget re-runs this query each time the dashboard refreshes.</p>
        </DialogBody>
        <DialogFooter>
          <Button variant="tertiary" onClick={() => setOpen(false)}>Cancel</Button>
          <Button onClick={() => void save()} pending={saving}>Add widget</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
