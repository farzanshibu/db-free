// SOT: tooltip, hover-hint
import type * as React from "react";
import { Tooltip as TooltipPrimitive } from "radix-ui";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Tooltip (Radix). The provider is mounted once in App.tsx
//        so every tooltip shares one delay and the "skip delay" grace period.
// WHERE: https://ui.shadcn.com/docs/components/tooltip
export const Tooltip = TooltipPrimitive.Root;
export const TooltipTrigger = TooltipPrimitive.Trigger;

export function TooltipProvider({ delayDuration = 400, skipDelayDuration = 300, ...props }: React.ComponentProps<typeof TooltipPrimitive.Provider>) {
  return <TooltipPrimitive.Provider delayDuration={delayDuration} skipDelayDuration={skipDelayDuration} {...props} />;
}

export function TooltipContent({ className, sideOffset = 6, children, ...props }: React.ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        data-slot="tooltip-content"
        sideOffset={sideOffset}
        className={cn(
          "z-50 w-fit max-w-64 origin-(--radix-tooltip-content-transform-origin) rounded-md",
          "bg-surface-tertiary px-2 py-1 text-xs text-foreground shadow-md backdrop-blur-md",
          "data-[state=delayed-open]:animate-in data-[state=delayed-open]:fade-in-0 data-[state=delayed-open]:zoom-in-95",
          "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
          className,
        )}
        {...props}
      >
        {children}
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  );
}
