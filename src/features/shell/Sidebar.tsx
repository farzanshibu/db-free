// SOT: sidebar-switch, sidebar-modes, drag-to-expand, sidebar-footer, sidebar-collapse, sidebar-width-per-connection
import { useCallback, useEffect } from "react";
import { useWorkspace, type SidebarMode } from "@/stores/workspace";
import { readSidebarWidth } from "@/lib/layouts";
import { Resizer } from "@/components/global/Resizer";
import { TablesPanel } from "./TablesPanel";
import { ConnectionHealth } from "./ConnectionHealth";
import { ObjectsPanel } from "@/features/objects/ObjectsPanel";
import { QueriesPanel } from "@/features/queries/QueriesPanel";
import { DocumentsPanel } from "@/features/documents/DocumentsPanel";

function renderPanel(mode: SidebarMode) {
  switch (mode) {
    case "tables":
      return <TablesPanel />;
    case "objects":
      return <ObjectsPanel />;
    case "queries":
      return <QueriesPanel />;
    case "dashboards":
      return <DocumentsPanel kind="dashboard" />;
    case "workflows":
      return <DocumentsPanel kind="workflow" />;
    case "diagrams":
      return <DocumentsPanel kind="diagram" />;
  }
}

// WHAT:  The left sidebar: the panel for the chosen mode and the connection
//        health footer, resizable, and hideable (⌘J).
// WHY:   A schema of three tables and one of four hundred want different widths,
//        so the width is remembered per connection; hiding the sidebar gives the
//        whole window to the tabs (the Focus layout).
// HOW:   Width and collapsed state live in the workspace store (layout presets
//        set them too). Switching connection loads that connection's width; a
//        drag updates the store live and is written down when released.
// WHERE: src/stores/workspace.ts (sidebarWidth, sidebarCollapsed), src/lib/layouts.ts
export function Sidebar() {
  const mode = useWorkspace((s) => s.sidebar);
  const collapsed = useWorkspace((s) => s.sidebarCollapsed);
  const width = useWorkspace((s) => s.sidebarWidth);
  const connectionId = useWorkspace((s) => s.activeConnectionId);
  const setWidth = useWorkspace((s) => s.setSidebarWidth);

  useEffect(() => {
    setWidth(readSidebarWidth(connectionId));
  }, [connectionId, setWidth]);

  const handleResize = useCallback(
    (delta: number) => {
      setWidth(useWorkspace.getState().sidebarWidth + delta);
    },
    [setWidth],
  );
  const remember = useCallback(() => {
    setWidth(useWorkspace.getState().sidebarWidth, connectionId);
  }, [connectionId, setWidth]);

  if (collapsed) return null;
  return (
    <div className="relative flex h-full shrink-0" style={{ width }}>
      <div className="flex h-full w-full min-w-0 flex-col">
        <div className="flex min-h-0 flex-1 flex-col">{renderPanel(mode)}</div>
        <ConnectionHealth />
      </div>
      <Resizer direction="horizontal" onResize={handleResize} onDragEnd={remember} className="absolute -right-0.5 top-0 bottom-0" />
    </div>
  );
}
