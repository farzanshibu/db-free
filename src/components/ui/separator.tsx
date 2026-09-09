// SOT: separator, divider-rule
import type * as React from "react";
import { Separator as SeparatorPrimitive } from "radix-ui";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Separator (Radix), decorative by default.
// WHERE: https://ui.shadcn.com/docs/components/separator
export function Separator({ className, orientation = "horizontal", decorative = true, ...props }: React.ComponentProps<typeof SeparatorPrimitive.Root>) {
  return (
    <SeparatorPrimitive.Root
      data-slot="separator"
      decorative={decorative}
      orientation={orientation}
      className={cn("shrink-0 bg-separator", orientation === "horizontal" ? "h-px w-full" : "h-full w-px", className)}
      {...props}
    />
  );
}
