// SOT: backup-dialog, restore-dialog, native-backup-ui, backup-log-pane
import { useEffect, useRef, useState, type RefObject } from "react";
import type { BackupFormat, BackupMethod, BackupOptions, BackupReport, BackupSupport, ConnectionSummary, NativeToolStatus } from "@/lib/bindings";
import { errorMessage, ipc, normalizeError, onBackupProgress } from "@/lib/ipc";
import { pickBackupSavePath, pickBackupSource } from "@/lib/native";
import { engineMeta } from "@/lib/engines";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { useWorkspace } from "@/stores/workspace";
import { useBackupDialog } from "@/features/backup/useBackupDialog";
import { AppSelect, Field, Segmented, Toggle } from "@/components/global/Field";
import { EngineIcon } from "@/components/global/EngineIcon";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";

type Mode = "backup" | "restore";
type Content = "all" | "schema" | "data";

/// Lines kept in the log pane; a verbose dump of a big schema prints thousands.
const LOG_LIMIT = 2_000;

const FORMAT_META = {
  custom: { label: "Custom archive (-Fc)", extension: "dump", hint: "Compressed; restore selectively with pg_restore." },
  plain: { label: "Plain SQL", extension: "sql", hint: "A script any SQL client can replay." },
  directory: { label: "Directory (-Fd)", extension: "", hint: "One file per table, in a folder pg_dump creates." },
  tar: { label: "Tar archive (-Ft)", extension: "tar", hint: "Restorable with pg_restore." },
  archive: { label: "Archive", extension: "archive", hint: "mongodump --archive; restore with mongorestore." },
  file: { label: "Database file", extension: "db", hint: "A consistent copy of the database file." },
} satisfies Record<BackupFormat, { label: string; extension: string; hint: string }>;

const METHOD_LABEL = {
  pg_dump: "pg_dump / pg_restore",
  mysqldump: "mysqldump / mysql",
  mongodump: "mongodump / mongorestore",
  sqlite_copy: "SQLite VACUUM INTO",
  file_copy: "File copy",
} satisfies Record<BackupMethod, string>;

function defaultOptions(format: BackupFormat): BackupOptions {
  return {
    format,
    schemaOnly: false,
    dataOnly: false,
    schemas: [],
    tables: [],
    clean: false,
    noOwner: false,
    routines: true,
    triggers: true,
    singleTransaction: true,
    gzip: true,
    drop: false,
  };
}

function list(text: string): string[] {
  return text
    .split(/[,\n]/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

function stamp(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}`;
}

function fileExtension(connection: ConnectionSummary, options: BackupOptions): string {
  if (options.format === "file") return connection.engine === "duckdb" ? "duckdb" : "db";
  if (options.format === "archive") return options.gzip ? "gz" : "archive";
  return FORMAT_META[options.format].extension;
}

function mb(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function targetOf(connection: ConnectionSummary): string {
  if (connection.filePath) return connection.filePath;
  return connection.database && connection.database.length > 0 ? connection.database : "the default database";
}

// WHAT:  Backup / Restore dialog, mounted once in App.tsx and opened for one
//        connection through useBackupDialog.
export function BackupDialog() {
  const connectionId = useBackupDialog((s) => s.connectionId);
  const close = useBackupDialog((s) => s.close);
  const connection = useWorkspace((s) => s.connections.find((c) => c.id === connectionId) ?? null);
  const [running, setRunning] = useState(false);
  if (!connection) return null;
  return (
    // A run in flight must be stopped with Cancel, not abandoned by Escape:
    // the tool would keep writing with nobody watching.
    <Dialog open onOpenChange={(open) => { if (!open && !running) close(); }}>
      <DialogContent className="sm:max-w-[760px]" showCloseButton={!running}>
        <BackupBody key={connection.id} connection={connection} running={running} setRunning={setRunning} />
      </DialogContent>
    </Dialog>
  );
}

function BackupBody({ connection, running, setRunning }: { connection: ConnectionSummary; running: boolean; setRunning: (v: boolean) => void }) {
  const live = useWorkspace((s) => s.sessions.includes(connection.id));
  const disconnect = useWorkspace((s) => s.disconnect);
  const goSettings = useWorkspace((s) => s.goSettings);
  const showInfo = useWorkspace((s) => s.showInfo);
  const close = useBackupDialog((s) => s.close);

  const [mode, setMode] = useState<Mode>("backup");
  const [support, setSupport] = useState<BackupSupport | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [options, setOptions] = useState<BackupOptions>(() => defaultOptions("custom"));
  const [schemasText, setSchemasText] = useState("");
  const [tablesText, setTablesText] = useState("");
  const [path, setPath] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [log, setLog] = useState<string[]>([]);
  const [progress, setProgress] = useState<{ bytes: number; total: number | null } | null>(null);
  const [report, setReport] = useState<BackupReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const runId = useRef<string | null>(null);
  const logEnd = useRef<HTMLDivElement | null>(null);

  // WHAT:  Which tools are installed for this engine.
  // HOW:   The body is keyed by connection, so this runs once per dialog.
  useEffect(() => {
    void (async () => {
      try {
        const found = await ipc("detect_native_tools", { connectionId: connection.id });
        setSupport(found);
        const first = found.formats[0];
        if (first !== undefined) setOptions(defaultOptions(first));
      } catch (raw) {
        setLoadError(errorMessage(normalizeError(raw)));
      }
    })();
  }, [connection.id]);

  // WHAT:  The live log and byte count of the run this dialog started.
  useEffect(() => {
    const pending = onBackupProgress((event) => {
      if (event.runId !== runId.current) return;
      if (event.type === "log") {
        setLog((prev) => (prev.length >= LOG_LIMIT ? [...prev.slice(prev.length - LOG_LIMIT + 1), event.line] : [...prev, event.line]));
      } else {
        setProgress({ bytes: event.bytes, total: event.total });
      }
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    logEnd.current?.scrollIntoView({ block: "end" });
  }, [log]);

  const method = support?.method ?? null;
  const tools = support?.tools ?? [];
  const neededTool = (m: Mode): NativeToolStatus | undefined => {
    if (method === "pg_dump") return tools.find((t) => t.tool === (m === "backup" ? "pg_dump" : "pg_restore"));
    return m === "backup" ? tools[0] : tools[1];
  };
  const needed = neededTool(mode);
  const missing = method !== null && tools.length > 0 && (needed?.path ?? null) === null;
  const mustDisconnect = live && ((mode === "restore" && support?.restoreNeedsDisconnect === true) || (mode === "backup" && support?.backupNeedsDisconnect === true));
  const content: Content = options.schemaOnly ? "schema" : options.dataOnly ? "data" : "all";
  const patch = (partial: Partial<BackupOptions>) => setOptions((o) => ({ ...o, ...partial }));
  const target = targetOf(connection);

  const switchMode = (next: Mode) => {
    setMode(next);
    setPath("");
    setConfirmed(false);
    setReport(null);
    setError(null);
  };

  const choosePath = async () => {
    const picked =
      mode === "backup"
        ? await pickBackupSavePath(`${connection.name.replace(/[^\w.-]+/g, "_")}-${stamp()}`, fileExtension(connection, options), FORMAT_META[options.format].label)
        : await pickBackupSource(method === "pg_dump" && options.format === "directory");
    if (picked !== null) setPath(picked);
  };

  const start = async () => {
    const id = crypto.randomUUID();
    runId.current = id;
    setRunning(true);
    setLog([]);
    setProgress(null);
    setReport(null);
    setError(null);
    const request = { ...options, schemas: list(schemasText), tables: list(tablesText) };
    try {
      const done =
        mode === "backup"
          ? await ipc("backup_database", { connectionId: connection.id, runId: id, path, options: request })
          : await ipc("restore_database", { connectionId: connection.id, runId: id, path, options: request, confirmDestructive: confirmed });
      setReport(done);
      if (!done.cancelled) showInfo(mode === "backup" ? "Backup finished." : "Restore finished. Refresh the sidebar to see the restored objects.");
    } catch (raw) {
      setError(errorMessage(normalizeError(raw)));
    } finally {
      setRunning(false);
      setConfirmed(false);
    }
  };

  const cancel = async () => {
    const id = runId.current;
    if (id === null) return;
    try {
      await ipc("cancel_backup", { runId: id });
    } catch (raw) {
      setError(errorMessage(normalizeError(raw)));
    }
  };

  const ready = method !== null && !missing && !mustDisconnect && path.length > 0 && !running && (mode === "backup" || (confirmed && !connection.readOnly));

  return (
    <>
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <EngineIcon engine={connection.engine} size={18} />
          Backup / Restore — {connection.name}
        </DialogTitle>
        <DialogDescription>
          {engineMeta(connection.engine).label} · {method !== null ? METHOD_LABEL[method] : "no native backup"}
        </DialogDescription>
      </DialogHeader>
      <DialogBody className="flex flex-col gap-3.5">
        {loadError !== null ? (
          <Alert variant="danger">
            <AlertIndicator />
            <AlertContent>
              <AlertDescription>{loadError}</AlertDescription>
            </AlertContent>
          </Alert>
        ) : null}
        {support === null && loadError === null ? (
          <div className="flex items-center gap-2 text-xs text-muted">
            <Spinner size="sm" /> Looking for backup tools…
          </div>
        ) : null}
        {support !== null && method === null ? (
          <Alert>
            <AlertIndicator />
            <AlertContent>
              <AlertDescription>{support.note ?? "This engine has no native backup."}</AlertDescription>
            </AlertContent>
          </Alert>
        ) : null}

        {method !== null ? (
          <>
            <Segmented<Mode>
              label="Backup or restore"
              value={mode}
              onChange={switchMode}
              options={[
                { value: "backup", label: "Backup", disabled: running },
                { value: "restore", label: "Restore", disabled: running },
              ]}
            />

            {tools.length > 0 ? <ToolList tools={tools} onSettings={() => { close(); goSettings(); }} /> : null}

            {mustDisconnect ? (
              <Alert variant="warning">
                <AlertIndicator />
                <AlertContent>
                  <AlertTitle>Disconnect first</AlertTitle>
                  <AlertDescription>
                    {mode === "restore" ? "Restoring replaces the database file" : "DuckDB locks its file while it is open"}, so the app must let go of it.
                  </AlertDescription>
                </AlertContent>
                <Button size="sm" variant="secondary" onClick={() => void disconnect(connection.id)}>
                  Disconnect
                </Button>
              </Alert>
            ) : null}

            <MethodOptions
              method={method}
              mode={mode}
              formats={support?.formats ?? []}
              options={options}
              content={content}
              patch={patch}
              schemasText={schemasText}
              setSchemasText={setSchemasText}
              tablesText={tablesText}
              setTablesText={setTablesText}
              disabled={running}
            />

            <div className="flex items-end gap-2">
              <Field
                label={mode === "backup" ? "Save backup to" : "Restore from"}
                value={path}
                onChange={setPath}
                placeholder={mode === "backup" ? "Choose where to write the backup…" : "Choose a backup file…"}
                mono
                disabled={running}
              />
              <Button variant="secondary" onClick={() => void choosePath()} disabled={running}>
                <Icon name="folder" size={14} />
                Browse…
              </Button>
            </div>

            {mode === "restore" ? (
              <Alert variant="danger">
                <AlertIndicator />
                <AlertContent className="gap-2">
                  <AlertTitle>This overwrites data in {target}</AlertTitle>
                  <AlertDescription>
                    Restoring into <span className="font-semibold text-danger">{connection.name}</span> ({target}) replaces what is there with the contents of the backup. It cannot be undone
                    from here — take a backup first if you may need the current data.
                  </AlertDescription>
                  {connection.readOnly ? null : <Toggle checked={confirmed} onChange={setConfirmed} label={`I understand — overwrite ${target}`} />}
                  {connection.readOnly ? <AlertDescription className="text-danger">This connection is read-only, so restoring is blocked.</AlertDescription> : null}
                </AlertContent>
              </Alert>
            ) : null}

            {running || log.length > 0 || report !== null || error !== null ? (
              <RunPane running={running} log={log} progress={progress} report={report} error={error} logEnd={logEnd} />
            ) : null}
          </>
        ) : null}
      </DialogBody>
      <DialogFooter>
        {running ? (
          <Button variant="danger-soft" onClick={() => void cancel()}>
            <Icon name="x" size={14} />
            Cancel
          </Button>
        ) : (
          <Button variant="secondary" onClick={close}>
            Close
          </Button>
        )}
        {method !== null ? (
          <Button variant={mode === "restore" ? "danger" : "primary"} disabled={!ready} pending={running} onClick={() => void start()}>
            <Icon name={mode === "backup" ? "download" : "database-sync"} size={14} />
            {mode === "backup" ? "Start backup" : `Restore into ${connection.name}`}
          </Button>
        ) : null}
      </DialogFooter>
    </>
  );
}

function ToolList({ tools, onSettings }: { tools: NativeToolStatus[]; onSettings: () => void }) {
  const anyMissing = tools.some((t) => t.path === null);
  return (
    <div className="flex flex-col gap-1.5 rounded-lg border border-border/60 bg-surface/60 px-3 py-2.5">
      {tools.map((t) => (
        <div key={t.tool} className="flex min-w-0 items-center gap-2 text-xs">
          <Icon name={t.path !== null ? "check" : "alert"} size={13} className={cn("shrink-0", t.path !== null ? "text-success" : "text-warning")} />
          <span className="shrink-0 font-mono font-semibold text-foreground">{t.tool}</span>
          {t.path !== null ? (
            <span className="min-w-0 truncate font-mono text-[11px] text-muted" title={t.path}>
              {t.version ?? t.path}
            </span>
          ) : (
            <span className="min-w-0 truncate text-muted">
              not found — install {t.package} or set its path in Settings → Advanced
            </span>
          )}
          {t.overridden ? (
            <Badge size="sm" variant="outline" className="ml-auto">
              override
            </Badge>
          ) : null}
        </div>
      ))}
      {anyMissing ? (
        <Button size="xs" variant="link" className="self-start px-0" onClick={onSettings}>
          Open Settings
        </Button>
      ) : null}
    </div>
  );
}

interface MethodOptionsProps {
  method: BackupMethod;
  mode: Mode;
  formats: readonly BackupFormat[];
  options: BackupOptions;
  content: Content;
  patch: (partial: Partial<BackupOptions>) => void;
  schemasText: string;
  setSchemasText: (v: string) => void;
  tablesText: string;
  setTablesText: (v: string) => void;
  disabled: boolean;
}

// WHAT:  The knobs that apply to this engine and direction, and only those.
function MethodOptions({ method, mode, formats, options, content, patch, schemasText, setSchemasText, tablesText, setTablesText, disabled }: MethodOptionsProps) {
  const contentPicker = (
    <Segmented<Content>
      label="What to include"
      value={content}
      onChange={(v) => patch({ schemaOnly: v === "schema", dataOnly: v === "data" })}
      options={[
        { value: "all", label: "Schema + data", disabled },
        { value: "schema", label: "Schema only", disabled },
        { value: "data", label: "Data only", disabled },
      ]}
    />
  );

  if (method === "sqlite_copy" || method === "file_copy") {
    return (
      <p className="text-xs text-muted">
        {mode === "backup"
          ? method === "sqlite_copy"
            ? "Writes a consistent, compacted copy with VACUUM INTO — safe while the database is open."
            : "Copies the database file (and its WAL) while no one has it open."
          : "Checks the backup, then replaces the database file with it. Stale journal and WAL files are removed."}
      </p>
    );
  }

  if (method === "pg_dump") {
    return (
      <div className="flex flex-col gap-3">
        {mode === "backup" ? (
          <AppSelect<BackupFormat>
            label="Format"
            value={options.format}
            options={formats.map((f) => ({ value: f, label: FORMAT_META[f].label }))}
            onChange={(format) => patch({ format })}
            disabled={disabled}
          />
        ) : (
          <>
            <p className="text-xs text-muted">The format is detected from the file: archives go to pg_restore, SQL scripts to psql.</p>
            <Toggle checked={options.format === "directory"} onChange={(dir) => patch({ format: dir ? "directory" : "custom" })} label="The backup is a directory (-Fd)" />
          </>
        )}
        {mode === "backup" ? <p className="-mt-1.5 text-xs text-muted">{FORMAT_META[options.format].hint}</p> : null}
        {contentPicker}
        <div className="grid grid-cols-2 gap-3">
          <Field label="Schemas" optional value={schemasText} onChange={setSchemasText} placeholder="public, sales" mono disabled={disabled} />
          <Field label="Tables" optional value={tablesText} onChange={setTablesText} placeholder="public.orders, public.items" mono disabled={disabled} />
        </div>
        <div className="grid grid-cols-2 gap-2">
          <Toggle checked={options.noOwner} onChange={(noOwner) => patch({ noOwner })} label="No owner (--no-owner)" />
          {mode === "restore" || options.format === "plain" ? (
            <Toggle checked={options.clean} onChange={(clean) => patch({ clean })} label="Drop objects first (--clean)" />
          ) : null}
          {mode === "restore" ? <Toggle checked={options.singleTransaction} onChange={(singleTransaction) => patch({ singleTransaction })} label="Single transaction" /> : null}
        </div>
      </div>
    );
  }

  if (method === "mysqldump") {
    if (mode === "restore") {
      return <p className="text-xs text-muted">The SQL file is streamed into the mysql client against this connection&apos;s database.</p>;
    }
    return (
      <div className="flex flex-col gap-3">
        {contentPicker}
        <Field label="Tables" optional value={tablesText} onChange={setTablesText} placeholder="orders, items" mono disabled={disabled} description="Needs a database on the connection; empty dumps every table." />
        <div className="grid grid-cols-2 gap-2">
          <Toggle checked={options.singleTransaction} onChange={(singleTransaction) => patch({ singleTransaction })} label="Single transaction (InnoDB snapshot)" />
          <Toggle checked={options.routines} onChange={(routines) => patch({ routines })} label="Routines" />
          <Toggle checked={options.triggers} onChange={(triggers) => patch({ triggers })} label="Triggers" />
        </div>
      </div>
    );
  }

  // mongodump
  return mode === "backup" ? (
    <div className="flex flex-col gap-3">
      <Field label="Collection" optional value={tablesText} onChange={setTablesText} placeholder="users" mono disabled={disabled} description="One collection, or empty for the whole database." />
      <Toggle checked={options.gzip} onChange={(gzip) => patch({ gzip })} label="Gzip the archive" />
    </div>
  ) : (
    <Toggle checked={options.drop} onChange={(drop) => patch({ drop })} label="Drop each collection before restoring (--drop)" />
  );
}

function RunPane({
  running,
  log,
  progress,
  report,
  error,
  logEnd,
}: {
  running: boolean;
  log: string[];
  progress: { bytes: number; total: number | null } | null;
  report: BackupReport | null;
  error: string | null;
  logEnd: RefObject<HTMLDivElement | null>;
}) {
  const percent = progress !== null && progress.total !== null && progress.total > 0 ? Math.min(100, Math.round((progress.bytes / progress.total) * 100)) : null;
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2 text-xs">
        {running ? <Spinner size="sm" /> : null}
        <span className="font-medium text-foreground">
          {running ? "Running…" : report?.cancelled ? "Cancelled" : report !== null ? `Done in ${(report.elapsedMs / 1000).toFixed(1)} s` : error !== null ? "Failed" : ""}
        </span>
        {progress !== null ? (
          <span className="font-mono text-muted">
            {mb(progress.bytes)}
            {progress.total !== null ? ` of ${mb(progress.total)}` : ""}
          </span>
        ) : null}
        {report !== null && !report.cancelled && report.bytes !== null ? <span className="ml-auto font-mono text-muted">{mb(report.bytes)} · {report.tool}</span> : null}
      </div>
      {percent !== null ? (
        <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-secondary">
          <div className="h-full rounded-full bg-accent transition-[width]" style={{ width: `${percent}%` }} />
        </div>
      ) : running ? (
        <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-secondary">
          <div className="h-full w-1/3 animate-pulse rounded-full bg-accent/60" />
        </div>
      ) : null}
      {error !== null ? (
        <Alert variant="danger">
          <AlertIndicator />
          <AlertContent>
            <AlertDescription className="whitespace-pre-wrap font-mono text-[11px]">{error}</AlertDescription>
          </AlertContent>
        </Alert>
      ) : null}
      <div className="h-48 overflow-auto rounded-lg border border-border/60 bg-background/70 px-2.5 py-2 font-mono text-[11px] leading-relaxed text-muted" aria-label="Backup log" role="log">
        {log.map((line, i) => (
          <div key={i} className={cn("whitespace-pre-wrap break-all", line.startsWith("$ ") ? "text-foreground" : "")}>
            {line}
          </div>
        ))}
        <div ref={logEnd} />
      </div>
      {report !== null && !report.cancelled ? <p className="truncate font-mono text-[11px] text-muted" title={report.path}>{report.path}</p> : null}
    </div>
  );
}
