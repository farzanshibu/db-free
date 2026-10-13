// SOT: theme-file, theme-import, theme-export, theme-color-validation, dbpro-theme-import
import type { ThemeBase } from "./bindings";
import type { JsonValue } from "./bindings/serde_json/JsonValue";
import { parseJson } from "./json";

// WHAT:  The portable form of a custom theme (a small JSON file), and the
//        strict reading of one — including a DB Pro theme file.
// WHY:   A theme file comes from outside the app: a colleague, a download. It
//        ends up as CSS variable values on the document, so every value is
//        checked here before it can reach `style.setProperty` — no `url()`, no
//        `var()`, no script-looking characters, and nothing over 64 KB.
// HOW:   `parseThemeFile` returns a result instead of throwing, so the dialog
//        can show the reason. Native files (`dbfree-theme`) name this app's
//        tokens directly; DB Pro files (`dbpro-theme`) name DB Pro's, which are
//        translated through DBPRO_TOKENS (values are HSL triples there, colour
//        strings here). Tokens with no counterpart are counted, not guessed.
// WHERE: src/lib/themes.ts (the token catalogue), src/features/settings/ThemesSection.tsx

export const NATIVE_THEME_TYPE = "dbfree-theme";
export const DBPRO_THEME_TYPE = "dbpro-theme";
export const THEME_FILE_VERSION = 1;
/// Largest theme file accepted, in bytes.
export const MAX_THEME_FILE_BYTES = 64 * 1024;
const MAX_NAME_LENGTH = 80;
const MAX_VALUE_LENGTH = 120;

export interface ThemeDraft {
  name: string;
  base: ThemeBase;
  /// Token → colour; a token with no value (absent, or undefined as settings types it) is left out of a file.
  colors: Readonly<Record<string, string | undefined>>;
}

export type ThemeParseResult =
  | { ok: true; theme: ThemeDraft; source: "dbfree" | "dbpro"; skipped: number }
  | { ok: false; error: string };

const HEX = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
// Colour functions only; the argument list is numbers, units and keywords such as `none`,
// never a nested call, so `var(...)` and `url(...)` cannot be smuggled inside.
const COLOR_FUNCTION = /^(?:rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch)\(\s*[-+0-9a-z.%\s,/]*\)$/i;
const FORBIDDEN = /[;{}<>\\\u0000-\u001f]|url\s*\(|var\s*\(|expression|@import/i;

/// A colour value safe to put in a CSS variable: hex, `transparent`, or a colour function.
export function isSafeColor(value: string): boolean {
  const text = value.trim();
  if (text.length === 0 || text.length > MAX_VALUE_LENGTH || FORBIDDEN.test(text)) return false;
  return text.toLowerCase() === "transparent" || HEX.test(text) || COLOR_FUNCTION.test(text);
}

export function isThemeBase(value: string): value is ThemeBase {
  return value === "light" || value === "dark";
}

/// The file contents for a theme (pretty-printed so it diffs well in a repo).
export function serializeTheme(theme: ThemeDraft): string {
  const sorted = Object.fromEntries(Object.entries(theme.colors).filter((entry): entry is [string, string] => entry[1] !== undefined).sort(([a], [b]) => a.localeCompare(b)));
  return JSON.stringify({ type: NATIVE_THEME_TYPE, version: THEME_FILE_VERSION, theme: { name: theme.name, base: theme.base, colors: sorted } }, null, 2);
}

/// `My theme` → `My theme.json`, with the characters a filename cannot hold removed.
export function themeFileName(name: string): string {
  const safe = name.replace(/[\\/:*?"<>|]+/g, "").trim();
  return `${safe.length > 0 ? safe : "theme"}.json`;
}

type JsonObject = Record<string, JsonValue>;

function asObject(value: JsonValue | undefined): JsonObject | null {
  return value !== undefined && value !== null && typeof value === "object" && !Array.isArray(value) ? value : null;
}

// WHAT:  Reads a theme file. `known` is the set of token names this app accepts.
export function parseThemeFile(text: string, known: ReadonlySet<string>): ThemeParseResult {
  const trimmed = text.trim();
  if (trimmed.length === 0) return { ok: false, error: "The theme file is empty." };
  if (new TextEncoder().encode(trimmed).length > MAX_THEME_FILE_BYTES) return { ok: false, error: "The theme file is larger than 64 KB." };
  if (!trimmed.startsWith("{")) return { ok: false, error: "The theme file is not valid JSON." };
  const root = asObject(parseJson(trimmed));
  if (root === null) return { ok: false, error: "The theme file is not valid JSON." };

  const type = root.type;
  if (type !== NATIVE_THEME_TYPE && type !== DBPRO_THEME_TYPE) return { ok: false, error: "This is not a DB Free or DB Pro theme file." };
  if (root.version !== THEME_FILE_VERSION) return { ok: false, error: "This theme file uses an unsupported version." };
  const theme = asObject(root.theme);
  if (theme === null) return { ok: false, error: "The theme file has no theme data." };

  const rawName = theme.name;
  const name = typeof rawName === "string" ? rawName.trim() : "";
  if (name.length === 0 || name.length > MAX_NAME_LENGTH) return { ok: false, error: "The theme name is missing or longer than 80 characters." };
  const base = theme.base;
  if (typeof base !== "string" || !isThemeBase(base)) return { ok: false, error: "The theme base must be \"light\" or \"dark\"." };
  const rawColors = asObject(theme.colors);
  if (rawColors === null) return { ok: false, error: "The theme has no colors." };

  const colors: Record<string, string> = {};
  let skipped = 0;
  const source = type === DBPRO_THEME_TYPE ? "dbpro" : "dbfree";
  for (const [token, value] of Object.entries(rawColors)) {
    if (typeof value !== "string") return { ok: false, error: `The value for ${token} is not text.` };
    if (source === "dbfree") {
      if (!known.has(token)) {
        skipped += 1;
        continue;
      }
      if (!isSafeColor(value)) return { ok: false, error: `The value for ${token} is not a valid colour.` };
      colors[token] = value.trim();
      continue;
    }
    const mapped = translateDbProToken(token, value);
    if (mapped === "invalid") return { ok: false, error: `The value for ${token} is not a valid colour.` };
    if (mapped === null) {
      skipped += 1;
      continue;
    }
    for (const [target, converted] of mapped) {
      if (known.has(target)) colors[target] = converted;
    }
  }
  if (Object.keys(colors).length === 0) return { ok: false, error: "The theme has no colours this app can use." };
  if (source === "dbpro") fillDerivedTokens(colors, known);
  return { ok: true, theme: { name, base, colors }, source, skipped };
}

// ---------------------------------------------------------------- DB Pro files

/// How one DB Pro token lands here: the target token(s) and an alpha to apply to the colour.
interface Translation {
  to: readonly { token: string; alpha?: number }[];
}

// DB Pro's palette names (HSL triples) → this app's tokens. Only tokens with a
// real counterpart are listed; the rest (sidebar-ring, pending-*, ...) are skipped.
const DBPRO_TOKENS: Readonly<Record<string, Translation>> = {
  "--background": { to: [{ token: "--background" }] },
  "--foreground": { to: [{ token: "--foreground" }] },
  "--card": { to: [{ token: "--surface" }] },
  "--surface": { to: [{ token: "--surface-secondary" }] },
  "--surface-hover": { to: [{ token: "--surface-tertiary" }] },
  "--popover-surface": { to: [{ token: "--overlay" }] },
  "--muted-foreground": { to: [{ token: "--muted" }] },
  "--button-primary": { to: [{ token: "--accent" }] },
  "--button-primary-foreground": { to: [{ token: "--accent-foreground" }] },
  "--destructive": { to: [{ token: "--danger" }] },
  "--border": { to: [{ token: "--border" }] },
  "--border-subtle": { to: [{ token: "--separator" }] },
  "--input": { to: [{ token: "--field-border" }] },
  "--background-inset": { to: [{ token: "--field" }] },
  "--sidebar": { to: [{ token: "--glass-sidebar", alpha: 0.85 }, { token: "--glass-dock", alpha: 0.88 }] },
  "--row-selected-border": { to: [{ token: "--color-selection", alpha: 0.45 }, { token: "--color-selection-strong", alpha: 0.3 }] },
  "--chart-1": { to: [{ token: "--color-series-1" }] },
  "--chart-2": { to: [{ token: "--color-series-2" }] },
  "--chart-3": { to: [{ token: "--color-series-3" }] },
  "--chart-4": { to: [{ token: "--color-series-4" }] },
  "--chart-5": { to: [{ token: "--color-series-5" }] },
  "--scrollbar-thumb": { to: [{ token: "--scrollbar-thumb" }] },
  "--scrollbar-thumb-hover": { to: [{ token: "--scrollbar-thumb-hover" }] },
  "--scrollbar-track": { to: [{ token: "--scrollbar-track" }] },
  "--editor-keyword": { to: [{ token: "--color-syntax-keyword" }] },
  "--editor-string": { to: [{ token: "--color-syntax-string" }] },
  "--editor-number": { to: [{ token: "--color-syntax-number" }] },
  "--editor-comment": { to: [{ token: "--color-syntax-comment" }] },
  "--editor-operator": { to: [{ token: "--color-syntax-operator" }] },
  "--editor-identifier": { to: [{ token: "--color-syntax-type" }] },
  "--status-success": { to: [{ token: "--success" }] },
  "--status-warning": { to: [{ token: "--warning" }] },
  "--status-error": { to: [{ token: "--danger" }] },
};

const HSL_TRIPLE = /^(-?\d+(?:\.\d+)?)[ ,]+(\d+(?:\.\d+)?)%[ ,]+(\d+(?:\.\d+)?)%$/;

/// One DB Pro value as a CSS colour, or null when it is neither a triple nor a safe colour.
export function dbProColor(value: string, alpha?: number): string | null {
  const text = value.trim();
  const triple = HSL_TRIPLE.exec(text);
  if (triple !== null) {
    const [, h, s, l] = triple;
    return alpha === undefined ? `hsl(${h} ${s}% ${l}%)` : `hsl(${h} ${s}% ${l}% / ${alpha})`;
  }
  if (!isSafeColor(text)) return null;
  return text;
}

function translateDbProToken(token: string, value: string): readonly (readonly [string, string])[] | null | "invalid" {
  const translation = DBPRO_TOKENS[token];
  if (translation === undefined) return null;
  const out: [string, string][] = [];
  for (const target of translation.to) {
    const color = dbProColor(value, target.alpha);
    if (color === null) return "invalid";
    out.push([target.token, color]);
  }
  return out;
}

// WHAT:  Fills the window-ground and glass tokens an imported DB Pro theme
//        has no equivalent for, from the colours it did bring.
// WHY:   Without them the page gradient and the translucent panels keep the
//        base theme's tint, which clashes with a different background.
function fillDerivedTokens(colors: Record<string, string>, known: ReadonlySet<string>): void {
  const set = (token: string, value: string | undefined) => {
    if (value !== undefined && known.has(token) && !(token in colors)) colors[token] = value;
  };
  const background = colors["--background"];
  set("--page-base", background);
  set("--page-top", background);
  set("--page-mid", background);
  set("--page-bottom", background);
  set("--page-glow", "transparent");
  set("--page-corner", "transparent");
  set("--glass-header", withAlpha(background, 0.85));
  set("--glass-card", withAlpha(colors["--surface"], 0.7));
  set("--glass-card-hover", withAlpha(colors["--surface-secondary"], 0.75));
  set("--glass-pill", withAlpha(colors["--surface-secondary"], 0.6));
  set("--glass-input", withAlpha(colors["--field"], 0.65));
  set("--surface-elevated", colors["--surface"]);
}

/// `hsl(H S% L%)` with an alpha; any other colour form is returned untouched.
function withAlpha(color: string | undefined, alpha: number): string | undefined {
  if (color === undefined) return undefined;
  const match = /^hsl\((.+)\)$/i.exec(color);
  if (match === null || color.includes("/")) return color;
  return `hsl(${match[1] ?? ""} / ${alpha})`;
}
