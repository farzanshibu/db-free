// SOT: workflow-inspector, node-settings-panel, node-run-controls
import { useState } from "react";
import type { NodePreview, NodeUpdate, WorkflowNode } from "@/lib/bindings";
import { Segmented, Toggle } from "@/components/global/Field";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { BranchForm, DelayForm, FilterForm, GateForm, NoteForm, RepeatForm, ScheduleForm } from "./forms/FlowForms";
import { HttpForm, QueryForm, ReadFileForm } from "./forms/DataForms";
import { EmailForm, NotificationForm, SlackForm } from "./forms/NotifyForms";
import { InsertForm, SaveFileForm } from "./forms/OutputForms";
import { JavascriptForm, RegexForm } from "./forms/TransformForms";
import type { FormContext } from "./forms/common";
import { NODE_SPECS } from "./model";
import { NodeHelp } from "./NodeHelp";
import { OutputPanel } from "./OutputPanel";

type Tab = "settings" | "output" | "help";

interface InspectorProps {
  node: WorkflowNode;
  workflowConnectionId: string;
  /// Columns the node receives, from the last run of what feeds it.
  columns: readonly string[];
  update: NodeUpdate | null;
  previews: readonly NodePreview[];
  running: boolean;
  onChange: (node: WorkflowNode) => void;
  onRun: () => void;
  onStop: () => void;
  onDelete: () => void;
}

// WHAT:  The right-hand panel for the selected node: its settings, its last output
//        and a page of help, with Run node / Stop / Delete in the header.
// WHY:   Build, run and look at the result without leaving the node you are working on.
export function Inspector({ node, workflowConnectionId, columns, update, previews, running, onChange, onRun, onStop, onDelete }: InspectorProps) {
  const [tab, setTab] = useState<Tab>("settings");
  const spec = NODE_SPECS[node.config.kind];
  const runnable = node.config.kind !== "note";
  const context: FormContext = { node, workflowConnectionId, columns, runNode: onRun };

  return (
    <aside className="flex w-[360px] shrink-0 flex-col border-l border-border bg-surface" aria-label={`${spec.label} settings`}>
      <div className="flex flex-col gap-2 border-b border-border px-2.5 py-1.5">
        <div className="flex items-center gap-2">
          <span className="flex size-7 shrink-0 items-center justify-center rounded-md" style={{ background: `color-mix(in oklch, var(--color-conn-${spec.color}) 20%, transparent)`, color: `var(--color-conn-${spec.color})` }}>
            <Icon name={spec.icon} size={14} />
          </span>
          <Input
            value={node.label}
            placeholder={spec.label}
            aria-label="Node title"
            className="h-6.5 flex-1 border-transparent bg-transparent px-1.5 text-[13px] font-medium hover:border-border"
            onChange={(event) => { onChange({ ...node, label: event.target.value }); }}
          />
        </div>
        {runnable ? (
          <div className="flex items-center gap-1.5">
            {running ? (
              <Button size="sm" variant="danger-soft" onClick={onStop}>
                <Icon name="stop" size={12} />
                Stop
              </Button>
            ) : (
              <Button
                size="sm"
                onClick={() => {
                  setTab("output");
                  onRun();
                }}
              >
                <Icon name="play" size={12} />
                Run node
              </Button>
            )}
            <span className="text-[11px] text-muted">Runs this node and everything before it.</span>
          </div>
        ) : null}
        <Segmented
          value={tab}
          label="Panel"
          onChange={setTab}
          options={[
            { value: "settings", label: "Settings" },
            { value: "output", label: "Output", disabled: !runnable },
            { value: "help", label: "Help" },
          ]}
        />
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-2.5 p-2.5">
          {tab === "settings" ? (
            <>
              {renderForm(node, onChange, context)}
              {runnable ? (
                <Toggle
                  checked={node.continueOnError}
                  onChange={(continueOnError) => { onChange({ ...node, continueOnError }); }}
                  label="Keep going if this node fails"
                  description="The run carries on with an empty result from this node. The failure is still reported."
                />
              ) : null}
            </>
          ) : null}
          {tab === "output" ? <OutputPanel update={update} previews={previews} /> : null}
          {tab === "help" ? <NodeHelp kind={node.config.kind} /> : null}
        </div>
      </ScrollArea>
      <div className="flex items-center border-t border-border px-2.5 py-1.5">
        <Button size="sm" variant="danger-soft" className="ml-auto" onClick={onDelete}>
          <Icon name="trash" size={12} />
          Delete node
        </Button>
      </div>
    </aside>
  );
}

// WHAT:  The settings form for the node's kind.
function renderForm(node: WorkflowNode, onChange: (node: WorkflowNode) => void, context: FormContext) {
  const config = node.config;
  switch (config.kind) {
    case "schedule":
      return <ScheduleForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "query":
      return <QueryForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "http":
      return <HttpForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "read_file":
      return <ReadFileForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "save_file":
      return <SaveFileForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "insert":
      return <InsertForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "filter":
      return <FilterForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "row_count_gate":
      return <GateForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "branch":
      return <BranchForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "repeat":
      return <RepeatForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "delay":
      return <DelayForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "regex":
      return <RegexForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "javascript":
      return <JavascriptForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "notification":
      return <NotificationForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "email":
      return <EmailForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "slack":
      return <SlackForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
    case "note":
      return <NoteForm config={config} context={context} onChange={(next) => { onChange({ ...node, config: next }); }} />;
  }
}
