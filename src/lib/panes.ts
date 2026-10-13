// SOT: pane-layout, pane-tree, pane-split, pane-drop-zones, pane-normalize, tab-reorder

// WHAT:  The editor area as a tree of up to four panes, and every operation on
//        it — split, close, move a tab, reorder, resize, normalise — as pure
//        functions over plain data.
// WHY:   Putting a query beside the table it reads (or two results side by side,
//        or a diagram above a grid) is the whole point of panes. The rules are
//        fiddly (an emptied pane must disappear, closing one must not lose its
//        tabs, focus must land somewhere real), so they live here, away from
//        React, where they can be reasoned about as plain data.
// HOW:   The workspace store keeps `tabs` as one flat ordered list — every
//        other feature reads and appends to it — and a PaneLayout says which
//        pane each tab sits in. A pane's tab bar is the flat list filtered to
//        its tabs, in list order, so reordering inside a pane is moving an item
//        in the flat list. A tab with no recorded pane belongs to the focused
//        one, which is how a feature that just appends to `tabs` still lands
//        its tab in the pane the user is working in. `normalize` is the single
//        repair step: it is run after any write, whoever made it.
// WHERE: src/stores/workspace.ts (state and actions), src/features/shell/PaneArea.tsx,
//        src/features/shell/TabBar.tsx

export const MAX_PANES = 4;
export const MIN_RATIO = 0.15;

/// `row`: panes side by side (a vertical divider). `column`: panes stacked.
export type SplitDirection = "row" | "column";

export type PaneNode =
  | { kind: "leaf"; id: string }
  | { kind: "split"; id: string; direction: SplitDirection; ratio: number; first: PaneNode; second: PaneNode };

export interface PaneLayout {
  root: PaneNode;
  /// The pane that has the keyboard: the one whose active tab is "the" active tab.
  focused: string;
  /// The tab each pane shows (null while it has none).
  active: Readonly<Record<string, string | null>>;
  /// Which pane each tab is in. A tab missing here belongs to the focused pane.
  owner: Readonly<Record<string, string>>;
  /// Counter behind new pane and split ids, so ids are never reused within a session.
  counter: number;
}

export function initialPanes(): PaneLayout {
  return { root: { kind: "leaf", id: "p1" }, focused: "p1", active: { p1: null }, owner: {}, counter: 1 };
}

/// Pane ids in reading order (left to right, top to bottom).
export function paneIds(node: PaneNode): string[] {
  return node.kind === "leaf" ? [node.id] : [...paneIds(node.first), ...paneIds(node.second)];
}

/// Panes whose top edge is the top of the editor area, left to right (their tab strips
/// double as the window's drag region; the last one carries the Changes button).
export function topEdgePanes(node: PaneNode): string[] {
  if (node.kind === "leaf") return [node.id];
  return node.direction === "row" ? [...topEdgePanes(node.first), ...topEdgePanes(node.second)] : topEdgePanes(node.first);
}

export function paneCount(layout: PaneLayout): number {
  return paneIds(layout.root).length;
}

export function hasPane(layout: PaneLayout, paneId: string): boolean {
  return paneIds(layout.root).includes(paneId);
}

/// The pane a tab is in: its recorded pane if that still exists, else the focused one.
export function paneOf(layout: PaneLayout, tabId: string): string {
  const recorded = layout.owner[tabId];
  return recorded !== undefined && hasPane(layout, recorded) ? recorded : layout.focused;
}

/// The tabs of one pane, in tab-list order.
export function tabsInPane<T extends { id: string }>(layout: PaneLayout, tabs: readonly T[], paneId: string): T[] {
  return tabs.filter((t) => paneOf(layout, t.id) === paneId);
}

/// The tab that is "the" active one: the focused pane's.
export function activeOf(layout: PaneLayout): string | null {
  return layout.active[layout.focused] ?? null;
}

// ------------------------------------------------------------------ the tree

function removeLeaf(node: PaneNode, paneId: string): PaneNode | null {
  if (node.kind === "leaf") return node.id === paneId ? null : node;
  const first = removeLeaf(node.first, paneId);
  const second = removeLeaf(node.second, paneId);
  if (first === null) return second;
  if (second === null) return first;
  return first === node.first && second === node.second ? node : { ...node, first, second };
}

function splitLeaf(node: PaneNode, paneId: string, newPane: PaneNode, splitId: string, direction: SplitDirection, position: "before" | "after"): PaneNode {
  if (node.kind === "leaf") {
    if (node.id !== paneId) return node;
    return { kind: "split", id: splitId, direction, ratio: 0.5, first: position === "before" ? newPane : node, second: position === "before" ? node : newPane };
  }
  const first = splitLeaf(node.first, paneId, newPane, splitId, direction, position);
  const second = splitLeaf(node.second, paneId, newPane, splitId, direction, position);
  return first === node.first && second === node.second ? node : { ...node, first, second };
}

function withRatio(node: PaneNode, splitId: string, ratio: number): PaneNode {
  if (node.kind === "leaf") return node;
  if (node.id === splitId) return node.ratio === ratio ? node : { ...node, ratio };
  const first = withRatio(node.first, splitId, ratio);
  const second = withRatio(node.second, splitId, ratio);
  return first === node.first && second === node.second ? node : { ...node, first, second };
}

/// A divider position kept inside the range that leaves both sides usable.
export function clampRatio(ratio: number): number {
  return Number.isFinite(ratio) ? Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, ratio)) : 0.5;
}

export function resizeSplit(layout: PaneLayout, splitId: string, ratio: number): PaneLayout {
  const root = withRatio(layout.root, splitId, clampRatio(ratio));
  return root === layout.root ? layout : { ...layout, root };
}

// ------------------------------------------------------------- split & close

/// Where a dropped tab lands relative to the pane under the pointer.
export type DropZone = "left" | "right" | "top" | "bottom" | "center";

// WHAT:  Which part of a pane the pointer is over: the outer quarter on the left
//        or right, the top or bottom quarter of the middle half, else the centre.
// WHY:   Edges split the pane, the centre moves the tab into it — the same
//        25% bands DB Pro uses, so the gesture carries over.
export function dropZoneAt(width: number, height: number, x: number, y: number): DropZone {
  if (width <= 0 || height <= 0) return "center";
  const fx = x / width;
  const fy = y / height;
  if (fx < 0.25) return "left";
  if (fx > 0.75) return "right";
  if (fy < 0.25) return "top";
  if (fy > 0.75) return "bottom";
  return "center";
}

export interface SplitResult {
  layout: PaneLayout;
  paneId: string;
}

// WHAT:  Splits `paneId` and gives the new pane the tab `tabId` (which must be
//        in a pane that keeps another tab, or the source would be left empty).
// HOW:   `left`/`right` put the new pane beside the source, `top`/`bottom`
//        above or below it. At the limit (four panes) nothing happens.
export function splitWithTab(layout: PaneLayout, paneId: string, zone: Exclude<DropZone, "center">, tabId: string): SplitResult | null {
  if (paneCount(layout) >= MAX_PANES || !hasPane(layout, paneId)) return null;
  const direction: SplitDirection = zone === "left" || zone === "right" ? "row" : "column";
  const position = zone === "left" || zone === "top" ? "before" : "after";
  const counter = layout.counter + 1;
  const newId = `p${String(counter)}`;
  const root = splitLeaf(layout.root, paneId, { kind: "leaf", id: newId }, `s${String(counter)}`, direction, position);
  const next: PaneLayout = {
    root,
    focused: newId,
    active: { ...layout.active, [newId]: tabId },
    owner: { ...layout.owner, [tabId]: newId },
    counter,
  };
  return { layout: next, paneId: newId };
}

// WHAT:  Closes a pane, handing its tabs to the pane that takes its place.
// WHY:   "Close pane" must never close the tabs in it; they move next door.
export function closePane(layout: PaneLayout, paneId: string): PaneLayout {
  if (paneCount(layout) <= 1 || !hasPane(layout, paneId)) return layout;
  const root = removeLeaf(layout.root, paneId);
  if (root === null) return layout;
  const remaining = paneIds(root);
  // The heir is the pane after it in reading order, else the one before.
  const before = paneIds(layout.root);
  const at = before.indexOf(paneId);
  const target = [before[at + 1], before[at - 1]].find((id) => id !== undefined && remaining.includes(id)) ?? remaining[0] ?? layout.focused;
  const owner: Record<string, string> = {};
  for (const [tab, pane] of Object.entries(layout.owner)) owner[tab] = pane === paneId ? target : pane;
  const active: Record<string, string | null> = {};
  for (const id of remaining) active[id] = layout.active[id] ?? null;
  const moved = layout.active[paneId];
  if (active[target] === null && moved !== undefined) active[target] = moved;
  const focused = layout.focused === paneId ? target : layout.focused;
  return { ...layout, root, focused, active, owner };
}

// ------------------------------------------------------------ focus & tabs

export function focusPane(layout: PaneLayout, paneId: string): PaneLayout {
  return layout.focused === paneId || !hasPane(layout, paneId) ? layout : { ...layout, focused: paneId };
}

/// Makes `tabId` the active tab of the pane it is in, and focuses that pane.
export function activateInLayout(layout: PaneLayout, tabId: string): PaneLayout {
  const pane = paneOf(layout, tabId);
  const active = layout.active[pane] === tabId ? layout.active : { ...layout.active, [pane]: tabId };
  const owner = layout.owner[tabId] === pane ? layout.owner : { ...layout.owner, [tabId]: pane };
  return active === layout.active && owner === layout.owner && layout.focused === pane ? layout : { ...layout, active, owner, focused: pane };
}

/// Moves a tab into another pane and makes it that pane's active tab.
export function moveToPane(layout: PaneLayout, tabId: string, toPane: string): PaneLayout {
  if (!hasPane(layout, toPane)) return layout;
  return { ...layout, owner: { ...layout.owner, [tabId]: toPane }, active: { ...layout.active, [toPane]: tabId }, focused: toPane };
}

// WHAT:  Moves `tabId` in the flat tab list so it sits just before `beforeId`,
//        or after the last tab of `paneId` when `beforeId` is null.
// WHY:   A pane's tab order is the flat order filtered to that pane, so a drag
//        inside a tab bar — or into another pane's bar — is one list move.
export function placeTab<T extends { id: string }>(tabs: readonly T[], layout: PaneLayout, tabId: string, paneId: string, beforeId: string | null): T[] {
  const moving = tabs.find((t) => t.id === tabId);
  if (moving === undefined || tabId === beforeId) return [...tabs];
  const rest = tabs.filter((t) => t.id !== tabId);
  let index = beforeId === null ? -1 : rest.findIndex((t) => t.id === beforeId);
  if (index < 0) {
    // After the last tab the pane already has (or at the end when it has none).
    let last = -1;
    rest.forEach((t, i) => {
      if (paneOf(layout, t.id) === paneId) last = i;
    });
    index = last < 0 ? rest.length : last + 1;
  }
  return [...rest.slice(0, index), moving, ...rest.slice(index)];
}

// ---------------------------------------------------------------- normalise

// WHAT:  Makes a layout consistent with the open tabs, whoever changed them.
// WHY:   Features append to `tabs` and set `activeTabId` directly (that is the
//        contract they were given). This is the one place the layout catches up:
//        forgotten tabs are dropped from it, new tabs join the focused pane,
//        every pane shows one of its own tabs, an emptied pane disappears (its
//        space goes to its neighbour) and focus lands on a pane that exists.
// HOW:   `requestedActive` is a tab someone just activated through the flat
//        `activeTabId`; if it is in another pane that pane takes the focus.
//        Returns the same object when nothing needed repair, so subscribers
//        see no change.
export function normalize(layout: PaneLayout, tabIds: readonly string[], requestedActive: string | null): PaneLayout {
  let next = layout;
  const known = new Set(tabIds);

  // 1. Every open tab has an owner that exists; closed tabs are forgotten.
  const owner: Record<string, string> = {};
  let ownerChanged = false;
  for (const id of tabIds) {
    const pane = paneOf(next, id);
    owner[id] = pane;
    if (next.owner[id] !== pane) ownerChanged = true;
  }
  if (!ownerChanged && Object.keys(next.owner).length !== tabIds.length) ownerChanged = true;
  if (ownerChanged) next = { ...next, owner };

  // 2. Panes with no tabs go away (never the last one).
  for (const id of paneIds(next.root)) {
    if (paneCount(next) > 1 && !tabIds.some((t) => paneOf(next, t) === id)) next = closePane(next, id);
  }

  // 3. A tab someone activated through the flat id focuses its own pane.
  if (requestedActive !== null && known.has(requestedActive) && activeOf(next) !== requestedActive) next = activateInLayout(next, requestedActive);

  // 4. Each pane shows one of its own tabs.
  const active: Record<string, string | null> = {};
  let activeChanged = false;
  for (const id of paneIds(next.root)) {
    const mine = tabIds.filter((t) => paneOf(next, t) === id);
    const current = next.active[id] ?? null;
    const shown = current !== null && mine.includes(current) ? current : (mine[mine.length - 1] ?? null);
    active[id] = shown;
    if (shown !== current || !(id in next.active)) activeChanged = true;
  }
  if (Object.keys(next.active).length !== Object.keys(active).length) activeChanged = true;
  if (activeChanged) next = { ...next, active };

  // 5. Focus is a real pane.
  if (!hasPane(next, next.focused)) next = { ...next, focused: paneIds(next.root)[0] ?? "p1" };
  return next;
}

/// The active tab of the first pane that is not the focused one (the old "split" tab), or null.
export function sideTabOf(layout: PaneLayout): string | null {
  for (const id of paneIds(layout.root)) {
    if (id !== layout.focused) return layout.active[id] ?? null;
  }
  return null;
}
