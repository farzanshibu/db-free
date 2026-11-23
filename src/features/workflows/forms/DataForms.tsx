// SOT: workflow-query-form, workflow-http-form, workflow-read-file-form, workflow-connection-choice
import { useEffect, useId, useMemo } from "react";
import type { ConnectionSummary, FileFormat, HttpMethod } from "@/lib/bindings";
import { AppSelect, Field, FieldLabel, Toggle } from "@/components/global/Field";
import { Disclosure } from "@/components/global/Disclosure";
import { SqlEditor } from "@/features/editor/SqlEditor";
import { buildCompletionSchema, mentionedTables } from "@/features/dashboards/completionSchema";
import { useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { pickDataFile } from "../pickers";
import { TemplateHint, type FormProps } from "./common";

/// The connection select: "the workflow's own" first, then every saved connection.
export function connectionOptions(connections: readonly ConnectionSummary[], workflowDefault: string): { value: string; label: string }[] {
  const own = connections.find((c) => c.id === workflowDefault);
  return [{ value: "", label: own ? `Workflow default (${own.name})` : "Workflow default" }, ...connections.map((c) => ({ value: c.id, label: c.name }))];
}

// WHAT:  The Query node: which connection, the SQL, and the destructive opt-in.
// WHY:   The SQL runs through the same guard as the editor's: read-only connections
//        refuse writes, and a destructive statement (DROP, TRUNCATE, DELETE without
//        WHERE …) runs only when "Allow destructive statements" is ticked here and
//        the workflow is saved, never because a run was confirmed once.
// HOW:   Completion offers the chosen connection's tables, with real columns for
//        the tables the SQL already names.
export function QueryForm({ config, onChange, context }: FormProps<"query">) {
  const connections = useWorkspace((s) => s.connections);
  const catalogs = useWorkspace((s) => s.catalogs);
  const columnsCache = useWorkspace((s) => s.columnsCache);
  const loadColumns = useWorkspace((s) => s.loadColumns);
  const effective = config.connectionId ?? (context.workflowConnectionId === "" ? null : context.workflowConnectionId);
  const engine = connections.find((c) => c.id === effective)?.engine ?? "postgres";
  const catalog = effective === null ? undefined : catalogs[effective];

  const tables = useMemo(() => mentionedTables(config.sql, catalog), [config.sql, catalog]);
  const tablesKey = tables.map((t) => `${t.schema ?? ""}.${t.name}`).join("|");
  useEffect(() => {
    if (effective === null) return;
    for (const table of tables) void loadColumns(effective, table).catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `tables` is keyed by `tablesKey`
  }, [effective, tablesKey, loadColumns]);
  const schema = useMemo(() => (effective === null ? {} : buildCompletionSchema(effective, catalog, columnsCache)), [effective, catalog, columnsCache]);

  return (
    <div className="flex flex-col gap-2">
      <AppSelect
        label="Connection"
        icon="database"
        value={config.connectionId ?? ""}
        options={connectionOptions(connections, context.workflowConnectionId)}
        onChange={(value) => { onChange({ ...config, connectionId: value === "" ? null : value }); }}
      />
      <div className="flex flex-col gap-1.5">
        <Label>SQL</Label>
        <div className="h-60 overflow-hidden rounded-md border border-border">
          <SqlEditor value={config.sql} onChange={(sql) => { onChange({ ...config, sql }); }} onRun={() => { context.runNode(); }} engine={engine} schema={schema} />
        </div>
        <TemplateHint>
          Use <code className="font-mono text-foreground">{"{{column}}"}</code> to bring in a value from the first row that reaches this node. Values are written as safe SQL literals for the connection:{" "}
          <code className="font-mono text-foreground">{"WHERE id = {{id}}"}</code> and <code className="font-mono text-foreground">{"WHERE name = '{{name}}'"}</code> both work. Arithmetic too:{" "}
          <code className="font-mono text-foreground">{"OFFSET {{i * 100}}"}</code>.
        </TemplateHint>
      </div>
      <Disclosure label="Advanced" count={1} defaultOpen={config.allowDestructive}>
        <Toggle
          checked={config.allowDestructive}
          onChange={(allowDestructive) => { onChange({ ...config, allowDestructive }); }}
          label="Allow destructive statements"
          tip="Without this the node refuses DROP, TRUNCATE and DELETE / UPDATE without WHERE. Read-only connections refuse them regardless."
        />
      </Disclosure>
      {config.allowDestructive ? (
        <p className="flex items-center gap-1.5 text-[11px] text-warning">
          <Icon name="alert" size={12} />
          This node can delete or overwrite data whenever the workflow runs, including from a schedule.
        </p>
      ) : null}
    </div>
  );
}

const METHODS: readonly { value: HttpMethod; label: string }[] = [
  { value: "get", label: "GET" },
  { value: "post", label: "POST" },
  { value: "put", label: "PUT" },
  { value: "patch", label: "PATCH" },
  { value: "delete", label: "DELETE" },
  { value: "head", label: "HEAD" },
];

// WHAT:  The HTTP node: a templated request whose JSON answer becomes rows.
export function HttpForm({ config, onChange }: FormProps<"http">) {
  const hasBody = config.method !== "get" && config.method !== "head";
  const bodyId = useId();
  const headersId = useId();
  return (
    <div className="flex flex-col gap-2">
      <AppSelect label="Method" value={config.method} options={METHODS} onChange={(method) => { onChange({ ...config, method }); }} />
      <Field label="URL" mono value={config.url} placeholder="https://api.example.com/items?page={{i + 1}}" onChange={(url) => { onChange({ ...config, url }); }} />
      {hasBody ? (
        <div className="flex flex-col gap-1.5">
          <FieldLabel htmlFor={bodyId} label="Body" tip="Set Content-Type in the headers when you send a body." />
          <Textarea id={bodyId} value={config.body} rows={5} spellCheck={false} placeholder={'{"name": "{{name}}"}'} onChange={(event) => { onChange({ ...config, body: event.target.value }); }} />
        </div>
      ) : null}
      <Field label="Response path" optional mono value={config.responsePath} placeholder="data.results" tip="Where the rows are in the JSON answer. Blank uses the whole response." onChange={(responsePath) => { onChange({ ...config, responsePath }); }} />
      <TemplateHint>The URL, the header values and the body can use placeholders from the first input row.</TemplateHint>
      <Disclosure label="Advanced" count={2} defaultOpen={config.headers.trim().length > 0 || config.allowPrivateNetwork}>
        <div className="flex flex-col gap-1.5">
          <FieldLabel htmlFor={headersId} label="Headers" tip="A JSON object. Values accept placeholders." />
          <Textarea id={headersId} value={config.headers} rows={3} spellCheck={false} placeholder={'{"Authorization": "Bearer {{token}}"}'} onChange={(event) => { onChange({ ...config, headers: event.target.value }); }} />
        </div>
        <Toggle
          checked={config.allowPrivateNetwork}
          onChange={(allowPrivateNetwork) => { onChange({ ...config, allowPrivateNetwork }); }}
          label="Allow private network addresses"
          tip="Lets this node call localhost, private-network and cloud-metadata (169.254.169.254) addresses. Leave it off unless you wrote this workflow and need a local service."
        />
      </Disclosure>
    </div>
  );
}

const READ_FORMATS: readonly { value: FileFormat; label: string }[] = [
  { value: "csv", label: "CSV" },
  { value: "tsv", label: "TSV" },
  { value: "json", label: "JSON" },
  { value: "ndjson", label: "NDJSON" },
];

export function ReadFileForm({ config, onChange }: FormProps<"read_file">) {
  const browse = async () => {
    const path = await pickDataFile(config.format);
    if (path !== null) onChange({ ...config, path });
  };
  return (
    <div className="flex flex-col gap-2">
      <AppSelect
        label="Format"
        tip="CSV / TSV: the first row is the header; quoted fields may hold the delimiter, quotes and line breaks; an empty field is NULL. JSON: an array of objects, or one object. NDJSON: one object per line. The file is read again on every run."
        value={config.format}
        options={READ_FORMATS}
        onChange={(format) => { onChange({ ...config, format }); }}
      />
      <div className="flex items-end gap-2">
        <Field label="File" mono value={config.path} placeholder="/path/to/data.csv" onChange={(path) => { onChange({ ...config, path }); }} />
        <Button variant="secondary" size="md" onClick={() => void browse()}>
          <Icon name="folder" size={14} />
          Browse
        </Button>
      </div>
    </div>
  );
}
