// SOT: switch, toggle-control
import type * as React from "react";
import { Switch as SwitchPrimitive } from "radix-ui";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Switch (Radix): an immediate on/off setting.
// WHERE: https://ui.shadcn.com/docs/components/switch
export function Switch({ className, ...props }: React.ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn(
        "peer inline-flex h-4.5 w-8 shrink-0 items-center rounded-full border border-transparent outline-none transition-colors",
        "data-[state=checked]:bg-accent data-[state=unchecked]:bg-surface-tertiary",
        "focus-visible:ring-2 focus-visible:ring-ring/60",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className={cn(
          "pointer-events-none block size-3.5 rounded-full bg-foreground shadow-sm ring-0 transition-transform",
          "data-[state=checked]:translate-x-[15px] data-[state=unchecked]:translate-x-0.5",
        )}
      />
    </SwitchPrimitive.Root>
  );
}
