// SOT: lock-tip, locked-control-reason, schema-form-lock-tooltip
import type { ReactNode } from "react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/cn";

// WHAT:  Wraps a control the engine locks and says why in a tooltip.
// WHY:   The schema forms grey out what an engine cannot change (renaming a
//        column, dropping a key…). A disabled control gets no pointer events or
//        focus, so the reason sits on a focusable wrapper; a native `title` is
//        slow, unstyled and invisible to keyboard users (docs/ui-refresh.md).
// HOW:   Without a reason it is a plain wrapper, so the layout is the same
//        whether or not the control is locked.
// WHERE: src/features/schema/ddlRules.ts (columnLocks), forms/{ColumnGrid,ForeignKeyEditor}.tsx
export function LockTip({ reason, children, className }: { reason: string | null; children: ReactNode; className?: string | undefined }) {
  if (reason === null) return <span className={className}>{children}</span>;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span tabIndex={0} className={cn("rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring/50", className)}>
          {children}
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-72 leading-snug">{reason}</TooltipContent>
    </Tooltip>
  );
}
