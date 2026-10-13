// SOT: export-connections-dialog, import-connections-dialog, connection-transfer-ui
import { useState } from "react";
import type { ConnectionExportReport, ConnectionImportReport } from "@/lib/bindings";
import { pickConnectionExportPath, pickConnectionImportFile } from "@/lib/connectionFiles";
import { ipc, normalizeError } from "@/lib/ipc";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { EngineIcon } from "@/components/global/EngineIcon";
import { Check, Toggle } from "@/components/global/Field";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { connectionTarget } from "@/lib/connectionGroups";
import { useConnectionOrg } from "./useConnectionOrg";
import { ScrollArea } from "@/components/ui/scroll-area";

// ============================================================================
// Export
// ============================================================================

// WHAT:  Pick connections, decide whether the file carries passwords, choose where
//        to save it.
// WHY:   "Include passwords" is an explicit opt-in with the consequence stated:
//        the file then holds them readable. Rust decrypts them straight into the
//        file; they never pass through this page.
// HOW:   Defaults to every saved connection, passwords off. The file is
//        `db-free-connections-YYYY-MM-DD.json`, which Import reads back.
export function ExportConnectionsDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[520px]">{open ? <ExportBody onClose={() => { onOpenChange(false); }} /> : null}</DialogContent>
    </Dialog>
  );
}

function ExportBody({ onClose }: { onClose: () => void }) {
  const connections = useWorkspace((s) => s.connections.filter((c) => !c.extra.transient));
  const [selected, setSelected] = useState<string[]>(() => connections.map((c) => c.id));
  const [includePasswords, setIncludePasswords] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<ConnectionExportReport | null>(null);
  const all = selected.length === connections.length && connections.length > 0;

  const run = async () => {
    setError(null);
    const path = await pickConnectionExportPath();
    if (path === null) return;
    setBusy(true);
    try {
      setReport(await ipc("export_connections", { ids: selected, includePasswords, path }));
    } catch (raw) {
      setError(normalizeError(raw).message);
    } finally {
      setBusy(false);
    }
  };

  if (report !== null) {
    return (
      <>
        <DialogHeader>
          <DialogTitle>Connections exported</DialogTitle>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-2">
          <Alert variant="success">
            <AlertIndicator />
            <AlertContent>
              <AlertTitle>
                {report.count} connection{report.count === 1 ? "" : "s"} written
              </AlertTitle>
              <AlertDescription className="selectable break-all font-mono">{report.path}</AlertDescription>
            </AlertContent>
          </Alert>
          {report.includesPasswords ? (
            <Alert variant="warning">
              <AlertIndicator />
              <AlertContent>
                <AlertTitle>The file contains passwords</AlertTitle>
                <AlertDescription>They are stored readable. Keep the file private and delete it once it has been imported.</AlertDescription>
              </AlertContent>
            </Alert>
          ) : null}
        </DialogBody>
        <DialogFooter>
          <Button onClick={onClose}>Done</Button>
        </DialogFooter>
      </>
    );
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>Export connections</DialogTitle>
        <DialogDescription>Saves the chosen connections to a db-free-connections-YYYY-MM-DD.json file that DB Free can import again.</DialogDescription>
      </DialogHeader>
      <DialogBody className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <label className="flex items-center gap-2 text-xs text-foreground">
            <Check checked={all} indeterminate={selected.length > 0 && !all} onChange={(next) => { setSelected(next ? connections.map((c) => c.id) : []); }} label="Select all connections" />
            Select all
          </label>
          <span className="text-xs tabular-nums text-muted">
            {selected.length} of {connections.length} selected
          </span>
        </div>
        <div className="overflow-hidden rounded-lg border border-border/50">
        <ScrollArea className="max-h-64 p-1">
        <ul className="flex flex-col gap-0.5" aria-label="Connections to export">
          {connections.map((c) => (
            <li key={c.id}>
              <label className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1 hover:bg-surface-secondary/60">
                <Check checked={selected.includes(c.id)} onChange={(next) => { setSelected((prev) => (next ? [...prev, c.id] : prev.filter((id) => id !== c.id))); }} label={`Export ${c.name}`} />
                <EngineIcon engine={c.engine} size={14} />
                <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground">{c.name}</span>
                <span className="max-w-[10rem] truncate font-mono text-[11px] text-muted">{connectionTarget(c)}</span>
              </label>
            </li>
          ))}
          {connections.length === 0 ? <li className="px-2 py-3 text-center text-xs text-muted">No saved connections to export.</li> : null}
        </ul>
        </ScrollArea>
        </div>
        <Toggle checked={includePasswords} onChange={setIncludePasswords} label="Include passwords" description="Off by default. When on, passwords (and SSH secrets) are written into the file in readable form." />
        {includePasswords ? (
          <Alert variant="warning">
            <AlertIndicator />
            <AlertContent>
              <AlertDescription>Anyone who can open the file can read these passwords. Store it somewhere private.</AlertDescription>
            </AlertContent>
          </Alert>
        ) : null}
        {error !== null ? (
          <Alert variant="danger">
            <AlertIndicator />
            <AlertContent>
              <AlertDescription className="selectable">{error}</AlertDescription>
            </AlertContent>
          </Alert>
        ) : null}
      </DialogBody>
      <DialogFooter>
        <Button variant="tertiary" onClick={onClose}>
          Cancel
        </Button>
        <Button pending={busy} disabled={selected.length === 0} onClick={() => void run()}>
          <Icon name="download" size={13} />
          Export…
        </Button>
      </DialogFooter>
    </>
  );
}

// ============================================================================
// Import
// ============================================================================

// WHAT:  Import connections from a DB Free / DB Pro file or a Navicat `.ncx`.
// WHY:   Says up front what is and is not carried over (Navicat passwords never
//        are), imports only after the whole file has been understood, and
//        reports exactly what happened: how many, which were skipped and why.
export function ImportConnectionsDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[540px]">{open ? <ImportBody onClose={() => { onOpenChange(false); }} /> : null}</DialogContent>
    </Dialog>
  );
}

const SOURCE_LABEL: Record<ConnectionImportReport["source"], string> = {
  db_free: "DB Free",
  db_pro: "DB Pro",
  navicat: "Navicat",
};

function ImportBody({ onClose }: { onClose: () => void }) {
  const refreshConnections = useWorkspace((s) => s.refreshConnections);
  const loadOrg = useConnectionOrg((s) => s.load);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<ConnectionImportReport | null>(null);

  const choose = async () => {
    setError(null);
    const path = await pickConnectionImportFile();
    if (path === null) return;
    setBusy(true);
    try {
      setReport(await ipc("import_connections", { path }));
      await Promise.all([refreshConnections(), loadOrg()]);
    } catch (raw) {
      setError(normalizeError(raw).message);
    } finally {
      setBusy(false);
    }
  };

  if (report !== null) {
    return (
      <>
        <DialogHeader>
          <DialogTitle>Import finished</DialogTitle>
          <DialogDescription>Read as a {SOURCE_LABEL[report.source]} file.</DialogDescription>
        </DialogHeader>
        <DialogBody className="flex flex-col gap-2">
          <Alert variant={report.imported > 0 ? "success" : "default"}>
            <AlertIndicator />
            <AlertContent>
              <AlertTitle>
                {report.imported} connection{report.imported === 1 ? "" : "s"} imported
                {report.skipped.length > 0 ? `, ${report.skipped.length} skipped` : ""}
              </AlertTitle>
              {report.importedNames.length > 0 ? <AlertDescription className="selectable">{report.importedNames.join(", ")}</AlertDescription> : null}
            </AlertContent>
          </Alert>
          {report.skipped.length > 0 ? (
            <div className="flex flex-col gap-1">
              <p className="text-xs font-semibold text-foreground">Skipped</p>
              <ScrollArea className="max-h-36">
              <ul className="flex flex-col gap-0.5 text-xs text-muted">
                {report.skipped.map((s, index) => (
                  <li key={`${s.label}:${index}`} className="selectable flex gap-2">
                    <span className="min-w-0 max-w-[45%] shrink-0 truncate font-medium text-foreground">{s.label}</span>
                    <span className="min-w-0 flex-1">{s.reason}</span>
                  </li>
                ))}
              </ul>
              </ScrollArea>
            </div>
          ) : null}
          {report.warnings.map((warning) => (
            <Alert key={warning} variant="warning">
              <AlertIndicator />
              <AlertContent>
                <AlertDescription>{warning}</AlertDescription>
              </AlertContent>
            </Alert>
          ))}
        </DialogBody>
        <DialogFooter>
          <Button onClick={onClose}>Done</Button>
        </DialogFooter>
      </>
    );
  }

  return (
    <>
      <DialogHeader>
        <DialogTitle>Import connections</DialogTitle>
        <DialogDescription>Choose a DB Free or DB Pro export, or a Navicat connections file. The whole file is checked before anything is saved.</DialogDescription>
      </DialogHeader>
      <DialogBody className="flex flex-col gap-2">
        <div className="grid gap-2 sm:grid-cols-2">
          <div className="rounded-lg border border-border/50 p-2.5">
            <p className="mb-1 flex items-center gap-2 text-xs font-medium text-foreground">
              <Icon name="file" size={14} />
              DB Free / DB Pro
            </p>
            <p className="text-xs text-muted">
              A <span className="font-mono text-foreground">.json</span> export from DB Free, or a <span className="font-mono text-foreground">.dbpro</span> file.
            </p>
          </div>
          <div className="rounded-lg border border-border/50 p-2.5">
            <p className="mb-1 flex items-center gap-2 text-xs font-medium text-foreground">
              <Icon name="xml" size={14} />
              Navicat
            </p>
            <p className="text-xs text-muted">
              A <span className="font-mono text-foreground">.ncx</span> file: MySQL, MariaDB, PostgreSQL, SQL Server, MongoDB, SQLite and Redis.
            </p>
          </div>
        </div>
        <Alert variant="warning">
          <AlertIndicator />
          <AlertContent>
            <AlertTitle>Credentials stay safe</AlertTitle>
            <AlertDescription>Navicat passwords, SSH passwords and private keys are not imported. Passwords in a file that includes them are sealed with this machine's key. Connections already saved (same engine, host, port, database and user) are skipped.</AlertDescription>
          </AlertContent>
        </Alert>
        {error !== null ? (
          <Alert variant="danger">
            <AlertIndicator />
            <AlertContent>
              <AlertTitle>Could not import</AlertTitle>
              <AlertDescription className="selectable">{error}</AlertDescription>
            </AlertContent>
          </Alert>
        ) : null}
      </DialogBody>
      <DialogFooter>
        <Button variant="tertiary" onClick={onClose}>
          Cancel
        </Button>
        <Button pending={busy} onClick={() => void choose()}>
          Choose file…
        </Button>
      </DialogFooter>
    </>
  );
}
