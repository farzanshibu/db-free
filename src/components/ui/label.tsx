// SOT: label, field-label
import type * as React from "react";
import { Label as LabelPrimitive } from "radix-ui";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Label (Radix), which forwards clicks to its control and
//        dims with a disabled field group.
// WHERE: https://ui.shadcn.com/docs/components/label
export function Label({ className, ...props }: React.ComponentProps<typeof LabelPrimitive.Root>) {
  return (
    <LabelPrimitive.Root
      data-slot="label"
      className={cn(
        "flex select-none items-center gap-1.5 text-xs font-medium text-muted",
        "group-data-[disabled=true]/field:opacity-50 peer-disabled:cursor-not-allowed peer-disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}
