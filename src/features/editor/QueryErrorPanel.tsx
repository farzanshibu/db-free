// SOT: query-error-panel, error-position-excerpt, error-actions
import type { AppError } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";

interface QueryErrorPanelProps {
  error: AppError;
  /// The statement that failed, for the line and column and the excerpt. Absent when no statement ran.
  sql: string | null;
  onGoToError?: (() => void) | undefined;
  onAskAi?: (() => void) | undefined;
  onCopy: (text: string) => void;
}

/// Where a 1-based UTF-16 offset sits in `sql`.
export function lineColumnOf(sql: string, position: number): { line: number; column: number; text: string } {
  const at = Math.min(Math.max(position - 1, 0), sql.length);
  const before = sql.slice(0, at);
  const line = before.split("\n").length;
  const lineStart = before.lastIndexOf("\n") + 1;
  const end = sql.indexOf("\n", at);
  return { line, column: at - lineStart + 1, text: sql.slice(lineStart, end === -1 ? sql.length : end) };
}

/// A long line cut to the stretch around the column, with the caret's new column.
function excerpt(text: string, column: number): { shown: string; caret: number } {
  const width = 100;
  if (text.length <= width) return { shown: text, caret: column - 1 };
  const start = Math.max(0, Math.min(column - 1 - Math.floor(width / 2), text.length - width));
  const cut = text.slice(start, start + width);
  return { shown: `${start > 0 ? "…" : ""}${cut}${start + width < text.length ? "…" : ""}`, caret: column - 1 - start + (start > 0 ? 1 : 0) };
}

function titleOf(error: AppError): string {
  switch (error.kind) {
    case "read_only":
      return "Blocked: this connection is read-only";
    case "not_connected":
      return "Not connected";
    case "timeout":
      return "The query timed out";
    case "invalid_input":
      return "Cannot run this";
    case "destructive_confirmation_required":
      return "Needs confirmation";
    case "driver":
      return "The database rejected the statement";
    default:
      return "Something went wrong";
  }
}

// WHAT:  A failed statement in the results area: what the database said, where in
//        the statement, and what to do about it.
// WHY:   "syntax error at or near FORM" is half an answer; the other half is the
//        line it is on, the server's hint, and a way to jump there or ask for a fix.
// HOW:   The line and column come from the position the database reported (see
//        `AppError::Driver`); only the Postgres, MySQL and ClickHouse families
//        report one. Hint and detail are the server's own follow-up lines.
// WHERE: src/features/editor/ResultsPane.tsx, src-tauri/src/integrations/error_detail.rs
export function QueryErrorPanel({ error, sql, onGoToError, onAskAi, onCopy }: QueryErrorPanelProps) {
  const position = error.kind === "driver" ? error.position : null;
  const hint = error.kind === "driver" ? error.hint : null;
  const detail = error.kind === "driver" ? error.detail : null;
  const where = position !== null && sql !== null ? lineColumnOf(sql, position) : null;
  const shown = where === null ? null : excerpt(where.text, where.column);
  const copyText = [error.message, where === null ? "" : `at line ${where.line}, column ${where.column}`, hint === null ? "" : `Hint: ${hint}`, detail ?? ""].filter((part) => part.length > 0).join("\n");

  return (
    <ScrollArea className="h-full">
      <div className="flex max-w-3xl flex-col gap-2 p-3" role="alert">
        <div className="flex items-start gap-2">
          <span className="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md bg-danger-soft text-danger">
            <Icon name="alert" size={14} />
          </span>
          <div className="min-w-0">
            <h3 className="text-xs font-semibold text-foreground">{titleOf(error)}</h3>
            <p className="selectable mt-1 whitespace-pre-wrap break-words text-xs text-danger">{error.message}</p>
            {where !== null ? (
              <p className="mt-1 text-[11px] text-muted">
                Line {where.line}, column {where.column}
              </p>
            ) : null}
          </div>
        </div>
        {shown !== null ? (
          <pre className="selectable overflow-x-auto rounded-lg border border-border/60 bg-surface-secondary/60 px-2.5 py-1.5 font-mono text-[11px] leading-5 text-foreground">
            {shown.shown}
            {"\n"}
            <span className="text-danger">{`${" ".repeat(Math.max(shown.caret, 0))}^`}</span>
          </pre>
        ) : null}
        {hint !== null ? (
          <p className="selectable rounded-lg bg-surface-secondary/60 px-2.5 py-1.5 text-xs text-foreground">
            <span className="font-semibold">Hint </span>
            {hint}
          </p>
        ) : null}
        {detail !== null ? <p className="selectable whitespace-pre-wrap text-xs text-muted">{detail}</p> : null}
        <div className="flex flex-wrap items-center gap-1.5">
          {position !== null && onGoToError !== undefined ? (
            <Button size="xs" variant="tertiary" onClick={onGoToError}>
              <Icon name="target" size={11} />
              Go to error
            </Button>
          ) : null}
          {onAskAi !== undefined ? (
            <Button size="xs" variant="soft" onClick={onAskAi}>
              <Icon name="sparkles" size={11} />
              Fix with AI
            </Button>
          ) : null}
          <Button size="xs" variant="ghost" onClick={() => { onCopy(copyText); }}>
            <Icon name="copy" size={11} />
            Copy error
          </Button>
        </div>
      </div>
    </ScrollArea>
  );
}
