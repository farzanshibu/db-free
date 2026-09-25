// SOT: keymap, shortcut-registry, shortcut-actions, chord-parsing, chord-display

// WHAT:  Every app-level keyboard shortcut, as one registry of actions with a
//        default chord, plus the parsing / matching / display of chords.
// WHY:   Shortcuts were string literals in five components and a hand-kept
//        list on the Settings page that could drift from them. One registry
//        means the list, the handlers and (later) user overrides agree.
// HOW:   A chord is "Mod+Shift+T": modifiers joined with "+", key last. Mod is
//        ⌘ on macOS and Ctrl elsewhere. Keys compare case-insensitively on
//        `KeyboardEvent.key`, with the named keys Enter, Tab, Escape, Space,
//        ArrowUp/Down/Left/Right, Backspace, Delete, F1–F12.
// WHERE: src/stores/useShortcut.ts (the hook), src/features/settings/SettingsPage.tsx
export type ShortcutAction =
  | "palette"
  | "run"
  | "run-all"
  | "format"
  | "commit-changes"
  | "new-query"
  | "close-tab"
  | "reopen-tab"
  | "next-tab"
  | "prev-tab";

export type ShortcutGroup = "General" | "Editor" | "Tabs" | "Data";

export interface ShortcutMeta {
  label: string;
  group: ShortcutGroup;
  keys: string;
}

export const SHORTCUTS = {
  palette: { label: "Command palette", group: "General", keys: "Mod+K" },
  "commit-changes": { label: "Commit pending changes", group: "Data", keys: "Mod+S" },
  run: { label: "Run statement / selection", group: "Editor", keys: "Mod+Enter" },
  "run-all": { label: "Run whole script", group: "Editor", keys: "Mod+Shift+Enter" },
  format: { label: "Format SQL", group: "Editor", keys: "Shift+Alt+F" },
  "new-query": { label: "New query tab", group: "Tabs", keys: "Mod+T" },
  "close-tab": { label: "Close tab", group: "Tabs", keys: "Mod+W" },
  "reopen-tab": { label: "Reopen closed tab", group: "Tabs", keys: "Mod+Shift+T" },
  "next-tab": { label: "Next tab", group: "Tabs", keys: "Ctrl+Tab" },
  "prev-tab": { label: "Previous tab", group: "Tabs", keys: "Ctrl+Shift+Tab" },
} satisfies Record<ShortcutAction, ShortcutMeta>;

export const SHORTCUT_ACTIONS: readonly ShortcutAction[] = ["palette", "commit-changes", "run", "run-all", "format", "new-query", "close-tab", "reopen-tab", "next-tab", "prev-tab"];

export function isShortcutAction(value: string): value is ShortcutAction {
  return value in SHORTCUTS;
}

/// One override as stored in settings; an empty `keys` unbinds the action.
export interface KeyOverride {
  action: string;
  keys: string;
}

export type Keymap = Record<ShortcutAction, string>;

export function resolveKeymap(overrides: readonly KeyOverride[] | undefined): Keymap {
  const map: Keymap = {
    palette: SHORTCUTS.palette.keys,
    run: SHORTCUTS.run.keys,
    "run-all": SHORTCUTS["run-all"].keys,
    format: SHORTCUTS.format.keys,
    "commit-changes": SHORTCUTS["commit-changes"].keys,
    "new-query": SHORTCUTS["new-query"].keys,
    "close-tab": SHORTCUTS["close-tab"].keys,
    "reopen-tab": SHORTCUTS["reopen-tab"].keys,
    "next-tab": SHORTCUTS["next-tab"].keys,
    "prev-tab": SHORTCUTS["prev-tab"].keys,
  };
  for (const o of overrides ?? []) if (isShortcutAction(o.action)) map[o.action] = normalizeChord(o.keys);
  return map;
}

export const IS_MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);

interface Chord {
  mod: boolean;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
  key: string;
}

function parseChord(chord: string): Chord | null {
  const parts = chord.split("+").map((p) => p.trim()).filter((p) => p.length > 0);
  const key = parts.pop();
  if (key === undefined) return null;
  const has = (name: string) => parts.some((p) => p.toLowerCase() === name);
  return { mod: has("mod"), ctrl: has("ctrl"), alt: has("alt"), shift: has("shift"), meta: has("meta") || has("cmd"), key: key.toLowerCase() };
}

/// Canonical spelling: modifiers in a fixed order, the key as typed.
export function normalizeChord(chord: string): string {
  const parsed = parseChord(chord);
  if (!parsed) return "";
  const out: string[] = [];
  if (parsed.mod) out.push("Mod");
  if (parsed.ctrl) out.push("Ctrl");
  if (parsed.meta) out.push("Meta");
  if (parsed.alt) out.push("Alt");
  if (parsed.shift) out.push("Shift");
  out.push(parsed.key.length === 1 ? parsed.key.toUpperCase() : parsed.key.charAt(0).toUpperCase() + parsed.key.slice(1));
  return out.join("+");
}

function eventKey(event: KeyboardEvent): string {
  if (event.key === " ") return "space";
  // With Alt held, macOS reports the composed character (Alt+F is "ƒ"); the
  // physical key is what the chord names.
  if (event.code.startsWith("Key")) return event.code.slice(3).toLowerCase();
  if (event.code.startsWith("Digit")) return event.code.slice(5);
  return event.key.toLowerCase();
}

export function matchesChord(event: KeyboardEvent, chord: string): boolean {
  const c = parseChord(chord);
  if (!c) return false;
  const wantCtrl = c.ctrl || (c.mod && !IS_MAC);
  const wantMeta = c.meta || (c.mod && IS_MAC);
  return event.ctrlKey === wantCtrl && event.metaKey === wantMeta && event.altKey === c.alt && event.shiftKey === c.shift && eventKey(event) === c.key;
}

/// The chord a key event spells, for recording a new binding. Null while only
/// modifiers are held.
export function chordFromEvent(event: KeyboardEvent): string | null {
  if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return null;
  const parts: string[] = [];
  if (IS_MAC ? event.metaKey : event.ctrlKey) parts.push("Mod");
  if (IS_MAC && event.ctrlKey) parts.push("Ctrl");
  if (!IS_MAC && event.metaKey) parts.push("Meta");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  parts.push(eventKey(event));
  return normalizeChord(parts.join("+"));
}

/// CodeMirror keymap spelling ("Mod-Shift-Enter").
export function toCodeMirrorKey(chord: string): string | null {
  const c = parseChord(chord);
  if (!c) return null;
  const parts: string[] = [];
  if (c.mod) parts.push("Mod");
  if (c.ctrl) parts.push("Ctrl");
  if (c.meta) parts.push("Meta");
  if (c.alt) parts.push("Alt");
  if (c.shift) parts.push("Shift");
  parts.push(c.key.length === 1 ? c.key : c.key.charAt(0).toUpperCase() + c.key.slice(1));
  return parts.join("-");
}

/// "⌘ ⇧ T" on macOS, "Ctrl + Shift + T" elsewhere.
export function displayChord(chord: string): string {
  const c = parseChord(chord);
  if (!c) return "—";
  const key = c.key.length === 1 ? c.key.toUpperCase() : c.key.charAt(0).toUpperCase() + c.key.slice(1);
  if (IS_MAC) {
    return [c.ctrl ? "⌃" : "", c.alt ? "⌥" : "", c.shift ? "⇧" : "", c.mod || c.meta ? "⌘" : "", key === "Enter" ? "↩" : key].filter((p) => p.length > 0).join(" ");
  }
  return [c.mod || c.ctrl ? "Ctrl" : "", c.meta ? "Win" : "", c.alt ? "Alt" : "", c.shift ? "Shift" : "", key].filter((p) => p.length > 0).join(" + ");
}
