// SOT: connection-tag-palette, tag-colour-tokens, tag-chip-style, tag-assignment-helpers
import type { CSSProperties } from "react";
import type { ConnectionTag, ConnectionTagColor } from "@/lib/bindings";
import { keysOf } from "@/lib/records";

// WHAT:  The 19 colours a connection tag can wear, by name.
// WHY:   Rust owns the palette (`ConnectionTagColor`); this registry fails
//        `satisfies` until every key has a label, so a colour added there cannot
//        reach the picker unnamed. The colours themselves are the `--tag-<key>`
//        design tokens in globals.css, redefined for the light theme.
// WHERE: src-tauri/src/model/connection_org.rs, src/styles/globals.css
export const TAG_COLORS = {
  red: { label: "Red" },
  rose: { label: "Rose" },
  pink: { label: "Pink" },
  orange: { label: "Orange" },
  amber: { label: "Amber" },
  yellow: { label: "Yellow" },
  lime: { label: "Lime" },
  green: { label: "Green" },
  emerald: { label: "Emerald" },
  teal: { label: "Teal" },
  cyan: { label: "Cyan" },
  sky: { label: "Sky" },
  blue: { label: "Blue" },
  indigo: { label: "Indigo" },
  violet: { label: "Violet" },
  purple: { label: "Purple" },
  fuchsia: { label: "Fuchsia" },
  slate: { label: "Slate" },
  gray: { label: "Gray" },
} satisfies Record<ConnectionTagColor, { label: string }>;

export const TAG_COLOR_ORDER: ConnectionTagColor[] = keysOf(TAG_COLORS);

export function tagColorLabel(color: ConnectionTagColor): string {
  return TAG_COLORS[color].label;
}

// WHAT:  A solid swatch: the colour token as a background.
export function tagSolidStyle(color: ConnectionTagColor): CSSProperties {
  return { backgroundColor: `var(--tag-${color})` };
}

// WHAT:  A tag pill: tinted fill, tinted border, token-coloured text.
// WHY:   `color-mix` keeps one token per colour while the pill follows the theme:
//        the light theme's darker token keeps the text legible on white.
export function tagChipStyle(color: ConnectionTagColor): CSSProperties {
  return {
    color: `var(--tag-${color})`,
    backgroundColor: `color-mix(in oklch, var(--tag-${color}) 16%, transparent)`,
    borderColor: `color-mix(in oklch, var(--tag-${color}) 42%, transparent)`,
  };
}

// WHAT:  The tags a connection carries, in the order Rust returned them.
export function tagsOf(tagIds: readonly string[], tags: readonly ConnectionTag[]): ConnectionTag[] {
  return tagIds.flatMap((id) => {
    const tag = tags.find((t) => t.id === id);
    return tag ? [tag] : [];
  });
}

// WHAT:  The colour a new tag starts with: the first palette colour no tag uses yet.
export function nextFreeColor(tags: readonly ConnectionTag[]): ConnectionTagColor {
  const used = new Set(tags.map((t) => t.color));
  return TAG_COLOR_ORDER.find((c) => !used.has(c)) ?? "blue";
}

// WHAT:  Whether `name` would clash with another tag (case-insensitive), ignoring `exceptId`.
// WHY:   Rust refuses a duplicate too; checking here shows the message as the
//        user types instead of after the round trip.
export function tagNameTaken(name: string, tags: readonly ConnectionTag[], exceptId: string | null): boolean {
  const wanted = name.trim().toLowerCase();
  return wanted.length > 0 && tags.some((t) => t.id !== exceptId && t.name.toLowerCase() === wanted);
}

export function toggled(ids: readonly string[], id: string): string[] {
  return ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id];
}
