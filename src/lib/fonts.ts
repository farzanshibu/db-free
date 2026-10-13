// SOT: font-registry, ui-font, editor-font, font-stacks
/// Font families the app ships. Every face is bundled through @fontsource-variable
/// (imported in src/main.tsx, and the editor additions below), so the picker works
/// offline: nothing is fetched from Google's CDN at runtime. `key` is what
/// AppSettings stores; App.tsx resolves it to `stack` and writes --font-sans /
/// --font-mono. "System" is the platform's own monospace face and ships nothing.
import "@fontsource-variable/inconsolata";
import "@fontsource-variable/victor-mono";
import "@fontsource-variable/martian-mono";
// IBM Plex Mono, Space Mono and Ubuntu Mono have no variable build: regular and bold.
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/700.css";
import "@fontsource/space-mono/400.css";
import "@fontsource/space-mono/700.css";
import "@fontsource/ubuntu-mono/400.css";
import "@fontsource/ubuntu-mono/700.css";

export interface FontFamily {
  key: string;
  label: string;
  stack: string;
  /// Monospace faces are the only ones offered for the editor and grid.
  mono: boolean;
}

const SANS_FALLBACK = 'ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif';
const MONO_FALLBACK = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace";

export const FONTS: readonly FontFamily[] = [
  { key: "jetbrains-mono", label: "JetBrains Mono", stack: `"JetBrains Mono Variable", ${MONO_FALLBACK}`, mono: true },
  { key: "fira-code", label: "Fira Code", stack: `"Fira Code Variable", ${MONO_FALLBACK}`, mono: true },
  { key: "source-code-pro", label: "Source Code Pro", stack: `"Source Code Pro Variable", ${MONO_FALLBACK}`, mono: true },
  { key: "roboto-mono", label: "Roboto Mono", stack: `"Roboto Mono Variable", ${MONO_FALLBACK}`, mono: true },
  { key: "geist-mono", label: "Geist Mono", stack: `"Geist Mono Variable", ${MONO_FALLBACK}`, mono: true },
  { key: "system", label: "System", stack: `ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace`, mono: true },
  { key: "inconsolata", label: "Inconsolata", stack: `"Inconsolata Variable", ${MONO_FALLBACK}`, mono: true },
  { key: "victor-mono", label: "Victor Mono", stack: `"Victor Mono Variable", ${MONO_FALLBACK}`, mono: true },
  { key: "martian-mono", label: "Martian Mono", stack: `"Martian Mono Variable", ${MONO_FALLBACK}`, mono: true },
  { key: "ibm-plex-mono", label: "IBM Plex Mono", stack: `"IBM Plex Mono", ${MONO_FALLBACK}`, mono: true },
  { key: "space-mono", label: "Space Mono", stack: `"Space Mono", ${MONO_FALLBACK}`, mono: true },
  { key: "ubuntu-mono", label: "Ubuntu Mono", stack: `"Ubuntu Mono", ${MONO_FALLBACK}`, mono: true },
  { key: "inter", label: "Inter", stack: `"Inter Variable", ${SANS_FALLBACK}`, mono: false },
  { key: "geist", label: "Geist", stack: `"Geist Variable", ${SANS_FALLBACK}`, mono: false },
  { key: "open-sans", label: "Open Sans", stack: `"Open Sans Variable", ${SANS_FALLBACK}`, mono: false },
  { key: "source-sans-3", label: "Source Sans 3", stack: `"Source Sans 3 Variable", ${SANS_FALLBACK}`, mono: false },
  { key: "space-grotesk", label: "Space Grotesk", stack: `"Space Grotesk Variable", ${SANS_FALLBACK}`, mono: false },
];

/// Every family, for the UI picker: the chrome may use a monospace face too.
export const UI_FONT_OPTIONS = FONTS.map((f) => ({ value: f.key, label: f.label }));
/// Code, grids and values stay monospace, so the editor picker is filtered.
export const EDITOR_FONT_OPTIONS = FONTS.filter((f) => f.mono).map((f) => ({ value: f.key, label: f.label }));

export function fontStack(key: string, fallback: string): string {
  return FONTS.find((f) => f.key === key)?.stack ?? fallback;
}
