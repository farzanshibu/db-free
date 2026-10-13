// SOT: chat-markdown, streaming-markdown, assistant-code-block, markdown-renderer, inline-code-run
import { type ReactNode, createContext, memo, useContext, useMemo, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import type { Engine, QueryOutcome } from "@/lib/bindings";
import { errorMessage, ipc, normalizeError } from "@/lib/ipc";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Alert, AlertContent, AlertDescription, AlertIndicator, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";
import { ResultTable, ResultToolbar } from "./InlineResult";
import { firstRows } from "./resultExport";

// WHAT:  Renders assistant text as markdown, including while it is still being
//        written, and turns fenced code into a card the user can act on — copy it,
//        open it in the editor, or run it right there and see the rows.
// WHY:   The assistant answers in markdown — tables, headings, fenced SQL — and
//        the chat used to print it as raw text in a <p>. A half-received fence
//        has to render as a code block the moment it opens, not when it closes,
//        or the message visibly rewrites itself as the stream lands.
// HOW:   remark-gfm for tables and strikethrough. `pre` is unwrapped so the code
//        card is not nested inside a <pre> (invalid HTML); `code` then decides
//        inline vs block by looking for a language class or a newline. What the
//        card needs (handlers, the connection to run against) arrives through
//        context, so the markdown `components` map is built once: a new function
//        identity per render would remount every card and wipe its result.
//        Running goes through `execute_query`, the same guarded path as the
//        editor's Run: a read-only connection refuses writes, and a destructive
//        statement asks first.
// WHERE: src/features/chat/ChatTab.tsx, src/features/editor/QueryPane.tsx

/// What a code card runs against: set it and the card grows a Run button.
export interface CodeRunner {
  connectionId: string;
  engine: Engine;
  /// Engines that speak JSON (Mongo, search…) run ```json blocks; SQL engines do not.
  allowJson: boolean;
}

interface MarkdownProps {
  text: string;
  /// Label for the code card's chip: "SQL", "Cypher", "Redis command"…
  language: string;
  /// Hands the code to the parent to run (legacy). Ignored when `runner` is set.
  onRun?: (code: string) => void;
  onOpen?: (code: string) => void;
  onCopy?: (code: string) => void;
  /// Run blocks inline against this connection and show the rows under them.
  runner?: CodeRunner;
  /// Dims the trailing caret while a run is streaming, and holds Run back.
  streaming?: boolean;
}

interface CodeContextValue {
  language: string;
  onRun: ((code: string) => void) | undefined;
  onOpen: ((code: string) => void) | undefined;
  onCopy: ((code: string) => void) | undefined;
  runner: CodeRunner | undefined;
  streaming: boolean;
}

const CodeContext = createContext<CodeContextValue>({
  language: "",
  onRun: undefined,
  onOpen: undefined,
  onCopy: undefined,
  runner: undefined,
  streaming: false,
});

/// Rows an inline run returns: a glance in a chat bubble, not an export.
const INLINE_ROWS = 500;

/// Tags the assistant puts on a statement the connection can run.
const RUNNABLE_TAGS = new Set([
  "sql", "pgsql", "postgres", "postgresql", "plpgsql", "mysql", "mariadb", "sqlite", "tsql", "mssql", "plsql", "oracle",
  "redis", "mongo", "mongodb", "javascript", "js", "shell", "cypher", "aql", "cql", "sparql", "promql", "influxql", "flux",
  "partiql", "surrealql", "xquery", "gsql", "kql", "mql", "command", "query", "text",
]);

// WHAT:  Whether a fenced block is something to run.
// WHY:   Untagged blocks are statements more often than not; a tagged block is
//        runnable only for a language a database speaks, so a ```python or
//        ```yaml block never gets a Run button.
export function isRunnable(tag: string | undefined, allowJson: boolean): boolean {
  if (tag === undefined || tag === "") return true;
  const lower = tag.toLowerCase();
  return RUNNABLE_TAGS.has(lower) || (allowJson && lower === "json");
}

function SummaryLine({ outcome }: { outcome: QueryOutcome }) {
  const affected = outcome.statements.reduce((total, s) => (s.kind === "affected" ? total + s.rowsAffected : total), 0);
  const hasRows = firstRows(outcome) !== null;
  if (hasRows) return null;
  return (
    <p className="px-2 py-1.5 text-[11px] text-muted">
      {affected > 0 ? `${affected} ${affected === 1 ? "row" : "rows"} affected.` : "Done. The statement returned nothing."}
    </p>
  );
}

// WHAT:  A fenced block, with the actions that make it useful here.
function CodeCard({ code, label, tag }: { code: string; label: string; tag: string | undefined }) {
  const { onRun, onOpen, onCopy, runner, streaming } = useContext(CodeContext);
  const [outcome, setOutcome] = useState<QueryOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [confirm, setConfirm] = useState<string[] | null>(null);

  const runnable = runner !== undefined && isRunnable(tag, runner.allowJson);

  const run = async (confirmed: boolean) => {
    if (runner === undefined) return;
    setRunning(true);
    setError(null);
    setConfirm(null);
    try {
      setOutcome(
        await ipc("execute_query", {
          connectionId: runner.connectionId,
          sql: code,
          confirmDestructive: confirmed,
          maxRows: INLINE_ROWS,
          schema: null,
        }),
      );
    } catch (raw) {
      const failure = normalizeError(raw);
      if (failure.kind === "destructive_confirmation_required") {
        setConfirm(failure.statements);
      } else {
        setOutcome(null);
        setError(errorMessage(failure));
      }
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="my-2 overflow-hidden rounded-lg border border-border/60 bg-surface-secondary/70">
      <div className="flex items-center justify-between gap-2 border-b border-border/40 px-2 py-1">
        <span className="font-mono text-[10px] font-semibold uppercase tracking-wider text-muted">{label}</span>
        <div className="flex items-center gap-1">
          <Button
            size="sm"
            variant="ghost"
            className="h-5 rounded px-1.5 text-[10.5px] text-muted liquid-hover hover:text-foreground"
            onClick={() => onCopy?.(code)}
          >
            <Icon name="copy" size={10} />
            Copy
          </Button>
          {onOpen ? (
            <Button
              size="sm"
              variant="ghost"
              className="h-5 rounded px-1.5 text-[10.5px] text-muted liquid-hover hover:text-foreground"
              onClick={() => onOpen(code)}
            >
              Open in Studio
            </Button>
          ) : null}
          {runnable ? (
            <Button
              size="sm"
              variant="secondary"
              className="glass-pill h-5 rounded px-2 text-[10.5px] font-medium text-accent liquid-hover"
              disabled={running || streaming}
              onClick={() => void run(false)}
            >
              {running ? <Spinner size="sm" /> : <Icon name="play" size={9} />}
              Run
            </Button>
          ) : runner === undefined && onRun ? (
            <Button
              size="sm"
              variant="secondary"
              className="glass-pill h-5 rounded px-2 text-[10.5px] font-medium text-accent liquid-hover"
              onClick={() => onRun(code)}
            >
              <Icon name="play" size={9} />
              Run
            </Button>
          ) : null}
        </div>
      </div>
      <ScrollArea orientation="both" className="max-h-64">
        <pre className="selectable p-2 font-mono text-[11px] leading-relaxed text-foreground">
          {code}
        </pre>
      </ScrollArea>

      {error !== null ? (
        <div className="border-t border-danger/30 bg-danger/10 px-2 py-1 text-[11px] text-danger">
          <p className="selectable whitespace-pre-wrap">{error}</p>
        </div>
      ) : null}

      {outcome !== null && runner !== undefined ? (
        <div className="border-t border-border/40 bg-surface/60">
          <SummaryLine outcome={outcome} />
          {firstRows(outcome) !== null ? <ResultTable outcome={outcome} /> : null}
          <ResultToolbar outcome={outcome} engine={runner.engine} sql={code} />
          <div className="flex justify-end border-t border-border/30 px-2 py-0.5">
            <Button
              size="sm"
              variant="ghost"
              className="h-5 rounded px-1.5 text-[10.5px] text-muted hover:text-foreground"
              onClick={() => { setOutcome(null); }}
            >
              <Icon name="x" size={9} />
              Hide result
            </Button>
          </div>
        </div>
      ) : null}

      <Dialog open={confirm !== null} onOpenChange={(open) => { if (!open) setConfirm(null); }}>
        <DialogContent className="sm:max-w-[520px]">
          <DialogHeader>
            <DialogTitle>Run destructive statements?</DialogTitle>
          </DialogHeader>
          <DialogBody className="space-y-2">
            <Alert variant="danger" className="rounded-lg">
              <AlertIndicator />
              <AlertContent>
                <AlertTitle className="text-xs font-semibold">This changes or removes data</AlertTitle>
                <AlertDescription className="text-xs">
                  The assistant wrote these. Read them before you run them.
                </AlertDescription>
              </AlertContent>
            </Alert>
            <ul className="flex flex-col gap-1.5">
              {confirm?.map((statement) => (
                <li key={statement} className="selectable rounded-md bg-danger-soft px-2 py-1 font-mono text-[11px] text-danger">
                  {statement}
                </li>
              ))}
            </ul>
          </DialogBody>
          <DialogFooter>
            <Button variant="tertiary" onClick={() => { setConfirm(null); }}>
              Cancel
            </Button>
            <Button variant="danger" onClick={() => void run(true)}>
              Run anyway
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

// WHAT:  The text inside a node, without risking "[object Object]".
// WHY:   A fenced block's children are usually one string, but remark can hand
//        back an array or an element; String() on those stringifies the object.
function textOf(node: ReactNode): string {
  if (typeof node === "string") return node;
  if (typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return "";
}

/// The `code` element: inline code, or a card for a fenced block. A module-level
/// component so its identity never changes between renders.
function MarkdownCode({ className, children }: { className?: string | undefined; children?: ReactNode }) {
  const { language } = useContext(CodeContext);
  const text = textOf(children).replace(/\n$/, "");
  const fenced = /language-([\w-]+)/.exec(className ?? "");
  if (fenced === null && !text.includes("\n")) {
    return (
      <code className="selectable rounded bg-surface-tertiary/70 px-1 py-0.5 font-mono text-[11px] text-foreground">
        {children}
      </code>
    );
  }
  const tag = fenced?.[1];
  return <CodeCard code={text} label={tag !== undefined && tag.length > 0 ? tag : language} tag={tag} />;
}

const COMPONENTS: Components = {
  // Unwrapped: the card replaces the <pre> entirely.
  pre: ({ children }) => <>{children}</>,
  code: ({ className, children }) => <MarkdownCode className={className}>{children}</MarkdownCode>,
  p: ({ children }) => <p className="my-1.5 leading-relaxed first:mt-0 last:mb-0">{children}</p>,
  h1: ({ children }) => <h1 className="mt-3 mb-1.5 text-[13px] font-semibold text-foreground">{children}</h1>,
  h2: ({ children }) => <h2 className="mt-3 mb-1.5 text-xs font-semibold text-foreground">{children}</h2>,
  h3: ({ children }) => (
    <h3 className="mt-2.5 mb-1 text-xs font-semibold text-muted uppercase tracking-wide">{children}</h3>
  ),
  ul: ({ children }) => <ul className="my-1.5 ml-4 list-disc space-y-0.5 marker:text-muted">{children}</ul>,
  ol: ({ children }) => <ol className="my-1.5 ml-4 list-decimal space-y-0.5 marker:text-muted">{children}</ol>,
  li: ({ children }) => <li className="leading-relaxed">{children}</li>,
  strong: ({ children }) => <strong className="font-semibold text-foreground">{children}</strong>,
  em: ({ children }) => <em className="italic">{children}</em>,
  blockquote: ({ children }) => (
    <blockquote className="my-2 border-l-2 border-accent/40 pl-2 text-muted">{children}</blockquote>
  ),
  hr: () => <hr className="my-3 border-border/50" />,
  a: ({ children, href }) => (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="text-accent underline decoration-accent/40 underline-offset-2"
    >
      {children}
    </a>
  ),
  // Result tables can be wide; the page must never scroll sideways.
  table: ({ children }) => (
    <div className="my-2 overflow-hidden rounded-lg border border-border/50">
      <ScrollArea orientation="horizontal">
        <table className="w-full border-collapse text-[11px]">{children}</table>
      </ScrollArea>
    </div>
  ),
  thead: ({ children }) => <thead className="bg-surface-secondary/70">{children}</thead>,
  th: ({ children }) => (
    <th className="border-b border-border/50 px-2 py-1 text-left font-semibold text-muted">{children}</th>
  ),
  td: ({ children }) => (
    <td className="selectable border-b border-border/30 px-2 py-1 align-top tabular-nums">{children}</td>
  ),
};

function MarkdownBody({ text, language, onRun, onOpen, onCopy, runner, streaming = false }: MarkdownProps) {
  const context = useMemo<CodeContextValue>(
    () => ({ language, onRun, onOpen, onCopy, runner, streaming }),
    [language, onCopy, onOpen, onRun, runner, streaming],
  );

  return (
    <CodeContext.Provider value={context}>
      <div className={cn("selectable text-xs text-foreground", streaming && "[&>*:last-child]:after:content-['▌']")}>
        <ReactMarkdown remarkPlugins={[remarkGfm]} components={COMPONENTS}>
          {text}
        </ReactMarkdown>
      </div>
    </CodeContext.Provider>
  );
}

// Streaming appends a token at a time; memoising keeps React from rebuilding
// the whole tree for every other message in the thread on each frame.
export const Markdown = memo(MarkdownBody);
