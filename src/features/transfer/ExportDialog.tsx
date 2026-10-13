// SOT: export-dialog, row-export-dialog, export-preview, copy-or-save-rows, export-scope, column-picker
import { useEffect, useMemo, useState } from "react";
import type { ExportOptions, TableRef, Value } from "@/lib/bindings";
import { downloadTextFile, DEFAULT_EXPORT_OPTIONS, EXPORT_FORMAT_FACTS, EXPORT_FORMATS, exportFilename, selectColumns, serializeRows, type ExportColumn } from "@/lib/export";
import { ipc, normalizeError } from "@/lib/ipc";
import { formatCount } from "@/lib/format";
import { useWorkspace } from "@/stores/workspace";
import { AppSelect, Check, Field } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Icon } from "@/lib/icons";
import { ExportOptionsForm } from "./ExportOptionsForm";

// WHAT:  The export dialog for rows already on screen: pick the format, the
//        columns and the options, see the first 20 rows exactly as they will be
//        written, then copy them or save a file.
// WHY:   "Export" used to mean two fixed formats and a menu. The same rows are
//        wanted as TSV for a spreadsheet, NDJSON for a pipeline, Markdown for a
//        ticket or INSERTs for another database, with the NULLs, the header and
//        the delimiter the destination expects.
// HOW:   The table browser and the result grid each pass their scopes (the
//        selection, the page, everything the query returned, every row of a
//        filtered table) with a loader for each; the dialog serializes with the
//        shared writer in src/lib/export.ts. SQL INSERTs are rendered by the
//        connection's own dialect (`rows_as_sql`), so identifiers and literals
//        are quoted for that engine.
// WHERE: src/features/grid/TableTab.tsx, src/features/editor/ResultsPane.tsx
const PREVIEW_ROWS = 20;

type Rows = readonly (readonly Value[])[];

export interface ExportScope {
  id: string;
  label: string;
  /// How many rows the scope holds, when known (shown beside its name).
  count: number | null;
  /// The first rows, for the preview. Always in hand.
  preview: Rows;
  /// Every row of the scope; may fetch.
  load: () => Promise<Rows>;
}

interface ExportDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  connectionId: string;
  /// Shown in the title ("users", "Result 2").
  title: string;
  /// File name stem and the JSON envelope's table name.
  baseName: string;
  /// The table the rows come from; null for a query result (SQL asks for a name).
  table: TableRef | null;
  columns: readonly ExportColumn[];
  scopes: readonly ExportScope[];
}

export function ExportDialog(props: ExportDialogProps) {
  // The body mounts only while open, so its state starts fresh on every opening.
  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange}>
      {props.open ? <ExportDialogContent {...props} /> : null}
    </Dialog>
  );
}

function ExportDialogContent({ onOpenChange, connectionId, title, baseName, table, columns: openedColumns, scopes: openedScopes }: ExportDialogProps) {
  // What the dialog was opened with is what it exports: the table behind it may
  // refresh while the user reads the preview, and the rows must not shift under them.
  const [columns] = useState(openedColumns);
  const [scopes] = useState(openedScopes);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  const speaksSql = useWorkspace((s) => s.sessionInfos[connectionId]?.capabilities.sql ?? false);
  const [options, setOptions] = useState<ExportOptions>(DEFAULT_EXPORT_OPTIONS);
  const [kept, setKept] = useState<ReadonlySet<string>>(() => new Set(columns.map((c) => c.name)));
  const [scopeId, setScopeId] = useState(scopes[0]?.id ?? "");
  const [sqlTable, setSqlTable] = useState(table?.name ?? "exported_rows");
  const [preview, setPreview] = useState<{ key: string; text: string; problem: string | null }>({ key: "", text: "", problem: null });
  const [busy, setBusy] = useState<"copy" | "save" | null>(null);

  const scope = scopes.find((s) => s.id === scopeId) ?? scopes[0];
  const formats = speaksSql ? EXPORT_FORMATS : EXPORT_FORMATS.filter((f) => f !== "sql");
  const chosen = useMemo(() => columns.filter((c) => kept.has(c.name)), [columns, kept]);
  const schema = table?.schema ?? null;
  const target = useMemo<TableRef>(() => ({ schema, name: sqlTable.trim() === "" ? "exported_rows" : sqlTable.trim() }), [schema, sqlTable]);
  const envelopeName = table === null ? baseName : table.schema === null ? table.name : `${table.schema}.${table.name}`;

  // WHAT:  Text for `rows` in the chosen format and columns.
  const render = async (rows: Rows): Promise<string> => {
    const narrowed = selectColumns(columns, rows, kept);
    if (options.format !== "sql") return serializeRows(narrowed.columns, narrowed.rows, options, envelopeName);
    const text = await ipc("rows_as_sql", { connectionId, table: target, kind: "insert", columns: narrowed.columns.map((c) => c.name), keyColumns: [], rows: narrowed.rows });
    return options.bom ? `\uFEFF${text}` : text;
  };

  // The preview follows every change, after a short pause.
  const previewRows = useMemo(() => scope?.preview.slice(0, PREVIEW_ROWS), [scope]);
  // What the preview shows is for the inputs it was made from; "rendering" is
  // simply that the inputs now wanted have not been answered yet.
  const previewKey = useMemo(() => JSON.stringify([options, [...kept].sort(), scopeId, target, envelopeName]), [options, kept, scopeId, target, envelopeName]);
  const noColumns = chosen.length === 0;
  const rendering = !noColumns && preview.key !== previewKey;
  useEffect(() => {
    if (previewRows === undefined || noColumns) return;
    const token = { stale: false };
    const id = window.setTimeout(() => {
      void (async () => {
        try {
          const narrowed = selectColumns(columns, previewRows, kept);
          const text =
            options.format === "sql"
              ? await ipc("rows_as_sql", { connectionId, table: target, kind: "insert", columns: narrowed.columns.map((c) => c.name), keyColumns: [], rows: narrowed.rows })
              : serializeRows(narrowed.columns, narrowed.rows, options, envelopeName);
          if (!token.stale) setPreview({ key: previewKey, text: previewRows.length === 0 ? "(no rows)" : text, problem: null });
        } catch (raw) {
          if (!token.stale) setPreview({ key: previewKey, text: "", problem: normalizeError(raw).message });
        }
      })();
    }, 150);
    return () => {
      token.stale = true;
      window.clearTimeout(id);
    };
  }, [previewRows, noColumns, previewKey, kept, options, columns, connectionId, target, envelopeName]);

  const run = async (mode: "copy" | "save") => {
    if (scope === undefined || chosen.length === 0) return;
    setBusy(mode);
    try {
      const rows = await scope.load();
      const text = await render(rows);
      if (mode === "copy") {
        await navigator.clipboard.writeText(text);
        showInfo(`Copied ${formatCount(rows.length)} row(s) as ${EXPORT_FORMAT_FACTS[options.format].label}.`);
      } else {
        const facts = EXPORT_FORMAT_FACTS[options.format];
        downloadTextFile(exportFilename(baseName, options.format), text, facts.mime);
        showInfo(`Saved ${formatCount(rows.length)} row(s) as ${facts.label}.`);
      }
      onOpenChange(false);
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setBusy(null);
    }
  };

  const toggleColumn = (name: string) => {
    setKept((current) => {
      const next = new Set(current);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  return (
    <DialogContent className="sm:max-w-[800px]">
      <DialogHeader>
        <DialogTitle>Export {title}</DialogTitle>
        <DialogDescription>Preview the first {PREVIEW_ROWS} rows as they will be written, then copy them or save a file.</DialogDescription>
      </DialogHeader>
      <DialogBody className="grid min-h-0 gap-3 md:grid-cols-[15rem_minmax(0,1fr)]">
        <ScrollArea className="max-h-[60vh] pr-2">
          <div className="flex flex-col gap-2">
            {scopes.length > 1 ? <AppSelect label="Rows" value={scopeId} options={scopes.map((s) => ({ value: s.id, label: s.count === null ? s.label : `${s.label} (${formatCount(s.count)})` }))} onChange={setScopeId} /> : null}
            <ExportOptionsForm options={options} onChange={setOptions} formats={formats} />
            {options.format === "sql" && table === null ? <Field label="Table name in the INSERTs" value={sqlTable} onChange={setSqlTable} mono /> : null}
            <div className="flex flex-col gap-1.5">
              <div className="flex items-center justify-between">
                <span className="text-[11px] font-medium text-muted">
                  Columns ({chosen.length} of {columns.length})
                </span>
                <span className="flex gap-1">
                  <Button size="xs" variant="ghost" onClick={() => { setKept(new Set(columns.map((c) => c.name))); }}>
                    All
                  </Button>
                  <Button size="xs" variant="ghost" onClick={() => { setKept(new Set()); }}>
                    None
                  </Button>
                </span>
              </div>
              <div className="flex flex-col gap-0.5">
                {columns.map((c) => (
                  <label key={c.name} className="flex h-6 cursor-pointer items-center gap-2 rounded-sm px-1 text-xs hover:bg-surface-secondary">
                    <Check label={`Include ${c.name}`} checked={kept.has(c.name)} onChange={() => { toggleColumn(c.name); }} />
                    <span className="min-w-0 truncate font-mono">{c.name}</span>
                  </label>
                ))}
              </div>
            </div>
          </div>
        </ScrollArea>
        <div className="flex min-h-0 min-w-0 flex-col gap-1.5">
          <span className="text-[11px] font-medium text-muted">Preview</span>
          <div className="h-[52vh] overflow-hidden rounded-lg border border-border bg-background">
          <ScrollArea orientation="both" className="h-full">
            {noColumns || preview.problem !== null ? (
              <p className="p-2.5 text-xs text-danger">{noColumns ? "Select at least one column." : preview.problem}</p>
            ) : (
              <pre className="selectable min-w-max p-2.5 font-mono text-[11px] leading-relaxed text-foreground/90">{rendering && preview.text === "" ? "Rendering…" : preview.text}</pre>
            )}
          </ScrollArea>
          </div>
        </div>
      </DialogBody>
      <DialogFooter>
        <Button variant="secondary" onClick={() => { onOpenChange(false); }}>
          Cancel
        </Button>
        <Button variant="secondary" pending={busy === "copy"} disabled={busy !== null || chosen.length === 0} onClick={() => void run("copy")}>
          <Icon name="copy" size={13} />
          Copy
        </Button>
        <Button pending={busy === "save"} disabled={busy !== null || chosen.length === 0} onClick={() => void run("save")}>
          <Icon name="download" size={13} />
          Save file
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}
