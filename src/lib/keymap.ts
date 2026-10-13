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
import type { KeyBinding } from "@/lib/bindings";

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
  | "prev-tab"
  | "dashboard-refresh"
  | "dashboard-save"
  | "refresh-table"
  | "focus-filter"
  | "apply-changes"
  | "find-in-grid"
  | "find-in-query"
  | "inline-ai"
  | "save-query"
  | "open-sql-file"
  | "toggle-sidebar"
  | "toggle-inspector"
  | "erd"
  | "settings"
  | "goto-prev-tab"
  | "goto-next-tab"
  | "zoom-in"
  | "zoom-out"
  | "zoom-reset"
  | "new-window"
  | "close-window";

export type ShortcutGroup = "General" | "Editor" | "Tabs" | "Data" | "View" | "Window";

/// Display order of the groups on the Settings page.
export const SHORTCUT_GROUPS: readonly ShortcutGroup[] = ["General", "Editor", "Tabs", "Data", "View", "Window"];

/// Where a chord is live. Two actions may share a chord when their scopes
/// never overlap (⌘↵ runs SQL in the editor and applies changes in the grid).
export type ShortcutScope = "global" | "editor" | "grid" | "document";

export interface ShortcutMeta {
  label: string;
  group: ShortcutGroup;
  keys: string;
  /// Omitted means "global".
  scope?: ShortcutScope;
}

export const SHORTCUTS = {
  palette: { label: "Command palette", group: "General", keys: "Mod+K" },
  "commit-changes": { label: "Commit pending changes", group: "Data", keys: "Mod+S", scope: "grid" },
  run: { label: "Run statement / selection", group: "Editor", keys: "Mod+Enter", scope: "editor" },
  "run-all": { label: "Run whole script", group: "Editor", keys: "Mod+Shift+Enter", scope: "editor" },
  format: { label: "Format SQL", group: "Editor", keys: "Alt+Shift+F", scope: "editor" },
  "new-query": { label: "New query tab", group: "Tabs", keys: "Mod+N" },
  "close-tab": { label: "Close tab", group: "Tabs", keys: "Mod+W" },
  "reopen-tab": { label: "Reopen closed tab", group: "Tabs", keys: "Mod+Shift+T" },
  "next-tab": { label: "Next tab", group: "Tabs", keys: "Ctrl+Tab" },
  "prev-tab": { label: "Previous tab", group: "Tabs", keys: "Ctrl+Shift+Tab" },
  "dashboard-refresh": { label: "Refresh dashboard", group: "Data", keys: "R", scope: "document" },
  "dashboard-save": { label: "Save dashboard", group: "Data", keys: "Mod+S", scope: "document" },
  "refresh-table": { label: "Refresh table", group: "Data", keys: "Mod+R", scope: "grid" },
  "focus-filter": { label: "Filter rows", group: "Data", keys: "Mod+Shift+F", scope: "grid" },
  "apply-changes": { label: "Apply pending changes", group: "Data", keys: "Mod+Enter", scope: "grid" },
  "find-in-grid": { label: "Find in grid", group: "Data", keys: "Mod+F", scope: "grid" },
  "find-in-query": { label: "Find and replace in query", group: "Editor", keys: "Mod+F", scope: "editor" },
  "inline-ai": { label: "Ask AI at the cursor", group: "Editor", keys: "Mod+I", scope: "editor" },
  "save-query": { label: "Save query", group: "Editor", keys: "Mod+S", scope: "editor" },
  "open-sql-file": { label: "Open SQL file", group: "Editor", keys: "Mod+O", scope: "editor" },
  "toggle-sidebar": { label: "Toggle sidebar", group: "View", keys: "Mod+J" },
  "toggle-inspector": { label: "Toggle record inspector", group: "View", keys: "Mod+L" },
  erd: { label: "Open ER diagram", group: "General", keys: "Mod+Shift+D" },
  settings: { label: "Open settings", group: "General", keys: "Mod+," },
  "goto-prev-tab": { label: "Previous tab (alternate)", group: "Tabs", keys: "Mod+Shift+[" },
  "goto-next-tab": { label: "Next tab (alternate)", group: "Tabs", keys: "Mod+Shift+]" },
  "zoom-in": { label: "Zoom in", group: "View", keys: "Mod+=" },
  "zoom-out": { label: "Zoom out", group: "View", keys: "Mod+-" },
  "zoom-reset": { label: "Actual size", group: "View", keys: "Mod+0" },
  "new-window": { label: "New window", group: "Window", keys: "Mod+Shift+N" },
  "close-window": { label: "Close window", group: "Window", keys: "Mod+Shift+W" },
} satisfies Record<ShortcutAction, ShortcutMeta>;

export const SHORTCUT_ACTIONS: readonly ShortcutAction[] = [
  "palette",
  "commit-changes",
  "run",
  "run-all",
  "format",
  "new-query",
  "close-tab",
  "reopen-tab",
  "next-tab",
  "prev-tab",
  "dashboard-refresh",
  "dashboard-save",
  "refresh-table",
  "focus-filter",
  "apply-changes",
  "find-in-grid",
  "find-in-query",
  "inline-ai",
  "save-query",
  "open-sql-file",
  "toggle-sidebar",
  "toggle-inspector",
  "erd",
  "settings",
  "goto-prev-tab",
  "goto-next-tab",
  "zoom-in",
  "zoom-out",
  "zoom-reset",
  "new-window",
  "close-window",
];

export function isShortcutAction(value: string): value is ShortcutAction {
  return value in SHORTCUTS;
}

export type Keymap = Record<ShortcutAction, string>;

// WHAT:  Registry defaults with the user's rebinds (AppSettings.keybindings) on top.
// HOW:   An override naming no registered action is skipped, so a binding saved for
//        an action a later build removed is harmless. An empty `keys` unbinds.
export function resolveKeymap(overrides: readonly KeyBinding[] | undefined): Keymap {
  // Built from the registry itself, so an action added to SHORTCUTS needs no
  // second list here: nothing can be registered and yet missing from the map.
  const map: Record<string, string> = {};
  for (const [action, meta] of Object.entries(SHORTCUTS)) map[action] = meta.keys;
  for (const o of overrides ?? []) if (isShortcutAction(o.action)) map[o.action] = normalizeChord(o.keys);
  if (!isKeymap(map)) throw new Error("keymap is missing a registered shortcut action");
  return map;
}

/// True when every registered action has a chord (it always does: the map is built from the registry).
function isKeymap(map: Readonly<Record<string, string>>): map is Keymap {
  return Object.keys(SHORTCUTS).every((action) => action in map);
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

/// Physical keys that are not letters or digits, named by what an ANSI board
/// prints on them, so a chord means the same key on every layout (⌘⇧[ is the
/// key right of P whatever it is labelled). The numpad keys share their row's name.
const CODE_KEYS: Readonly<Record<string, string>> = {
  Equal: "=",
  Minus: "-",
  BracketLeft: "[",
  BracketRight: "]",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Backquote: "`",
  NumpadAdd: "=",
  NumpadSubtract: "-",
  NumpadEnter: "enter",
};

function eventKey(event: KeyboardEvent): string {
  if (event.key === " ") return "space";
  // With Alt held, macOS reports the composed character (Alt+F is "ƒ"); the
  // physical key is what the chord names.
  if (event.code.startsWith("Key")) return event.code.slice(3).toLowerCase();
  if (event.code.startsWith("Digit")) return event.code.slice(5);
  if (/^Numpad\d$/.test(event.code)) return event.code.slice(6);
  const physical = CODE_KEYS[event.code];
  if (physical !== undefined) return physical;
  return event.key.toLowerCase();
}

export function matchesChord(event: KeyboardEvent, chord: string): boolean {
  const c = parseChord(chord);
  if (!c) return false;
  const wantCtrl = c.ctrl || (c.mod && !IS_MAC);
  const wantMeta = c.meta || (c.mod && IS_MAC);
  if (event.ctrlKey !== wantCtrl || event.metaKey !== wantMeta || event.altKey !== c.alt) return false;
  // "Zoom in" is the + key: it needs Shift on most layouts and none on others,
  // and ⌘= and ⌘+ both mean it. Whatever the layout types, `key` says "+" or "=".
  if (c.key === "=") return eventKey(event) === "=" || event.key === "+" || event.key === "=";
  return event.shiftKey === c.shift && eventKey(event) === c.key;
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

/// The keycaps of a chord for the host platform: ["⌘", "⇧", "T"] on macOS,
/// ["Ctrl", "Shift", "T"] elsewhere. Empty for an unbound action.
export function chordParts(chord: string): string[] {
  const c = parseChord(chord);
  if (!c) return [];
  const typed = c.key.length === 1 ? c.key.toUpperCase() : c.key.charAt(0).toUpperCase() + c.key.slice(1);
  // The plus key is bound as "=" (same physical key); show what is printed on it.
  const key = typed === "=" ? "+" : typed;
  if (IS_MAC) {
    return [c.ctrl ? "⌃" : "", c.alt ? "⌥" : "", c.shift ? "⇧" : "", c.mod || c.meta ? "⌘" : "", key === "Enter" ? "↩" : key].filter((p) => p.length > 0);
  }
  return [c.mod || c.ctrl ? "Ctrl" : "", c.meta ? "Win" : "", c.alt ? "Alt" : "", c.shift ? "Shift" : "", key].filter((p) => p.length > 0);
}

/// "⌘ ⇧ T" on macOS, "Ctrl + Shift + T" elsewhere.
export function displayChord(chord: string): string {
  const parts = chordParts(chord);
  if (parts.length === 0) return "—";
  return parts.join(IS_MAC ? " " : " + ");
}

/// Whether two scopes are ever live at the same time. Global overlaps everything;
/// the editor and the grid are never focused together, so they may share a chord.
export function scopesOverlap(a: ShortcutScope | undefined, b: ShortcutScope | undefined): boolean {
  const left = a ?? "global";
  const right = b ?? "global";
  return left === "global" || right === "global" || left === right;
}

function scopeOf(action: ShortcutAction): ShortcutScope | undefined {
  const meta: ShortcutMeta = SHORTCUTS[action];
  return meta.scope;
}

// WHAT:  For each action that shares its chord, the other actions on it.
// WHY:   Two actions on one chord means only one of them ever fires (whichever
//        listener runs first); the Settings page warns instead of guessing.
// HOW:   Only actions whose scopes can be live together collide: ⌘↵ may run SQL
//        in the editor and apply changes in the grid, because the two are never
//        focused at once. A global action collides with every scope.
export function conflictsOf(keymap: Keymap): Map<ShortcutAction, ShortcutAction[]> {
  const byChord = new Map<string, ShortcutAction[]>();
  for (const action of SHORTCUT_ACTIONS) {
    const chord = keymap[action];
    if (chord.length === 0) continue;
    byChord.set(chord, [...(byChord.get(chord) ?? []), action]);
  }
  const out = new Map<ShortcutAction, ShortcutAction[]>();
  for (const actions of byChord.values()) {
    if (actions.length < 2) continue;
    for (const action of actions) {
      const clashes = actions.filter((other) => other !== action && scopesOverlap(scopeOf(action), scopeOf(other)));
      if (clashes.length > 0) out.set(action, clashes);
    }
  }
  return out;
}

/// How a scope reads in the Shortcuts list; global actions carry no tag.
export function scopeLabel(scope: ShortcutScope | undefined): string | null {
  if (scope === "editor") return "SQL editor";
  if (scope === "grid") return "Data grid";
  if (scope === "document") return "Dashboards and diagrams";
  return null;
}

// WHAT:  The override list after binding `action` to `keys` (null: back to the
//        default). A chord equal to the default drops the override, so the
//        stored list only ever holds real rebinds.
export function withBinding(overrides: readonly KeyBinding[], action: ShortcutAction, keys: string | null): KeyBinding[] {
  const rest = overrides.filter((o) => o.action !== action);
  if (keys === null || normalizeChord(keys) === normalizeChord(SHORTCUTS[action].keys)) return rest;
  return [...rest, { action, keys: normalizeChord(keys) }];
}
