// SOT: diagram-palette, diagram-colour-tokens, table-header-colours, line-widths
import type { ConnectionColor, DiagramLineWidth } from "@/lib/bindings";
import { CONNECTION_COLOR_ORDER, CONNECTION_COLORS } from "@/lib/connectionGroups";

// WHAT:  The colours a table header, a sticky note or a relation line can take,
//        and how each is drawn.
// WHY:   Diagrams store a palette key (`ConnectionColor`), never a hex value, so a
//        diagram follows the theme and the guardrail's "colours only through
//        tokens" holds. The keys are the connection colour tags, which already
//        have tokens in both themes (`--color-conn-*`).
// HOW:   `colorVar` is the fill; `inkVar` is the text colour that stays readable
//        on it (the light fills want dark ink, the saturated ones light ink).
// WHERE: src/styles/globals.css (--color-conn-*, --diagram-ink-*), src/lib/connectionGroups.ts

export const DIAGRAM_COLORS: readonly ConnectionColor[] = CONNECTION_COLOR_ORDER;

export const DEFAULT_NOTE_COLOR: ConnectionColor = "amber";
/// A relation without a colour of its own is drawn in the accent.
export const DEFAULT_LINE_COLOR = "var(--accent)";

export function colorVar(color: ConnectionColor): string {
  return `var(--color-conn-${color})`;
}

const INK = {
  blue: "light",
  teal: "dark",
  green: "dark",
  amber: "dark",
  orange: "dark",
  red: "light",
  pink: "dark",
  violet: "light",
  gray: "light",
} satisfies Record<ConnectionColor, "dark" | "light">;

export function inkVar(color: ConnectionColor): string {
  return INK[color] === "dark" ? "var(--diagram-ink-dark)" : "var(--diagram-ink-light)";
}

export function colorLabel(color: ConnectionColor): string {
  return CONNECTION_COLORS[color].label;
}

// WHAT:  Relation line widths in px. Four sizes, as in the inspector's picker.
export const LINE_WIDTHS = {
  thin: { label: "Thin", px: 1 },
  normal: { label: "Normal", px: 1.6 },
  thick: { label: "Thick", px: 2.6 },
  heavy: { label: "Heavy", px: 4 },
} satisfies Record<DiagramLineWidth, { label: string; px: number }>;

export const LINE_WIDTH_ORDER: readonly DiagramLineWidth[] = ["thin", "normal", "thick", "heavy"];

export function lineWidthPx(width: DiagramLineWidth | null): number {
  return LINE_WIDTHS[width ?? "normal"].px;
}

/// Text sizes a note offers (px).
export const NOTE_FONT_SIZES = [
  { value: 10, label: "Small" },
  { value: 12, label: "Normal" },
  { value: 14, label: "Medium" },
  { value: 18, label: "Large" },
] as const;

export const DEFAULT_NOTE_FONT_SIZE = 12;
