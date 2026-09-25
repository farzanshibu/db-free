// SOT: theme-choice, resolved-theme, system-theme, theme-hook
import { useSyncExternalStore } from "react";
import { useWorkspace } from "@/stores/workspace";

// WHAT:  The colour theme the user picked (AppSettings.theme) and the one that
//        is actually drawn, with "system" following the OS live.
// WHY:   App.tsx writes `data-theme` on <html> from this, and the few surfaces
//        that cannot read CSS tokens (CodeMirror's dark flag, React Flow's
//        colorMode) ask the same hook, so they can never disagree.
// HOW:   `prefers-color-scheme` is read through useSyncExternalStore, so an OS
//        switch re-renders every consumer without an effect.
// WHERE: src/styles/globals.css (`:root[data-theme="light"]`), src/App.tsx
export type ThemeChoice = "dark" | "light" | "system";
export type ResolvedTheme = "dark" | "light";

export const THEME_OPTIONS = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
  { value: "system", label: "System" },
] satisfies readonly { value: ThemeChoice; label: string }[];

export function isThemeChoice(value: string): value is ThemeChoice {
  return THEME_OPTIONS.some((o) => o.value === value);
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

export function resolveTheme(choice: string | undefined, system: ResolvedTheme): ResolvedTheme {
  if (choice === "light") return "light";
  if (choice === "system") return system;
  return "dark";
}

export function useResolvedTheme(): ResolvedTheme {
  const choice = useWorkspace((s) => s.settings?.theme);
  const system = useSyncExternalStore(subscribe, systemTheme, darkTheme);
  return resolveTheme(choice, system);
}
