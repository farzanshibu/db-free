// SOT: workflow-empty-canvas, ask-ai-workflow-prompt
import { useState } from "react";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { NodePicker } from "./NodePicker";
import type { NodeKind } from "./model";

interface EmptyCanvasProps {
  onPick: (kind: NodeKind) => void;
  /// Hands a request to the AI assistant; null when there is no connection to ask about.
  onAskAi: ((prompt: string) => void) | null;
}

const EXAMPLES = [
  "Every morning at 9, email me the orders placed yesterday",
  "Export active users to a CSV every hour",
  "Post to Slack when more than 10 signups fail validation",
];

// WHAT:  What a blank workflow shows: add a first node, or describe the workflow
//        to the AI assistant.
// WHY:   An empty canvas gives no hint of what a workflow is. A sentence to the
//        assistant is the quickest way to a first draft, which can then be edited here.
export function EmptyCanvas({ onPick, onAskAi }: EmptyCanvasProps) {
  const [prompt, setPrompt] = useState("");
  const ask = () => {
    if (onAskAi !== null && prompt.trim() !== "") onAskAi(prompt.trim());
  };
  return (
    <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center p-3">
      <div className="pointer-events-auto flex w-full max-w-sm flex-col gap-2.5 rounded-lg border border-border bg-surface-elevated p-3 shadow-md">
        <div className="flex flex-col gap-1">
          <h2 className="flex items-center gap-2 text-[13px] font-semibold">
            <Icon name="flow" size={14} className="text-accent" />
            Build a workflow
          </h2>
          <p className="text-xs leading-relaxed text-muted">Chain queries, files, API calls, filters and notifications into something that runs on demand or on a schedule.</p>
        </div>
        <NodePicker onPick={onPick} label="Add the first node" />
        <div className="flex flex-col gap-2 border-t border-border pt-2.5">
          <label htmlFor="workflow-ai-prompt" className="flex items-center gap-1.5 text-xs font-medium">
            <Icon name="sparkles" size={13} className="text-accent" />
            Or describe it and let AI build it
          </label>
          <Textarea
            id="workflow-ai-prompt"
            value={prompt}
            rows={3}
            className="font-sans text-xs"
            placeholder={EXAMPLES[0]}
            disabled={onAskAi === null}
            onChange={(event) => { setPrompt(event.target.value); }}
            onKeyDown={(event) => {
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) ask();
            }}
          />
          <div className="flex flex-wrap items-center gap-1.5">
            {EXAMPLES.map((example) => (
              <Button key={example} size="xs" variant="tertiary" disabled={onAskAi === null} onClick={() => { setPrompt(example); }}>
                {example}
              </Button>
            ))}
          </div>
          <Button size="sm" className="self-start" disabled={onAskAi === null || prompt.trim() === ""} onClick={ask}>
            <Icon name="sparkles" size={12} />
            Ask AI to build this workflow
          </Button>
          {onAskAi === null ? <p className="text-[11px] text-muted">Pick a connection in the toolbar to ask the assistant about it.</p> : null}
        </div>
      </div>
    </div>
  );
}
