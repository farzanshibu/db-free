// SOT: scroll-area, scroll-shadow, overflow-container
import { useCallback, useEffect, useRef, useState, type ComponentProps, type CSSProperties } from "react";
import { cn } from "@/lib/cn";

type Orientation = "vertical" | "horizontal";

export interface ScrollAreaProps extends ComponentProps<"div"> {
  /// Axis that scrolls. The other axis is left to the caller's own classes.
  orientation?: Orientation;
  /// Hide the scrollbar track entirely (dense toolbars, tab strips).
  hideScrollBar?: boolean;
}

/// Distance from an edge, in pixels, still counted as "against it".
const EPSILON = 2;
/// Depth of the fade at an edge that has more content behind it.
const FADE = "12px";

// WHAT:  A scroll container that fades its content out at the edges it can
//        still scroll toward — and only those.
// WHY:   shadcn's ScrollArea wraps Radix, which replaces the native scroller
//        with its own viewport element. Three things here need the real one:
//        TanStack Virtual measures the scrolling element directly (the data
//        grid), and the `min-h-0 flex-1` panels and `max-h-*` popovers rely on
//        intrinsic sizing that a Radix viewport's inner wrapper collapses. So
//        this keeps native overflow and does the shadcn job — chrome-free
//        scrolling with an edge affordance.
// HOW:   The fade is a mask, recomputed on scroll and on resize. A static mask
//        would dim the first and last 12px of every panel permanently, which
//        reads as content disappearing under a gradient rather than as an
//        affordance; there is no CSS-only way to ask "can this still scroll?".
// WHERE: Panels, dialog bodies, tab strips — the app's default scroll surface.
export function ScrollArea({ className, orientation = "vertical", hideScrollBar = false, style, children, ...props }: ScrollAreaProps) {
  const viewport = useRef<HTMLDivElement | null>(null);
  const [edges, setEdges] = useState({ start: false, end: false });

  const measure = useCallback(() => {
    const node = viewport.current;
    if (!node) return;
    const [offset, size, client] =
      orientation === "vertical"
        ? [node.scrollTop, node.scrollHeight, node.clientHeight]
        : [node.scrollLeft, node.scrollWidth, node.clientWidth];
    const next = { start: offset > EPSILON, end: offset + client < size - EPSILON };
    setEdges((prev) => (prev.start === next.start && prev.end === next.end ? prev : next));
  }, [orientation]);

  useEffect(() => {
    const node = viewport.current;
    if (!node) return;
    measure();
    // Content that grows, a panel that resizes, or a filter that shortens a list
    // all change whether an edge can still be scrolled toward.
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    for (const child of node.children) observer.observe(child);
    return () => { observer.disconnect(); };
  }, [measure, children]);

  const mask = maskImage(orientation, edges.start, edges.end);
  const merged: CSSProperties = { ...style, ...(mask === null ? {} : { maskImage: mask, WebkitMaskImage: mask }) };

  return (
    <div
      ref={viewport}
      onScroll={measure}
      data-slot="scroll-area"
      data-orientation={orientation}
      className={cn(
        "min-h-0 [scrollbar-color:var(--color-surface-tertiary)_transparent] [scrollbar-width:thin]",
        orientation === "vertical" ? "overflow-y-auto" : "overflow-x-auto",
        hideScrollBar ? "[-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden" : "",
        className,
      )}
      style={merged}
      {...props}
    >
      {children}
    </div>
  );
}

/// `null` when neither edge has anything behind it — no mask at all, so nothing
/// is dimmed in a panel whose content already fits.
function maskImage(orientation: Orientation, start: boolean, end: boolean): string | null {
  if (!start && !end) return null;
  const direction = orientation === "vertical" ? "to bottom" : "to right";
  const from = start ? `transparent 0, black ${FADE}` : "black 0";
  const to = end ? `black calc(100% - ${FADE}), transparent 100%` : "black 100%";
  return `linear-gradient(${direction}, ${from}, ${to})`;
}
