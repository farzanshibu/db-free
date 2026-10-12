// SOT: tab-bar, workspace-tabs, window-drag-region, tab-drag-reorder, tab-context-menu, preview-tab-style, pane-tab-strip
import { useMemo, type MouseEvent as ReactMouseEvent } from "react";
import { usePendingCount, useWorkspace, type Tab } from "@/stores/workspace";
import { useContextMenu, type MenuEntry } from "@/components/global/ContextMenu";
import { Icon, type IconName } from "@/lib/icons";
import { OBJECT_KINDS, TOOLS } from "@/lib/objects";
import { MAX_PANES, paneCount, tabsInPane, topEdgePanes } from "@/lib/panes";
import { cn } from "@/lib/cn";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { identityLabel } from "@/lib/tabIdentity";
import { beginPaneDrag, usePaneDrag } from "./paneDrag";
import { ConnectionChip, TabTooltipBody, tabSchema, useConnectionIdentity, useTabIdentity } from "./TabIdentity";

// WHAT:  One pane's tab strip: its tabs, the "+" button and, on the pane in the
//        top-right corner, the Changes button. On the top row it is also the
//        window's drag region.
// WHY:   Every pane has its own strip so a tab is always visibly "in" a pane;
//        dragging a tab along a strip reorders it, onto another strip moves it.
// HOW:   Each tab carries its own connection's colour, name and environment
//        (TabIdentity.tsx) and acts on that connection, whichever one is the
//        active connection of the window. The pane's tabs are the flat tab list filtered to the pane
//        (src/lib/panes.ts). `data-pane-strip` / `data-tab-id` are what the drag
//        controller reads to find an insertion point (paneDrag.ts).
export function TabBar({ paneId }: { paneId: string }) {
  const tabs = useWorkspace((s) => s.tabs);
  const panes = useWorkspace((s) => s.panes);
  const previewTabId = useWorkspace((s) => s.previewTabId);
  const dirtyTabs = useWorkspace((s) => s.dirtyTabs);
  const activateTab = useWorkspace((s) => s.activateTab);
  const promoteTab = useWorkspace((s) => s.promoteTab);
  const requestClose = useWorkspace((s) => s.requestCloseTabs);
  const focusPane = useWorkspace((s) => s.focusPane);
  const splitPane = useWorkspace((s) => s.splitPane);
  const closePane = useWorkspace((s) => s.closePane);
  const activeConnectionId = useWorkspace((s) => s.activeConnectionId);
  const openQuery = useWorkspace((s) => s.openQuery);
  const pending = usePendingCount(activeConnectionId);
  const panelOpen = useWorkspace((s) => s.changesPanelOpen);
  const setPanelOpen = useWorkspace((s) => s.setChangesPanelOpen);
  const reopen = useWorkspace((s) => s.reopenClosedTab);
  const canReopen = useWorkspace((s) => s.closedTabs.length > 0);
  const dropBefore = usePaneDrag((s) => (s.target?.type === "strip" && s.target.paneId === paneId ? s.target.beforeId : undefined));
  const menu = useContextMenu();

  const mine = useMemo(() => tabsInPane(panes, tabs, paneId), [panes, tabs, paneId]);
  const activeId = panes.active[paneId] ?? null;
  const activeTab = mine.find((t) => t.id === activeId) ?? null;
  // The "+" belongs to the pane: it opens a query on the connection of the pane's
  // own tab, which is not always the window's active connection.
  const connectionId = activeTab?.connectionId ?? activeConnectionId;
  const newTabIdentity = useConnectionIdentity(connectionId);
  // The Changes panel shows the active connection's staged edits, so the button names it.
  const changesIdentity = useConnectionIdentity(activeConnectionId);
  const count = paneCount(panes);
  const edge = topEdgePanes(panes.root);
  const onTop = edge.includes(paneId);
  const carriesChanges = edge[edge.length - 1] === paneId;
  const focused = panes.focused === paneId;
  const canSplit = count < MAX_PANES;

  const tabEntries = (tab: Tab, index: number): MenuEntry[] => [
    { id: "close", label: "Close", icon: "x" },
    { id: "close-others", label: "Close others", icon: "x", disabled: mine.length < 2 },
    { id: "close-right", label: "Close to the right", icon: "chevron-right", disabled: index >= mine.length - 1 },
    { id: "close-all", label: "Close all", icon: "trash", danger: true, group: "all" },
    { id: "split-right", label: "Split right", icon: "columns", group: "split", disabled: !canSplit || mine.length < 2 },
    { id: "split-down", label: "Split down", icon: "rows", group: "split", disabled: !canSplit || mine.length < 2 },
    { id: "close-pane", label: "Close pane", icon: "x", group: "split", disabled: count < 2 },
    { id: "duplicate", label: "New query tab", icon: "plus", group: "new", disabled: tab.connectionId === null },
    { id: "reopen", label: "Reopen closed tab", icon: "history", group: "new", disabled: !canReopen },
  ];

  const onAction = (tab: Tab, index: number, id: string) => {
    if (id === "close") requestClose([tab.id]);
    else if (id === "close-others") requestClose(mine.filter((t) => t.id !== tab.id).map((t) => t.id));
    else if (id === "close-right") requestClose(mine.slice(index + 1).map((t) => t.id));
    else if (id === "close-all") requestClose(mine.map((t) => t.id));
    else if (id === "split-right") splitPane(paneId, "right", tab.id);
    else if (id === "split-down") splitPane(paneId, "down", tab.id);
    else if (id === "close-pane") closePane(paneId);
    else if (id === "duplicate" && tab.connectionId !== null) {
      focusPane(paneId);
      openQuery(tab.connectionId);
    } else if (id === "reopen") reopen();
  };

  return (
    <div
      className={cn(
        "flex shrink-0 items-center border-b glass-header px-2",
        onTop ? "drag-region app-titlebar" : "h-7.5",
        focused && count > 1 ? "border-accent/50" : "border-border/40",
      )}
      data-tauri-drag-region={onTop ? true : undefined}
      data-pane-strip={paneId}
      role="tablist"
      aria-label="Open tabs"
    >
      <ScrollArea orientation="horizontal" hideScrollBar className="flex h-full items-end gap-0.5 pt-1">
        {mine.map((tab, index) => (
          <TabItem
            key={tab.id}
            tab={tab}
            active={tab.id === activeId}
            focusedPane={focused}
            preview={tab.id === previewTabId}
            dirty={dirtyTabs[tab.id] === true}
            dropBefore={dropBefore === tab.id}
            onActivate={() => {
              focusPane(paneId);
              activateTab(tab.id);
              promoteTab(tab.id);
            }}
            onPromote={() => promoteTab(tab.id)}
            onClose={() => requestClose([tab.id])}
            onContextMenu={(e) => menu.open(e, tabEntries(tab, index), (id) => onAction(tab, index, id))}
          />
        ))}
        {dropBefore === null ? <span aria-hidden className="h-5 w-0.5 shrink-0 rounded-full bg-accent" /> : null}
      </ScrollArea>
      {connectionId ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="sm"
              aria-label={newTabIdentity === null ? "New query tab" : `New query tab on ${newTabIdentity.name}`}
              onClick={() => {
                focusPane(paneId);
                openQuery(connectionId);
              }}
              className="ml-1 size-6 min-w-6 rounded-md text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover"
            >
              <Icon name="plus" size={12} />
            </Button>
          </TooltipTrigger>
          <TooltipContent>{newTabIdentity === null ? "New query tab" : `New query tab on ${identityLabel(newTabIdentity)}`}</TooltipContent>
        </Tooltip>
      ) : null}
      <div className={cn("h-full min-w-6 flex-1", onTop ? "drag-region" : "")} data-tauri-drag-region={onTop ? true : undefined} />
      {carriesChanges && activeConnectionId ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              size="sm"
              variant={pending > 0 ? "secondary" : "ghost"}
              className={cn(
                "mr-1.5 rounded-md text-xs font-medium liquid-hover",
                pending > 0 ? "glass-pill text-foreground border-warning/50" : "text-muted hover:bg-surface-secondary/70 hover:text-foreground",
              )}
              onClick={() => setPanelOpen(!panelOpen)}
            >
              {changesIdentity !== null ? <span aria-hidden className={cn("size-1.5 shrink-0 rounded-full", changesIdentity.classes.stripe)} /> : null}
              Changes
              {pending > 0 ? (
                <Badge size="sm" variant="warning" className="ml-1 font-bold text-[10px] h-3.5 min-w-3.5 p-0">
                  {pending}
                </Badge>
              ) : null}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{changesIdentity === null ? "Pending changes" : `Pending changes on ${identityLabel(changesIdentity)}`}</TooltipContent>
        </Tooltip>
      ) : null}
      {menu.node}
    </div>
  );
}

export function tabPresentation(tab: Tab): { label: string; icon: IconName } {
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
    case "schema-compare":
      return { label: "Schema compare", icon: "git-branch" };
    case "data-compare":
      return { label: tab.table ? `Compare: ${tab.table.name}` : "Data compare", icon: "exchange" };
  }
}

function TabItem({
  tab,
  active,
  focusedPane,
  preview,
  dirty,
  dropBefore,
  onActivate,
  onPromote,
  onClose,
  onContextMenu,
}: {
  tab: Tab;
  active: boolean;
  /// The pane this tab is in has the keyboard: its active tab gets the accent rule.
  focusedPane: boolean;
  /// Shown in italics; the next table opened from the sidebar replaces it.
  preview: boolean;
  /// Has unsaved work: closing it asks first.
  dirty: boolean;
  /// A dragged tab would land just before this one.
  dropBefore: boolean;
  onActivate: () => void;
  onPromote: () => void;
  onClose: () => void;
  onContextMenu: (event: ReactMouseEvent) => void;
}) {
  const docName = useWorkspace((s) => (tab.kind === "document" ? s.documents[tab.documentKind].find((d) => d.id === tab.documentId)?.name : undefined));
  const dragging = usePaneDrag((s) => s.payload?.kind === "tab" && s.payload.tabId === tab.id);
  const identity = useTabIdentity(tab);
  const base = tabPresentation(tab);
  const label = docName ?? base.label;
  const envTint = identity?.environmentMeta.tint ?? "";
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          role="tab"
          aria-selected={active}
          aria-label={identity === null ? label : `${label} — ${identityLabel(identity)}`}
          tabIndex={0}
          data-tab-id={tab.id}
          data-connection-id={tab.connectionId ?? undefined}
          onClick={onActivate}
          onDoubleClick={onPromote}
          onPointerDown={(e) => beginPaneDrag(e, { kind: "tab", tabId: tab.id, label, icon: base.icon })}
          onKeyDown={(e) => {
            if (e.key === "Enter") onActivate();
          }}
          onAuxClick={(e) => {
            if (e.button === 1) onClose();
          }}
          onContextMenu={onContextMenu}
          // WHAT:  A flat editor tab: tone carries the selection, an accent rule
          //        underlines it, and the close affordance appears on approach.
          //        Above it, a stripe in the connection's colour — and a wash for
          //        staging / production — says which connection the tab is on.
          // WHY:   Every tab drawing its own bordered pill and a permanent close
          //        button made the strip read as a row of buttons rather than as
          //        one control with a current item. And tabs from several
          //        connections share one strip: the colour is what keeps them apart.
          className={cn(
            "group relative isolate flex h-7 max-w-64 shrink-0 cursor-default items-center gap-1.5 rounded-t-md px-2",
            "text-xs font-medium transition-colors duration-150",
            "after:pointer-events-none after:absolute after:inset-x-1.5 after:bottom-0 after:h-0.5 after:rounded-full after:transition-colors",
            active
              ? cn("bg-surface-secondary/70 text-foreground", focusedPane ? "after:bg-accent" : "after:bg-accent/40")
              : "text-muted after:bg-transparent hover:bg-surface-secondary/35 hover:text-foreground",
            dragging && "opacity-40",
            dropBefore && "before:pointer-events-none before:absolute before:-left-1 before:top-1 before:bottom-1 before:w-0.5 before:rounded-full before:bg-accent",
          )}
        >
          {identity !== null ? (
            <>
              <span aria-hidden className={cn("pointer-events-none absolute inset-0 -z-10 rounded-t-md", active ? identity.classes.tintActive : identity.classes.tint)} />
              {envTint !== "" ? <span aria-hidden className={cn("pointer-events-none absolute inset-0 -z-10 rounded-t-md", envTint)} /> : null}
              <span aria-hidden className={cn("pointer-events-none absolute inset-x-0 top-0 h-0.5 rounded-t-md", identity.classes.stripe, active ? "opacity-100" : "opacity-60")} />
            </>
          ) : null}
          <Icon name={base.icon} size={12} className={cn("shrink-0", active ? "text-accent" : "text-muted")} />
          <span className={cn("min-w-8 flex-1 truncate", preview && "italic")}>{label}</span>
          {identity !== null ? <ConnectionChip identity={identity} /> : null}
          {dirty ? <span aria-label="Unsaved changes" className="size-1.5 shrink-0 rounded-full bg-warning group-hover:hidden" /> : null}
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={`Close ${label}`}
            // The strip's own click handler activates a tab, and a press starts a drag;
            // without stopping both here, closing one would select or drag it on the way out.
            onPointerDown={(event) => {
              event.stopPropagation();
            }}
            onClick={(event) => {
              event.stopPropagation();
              onClose();
            }}
            className={cn(
              "size-4 min-w-4 rounded-sm p-0 transition-opacity",
              dirty ? "hidden group-hover:inline-flex group-hover:opacity-70 group-hover:hover:opacity-100" : active ? "opacity-70 hover:opacity-100" : "opacity-0 group-hover:opacity-70 group-hover:hover:opacity-100",
            )}
          >
            <Icon name="x" size={10} />
          </Button>
        </div>
      </TooltipTrigger>
      <TooltipContent side="bottom" align="start">
        <TabTooltipBody label={label} identity={identity} schema={tabSchema(tab)} />
      </TooltipContent>
    </Tooltip>
  );
}
