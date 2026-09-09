// SOT: checkbox, row-selection-control
import type * as React from "react";
import { Checkbox as CheckboxPrimitive } from "radix-ui";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";

// WHAT:  The shadcn/ui Checkbox (Radix), including the indeterminate state the
//        grid's select-all header needs.
// WHERE: https://ui.shadcn.com/docs/components/checkbox
export function Checkbox({ className, ...props }: React.ComponentProps<typeof CheckboxPrimitive.Root>) {
  return (
    <CheckboxPrimitive.Root
      data-slot="checkbox"
      className={cn(
        "peer size-3.5 shrink-0 rounded-[4px] border border-input bg-field outline-none transition-shadow",
        "data-[state=checked]:border-accent data-[state=checked]:bg-accent data-[state=checked]:text-accent-foreground",
        "data-[state=indeterminate]:border-accent data-[state=indeterminate]:bg-accent data-[state=indeterminate]:text-accent-foreground",
        "focus-visible:ring-2 focus-visible:ring-ring/50",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator data-slot="checkbox-indicator" className="flex items-center justify-center text-current">
        {props.checked === "indeterminate" ? <Icon name="minus" size={11} /> : <Icon name="check" size={11} />}
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}
