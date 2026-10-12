// SOT: empty-dashboard-state, dashboard-ai-prompt-box
import { useState } from "react";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";

// WHAT:  What an empty dashboard says: draw a tile on the grid, or describe the
//        dashboard you want and hand it to the AI chat.
// WHY:   An empty canvas is the moment people most need a way in. Drawing is one;
//        for someone who knows what they want to see but not how to write it,
//        a sentence is another.
// HOW:   The prompt box calls `onAsk` with the text; the dashboard turns that
//        into a chat prompt (`useWorkspace.getState().openChat`). The box is
//        only offered when there is a connection for the chat to work against.
// WHERE: ./DashboardTab.tsx, src/stores/workspace.ts (openChat)

export function EmptyDashboard({ canAsk, onAsk }: { canAsk: boolean; onAsk: (prompt: string) => void }) {
  const [text, setText] = useState("");
  const prompt = text.trim();
  const submit = () => {
    if (prompt.length === 0) return;
    onAsk(prompt);
    setText("");
  };
  return (
    <div className="pointer-events-auto flex w-[360px] max-w-[90%] flex-col items-center gap-2.5 text-center">
      <div className="relative h-12 w-20" aria-hidden="true">
        <div className="absolute inset-1.5 rounded-md border border-dashed border-muted/50" />
        <div className="absolute right-3 bottom-2.5 size-2.5 rounded-full bg-accent/70" />
      </div>
      <div className="flex flex-col gap-1">
        <p className="text-xs font-medium text-foreground">Click and drag on the grid to add a tile</p>
        <p className="text-xs text-muted">Give it a SQL query and pick a chart type.</p>
      </div>
      {canAsk ? (
        <div className="relative w-full">
          <Textarea
            aria-label="Describe the dashboard you want"
            rows={2}
            value={text}
            placeholder="Or describe the dashboard you want to build…"
            className="min-h-12 resize-none pr-9 font-sans text-xs"
            onChange={(event) => { setText(event.target.value); }}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                submit();
              }
            }}
          />
          <Button size="icon-sm" variant="primary" aria-label="Ask AI to build it" disabled={prompt.length === 0} onClick={submit} className="absolute right-1.5 bottom-1.5">
            <Icon name="sparkles" size={13} />
          </Button>
        </div>
      ) : null}
    </div>
  );
}
