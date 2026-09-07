// SOT: results-pane, statement-tabs, query-result-grid, result-row-total
import { useState } from "react";
import { Button, Chip } from "@heroui/react";
import type { QueryOutcome } from "@/lib/bindings";
import { DENSITIES, formatCount, formatMs } from "@/lib/format";
import { useWorkspace } from "@/stores/workspace";
import { DataGrid } from "@/features/grid/DataGrid";
import { EmptyState } from "@/components/global/EmptyState";
import { cn } from "@/lib/cn";

// WHAT:  Says how many rows the script really has, not just how many fit.
// WHY:   A row cap that reads as the whole answer is worse than no cap: "1,000
//        rows" and "1,000 of 84,213 rows" lead to opposite conclusions.
// WHERE: src-tauri/src/services/query.rs (QueryOutcome.totalRows)
function rowSummary(shown: number, total: number | null, truncated: boolean): string {
  if (!truncated) return `${formatCount(shown)} rows`;
  if (total === null) return `${formatCount(shown)} rows (capped)`;
  return `${formatCount(shown)} of ${formatCount(total)} rows`;
}

export function ResultsPane({ outcome }: { outcome: QueryOutcome | null }) {
  const density = useWorkspace((s) => s.density);
  const [active, setActive] = useState(0);

  if (!outcome) {
    return <EmptyState icon="terminal" title="No results yet" body="Run a query with ⌘/Ctrl + Enter. Results appear here." />;
  }
  const statements = outcome.statements;
  const index = Math.min(active, Math.max(0, statements.length - 1));
  const current = statements[index];
  const truncated = statements.some((s) => s.kind === "rows" && s.result.truncated);
  const shown = statements.reduce((sum, s) => sum + (s.kind === "rows" ? s.result.rows.length : 0), 0);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-border bg-surface px-2 text-xs">
        {statements.map((s, i) => (
          <Button
            key={i}
            size="sm"
            variant={i === index ? "secondary" : "ghost"}
            onPress={() => setActive(i)}
            className={cn("h-6 rounded-md px-2 py-0.5 text-xs", i === index ? "text-foreground font-medium" : "text-muted hover:text-foreground")}
          >
            {s.kind === "rows" ? `Result ${i + 1} · ${formatCount(s.result.rows.length)} rows` : `Statement ${i + 1}`}
          </Button>
        ))}
        <span className="ml-auto flex items-center gap-2">
          {truncated ? (
            <Chip size="sm" color="warning" variant="soft">
              {rowSummary(shown, outcome.totalRows, truncated)}
            </Chip>
          ) : (
            <Chip size="sm" variant="soft">
              {rowSummary(shown, outcome.totalRows, truncated)}
            </Chip>
          )}
          <Chip size="sm" color="success" variant="soft">
            {formatMs(outcome.elapsedMs)}
          </Chip>
        </span>
      </div>
      <div className="min-h-0 flex-1">
        {current === undefined ? (
          <EmptyState title="Statement executed" body="No result set was returned." />
        ) : current.kind === "affected" ? (
          <EmptyState icon="check" title="Statement OK" body={`${formatCount(current.rowsAffected)} row(s) affected.`} />
        ) : current.result.columns.length === 0 ? (
          <EmptyState title="Empty result" body="The statement returned no rows." />
        ) : (
          <DataGrid
            columns={current.result.columns.map((c) => ({ name: c.name, typeName: c.typeName }))}
            rowCount={current.result.rows.length}
            getRow={(i) => current.result.rows[i]}
            rowHeight={DENSITIES[density].rowHeight}
          />
        )}
      </div>
    </div>
  );
}
