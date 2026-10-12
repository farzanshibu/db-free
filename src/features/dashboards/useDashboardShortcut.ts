// SOT: dashboard-shortcut-hook, capture-phase-shortcut, dashboard-active-pane
import { useEffect, useRef } from "react";
import { matchesChord, type ShortcutAction } from "@/lib/keymap";
import { useKeymap } from "@/stores/useShortcut";

// WHAT:  Runs a handler when the chord bound to a dashboard action is pressed,
//        and only when the handler says it took the key.
// WHY:   `useShortcut` swallows every matching key before its handler looks at
//        it. A dashboard's Refresh is a bare `R`: swallowing that would stop
//        anyone typing the letter into a field. ⌘S must reach the dashboard
//        without also committing pending table edits, whose handler listens on
//        the same chord.
// HOW:   One capture-phase listener on the window, so it runs first. The handler
//        returns true to claim the key (default prevented, propagation stopped)
//        or false to leave it to everyone else. It reads the latest closure
//        through a ref, so it is attached once per chord.
// WHERE: src/lib/keymap.ts (dashboard-refresh, dashboard-save), ./DashboardTab.tsx
export function useDashboardShortcut(action: ShortcutAction, handler: (event: KeyboardEvent) => boolean): void {
  const chord = useKeymap()[action];
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  useEffect(() => {
    if (chord.length === 0) return;
    const onKey = (event: KeyboardEvent) => {
      if (!matchesChord(event, chord) || !latest.current(event)) return;
      event.preventDefault();
      event.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => { window.removeEventListener("keydown", onKey, true); };
  }, [chord]);
}
