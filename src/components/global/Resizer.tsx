// SOT: resizer-handle, drag-to-expand, splitters
import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { cn } from "@/lib/cn";

export interface ResizerProps {
  direction?: "horizontal" | "vertical";
  /// Incremental delta (px) since the previous move event.
  onResize: (delta: number) => void;
  /// Fires once when the pointer is released after a drag.
  onDragEnd?: (() => void) | undefined;
  /// Accessible name of the handle ("Resize sidebar").
  label?: string | undefined;
  className?: string;
}

/// Arrow-key step (px); Shift makes it a coarse step.
const KEY_STEP = 16;
const KEY_STEP_COARSE = 64;

// WHAT:  Draggable splitter handle for expanding/collapsing sidebars,
//        inspectors, split panes and grid columns with fluid macOS-style
//        visual feedback.
// HOW:   Pointer events with pointer capture, not mouse events on `window`:
//        the grid re-renders on every delta (each column width is state), and
//        capture keeps the whole drag bound to this element even while the
//        surrounding virtualized header re-renders under the cursor. Touch and
//        pen drags come free; `touch-action: none` stops the pane scrolling
//        instead of resizing. The callbacks live in refs so a caller can hand
//        over a fresh closure every render (the grid does, per column) without
//        the drag being torn down and the body cursor reset mid-drag.
export function Resizer({ direction = "horizontal", onResize, onDragEnd, label, className }: ResizerProps) {
  const [dragging, setDragging] = useState(false);
  // Gate the move handler on a ref, not on `dragging`: the state set in
  // pointerdown has not necessarily been committed when the browser delivers
  // the first pointermove of the same gesture, and a dropped first move is the
  // difference between a drag that tracks the pointer and one that never starts.
  const active = useRef(false);
  const startPos = useRef(0);
  const resizeRef = useRef(onResize);
  const endRef = useRef(onDragEnd);
  useEffect(() => {
    resizeRef.current = onResize;
    endRef.current = onDragEnd;
  });

  // The body cursor survives a mid-drag unmount (a column scrolling out of the
  // virtualized window) only if something clears it.
  useEffect(
    () => () => {
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    },
    [],
  );

  const begin = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      e.stopPropagation();
      e.currentTarget.setPointerCapture(e.pointerId);
      startPos.current = direction === "horizontal" ? e.clientX : e.clientY;
      active.current = true;
      setDragging(true);
      document.body.style.cursor = direction === "horizontal" ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";
    },
    [direction],
  );

  const move = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      if (!active.current) return;
      const pos = direction === "horizontal" ? e.clientX : e.clientY;
      const delta = pos - startPos.current;
      if (delta === 0) return;
      startPos.current = pos;
      resizeRef.current(delta);
    },
    [direction],
  );

  const end = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      if (!active.current) return;
      active.current = false;
      if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
      setDragging(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      endRef.current?.();
    },
    [],
  );

  return (
    <div
      role="separator"
      tabIndex={0}
      aria-orientation={direction}
      aria-label={label ?? "Resize"}
      // The handle is focusable, so it is also keyboard-resizable: arrows step
      // 16 px (Shift: 64), and each step is a whole "drag" so it is remembered.
      onKeyDown={(e) => {
        const back = direction === "horizontal" ? "ArrowLeft" : "ArrowUp";
        const forward = direction === "horizontal" ? "ArrowRight" : "ArrowDown";
        if (e.key !== back && e.key !== forward) return;
        e.preventDefault();
        const step = e.shiftKey ? KEY_STEP_COARSE : KEY_STEP;
        resizeRef.current(e.key === back ? -step : step);
        endRef.current?.();
      }}
      onPointerDown={begin}
      onPointerMove={move}
      onPointerUp={end}
      onPointerCancel={end}
      // No position utility here: cn() concatenates rather than merging, so a
      // `relative` in this base would beat a caller's `absolute` (Tailwind emits
      // .relative after .absolute) and leave the handle in flow with no height —
      // a separator nothing can grab. Callers position it; the line inside gets
      // its own positioned box.
      className={cn(
        "group shrink-0 touch-none select-none transition-colors z-20",
        direction === "horizontal" ? "w-1.5 cursor-col-resize" : "h-1.5 cursor-row-resize",
        dragging ? "bg-accent/40" : "hover:bg-accent/25",
        className,
      )}
    >
      <div className="relative h-full w-full">
        <div
          className={cn(
            "pointer-events-none absolute bg-border/60 transition-colors group-hover:bg-accent",
            dragging && "bg-accent",
            direction === "horizontal" ? "top-0 bottom-0 left-[2px] w-[1px]" : "left-0 right-0 top-[2px] h-[1px]",
          )}
        />
      </div>
    </div>
  );
}
