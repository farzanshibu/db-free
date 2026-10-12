// SOT: tab-view, tab-body-routing
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
import { Button } from "@/components/ui/button";

// WHAT:  The body of one tab, routed by kind.
// WHY:   Every pane renders its active tab through this, so the routing lives in
//        one component rather than inline in PaneArea.
// HOW:   The tab runs against its own connection (useTabConnection) and never
//        against another: a tab whose connection was deleted says so instead of
//        quietly running on whichever connection is active. Only a tab that
//        belongs to no connection (a loose dashboard or diagram) follows the
//        active one.
export function TabBody({ tab }: { tab: Tab | null }) {
  const active = useActiveConnection();
  const own = useTabConnection(tab);
  const closeTab = useWorkspace((s) => s.closeTab);
  if (tab !== null && tab.connectionId !== null && own === null) {
    return (
      <EmptyState
        icon="plug"
        title="This connection was removed"
        body="The tab belonged to a connection that no longer exists, so it will not run against another one."
        action={
          <Button variant="secondary" onClick={() => { closeTab(tab.id); }}>
            Close tab
          </Button>
        }
      />
    );
  }
  const connection = tab !== null && tab.connectionId !== null ? own : active;
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
