// SOT: prompt-input, chat-composer, prompt-submit
import type { ComponentProps } from "react";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { Icon } from "@/lib/icons";

// WHAT:  The shadcn AI Elements PromptInput: the composer at the foot of a chat.
// WHY:   Submitting has four states — ready, submitted, streaming, error — and
//        the same button has to serve all of them, including "stop" while the
//        answer streams. Enter sends, Shift+Enter is a newline.
// WHERE: https://ai-sdk.dev/elements/components/prompt-input
export function PromptInput({ className, ...props }: ComponentProps<"form">) {
  return (
    <form
      data-slot="prompt-input"
      className={cn("mx-auto flex w-full max-w-3xl items-end gap-2", className)}
      {...props}
    />
  );
}

export function PromptInputBody({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="prompt-input-body" className={cn("flex min-w-0 flex-1 flex-col gap-1.5", className)} {...props} />;
}

export interface PromptInputTextareaProps extends ComponentProps<typeof Textarea> {
  /// Enter submits the enclosing form; Shift+Enter inserts a newline.
  submitOnEnter?: boolean;
}

export function PromptInputTextarea({ className, submitOnEnter = true, onKeyDown, ...props }: PromptInputTextareaProps) {
  return (
    <Textarea
      data-slot="prompt-input-textarea"
      rows={1}
      className={cn("max-h-40 min-h-9 w-full resize-none font-sans", className)}
      onKeyDown={(event) => {
        onKeyDown?.(event);
        if (submitOnEnter && event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
          event.preventDefault();
          event.currentTarget.form?.requestSubmit();
        }
      }}
      {...props}
    />
  );
}

export function PromptInputToolbar({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="prompt-input-toolbar" className={cn("flex items-center gap-1.5", className)} {...props} />;
}

export function PromptInputTools({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="prompt-input-tools" className={cn("flex items-center gap-1", className)} {...props} />;
}

export type PromptInputStatus = "ready" | "submitted" | "streaming" | "error";

export interface PromptInputSubmitProps extends Omit<ComponentProps<typeof Button>, "children"> {
  status?: PromptInputStatus;
  /// Called instead of submitting while the answer is streaming.
  onStop?: (() => void) | undefined;
  label?: string;
}

export function PromptInputSubmit({ className, status = "ready", onStop, label = "Send", disabled, ...props }: PromptInputSubmitProps) {
  const busy = status === "submitted" || status === "streaming";
  if (busy && onStop) {
    return (
      <Button
        type="button"
        variant="secondary"
        size="sm"
        aria-label="Stop generating"
        onClick={onStop}
        className={cn("h-8 shrink-0 rounded-lg px-3 glass-pill text-danger", className)}
      >
        <span aria-hidden className="size-2.5 rounded-xs bg-current" />
        <span className="hidden sm:inline">Stop</span>
      </Button>
    );
  }
  return (
    <Button
      type="submit"
      variant="primary"
      size="sm"
      aria-label={label}
      disabled={disabled === true || busy}
      className={cn("h-8 shrink-0 rounded-lg px-3 font-semibold shadow-sm shadow-accent/30 glass-pill liquid-hover", className)}
      {...props}
    >
      {busy ? <Spinner size="sm" className="text-accent-foreground" /> : <Icon name="send" className="size-3" />}
      <span className="hidden sm:inline">{label}</span>
    </Button>
  );
}
