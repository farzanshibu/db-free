// SOT: remove-table-dialog, truncate-table-form, drop-table-form, drop-view-form
import { useMemo, useState } from "react";
import type { DdlOperation, DdlProfile, TableKind, TableRef } from "@/lib/bindings";
import { Alert, AlertContent, AlertDescription, AlertIndicator } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Toggle } from "@/components/global/Field";
import { nounOf, qualifiedLabel } from "../ddlForm";
import { DdlDialog } from "./DdlDialog";
import { ProfileGate } from "./ProfileGate";
import { useDdlRunner, type DdlEffects } from "./useDdlRunner";

interface RemoveTableDialogProps {
  connectionId: string;
  table: TableRef;
  tableKind: TableKind;
  action: "truncate" | "drop";
  onClose: () => void;
}

export function RemoveTableDialog(props: RemoveTableDialogProps) {
  const word = props.action === "truncate" ? "Empty" : "Delete";
  return (
    <ProfileGate connectionId={props.connectionId} title={`${word} ${props.table.name}`} onClose={props.onClose}>
      {(profile) => <RemoveForm profile={profile} {...props} />}
    </ProfileGate>
  );
}

// WHAT:  Empties a table, or deletes a table or view, after showing the statement
//        and making the user type its name.
// WHY:   These are the two operations that lose data without a way back, so they
//        always go through the typed-name confirmation Rust asks for, and never
//        run from a one-click menu entry.
// HOW:   Wording follows the object (a view is "deleted", a table "emptied") and
//        the engine's noun (a Mongo collection). Dropping closes the table's tabs
//        and forgets its tags; emptying refreshes an open tab in place.
function RemoveForm({ profile, connectionId, table, tableKind, action, onClose }: RemoveTableDialogProps & { profile: DdlProfile }) {
  const [cascade, setCascade] = useState(false);
  const [ifExists, setIfExists] = useState(false);
  const noun = tableKind === "view" ? "view" : nounOf(profile);
  const supported = action === "truncate" ? profile.support.truncateTable : profile.support.dropTable;
  const label = qualifiedLabel(table);

  const operation = useMemo<DdlOperation | null>(
    () => (supported ? { op: "remove_table", spec: { table, kind: tableKind, action, cascade, ifExists } } : null),
    [supported, table, tableKind, action, cascade, ifExists],
  );
  const effects: DdlEffects = {
    message: action === "truncate" ? `Emptied ${label}.` : `Deleted ${label}.`,
    removed: action === "drop" ? [table] : [],
    refresh: action === "truncate" ? [table] : [],
    tagMoves: action === "drop" ? [{ schema: table.schema, from: table.name, to: null }] : [],
    open: null,
  };
  const runner = useDdlRunner(connectionId, operation, effects, onClose);

  const title = action === "truncate" ? `Empty ${noun}` : `Delete ${noun}`;
  const description =
    action === "truncate"
      ? `Removes every row from ${label}. The ${noun} itself stays.`
      : `Permanently removes ${label}${tableKind === "view" ? "" : " and all of its data"}.`;

  return (
    <DdlDialog
      connectionId={connectionId}
      title={title}
      description={description}
      runner={runner}
      ready={supported}
      idleHint={`${profile.dialect} cannot ${action === "truncate" ? "empty" : "delete"} a ${noun} from here.`}
      engineNotes={profile.notes}
      runLabel={title}
      onClose={onClose}
    >
      <div className="flex items-center gap-2 text-xs">
        <span className="text-muted">{tableKind === "view" ? "View" : profile.noun}</span>
        <Badge variant="secondary" className="font-mono">
          {label}
        </Badge>
      </div>
      {supported ? (
        <div className="flex flex-col gap-2">
          <Toggle
            checked={cascade}
            onChange={setCascade}
            label={action === "truncate" ? "Also empty tables that reference it (CASCADE)" : "Also delete objects that depend on it (CASCADE)"}
            description="Only where the engine has CASCADE; the statement below shows what will run."
          />

          {action === "drop" ? <Toggle checked={ifExists} onChange={setIfExists} label="Skip if it does not exist (IF EXISTS)" /> : null}
        </div>
      ) : (
        <Alert variant="warning">
          <AlertIndicator />
          <AlertContent>
            <AlertDescription>
              {profile.dialect} cannot {action === "truncate" ? "empty" : "delete"} a {noun} from the schema forms.
            </AlertDescription>
          </AlertContent>
        </Alert>
      )}
    </DdlDialog>
  );
}
