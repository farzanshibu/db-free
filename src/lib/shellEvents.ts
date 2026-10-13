// SOT: shell-events, window-context, menu-action-listener, workspace-sync-broadcast, app-error-listener
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppErrorEvent, MenuActionEvent } from "./bindings";

// WHAT:  The events that cross window boundaries or come from the native shell,
//        and what a window knows about itself.
// WHY:   A window is told which connection it was opened for through the URL
//        (`?window=conn-<id>&connection=<id>`), and the native menu, a panic and
//        a settings save in another window all arrive as events. Keeping both in
//        one module means no component imports the event API directly.
// HOW:   Every `on…` returns an unlisten function and never throws: in a plain
//        browser (`pnpm preview:ui`) there is no event bridge, and the app must
//        still render.
// WHERE: src-tauri/src/commands/windows.rs (opens windows), src-tauri/src/menu.rs
//        and src-tauri/src/diagnostics.rs (emit), src/features/shell/useNativeMenu.ts

export interface WindowContext {
  /// `main`, `conn-<id>` or `win-<n>`.
  label: string;
  /// The connection this window was opened for; null for `main` and extra windows.
  connectionId: string | null;
}

let cachedContext: WindowContext | null = null;

export function windowContext(): WindowContext {
  if (cachedContext !== null) return cachedContext;
  const query = new URLSearchParams(typeof window === "undefined" ? "" : window.location.search);
  const label = query.get("window");
  const connection = query.get("connection");
  cachedContext = { label: label !== null && label.length > 0 ? label : "main", connectionId: connection !== null && connection.length > 0 ? connection : null };
  return cachedContext;
}

export function isMainWindow(): boolean {
  return windowContext().label === "main";
}

/// Nothing to undo: what `on…` returns when the event bridge is missing.
function nothing(): void {
  return undefined;
}

/// Starts a listener; a missing event bridge yields a listener that does nothing.
async function guarded(start: () => Promise<UnlistenFn>): Promise<UnlistenFn> {
  try {
    return await start();
  } catch {
    return nothing;
  }
}

// WHAT:  A native menu item was chosen in this window.
// WHY:   The menu is app-wide and its event is broadcast; the payload names the
//        focused window and every other window drops the event.
export function onMenuAction(handler: (action: string) => void): Promise<UnlistenFn> {
  const label = windowContext().label;
  return guarded(() =>
    listen<MenuActionEvent>("menu:action", (e) => {
      if (e.payload.target === label) handler(e.payload.action);
    }),
  );
}

/// Something failed on the Rust side with no request to carry the error back (a panic).
export function onAppError(handler: (message: string) => void): Promise<UnlistenFn> {
  return guarded(() =>
    listen<AppErrorEvent>("app:error", (e) => {
      handler(e.payload.message);
    }),
  );
}

// ------------------------------------------------------- cross-window state

/// What another window changed that this one should reload.
export type SyncKind = "settings" | "connections" | "sessions";

interface SyncEvent {
  source: string;
  kind: SyncKind;
}

// WHAT:  Tells the other windows that settings, the connection list or the live
//        sessions changed here.
// WHY:   Each window keeps its own copy of that state in memory; without a nudge
//        a theme change or a new connection would only show up after a restart.
export async function broadcastSync(kind: SyncKind): Promise<void> {
  try {
    const event: SyncEvent = { source: windowContext().label, kind };
    await emit("workspace:sync", event);
  } catch {
    // No event bridge (browser preview) or a window mid-close: other windows catch up on focus.
  }
}

export function onSync(handler: (kind: SyncKind) => void): Promise<UnlistenFn> {
  const label = windowContext().label;
  return guarded(() =>
    listen<SyncEvent>("workspace:sync", (e) => {
      if (e.payload.source !== label) handler(e.payload.kind);
    }),
  );
}
