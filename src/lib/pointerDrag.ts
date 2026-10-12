// SOT: pointer-drag, drag-to-reorder, drag-to-folder, drop-targets
import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

// WHAT:  Drag and drop built on pointer events: press, move past a small distance,
//        release over an element that carries `data-drop-id`.
// WHY:   HTML5 drag-and-drop does not reach the page on Windows while the window
//        accepts OS file drops (Tauri's default), and no dependency may be added
//        for this. Pointer events work the same in every webview.
// HOW:   `begin` is called from `onPointerDown` of the draggable. Nothing starts
//        until the pointer has moved DRAG_THRESHOLD px, so a plain click still
//        clicks. While dragging, `drag` holds the payload, pointer position and
//        the id of the drop target under it; Escape cancels. `wasDragged()` is
//        true right after a drag ended so the click that follows can be ignored.
// WHERE: src/features/documents/LibraryTree.tsx, src/features/home/PinnedBoard.tsx

const DRAG_THRESHOLD = 5;

export interface PointerDrag<K> {
  payload: K;
  label: string;
  x: number;
  y: number;
  /// `data-drop-id` of the target under the pointer, or null.
  overId: string | null;
}

interface PointerDragOptions<K> {
  /// Called each time the pointer moves onto a different target (or off all of them).
  onOver?: (payload: K, overId: string | null) => void;
  /// Called on release. `overId` is null when released over no target.
  onDrop: (payload: K, overId: string | null) => void;
  /// Called when a drag that had started is cancelled (Escape, or the pointer was lost).
  onCancel?: (payload: K) => void;
}

function targetUnder(x: number, y: number): string | null {
  const element = document.elementFromPoint(x, y);
  const target = element instanceof HTMLElement ? element.closest<HTMLElement>("[data-drop-id]") : null;
  return target?.dataset.dropId ?? null;
}

export function usePointerDrag<K>(options: PointerDragOptions<K>) {
  const [drag, setDrag] = useState<PointerDrag<K> | null>(null);
  const latest = useRef(options);
  const cleanup = useRef<(() => void) | null>(null);
  const justDragged = useRef(false);

  useEffect(() => {
    latest.current = options;
  });
  // A drag in flight when the component goes away must not leave listeners behind.
  useEffect(() => () => cleanup.current?.(), []);

  const begin = useCallback((event: ReactPointerEvent<HTMLElement>, payload: K, label: string) => {
    if (event.button !== 0) return;
    cleanup.current?.();
    const startX = event.clientX;
    const startY = event.clientY;
    let started = false;
    let over: string | null = null;

    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", key);
      cleanup.current = null;
    };
    const finish = (drop: boolean, x: number, y: number) => {
      stop();
      if (started) {
        justDragged.current = true;
        // The click that follows a drag arrives in the same task; clear the flag after it.
        window.setTimeout(() => {
          justDragged.current = false;
        }, 0);
        setDrag(null);
        if (drop) latest.current.onDrop(payload, targetUnder(x, y));
        else latest.current.onCancel?.(payload);
      }
    };
    function move(e: PointerEvent) {
      if (!started) {
        if (Math.hypot(e.clientX - startX, e.clientY - startY) < DRAG_THRESHOLD) return;
        started = true;
      }
      const next = targetUnder(e.clientX, e.clientY);
      if (next !== over) {
        over = next;
        latest.current.onOver?.(payload, next);
      }
      setDrag({ payload, label, x: e.clientX, y: e.clientY, overId: next });
    }
    function up(e: PointerEvent) {
      finish(true, e.clientX, e.clientY);
    }
    function cancel(e: PointerEvent) {
      finish(false, e.clientX, e.clientY);
    }
    function key(e: KeyboardEvent) {
      if (e.key === "Escape") finish(false, 0, 0);
    }
    cleanup.current = stop;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("keydown", key);
  }, []);

  const wasDragged = useCallback(() => justDragged.current, []);
  return { drag, begin, wasDragged };
}
