// SOT: skeleton, loading-placeholder
import type * as React from "react";
import { cn } from "@/lib/cn";

// WHAT:  The shadcn/ui Skeleton: a pulsing block standing in for content whose
//        shape is already known (table lists, object trees).
// WHERE: https://ui.shadcn.com/docs/components/skeleton
export function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return <div data-slot="skeleton" className={cn("animate-pulse rounded-md bg-surface-secondary", className)} {...props} />;
}
