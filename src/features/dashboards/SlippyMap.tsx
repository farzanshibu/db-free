// SOT: slippy-map, map-tile-viewer, map-markers, map-pan-zoom
import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { Icon } from "@/lib/icons";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { clampView, fitView, panBy, pointsKey, toScreen, visibleTiles, zoomAt, type Basemap, type MapPoint, type MapView } from "./mapTiles";

// WHAT:  The map tile's body: a Carto basemap drawn as raster tiles in <img>
//        elements, markers with tooltips, drag to pan, pinch / Ctrl+wheel /
//        buttons to zoom, and a fit-to-points reset.
// WHY:   A map library is a renderer, a style engine and a worker; this tile
//        needs a background and some dots. Raster tiles and the arithmetic in
//        ./mapTiles.ts do the job with nothing to install and nothing that
//        a content-security policy has to learn beyond the tile host.
// HOW:   The view (centre, fractional zoom) lives in state; every pointer
//        gesture is a pure function of the view. It refits when the *set of
//        points* changes, not on every refresh that returns the same ones, so a
//        user's pan survives an auto-refresh. A plain wheel scrolls the
//        dashboard; the zoom wheel needs Ctrl / ⌘ (which is also what a
//        trackpad pinch sends).
// WHERE: ./charts.tsx (MapChart), ./mapTiles.ts, src-tauri/tauri.conf.json (img-src)

interface Size {
  width: number;
  height: number;
}

const MARKER_LIMIT = 600;
const WHEEL_ZOOM_PER_PIXEL = 0.01;

export function SlippyMap({ points, tint, pulse, basemap, hint }: { points: readonly MapPoint[]; tint: string; pulse: boolean; basemap: Basemap; hint: string }) {
  const host = useRef<HTMLDivElement | null>(null);
  const [size, setSize] = useState<Size>({ width: 0, height: 0 });
  // The view and the set of points it was fitted for: a different set refits.
  const [fitted, setFitted] = useState<{ key: string; view: MapView } | null>(null);
  const [hover, setHover] = useState<number | null>(null);
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const pinch = useRef<number | null>(null);
  const key = useMemo(() => pointsKey(points), [points]);

  useEffect(() => {
    const node = host.current;
    if (!node) return;
    const observer = new ResizeObserver((entries) => {
      const box = entries[0]?.contentRect;
      if (box) setSize((prev) => (prev.width === box.width && prev.height === box.height ? prev : { width: box.width, height: box.height }));
    });
    observer.observe(node);
    return () => { observer.disconnect(); };
  }, []);

  // Fit when the points change (or the first time the size is known); a refresh
  // that returns the same points leaves the user's view alone. Done during
  // render, from the props, rather than in an effect that sets state.
  const measured = size.width > 0 && size.height > 0;
  if (measured && fitted?.key !== key) setFitted({ key, view: fitView(points, size.width, size.height) });
  // A resized tile keeps its centre and only corrects a zoom the new size forbids.
  const view = fitted === null || !measured ? null : clampView(fitted.view, size.width, size.height);
  const update = useCallback((change: (v: MapView) => MapView) => { setFitted((f) => (f === null ? f : { key: f.key, view: change(f.view) })); }, []);

  // Zoom with the wheel only while Ctrl / ⌘ is down; otherwise let the dashboard scroll.
  useEffect(() => {
    const node = host.current;
    if (!node) return;
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();
      const rect = node.getBoundingClientRect();
      update((v) => zoomAt(v, -event.deltaY * WHEEL_ZOOM_PER_PIXEL, event.clientX - rect.left, event.clientY - rect.top, rect.width, rect.height));
    };
    node.addEventListener("wheel", onWheel, { passive: false });
    return () => { node.removeEventListener("wheel", onWheel); };
  }, [update]);

  const local = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const rect = host.current?.getBoundingClientRect();
    return { x: event.clientX - (rect?.left ?? 0), y: event.clientY - (rect?.top ?? 0) };
  }, []);

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || (event.target instanceof Element && event.target.closest("[data-map-control]") !== null)) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    pointers.current.set(event.pointerId, local(event));
    if (pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      pinch.current = a && b ? Math.hypot(a.x - b.x, a.y - b.y) : null;
    }
  };

  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const before = pointers.current.get(event.pointerId);
    if (before === undefined) return;
    const now = local(event);
    pointers.current.set(event.pointerId, now);
    if (pointers.current.size === 1) {
      update((v) => panBy(v, now.x - before.x, now.y - before.y, size.width, size.height));
      return;
    }
    const [a, b] = [...pointers.current.values()];
    if (a === undefined || b === undefined || pinch.current === null) return;
    const distance = Math.hypot(a.x - b.x, a.y - b.y);
    const delta = Math.log2(Math.max(distance, 1) / Math.max(pinch.current, 1));
    pinch.current = distance;
    update((v) => zoomAt(v, delta, (a.x + b.x) / 2, (a.y + b.y) / 2, size.width, size.height));
  };

  const onPointerEnd = (event: ReactPointerEvent<HTMLDivElement>) => {
    pointers.current.delete(event.pointerId);
    if (pointers.current.size < 2) pinch.current = null;
  };

  const zoomBy = (delta: number) => { update((v) => zoomAt(v, delta, size.width / 2, size.height / 2, size.width, size.height)); };
  const refit = () => { update(() => fitView(points, size.width, size.height)); };

  const ready = view !== null;
  const retina = typeof window !== "undefined" && window.devicePixelRatio > 1;
  const tiles = ready ? visibleTiles(view, size.width, size.height, basemap, retina) : [];
  const markers = ready
    ? points.slice(0, MARKER_LIMIT).map((p, index) => ({ index, p, at: toScreen(p, view, size.width, size.height) })).filter((m) => m.at.x > -24 && m.at.x < size.width + 24 && m.at.y > -24 && m.at.y < size.height + 24)
    : [];
  const hovered = hover === null ? undefined : markers.find((m) => m.index === hover);

  return (
    <div
      ref={host}
      role="application"
      aria-label="Map"
      tabIndex={0}
      className="relative h-full w-full cursor-grab touch-none overflow-hidden bg-surface-secondary outline-none select-none focus-visible:ring-2 focus-visible:ring-ring/60 active:cursor-grabbing"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
      onKeyDown={(event) => {
        if (event.key === "+" || event.key === "=") zoomBy(1);
        else if (event.key === "-") zoomBy(-1);
        else if (event.key === "0") refit();
      }}
    >
      {tiles.map((t) => (
        <img
          key={t.key}
          src={t.url}
          alt=""
          draggable={false}
          referrerPolicy="no-referrer"
          className="pointer-events-none absolute max-w-none"
          style={{ left: t.left, top: t.top, width: t.size + 0.5, height: t.size + 0.5 }}
          onError={(event) => { event.currentTarget.style.visibility = "hidden"; }}
        />
      ))}
      {markers.map(({ index, p, at }) => (
        <span
          key={index}
          className="absolute"
          style={{ left: at.x, top: at.y, transform: "translate(-50%, -50%)" }}
          onPointerEnter={() => { setHover(index); }}
          onPointerLeave={() => { setHover((h) => (h === index ? null : h)); }}
        >
          {pulse ? <span className="absolute inset-0 -m-1.5 animate-ping rounded-full opacity-40" style={{ background: tint }} /> : null}
          <span className="relative block size-3 rounded-full border-2" style={{ background: tint, borderColor: "var(--surface)" }} title={p.label.length > 0 ? p.label : `${p.lat.toFixed(3)}, ${p.lon.toFixed(3)}`} />
        </span>
      ))}
      {hovered ? (
        <div
          className="pointer-events-none absolute z-10 max-w-56 rounded-md border border-border bg-overlay px-2 py-1 text-[11px] text-foreground shadow"
          style={{ left: hovered.at.x, top: hovered.at.y - 12, transform: "translate(-50%, -100%)" }}
        >
          {hovered.p.label.length > 0 ? <div className="truncate">{hovered.p.label}</div> : null}
          <div className="text-muted tabular-nums">{hovered.p.lat.toFixed(4)}, {hovered.p.lon.toFixed(4)}</div>
        </div>
      ) : null}
      {points.length === 0 && ready ? (
        <div className="pointer-events-none absolute inset-x-0 top-1/2 flex -translate-y-1/2 justify-center px-3">
          <span className="rounded-md border border-border bg-overlay px-2 py-1 text-center text-xs text-muted">{hint}</span>
        </div>
      ) : null}
      <div data-map-control className="absolute top-1.5 right-1.5 z-10 flex flex-col gap-1">
        <Button size="icon-sm" variant="secondary" aria-label="Zoom in" onClick={() => { zoomBy(1); }}>
          <Icon name="plus" size={13} />
        </Button>
        <Button size="icon-sm" variant="secondary" aria-label="Zoom out" onClick={() => { zoomBy(-1); }}>
          <Icon name="minus" size={13} />
        </Button>
        <Button size="icon-sm" variant="secondary" aria-label="Fit to points" disabled={points.length === 0} onClick={refit}>
          <Icon name="expand" size={13} />
        </Button>
      </div>
      <span className={cn("pointer-events-none absolute right-1.5 bottom-0.5 z-10 rounded bg-overlay/70 px-1 text-[9px] text-muted")}>© OpenStreetMap contributors © CARTO</span>
    </div>
  );
}
