// SOT: inspector-sql-tab, row-statements, select-insert-update-delete-by-key
import { useEffect, useState } from "react";
import type { RowSqlKind, TableRef, Value } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { Segmented } from "@/components/global/Field";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { InspectorColumn } from "./inspectorValues";

interface SqlTabProps {
  connectionId: string | undefined;
  table: TableRef | null;
  columns: readonly InspectorColumn[];
  keyColumns: readonly string[];
  rows: readonly (readonly Value[])[];
  /// A local INSERT for inspectors that have no connection to ask (query results).
  fallback: string;
  onCopied: ((what: string) => void) | undefined;
}

const KINDS: readonly { value: RowSqlKind; label: string }[] = [
  { value: "select", label: "SELECT" },
  { value: "insert", label: "INSERT" },
  { value: "update", label: "UPDATE" },
  { value: "delete", label: "DELETE" },
];

type Statement = { state: "loading" } | { state: "ready"; sql: string } | { state: "error"; message: string };

// WHAT:  The inspector's SQL tab: SELECT, INSERT, UPDATE or DELETE for the row (or
//        `IN (…)` for several), written by the engine's own dialect rules.
// WHY:   Copying a row out as a statement is what people do with it next; quoting
//        and escaping differ per engine, so Rust writes it.
// HOW:   SELECT / UPDATE / DELETE address rows by primary key; without one they say so.
//        A result that is not one table's rows gets a plain INSERT.
// WHERE: src-tauri/src/services/changes.rs (`rows_as_sql`)
export function InspectorSql({ connectionId, table, columns, keyColumns, rows, fallback, onCopied }: SqlTabProps) {
  const [kind, setKind] = useState<RowSqlKind>("select");
  const [statement, setStatement] = useState<{ key: string; value: Statement } | null>(null);
  const names = columns.map((c) => c.name);
  const request = connectionId !== undefined && table !== null ? { connectionId, table, kind, columns: names, keyColumns: [...keyColumns], rows: rows.map((r) => [...r]) } : null;
  const key = JSON.stringify(request);

  useEffect(() => {
    if (request === null) return;
    const token = { cancelled: false };
    void (async () => {
      try {
        const sql = await ipc("rows_as_sql", request);
        if (!token.cancelled) setStatement({ key, value: { state: "ready", sql } });
      } catch (raw) {
        if (!token.cancelled) setStatement({ key, value: { state: "error", message: normalizeError(raw).message } });
      }
    })();
    return () => {
      token.cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `key` encodes the whole request
  }, [key]);

  const current: Statement = request === null ? { state: "ready", sql: fallback } : statement !== null && statement.key === key ? statement.value : { state: "loading" };
  return (
    <>
      {request !== null ? (
        <div className="px-2.5 pb-1.5">
          <Segmented label="Statement" value={kind} onChange={setKind} options={KINDS} />
        </div>
      ) : null}
      <ScrollArea className="min-h-0 flex-1">
        {current.state === "ready" ? (
          <pre className="selectable p-2.5 font-mono text-[11px] leading-relaxed break-all whitespace-pre-wrap text-foreground">{current.sql}</pre>
        ) : current.state === "error" ? (
          <p className="p-2.5 text-xs text-muted">{current.message}</p>
        ) : (
          <p className="p-2.5 text-xs text-muted">Writing the statement…</p>
        )}
      </ScrollArea>
      {current.state === "ready" ? (
        <div className="flex shrink-0 items-center border-t border-border/40 glass-header p-2">
          <Button
            size="sm"
            variant="tertiary"
            className="ml-auto"
            onClick={() => {
              void navigator.clipboard.writeText(current.sql);
              onCopied?.("SQL");
            }}
          >
            Copy
          </Button>
        </div>
      ) : null}
    </>
  );
}
