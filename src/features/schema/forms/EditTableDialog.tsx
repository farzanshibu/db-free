// SOT: edit-table-dialog, alter-table-form, table-structure-editor
import { useMemo, useState } from "react";
import type { DdlOperation, DdlProfile, TableRef, TableSpec } from "@/lib/bindings";
import { Alert, AlertContent, AlertDescription, AlertIndicator } from "@/components/ui/alert";
import { fromTableSpec, hasColumns, isDirty, nounOf, qualifiedLabel, toTableSpec, withColumns, type TableDraft } from "../ddlForm";
import { foreignKeyLock, renameTableLock, validateDraft } from "../ddlRules";
import { ColumnGrid } from "./ColumnGrid";
import { DdlDialog } from "./DdlDialog";
import { ForeignKeyEditor } from "./ForeignKeyEditor";
import { ProfileGate } from "./ProfileGate";
import { StatusDialog } from "./StatusDialog";
import { TableNameFields } from "./TableNameFields";
import { useDescribedTable } from "./useDescribedTable";
import { useDdlRunner, type DdlEffects } from "./useDdlRunner";

interface EditTableDialogProps {
  connectionId: string;
  table: TableRef;
  onClose: () => void;
}

export function EditTableDialog({ connectionId, table, onClose }: EditTableDialogProps) {
  return (
    <ProfileGate connectionId={connectionId} title={`Edit ${table.name}`} onClose={onClose}>
      {(profile) => <EditLoader profile={profile} connectionId={connectionId} table={table} onClose={onClose} />}
    </ProfileGate>
  );
}

function EditLoader({ profile, connectionId, table, onClose }: EditTableDialogProps & { profile: DdlProfile }) {
  const { spec, error, retry } = useDescribedTable(connectionId, table);
  if (spec === null) {
    return <StatusDialog title={`Edit ${table.name}`} description={`Reading ${qualifiedLabel(table)} from the server…`} onClose={onClose} error={error} onRetry={retry} />;
  }
  return <EditForm profile={profile} connectionId={connectionId} loaded={spec} onClose={onClose} />;
}

// WHAT:  Edits an existing table: rename it, add / drop / rename columns, change
//        type, nullability, default and uniqueness, add or drop foreign keys, and
//        previews the ALTER statements before running them.
// WHY:   The diff is computed in Rust from (the table as read, the table as the
//        form leaves it), so the form never has to know how a given engine spells
//        or orders an ALTER, or when it has to rebuild the table instead.
// HOW:   Rows keep their `originalName`, which is how a rename is told from a drop
//        plus an add. Controls the engine cannot honour on an existing column are
//        disabled with the reason; an engine that rebuilds (SQLite) locks nothing.
//        Run stays off until something differs from the table as it was read.
// WHERE: src-tauri/src/integrations/ddl/ (alter_table), src/features/schema/ddlForm.ts
function EditForm({ profile, connectionId, loaded, onClose }: { profile: DdlProfile; connectionId: string; loaded: TableSpec; onClose: () => void }) {
  const [baseline] = useState<TableDraft>(() => fromTableSpec(profile, loaded));
  const [draft, setDraft] = useState<TableDraft>(baseline);
  const noun = nounOf(profile);
  const dirty = isDirty(profile, baseline, draft);
  const issues = useMemo(() => validateDraft(profile, draft, loaded.table), [profile, draft, loaded.table]);
  const ready = dirty && issues.count === 0;
  const renamed = draft.name !== loaded.table.name;
  const ref: TableRef = { schema: loaded.table.schema, name: draft.name };

  const operation = useMemo<DdlOperation | null>(
    () => (ready ? { op: "alter_table", spec: { original: loaded, desired: toTableSpec(profile, draft) } } : null),
    [ready, loaded, profile, draft],
  );
  const effects: DdlEffects = {
    message: `Updated ${qualifiedLabel(ref)}.`,
    removed: renamed ? [loaded.table] : [],
    refresh: renamed ? [] : [loaded.table],
    tagMoves: renamed ? [{ schema: loaded.table.schema, from: loaded.table.name, to: draft.name }] : [],
    open: renamed ? ref : null,
  };
  const runner = useDdlRunner(connectionId, operation, effects, onClose);

  const columnNames = draft.columns.filter((c) => !c.removed).map((c) => c.name);
  const fkLock = foreignKeyLock(profile);

  return (
    <DdlDialog
      connectionId={connectionId}
      title={`Edit ${noun}`}
      description={`${qualifiedLabel(loaded.table)} — change the structure; the ALTER statements are shown below before they run.`}
      wide={hasColumns(profile)}
      runner={runner}
      ready={ready}
      idleHint={dirty ? "Fix the highlighted fields to see the statements." : "Nothing to change yet. Edit a column or key to see the statements."}
      engineNotes={profile.notes}
      runLabel="Apply changes"
      onClose={onClose}
    >
      {profile.support.rebuilds ? (
        <Alert variant="default">
          <AlertIndicator />
          <AlertContent>
            <AlertDescription>
              {profile.dialect} cannot ALTER every part of a {noun}. Changes it cannot make in place are applied by rebuilding the {noun}: its rows are copied into a new one that replaces the original.
            </AlertDescription>
          </AlertContent>
        </Alert>
      ) : null}
      <TableNameFields
        connectionId={connectionId}
        noun={profile.noun}
        name={draft.name}
        onName={(name) => { setDraft((d) => ({ ...d, name })); }}
        nameIssue={issues.table}
        nameLock={renameTableLock(profile)}
        schema={draft.schema}
      />
      {hasColumns(profile) ? (
        <>
          <ColumnGrid profile={profile} columns={draft.columns} onChange={(next) => { setDraft((d) => withColumns(d, next)); }} issues={issues.columns} edit />
          {profile.support.foreignKeys || draft.foreignKeys.length > 0 ? (
            <ForeignKeyEditor
              connectionId={connectionId}
              own={ref}
              columnNames={columnNames}
              foreignKeys={draft.foreignKeys}
              onChange={(foreignKeys) => { setDraft((d) => ({ ...d, foreignKeys })); }}
              issues={issues.foreignKeys}
              lock={fkLock}
            />
          ) : null}
        </>
      ) : (
        <p className="text-xs text-muted">A {noun} has no fixed columns; only its name can be changed here.</p>
      )}
      {draft.indexes.length > 0 ? (
        <p className="text-xs text-muted">
          {draft.indexes.length === 1 ? "1 index is" : `${String(draft.indexes.length)} indexes are`} kept as they are: {draft.indexes.map((i) => i.name).join(", ")}.
        </p>
      ) : null}
      {issues.general.map((message) => (
        <p key={message} className="text-[11px] text-danger">
          {message}
        </p>
      ))}
    </DdlDialog>
  );
}
