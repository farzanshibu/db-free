// SOT: layout-presets, sidebar-width-per-connection, inspector-width, layout-storage
import { readStored, writeStored } from "./storage";

// WHAT:  The shape of the workbench — left sidebar, right inspector, the chat —
//        as a handful of numbers, and the five named presets that set them.
// WHY:   "Focus" (nothing but the tabs), "Explore" (a wide sidebar for browsing
//        a big schema), "Inspect" (record inspector open) and "AI" (chat first)
//        are one command each instead of several drags and clicks.
// HOW:   Pure data plus tiny storage helpers; the store (workspace.ts) owns the
//        live values. A sidebar width is remembered per connection, because a
//        three-table SQLite file and a four-hundred-table warehouse want
//        different widths.
// WHERE: src/stores/workspace.ts (applyLayout), src/features/shell/Sidebar.tsx

export type LayoutPresetId = "default" | "focus" | "explore" | "inspect" | "ai";

export interface LayoutState {
  sidebarCollapsed: boolean;
  sidebarWidth: number;
  inspectorOpen: boolean;
  inspectorWidth: number;
}

export interface LayoutPreset {
  id: LayoutPresetId;
  label: string;
  description: string;
  layout: LayoutState;
  /// DB Free's assistant is a tab, not a side panel: "AI" also opens it.
  opensChat: boolean;
}

export const SIDEBAR_MIN = 180;
export const SIDEBAR_MAX = 520;
export const INSPECTOR_MIN = 280;
export const INSPECTOR_MAX = 720;

export const DEFAULT_LAYOUT: LayoutState = { sidebarCollapsed: false, sidebarWidth: 330, inspectorOpen: false, inspectorWidth: 400 };

export const LAYOUT_PRESETS: readonly LayoutPreset[] = [
  { id: "default", label: "Default", description: "Sidebar open, inspector closed.", layout: DEFAULT_LAYOUT, opensChat: false },
  { id: "focus", label: "Focus", description: "Sidebar hidden: tabs and results only.", layout: { ...DEFAULT_LAYOUT, sidebarCollapsed: true }, opensChat: false },
  { id: "explore", label: "Explore", description: "A wider sidebar for browsing large schemas.", layout: { ...DEFAULT_LAYOUT, sidebarWidth: 400 }, opensChat: false },
  {
    id: "inspect",
    label: "Inspect",
    description: "Narrow sidebar, record inspector open and wide.",
    layout: { ...DEFAULT_LAYOUT, sidebarWidth: 280, inspectorOpen: true, inspectorWidth: 480 },
    opensChat: false,
  },
  { id: "ai", label: "AI", description: "Sidebar hidden, the assistant open.", layout: { ...DEFAULT_LAYOUT, sidebarCollapsed: true }, opensChat: true },
];

export function layoutPreset(id: LayoutPresetId): LayoutPreset | undefined {
  return LAYOUT_PRESETS.find((p) => p.id === id);
}

export function isLayoutPresetId(value: string): value is LayoutPresetId {
  return LAYOUT_PRESETS.some((p) => p.id === value);
}

export function clampWidth(width: number, min: number, max: number): number {
  return Number.isFinite(width) ? Math.min(max, Math.max(min, Math.round(width))) : min;
}

const SIDEBAR_WIDTH_KEY = "db-free:sidebar-width";
const COLLAPSED_KEY = "db-free:sidebar-collapsed";
const INSPECTOR_WIDTH_KEY = "db-free:inspector-width";

function readWidth(key: string, min: number, max: number): number | null {
  const raw = readStored(key);
  if (raw === null) return null;
  const value = Number(raw);
  return Number.isFinite(value) ? clampWidth(value, min, max) : null;
}

/// The width remembered for a connection, else the old single global width, else the default.
export function readSidebarWidth(connectionId: string | null): number {
  const own = connectionId === null ? null : readWidth(`${SIDEBAR_WIDTH_KEY}:${connectionId}`, SIDEBAR_MIN, SIDEBAR_MAX);
  return own ?? readWidth(SIDEBAR_WIDTH_KEY, SIDEBAR_MIN, SIDEBAR_MAX) ?? DEFAULT_LAYOUT.sidebarWidth;
}

export function writeSidebarWidth(connectionId: string | null, width: number): void {
  const key = connectionId === null ? SIDEBAR_WIDTH_KEY : `${SIDEBAR_WIDTH_KEY}:${connectionId}`;
  writeStored(key, String(clampWidth(width, SIDEBAR_MIN, SIDEBAR_MAX)));
}

export function readSidebarCollapsed(): boolean {
  return readStored(COLLAPSED_KEY) === "1";
}

export function writeSidebarCollapsed(collapsed: boolean): void {
  writeStored(COLLAPSED_KEY, collapsed ? "1" : "0");
}

export function readInspectorWidth(): number {
  return readWidth(INSPECTOR_WIDTH_KEY, INSPECTOR_MIN, INSPECTOR_MAX) ?? DEFAULT_LAYOUT.inspectorWidth;
}

export function writeInspectorWidth(width: number): void {
  writeStored(INSPECTOR_WIDTH_KEY, String(clampWidth(width, INSPECTOR_MIN, INSPECTOR_MAX)));
}
