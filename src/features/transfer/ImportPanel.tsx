// SOT: import-panel, import-file-picker, import-drop-zone, import-file-routing
import { useEffect, useRef, useState } from "react";
import type { JsonImportPlan } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { onFilesDropped, pickTransferFile } from "@/lib/native";
import { useWorkspace } from "@/stores/workspace";
import { EmptyState } from "@/components/global/EmptyState";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { JsonImport } from "./JsonImport";
import { SqlImport } from "./SqlImport";
import { TableImport } from "./TableImport";
import { fileKindOf } from "./sizes";

// WHAT:  Import tab: take a file by dialog or by dropping it on the tab, work out
//        what it is, and hand it to the right importer — rows into one table
//        (CSV, TSV, or a JSON array), a checklist of tables (JSON object), or a
//        SQL script.
// WHY:   People arrive with a file, not with a format. The extension says CSV or
//        SQL; a JSON file has to be opened to tell one table from several.
// WHERE: src/features/transfer/{TableImport,JsonImport,SqlImport}.tsx
type Source =
  | { path: string; kind: "table"; format: "csv" | "json" }
  | { path: string; kind: "tables"; plan: JsonImportPlan }
  | { path: string; kind: "sql" };

export function ImportPanel({ connectionId }: { connectionId: string }) {
  const showError = useWorkspace((s) => s.showError);
  const [source, setSource] = useState<Source | null>(null);
  const [opening, setOpening] = useState(false);
  const [dragging, setDragging] = useState(false);
  const opened = useRef(0);

  const open = async (path: string) => {
    const kind = fileKindOf(path);
    if (kind === null) {
      showError("Choose a .csv, .tsv, .txt, .json or .sql file.");
      return;
    }
    const mine = ++opened.current;
    setOpening(true);
    try {
      if (kind === "json") {
        const plan = await ipc("plan_json_import", { connectionId, path });
        if (mine === opened.current) setSource(plan.shape === "rows" ? { path, kind: "table", format: "json" } : { path, kind: "tables", plan });
      } else if (mine === opened.current) {
        setSource(kind === "sql" ? { path, kind: "sql" } : { path, kind: "table", format: "csv" });
      }
    } catch (raw) {
      if (mine === opened.current) showError(normalizeError(raw));
    } finally {
      if (mine === opened.current) setOpening(false);
    }
  };

  const choose = async (kind: "data" | "sql") => {
    const path = await pickTransferFile(kind);
    if (path !== null) await open(path);
  };

  // A file dropped anywhere on the window opens here while this tab is mounted.
  const openRef = useRef(open);
  useEffect(() => {
    openRef.current = open;
  });
  useEffect(() => {
    let stop: (() => void) | null = null;
    let gone = false;
    void onFilesDropped((paths) => {
      setDragging(false);
      const first = paths[0];
      if (first !== undefined) void openRef.current(first);
    })
      .then((unlisten) => {
        if (gone) unlisten();
        else stop = unlisten;
      })
      .catch(() => undefined);
    return () => {
      gone = true;
      stop?.();
    };
  }, []);

  const reset = () => {
    opened.current += 1;
    setSource(null);
  };

  if (source?.kind === "table") return <TableImport key={source.path} connectionId={connectionId} path={source.path} format={source.format} onReset={reset} />;
  if (source?.kind === "tables") return <JsonImport key={source.path} connectionId={connectionId} path={source.path} plan={source.plan} onReset={reset} />;
  if (source?.kind === "sql") return <SqlImport key={source.path} connectionId={connectionId} path={source.path} onReset={reset} />;

  return (
    <div
      className={cn("m-2 flex min-h-0 flex-1 items-center justify-center rounded-lg border border-dashed transition-colors", dragging ? "border-accent bg-accent/5" : "border-border")}
      onDragEnter={() => { setDragging(true); }}
      onDragLeave={() => { setDragging(false); }}
    >
      {opening ? (
        <Spinner size="sm" />
      ) : (
        <EmptyState
          icon="download"
          title="Import a file"
          body="Drop a CSV, TSV, JSON or SQL file here. A CSV or a JSON array loads into one table, a JSON object of tables becomes a checklist, and a .sql script runs statement by statement."
          action={
            <div className="flex gap-2">
              <Button size="sm" onClick={() => void choose("data")}>
                <Icon name="folder" size={13} />
                Choose data file…
              </Button>
              <Button size="sm" variant="secondary" onClick={() => void choose("sql")}>
                <Icon name="code" size={13} />
                Choose SQL script…
              </Button>
            </div>
          }
        />
      )}
    </div>
  );
}
