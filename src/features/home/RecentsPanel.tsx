// SOT: recents-panel, recent-chips, recents-full-list, clear-recents
import { useState } from "react";
import type { HomeEntity, HomeRecent } from "@/lib/bindings";
import { HOME_KINDS, entityKey } from "@/lib/home";
import { normalizeError } from "@/lib/ipc";
import { relativeTime } from "@/lib/relativeTime";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { useRecents } from "@/stores/recents";
import { useWorkspace } from "@/stores/workspace";
import { EngineIcon } from "@/components/global/EngineIcon";
import { Button } from "@/components/ui/button";
import { openHomeEntity } from "./openEntity";
import { useResolveEntity, type ResolvedEntity } from "./useResolvedEntity";

/// How many recents show as chips before "Show all".
const CHIPS = 5;

interface Row {
  recent: HomeRecent;
  resolved: ResolvedEntity;
}

function Glyph({ entity, resolved, size }: { entity: HomeEntity; resolved: ResolvedEntity; size: number }) {
  return entity.kind === "connection" ? (
    <EngineIcon engine={resolved.connection.engine} size={size} />
  ) : (
    <Icon name={HOME_KINDS[entity.kind].icon} size={size - 2} className="shrink-0 text-accent" />
  );
}

// WHAT:  The Home "Recents" section: the last five things opened as chips, and with
//        "Show all" the full list (up to fifty) with when and where; "Clear" empties it.
//        Each entry can be pinned to the board.
// WHY:   Getting back to what you had open yesterday should not mean remembering which
//        connection it was on.
// HOW:   Recents are recorded by the workspace subscription in src/stores/recents.ts.
//        Entries whose item no longer exists are left out; names are the current ones.
// WHERE: src-tauri/src/store/home.rs
export function RecentsPanel() {
  const recents = useRecents((s) => s.recents);
  const clear = useRecents((s) => s.clear);
  const pins = useWorkspace((s) => s.pins);
  const pinItem = useWorkspace((s) => s.pinItem);
  const unpinItem = useWorkspace((s) => s.unpinItem);
  const showError = useWorkspace((s) => s.showError);
  const resolve = useResolveEntity();
  const [all, setAll] = useState(false);

  const rows: Row[] = recents.flatMap((recent) => {
    const resolved = resolve(recent.entity);
    return resolved === null ? [] : [{ recent, resolved }];
  });
  if (rows.length === 0) return null;
  const shown = all ? rows : rows.slice(0, CHIPS);
  const isPinned = (entity: HomeEntity) => pins.some((p) => p.entity.kind === entity.kind && p.entity.itemId === entity.itemId);
  const togglePin = (entity: HomeEntity) => void (isPinned(entity) ? unpinItem(entity.kind, entity.itemId) : pinItem(entity));

  const doClear = async () => {
    try {
      await clear();
    } catch (raw) {
      showError(normalizeError(raw));
    }
  };

  return (
    <section className="flex flex-col gap-2" aria-label="Recents">
      <div className="flex items-center gap-2">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-muted">Recents</h2>
        <span className="text-xs tabular-nums text-muted/70">{rows.length}</span>
        <span className="ml-auto flex items-center gap-1">
          {rows.length > CHIPS ? (
            <Button variant="ghost" size="xs" onClick={() => setAll((v) => !v)}>
              {all ? "Show fewer" : `Show all ${rows.length}`}
            </Button>
          ) : null}
          <Button variant="ghost" size="xs" onClick={() => void doClear()}>
            Clear
          </Button>
        </span>
      </div>
      {all ? (
        <ul className="flex flex-col gap-0.5">
          {shown.map(({ recent, resolved }) => {
            const { entity } = recent;
            const pinned = isPinned(entity);
            return (
              <li key={entityKey(entity)} className="group flex items-center gap-1 rounded-lg hover:bg-surface-secondary/60">
                <Button
                  variant="ghost"
                  onClick={() => void openHomeEntity(entity)}
                  className="flex h-6 min-w-0 flex-1 items-center justify-start gap-2 bg-transparent px-2 text-left font-normal hover:bg-transparent"
                >
                  <Glyph entity={entity} resolved={resolved} size={14} />
                  <span className="min-w-0 truncate text-xs text-foreground">{resolved.label}</span>
                  <span className="shrink-0 rounded bg-surface-secondary/70 px-1.5 py-0.5 text-[10px] font-medium text-muted">{HOME_KINDS[entity.kind].label}</span>
                  {entity.kind !== "connection" ? <span className="min-w-0 truncate text-[11px] text-muted">{resolved.connection.name}</span> : null}
                  <span className="ml-auto shrink-0 text-[11px] text-muted/80">{relativeTime(recent.openedAt)}</span>
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={pinned ? `Unpin ${resolved.label}` : `Pin ${resolved.label} to Home`}
                  title={pinned ? "Unpin" : "Pin to Home"}
                  onClick={() => togglePin(entity)}
                  className={cn("shrink-0", pinned ? "text-accent" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100")}
                >
                  <Icon name="pin" size={12} />
                </Button>
              </li>
            );
          })}
        </ul>
      ) : (
        <div className="flex flex-wrap gap-2">
          {shown.map(({ recent, resolved }) => {
            const { entity } = recent;
            const pinned = isPinned(entity);
            return (
              <div
                key={entityKey(entity)}
                className="group inline-flex max-w-56 items-center rounded-lg border border-border/40 bg-surface/60 text-xs text-foreground liquid-hover hover:bg-surface-secondary/70"
              >
                <Button
                  variant="ghost"
                  size="sm"
                  title={`${HOME_KINDS[entity.kind].label}: ${resolved.label} · ${relativeTime(recent.openedAt)}`}
                  onClick={() => void openHomeEntity(entity)}
                  className="flex h-6 min-w-0 items-center gap-1.5 bg-transparent pr-1 pl-2 font-normal text-foreground hover:bg-transparent"
                >
                  <Glyph entity={entity} resolved={resolved} size={14} />
                  <span className="truncate">{resolved.label}</span>
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={pinned ? `Unpin ${resolved.label}` : `Pin ${resolved.label} to Home`}
                  title={pinned ? "Unpin" : "Pin to Home"}
                  onClick={() => togglePin(entity)}
                  className={cn("size-6 shrink-0", pinned ? "text-accent" : "w-0 overflow-hidden px-0 opacity-0 group-hover:w-6 group-hover:opacity-100 focus-visible:w-6 focus-visible:opacity-100")}
                >
                  <Icon name="pin" size={12} />
                </Button>
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
