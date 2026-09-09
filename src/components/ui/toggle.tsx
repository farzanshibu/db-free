// SOT: toggle-button, pressed-state-button
import type * as React from "react";
import { Toggle as TogglePrimitive } from "radix-ui";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Toggle (Radix): a button that stays pressed — word wrap,
//        pretty-print, pin.
// WHERE: https://ui.shadcn.com/docs/components/toggle
export function Toggle({ className, ...props }: React.ComponentProps<typeof TogglePrimitive.Root>) {
  return (
    <TogglePrimitive.Root
      data-slot="toggle"
      className={cn(
        "inline-flex h-7 min-w-7 shrink-0 items-center justify-center gap-1.5 rounded-md px-2 text-xs font-medium outline-none transition-colors",
        "text-muted hover:bg-surface-secondary hover:text-foreground",
        "data-[state=on]:bg-accent/15 data-[state=on]:text-accent",
        "focus-visible:ring-2 focus-visible:ring-ring/50",
        "disabled:pointer-events-none disabled:opacity-45",
        "[&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5",
        className,
      )}
      {...props}
    />
  );
}
