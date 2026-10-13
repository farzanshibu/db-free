// SOT: sql-script-import, sql-file-scan-summary, destructive-script-confirmation, sql-import-progress
import { useEffect, useState } from "react";
import type { SqlFileInfo, SqlImportProgress, SqlImportReport } from "@/lib/bindings";
import { ipc, ipcProgress, normalizeError } from "@/lib/ipc";
import { formatCount } from "@/lib/format";
import { useWorkspace } from "@/stores/workspace";
import { Toggle } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { baseName, formatElapsed, formatSize } from "./sizes";

// WHAT:  Runs a .sql script (a dump, a migration) against the connection, a
//        statement at a time, with progress, Stop, and a report of exactly how
//        far it got.
// WHY:   Opening a 400 MB dump in the editor is not an option, and "it stopped
//        somewhere" is not an answer. The splitter understands the dump's own
//        dialect (quotes, comments, `$$` bodies, MySQL's DELIMITER, SQL Server's
//        GO), every statement passes the read-only lock and the history log, and
//        a script that drops or empties things needs the user to say so first.
// HOW:   `inspect_sql_file` scans the file with the same splitter and the guard's
//        own classifier, so the counts are the run's; the destructive switch is
//        the confirmation the run then passes along. A failure stops the run:
//        there is no wrapping transaction, so everything before it stays.
// WHERE: src-tauri/src/services/sql_import.rs, src-tauri/src/commands/transfer.rs
interface SqlImportProps {
  connectionId: string;
  path: string;
  onReset: () => void;
}

export function SqlImport({ connectionId, path, onReset }: SqlImportProps) {
  const readOnly = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.readOnly ?? false);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [info, setInfo] = useState<{ value: SqlFileInfo | null; problem: string | null }>({ value: null, problem: null });
  const [confirmed, setConfirmed] = useState(false);
  const [running, setRunning] = useState<{ runId: string; progress: SqlImportProgress | null } | null>(null);
  const [report, setReport] = useState<SqlImportReport | null>(null);

  // The tab is keyed by the file's path, so a new file starts from fresh state.
  useEffect(() => {
    const token = { stale: false };
    void (async () => {
      try {
        const value = await ipc("inspect_sql_file", { connectionId, path });
        if (!token.stale) setInfo({ value, problem: null });
      } catch (raw) {
        if (!token.stale) setInfo({ value: null, problem: normalizeError(raw).message });
      }
    })();
    return () => {
      token.stale = true;
    };
  }, [connectionId, path]);

  const value = info.value;
  const needsConfirmation = value !== null && value.destructive > 0;
  const ready = value !== null && value.problem === null && value.statements > 0 && (!needsConfirmation || confirmed) && !readOnly && running === null;

  const run = async () => {
    if (value === null) return;
    const runId = crypto.randomUUID();
    setRunning({ runId, progress: null });
    setReport(null);
    try {
      const result = await ipcProgress(
        "import_sql_file",
        { connectionId, path, confirmDestructive: confirmed, expectedSize: value.size, expectedModifiedMs: value.modifiedMs, runId },
        (progress) => {
          setRunning((current) => (current?.runId === runId ? { runId, progress } : current));
        },
      );
      setReport(result);
      if (result.error === null) showInfo(`Ran ${formatCount(result.statements)} statements.`);
      window.dispatchEvent(new CustomEvent("db-free:refresh-tables"));
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setRunning(null);
    }
  };

  const progress = running?.progress ?? null;
  const percent = progress !== null && progress.bytesTotal > 0 ? Math.min(100, Math.round((progress.bytesRead / progress.bytesTotal) * 100)) : 0;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-7 shrink-0 items-center gap-2 border-b border-border/40 px-2.5">
        <Icon name="file" size={14} className="shrink-0 text-muted" />
        <span className="min-w-0 truncate text-xs text-foreground" title={path}>
          {baseName(path)}
        </span>
        <Button size="xs" variant="ghost" onClick={onReset} disabled={running !== null}>
          Choose another file
        </Button>
        <div className="ml-auto flex items-center gap-2">
          {running !== null ? (
            <>
              <div className="h-1.5 w-40 overflow-hidden rounded-full bg-surface-tertiary">
                <div className="h-full bg-accent transition-[width]" style={{ width: `${percent}%` }} />
              </div>
              <span className="text-xs text-muted">{progress === null ? "Starting…" : `${formatCount(progress.statements)} statements · ${percent}%`}</span>
              <Button
                size="sm"
                variant="danger-soft"
                onClick={() => {
                  void ipc("cancel_query", { runId: running.runId });
                }}
              >
                <Icon name="stop" size={13} />
                Stop
              </Button>
            </>
          ) : (
            <Button size="sm" disabled={!ready} onClick={() => void run()}>
              <Icon name="play" size={13} />
              Run script
            </Button>
          )}
        </div>
      </div>

      <ScrollArea className="min-h-0 flex-1 p-2.5">
        <div className="flex max-w-3xl flex-col gap-3">
          {readOnly ? <p className="rounded-lg bg-warning/10 p-2 text-xs text-warning">This connection is read-only, so a script cannot run.</p> : null}
          {info.problem !== null ? <p className="rounded-lg bg-danger/10 p-2.5 text-xs text-danger">{info.problem}</p> : null}
          {value === null && info.problem === null ? (
            <div className="flex items-center gap-2 text-xs text-muted">
              <Spinner size="sm" />
              Reading the script…
            </div>
          ) : null}
          {value !== null ? (
            <>
              <section className="grid grid-cols-3 gap-2">
                <Fact label="Size" value={formatSize(value.size)} />
                <Fact label="Statements" value={formatCount(value.statements)} />
                <Fact label="Drop / empty" value={formatCount(value.destructive)} warn={value.destructive > 0} />
              </section>
              {value.problem !== null ? <p className="rounded-lg bg-danger/10 p-2.5 text-xs text-danger">{value.problem}</p> : null}
              {needsConfirmation ? (
                <section className="flex flex-col gap-2 rounded-lg border border-warning/40 bg-warning/10 p-2.5">
                  <p className="text-xs font-medium text-warning">{formatCount(value.destructive)} statement{value.destructive === 1 ? "" : "s"} remove or overwrite data</p>
                  <ul className="list-disc space-y-0.5 pl-3 font-mono text-[11px] text-foreground/90">
                    {value.destructiveExamples.map((e, i) => (
                      <li key={i}>{e}</li>
                    ))}
                    {value.destructive > value.destructiveExamples.length ? <li className="list-none text-muted">… and {formatCount(value.destructive - value.destructiveExamples.length)} more</li> : null}
                  </ul>
                  <Toggle checked={confirmed} onChange={setConfirmed} label="Run them" description="Without this, the first such statement stops the script." />
                </section>
              ) : null}
              <section className="flex flex-col gap-1.5">
                <h3 className="text-xs font-semibold text-foreground">Start of the file</h3>
                <div className="overflow-hidden rounded-lg border border-border bg-background">
                  <ScrollArea orientation="both" className="max-h-56">
                    <pre className="selectable p-2.5 font-mono text-[11px] leading-relaxed text-foreground/90">{value.head}</pre>
                  </ScrollArea>
                </div>
              </section>
            </>
          ) : null}
          {report !== null ? <ReportCard report={report} /> : null}
        </div>
      </ScrollArea>
    </div>
  );
}

function Fact({ label, value, warn = false }: { label: string; value: string; warn?: boolean }) {
  return (
    <div className="rounded-lg border border-border bg-surface px-2.5 py-1.5">
      <p className="text-[11px] uppercase tracking-wide text-muted">{label}</p>
      <p className={cn("text-[13px] font-semibold tabular-nums", warn ? "text-warning" : "text-foreground")}>{value}</p>
    </div>
  );
}

function ReportCard({ report }: { report: SqlImportReport }) {
  const failed = report.error !== null;
  return (
    <section className={cn("flex flex-col gap-1 rounded-lg border p-2.5 text-xs", failed ? "border-warning/40 bg-warning/10" : "border-success/40 bg-success/10")}>
      <p className={cn("font-medium", failed ? "text-warning" : "text-success")}>
        {failed ? `Stopped after ${formatCount(report.statements)} statements` : `Ran ${formatCount(report.statements)} statements`} in {formatElapsed(report.elapsedMs)}
      </p>
      {failed ? <p className="text-foreground">{report.error}</p> : null}
      {report.failedLine !== null ? <p className="text-muted">The failing statement starts on line {formatCount(report.failedLine)}:</p> : null}
      {report.failedStatement !== null ? <div className="overflow-hidden rounded border border-border bg-background">
          <ScrollArea orientation="both">
            <pre className="selectable p-2 font-mono text-[11px] text-foreground/90">{report.failedStatement}</pre>
          </ScrollArea>
        </div> : null}
      {failed ? <p className="text-muted">Statements run one by one with no wrapping transaction: the {formatCount(report.statements)} before this point are committed.</p> : null}
    </section>
  );
}
