// SOT: tab-view, tab-body-routing, split-view, split-pane
import { useCallback, useState } from "react";
import { useActiveConnection, useTabConnection, useWorkspace, type Tab } from "@/stores/workspace";
import { isKeyValueEngine } from "@/lib/engines";
import { QueryPane } from "@/features/editor/QueryPane";
import { TableTab } from "@/features/grid/TableTab";
import { KeyTab } from "@/features/keys/KeyTab";
import { HistoryTab } from "@/features/history/HistoryTab";
import { TransferTab } from "@/features/transfer/TransferTab";
import { ErdTab } from "@/features/diagrams/ErdTab";
import { DocumentTab } from "@/features/documents/DocumentTab";
import { ChatTab } from "@/features/chat/ChatTab";
import { ObjectTab } from "@/features/objects/ObjectTab";
import { AdminTab } from "@/features/admin/AdminTab";
import { ToolTab } from "@/features/tools/ToolTab";
import { SchemaCompareTab } from "@/features/compare/SchemaCompareTab";
import { DataCompareTab } from "@/features/compare/DataCompareTab";
import { EmptyState } from "@/components/global/EmptyState";
import { RunShortcut } from "@/components/global/Kbd";
import { Resizer } from "@/components/global/Resizer";
import { IconButton } from "@/components/global/Button";
import { Icon } from "@/lib/icons";
import { tabPresentation } from "./TabBar";

// WHAT:  The body of one tab, routed by kind.
// WHY:   The same tab can render in the main area or in the split pane, so the
//        routing lives in one component rather than inline in App.
// HOW:   The tab runs against its own connection (useTabConnection), falling
//        back to the active one for a tab whose connection was deleted.
export function TabBody({ tab }: { tab: Tab | null }) {
  const active = useActiveConnection();
  const own = useTabConnection(tab);
  const connection = own ?? active;
  if (tab === null || connection === null) {
    return <EmptyState icon="table" title="Pick a table" body="Select a table on the left to browse it, or open a query tab. Run with" action={<RunShortcut />} />;
  }
  switch (tab.kind) {
    case "table":
      return isKeyValueEngine(connection.engine) ? <KeyTab key={tab.id} connectionId={tab.connectionId} table={tab.table} /> : <TableTab key={`${tab.id}:${tab.filterKey}`} connectionId={tab.connectionId} table={tab.table} initialFilters={tab.initialFilters} />;
    case "query":
      return <QueryPane key={tab.id} tabId={tab.id} title={tab.title} connection={connection} seedSql={tab.seedSql} />;
    case "history":
      return <HistoryTab key={tab.id} connectionId={tab.connectionId} />;
    case "transfer":
      return <TransferTab key={tab.id} connectionId={tab.connectionId} />;
    case "erd":
      return <ErdTab key={tab.id} connectionId={tab.connectionId} schema={tab.schema} />;
    case "chat":
      return <ChatTab key={tab.id} connectionId={tab.connectionId} />;
    case "object":
      return <ObjectTab key={tab.id} connectionId={tab.connectionId} reference={tab.reference} />;
    case "admin":
      return <AdminTab key={tab.id} connectionId={tab.connectionId} />;
    case "tool":
      return <ToolTab key={tab.id} connectionId={tab.connectionId} tool={tab.tool} />;
    case "schema-compare":
      return <SchemaCompareTab key={tab.id} connectionId={tab.connectionId} schema={tab.schema} />;
    case "data-compare":
      return <DataCompareTab key={tab.id} connectionId={tab.connectionId} table={tab.table} />;
    case "document":
      return <DocumentTab key={tab.id} kind={tab.documentKind} documentId={tab.documentId} connectionId={tab.connectionId} />;
  }
}

const SPLIT_WIDTH_KEY = "db-free:split-width";
const SPLIT_MIN = 320;

function readSplitWidth(): number {
  try {
    const saved = Number(localStorage.getItem(SPLIT_WIDTH_KEY));
    return Number.isFinite(saved) && saved >= SPLIT_MIN ? saved : 560;
  } catch {
    return 560;
  }
}

// WHAT:  The main tab and, when a tab is opened to the side, a second pane
//        beside it with a draggable splitter.
// WHY:   Writing a query while looking at the table it reads, or comparing
//        two results, needs both on screen; switching tabs back and forth
//        loses the place in each.
// HOW:   The side pane shows `splitTabId`; the store keeps it distinct from
//        the active tab. The pane that was clicked last is outlined so it is
//        clear which one the keyboard is in.
// WHERE: src/stores/workspace.ts (openToSide, closeSplit)
export function TabArea({ tab }: { tab: Tab | null }) {
  const splitTab = useWorkspace((s) => s.tabs.find((t) => t.id === s.splitTabId) ?? null);
  const closeSplit = useWorkspace((s) => s.closeSplit);
  const swapSplit = useWorkspace((s) => s.swapSplit);
  const [width, setWidth] = useState(readSplitWidth);

  const onResize = useCallback((delta: number) => {
    setWidth((prev) => {
      const next = Math.max(SPLIT_MIN, Math.min(window.innerWidth - 480, prev - delta));
      try {
        localStorage.setItem(SPLIT_WIDTH_KEY, String(next));
      } catch {
        // storage unavailable: the width resets next launch
      }
      return next;
    });
  }, []);

  if (splitTab === null) return <TabBody tab={tab} />;
  const side = tabPresentation(splitTab);
  return (
    <div className="flex h-full min-h-0">
      <div className="min-w-0 flex-1">
        <TabBody tab={tab} />
      </div>
      <div className="relative flex shrink-0 flex-col border-l border-border/40" style={{ width }}>
        <Resizer direction="horizontal" onResize={onResize} className="absolute -left-1 top-0 bottom-0 z-10" />
        <div className="flex h-7 shrink-0 items-center gap-2 border-b border-border/40 glass-header px-2 text-[12px] text-muted">
          <Icon name={side.icon} size={12} className="shrink-0 text-accent" />
          <span className="min-w-0 flex-1 truncate font-medium text-foreground">{side.label}</span>
          <IconButton icon="columns" label="Swap panes" onClick={swapSplit} size={12} className="size-5 min-w-5" />
          <IconButton icon="x" label="Close split" onClick={closeSplit} size={12} className="size-5 min-w-5" />
        </div>
        <div className="min-h-0 flex-1">
          <TabBody tab={splitTab} />
        </div>
      </div>
    </div>
  );
}
