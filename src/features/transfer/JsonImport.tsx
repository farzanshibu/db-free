// SOT: json-import, import-operation-checklist, import-target-override, import-error-list
import { useMemo, useState } from "react";
import type { ImportOperation, ImportOperationResult, JsonImportPlan, JsonImportReport } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { formatCount } from "@/lib/format";
import { useWorkspace } from "@/stores/workspace";
import { Check } from "@/components/global/Field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { baseName, formatElapsed } from "./sizes";

// WHAT:  Imports a JSON file of several tables — `{ "users": [rows], "orders":
//        [rows], "__schema": { … } }` — as a checklist: create this table, insert
//        those rows, each ticked or not, each with the table it lands in.
// WHY:   A file like that can both rebuild a database and top one up. Tables the
//        database already has should not be created again; tables it lacks need a
//        CREATE first; and one table failing must not undo the others.
// HOW:   `plan_json_import` reads the file into operations (CREATE TABLE from the
//        `__schema` block or from the data, INSERT per table). `run_json_import`
//        runs the ticked ones in order through the block and answers per
//        operation, so a failure is a line in the list rather than an abort.
// WHERE: src-tauri/src/services/transfer.rs (plan_operations), src-tauri/src/commands/transfer.rs
interface JsonImportProps {
  connectionId: string;
  path: string;
  plan: JsonImportPlan;
  onReset: () => void;
}

function describe(operation: ImportOperation): string {
  return operation.kind === "create_table" ? `Create table (${operation.columns.length} columns)` : `Insert ${formatCount(operation.rowCount)} rows`;
}

export function JsonImport({ connectionId, path, plan, onReset }: JsonImportProps) {
  const readOnly = useWorkspace((s) => s.connections.find((c) => c.id === connectionId)?.readOnly ?? false);
  const showError = useWorkspace((s) => s.showError);
  const showInfo = useWorkspace((s) => s.showInfo);
  // A CREATE for a table that is already there starts unticked; everything else ticked.
  const [ticked, setTicked] = useState<ReadonlySet<string>>(() => new Set(plan.operations.filter((o) => !(o.kind === "create_table" && o.exists)).map((o) => o.id)));
  const [targets, setTargets] = useState<Readonly<Record<string, string>>>({});
  const [expanded, setExpanded] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [report, setReport] = useState<JsonImportReport | null>(null);

  const results = useMemo(() => new Map<string, ImportOperationResult>((report?.results ?? []).map((r) => [r.id, r])), [report]);
  const toggle = (id: string) => {
    setTicked((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const run = async () => {
    const operations = plan.operations
      .filter((o) => ticked.has(o.id))
      .map((o) => {
        const typed = targets[o.table]?.trim();
        return { id: o.id, target: typed === undefined || typed === "" ? null : typed };
      });
    if (operations.length === 0) return;
    setRunning(true);
    setReport(null);
    try {
      const result = await ipc("run_json_import", { connectionId, path, operations });
      setReport(result);
      const failed = result.results.filter((r) => !r.ok).length;
      showInfo(failed === 0 ? `Ran ${result.results.length} operation(s).` : `${result.results.length - failed} operation(s) succeeded, ${failed} failed.`);
      window.dispatchEvent(new CustomEvent("db-free:refresh-tables"));
    } catch (raw) {
      showError(normalizeError(raw));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-7 shrink-0 items-center gap-2 border-b border-border/40 px-2.5">
        <Icon name="file" size={14} className="shrink-0 text-muted" />
        <span className="min-w-0 truncate text-xs text-foreground" title={path}>
          {baseName(path)}
        </span>
        <Button size="xs" variant="ghost" onClick={onReset} disabled={running}>
          Choose another file
        </Button>
        <div className="ml-auto flex items-center gap-2">
          <Button
            size="xs"
            variant="ghost"
            onClick={() => {
              setTicked(ticked.size === plan.operations.length ? new Set() : new Set(plan.operations.map((o) => o.id)));
            }}
          >
            {ticked.size === plan.operations.length ? "Untick all" : "Tick all"}
          </Button>
          <Button size="sm" pending={running} disabled={ticked.size === 0 || readOnly} onClick={() => void run()}>
            <Icon name="play" size={13} />
            Run {ticked.size} operation{ticked.size === 1 ? "" : "s"}
          </Button>
        </div>
      </div>
      {readOnly ? <p className="border-b border-border/40 bg-warning/10 px-2.5 py-1 text-xs text-warning">This connection is read-only, so nothing here can run.</p> : null}
      {plan.errors.length > 0 ? (
        <div className="border-b border-border/40 bg-warning/10 px-2.5 py-1 text-xs text-warning">
          {plan.errors.map((e) => (
            <p key={e}>{e}</p>
          ))}
        </div>
      ) : null}
      <ScrollArea className="min-h-0 flex-1">
        {plan.operations.map((operation) => {
          const result = results.get(operation.id);
          const open = expanded === operation.id;
          return (
            <div key={operation.id} className="border-b border-border/40 px-2.5 py-1.5">
              <div className="flex items-center gap-2">
                <Check label={`Run ${describe(operation)} for ${operation.table}`} checked={ticked.has(operation.id)} onChange={() => { toggle(operation.id); }} />
                <Icon name={operation.kind === "create_table" ? "plus" : "download"} size={13} className={cn("shrink-0", operation.kind === "create_table" ? "text-accent" : "rotate-180 text-muted")} />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-xs text-foreground">
                    <span className="font-mono">{operation.table}</span> <span className="text-muted">· {describe(operation)}</span>
                  </p>
                  {result !== undefined ? <p className={cn("truncate text-xs", result.ok ? "text-success" : "text-danger")} title={result.message}>{result.message}</p> : null}
                </div>
                {operation.exists ? <Badge size="sm" variant="outline">{operation.kind === "create_table" ? "already exists" : "table exists"}</Badge> : <Badge size="sm" variant="soft">new</Badge>}
                <Input
                  aria-label={`Table for ${operation.table}`}
                  placeholder={operation.table}
                  value={targets[operation.table] ?? ""}
                  onChange={(event) => {
                    setTargets((current) => ({ ...current, [operation.table]: event.target.value }));
                  }}
                  className="h-6 w-44 font-mono text-xs"
                />
                <Button size="icon-sm" variant="ghost" aria-label={open ? "Hide details" : "Show details"} onClick={() => { setExpanded(open ? null : operation.id); }}>
                  <Icon name={open ? "chevron-down" : "chevron-right"} size={13} />
                </Button>
              </div>
              {open ? (
                <div className="ml-8 mt-2 overflow-hidden rounded-lg border border-border bg-background">
                  <ScrollArea orientation="both" className="max-h-64">
                    <pre className="selectable p-2 font-mono text-[11px] text-foreground/90">
                      {operation.sql}
                      {operation.kind === "insert" ? `\n-- columns: ${operation.columns.map((c) => c.name).join(", ")}` : ""}
                    </pre>
                  </ScrollArea>
                </div>
              ) : null}
            </div>
          );
        })}
        {report !== null ? <p className="p-2.5 text-xs text-muted">Finished in {formatElapsed(report.elapsedMs)}. Operations run independently: a failure does not undo the others.</p> : null}
      </ScrollArea>
    </div>
  );
}
