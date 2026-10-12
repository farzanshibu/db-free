// SOT: whats-new-page, changelog-page, release-notes-view
import { useEffect, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import type { AppInfo } from "@/lib/bindings";
import { ipc } from "@/lib/ipc";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { PROJECT_URL, runShellAction } from "./shellActions";
import { useUpdates } from "./useUpdates";

// The repository's CHANGELOG.md, when the build includes one. A glob (not an import)
// so a checkout without the file still builds: the page then falls back to the
// release notes the update feed carries.
const bundled = import.meta.glob<string>("/CHANGELOG.md", { query: "?raw", import: "default", eager: true });
const CHANGELOG: string | null = Object.values(bundled)[0] ?? null;

function openLink(href: string | undefined): void {
  if (href !== undefined) void ipc("open_external", { url: href }).catch(() => undefined);
}

// Links never navigate the webview; the OS opens them, and only project links are allowed to.
const COMPONENTS: Components = {
  h1: ({ children }) => <h2 className="mt-6 mb-2 text-base font-semibold text-foreground first:mt-0">{children}</h2>,
  h2: ({ children }) => <h3 className="mt-5 mb-1.5 text-[13px] font-semibold text-foreground">{children}</h3>,
  h3: ({ children }) => <h4 className="mt-4 mb-1 text-xs font-semibold tracking-wider text-muted uppercase">{children}</h4>,
  p: ({ children }) => <p className="my-2 text-xs leading-relaxed text-foreground">{children}</p>,
  ul: ({ children }) => <ul className="my-2 list-disc space-y-1 pl-3 text-xs text-foreground">{children}</ul>,
  ol: ({ children }) => <ol className="my-2 list-decimal space-y-1 pl-3 text-xs text-foreground">{children}</ol>,
  code: ({ children }) => <code className="rounded bg-surface-secondary px-1 py-0.5 font-mono text-[12px]">{children}</code>,
  a: ({ href, children }) => (
    <button type="button" className="text-link underline-offset-2 hover:underline" onClick={() => { openLink(href); }}>
      {children}
    </button>
  ),
};

// WHAT:  What's New: the changelog of this build, or — when none is bundled — the
//        release notes of the update that is waiting.
// WHY:   People update to see what changed. The page is reachable from the Help
//        menu, the About dialog and the update bar.
// HOW:   Markdown rendered with remark-gfm. No changelog and no pending update is
//        said plainly, with a link to the releases page, rather than shown as an
//        empty page.
// WHERE: CHANGELOG.md at the repository root (optional), src/features/shell/useUpdates.ts
export function WhatsNewPage() {
  const goBack = useWorkspace((s) => (s.activeConnectionId !== null && s.sessions.includes(s.activeConnectionId) ? s.goWorkspace : s.goConnections));
  const available = useUpdates((s) => s.available);
  const notes = useUpdates((s) => s.notes);
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    void ipc("app_info")
      .then(setInfo)
      .catch(() => undefined);
  }, []);

  const hasContent = CHANGELOG !== null || (available !== null && notes !== null);
  return (
    <div className="grid-bg relative flex h-full min-h-0 flex-1 flex-col">
      <div className="drag-region flex app-titlebar app-pad-x shrink-0 items-center gap-2 border-b border-border/40 glass-header" data-tauri-drag-region>
        <Button variant="ghost" size="sm" onClick={goBack} className="rounded-lg text-muted hover:bg-surface-secondary/70 hover:text-foreground liquid-hover">
          <Icon name="chevron-left" size={14} />
          Back
        </Button>
        <span className="text-[13px] font-semibold tracking-tight text-foreground" data-tauri-drag-region>
          What&apos;s new
        </span>
        {info ? <span className="text-xs text-muted">You are on {info.version}</span> : null}
        <div className="drag-region h-full flex-1" data-tauri-drag-region />
        <Button variant="secondary" size="sm" onClick={() => runShellAction("check-updates")}>
          <Icon name="refresh" size={13} />
          Check for updates
        </Button>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="selectable mx-auto w-full max-w-[720px] px-5 py-3">
          {available !== null && notes !== null ? (
            <section className="mb-4 rounded-lg glass-card px-3 py-2">
              <h3 className="flex items-center gap-2 text-[13px] font-semibold text-foreground">
                <Icon name="sparkles" size={14} className="text-accent" />
                Version {available} is ready to install
              </h3>
              <ReactMarkdown remarkPlugins={[remarkGfm]} components={COMPONENTS}>
                {notes}
              </ReactMarkdown>
            </section>
          ) : null}
          {CHANGELOG !== null ? (
            <ReactMarkdown remarkPlugins={[remarkGfm]} components={COMPONENTS}>
              {CHANGELOG}
            </ReactMarkdown>
          ) : null}
          {!hasContent ? (
            <div className="flex flex-col items-start gap-2 py-6">
              <p className="text-[13px] text-foreground">This build does not carry a changelog.</p>
              <p className="text-xs text-muted">Release notes are published with every release on GitHub. If an update is available, its notes appear here once it has been found.</p>
              <span className="flex gap-2">
                <Button size="sm" variant="secondary" onClick={() => { openLink(`${PROJECT_URL}/releases`); }}>
                  <Icon name="globe" size={13} />
                  Open the releases page
                </Button>
              </span>
            </div>
          ) : null}
        </div>
      </ScrollArea>
    </div>
  );
}
