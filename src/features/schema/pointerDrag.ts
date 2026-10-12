// SOT: tag-reorder-drag, drag-ghost-label, drop-zone-hit-test-with-placement, escape-cancels-drag

// WHAT:  A drag-and-drop built on pointer events: press, move past a threshold, a
//        ghost label follows the pointer, release over a `data-drop` element.
// WHY:   HTML5 drag and drop is switched off inside a Tauri webview on Windows
//        (the window's native file-drop handler swallows it), so a sidebar that
//        reorders its tag groups cannot rely on it. Pointer events work the same
//        on every platform and let Escape cancel. Dragging a table into a pane is
//        not this: that goes through the shell's `beginPaneDrag`, which owns the
//        panes' drop zones; this one only reports which header the pointer is
//        over and whether it is in its upper or lower half.
// HOW:   `beginPointerDrag` is called from a `pointerdown` handler. Nothing happens
//        until the pointer travels `threshold` pixels, so a plain click still
//        clicks; after a real drag the click that follows the release is
//        swallowed. Drop targets are plain DOM, so an element with `data-drop="<zone>"`
//        (and optionally `data-drop-id`) whose zone the caller listed.
// WHERE: src/features/shell/TablesPanel.tsx (tag group reorder)

export type DropPlacement = "before" | "after";

export interface DropHit {
  zone: string;
  /// `data-drop-id` of the element, if it has one.
  id: string | null;
  /// Which half of the element the pointer is in (for "insert before / after").
  placement: DropPlacement;
  element: HTMLElement;
}

export interface PointerStart {
  button: number;
  clientX: number;
  clientY: number;
}

export interface PointerDragOptions<T> {
  payload: T;
  /// Text of the ghost that follows the pointer.
  label: string;
  /// Pixels of travel before a press becomes a drag.
  threshold?: number;
  /// Only elements whose `data-drop` is one of these count as targets.
  zones: readonly string[];
  /// The press crossed the threshold.
  onStart?: (payload: T) => void;
  /// The pointer moved; `hit` is the target under it, if any.
  onHover?: (hit: DropHit | null, point: { x: number; y: number }) => void;
  /// Released over a target.
  onDrop: (hit: DropHit, payload: T) => void;
  /// A drag that started has ended, dropped or cancelled.
  onEnd?: () => void;
}

function createGhost(label: string): HTMLElement {
  const ghost = document.createElement("div");
  ghost.className = "pointer-events-none fixed top-0 left-0 z-[100] max-w-64 truncate rounded-md border border-border bg-popover px-2 py-1 text-xs text-foreground shadow-md";
  ghost.setAttribute("aria-hidden", "true");
  ghost.textContent = label;
  return ghost;
}

function hitTest(x: number, y: number, zones: readonly string[]): DropHit | null {
  for (const node of document.elementsFromPoint(x, y)) {
    const element = node.closest<HTMLElement>("[data-drop]");
    if (element === null) continue;
    const zone = element.dataset.drop;
    if (zone === undefined || !zones.includes(zone)) continue;
    const box = element.getBoundingClientRect();
    return { zone, id: element.dataset.dropId ?? null, placement: y < box.top + box.height / 2 ? "before" : "after", element };
  }
  return null;
}

export function beginPointerDrag<T>(start: PointerStart, options: PointerDragOptions<T>): void {
  if (start.button !== 0) return;
  const threshold = options.threshold ?? 5;
  let dragging = false;
  let ghost: HTMLElement | null = null;
  let hit: DropHit | null = null;

  const move = (event: PointerEvent): void => {
    if (!dragging) {
      if (Math.hypot(event.clientX - start.clientX, event.clientY - start.clientY) < threshold) return;
      dragging = true;
      document.body.style.userSelect = "none";
      document.body.style.cursor = "grabbing";
      options.onStart?.(options.payload);
    }
    const element = ghost ?? createGhost(options.label);
    ghost = element;
    if (element.parentElement === null) document.body.append(element);
    element.style.transform = `translate(${event.clientX + 14}px, ${event.clientY + 14}px)`;
    hit = hitTest(event.clientX, event.clientY, options.zones);
    options.onHover?.(hit, { x: event.clientX, y: event.clientY });
  };

  const finish = (drop: boolean): void => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("keydown", key, true);
    if (!dragging) return;
    ghost?.remove();
    document.body.style.userSelect = "";
    document.body.style.cursor = "";
    const target = hit;
    options.onHover?.(null, { x: 0, y: 0 });
    if (drop && target !== null) options.onDrop(target, options.payload);
    options.onEnd?.();
    // The release would otherwise count as a click on whatever is under it.
    const swallow = (event: MouseEvent): void => {
      event.stopPropagation();
      event.preventDefault();
    };
    window.addEventListener("click", swallow, { capture: true, once: true });
    window.setTimeout(() => { window.removeEventListener("click", swallow, true); }, 0);
  };

  const up = (): void => { finish(true); };
  const cancel = (): void => { finish(false); };
  const key = (event: KeyboardEvent): void => {
    if (event.key !== "Escape") return;
    event.stopPropagation();
    finish(false);
  };

  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
  window.addEventListener("keydown", key, true);
}
