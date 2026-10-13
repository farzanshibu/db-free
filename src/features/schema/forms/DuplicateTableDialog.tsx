// SOT: duplicate-table-dialog, copy-table-form
import { useMemo, useState } from "react";
import type { DdlOperation, DdlProfile, TableRef } from "@/lib/bindings";
import { Alert, AlertContent, AlertDescription, AlertIndicator } from "@/components/ui/alert";
import { Toggle } from "@/components/global/Field";
import { nounOf, qualifiedLabel } from "../ddlForm";
import { validateNewName } from "../ddlRules";
import { useWorkspace } from "@/stores/workspace";
import { DdlDialog } from "./DdlDialog";
import { ProfileGate } from "./ProfileGate";
import { TableNameFields } from "./TableNameFields";
import { useDdlRunner, type DdlEffects } from "./useDdlRunner";

interface DuplicateTableDialogProps {
  connectionId: string;
  table: TableRef;
  onClose: () => void;
}

export function DuplicateTableDialog({ connectionId, table, onClose }: DuplicateTableDialogProps) {
  return (
    <ProfileGate connectionId={connectionId} title={`Duplicate ${table.name}`} onClose={onClose}>
      {(profile) => <DuplicateForm profile={profile} connectionId={connectionId} table={table} onClose={onClose} />}
    </ProfileGate>
  );
}

// WHAT:  Copies a table under a new name: its structure, optionally its rows and
//        its indexes.
// WHY:   "Duplicate" used to open `CREATE TABLE … AS SELECT *`, which drops every
//        key, default and index. The builder reads the source's real structure and
//        writes the copy from it, so the duplicate behaves like the original.
// HOW:   The form sends the source, the target and two switches; Rust describes the
//        source itself, so the form never holds a stale copy of its columns.
// WHERE: src-tauri/src/services/schema.rs (preview_ddl), src-tauri/src/integrations/ddl/ (duplicate_table)
function DuplicateForm({ profile, connectionId, table, onClose }: DuplicateTableDialogProps & { profile: DdlProfile }) {
  const noun = nounOf(profile);
  const [name, setName] = useState(`${table.name}_copy`);
  const [schema, setSchema] = useState(table.schema);
  const [includeData, setIncludeData] = useState(true);
  const [includeIndexes, setIncludeIndexes] = useState(true);
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const taken = useMemo(() => (catalog?.schemas ?? []).flatMap((s) => s.tables.filter((t) => t.schema === schema).map((t) => t.name)), [catalog, schema]);
  const issue = validateNewName(profile.noun, name, taken);
  const target: TableRef = { schema, name };
  const supported = profile.support.duplicateTable;
  const ready = supported && issue === null;

  const operation = useMemo<DdlOperation | null>(
    () => (ready ? { op: "duplicate_table", spec: { source: table, target: { schema, name }, includeData, includeIndexes } } : null),
    [ready, table, schema, name, includeData, includeIndexes],
  );
  const effects: DdlEffects = { message: `Created ${qualifiedLabel(target)} from ${qualifiedLabel(table)}.`, removed: [], refresh: [], tagMoves: [], open: target };
  const runner = useDdlRunner(connectionId, operation, effects, onClose);

  return (
    <DdlDialog
      connectionId={connectionId}
      title={`Duplicate ${noun}`}
      description={`Copy ${qualifiedLabel(table)} to a new ${noun}.`}
      runner={runner}
      ready={ready}
      idleHint={supported ? "Choose a new name to see the statements." : `${profile.dialect} cannot duplicate a ${noun} from here.`}
      engineNotes={profile.notes}
      runLabel={`Duplicate ${noun}`}
      onClose={onClose}
    >
      {supported ? (
        <>
          <TableNameFields connectionId={connectionId} noun={`New ${noun}`} name={name} onName={setName} nameIssue={issue} schema={schema} onSchema={setSchema} autoFocus />
          <div className="flex flex-col gap-2">
            <Toggle checked={includeData} onChange={setIncludeData} label="Copy the rows" description="Off creates an empty copy with the same structure." />
            <Toggle checked={includeIndexes} onChange={setIncludeIndexes} label="Copy the indexes" />
          </div>
        </>
      ) : (
        <Alert variant="warning">
          <AlertIndicator />
          <AlertContent>
            <AlertDescription>
              {profile.dialect} has no way to duplicate a {noun} from the schema forms.
            </AlertDescription>
          </AlertContent>
        </Alert>
      )}
    </DdlDialog>
  );
}
