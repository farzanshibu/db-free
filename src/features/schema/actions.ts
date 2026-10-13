// SOT: schema-action, schema-dialog-routing, schema-form-entry-points
import type { NamespaceKind, TableKind, TableRef } from "@/lib/bindings";

// WHAT:  Every schema form the sidebar (or the palette) can open, as data.
// WHY:   The sidebar decides *what* the user asked for (a right-click on a view,
//        a schema header, the New-table button); the forms decide *how*. One
//        union between them means the sidebar holds a single `SchemaAction | null`
//        and renders one host, instead of one boolean and one dialog per form.
// HOW:   `SchemaActionHost` (SchemaActionHost.tsx) maps each kind to its dialog.
//        Names are plain strings: a schema or database is only ever addressed by
//        the name the catalog / session reported.
// WHERE: src/features/schema/SchemaActionHost.tsx, src/features/shell/TablesPanel.tsx
export type SchemaAction =
  /// New table in `schema` (null = the engine's only / default namespace).
  | { kind: "create-table"; schema: string | null }
  | { kind: "edit-table"; table: TableRef }
  | { kind: "duplicate-table"; table: TableRef }
  /// Truncate or drop a table, or drop a view (`tableKind` picks DROP VIEW).
  | { kind: "remove-table"; table: TableRef; tableKind: TableKind; action: "truncate" | "drop" }
  /// `namespace` says whether the user is working on a schema or a database.
  | { kind: "create-namespace"; namespace: NamespaceKind }
  | { kind: "rename-namespace"; namespace: NamespaceKind; name: string }
  | { kind: "drop-namespace"; namespace: NamespaceKind; name: string };

export interface SchemaActionHostProps {
  connectionId: string;
  /// null renders nothing.
  action: SchemaAction | null;
  onClose: () => void;
}
