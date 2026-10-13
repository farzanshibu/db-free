// SOT: query-versions-dialog, saved-query-history-ui, restore-saved-query-version
import { useEffect, useState } from "react";
import type { SavedQuery, SavedQueryVersion } from "@/lib/bindings";
import { errorMessage, ipc, normalizeError } from "@/lib/ipc";
import { relativeTime } from "@/lib/relativeTime";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogBody, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Spinner } from "@/components/ui/spinner";

interface QueryVersionsDialogProps {
  query: SavedQuery;
  /// Opens a version's SQL in a new query tab without changing the saved query.
  onOpenVersion: (sql: string, label: string) => void;
  /// Called after a restore, with the query as it is now.
  onRestored?: (restored: SavedQuery) => void;
  onClose: () => void;
}

function preview(sql: string): string {
  const flat = sql.replace(/\s+/g, " ").trim();
  return flat.length > 72 ? `${flat.slice(0, 69)}…` : flat;
}

// WHAT:  A saved query's version history: every earlier SQL text (the newest ten),
//        with Restore and "Open in a tab".
// WHY:   Saving over a query used to lose the old text. A version is added whenever a
//        save changes the SQL, so a bad edit is one click from being undone.
// HOW:   Restoring makes that text the query's SQL again and records it as a new
//        version, so the restore itself can be undone the same way.
// WHERE: src-tauri/src/store/saved_queries.rs (versions), src/features/queries/QueriesPanel.tsx
export function QueryVersionsDialog({ query, onOpenVersion, onRestored, onClose }: QueryVersionsDialogProps) {
  const [versions, setVersions] = useState<SavedQueryVersion[] | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const token = { cancelled: false };
    void (async () => {
      try {
        const listed = await ipc("list_saved_query_versions", { id: query.id });
        if (token.cancelled) return;
        setVersions(listed);
        setSelectedId(listed[0]?.id ?? null);
      } catch (raw) {
        if (token.cancelled) return;
        setError(errorMessage(normalizeError(raw)));
        setVersions([]);
      }
    })();
    return () => {
      token.cancelled = true;
    };
  }, [query.id]);

  const selected = versions?.find((v) => v.id === selectedId) ?? null;
  const latestId = versions?.[0]?.id ?? null;

  const restore = async (version: SavedQueryVersion) => {
    setPending(true);
    setError(null);
    try {
      const restored = await ipc("restore_saved_query_version", { id: version.id });
      await useWorkspace.getState().loadSavedQueries();
      useWorkspace.getState().showInfo(`Restored version ${version.version} of “${restored.name}”.`);
      onRestored?.(restored);
      onClose();
    } catch (raw) {
      setError(errorMessage(normalizeError(raw)));
      setPending(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-[760px]">
        <DialogHeader>
          <DialogTitle>Version history · {query.name}</DialogTitle>
        </DialogHeader>
        <DialogBody className="p-0">
          {versions === null ? (
            <div className="flex h-48 items-center justify-center">
              <Spinner size="sm" />
            </div>
          ) : versions.length === 0 ? (
            <p className="px-3 py-3 text-xs text-muted">{error ?? "No versions yet. One is added each time a save changes the SQL."}</p>
          ) : (
            <div className="grid h-[320px] grid-cols-[220px_1fr]">
              <ScrollArea className="border-r border-border/50">
                <div role="listbox" aria-label="Versions" className="flex flex-col p-1.5">
                  {versions.map((v) => {
                    const isSelected = v.id === selectedId;
                    return (
                      <Button
                        key={v.id}
                        variant="ghost"
                        role="option"
                        aria-selected={isSelected}
                        onClick={() => setSelectedId(v.id)}
                        className={cn(
                          "flex h-auto w-full flex-col items-stretch gap-0.5 rounded-md px-2 py-1 text-left font-normal",
                          isSelected ? "bg-accent/15 text-foreground hover:bg-accent/20" : "text-foreground",
                        )}
                      >
                        <span className="flex items-center gap-1.5">
                          <span className={cn("text-[11px] font-semibold", v.id === latestId ? "text-accent" : "text-muted")}>
                            {v.id === latestId ? "Latest" : `v${v.version}`}
                          </span>
                          <span className="ml-auto text-[10.5px] text-muted">{relativeTime(v.createdAt)}</span>
                        </span>
                        <span className="truncate font-mono text-[11px] text-muted">{preview(v.sql)}</span>
                      </Button>
                    );
                  })}
                </div>
              </ScrollArea>
              <div className="flex min-w-0 flex-col">
                <div className="flex shrink-0 items-center gap-2 border-b border-border/50 px-2.5 py-1.5 text-xs text-muted">
                  <Icon name="history" size={13} />
                  {selected !== null ? (
                    <>
                      <span>Version {selected.version}</span>
                      <Badge size="sm" variant="soft" className="text-[10px]">
                        {selected.id === latestId ? "current" : relativeTime(selected.createdAt)}
                      </Badge>
                    </>
                  ) : null}
                </div>
                <ScrollArea className="min-h-0 flex-1">
                  <pre className="whitespace-pre-wrap break-words p-2.5 font-mono text-xs leading-relaxed text-foreground">{selected?.sql ?? ""}</pre>
                </ScrollArea>
              </div>
            </div>
          )}
        </DialogBody>
        {error !== null && versions !== null && versions.length > 0 ? <p className="px-3 pb-1.5 text-xs text-danger">{error}</p> : null}
        <DialogFooter>
          <Button variant="tertiary" onClick={onClose}>
            Close
          </Button>
          <Button variant="secondary" disabled={selected === null} onClick={() => selected !== null && onOpenVersion(selected.sql, `${query.name} (v${selected.version})`)}>
            Open in a tab
          </Button>
          <Button pending={pending} disabled={selected === null || selected.id === latestId} onClick={() => selected !== null && void restore(selected)}>
            Restore this version
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
