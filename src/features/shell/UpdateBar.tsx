// SOT: update-bar, global-update-bar, update-restart-prompt
import { useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { useUpdates } from "./useUpdates";

function openNotes(): void {
  useWorkspace.setState({ page: { kind: "whats-new" } });
}

// WHAT:  A slim bar along the bottom of the window while a release is downloading
//        or waiting for a restart.
// WHY:   A toast that expires is easy to miss and impossible to come back to; the
//        bar stays until the user acts on it or closes it, and says plainly what
//        is happening (downloading, ready) and what restarting will do.
// HOW:   Reads the shared update state (useUpdates), so a check started from the
//        menu, Settings or launch all show here. "What's new" opens the release
//        notes page; closing the bar hides it for this release only.
// WHERE: src/features/shell/useUpdates.ts, src/App.tsx
export function UpdateBar() {
  const phase = useUpdates((s) => s.phase);
  const available = useUpdates((s) => s.available);
  const progress = useUpdates((s) => s.progress);
  const dismissed = useUpdates((s) => s.dismissed);
  const dismiss = useUpdates((s) => s.dismiss);
  const install = useUpdates((s) => s.install);
  if (dismissed || (phase !== "ready" && phase !== "downloading")) return null;

  const percent = progress?.total ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100)) : null;
  return (
    <div role="status" className="relative flex h-7 shrink-0 items-center gap-2 border-t border-border/40 glass-header px-2.5 text-xs">
      {phase === "downloading" ? (
        <>
          <Icon name="download" size={13} className="text-accent" />
          <span className="text-foreground">Downloading the update{percent !== null ? ` — ${String(percent)}%` : "…"}</span>
          <div className="h-1 w-40 overflow-hidden rounded-full bg-surface-secondary">
            <div className="h-full rounded-full bg-accent transition-[width] duration-200" style={{ width: `${String(percent ?? 100)}%` }} />
          </div>
        </>
      ) : (
        <>
          <Icon name="sparkles" size={13} className="text-accent" />
          <span className="text-foreground">DB Free {available ?? "update"} is ready to install.</span>
          <span className="text-muted">Restarting takes a moment.</span>
          <span className="ml-auto flex items-center gap-1.5">
            <Button size="sm" variant="ghost" className="h-5 px-1.5 text-xs" onClick={openNotes}>
              What&apos;s new
            </Button>
            <Button size="sm" className="h-5 px-1.5 text-xs font-semibold" onClick={() => void install()}>
              Restart now
            </Button>
          </span>
        </>
      )}
      <Button size="icon-sm" variant="ghost" aria-label="Hide update notice" className={phase === "ready" ? "size-5" : "ml-auto size-5"} onClick={dismiss}>
        <Icon name="x" size={12} />
      </Button>
    </div>
  );
}
