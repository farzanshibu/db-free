// SOT: create-table-dialog, create-collection-form, new-table-form
import { useMemo, useState } from "react";
import type { DdlOperation, DdlProfile, TableRef } from "@/lib/bindings";
import { AppSelect, Toggle, type Option } from "@/components/global/Field";
import { Disclosure } from "@/components/global/Disclosure";
import { AutoGrid } from "@/components/global/Layout";
import { blankDraft, hasColumns, nounOf, qualifiedLabel, toTableSpec, withColumns, type TableDraft } from "../ddlForm";
import { validateDraft } from "../ddlRules";
import { ColumnGrid } from "./ColumnGrid";
import { DdlDialog } from "./DdlDialog";
import { ForeignKeyEditor } from "./ForeignKeyEditor";
import { MultiPick } from "./MultiPick";
import { ProfileGate } from "./ProfileGate";
import { TableNameFields } from "./TableNameFields";
import { useDdlRunner, type DdlEffects } from "./useDdlRunner";

interface CreateTableDialogProps {
  connectionId: string;
  /// The schema the new table goes into (null = the engine's only namespace).
  schema: string | null;
  onClose: () => void;
}

export function CreateTableDialog({ connectionId, schema, onClose }: CreateTableDialogProps) {
  return (
    <ProfileGate connectionId={connectionId} title="New table" onClose={onClose}>
      {(profile) => <CreateForm profile={profile} connectionId={connectionId} schema={schema} onClose={onClose} />}
    </ProfileGate>
  );
}

// WHAT:  Collects a name, columns, foreign keys and (ClickHouse) an engine and
//        ORDER BY, previews the CREATE statement the engine's builder writes,
//        and runs it.
// WHY:   Creation used to open an unquoted, hand-assembled statement in a query
//        tab. The typed spec lets Rust quote per dialect, spell types and
//        auto-increment correctly, and refuse what the engine cannot do.
// HOW:   A collection store (no column types) asks for a name only. The preview
//        waits until the form validates, so the statements never flash an error
//        for a half-typed form; `validateDraft` explains what is missing.
// WHERE: src-tauri/src/integrations/ddl/ (create_table), src/features/schema/ddlForm.ts
function CreateForm({ profile, connectionId, schema, onClose }: CreateTableDialogProps & { profile: DdlProfile }) {
  const [draft, setDraft] = useState<TableDraft>(() => blankDraft(profile, schema));
  const issues = useMemo(() => validateDraft(profile, draft, null), [profile, draft]);
  const noun = nounOf(profile);
  const ready = draft.name !== "" && issues.count === 0;
  const ref: TableRef = { schema: draft.schema, name: draft.name };

  const operation = useMemo<DdlOperation | null>(() => (ready ? { op: "create_table", spec: toTableSpec(profile, draft) } : null), [ready, profile, draft]);
  const effects: DdlEffects = { message: `Created ${qualifiedLabel(ref)}.`, removed: [], refresh: [], tagMoves: [], open: ref };
  const runner = useDdlRunner(connectionId, operation, effects, onClose);

  const columnNames = draft.columns.filter((c) => !c.removed).map((c) => c.name);
  const engineOptions: Option<string>[] = profile.engines.map((e) => ({ value: e, label: e }));
  // Storage (engine, ORDER BY) and IF NOT EXISTS are rarely changed: they sit
  // under Advanced, opened when one differs from what `blankDraft` set.
  const tableEngine = hasColumns(profile) && profile.support.tableEngine;
  const advancedCount = tableEngine ? 3 : 1;
  const advancedChanged = draft.ifNotExists || draft.orderBy.length > 0 || (tableEngine && draft.engine !== (profile.engines[0] ?? null));

  return (
    <DdlDialog
      connectionId={connectionId}
      title={`New ${noun}`}
      description={`Written for ${profile.dialect}, previewed before it runs.`}
      wide={hasColumns(profile)}
      runner={runner}
      ready={ready}
      idleHint={draft.name === "" ? `Name the ${noun} to see the statements.` : "Fix the highlighted fields to see the statements."}
      engineNotes={profile.notes}
      runLabel={`Create ${noun}`}
      onClose={onClose}
    >
      <TableNameFields
        connectionId={connectionId}
        noun={profile.noun}
        name={draft.name}
        onName={(name) => { setDraft((d) => ({ ...d, name })); }}
        nameIssue={issues.table}
        schema={draft.schema}
        onSchema={(next) => { setDraft((d) => ({ ...d, schema: next })); }}
        autoFocus
        placeholder={`my_${noun}`}
      />
      {hasColumns(profile) ? (
        <>
          <ColumnGrid profile={profile} columns={draft.columns} onChange={(next) => { setDraft((d) => withColumns(d, next)); }} issues={issues.columns} edit={false} />
          {profile.support.foreignKeys ? (
            <ForeignKeyEditor
              connectionId={connectionId}
              own={ref}
              columnNames={columnNames}
              foreignKeys={draft.foreignKeys}
              onChange={(foreignKeys) => { setDraft((d) => ({ ...d, foreignKeys })); }}
              issues={issues.foreignKeys}
              lock={null}
            />
          ) : null}
        </>
      ) : (
        <p className="text-xs text-muted">No fixed columns: it fills as documents are inserted.</p>
      )}
      <Disclosure label="Advanced" count={advancedCount} defaultOpen={advancedChanged}>
        {tableEngine ? (
          <section aria-label="Table engine">
            <AutoGrid min="sm">
              <AppSelect label="Engine" value={draft.engine ?? ""} options={engineOptions} onChange={(engine) => { setDraft((d) => ({ ...d, engine })); }} />
              <div className="flex flex-col gap-1.5">
                <span className="text-xs font-medium text-muted">Order by</span>
                <MultiPick options={columnNames} value={draft.orderBy} onChange={(orderBy) => { setDraft((d) => ({ ...d, orderBy })); }} ariaLabel="Order by columns" placeholder="Primary key columns" />
              </div>
            </AutoGrid>
          </section>
        ) : null}
        <Toggle checked={draft.ifNotExists} onChange={(ifNotExists) => { setDraft((d) => ({ ...d, ifNotExists })); }} label={`Skip if the ${noun} already exists (IF NOT EXISTS)`} />
      </Disclosure>
      {issues.general.map((message) => (
        <p key={message} className="text-[11px] text-danger">
          {message}
        </p>
      ))}
    </DdlDialog>
  );
}
