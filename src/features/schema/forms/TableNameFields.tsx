// SOT: table-name-fields, schema-picker-field
import { Field, AppSelect, type Option } from "@/components/global/Field";
import { useWorkspace } from "@/stores/workspace";

interface TableNameFieldsProps {
  connectionId: string;
  /// "Table", "Collection"…
  noun: string;
  name: string;
  onName: (name: string) => void;
  /// Shown under the name once something has been typed.
  nameIssue: string | null;
  /// Why the name cannot be changed (Edit on an engine that cannot rename), or null.
  nameLock?: string | null;
  schema: string | null;
  onSchema?: ((schema: string) => void) | undefined;
  autoFocus?: boolean;
  placeholder?: string;
}

// WHAT:  The name of the table a form creates / copies / renames, and the schema
//        it goes into when the engine has several.
// WHY:   Create, Edit and Duplicate all start from this pair; the schema picker only
//        appears when there is a choice, and never on a form that cannot move a table.
// HOW:   The schemas are the sidebar's own catalog, so the picker lists exactly what
//        the tree shows. Without `onSchema` the schema is displayed, not editable.
// WHERE: src/features/schema/forms/CreateTableDialog.tsx
export function TableNameFields({ connectionId, noun, name, onName, nameIssue, nameLock = null, schema, onSchema, autoFocus = false, placeholder }: TableNameFieldsProps) {
  const schemas = useWorkspace((s) => s.catalogs[connectionId]?.schemas ?? []);
  const options: Option<string>[] = schemas.map((s) => ({ value: s.name, label: s.name }));
  const choosable = onSchema !== undefined && schemas.length > 1;
  return (
    <div className="grid grid-cols-1 gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,14rem)]">
      <div title={nameLock ?? undefined}>
        <Field
          label={`${noun} name`}
          value={name}
          onChange={onName}
          mono
          autoFocus={autoFocus}
          disabled={nameLock !== null}
          {...(placeholder !== undefined ? { placeholder } : {})}
          description={nameLock ?? (name !== "" && nameIssue !== null ? nameIssue : undefined)}
          className={name !== "" && nameIssue !== null && nameLock === null ? "[&_p]:text-danger" : ""}
        />
      </div>
      {choosable && schema !== null ? <AppSelect label="Schema" value={schema} options={options} onChange={onSchema} /> : null}
    </div>
  );
}
