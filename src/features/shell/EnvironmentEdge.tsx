// SOT: environment-edge, workspace-env-tint
import type { Environment } from "@/lib/bindings";
import { environmentMeta } from "@/lib/environments";
import { paneCount } from "@/lib/panes";
import { useWorkspace } from "@/stores/workspace";
import { cn } from "@/lib/cn";

// WHAT:  A thin coloured edge along the top of the workspace for staging and
//        production connections.
// WHY:   The environment badge is small and sits in the sidebar; an edge across
//        the whole window is hard to miss before running a statement against
//        production.
// HOW:   Colour comes from the environment registry (`stripe`, env-* tokens);
//        local and none draw nothing, so the edge itself is the warning.
//        With several panes open the window-wide edge would speak for one pane
//        only, so it steps aside and every pane draws its own (`perPane`, from
//        that pane's active tab's connection).
// WHERE: src/lib/environments.ts, src/styles/globals.css (--color-env-*),
//        src/features/shell/PaneArea.tsx
export function EnvironmentEdge({ environment, perPane = false }: { environment: Environment; perPane?: boolean }) {
  const meta = environmentMeta(environment);
  const crowded = useWorkspace((s) => paneCount(s.panes) > 1);
  if (!meta.tintWorkspace || (crowded && !perPane)) return null;
  return (
    <div role="presentation" aria-hidden className={cn("pointer-events-none h-0.5 w-full shrink-0", meta.stripe)} />
  );
}
