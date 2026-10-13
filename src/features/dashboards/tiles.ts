// SOT: tile-colour-registry, tile-swatches, tile-gradients, tile-tint
// WHAT:  The colours a dashboard tile can wear: 14 swatches, each a solid hue
//        (a chart's line, a tile's top edge) and a gradient background, plus the
//        seven categorical series colours older dashboards stored as `series-N`.
// WHY:   A swatch is stored by name, never as a colour, so the dark and light
//        themes each draw their own tuned version of it. The palette lives in
//        globals.css (`--tile-<name>`, `--tile-gradient-<name>`); this registry
//        names what the picker offers and turns a stored name into a CSS value.
// HOW:   Names resolve to `var(--tile-…)` strings used in inline styles. A name
//        the registry does not know (a hand-edited dashboard) falls back to the
//        first series colour rather than rendering transparent.
// WHERE: src/styles/globals.css (tokens, both themes), ./TileColorPicker.tsx, ./charts.tsx

export const TILE_SWATCHES = [
  { name: "slate", label: "Slate" },
  { name: "red", label: "Red" },
  { name: "orange", label: "Orange" },
  { name: "amber", label: "Amber" },
  { name: "green", label: "Green" },
  { name: "emerald", label: "Emerald" },
  { name: "teal", label: "Teal" },
  { name: "cyan", label: "Cyan" },
  { name: "blue", label: "Blue" },
  { name: "indigo", label: "Indigo" },
  { name: "violet", label: "Violet" },
  { name: "purple", label: "Purple" },
  { name: "pink", label: "Pink" },
  { name: "rose", label: "Rose" },
] as const satisfies readonly { name: string; label: string }[];

export type SwatchName = (typeof TILE_SWATCHES)[number]["name"];

/// The categorical series colours (--color-series-1..7), the default tint.
export const SERIES_TINT_PREFIX = "series-";

export function isSwatch(name: string | null | undefined): name is SwatchName {
  return name !== null && name !== undefined && TILE_SWATCHES.some((s) => s.name === name);
}

export function swatchLabel(name: string | null | undefined): string {
  return TILE_SWATCHES.find((s) => s.name === name)?.label ?? "Default";
}

/// A legacy `series-1`..`series-7` tint, or null.
function seriesIndex(tint: string): number | null {
  if (!tint.startsWith(SERIES_TINT_PREFIX)) return null;
  const n = Number(tint.slice(SERIES_TINT_PREFIX.length));
  return Number.isInteger(n) && n >= 1 && n <= 7 ? n : null;
}

/// The solid colour of a stored tint (swatch name, legacy series name, or none).
export function tintCss(tint: string | null | undefined): string {
  if (tint === null || tint === undefined) return "var(--color-series-1)";
  if (isSwatch(tint)) return `var(--tile-${tint})`;
  const series = seriesIndex(tint);
  return series === null ? "var(--color-series-1)" : `var(--color-series-${series})`;
}

/// The gradient behind a metric / text / image tile for a swatch name, or
/// undefined for no background.
export function gradientCss(name: string | null | undefined): string | undefined {
  return isSwatch(name) ? `var(--tile-gradient-${name})` : undefined;
}

/// Text drawn on a gradient: white in both themes.
export const ON_GRADIENT = "var(--tile-on-gradient)";

/// The two gradients a metric can wear when it is coloured by its change.
export const CHANGE_UP: SwatchName = "emerald";
export const CHANGE_DOWN: SwatchName = "red";
