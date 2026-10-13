// SOT: theme-choice, resolved-theme, system-theme, theme-hook, active-theme
import { useMemo, useSyncExternalStore } from "react";
import type { ThemeBase } from "@/lib/bindings";
import { BUILT_IN_THEMES, SYSTEM_THEME, builtInTheme, resolveActiveTheme, type ActiveTheme } from "@/lib/themes";
import { useWorkspace } from "@/stores/workspace";

// WHAT:  The colour theme the user picked (AppSettings.theme) and the one that
//        is actually drawn, with "system" following the OS live.
// WHY:   App.tsx writes `data-theme` on <html> from this, and the few surfaces
//        that cannot read CSS tokens (CodeMirror's dark flag, React Flow's
//        colorMode, Sonner) ask the same hook, so they can never disagree.
//        Those surfaces only care whether the ground is dark or light, so
//        `useResolvedTheme` still answers with the scheme, whichever of the
//        seven built-in palettes or the user's own is drawn.
// HOW:   `prefers-color-scheme` is read through useSyncExternalStore, so an OS
//        switch re-renders every consumer without an effect.
// WHERE: src/lib/themes.ts (registry), src/styles/globals.css (palettes), src/App.tsx
export type ThemeChoice = string;
export type ResolvedTheme = ThemeBase;

/// Options for a plain theme picker: System plus every built-in palette.
export const THEME_OPTIONS: readonly { value: ThemeChoice; label: string }[] = [
  ...BUILT_IN_THEMES.map((t) => ({ value: t.id, label: t.label })),
  { value: SYSTEM_THEME, label: "System" },
];

export function isThemeChoice(value: string): boolean {
  return value === SYSTEM_THEME || builtInTheme(value) !== undefined;
}

const QUERY = "(prefers-color-scheme: light)";

/// Nothing to unsubscribe from where matchMedia is missing (tests, SSR).
function noop(): void {
  return undefined;
}

/// The server / first-paint snapshot: black, like index.html.
function darkTheme(): ResolvedTheme {
  return "dark";
}

function subscribe(onChange: () => void): () => void {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return noop;
  const media = window.matchMedia(QUERY);
  media.addEventListener("change", onChange);
  return () => media.removeEventListener("change", onChange);
}

function systemTheme(): ResolvedTheme {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return "dark";
  return window.matchMedia(QUERY).matches ? "light" : "dark";
}

/// The scheme a theme choice resolves to (custom themes need the list, so they read as dark here).
export function resolveTheme(choice: string | undefined, system: ResolvedTheme): ResolvedTheme {
  return resolveActiveTheme(choice, system, []).base;
}

const NO_THEMES: readonly never[] = [];

// WHAT:  The whole drawn theme: which `data-theme` value, which scheme, which
//        custom colours. Only App.tsx and the theme preview need more than the scheme.
export function useActiveTheme(): ActiveTheme {
  const choice = useWorkspace((s) => s.settings?.theme);
  const custom = useWorkspace((s) => s.settings?.customThemes ?? NO_THEMES);
  const system = useSyncExternalStore(subscribe, systemTheme, darkTheme);
  return useMemo(() => resolveActiveTheme(choice, system, custom), [choice, custom, system]);
}

/// The operating system's own scheme, whatever theme is drawn (Settings shows what "System" resolves to).
export function useSystemTheme(): ResolvedTheme {
  return useSyncExternalStore(subscribe, systemTheme, darkTheme);
}

export function useResolvedTheme(): ResolvedTheme {
  return useActiveTheme().base;
}

/// Whether the setting follows the OS (so the native window must too).
export function useFollowsSystem(): boolean {
  return useWorkspace((s) => s.settings?.theme === SYSTEM_THEME);
}
