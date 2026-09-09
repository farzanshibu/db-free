// SOT: agent-tool-trace, agent-run-timeline, agent-permission-prompt, agent-loading-state
import { useState } from "react";
import type { PermissionDecision, PermissionRequest, ToolCallRecord, ToolStatus } from "@/lib/bindings";
import { Icon, type IconName } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";

// WHAT:  The timeline of what the assistant did to answer — every tool call, its
//        arguments and its result — plus the prompt it raises before writing.
// WHY:   An agent that works for twenty seconds behind a spinner is impossible
//        to trust or debug. Showing each step as it happens turns the wait into
//        progress the user can read, and makes a wrong answer diagnosable:
//        you can see which table it looked at and what came back.
// WHERE: src/features/chat/ChatTab.tsx, src-tauri/src/model/agent.rs

/// One icon per tool family, so the timeline is scannable without reading it.
function iconFor(tool: string): IconName {
  if (tool.startsWith("render_")) return "chart";
  if (tool.startsWith("list_")) return "list";
  if (tool.startsWith("describe_")) return "columns";
  if (tool === "search_schema") return "search";
  if (tool === "sample_rows") return "table";
  if (tool === "run_query") return "play";
  if (tool === "explain_query") return "gauge";
  if (tool === "server_stats") return "activity";
  if (tool.endsWith("skill") || tool.endsWith("skills")) return "book";
  return "terminal";
}

function StatusDot({ status }: { status: ToolStatus }) {
  if (status === "running") return <Spinner size="sm" />;
  if (status === "ok") return <Icon name="check" size={11} className="text-success" />;
  if (status === "denied") return <Icon name="shield" size={11} className="text-warning" />;
  return <Icon name="alert" size={11} className="text-danger" />;
}

function ToolRow({ call }: { call: ToolCallRecord }) {
  const [open, setOpen] = useState(false);
  const failed = call.status === "error" || call.status === "denied";
  return (
    <div className="rounded-lg border border-border/40 bg-surface-secondary/40">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-2 py-1.5 text-left liquid-hover"
      >
        <Icon
          name={open ? "chevron-down" : "chevron-right"}
          size={10}
          className="shrink-0 text-muted"
        />
        <Icon name={iconFor(call.tool)} size={11} className="shrink-0 text-muted" />
        <span className="min-w-0 flex-1 truncate text-[11px] text-foreground">{call.title}</span>
        {call.summary !== null ? (
          <span className="shrink-0 text-[10px] text-muted tabular-nums">{call.summary}</span>
        ) : null}
        {call.elapsedMs > 0 ? (
          <span className="shrink-0 text-[10px] text-muted tabular-nums">{call.elapsedMs}ms</span>
        ) : null}
        <StatusDot status={call.status} />
      </button>
      {open ? (
        <div className="border-t border-border/30 px-2 py-1.5">
          <span className="font-mono text-[10px] font-semibold uppercase tracking-wider text-muted">
            {call.tool}
          </span>
          <pre className="selectable mt-1 overflow-x-auto font-mono text-[10.5px] whitespace-pre-wrap text-muted">
            {call.input}
          </pre>
          {failed && call.error !== null ? (
            <p className="selectable mt-1.5 rounded bg-danger-soft/60 p-1.5 font-mono text-[10.5px] text-danger">
              {call.error}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

export function ToolTrace({ calls }: { calls: ToolCallRecord[] }) {
  const [expanded, setExpanded] = useState(false);
  if (calls.length === 0) return null;

  const running = calls.some((call) => call.status === "running");
  // While it works, show the live step. Once done, collapse to a single line so
  // finished messages stay readable — the detail is one click away.
  const visible = expanded || running ? calls : calls.slice(-1);

  return (
    <div className="mb-2 flex flex-col gap-1">
      {calls.length > 1 ? (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="flex items-center gap-1 self-start text-[10px] text-muted liquid-hover hover:text-foreground"
        >
          <Icon name={expanded ? "collapse" : "expand"} size={9} />
          {expanded ? "Hide steps" : `${calls.length} steps`}
        </button>
      ) : null}
      {visible.map((call) => (
        <ToolRow key={call.id} call={call} />
      ))}
    </div>
  );
}

// WHAT:  The block before a write runs.
// WHY:   The agent is not the user. Nothing that changes data happens until a
//        human has read the statement — and "Allow for this run" exists so a
//        legitimate batch does not train people to click Allow without looking.
export function PermissionPrompt({
  request,
  onDecide,
}: {
  request: PermissionRequest;
  onDecide: (decision: PermissionDecision) => void;
}) {
  const destructive = request.intent === "destructive";
  return (
    <div
      className={cn(
        "my-2 rounded-xl border p-2.5 glass-modal",
        destructive ? "border-danger/40 bg-danger-soft/30" : "border-warning/40 bg-surface-secondary/60",
      )}
    >
      <div className="flex items-start gap-2">
        <Icon
          name={destructive ? "alert" : "shield"}
          size={14}
          className={cn("mt-0.5 shrink-0", destructive ? "text-danger" : "text-warning")}
        />
        <div className="min-w-0 flex-1">
          <p className="text-[11px] font-semibold text-foreground">{request.title}</p>
          <p className="mt-0.5 text-[10.5px] text-muted">{request.reason}</p>

          {request.statement !== null ? (
            <pre className="selectable mt-1.5 max-h-40 overflow-auto rounded-lg border border-border/50 bg-surface/80 p-2 font-mono text-[10.5px] whitespace-pre-wrap text-foreground">
              {request.statement}
            </pre>
          ) : null}

          {request.statements.length > 0 ? (
            <ul className="mt-1.5 space-y-0.5">
              {request.statements.map((line) => (
                <li key={line} className="text-[10.5px] text-muted">
                  • {line}
                </li>
              ))}
            </ul>
          ) : null}

          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            <Button
              size="sm"
              variant="secondary"
              className={cn(
                "glass-pill h-6 rounded-lg px-2.5 text-[11px] font-semibold liquid-hover",
                destructive ? "text-danger" : "text-accent",
              )}
              onClick={() => onDecide("allow")}
            >
              <Icon name="check" size={10} />
              Run once
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-6 rounded-lg px-2.5 text-[11px] text-muted liquid-hover hover:text-foreground"
              onClick={() => onDecide("allow_for_run")}
            >
              Allow for this answer
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-6 rounded-lg px-2.5 text-[11px] text-muted liquid-hover hover:text-foreground"
              onClick={() => onDecide("deny")}
            >
              <Icon name="x" size={10} />
              Don&apos;t run
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
