// SOT: history-panel, query-history-ui
import { useEffect, useMemo, useState } from "react";
import type { HistoryEntry } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { formatMs } from "@/lib/format";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";

/// How many distinct statements the panel keeps in view.
const RECENT = 100;

// WHAT:  The editor's side panel of recent statements: each text once, newest
//        first, click one to put it in the editor.
// WHY:   Running the same query twenty times should leave one line to click, not
//        twenty. Only what was typed in the editor is here; the queries the app
//        runs for table pages and dashboards are in the History tab.
// WHERE: src-tauri/src/store/history.rs (recent_editor_queries)
export function HistoryPanel({ connectionId, refreshKey, onPick }: { connectionId: string; refreshKey: number; onPick: (sql: string) => void }) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [needle, setNeedle] = useState("");

  useEffect(() => {
    const token = { cancelled: false };
    void (async () => {
      try {
        const rows = await ipc("list_editor_history", { connectionId, limit: RECENT });
        if (!token.cancelled) {
          setEntries(rows);
          setError(null);
        }
      } catch (raw) {
        if (!token.cancelled) setError(normalizeError(raw).message);
      }
    })();
    return () => {
      token.cancelled = true;
    };
  }, [connectionId, refreshKey]);

  const shown = useMemo(() => {
    const wanted = needle.trim().toLowerCase();
    return wanted.length === 0 ? entries : entries.filter((e) => e.sql.toLowerCase().includes(wanted));
  }, [entries, needle]);

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex h-7 shrink-0 items-center border-b border-border/40 px-2.5 text-xs font-semibold text-muted tracking-tight">History</div>
      <div className="shrink-0 border-b border-border/40 p-2">
        <SearchInput value={needle} onChange={setNeedle} placeholder="Search history" aria-label="Search history" className="h-6 text-xs" />
      </div>
      {error ? (
        <Alert variant="danger" className="m-2 rounded-lg text-xs">
          <AlertIndicator />
          <AlertContent>
            <AlertTitle>History Error</AlertTitle>
            <AlertDescription>{error}</AlertDescription>
          </AlertContent>
        </Alert>
      ) : null}
      <ScrollArea className="flex-1">
        <ul className="divide-y divide-border/20">
          {entries.length === 0 ? <li className="p-2.5 text-xs text-muted">Nothing run yet.</li> : null}
          {entries.length > 0 && shown.length === 0 ? <li className="p-2.5 text-xs text-muted">Nothing matches.</li> : null}
          {shown.map((e) => (
            <li key={e.id} title={e.error ?? e.sql}>
              <Button
                variant="ghost"
                onClick={() => { onPick(e.sql); }}
                className="h-auto w-full flex-col items-start justify-start gap-1 rounded-none px-2.5 py-1.5 text-left bg-transparent hover:bg-surface-secondary/70"
              >
                <span className="w-full truncate font-mono text-[11px] text-foreground">{e.sql.replace(/\s+/g, " ")}</span>
                <span className="flex w-full items-center gap-2 text-[10px] text-muted">
                  <Badge size="sm" variant={e.status === "ok" ? "success" : "danger"} className="text-[9.5px]">
                    {e.status === "ok" ? "ok" : "error"}
                  </Badge>
                  <span>{formatMs(e.elapsedMs)}</span>
                  {e.rowCount !== null ? <span>{e.rowCount} rows</span> : null}
                  <span className="ml-auto">{new Date(e.executedAt).toLocaleTimeString()}</span>
                </span>
              </Button>
            </li>
          ))}
        </ul>
      </ScrollArea>
    </div>
  );
}
