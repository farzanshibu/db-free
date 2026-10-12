// SOT: dashboard-grid, tile-frame, tile-drag, tile-resize, draw-to-create-tile, tile-context-menu, tile-keyboard-move
import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import type { Widget } from "@/lib/bindings";
import { cn } from "@/lib/cn";
import { Icon } from "@/lib/icons";
import { useContextMenu, type MenuEntry } from "@/components/global/ContextMenu";
import { Spinner } from "@/components/ui/spinner";
import { tileAppearance } from "./appearance";
import type { TileRun } from "./dashboardResults";
import {
  DRAG_THRESHOLD,
  NARROW_WIDTH,
  cellRect,
  drawnCell,
  finalizeDrawn,
  gridHeight,
  metricsFor,
  nudge,
  pixelsToCells,
  pointToCell,
  previewMove,
  resizeTo,
  rowsUsed,
  settleDrop,
  slotForDrawn,
  stackLayout,
  type Cell,
  type Placed,
  type Rect,
} from "./layout";
import { TileBody } from "./TileBody";
import { ON_GRADIENT } from "./tiles";

// WHAT:  The dashboard canvas: tiles at their saved x / y / w / h on a 12-column
//        grid, dragged by their title bar (dropping onto a tile swaps them),
//        resized from the corner (stopping at the next tile), created by
//        drawing a rectangle on empty space, and stacked in one column when the
//        canvas is narrow.
// WHY:   The saved geometry used to be ignored: a CSS grid flowed the tiles in
//        order and two sizes were all you could pick. Placement is the point of
//        a dashboard.
// HOW:   All geometry is ./layout.ts; this file owns the pointer handling. A
//        gesture keeps the *base* layout from when it started and shows a
//        derived preview, so nothing is written until release, and a release
//        that did not move is a click. Pointer capture keeps the gesture alive
//        outside the element. Alt+arrows move a focused tile, Alt+Shift+arrows
//        resize it, for those without a pointer.
// WHERE: ./DashboardTab.tsx (owns the widgets), ./TileBody.tsx (what a tile draws)

interface GridProps {
  widgets: readonly Widget[];
  runs: Readonly<Record<string, TileRun | undefined>>;
  running: ReadonlySet<string>;
  selectedId: string | null;
  /// Row height of table tiles, from the density setting.
  rowHeight: number;
  onSelect: (id: string | null) => void;
  /// The widgets with new x / y / w / h.
  onLayout: (next: Widget[]) => void;
  /// A rectangle was drawn: where the new tile goes.
  onCreate: (cell: Cell) => void;
  onEdit: (id: string) => void;
  onDuplicate: (id: string) => void;
  onRemove: (id: string) => void;
  /// Shown, centred, while there are no tiles.
  empty: ReactNode;
}

type Gesture =
  | { kind: "move" | "resize"; id: string; pointerId: number; startX: number; startY: number; startScroll: number; origin: Placed; base: Placed[]; active: boolean }
  | { kind: "draw"; pointerId: number; startX: number; startY: number; active: boolean };

interface Live {
  layout: Placed[];
  /// The tile being dragged follows the pointer by this many pixels.
  drag: { id: string; dx: number; dy: number } | null;
  /// The rectangle being drawn.
  draw: Cell | null;
}

const MENU: readonly MenuEntry[] = [
  { id: "edit", label: "Edit", icon: "pencil", group: "main" },
  { id: "duplicate", label: "Duplicate", icon: "copy", group: "main" },
  { id: "remove", label: "Remove", icon: "trash", danger: true, group: "danger" },
];

const EDGE_SCROLL_ZONE = 36;
const EDGE_SCROLL_STEP = 14;
/// Rows of empty space kept under the last tile, so there is always room to draw.
const SPARE_ROWS = 4;

function samePlacement(a: Cell, b: Cell): boolean {
  return a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}

export function DashboardGrid({ widgets, runs, running, selectedId, rowHeight, onSelect, onLayout, onCreate, onEdit, onDuplicate, onRemove, empty }: GridProps) {
  const scroller = useRef<HTMLDivElement | null>(null);
  const inner = useRef<HTMLDivElement | null>(null);
  const [width, setWidth] = useState(0);
  const [viewport, setViewport] = useState(0);
  const [live, setLive] = useState<Live | null>(null);
  const liveRef = useRef<Live | null>(null);
  const gesture = useRef<Gesture | null>(null);
  const lastGestureAt = useRef(0);
  const menu = useContextMenu();

  useEffect(() => {
    const node = scroller.current;
    if (!node) return;
    const observer = new ResizeObserver((entries) => {
      const box = entries[0]?.contentRect;
      if (!box) return;
      setWidth(box.width);
      setViewport(box.height);
    });
    observer.observe(node);
    return () => { observer.disconnect(); };
  }, []);

  const narrow = width > 0 && width < NARROW_WIDTH;
  const metrics = useMemo(() => metricsFor(Math.max(width, 1)), [width]);
  const base = useMemo<Placed[]>(() => widgets.map((w) => ({ id: w.id, x: w.x, y: w.y, w: w.w, h: w.h })), [widgets]);
  const stacked = useMemo(() => (narrow ? stackLayout(base) : null), [narrow, base]);
  const shown = stacked ?? live?.layout ?? base;
  const byId = useMemo(() => new Map(widgets.map((w) => [w.id, w])), [widgets]);

  const setLiveNow = (next: Live | null) => {
    liveRef.current = next;
    setLive(next);
  };

  const commit = (layout: readonly Placed[]) => {
    const next = widgets.map((w) => {
      const at = layout.find((p) => p.id === w.id);
      return at === undefined || samePlacement(at, w) ? w : { ...w, x: at.x, y: at.y, w: at.w, h: at.h };
    });
    if (next.some((w, i) => w !== widgets[i])) onLayout(next);
  };

  const autoScroll = (clientY: number) => {
    const node = scroller.current;
    if (!node) return;
    const box = node.getBoundingClientRect();
    if (clientY > box.bottom - EDGE_SCROLL_ZONE) node.scrollTop += EDGE_SCROLL_STEP;
    else if (clientY < box.top + EDGE_SCROLL_ZONE) node.scrollTop -= EDGE_SCROLL_STEP;
  };

  // ---------------------------------------------------------- move and resize
  const begin = (kind: "move" | "resize", id: string, event: ReactPointerEvent<HTMLElement>) => {
    if (narrow || event.button !== 0) return;
    const origin = base.find((t) => t.id === id);
    if (origin === undefined) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    gesture.current = { kind, id, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, startScroll: scroller.current?.scrollTop ?? 0, origin, base, active: false };
  };

  const move = (event: ReactPointerEvent<HTMLElement>) => {
    const g = gesture.current;
    if (g === null || g.kind === "draw" || g.pointerId !== event.pointerId) return;
    const dx = event.clientX - g.startX;
    const dy = event.clientY - g.startY + ((scroller.current?.scrollTop ?? 0) - g.startScroll);
    if (!g.active) {
      if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
      g.active = true;
    }
    autoScroll(event.clientY);
    const cols = pixelsToCells(dx, metrics.col, metrics.gap);
    const rows = pixelsToCells(dy, metrics.row, metrics.gap);
    if (g.kind === "move") {
      setLiveNow({ layout: previewMove(g.base, g.id, { x: g.origin.x + cols, y: g.origin.y + rows }), drag: { id: g.id, dx, dy }, draw: null });
    } else {
      setLiveNow({ layout: resizeTo(g.base, g.id, { w: g.origin.w + cols, h: g.origin.h + rows }), drag: null, draw: null });
    }
  };

  const end = (event: ReactPointerEvent<HTMLElement>) => {
    const g = gesture.current;
    if (g === null || g.kind === "draw" || g.pointerId !== event.pointerId) return;
    gesture.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    const last = liveRef.current;
    setLiveNow(null);
    if (!g.active || last === null) return;
    lastGestureAt.current = event.timeStamp;
    commit(g.kind === "move" ? settleDrop(last.layout, g.id) : last.layout);
  };

  const cancel = (event: ReactPointerEvent<HTMLElement>) => {
    if (gesture.current?.pointerId === event.pointerId) gesture.current = null;
    setLiveNow(null);
  };

  // ---------------------------------------------------------- draw to create
  const point = (event: ReactPointerEvent<HTMLElement>) => {
    const box = inner.current?.getBoundingClientRect();
    return { x: event.clientX - (box?.left ?? 0), y: event.clientY - (box?.top ?? 0) };
  };

  const drawDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (narrow || event.button !== 0 || event.target !== event.currentTarget) return;
    // A popover or menu being dismissed by this press must not also start a tile.
    if (document.querySelector("[data-radix-popper-content-wrapper]") !== null) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    const at = point(event);
    gesture.current = { kind: "draw", pointerId: event.pointerId, startX: at.x, startY: at.y, active: false };
  };

  const drawMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const g = gesture.current;
    if (g?.kind !== "draw" || g.pointerId !== event.pointerId) return;
    const at = point(event);
    if (!g.active) {
      if (Math.hypot(at.x - g.startX, at.y - g.startY) < DRAG_THRESHOLD) return;
      g.active = true;
    }
    autoScroll(event.clientY);
    setLiveNow({ layout: base, drag: null, draw: drawnCell(pointToCell(g.startX, g.startY, metrics), pointToCell(at.x, at.y, metrics)) });
  };

  const drawEnd = (event: ReactPointerEvent<HTMLDivElement>) => {
    const g = gesture.current;
    if (g?.kind !== "draw" || g.pointerId !== event.pointerId) return;
    gesture.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    const drawn = liveRef.current?.draw ?? null;
    setLiveNow(null);
    if (!g.active || drawn === null) {
      onSelect(null);
      return;
    }
    const cell = finalizeDrawn(drawn);
    onCreate({ ...cell, ...slotForDrawn(base, cell) });
  };

  // ---------------------------------------------------------- keyboard
  const keyDown = (id: string, event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.target !== event.currentTarget) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect(id);
      return;
    }
    const step = event.key === "ArrowLeft" ? [-1, 0] : event.key === "ArrowRight" ? [1, 0] : event.key === "ArrowUp" ? [0, -1] : event.key === "ArrowDown" ? [0, 1] : null;
    if (step === null || !event.altKey || narrow) return;
    event.preventDefault();
    const [dx = 0, dy = 0] = step;
    const tile = base.find((t) => t.id === id);
    if (tile === undefined) return;
    commit(event.shiftKey ? resizeTo(base, id, { w: tile.w + dx, h: tile.h + dy }) : nudge(base, id, dx, dy));
  };

  const rows = rowsUsed(shown);
  const height = Math.max(gridHeight(rows + SPARE_ROWS, metrics), viewport);
  const dragging = live?.drag ?? null;
  const ghost = dragging === null ? null : shown.find((t) => t.id === dragging.id);

  return (
    <div ref={scroller} className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto">
      <div
        ref={inner}
        className="relative select-none"
        style={{ height, touchAction: narrow ? "auto" : "pan-y" }}
        onPointerDown={drawDown}
        onPointerMove={drawMove}
        onPointerUp={drawEnd}
        onPointerCancel={() => { gesture.current = null; setLiveNow(null); }}
      >
        {width > 0
          ? shown.map((cell) => {
              const widget = byId.get(cell.id);
              if (widget === undefined) return null;
              const offset = dragging !== null && dragging.id === cell.id ? dragging : null;
              const home = offset === null ? cell : (base.find((t) => t.id === cell.id) ?? cell);
              const rect = cellRect(home, metrics);
              return (
                <TileFrame
                  key={cell.id}
                  widget={widget}
                  run={runs[cell.id]}
                  running={running.has(cell.id)}
                  selected={selectedId === cell.id}
                  left={rect.left + (offset?.dx ?? 0)}
                  top={rect.top + (offset?.dy ?? 0)}
                  width={rect.width}
                  height={rect.height}
                  dragging={offset !== null}
                  locked={narrow}
                  rowHeight={rowHeight}
                  onGrab={(event) => { begin("move", cell.id, event); }}
                  onGrow={(event) => { begin("resize", cell.id, event); }}
                  onMove={move}
                  onEnd={end}
                  onCancel={cancel}
                  onKey={(event) => { keyDown(cell.id, event); }}
                  onSelect={(event) => { if (event.timeStamp - lastGestureAt.current > 250) onSelect(cell.id); }}
                  onMenu={(event) => {
                    menu.open(event, MENU, (action) => {
                      if (action === "edit") onEdit(cell.id);
                      else if (action === "duplicate") onDuplicate(cell.id);
                      else onRemove(cell.id);
                    });
                  }}
                />
              );
            })
          : null}
        {ghost ? <Outline rect={cellRect(ghost, metrics)} /> : null}
        {live?.draw ? <Outline rect={cellRect(live.draw, metrics)} /> : null}
        {widgets.length === 0 && (live?.draw ?? null) === null ? <div className="pointer-events-none absolute inset-0 flex items-center justify-center">{empty}</div> : null}
      </div>
      {menu.node}
    </div>
  );
}

function Outline({ rect }: { rect: Rect }) {
  return <div className="pointer-events-none absolute rounded-lg border border-dashed border-accent/70 bg-accent/10 transition-[left,top,width,height] duration-100" style={{ left: rect.left, top: rect.top, width: rect.width, height: rect.height }} />;
}

interface FrameProps {
  widget: Widget;
  run: TileRun | undefined;
  running: boolean;
  selected: boolean;
  left: number;
  top: number;
  width: number;
  height: number;
  dragging: boolean;
  locked: boolean;
  rowHeight: number;
  onGrab: (event: ReactPointerEvent<HTMLElement>) => void;
  onGrow: (event: ReactPointerEvent<HTMLElement>) => void;
  onMove: (event: ReactPointerEvent<HTMLElement>) => void;
  onEnd: (event: ReactPointerEvent<HTMLElement>) => void;
  onCancel: (event: ReactPointerEvent<HTMLElement>) => void;
  onKey: (event: ReactKeyboardEvent<HTMLElement>) => void;
  onSelect: (event: ReactMouseEvent<HTMLElement>) => void;
  onMenu: (event: ReactMouseEvent<HTMLElement>) => void;
}

// WHAT:  One tile: a title bar that is the drag handle, the body, a resize corner.
// HOW:   Dressed by `tileAppearance` (accent edge, gradient, white text on a
//        gradient). The frame is cheap; what is costly, the body, is memoised on
//        the widget and the result, so moving a frame never redraws a chart.
function TileFrame({ widget, run, running, selected, left, top, width, height, dragging, locked, rowHeight, onGrab, onGrow, onMove, onEnd, onCancel, onKey, onSelect, onMenu }: FrameProps) {
  const outcome = run?.outcome ?? null;
  const appearance = useMemo(() => tileAppearance(widget, outcome), [widget, outcome]);
  const ink = appearance.onGradient ? { color: ON_GRADIENT } : undefined;
  const title = widget.title.length > 0 ? widget.title : "Untitled tile";
  return (
    <div
      role="group"
      aria-label={title}
      className={cn(
        "absolute flex flex-col overflow-hidden rounded-lg border bg-surface",
        selected ? "border-accent" : "border-border",
        dragging ? "z-20 opacity-90 shadow-md" : "transition-[left,top,width,height] duration-150",
      )}
      style={{ left, top, width, height, borderTopWidth: 2, borderTopColor: appearance.accent, backgroundImage: appearance.background }}
      onClick={onSelect}
      onContextMenu={onMenu}
    >
      <div
        role="button"
        tabIndex={0}
        aria-label={`${title}. Enter to edit; Alt with the arrow keys moves it, with Shift resizes it.`}
        className={cn("flex h-6 shrink-0 items-center gap-2 px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/60 focus-visible:ring-inset", locked ? "" : "cursor-grab touch-none active:cursor-grabbing")}
        onPointerDown={onGrab}
        onPointerMove={onMove}
        onPointerUp={onEnd}
        onPointerCancel={onCancel}
        onKeyDown={onKey}
      >
        <span className={cn("truncate", widget.title.length > 0 ? "text-muted" : "text-muted/60")} style={ink}>{title}</span>
        {run?.error ? <Icon name="alert" size={12} className="shrink-0 text-danger" /> : null}
        {running ? <Spinner size="sm" className="ml-auto shrink-0" /> : null}
      </div>
      <div className="min-h-0 flex-1">
        <TileBody widget={widget} outcome={outcome} appearance={appearance} rowHeight={rowHeight} error={run?.error ?? null} />
      </div>
      {locked ? null : (
        <div
          aria-hidden="true"
          className="absolute right-0 bottom-0 z-10 size-4 cursor-se-resize touch-none"
          onPointerDown={onGrow}
          onPointerMove={onMove}
          onPointerUp={onEnd}
          onPointerCancel={onCancel}
        >
          <span className="absolute right-1 bottom-1 size-2 border-r-2 border-b-2 border-muted/70" />
        </div>
      )}
    </div>
  );
}

