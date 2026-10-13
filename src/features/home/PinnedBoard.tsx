// SOT: pinned-board, pin-cards, pin-reorder, unpin
import { useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { HomePin } from "@/lib/bindings";
import { HOME_KINDS, entityKey } from "@/lib/home";
import { usePointerDrag } from "@/lib/pointerDrag";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { useWorkspace } from "@/stores/workspace";
import { EnvBadge } from "@/components/global/Badge";
import { EngineIcon } from "@/components/global/EngineIcon";
import { Button } from "@/components/ui/button";
import { openHomeEntity } from "./openEntity";
import { useResolveEntity } from "./useResolvedEntity";

// WHAT:  Moves `id` to where `overId` is in the list (the dragged card takes the
//        target's slot, the rest shift).
function moveTo(order: readonly string[], id: string, overId: string): string[] {
  const from = order.indexOf(id);
  const to = order.indexOf(overId);
  if (from < 0 || to < 0 || from === to) return [...order];
  const next = [...order];
  next.splice(from, 1);
  next.splice(to, 0, id);
  return next;
}

// WHAT:  The Home "Pinned" board: one card per pinned connection, table, saved query,
//        dashboard, workflow or diagram. Click opens it (connecting first), the pin
//        icon unpins, drag reorders — and Alt+←/→ does the same from the keyboard.
// WHY:   The few things someone returns to daily should be one click from launch, and
//        the order they chose should stay.
// HOW:   Drag is pointer-event based (src/lib/pointerDrag.ts): while dragging, the
//        cards re-sort live in a local preview; on release the order is stored.
//        Cards whose connection or item no longer exists are left out.
// WHERE: src/stores/workspace.ts (pins), src-tauri/src/store/home.rs
export function PinnedBoard({ pins }: { pins: readonly HomePin[] }) {
  const unpinItem = useWorkspace((s) => s.unpinItem);
  const reorderPins = useWorkspace((s) => s.reorderPins);
  const resolve = useResolveEntity();
  const [preview, setPreview] = useState<string[] | null>(null);
  const working = useRef<string[] | null>(null);

  const shown = useMemo(() => pins.filter((p) => resolve(p.entity) !== null), [pins, resolve]);
  const ids = useMemo(() => shown.map((p) => p.id), [shown]);
  const order = preview ?? ids;
  const byId = new Map(shown.map((p) => [p.id, p]));

  const { drag, begin, wasDragged } = usePointerDrag<string>({
    onOver: (id, overId) => {
      if (overId === null || overId === id) return;
      const next = moveTo(working.current ?? ids, id, overId);
      working.current = next;
      setPreview(next);
    },
    onDrop: () => {
      const final = working.current;
      working.current = null;
      setPreview(null);
      if (final !== null && final.join() !== ids.join()) void reorderPins(final);
    },
    onCancel: () => {
      working.current = null;
      setPreview(null);
    },
  });

  const nudge = (event: ReactKeyboardEvent, id: string, step: -1 | 1) => {
    if (!event.altKey) return;
    const at = ids.indexOf(id);
    const target = ids[at + step];
    if (target === undefined) return;
    event.preventDefault();
    void reorderPins(moveTo(ids, id, target));
  };

  return (
    <div className="flex flex-wrap gap-2">
      {order.map((id) => {
        const pin = byId.get(id);
        if (pin === undefined) return null;
        const resolved = resolve(pin.entity);
        if (resolved === null) return null;
        const { entity } = pin;
        const kind = HOME_KINDS[entity.kind];
        const dragging = drag?.payload === id;
        return (
          <div
            key={entityKey(entity)}
            data-drop-id={pin.id}
            onPointerDown={(event) => begin(event, pin.id, resolved.label)}
            className={cn(
              "group relative w-40 cursor-grab touch-none select-none rounded-lg border border-border/40 glass-card glass-card-hover",
              dragging ? "opacity-40" : "",
            )}
          >
            <Button
              variant="ghost"
              title={`${kind.label}: ${resolved.label}`}
              onClick={() => {
                if (!wasDragged()) void openHomeEntity(entity);
              }}
              onKeyDown={(event) => {
                if (event.key === "ArrowLeft") nudge(event, pin.id, -1);
                else if (event.key === "ArrowRight") nudge(event, pin.id, 1);
              }}
              className="flex h-auto w-full flex-col items-start justify-start gap-1 rounded-lg bg-transparent p-2 pr-5 text-left font-normal hover:bg-transparent"
            >
              <span className="flex w-full min-w-0 items-center gap-2">
                {entity.kind === "connection" ? (
                  <EngineIcon engine={resolved.connection.engine} size={16} />
                ) : (
                  <Icon name={kind.icon} size={14} className="shrink-0 text-accent" />
                )}
                <span className="min-w-0 truncate text-xs font-semibold tracking-tight text-foreground">{resolved.label}</span>
              </span>
              <span className="flex w-full min-w-0 items-center gap-1.5 text-[11px] text-muted">
                <span className="min-w-0 truncate">{entity.kind === "connection" ? kind.label : resolved.connection.name}</span>
                <EnvBadge environment={resolved.connection.environment} />
              </span>
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={`Unpin ${resolved.label}`}
              title="Unpin"
              onPointerDown={(event) => event.stopPropagation()}
              onClick={() => void unpinItem(entity.kind, entity.itemId)}
              className="absolute top-0.5 right-0.5 size-5 min-w-5 text-accent"
            >
              <Icon name="pin" size={12} />
            </Button>
          </div>
        );
      })}
    </div>
  );
}
