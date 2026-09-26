// SOT: environment-edge, workspace-env-tint
import type { Environment } from "@/lib/bindings";
import { environmentMeta } from "@/lib/environments";
import { cn } from "@/lib/cn";

// WHAT:  A thin coloured edge along the top of the workspace for staging and
//        production connections.
// WHY:   The environment badge is small and sits in the sidebar; an edge across
//        the whole window is hard to miss before running a statement against
//        production.
// HOW:   Colour comes from the environment registry (`stripe`, env-* tokens);
//        local and none draw nothing, so the edge itself is the warning.
// WHERE: src/lib/environments.ts, src/styles/globals.css (--color-env-*)
export function EnvironmentEdge({ environment }: { environment: Environment }) {
  const meta = environmentMeta(environment);
  if (!meta.tintWorkspace) return null;
  return (
    <div role="presentation" aria-hidden className={cn("pointer-events-none h-0.5 w-full shrink-0", meta.stripe)} />
  );
}
