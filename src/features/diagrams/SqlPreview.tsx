// SOT: diagram-sql-preview, designer-sql-dialog, table-sql-preview, use-diagram-sql
import { useEffect, useState, type ReactNode } from "react";
import type { DiagramBody, DiagramDdl } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";

const DEBOUNCE_MS = 350;

interface SqlResult {
  body: DiagramBody;
  tableId: string | null;
  ddl: DiagramDdl;
}

// WHAT:  The SQL for a diagram (or one table of it) in its connection's dialect,
//        regenerated shortly after the diagram stops changing.
// WHY:   The statements are written in Rust (quoting, schema qualification, FK
//        actions per engine), so the preview asks for them rather than guessing
//        in TypeScript; the debounce keeps a drag from asking once per frame.
// HOW:   A result remembers which body it was made from; `pending` is simply
//        "the latest result is for an older body".
export function useDiagramSql(connectionId: string | null, body: DiagramBody, tableId: string | null): { ddl: DiagramDdl | null; pending: boolean } {
  const showError = useWorkspace((s) => s.showError);
  const [result, setResult] = useState<SqlResult | null>(null);
  useEffect(() => {
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const ddl = await ipc("diagram_ddl", { connectionId, body, tableId });
          if (!cancelled) setResult({ body, tableId, ddl });
        } catch (raw) {
          if (!cancelled) showError(normalizeError(raw));
        }
      })();
    }, DEBOUNCE_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [connectionId, body, tableId, showError]);
  const current = result !== null && result.body === body && result.tableId === tableId;
  return { ddl: result?.ddl ?? null, pending: !current };
}

interface SqlViewProps {
  connectionId: string | null;
  ddl: DiagramDdl | null;
  pending: boolean;
  /// Title for the query tab "Open in query tab" creates.
  queryTitle: string;
  className?: string;
  /// Cap on the SQL area's height.
  maxHeight?: string;
}

// WHAT:  The generated SQL with what the dialect could not express, and the two
//        things to do with it: copy it, or open it in a query tab.
export function SqlView({ connectionId, ddl, pending, queryTitle, className, maxHeight = "max-h-[50vh]" }: SqlViewProps) {
  const openQuery = useWorkspace((s) => s.openQuery);
  const showInfo = useWorkspace((s) => s.showInfo);
  const sql = ddl?.sql ?? "";
  const empty = ddl !== null && sql.trim() === "";
  return (
    <div className={cn("flex min-h-0 flex-col gap-2", className)}>
      {ddl && ddl.warnings.length > 0 ? (
        <ul className="flex flex-col gap-1 rounded-md border border-warning/40 bg-warning/10 p-2 text-[11px] text-foreground">
          {ddl.warnings.map((warning) => (
            <li key={warning} className="flex gap-1.5">
              <Icon name="alert" size={12} className="mt-0.5 shrink-0 text-warning" />
              <span>{warning}</span>
            </li>
          ))}
        </ul>
      ) : null}
      <div className="overflow-hidden rounded-md bg-background">
      <ScrollArea orientation="both" className={cn("p-2.5", maxHeight)}>
        {ddl === null ? (
          <div className="flex items-center justify-center py-2.5">
            <Spinner size="sm" />
          </div>
        ) : empty ? (
          <p className="text-xs text-muted">Nothing to write yet. Add a table with at least one named column.</p>
        ) : (
          <pre className={cn("selectable font-mono text-[11px] whitespace-pre text-foreground transition-opacity", pending ? "opacity-60" : "")}>{sql}</pre>
        )}
      </ScrollArea>
      </div>
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          variant="secondary"
          disabled={sql.trim() === ""}
          onClick={() => {
            void navigator.clipboard.writeText(sql).then(() => {
              showInfo("SQL copied.");
            });
          }}
        >
          <Icon name="copy" size={13} />
          Copy
        </Button>
        <Button
          size="sm"
          variant="secondary"
          disabled={sql.trim() === "" || connectionId === null}
          title={connectionId === null ? "Connect to a database to open the SQL in a query tab." : undefined}
          onClick={() => {
            if (connectionId !== null) openQuery(connectionId, sql, queryTitle);
          }}
        >
          <Icon name="terminal" size={13} />
          Open in query tab
        </Button>
      </div>
    </div>
  );
}

interface SqlDialogProps {
  connectionId: string | null;
  body: DiagramBody;
  name: string;
  /// The control that opens the dialog (rendered through DialogTrigger asChild).
  children: ReactNode;
}

// WHAT:  The whole diagram's SQL in a dialog. The generator only runs while it is open.
export function SqlDialog({ connectionId, body, name, children }: SqlDialogProps) {
  return (
    <Dialog>
      <DialogTrigger asChild>{children}</DialogTrigger>
      <DialogContent className="sm:max-w-[760px]">
        <DialogHeader>
          <DialogTitle>Generated SQL</DialogTitle>
        </DialogHeader>
        <DialogBody>
          <DialogSql connectionId={connectionId} body={body} name={name} />
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}

function DialogSql({ connectionId, body, name }: { connectionId: string | null; body: DiagramBody; name: string }) {
  const { ddl, pending } = useDiagramSql(connectionId, body, null);
  return <SqlView connectionId={connectionId} ddl={ddl} pending={pending} queryTitle={`${name} SQL`} />;
}
