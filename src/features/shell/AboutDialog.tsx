// SOT: about-dialog, app-version-display, log-folder-link
import { useEffect, useState } from "react";
import type { AppInfo } from "@/lib/bindings";
import { ipc, normalizeError } from "@/lib/ipc";
import { useWorkspace } from "@/stores/workspace";
import { Icon } from "@/lib/icons";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { PROJECT_URL, runShellAction } from "./shellActions";

// WHAT:  About DB Free: the version, platform, where this install keeps its
//        files, and shortcuts to the release notes, updates and the log folder.
// WHY:   A bug report starts with "which build, on what, and where are the logs";
//        the answer should be one menu click away rather than a terminal.
// HOW:   The details come from `app_info` when the dialog opens. Opened by the
//        app menu, ⌘K and Settings → Updates through the store's `aboutOpen`.
// WHERE: src-tauri/src/commands/updates.rs (app_info), src/features/shell/shellActions.ts
export function AboutDialog() {
  const open = useWorkspace((s) => s.aboutOpen);
  const setOpen = useWorkspace((s) => s.setAboutOpen);
  const showError = useWorkspace((s) => s.showError);
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    if (!open || info !== null) return;
    void (async () => {
      try {
        setInfo(await ipc("app_info"));
      } catch (raw) {
        showError(normalizeError(raw));
      }
    })();
  }, [open, info, showError]);

  const run = (action: string) => {
    setOpen(false);
    runShellAction(action);
  };

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <span className="grid size-8 place-items-center rounded-lg bg-accent text-accent-foreground">
              <Icon name="database" size={16} />
            </span>
            <div>
              <DialogTitle className="text-sm">{info?.name ?? "DB Free"}</DialogTitle>
              <DialogDescription>{info ? `Version ${info.version}` : "Lightweight native database workbench"}</DialogDescription>
            </div>
          </div>
        </DialogHeader>
        <DialogBody>
          <dl className="selectable grid grid-cols-[7rem_1fr] gap-x-2 gap-y-1.5 text-xs">
            <dt className="text-muted">Platform</dt>
            <dd className="text-foreground">{info ? `${info.os} · ${info.arch}` : "…"}</dd>
            <dt className="text-muted">Identifier</dt>
            <dd className="font-mono text-foreground">{info?.identifier ?? "…"}</dd>
            <dt className="text-muted">App data</dt>
            <dd className="font-mono break-all text-foreground">{info?.dataDir ?? "—"}</dd>
            <dt className="text-muted">Logs</dt>
            <dd className="font-mono break-all text-foreground">{info?.logDir ?? "—"}</dd>
            <dt className="text-muted">Source</dt>
            <dd className="font-mono break-all text-foreground">{PROJECT_URL}</dd>
          </dl>
        </DialogBody>
        <DialogFooter className="flex-wrap justify-between">
          <span className="flex gap-1.5">
            <Button size="sm" variant="secondary" onClick={() => run("check-updates")}>
              <Icon name="refresh" size={13} />
              Check for updates
            </Button>
            <Button size="sm" variant="ghost" onClick={() => run("whats-new")}>
              What&apos;s new
            </Button>
          </span>
          <span className="flex gap-1.5">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                void (async () => {
                  try {
                    await ipc("open_log_folder");
                  } catch (raw) {
                    showError(normalizeError(raw));
                  }
                })();
              }}
            >
              <Icon name="folder" size={13} />
              Open log folder
            </Button>
            <Button size="sm" variant="ghost" onClick={() => run("learn-more")}>
              <Icon name="globe" size={13} />
              GitHub
            </Button>
          </span>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
