// SOT: run-workflow-from-list, workflow-run-confirm, workflow-run-toast
import type { Document } from "@/lib/bindings";
import { errorMessage, ipc, normalizeError } from "@/lib/ipc";
import { toast } from "@/components/ui/sonner";

// WHAT:  Runs a workflow without opening it — the command palette's "Run workflow".
// WHY:   A workflow executes statements against real databases, so the palette asks
//        first (a toast with Run / Cancel) rather than running on Enter.
// HOW:   Goes through the existing `run_workflow` command; the outcome — every node
//        succeeded, or which one failed and why — is reported in the same toast.
// WHERE: src/features/palette/CommandPalette.tsx, src-tauri/src/commands/workflows.rs
async function runNow(doc: Document): Promise<void> {
  const id = toast.loading(`Running “${doc.name}”…`);
  try {
    const report = await ipc("run_workflow", { id: doc.id });
    const first = report.nodes.find((node) => node.state === "error");
    if (first !== undefined) {
      const detail = first.error ?? "it failed";
      toast.error(`“${doc.name}” finished with errors: node “${first.name}” — ${detail}`, { id, duration: 10_000 });
    } else {
      const ran = report.nodes.filter((node) => node.state === "success").length;
      const outcome = report.status === "stopped" ? "stopped" : "finished";
      toast.success(`“${doc.name}” ${outcome}: ${ran} ${ran === 1 ? "node" : "nodes"} ran.`, { id });
    }
  } catch (raw) {
    toast.error(`“${doc.name}” could not run: ${errorMessage(normalizeError(raw))}`, { id, duration: 10_000 });
  }
}

export function confirmRunWorkflow(doc: Document): void {
  toast(`Run “${doc.name}” now?`, {
    description: "Its nodes execute immediately: queries run on the connections they use and notifications are sent.",
    duration: 15_000,
    action: { label: "Run", onClick: () => void runNow(doc) },
    cancel: { label: "Cancel", onClick: () => undefined },
  });
}
