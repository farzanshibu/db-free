// SOT: workflow-output-panel, node-output-preview, node-console, node-run-status
import { useState } from "react";
import type { NodePreview, NodeUpdate } from "@/lib/bindings";
import { DENSITIES, formatCount, formatMs } from "@/lib/format";
import { useWorkspace } from "@/stores/workspace";
import { DataGrid } from "@/features/grid/DataGrid";
import { Segmented } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Alert, AlertContent, AlertDescription, AlertIndicator } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";

const STATE_LABEL = {
  running: "Running",
  success: "Succeeded",
  error: "Failed",
  skipped: "Skipped",
  stopped: "Stopped",
} as const;

const STATE_VARIANT = {
  running: "soft",
  success: "success",
  error: "danger",
  skipped: "secondary",
  stopped: "warning",
} as const;

const LOG_TONE = {
  log: "text-foreground",
  info: "text-foreground",
  debug: "text-muted",
  warn: "text-warning",
  error: "text-danger",
} satisfies Record<string, string>;

function toneOf(level: string): string {
  return level === "warn" || level === "error" || level === "debug" ? LOG_TONE[level] : LOG_TONE.log;
}

function Preview({ preview }: { preview: NodePreview }) {
  const density = useWorkspace((s) => s.density);
  if (preview.columns.length === 0) {
    return <p className="rounded-md bg-surface-secondary/50 px-2.5 py-2.5 text-center text-xs text-muted">No columns: this output is empty.</p>;
  }
  const shown = preview.rows.length;
  return (
    <div className="flex flex-col gap-1.5">
      <div className="h-72 overflow-hidden rounded-md border border-border">
        <DataGrid columns={preview.columns.map((c) => ({ name: c.name, typeName: c.typeName }))} rowCount={shown} getRow={(i) => preview.rows[i]} rowHeight={DENSITIES[density].rowHeight} />
      </div>
      <p className="text-[11px] text-muted">
        {preview.totalRows > shown ? `First ${formatCount(shown)} of ${formatCount(preview.totalRows)} rows` : `${formatCount(preview.totalRows)} row${preview.totalRows === 1 ? "" : "s"}`}
        {" · "}
        {preview.columns.length} column{preview.columns.length === 1 ? "" : "s"}
      </p>
    </div>
  );
}

// WHAT:  A node's last run: how it ended, what it printed, and the first rows it produced.
// WHY:   Wiring a workflow is trial and error; seeing each node's output (columns and
//        up to 200 rows) next to its settings is what makes that quick.
// HOW:   Status and console come from the live run (`update`); the preview is the
//        last output the node produced and survives until it produces another.
export function OutputPanel({ update, previews }: { update: NodeUpdate | null; previews: readonly NodePreview[] }) {
  const [side, setSide] = useState<"true" | "false">("true");
  const branching = previews.length > 1;
  const shown = branching ? (previews.find((p) => p.handle === side) ?? previews[0]) : previews[0];
  const filePath = update?.filePath ?? null;
  const note = update?.note ?? null;
  const error = update !== null && update.state === "error" ? update.error : null;

  if (update === null && shown === undefined) {
    return (
      <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed border-border px-3 py-4 text-center">
        <Icon name="play" size={16} className="text-muted" />
        <p className="text-xs text-muted">Run this node to see what it produces here.</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {update !== null ? (
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant={STATE_VARIANT[update.state]}>{STATE_LABEL[update.state]}</Badge>
          {update.rows !== null ? <Badge variant="outline">{formatCount(update.rows)} row{update.rows === 1 ? "" : "s"}</Badge> : null}
          <Badge variant="outline">{formatMs(update.elapsedMs)}</Badge>
          {update.passes !== null && update.passes > 1 && update.pass !== null ? <Badge variant="outline">pass {update.pass} of {update.passes}</Badge> : null}
        </div>
      ) : null}
      {filePath !== null ? (
        <p className="flex items-center gap-1.5 break-all font-mono text-[11px] text-muted">
          <Icon name="file" size={12} className="shrink-0" />
          {filePath}
        </p>
      ) : null}
      {note !== null ? <p className="text-xs text-muted">{note}</p> : null}
      {error !== null ? (
        <Alert variant="danger" className="rounded-lg text-xs">
          <AlertIndicator />
          <AlertContent>
            <AlertDescription className="selectable font-mono text-[11px]">{error}</AlertDescription>
          </AlertContent>
        </Alert>
      ) : null}
      {update !== null && update.logs.length > 0 ? (
        <div className="flex flex-col gap-1">
          <h4 className="text-[11px] font-semibold uppercase tracking-wide text-muted">Console</h4>
          <div className="selectable max-h-40 overflow-auto rounded-md border border-border bg-surface-secondary/50 p-2 font-mono text-[11px] leading-relaxed">
            {update.logs.map((line, index) => (
              <div key={index} className={cn("whitespace-pre-wrap break-words", toneOf(line.level))}>
                {line.text}
              </div>
            ))}
          </div>
        </div>
      ) : null}
      {branching ? (
        <Segmented
          value={side}
          label="Output"
          onChange={setSide}
          options={[
            { value: "true", label: `True (${formatCount(previews.find((p) => p.handle === "true")?.totalRows ?? 0)})` },
            { value: "false", label: `False (${formatCount(previews.find((p) => p.handle === "false")?.totalRows ?? 0)})` },
          ]}
        />
      ) : null}
      {shown !== undefined ? <Preview preview={shown} /> : null}
    </div>
  );
}
