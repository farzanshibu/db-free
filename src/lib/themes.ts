// SOT: theme-registry, theme-tokens, theme-groups, theme-apply, theme-snapshot, custom-theme-ids
import type { CustomTheme, ThemeBase } from "./bindings";
import { isSafeColor } from "./themeFile";

// WHAT:  Every theme the app can draw — the seven built in (Dark, Light, Ember,
//        Nocturne, Glacier, Midnight, Parchment), "System", and the user's own —
//        and the catalogue of colour tokens a custom theme may set.
// WHY:   The palettes themselves live in src/styles/globals.css (one block per
//        theme id). This is the other half: which ids exist, which scheme each
//        belongs to (the editor, the diagrams and the OS window chrome need
//        light-or-dark, not a palette), and which CSS variables the theme
//        editor offers, grouped the way DB Pro groups them.
// HOW:   A built-in theme is `data-theme="<id>"` on <html>. A custom theme is
//        `data-theme="<its base>"` plus its colours as inline variables, which
//        is why applying one never needs a stylesheet. Colours go through
//        isSafeColor before they touch the document.
// WHERE: src/App.tsx (applies), src/stores/useTheme.ts (resolves the setting),
//        src/features/settings/ThemesSection.tsx (picks and edits)

export interface BuiltInTheme {
  id: string;
  label: string;
  base: ThemeBase;
}

export const BUILT_IN_THEMES: readonly BuiltInTheme[] = [
  { id: "dark", label: "Dark", base: "dark" },
  { id: "light", label: "Light", base: "light" },
  { id: "ember", label: "Ember", base: "dark" },
  { id: "nocturne", label: "Nocturne", base: "dark" },
  { id: "glacier", label: "Glacier", base: "dark" },
  { id: "midnight", label: "Midnight", base: "dark" },
  { id: "parchment", label: "Parchment", base: "light" },
];

export const SYSTEM_THEME = "system";
export const CUSTOM_THEME_PREFIX = "custom-";

export function isCustomThemeId(id: string): boolean {
  return id.startsWith(CUSTOM_THEME_PREFIX);
}

export function builtInTheme(id: string): BuiltInTheme | undefined {
  return BUILT_IN_THEMES.find((t) => t.id === id);
}

/// Dark and Light take the Accent setting; every other theme brings its own accent.
export function followsAccentSetting(id: string): boolean {
  return id === "dark" || id === "light";
}

export interface ThemeToken {
  /// The CSS variable, `--surface`.
  name: string;
  label: string;
}

export type ThemeGroupId = "core" | "surfaces" | "sidebar" | "buttons" | "grid" | "editor" | "status" | "charts" | "scrollbars";

export interface ThemeGroup {
  id: ThemeGroupId;
  label: string;
  description: string;
  tokens: readonly ThemeToken[];
}

const token = (name: string, label: string): ThemeToken => ({ name, label });

export const THEME_GROUPS: readonly ThemeGroup[] = [
  {
    id: "core",
    label: "Core",
    description: "The base palette. Changing these moves the whole app.",
    tokens: [
      token("--background", "Background"),
      token("--foreground", "Text"),
      token("--muted", "Secondary text"),
      token("--border", "Border"),
      token("--separator", "Separator"),
      token("--focus", "Focus ring"),
      token("--link", "Link"),
    ],
  },
  {
    id: "surfaces",
    label: "Surfaces",
    description: "Panels, menus, fields and the window ground behind everything.",
    tokens: [
      token("--surface", "Surface"),
      token("--surface-secondary", "Surface, raised"),
      token("--surface-tertiary", "Surface, highest"),
      token("--surface-hover", "Hover wash"),
      token("--overlay", "Menus and dialogs"),
      token("--surface-elevated", "Tooltips and popups"),
      token("--backdrop", "Dialog backdrop"),
      token("--field", "Input background"),
      token("--field-border", "Input border"),
      token("--field-placeholder", "Input placeholder"),
      token("--default", "Secondary button"),
      token("--segment", "Segmented control"),
      token("--page-base", "Window ground"),
      token("--page-top", "Window gradient, top"),
      token("--page-mid", "Window gradient, middle"),
      token("--page-bottom", "Window gradient, bottom"),
      token("--page-glow", "Window glow"),
      token("--page-corner", "Window corner glow"),
    ],
  },
  {
    id: "sidebar",
    label: "Sidebar",
    description: "The icon rail, the sidebar, the header and tab bar, and glass cards.",
    tokens: [
      token("--glass-dock", "Icon rail"),
      token("--glass-sidebar", "Sidebar"),
      token("--glass-header", "Header and tab bar"),
      token("--glass-card", "Card"),
      token("--glass-card-hover", "Card, hovered"),
      token("--glass-pill", "Pill"),
      token("--glass-input", "Search field"),
      token("--glass-edge", "Edge"),
      token("--glass-edge-strong", "Edge, strong"),
      token("--glass-edge-hover", "Edge, hovered"),
    ],
  },
  {
    id: "buttons",
    label: "Buttons",
    description: "The primary action colour, used for buttons, selections and links.",
    tokens: [token("--accent", "Primary"), token("--accent-foreground", "Text on primary")],
  },
  {
    id: "grid",
    label: "Data grid",
    description: "Row striping, grid lines and selection.",
    tokens: [
      token("--row-stripe", "Alternating row"),
      token("--color-grid-line", "Grid line"),
      token("--color-selection", "Selection"),
      token("--color-selection-strong", "Selected row"),
    ],
  },
  {
    id: "editor",
    label: "SQL editor",
    description: "Syntax highlighting in the query editor.",
    tokens: [
      token("--color-syntax-keyword", "Keyword"),
      token("--color-syntax-string", "String"),
      token("--color-syntax-number", "Number"),
      token("--color-syntax-comment", "Comment"),
      token("--color-syntax-operator", "Operator"),
      token("--color-syntax-type", "Type"),
    ],
  },
  {
    id: "status",
    label: "Status",
    description: "Success, warning and error colours, and the environment tags.",
    tokens: [
      token("--success", "Success"),
      token("--success-foreground", "Text on success"),
      token("--warning", "Warning"),
      token("--warning-foreground", "Text on warning"),
      token("--danger", "Error"),
      token("--danger-foreground", "Text on error"),
      token("--color-env-local", "Local environment"),
      token("--color-env-staging", "Staging environment"),
      token("--color-env-production", "Production environment"),
    ],
  },
  {
    id: "charts",
    label: "Charts",
    description: "The categorical series palette used by dashboards and charts.",
    tokens: [1, 2, 3, 4, 5, 6, 7].map((n) => token(`--color-series-${String(n)}`, `Series ${String(n)}`)),
  },
  {
    id: "scrollbars",
    label: "Scrollbars",
    description: "Scrollbar thumb and track.",
    tokens: [token("--scrollbar-thumb", "Thumb"), token("--scrollbar-thumb-hover", "Thumb, hovered"), token("--scrollbar-track", "Track")],
  },
];

export const ALL_THEME_TOKENS: readonly ThemeToken[] = THEME_GROUPS.flatMap((g) => g.tokens);
export const THEME_TOKEN_NAMES: ReadonlySet<string> = new Set(ALL_THEME_TOKENS.map((t) => t.name));

/// What an applied theme is: the `data-theme` value, its scheme, and the custom colours (if any).
export interface ActiveTheme {
  id: string;
  base: ThemeBase;
  custom: CustomTheme | null;
}

// WHAT:  Turns the `theme` setting into what to draw.
// HOW:   "system" follows the OS; a built-in id is itself; `custom-…` is found in
//        the user's list. Anything else — a theme deleted in another window, a
//        value from a newer build — falls back to Dark rather than to nothing.
export function resolveActiveTheme(choice: string | undefined, system: ThemeBase, custom: readonly CustomTheme[]): ActiveTheme {
  if (choice === undefined) return { id: "dark", base: "dark", custom: null };
  if (choice === SYSTEM_THEME) return { id: system, base: system, custom: null };
  const builtIn = builtInTheme(choice);
  if (builtIn !== undefined) return { id: builtIn.id, base: builtIn.base, custom: null };
  const own = custom.find((t) => t.id === choice);
  if (own !== undefined) return { id: own.base, base: own.base, custom: own };
  return { id: "dark", base: "dark", custom: null };
}

/// Writes a custom theme's colours onto <html>; tokens it does not set are cleared.
/// (Colours arrive from settings, where a token with no value may be absent or undefined.)
export function applyThemeColors(root: HTMLElement, colors: Readonly<Record<string, string | undefined>>): void {
  for (const { name } of ALL_THEME_TOKENS) {
    const value = colors[name];
    if (value !== undefined && isSafeColor(value)) root.style.setProperty(name, value);
    else root.style.removeProperty(name);
  }
}

export function clearThemeColors(root: HTMLElement): void {
  for (const { name } of ALL_THEME_TOKENS) root.style.removeProperty(name);
}

// WHAT:  The value of every catalogued token under a built-in theme.
// WHY:   "New theme from Ember" copies Ember's colours, and the gallery draws
//        each theme's preview from them. The stylesheet is the only source of
//        truth, so they are read back from it rather than restated here.
// HOW:   Inline overrides (an active custom theme) would win over the
//        stylesheet, so they are lifted, the theme attribute is swapped, the
//        values are read, and everything is put back — all in one synchronous
//        turn, so nothing is ever painted in between.
const snapshots = new Map<string, Record<string, string>>();

export function snapshotTheme(id: string): Record<string, string> {
  const cached = snapshots.get(id);
  if (cached !== undefined) return cached;
  const root = document.documentElement;
  const previousAttribute = root.getAttribute("data-theme");
  const lifted = ALL_THEME_TOKENS.map(({ name }) => [name, root.style.getPropertyValue(name)] as const);
  for (const { name } of ALL_THEME_TOKENS) root.style.removeProperty(name);
  root.setAttribute("data-theme", id);
  const computed = getComputedStyle(root);
  const colors: Record<string, string> = {};
  for (const { name } of ALL_THEME_TOKENS) {
    const value = computed.getPropertyValue(name).trim();
    if (value.length > 0) colors[name] = value;
  }
  if (previousAttribute === null) root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", previousAttribute);
  for (const [name, value] of lifted) if (value.length > 0) root.style.setProperty(name, value);
  snapshots.set(id, colors);
  return colors;
}

export function newCustomThemeId(): string {
  const random = Math.random().toString(36).slice(2, 8);
  return `${CUSTOM_THEME_PREFIX}${Date.now().toString(36)}${random}`;
}

/// `name`, or `name 2`, `name 3`… when taken.
export function uniqueThemeName(desired: string, existing: readonly { name: string }[]): string {
  const taken = new Set(existing.map((t) => t.name));
  if (!taken.has(desired)) return desired;
  let n = 2;
  while (taken.has(`${desired} ${String(n)}`)) n += 1;
  return `${desired} ${String(n)}`;
}

// ------------------------------------------------------------- colour helpers

/// sRGB + alpha (0–1) of any CSS colour, via the browser's own parser; null when invalid.
export function cssToRgba(css: string): [number, number, number, number] | null {
  const canvas = document.createElement("canvas");
  canvas.width = 1;
  canvas.height = 1;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (context === null) return null;
  // An invalid assignment is ignored, so two different sentinels tell "invalid" from "same colour".
  context.fillStyle = "#010203";
  context.fillStyle = css;
  const first = context.fillStyle;
  context.fillStyle = "#040506";
  context.fillStyle = css;
  if (first !== context.fillStyle) return null;
  context.clearRect(0, 0, 1, 1);
  context.fillRect(0, 0, 1, 1);
  const [r, g, b, a] = context.getImageData(0, 0, 1, 1).data;
  if (r === undefined || g === undefined || b === undefined || a === undefined) return null;
  // Canvas stores premultiplied-free 8-bit channels; recover alpha as a fraction.
  return [r, g, b, Math.round((a / 255) * 100) / 100];
}

const hex2 = (n: number): string => Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, "0");

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
export function rgbaToHex(r: number, g: number, b: number, a: number): string {
  const base = `#${hex2(r)}${hex2(g)}${hex2(b)}`;
  return a >= 1 ? base : `${base}${hex2(a * 255)}`;
}

/// The `#rrggbb` an `<input type="color">` needs for a CSS colour (alpha dropped), or a neutral grey.
export function cssToPickerHex(css: string): string {
  const rgba = cssToRgba(css);
  return rgba === null ? "#808080" : rgbaToHex(rgba[0], rgba[1], rgba[2], 1);
}

/// A picked `#rrggbb` carrying over the alpha of the colour it replaces.
export function pickerHexWithAlphaOf(picked: string, previous: string): string {
  const rgba = cssToRgba(picked);
  if (rgba === null) return picked;
  const before = cssToRgba(previous);
  return rgbaToHex(rgba[0], rgba[1], rgba[2], before === null ? 1 : before[3]);
}
