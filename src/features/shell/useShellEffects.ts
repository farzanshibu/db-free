// SOT: shell-shortcuts-hook, native-menu-hook, window-effects, zoom-apply, native-window-theme, ui-error-log, cross-window-sync-hook, update-launch-check
import { useEffect } from "react";
import { ipc, normalizeError } from "@/lib/ipc";
import { matchesChord } from "@/lib/keymap";
import { isMainWindow, onAppError, onMenuAction, onSync } from "@/lib/shellEvents";
import { useKeymap } from "@/stores/useShortcut";
import { useFollowsSystem, useResolvedTheme } from "@/stores/useTheme";
import { useWorkspace } from "@/stores/workspace";
import { MENU_SHORTCUTS, SHELL_SHORTCUTS, runShellAction } from "./shellActions";
import { launchCheckDue, useUpdates } from "./useUpdates";

// WHAT:  The window-level behaviours App.tsx turns on once: keyboard actions, the
//        native menu, zoom, the OS window theme, error reporting, other windows'
//        changes and the update check at launch.
// WHY:   Each is a side effect with its own setup and teardown. Keeping them out
//        of App.tsx leaves it as layout, and gives each one a name to read.
// WHERE: src/App.tsx

/// ⌘N, ⌘W, ⌘J … : one listener over every shell action, resolved through the live keymap.
export function useShellShortcuts(): void {
  const keymap = useKeymap();
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      // A key the focused widget already used (the editor's own ⌘L, say) is not ours to take.
      if (event.defaultPrevented) return;
      for (const action of SHELL_SHORTCUTS) {
        const chord = keymap[action];
        if (chord.length === 0 || !matchesChord(event, chord)) continue;
        event.preventDefault();
        runShellAction(action);
        return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [keymap]);
}

/// Native menu choices run the same actions, and the menu's key hints follow the user's keymap.
export function useNativeMenu(): void {
  const keymap = useKeymap();
  useEffect(() => {
    const pending = onMenuAction((action) => {
      runShellAction(action);
    });
    return () => {
      void pending.then((unlisten) => {
        unlisten();
      });
    };
  }, []);
  useEffect(() => {
    const shortcuts = Object.fromEntries(MENU_SHORTCUTS.map((action) => [action, keymap[action]]));
    // No native menu (browser preview) or a window mid-close: the keys still work.
    void ipc("set_menu_shortcuts", { shortcuts }).catch(() => undefined);
  }, [keymap]);
}

export function useWindowEffects(): void {
  const settingsLoaded = useWorkspace((s) => s.settings !== null);
  const zoom = useWorkspace((s) => s.settings?.uiZoom);
  const showError = useWorkspace((s) => s.showError);
  const syncShared = useWorkspace((s) => s.syncShared);
  const follows = useFollowsSystem();
  const base = useResolvedTheme();

  // Whole-app zoom: the webview applies it; settings saved in another window arrive the same way.
  useEffect(() => {
    if (zoom === undefined) return;
    void ipc("set_ui_zoom", { zoom }).catch(() => undefined);
  }, [zoom]);

  // The OS window chrome follows the app. "System" hands it back to the OS — forcing a
  // scheme there would also pin the webview's own `prefers-color-scheme`.
  useEffect(() => {
    if (!settingsLoaded) return;
    void ipc("set_window_theme", { theme: follows ? null : base }).catch(() => undefined);
  }, [settingsLoaded, follows, base]);

  // A failure on the Rust side with no request to answer (a panic) still reaches the user.
  useEffect(() => {
    const pending = onAppError((message) => {
      showError(message);
    });
    return () => {
      void pending.then((unlisten) => {
        unlisten();
      });
    };
  }, [showError]);

  // Uncaught errors go to the log file, where a bug report can attach them.
  useEffect(() => {
    const report = (source: string, message: string) => {
      void ipc("log_ui_error", { message, source }).catch(() => undefined);
    };
    const onError = (event: ErrorEvent) => {
      report("window.onerror", event.error instanceof Error ? (event.error.stack ?? event.message) : event.message);
    };
    const onRejection = (event: PromiseRejectionEvent) => {
      report("unhandledrejection", normalizeError(event.reason).message);
    };
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
    };
  }, []);

  // Another window saved settings or changed connections: reload them. A window coming
  // back to the front catches up on everything it may have missed.
  useEffect(() => {
    const pending = onSync((kind) => {
      void syncShared(kind);
    });
    const onFocus = () => {
      void syncShared("all");
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
      void pending.then((unlisten) => {
        unlisten();
      });
    };
  }, [syncShared]);
}

/// Once per app launch, in the main window only; guards against StrictMode running the effect twice.
let launchChecked = false;

// WHAT:  The update check at launch.
// WHY:   Telling someone to open Settings and find Updates is three steps too
//        many; checking in the background and downloading first means the only
//        question left is when to restart. Throttled, and switchable in Settings.
export function useLaunchUpdateCheck(): void {
  const enabled = useWorkspace((s) => s.settings?.checkUpdatesOnLaunch);
  useEffect(() => {
    if (launchChecked || enabled === undefined || !isMainWindow()) return;
    launchChecked = true;
    if (enabled && launchCheckDue()) void useUpdates.getState().check(true);
  }, [enabled]);
}
