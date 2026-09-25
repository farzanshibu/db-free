// SOT: use-shortcut, keymap-hook, global-shortcut-listener
import { useEffect, useMemo, useRef } from "react";
import { matchesChord, resolveKeymap, type Keymap, type ShortcutAction } from "@/lib/keymap";

// WHAT:  The effective keymap, and a hook that runs a handler on an action's chord.
// WHY:   Components name the action ("reopen-tab"), never the keys, so a
//        rebinding in Settings reaches every handler at once.
// HOW:   One window keydown listener per hook; the handler lives in a ref so a
//        new closure each render does not re-register it. An empty chord
//        (unbound action) never matches.
// WHERE: src/lib/keymap.ts (registry and matching)
export function useKeymap(): Keymap {
  return useMemo(() => resolveKeymap(undefined), []);
}

export function useShortcut(action: ShortcutAction, handler: (event: KeyboardEvent) => void, enabled = true): void {
  const keymap = useKeymap();
  const chord = keymap[action];
  const handlerRef = useRef(handler);
  useEffect(() => {
    handlerRef.current = handler;
  });
  useEffect(() => {
    if (!enabled || chord.length === 0) return;
    const onKey = (event: KeyboardEvent) => {
      if (!matchesChord(event, chord)) return;
      event.preventDefault();
      handlerRef.current(event);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [chord, enabled]);
}
