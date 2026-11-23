// SOT: tool-shell, playground-header, playground-layout, collection-options
import { useMemo, type ReactNode } from "react";
import type { Tool } from "@/lib/bindings";
import { toolMeta } from "@/lib/objects";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { InfoTip } from "@/components/global/InfoTip";
import { SplitLayout } from "@/components/global/Layout";
import { ScrollArea } from "@/components/ui/scroll-area";

// WHAT:  Common frame for the playground tabs: header (icon, name, a tip
//        with the tool's one-line hint, optional right-hand controls) above
//        the tool body.
// WHERE: src/features/tools/ToolTab.tsx
export function ToolShell({ tool, right, children }: { tool: Tool; right?: ReactNode; children: ReactNode }) {
  const meta = toolMeta(tool);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex app-toolbar shrink-0 items-center gap-2 border-b border-border/40 glass-header ">
        <Icon name={meta.icon} size={14} className="text-accent" />
        <span className="text-[13px] font-semibold tracking-tight text-foreground">{meta.label}</span>
        <InfoTip label={`About ${meta.label}`} side="bottom">
          {meta.hint}
        </InfoTip>
        <span className="ml-auto flex min-w-0 items-center gap-1.5">{right}</span>
      </div>
      <div className="min-h-0 flex-1">{children}</div>
    </div>
  );
}

// WHAT:  The catalog's collections / tables / indexes / topics for a
//        connection as select options (every playground picks one first).
export function useCollectionOptions(connectionId: string): { value: string; label: string }[] {
  // The catalog reference is stable between loads, so the options only rebuild on reload.
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  return useMemo(() => {
    if (!catalog) return [];
    return catalog.schemas.flatMap((schema) => schema.tables.map((t) => ({ value: t.name, label: t.schema && catalog.schemas.length > 1 ? `${t.schema}.${t.name}` : t.name })));
  }, [catalog]);
}

// WHAT:  Two-pane playground body: a form column on the left, results filling the rest.
// WHY:   One resizable, remembered form column for every playground (the
//        refresh rule for side panels): a long filter or vector wants more
//        room than a topic picker, and the width carries across tools.
// WHERE: src/components/global/Layout.tsx (SplitLayout)
export function ToolBody({ form, children }: { form: ReactNode; children: ReactNode }) {
  return (
    <SplitLayout
      side="left"
      defaultSize={288}
      min={220}
      max={560}
      storageKey="db-free:tool-panel-width"
      resizeLabel="Resize form panel"
      panelClassName="border-r border-border/40"
      panel={<ScrollArea hideScrollBar className="flex min-h-0 min-w-0 flex-1 flex-col gap-2 p-2.5">{form}</ScrollArea>}
    >
      {children}
    </SplitLayout>
  );
}
