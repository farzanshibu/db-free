// SOT: grid-toolbar-label, folding-trigger-label, trigger-tooltip
import type { ReactElement, ReactNode } from "react";
import type { ShortcutAction } from "@/lib/keymap";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Shortcut } from "@/components/global/Kbd";

/// Text beside a toolbar icon, shown only once the toolbar (the nearest
/// `@container`) is wide. Literal, so Tailwind's scanner sees it.
export const WIDE_LABEL = "hidden @4xl:inline";

// WHAT:  A tooltip around a toolbar trigger (a popover or menu trigger, asChild),
//        naming it — and its shortcut — while its text label is folded away.
// WHY:   Grid and result toolbars drop their text labels on a narrow pane; the
//        name has to stay one hover (or focus) away, and a trigger that is
//        already a Popover/DropdownMenu trigger cannot use IconButton.
// HOW:   Radix Tooltip with an explicit asChild trigger, so the slot chain ends
//        on the caller's Button. `when` false renders the child untouched.
// WHERE: docs/ui-refresh.md ("Toolbar overflow"), src/components/global/Toolbar.tsx
export function TriggerTip({ tip, shortcut, when = true, children }: { tip: ReactNode; shortcut?: ShortcutAction | undefined; when?: boolean; children: ReactElement }) {
  if (!when) return children;
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent className="flex items-center gap-1.5">
        {tip}
        {shortcut !== undefined ? <Shortcut action={shortcut} /> : null}
      </TooltipContent>
    </Tooltip>
  );
}
