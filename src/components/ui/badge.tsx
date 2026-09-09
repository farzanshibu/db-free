// SOT: badge, chip, pill-label
import type * as React from "react";
import { Slot } from "radix-ui";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Badge — a non-interactive status pill.
// WHY:   Row counts, engine names, environment labels, object kinds and error
//        codes all read as one shape so the eye learns it once.
// WHERE: https://ui.shadcn.com/docs/components/badge
const badgeVariants = cva(
  [
    "inline-flex w-fit shrink-0 items-center justify-center gap-1 whitespace-nowrap",
    "rounded-md border px-1.5 py-0.5 text-[11px] font-medium",
    "[&_svg]:pointer-events-none [&_svg]:size-3",
  ],
  {
    variants: {
      variant: {
        default: "border-transparent bg-accent text-accent-foreground",
        secondary: "border-border bg-surface-secondary text-foreground",
        soft: "border-transparent bg-accent/15 text-accent",
        outline: "border-border bg-transparent text-muted",
        success: "border-transparent bg-success/15 text-success",
        warning: "border-transparent bg-warning/15 text-warning",
        danger: "border-transparent bg-danger/15 text-danger",
      },
      size: {
        sm: "h-4 px-1 text-[10px]",
        md: "h-5",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  },
);

export interface BadgeProps extends React.ComponentProps<"span">, VariantProps<typeof badgeVariants> {
  asChild?: boolean;
}

export function Badge({ className, variant, size, asChild = false, ...props }: BadgeProps) {
  const Comp = asChild ? Slot.Root : "span";
  return <Comp data-slot="badge" className={cn(badgeVariants({ variant, size }), className)} {...props} />;
}

export { badgeVariants };
