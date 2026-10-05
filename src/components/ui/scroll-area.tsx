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
export function ScrollArea({ className, orientation = "vertical", hideScrollBar = false, style, children, ref, ...props }: ScrollAreaProps) {
  const viewport = useRef<HTMLDivElement | null>(null);
  const [edges, setEdges] = useState({ start: false, end: false });

  // WHAT:  One element, two owners: this component measures it and a caller
  //        (the data grid's virtualizer, the tab strip) may hold it too.
  // WHY:   React 19 passes `ref` as a prop, so spreading the caller's props
  //        after `ref={viewport}` silently replaced the internal ref, and the
  //        edge fade never measured on any ScrollArea that was given one.
  const setRef = useCallback(
    (node: HTMLDivElement | null) => {
      viewport.current = node;
      if (typeof ref === "function") ref(node);
      else if (ref) ref.current = node;
    },
    [ref],
  );

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

  // WHAT:  A plain mouse wheel scrolls a horizontal strip sideways.
  // WHY:   Wheels report deltaY. Without this a tab strip or toolbar that has
  //        overflowed can only be scrolled with a trackpad or shift+wheel, and
  //        the wheel either does nothing or scrolls the page behind it.
  // HOW:   Only when the strip itself has no vertical travel (the data grid is
  //        horizontal *and* vertical and keeps its native wheel), and only for
  //        a wheel with no sideways component of its own. Non-passive so the
  //        page behind does not scroll as well.
  useEffect(() => {
    const node = viewport.current;
    if (!node || orientation !== "horizontal") return;
    const onWheel = (event: WheelEvent) => {
      if (event.deltaY === 0 || Math.abs(event.deltaX) > Math.abs(event.deltaY)) return;
      if (node.scrollHeight > node.clientHeight + EPSILON) return;
      if (node.scrollWidth <= node.clientWidth + EPSILON) return;
      event.preventDefault();
      node.scrollLeft += event.deltaMode === 1 ? event.deltaY * 16 : event.deltaY;
    };
    node.addEventListener("wheel", onWheel, { passive: false });
    return () => { node.removeEventListener("wheel", onWheel); };
  }, [orientation]);

  const mask = maskImage(orientation, edges.start, edges.end);
  const merged: CSSProperties = { ...style, ...(mask === null ? {} : { maskImage: mask, WebkitMaskImage: mask }) };

  return (
    <div
      {...props}
      ref={setRef}
      onScroll={measure}
      data-slot="scroll-area"
      data-orientation={orientation}
      className={cn(
        "min-h-0 [scrollbar-color:var(--color-surface-tertiary)_transparent] [scrollbar-width:thin]",
        // `overflow-x: auto` alone computes `overflow-y` to auto as well, so a
        // horizontal strip whose children overhang by a pixel (a tab's
        // underline, a focus ring) grew a vertical scroll. Pin the cross axis;
        // a caller that scrolls both ways (the data grid) overrides it.
        orientation === "vertical" ? "overflow-x-hidden overflow-y-auto" : "overflow-x-auto overflow-y-hidden",
        hideScrollBar ? "[-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden" : "",
        className,
      )}
      style={merged}
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
