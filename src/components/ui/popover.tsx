// SOT: popover, floating-panel
import type * as React from "react";
import { Popover as PopoverPrimitive } from "radix-ui";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Popover (Radix): a non-modal floating panel anchored to
//        its trigger — column filters, the run-options panel, detail peeks.
// WHERE: https://ui.shadcn.com/docs/components/popover
export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;
export const PopoverAnchor = PopoverPrimitive.Anchor;

export function PopoverContent({ className, align = "start", sideOffset = 6, ...props }: React.ComponentProps<typeof PopoverPrimitive.Content>) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        data-slot="popover-content"
        align={align}
        sideOffset={sideOffset}
        className={cn(
          "z-50 w-72 origin-(--radix-popover-content-transform-origin) rounded-xl glass-modal p-3 text-popover-foreground outline-none",
          "data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
          "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
          "data-[side=bottom]:slide-in-from-top-1 data-[side=top]:slide-in-from-bottom-1",
          "data-[side=left]:slide-in-from-right-1 data-[side=right]:slide-in-from-left-1",
          className,
        )}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
}

export function PopoverHeading({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="popover-heading" className={cn("mb-2 text-xs font-semibold text-foreground", className)} {...props} />;
}
