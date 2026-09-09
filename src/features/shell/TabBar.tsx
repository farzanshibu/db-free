// SOT: tab-bar, workspace-tabs, window-drag-region
import type { MouseEvent as ReactMouseEvent } from "react";
import { usePendingCount, useWorkspace, type Tab } from "@/stores/workspace";
import { useContextMenu, type MenuEntry } from "@/components/global/ContextMenu";
import { Icon, type IconName } from "@/lib/icons";
import { OBJECT_KINDS, TOOLS } from "@/lib/objects";
import { cn } from "@/lib/cn";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";

// WHAT:  Open tabs above the main area (also a drag region) + the Changes button.
export function TabBar() {
  const tabs = useWorkspace((s) => s.tabs);
  const activeTabId = useWorkspace((s) => s.activeTabId);
  const activateTab = useWorkspace((s) => s.activateTab);
  const closeTab = useWorkspace((s) => s.closeTab);
  const activeId = useWorkspace((s) => s.activeConnectionId);
  const openQuery = useWorkspace((s) => s.openQuery);
  const pending = usePendingCount(activeId);
  const panelOpen = useWorkspace((s) => s.changesPanelOpen);
  const setPanelOpen = useWorkspace((s) => s.setChangesPanelOpen);
  const closeOthers = useWorkspace((s) => s.closeOtherTabs);
  const closeToRight = useWorkspace((s) => s.closeTabsToRight);
  const closeAll = useWorkspace((s) => s.closeAllTabs);
  const menu = useContextMenu();

  const tabEntries = (tab: Tab, index: number): MenuEntry[] => [
    { id: "close", label: "Close", icon: "x" },
    { id: "close-others", label: "Close others", icon: "x", disabled: tabs.length < 2 },
    { id: "close-right", label: "Close to the right", icon: "chevron-right", disabled: index >= tabs.length - 1 },
    { id: "close-all", label: "Close all", icon: "trash", danger: true, group: "all" },
    { id: "duplicate", label: "New query tab", icon: "plus", group: "new", disabled: tab.connectionId === null },
  ];

  return (
    <div className="drag-region flex h-11 shrink-0 items-center border-b border-border/40 glass-header px-2" data-tauri-drag-region role="tablist" aria-label="Open tabs">
      <ScrollArea orientation="horizontal" hideScrollBar className="flex h-full items-center gap-1 py-1">
        {tabs.map((tab, index) => (
          <TabItem
            key={tab.id}
            tab={tab}
            active={tab.id === activeTabId}
            onActivate={() => activateTab(tab.id)}
            onClose={() => closeTab(tab.id)}
            onContextMenu={(e) =>
              menu.open(e, tabEntries(tab, index), (id) => {
                if (id === "close") closeTab(tab.id);
                else if (id === "close-others") closeOthers(tab.id);
                else if (id === "close-right") closeToRight(tab.id);
                else if (id === "close-all") closeAll();
                else if (id === "duplicate" && tab.connectionId !== null) openQuery(tab.connectionId);
              })
            }
          />
        ))}
      </ScrollArea>
      {activeId ? (
        <Button
          variant="ghost"
          size="sm"
          aria-label="New query tab"
          onClick={() => openQuery(activeId)}
          className="ml-1 size-7 min-w-7 rounded-lg text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
        >
          <Icon name="plus" size={13} />
        </Button>
      ) : null}
      <div className="drag-region h-full min-w-6 flex-1" data-tauri-drag-region />
      {activeId ? (
        <Button
          size="sm"
          variant={pending > 0 ? "secondary" : "ghost"}
          className={cn(
            "mr-2 h-7.5 rounded-lg text-xs font-medium liquid-hover",
            pending > 0 ? "glass-pill text-foreground border-warning/50" : "text-muted hover:bg-surface-secondary/70 hover:text-foreground",
          )}
          onClick={() => setPanelOpen(!panelOpen)}
        >
          Changes
          {pending > 0 ? (
            <Badge size="sm" variant="warning" className="ml-1.5 font-bold text-[10px] h-4 min-w-4 p-0">
              {pending}
            </Badge>
          ) : null}
        </Button>
      ) : null}
      {menu.node}
    </div>
  );
}

function tabPresentation(tab: Tab): { label: string; icon: IconName } {
  switch (tab.kind) {
    case "table":
      return { label: tab.table.name, icon: "table" };
    case "query":
      return { label: tab.title, icon: "terminal" };
    case "history":
      return { label: "Query History", icon: "history" };
    case "transfer":
      return { label: "Export / Import", icon: "download" };
    case "erd":
      return { label: `Diagram: ${tab.schema ?? "all"}`, icon: "view" };
    case "document":
      return { label: tab.documentKind === "dashboard" ? "Dashboard" : tab.documentKind === "workflow" ? "Workflow" : "Diagram", icon: tab.documentKind === "dashboard" ? "columns" : tab.documentKind === "workflow" ? "play" : "view" };
    case "chat":
      return { label: "Chat DB", icon: "braces" };
    case "object":
      return { label: tab.reference.name, icon: OBJECT_KINDS[tab.reference.kind].icon };
    case "admin":
      return { label: "Server", icon: "server" };
    case "tool":
      return { label: TOOLS[tab.tool].label, icon: TOOLS[tab.tool].icon };
  }
}

function TabItem({ tab, active, onActivate, onClose, onContextMenu }: { tab: Tab; active: boolean; onActivate: () => void; onClose: () => void; onContextMenu: (event: ReactMouseEvent) => void }) {
  const docName = useWorkspace((s) => (tab.kind === "document" ? s.documents[tab.documentKind].find((d) => d.id === tab.documentId)?.name : undefined));
  const base = tabPresentation(tab);
  const label = docName ?? base.label;
  return (
    <div
      role="tab"
      aria-selected={active}
      tabIndex={0}
      onClick={onActivate}
      onKeyDown={(e) => {
        if (e.key === "Enter") onActivate();
      }}
      onAuxClick={(e) => {
        if (e.button === 1) onClose();
      }}
      onContextMenu={onContextMenu}
      // WHAT:  A flat editor tab: tone carries the selection, an accent rule
      //        underlines it, and the close affordance appears on approach.
      // WHY:   Every tab drawing its own bordered pill and a permanent close
      //        button made the strip read as a row of buttons rather than as
      //        one control with a current item.
      className={cn(
        "group relative flex h-8 max-w-[200px] shrink-0 cursor-default items-center gap-2 rounded-t-md px-2.5",
        "text-[12.5px] font-medium transition-colors duration-150",
        "after:pointer-events-none after:absolute after:inset-x-1.5 after:bottom-0 after:h-0.5 after:rounded-full after:transition-colors",
        active
          ? "bg-surface-secondary/70 text-foreground after:bg-accent"
          : "text-muted after:bg-transparent hover:bg-surface-secondary/35 hover:text-foreground",
      )}
    >
      <Icon name={base.icon} size={12.5} className={cn("shrink-0", active ? "text-accent" : "text-muted")} />
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={`Close ${label}`}
        // The strip's own click handler activates a tab; without stopping here,
        // closing one would select it on the way out.
        onClick={(event) => {
          event.stopPropagation();
          onClose();
        }}
        className={cn(
          "size-4 rounded-sm p-0 transition-opacity",
          active ? "opacity-70 hover:opacity-100" : "opacity-0 group-hover:opacity-70 group-hover:hover:opacity-100",
        )}
      >
        <Icon name="x" size={11} />
      </Button>
    </div>
  );
}
