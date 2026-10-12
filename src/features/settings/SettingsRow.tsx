// SOT: settings-row, settings-card-row
import type { ReactNode } from "react";
import { Card, CardContent } from "@/components/ui/card";

// WHAT:  One labelled setting: title and explanation on the left, the control on the right.
// WHY:   Every section of the Settings page is a stack of these; one component
//        keeps their spacing and type identical.
export function SettingsRow({ title, body, children }: { title: string; body: string; children: ReactNode }) {
  return (
    <Card className="rounded-lg glass-card border-border/40 px-2.5 py-1.5 shadow-xs">
      <CardContent className="flex flex-row items-center gap-3 p-0 w-full">
        <div className="min-w-0 flex-1">
          <p className="text-xs font-semibold text-foreground tracking-tight">{title}</p>
          <p className="text-xs text-muted mt-0.5">{body}</p>
        </div>
        <div className="shrink-0">{children}</div>
      </CardContent>
    </Card>
  );
}
