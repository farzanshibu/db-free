// SOT: export-panel, multi-table-export, export-progress, export-stop, export-preview-pane
import { useEffect, useMemo, useState } from "react";
import type { ExportLayout, ExportOptions, ExportPreview, ExportProgress, ExportReport, TableRef } from "@/lib/bindings";
import { bundleExtension, DEFAULT_EXPORT_OPTIONS, EXPORT_FORMAT_FACTS } from "@/lib/export";
import { ipc, ipcProgress, normalizeError } from "@/lib/ipc";
import { pickDirectory, pickSaveFile } from "@/lib/native";
import { formatCount } from "@/lib/format";
import { tableKey, useWorkspace } from "@/stores/workspace";
import { Check, Segmented } from "@/components/global/Field";
import { EmptyState } from "@/components/global/EmptyState";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { ExportOptionsForm } from "./ExportOptionsForm";
import { formatElapsed, formatSize } from "./sizes";

// WHAT:  Export tab: tick tables, choose the format and its options, read the
//        first rows of what will be written, then write one file (a ZIP of
//        per-table files, one combined JSON, one combined SQL script) or a folder
//        of files.
// WHY:   Moving a database around is the commonest reason to export; one
//        combined file is easier to send and to re-import than forty.
// HOW:   The preview is the Rust writer's own output (`preview_export`) for the
//        first few ticked tables, so it shows the real file. A run reports
//        progress on a channel and can be stopped; the file is written under a
//        temporary name and moved into place only when it is complete.
// WHERE: src-tauri/src/services/export.rs, src-tauri/src/commands/transfer.rs
const PREVIEW_TABLES = 8;
const PREVIEW_PAUSE_MS = 300;

interface Running {
  runId: string;
  progress: ExportProgress | null;
}

export function ExportPanel({ connectionId }: { connectionId: string }) {
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const readable = useWorkspace((s) => s.sessionInfos[connectionId]?.capabilities.sql ?? true);
  const [options, setOptions] = useState<ExportOptions>(DEFAULT_EXPORT_OPTIONS);
  const [layout, setLayout] = useState<ExportLayout>("bundle");
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [filter, setFilter] = useState("");
  const [preview, setPreview] = useState<{ key: string; value: ExportPreview | null; problem: string | null }>({ key: "", value: null, problem: null });
  const [running, setRunning] = useState<Running | null>(null);
  const [report, setReport] = useState<ExportReport | null>(null);

  const tables = useMemo<TableRef[]>(
    () => (catalog?.schemas ?? []).flatMap((s) => s.tables.filter((t) => t.kind === "table").map((t) => ({ schema: t.schema, name: t.name }))),
    [catalog],
  );
  const needle = filter.trim().toLowerCase();
  const visible = useMemo(() => (needle === "" ? tables : tables.filter((t) => tableKey(t).toLowerCase().includes(needle))), [tables, needle]);
  const chosen = useMemo(() => tables.filter((t) => selected.has(tableKey(t))), [tables, selected]);
  // The SQL format only makes sense where INSERT does.
  const formats = readable ? undefined : (["csv", "tsv", "json", "ndjson", "markdown"] as const);

  // The preview follows the selection and the options, after a pause. It is
  // kept with the inputs it was made from, so "loading" is only that the inputs
  // now wanted have not been answered yet.
  const previewKey = useMemo(() => JSON.stringify([chosen.slice(0, PREVIEW_TABLES), options]), [chosen, options]);
  const previewLoading = chosen.length > 0 && preview.key !== previewKey;
  useEffect(() => {
    if (chosen.length === 0) return;
    const token = { stale: false };
    const id = window.setTimeout(() => {
      void (async () => {
        try {
          const value = await ipc("preview_export", { connectionId, tables: chosen.slice(0, PREVIEW_TABLES), options, rowsPerTable: 50 });
          if (!token.stale) setPreview({ key: previewKey, value, problem: null });
        } catch (raw) {
          if (!token.stale) setPreview({ key: previewKey, value: null, problem: normalizeError(raw).message });
        }
      })();
    }, PREVIEW_PAUSE_MS);
    return () => {
      token.stale = true;
      window.clearTimeout(id);
    };
  }, [chosen, options, connectionId, previewKey]);

  const toggle = (key: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const start = async () => {
    if (chosen.length === 0 || running !== null) return;
    const stem = chosen.length === 1 ? tableKey(chosen[0] ?? { schema: null, name: "export" }) : `export-${new Date().toISOString().slice(0, 10)}`;
    const path = layout === "bundle" ? await pickSaveFile(`${stem}.${bundleExtension(options, chosen.length)}`) : await pickDirectory();
    if (path === null) return;
    const runId = crypto.randomUUID();
    setRunning({ runId, progress: null });
    setReport(null);
    try {
      const result = await ipcProgress("export_tables", { connectionId, tables: chosen, options, layout, path, maxRows: null, runId }, (progress) => {
        setRunning((current) => (current?.runId === runId ? { runId, progress } : current));
      });
      setReport(result);
      showInfo(`Exported ${formatCount(result.files.length)} table(s), ${formatCount(result.files.reduce((n, f) => n + f.rows, 0))} rows.`);
    } catch (raw) {
      const error = normalizeError(raw);
      if (error.kind !== "cancelled") showError(error);
      else showInfo("Export stopped. No file was written.");
    } finally {
      setRunning(null);
    }
  };

  const stop = () => {
    if (running !== null) void ipc("cancel_query", { runId: running.runId });
  };

  const progress = running?.progress ?? null;
  const percent = progress !== null && progress.tablesTotal > 0 ? Math.min(100, Math.round((progress.tablesDone / progress.tablesTotal) * 100)) : 0;
  const rowsWritten = report?.files.reduce((n, f) => n + f.rows, 0) ?? 0;

  return (
    <div className="flex min-h-0 flex-1">
      <div className="flex w-60 shrink-0 flex-col border-r border-border/40">
        <div className="flex items-center gap-2 px-2.5 py-1.5">
          <Badge size="sm" variant="soft" className="font-mono text-[10px]">
            {selected.size} of {tables.length}
          </Badge>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto px-1.5 text-xs text-accent"
            onClick={() => {
              setSelected(selected.size === tables.length ? new Set() : new Set(tables.map(tableKey)));
            }}
          >
            {selected.size === tables.length && tables.length > 0 ? "Clear" : "Select all"}
          </Button>
        </div>
        <div className="px-2.5 pb-1.5">
          <SearchInput value={filter} onChange={setFilter} aria-label="Filter tables" placeholder="Filter tables…" className="h-6 text-xs" />
        </div>
        <ScrollArea className="min-h-0 flex-1">
          {visible.length === 0 ? (
            <p className="px-2.5 py-2 text-xs text-muted">{tables.length === 0 ? "No tables in this database." : "No table matches."}</p>
          ) : (
            visible.map((t) => {
              const key = tableKey(t);
              return (
                <label key={key} className={cn("flex h-6 cursor-pointer items-center gap-2 px-2.5 text-xs", selected.has(key) ? "bg-surface-tertiary text-foreground" : "text-muted hover:bg-surface-secondary hover:text-foreground")}>
                  <Check label={`Select ${key}`} checked={selected.has(key)} onChange={() => { toggle(key); }} />
                  <Icon name="table" size={13} className="shrink-0" />
                  <span className="truncate">{key}</span>
                </label>
              );
            })
          )}
        </ScrollArea>
      </div>

      <ScrollArea className="w-64 shrink-0 border-r border-border/40 p-2.5">
        <ExportOptionsForm options={options} onChange={setOptions} structure disabled={running !== null} {...(formats === undefined ? {} : { formats })} />
        <div className="mt-3 flex flex-col gap-1.5">
          <span className="text-[11px] font-medium text-muted">Output</span>
          <Segmented<ExportLayout>
            label="Output layout"
            value={layout}
            onChange={setLayout}
            options={[
              { value: "bundle", label: "One file" },
              { value: "folder", label: "Folder of files" },
            ]}
          />
          <p className="text-xs text-muted">
            {layout === "folder"
              ? "One file per table in the folder you choose."
              : chosen.length === 0
                ? "Choose tables to see the file this writes."
                : `Writes one .${bundleExtension(options, chosen.length)} file${bundleExtension(options, chosen.length) === "zip" ? ` holding ${chosen.length} ${EXPORT_FORMAT_FACTS[options.format].label} files` : ""}.`}
          </p>
        </div>
      </ScrollArea>

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-7 shrink-0 items-center gap-2 border-b border-border/40 px-2.5">
          {running === null ? (
            <Button size="sm" onClick={() => void start()} disabled={chosen.length === 0}>
              <Icon name="download" size={13} />
              Export{chosen.length > 0 ? ` ${chosen.length} table${chosen.length === 1 ? "" : "s"}` : ""}…
            </Button>
          ) : (
            <>
              <Button size="sm" variant="danger-soft" onClick={stop}>
                <Icon name="stop" size={13} />
                Stop
              </Button>
              <div className="flex min-w-0 flex-1 items-center gap-2 text-xs text-muted">
                <div className="h-1.5 w-40 shrink-0 overflow-hidden rounded-full bg-surface-tertiary">
                  <div className="h-full bg-accent transition-[width]" style={{ width: `${percent}%` }} />
                </div>
                <span className="truncate">
                  {progress === null ? "Starting…" : progress.stage === "finishing" ? "Finishing…" : `${progress.table ?? ""} · table ${progress.tablesDone + 1} of ${progress.tablesTotal} · ${formatCount(progress.rows)} rows`}
                </span>
              </div>
            </>
          )}
          {report !== null && running === null ? (
            <span className="truncate text-xs text-muted">
              Wrote {formatCount(report.files.length)} table{report.files.length === 1 ? "" : "s"}, {formatCount(rowsWritten)} rows, {formatSize(report.bytes)} in {formatElapsed(report.elapsedMs)}
            </span>
          ) : null}
        </div>
        {report !== null && report.warnings.length > 0 && running === null ? (
          <div className="border-b border-border/40 bg-warning/10 px-2.5 py-1 text-xs text-warning">
            {report.warnings.map((w) => (
              <p key={w}>{w}</p>
            ))}
          </div>
        ) : null}
        <div className="flex h-7 shrink-0 items-center gap-2 px-2.5 text-xs text-muted">
          <span>Preview</span>
          {previewLoading ? <Spinner size="sm" /> : null}
          {preview.value !== null ? (
            <span>
              first 50 rows of {Math.min(chosen.length, PREVIEW_TABLES)} table{Math.min(chosen.length, PREVIEW_TABLES) === 1 ? "" : "s"}
              {chosen.length > PREVIEW_TABLES ? ` (of ${chosen.length})` : ""}
              {preview.value.truncated ? " · cut at 1 MiB" : ""}
            </span>
          ) : null}
        </div>
        <ScrollArea orientation="both" className="min-h-0 flex-1 px-2.5 pb-2">
          {chosen.length === 0 ? (
            <EmptyState icon="download" title="Choose tables to export" body="Tick tables on the left. The preview shows the file as it will be written." />
          ) : preview.problem !== null ? (
            <p className="text-xs text-danger">{preview.problem}</p>
          ) : preview.value === null ? (
            <div className="flex h-full items-center justify-center">
              <Spinner size="sm" />
            </div>
          ) : (
            <>
              {preview.value.entries.length > 0 ? (
                <p className="mb-2 text-xs text-muted">ZIP entries: {preview.value.entries.slice(0, 12).join(", ")}{preview.value.entries.length > 12 ? `, … (${preview.value.entries.length} in all)` : ""}</p>
              ) : null}
              <pre className="selectable min-w-max rounded-lg border border-border bg-background p-2.5 font-mono text-[11px] leading-relaxed text-foreground/90">{preview.value.text}</pre>
            </>
          )}
        </ScrollArea>
      </div>
    </div>
  );
}
