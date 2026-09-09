// SOT: textarea, multiline-input
import type * as React from "react";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Textarea. Monospace by default is deliberate: nearly
//        every multi-line field in this app holds a query, a document or a value.
// WHERE: https://ui.shadcn.com/docs/components/textarea
export function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "flex min-h-16 w-full rounded-[var(--field-radius)] border border-input bg-field px-2.5 py-2 font-mono text-[13px] text-field-foreground",
        "shadow-[var(--field-shadow)] outline-none transition-[color,box-shadow,border-color] field-sizing-content",
        "placeholder:font-sans placeholder:text-field-placeholder",
        "focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}
