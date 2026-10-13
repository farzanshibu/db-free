// SOT: home-board, home-pinned-section, home-recents-section, home-empty-state
import { useEffect } from "react";
import type { DocumentKind } from "@/lib/bindings";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { Button } from "@/components/ui/button";
import { PinnedBoard } from "./PinnedBoard";
import { RecentsPanel } from "./RecentsPanel";
import { useResolveEntity } from "./useResolvedEntity";

const DOCUMENT_KINDS: readonly DocumentKind[] = ["dashboard", "workflow", "diagram"];

// WHAT:  The top of the Home page: the Pinned board and the Recents, each hidden by
//        its setting (Settings → Home), above the connection list.
// WHY:   Home should lead to the thing you were doing, not only to the servers.
//        Pinned shows an empty state when nothing is pinned so the feature can be
//        found; with no saved connection at all the whole board stays out of the way.
// HOW:   Pins are loaded at startup (src/stores/recents.ts). Documents load per kind
//        when a sidebar opens, so any kind that has a pin is loaded here to show
//        current names and to drop pins whose document was deleted.
// WHERE: src/features/connections/ConnectionsPage.tsx (mounts it), src/features/home/PinnedBoard.tsx
export function HomeBoard() {
  const connections = useWorkspace((s) => s.connections);
  const pins = useWorkspace((s) => s.pins);
  const documents = useWorkspace((s) => s.documents);
  const loadDocuments = useWorkspace((s) => s.loadDocuments);
  const showPinned = useWorkspace((s) => s.settings?.showPinned ?? true);
  const showRecents = useWorkspace((s) => s.settings?.showRecents ?? true);
  const goSettingsSection = useWorkspace((s) => s.goSettingsSection);
  const resolve = useResolveEntity();

  const needed = DOCUMENT_KINDS.filter((kind) => pins.some((p) => p.entity.kind === kind) && documents[kind].length === 0);
  const neededKey = needed.join(",");
  useEffect(() => {
    for (const kind of neededKey.split(",")) {
      const known = DOCUMENT_KINDS.find((k) => k === kind);
      if (known !== undefined) void loadDocuments(known).catch(() => undefined);
    }
  }, [neededKey, loadDocuments]);

  if (connections.length === 0) return null;
  const visiblePins = pins.filter((p) => resolve(p.entity) !== null);

  return (
    <div className="mb-4 flex flex-col gap-3">
      {showPinned ? (
        <section className="flex flex-col gap-2" aria-label="Pinned">
          <div className="flex items-center gap-2">
            <h2 className="text-xs font-semibold uppercase tracking-wide text-muted">Pinned</h2>
            {visiblePins.length > 0 ? <span className="text-xs tabular-nums text-muted/70">{visiblePins.length}</span> : null}
            <Button variant="ghost" size="icon-sm" className="ml-auto" aria-label="Home settings" title="Home settings" onClick={() => goSettingsSection("home")}>
              <Icon name="settings" size={13} />
            </Button>
          </div>
          {visiblePins.length > 0 ? (
            <PinnedBoard pins={pins} />
          ) : (
            <div className="flex flex-col items-center gap-1.5 rounded-lg border border-dashed border-border/60 px-3 py-2 text-center">
              <Icon name="pin" size={14} className="text-muted" />
              <p className="max-w-md text-xs text-muted">Nothing pinned yet. Pin a connection, table, saved query, dashboard, workflow or diagram to keep it one click away.</p>
            </div>
          )}
        </section>
      ) : null}
      {showRecents ? <RecentsPanel /> : null}
    </div>
  );
}
