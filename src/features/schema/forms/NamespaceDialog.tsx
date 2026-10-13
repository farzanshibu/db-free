// SOT: namespace-dialog, create-database-form, create-schema-form, rename-schema-form, drop-schema-form
import { useMemo, useState } from "react";
import type { DdlOperation, DdlProfile, NamespaceCaps, NamespaceKind, TableRef } from "@/lib/bindings";
import { Alert, AlertContent, AlertDescription, AlertIndicator } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Field, Toggle } from "@/components/global/Field";
import { useWorkspace } from "@/stores/workspace";
import { validateNewName } from "../ddlRules";
import { DdlDialog } from "./DdlDialog";
import { ProfileGate } from "./ProfileGate";
import { StatusDialog } from "./StatusDialog";
import { useDdlRunner, type DdlEffects } from "./useDdlRunner";

type NamespaceStep = "create" | "rename" | "drop";

interface NamespaceDialogProps {
  connectionId: string;
  namespace: NamespaceKind;
  step: NamespaceStep;
  /// The schema or database being renamed or dropped; null when creating.
  name: string | null;
  onClose: () => void;
}

export function NamespaceDialog(props: NamespaceDialogProps) {
  const noun = props.namespace;
  const title = `${props.step === "create" ? "New" : props.step === "rename" ? "Rename" : "Delete"} ${noun}`;
  return (
    <ProfileGate connectionId={props.connectionId} title={title} onClose={props.onClose}>
      {(profile) => {
        const caps = profile.namespaces.find((c) => c.kind === props.namespace);
        if (!caps?.[props.step]) {
          return (
            <StatusDialog
              title={title}
              description={`${profile.dialect} cannot ${props.step} a ${noun} from here.`}
              onClose={props.onClose}
              notes={[caps?.note ?? `${profile.dialect} has no ${noun} that the schema forms can ${props.step}.`]}
            />
          );
        }
        return <NamespaceForm profile={profile} caps={caps} title={title} {...props} />;
      }}
    </ProfileGate>
  );
}

// WHAT:  Creates, renames or drops a database or a schema.
// WHY:   Engines disagree on what exists (MySQL has no RENAME DATABASE, MongoDB
//        creates a database with its first collection), so the engine's
//        capabilities decide which forms open at all and carry the reason when one
//        does not; this form only runs for what the engine can do.
// HOW:   Dropping goes through the typed-name confirmation Rust asks for. After a
//        change the catalog and session info are reloaded (so the switchers list
//        the new names), tabs of tables that lived in a dropped or renamed schema
//        close, and a schema filter that pointed at the old name follows or clears.
// WHERE: src-tauri/src/integrations/ddl/ (namespace), src-tauri/src/model/ddl.rs (NamespaceCaps)
function NamespaceForm({ profile, caps, title, connectionId, namespace, step, name: target, onClose }: NamespaceDialogProps & { profile: DdlProfile; caps: NamespaceCaps; title: string }) {
  const noun = namespace;
  const [input, setInput] = useState(step === "rename" ? (target ?? "") : "");
  const [owner, setOwner] = useState("");
  const [ifNotExists, setIfNotExists] = useState(false);
  const [cascade, setCascade] = useState(false);
  const [ifExists, setIfExists] = useState(false);
  const catalog = useWorkspace((s) => s.catalogs[connectionId]);
  const info = useWorkspace((s) => s.sessionInfos[connectionId]);
  const filtered = useWorkspace((s) => s.schemaFilter[connectionId] ?? null);
  const setSchemaFilter = useWorkspace((s) => s.setSchemaFilter);

  const existing = useMemo(() => (namespace === "schema" ? (catalog?.schemas.map((s) => s.name) ?? []) : (info?.databases ?? [])), [namespace, catalog, info]);
  const label = noun.charAt(0).toUpperCase() + noun.slice(1);
  const unchanged = step === "rename" && target !== null && input === target;
  const issue = step === "drop" || unchanged ? null : validateNewName(label, input, existing);
  const ready = issue === null && !unchanged && (step === "create" || target !== null);

  const operation = useMemo<DdlOperation | null>(() => {
    if (step === "drop") {
      return target === null ? null : { op: "namespace", spec: { kind: namespace, name: target, op: { op: "drop", cascade, ifExists } } };
    }
    if (!ready) return null;
    if (step === "create") {
      return { op: "namespace", spec: { kind: namespace, name: input, op: { op: "create", owner: caps.owner && owner.trim() !== "" ? owner.trim() : null, ifNotExists } } };
    }
    return target === null ? null : { op: "namespace", spec: { kind: namespace, name: target, op: { op: "rename", to: input } } };
  }, [step, ready, namespace, target, input, owner, caps.owner, ifNotExists, cascade, ifExists]);

  // Tables that lived in a schema that is dropped or renamed away are gone from their old name.
  const orphaned: TableRef[] =
    namespace === "schema" && step !== "create" && target !== null
      ? (catalog?.schemas.find((s) => s.name === target)?.tables.map((t) => ({ schema: t.schema, name: t.name })) ?? [])
      : [];
  const pointedAtTarget = namespace === "schema" && target !== null && filtered === target;
  const effects: DdlEffects = {
    message: step === "create" ? `Created ${noun} ${input}.` : step === "rename" ? `Renamed ${noun} ${target ?? ""} to ${input}.` : `Deleted ${noun} ${target ?? ""}.`,
    removed: orphaned,
    refresh: [],
    tagMoves: [],
    open: null,
    after: () => {
      if (pointedAtTarget) setSchemaFilter(connectionId, step === "rename" ? input : null);
    },
  };
  const runner = useDdlRunner(connectionId, operation, effects, onClose);

  const connectedHere = namespace === "database" && step === "drop" && target !== null && info?.database === target;

  return (
    <DdlDialog
      connectionId={connectionId}
      title={title}
      description={
        step === "create"
          ? `Create a ${noun} on this server.`
          : step === "rename"
            ? `Give the ${noun} ${target ?? ""} a new name.`
            : `Permanently delete the ${noun} ${target ?? ""}${namespace === "schema" ? " and, with CASCADE, everything in it" : " and everything in it"}.`
      }
      runner={runner}
      ready={operation !== null}
      idleHint={step === "create" ? `Name the ${noun} to see the statements.` : "Choose a new name to see the statements."}
      engineNotes={[...(caps.note !== null ? [caps.note] : []), ...profile.notes]}
      runLabel={title}
      onClose={onClose}
    >
      {step !== "create" && target !== null ? (
        <div className="flex items-center gap-2 text-xs">
          <span className="text-muted">{label}</span>
          <Badge variant="secondary" className="font-mono">
            {target}
          </Badge>
        </div>
      ) : null}
      {step === "create" || step === "rename" ? (
        <Field
          label={step === "create" ? `${label} name` : "New name"}
          value={input}
          onChange={setInput}
          mono
          autoFocus
          description={input !== "" && issue !== null ? issue : undefined}
          className={input !== "" && issue !== null ? "[&_p]:text-danger" : ""}
        />
      ) : null}
      {step === "create" && caps.owner ? <Field label="Owner" value={owner} onChange={setOwner} mono optional placeholder="role name" /> : null}
      {step === "create" ? <Toggle checked={ifNotExists} onChange={setIfNotExists} label={`Skip if the ${noun} already exists (IF NOT EXISTS)`} /> : null}
      {step === "drop" ? (
        <div className="flex flex-col gap-2">
          {connectedHere ? (
            <Alert variant="warning">
              <AlertIndicator />
              <AlertContent>
                <AlertDescription>You are connected to this database. Most servers refuse to drop it; switch to another database first.</AlertDescription>
              </AlertContent>
            </Alert>
          ) : null}
          <Toggle
            checked={cascade}
            onChange={setCascade}
            label="Also delete everything inside it (CASCADE)"
            description={cascade ? "Every table, view and object in it is deleted with it." : "Without CASCADE the server refuses while the schema is not empty, where it supports the keyword."}
          />
          <Toggle checked={ifExists} onChange={setIfExists} label="Skip if it does not exist (IF EXISTS)" />
        </div>
      ) : null}
    </DdlDialog>
  );
}
