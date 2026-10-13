// SOT: scroll-area, scroll-shadow, overflow-container
import { useCallback, useEffect, useRef, useState, type ComponentProps, type CSSProperties } from "react";
import { cn } from "@/lib/cn";

type Orientation = "vertical" | "horizontal" | "both";

export interface ScrollAreaProps extends ComponentProps<"div"> {
  /// Axis that scrolls. `both` scrolls and fades on all four edges.
  orientation?: Orientation;
  /// Hide the scrollbar track entirely (dense toolbars, tab strips).
  hideScrollBar?: boolean;
}

/// Distance from an edge, in pixels, still counted as "against it".
const EPSILON = 2;
/// Depth of the fade at an edge that has more content behind it.
const FADE = "12px";

interface Axis { start: boolean; end: boolean }
const NONE: Axis = { start: false, end: false };

// WHAT:  A scroll container that fades its content out at the edges it can
//        still scroll toward — and only those. Vertical, horizontal or both.
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
//        For `both`, one gradient per axis, intersected.
// WHERE: Panels, dialog bodies, tab strips — the app's default scroll surface.
//        Put borders and backgrounds on a wrapper, not on this element: the
//        mask fades the element's own border along with its content.
export function ScrollArea({ className, orientation = "vertical", hideScrollBar = false, style, children, ref, onScroll, ...props }: ScrollAreaProps) {
  const viewport = useRef<HTMLDivElement | null>(null);
  const [edges, setEdges] = useState<{ y: Axis; x: Axis }>({ y: NONE, x: NONE });

  const measure = useCallback(() => {
    const node = viewport.current;
    if (!node) return;
    const axis = (offset: number, size: number, client: number): Axis => ({
      start: offset > EPSILON,
      end: offset + client < size - EPSILON,
    });
    const next = {
      y: orientation === "horizontal" ? NONE : axis(node.scrollTop, node.scrollHeight, node.clientHeight),
      x: orientation === "vertical" ? NONE : axis(node.scrollLeft, node.scrollWidth, node.clientWidth),
    };
    setEdges((prev) =>
      prev.y.start === next.y.start && prev.y.end === next.y.end && prev.x.start === next.x.start && prev.x.end === next.x.end ? prev : next,
    );
  }, [orientation]);

  const attach = useCallback(
    (node: HTMLDivElement | null) => {
      viewport.current = node;
      if (typeof ref === "function") ref(node);
      else if (ref) ref.current = node;
    },
    [ref],
  );

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

  const masks = [
    orientation === "horizontal" ? null : maskImage("to bottom", edges.y),
    orientation === "vertical" ? null : maskImage("to right", edges.x),
  ].filter((mask): mask is string => mask !== null);
  const merged: CSSProperties = {
    ...style,
    ...(masks.length === 0
      ? {}
      : {
          maskImage: masks.join(", "),
          WebkitMaskImage: masks.join(", "),
          maskComposite: "intersect",
          WebkitMaskComposite: "source-in",
        }),
  };

  return (
    <div
      ref={attach}
      onScroll={(event) => { measure(); onScroll?.(event); }}
      data-slot="scroll-area"
      data-orientation={orientation}
      className={cn(
        "min-h-0 [scrollbar-color:var(--scrollbar-thumb)_var(--scrollbar-track)] [scrollbar-width:thin]",
        orientation === "vertical" ? "overflow-y-auto overflow-x-hidden" : orientation === "horizontal" ? "overflow-x-auto overflow-y-hidden" : "overflow-auto",
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
/// is dimmed in a panel whose content already fits. With `both`, an axis that
/// cannot scroll still contributes no mask, so it never dims anything.
function maskImage(direction: "to bottom" | "to right", axis: Axis): string | null {
  if (!axis.start && !axis.end) return null;
  const from = axis.start ? `transparent 0, black ${FADE}` : "black 0";
  const to = axis.end ? `black calc(100% - ${FADE}), transparent 100%` : "black 100%";
  return `linear-gradient(${direction}, ${from}, ${to})`;
}
