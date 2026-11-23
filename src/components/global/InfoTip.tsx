// SOT: info-tip, help-tooltip, help-text, inline-help
import type { ReactNode } from "react";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

interface InfoTipProps {
  /// The help text. Keep it to one or two sentences; anything longer is a doc.
  children: ReactNode;
  /// Accessible name of the icon. Defaults to "More info"; name the subject
  /// ("About read-only mode") when several tips share a view.
  label?: string | undefined;
  side?: "top" | "right" | "bottom" | "left";
  align?: "start" | "center" | "end";
  className?: string | undefined;
}

// WHAT:  A small info glyph that shows help text in a tooltip.
// WHY:   The UI refresh moves explanations off the screen and behind a hover:
//        a view keeps its labels and controls, and the "why" sits one pointer
//        move away. A real <button> (not a bare icon) so the tip is reachable by
//        keyboard focus as well as by hover — help that only a mouse can open is
//        help a keyboard user never gets.
// HOW:   Radix Tooltip with an explicit trigger; the provider is mounted once
//        in App.tsx. `type="button"` keeps it from submitting a surrounding form.
// WHERE: docs/ui-refresh.md ("Tooltip vs inline text"), src/components/ui/tooltip.tsx
export function InfoTip({ children, label = "More info", side = "top", align = "center", className }: InfoTipProps) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          onClick={(event) => {
            // Inside a label, a row button or a collapsible header the click
            // belongs to the tip, not to the control around it.
            event.preventDefault();
            event.stopPropagation();
          }}
          className={cn(
            "inline-flex size-3.5 shrink-0 cursor-help items-center justify-center rounded-full align-middle text-muted/70 outline-none transition-colors",
            "hover:text-foreground focus-visible:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50",
            className,
          )}
        >
          <Icon name="info" size={12} />
        </button>
      </TooltipTrigger>
      <TooltipContent side={side} align={align} className="max-w-72 leading-snug">
        {children}
      </TooltipContent>
    </Tooltip>
  );
}

// WHAT:  A label (or heading) followed by its optional tip, on one baseline.
// WHY:   "Title + (i)" is the shape every refreshed row, card and section
//        header takes; one component keeps the gap and alignment identical.
export function WithTip({ children, tip, tipLabel, className }: { children: ReactNode; tip?: ReactNode; tipLabel?: string | undefined; className?: string | undefined }) {
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1", className)}>
      {children}
      {tip !== undefined && tip !== null && tip !== "" ? <InfoTip label={tipLabel}>{tip}</InfoTip> : null}
    </span>
  );
}
