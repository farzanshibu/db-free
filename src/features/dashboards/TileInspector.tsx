// SOT: tile-inspector, tile-options-panel, tile-draft, tile-sql-editor, tile-preview, tile-presentation-options
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { DashboardVariable, Engine, QueryOutcome, Widget, WidgetKind } from "@/lib/bindings";
import { formatCount, formatMs } from "@/lib/format";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { useWorkspace } from "@/stores/workspace";
import { AppSelect, Field, Toggle } from "@/components/global/Field";
import { IconButton } from "@/components/global/Button";
import { DataGrid } from "@/features/grid/DataGrid";
import { SqlEditor } from "@/features/editor/SqlEditor";
import { templateVariablesFromTokens } from "@/features/editor/completion";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { tileAppearance } from "./appearance";
import { rowsOf } from "./cells";
import { buildCompletionSchema, mentionedTables } from "./completionSchema";
import { ConditionsEditor } from "./ConditionsEditor";
import { useDashboardResults } from "./dashboardResults";
import { CURRENCY_CODES, LEGEND_MODES, METRIC_FORMATS, VALUE_FORMATS, currencyExample, currencySymbol, formatChartValue } from "./format";
import { TileBody } from "./TileBody";
import { TileColorPicker } from "./TileColorPicker";
import { variableTokens } from "./variables";

// WHAT:  The side panel that edits one tile: title and kind, a live preview, the
//        options of its kind, the SQL (with column-aware completion and a Run
//        that previews the result), and its conditions.
// WHY:   Editing a tile should not change the dashboard until the edit is
//        accepted, and a half-finished edit should survive clicking another tile.
//        So the panel edits a *draft*: Save applies it, Reset returns to what is
//        applied, Cancel closes and throws it away (and removes a tile that was
//        only just drawn). A draft left open is kept in memory per tile.
// HOW:   Layout (x / y / w / h) is the grid's, not the draft's: Save merges the
//        draft's content with the tile's current position, so moving a tile
//        while its panel is open is not undone. Run sends the draft's SQL with
//        the dashboard's variables filled in, without touching the tile's cached
//        result, and the preview draws that. Completion learns the columns of the
//        tables the statement mentions.
// WHERE: ./DashboardTab.tsx, ./ConditionsEditor.tsx, ./completionSchema.ts, ./dashboardResults.ts

export const WIDGET_KINDS: readonly { value: WidgetKind; label: string }[] = [
  { value: "area", label: "Area Chart" },
  { value: "line", label: "Line Chart" },
  { value: "bar", label: "Bar Chart" },
  { value: "pie", label: "Pie Chart" },
  { value: "sankey", label: "Sankey Diagram" },
  { value: "table", label: "Table" },
  { value: "metric", label: "Metric" },
  { value: "sparkline", label: "Sparkline" },
  { value: "map", label: "Map" },
  { value: "progress", label: "Progress" },
  { value: "text", label: "Text" },
  { value: "image", label: "Image" },
  { value: "gif", label: "GIF" },
];

const CHARTS: readonly WidgetKind[] = ["area", "line", "bar", "pie"];
const CONDITIONAL: readonly WidgetKind[] = ["metric", "text", "image", "gif"];
const BACKGROUNDS: readonly WidgetKind[] = ["metric", "text", "image", "gif"];
const NO_TINT: readonly WidgetKind[] = ["table", "text", "image", "gif"];

const CURRENCY_OPTIONS = CURRENCY_CODES.map((code) => ({ value: code, label: `${code} (${currencySymbol(code)})` }));

/// The tile without its position, as text: what "has this been edited" compares.
function contentKey(w: Widget): string {
  return JSON.stringify({ ...w, x: 0, y: 0, w: 0, h: 0 });
}

interface InspectorProps {
  dashboardId: string;
  /// The tile as the dashboard has it.
  widget: Widget;
  /// Drawn a moment ago and never accepted: Cancel removes it.
  isNew: boolean;
  connectionId: string | null;
  engine: Engine;
  variables: readonly DashboardVariable[];
  /// The tile's latest result in the grid, shown until Run is pressed.
  cachedOutcome: QueryOutcome | null;
  /// Why the tile's own last run failed, if it did.
  cachedError: string | null;
  rowHeight: number;
  preview: (sql: string) => Promise<{ outcome: QueryOutcome | null; error: string | null }>;
  onApply: (widget: Widget) => void;
  /// Close the panel, keeping an unsaved edit for next time.
  onClose: () => void;
  /// Discard the edit and close (removing a tile that was only just drawn).
  onCancel: () => void;
  onDelete: () => void;
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-2 border-b border-border px-2.5 py-1.5 text-xs">
      <span className="font-medium text-foreground">{title}</span>
      {children}
    </div>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-2">
      <span className="w-24 shrink-0 text-muted">{label}</span>
      {children}
    </div>
  );
}

export function TileInspector({ dashboardId, widget, isNew, connectionId, engine, variables, cachedOutcome, cachedError, rowHeight, preview, onApply, onClose, onCancel, onDelete }: InspectorProps) {
  const stored = useDashboardResults((s) => s.drafts[dashboardId]?.[widget.id]);
  const setStored = useDashboardResults((s) => s.setDraft);
  const [draft, setDraft] = useState<Widget>(stored ?? widget);
  const [ran, setRan] = useState<{ sql: string; outcome: QueryOutcome | null; error: string | null } | null>(null);
  const [busy, setBusy] = useState(false);
  const textarea = useRef<HTMLTextAreaElement | null>(null);
  const catalog = useWorkspace((s) => (connectionId === null ? undefined : s.catalogs[connectionId]));
  const columnsCache = useWorkspace((s) => s.columnsCache);
  const loadColumns = useWorkspace((s) => s.loadColumns);

  const patch = (partial: Partial<Widget>) => { setDraft((d) => ({ ...d, ...partial })); };
  const dirty = contentKey(draft) !== contentKey(widget);

  // An edit left open is kept for when this tile is selected again.
  useEffect(() => {
    setStored(dashboardId, widget.id, dirty ? draft : null);
  }, [dashboardId, widget.id, dirty, draft, setStored]);

  // Completion: columns for the tables the statement names.
  const tables = useMemo(() => mentionedTables(draft.sql, catalog), [draft.sql, catalog]);
  const tablesKey = tables.map((t) => `${t.schema ?? ""}.${t.name}`).join("|");
  useEffect(() => {
    if (connectionId === null) return;
    for (const t of tables) void loadColumns(connectionId, t).catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `tables` is keyed by `tablesKey`
  }, [connectionId, tablesKey, loadColumns]);
  const schema = useMemo(() => (connectionId === null ? {} : buildCompletionSchema(connectionId, catalog, columnsCache)), [connectionId, catalog, columnsCache]);

  // What the preview shows: the last Run of exactly this SQL, else the tile's own result.
  const shown = ran !== null && ran.sql === draft.sql ? ran : draft.sql === widget.sql ? { sql: widget.sql, outcome: cachedOutcome, error: cachedError } : null;
  const outcome = shown?.outcome ?? null;
  const appearance = useMemo(() => tileAppearance(draft, outcome), [draft, outcome]);
  const rows = rowsOf(outcome);
  const columnNames = rows?.columns.map((c) => c.name) ?? [];

  const run = async () => {
    if (draft.sql.trim().length === 0 || connectionId === null) return;
    setBusy(true);
    const result = await preview(draft.sql);
    setRan({ sql: draft.sql, ...result });
    setBusy(false);
  };

  const insertToken = (column: string) => {
    const el = textarea.current;
    const token = `{{${column}}}`;
    const text = draft.text ?? "";
    const at = el?.selectionStart ?? text.length;
    const end = el?.selectionEnd ?? at;
    patch({ text: text.slice(0, at) + token + text.slice(end) });
  };

  const save = () => { onApply({ ...draft, x: widget.x, y: widget.y, w: widget.w, h: widget.h }); };
  const isChart = CHARTS.includes(draft.kind);
  const showTint = !NO_TINT.includes(draft.kind);
  const tintLabel = draft.kind === "sankey" ? "Flow colour" : draft.kind === "line" || draft.kind === "area" ? "Line colour" : "Colour";
  const pending = isNew || dirty;

  return (
    <>
      <div className="flex items-center gap-2 border-b border-border/40 px-2.5 py-1.5">
        <Field label="" value={draft.title} onChange={(title) => { patch({ title }); }} placeholder="Tile title" className="[&_label]:hidden" />
        <AppSelect ariaLabel="Tile type" value={draft.kind} options={WIDGET_KINDS} onChange={(kind) => { patch({ kind }); }} className="w-40 shrink-0" />
        <IconButton icon="x" label="Close panel" onClick={onClose} />
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div
          className="relative h-48 shrink-0 overflow-hidden border-b border-border bg-surface"
          style={{ borderTopWidth: 2, borderTopColor: appearance.accent, backgroundImage: appearance.background }}
        >
          <div className="flex h-6 items-center px-2 text-xs">
            <span className="truncate text-muted" style={appearance.onGradient ? { color: "var(--tile-on-gradient)" } : undefined}>{draft.title.length > 0 ? draft.title : "Preview"}</span>
          </div>
          <div className="h-[calc(100%-24px)]">
            <TileBody widget={draft} outcome={outcome} appearance={appearance} rowHeight={rowHeight} error={shown?.error ?? null} />
          </div>
        </div>

        <Section title="Options">
          {showTint ? (
            <Row label={tintLabel}>
              <TileColorPicker kind="tint" label={tintLabel} value={draft.tint} onChange={(tint) => { patch({ tint }); }} />
            </Row>
          ) : null}
          {BACKGROUNDS.includes(draft.kind) ? (
            <Row label="Background">
              <TileColorPicker kind="background" label="Background" value={draft.bg} onChange={(bg) => { patch({ bg }); }} />
            </Row>
          ) : null}

          {draft.kind === "metric" ? (
            <>
              <Row label="Format">
                <AppSelect ariaLabel="Metric format" size="sm" className="w-32" value={draft.format} options={METRIC_FORMATS} onChange={(format) => { patch({ format }); }} />
                {draft.format === "currency" ? <AppSelect ariaLabel="Currency" size="sm" className="w-36" value={draft.currency} options={CURRENCY_OPTIONS} onChange={(currency) => { patch({ currency }); }} /> : null}
                <span className="font-mono text-[11px] text-muted">{draft.format === "currency" ? currencyExample(draft.currency) : ""}</span>
              </Row>
              <Toggle checked={draft.showChange} onChange={(showChange) => { patch(showChange ? { showChange } : { showChange, colorByChange: false }); }} label="Show change" description="Compares the first number with the second column's number, or the last row with the one before." />
              <Toggle checked={draft.colorByChange} onChange={(colorByChange) => { patch({ colorByChange }); }} label="Colour by change" description={draft.showChange ? "Green when up, red when down." : "Turn on Show change first."} />
            </>
          ) : null}
          {draft.kind === "sparkline" ? <Toggle checked={draft.showChange} onChange={(showChange) => { patch({ showChange }); }} label="Show change" /> : null}
          {draft.kind === "area" ? <Toggle checked={draft.stacked} onChange={(stacked) => { patch({ stacked }); }} label="Stacked" description="Series add up instead of overlapping." /> : null}
          {draft.kind === "bar" ? <Toggle checked={draft.horizontal} onChange={(horizontal) => { patch({ horizontal }); }} label="Horizontal" /> : null}
          {draft.kind === "map" ? <Toggle checked={draft.pulse} onChange={(pulse) => { patch({ pulse }); }} label="Pulse" /> : null}
          {draft.kind === "progress" ? (
            <>
              <Toggle checked={draft.showPercent} onChange={(showPercent) => { patch({ showPercent }); }} label="Show %" />
              <Toggle checked={draft.showValues} onChange={(showValues) => { patch({ showValues }); }} label="Show values" />
              <Field label="Max" type="number" value={draft.maxValue === null ? "" : String(draft.maxValue)} onChange={(v) => { patch({ maxValue: v.trim().length > 0 && Number.isFinite(Number(v)) ? Number(v) : null }); }} placeholder="from 2nd column" className="w-36" />
            </>
          ) : null}
          {draft.kind === "line" || draft.kind === "area" || draft.kind === "bar" ? (
            <div className="flex items-end gap-2">
              <Field label="X label" value={draft.xLabel ?? ""} onChange={(v) => { patch({ xLabel: v.length > 0 ? v : null }); }} placeholder="e.g. Month" />
              <Field label="Y label" value={draft.yLabel ?? ""} onChange={(v) => { patch({ yLabel: v.length > 0 ? v : null }); }} placeholder="e.g. Revenue" />
            </div>
          ) : null}
          {draft.kind === "text" ? (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="tile-text">Content</Label>
              <Textarea id="tile-text" ref={textarea} value={draft.text ?? ""} placeholder="Content to display" className="min-h-16 text-xs" onChange={(event) => { patch({ text: event.target.value }); }} />
              <p className="text-muted">Use {"{{column_name}}"} to insert a value, or arithmetic like {"{{round(won / total * 100, 1)}}"}.</p>
              {columnNames.length > 0 ? (
                <div className="flex flex-wrap items-center gap-1">
                  <span className="text-[10px] text-muted">Columns</span>
                  {columnNames.map((name) => (
                    <button key={name} type="button" className="rounded bg-accent/10 px-1.5 py-0.5 font-mono text-[10px] text-accent ring-1 ring-accent/25 hover:bg-accent/20" onClick={() => { insertToken(name); }}>
                      {name}
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
          ) : null}
          {draft.kind === "image" || draft.kind === "gif" ? (
            <Field label={draft.kind === "gif" ? "GIF URL" : "Image URL"} value={draft.url ?? ""} onChange={(v) => { patch({ url: v }); }} placeholder="https://example.com/image.png" description="Use {{column_name}} to build the URL from query values." mono />
          ) : null}
          {draft.kind === "sankey" ? <p className="text-muted">The first three columns are read as source, target, and value. Each row is a flow from source to target; duplicate pairs are summed.</p> : null}
          {draft.kind === "map" ? <p className="text-muted">Return lat and lon columns plus an optional label column. Drag to pan; Ctrl or ⌘ with the wheel, or the buttons, to zoom.</p> : null}
          {draft.kind === "table" ? <p className="font-mono text-muted">SELECT * FROM table LIMIT 100</p> : null}
        </Section>

        {isChart ? (
          <Section title="Presentation">
            <Row label="Legend">
              <AppSelect ariaLabel="Legend" size="sm" className="w-36" value={draft.legendMode} options={LEGEND_MODES} onChange={(legendMode) => { patch({ legendMode }); }} />
            </Row>
            <Row label="Value format">
              <AppSelect ariaLabel="Value format" size="sm" className="w-36" value={draft.valueFormat} options={VALUE_FORMATS} onChange={(valueFormat) => { patch({ valueFormat }); }} />
              <span className="font-mono text-[11px] text-muted">{formatChartValue(1234.5, draft.valueFormat, draft.currency)}</span>
            </Row>
            {draft.valueFormat === "currency" ? (
              <Row label="Currency">
                <AppSelect ariaLabel="Currency" size="sm" className="w-36" value={draft.currency} options={CURRENCY_OPTIONS} onChange={(currency) => { patch({ currency }); }} />
              </Row>
            ) : null}
          </Section>
        ) : null}

        <div className="flex h-60 shrink-0 flex-col border-b border-border">
          <div className="min-h-0 flex-1">
            <SqlEditor value={draft.sql} onChange={(sql) => { patch({ sql }); }} onRun={() => void run()} engine={engine} schema={schema} templateVars={templateVariablesFromTokens(variableTokens(variables))} />
          </div>
          <div className="flex items-center gap-2 border-t border-border px-2 py-1">
            <Button size="sm" onClick={() => void run()} disabled={connectionId === null || draft.sql.trim().length === 0} pending={busy}>
              <Icon name="play" size={12} />
              Run Query
            </Button>
            {variables.length > 0 ? <span className="truncate text-[11px] text-muted" title={variableTokens(variables).map((t) => `{{${t.token}}}`).join("  ")}>Variables: {variableTokens(variables).slice(0, 3).map((t) => `{{${t.token}}}`).join(" ")}{variableTokens(variables).length > 3 ? " …" : ""}</span> : null}
            {rows ? <span className="ml-auto shrink-0 text-[11px] text-muted">{formatCount(rows.rows.length)} row{rows.rows.length === 1 ? "" : "s"}, {formatMs(outcome?.elapsedMs ?? 0)}</span> : null}
          </div>
          {shown?.error ? <p className="border-t border-border px-2.5 py-1 text-[11px] break-words text-danger">{shown.error}</p> : null}
          {rows && rows.columns.length > 0 ? (
            <div className="h-20 shrink-0 border-t border-border">
              <DataGrid columns={rows.columns.map((c) => ({ name: c.name, typeName: c.typeName }))} rowCount={Math.min(rows.rows.length, 5)} getRow={(i) => rows.rows[i]} rowHeight={22} />
            </div>
          ) : null}
        </div>

        {CONDITIONAL.includes(draft.kind) ? <ConditionsEditor widget={draft} outcome={outcome} onChange={(conditions) => { patch({ conditions }); }} /> : null}
      </ScrollArea>
      <div className="flex shrink-0 items-center gap-1.5 border-t border-border px-2.5 py-1.5">
        <Button size="sm" variant="danger-soft" onClick={onDelete}>
          <Icon name="trash" size={12} />
          Delete
        </Button>
        <span className="ml-auto" />
        <Button size="sm" variant="ghost" onClick={() => { setDraft(widget); setRan(null); }} disabled={!dirty}>
          Reset
        </Button>
        <Button size="sm" variant="secondary" onClick={onCancel}>
          Cancel
        </Button>
        <Button size="sm" onClick={save} disabled={!pending} className={cn(pending ? "" : "opacity-60")}>
          Save
        </Button>
      </div>
    </>
  );
}

