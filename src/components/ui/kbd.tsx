// SOT: kbd, keyboard-shortcut-display
import type * as React from "react";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Kbd: one key cap. `KbdGroup` sets a chord.
// WHERE: https://ui.shadcn.com/docs/components/kbd
export function Kbd({ className, ...props }: React.ComponentProps<"kbd">) {
  return (
    <kbd
      data-slot="kbd"
      className={cn(
        "inline-flex h-4 w-fit min-w-4 select-none items-center justify-center gap-1 rounded-sm",
        "bg-surface-tertiary px-1 font-sans text-[10px] font-medium text-muted",
        className,
      )}
      {...props}
    />
  );
}

export function KbdGroup({ className, ...props }: React.ComponentProps<"span">) {
  return <span data-slot="kbd-group" className={cn("inline-flex items-center gap-0.5", className)} {...props} />;
}
