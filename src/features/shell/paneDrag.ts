// SOT: pane-drag, tab-drag, sidebar-item-drag, drop-target-detection, drag-state
import type { PointerEvent as ReactPointerEvent } from "react";
import { create } from "zustand";
import type { IconName } from "@/lib/icons";
import { MAX_PANES, dropZoneAt, paneCount, paneOf, type DropZone } from "@/lib/panes";
import { useWorkspace } from "@/stores/workspace";

// WHAT:  Dragging a tab — to reorder it, to move it to another pane, or to split a
//        pane with it — or dragging an item out of a sidebar into a pane.
// WHY:   Built on pointer events, not HTML5 drag and drop: Tauri's WebView2 host
//        swallows HTML5 drag events on Windows unless file drops are disabled,
//        and a plain pointer drag also lets the ghost, the drop zones and the
//        insertion marker be drawn exactly the way the rest of the UI is.
// HOW:   `beginPaneDrag` is called from a `pointerdown`. Nothing happens until
//        the pointer has moved a few pixels, so a click is still a click. While
//        dragging, the element under the pointer decides the target: a tab strip
//        (`data-pane-strip`) gives an insertion point between its tabs
//        (`data-tab-id`), a pane (`data-pane-id`) gives one of five zones — the
//        outer quarters split it, the middle moves the tab in. Releasing drops;
//        Escape or losing the window cancels.
// WHERE: src/features/shell/TabBar.tsx (tabs), src/features/shell/PaneArea.tsx
//        (drop overlay), src/features/shell/DragLayer.tsx (ghost)

export type DragPayload =
  /// An open tab being moved.
  | { kind: "tab"; tabId: string; label: string; icon: IconName }
  /// Something that is not a tab yet (a sidebar row): `open` opens it, wherever
  /// the drop then puts the tab it opened.
  | { kind: "open"; label: string; icon: IconName; open: () => void };

export type DropTarget = { type: "pane"; paneId: string; zone: DropZone } | { type: "strip"; paneId: string; beforeId: string | null };

interface PaneDragState {
  payload: DragPayload | null;
  x: number;
  y: number;
  target: DropTarget | null;
}

export const usePaneDrag = create<PaneDragState>(() => ({ payload: null, x: 0, y: 0, target: null }));

/// Pixels the pointer must travel before a press becomes a drag.
const THRESHOLD = 5;

function datasetOf(element: Element, key: "paneStrip" | "paneId"): string | undefined {
  return element instanceof HTMLElement ? element.dataset[key] : undefined;
}

// WHAT:  What a drop at (x, y) would do.
function targetAt(x: number, y: number): DropTarget | null {
  const stack = document.elementsFromPoint(x, y);
  for (const element of stack) {
    const paneId = datasetOf(element, "paneStrip");
    if (paneId === undefined) continue;
    const tabs = [...element.querySelectorAll<HTMLElement>("[data-tab-id]")];
    // Just before the first tab whose middle is still to the right of the pointer.
    const before = tabs.find((tab) => {
      const box = tab.getBoundingClientRect();
      return x < box.left + box.width / 2;
    });
    return { type: "strip", paneId, beforeId: before?.dataset.tabId ?? null };
  }
  for (const element of stack) {
    const paneId = datasetOf(element, "paneId");
    if (paneId === undefined) continue;
    const box = element.getBoundingClientRect();
    const zone = dropZoneAt(box.width, box.height, x - box.left, y - box.top);
    // At the pane limit an edge cannot split, so it moves the tab in instead.
    const full = paneCount(useWorkspace.getState().panes) >= MAX_PANES;
    return { type: "pane", paneId, zone: full ? "center" : zone };
  }
  return null;
}

// WHAT:  Carries out a drop.
// HOW:   A tab goes through the store's `moveTab` / `dropTab`. An item that is
//        not a tab yet is opened first, and the tab that open activated is the
//        one placed — so dropping an already open table moves its tab instead
//        of opening a second one.
function drop(payload: DragPayload, target: DropTarget): void {
  const store = useWorkspace.getState();
  let tabId: string;
  if (payload.kind === "tab") {
    tabId = payload.tabId;
    // Letting go over the middle of its own pane is not a move.
    if (target.type === "pane" && target.zone === "center" && paneOf(store.panes, tabId) === target.paneId) return;
  } else {
    payload.open();
    const opened = useWorkspace.getState().activeTabId;
    if (opened === null) return;
    tabId = opened;
  }
  if (target.type === "strip") useWorkspace.getState().moveTab(tabId, target.paneId, target.beforeId);
  else useWorkspace.getState().dropTab(tabId, target.paneId, target.zone);
}

export function beginPaneDrag(event: ReactPointerEvent, payload: DragPayload): void {
  if (event.button !== 0) return;
  const startX = event.clientX;
  const startY = event.clientY;
  let dragging = false;

  const stop = (commit: boolean) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", cancel);
    window.removeEventListener("keydown", key, { capture: true });
    window.removeEventListener("blur", cancel);
    if (!dragging) return;
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
    const { target } = usePaneDrag.getState();
    usePaneDrag.setState({ payload: null, target: null });
    // The click that ends this gesture must not also activate whatever was under it.
    const swallow = (e: MouseEvent) => {
      e.stopPropagation();
      e.preventDefault();
    };
    window.addEventListener("click", swallow, { capture: true, once: true });
    window.setTimeout(() => {
      window.removeEventListener("click", swallow, { capture: true });
    }, 0);
    if (commit && target !== null) drop(payload, target);
  };

  function move(e: PointerEvent) {
    if (!dragging) {
      if (Math.hypot(e.clientX - startX, e.clientY - startY) < THRESHOLD) return;
      dragging = true;
      document.body.style.cursor = "grabbing";
      document.body.style.userSelect = "none";
    }
    usePaneDrag.setState({ payload, x: e.clientX, y: e.clientY, target: targetAt(e.clientX, e.clientY) });
  }
  function up() {
    stop(true);
  }
  function cancel() {
    stop(false);
  }
  function key(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    e.stopPropagation();
    stop(false);
  }

  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", cancel);
  window.addEventListener("keydown", key, { capture: true });
  window.addEventListener("blur", cancel);
}
