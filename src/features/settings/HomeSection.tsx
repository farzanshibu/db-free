// SOT: home-settings, show-pinned-setting, show-recents-setting
import type { ReactNode } from "react";
import { Toggle } from "@/components/global/Field";
import { Card, CardContent } from "@/components/ui/card";

interface HomeSectionProps {
  showPinned: boolean;
  showRecents: boolean;
  onChange: (partial: { showPinned?: boolean; showRecents?: boolean }) => void;
}

function Row({ title, body, children }: { title: string; body: string; children: ReactNode }) {
  return (
    <Card className="rounded-lg glass-card border-border/40 px-2.5 py-1.5 shadow-xs">
      <CardContent className="flex w-full flex-row items-center gap-3 p-0">
        <div className="min-w-0 flex-1">
          <p className="text-xs font-semibold tracking-tight text-foreground">{title}</p>
          <p className="mt-0.5 text-xs text-muted">{body}</p>
        </div>
        <div className="shrink-0">{children}</div>
      </CardContent>
    </Card>
  );
}

// WHAT:  Settings → Home: whether the Pinned board and the Recents show above the
//        connection list.
// WHY:   Both are conveniences; someone who wants a plain list of connections can
//        switch them off without losing what is pinned or recorded.
// HOW:   Edits the draft's `showPinned` / `showRecents`; Save on the settings dock persists them.
// WHERE: src/features/home/HomeBoard.tsx, AppSettings.showPinned / showRecents
export function HomeSection({ showPinned, showRecents, onChange }: HomeSectionProps) {
  return (
    <>
      <h2 className="text-[13px] font-semibold text-foreground">Home</h2>
      <Row title="Show Pinned on Home" body="The board of pinned connections, tables, saved queries, dashboards, workflows and diagrams.">
        <Toggle checked={showPinned} onChange={(value) => onChange({ showPinned: value })} label="" />
      </Row>
      <Row title="Show Recents on Home" body="What you opened lately, as shortcuts. Clear removes them; turning this off only hides them.">
        <Toggle checked={showRecents} onChange={(value) => onChange({ showRecents: value })} label="" />
      </Row>
    </>
  );
}
