// SOT: workflow-node-card, workflow-node-status-badge, workflow-node-handles, workflow-note-node
import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import type { NodeUpdate, WorkflowNode } from "@/lib/bindings";
import { formatCount, formatMs } from "@/lib/format";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Spinner } from "@/components/ui/spinner";
import { NODE_SPECS, NODE_WIDTH, summarize, titleOf } from "./model";

// React Flow node data must be assignable to its record constraint; a type alias
// (implicit index signature) satisfies that where an interface cannot.
// eslint-disable-next-line @typescript-eslint/consistent-type-definitions
export type StepData = {
  node: WorkflowNode;
  /// The node's latest status in the current or last run.
  update: NodeUpdate | null;
  /// What is missing before it can run, or null.
  problem: string | null;
  /// Edits a Note's text in place.
  onNoteText: (id: string, text: string) => void;
};

export type StepFlowNode = Node<StepData, "step">;

function tint(color: string, percent: number): string {
  return `color-mix(in oklch, var(--color-conn-${color}) ${percent}%, transparent)`;
}

function baseName(path: string): string {
  return path.split(/[\\/]/).at(-1) ?? path;
}

// WHAT:  The line of status under a node's summary.
function Status({ update }: { update: NodeUpdate }) {
  const passes = update.passes !== null && update.passes > 1 && update.pass !== null ? ` · pass ${update.pass}/${update.passes}` : "";
  switch (update.state) {
    case "running":
      return (
        <span className="flex items-center gap-1.5 text-[11px] text-muted">
          <Spinner size="sm" />
          Running{passes}
        </span>
      );
    case "success": {
      const where = update.filePath !== null ? ` → ${baseName(update.filePath)}` : "";
      const rows = update.rows === null ? "done" : `${formatCount(update.rows)} row${update.rows === 1 ? "" : "s"}`;
      return (
        <span className="flex min-w-0 items-center gap-1.5 text-[11px] text-success">
          <Icon name="check" size={12} className="shrink-0" />
          <span className="truncate">
            {rows}
            {where} · {formatMs(update.elapsedMs)}
            {passes}
          </span>
        </span>
      );
    }
    case "error":
      return (
        <span className="flex min-w-0 items-start gap-1.5 text-[11px] text-danger">
          <Icon name="alert" size={12} className="mt-px shrink-0" />
          <span className="line-clamp-2 break-words">{update.error ?? "Failed"}</span>
        </span>
      );
    case "skipped":
      return <span className="text-[11px] text-muted">Skipped: nothing reached it</span>;
    case "stopped":
      return <span className="text-[11px] text-warning">Stopped</span>;
  }
}

// WHAT:  A node on the canvas: a coloured icon, the title, a one-line summary, and
//        while a run is live or just done, its status.
// WHY:   The canvas has to answer "what does this do, is it set up, did it work?"
//        without opening anything. The handles carry the wiring rules (model.ts):
//        a kind with no input draws no left handle, a sink no right one, a Branch
//        two on the right, True above False.
export function NodeCard({ data, selected }: NodeProps<StepFlowNode>) {
  const { node, update, problem } = data;
  const spec = NODE_SPECS[node.config.kind];

  if (node.config.kind === "note") {
    return (
      <div
        className={cn("select-none rounded-md p-2 transition-colors hover:bg-surface-secondary/40", selected ? "outline-dashed outline-1 outline-offset-2 outline-muted" : "")}
        style={{ width: NODE_WIDTH }}
      >
        <textarea
          value={node.config.text}
          onChange={(event) => { data.onNoteText(node.id, event.target.value); }}
          placeholder="Note"
          rows={Math.max(1, node.config.text.split("\n").length)}
          aria-label="Note"
          className="nodrag nowheel block w-full resize-none border-0 bg-transparent p-0 text-xs font-medium leading-snug text-foreground outline-none placeholder:text-muted/50"
        />
      </div>
    );
  }

  return (
    <div
      className={cn(
        "relative rounded-lg border bg-surface-elevated text-foreground shadow-md transition-colors",
        selected ? "border-accent ring-1 ring-accent" : "border-border hover:border-muted/50",
        update?.state === "error" ? "border-danger/70" : "",
        update?.state === "running" ? "border-accent/70" : "",
      )}
      style={{ width: NODE_WIDTH }}
    >
      {spec.input ? <Handle type="target" position={Position.Left} className="!size-2.5 !border-border !bg-surface-tertiary" /> : null}
      {spec.outputs.map((handle, index) => {
        const branching = spec.outputs.length > 1;
        return (
          <Handle
            key={handle}
            {...(handle === "default" ? {} : { id: handle })}
            type="source"
            position={Position.Right}
            style={branching ? { top: index === 0 ? "32%" : "68%" } : undefined}
            className={cn("!size-2.5 !border-border", handle === "true" ? "!bg-success" : handle === "false" ? "!bg-danger" : "!bg-surface-tertiary")}
          />
        );
      })}
      {spec.outputs.length > 1 ? (
        <>
          <span className="pointer-events-none absolute -right-9 top-[32%] -translate-y-1/2 text-[10px] font-medium text-success">true</span>
          <span className="pointer-events-none absolute -right-9 top-[68%] -translate-y-1/2 text-[10px] font-medium text-danger">false</span>
        </>
      ) : null}
      <div className="flex items-center gap-2 px-2.5 py-1.5">
        <span className="flex size-7 shrink-0 items-center justify-center rounded-md" style={{ background: tint(spec.color, 20), color: `var(--color-conn-${spec.color})` }}>
          <Icon name={spec.icon} size={14} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-1.5">
            <span className="truncate text-xs font-medium">{titleOf(node)}</span>
            {problem !== null && update === null ? (
              <span className="size-1.5 shrink-0 rounded-full bg-warning" title={problem} aria-label={problem} />
            ) : null}
          </span>
          <span className="block truncate font-mono text-[11px] text-muted">{summarize(node.config)}</span>
        </span>
      </div>
      {update !== null ? (
        <div className="border-t border-border/60 px-2.5 py-1">
          <Status update={update} />
        </div>
      ) : null}
    </div>
  );
}
