// SOT: conversation, chat-thread, stick-to-bottom, conversation-empty-state
import { useCallback, useEffect, useRef, useState, type ComponentProps, type ReactNode } from "react";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { Icon } from "@/lib/icons";

// WHAT:  The shadcn AI Elements Conversation: the scrolling transcript of a chat.
// WHY:   A streaming answer grows while it is read. The thread must follow the
//        tail as tokens arrive, yet stop following the moment the reader scrolls
//        up to re-read something — and say so with a button back to the bottom.
// HOW:   `isAtBottom` is measured on scroll with a small threshold; a
//        ResizeObserver re-pins the view while content grows and the reader has
//        not taken over.
// WHERE: https://ai-sdk.dev/elements/components/conversation

interface ConversationContextValue {
  atBottom: boolean;
  scrollToBottom: (behavior?: ScrollBehavior) => void;
}

/// Shared by ref rather than context: the scroll button is always a sibling of
/// the viewport inside the same Conversation, and a context here would re-render
/// the whole transcript on every scroll event.
const noop: ConversationContextValue = { atBottom: true, scrollToBottom: () => undefined };

export interface ConversationProps extends ComponentProps<"div"> {
  /// Distance from the bottom, in pixels, still counted as "at the bottom".
  threshold?: number;
  children: ReactNode;
}

export function Conversation({ className, threshold = 32, children, ...props }: ConversationProps) {
  const viewport = useRef<HTMLDivElement | null>(null);
  const [atBottom, setAtBottom] = useState(true);
  // Read by the resize observer without making it depend on React state.
  const stick = useRef(true);

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "smooth") => {
    const node = viewport.current;
    if (!node) return;
    node.scrollTo({ top: node.scrollHeight, behavior });
  }, []);

  const onScroll = useCallback(() => {
    const node = viewport.current;
    if (!node) return;
    const bottom = node.scrollHeight - node.scrollTop - node.clientHeight <= threshold;
    stick.current = bottom;
    setAtBottom(bottom);
  }, [threshold]);

  useEffect(() => {
    const node = viewport.current;
    if (!node) return;
    const observer = new ResizeObserver(() => {
      if (stick.current) node.scrollTo({ top: node.scrollHeight });
    });
    for (const child of node.children) observer.observe(child);
    return () => { observer.disconnect(); };
  }, []);

  return (
    <div data-slot="conversation" className={cn("relative flex min-h-0 flex-1 flex-col", className)} {...props}>
      <div
        ref={viewport}
        onScroll={onScroll}
        data-slot="conversation-viewport"
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain [scrollbar-width:thin]"
      >
        {children}
      </div>
      <ConversationScrollButton atBottom={atBottom} onScrollToBottom={scrollToBottom} />
    </div>
  );
}

export function ConversationContent({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="conversation-content" className={cn("mx-auto flex w-full max-w-3xl flex-col gap-3 px-3 py-3 sm:px-4", className)} {...props} />;
}

export interface ConversationEmptyStateProps extends ComponentProps<"div"> {
  icon?: ReactNode;
  title: string;
  description?: ReactNode;
}

export function ConversationEmptyState({ className, icon, title, description, children, ...props }: ConversationEmptyStateProps) {
  return (
    <div data-slot="conversation-empty-state" className={cn("mx-auto flex max-w-xl flex-col items-center justify-center px-4 pt-10 text-center", className)} {...props}>
      {icon ? <div className="mb-3">{icon}</div> : null}
      <h2 className="text-sm font-semibold tracking-tight text-foreground">{title}</h2>
      {description ? <p className="mt-1.5 max-w-md text-xs leading-relaxed text-muted">{description}</p> : null}
      {children}
    </div>
  );
}

interface ConversationScrollButtonProps {
  atBottom: boolean;
  onScrollToBottom: (behavior?: ScrollBehavior) => void;
}

export function ConversationScrollButton({ atBottom, onScrollToBottom }: ConversationScrollButtonProps) {
  if (atBottom) return null;
  return (
    <Button
      variant="secondary"
      size="icon-sm"
      aria-label="Scroll to latest message"
      onClick={() => { onScrollToBottom(); }}
      className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full glass-pill shadow-lg"
    >
      <Icon name="arrow-down" />
    </Button>
  );
}

export { noop as conversationFallback };
