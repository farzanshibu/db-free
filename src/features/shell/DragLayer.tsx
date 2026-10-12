// SOT: drag-ghost, drag-layer
import { Icon } from "@/lib/icons";
import { usePaneDrag } from "./paneDrag";

// WHAT:  The label that follows the pointer while a tab or a sidebar item is dragged.
// WHY:   Without it a drag is invisible until it lands. It is a plain fixed
//        element, pointer-events off, so it never becomes the drop target itself.
// WHERE: src/features/shell/paneDrag.ts (state), src/App.tsx (mounted once)
export function DragLayer() {
  const payload = usePaneDrag((s) => s.payload);
  const x = usePaneDrag((s) => s.x);
  const y = usePaneDrag((s) => s.y);
  if (payload === null) return null;
  return (
    <div
      role="presentation"
      aria-hidden
      className="pointer-events-none fixed z-[100] flex max-w-[220px] items-center gap-1.5 rounded-lg px-2 py-1 text-[12px] font-medium text-foreground glass-modal"
      style={{ left: x + 14, top: y + 12 }}
    >
      <Icon name={payload.icon} size={12.5} className="text-accent" />
      <span className="truncate">{payload.label}</span>
    </div>
  );
}
