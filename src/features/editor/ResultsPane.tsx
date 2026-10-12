// SOT: results-pane, statement-tabs, query-result-grid, result-row-total, result-export, file-download, grid-chart-toggle, statement-status-tabs, results-collapse, run-without-limit
import { useState } from "react";
import type { QueryOutcome, StatementResult, Value } from "@/lib/bindings";
import { formatCount, formatMs } from "@/lib/format";
import { downloadTextFile, exportFilename, toCsvText, toJsonText, type ExportFormat } from "@/lib/export";
import { useWorkspace } from "@/stores/workspace";
import type { StatementRun, TabRun } from "@/stores/queryRuns";
import { ResultGrid } from "./ResultGrid";
import { ResultChart } from "./ResultChart";
import { QueryErrorPanel } from "./QueryErrorPanel";
import { Segmented } from "@/components/global/Field";
import { EmptyState } from "@/components/global/EmptyState";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { DropdownMenu, DropdownMenuContent, DropdownMenuGroup, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { ExportDialog } from "@/features/transfer/ExportDialog";
import { resultScopes } from "@/features/transfer/exportScopes";

// WHAT:  Says how many rows the script really has, not just how many fit.
// WHY:   A row cap that reads as the whole answer is worse than no cap: "1,000
//        rows" and "1,000 of 84,213 rows" lead to opposite conclusions.
// WHERE: src-tauri/src/services/query.rs (QueryOutcome.totalRows)
function rowSummary(shown: number, total: number | null, truncated: boolean): string {
  if (!truncated) return `${formatCount(shown)} rows`;
  if (total === null) return `${formatCount(shown)} rows (capped)`;
  return `${formatCount(shown)} of ${formatCount(total)} rows`;
}

/// One number per outcome object, so per-result view state (filters, sort,
/// edits) starts clean when a new run replaces the outcome.
const outcomeIds = new WeakMap<QueryOutcome, number>();
let outcomeCounter = 0;
function outcomeId(outcome: QueryOutcome): number {
  const known = outcomeIds.get(outcome);
  if (known !== undefined) return known;
  outcomeCounter += 1;
  outcomeIds.set(outcome, outcomeCounter);
  return outcomeCounter;
}

// WHAT:  One tab of the strip: a statement, or — when a statement returned
//        several result sets — one of them.
interface ResultTab {
  key: string;
  statement: StatementRun;
  result: StatementResult | null;
  label: string;
}

function tabsOf(run: TabRun): ResultTab[] {
  const tabs: ResultTab[] = [];
  for (const statement of run.statements) {
    const results = statement.outcome?.statements ?? [];
    if (results.length > 1) {
      results.forEach((result, sub) => tabs.push({ key: `${statement.index}:${sub}`, statement, result, label: "" }));
    } else {
      tabs.push({ key: `${statement.index}:0`, statement, result: results[0] ?? null, label: "" });
    }
  }
  return tabs.map((tab, i) => ({ ...tab, label: tabLabel(tab, i, tabs) }));
}

// WHAT:  A tab's name: the shell command that produced it (`\dt`, `.schema users`), else "Result n".
// WHY:   `\d users` answers with several result sets; a tab named after what was typed
//        says which question it answers, and the number tells its sections apart.
function tabLabel(tab: ResultTab, index: number, all: readonly ResultTab[]): string {
  const command = shellCommandTitle(tab.statement.sql);
  if (command === null) return `Result ${index + 1}`;
  const siblings = all.filter((t) => t.statement.index === tab.statement.index);
  return siblings.length > 1 ? `${command} · ${siblings.indexOf(tab) + 1}` : command;
}

/// The first line of a psql / sqlite3 style command, shortened for a tab; null for anything else.
function shellCommandTitle(sql: string): string | null {
  const line = (sql.trim().split("\n")[0] ?? "").trim();
  if (!/^(\\|\.[A-Za-z])/.test(line)) return null;
  return line.length > 24 ? `${line.slice(0, 23)}…` : line;
}

/// The tab to show before the user picks one: the one still working, then the first failure, then the first.
function defaultTab(run: TabRun, tabs: readonly ResultTab[]): ResultTab | undefined {
  if (run.running) {
    const started = tabs.filter((t) => t.statement.state !== "pending");
    return started[started.length - 1] ?? tabs[0];
  }
  return tabs.find((t) => t.statement.state === "error") ?? tabs[0];
}

function tabMeta(tab: ResultTab): string | null {
  const { statement, result } = tab;
  if (statement.state === "success") {
    const detail = result?.kind === "rows" ? `${formatCount(result.result.rows.length)} rows` : result?.kind === "affected" ? "OK" : null;
    const time = statement.elapsedMs === null ? null : formatMs(statement.elapsedMs);
    return [detail, time].filter((part): part is string => part !== null).join(" · ");
  }
  if (statement.state === "error" || statement.state === "cancelled") return statement.elapsedMs === null ? null : formatMs(statement.elapsedMs);
  return null;
}

function StatusIcon({ state }: { state: StatementRun["state"] }) {
  switch (state) {
    case "running":
      return <Spinner size="sm" className="text-accent" />;
    case "success":
      return <Icon name="check" size={11} className="text-success" />;
    case "error":
      return <Icon name="alert" size={11} className="text-danger" />;
    case "cancelled":
      return <Icon name="stop" size={11} className="text-warning" />;
    case "skipped":
      return <Icon name="minus" size={11} className="text-muted" />;
    case "pending":
      return <Icon name="clock" size={11} className="text-muted" />;
  }
}

interface ResultsPaneProps {
  /// The tab's last run, or nothing before the first.
  run: TabRun | undefined;
  connectionId: string;
  collapsed: boolean;
  onToggleCollapsed: () => void;
  /// Jump the editor to the spot a failed statement pointed at.
  onGoToError: (statement: StatementRun) => void;
  /// Hand a failed statement and its error to the AI.
  onAskAi: ((statement: StatementRun) => void) | undefined;
  /// Run a capped statement again with no row cap.
  onRunWithoutLimit: (statement: StatementRun) => void;
}

export function ResultsPane({ run, connectionId, collapsed, onToggleCollapsed, onGoToError, onAskAi, onRunWithoutLimit }: ResultsPaneProps) {
  const showInfo = useWorkspace((s) => s.showInfo);
  const [picked, setPicked] = useState<{ seq: number; key: string } | null>(null);
  // Grid or chart; kept across runs, since re-running a charted query should stay a chart.
  const [view, setView] = useState<"grid" | "chart">("grid");
  const [exportOpen, setExportOpen] = useState(false);
  const [ticked, setTicked] = useState<readonly (readonly Value[])[]>([]);

  const tabs = run === undefined ? [] : tabsOf(run);
  const chosen = run !== undefined && picked?.seq === run.seq ? tabs.find((t) => t.key === picked.key) : undefined;
  const current = chosen ?? (run === undefined ? undefined : defaultTab(run, tabs));
  const rowsResult = current?.result?.kind === "rows" ? current.result.result : null;
  const exportable = rowsResult !== null && rowsResult.columns.length > 0 && rowsResult.rows.length > 0;
  const truncated = rowsResult?.truncated === true;
  const outcome = current?.statement.outcome ?? null;
  const total = outcome !== null && outcome.statements.length === 1 ? outcome.totalRows : null;
  const params = run?.params ?? null;
  const capped = params !== null && params.maxRows !== null;
  const rerunning = current !== undefined && run?.rerunning === current.statement.index;
  const summary = run?.summary ?? null;
  const elapsed = current?.statement.elapsedMs ?? null;
  // The tab's place in the strip, for the export's file name and title.
  const position = current === undefined ? 1 : tabs.indexOf(current) + 1;

  // WHAT:  Export for the visible result set: clipboard copy or a real file
  //        download, in CSV or JSON. The outcome already holds every returned
  //        row (up to the tab's row cap), so this is the complete result.
  // HOW:   Serialized with the same shared helper the table grid uses, so
  //        escaping and NULL handling cannot disagree between the two.
  const exportResult = async (mode: "copy" | "download", format: ExportFormat) => {
    if (rowsResult === null) return;
    const text = format === "json" ? toJsonText(rowsResult.columns, rowsResult.rows) : toCsvText(rowsResult.columns, rowsResult.rows);
    if (mode === "copy") {
      await navigator.clipboard.writeText(text);
      showInfo(`Copied ${formatCount(rowsResult.rows.length)} row(s) as ${format.toUpperCase()} to the clipboard.`);
    } else {
      downloadTextFile(exportFilename(`result-${position}`, format), text, format === "json" ? "application/json" : "text/csv");
      showInfo(`Downloaded ${formatCount(rowsResult.rows.length)} row(s) as ${format.toUpperCase()}.`);
    }
  };

  const onExportAction = (key: string) => {
    if (key === "copy-csv") void exportResult("copy", "csv");
    else if (key === "copy-json") void exportResult("copy", "json");
    else if (key === "download-csv") void exportResult("download", "csv");
    else if (key === "download-json") void exportResult("download", "json");
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-7 shrink-0 items-center gap-1 border-b border-border bg-surface px-2 text-xs">
        <div className="flex min-w-0 items-center gap-1 overflow-x-auto">
          {tabs.length > 1 ? (
            tabs.map((tab) => (
              <Button
                key={tab.key}
                size="sm"
                variant={tab.key === current?.key ? "secondary" : "ghost"}
                onClick={() => { if (run !== undefined) setPicked({ seq: run.seq, key: tab.key }); }}
                title={tab.statement.sql}
                className={cn("h-6 shrink-0 gap-1.5 rounded-md px-2 py-0.5 text-xs", tab.key === current?.key ? "font-medium text-foreground" : "text-muted hover:text-foreground")}
              >
                <StatusIcon state={tab.statement.state} />
                {tab.label}
                {tabMeta(tab) !== null ? <span className="font-normal text-muted">{tabMeta(tab)}</span> : null}
              </Button>
            ))
          ) : (
            <span className="px-1 font-medium text-muted">Results</span>
          )}
        </div>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {rowsResult !== null && rowsResult.columns.length > 0 && !collapsed ? (
            <Segmented label="Result view" value={view} onChange={setView} options={[{ value: "grid", label: "Grid" }, { value: "chart", label: "Chart" }]} />
          ) : null}
          {rowsResult !== null && !collapsed ? (
            <>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button size="sm" variant="ghost" className="h-6 rounded-md px-2 text-xs text-muted hover:text-foreground" disabled={!exportable}>
                  <Icon name="download" size={12} />
                  Export
                  <Icon name="chevron-down" size={11} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent className="glass-modal rounded-lg">
                <DropdownMenuGroup>
                  <DropdownMenuItem textValue="Copy as CSV" onSelect={() => { onExportAction("copy-csv"); }}><span>Copy as CSV</span></DropdownMenuItem>
                  <DropdownMenuItem textValue="Copy as JSON" onSelect={() => { onExportAction("copy-json"); }}><span>Copy as JSON</span></DropdownMenuItem>
                  <DropdownMenuItem textValue="Download as CSV" onSelect={() => { onExportAction("download-csv"); }}><span>Download as CSV</span></DropdownMenuItem>
                  <DropdownMenuItem textValue="Download as JSON" onSelect={() => { onExportAction("download-json"); }}><span>Download as JSON</span></DropdownMenuItem>
                  <DropdownMenuItem textValue="Export with options" onSelect={() => { setExportOpen(true); }}><span>Export with options…</span></DropdownMenuItem>
                </DropdownMenuGroup>
              </DropdownMenuContent>
            </DropdownMenu>
            <ExportDialog
              open={exportOpen}
              onOpenChange={setExportOpen}
              connectionId={connectionId}
              title={`result ${position}`}
              baseName={`result-${position}`}
              table={null}
              columns={rowsResult.columns}
              scopes={resultScopes(rowsResult.rows, ticked)}
            />
            </>
          ) : null}
          {truncated && capped && current !== undefined && !collapsed ? (
            <Button size="xs" variant="soft" pending={rerunning} onClick={() => { onRunWithoutLimit(current.statement); }}>
              <Icon name="refresh" size={11} />
              Run without limit
            </Button>
          ) : null}
          {rowsResult !== null ? (
            <Badge size="sm" variant={truncated ? "warning" : "soft"}>
              {rowSummary(rowsResult.rows.length, total, truncated)}
            </Badge>
          ) : null}
          {summary !== null && tabs.length > 1 ? (
            <Badge size="sm" variant={summary.failed > 0 ? "danger" : "success"}>
              {summary.failed > 0 ? `${summary.succeeded} ok · ${summary.failed} failed` : `${summary.succeeded} ok`}
              {summary.skipped > 0 ? ` · ${summary.skipped} skipped` : ""}
              {` · ${formatMs(summary.elapsedMs)}`}
            </Badge>
          ) : elapsed !== null ? (
            <Badge size="sm" variant={current?.statement.state === "error" ? "danger" : "success"}>
              {formatMs(elapsed)}
            </Badge>
          ) : null}
          <Button size="icon-sm" variant="ghost" aria-label={collapsed ? "Show results" : "Hide results"} aria-expanded={!collapsed} onClick={onToggleCollapsed}>
            <Icon name="chevron-down" size={12} className={collapsed ? "rotate-180" : undefined} />
          </Button>
        </span>
      </div>
      <div className={cn("min-h-0 flex-1", collapsed ? "hidden" : undefined)}>
        {run === undefined || (run.statements.length === 0 && run.fatal === null && !run.running) ? (
          <EmptyState icon="terminal" title="No results yet" body="Run a query with ⌘/Ctrl + Enter. Results appear here." />
        ) : run.statements.length === 0 ? (
          run.fatal !== null ? (
            <QueryErrorPanel error={run.fatal} sql={null} onCopy={copyError(showInfo)} />
          ) : (
            <Waiting label="Starting…" />
          )
        ) : current === undefined ? null : (
          <TabBody
            run={run}
            tab={current}
            view={view}
            connectionId={connectionId}
            onGoToError={onGoToError}
            onAskAi={onAskAi}
            onTick={setTicked}
            onCopy={copyError(showInfo)}
          />
        )}
      </div>
    </div>
  );
}

function copyError(showInfo: (message: string) => void): (text: string) => void {
  return (text) => {
    void navigator.clipboard.writeText(text);
    showInfo("Copied the error.");
  };
}

function Waiting({ label }: { label: string }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-xs text-muted">
      <Spinner size="lg" className="text-accent" />
      {label}
    </div>
  );
}

interface TabBodyProps {
  run: TabRun;
  tab: ResultTab;
  view: "grid" | "chart";
  connectionId: string;
  onGoToError: (statement: StatementRun) => void;
  onAskAi: ((statement: StatementRun) => void) | undefined;
  onTick: (rows: readonly (readonly Value[])[]) => void;
  onCopy: (text: string) => void;
}

// WHAT:  The body of one tab, by how its statement ended.
function TabBody({ run, tab, view, connectionId, onGoToError, onAskAi, onTick, onCopy }: TabBodyProps) {
  const { statement, result } = tab;
  switch (statement.state) {
    case "pending":
      return <EmptyState icon="clock" title="Waiting" body="This statement runs after the ones above it." />;
    case "running":
      return <Waiting label="Running…" />;
    case "skipped":
      return <EmptyState icon="minus" title="Skipped" body={run.params?.stopOnError === true ? "An earlier statement failed, and the run stops at the first failure." : "The run was stopped before this statement."} />;
    case "cancelled":
      return <EmptyState icon="stop" title="Cancelled" body="You stopped this statement before it finished." />;
    case "error":
      return statement.error === null ? (
        <EmptyState icon="alert" title="Failed" body="The statement failed." />
      ) : (
        <QueryErrorPanel
          error={statement.error}
          sql={statement.sql}
          onGoToError={() => { onGoToError(statement); }}
          onAskAi={onAskAi === undefined ? undefined : () => { onAskAi(statement); }}
          onCopy={onCopy}
        />
      );
    case "success":
      break;
  }
  const outcome = statement.outcome;
  if (result === null || outcome === null) return <EmptyState title="Statement executed" body="No result set was returned." />;
  if (result.kind === "affected") return <EmptyState icon="check" title="Statement OK" body={`${formatCount(result.rowsAffected)} row(s) affected.`} />;
  if (result.result.columns.length === 0) return <EmptyState title="Empty result" body="The statement returned no rows." />;
  const key = `${outcomeId(outcome)}:${tab.key}`;
  return (
    <>
      {/* Both views stay mounted so the grid's filters, sort and edits and the chart's picks survive a switch. */}
      <div className={view === "grid" ? "h-full" : "hidden"}>
        <ResultGrid key={key} connectionId={connectionId} sql={statement.sql} result={result.result} single={run.statements.length === 1 && outcome.statements.length === 1} onSelectionChange={onTick} />
      </div>
      <div className={view === "chart" ? "h-full" : "hidden"}>
        <ResultChart key={key} connectionId={connectionId} sql={statement.sql} result={result.result} />
      </div>
    </>
  );
}
