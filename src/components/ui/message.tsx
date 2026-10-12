// SOT: message, chat-bubble, message-avatar, message-content
import type { ComponentProps, ReactNode } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn AI Elements Message: one turn in a conversation, laid out by
//        who said it.
// WHY:   A user's turn is a short bubble on the right; an assistant's turn is
//        prose, tool traces and charts that need the full column. Reading who
//        said what should not depend on reading the text.
// WHERE: https://ai-sdk.dev/elements/components/message
export interface MessageProps extends ComponentProps<"div"> {
  from: "user" | "assistant" | "system";
}

export function Message({ className, from, ...props }: MessageProps) {
  return (
    <div
      data-slot="message"
      data-from={from}
      className={cn("flex w-full min-w-0 gap-1.5", from === "user" ? "justify-end" : "justify-start", className)}
      {...props}
    />
  );
}

const messageContentVariants = cva("min-w-0 text-xs leading-relaxed selectable", {
  variants: {
    variant: {
      /// A bubble: the user's own words, and short system notices.
      contained: "max-w-[85%] rounded-xl px-2.5 py-1.5 shadow-sm",
      /// No bubble: assistant output that carries its own blocks (code, charts).
      flat: "flex w-full flex-col",
    },
    from: {
      user: "rounded-tr-sm bg-accent text-accent-foreground",
      assistant: "text-foreground",
      system: "bg-surface-secondary text-muted",
    },
  },
  defaultVariants: { variant: "contained", from: "assistant" },
});

export interface MessageContentProps extends ComponentProps<"div">, VariantProps<typeof messageContentVariants> {}

export function MessageContent({ className, variant, from, ...props }: MessageContentProps) {
  return <div data-slot="message-content" className={cn(messageContentVariants({ variant, from }), className)} {...props} />;
}

export interface MessageAvatarProps extends ComponentProps<"div"> {
  /// Rendered inside the badge — an icon, or initials.
  children: ReactNode;
}

export function MessageAvatar({ className, children, ...props }: MessageAvatarProps) {
  return (
    <div
      data-slot="message-avatar"
      className={cn("flex size-5 shrink-0 items-center justify-center rounded-md bg-accent/15 text-accent", className)}
      {...props}
    >
      {children}
    </div>
  );
}
