// SOT: schema-action-host, schema-form-router
import type { SchemaActionHostProps } from "./actions";
import { CreateTableDialog } from "./forms/CreateTableDialog";
import { DuplicateTableDialog } from "./forms/DuplicateTableDialog";
import { EditTableDialog } from "./forms/EditTableDialog";
import { NamespaceDialog } from "./forms/NamespaceDialog";
import { RemoveTableDialog } from "./forms/RemoveTableDialog";

// WHAT:  Mounts the schema form an action asks for.
// WHY:   The sidebar holds one `SchemaAction | null`; this is the only place that
//        knows which dialog each kind opens, so adding a form is one case here.
// HOW:   Each dialog is keyed by its action, so a different table or a different
//        kind starts from a fresh form instead of inheriting the last one's state.
// WHERE: src/features/schema/actions.ts, src/features/shell/TablesPanel.tsx
export function SchemaActionHost({ connectionId, action, onClose }: SchemaActionHostProps) {
  if (action === null) return null;
  const key = JSON.stringify(action);
  switch (action.kind) {
    case "create-table":
      return <CreateTableDialog key={key} connectionId={connectionId} schema={action.schema} onClose={onClose} />;
    case "edit-table":
      return <EditTableDialog key={key} connectionId={connectionId} table={action.table} onClose={onClose} />;
    case "duplicate-table":
      return <DuplicateTableDialog key={key} connectionId={connectionId} table={action.table} onClose={onClose} />;
    case "remove-table":
      return <RemoveTableDialog key={key} connectionId={connectionId} table={action.table} tableKind={action.tableKind} action={action.action} onClose={onClose} />;
    case "create-namespace":
      return <NamespaceDialog key={key} connectionId={connectionId} namespace={action.namespace} step="create" name={null} onClose={onClose} />;
    case "rename-namespace":
      return <NamespaceDialog key={key} connectionId={connectionId} namespace={action.namespace} step="rename" name={action.name} onClose={onClose} />;
    case "drop-namespace":
      return <NamespaceDialog key={key} connectionId={connectionId} namespace={action.namespace} step="drop" name={action.name} onClose={onClose} />;
  }
}
