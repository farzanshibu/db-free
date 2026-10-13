// SOT: app-shell, layout, page-routing, settings-css-vars, theme-attribute, window-effects-mount
import { useEffect } from "react";
import { useActiveConnection, useWorkspace } from "@/stores/workspace";
import { fontStack } from "@/lib/fonts";
import { applyThemeColors, clearThemeColors, followsAccentSetting } from "@/lib/themes";
import { UiPrefsProvider } from "@/lib/uiPrefs";
import { IconRail } from "@/features/shell/IconRail";
import { Sidebar } from "@/features/shell/Sidebar";
import { PaneArea } from "@/features/shell/PaneArea";
import { EnvironmentEdge } from "@/features/shell/EnvironmentEdge";
import { UpdateBar } from "@/features/shell/UpdateBar";
import { AboutDialog } from "@/features/shell/AboutDialog";
import { CloseConfirmDialog } from "@/features/shell/CloseConfirmDialog";
import { DragLayer } from "@/features/shell/DragLayer";
import { WhatsNewPage } from "@/features/shell/WhatsNewPage";
import { useLaunchUpdateCheck, useNativeMenu, useShellShortcuts, useWindowEffects } from "@/features/shell/useShellEffects";
import { ConnectionsPage } from "@/features/connections/ConnectionsPage";
import { ConnectionPicker } from "@/features/connections/ConnectionPicker";
import { ConnectionForm } from "@/features/connections/ConnectionForm";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { CapabilityMatrixPage } from "@/features/engines/CapabilityMatrixPage";
import { PendingChangesPanel } from "@/features/changes/PendingChangesPanel";
import { CommandPalette } from "@/features/palette/CommandPalette";
import { BackupDialog } from "@/features/backup/BackupDialog";
import { ConnectionEvents } from "@/features/connections/ConnectionEvents";
import { Toaster } from "@/components/global/Toaster";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useActiveTheme } from "@/stores/useTheme";

/// Stacks used until the settings load (they match globals.css).
const UI_FONT_FALLBACK = '"JetBrains Mono Variable", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
const EDITOR_FONT_FALLBACK = UI_FONT_FALLBACK;

/// Accent colour plus its oklch hue: the hue also drives every tinted surface
/// (window gradient, glass panels, selection) through the --accent-hue variable.
/// `light` is the same hue darkened for a white ground: the dark-theme accents
/// (L 0.6–0.7) fall under 3:1 as text on white; these hold ≥ 4.5:1, and white
/// text on them stays readable for the filled Run / Save buttons.
const ACCENTS: Record<string, { color: string; light: string; hue: number }> = {
  blue: { color: "oklch(0.6 0.2 258)", light: "oklch(0.52 0.2 258)", hue: 258 },
  violet: { color: "oklch(0.62 0.2 295)", light: "oklch(0.52 0.2 295)", hue: 295 },
  green: { color: "oklch(0.68 0.17 150)", light: "oklch(0.52 0.15 150)", hue: 150 },
  orange: { color: "oklch(0.7 0.17 55)", light: "oklch(0.56 0.16 50)", hue: 55 },
  rose: { color: "oklch(0.64 0.2 10)", light: "oklch(0.53 0.2 10)", hue: 10 },
};

export function App() {
  const bootstrap = useWorkspace((s) => s.bootstrap);
  const ready = useWorkspace((s) => s.ready);
  const page = useWorkspace((s) => s.page);
  const settings = useWorkspace((s) => s.settings);
  const density = useWorkspace((s) => s.density);
  const connection = useActiveConnection();
  const connected = useWorkspace((s) => (connection ? s.sessions.includes(connection.id) : false));
  const changesOpen = useWorkspace((s) => s.changesPanelOpen);
  const theme = useActiveTheme();

  // Keyboard actions, the native menu, zoom, the OS window theme, error reporting,
  // other windows' changes and the update check all live in useShellEffects.ts.
  useShellShortcuts();
  useNativeMenu();
  useWindowEffects();
  useLaunchUpdateCheck();

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  // WHAT:  The drawn theme (Settings → Themes; "system" follows the OS live) as
  //        `data-theme` on <html>, which is all globals.css keys off. A custom
  //        theme is its base scheme plus its colours as inline variables.
  useEffect(() => {
    const root = document.documentElement;
    root.setAttribute("data-theme", theme.id);
    root.classList.toggle("dark", theme.base === "dark");
    if (theme.custom === null) clearThemeColors(root);
    else applyThemeColors(root, theme.custom.colors);
  }, [theme]);

  // Density drives the chrome's spacing tokens (globals.css), not just row height.
  useEffect(() => {
    document.documentElement.setAttribute("data-density", density);
  }, [density]);

  // WHAT:  Suppress the webview's own right-click menu (Cut/Copy/Paste, Speech,
  //        Inspect Element) app-wide.
  // WHY:   It is a browser menu in a desktop app: it offers nothing the app can
  //        honour and hides the app's own context menus behind a second click.
  // HOW:   Capture phase, preventDefault only — propagation continues, so the
  //        React handlers that open the app's menus still run.
  useEffect(() => {
    const suppress = (event: MouseEvent) => event.preventDefault();
    document.addEventListener("contextmenu", suppress, { capture: true });
    return () => document.removeEventListener("contextmenu", suppress, { capture: true });
  }, []);

  // WHAT:  Settings that are pure CSS (accent, font sizes, scrollbar size) apply as root variables.
  // HOW:   The Accent setting recolours Dark, Light and any custom theme that does not
  //        set its own accent; the other built-in themes bring theirs (globals.css).
  //        Declared after the theme effect, so its inline `--accent` lands last.
  useEffect(() => {
    if (!settings) return;
    const root = document.documentElement.style;
    const accent = ACCENTS[settings.accent] ?? ACCENTS.blue;
    const ownAccent = theme.custom !== null && "--accent" in theme.custom.colors;
    if (accent) {
      root.setProperty("--accent-hue", String(accent.hue));
      if (!ownAccent && (followsAccentSetting(theme.id) || theme.custom !== null)) root.setProperty("--accent", theme.base === "light" ? accent.light : accent.color);
    }
    root.setProperty("--font-sans", fontStack(settings.uiFont, UI_FONT_FALLBACK));
    root.setProperty("--font-mono", fontStack(settings.editorFont, EDITOR_FONT_FALLBACK));
    root.setProperty("--ui-font-size", `${settings.uiFontSize}px`);
    root.setProperty("--editor-font-size", `${settings.editorFontSize}px`);
    document.body.style.fontSize = `${settings.uiFontSize}px`;
    document.documentElement.setAttribute("data-scrollbar", settings.gridScrollbarSize === "large" ? "large" : "standard");
  }, [settings, theme]);

  return (
    <TooltipProvider>
      <UiPrefsProvider datePickerEnabled={settings?.datePickerEnabled ?? true}>
        <div className="grid-bg flex h-full flex-col text-foreground">
          <div className="flex min-h-0 flex-1">
            <IconRail />
            {!ready ? null : page.kind === "settings" ? (
              <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
                <SettingsPage />
              </div>
            ) : page.kind === "capabilities" ? (
              <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
                <CapabilityMatrixPage />
              </div>
            ) : page.kind === "whats-new" ? (
              <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
                <WhatsNewPage />
              </div>
            ) : page.kind === "connection-picker" ? (
              <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
                <ConnectionPicker />
              </div>
            ) : page.kind === "connection-form" ? (
              <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
                <ConnectionForm />
              </div>
            ) : page.kind === "connections" || !connection || !connected ? (
              <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
                <ConnectionsPage />
              </div>
            ) : (
              <>
                <Sidebar />
                <main className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
                  <EnvironmentEdge environment={connection.environment} />
                  <div className="flex min-h-0 flex-1">
                    <PaneArea />
                    {changesOpen ? <PendingChangesPanel connectionId={connection.id} /> : null}
                  </div>
                </main>
              </>
            )}
          </div>
          <UpdateBar />
          <CommandPalette />
          <BackupDialog />
          {/* OS-opened database files and window drops work on every page. */}
          <ConnectionEvents />
          <AboutDialog />
          <CloseConfirmDialog />
          <DragLayer />
          <Toaster />
        </div>
      </UiPrefsProvider>
    </TooltipProvider>
  );
}
