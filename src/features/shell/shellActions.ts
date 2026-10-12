// SOT: shell-actions, shortcut-action-handlers, zoom-steps, menu-action-map, project-links
import { useWorkspace } from "@/stores/workspace";
import { ipc, normalizeError } from "@/lib/ipc";
import { isLayoutPresetId } from "@/lib/layouts";
import type { ShortcutAction } from "@/lib/keymap";
import { useUpdates } from "./useUpdates";

// WHAT:  What every window-level action does — the keyboard shortcuts, the native
//        menu items and the palette entries that share them — as plain functions
//        over the workspace store.
// WHY:   "Close tab" must do exactly the same thing from ⌘W, from File → Close Tab
//        and from the tab's own button, and a rebinding in Settings must reach all
//        of them. One function per action, looked up by id, makes that structural.
// HOW:   `runShellAction(id)` returns whether it knew the id. Ids are keymap
//        actions (`close-tab`) or menu-only ones (`about`, `layout-focus`).
// WHERE: src/features/shell/useShellShortcuts.ts (keys), src/features/shell/useNativeMenu.ts (menu)

export const PROJECT_URL = "https://github.com/farzanshibu/db-free";

/// Whole-app zoom stops (Settings, ⌘+ / ⌘-); the setting always holds one of these.
export const ZOOM_STEPS: readonly number[] = [0.9, 1, 1.1, 1.25, 1.5];

/// The stop one step in `direction` from `current`; stays at the ends.
export function stepZoom(current: number, direction: 1 | -1): number {
  const nearest = ZOOM_STEPS.reduce((best, step, i) => (Math.abs(step - current) < Math.abs((ZOOM_STEPS[best] ?? 1) - current) ? i : best), 0);
  const next = Math.min(ZOOM_STEPS.length - 1, Math.max(0, nearest + direction));
  return ZOOM_STEPS[next] ?? 1;
}

/// Keyboard actions the shell handles (the palette handles its own chord).
export const SHELL_SHORTCUTS: readonly ShortcutAction[] = [
  "new-query",
  "close-tab",
  "reopen-tab",
  "next-tab",
  "prev-tab",
  "goto-next-tab",
  "goto-prev-tab",
  "settings",
  "erd",
  "toggle-sidebar",
  "toggle-inspector",
  "zoom-in",
  "zoom-out",
  "zoom-reset",
  "new-window",
  "close-window",
];

/// Actions that have a native menu item, and so a hint the menu shows next to it.
export const MENU_SHORTCUTS: readonly ShortcutAction[] = [
  "new-query",
  "new-window",
  "close-tab",
  "close-window",
  "settings",
  "palette",
  "toggle-sidebar",
  "toggle-inspector",
  "erd",
  "zoom-in",
  "zoom-out",
  "zoom-reset",
  "prev-tab",
  "next-tab",
];

/// Whatever a rejected promise or a `catch` block hands over (the type `normalizeError` accepts).
type Rejection = Parameters<typeof normalizeError>[0];

function fail(raw: Rejection): void {
  useWorkspace.getState().showError(normalizeError(raw));
}

// WHAT:  Applies a zoom factor to this window and remembers it.
// HOW:   The webview applies the factor (Rust clamps it); the setting is saved so
//        the next launch and every other window use it too.
export async function applyZoom(zoom: number): Promise<void> {
  const state = useWorkspace.getState();
  const settings = state.settings;
  if (settings === null || settings.uiZoom === zoom) return;
  try {
    await state.saveSettings({ ...settings, uiZoom: zoom }, null);
  } catch (raw) {
    fail(raw);
  }
}

function activeConnection(): string | null {
  const s = useWorkspace.getState();
  return s.tabs.find((t) => t.id === s.activeTabId)?.connectionId ?? s.activeConnectionId;
}

// WHAT:  Runs one action by id; false when the id is not a shell action.
export function runShellAction(action: string): boolean {
  const s = useWorkspace.getState();
  switch (action) {
    case "new-query": {
      const id = activeConnection();
      if (id !== null && s.sessions.includes(id)) s.openQuery(id);
      return true;
    }
    case "close-tab":
      if (s.activeTabId !== null) s.requestCloseTabs([s.activeTabId]);
      return true;
    case "reopen-tab":
      s.reopenClosedTab();
      return true;
    case "next-tab":
    case "goto-next-tab":
      s.cycleTab(1);
      return true;
    case "prev-tab":
    case "goto-prev-tab":
      s.cycleTab(-1);
      return true;
    case "palette":
      s.setPaletteOpen(!s.paletteOpen);
      return true;
    case "settings":
      s.goSettings();
      return true;
    case "erd": {
      const id = activeConnection();
      if (id !== null && s.sessions.includes(id)) s.openErd(id, s.schemaFilter[id] ?? null);
      return true;
    }
    case "toggle-sidebar":
      s.toggleSidebar();
      return true;
    case "toggle-inspector":
      s.toggleInspector();
      return true;
    case "zoom-in":
      void applyZoom(stepZoom(s.settings?.uiZoom ?? 1, 1));
      return true;
    case "zoom-out":
      void applyZoom(stepZoom(s.settings?.uiZoom ?? 1, -1));
      return true;
    case "zoom-reset":
      void applyZoom(1);
      return true;
    case "new-window":
      void ipc("open_window", { connectionId: null }).catch(fail);
      return true;
    case "close-window":
      void ipc("close_window").catch(fail);
      return true;
    case "about":
      s.setAboutOpen(true);
      return true;
    case "check-updates":
      void useUpdates.getState().check(false);
      return true;
    case "whats-new":
      useWorkspace.setState({ page: { kind: "whats-new" } });
      return true;
    case "learn-more":
      void ipc("open_external", { url: PROJECT_URL }).catch(fail);
      return true;
    default: {
      const preset = action.startsWith("layout-") ? action.slice("layout-".length) : null;
      if (preset !== null && isLayoutPresetId(preset)) {
        s.applyLayout(preset);
        return true;
      }
      return false;
    }
  }
}
