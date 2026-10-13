// SOT: workflow-save-file-form, workflow-insert-form
import { useId } from "react";
import type { SaveFormat } from "@/lib/bindings";
import { AppSelect, Field } from "@/components/global/Field";
import { useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { pickOutputFile, SAVE_EXTENSIONS } from "../pickers";
import { connectionOptions } from "./DataForms";
import type { FormProps } from "./common";

const SAVE_FORMATS: readonly { value: SaveFormat; label: string }[] = [
  { value: "csv", label: "CSV" },
  { value: "tsv", label: "TSV" },
  { value: "json", label: "JSON" },
  { value: "ndjson", label: "NDJSON (one object per line)" },
  { value: "markdown", label: "Markdown table" },
  { value: "sql", label: "SQL INSERT statements" },
];

/// A path with its extension swapped for the new format's, so changing the format keeps the folder and the name.
function withExtension(path: string, format: SaveFormat): string {
  if (path.trim() === "") return path;
  const stripped = path.replace(/\.[A-Za-z0-9]{1,6}$/, "");
  return `${stripped}.${SAVE_EXTENSIONS[format]}`;
}

// WHAT:  The Save to file node: format and destination.
// WHY:   Left blank, the path is asked for the first time the workflow is run from
//        the editor (a save dialog) and remembered after that; a scheduled run never
//        asks, so a blank path there is an error.
export function SaveFileForm({ config, onChange }: FormProps<"save_file">) {
  const browse = async () => {
    const path = await pickOutputFile(config.path.split(/[\\/]/).at(-1) ?? "output", config.format);
    if (path !== null) onChange({ ...config, path });
  };
  return (
    <div className="flex flex-col gap-2">
      <AppSelect label="Format" value={config.format} options={SAVE_FORMATS} onChange={(format) => { onChange({ ...config, format, path: withExtension(config.path, format) }); }} />
      <div className="flex items-end gap-2">
        <Field label="File" mono value={config.path} placeholder="Asked on the first run" onChange={(path) => { onChange({ ...config, path }); }} />
        <Button variant="secondary" size="md" onClick={() => void browse()}>
          <Icon name="folder" size={14} />
          Browse
        </Button>
      </div>
      {config.format === "sql" ? (
        <Field label="Table name" optional mono value={config.tableName} placeholder="The file's name" onChange={(tableName) => { onChange({ ...config, tableName }); }} />
      ) : null}
      <p className="text-[11px] leading-relaxed text-muted">The file is replaced on every run. Several inputs are merged: columns are united and a missing value is NULL.</p>
    </div>
  );
}

// WHAT:  The Insert node: the table the rows go into.
// WHY:   Fields are matched to the table's columns by name; fields the table has no
//        column for are skipped. Rows go in one at a time and the first failure
//        stops the node with its row number.
export function InsertForm({ config, onChange, context }: FormProps<"insert">) {
  const connections = useWorkspace((s) => s.connections);
  const catalogs = useWorkspace((s) => s.catalogs);
  const listId = useId();
  const effective = config.connectionId ?? (context.workflowConnectionId === "" ? null : context.workflowConnectionId);
  const tables = (effective === null ? undefined : catalogs[effective])?.schemas.flatMap((s) => s.tables.filter((t) => t.kind === "table").map((t) => t.name)) ?? [];
  return (
    <div className="flex flex-col gap-2">
      <AppSelect
        label="Connection"
        icon="database"
        value={config.connectionId ?? ""}
        options={connectionOptions(connections, context.workflowConnectionId)}
        onChange={(value) => { onChange({ ...config, connectionId: value === "" ? null : value }); }}
      />
      <Field label="Schema" optional mono value={config.schema} placeholder="Default schema" onChange={(schema) => { onChange({ ...config, schema }); }} />
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={`${listId}-input`}>Table</Label>
        <Input id={`${listId}-input`} list={listId} className="font-mono" value={config.table} placeholder="orders" onChange={(event) => { onChange({ ...config, table: event.target.value }); }} />
        <datalist id={listId}>
          {tables.map((name) => (
            <option key={name} value={name} />
          ))}
        </datalist>
      </div>
      <p className="text-[11px] leading-relaxed text-muted">
        The table must already exist (create it with a Query node first if needed). Writes follow the connection's read-only lock.
      </p>
    </div>
  );
}
