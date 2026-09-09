// SOT: tabs, segmented-control-base
import type * as React from "react";
import { Tabs as TabsPrimitive } from "radix-ui";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Tabs (Radix), in two shapes: `segmented` — a filled
//        track holding pill triggers, used as a segmented control — and
//        `underline`, for a real tabbed page.
// WHERE: https://ui.shadcn.com/docs/components/tabs
export function Tabs({ className, ...props }: React.ComponentProps<typeof TabsPrimitive.Root>) {
  return <TabsPrimitive.Root data-slot="tabs" className={cn("flex flex-col gap-2", className)} {...props} />;
}

export interface TabsListProps extends React.ComponentProps<typeof TabsPrimitive.List> {
  variant?: "segmented" | "underline";
}

export function TabsList({ className, variant = "segmented", ...props }: TabsListProps) {
  return (
    <TabsPrimitive.List
      data-slot="tabs-list"
      data-variant={variant}
      className={cn(
        "inline-flex w-fit items-center",
        variant === "segmented" ? "h-7 gap-0.5 rounded-lg bg-segment p-0.5" : "h-8 gap-3 border-b border-border",
        className,
      )}
      {...props}
    />
  );
}

export interface TabsTriggerProps extends React.ComponentProps<typeof TabsPrimitive.Trigger> {
  variant?: "segmented" | "underline";
}

export function TabsTrigger({ className, variant = "segmented", ...props }: TabsTriggerProps) {
  return (
    <TabsPrimitive.Trigger
      data-slot="tabs-trigger"
      className={cn(
        "inline-flex flex-1 items-center justify-center gap-1.5 whitespace-nowrap text-xs font-medium outline-none transition-colors",
        "disabled:pointer-events-none disabled:opacity-45",
        "focus-visible:ring-2 focus-visible:ring-ring/50",
        "[&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5",
        variant === "segmented"
          ? "h-6 rounded-md px-2.5 text-muted hover:text-foreground data-[state=active]:bg-surface-tertiary data-[state=active]:text-foreground"
          : "h-8 border-b-2 border-transparent px-0.5 text-muted hover:text-foreground data-[state=active]:border-accent data-[state=active]:text-foreground",
        className,
      )}
      {...props}
    />
  );
}

export function TabsContent({ className, ...props }: React.ComponentProps<typeof TabsPrimitive.Content>) {
  return <TabsPrimitive.Content data-slot="tabs-content" className={cn("min-h-0 flex-1 outline-none", className)} {...props} />;
}
