// SOT: disclosure, advanced-section, more-options, progressive-disclosure
import { useState, type ReactNode } from "react";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { InfoTip } from "./InfoTip";

interface DisclosureProps {
  /// Header text. "Advanced" for rarely-changed settings, "More" for extra
  /// fields of a common task.
  label?: string;
  /// Help for the whole group, behind an info icon beside the header.
  tip?: ReactNode;
  /// How many controls sit inside. Shown while closed, so a user knows there is
  /// something there without opening it.
  count?: number | undefined;
  defaultOpen?: boolean;
  /// Controlled mode: pass both, e.g. to force the group open when one of its
  /// fields has a validation error.
  open?: boolean | undefined;
  onOpenChange?: ((open: boolean) => void) | undefined;
  /// `section`: a full-width header row (forms, settings, side panels).
  /// `inline`: a small text toggle that sits under the fields it extends.
  variant?: "section" | "inline";
  children: ReactNode;
  className?: string | undefined;
}

// WHAT:  A collapsible "Advanced" / "More" group, closed by default.
// WHY:   Progressive disclosure: a view shows what most people change most of
//        the time; the rest stays one click away and is never removed. The
//        count keeps hidden options discoverable.
// HOW:   Radix Collapsible (vendored shadcn). Uncontrolled unless `open` and
//        `onOpenChange` are both given. The trigger is a real button, so it is
//        reachable by Tab and toggled by Space / Enter.
// WHERE: docs/ui-refresh.md ("When to collapse"), src/components/ui/collapsible.tsx
export function Disclosure({ label = "Advanced", tip, count, defaultOpen = false, open, onOpenChange, variant = "section", children, className }: DisclosureProps) {
  const [own, setOwn] = useState(defaultOpen);
  const controlled = open !== undefined && onOpenChange !== undefined;
  const isOpen = controlled ? open : own;
  const setOpen = controlled ? onOpenChange : setOwn;

  return (
    <Collapsible open={isOpen} onOpenChange={setOpen} className={cn("flex min-w-0 flex-col", className)}>
      <div className={cn("flex items-center gap-1", variant === "section" ? "border-b border-border/40 pb-1" : "")}>
        <CollapsibleTrigger asChild>
          <button
            type="button"
            className={cn(
              "group/disclosure flex min-w-0 items-center gap-1 rounded-sm text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
              variant === "section"
                ? "h-6 flex-1 text-[11px] font-semibold uppercase tracking-wide text-muted hover:text-foreground"
                : "h-5 text-xs text-muted hover:text-foreground",
            )}
          >
            <Icon name="chevron-right" size={12} className={cn("shrink-0 transition-transform duration-150", isOpen ? "rotate-90" : "")} />
            <span className="truncate">{label}</span>
            {!isOpen && count !== undefined && count > 0 ? (
              <span className="ml-1 rounded-sm bg-surface-secondary px-1 text-[10px] font-medium tabular-nums text-muted normal-case tracking-normal">{count}</span>
            ) : null}
          </button>
        </CollapsibleTrigger>
        {tip !== undefined ? <InfoTip label={`About ${label}`}>{tip}</InfoTip> : null}
      </div>
      <CollapsibleContent className={cn("flex min-w-0 flex-col gap-2", variant === "section" ? "pt-2" : "pt-1.5")}>{children}</CollapsibleContent>
    </Collapsible>
  );
}
