// SOT: agent-artifact-view, agent-generated-ui, agent-ui-blocks, chat-chart, chat-graph, chat-result-grid, live-chart-switch
import { type ReactNode, useEffect, useMemo, useState } from "react";
import type { AgentArtifact, DocumentKind, Engine, QueryOutcome, UiBlock, UiStat, UiTone, WidgetKind } from "@/lib/bindings";
import { BarChart, LineChart, PieChart, SERIES_COLORS, chartData } from "@/features/dashboards/charts";
import { type Graph, extractGraph, labelColors, layout } from "@/features/tools/GraphViewTab";
import { ipc } from "@/lib/ipc";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { useWorkspace } from "@/stores/workspace";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { ResultTable, ResultToolbar } from "./InlineResult";
import { Markdown } from "./Markdown";
import { ipcMessage } from "./ipcMessage";

// WHAT:  Draws whatever the assistant asked the UI to draw — a chart, a grid, a
//        relationship graph, or a whole layout it composed out of blocks.
// WHY:   Some answers are a shape, not a sentence. Letting the agent assemble a
//        view from the app's own components means it can answer "how does this
//        database fit together" or "how healthy is this table" with something
//        the user reads at a glance and can then poke at.
// HOW:   The block vocabulary is closed and typed — the model picks components,
//        it never emits markup or code, so nothing it produces can execute here.
//        Charts and graphs reuse the dashboard and Graph-tool renderers, so a
//        view in the conversation looks identical to one built by hand.
// WHERE: src-tauri/src/services/agent/tools.rs (render_chart / render_table /
//        render_graph / render_ui), src/features/dashboards/charts.tsx

/// The forms a chart can be switched between after the fact, all from the same rows.
const CHART_FORMS: readonly { value: WidgetKind; label: string; icon: IconName }[] = [
  { value: "bar", label: "Bar", icon: "chart-bar" },
  { value: "line", label: "Line", icon: "chart" },
  { value: "area", label: "Area", icon: "activity" },
  { value: "pie", label: "Pie", icon: "gauge" },
  { value: "table", label: "Table", icon: "table" },
];

const TONE_CLASS: Record<UiTone, string> = {
  info: "border-accent/40 bg-accent/10",
  success: "border-success/40 bg-success-soft/50",
  warning: "border-warning/40 bg-warning-soft/50",
  danger: "border-danger/40 bg-danger-soft/50",
};

const TONE_ICON: Record<UiTone, IconName> = {
  info: "info",
  success: "check",
  warning: "alert",
  danger: "alert",
};

// ---------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------

function ChartBody({
  form,
  outcome,
  xLabel,
  yLabel,
}: {
  form: WidgetKind;
  outcome: QueryOutcome;
  xLabel: string | null;
  yLabel: string | null;
}) {
  const data = useMemo(() => chartData(outcome), [outcome]);
  if (form === "table") return <ResultTable outcome={outcome} />;
  if (data.series.length === 0 || data.labels.length === 0) {
    return (
      <div className="p-2">
        <p className="px-1 pb-1 text-[10.5px] text-muted">No numeric column to plot — showing the rows.</p>
        <ResultTable outcome={outcome} />
      </div>
    );
  }
  return (
    <div className="h-56 p-2">
      {form === "pie" ? (
        <PieChart data={data} tint={SERIES_COLORS[0]} />
      ) : form === "bar" ? (
        <BarChart data={data} colors={SERIES_COLORS} xLabel={xLabel} yLabel={yLabel} />
      ) : (
        <LineChart data={data} colors={SERIES_COLORS} area={form === "area"} xLabel={xLabel} yLabel={yLabel} />
      )}
    </div>
  );
}

// WHAT:  A compact, non-interactive graph for inside a message.
// WHY:   The Graph tool's canvas pans, zooms and expands — too much for a chat
//        bubble. This reuses that tool's layout and colour assignment, so the
//        same database draws the same picture in both places.
function MiniGraph({ graph }: { graph: Graph }) {
  const width = 560;
  const height = 260;
  const placed = useMemo(() => layout(graph, width, height), [graph]);
  const colors = useMemo(() => labelColors(graph), [graph]);
  const [hover, setHover] = useState<string | null>(null);

  if (graph.nodes.length === 0) {
    return <div className="p-2.5 text-[11px] text-muted">Nothing to draw.</div>;
  }

  return (
    <div className="p-2">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        className="h-56 w-full"
        role="img"
        aria-label={`Graph of ${graph.nodes.length} nodes`}
      >
        {graph.edges.map((edge) => {
          const from = placed.get(edge.from);
          const to = placed.get(edge.to);
          if (from === undefined || to === undefined) return null;
          const lit = hover === edge.from || hover === edge.to;
          return (
            <line
              key={edge.id}
              x1={from.x}
              y1={from.y}
              x2={to.x}
              y2={to.y}
              stroke="var(--separator)"
              strokeWidth={lit ? 1.6 : 1}
              opacity={hover === null || lit ? 0.9 : 0.25}
              vectorEffect="non-scaling-stroke"
            />
          );
        })}
        {graph.nodes.map((node) => {
          const spot = placed.get(node.id);
          if (spot === undefined) return null;
          const dim = hover !== null && hover !== node.id;
          return (
            <g
              key={node.id}
              opacity={dim ? 0.35 : 1}
              onMouseEnter={() => setHover(node.id)}
              onMouseLeave={() => setHover(null)}
            >
              <circle cx={spot.x} cy={spot.y} r={7} fill={colors.get(node.label) ?? SERIES_COLORS[0]} />
              <text
                x={spot.x}
                y={spot.y - 11}
                textAnchor="middle"
                fontSize={9}
                fill="var(--muted)"
                className="pointer-events-none"
              >
                {node.caption}
              </text>
            </g>
          );
        })}
      </svg>
      <div className="flex flex-wrap gap-1.5 px-1 pt-1">
        {[...colors.entries()].map(([label, color]) => (
          <span key={label} className="flex items-center gap-1 text-[10px] text-muted">
            <span className="size-2 rounded-full" style={{ backgroundColor: color }} />
            {label}
          </span>
        ))}
      </div>
    </div>
  );
}

function StatsRow({ stats }: { stats: UiStat[] }) {
  return (
    <div className="grid grid-cols-2 gap-2 p-2 sm:grid-cols-3 lg:grid-cols-4">
      {stats.map((stat) => (
        <div key={stat.label} className="rounded-lg border border-border/40 bg-surface-secondary/40 p-2">
          <p className="truncate text-[10px] text-muted">{stat.label}</p>
          <p className="selectable mt-0.5 truncate text-[13px] font-semibold tabular-nums text-foreground">
            {stat.value}
          </p>
          <div className="flex items-center gap-1">
            {stat.trend !== null ? (
              <span className={cn("text-[10px] tabular-nums", stat.trend >= 0 ? "text-success" : "text-danger")}>
                {stat.trend >= 0 ? "▲" : "▼"} {Math.abs(stat.trend).toFixed(1)}%
              </span>
            ) : null}
            {stat.hint !== null ? <span className="truncate text-[10px] text-muted">{stat.hint}</span> : null}
          </div>
        </div>
      ))}
    </div>
  );
}

function FactsList({ rows }: { rows: UiStat[] }) {
  return (
    <dl className="divide-y divide-border/30">
      {rows.map((row) => (
        <div key={row.label} className="flex items-baseline justify-between gap-2 px-2 py-1">
          <dt className="shrink-0 text-[11px] text-muted">{row.label}</dt>
          <dd className="selectable min-w-0 truncate text-right text-[11px] tabular-nums text-foreground">
            {row.value}
            {row.hint !== null ? <span className="ml-1 text-muted">{row.hint}</span> : null}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function ChartSwitch({ form, onChange }: { form: WidgetKind; onChange: (next: WidgetKind) => void }) {
  return (
    <div className="flex shrink-0 items-center gap-0.5 rounded-lg bg-surface-secondary/70 p-0.5">
      {CHART_FORMS.map((option) => (
        <button
          key={option.value}
          type="button"
          aria-label={option.label}
          title={option.label}
          onClick={() => onChange(option.value)}
          className={cn(
            "flex size-5 items-center justify-center rounded transition-colors",
            form === option.value ? "bg-accent/15 text-accent" : "text-muted hover:text-foreground",
          )}
        >
          <Icon name={option.icon} size={10} />
        </button>
      ))}
    </div>
  );
}

function ChartBlock({
  title,
  chart,
  outcome,
  xLabel,
  yLabel,
}: {
  title: string;
  chart: WidgetKind;
  outcome: QueryOutcome;
  xLabel: string | null;
  yLabel: string | null;
}) {
  // The assistant's choice is a starting point, not a verdict.
  const [form, setForm] = useState<WidgetKind>(chart);
  return (
    <div>
      <div className="flex items-center justify-between gap-2 px-2 pt-1">
        <span className="truncate text-[11px] font-medium text-foreground">{title}</span>
        <ChartSwitch form={form} onChange={setForm} />
      </div>
      <ChartBody form={form} outcome={outcome} xLabel={xLabel} yLabel={yLabel} />
    </div>
  );
}

/// Server-built nodes carry no properties; the shared canvas expects the field.
function toGraph(
  nodes: readonly { id: string; label: string; caption: string }[],
  edges: readonly { id: string; from: string; to: string; label: string }[],
): Graph {
  return {
    nodes: nodes.map((node) => ({ ...node, properties: null })),
    edges: edges.map((edge) => ({ id: edge.id, from: edge.from, to: edge.to, type: edge.label, properties: null })),
  };
}

function BlockView({ block, language }: { block: UiBlock; language: string }) {
  switch (block.block) {
    case "heading":
      return <h3 className="px-2 pt-1.5 text-xs font-semibold text-foreground">{block.text}</h3>;
    case "text":
      return (
        <div className="px-2 py-1">
          <Markdown text={block.markdown} language={language} />
        </div>
      );
    case "stats":
      return <StatsRow stats={block.stats} />;
    case "facts":
      return (
        <div className="py-1">
          {block.title.length > 0 ? (
            <p className="px-2 pb-1 text-[10px] font-semibold tracking-wider text-muted uppercase">
              {block.title}
            </p>
          ) : null}
          <FactsList rows={block.rows} />
        </div>
      );
    case "chart":
      return (
        <ChartBlock
          title={block.title}
          chart={block.chart}
          outcome={block.outcome}
          xLabel={block.xLabel}
          yLabel={block.yLabel}
        />
      );
    case "table":
      return (
        <div>
          {block.title.length > 0 ? (
            <p className="px-2 pt-1 text-[11px] font-medium text-foreground">{block.title}</p>
          ) : null}
          <ResultTable outcome={block.outcome} />
        </div>
      );
    case "graph":
      return (
        <div>
          {block.title.length > 0 ? (
            <p className="px-2 pt-1 text-[11px] font-medium text-foreground">{block.title}</p>
          ) : null}
          <MiniGraph graph={toGraph(block.nodes, block.edges)} />
        </div>
      );
    case "callout":
      return (
        <div className={cn("mx-2.5 my-1.5 rounded-lg border p-2", TONE_CLASS[block.tone])}>
          <div className="flex items-start gap-1.5">
            <Icon name={TONE_ICON[block.tone]} size={12} className="mt-0.5 shrink-0 text-muted" />
            <div className="min-w-0">
              <p className="text-[11px] font-semibold text-foreground">{block.title}</p>
              {block.body.length > 0 ? (
                <p className="selectable mt-0.5 text-[10.5px] text-muted">{block.body}</p>
              ) : null}
            </div>
          </div>
        </div>
      );
    case "divider":
      return <hr className="my-1.5 border-border/40" />;
  }
}

// ---------------------------------------------------------------------------
// Frame
// ---------------------------------------------------------------------------

function Frame({
  title,
  sql,
  right,
  children,
  onOpen,
}: {
  title: string;
  sql: string;
  right?: ReactNode;
  children: ReactNode;
  onOpen?: ((sql: string) => void) | undefined;
}) {
  const [showSql, setShowSql] = useState(false);
  const hasSql = sql.trim().length > 0;
  return (
    <div className="glass-card my-2 overflow-hidden rounded-lg border border-border/60 bg-surface/70">
      <div className="flex items-center justify-between gap-2 border-b border-border/40 px-2 py-1">
        <span className="truncate text-[11px] font-semibold text-foreground">{title}</span>
        <div className="flex shrink-0 items-center gap-1">
          {right}
          {hasSql ? (
            <>
              <Button
                size="sm"
                variant="ghost"
                className={cn("h-5 px-1.5 text-[10.5px]", showSql ? "text-accent" : "text-muted")}
                onClick={() => setShowSql((v) => !v)}
              >
                <Icon name="code" size={10} />
                Query
              </Button>
              {onOpen ? (
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-5 px-1.5 text-[10.5px] text-muted"
                  onClick={() => onOpen(sql)}
                >
                  Open in Studio
                </Button>
              ) : null}
            </>
          ) : null}
        </div>
      </div>
      {showSql && hasSql ? (
        <pre className="selectable border-b border-border/40 bg-surface-secondary/60 p-2 font-mono text-[10.5px] whitespace-pre-wrap text-muted">
          {sql}
        </pre>
      ) : null}
      {children}
    </div>
  );
}

const DOCUMENT_ICON: Record<DocumentKind, IconName> = {
  dashboard: "chart-bar",
  workflow: "flow",
  diagram: "chart-relationship",
};

const DOCUMENT_NOUN: Record<DocumentKind, string> = {
  dashboard: "dashboard",
  workflow: "workflow",
  diagram: "diagram",
};

// WHAT:  The card for a dashboard, workflow or diagram the assistant saved.
// WHY:   The tool already saved it (after the user approved); what is left is to
//        get from the conversation to the thing. Opening refreshes the list
//        first, because the editor reads it from the store's cache and an
//        updated document would otherwise open as it was.
function DocumentCard({
  title,
  kind,
  documentId,
  connectionId,
  updated,
  summary,
}: {
  title: string;
  kind: DocumentKind;
  documentId: string;
  connectionId: string | null;
  updated: boolean;
  summary: string;
}) {
  const loadDocuments = useWorkspace((s) => s.loadDocuments);
  const openDocument = useWorkspace((s) => s.openDocument);
  const closeTab = useWorkspace((s) => s.closeTab);
  const showError = useWorkspace((s) => s.showError);

  useEffect(() => {
    void (async () => {
      try {
        await loadDocuments(kind);
      } catch (raw) {
        showError(ipcMessage(raw));
      }
    })();
  }, [kind, loadDocuments, showError]);

  const open = () => {
    // An editor already open on this document holds the old copy and could save
    // it back over the update; a fresh tab reads the saved one.
    if (updated) closeTab(`doc:${kind}:${documentId}`);
    openDocument(kind, documentId, connectionId);
  };

  return (
    <div className="glass-card my-2 flex items-center gap-2 rounded-lg border border-border/60 bg-surface/70 p-2">
      <div className="flex size-7 shrink-0 items-center justify-center rounded-md bg-accent/15 text-accent">
        <Icon name={DOCUMENT_ICON[kind]} size={14} />
      </div>
      <div className="min-w-0 flex-1">
        <p className="truncate text-[11px] font-semibold text-foreground">{title}</p>
        <p className="truncate text-[10.5px] text-muted">
          {updated ? "Updated" : "Created"} {DOCUMENT_NOUN[kind]} · {summary}
        </p>
      </div>
      <Button size="sm" variant="secondary" className="glass-pill h-6 shrink-0 rounded-lg px-2 text-[11px] text-accent" onClick={open}>
        <Icon name="expand" size={10} />
        Open {DOCUMENT_NOUN[kind]}
      </Button>
    </div>
  );
}

// WHAT:  "Open in designer" for a relationship graph.
// WHY:   The graph in the chat is a picture of table names; the designer holds
//        real tables with columns. The server builds that diagram from the live
//        schema and saves it, so the button is one click from picture to editor.
function OpenInDesigner({ connectionId, title, tables }: { connectionId: string; title: string; tables: readonly string[] }) {
  const openDocument = useWorkspace((s) => s.openDocument);
  const loadDocuments = useWorkspace((s) => s.loadDocuments);
  const showError = useWorkspace((s) => s.showError);
  const [busy, setBusy] = useState(false);

  const open = () => {
    setBusy(true);
    void (async () => {
      try {
        const saved = await ipc("ai_diagram_from_schema", { connectionId, name: title, tables: [...tables] });
        await loadDocuments("diagram");
        openDocument("diagram", saved.id, connectionId);
      } catch (raw) {
        showError(ipcMessage(raw));
      } finally {
        setBusy(false);
      }
    })();
  };

  return (
    <Button size="sm" variant="ghost" className="h-5 px-1.5 text-[10.5px] text-muted" disabled={busy} onClick={open}>
      {busy ? <Spinner size="sm" /> : <Icon name="chart-relationship" size={10} />}
      Open in designer
    </Button>
  );
}

export function ArtifactView({
  artifact,
  language,
  engine,
  connectionId,
  onOpen,
}: {
  artifact: AgentArtifact;
  language: string;
  engine: Engine;
  connectionId: string;
  onOpen?: ((sql: string) => void) | undefined;
}) {
  const [form, setForm] = useState<WidgetKind>(artifact.kind === "chart" ? artifact.chart : "table");

  switch (artifact.kind) {
    case "table":
      return (
        <Frame title={artifact.title} sql={artifact.sql} onOpen={onOpen}>
          <ResultTable outcome={artifact.outcome} />
          <ResultToolbar outcome={artifact.outcome} engine={engine} sql={artifact.sql} />
        </Frame>
      );

    case "graph": {
      // Server-built (foreign keys) wins; otherwise read the rows the way the
      // Graph tool does, which already understands each engine's result shape.
      const fromKeys = artifact.nodes.length > 0;
      const graph = fromKeys
        ? toGraph(artifact.nodes, artifact.edges)
        : artifact.outcome !== null
          ? extractGraph(artifact.outcome)
          : { nodes: [], edges: [] };
      return (
        <Frame
          title={artifact.title}
          sql={artifact.sql}
          onOpen={onOpen}
          right={
            // Only a relationship map has tables behind it; a query's graph is data.
            fromKeys && artifact.sql.length === 0 ? (
              <OpenInDesigner connectionId={connectionId} title={artifact.title} tables={artifact.nodes.map((node) => node.id)} />
            ) : null
          }
        >
          <MiniGraph graph={graph} />
        </Frame>
      );
    }

    case "ui":
      return (
        <Frame title={artifact.title} sql="">
          <div className="flex flex-col py-0.5">
            {artifact.blocks.map((block, index) => (
              // Blocks are a positional list; there is no id to key on.
              <BlockView key={index} block={block} language={language} />
            ))}
          </div>
        </Frame>
      );

    case "chart":
      return (
        <Frame
          title={artifact.title}
          sql={artifact.sql}
          right={<ChartSwitch form={form} onChange={setForm} />}
          onOpen={onOpen}
        >
          <ChartBody form={form} outcome={artifact.outcome} xLabel={artifact.xLabel} yLabel={artifact.yLabel} />
          <ResultToolbar outcome={artifact.outcome} engine={engine} sql={artifact.sql} />
        </Frame>
      );

    case "document":
      return (
        <DocumentCard
          title={artifact.title}
          kind={artifact.documentKind}
          documentId={artifact.documentId}
          connectionId={artifact.connectionId}
          updated={artifact.updated}
          summary={artifact.summary}
        />
      );
  }
}
