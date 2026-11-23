// SOT: settings-row, settings-card-row
import type { ReactNode } from "react";
import { WithTip } from "@/components/global/InfoTip";
import { Card, CardContent } from "@/components/ui/card";

// WHAT:  One labelled setting: its title (with the explanation in a tip) on the
//        left, the control on the right.
// WHY:   Every section of the Settings page is a stack of these; one component
//        keeps their spacing and type identical. The explanation used to be a
//        second line under every title — read once, then only in the way — so
//        the UI refresh moved it behind an info icon beside the title.
// HOW:   `data-slot="settings-row"` lets the Settings frame lay rows out in a
//        fluid grid while headings, lists and whole-section components span
//        every column. The control wraps below the title when the cell is
//        narrower than both (a row of layout buttons, a long select).
// WHERE: src/features/settings/SettingsPage.tsx (the grid), docs/ui-refresh.md
export function SettingsRow({ title, body, children }: { title: string; body: string; children: ReactNode }) {
  return (
    <Card data-slot="settings-row" className="rounded-lg glass-card border-border/40 px-2.5 py-1.5 shadow-xs">
      <CardContent className="flex w-full flex-row flex-wrap items-center justify-between gap-x-3 gap-y-1.5 p-0">
        <WithTip tip={body} tipLabel={`About ${title}`} className="min-h-6 min-w-0 flex-1">
          <span className="truncate text-xs font-semibold tracking-tight text-foreground" title={title}>
            {title}
          </span>
        </WithTip>
        <div className="flex shrink-0 justify-end">{children}</div>
      </CardContent>
    </Card>
  );
}
