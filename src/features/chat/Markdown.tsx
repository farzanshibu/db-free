// SOT: chat-markdown, streaming-markdown, assistant-code-block, markdown-renderer
import { type ReactNode, memo, useMemo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";

// WHAT:  Renders assistant text as markdown, including while it is still being
//        written, and turns fenced code into a card the user can act on.
// WHY:   The assistant answers in markdown — tables, headings, fenced SQL — and
//        the chat used to print it as raw text in a <p>. A half-received fence
//        has to render as a code block the moment it opens, not when it closes,
//        or the message visibly rewrites itself as the stream lands.
// HOW:   remark-gfm for tables and strikethrough. `pre` is unwrapped so the code
//        card is not nested inside a <pre> (invalid HTML); `code` then decides
//        inline vs block by looking for a language class or a newline.
// WHERE: src/features/chat/ChatTab.tsx, src/features/editor/QueryPane.tsx

interface MarkdownProps {
  text: string;
  /// Label for the code card's chip: "SQL", "Cypher", "Redis command"…
  language: string;
  onRun?: (code: string) => void;
  onOpen?: (code: string) => void;
  onCopy?: (code: string) => void;
  /// Dims the trailing caret while a run is streaming.
  streaming?: boolean;
}

// WHAT:  A fenced block, with the actions that make it useful here.
function CodeCard({
  code,
  label,
  onRun,
  onOpen,
  onCopy,
}: {
  code: string;
  label: string;
  onRun?: ((code: string) => void) | undefined;
  onOpen?: ((code: string) => void) | undefined;
  onCopy?: ((code: string) => void) | undefined;
}) {
  return (
    <div className="my-2 overflow-hidden rounded-xl border border-border/60 bg-surface-secondary/70">
      <div className="flex items-center justify-between gap-2 border-b border-border/40 px-2.5 py-1.5">
        <span className="font-mono text-[10px] font-semibold uppercase tracking-wider text-muted">
          {label}
        </span>
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
          {onRun ? (
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
      <ScrollArea className="max-h-64">
        <pre className="selectable overflow-x-auto p-2.5 font-mono text-[11px] leading-relaxed text-foreground">
          {code}
        </pre>
      </ScrollArea>
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

function MarkdownBody({ text, language, onRun, onOpen, onCopy, streaming = false }: MarkdownProps) {
  // Rebuilt only when a handler identity changes, not on every streamed token.
  const components = useMemo<Components>(
    () => ({
      // Unwrapped: the card below replaces the <pre> entirely.
      pre: ({ children }) => <>{children}</>,
      code: ({ className, children }) => {
        const text = textOf(children).replace(/\n$/, "");
        const fenced = /language-(\w+)/.exec(className ?? "");
        if (fenced === null && !text.includes("\n")) {
          return (
            <code className="selectable rounded bg-surface-tertiary/70 px-1 py-0.5 font-mono text-[11px] text-foreground">
              {children}
            </code>
          );
        }
        const tag = fenced?.[1];
        return (
          <CodeCard
            code={text}
            label={tag !== undefined && tag.length > 0 ? tag : language}
            onRun={onRun}
            onOpen={onOpen}
            onCopy={onCopy}
          />
        );
      },
      p: ({ children }) => <p className="my-1.5 leading-relaxed first:mt-0 last:mb-0">{children}</p>,
      h1: ({ children }) => <h1 className="mt-3 mb-1.5 text-sm font-semibold text-foreground">{children}</h1>,
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
        <blockquote className="my-2 border-l-2 border-accent/40 pl-2.5 text-muted">{children}</blockquote>
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
        <div className="my-2 overflow-x-auto rounded-lg border border-border/50">
          <table className="w-full border-collapse text-[11px]">{children}</table>
        </div>
      ),
      thead: ({ children }) => <thead className="bg-surface-secondary/70">{children}</thead>,
      th: ({ children }) => (
        <th className="border-b border-border/50 px-2 py-1 text-left font-semibold text-muted">{children}</th>
      ),
      td: ({ children }) => (
        <td className="selectable border-b border-border/30 px-2 py-1 align-top tabular-nums">{children}</td>
      ),
    }),
    [language, onCopy, onOpen, onRun],
  );

  return (
    <div className={cn("selectable text-xs text-foreground", streaming && "[&>*:last-child]:after:content-['▌']")}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
}

// Streaming appends a token at a time; memoising keeps React from rebuilding
// the whole tree for every other message in the thread on each frame.
export const Markdown = memo(MarkdownBody);
