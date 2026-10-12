// SOT: inline-result-grid, chat-result-toolbar, chat-copy-export-menu
import { useMemo } from "react";
import type { Engine, QueryOutcome } from "@/lib/bindings";
import { isRows } from "@/features/dashboards/charts";
import { cellClass, formatCell, formatCount, formatMs } from "@/lib/format";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ipcMessage } from "./ipcMessage";
import {
  RESULT_FORMATS,
  firstRows,
  guessTable,
  resultText,
  saveTextFile,
  type ResultFormat,
} from "./resultExport";

// WHAT:  A small result grid for inside a message, and the copy / export menu
//        that goes with it.
// WHY:   The chat shows rows in two places — a chart or table the assistant drew
//        and the result of a statement the user ran from a code block — and both
//        need the same grid and the same way to take the rows away.
// HOW:   The grid is deliberately plain (no virtualisation, no editing): a chat
//        result is a glance, capped at a few hundred rows by the caller. Export
//        uses the shared CSV/JSON serializers plus an INSERT-statement writer.
// WHERE: src/features/chat/Markdown.tsx (code blocks), ArtifactView.tsx (tables)

export function ResultTable({ outcome }: { outcome: QueryOutcome }) {
  const first = outcome.statements.find(isRows);
  if (first === undefined || first.result.rows.length === 0) {
    return <div className="p-2.5 text-[11px] text-muted">No rows returned.</div>;
  }
  const { columns, rows } = first.result;
  return (
    <ScrollArea className="max-h-72">
      <div className="overflow-x-auto">
        <table className="w-full border-collapse text-[11px]">
          <thead className="sticky top-0 bg-surface-secondary/90">
            <tr>
              {columns.map((column, index) => (
                <th
                  // Two columns can share a name (a join); position keeps the key unique.
                  key={`${index}:${column.name}`}
                  className="border-b border-border/50 px-2 py-1 text-left font-semibold whitespace-nowrap text-muted"
                >
                  {column.name}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, index) => (
              // Result rows have no natural key; position is the identity here.
              <tr key={index} className="row-stripe">
                {row.map((cell, cellIndex) => {
                  // Same formatter and type tint the data grid uses, so a value
                  // reads identically wherever it appears.
                  const formatted = formatCell(cell);
                  return (
                    <td
                      key={`${cellIndex}:${columns[cellIndex]?.name ?? ""}`}
                      className={cn(
                        "selectable border-b border-border/30 px-2 py-1 align-top whitespace-nowrap",
                        formatted.align === "right" && "text-right",
                        cellClass(formatted.kind),
                      )}
                    >
                      {formatted.text}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </ScrollArea>
  );
}

interface ResultToolbarProps {
  outcome: QueryOutcome;
  engine: Engine;
  /// The statement that produced the rows; its FROM names the table for SQL export.
  sql: string;
  className?: string;
}

// WHAT:  "N rows · 12 ms" and the Copy / Save menu.
export function ResultToolbar({ outcome, engine, sql, className }: ResultToolbarProps) {
  const showInfo = useWorkspace((s) => s.showInfo);
  const showError = useWorkspace((s) => s.showError);
  const data = useMemo(() => firstRows(outcome), [outcome]);
  const rowCount = data?.rows.length ?? 0;
  const table = useMemo(() => guessTable(sql) ?? "table_name", [sql]);

  const copy = (format: ResultFormat) => {
    if (data === null) return;
    const text = resultText(format, data, { engine, table });
    void (async () => {
      try {
        await navigator.clipboard.writeText(text);
        showInfo(`Copied ${formatCount(rowCount)} rows as ${format.toUpperCase()}.`);
      } catch (raw) {
        showError(ipcMessage(raw));
      }
    })();
  };

  const save = (format: ResultFormat) => {
    if (data === null) return;
    const spec = RESULT_FORMATS.find((candidate) => candidate.value === format);
    if (spec === undefined) return;
    const stem = table.replace(/[^A-Za-z0-9._-]+/g, "_") || "result";
    saveTextFile(`${stem}.${spec.extension}`, resultText(format, data, { engine, table }), spec.mime);
  };

  return (
    <div className={cn("flex items-center justify-between gap-2 border-t border-border/40 px-2 py-1", className)}>
      <span className="text-[10.5px] text-muted tabular-nums">
        {formatCount(rowCount)} {rowCount === 1 ? "row" : "rows"} · {formatMs(outcome.elapsedMs)}
        {outcome.statements.some((s) => s.kind === "rows" && s.result.truncated) ? " · truncated" : ""}
      </span>
      {data !== null && rowCount > 0 ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="sm" variant="ghost" className="h-5 rounded px-1.5 text-[10.5px] text-muted hover:text-foreground">
              <Icon name="copy" size={10} />
              Copy / export
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuLabel>Copy results as</DropdownMenuLabel>
            {RESULT_FORMATS.map((format) => (
              <DropdownMenuItem key={`copy-${format.value}`} onSelect={() => { copy(format.value); }}>
                <Icon name="clipboard" />
                {format.label}
              </DropdownMenuItem>
            ))}
            <DropdownMenuSeparator />
            <DropdownMenuLabel>Save as file</DropdownMenuLabel>
            {RESULT_FORMATS.map((format) => (
              <DropdownMenuItem key={`save-${format.value}`} onSelect={() => { save(format.value); }}>
                <Icon name="download" />
                {format.label}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );
}
