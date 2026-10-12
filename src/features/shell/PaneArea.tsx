// SOT: pane-area, split-view, split-pane, pane-tree-render, pane-drop-overlay
import { useCallback, useRef } from "react";
import { useWorkspace } from "@/stores/workspace";
import { paneCount, type DropZone, type PaneNode } from "@/lib/panes";
import { cn } from "@/lib/cn";
import { Resizer } from "@/components/global/Resizer";
import { EnvironmentEdge } from "./EnvironmentEdge";
import { TabBar } from "./TabBar";
import { useTabIdentity } from "./TabIdentity";
import { TabBody } from "./TabView";
import { usePaneDrag } from "./paneDrag";

// WHAT:  The editor area: up to four panes in a split tree, each with its own tab
//        strip and body, divided by draggable splitters.
// WHY:   Writing a query beside the table it reads, or comparing two results,
//        needs both on screen; switching tabs back and forth loses the place in
//        each. The tree (src/lib/panes.ts) lets panes split sideways or
//        downwards, to a depth of four panes.
// HOW:   The root is rendered recursively: a split lays its two children out in
//        a row or a column, the first taking `ratio` of the space; a leaf is one
//        pane. Pressing anywhere in a pane gives it the keyboard focus, which is
//        what makes its active tab "the" active tab for shortcuts and the sidebar.
// WHERE: src/stores/workspace.ts (panes), src/lib/panes.ts, src/App.tsx
export function PaneArea() {
  const root = useWorkspace((s) => s.panes.root);
  return (
    <div className="flex h-full min-h-0 min-w-0 flex-1">
      <PaneTree node={root} />
    </div>
  );
}

function PaneTree({ node }: { node: PaneNode }) {
  return node.kind === "leaf" ? <Pane paneId={node.id} /> : <SplitView node={node} />;
}

function SplitView({ node }: { node: Extract<PaneNode, { kind: "split" }> }) {
  const container = useRef<HTMLDivElement | null>(null);
  const resize = useWorkspace((s) => s.resizeSplit);
  const row = node.direction === "row";
  const ratio = node.ratio;
  const splitId = node.id;

  const onResize = useCallback(
    (delta: number) => {
      const size = row ? container.current?.clientWidth : container.current?.clientHeight;
      if (size === undefined || size === 0) return;
      resize(splitId, ratio + delta / size);
    },
    [resize, row, ratio, splitId],
  );

  return (
    <div ref={container} className={cn("flex min-h-0 min-w-0 flex-1", row ? "flex-row" : "flex-col")}>
      <div className="flex min-h-0 min-w-0" style={{ flex: `${String(ratio)} 1 0%` }}>
        <PaneTree node={node.first} />
      </div>
      <Resizer direction={row ? "horizontal" : "vertical"} onResize={onResize} className="shrink-0" />
      <div className="flex min-h-0 min-w-0" style={{ flex: `${String(1 - ratio)} 1 0%` }}>
        <PaneTree node={node.second} />
      </div>
    </div>
  );
}

function Pane({ paneId }: { paneId: string }) {
  const tab = useWorkspace((s) => s.tabs.find((t) => t.id === s.panes.active[paneId]) ?? null);
  const focused = useWorkspace((s) => s.panes.focused === paneId);
  const crowded = useWorkspace((s) => paneCount(s.panes) > 1);
  const focusPane = useWorkspace((s) => s.focusPane);
  // The pane's own connection: its environment edge is drawn here, not once for the window.
  const identity = useTabIdentity(tab);
  return (
    <section
      data-pane-id={paneId}
      data-connection-id={tab?.connectionId ?? undefined}
      aria-label="Editor pane"
      // Capture phase: a pane takes the keyboard focus on the press, before whatever
      // is inside it (a grid cell, the editor) handles the same event.
      onPointerDownCapture={() => {
        if (!focused) focusPane(paneId);
      }}
      className="relative flex min-h-0 min-w-0 flex-1 flex-col"
    >
      {crowded && identity !== null ? <EnvironmentEdge environment={identity.environment} perPane /> : null}
      <TabBar paneId={paneId} />
      <div className={cn("relative min-h-0 flex-1", crowded && !focused && "opacity-95")}>
        <TabBody tab={tab} />
        <DropOverlay paneId={paneId} />
      </div>
    </section>
  );
}

/// The part of the pane a drop in `zone` would take: an edge split gets the half it adds.
const ZONE_BOX: Record<DropZone, string> = {
  left: "inset-y-0 left-0 w-1/2",
  right: "inset-y-0 right-0 w-1/2",
  top: "inset-x-0 top-0 h-1/2",
  bottom: "inset-x-0 bottom-0 h-1/2",
  center: "inset-0",
};

// WHAT:  Shows where a dragged tab would land while the pointer is over this pane.
function DropOverlay({ paneId }: { paneId: string }) {
  const zone = usePaneDrag((s) => (s.target?.type === "pane" && s.target.paneId === paneId ? s.target.zone : null));
  if (zone === null) return null;
  return <div aria-hidden className={cn("pointer-events-none absolute z-40 rounded-sm border-2 border-accent/60 bg-accent/15", ZONE_BOX[zone])} />;
}
