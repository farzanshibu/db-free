// SOT: table-import, csv-import-options, import-column-mapping, import-truncate-first, import-conflict-mode, import-preview-grid, import-progress
import { useEffect, useMemo, useState } from "react";
import type { CsvImportOptions, ImportConflict, ImportMapping, ImportPreview, ImportProgress, ImportReport, TableRef } from "@/lib/bindings";
import { ipc, ipcProgress, normalizeError } from "@/lib/ipc";
import { formatCount } from "@/lib/format";
import { tableKey, useWorkspace } from "@/stores/workspace";
import { AppSelect, Field, Segmented, Toggle } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { baseName, formatElapsed, isTsv } from "./sizes";

// WHAT:  Imports one CSV / TSV / JSON file into one table: read the file with the
//        delimiter and quote the user names, see its first rows, map its columns
//        onto the table's (or Skip them), empty the table first, choose what to do
//        with rows that already exist, and run it with a progress bar.
// WHY:   Files rarely arrive in the shape of the table. Export from a spreadsheet
//        uses `;`, the columns are in another order, an ID column is not wanted.
// HOW:   `preview_import` reads the file and the table's columns and suggests a
//        mapping by name; `import_file` writes in chunks, each its own
//        transaction (a failure part-way says how far it got). Truncate-first is
//        confirmed in a dialog before anything is sent.
// WHERE: src-tauri/src/commands/transfer.rs (preview_import, import_file)
interface TableImportProps {
  connectionId: string;
  path: string;
  format: "csv" | "json";
  onReset: () => void;
}

const DELIMITERS = [
  { value: ",", label: "Comma  ," },
  { value: ";", label: "Semicolon  ;" },
  { value: "\t", label: "Tab" },
  { value: "|", label: "Pipe  |" },
] as const;

const QUOTES = [
  { value: '"', label: 'Double  "' },
  { value: "'", label: "Single  '" },
] as const;

const SKIP = "skip";

function cellLabel(header: string, index: number): string {
  return header.trim() === "" ? `Column ${index + 1} (unnamed)` : header;
}

export function TableImport({ connectionId, path, format, onReset }: TableImportProps) {
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const readOnly = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.readOnly ?? false);
  const speaksSql = useWorkspace((s) => s.sessionInfos[connectionId]?.capabilities.sql ?? true);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const [target, setTarget] = useState("");
  const [csv, setCsv] = useState<CsvImportOptions>({ delimiter: isTsv(path) ? "\t" : ",", quote: '"', header: true, nullText: "" });
  const [preview, setPreview] = useState<{ key: string; value: ImportPreview | null; problem: string | null }>({ key: "", value: null, problem: null });
  const [mapping, setMapping] = useState<readonly ImportMapping[]>([]);
  const [truncate, setTruncate] = useState(false);
  const [conflict, setConflict] = useState<ImportConflict>("error");
  const [running, setRunning] = useState<{ runId: string; progress: ImportProgress | null } | null>(null);
  const [report, setReport] = useState<ImportReport | null>(null);
  const [confirming, setConfirming] = useState(false);

  const tables = useMemo<TableRef[]>(() => (catalog?.schemas ?? []).flatMap((s) => s.tables.filter((t) => t.kind === "table").map((t) => ({ schema: t.schema, name: t.name }))), [catalog]);
  const byKey = useMemo(() => new Map(tables.map((t) => [tableKey(t), t])), [tables]);
  const targetRef = byKey.get(target) ?? null;

  // The file is read again whenever the options or the target change. The
  // result is kept with the inputs it answers, so "loading" is only that the
  // inputs now wanted have no answer yet.
  const previewKey = useMemo(() => JSON.stringify([target, path, format, csv]), [target, path, format, csv]);
  const previewLoading = preview.key !== previewKey;
  useEffect(() => {
    const token = { stale: false };
    const id = window.setTimeout(() => {
      void (async () => {
        try {
          const value = await ipc("preview_import", { connectionId, table: targetRef, path, format, csv });
          if (token.stale) return;
          setPreview({ key: previewKey, value, problem: null });
          setMapping(value.mapping);
        } catch (raw) {
          if (!token.stale) setPreview({ key: previewKey, value: null, problem: normalizeError(raw).message });
        }
      })();
    }, 200);
    return () => {
      token.stale = true;
      window.clearTimeout(id);
    };
  }, [connectionId, targetRef, path, format, csv, previewKey]);

  const value = preview.value;
  const support = value?.conflict ?? { skip: false, update: false };
  const hasKey = value?.targetColumns.some((c) => c.primaryKey) ?? false;
  const mapped = mapping.filter((m) => m.source !== null).length;
  const ready = targetRef !== null && value !== null && mapped > 0 && !readOnly && running === null;

  const setSource = (column: string, source: number | null) => {
    setMapping((current) => current.map((m) => (m.target === column ? { ...m, source } : m)));
  };

  const run = async (confirmed: boolean) => {
    if (targetRef === null || value === null) return;
    const runId = crypto.randomUUID();
    setRunning({ runId, progress: null });
    setReport(null);
    try {
      const result = await ipcProgress(
        "import_file",
        { connectionId, table: targetRef, path, format, csv, mapping: [...mapping], truncateFirst: truncate, onConflict: conflict, confirmDestructive: confirmed, rowsTotal: value.rowCount, runId },
        (progress) => {
          setRunning((current) => (current?.runId === runId ? { runId, progress } : current));
        },
      );
      setReport(result);
      if (result.error === null) showInfo(`Imported ${formatCount(result.rowsInserted)} rows into ${tableKey(targetRef)}.`);
      window.dispatchEvent(new CustomEvent("db-free:refresh-tables"));
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setRunning(null);
    }
  };

  const progress = running?.progress ?? null;
  const percent = progress !== null && progress.rowsTotal > 0 ? Math.min(100, Math.round((progress.rowsDone / progress.rowsTotal) * 100)) : 0;
  const importButton = (
    <Button size="sm" disabled={!ready} className="shrink-0">
      <Icon name="download" size={13} className="rotate-180" />
      Import{value !== null ? ` ${formatCount(value.rowCount)} rows` : ""}
    </Button>
  );

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
              <span className="text-xs text-muted">{progress === null ? "Starting…" : `${formatCount(progress.rowsDone)} of ${formatCount(progress.rowsTotal)} rows`}</span>
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
          ) : truncate ? (
            <Dialog open={confirming} onOpenChange={setConfirming}>
              <DialogTrigger asChild>{importButton}</DialogTrigger>
              <DialogContent className="sm:max-w-[460px]">
                <DialogHeader>
                  <DialogTitle>Empty {target} first?</DialogTitle>
                  <DialogDescription>Every row in the table is deleted, in the same transaction as the first rows of the file.</DialogDescription>
                </DialogHeader>
                <DialogBody>
                  <p className="text-xs text-foreground">
                    <span className="font-mono">{target}</span> will be emptied, then {value !== null ? formatCount(value.rowCount) : "the file's"} rows are imported. A large file is imported in several transactions, so a failure after the first leaves the table partly filled.
                  </p>
                </DialogBody>
                <DialogFooter>
                  <Button
                    variant="secondary"
                    onClick={() => {
                      setConfirming(false);
                    }}
                  >
                    Cancel
                  </Button>
                  <Button
                    variant="danger"
                    onClick={() => {
                      setConfirming(false);
                      void run(true);
                    }}
                  >
                    Empty the table and import
                  </Button>
                </DialogFooter>
              </DialogContent>
            </Dialog>
          ) : (
            <Button size="sm" disabled={!ready} onClick={() => void run(false)}>
              <Icon name="download" size={13} className="rotate-180" />
              Import{value !== null ? ` ${formatCount(value.rowCount)} rows` : ""}
            </Button>
          )}
        </div>
      </div>

      <div className="flex min-h-0 flex-1">
        <ScrollArea className="w-[270px] shrink-0 border-r border-border/40 p-2.5">
          <div className="flex flex-col gap-2">
            {!speaksSql ? <p className="rounded-lg bg-warning/10 p-2 text-xs text-warning">This engine has no INSERT statement, so it cannot import a file.</p> : null}
            {readOnly ? <p className="rounded-lg bg-warning/10 p-2 text-xs text-warning">This connection is read-only, so it cannot import.</p> : null}
            <AppSelect label="Import into" value={target} options={[{ value: "", label: "Choose a table…" }, ...tables.map((t) => ({ value: tableKey(t), label: tableKey(t) }))]} onChange={setTarget} />
            {format === "csv" ? (
              <>
                <AppSelect label="Delimiter" value={csv.delimiter} options={DELIMITERS} onChange={(delimiter) => { setCsv({ ...csv, delimiter }); }} />
                <AppSelect label="Quote character" value={csv.quote} options={QUOTES} onChange={(quote) => { setCsv({ ...csv, quote }); }} />
                <Toggle checked={csv.header} onChange={(header) => { setCsv({ ...csv, header }); }} label="First row is a header" />
                <Field label="Treat as NULL" value={csv.nullText} onChange={(nullText) => { setCsv({ ...csv, nullText }); }} placeholder="(empty cells)" mono description="Cells equal to this text become NULL. Empty cells always do when this is blank." />
              </>
            ) : null}
            <div className="flex flex-col gap-1.5">
              <span className="text-[11px] font-medium text-muted">If a row already exists</span>
              <Segmented<ImportConflict>
                label="Conflict handling"
                value={conflict}
                onChange={setConflict}
                options={[
                  { value: "error", label: "Fail" },
                  { value: "skip", label: "Skip", disabled: !support.skip },
                  { value: "update", label: "Update", disabled: !support.update || !hasKey },
                ]}
              />
              <p className="text-xs text-muted">
                {!support.skip ? "This engine cannot skip or update existing rows; a duplicate fails the import." : conflict === "update" && !hasKey ? "Update needs a primary key." : "Rows are matched by the table's primary key."}
              </p>
            </div>
            <Toggle checked={truncate} onChange={setTruncate} label="Empty the table first" description="Deletes every existing row, then imports. You are asked to confirm." />
          </div>
        </ScrollArea>

        <ScrollArea className="min-w-0 flex-1 p-2.5">
          {previewLoading && value === null && preview.problem === null ? (
            <div className="flex h-40 items-center justify-center">
              <Spinner size="sm" />
            </div>
          ) : preview.problem !== null ? (
            <p className="rounded-lg bg-danger/10 p-2.5 text-xs text-danger">{preview.problem}</p>
          ) : value !== null ? (
            <div className="flex flex-col gap-3">
              <section className="flex flex-col gap-1.5">
                <div className="flex items-center gap-2">
                  <h3 className="text-xs font-semibold text-foreground">Preview</h3>
                  <span className="text-xs text-muted">
                    {formatCount(value.rowCount)} rows · {value.headers.length} columns · first {Math.min(50, value.rows.length)} shown
                  </span>
                  {previewLoading ? <Spinner size="sm" /> : null}
                </div>
                <div className="max-h-64 overflow-auto rounded-lg border border-border">
                  <table className="w-max min-w-full border-collapse text-xs">
                    <thead className="sticky top-0 bg-surface text-left text-muted">
                      <tr>
                        <th className="w-10 px-2 py-1 font-normal">#</th>
                        {value.headers.map((h, i) => (
                          <th key={i} className="whitespace-nowrap px-2 py-1 font-medium">
                            {cellLabel(h, i)}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {value.rows.map((row, r) => (
                        <tr key={r} className="border-t border-border/40">
                          <td className="px-2 py-0.5 text-muted">{r + 1}</td>
                          {value.headers.map((_, c) => {
                            const cell = row[c] ?? null;
                            return (
                              <td key={c} className={cn("max-w-64 truncate px-2 py-0.5 font-mono", cell === null ? "italic text-muted" : "text-foreground")} title={cell ?? "NULL"}>
                                {cell ?? "NULL"}
                              </td>
                            );
                          })}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </section>

              <section className="flex flex-col gap-1.5">
                <h3 className="text-xs font-semibold text-foreground">Column mapping</h3>
                {targetRef === null ? (
                  <p className="text-xs text-muted">Choose the table to import into; its columns are matched to the file's by name.</p>
                ) : (
                  <div className="overflow-hidden rounded-lg border border-border">
                    <div className="grid grid-cols-[minmax(0,1fr)_1.25rem_minmax(0,1fr)] items-center gap-2 border-b border-border bg-surface px-2.5 py-1 text-[11px] uppercase tracking-wide text-muted">
                      <span>Table column</span>
                      <span />
                      <span>File column</span>
                    </div>
                    {value.targetColumns.map((column) => {
                      const current = mapping.find((m) => m.target === column.name)?.source ?? null;
                      return (
                        <div key={column.name} className="grid grid-cols-[minmax(0,1fr)_1.25rem_minmax(0,1fr)] items-center gap-2 border-b border-border/40 px-2.5 py-1 last:border-b-0">
                          <span className="flex min-w-0 items-center gap-1.5 text-xs">
                            {column.primaryKey ? <Icon name="key" size={11} className="shrink-0 text-warning" /> : null}
                            <span className="truncate font-mono text-foreground">{column.name}</span>
                            <span className="shrink-0 font-mono text-[10px] text-muted">{column.dataType}</span>
                          </span>
                          <Icon name="chevron-left" size={12} className="text-muted" />
                          <AppSelect
                            ariaLabel={`File column for ${column.name}`}
                            size="sm"
                            value={current === null ? SKIP : String(current)}
                            options={[{ value: SKIP, label: "Skip" }, ...value.headers.map((h, i) => ({ value: String(i), label: cellLabel(h, i) }))]}
                            onChange={(next) => {
                              setSource(column.name, next === SKIP ? null : Number(next));
                            }}
                          />
                        </div>
                      );
                    })}
                  </div>
                )}
              </section>

              {report !== null ? <ReportCard report={report} /> : null}
            </div>
          ) : null}
        </ScrollArea>
      </div>
    </div>
  );
}

function ReportCard({ report }: { report: ImportReport }) {
  const failed = report.error !== null;
  return (
    <section className={cn("flex flex-col gap-1 rounded-lg border p-2.5 text-xs", failed ? "border-warning/40 bg-warning/10" : "border-success/40 bg-success/10")}>
      <p className={cn("font-medium", failed ? "text-warning" : "text-success")}>
        {failed ? `Stopped after ${formatCount(report.rowsInserted)} of ${formatCount(report.rowsTotal)} rows` : `Imported ${formatCount(report.rowsInserted)} rows`} in {formatElapsed(report.elapsedMs)}
      </p>
      {failed ? <p className="text-foreground">{report.error}</p> : null}
      {failed && report.rowsInserted > 0 ? <p className="text-muted">The {formatCount(report.rowsInserted)} rows before the failure were committed.</p> : null}
      {report.rowsAffected !== null && report.rowsAffected !== report.rowsInserted ? <p className="text-muted">The server reports {formatCount(report.rowsAffected)} rows changed (updates and skipped rows count differently).</p> : null}
    </section>
  );
}
